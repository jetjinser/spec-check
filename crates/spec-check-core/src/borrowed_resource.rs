//! A tuple-produced owner, immutable permission borrow, and explicit restoration.
use crate::{
    Result, hash,
    ir::{self, Record},
    resource,
    vir::{self, Node},
    write_json,
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(crate) fn name(f: &Node) -> Result<&str> {
    f.field("name")
        .and_then(|n| n.field("path"))
        .and_then(Node::atom)
        .ok_or_else(|| "missing native function identity".into())
}
pub(crate) fn params(f: &Node) -> Result<Vec<&Node>> {
    f.field("params")
        .and_then(Node::list)
        .ok_or("missing parameters")?
        .iter()
        .map(|p| {
            p.field("typ")
                .ok_or_else(|| "missing parameter type".into())
        })
        .collect()
}
pub(crate) fn ret(f: &Node) -> Result<&Node> {
    f.field("outer_ret")
        .and_then(|p| p.field("typ"))
        .ok_or_else(|| "missing return type".into())
}
pub(crate) fn one<'a>(rows: &'a [Vec<String>], kind: &str, key: &str) -> Result<&'a [String]> {
    let found = rows
        .iter()
        .filter(|r| r.first().is_some_and(|v| v == kind) && r.get(1).is_some_and(|v| v == key))
        .collect::<Vec<_>>();
    match found.as_slice() {
        [r] => Ok(r),
        _ => Err(format!("missing or ambiguous {kind} for {key}").into()),
    }
}
fn self_argument(call: &Record) -> Result<&str> {
    let text = call["generic_args"]
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("missing generic instantiation")?;
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("invalid generic instantiation")?
            }
            ',' if depth == 0 => return Ok(text[..i].trim()),
            _ => {}
        }
    }
    if depth != 0 || text.is_empty() {
        return Err("invalid Self instantiation".into());
    }
    Ok(text.trim())
}

