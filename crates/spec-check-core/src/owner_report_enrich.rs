//! Enrich a frozen owner-scan report with receiver types from the same VIR capture.
use crate::{Result, hash, vir, write_json};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn datatype_path(node: &vir::Node) -> Option<&str> {
    let parts = node.list()?;
    (parts.first()?.atom() == Some("Dt") && parts.get(1)?.atom() == Some("Path"))
        .then(|| parts.get(2)?.atom())
        .flatten()
}

fn type_path(typ: &vir::Node) -> Option<&str> {
    let parts = typ.unwrap().list()?;
    (parts.first()?.atom() == Some("Typ") && parts.get(1)?.atom() == Some("Datatype"))
        .then(|| parts.get(2).and_then(datatype_path))
        .flatten()
}

fn function_path(function: &vir::Node) -> Option<&str> {
    function
        .field("name")
        .and_then(|name| name.field("path"))
        .and_then(vir::Node::atom)
}

fn receiver_types(function: &vir::Node) -> Vec<Value> {
    function
        .field("params")
        .and_then(vir::Node::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|parameter| {
            let name = parameter
                .field("name")
                .and_then(vir::Node::list)
                .and_then(|parts| parts.get(1))
                .and_then(vir::Node::atom)?;
            let typ = parameter.field("typ")?;
            let typ = typ
                .unwrap()
                .list()
                .and_then(|parts| {
                    (parts.first()?.atom() == Some("Typ")
                        && parts.get(1)?.atom() == Some("Decorate")
                        && matches!(parts.get(2)?.list()?.get(1)?.atom(), Some("Ref" | "MutRef")))
                    .then(|| parts.last())
                    .flatten()
                })
                .unwrap_or(typ);
            Some(json!({
                "name": name,
                "type_path": type_path(typ)
            }))
        })
        .collect()
}

pub fn run(vir_path: &Path, report_path: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("owner report enrichment output directory already exists".into());
    }
    let vir_path = vir_path.canonicalize()?;
    let vir_bytes = fs::read(&vir_path)?;
    let vir_text = std::str::from_utf8(&vir_bytes)?;
    let mut report: Value = serde_json::from_slice(&fs::read(report_path)?)?;
    let expected_sha = report
        .pointer("/input/sha256")
        .and_then(Value::as_str)
        .ok_or("owner report is missing input.sha256")?;
    let actual_sha = hash(&vir_bytes)?;
    if actual_sha != expected_sha {
        return Err("VIR input hash differs from frozen owner report".into());
    }

    let functions = vir::functions(vir_text)?;
    let receiver_by_function = functions
        .iter()
        .filter_map(|function| {
            Some((
                function_path(function)?.to_owned(),
                receiver_types(function),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let entries = report
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or("owner report is missing entries")?;
    let entry_count = entries.len();
    {
        for entry in entries {
            let function = entry
                .get("function")
                .and_then(Value::as_str)
                .ok_or("owner report entry is missing function")?;
            let Some(parameters) = receiver_by_function.get(function) else {
                continue;
            };
            entry["parameter_types"] = json!(parameters);
        }
    }

    report["receiver_type_enrichment"] = json!({
        "vir_sha256": actual_sha,
        "functions_with_receiver_types": receiver_by_function.len(),
        "entry_count": entry_count
    });

    fs::create_dir(out)?;
    write_json(&out.join("report.json"), &report)?;
    write_json(
        &out.join("enrichment.json"),
        &json!({
            "vir_path": vir_path,
            "vir_sha256": actual_sha,
            "scan_report_path": report_path,
            "functions_with_receiver_types": receiver_by_function.len(),
            "entry_count": entry_count,
            "tool": "cargo-spec-check owner-report-enrich"
        }),
    )?;
    Ok(())
}
