use crate::{
    Result, hash,
    ir::{self, Edit, Record},
    vir::{self, Node},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

pub(crate) fn source(span: &str, cwd: &Path) -> Result<(std::path::PathBuf, usize, usize, String)> {
    let (file, coords) = ir::span(span, cwd)?;
    let text = fs::read_to_string(&file)?;
    let (start, end) = ir::offsets(&text, coords)?;
    Ok((fs::canonicalize(file)?, start, end, text[start..end].into()))
}
fn span(node: &Node) -> Option<&str> {
    let values = node.list()?;
    if !matches!(values.first()?.atom(), Some("@" | "@@")) {
        return None;
    }
    values.get(1)?.atom()
}
fn returned_reads<'a>(node: &'a Node, ret: &Node, spans: &mut BTreeSet<&'a str>) {
    if let Some(v) = node.unwrap().list()
        && v.first().and_then(Node::atom) == Some(">")
        && v.get(1).and_then(Node::atom) == Some("ReadPlace")
        && let Some(p) = v.get(2).and_then(|n| n.unwrap().list())
        && p.first().and_then(Node::atom) == Some("Place")
        && p.get(1).and_then(Node::atom) == Some("Local")
        && p.get(2) == Some(ret)
        && let Some(s) = span(node)
    {
        spans.insert(s);
    }
    if let Some(v) = node.list() {
        for n in v {
            returned_reads(n, ret, spans)
        }
    }
}
fn instantiate(function: &Node, replacement: &str, cwd: &Path, pkg: &Path) -> Result<Vec<Value>> {
    let f = function.unwrap();
    let ret = f
        .field("inner_ret")
        .and_then(|n| n.field("name"))
        .ok_or("contract has no return variable")?;
    let name = ret
        .list()
        .and_then(|v| v.get(1))
        .and_then(Node::atom)
        .ok_or("unsupported return variable")?;
    let clauses = f
        .field("ensure")
        .and_then(Node::list)
        .and_then(|v| v.get(1))
        .and_then(Node::list)
        .ok_or("unsupported ensures representation")?;
    let mut result = Vec::new();
    for clause in clauses {
        let clause_span = span(clause).ok_or("contract clause has no source span")?;
        let (file, start, end, mut expression) = source(clause_span, cwd)?;
        if !file.starts_with(pkg) {
            return Err("guidance contract is outside the recorded package".into());
        }
        let mut reads = BTreeSet::new();
        returned_reads(clause, ret, &mut reads);
        let mut edits = Vec::new();
        for read in reads {
            let (p, a, b, text) = source(read, cwd)?;
            if p != file || a < start || b > end || text != name {
                return Err("return-variable source binding mismatch".into());
            }
            edits.push((a - start, b - start));
        }
        if edits.is_empty() {
            continue;
        }
        edits.sort();
        edits.dedup();
        if edits.windows(2).any(|v| v[0].1 > v[1].0) {
            return Err("overlapping return-variable bindings".into());
        }
        for (a, b) in edits.into_iter().rev() {
            expression.replace_range(a..b, replacement)
        }
        result.push(json!({"expression":expression,"contract_span":clause_span,"contract_source_sha256":hash(&fs::read(file)?)?}));
    }
    Ok(result)
}

fn unfold_candidates(functions: &[Node], cwd: &Path, pkg: &Path) -> Result<Vec<Value>> {
    let mut used = BTreeSet::new();
    for function in functions {
        if function.field("mode").and_then(Node::atom) != Some("Exec") {
            continue;
        }
        let Some(s) = span(function) else { continue };
        let Ok((file, _)) = ir::span(s, cwd) else {
            continue;
        };
        if !fs::canonicalize(file)?.starts_with(pkg) {
            continue;
        }
        if let Some(require) = function.field("require") {
            let mut calls = Vec::new();
            vir::walk_calls(require, &mut calls);
            used.extend(calls);
        }
    }
    let identifier = regex::Regex::new(r"^[A-Za-z_]\w*(?:::[A-Za-z_]\w*)+$")?;
    let mut candidates = Vec::new();
    for function in functions {
        let Some(path) = function
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(Node::atom)
        else {
            continue;
        };
        if !used.contains(path)
            || !identifier.is_match(path)
            || function.field("mode").and_then(Node::atom) != Some("Spec")
            || function.field("typ_params").and_then(Node::list) != Some(&[])
            || function
                .field("opaqueness")
                .and_then(Node::list)
                .is_none_or(|v| v.len() != 2 || v[1].atom() != Some("Opaque"))
            || function.field("body").and_then(Node::list).is_none()
        {
            continue;
        }
        let Some(s) = span(function) else { continue };
        let (file, _, _, _) = source(s, cwd)?;
        candidates.push(json!({"path":path,"declaration_span":s,"source_sha256":hash(&fs::read(file)?)?,"fuel":4}));
    }
    Ok(candidates)
}