pub fn run(
    raw: &Path,
    hir: &Path,
    native: &Path,
    cwd: &Path,
    out: &Path,
    after: Option<&Path>,
) -> Result<()> {
    fs::create_dir(out)?;
    let raw_text = fs::read_to_string(raw)?;
    let native_text = fs::read_to_string(native)?;
    let rows: Vec<Vec<String>> = serde_json::from_slice(&fs::read(hir)?)?;
    for row in &rows {
        let size = match row.first().map(String::as_str) {
            Some("SOURCE_OWNER" | "SOURCE_SELF" | "SOURCE_BODY") => 3,
            Some("SOURCE_END") => 2,
            Some("SOURCE_ARGUMENT" | "SOURCE_QUALIFIED") => 4,
            Some(
                "SOURCE_BINDING" | "SOURCE_COMPONENT" | "SOURCE_TARGET" | "SOURCE_CALL"
                | "SOURCE_METHOD",
            ) => 5,
            _ => return Err("unsupported HIR source record".into()),
        };
        if row.len() != size {
            return Err("invalid HIR source record arity".into());
        }
    }
    let owners = rows
        .iter()
        .filter(|r| r.first().is_some_and(|v| v == "SOURCE_OWNER"))
        .collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("one HIR caller required".into());
    };
    let bodies = crate::client::borrow_records(&raw_text)?;
    if bodies.len() != 1 {
        return Err("nested input callers currently unsupported".into());
    }
    let records = bodies
        .get(&owner[1])
        .ok_or("typed HIR caller identity missing")?
        .clone();
    let functions = vir::functions(&native_text)?;
    let (source, report) = plan(&records, &rows, &functions, cwd)?;
    fs::write(out.join("connected.rs"), &source)?;
    let mut standard = Vec::new();
    for (output, control) in [("checked.rs", false), ("resource-control.rs", true)] {
        if let Some(copy) = crate::std_body::pointer_propose(&records, cwd, control)? {
            let mut edits = copy
                .edits
                .iter()
                .map(|e| (e.start, e.end, e.replacement.clone()))
                .collect::<Vec<_>>();
            edits.extend(serde_json::from_value::<Vec<(usize, usize, String)>>(
                report["edits"].clone(),
            )?);
            edits.sort_by_key(|e| e.0);
            if edits.windows(2).any(|w| w[0].1 > w[1].0) {
                return Err("overlapping resource/std edits".into());
            }
            let mut checked = fs::read_to_string(&copy.file)?;
            for (s, e, text) in edits.iter().rev() {
                checked.replace_range(*s..*e, text)
            }
            fs::write(out.join(output), checked)?;
            standard.push(json!({"output":output,"evidence":copy.evidence}));
        }
    }
    let correspondence = if let Some(after) = after {
        let bodies = crate::client::borrow_records(&fs::read_to_string(after)?)?;
        let after = bodies
            .get(&owner[1])
            .ok_or("generated caller identity missing")?;
        let body = after
            .iter()
            .find(|r| r["kind"] == "body")
            .ok_or("missing generated body")?;
        let (file, _) = ir::span(&body["span"], cwd)?;
        if fs::read_to_string(file)? != source {
            return Err("compiled resource source differs from generated plan".into());
        }
        let connections = json!({"connections":[{"kind":"constructor","token":"__spec_resource_owner","binding":"__spec_resource_capture","call":report["factory"]}]});
        let erased = erase_fallbacks(after, after, cwd)?;
        let erased = resource::erase_captures(&erased, &connections)?;
        let before = ir::remap(
            records.iter().filter(|r| ir::runtime(r)).cloned().collect(),
            &BTreeMap::new(),
            true,
        )?;
        let erased = ir::remap(erased, &BTreeMap::new(), true)?;
        write_json(
            &out.join("before-normalized.json"),
            &ir::normalize(&before, None)?,
        )?;
        write_json(
            &out.join("after-normalized.json"),
            &ir::normalize(&erased, None)?,
        )?;
        json!({"matches":ir::compare(&before,&erased,None)?,"scope":"Compiled MIR and borrow events after checked ZST capture/fallback erasure; the caller records its build configuration. Std/assertion transformations and runtime execution are outside this comparison."})
    } else {
        Value::Null
    };
    let mut generated_sources = std::collections::BTreeMap::new();
    for file in ["connected.rs", "checked.rs", "resource-control.rs"] {
        if out.join(file).is_file() {
            generated_sources.insert(file, hash(&fs::read(out.join(file))?)?);
        }
    }
    write_json(
        &out.join("plan.json"),
        &json!({"status":"generated_unchecked", "connections":report,
        "mir_sha256":hash(raw_text.as_bytes())?,"hir_sha256":hash(&fs::read(hir)?)?,
        "vir_sha256":hash(native_text.as_bytes())?,"manual_token_mappings":0,
        "new_entry_assumptions":0,"new_trusted_guarantees":0,"runtime_executed":false,"correspondence":correspondence,"standard_body_copies":standard,"generated_sources":generated_sources}),
    )?;
    if after.is_some() && correspondence["matches"] != true {
        return Err("resource transformation correspondence failed".into());
    }
    Ok(())
}

