//! `mutation-triage`: generic review of mutation survivors.
//!
//! This is the project-neutral part of triage: join the manifest and results,
//! classify the mutated line as implementation vs proof/spec, infer a property
//! template, scan local tests/source for intent references, and optionally
//! match a findings registry. It never promotes a survivor to a confirmed
//! defect. Project-specific intent review remains outside the tool.

use regex::Regex;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub function: String,
    pub line: Option<usize>,
    pub operator: String,
    pub region: String,
    pub property_template: String,
    pub equivalence_risk: String,
    pub confirmation_route: String,
    pub registry_matches: Vec<String>,
    pub intent_refs: Vec<String>,
    pub old: String,
    pub new: String,
    pub focus_log: Option<String>,
    pub full_log: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub manifest: String,
    pub results: String,
    pub source: String,
    pub survivors: usize,
    pub routes: BTreeMap<String, usize>,
    pub entries: Vec<Entry>,
}

fn parse_description(description: &str) -> (String, Option<usize>, String) {
    let (function, line) = match description.rsplit_once(" at line ") {
        Some((head, tail)) => (head, tail.trim().parse().ok()),
        None => (description, None),
    };
    let (operator, function) = match function.split_once(": ") {
        Some((op, f)) => (op, f),
        None => ("unknown", function),
    };
    (function.to_string(), line, operator.to_string())
}

fn property_template(function: &str, old: &str, new: &str) -> &'static str {
    let text = format!("{function} {old} {new}").to_lowercase();
    if ["tlb", "flush", "invalidate", "revoke", "unmap"]
        .iter()
        .any(|k| text.contains(k))
    {
        return "revocation";
    }
    if ["ref_count", "unique", "owner", "frame", "slot"]
        .iter()
        .any(|k| text.contains(k))
    {
        return "ownership_lifecycle";
    }
    if ["err", "error", "try_lock", "get_from_unused", "max_paddr"]
        .iter()
        .any(|k| text.contains(k))
    {
        return "success_error";
    }
    if ["range", "iter", "seq", "split", "len", "count"]
        .iter()
        .any(|k| text.contains(k))
    {
        return "capacity_order";
    }
    "state_invariant"
}

fn proof_like_lines(source: &str) -> BTreeSet<usize> {
    const MARKERS: [&str; 8] = [
        "&&&", "|||", "==>", "implies", "forall", "exists", "old(", "final(",
    ];
    let lines: Vec<&str> = source.lines().collect();
    let mut marked = BTreeSet::new();
    let mut depth = 0i32;
    for (index, line) in lines.iter().enumerate() {
        let number = index + 1;
        let text = line.trim();
        let starts = text.starts_with("proof")
            || (text.starts_with("assert") && text.contains(" by"))
            || text.starts_with("broadcast")
            || text.starts_with("reveal");
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;
        if depth > 0 || starts {
            depth = (depth + opens - closes).max(0);
            marked.insert(number);
            continue;
        }
        if MARKERS.iter().any(|marker| text.contains(marker)) {
            marked.insert(number);
        }
    }
    // Contract blocks after `invariant` / `ensures` are not executable bodies.
    for (index, line) in lines.iter().enumerate() {
        let text = line.trim();
        if text == "invariant"
            || text.starts_with("invariant")
            || text == "ensures"
            || text.starts_with("ensures")
        {
            let indent = line.len() - line.trim_start().len();
            let mut j = index + 1;
            while j < lines.len() {
                let next = lines[j];
                let next_indent = next.len() - next.trim_start().len();
                if next.trim() == "{" || (next_indent <= indent && next.contains('{')) {
                    break;
                }
                marked.insert(j + 1);
                j += 1;
            }
        }
    }
    marked
}

fn local_intent(source: &Path, function: &str, limit: usize) -> Vec<String> {
    let mut refs = Vec::new();
    let test = source.parent().map(|p| p.join("test.rs"));
    let candidates = [test, Some(source.to_path_buf())];
    for path in candidates.into_iter().flatten() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if line.contains(function) {
                refs.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
                if refs.len() >= limit {
                    return refs;
                }
            }
        }
    }
    refs
}

