//! Checked candidates for monotone boolean-sequence prefix loops.
use crate::{
    Result,
    guidance::source,
    hash,
    ir::{Edit, Record},
    state_semantics,
    vir::Node,
};
use regex::Regex;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn call_span<'a>(records: &'a [Record], call: &Record) -> Option<&'a str> {
    let terms = records
        .iter()
        .filter(|r| r["kind"] == "term" && r["block"] == call["block"])
        .collect::<Vec<_>>();
    (terms.len() == 1).then(|| terms[0]["span"].as_str())
}
fn same_location(a: &str, b: &str, cwd: &Path) -> bool {
    crate::ir::span(a, cwd)
        .ok()
        .zip(crate::ir::span(b, cwd).ok())
        .is_some_and(|(a, b)| a == b)
}
fn declarations(functions: &[Node], paths: &[&str], cwd: &Path) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for function in functions {
        let Some(path) = function
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(Node::atom)
        else {
            continue;
        };
        if !paths.contains(&path) {
            continue;
        }
        let Some(span) = function.list().and_then(|v| v.get(1)).and_then(Node::atom) else {
            continue;
        };
        let (file, _, _, _) = source(span, cwd)?;
        result.push(json!({"path":path,"span":span,"source_sha256":hash(&fs::read(file)?)?}));
    }
    if result.len() != paths.len() {
        return Err("ambiguous state guidance declarations".into());
    }
    Ok(result)
}
fn helpers(module: &str, selector: &str, predicate: &str, witness: &str) -> String {
    format!(
        r#"
use vstd::prelude::{{SpecOrd as _, View as _}};
mod {module} {{
    use vstd::prelude::*;
    verus! {{
        pub proof fn prefix(s: Seq<bool>, k: int)
            requires 0 <= k <= s.len(),
                forall|j: int| 0 <= j < s.len() ==> s[j] == (j < k),
            ensures {selector}(s) == k,
        {{
            {witness}(s);
            assert({predicate}(s, k));
            assert({selector}(s) == k);
        }}
        pub broadcast proof fn uniform_interval(s: Seq<bool>, r: core::ops::Range<usize>)
            requires forall|j: int| 0 <= j < s.len() ==> s[j],
                #[trigger] r.end <= #[trigger] s.len(),
                r.start < r.end,
                forall|j: int| r.start <= j < r.end ==> !s[j],
            ensures false,
        {{
            assert(s[r.start as int]);
            assert(!s[r.start as int]);
        }}
    }}
}}
"#
    )
}