pub fn propose(
    records: &[Record],
    vir_text: &str,
    cwd: &Path,
    pkg: &Path,
) -> Result<(Vec<Edit>, Value)> {
    let functions = vir::functions(vir_text)?;
    let mut calls = Vec::new();
    for call in records.iter().filter(|r| r["kind"] == "call") {
        let Ok(location) = ir::span(&call["definition_span"], cwd) else {
            continue;
        };
        let matched = functions
            .iter()
            .filter(|f| span(f).and_then(|s| ir::span(s, cwd).ok()).as_ref() == Some(&location))
            .collect::<Vec<_>>();
        if matched.len() != 1 {
            continue;
        }
        let f = matched[0].unwrap();
        if f.field("mode").and_then(Node::atom) != Some("Exec")
            || f.field("params").and_then(Node::list) != Some(&[])
            || f.field("typ_params").and_then(Node::list) != Some(&[])
        {
            continue;
        }
        calls.push((call, matched[0]));
    }
    let argument = regex::Regex::new(r"^\[Spanned \{ node: move (_\d+), span: (.*) \}\]$")?;
    let identifier = regex::Regex::new(r"^[A-Za-z_]\w*$")?;
    let (mut edits, mut candidates) = crate::state::propose(records, &functions, cwd, pkg)?;
    let (set_edits, set_candidates) = crate::set_state::propose(records, &functions, cwd, pkg)?;
    edits.extend(set_edits);
    candidates.extend(set_candidates);
    let mut unsupported = Vec::new();
    let arrays = crate::literal::arrays(records, cwd)?;
    let index_loops = crate::literal::index_loops(records, cwd, &arrays)?;
    let unfolds = unfold_candidates(&functions, cwd, pkg)?;
    for loop_ in records
        .iter()
        .filter(|r| r["kind"] == "source_expr" && r["expression_kind"] == "loop")
    {
        let (file, start, end, text) = source(&loop_["span"], cwd)?;
        if !text.starts_with("for ") {
            continue;
        }
        if candidates.iter().any(|c| {
            (c["kind"] == "boolean_prefix_state" || c["kind"] == "set_membership_state")
                && c["span"] == loop_["span"]
        }) {
            continue;
        }
        // Preserve existing loop annotations; combining independently generated ones is unsupported.
        let full_source = fs::read_to_string(&file)?;
        if full_source[..start].trim_end().ends_with(']') {
            unsupported.push(json!({"span":loop_["span"],"reason":"existing loop attribute"}));
            continue;
        }
        if let Some(index_loop) = index_loops.iter().find(|l| l.span == loop_["span"]) {
            let mut clauses = Vec::new();
            for array in &arrays {
                let (p, _, b, literal) = source(&array.span, cwd)?;
                if p == file
                    && p.starts_with(pkg)
                    && b < start
                    && !full_source[b..start].contains(['{', '}'])
                    && records
                        .iter()
                        .filter(|r| r["kind"] == "binding" && r["name"] == array.name)
                        .count()
                        == 1
                {
                    clauses.push(json!({
                        "expression":format!("{}@ == ({literal})@", array.name),
                        "origin":"immutable_literal_array",
                        "initializer_span":array.span,
                        "place":array.place,
                        "source_sha256":hash(&fs::read(&p)?)?
                    }));
                }
            }
            if !clauses.is_empty() {
                let annotation = format!(
                    "#[vstd::prelude::verus_spec(invariant {},)]\n",
                    clauses
                        .iter()
                        .map(|c| c["expression"].as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                edits.push(Edit {
                    start,
                    end: start,
                    replacement: annotation.clone(),
                    origin: loop_["span"].clone(),
                });
                if !unfolds.is_empty() {
                    let offset = start + text.find('{').ok_or("missing loop body")? + 1;
                    let reveal = unfolds
                        .iter()
                        .map(|c| format!("reveal_with_fuel({}, 4);", c["path"].as_str().unwrap()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    edits.push(Edit {
                        start: offset,
                        end: offset,
                        replacement: format!("\nvstd::prelude::proof! {{ {reveal} }}\n"),
                        origin: loop_["span"].clone(),
                    });
                }
                let mut cases = Vec::new();
                for (edit, provenance) in crate::finite::propose(records, &arrays, index_loop, cwd)?
                {
                    edits.push(edit);
                    cases.push(provenance);
                }
                candidates.push(json!({"span":loop_["span"],"clauses":clauses,"annotation":annotation,"unfolding":unfolds,"finite_cases":cases}));
                continue;
            }
        }
        let mut producers = Vec::new();
        for conversion in records
            .iter()
            .filter(|r| r["kind"] == "call" && r["target"] == "std::iter::IntoIterator::into_iter")
        {
            let Some(args) = argument.captures(&conversion["arguments"]) else {
                continue;
            };
            let Ok((p, a, b, _)) = source(&args[2], cwd) else {
                continue;
            };
            if p != file || a < start || b > end {
                continue;
            }
            producers.extend(
                calls
                    .iter()
                    .copied()
                    .filter(|(call, _)| call["destination"] == args[1]),
            );
        }
        if producers.len() != 1 {
            unsupported.push(json!({"span":loop_["span"],"reason":"unique no-argument iterator producer unavailable"}));
            continue;
        }
        let (producer, function) = producers[0];
        let ghost = "__contract_iter";
        if fs::read_to_string(&file)?.contains(ghost) {
            return Err("generated loop iterator name collides".into());
        }
        let mut clauses = instantiate(function, &format!("{ghost}.snapshot@"), cwd, pkg)?;
        for c in &mut clauses {
            c["origin"] = json!("iterator_initial_snapshot");
            c["target"] = json!(producer["target"]);
        }
        for mut c in instantiate(function, &format!("{ghost}.iter"), cwd, pkg)? {
            if c["expression"] == format!("{ghost}.iter.obeys_prophetic_iter_laws()") {
                c["origin"] = json!("iterator_step_law");
                c["target"] = json!(producer["target"]);
                clauses.push(c);
            }
        }
        for (call, function) in &calls {
            let bindings = records
                .iter()
                .filter(|r| {
                    r["kind"] == "binding"
                        && r["place"] == call["destination"]
                        && identifier.is_match(&r["name"])
                })
                .collect::<Vec<_>>();
            if bindings.len() != 1 {
                continue;
            }
            let (p, _, b, _) = source(&bindings[0]["span"], cwd)?;
            if p != file || b >= start {
                continue;
            }
            for mut c in instantiate(function, &bindings[0]["name"], cwd, pkg)? {
                c["origin"] = json!("earlier_constructor");
                c["target"] = json!(call["target"]);
                clauses.push(c);
            }
        }
        if clauses.is_empty() {
            continue;
        }
        let annotation = format!(
            "#[vstd::prelude::verus_spec({ghost} => invariant {},)]\n",
            clauses
                .iter()
                .map(|c| c["expression"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join(", ")
        );
        edits.push(Edit {
            start,
            end: start,
            replacement: annotation.clone(),
            origin: loop_["span"].clone(),
        });
        candidates.push(json!({"span":loop_["span"],"clauses":clauses,"annotation":annotation}));
    }
    edits.sort_by_key(|edit| edit.start);
    if edits.windows(2).any(|pair| pair[0].start == pair[1].start) {
        return Err("ambiguous loop guidance insertion".into());
    }
    Ok((
        edits,
        json!({"candidates":candidates,"unsupported":unsupported,"scope":"Source-bound candidates from Rustc/VIR contracts, literal tables, boolean-prefix transitions or set-membership updates; Verus checks generated helpers, initialization and preservation. No added client requires."}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires SPEC_LITERAL_TARGET from a recorded align_ext Cargo check"]
    fn real_literal_tables() {
        let root = std::path::PathBuf::from(std::env::var("SPEC_LITERAL_TARGET").unwrap());
        let context: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(root.join("context.json")).unwrap()).unwrap();
        let extracted: Value =
            serde_json::from_slice(&fs::read(root.join("sample-0/original.json")).unwrap())
                .unwrap();
        let records: Vec<Record> = serde_json::from_value(extracted["records"].clone()).unwrap();
        let pkg = Path::new(context.env.get("CARGO_MANIFEST_DIR").unwrap());
        let (edits, report) = propose(
            &records,
            &fs::read_to_string(root.join("baseline-vir/crate.vir")).unwrap(),
            &context.cwd,
            pkg,
        )
        .unwrap();
        assert_eq!(edits.len(), 3);
        let clauses = report["candidates"][0]["clauses"].as_array().unwrap();
        assert_eq!(clauses.len(), 3);
        assert!(
            clauses
                .iter()
                .all(|c| c["origin"] == "immutable_literal_array")
        );
        assert!(edits[0].replacement.contains("input_ns@ == ([0usize, 1, 2"));
        assert_eq!(
            report["candidates"][0]["finite_cases"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    #[ignore = "requires SPEC_GUIDANCE_TARGET from a recorded CPU Cargo check"]
    fn real_contract_bindings() {
        let root = std::path::PathBuf::from(std::env::var("SPEC_GUIDANCE_TARGET").unwrap());
        let context: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(root.join("context.json")).unwrap()).unwrap();
        let extracted: Value =
            serde_json::from_slice(&fs::read(root.join("sample-1/original.json")).unwrap())
                .unwrap();
        let records: Vec<Record> = serde_json::from_value(extracted["records"].clone()).unwrap();
        let (_, report) = propose(
            &records,
            &fs::read_to_string(root.join("baseline-vir/crate.vir")).unwrap(),
            &context.cwd,
            &context.cwd,
        )
        .unwrap();
        println!("{report}");
        assert_eq!(report["candidates"].as_array().unwrap().len(), 1);
    }
}