fn registry_matches(registry: Option<&Path>, function: &str) -> Result<Vec<String>, String> {
    let Some(path) = registry else {
        return Ok(Vec::new());
    };
    let data: serde_json::Value =
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let re = Regex::new(&format!(r"\b{}\b", regex::escape(function))).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(findings) = data.get("findings").and_then(|v| v.as_array()) {
        for finding in findings {
            let text = format!(
                "{} {}",
                finding
                    .get("subject")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                finding
                    .get("dedup_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            );
            if re.is_match(&text)
                && let Some(id) = finding.get("id").and_then(|v| v.as_str())
            {
                out.push(id.to_string());
            }
        }
    }
    Ok(out)
}

/// Run generic triage. The manifest/results are the same files consumed by
/// `mutation`; source is the pristine file; registry is optional.
pub fn run(
    manifest_path: &Path,
    results_path: &Path,
    source_path: &Path,
    registry_path: Option<&Path>,
    output_path: &Path,
) -> Result<Report, String> {
    let manifest: crate::mutation::Manifest =
        serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let by_id: BTreeMap<String, _> = manifest
        .mutations
        .iter()
        .map(|m| (m.id.clone(), m))
        .collect();
    let source =
        fs::read_to_string(source_path).map_err(|e| format!("{}: {e}", source_path.display()))?;
    let marked = proof_like_lines(&source);
    let mut entries = Vec::new();
    for line in fs::read_to_string(results_path)
        .map_err(|e| e.to_string())?
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        let row: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if row.get("final_status").and_then(|v| v.as_str()) != Some("verified_survivor") {
            continue;
        }
        let id = row.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let Some(mutation) = by_id.get(id) else {
            continue;
        };
        let (function, line_no, operator) = parse_description(mutation.description.as_str());
        let region = if line_no.is_some_and(|n| marked.contains(&n)) {
            "spec_or_proof"
        } else {
            "implementation"
        };
        let matches = registry_matches(registry_path, &function)?;
        let refs = if region == "implementation" {
            local_intent(source_path, &function, 5)
        } else {
            Vec::new()
        };
        let (risk, route) = if region == "spec_or_proof" {
            ("high_invalid_region", "exclude_non_implementation")
        } else if !matches.is_empty() {
            ("unknown", "reuse_registry_evidence")
        } else if !refs.is_empty() {
            ("unknown", "runtime_or_test")
        } else {
            ("unknown", "manual_intent_review")
        };
        let template = property_template(&function, &mutation.old, &mutation.new).to_string();
        entries.push(Entry {
            id: id.to_string(),
            function,
            line: line_no,
            operator,
            region: region.to_string(),
            property_template: template,
            equivalence_risk: risk.to_string(),
            confirmation_route: route.to_string(),
            registry_matches: matches,
            intent_refs: refs,
            old: mutation.old.clone(),
            new: mutation.new.clone(),
            focus_log: row
                .get("focus")
                .and_then(|v| v.get("log"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            full_log: row
                .get("full")
                .and_then(|v| v.get("log"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }
    let mut routes = BTreeMap::new();
    for entry in &entries {
        *routes.entry(entry.confirmation_route.clone()).or_default() += 1;
    }
    let report = Report {
        manifest: manifest_path.display().to_string(),
        results: results_path.display().to_string(),
        source: source_path.display().to_string(),
        survivors: entries.len(),
        routes,
        entries,
    };
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(
        output_path,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generated_description() {
        let (function, line, operator) = parse_description("comparison_flip: split at line 589");
        assert_eq!(function, "split");
        assert_eq!(line, Some(589));
        assert_eq!(operator, "comparison_flip");
    }

    #[test]
    fn classifies_templates() {
        assert_eq!(property_template("unmap", "", ""), "revocation");
        assert_eq!(
            property_template("from_unique", "ref_count", ""),
            "ownership_lifecycle"
        );
        assert_eq!(
            property_template("get_from_unused", "Err", ""),
            "success_error"
        );
        assert_eq!(
            property_template("iter", "0..end", "1..end"),
            "capacity_order"
        );
    }

    #[test]
    fn marks_invariant_lines() {
        let source = "fn f()\n    invariant\n        forall|i: int| 0 <= i,\n    ensures\n        true,\n{\n    let x = 0;\n}\n";
        let marked = proof_like_lines(source);
        assert!(marked.contains(&3));
        assert!(marked.contains(&5));
        assert!(!marked.contains(&7));
    }
}