pub fn propose(
    records: &[Record],
    functions: &[Node],
    cwd: &Path,
    pkg: &Path,
) -> Result<(Vec<Edit>, Vec<Value>)> {
    let patterns = state_semantics::discover(functions)?;
    if patterns.is_empty() {
        return Ok((vec![], vec![]));
    }
    let body = records
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("missing client body")?;
    let (file, body_start, body_end, _) = source(&body["span"], cwd)?;
    if !file.starts_with(pkg) {
        return Ok((vec![], vec![]));
    }
    let text = fs::read_to_string(&file)?;
    let header = Regex::new(r"^for\s+(?:_|[A-Za-z_]\w*)\s+in\s+0\s*\.\.\s*([A-Za-z_]\w*)\s*\{")?;
    let method = Regex::new(r"^([A-Za-z_]\w*)\.[A-Za-z_]\w*\(\)$")?;
    let operand = Regex::new(r"^\[Spanned \{ node: [^,]+, span: (.+) \}\]$")?;
    let borrow = Regex::new(r"^\[Spanned \{ node: (?:move|copy) (_\d+), span: .+ \}\]$")?;
    let path = Regex::new(r"^[A-Za-z_]\w*(?:::[A-Za-z_]\w*)+$")?;
    let mut proposals = Vec::new();
    for loop_ in records
        .iter()
        .filter(|r| r["kind"] == "source_expr" && r["expression_kind"] == "loop")
    {
        let (p, start, end, loop_text) = source(&loop_["span"], cwd)?;
        let Some(h) = header.captures(&loop_text) else {
            continue;
        };
        if p != file
            || start < body_start
            || end > body_end
            || text[..start].trim_end().ends_with(']')
        {
            continue;
        }
        let bound = &h[1];
        if records
            .iter()
            .filter(|r| r["kind"] == "binding" && r["name"] == bound && r["type"] == "usize")
            .count()
            != 1
        {
            continue;
        }
        for pattern in &patterns {
            if ![
                &pattern.selector,
                &pattern.predicate,
                &pattern.witness_lemma,
            ]
            .iter()
            .all(|s| path.is_match(s))
            {
                continue;
            }
            let Some(inv) = &pattern.invariant_trait_call else {
                continue;
            };
            if !path.is_match(inv) {
                continue;
            }
            let constructors = records
                .iter()
                .filter(|r| {
                    r["kind"] == "call"
                        && same_location(&r["definition_span"], &pattern.ctor_span, cwd)
                })
                .collect::<Vec<_>>();
            let transitions = records
                .iter()
                .filter(|r| {
                    r["kind"] == "call"
                        && same_location(&r["definition_span"], &pattern.transition_span, cwd)
                })
                .collect::<Vec<_>>();
            for transition in transitions {
                let Some(transition_span) = call_span(records, transition) else {
                    continue;
                };
                let (p, a, b, call_text) = source(transition_span, cwd)?;
                let Some(m) = method.captures(&call_text) else {
                    continue;
                };
                if p != file || a < start || b > end {
                    continue;
                }
                let name = &m[1];
                let bindings = records
                    .iter()
                    .filter(|r| r["kind"] == "binding" && r["name"] == name)
                    .collect::<Vec<_>>();
                if bindings.len() != 1 {
                    continue;
                }
                let state = bindings[0];
                let Some(arg) = borrow.captures(&transition["arguments"]) else {
                    continue;
                };
                let loans = records
                    .iter()
                    .filter(|r| {
                        r["kind"] == "loan"
                            && r["assigned_place"] == arg[1]
                            && r["borrowed_place"] == state["place"]
                            && r["borrow_kind"].starts_with("Mut")
                    })
                    .collect::<Vec<_>>();
                if loans.len() != 1 {
                    continue;
                }
                let ctors = constructors
                    .iter()
                    .filter(|c| c["destination"] == state["place"])
                    .collect::<Vec<_>>();
                if ctors.len() != 1 {
                    continue;
                }
                let ctor = ctors[0];
                let Some(args) = operand.captures(&ctor["arguments"]) else {
                    continue;
                };
                let (p, _, _, actual) = source(&args[1], cwd)?;
                let Some(ctor_span) = call_span(records, ctor) else {
                    continue;
                };
                let (cp, _, cb, _) = source(ctor_span, cwd)?;
                if p != file
                    || cp != file
                    || actual != bound
                    || cb >= start
                    || text[cb..start].contains(['{', '}'])
                {
                    continue;
                }
                let assertions = records
                    .iter()
                    .filter(|r| {
                        r["kind"] == "assert_operand"
                            && r["macro"] == "std::assert"
                            && r["index"] == "0"
                    })
                    .filter_map(|r| {
                        let (p, x, y, _) = source(&r["span"], cwd).ok()?;
                        (p == file && x <= a && b <= y).then_some(r)
                    })
                    .collect::<Vec<_>>();
                if assertions.len() != 1 {
                    continue;
                }
                let (_, assertion_start, _, _) = source(&assertions[0]["assertion_span"], cwd)?;
                let module = format!("__spec_state_{start}");
                let iter = format!("__spec_iter_{start}");
                if text.contains(&module) || text.contains(&iter) {
                    return Err("state guidance identifier collision".into());
                }
                let clauses = vec![
                    format!("{inv}({name})"),
                    format!("{name}@.len() == {bound}"),
                    format!(
                        "forall|j: vstd::prelude::int| 0 <= j < {bound} ==> {name}@[j] == (j < {iter}.index())"
                    ),
                ];
                let edits = vec![
                    Edit {
                        start,
                        end: start,
                        replacement: format!(
                            "#[vstd::prelude::verus_spec({iter} => invariant {},)]\n",
                            clauses.join(", ")
                        ),
                        origin: loop_["span"].clone(),
                    },
                    Edit {
                        start: body_start + 1,
                        end: body_start + 1,
                        replacement: format!(
                            "\nvstd::prelude::proof! {{ broadcast use {module}::uniform_interval; }}\n"
                        ),
                        origin: body["span"].clone(),
                    },
                    Edit {
                        start: assertion_start,
                        end: assertion_start,
                        replacement: format!(
                            "vstd::prelude::proof! {{ {module}::prefix({name}@, {iter}.index()); }}\n"
                        ),
                        origin: assertions[0]["assertion_span"].clone(),
                    },
                    Edit {
                        start: body_end,
                        end: body_end,
                        replacement: helpers(
                            &module,
                            &pattern.selector,
                            &pattern.predicate,
                            &pattern.witness_lemma,
                        ),
                        origin: body["span"].clone(),
                    },
                ];
                let declarations = declarations(
                    functions,
                    &[
                        &pattern.constructor_path,
                        &pattern.transition_path,
                        &pattern.selector,
                        &pattern.predicate,
                        &pattern.witness_lemma,
                    ],
                    cwd,
                )?;
                proposals.push((edits,json!({"kind":"boolean_prefix_state","span":loop_["span"],"constructor":pattern.constructor_path,"constructor_span":pattern.ctor_span,"transition":pattern.transition_path,"transition_span":pattern.transition_span,"selector":pattern.selector,"predicate":pattern.predicate,"witness_lemma":pattern.witness_lemma,"receiver_place":state["place"],"receiver_binding_span":state["span"],"bound":bound,"constructor_bound_parameter":pattern.constructor_length,"clauses":clauses,"declarations":declarations,"source_sha256":hash(text.as_bytes())?,"helper_schema":"uniform boolean prefix and nonempty opposite-valued interval; all obligations checked by Verus"})));
            }
        }
    }
    if proposals.len() != 1 {
        return Ok((vec![], vec![]));
    }
    let (mut edits, report) = proposals.remove(0);
    edits.sort_by_key(|e| e.start);
    Ok((edits, vec![report]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires SPEC_STATE_TARGET from an IdAlloc Cargo check"]
    fn real_state_guidance() {
        let root = std::path::PathBuf::from(std::env::var("SPEC_STATE_TARGET").unwrap());
        let context: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(root.join("context.json")).unwrap()).unwrap();
        let extracted: Value =
            serde_json::from_slice(&fs::read(root.join("sample-0/original.json")).unwrap())
                .unwrap();
        let records: Vec<Record> = serde_json::from_value(extracted["records"].clone()).unwrap();
        let functions = crate::vir::functions(
            &fs::read_to_string(root.join("baseline-vir/crate.vir")).unwrap(),
        )
        .unwrap();
        let pkg = Path::new(context.env.get("CARGO_MANIFEST_DIR").unwrap());
        let (edits, report) = propose(&records, &functions, &context.cwd, pkg).unwrap();
        assert_eq!(edits.len(), 4);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0]["bound"], "capacity");
        assert!(
            edits
                .iter()
                .any(|e| e.replacement.contains("Inv::inv(bitmap)"))
        );
    }
}
