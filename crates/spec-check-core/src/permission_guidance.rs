//! Checked proof search for carriers of the SDK's exclusive PCell permission.
use crate::{
    Result,
    ir::{self, Edit, Record},
    vir::{self, Node},
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const PERMISSION: &str = "vstd::cell::pcell::PointsTo";
type FileEdits = Vec<(PathBuf, Edit)>;

/// Locate actual uses of the supported carrier types, without API-name filters.
pub(crate) fn usage_inventory(
    carriers: &[Carrier],
    functions: &[Node],
    inventory: &Value,
    cwd: &Path,
) -> Result<Value> {
    fn mentions(n: &Node, path: &str) -> bool {
        n.atom() == Some(path)
            || n.list()
                .is_some_and(|xs| xs.iter().any(|n| mentions(n, path)))
    }
    let mut endpoints = BTreeMap::new();
    for f in functions
        .iter()
        .filter(|f| f.field("mode").and_then(Node::atom) == Some("Exec"))
    {
        let used = carriers
            .iter()
            .filter(|c| {
                ["params", "outer_ret"]
                    .iter()
                    .filter_map(|k| f.field(k))
                    .any(|n| mentions(n, &c.path))
            })
            .map(|c| c.path.clone())
            .collect::<Vec<_>>();
        if used.is_empty() {
            continue;
        }
        let Some(origin) = f.list().and_then(|xs| xs.get(1)).and_then(Node::atom) else {
            continue;
        };
        if let Ok((file, coords)) = ir::span(origin, cwd)
            && let Ok(file) = fs::canonicalize(file)
        {
            endpoints
                .entry((file, coords[0], coords[1]))
                .or_insert_with(Vec::new)
                .extend(used);
        }
    }
    let records = inventory["records"]
        .as_array()
        .ok_or("missing compiler inventory")?;
    let owners = records
        .iter()
        .filter(|r| r["kind"] == "owner")
        .filter_map(|r| Some((r["owner"].as_str()?, r)))
        .collect::<BTreeMap<_, _>>();
    let mut calls = Vec::new();
    let mut contexts = BTreeMap::new();
    for call in records.iter().filter(|r| r["kind"] == "mir_call") {
        let Some((file, coords)) = call["definition_span"]
            .as_str()
            .and_then(|s| ir::span(s, cwd).ok())
        else {
            continue;
        };
        let Ok(file) = fs::canonicalize(file) else {
            continue;
        };
        let Some(carriers) = endpoints.get(&(file, coords[0], coords[1])) else {
            continue;
        };
        let mut carriers = carriers.clone();
        carriers.sort();
        carriers.dedup();
        let owner = call["owner"]
            .as_str()
            .and_then(|id| owners.get(id))
            .copied();
        if let Some(owner) = owner
            && let Some(id) = owner["owner"].as_str()
            && !contexts.contains_key(id)
        {
            let context = if owner["definition_kind"] == "Closure" {
                json!({"status":"nested_closure","scope":"not a separate ordinary entry"})
            } else if let Some(span) = owner["span"].as_str() {
                let definition = BTreeMap::from([("definition_span".into(), span.to_owned())]);
                match crate::resource::signature(&definition, functions, cwd) {
                    Ok(f) => {
                        json!({"status":if f.field("attrs").and_then(|a|a.field("is_external_body")).and_then(Node::atom)==Some("false") && f.field("body").is_some_and(|b|b.list().is_some()) {"native_body_present"}else{"external_contract"},"native_function":f.field("name").and_then(|n|n.field("path")).and_then(Node::atom)})
                    }
                    Err(error) => {
                        json!({"status":"native_entry_unresolved","reason":error.to_string()})
                    }
                }
            } else {
                json!({"status":"source_unavailable"})
            };
            contexts.insert(id.to_owned(), context);
        }
        calls.push(
            json!({"caller":owner,"caller_context":call["owner"].as_str().and_then(|id|contexts.get(id)),
            "call":call,"carrier_types":carriers}),
        );
    }
    Ok(
        json!({"scope":"Compiler calls whose source-bound VIR signature mentions a supported single-PointsTo carrier. These are investigation candidates, not complete resource paths or proof results.",
        "carrier_types":carriers.iter().map(|c| &c.path).collect::<Vec<_>>(),"calls":calls}),
    )
}
#[derive(Debug)]
pub(crate) struct Carrier {
    pub(crate) path: String,
    debug_path: String,
    rust_path: String,
    pub(crate) field: String,
    reveals: Vec<String>,
    crate_name: String,
    scopes: Vec<String>,
}
fn datatype(typ: &Node) -> Option<&str> {
    let xs = typ.unwrap().list()?;
    match xs.get(1)?.atom()? {
        "Decorate" => datatype(xs.get(4)?),
        "Datatype" => xs.get(2)?.list()?.get(2)?.atom(),
        _ => None,
    }
}
pub(crate) fn direct_carrier<'a>(typ: &Node, carriers: &'a [Carrier]) -> Option<&'a str> {
    if typ.list()?.get(1)?.atom()? != "Datatype" {
        return None;
    }
    let path = datatype(typ)?;
    carriers
        .iter()
        .find(|c| c.path == path)
        .map(|c| c.path.as_str())
}
pub(crate) fn tracked_permission(typ: &Node) -> bool {
    typ.list().is_some_and(|xs| {
        xs.get(1).and_then(Node::atom) == Some("Decorate")
            && xs
                .get(2)
                .and_then(Node::list)
                .and_then(|n| n.get(1))
                .and_then(Node::atom)
                == Some("Tracked")
            && xs
                .get(4)
                .and_then(Node::list)
                .and_then(|n| n.get(1))
                .and_then(Node::atom)
                == Some("Datatype")
            && xs.get(4).and_then(datatype) == Some(PERMISSION)
    })
}
pub(crate) fn catalogue(text: &str, crate_name: &str) -> Result<Vec<Carrier>> {
    let functions = vir::functions(text)?;
    let mut used = BTreeSet::new();
    for f in &functions {
        if f.field("mode").and_then(Node::atom) == Some("Exec")
            && let Some(ensure) = f.field("ensure")
        {
            let mut calls = Vec::new();
            vir::walk_calls(ensure, &mut calls);
            used.extend(calls);
        }
    }
    let identifier = Regex::new(r"^[A-Za-z_][A-Za-z_0-9]*$")?;
    let mut result = Vec::new();
    for d in vir::datatypes(text)? {
        let Some(path) = d
            .field("name")
            .and_then(Node::list)
            .and_then(|xs| xs.get(2))
            .and_then(Node::atom)
        else {
            continue;
        };
        let Some([variant]) = d.field("variants").and_then(Node::list) else {
            continue;
        };
        let Some(fields) = variant.field("fields").and_then(Node::list) else {
            continue;
        };
        let fields = fields
            .iter()
            .filter_map(|f| {
                let xs = f.list()?;
                let name = xs.get(1)?.atom()?;
                let tuple = xs.get(2)?.list()?;
                (identifier.is_match(name) && tracked_permission(tuple.get(1)?)).then(|| {
                    (
                        name.to_owned(),
                        tuple
                            .get(3)
                            .and_then(|v| v.field("restricted_to"))
                            .and_then(Node::atom)
                            .map(str::to_owned),
                    )
                })
            })
            .collect::<Vec<_>>();
        let [(field, Some(field_scope))] = fields.as_slice() else {
            continue;
        };
        let Some(type_scope) = d
            .field("visibility")
            .and_then(|v| v.field("restricted_to"))
            .and_then(Node::atom)
        else {
            continue;
        };
        let mut scopes = vec![field_scope.clone(), type_scope.to_owned()];
        let mut reveals = Vec::new();
        let mut invariant = false;
        let debug_path = path
            .strip_prefix(&format!("{crate_name}::"))
            .unwrap_or(path)
            .to_owned();
        let rust_path = if path.starts_with(&format!("{crate_name}::")) {
            format!("crate::{debug_path}")
        } else {
            path.to_owned()
        };
        for f in &functions {
            if f.field("mode").and_then(Node::atom) != Some("Spec") {
                continue;
            }
            let Some(first) = f
                .field("params")
                .and_then(Node::list)
                .and_then(|p| p.first())
            else {
                continue;
            };
            if first.field("typ").and_then(datatype) != Some(path) {
                continue;
            }
            if first
                .field("name")
                .and_then(Node::list)
                .and_then(|n| n.get(1))
                .and_then(Node::atom)
                != Some("self")
            {
                continue;
            }
            let Some(name) = f
                .field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
            else {
                continue;
            };
            let inv = f
                .field("attrs")
                .and_then(|a| a.field("is_type_invariant_fn"))
                .and_then(Node::atom)
                == Some("true");
            invariant |= inv;
            let closed = f
                .field("opaqueness")
                .and_then(|o| o.field("visibility"))
                .and_then(|v| v.field("restricted_to"))
                .and_then(Node::atom)
                .is_some_and(|v| v != "None");
            let method = name.rsplit("::").next().unwrap();
            if (inv || (closed && used.contains(name))) && identifier.is_match(method) {
                let Some(visibility) = f
                    .field("visibility")
                    .and_then(|v| v.field("restricted_to"))
                    .and_then(Node::atom)
                else {
                    continue;
                };
                scopes.push(visibility.to_owned());
                if let Some(scope) = f
                    .field("opaqueness")
                    .and_then(|o| o.field("visibility"))
                    .and_then(|v| v.field("restricted_to"))
                    .and_then(Node::atom)
                {
                    scopes.push(scope.to_owned());
                }
                reveals.push(format!("{rust_path}::{method}"));
            }
        }
        if invariant {
            reveals.sort();
            reveals.dedup();
            result.push(Carrier {
                path: path.to_owned(),
                debug_path,
                rust_path,
                field: field.clone(),
                reveals,
                crate_name: crate_name.to_owned(),
                scopes: scopes.into_iter().filter(|s| s != "None").collect(),
            });
        }
    }
    Ok(result)
}
fn source(span: &str, cwd: &Path) -> Result<(PathBuf, String, usize, usize)> {
    let (file, coords) = ir::span(span, cwd)?;
    let text = fs::read_to_string(&file)?;
    let (a, b) = ir::offsets(&text, coords)?;
    Ok((file, text, a, b))
}
fn live_at(records: &[Record], place: &str, target: &str) -> Result<bool> {
    let moved = Regex::new(&format!(r"\bmove \(?{}\b", regex::escape(place)))?;
    let blocks = Regex::new(r"bb\d+")?;
    let normal = Regex::new(r" -> \[return: (bb\d+)(?:,|\])")?;
    let mut todo = std::collections::VecDeque::from([("bb0".to_owned(), false)]);
    let mut seen = BTreeSet::new();
    let mut reached = false;
    while let Some((block, mut live)) = todo.pop_front() {
        if !seen.insert((block.clone(), live)) {
            continue;
        }
        let mut statements = records
            .iter()
            .filter(|r| r["kind"] == "stmt" && r["block"] == block)
            .collect::<Vec<_>>();
        statements.sort_by_key(|r| r["statement_index"].parse::<usize>().unwrap());
        for s in statements {
            if moved.is_match(&s["operation"])
                || s["operation"] == format!("StorageDead({place})")
                || s["operation"].starts_with(&format!("{place} = "))
            {
                live = false;
            }
        }
        let call = records
            .iter()
            .find(|r| r["kind"] == "call" && r["block"] == block);
        if call.is_some_and(|c| moved.is_match(&c["arguments"])) {
            live = false
        }
        if block == target {
            reached = true;
            if !live {
                return Ok(false);
            }
        }
        let term = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == block);
        if term.is_some_and(|t| t["operation"].starts_with(&format!("drop({place})"))) {
            live = false
        }
        let ret = term
            .and_then(|t| normal.captures(&t["operation"]))
            .map(|c| c[1].to_owned());
        if let Some(edge) = records
            .iter()
            .find(|r| r["kind"] == "edge" && r["block"] == block)
        {
            for successor in blocks.find_iter(&edge["successors"]) {
                let initialized = call.is_some_and(|c| c["destination"] == place)
                    && ret.as_deref() == Some(successor.as_str());
                todo.push_back((successor.as_str().to_owned(), live || initialized));
            }
        }
    }
    Ok(reached)
}
pub(crate) fn propose(
    records: &[Record],
    carriers: &[Carrier],
    cwd: &Path,
    owner: &str,
) -> Result<(FileEdits, Vec<Value>)> {
    let types = records
        .iter()
        .filter(|r| r["kind"] == "local")
        .map(|r| (r["place"].as_str(), r["type"].as_str()))
        .collect::<BTreeMap<_, _>>();
    let region = Regex::new(r"'\?\d+|'_")?;
    let argument = Regex::new(r"^\[Spanned \{ node: move (_\d+), span: [^}]+ \}\]$")?;
    let literal = Regex::new(r#"node: const ("(?:[^"\\]|\\.)*"), span:"#)?;
    let ident = Regex::new(r"^[A-Za-z_][A-Za-z_0-9]*$")?;
    let movement = Regex::new(r"^(_\d+) = move (_\d+)$")?;
    let mut edits = Vec::new();
    let mut reports = Vec::new();
    for call in records
        .iter()
        .filter(|r| r["kind"] == "call" && r["target"] == "core::option::Option::<T>::is_none")
    {
        let Some(arg) = argument.captures(&call["arguments"]) else {
            continue;
        };
        let loans = records
            .iter()
            .filter(|r| r["kind"] == "loan" && r["assigned_place"] == arg[1])
            .collect::<Vec<_>>();
        let [loan] = loans.as_slice() else { continue };
        let Some(option) = types.get(loan["borrowed_place"].as_str()) else {
            continue;
        };
        let Some(inner) = option
            .strip_prefix("core::option::Option<")
            .and_then(|s| s.strip_suffix('>'))
        else {
            continue;
        };
        let shapes = carriers
            .iter()
            .filter(|c| inner == c.debug_path || inner.starts_with(&format!("{}<", c.debug_path)))
            .collect::<Vec<_>>();
        let [carrier] = shapes.as_slice() else {
            continue;
        };
        if !crate::return_forwarding::supports_declaration(records)? {
            reports.push(json!({"status":"unsupported","carrier":carrier.path,"reason":"permission insertion correspondence currently requires fn name()"}));
            continue;
        }
        let caller = if owner.starts_with(&format!("{}::", carrier.crate_name)) {
            owner.to_owned()
        } else {
            format!("{}::{owner}", carrier.crate_name)
        };
        if !carrier
            .scopes
            .iter()
            .all(|scope| caller.starts_with(&format!("{scope}::")))
        {
            reports.push(json!({"status":"unsupported","carrier":carrier.path,"reason":"permission fields or required spec bodies are private","required_scopes":carrier.scopes}));
            continue;
        }
        let creators = records
            .iter()
            .filter(|r| r["kind"] == "call" && r["destination"] == loan["borrowed_place"])
            .collect::<Vec<_>>();
        let [creator] = creators.as_slice() else {
            continue;
        };
        let Some(term) = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == creator["block"])
        else {
            continue;
        };
        let (file, text, a, b) = source(&term["span"], cwd)?;
        let Some(outer) = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == call["block"])
        else {
            continue;
        };
        let (of, _, x, y) = source(&outer["span"], cwd)?;
        let assertions = records
            .iter()
            .filter(|r| {
                r["kind"] == "assert_operand"
                    && r["macro"] == "core::assert"
                    && r["index"] == "0"
                    && r["span"] == outer["span"]
            })
            .collect::<Vec<_>>();
        let [assertion] = assertions.as_slice() else {
            continue;
        };
        let (af, _, start, end) = source(&assertion["assertion_span"], cwd)?;
        if af != file || of != file || !(start <= x && x <= a && a < b && b <= y && y <= end) {
            continue;
        }
        if text[start..end] != format!("assert!({})", &text[x..y]) {
            continue;
        }
        let held = records
            .iter()
            .filter(|r| {
                r["kind"] == "binding"
                    && types.get(r["place"].as_str()).is_some_and(|t| {
                        region.replace_all(t, "R") == region.replace_all(inner, "R")
                    })
            })
            .filter_map(|r| {
                let (f, _, s, e) = source(&r["span"], cwd).ok()?;
                if f != file || e > text.len() {
                    return None;
                }
                let line = text[..s].rfind('\n').map_or(0, |i| i + 1);
                (f == file
                    && e < start
                    && text[line..s].trim() == "let"
                    && ident.is_match(&r["name"])
                    && r["name"] != "_"
                    && records
                        .iter()
                        .filter(|c| c["kind"] == "call" && c["destination"] == r["place"])
                        .count()
                        == 1)
                    .then_some((r, s, e))
            })
            .collect::<Vec<_>>();
        let [(held, hs, he)] = held.as_slice() else {
            continue;
        };
        if !live_at(records, &held["place"], &creator["block"])? {
            reports.push(json!({"status":"unsupported","held_binding":held,"reason":"held resource is not initialized and unconsumed on every incoming path"}));
            continue;
        }
        let panic = records
            .iter()
            .filter(|r| r["kind"] == "call" && r["target"] == "core::panicking::panic")
            .filter_map(|r| {
                let t = records
                    .iter()
                    .find(|t| t["kind"] == "term" && t["block"] == r["block"])?;
                let (f, _, s, e) = source(&t["span"], cwd).ok()?;
                if f != file || s != start || e != end {
                    return None;
                }
                Some(
                    literal
                        .captures(&r["arguments"])?
                        .get(1)?
                        .as_str()
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let [message] = panic.as_slice() else {
            continue;
        };
        let name = &held["name"];
        let binding = &text[*hs..*he];
        if binding == name {
            edits.push((
                file.clone(),
                Edit {
                    start: *hs,
                    end: *he,
                    replacement: format!("mut {name}"),
                    origin: held["span"].clone(),
                },
            ));
        } else if binding != format!("mut {name}") {
            continue;
        }
        let n = reports.len();
        let fresh = format!("__spec_quality_result_{n}");
        let other = format!("__spec_quality_other_{n}");
        if text.contains(&fresh) || text.contains(&other) {
            return Err("generated permission names collide with source".into());
        }
        let reveals = carrier
            .reveals
            .iter()
            .map(|p| format!("reveal({p});"))
            .collect::<Vec<_>>()
            .join("\n");
        let field = &carrier.field;
        let lemma_statement =
            format!("{name}.{field}.borrow_mut().is_exclusive({other}.{field}.borrow());");
        let expression = format!(
            "{{ let mut {fresh} = {}; vstd::prelude::proof! {{ {reveals}\n if let Some(ref mut {other}) = {fresh} {{ vstd::prelude::use_type_invariant(&*{other}); {lemma_statement} assert(false); }} }} {fresh} }}",
            &text[a..b]
        );
        let condition = format!("{}{}{}", &text[x..a], expression, &text[b..y]);
        edits.push((
            file.clone(),
            Edit {
                start,
                end,
                replacement: format!(
                    "if {condition} {{}} else {{ core::panicking::panic({message}); }}"
                ),
                origin: assertion["assertion_span"].clone(),
            },
        ));
        let initialization = records
            .iter()
            .find(|c| c["kind"] == "call" && c["destination"] == held["place"])
            .unwrap();
        let receiver = argument
            .captures(&initialization["arguments"])
            .and_then(|arg| {
                let loans = records
                    .iter()
                    .filter(|r| r["kind"] == "loan" && r["assigned_place"] == arg[1])
                    .collect::<Vec<_>>();
                let [loan] = loans.as_slice() else {
                    return None;
                };
                let producers = records
                    .iter()
                    .filter(|r| r["kind"] == "call" && r["destination"] == loan["borrowed_place"])
                    .collect::<Vec<_>>();
                let [producer] = producers.as_slice() else {
                    return None;
                };
                Some(json!({"kind":"prior_call","reference":loan,"producer":producer}))
            });
        let mut aliases = BTreeSet::from([held["place"].clone()]);
        loop {
            let old = aliases.len();
            for r in records.iter().filter(|r| r["kind"] == "stmt") {
                if let Some(m) = movement.captures(&r["operation"])
                    && aliases.contains(&m[2])
                    && types.get(&m[1]).is_some_and(|t| {
                        region.replace_all(t, "R") == region.replace_all(inner, "R")
                    })
                {
                    aliases.insert(m[1].to_owned());
                }
            }
            if aliases.len() == old {
                break;
            }
        }
        let consumers = records
            .iter()
            .filter(|r| {
                r["kind"] == "call"
                    && argument
                        .captures(&r["arguments"])
                        .is_some_and(|a| aliases.contains(&a[1]))
            })
            .collect::<Vec<_>>();
        reports.push(json!({"assertion":assertion,"result_call":creator,"held_binding":held,"held_initializer":initialization,"receiver_origin":receiver,"possible_consuming_calls":consumers,"carrier":carrier.path,"permission_field":field,"rust_carrier":carrier.rust_path,"reveals":carrier.reveals,"lemma":"vstd::cell::pcell::PointsTo::is_exclusive","lemma_statement":lemma_statement,"primitive_origin":"existing SDK axiom","new_entry_assumptions":0,"manual_token_map":false,"status":"generated_candidate_requires_verification"}));
    }
    Ok((edits, reports))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires PERMISSION_CASE, PERMISSION_CONTEXT, PERMISSION_SDK, PERMISSION_OUT and PERMISSION_MODE"]
    fn replay_generated_proof() {
        use crate::{capture::Invocation, client, runner, write_json};
        let case = PathBuf::from(std::env::var_os("PERMISSION_CASE").unwrap());
        let out = PathBuf::from(std::env::var_os("PERMISSION_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let mode = std::env::var("PERMISSION_MODE").unwrap();
        assert!(matches!(
            mode.as_str(),
            "full" | "independent" | "baseline" | "missing_lemma" | "missing_return_permission"
        ));
        let inv: Invocation = serde_json::from_slice(
            &fs::read(std::env::var_os("PERMISSION_CONTEXT").unwrap()).unwrap(),
        )
        .unwrap();
        let guides: Vec<Value> = serde_json::from_slice::<Vec<Value>>(
            &fs::read(case.join("permission-guidance.json")).unwrap(),
        )
        .unwrap()
        .into_iter()
        .filter(|g| g["status"] == "generated_candidate_requires_verification")
        .collect();
        assert!(!guides.is_empty());
        let package = case.join("package");
        let snapshot = out.join("package");
        runner::copy_package(&package, &snapshot).unwrap();
        let mut suppressed = Vec::new();
        if mode != "full" {
            let bodies =
                client::borrow_records(&fs::read_to_string(case.join("generated.stderr")).unwrap())
                    .unwrap();
            let mut files = BTreeMap::<PathBuf, Vec<Edit>>::new();
            for body in bodies.values() {
                for a in body.iter().filter(|r| r["kind"] == "assert_operand") {
                    assert_eq!(a["macro"], "core::assert");
                    assert_eq!(a["index"], "0");
                    assert_eq!(a["type"], "bool");
                    let (file, text, start, end) = source(&a["assertion_span"], &inv.cwd).unwrap();
                    let (_, _, x, y) = source(&a["span"], &inv.cwd).unwrap();
                    files.entry(file).or_default().push(Edit {
                        start,
                        end,
                        replacement: format!("let _ = ({})", &text[x..y]),
                        origin: a["assertion_span"].clone(),
                    });
                    suppressed.push(a.clone());
                }
            }
            for (file, mut edits) in files {
                edits.sort_by_key(|e| e.start);
                assert!(edits.windows(2).all(|w| w[0].end <= w[1].start));
                let mut text = fs::read_to_string(&file).unwrap();
                for e in edits.iter().rev() {
                    text.replace_range(e.start..e.end, &e.replacement);
                }
                fs::write(snapshot.join(file.strip_prefix(&package).unwrap()), text).unwrap();
            }
        }
        if mode == "baseline" {
            let result = std::process::Command::new("git")
                .args(["apply", "--reverse", "-p2"])
                .arg(std::env::var_os("PERMISSION_REVERSE_PATCH").unwrap())
                .current_dir(&snapshot)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        if mode == "missing_lemma" {
            for g in &guides {
                let statement = g["lemma_statement"].as_str().unwrap();
                let (file, _, _, _) =
                    source(g["assertion"]["assertion_span"].as_str().unwrap(), &inv.cwd).unwrap();
                let file = snapshot.join(
                    file.strip_prefix(Path::new(inv.env.get("CARGO_MANIFEST_DIR").unwrap()))
                        .unwrap(),
                );
                let text = fs::read_to_string(&file).unwrap();
                assert_eq!(text.matches(statement).count(), 1);
                fs::write(file, text.replace(statement, "")).unwrap();
            }
        }
        if mode == "missing_return_permission" {
            assert_eq!(guides.len(), 1);
            let g = &guides[0];
            let (file, _) =
                ir::span(g["assertion"]["assertion_span"].as_str().unwrap(), &inv.cwd).unwrap();
            let file = snapshot.join(
                file.strip_prefix(Path::new(inv.env.get("CARGO_MANIFEST_DIR").unwrap()))
                    .unwrap(),
            );
            let text = fs::read_to_string(&file).unwrap();
            let handoff = format!(
                "proof_with!(self.{});",
                g["permission_field"].as_str().unwrap()
            );
            assert_eq!(text.matches(&handoff).count(), 1);
            fs::write(file, text.replace(&handoff, "")).unwrap();
        }
        let command: Value =
            serde_json::from_slice(&fs::read(case.join("check.command.json")).unwrap()).unwrap();
        let original = command["args"].as_array().unwrap();
        assert_eq!(original[0], "rustc");
        let mut args = Vec::new();
        let mut i = 1;
        while i < original.len() {
            let arg = original[i].as_str().unwrap();
            if matches!(arg, "--log" | "--log-dir") {
                i += 2;
                continue;
            }
            args.push(arg.replace(case.to_str().unwrap(), out.to_str().unwrap()));
            i += 1;
        }
        fs::create_dir(out.join("generated-output")).unwrap();
        if let Ok(module) = std::env::var("PERMISSION_VERIFY_MODULE") {
            let i = args
                .iter()
                .position(|s| s == "--verify-only-module")
                .unwrap();
            args[i + 1] = module;
        }
        args.extend(["--multiple-errors".into(), "20".into()]);
        let proof = runner::prove(
            Path::new(&std::env::var_os("PERMISSION_SDK").unwrap()),
            &inv,
            &args,
            &out,
            "proof",
        )
        .unwrap();
        let expected = std::env::var("PERMISSION_EXPECT_STATUS").unwrap_or_else(|_| {
            if mode == "independent" {
                "proved"
            } else {
                "not_proved"
            }
            .to_owned()
        });
        write_json(&out.join("result.json"),&json!({"mode":mode,"suppressed_source_assertions":suppressed,"permission_guidance":guides,"verification":proof,"native_ktest_executed":false,"scope":"Independent proof control changes assertion behavior and does not replace the full original client check"})).unwrap();
        assert_eq!(proof["status"], expected, "{proof}");
    }
    fn metadata() -> String {
        "(Datatype :name (Dt Path pkg::Carrier) :visibility (Visibility :restricted_to pkg) :variants ((Variant :fields ((-> ticket (tuple (Typ Decorate (TypDecoration Tracked) None (Typ Datatype (Dt Path vstd::cell::pcell::PointsTo) () ())) Exec (Visibility :restricted_to pkg))))))) (Function :name (Fun :path pkg::impl&%0::arbitrary) :mode Spec :visibility (Visibility :restricted_to pkg) :opaqueness (Opaqueness Revealed :visibility (Visibility :restricted_to pkg)) :attrs (Attrs :is_type_invariant_fn true) :params ((Param :name (VarIdent self) :typ (Typ Datatype (Dt Path pkg::Carrier) () ()))))".to_owned()
    }
    #[test]
    fn carrier_uses_typed_field_and_invariant_metadata() {
        let text = metadata();
        let carriers = catalogue(&text, "pkg").unwrap();
        assert_eq!(carriers.len(), 1);
        assert_eq!(carriers[0].field, "ticket");
        assert_eq!(carriers[0].reveals, vec!["crate::Carrier::arbitrary"]);
        assert!(
            catalogue(
                &text.replace("is_type_invariant_fn true", "is_type_invariant_fn false"),
                "pkg"
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            catalogue(&text.replace(PERMISSION, "other::PointsTo"), "pkg")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            catalogue(&text.replace("ticket", "renamed_permission"), "pkg").unwrap()[0].field,
            "renamed_permission"
        );
    }
    #[test]
    fn generated_names_come_from_owned_local_and_compiler_calls() {
        let path =
            std::env::temp_dir().join(format!("permission-guidance-{}.rs", std::process::id()));
        let text = "fn client() {\n    let held = make();\n    assert!(try_make().is_none());\n}\n";
        fs::write(&path, text).unwrap();
        let span = |line: usize, part: &str| {
            let s = text.lines().nth(line - 1).unwrap();
            let a = s.find(part).unwrap() + 1;
            format!(
                "{}:{line}:{a}: {line}:{} (#0)",
                path.display(),
                a + part.len()
            )
        };
        let inner = span(3, "try_make()");
        let outer = span(3, "try_make().is_none()");
        let assertion = span(3, "assert!(try_make().is_none())");
        let mut records=ir::parse(&format!("CLIENT_LOCAL\t_1\tCarrier\nCLIENT_LOCAL\t_2\tcore::option::Option<Carrier>\nCLIENT_LOCAL\t_3\t&core::option::Option<Carrier>\nCLIENT_BINDING\theld\t_1\t{}\tCarrier\nCLIENT_CALL\tbb0\tmake\tunused\t[]\t_1\t[]\nCLIENT_CALL\tbb1\ttry_make\tunused\t[]\t_2\t[]\nCLIENT_TERM\tbb1\t{inner}\tcall\nCLIENT_LOAN\tbb2[0]\tShared\t_2\t_3\t'?1\tNotTwoPhase\nCLIENT_CALL\tbb2\tcore::option::Option::<T>::is_none\tunused\t[Spanned {{ node: move _3, span: unused }}]\t_4\t[]\nCLIENT_TERM\tbb2\t{outer}\tcall\nCLIENT_ASSERT_OPERAND\tcore::assert\t{assertion}\t0\t{outer}\tbool\tunused\nCLIENT_CALL\tbb4\tcore::panicking::panic\tunused\t[Spanned {{ node: const \"assertion failed: try_make().is_none()\", span: unused }}]\t_5\t[]\nCLIENT_TERM\tbb4\t{assertion}\tcall\n",span(2,"held"))).unwrap();
        records.insert(
            0,
            ir::parse("CLIENT_DECL\tunused\tSome(\"fn client()\")\n")
                .unwrap()
                .remove(0),
        );
        records.extend(ir::parse("CLIENT_TERM\tbb0\tunused\t_1 = make() -> [return: bb1]\nCLIENT_EDGE\tbb0\t[bb1]\nCLIENT_EDGE\tbb1\t[bb2]\n").unwrap());
        for r in &mut records {
            if r["kind"] == "term" && r["block"] == "bb1" {
                r.insert(
                    "operation".into(),
                    "_2 = try_make() -> [return: bb2]".into(),
                );
            }
        }
        let carriers = catalogue(&metadata(), "pkg").unwrap();
        let (edits, reports) = propose(&records, &carriers, Path::new("/"), "client").unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0]["held_binding"]["name"], "held");
        assert!(edits.iter().any(|(_, e)| e.replacement == "mut held"));
        assert!(edits.iter().any(|(_, e)| {
            e.replacement
                .contains("held.ticket.borrow_mut().is_exclusive")
        }));
        let mut consumed = records.clone();
        consumed.extend(ir::parse("CLIENT_STMT\tbb1\t0\tunused\t_7 = move _1\n").unwrap());
        let (edits, notes) = propose(&consumed, &carriers, Path::new("/"), "client").unwrap();
        assert!(edits.is_empty());
        assert_eq!(notes[0]["status"], "unsupported");
        let mut parameterized = records.clone();
        parameterized[0].insert("snippet".into(), "Some(\"fn client(x: Carrier)\")".into());
        assert!(
            propose(&parameterized, &carriers, Path::new("/"), "client")
                .unwrap()
                .0
                .is_empty()
        );
        let private = catalogue(
            &metadata()
                .replace("pkg::", "pkg::inner::")
                .replace(":restricted_to pkg)", ":restricted_to pkg::inner)"),
            "pkg",
        )
        .unwrap();
        let mut outside = records.clone();
        for r in &mut outside {
            if let Some(t) = r.get_mut("type") {
                *t = t.replace("Carrier", "inner::Carrier");
            }
        }
        let (edits, notes) =
            propose(&outside, &private, Path::new("/"), "outside::client").unwrap();
        assert!(edits.is_empty());
        assert_eq!(notes[0]["status"], "unsupported");
        records.retain(|r| r["kind"] != "call" || r["target"] != "make");
        assert!(
            propose(&records, &carriers, Path::new("/"), "client")
                .unwrap()
                .1
                .is_empty()
        );
        fs::remove_file(path).unwrap();
    }
}
