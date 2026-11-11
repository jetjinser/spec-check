use crate::{
    Result,
    ir::{self, Edit, Record},
    vir::{self, Node},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

fn original_location(offset: usize, edits: &[Edit], source: &str, file: &Path) -> Result<Value> {
    let mut delta = 0isize;
    for edit in edits {
        let start = usize::try_from(edit.start as isize + delta)?;
        if offset < start {
            break;
        }
        if offset < start + edit.replacement.len() {
            return Ok(json!({"span":edit.origin,"relation":"generated_from"}));
        }
        delta += edit.replacement.len() as isize - (edit.end - edit.start) as isize;
    }
    let original = usize::try_from(offset as isize - delta)?;
    let prefix = source
        .get(..original)
        .ok_or("diagnostic offset outside original source")?;
    Ok(
        json!({"file":file,"offset":original,"line":prefix.bytes().filter(|c|*c==b'\n').count()+1,"column":prefix.rsplit('\n').next().unwrap_or("").chars().count()+1,"relation":"unchanged_source"}),
    )
}

pub fn summarize(
    proof: &Value,
    vir_text: &str,
    cwd: &Path,
    original: &Path,
    candidate: &Path,
    edits: &[Edit],
    records: &[Record],
) -> Result<Value> {
    let functions = vir::functions(vir_text)?;
    let source = fs::read_to_string(original)?;
    let mut obligations = Vec::new();
    for (index, diagnostic) in proof["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let message = diagnostic["message"].as_str().unwrap_or("");
        if message.starts_with("aborting ") {
            continue;
        }
        let mut related = Vec::new();
        let mut locations = Vec::new();
        let mut boolean_assertion = false;
        let mut spans = Vec::new();
        for root in diagnostic["spans"].as_array().into_iter().flatten() {
            let mut span = root;
            let mut depth = 0;
            while span.is_object() {
                let mut expanded = span.clone();
                expanded["is_primary"] = root["is_primary"].clone();
                expanded["macro_expansion_depth"] = json!(depth);
                spans.push(expanded);
                span = &span["expansion"]["span"];
                depth += 1;
            }
        }
        for span in &spans {
            let Some(name) = span["file_name"].as_str() else {
                continue;
            };
            let file = cwd.join(name);
            if same_file(&file, candidate)
                && let Some(offset) = span["byte_start"].as_u64()
            {
                let location = original_location(offset as usize, edits, &source, original)?;
                if span["is_primary"] == true
                    && let Some(end) = span["byte_end"].as_u64()
                {
                    let end_location = original_location(end as usize, edits, &source, original)?;
                    for assertion in records.iter().filter(|r| {
                        r["kind"] == "assert_operand"
                            && matches!(r["macro"].as_str(), "std::assert" | "core::assert")
                    }) {
                        let (path, coords) = ir::span(&assertion["assertion_span"], cwd)?;
                        let (a, b) = ir::offsets(&source, coords)?;
                        if same_file(&path, original)
                            && location["offset"] == a
                            && end_location["offset"] == b
                        {
                            boolean_assertion = true;
                        }
                    }
                }
                locations.push(
                    json!({"label":span["label"],"primary":span["is_primary"],"original":location,"macro_expansion_depth":span["macro_expansion_depth"]}),
                );
            }
            if span["label"] == "type invariant declared here" {
                for function in &functions {
                    if function
                        .field("attrs")
                        .and_then(|a| a.field("is_type_invariant_fn"))
                        .and_then(Node::atom)
                        != Some("true")
                    {
                        continue;
                    }
                    let Some(origin) = function.list().and_then(|v| v.get(1)).and_then(Node::atom)
                    else {
                        continue;
                    };
                    let Ok((path, [sl, sc, el, ec])) = ir::span(origin, cwd) else {
                        continue;
                    };
                    if same_file(&path, &file)
                        && span["line_start"] == sl
                        && span["column_start"] == sc
                        && span["line_end"] == el
                        && span["column_end"] == ec
                    {
                        related.push(json!({"function":function.field("name").and_then(|n|n.field("path")).and_then(Node::atom),"span":origin,
                            "kind":"type_invariant","condition":function.field("body").map(crate::contracts::render),
                            "condition_source_lines":span["text"],"contract_origin":"captured_verus_vir",
                            "external_body":function.field("attrs").and_then(|a|a.field("is_external_body")).and_then(Node::atom)}));
                    }
                }
            }
            if span["label"] != "failed precondition" {
                continue;
            }
            for function in &functions {
                let Some(clauses) = function.field("require").and_then(Node::list) else {
                    continue;
                };
                for clause in clauses {
                    let Some(origin) = clause.list().and_then(|v| v.get(1)).and_then(Node::atom)
                    else {
                        continue;
                    };
                    let Ok((path, [sl, sc, el, ec])) = ir::span(origin, cwd) else {
                        continue;
                    };
                    if same_file(&path, &file)
                        && span["line_start"] == sl
                        && span["column_start"] == sc
                        && span["line_end"] == el
                        && span["column_end"] == ec
                    {
                        related.push(json!({"function":function.field("name").and_then(|n|n.field("path")).and_then(Node::atom),"span":origin,
                            "required_condition":crate::contracts::render(clause),
                            "condition_source_lines":span["text"],
                            "requires":function.field("require").map(crate::contracts::render),
                            "ensures":function.field("ensure").map(crate::contracts::render),
                            "returns":function.field("returns").map(crate::contracts::render),
                            "contract_origin":"captured_verus_vir",
                            "body_available":function.field("body").is_some_and(|b| b.atom()!=Some("None")),
                            "external_body":function.field("attrs").and_then(|a|a.field("is_external_body")).and_then(Node::atom)}));
                    }
                }
            }
        }
        let assertion_expansion = spans.iter().any(|s| {
            matches!(
                s["expansion"]["macro_decl_name"].as_str(),
                Some("assert!" | "$crate::assert!" | "debug_assert!")
            )
        });
        let equality_assertion = spans.iter().any(|s| {
            matches!(
                s["expansion"]["macro_decl_name"].as_str(),
                Some("assert_eq!" | "$crate::assert_eq!" | "assert_ne!" | "$crate::assert_ne!")
            )
        }) && related
            .iter()
            .any(|r| r["function"] == "core::panicking::assert_failed");
        let kind = if message.contains("loop invariant") {
            "loop_invariant"
        } else if message == "precondition not satisfied" {
            if boolean_assertion
                || equality_assertion
                || (assertion_expansion
                    && related
                        .iter()
                        .any(|r| r["function"] == "core::panicking::panic"))
                || related.iter().any(|r| {
                    r["function"]
                        .as_str()
                        .is_some_and(|s| s.ends_with("::checked_eq_failure"))
                })
            {
                "assertion"
            } else {
                "call_precondition"
            }
        } else if message.contains("assertion failed") {
            "assertion"
        } else if message.contains("arithmetic underflow/overflow") {
            "arithmetic_safety"
        } else if message.contains("declared type invariant") {
            "type_invariant"
        } else if message.contains("postcondition") {
            "postcondition"
        } else if message.contains("not supported") || message.contains("ignored") {
            "unsupported"
        } else {
            "other"
        };
        obligations.push(json!({"diagnostic_index":index,"kind":kind,"message":message,"client_locations":locations,"related_contracts":related,"native_spans":diagnostic["spans"],"assessment":"undetermined"}));
    }
    Ok(
        json!({"obligations":obligations,"scope":"Classification of native diagnostics and local VIR preconditions; unlinked dependencies retain their source spans. Not an automatic attribution of spec defects."}),
    )
}

pub fn summarize_context(
    proof: &Value,
    vir_text: &str,
    cwd: &Path,
    roots: (&Path, &Path),
    files: &BTreeMap<std::path::PathBuf, Vec<Edit>>,
    records: &[Record],
) -> Result<Value> {
    let (package, snapshot) = roots;
    let mut merged = BTreeMap::<usize, Value>::new();
    for (original, edits) in files {
        let candidate = snapshot.join(original.strip_prefix(package)?);
        let summary = summarize(proof, vir_text, cwd, original, &candidate, edits, records)?;
        for row in summary["obligations"]
            .as_array()
            .ok_or("missing obligations")?
        {
            let index = row["diagnostic_index"]
                .as_u64()
                .ok_or("missing diagnostic identity")? as usize;
            if let Some(previous) = merged.get_mut(&index) {
                previous["client_locations"]
                    .as_array_mut()
                    .unwrap()
                    .extend(row["client_locations"].as_array().unwrap().iter().cloned());
                if row["kind"] == "assertion" {
                    previous["kind"] = json!("assertion");
                }
            } else {
                merged.insert(index, row.clone());
            }
        }
    }
    Ok(
        json!({"obligations":merged.into_values().collect::<Vec<_>>(),
        "scope":"Native diagnostics across the selected caller and helper files; generated proof spans retain their edit origin. Failures remain undetermined without separate semantic evidence."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires NATIVE_REVIEW_CASE from an actual BTreeMap source-function check"]
    fn native_index_and_assertion_have_distinct_obligations() {
        let case = std::path::PathBuf::from(std::env::var("NATIVE_REVIEW_CASE").unwrap());
        let command: Value =
            serde_json::from_slice(&fs::read(case.join("check.command.json")).unwrap()).unwrap();
        let candidate = command["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .find(|s| s.ends_with(".rs"))
            .unwrap();
        let candidate = Path::new(candidate);
        let original = case
            .join("runtime-package")
            .join(candidate.strip_prefix(case.join("package")).unwrap());
        let marker = "#[vstd::prelude::verus_verify]\n";
        let offset = fs::read_to_string(candidate).unwrap().find(marker).unwrap();
        let edits = [Edit {
            start: offset,
            end: offset,
            replacement: marker.into(),
            origin: "verification marker".into(),
        }];
        let text = fs::read_to_string(case.join("check-vir/crate.vir")).unwrap();
        let mut proof: Value =
            serde_json::from_slice(&fs::read(case.join("check.json")).unwrap()).unwrap();
        let summarize = |proof: &Value| {
            super::summarize(
                proof,
                &text,
                Path::new(command["cwd"].as_str().unwrap()),
                &original,
                candidate,
                &edits,
                &[],
            )
            .unwrap()
        };
        let result = summarize(&proof);
        let rows = result["obligations"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "call_precondition");
        assert_eq!(
            rows[0]["related_contracts"][0]["function"],
            "core::ops::index::Index::index"
        );
        assert_eq!(rows[1]["kind"], "assertion");
        for row in rows {
            assert!(!row["client_locations"].as_array().unwrap().is_empty());
            assert_eq!(row["assessment"], "undetermined");
        }
        let span = &mut proof["diagnostics"][0]["spans"][0];
        span["line_start"] = json!(span["line_start"].as_u64().unwrap() + 1);
        assert!(
            summarize(&proof)["obligations"][0]["related_contracts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    #[ignore = "requires ORDINARY_REVIEW_CASE and ORDINARY_REVIEW_OUT from an actual ordinary-check run"]
    fn replay_complete_caller_diagnostics() {
        let case = std::path::PathBuf::from(std::env::var("ORDINARY_REVIEW_CASE").unwrap());
        let out = std::path::PathBuf::from(std::env::var("ORDINARY_REVIEW_OUT").unwrap());
        let files: BTreeMap<std::path::PathBuf, Vec<Edit>> =
            serde_json::from_slice(&fs::read(case.join("context-edits.json")).unwrap()).unwrap();
        let command: Value =
            serde_json::from_slice(&fs::read(case.join("check.command.json")).unwrap()).unwrap();
        let proof: Value =
            serde_json::from_slice(&fs::read(case.join("check.json")).unwrap()).unwrap();
        let cwd = Path::new(command["cwd"].as_str().unwrap());
        let package = files
            .keys()
            .next()
            .unwrap()
            .ancestors()
            .find(|p| p.join("Cargo.toml").is_file())
            .unwrap();
        let records = crate::client::borrow_records(
            &fs::read_to_string(case.join("original.stderr")).unwrap(),
        )
        .unwrap()
        .into_values()
        .flatten()
        .collect::<Vec<_>>();
        let text = fs::read_to_string(case.join("check-vir/crate.vir")).unwrap();
        let report = summarize_context(
            &proof,
            &text,
            cwd,
            (package, &case.join("package")),
            &files,
            &records,
        )
        .unwrap();
        let expected = proof["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| !d["message"].as_str().unwrap_or("").starts_with("aborting "))
            .count();
        let obligations = report["obligations"].as_array().unwrap();
        assert_eq!(obligations.len(), expected);
        assert!(
            obligations
                .iter()
                .any(|r| !r["client_locations"].as_array().unwrap().is_empty())
        );
        assert!(
            obligations
                .iter()
                .all(|r| r["assessment"] == "undetermined")
        );
        if proof["diagnostics"].as_array().unwrap().iter().any(|d| {
            d["message"]
                .as_str()
                .unwrap_or("")
                .contains("declared type invariant")
        }) {
            assert!(obligations.iter().any(|r| {
                r["kind"] == "type_invariant"
                    && r["related_contracts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|c| c["kind"] == "type_invariant")
            }));
        }
        crate::write_json(&out, &report).unwrap();
    }
    #[test]
    fn helper_diagnostics_are_merged_once_and_keep_generated_origins() {
        let root = std::env::temp_dir().join(format!("spec-review-context-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let caller = root.join("caller.rs");
        let helper = root.join("helper.rs");
        fs::write(&caller, "call();").unwrap();
        fs::write(&helper, "assert!(true);").unwrap();
        let snapshot = root.join("snapshot");
        let edits = BTreeMap::from([
            (caller.clone(), vec![]),
            (
                helper.clone(),
                vec![Edit {
                    start: 0,
                    end: 0,
                    replacement: "proof {}".into(),
                    origin: "permission origin".into(),
                }],
            ),
        ]);
        let proof = json!({"diagnostics":[{"message":"assertion failed","spans":[
            {"file_name":snapshot.join("caller.rs"),"byte_start":0,"is_primary":false},
            {"file_name":snapshot.join("helper.rs"),"byte_start":2,"is_primary":true}
        ]}]});
        let report = summarize_context(&proof, "", &root, (&root, &snapshot), &edits, &[]).unwrap();
        let obligations = report["obligations"].as_array().unwrap();
        assert_eq!(obligations.len(), 1);
        assert_eq!(obligations[0]["assessment"], "undetermined");
        let locations = obligations[0]["client_locations"].as_array().unwrap();
        assert_eq!(locations.len(), 2);
        assert!(
            locations
                .iter()
                .any(|r| r["original"]["span"] == "permission origin")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn an_operand_precondition_is_distinct_from_the_assertion() {
        let file = std::env::temp_dir().join(format!(
            "spec-review-{}-{}.rs",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&file, "assert!(f())").unwrap();
        for macro_name in ["std::assert", "core::assert"] {
            let records = vec![Record::from([
                ("kind".into(), "assert_operand".into()),
                ("macro".into(), macro_name.into()),
                (
                    "assertion_span".into(),
                    format!("{}:1:1: 1:13 (#0)", file.display()),
                ),
            ])];
            for (start, end, kind) in [(0, 12, "assertion"), (8, 11, "call_precondition")] {
                let proof = json!({"diagnostics":[{"message":"precondition not satisfied","spans":[{"file_name":file,"byte_start":start,"byte_end":end,"is_primary":true}]}]});
                let result =
                    summarize(&proof, "", Path::new("/"), &file, &file, &[], &records).unwrap();
                assert_eq!(result["obligations"][0]["kind"], kind);
            }
        }
        fs::remove_file(file).unwrap();
    }
    #[test]
    fn diagnostic_positions_follow_edits_without_guessing() {
        let edits = vec![
            Edit {
                start: 0,
                end: 0,
                replacement: "ATTR\n".into(),
                origin: "decl".into(),
            },
            Edit {
                start: 4,
                end: 7,
                replacement: "LONGER".into(),
                origin: "assert".into(),
            },
        ];
        let file = Path::new("original.rs");
        let source = "abc\ndef\nxyz";
        assert_eq!(
            original_location(7, &edits, source, file).unwrap()["line"],
            1
        );
        assert_eq!(
            original_location(10, &edits, source, file).unwrap()["span"],
            "assert"
        );
        assert_eq!(
            original_location(16, &edits, source, file).unwrap()["line"],
            3
        );
    }
}
