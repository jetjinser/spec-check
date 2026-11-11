//! Finite, source-bound case facts for small literal-array loops.

use crate::{
    Result, ir,
    literal::{ArrayLiteral, IndexLoop},
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn source(span: &str, cwd: &Path) -> Result<(PathBuf, usize, usize, String)> {
    let (path, coords) = ir::span(span, cwd)?;
    let path = fs::canonicalize(path)?;
    let text = fs::read_to_string(&path)?;
    let (a, b) = ir::offsets(&text, coords)?;
    Ok((path, a, b, text[a..b].to_owned()))
}

fn inside(a: usize, b: usize, outer_a: usize, outer_b: usize) -> bool {
    outer_a <= a && b <= outer_b
}

pub fn propose(
    records: &[ir::Record],
    arrays: &[ArrayLiteral],
    loop_: &IndexLoop,
    cwd: &Path,
) -> Result<Vec<(ir::Edit, Value)>> {
    let (file, loop_start, loop_end, loop_text) = source(&loop_.span, cwd)?;
    let (range_file, _, _, _) = source(&loop_.range_span, cwd)?;
    if file != range_file || !loop_text.starts_with("for ") {
        return Ok(Vec::new());
    }
    let bindings = records
        .iter()
        .filter(|r| r["kind"] == "binding")
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::<&str, usize>::new();
    for b in &bindings {
        *counts.entry(&b["name"]).or_default() += 1;
    }
    if counts.values().any(|&n| n != 1) {
        return Ok(Vec::new());
    }
    let stmts = records
        .iter()
        .filter(|r| r["kind"] == "stmt")
        .collect::<Vec<_>>();
    let loop_binding = {
        let found = bindings
            .iter()
            .filter(|b| b["name"] == loop_.index && b["type"] == "usize")
            .collect::<Vec<_>>();
        if found.len() != 1 {
            return Ok(Vec::new());
        }
        let (p, a, b, text) = source(&found[0]["span"], cwd)?;
        if p != file || text != loop_.index || !inside(a, b, loop_start, loop_end) {
            return Ok(Vec::new());
        }
        found[0]
    };
    let index_re = Regex::new(&format!(
        r"^(_\d+) = copy {}$",
        regex::escape(&loop_binding["place"])
    ))?;
    let mut index_temps = BTreeSet::new();
    for s in &stmts {
        let Some(c) = index_re.captures(&s["operation"]) else {
            continue;
        };
        let (p, a, b, _) = source(&s["span"], cwd)?;
        if p == file && inside(a, b, loop_start, loop_end) {
            index_temps.insert(c[1].to_owned());
        }
    }
    if index_temps.is_empty() {
        return Ok(Vec::new());
    }
    let array_by_name = arrays
        .iter()
        .filter(|a| a.name == loop_.array_name)
        .collect::<Vec<_>>();
    if array_by_name.len() != 1 {
        return Ok(Vec::new());
    }
    let range_array = array_by_name[0];
    let (array_file, _, array_end, _) = source(&range_array.span, cwd)?;
    let array_bindings = bindings
        .iter()
        .filter(|b| b["place"] == range_array.place && b["name"] == range_array.name)
        .collect::<Vec<_>>();
    if array_file != file || array_bindings.len() != 1 {
        return Ok(Vec::new());
    }
    let (bp, _, bb, binding_text) = source(&array_bindings[0]["span"], cwd)?;
    if bp != file || binding_text != range_array.name || bb >= loop_start || array_end >= loop_start
    {
        return Ok(Vec::new());
    }
    // Any brace between this declaration and loop makes lexical-scope identity uncertain.
    let file_text = fs::read_to_string(&file)?;
    if file_text[bb..loop_start].contains('{') || file_text[bb..loop_start].contains('}') {
        return Ok(Vec::new());
    }

    let stmt_re = Regex::new(r"^(_\d+) = copy (_\d+)\[(_\d+)\]$")?;
    let ident_re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*")?;
    let src_re = Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\[\s*([A-Za-z_][A-Za-z0-9_]*)\s*\]$")?;
    let mut edits = Vec::new();
    let mut seen_assertions = BTreeSet::<String>::new();
    for operand in records.iter().filter(|r| {
        r["kind"] == "assert_operand"
            && r["macro"] == "std::assert"
            && r["type"] == "bool"
            && r["index"] == "0"
    }) {
        let (p, a, b, condition) = source(&operand["span"], cwd)?;
        let (ap, assert_start, assert_end, _) = source(&operand["assertion_span"], cwd)?;
        if p != file
            || ap != file
            || !inside(a, b, loop_start, loop_end)
            || !inside(assert_start, assert_end, loop_start, loop_end)
        {
            continue;
        }
        if !seen_assertions.insert(operand["assertion_span"].clone()) {
            return Ok(Vec::new());
        }
        if condition.contains('"')
            || condition.contains('\'')
            || condition.matches("==").count() != 1
        {
            continue;
        }
        let mut vars = Vec::new();
        for m in ident_re.find_iter(&condition) {
            let name = m.as_str();
            let found = bindings
                .iter()
                .filter(|b| b["name"] == name)
                .collect::<Vec<_>>();
            if found.is_empty() {
                continue;
            }
            if found.len() != 1 || found[0]["type"] != "usize" {
                vars.clear();
                break;
            }
            let binding = found[0];
            let (bp, bs, be, binding_text) = source(&binding["span"], cwd)?;
            if bp != file || binding_text != name || !inside(bs, be, loop_start, loop_end) {
                vars.clear();
                break;
            }
            let candidates = stmts
                .iter()
                .filter_map(|s| {
                    let c = stmt_re.captures(&s["operation"])?;
                    if c[1] != binding["place"] {
                        return None;
                    }
                    let (sp, sa, sb, snippet) = source(&s["span"], cwd).ok()?;
                    if sp != file || !inside(sa, sb, loop_start, loop_end) || sb > assert_start {
                        return None;
                    }
                    let src = src_re.captures(snippet.trim())?;
                    let array = arrays
                        .iter()
                        .filter(|x| x.name == src[1] && x.place == c[2])
                        .collect::<Vec<_>>();
                    if array.len() != 1 || src[2] != loop_.index || !index_temps.contains(&c[3]) {
                        return None;
                    }
                    Some((array[0], s, snippet))
                })
                .collect::<Vec<_>>();
            if candidates.len() != 1 {
                vars.clear();
                break;
            }
            let (array, init, snippet) = &candidates[0];
            if array.elements.len() != range_array.elements.len()
                || array.elements.is_empty()
                || array.elements.len() > 16
            {
                vars.clear();
                break;
            }
            let (arr_file, _, arr_end, _) = source(&array.span, cwd)?;
            let arr_binding = bindings
                .iter()
                .filter(|x| x["place"] == array.place && x["name"] == array.name)
                .collect::<Vec<_>>();
            if arr_file != file || arr_binding.len() != 1 {
                vars.clear();
                break;
            }
            let (arr_bp, _, arr_bb, _) = source(&arr_binding[0]["span"], cwd)?;
            let (_, _, _, arr_binding_text) = source(&arr_binding[0]["span"], cwd)?;
            if arr_bp != file
                || arr_binding_text != array.name
                || arr_bb >= loop_start
                || arr_end >= loop_start
                || file_text[arr_bb..loop_start].contains('{')
                || file_text[arr_bb..loop_start].contains('}')
            {
                vars.clear();
                break;
            }
            vars.push((
                name.to_owned(),
                binding["place"].clone(),
                *array,
                init["span"].clone(),
                snippet.clone(),
            ));
        }
        vars.sort_by(|x, y| x.1.cmp(&y.1));
        vars.dedup_by(|x, y| x.1 == y.1);
        if vars.len() < 2 {
            continue;
        }
        let boundary = file_text[..assert_start]
            .char_indices()
            .rev()
            .find(|(_, c)| matches!(c, ';' | '{' | '}'))
            .map_or(0, |(i, c)| i + c.len_utf8());
        if file_text[boundary..assert_start].contains("#[") {
            continue;
        }
        let n = range_array.elements.len();
        if !(1..=16).contains(&n) || vars.iter().any(|v| v.2.elements.len() != n) {
            continue;
        }
        let mut proof = String::from("vstd::prelude::proof! {\n");
        for i in 0..n {
            let facts = vars
                .iter()
                .map(|v| format!("{} == {}", v.0, v.2.elements[i]))
                .collect::<Vec<_>>()
                .join(" && ");
            proof.push_str(&format!(
                "if {} == {} {{ assert({facts}); }} else ",
                loop_.index, i
            ));
        }
        proof.push_str("{ assert(false); }\n}");
        let provenance = json!({"origin":"finite_literal_cases","assertion_span":operand["assertion_span"],"loop_span":loop_.span,"index":loop_.index,"range_array":{"name":range_array.name,"place":range_array.place,"initializer_span":range_array.span},"bindings":vars.iter().map(|v|json!({"name":v.0,"place":v.1,"array":v.2.name,"array_place":v.2.place,"initializer_span":v.2.span,"binding_span":bindings.iter().find(|b| b["place"] == v.1).map(|b| b["span"].clone()),"load_span":v.3,"load_source":v.4})).collect::<Vec<_>>()});
        edits.push((
            ir::Edit {
                start: assert_start,
                end: assert_start,
                replacement: format!("{proof}\n"),
                origin: operand["assertion_span"].clone(),
            },
            provenance,
        ));
    }
    Ok(edits)
}
