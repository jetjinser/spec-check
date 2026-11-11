//! Connect explicit resource slots from compiler provenance; Verus checks the result.
use crate::{
    Result,
    capture::Invocation,
    hash,
    ir::{self, Record},
    runner,
    vir::{self, Node},
    write_json,
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

pub(crate) fn arguments(call: &Record) -> Result<Vec<String>> {
    let re = Regex::new(r"Spanned \{ node: (.*?), span: ")?;
    Ok(re
        .captures_iter(&call["arguments"])
        .map(|c| c[1].to_owned())
        .collect())
}
pub(crate) fn place(operand: &str) -> Option<&str> {
    operand
        .strip_prefix("move ")
        .or_else(|| operand.strip_prefix("copy "))
        .filter(|s| s.starts_with('_') && s[1..].chars().all(|c| c.is_ascii_digit()))
}
fn local_type<'a>(records: &'a [Record], place: &str) -> Option<&'a str> {
    records
        .iter()
        .find(|r| r["kind"] == "local" && r["place"] == place)
        .map(|r| r["type"].as_str())
}
fn tracked(typ: &str) -> bool {
    typ.starts_with("vstd::prelude::Tracked<")
}
/// Select a borrowed permission conversion by its VIR types, never its name.
/// The caller must still prove its preconditions and check the instantiated call.
pub(crate) fn borrow_conversion<'a>(
    owner: &Node,
    borrowed: &Node,
    functions: &'a [Node],
) -> Result<&'a Node> {
    let matches = functions
        .iter()
        .filter(|f| f.field("mode").and_then(Node::atom) == Some("Proof"))
        .filter(|f| {
            let Some([param]) = f.field("params").and_then(Node::list) else {
                return false;
            };
            let Some(typ) = param.field("typ").and_then(Node::list) else {
                return false;
            };
            typ.len() == 5
                && typ[0].atom() == Some("Typ")
                && typ[1].atom() == Some("Decorate")
                && typ[2].list().is_some_and(|v| {
                    v.len() == 2
                        && v[0].atom() == Some("TypDecoration")
                        && v[1].atom() == Some("Ref")
                })
                && typ[4] == *owner
                && f.field("outer_ret").and_then(|r| r.field("typ")) == Some(borrowed)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [conversion] => Ok(conversion),
        [] => Err("no proof conversion from borrowed owner to required permission".into()),
        _ => Err("ambiguous borrowed permission conversion".into()),
    }
}
fn resource_calls(records: &[Record]) -> Result<Vec<&Record>> {
    let mut result = Vec::new();
    for c in records.iter().filter(|r| r["kind"] == "call") {
        let args = arguments(c)?;
        let typ = local_type(records, &c["destination"]).unwrap_or("");
        let tuple = typ.starts_with('(') && typ.contains("vstd::prelude::Tracked<");
        let receiver = args
            .first()
            .and_then(|s| place(s))
            .and_then(|p| local_type(records, p));
        let resource = receiver.is_some_and(|t| !tracked(t) && !t.starts_with("Binder "))
            && args
                .iter()
                .skip(1)
                .filter_map(|a| place(a))
                .filter_map(|p| local_type(records, p))
                .any(tracked);
        if tuple || resource {
            result.push(c);
        }
    }
    if result.is_empty() {
        return Err("no supported resource calls".into());
    }
    Ok(result)
}
fn item(call: &Record) -> Result<String> {
    let target = &call["target"];
    let generic = call["generic_args"]
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("generic argument format")?;
    // Bound initially to simple type/const arguments, without trait-qualified targets or lifetimes.
    if !Regex::new(r"^[\w:, <>]*$")?.is_match(generic)
        || generic.contains('>')
        || generic.contains('<')
    {
        return Err("nested or lifetime generic resource arguments unsupported".into());
    }
    let result = if let Some(start) = target.find("::<") {
        let end = target[start..].find('>').ok_or("generic target format")? + start;
        format!("{}::<{}>{}", &target[..start], generic, &target[end + 1..])
    } else if generic.is_empty() {
        target.clone()
    } else {
        return Err("generic resource function target unsupported".into());
    };
    if !Regex::new(r"^[\w:, <>]+$")?.is_match(&result) {
        return Err("resource function path unsupported".into());
    }
    Ok(result)
}
fn catalogue(records: &[Record]) -> Result<String> {
    let paths = resource_calls(records)?
        .iter()
        .map(|c| item(c))
        .collect::<Result<BTreeSet<_>>>()?;
    Ok(format!(
        "use vstd::prelude::*;\nverus! {{ pub exec fn __spec_resource_catalogue() {{\n{}\n}} }}\n",
        paths
            .iter()
            .map(|s| format!("let _ = {s};"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}
fn annotation(node: &Node) -> Result<&str> {
    node.list()
        .and_then(|s| s.get(1))
        .and_then(Node::atom)
        .ok_or_else(|| "missing VIR span".into())
}
pub(crate) fn signature<'a>(call: &Record, functions: &'a [Node], cwd: &Path) -> Result<&'a Node> {
    let (path, pos) = ir::span(&call["definition_span"], cwd)?;
    let path = fs::canonicalize(path)?;
    let found = functions
        .iter()
        .filter(|f| {
            annotation(f)
                .ok()
                .and_then(|s| ir::span(s, cwd).ok())
                .is_some_and(|(p, c)| {
                    fs::canonicalize(p).is_ok_and(|p| p == path) && c[..2] == pos[..2]
                })
        })
        .collect::<Vec<_>>();
    let f = resolve_signature(&found)?;
    if f.field("mode").and_then(Node::atom) != Some("Exec") {
        return Err("resource callee is not exec".into());
    }
    Ok(f)
}
// Macro-generated interfaces share a source span. Match the executable signature
// after removing only explicit ghost components; names are not an identity test.
pub(crate) fn resolve_signature<'a>(found: &[&'a Node]) -> Result<&'a Node> {
    if let [f] = found {
        return Ok(f);
    }
    let stubs = found.iter().copied().filter(|f| {
        f.field("mode").and_then(Node::atom) == Some("Exec")
            && f.field("attrs").and_then(|a| a.field("is_external_body")).and_then(Node::atom) == Some("true")
            && f.field("require").and_then(Node::list).is_some_and(|r| {
                matches!(r, [r] if r.unwrap() == &Node::List(vec![
                    Node::Atom(">".into()), Node::Atom("Const".into()),
                    Node::List(vec![Node::Atom("Constant".into()), Node::Atom("Bool".into()), Node::Atom("false".into())])]))
            })
    }).collect::<Vec<_>>();
    let [stub] = stubs.as_slice() else {
        return Err("resource signature has no unique source identity".into());
    };
    let params = |f: &Node| -> Option<Vec<Node>> {
        f.field("params")?
            .list()?
            .iter()
            .map(|p| {
                let t = p.field("typ")?;
                Some((p.field("mode")?.atom()? == "Exec" && !ghost_type(t)).then(|| t.clone()))
            })
            .collect::<Option<Vec<_>>>()
            .map(|v| v.into_iter().flatten().collect())
    };
    // A stub with physical ghost arguments needs a different erasure contract.
    let raw = stub
        .field("params")
        .and_then(Node::list)
        .ok_or("missing stub parameters")?;
    let runtime_params = params(stub).ok_or("missing stub parameter types")?;
    if raw.len() != runtime_params.len() {
        return Err("runtime stub contains ghost parameters".into());
    }
    let ret = stub
        .field("outer_ret")
        .and_then(|r| r.field("typ"))
        .ok_or("missing stub return")?;
    let candidates = found
        .iter()
        .copied()
        .filter(|f| {
            f.field("mode").and_then(Node::atom) == Some("Exec")
                && f.field("attrs")
                    .and_then(|a| a.field("is_external_body"))
                    .and_then(Node::atom)
                    == Some("false")
                && f.field("typ_params") == stub.field("typ_params")
                && f.field("typ_bounds") == stub.field("typ_bounds")
                && params(f).as_ref() == Some(&runtime_params)
                && f.field("outer_ret")
                    .and_then(|r| r.field("typ"))
                    .and_then(runtime_return)
                    .as_ref()
                    == Some(ret)
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [f] => Ok(f),
        _ => Err("resource signature has no unique verified projection".into()),
    }
}
fn ghost_type(t: &Node) -> bool {
    decoration(t, "Tracked").is_some() || decoration(t, "Ghost").is_some()
}
fn runtime_return(t: &Node) -> Option<Node> {
    let v = t.list()?;
    if v.get(2)
        .and_then(Node::list)
        .and_then(|d| d.get(1))
        .and_then(Node::atom)
        == Some("Tuple")
    {
        let fields = v.get(3)?.list()?;
        if fields.iter().any(ghost_type) {
            let runtime = fields.iter().filter(|t| !ghost_type(t)).collect::<Vec<_>>();
            return match runtime.as_slice() {
                [t] => Some((*t).clone()),
                _ => None,
            };
        }
    }
    (!ghost_type(t)).then(|| t.clone())
}
// A decoration has the shape (Typ Decorate (TypDecoration X) None INNER).
pub(crate) fn decoration<'a>(typ: &'a Node, name: &str) -> Option<&'a Node> {
    let v = typ.list()?;
    (v.get(1)?.atom() == Some("Decorate") && v.get(2)?.list()?.get(1)?.atom() == Some(name))
        .then(|| &v[4])
}
fn token_mode(typ: &Node) -> Result<&'static str> {
    let inner = decoration(typ, "Tracked").ok_or("parameter is not Tracked")?;
    Ok(if decoration(inner, "Ref").is_some() {
        "borrow"
    } else if inner.list().and_then(|v| v.get(1)).and_then(Node::atom) == Some("MutRef") {
        "borrow_mut"
    } else {
        "consume"
    })
}
pub(crate) fn dominates(records: &[Record]) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let re = Regex::new(r"bb\d+")?;
    let edges = records
        .iter()
        .filter(|r| r["kind"] == "edge")
        .map(|r| {
            (
                r["block"].clone(),
                re.find_iter(&r["successors"])
                    .map(|m| m.as_str().to_owned())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut reachable = BTreeSet::from(["bb0".to_owned()]);
    loop {
        let n = reachable.len();
        for b in reachable.clone() {
            reachable.extend(edges.get(&b).into_iter().flatten().cloned());
        }
        if n == reachable.len() {
            break;
        }
    }
    let mut dom = reachable
        .iter()
        .map(|b| {
            (
                b.clone(),
                if b == "bb0" {
                    BTreeSet::from([b.clone()])
                } else {
                    reachable.clone()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    loop {
        let old = dom.clone();
        for b in reachable.iter().filter(|b| b.as_str() != "bb0") {
            let mut pred = edges
                .iter()
                .filter(|(p, es)| reachable.contains(*p) && es.contains(b))
                .map(|(p, _)| &old[p]);
            let mut d = pred.next().ok_or("missing predecessor")?.clone();
            for p in pred {
                d = d.intersection(p).cloned().collect();
            }
            d.insert(b.clone());
            dom.insert(b.clone(), d);
        }
        if old == dom {
            return Ok(dom);
        }
    }
}
pub(crate) fn origin(
    records: &[Record],
    local: &str,
    seeds: &BTreeMap<String, usize>,
    seen: &mut BTreeSet<String>,
) -> Result<usize> {
    if let Some(i) = seeds.get(local) {
        let prefix = format!("{local} = ");
        if records
            .iter()
            .filter(|r| r["kind"] == "stmt" && r["operation"].starts_with(&prefix))
            .count()
            != 1
            || records
                .iter()
                .any(|r| r["kind"] == "call" && r["destination"] == local)
        {
            return Err("constructor owner was reassigned".into());
        }
        return Ok(*i);
    }
    if !seen.insert(local.into()) {
        return Err("cyclic resource provenance".into());
    }
    let prefix = format!("{local} = ");
    let defs = records
        .iter()
        .filter(|r| r["kind"] == "stmt" && r["operation"].starts_with(&prefix))
        .collect::<Vec<_>>();
    let [d] = defs.as_slice() else {
        return Err("resource receiver needs a unique local definition".into());
    };
    if records
        .iter()
        .any(|r| r["kind"] == "call" && r["destination"] == local)
    {
        return Err("resource alias also has a call assignment".into());
    }
    let value = &d["operation"][prefix.len()..];
    let parent = value
        .strip_prefix('&')
        .and_then(|s| s.strip_prefix("mut ").or(Some(s)))
        .or_else(|| place(value))
        .ok_or("resource receiver alias unsupported")?;
    origin(records, parent, seeds, seen)
}
fn explicit_slot<'a>(
    records: &'a [Record],
    call: &Record,
    index: usize,
    file: &Path,
    source: &str,
    cwd: &Path,
) -> Result<(usize, usize, &'a Record)> {
    let args = arguments(call)?;
    let argument = args
        .get(index)
        .and_then(|a| place(a))
        .ok_or("resource argument must be local")?;
    let argument_type = local_type(records, argument).ok_or("resource argument type missing")?;
    if !tracked(argument_type) {
        return Err("VIR and Rustc resource type disagree".into());
    }
    let spans = Regex::new(r"Spanned \{ node: .*?, span: (.*?) \}")?;
    let argument_span = spans
        .captures_iter(&call["arguments"])
        .nth(index)
        .ok_or("missing resource argument span")?;
    let (argument_file, coordinates) = ir::span(&argument_span[1], cwd)?;
    let expected = ir::offsets(source, coordinates)?;
    let terms = records
        .iter()
        .filter(|r| r["kind"] == "term" && r["block"] == call["block"])
        .collect::<Vec<_>>();
    let [term] = terms.as_slice() else {
        return Err("unique resource call span required".into());
    };
    let (call_file, call_coordinates) = ir::span(&term["span"], cwd)?;
    if argument_file != file || call_file != file {
        return Err("resource argument source differs from caller".into());
    }
    // HIR erases regions; MIR prints inference-region numbers. Other type text is retained.
    let regions = Regex::new(r"'(?:\{erased\}|\?\d+)")?;
    let expected_type = regions.replace_all(argument_type, "'_");
    let slots = records
        .iter()
        .filter(|r| r["kind"] == "source_argument")
        .filter_map(|r| {
            if r["index"].parse::<usize>() != Ok(index)
                || regions.replace_all(&r["type"], "'_") != expected_type
            {
                return None;
            }
            let (parent, position) = ir::span(&r["call_span"], cwd).ok()?;
            if parent != call_file || position != call_coordinates {
                return None;
            }
            let (path, position) = ir::span(&r["span"], cwd).ok()?;
            let (start, end) = ir::offsets(source, position).ok()?;
            (path == file && (start, end) == expected && source[start..end] == *"Tracked(())")
                .then_some((start, end, r))
        })
        .collect::<Vec<_>>();
    match slots.as_slice() {
        [slot] => Ok(*slot),
        _ => Err("exactly one typed tracked slot for the resource call required".into()),
    }
}

fn plan(records: &[Record], native: &str, cwd: &Path) -> Result<(String, Value)> {
    let functions = vir::functions(native)?;
    let calls = resource_calls(records)?;
    let dom = dominates(records)?;
    for (i, a) in calls.iter().enumerate() {
        for b in calls.iter().skip(i + 1) {
            if !dom
                .get(&a["block"])
                .is_some_and(|d| d.contains(&b["block"]))
                && !dom
                    .get(&b["block"])
                    .is_some_and(|d| d.contains(&a["block"]))
            {
                return Err("branch-dependent resource order unsupported".into());
            }
        }
    }
    let mut calls = calls;
    calls.sort_by_key(|c| dom[&c["block"]].len());
    let body = records
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("missing body")?;
    let (file, coords) = ir::span(&body["span"], cwd)?;
    let source = fs::read_to_string(&file)?;
    let (start, end) = ir::offsets(&source, coords)?;
    if source.contains("__spec_resource") {
        return Err("generated resource name collides".into());
    }
    let mut edits = vec![
        (start + 1, start + 1, "\nverus_exec_expr! { {\n".to_owned()),
        (end - 1, end - 1, "\n} }\n".into()),
    ];
    let mut seeds = BTreeMap::new();
    let mut owners = Vec::new();
    let mut used = BTreeSet::new();
    let mut evidence = Vec::new();
    let projection = Regex::new(r"^(_\d+) = (?:copy|move) \((_\d+)\.0: ")?;
    for call in calls {
        let f = signature(call, &functions, cwd)?;
        let params = f
            .field("params")
            .and_then(Node::list)
            .ok_or("missing parameters")?;
        let ret = f
            .field("outer_ret")
            .and_then(|r| r.field("typ"))
            .ok_or("missing return type")?;
        let dest = local_type(records, &call["destination"]).unwrap_or("");
        if dest.starts_with('(') && dest.contains("vstd::prelude::Tracked<") {
            let tuple = ret.list().ok_or("constructor return type")?;
            if tuple
                .get(2)
                .and_then(Node::list)
                .and_then(|v| v.get(1))
                .and_then(Node::atom)
                != Some("Tuple")
            {
                return Err("resource constructor needs tuple return".into());
            }
            let fields = tuple.get(3).and_then(Node::list).ok_or("tuple fields")?;
            if fields.len() != 2
                || decoration(&fields[1], "Tracked").is_none()
                || decoration(&fields[0], "Tracked").is_some()
            {
                return Err("constructor needs value/token pair".into());
            }
            let projections = records
                .iter()
                .filter(|r| r["kind"] == "stmt")
                .filter_map(|r| {
                    projection
                        .captures(&r["operation"])
                        .filter(|c| c[2] == call["destination"])
                        .map(|c| c[1].to_owned())
                })
                .collect::<Vec<_>>();
            let [owner] = projections.as_slice() else {
                return Err("constructor value needs unique tuple projection".into());
            };
            let bindings = records
                .iter()
                .filter(|r| r["kind"] == "binding" && r["place"] == *owner)
                .collect::<Vec<_>>();
            let [binding] = bindings.as_slice() else {
                return Err("constructor needs source binding".into());
            };
            let (_, pos) = ir::span(&binding["span"], cwd)?;
            let (bs, _) = ir::offsets(&source, pos)?;
            let prefix = Regex::new(&format!(
                r"\A{}\s*,\s*(_)\s*\)\s*=",
                regex::escape(&binding["name"])
            ))?;
            let cap = prefix
                .captures(&source[bs..])
                .ok_or("constructor resource binding must be wildcard")?;
            let hole = cap.get(1).unwrap();
            let id = owners.len();
            let token = format!("__spec_resource{id}");
            edits.push((
                bs + hole.start(),
                bs + hole.end(),
                format!("Tracked(mut {token})"),
            ));
            seeds.insert(owner.clone(), id);
            owners.push((token.clone(), call["block"].clone()));
            evidence.push(json!({"kind":"constructor","call":call,"token":token,"owner_place":owner,"signature":annotation(f)?}));
        } else {
            let args = arguments(call)?;
            if args.len() != params.len() {
                return Err("resource parameter arity differs".into());
            }
            let receiver = args
                .first()
                .and_then(|s| place(s))
                .ok_or("resource needs direct receiver")?;
            let id = origin(records, receiver, &seeds, &mut BTreeSet::new())?;
            let (token, ctor) = &owners[id];
            if !dom[&call["block"]].contains(ctor) || used.contains(&id) {
                return Err("resource unavailable or already consumed".into());
            }
            let tracked_params = params
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.field("typ")
                        .is_some_and(|t| decoration(t, "Tracked").is_some())
                })
                .collect::<Vec<_>>();
            if tracked_params.len() != 1 {
                return Err("exactly one explicit tracked slot required per call".into());
            }
            let (index, param) = tracked_params[0];
            let (slot_start, slot_end, slot) =
                explicit_slot(records, call, index, &file, &source, cwd)?;
            let mode = token_mode(param.field("typ").unwrap())?;
            let value = match mode {
                "borrow" => format!("&{token}"),
                "borrow_mut" => format!("&mut {token}"),
                _ => {
                    used.insert(id);
                    token.clone()
                }
            };
            edits.push((slot_start, slot_end, format!("Tracked({value})")));
            evidence.push(json!({"kind":mode,"call":call,"token":token,"owner_place":receiver,"signature":annotation(f)?,"tracked_parameter_index":index,"source_argument":slot}));
        }
    }
    if owners.is_empty() || used.len() != owners.len() {
        return Err("complete construction-to-consumption path required".into());
    }
    edits.sort_by_key(|e| e.0);
    if edits.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err("overlapping resource edits".into());
    }
    let mut changed = source;
    for (s, e, text) in edits.iter().rev() {
        changed.replace_range(*s..*e, text)
    }
    Ok((
        changed,
        json!({"connections":evidence,"manual_call_mappings":0,"tokens":owners.len()}),
    ))
}

pub(crate) fn tuple_fields(t: &Node) -> Option<&[Node]> {
    let v = t.list()?;
    (v.get(2)?.list()?.get(1)?.atom() == Some("Tuple"))
        .then(|| v.get(3)?.list())
        .flatten()
}
fn rust_type(t: &Node) -> Result<String> {
    let v = t.list().ok_or("missing token type")?;
    if v.get(1).and_then(Node::atom) == Some("Int")
        && v.get(2)
            .and_then(Node::list)
            .and_then(|v| v.get(1))
            .and_then(Node::atom)
            == Some("USize")
    {
        return Ok("usize".into());
    }
    let d = v
        .get(2)
        .and_then(Node::list)
        .ok_or("token datatype missing")?;
    if v.get(1).and_then(Node::atom) != Some("Datatype")
        || d.get(1).and_then(Node::atom) != Some("Path")
    {
        return Err("unsupported token datatype".into());
    }
    let path = d.get(2).and_then(Node::atom).ok_or("token path missing")?;
    if !Regex::new(r"^\w+(::\w+)*$")?.is_match(path) {
        return Err("unsupported token path".into());
    }
    let args = v
        .get(3)
        .and_then(Node::list)
        .ok_or("token arguments missing")?
        .iter()
        .map(rust_type)
        .collect::<Result<Vec<_>>>()?;
    Ok(if args.is_empty() {
        path.into()
    } else {
        format!("{path}<{}>", args.join(", "))
    })
}
fn local_root(records: &[Record], local: &str, seen: &mut BTreeSet<String>) -> Result<String> {
    if !seen.insert(local.into()) {
        return Err("cyclic local definition".into());
    }
    let prefix = format!("{local} = ");
    let defs = records
        .iter()
        .filter(|r| r["kind"] == "stmt" && r["operation"].starts_with(&prefix))
        .collect::<Vec<_>>();
    let calls = records
        .iter()
        .filter(|r| r["kind"] == "call" && r["destination"] == local)
        .count();
    match (defs.as_slice(), calls) {
        ([], 1) => Ok(local.into()),
        ([d], 0) => {
            let value = &d["operation"][prefix.len()..];
            let parent = value
                .strip_prefix('&')
                .or_else(|| place(value))
                .ok_or("unsupported resource alias")?;
            local_root(records, parent, seen)
        }
        _ => Err("resource local must have exactly one definition".into()),
    }
}
fn optional_token(ret: &Node) -> Option<(&Node, &Node)> {
    let [_, token] = tuple_fields(ret)? else {
        return None;
    };
    let option = decoration(token, "Tracked")?;
    let v = option.list()?;
    if v.get(2)?.list()?.get(2)?.atom()? != "core::option::Option" {
        return None;
    }
    let [payload] = v.get(3)?.list()? else {
        return None;
    };
    Some((option, payload))
}
fn conditional_plan(records: &[Record], native: &str, cwd: &Path) -> Result<(String, Value)> {
    let functions = vir::functions(native)?;
    let calls = records
        .iter()
        .filter(|r| r["kind"] == "call")
        .collect::<Vec<_>>();
    // The first supported conditional flow has one producer, one guard, and one
    // consuming call. Additional resource flows require a separate consumption plan.
    let mut producers = Vec::new();
    let mut consumers = Vec::new();
    for c in &calls {
        let Ok(f) = signature(c, &functions, cwd) else {
            continue;
        };
        let ret = f
            .field("outer_ret")
            .and_then(|p| p.field("typ"))
            .ok_or("missing return")?;
        if let Some((option, payload)) = optional_token(ret) {
            producers.push((*c, f, option, payload));
        }
        let ps = f
            .field("params")
            .and_then(Node::list)
            .ok_or("missing parameters")?;
        let ts = ps
            .iter()
            .filter_map(|p| p.field("typ").and_then(|t| decoration(t, "Tracked")))
            .collect::<Vec<_>>();
        if let [payload] = ts.as_slice() {
            consumers.push((*c, f, *payload));
        }
    }
    let [(producer, pf, option, payload)] = producers.as_slice() else {
        return Err("one conditional resource producer required".into());
    };
    let [(consumer, cf, expected)] = consumers.as_slice() else {
        return Err("one consuming resource call required".into());
    };
    if payload != expected {
        return Err("producer and consumer token types differ".into());
    }
    let receiver = |c: &Record| -> Result<String> {
        let args = arguments(c)?;
        local_root(
            records,
            args.first()
                .and_then(|a| place(a))
                .ok_or("receiver missing")?,
            &mut BTreeSet::new(),
        )
    };
    let owner = receiver(producer)?;
    if receiver(consumer)? != owner {
        return Err("resource owner differs".into());
    }
    let guards = calls
        .iter()
        .filter(|c| c["target"] == "core::result::Result::<T, E>::is_ok")
        .filter(|c| {
            arguments(c)
                .ok()
                .and_then(|a| {
                    a.first()
                        .and_then(|a| place(a))
                        .and_then(|p| local_root(records, p, &mut BTreeSet::new()).ok())
                })
                .as_deref()
                == Some(&producer["destination"])
        })
        .copied()
        .collect::<Vec<_>>();
    let [guard] = guards.as_slice() else {
        return Err("unique compiler-resolved Result::is_ok guard required".into());
    };
    let dom = dominates(records)?;
    let branch = records
        .iter()
        .filter(|r| r["kind"] == "term")
        .find_map(|r| {
            Regex::new(&format!(
                r"^switchInt\((?:move|copy) {}\) -> \[0: (bb\d+), otherwise: (bb\d+)\]$",
                regex::escape(&guard["destination"])
            ))
            .ok()?
            .captures(&r["operation"])
            .map(|c| (r, c[2].to_owned()))
        })
        .ok_or("success branch not identified")?;
    if !dom[&consumer["block"]].contains(&branch.1)
        || !dom[&branch.0["block"]].contains(&producer["block"])
    {
        return Err("resource consumer is not dominated by successful production".into());
    }
    let other = calls
        .iter()
        .filter(|c| *c != producer && *c != consumer && *c != guard)
        .collect::<Vec<_>>();
    let [ctor] = other.as_slice() else {
        return Err("closed constructor/producer/guard/consumer flow required".into());
    };
    if ctor["destination"] != owner || !dom[&producer["block"]].contains(&ctor["block"]) {
        return Err("owner must be constructed before production".into());
    }
    for (call, f, extra) in [(producer, pf, 0), (consumer, cf, 1)] {
        let params = f
            .field("params")
            .and_then(Node::list)
            .ok_or("parameters missing")?;
        if arguments(call)?.len() + extra != params.len() {
            return Err("unsupported conditional argument projection".into());
        }
    }
    let body = records
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("body missing")?;
    let (file, _) = ir::span(&body["span"], cwd)?;
    let mut source = fs::read_to_string(&file)?;
    if source.contains("__spec_resource") {
        return Err("generated names collide".into());
    }
    let call_span = |c: &Record| -> Result<(usize, usize)> {
        let t = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == c["block"])
            .ok_or("call span missing")?;
        let (p, coords) = ir::span(&t["span"], cwd)?;
        if p != file {
            return Err("call outside client source".into());
        }
        ir::offsets(&source, coords)
    };
    let (ps, pe) = call_span(producer)?;
    let (cs, ce) = call_span(consumer)?;
    let statement = source[..ps].rfind('\n').map_or(0, |p| p + 1);
    if !source[statement..ps].trim_start().starts_with("let ") {
        return Err("producer needs a direct let binding".into());
    }
    let decl = records
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("client declaration missing")?;
    let (_, coords) = ir::span(&decl["span"], cwd)?;
    let (ds, _) = ir::offsets(&source, coords)?;
    let edits = vec![
        (ds, ds, "#[verus_verify]\n".into()),
        (
            statement,
            statement,
            format!(
                "    proof_decl! {{ let tracked __spec_resource0: {}; }}\n",
                rust_type(option)?
            ),
        ),
        (
            ps,
            pe,
            format!(
                "(#[verus_spec(with => Tracked(__spec_resource0))] {})",
                &source[ps..pe]
            ),
        ),
        (
            cs,
            ce,
            format!(
                "proof_decl! {{ let tracked __spec_resource_value0 = __spec_resource0.tracked_unwrap(); }}\n        #[verus_spec(with Tracked(__spec_resource_value0))]\n        {}",
                &source[cs..ce]
            ),
        ),
    ];
    for (a, b, t) in edits.into_iter().rev() {
        source.replace_range(a..b, &t);
    }
    Ok((
        source,
        json!({"status":"planned","producer":producer,"consumer":consumer,"guard":guard,"owner_place":owner,"producer_signature":annotation(pf)?,"consumer_signature":annotation(cf)?,"manual_call_mappings":0,"entry_permissions":0,"native_verification_required":true,"runtime_correspondence_required":true}),
    ))
}
pub(crate) fn conditional(records: &Path, native: &Path, cwd: &Path, out: &Path) -> Result<()> {
    fs::create_dir(out)?;
    let records = ir::parse(&fs::read_to_string(records)?)?;
    let native = fs::read_to_string(native)?;
    let (source, report) = conditional_plan(&records, &native, cwd)?;
    fs::write(out.join("connected.rs"), source)?;
    write_json(&out.join("connections.json"), &report)
}

// Remove only newly generated, unused, compiler-confirmed ZST token captures.
// Any read, call, borrow, destructor, or non-constructor assignment prevents erasure.
pub(crate) fn erase_captures(records: &[Record], connections: &Value) -> Result<Vec<Record>> {
    let binding_name = |e: &Value| {
        e["binding"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("verus_tmp_{}", e["token"].as_str().unwrap()))
    };
    let names = connections["connections"]
        .as_array()
        .ok_or("missing connections")?
        .iter()
        .filter(|e| e["kind"] == "constructor")
        .map(binding_name)
        .collect::<BTreeSet<_>>();
    let bindings = records
        .iter()
        .filter(|r| r["kind"] == "binding" && names.contains(&r["name"]))
        .collect::<Vec<_>>();
    if bindings.len() != names.len() {
        return Err("generated token capture identity missing".into());
    }
    let mut data = records
        .iter()
        .filter(|r| ir::runtime(r))
        .cloned()
        .collect::<Vec<_>>();
    let mut removed: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for binding in bindings {
        let local = &binding["place"];
        let layout = records
            .iter()
            .find(|r| r["kind"] == "resource_layout" && r["place"] == *local)
            .ok_or("missing resource layout")?;
        if !tracked(&binding["type"])
            || layout["definition"] != "vstd::prelude::Tracked"
            || layout["size"] != "0"
            || layout["align"] != "1"
            || layout["needs_drop"] != "false"
        {
            return Err("resource capture lacks builtin ZST/no-drop evidence".into());
        }
        let mentions = Regex::new(&format!(r"\b{}\b", regex::escape(local)))?;
        let projection = Regex::new(&format!(
            r"^{} = move \((_\d+)\.1: vstd::prelude::Tracked<.*>\)$",
            regex::escape(local)
        ))?;
        let mut assignments = 0;
        for r in data.iter().filter(|r| r["kind"] != "local") {
            let refers = r
                .iter()
                .filter(|(k, _)| !["span", "definition_span"].contains(&k.as_str()))
                .any(|(_, v)| mentions.is_match(v));
            if !refers {
                continue;
            }
            let Some(op) = r.get("operation").filter(|_| r["kind"] == "stmt") else {
                return Err("resource capture has a runtime use".into());
            };
            if let Some(c) = projection.captures(op) {
                let ctor = connections["connections"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e["kind"] == "constructor")
                    .find(|e| binding_name(e) == binding["name"])
                    .unwrap();
                let call = records
                    .iter()
                    .find(|r| {
                        r["kind"] == "call"
                            && r["destination"] == c[1]
                            && r["target"] == ctor["call"]["target"]
                    })
                    .ok_or("resource capture is not a constructor return")?;
                if local_type(records, &call["destination"]).is_none_or(|t| !t.starts_with('(')) {
                    return Err("resource capture tuple missing".into());
                }
                assignments += 1;
            } else if op != &format!("StorageLive({local})")
                && op != &format!("StorageDead({local})")
            {
                return Err("resource capture statement has an effect".into());
            }
            removed
                .entry(r["block"].clone())
                .or_default()
                .push(r["statement_index"].parse()?);
        }
        if assignments != 1 {
            return Err("resource capture needs exactly one constructor projection".into());
        }
        data.retain(|r| {
            if r["kind"] == "local" {
                r["place"] != *local
            } else {
                r.get("operation").is_none_or(|s| !mentions.is_match(s))
            }
        });
    }
    ir::remap(data, &removed, false)
}

pub(crate) fn run(
    context: &Path,
    source: &Path,
    function: &str,
    sdk: &Path,
    out: &Path,
) -> Result<()> {
    if out.exists() {
        return Err("resource output directory must be new".into());
    }
    fs::create_dir_all(out)?;
    let out = out.canonicalize()?;
    let sdk = sdk.canonicalize()?;
    let mut inv: Invocation = serde_json::from_slice(&fs::read(context)?)?;
    inv.env.insert("CLIENT_RESOURCE_LAYOUT".into(), "1".into());
    inv.env.insert("VERUS_USE_RUSTUP".into(), "0".into());
    inv.env.insert(
        "VERUS_Z3_PATH".into(),
        sdk.parent()
            .and_then(Path::parent)
            .ok_or("SDK layout")?
            .join("z3")
            .to_string_lossy()
            .into_owned(),
    );
    let roots = inv
        .args
        .iter()
        .filter(|s| s.ends_with(".rs") && inv.cwd.join(s).is_file())
        .collect::<Vec<_>>();
    let [original] = roots.as_slice() else {
        return Err("single Rust root required".into());
    };
    let original = inv.cwd.join(original);
    let input = out.join("input.rs");
    fs::copy(source, &input)?;
    write_json(
        &out.join("inputs.json"),
        &json!({"context_sha256":hash(&fs::read(context)?)?,"source":source,"source_sha256":hash(&fs::read(source)?)?,"function":function,"tool_sha256":hash(&fs::read(std::env::current_exe()?)?)?}),
    )?;
    let mut fingerprints = BTreeMap::new();
    for path in inv
        .args
        .windows(2)
        .filter(|w| w[0] == "--extern")
        .filter_map(|w| w[1].split_once('=').map(|(_, p)| p))
    {
        let path = Path::new(path);
        if path.is_file() {
            fingerprints.insert(path.to_owned(), hash(&fs::read(path)?)?);
        }
        let metadata = path.with_extension("vir");
        if metadata.is_file() {
            fingerprints.insert(metadata.clone(), hash(&fs::read(metadata)?)?);
        }
    }
    for name in ["verus", "rust_verify"] {
        let path = sdk.join(name);
        fingerprints.insert(path.clone(), hash(&fs::read(path)?)?);
    }
    write_json(&out.join("dependency-inputs.json"), &fingerprints)?;
    let probe = runner::build_probe(&inv, &out)?.ok_or("Rustc probe unavailable")?;
    let args = runner::clean(
        &inv.args,
        &out.join("compiler-output"),
        Some((&original, &input)),
        &inv.cwd,
    )?;
    let records = runner::extract_context(&probe, &inv, &args, function, &out, "input", false)?;
    write_json(&out.join("input-ir.json"), &records)?;
    let cat = out.join("catalogue.rs");
    fs::write(&cat, catalogue(&records)?)?;
    let catargs = runner::clean(
        &inv.args,
        &out.join("catalogue-output"),
        Some((&original, &cat)),
        &inv.cwd,
    )?;
    let checked = runner::prove(&sdk, &inv, &catargs, &out, "catalogue")?;
    if checked["status"] != "proved" {
        return Err("resource catalogue failed Verus verification".into());
    }
    let native = fs::read_to_string(out.join("catalogue-vir/crate.vir"))?;
    let (connected, connections) = match plan(&records, &native, &inv.cwd) {
        Ok(p) => p,
        Err(e) => {
            write_json(
                &out.join("report.json"),
                &json!({"status":"unsupported","reason":e.to_string()}),
            )?;
            return Ok(());
        }
    };
    write_json(&out.join("connections.json"), &connections)?;
    let generated = out.join("connected.rs");
    fs::write(&generated, connected)?;
    let genargs = runner::clean(
        &inv.args,
        &out.join("connected-output"),
        Some((&original, &generated)),
        &inv.cwd,
    )?;
    let after =
        runner::extract_context(&probe, &inv, &genargs, function, &out, "connected", false)?;
    write_json(
        &out.join("input-normalized.json"),
        &ir::normalize(&records, None)?,
    )?;
    write_json(
        &out.join("connected-normalized.json"),
        &ir::normalize(&after, None)?,
    )?;
    let erased = erase_captures(&after, &connections)?;
    write_json(
        &out.join("capture-erasure-normalized.json"),
        &ir::normalize(&erased, None)?,
    )?;
    let matches = ir::compare(&records, &erased, None)?;
    write_json(
        &out.join("correspondence.json"),
        &json!({"matches":matches,"scope":"compiler-erased runtime MIR and borrow records"}),
    )?;
    if !matches {
        write_json(
            &out.join("report.json"),
            &json!({"status":"unsupported","reason":"resource erasure MIR differs"}),
        )?;
        return Ok(());
    }
    let ghost = runner::extract_context(&probe, &inv, &genargs, function, &out, "ghost", true)?;
    let (_, lowered, edits) = ir::lower(&ghost, &inv.cwd)?;
    let lowered_path = out.join("checked.rs");
    fs::write(&lowered_path, lowered)?;
    write_json(&out.join("assertion-edits.json"), &edits)?;
    let checkargs = runner::clean(
        &inv.args,
        &out.join("checked-output"),
        Some((&original, &lowered_path)),
        &inv.cwd,
    )?;
    let verification = runner::prove(&sdk, &inv, &checkargs, &out, "checked")?;
    let marked =
        runner::extract_context(&probe, &inv, &checkargs, function, &out, "checked-ir", true)?;
    let helper = format!(
        "{}checked_eq_failure",
        function
            .rsplit_once("::")
            .map_or(String::new(), |(p, _)| format!("{p}::"))
    );
    let assertion_matches = ir::compare(
        &ghost,
        &marked,
        ghost
            .iter()
            .any(|r| r["kind"] == "assert_operand" && r["macro"] == "std::assert_eq")
            .then_some(helper.as_str()),
    )?;
    let crate_name = &inv
        .args
        .windows(2)
        .find(|w| w[0] == "--crate-name")
        .ok_or("missing crate name")?[1];
    let native = fs::read_to_string(out.join("checked-vir/crate.vir"))?;
    let target = vir::checked_target_in(
        &native,
        &format!("{crate_name}::{function}"),
        &out,
        &inv.cwd,
    )?;
    let target_fn = vir::functions(&native)?
        .into_iter()
        .find(|f| {
            f.field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
                == Some(format!("{crate_name}::{function}").as_str())
        })
        .ok_or("checked client not found")?;
    fn has_assume(n: &Node) -> bool {
        n.field("is_assume").and_then(Node::atom) == Some("true")
            || n.list().is_some_and(|v| v.iter().any(has_assume))
    }
    let closed = target
        .as_ref()
        .is_some_and(|t| t["existing_preconditions"] == 0)
        && target_fn
            .field("params")
            .and_then(Node::list)
            .is_some_and(|p| p.is_empty())
        && target_fn.field("body").is_some_and(|b| !has_assume(b));
    write_json(
        &out.join("contracts.json"),
        &crate::contracts::summarize(&native, &format!("{crate_name}::{function}"), &inv.cwd)?,
    )?;
    write_json(
        &out.join("target.json"),
        &json!({"target":target,"closed":closed,"assertion_correspondence":assertion_matches}),
    )?;

    let mut runtime =
        json!({"status":"not_run","reason":"runtime gated on successful proof and correspondence"});
    let unchanged = fingerprints
        .iter()
        .map(|(p, d)| Ok(hash(&fs::read(p)?)? == *d))
        .collect::<Result<Vec<bool>>>()?
        .iter()
        .all(|b| *b);
    if !unchanged {
        return Err("dependency artifacts changed during resource checking".into());
    }
    if verification["status"] == "proved" && assertion_matches && closed {
        let mut command = Command::new(&inv.rustc);
        runner::environment(&mut command, &inv);
        command
            .current_dir(&inv.cwd)
            .args(args.iter().filter(|s| !s.starts_with("--emit=")))
            .args(["--emit=link", "-o"])
            .arg(out.join("runtime-tests"));
        if runner::compile(&mut command, &out, "runtime-build", &inv.args)?
            .status
            .success()
        {
            runtime = runner::runtime(
                &out.join("runtime-tests").to_string_lossy(),
                function,
                &inv,
                &inv.cwd,
                &out,
                0,
            )?;
        }
    }
    write_json(
        &out.join("report.json"),
        &json!({"status":if assertion_matches && closed {verification["status"].clone()}else{json!("unsupported")},"dependency_artifacts_unchanged":unchanged,"target":target,"closed":closed,"assertion_correspondence":assertion_matches,"verification":verification,"runtime":runtime,"correspondence":{"matches":matches},"resources":connections,"scope":"explicit slots; single constructor value/token pairs; uniquely resolved local aliases and ordered calls"}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn tracked_slot_uses_call_parameter_type_and_rejects_ambiguity() {
        use super::*;
        let source = "p.call(Tracked(()));";
        let file = Path::new("/slot.rs");
        let begin = source.find("Tracked(())").unwrap();
        let end = begin + "Tracked(())".len();
        let call_span = format!("/slot.rs:1:1: 1:{} (#0)", source.len());
        let argument_span = format!("/slot.rs:1:{}: 1:{} (#0)", begin + 1, end + 1);
        let row = |fields: &[(&str, String)]| {
            fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<Record>()
        };
        let call = row(&[
            ("block", "bb1".into()),
            (
                "arguments",
                format!(
                    "[Spanned {{ node: copy _1, span: /slot.rs:1:1: 1:2 (#0) }}, Spanned {{ node: move _2, span: {argument_span} }}]"
                ),
            ),
        ]);
        let slot = row(&[
            ("kind", "source_argument".into()),
            ("index", "1".into()),
            (
                "type",
                "vstd::prelude::Tracked<&'{erased} Permission>".into(),
            ),
            ("span", argument_span.clone()),
            ("call_span", call_span.clone()),
        ]);
        let mut fallback = slot.clone();
        fallback.insert("index".into(), "0".into());
        fallback.insert("call_span".into(), argument_span.clone());
        fallback.insert("type".into(), "Closure(fallback)".into());
        let records = vec![
            row(&[
                ("kind", "term".into()),
                ("block", "bb1".into()),
                ("span", call_span),
            ]),
            row(&[
                ("kind", "local".into()),
                ("place", "_2".into()),
                ("type", "vstd::prelude::Tracked<&'?76 Permission>".into()),
            ]),
            slot.clone(),
            fallback,
        ];
        let selected = explicit_slot(&records, &call, 1, file, source, Path::new("/")).unwrap();
        assert_eq!((selected.0, selected.1), (begin, end));
        assert_eq!(selected.2, &slot);
        for mutation in [
            "duplicate",
            "call_span",
            "index",
            "type",
            "argument_span",
            "mir_type",
        ] {
            let mut changed = records.clone();
            match mutation {
                "duplicate" => changed.push(slot.clone()),
                "call_span" => {
                    changed[2].insert("call_span".into(), argument_span.clone());
                }
                "index" => {
                    changed[2].insert("index".into(), "0".into());
                }
                "type" => {
                    changed[2].insert(
                        "type".into(),
                        "vstd::prelude::Tracked<&'{erased} OtherPermission>".into(),
                    );
                }
                "argument_span" => {
                    changed[2].insert("span".into(), "/slot.rs:1:1: 1:2 (#0)".into());
                }
                "mir_type" => {
                    changed[1].insert(
                        "type".into(),
                        "vstd::prelude::Tracked<&'?76 mut Permission>".into(),
                    );
                }
                _ => unreachable!(),
            }
            assert!(
                explicit_slot(&changed, &call, 1, file, source, Path::new("/")).is_err(),
                "{mutation}"
            );
        }
    }

    #[test]
    #[ignore = "requires actual permission VIR in RESOURCE_BORROW_VIR"]
    fn native_borrow_conversion_uses_types() {
        use super::*;
        let text = fs::read_to_string(std::env::var("RESOURCE_BORROW_VIR").unwrap()).unwrap();
        let functions = vir::functions(&text).unwrap();
        let find = |suffix: &str| {
            functions
                .iter()
                .find(|f| {
                    f.field("name")
                        .and_then(|n| n.field("path"))
                        .and_then(Node::atom)
                        .is_some_and(|p| p == format!("ostd::sync::rcu::non_null::{suffix}"))
                })
                .unwrap()
        };
        let permission = |f: &Node| {
            f.field("params").unwrap().list().unwrap()[1]
                .field("typ")
                .unwrap()
                .list()
                .unwrap()[4]
                .clone()
        };
        let owner = permission(find("NonNullPtr::from_raw"));
        let borrowed = permission(find("NonNullPtrRef::raw_as_ref"));
        let selected = borrow_conversion(&owner, &borrowed, &functions).unwrap();
        assert!(
            selected
                .field("name")
                .unwrap()
                .field("path")
                .unwrap()
                .atom()
                .unwrap()
                .ends_with("NonNullPtrRef::borrow_perm_as_ref_perm")
        );
        // Identity spelling is deliberately removed from the selection input.
        let mut renamed = selected.unwrap().clone();
        let Node::List(fields) = &mut renamed else {
            panic!("function shape")
        };
        let i = fields
            .iter()
            .position(|n| n.atom() == Some(":name"))
            .unwrap();
        fields[i + 1] = Node::Atom("unrelated_name".into());
        assert!(borrow_conversion(&owner, &borrowed, &[renamed.clone()]).is_ok());
        assert!(borrow_conversion(&owner, &borrowed, &[renamed.clone(), renamed]).is_err());
        assert!(borrow_conversion(&borrowed, &owner, &functions).is_err());
    }
    use super::*;
    #[test]
    #[ignore = "requires SPEC_RESOURCE_CONNECT_RUN from resource-connect"]
    fn resource_capture_erasure_rejects_runtime_use_and_nonzero_layout() {
        let dir = std::path::PathBuf::from(std::env::var("SPEC_RESOURCE_CONNECT_RUN").unwrap());
        let records =
            ir::parse(&fs::read_to_string(dir.join("connected.stderr")).unwrap()).unwrap();
        let connections: Value =
            serde_json::from_slice(&fs::read(dir.join("connections.json")).unwrap()).unwrap();
        let before: Vec<Record> =
            serde_json::from_slice(&fs::read(dir.join("input-ir.json")).unwrap()).unwrap();
        assert!(
            ir::compare(
                &before,
                &erase_captures(&records, &connections).unwrap(),
                None
            )
            .unwrap()
        );
        let binding = records
            .iter()
            .find(|r| r["kind"] == "binding" && r["name"].starts_with("verus_tmp___spec_resource"))
            .unwrap();
        let mut changed = records.clone();
        let layout = changed
            .iter_mut()
            .find(|r| r["kind"] == "resource_layout" && r["place"] == binding["place"])
            .unwrap();
        layout.insert("size".into(), "8".into());
        assert!(erase_captures(&changed, &connections).is_err());
        let mut changed = records.clone();
        changed.push(BTreeMap::from([
            ("kind".into(), "call".into()),
            ("arguments".into(), format!("move {}", binding["place"])),
        ]));
        assert!(erase_captures(&changed, &connections).is_err());
        let mut changed = records.clone();
        changed
            .iter_mut()
            .find(|r| r["kind"] == "resource_layout" && r["place"] == binding["place"])
            .unwrap()
            .insert("needs_drop".into(), "true".into());
        assert!(erase_captures(&changed, &connections).is_err());
        let mut changed = records.clone();
        changed
            .iter_mut()
            .find(|r| {
                r["kind"] == "stmt"
                    && r["operation"].starts_with(&format!("{} = move (", binding["place"]))
            })
            .unwrap()
            .insert(
                "operation".into(),
                format!("{} = const ZeroSized", binding["place"]),
            );
        assert!(erase_captures(&changed, &connections).is_err());
    }
}

#[cfg(test)]
mod range_boundary {
    use super::*;
    #[test]
    #[ignore = "requires SPEC_RESOURCE_VIR and SPEC_RESOURCE_BOUNDARY_OUT"]
    fn range_macro_wrappers_resolve_by_erased_signature() {
        let path = std::path::PathBuf::from(std::env::var("SPEC_RESOURCE_VIR").unwrap());
        let native = fs::read_to_string(&path).unwrap();
        let functions = vir::functions(&native).unwrap();
        let mut report = Vec::new();
        for name in ["alloc_specific", "free"] {
            let selected = functions
                .iter()
                .filter(|f| {
                    f.field("name")
                        .and_then(|f| f.field("path"))
                        .and_then(Node::atom)
                        .is_some_and(|p| {
                            p.contains("::range_alloc::") && p.ends_with(&format!("::{name}"))
                        })
                })
                .collect::<Vec<_>>();
            let [runtime] = selected.as_slice() else {
                panic!("runtime wrapper missing")
            };
            let call = BTreeMap::from([(
                "definition_span".into(),
                annotation(runtime).unwrap().into(),
            )]);
            let resolved = signature(&call, &functions, Path::new("/")).unwrap();
            let verified = functions
                .iter()
                .find(|f| {
                    f.field("name")
                        .and_then(|f| f.field("path"))
                        .and_then(Node::atom)
                        .is_some_and(|p| {
                            p.contains("::range_alloc::")
                                && p.ends_with(&format!("::_VERUS_VERIFIED_{name}"))
                        })
                })
                .unwrap();
            assert_eq!(resolved, verified);
            let (source, coordinates) =
                ir::span(annotation(runtime).unwrap(), Path::new("/")).unwrap();
            let alias = std::env::temp_dir().join(format!(
                "resource-signature-{}-{name}.rs",
                std::process::id()
            ));
            std::os::unix::fs::symlink(&source, &alias).unwrap();
            let [sl, sc, el, ec] = coordinates;
            let alias_call = BTreeMap::from([(
                "definition_span".into(),
                format!("{}:{sl}:{sc}: {el}:{ec} (#0)", alias.display()),
            )]);
            let alias_result = signature(&alias_call, &functions, Path::new("/"));
            fs::remove_file(alias).unwrap();
            assert_eq!(alias_result.unwrap(), verified);
            assert!(resolve_signature(&[runtime, verified, verified]).is_err());
            let mut mismatched = verified.clone();
            fn change(node: &mut Node, field: &str, value: Node) -> bool {
                if let Node::List(items) = node {
                    if let Some(i) = items.iter().position(|n| n.atom() == Some(field)) {
                        items[i + 1] = value;
                        return true;
                    }
                    return items.iter_mut().any(|n| change(n, field, value.clone()));
                }
                false
            }
            assert!(change(
                &mut mismatched,
                ":typ",
                Node::List(vec![Node::Atom("Typ".into()), Node::Atom("Bool".into())])
            ));
            assert!(resolve_signature(&[runtime, &mismatched]).is_err());
            let mut external = verified.clone();
            assert!(change(
                &mut external,
                ":is_external_body",
                Node::Atom("true".into())
            ));
            assert!(resolve_signature(&[runtime, &external]).is_err());
            assert_eq!(
                ir::span(annotation(runtime).unwrap(), Path::new("/")).unwrap(),
                ir::span(annotation(verified).unwrap(), Path::new("/")).unwrap()
            );
            let verified_type = format!(
                "{:?}",
                verified
                    .field(if name == "free" {
                        "params"
                    } else {
                        "outer_ret"
                    })
                    .unwrap()
            );
            assert!(verified_type.contains("Tracked") && verified_type.contains("GhostSubRange"));
            if name == "alloc_specific" {
                assert!(verified_type.contains("option::Option"));
            }
            report.push(json!({"method":name,"runtime_span":annotation(runtime).unwrap(),"verified_span":annotation(verified).unwrap(),"unique_signature":"verified_projection","verified_resource_type":verified_type}));
        }
        write_json(Path::new(&std::env::var("SPEC_RESOURCE_BOUNDARY_OUT").unwrap()),&json!({"status":"interface_resolved","vir_sha256":hash(native.as_bytes()).unwrap(),"evidence":report,"missing":["conditional Option resource extraction and branch-local consumption","source edits for verus_spec extra return/argument attributes"]})).unwrap();
    }
}
