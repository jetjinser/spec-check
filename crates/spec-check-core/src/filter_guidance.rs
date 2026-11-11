//! Verified guidance for a resolved standard filter call; original contracts remain intact.
use crate::{Result, hash, ir, std_body::Copy};
use serde_json::json;
use std::{fs, path::Path};
const HELPER: &str = include_str!("../assets/experiments/test-spec/filter-guidance.template");

pub fn propose(records: &[ir::Record], cwd: &Path) -> Result<Option<Copy>> {
    let calls = records
        .iter()
        .filter(|r| {
            r["kind"] == "call"
                && matches!(
                    r["target"].as_str(),
                    "core::iter::Iterator::filter" | "std::iter::Iterator::filter"
                )
        })
        .collect::<Vec<_>>();
    let [call] = calls.as_slice() else {
        return Ok(None);
    };
    if records
        .iter()
        .any(|r| r["kind"] == "container" || r["kind"] == "closure_kind")
        || !records.iter().any(|r| {
            r["kind"] == "instance"
                && r["block"] == call["block"]
                && r["target"] == call["target"]
                && r["definition_span"] == call["definition_span"]
        })
    {
        return Ok(None);
    }
    let term = records
        .iter()
        .find(|r| r["kind"] == "term" && r["block"] == call["block"])
        .ok_or("filter terminator missing")?;
    let body = records
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("filter body missing")?;
    let (file, coordinates) = ir::span(&body["span"], cwd)?;
    let source = fs::read_to_string(&file)?;
    if source.contains("__spec_filter_") {
        return Err("filter guidance name collision".into());
    }
    let (body_start, anchor) = ir::offsets(&source, coordinates)?;
    let offsets = |span: &str| -> Result<(usize, usize)> {
        let (path, coordinates) = ir::span(span, cwd)?;
        if path != file {
            return Err("filter guidance spans source files".into());
        }
        ir::offsets(&source, coordinates)
    };
    let (start, end) = offsets(&term["span"])?;
    if start < body_start || end > anchor {
        return Err("filter call escapes enclosing body".into());
    }
    let mut arguments = records
        .iter()
        .filter(|r| r["kind"] == "source_argument" && r["call_span"] == term["span"])
        .collect::<Vec<_>>();
    arguments.sort_by_key(|r| &r["index"]);
    if arguments.len() != 2 || arguments[0]["index"] != "0" || arguments[1]["index"] != "1" {
        return Ok(None);
    }
    let mut names = Vec::new();
    for argument in arguments {
        let (a, b) = offsets(&argument["span"])?;
        if a < start || b > end {
            return Err("filter argument escapes call".into());
        }
        let name = &source[a..b];
        if argument["type"].starts_with('&')
            || !records
                .iter()
                .any(|r| r["kind"] == "binding" && r["name"] == name)
        {
            return Ok(None);
        }
        names.push(name);
    }
    let results = records
        .iter()
        .filter(|r| r["kind"] == "binding" && r["place"] == call["destination"])
        .collect::<Vec<_>>();
    let [result] = results.as_slice() else {
        return Ok(None);
    };
    if names.iter().any(|name| *name == result["name"]) {
        return Ok(None);
    }
    let (binding_start, binding_end) = offsets(&result["span"])?;
    if binding_start >= start || binding_end > start || source[binding_end..start].trim() != "=" {
        return Ok(None);
    }
    let semi = end + source[end..].len() - source[end..].trim_start().len();
    if source.as_bytes().get(semi) != Some(&b';') {
        return Ok(None);
    }
    let insertion = semi + 1;
    let replacement = format!(
        "\n    vstd::prelude::proof! {{ self::__spec_filter_laws({}, {}, {}); }}",
        names[0], names[1], result["name"]
    );
    let edits = vec![
        ir::Edit {
            start: insertion,
            end: insertion,
            replacement,
            origin: term["span"].clone(),
        },
        ir::Edit {
            start: anchor,
            end: anchor,
            replacement: format!("\n{HELPER}\n"),
            origin: body["span"].clone(),
        },
    ];
    Ok(Some(Copy {
        file,
        edits,
        evidence: json!({"call":call,"span":term["span"],"helper_sha256":hash(HELPER.as_bytes())?,"source_sha256":hash(source.as_bytes())?,"scope":"Original filter call retained; a generic proof lemma must verify from existing contracts. No new library guarantee or entry assumption."}),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires FILTER_GUIDANCE_CASE from actual ordinary-check output"]
    fn actual_filter_guidance_keeps_runtime_and_resolved_identity() {
        let case = std::path::PathBuf::from(std::env::var("FILTER_GUIDANCE_CASE").unwrap());
        let command: serde_json::Value =
            serde_json::from_slice(&fs::read(case.join("original.command.json")).unwrap()).unwrap();
        let cwd = Path::new(command["cwd"].as_str().unwrap());
        let bodies = crate::client::borrow_records(
            &fs::read_to_string(case.join("original.stderr")).unwrap(),
        )
        .unwrap();
        let records = &bodies["control"];
        let guidance = propose(records, cwd).unwrap().unwrap();
        assert!(guidance.edits.iter().all(|edit| edit.start == edit.end));
        let correspondence: serde_json::Value =
            serde_json::from_slice(&fs::read(case.join("correspondence.json")).unwrap()).unwrap();
        assert_eq!(correspondence["matches"], true);
        for (kind, field, replacement) in [
            ("instance", "target", "custom::filter"),
            ("call", "destination", "_missing"),
        ] {
            let mut changed = records.clone();
            for record in changed.iter_mut().filter(|r| {
                r["kind"] == kind
                    && r.get("target")
                        .is_some_and(|t| t.ends_with("Iterator::filter"))
            }) {
                record.insert(field.into(), replacement.into());
            }
            assert!(propose(&changed, cwd).unwrap().is_none());
        }
    }
}