fn erase_fallbacks(records: &[Record], metadata: &[Record], cwd: &Path) -> Result<Vec<Record>> {
    let mut data = records.to_vec();
    let mut removed = BTreeMap::<String, Vec<usize>>::new();
    let mut locals = BTreeSet::new();
    for call in records.iter().filter(|r| {
        r["kind"] == "call" && r["target"] == "vstd::prelude::Tracked::<A>::assume_new_fallback"
    }) {
        let (file, c) = ir::span(&call["definition_span"], cwd)?;
        let text = fs::read_to_string(file)?;
        let (a, b) = ir::offsets(&text, c)?;
        let end = text[b..]
            .find("\n    }")
            .ok_or("fallback builtin body unavailable")?
            + b
            + 6;
        let compact = text[a..end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if compact != "pubconstfnassume_new_fallback(_:fn()->A)->Self{Tracked{phantom:PhantomData}}"
        {
            return Err("fallback builtin source changed".into());
        }
        let layout = metadata
            .iter()
            .find(|r| r["kind"] == "resource_layout" && r["place"] == call["destination"])
            .ok_or("fallback result layout unavailable")?;
        if layout["definition"] != "vstd::prelude::Tracked"
            || layout["size"] != "0"
            || layout["align"] != "1"
            || layout["needs_drop"] != "false"
        {
            return Err("fallback result is not builtin ZST/no-drop".into());
        }
        let args = resource::arguments(call)?;
        let [arg] = args.as_slice() else {
            return Err("fallback argument arity changed".into());
        };
        let pointer = resource::place(arg).ok_or("fallback needs local function pointer")?;
        let prefix = format!("{pointer} = move ");
        let assignments = records
            .iter()
            .filter(|r| r["kind"] == "stmt" && r["operation"].starts_with(&prefix))
            .collect::<Vec<_>>();
        let [assignment] = assignments.as_slice() else {
            return Err("ambiguous fallback function pointer".into());
        };
        let cast = assignment["operation"].strip_prefix(&prefix).unwrap();
        let (closure, cast) = cast
            .split_once(" as fn() -> ")
            .ok_or("fallback closure coercion required")?;
        if !cast.ends_with(" (PointerCoercion(ClosureFnPointer(Safe), Implicit))") {
            return Err("fallback coercion changed".into());
        }
        let typ = metadata
            .iter()
            .find(|r| r["kind"] == "local" && r["place"] == closure)
            .ok_or("fallback closure type absent")?;
        if !typ["type"].starts_with("Closure(") || !typ["type"].ends_with(", ()])") {
            return Err("fallback closure captures unsupported".into());
        }
        let creation = format!("{closure} = {{closure@");
        let initializers = records
            .iter()
            .filter(|r| {
                r["kind"] == "stmt"
                    && r["operation"].starts_with(&creation)
                    && r["operation"].ends_with('}')
            })
            .count();
        if initializers != 1 {
            return Err("fallback closure initialization changed".into());
        }
        let mentions = Regex::new(&format!(
            r"\b(?:{}|{})\b",
            regex::escape(pointer),
            regex::escape(closure)
        ))?;
        for r in &mut data {
            if !ir::runtime(r) || r["kind"] == "local" {
                continue;
            }
            let refers = r
                .iter()
                .filter(|(k, _)| !["span", "definition_span"].contains(&k.as_str()))
                .any(|(_, v)| mentions.is_match(v));
            if !refers {
                continue;
            }
            if r["kind"] == "call" && r["block"] == call["block"] {
                r.insert(
                    "target".into(),
                    "vstd::prelude::Tracked::<A>::assume_new".into(),
                );
                r.insert("arguments".into(), "[]".into());
            } else if r["kind"] == "term" && r["block"] == call["block"] {
                let op = r["operation"]
                    .replace("::assume_new_fallback(", "::assume_new(")
                    .replace(&format!("move {pointer}"), "");
                r.insert("operation".into(), op);
            } else if r["kind"] == "stmt" {
                let op = &r["operation"];
                if op != &assignment["operation"]
                    && !op.starts_with(&creation)
                    && ![
                        format!("StorageLive({pointer})"),
                        format!("StorageDead({pointer})"),
                        format!("StorageLive({closure})"),
                        format!("StorageDead({closure})"),
                    ]
                    .contains(op)
                {
                    return Err("fallback temporaries have a runtime effect".into());
                }
                removed
                    .entry(r["block"].clone())
                    .or_default()
                    .push(r["statement_index"].parse()?);
            } else {
                return Err("fallback temporaries escape".into());
            }
        }
        locals.insert(pointer.to_owned());
        locals.insert(closure.to_owned());
    }
    data.retain(|r| {
        if r["kind"] == "local" {
            !locals.contains(&r["place"])
        } else if r["kind"] == "stmt" {
            !removed.get(&r["block"]).is_some_and(|xs| {
                r["statement_index"]
                    .parse::<usize>()
                    .is_ok_and(|i| xs.contains(&i))
            })
        } else {
            true
        }
    });
    ir::remap(data, &removed, false)
}

fn plan(
    records: &[Record],
    rows: &[Vec<String>],
    functions: &[Node],
    cwd: &Path,
) -> Result<(String, Value)> {
    let bodies = records
        .iter()
        .filter(|r| r["kind"] == "body")
        .collect::<Vec<_>>();
    let [body] = bodies.as_slice() else {
        return Err("one complete caller body required".into());
    };
    let (file, _) = ir::span(&body["span"], cwd)?;
    let source = fs::read_to_string(&file)?;
    if source.contains("__spec_resource") {
        return Err("resource name collision".into());
    }
    let decl = records
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("missing declaration")?;
    let (decl_file, decl_coords) = ir::span(&decl["span"], cwd)?;
    let (decl_start, decl_end) = ir::offsets(&source, decl_coords)?;
    if decl_file != file
        || !Regex::new(r"^fn\s+\w+\s*\(\s*\)\s*$")?.is_match(&source[decl_start..decl_end])
    {
        return Err(
            "resource bootstrap currently requires a zero-argument ordinary function".into(),
        );
    }
    let owners = rows
        .iter()
        .filter(|r| r.first().is_some_and(|v| v == "SOURCE_OWNER"))
        .collect::<Vec<_>>();
    if !matches!(owners.as_slice(), [r] if r.get(2) == Some(&decl["span"])) {
        return Err("HIR and typed caller identities disagree".into());
    }
    let locate = |span: &str| -> Result<(usize, usize)> {
        let (p, c) = ir::span(span, cwd)?;
        if fs::canonicalize(p)? != fs::canonicalize(&file)? {
            return Err("resource edit outside caller file".into());
        }
        ir::offsets(&source, c)
    };
    let dom = resource::dominates(records)?;
    let mut linked = Vec::new();
    let mut dependencies = Vec::new();
    for call in records.iter().filter(|r| r["kind"] == "call") {
        let term = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == call["block"])
            .ok_or("missing call location")?;
        let Ok(h) = one(rows, "SOURCE_CALL", &term["span"]) else {
            continue;
        };
        let candidates = functions
            .iter()
            .filter(|f| name(f).is_ok_and(|n| n == h[3]))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }
        let [f] = candidates.as_slice() else {
            return Err("ambiguous native resource identity".into());
        };
        let f = *f;
        if !h[3].ends_with(&format!("::{}", call["target"])) && h[3] != call["target"] {
            return Err("HIR and MIR call identities disagree".into());
        }
        let target = one(rows, "SOURCE_TARGET", &term["span"])?;
        let (compiled, c) = ir::span(&call["definition_span"], cwd)?;
        let (resolved, hc) = ir::span(&target[4], cwd)?;
        let annotation = f
            .list()
            .and_then(|v| v.get(1))
            .and_then(Node::atom)
            .ok_or("native definition has no source")?;
        let (native, nc) = ir::span(annotation, cwd)?;
        // Explicit snapshot correspondence: both definition coordinates and complete bytes.
        let bytes = fs::read(&compiled)?;
        if c[..2] != hc[..2]
            || c[..2] != nc[..2]
            || bytes != fs::read(&resolved)?
            || bytes != fs::read(&native)?
        {
            return Err("resource signature snapshots disagree".into());
        }
        if f.field("mode").and_then(Node::atom) != Some("Exec") {
            continue;
        }
        dependencies.push(json!({"identity":h[3],"compiler_definition":call["definition_span"],"native_definition":annotation,"source_sha256":hash(&bytes)?}));
        linked.push((call, f, term["span"].as_str()));
    }
    let mut roots = Vec::new();
    for (call, f, span) in &linked {
        let ps = params(f)?;
        let Some([pointer, permission]) = resource::tuple_fields(ret(f)?) else {
            continue;
        };
        let Some(owner) = resource::decoration(permission, "Tracked") else {
            continue;
        };
        if ps.len() != 1 {
            continue;
        }
        let borrowed = linked
            .iter()
            .filter_map(|(c, g, s)| {
                let gp = params(g).ok()?;
                if gp.len() != 2
                    || gp[0] != pointer
                    || self_argument(c).ok()? != self_argument(call).ok()?
                {
                    return None;
                }
                let read = resource::decoration(gp[1], "Tracked")?;
                if read == owner {
                    return None;
                }
                let conversion = resource::borrow_conversion(owner, read, functions).ok()?;
                Some((*c, *g, *s, conversion))
            })
            .collect::<Vec<_>>();
        if !borrowed.is_empty() {
            roots.push((*call, *f, *span, owner, borrowed));
        }
    }
    let [(factory, factory_fn, factory_span, owner, reads)] = roots.as_slice() else {
        return Err("one unambiguous owned-to-borrowed resource root required".into());
    };
    let fields = resource::tuple_fields(ret(factory_fn)?).unwrap();
    let original_type = params(factory_fn)?[0];
    let consumers = linked
        .iter()
        .filter(|(c, f, _)| {
            params(f).is_ok_and(|p| {
                p.len() == 2
                    && p[0] == &fields[0]
                    && resource::decoration(p[1], "Tracked") == Some(*owner)
            }) && ret(f).is_ok_and(|t| t == original_type)
                && self_argument(c).ok() == self_argument(factory).ok()
        })
        .collect::<Vec<_>>();
    let [(consumer, _, consumer_span)] = consumers.as_slice() else {
        return Err("exactly one explicit owner restoration required".into());
    };
    if !dom
        .get(&consumer["block"])
        .is_some_and(|d| d.contains(&factory["block"]))
    {
        return Err("owner producer does not dominate consumption".into());
    }
    let returns = records
        .iter()
        .filter(|r| r["kind"] == "term" && r["operation"] == "return")
        .collect::<Vec<_>>();
    if returns.is_empty() {
        return Err("normal return evidence required".into());
    }
    for r in returns {
        if !dom
            .get(&r["block"])
            .is_some_and(|d| d.contains(&consumer["block"]))
        {
            return Err("owner restoration omitted on a normal return path".into());
        }
    }
    let block_pattern = Regex::new(r"bb\d+")?;
    for block in std::iter::once(&factory["block"])
        .chain(std::iter::once(&consumer["block"]))
        .chain(reads.iter().map(|(c, _, _, _)| &c["block"]))
    {
        let mut pending = vec![block.clone()];
        let mut seen = BTreeSet::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(edge) = records
                .iter()
                .find(|r| r["kind"] == "edge" && r["block"] == current)
            {
                for next in block_pattern.find_iter(&edge["successors"]) {
                    if next.as_str() == block {
                        return Err("resource call in a cycle unsupported".into());
                    }
                    pending.push(next.as_str().to_owned());
                }
            }
        }
    }
    let projection = Regex::new(r"^(_\d+) = (?:copy|move) \((_\d+)\.0: ")?;
    let projections = records
        .iter()
        .filter(|r| r["kind"] == "stmt")
        .filter_map(|r| {
            let cap = projection.captures(&r["operation"])?;
            (cap[2] == factory["destination"]).then(|| cap[1].to_owned())
        })
        .collect::<Vec<_>>();
    let [pointer] = projections.as_slice() else {
        return Err("unique owner pointer projection required".into());
    };
    let seeds = BTreeMap::from([(pointer.clone(), 0)]);
    let binding = rows
        .iter()
        .filter(|r| {
            r.first().is_some_and(|v| v == "SOURCE_BINDING")
                && r.get(4) == Some(&format!("Some({factory_span})"))
        })
        .collect::<Vec<_>>();
    let [binding] = binding.as_slice() else {
        return Err("factory must initialize a direct tuple binding".into());
    };
    let components = rows
        .iter()
        .filter(|r| {
            r.first().is_some_and(|v| v == "SOURCE_COMPONENT") && r.get(1) == Some(&binding[2])
        })
        .collect::<Vec<_>>();
    if components.len() != 2 {
        return Err("resource tuple binding shape unsupported".into());
    }
    let slot = components
        .iter()
        .find(|r| r[2] == "1" && r[3] == "wildcard")
        .ok_or("discarded owner permission slot required")?;
    let (s, e) = locate(&slot[4])?;
    if &source[s..e] != "_" {
        return Err("owner slot source changed".into());
    }
    let (_, binding_end) = locate(&binding[1])?;
    let mut edits = vec![
        (s, e, "__spec_resource_capture".to_owned()),
        (
            binding_end,
            binding_end,
            "\nproof_decl! { let tracked __spec_resource_owner = __spec_resource_capture.get(); }"
                .into(),
        ),
    ];
    let mut evidence = Vec::new();
    for (call, _, span, conversion) in reads.iter().copied().chain(std::iter::once((
        *consumer,
        *factory_fn,
        *consumer_span,
        *factory_fn,
    ))) {
        let args = resource::arguments(call)?;
        let pointer = resource::place(args.first().ok_or("missing pointer")?)
            .ok_or("pointer must be a local")?;
        resource::origin(records, pointer, &seeds, &mut BTreeSet::new())?;
        if !dom
            .get(&call["block"])
            .is_some_and(|d| d.contains(&factory["block"]))
        {
            return Err("permission unavailable before producer".into());
        }
        let consuming = call["block"] == consumer["block"];
        if !consuming
            && !dom
                .get(&consumer["block"])
                .is_some_and(|d| d.contains(&call["block"]))
        {
            return Err("borrow must precede owner restoration on all paths".into());
        }
        let token = resource::place(args.get(1).ok_or("missing tracked argument")?)
            .ok_or("tracked argument must be local")?;
        let markers = records
            .iter()
            .filter(|r| r["kind"] == "call" && r["destination"] == token)
            .collect::<Vec<_>>();
        if !matches!(markers.as_slice(),[r] if r["target"]=="vstd::prelude::Tracked::<A>::assume_new" && r["arguments"]=="[]")
        {
            return Err("compiler placeholder identity missing".into());
        }
        let arg = rows
            .iter()
            .find(|r| {
                r.first().is_some_and(|v| v == "SOURCE_ARGUMENT")
                    && r.get(1).is_some_and(|v| v == span)
                    && r.get(2).is_some_and(|v| v == "1")
            })
            .ok_or("missing tracked argument source")?;
        let (a, b) = locate(&arg[3])?;
        let (start, end) = locate(span)?;
        if !(start <= a && a < b && b <= end) {
            return Err("tracked argument outside call".into());
        }
        let mut replacement = source[start..end].to_owned();
        let token = if consuming {
            "__spec_resource_owner".to_owned()
        } else {
            format!("__spec_resource_read{}", evidence.len())
        };
        replacement.replace_range(
            a - start..b - start,
            &format!("verus_exec_expr! {{ Tracked({token}) }}"),
        );
        let conversion_name = if consuming {
            None
        } else {
            Some(name(conversion)?)
        };
        if let Some(path) = conversion_name {
            let self_span = one(rows, "SOURCE_SELF", span)?;
            let (a, b) = locate(&self_span[2])?;
            let (trait_path, method) = path.rsplit_once("::").ok_or("conversion trait missing")?;
            let crate_prefix = trait_path
                .split_once("::")
                .ok_or("conversion crate missing")?
                .0;
            let path = format!("crate{}", &trait_path[crate_prefix.len()..]);
            replacement = format!(
                "{{ proof_decl! {{ let tracked {token} = <{} as {path}>::{method}(&__spec_resource_owner); }} {replacement} }}",
                &source[a..b]
            );
        }
        edits.push((start, end, replacement));
        evidence.push(json!({"call":call,"span":span,"kind":if consuming{"restore_owner"}else{"borrow_permission"},"conversion":conversion_name,"permission_source":factory_span}));
    }

    let decl = records
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("missing declaration")?;
    let (start, _) = locate(&decl["span"])?;
    edits.push((start, start, "#[vstd::prelude::verus_verify]\n".into()));
    edits.sort_by_key(|e| e.0);
    if edits.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err("overlapping resource edits".into());
    }
    let mut changed = source.clone();
    for (s, e, text) in edits.iter().rev() {
        changed.replace_range(*s..*e, text);
    }
    if changed[start..].contains("Tracked::assume_new") {
        return Err("unconnected compiler permission placeholder".into());
    }
    Ok((
        changed,
        json!({"source":file,"source_sha256":hash(source.as_bytes())?,"factory":factory,
        "dependencies":dependencies,"steps":evidence,"edits":edits,"scope":"Single tuple-produced owner, immutable proof conversion and explicit restoration on every normal return; unwind cleanup not checked."}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_self_argument_does_not_split_at_inner_comma() {
        let call = Record::from([(
            "generic_args".into(),
            "[Either<Arc<u32>, Box<u16>>, '?3]".into(),
        )]);
        assert_eq!(self_argument(&call).unwrap(), "Either<Arc<u32>, Box<u16>>");
        let malformed =
            Record::from([("generic_args".into(), "[Either<Arc<u32>, Box<u16>]".into())]);
        assert!(self_argument(&malformed).is_err());
    }

    #[test]
    #[ignore = "requires actual compiled source controls in BORROW_RESOURCE_CASES and RESOURCE_BORROW_VIR"]
    fn actual_resource_source_controls() {
        let root = std::path::PathBuf::from(std::env::var("BORROW_RESOURCE_CASES").unwrap());
        let native = fs::read_to_string(std::env::var("RESOURCE_BORROW_VIR").unwrap()).unwrap();
        let functions = vir::functions(&native).unwrap();
        for case in [
            "positive",
            "wrong-owner",
            "duplicate-consumption",
            "omitted-return",
        ] {
            let path = root.join(case);
            let raw = fs::read_to_string(path.join("typed.stderr")).unwrap();
            let rows: Vec<Vec<String>> =
                serde_json::from_slice(&fs::read(path.join("source.json")).unwrap()).unwrap();
            let result = plan(&ir::parse(&raw).unwrap(), &rows, &functions, Path::new("/"));
            assert_eq!(result.is_ok(), case == "positive", "{case}");
            if let Ok((source, report)) = result {
                assert!(!source.contains("Tracked::assume_new"));
                assert_eq!(report["steps"].as_array().unwrap().len(), 2);
                assert_eq!(report["steps"][0]["kind"], "borrow_permission");
                assert_eq!(report["steps"][1]["kind"], "restore_owner");
            }
        }
    }

    #[test]
    #[ignore = "requires actual generated MIR in BORROW_RESOURCE_AFTER"]
    fn fallback_erasure_requires_non_capturing_zst() {
        let path = std::path::PathBuf::from(std::env::var("BORROW_RESOURCE_AFTER").unwrap());
        let command: Value =
            serde_json::from_slice(&fs::read(path.with_file_name("command.json")).unwrap())
                .unwrap();
        let cwd = Path::new(command["cwd"].as_str().unwrap());
        let raw = fs::read_to_string(path).unwrap();
        let bodies = crate::client::borrow_records(&raw).unwrap();
        let (_, records) = bodies
            .iter()
            .find(|(name, _)| !name.contains("::{closure#"))
            .unwrap();
        assert!(erase_fallbacks(records, records, cwd).is_ok());
        let no_layout = records
            .iter()
            .filter(|r| r["kind"] != "resource_layout")
            .cloned()
            .collect::<Vec<_>>();
        assert!(erase_fallbacks(records, &no_layout, cwd).is_err());
        let mut captured = records.clone();
        for r in captured
            .iter_mut()
            .filter(|r| r["kind"] == "local" && r["type"].starts_with("Closure("))
        {
            r.insert("type".into(), r["type"].replace(", ()])", ", (usize,)])"));
        }
        assert!(erase_fallbacks(&captured, &captured, cwd).is_err());
    }
}
