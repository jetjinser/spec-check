//! Evidence dossiers preserve verification scope before attempting attribution.
use crate::{Result, hash, write_json};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn read(path: &Path, inputs: &mut BTreeMap<String, String>) -> Result<Value> {
    let path = fs::canonicalize(path)?;
    let bytes = fs::read(&path)?;
    let digest = hash(&bytes)?;
    if inputs
        .insert(path.to_string_lossy().into_owned(), digest.clone())
        .is_some_and(|old| old != digest)
    {
        return Err("evidence changed between reads".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn classification(status: &str, correspondence: bool) -> &'static str {
    if !correspondence {
        "correspondence_unestablished"
    } else if status == "proved" {
        "proved_under_existing_context"
    } else {
        "undetermined"
    }
}

pub fn ordinary(input: &Path, z3: &Path, out: &Path) -> Result<()> {
    let input = fs::canonicalize(input)?;
    let mut inputs = BTreeMap::new();
    let report = read(&input.join("report.json"), &mut inputs)?;
    let rows = report["results"]
        .as_array()
        .ok_or("missing ordinary caller results")?;
    fs::create_dir(out)?;
    let mut results = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let result = &row["result"];
        if result["status"] != "whole_body_checked" {
            results.push(
                json!({"caller":row["caller"],"status":result.get("status").unwrap_or(&row["status"]),"assessment":"not_checked",
                "original_result":row}),
            );
            continue;
        }
        let case = Path::new(row["evidence"].as_str().ok_or("missing caller evidence")?);
        let edits = read(&case.join("context-edits.json"), &mut inputs)?;
        let files: BTreeMap<std::path::PathBuf, Vec<crate::ir::Edit>> =
            serde_json::from_value(edits)?;
        let first = files.keys().next().ok_or("missing source context")?;
        let package = first
            .ancestors()
            .find(|p| p.join("Cargo.toml").is_file())
            .ok_or("missing original package")?;
        let snapshot = case.join("package");
        for (original, edits) in &files {
            let bytes = fs::read(original)?;
            inputs.insert(original.display().to_string(), hash(&bytes)?);
            let mut source = String::from_utf8(bytes)?;
            for edit in edits.iter().rev() {
                if edit.start > edit.end
                    || !source.is_char_boundary(edit.start)
                    || !source.is_char_boundary(edit.end)
                {
                    return Err("invalid context edit".into());
                }
                source.replace_range(edit.start..edit.end, &edit.replacement);
            }
            let generated = snapshot.join(original.strip_prefix(package)?);
            let bytes = fs::read(&generated)?;
            if bytes != source.as_bytes() {
                return Err("source or generated context no longer matches edits".into());
            }
            inputs.insert(generated.display().to_string(), hash(&bytes)?);
        }
        let command = read(&case.join("check.command.json"), &mut inputs)?;
        let cwd = Path::new(command["cwd"].as_str().ok_or("missing verifier cwd")?);
        let proof = read(&case.join("check.json"), &mut inputs)?;
        if proof != result["verification"] {
            return Err("verification record differs from caller report".into());
        }
        let correspondence = read(&case.join("correspondence.json"), &mut inputs)?;
        let records_path = case.join("original.stderr");
        let records_bytes = fs::read(&records_path)?;
        inputs.insert(records_path.display().to_string(), hash(&records_bytes)?);
        let records = crate::client::borrow_records(std::str::from_utf8(&records_bytes)?)?
            .into_values()
            .flatten()
            .collect::<Vec<_>>();
        let vir = case.join("check-vir/crate.vir");
        let bytes = fs::read(&vir)?;
        inputs.insert(vir.display().to_string(), hash(&bytes)?);
        let functions = crate::vir::functions(std::str::from_utf8(&bytes)?)?;
        let contracts = result["call_targets"]
            .as_array()
            .ok_or("missing complete call targets")?
            .iter()
            .map(|c| {
                Ok(crate::ordinary::call_contract(
                    &functions,
                    c["path"].as_str().ok_or("missing call target")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        drop(functions);
        let copies = if case.join("std-bodies.json").exists() {
            read(&case.join("std-bodies.json"), &mut inputs)?
        } else {
            json!({})
        };
        let copies: BTreeMap<String, Value> = serde_json::from_value(copies)?;
        let uses_copy = contracts.iter().any(|c| {
            c["path"]
                .as_str()
                .is_some_and(|p| p.ends_with(&format!("::{}", crate::std_body::HELPER)))
        });
        if uses_copy == copies.is_empty() {
            return Err("standard-library body copy provenance missing or orphaned".into());
        }
        crate::std_body::validate(&copies, package, &snapshot)?;
        let abstraction = if copies.is_empty() {
            json!({"library_body_copies":{},"interpretation":"Query conclusions apply only to the accepted contract fragment; execution reachability remains separate"})
        } else {
            let checked = read(&case.join("std-body-check.json"), &mut inputs)?;
            if checked["status"] != "proved" || checked["summary"] != json!([1, 0]) {
                return Err("unverified standard-library body copy".into());
            }
            for entries in copies.values().filter_map(Value::as_array) {
                for entry in entries {
                    let source = Path::new(
                        entry["library"]
                            .as_str()
                            .ok_or("missing standard-library source")?,
                    );
                    inputs.insert(source.display().to_string(), hash(&fs::read(source)?)?);
                }
            }
            json!({"relation":"conservative_library_return","library_body_copies":copies,
                "implementation_verification":checked,"original_library_defect":"not_established",
                "interpretation":"Copied bodies have no return guarantees. A countermodel involving their unconstrained result cannot establish a missing guarantee in the original library. Native diagnostics and contract-query evidence retain separate scopes."})
        };
        let failures = crate::review::summarize_context(
            &proof,
            std::str::from_utf8(&bytes)?,
            cwd,
            (package, &snapshot),
            &files,
            &records,
        )?;
        let destination = out.join(format!("caller-{index}"));
        crate::obligation::run(
            &vir,
            result["target"].as_str().ok_or("missing target")?,
            &snapshot,
            z3,
            &destination,
        )?;
        let query = read(&destination.join("report.json"), &mut inputs)?;
        let mut guidance = BTreeMap::new();
        for name in [
            "loop-guidance.json",
            "invariant-facts.json",
            "permission-guidance.json",
        ] {
            let path = case.join(name);
            guidance.insert(
                name,
                if path.is_file() {
                    read(&path, &mut inputs)?
                } else {
                    json!({"status":"not_recorded"})
                },
            );
        }
        results.push(json!({"caller":row["caller"],"status":"whole_body_checked",
            "assessment":classification(proof["status"].as_str().unwrap_or("unknown"),correspondence["matches"]==true),
            "target":result["target"],"targets":{"assertions":result["source_assertions"],"calls":contracts},
            "premises":{"entry":result["entry_conditions"],"added_entry_assumptions":result["new_entry_assumptions"],"guidance":guidance},
            "failures":failures,"verification":proof,"correspondence":correspondence,"abstraction":abstraction,
            "query":{"status":query["status"],"reason":query["reason"],"assessment":query["assessment"],"evidence":destination.join("report.json")},
            "scope":"Native module diagnostics may include dependencies. Scalar entailment evidence applies only to its accepted full caller shape; ordinary proof failures remain undetermined."}));
    }
    for (path, digest) in &inputs {
        if hash(&fs::read(path)?)? != *digest {
            return Err("evidence changed during diagnosis".into());
        }
    }
    write_json(
        &out.join("report.json"),
        &json!({"status":"ordinary_evidence_collected","inputs":inputs,"results":results,
        "limits":"Replays recorded verification context and checks edited source correspondence; does not establish runtime reachability or a general translation theorem. No automatic defect confirmation."}),
    )
}

pub fn run(input: &Path, out: &Path) -> Result<()> {
    let input = fs::canonicalize(input)?;
    let mut inputs = BTreeMap::new();
    let report = read(&input.join("report.json"), &mut inputs)?;
    if report["status"] != "checks_run" {
        return Err("input is not a completed project-check report".into());
    }
    crate::link::validate(Path::new(
        report["links"]
            .as_str()
            .ok_or("missing interface context")?,
    ))?;
    let mut rows = Vec::new();
    for profile in report["profiles"].as_array().ok_or("missing profiles")? {
        for unit in profile["units"].as_array().ok_or("missing units")? {
            let callers_path = Path::new(unit["callers"].as_str().ok_or("missing callers")?);
            let callers = read(callers_path, &mut inputs)?;
            let inventory_path = Path::new(report["capture"].as_str().ok_or("missing capture")?)
                .join(profile["profile"].as_str().ok_or("missing profile")?)
                .join(
                    callers_path
                        .parent()
                        .and_then(Path::file_name)
                        .ok_or("missing unit")?,
                )
                .join("compile/inventory-records.json");
            let inventory = read(&inventory_path, &mut inputs)?;
            let check = &unit["check"];
            let correspondence = if let Some(path) = check["correspondence"].as_str() {
                read(Path::new(path), &mut inputs)?
            } else {
                Value::Null
            };
            let checked = if let Some(path) = check["checked_targets"].as_str() {
                read(Path::new(path), &mut inputs)?
            } else {
                Value::Null
            };
            for caller in callers.as_array().ok_or("invalid caller inventory")? {
                let path = caller["owner"]["path"]
                    .as_str()
                    .ok_or("missing owner path")?;
                let sample = check["results"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|t| t["samples"].as_array().into_iter().flatten())
                    .find(|s| s["function"] == path);
                let native = caller["route"] == "existing_native_client" && checked[path] == true;
                let (scope, status, matched, verification, target, guidance) = if let Some(s) =
                    sample
                {
                    (
                        "generated_test",
                        s["status"].as_str().unwrap_or("unknown"),
                        s["correspondence"]["matches"] == true,
                        s["verification"].clone(),
                        s["target"].clone(),
                        json!({"loop":s["loop_guidance"],"call":s["call_guidance"]}),
                    )
                } else if native {
                    (
                        "existing_native_baseline",
                        check["verification"]["status"]
                            .as_str()
                            .unwrap_or("unknown"),
                        correspondence[path] == true,
                        check["verification"].clone(),
                        json!({"kind":"original_body_obligations","postconditions":caller["native_context"]["function"]["ensures"],"calls":caller["native_context"]["call_sites"]}),
                        json!("Original proof annotations retained"),
                    )
                } else {
                    (
                        "not_checked",
                        "not_checked",
                        false,
                        Value::Null,
                        Value::Null,
                        Value::Null,
                    )
                };
                let trusted = Path::new(
                    caller["trusted_context"]
                        .as_str()
                        .ok_or("missing trusted context")?,
                );
                if !inputs.contains_key(&fs::canonicalize(trusted)?.to_string_lossy().into_owned())
                {
                    read(trusted, &mut inputs)?;
                }
                let contracts = if let Some(path) = sample.and_then(|s| s["contracts"].as_str()) {
                    read(Path::new(path), &mut inputs)?
                } else {
                    Value::Null
                };
                rows.push(json!({"profile":profile["profile"],"caller":caller["owner"],"scope":scope,
                    "classification":if scope=="not_checked"{"not_checked"}else{classification(status,matched)},
                    "route":caller["route"],"definition":caller["definition"],"context_error":caller["context_error"],"verification_status":status,"target":target,"contracts":contracts,
                    "extraction":{"original":check["extractions"]["original"]["callers"][path],"generated":check["extractions"]["generated"]["callers"][path]},
                    "original_assertions":inventory["records"].as_array().into_iter().flatten().filter(|r|r["kind"]=="assert" && r["owner"]==caller["owner"]["owner"]).collect::<Vec<_>>(),
                    "premises":{"original_entry":sample.map(|s|json!({"count":s["target"]["existing_preconditions"],"declaration":s["original_declaration"]})).unwrap_or_else(||caller["native_context"]["function"].clone()),"sources":caller["context_sources"],"added":caller["new_entry_assumptions"]},
                    "proof_guidance":guidance,"verification":verification,
                    "failure_locations":verification["diagnostics"],
                    "conversion":{"correspondence":matched,"basis":if let Some(s)=sample{json!(s["correspondence"])}else{json!({"records":check["correspondence"],"checked_targets":check["checked_targets"]})}},
                    "evidence":{"callers":callers_path,"inventory":inventory_path,"trusted_context":trusted,"project_report":input.join("report.json"),"contracts":sample.map(|s|&s["contracts"])},
                    "attribution":{"status":"not_attempted","reason":"No mechanically bound scalar query or controlled replay attached"}}));
            }
            for prefix in check["prefix_checks"]["checks"]
                .as_array()
                .into_iter()
                .flatten()
            {
                rows.push(json!({"profile":profile["profile"],"caller":prefix["caller"],"scope":"entry_prefix_only",
                    "classification":if prefix["status"]=="prefix_checked"{classification(prefix["verification"]["status"].as_str().unwrap_or("unknown"),prefix["correspondence"]==true)}else{"not_checked"},
                    "target":{"kind":"call_precondition","interface":prefix["interface"],"requires":prefix["target_requires"]},
                    "premises":{"entry_arguments":prefix["entry_arguments"],"added":prefix["added_entry_assumptions"],"direction":prefix["context_direction"]},
                    "failure_locations":prefix["verification"]["diagnostics"],"original_call":prefix["original_call"],
                    "conversion":{"correspondence":prefix["correspondence"],"scope":prefix["scope"]},"evidence":prefix["evidence"],"generated_target":prefix["target"],
                    "attribution":{"status":if prefix["status"]=="prefix_checked"{"undetermined"}else{"not_applicable"},"reason":"Projected context does not establish original caller reachability or informal safety assumptions"}}));
            }
        }
    }
    for (path, digest) in &inputs {
        if hash(&fs::read(path)?)? != *digest {
            return Err("evidence changed during aggregation".into());
        }
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for row in &rows {
        *counts
            .entry(row["classification"].as_str().unwrap().into())
            .or_default() += 1;
    }
    fs::create_dir(out)?;
    write_json(
        &out.join("report.json"),
        &json!({"status":"evidence_collected","inputs":inputs,"counts":counts,"results":rows,
        "limits":"Existing native proofs are baselines. Proof failures remain undetermined. Contract consistency and program reachability are not inferred from Verus success."}),
    )
}

pub fn diagnose(input: &Path, z3: &Path, out: &Path) -> Result<()> {
    run(input, out)?;
    let path = out.join("report.json");
    let mut report: Value = serde_json::from_slice(&fs::read(&path)?)?;
    for (i, row) in report["results"]
        .as_array_mut()
        .ok_or("missing results")?
        .iter_mut()
        .enumerate()
    {
        let (case, function) = if row["scope"] == "generated_test" {
            let contracts = Path::new(
                row["evidence"]["contracts"]
                    .as_str()
                    .ok_or("missing contracts")?,
            );
            (
                contracts
                    .parent()
                    .ok_or("missing test directory")?
                    .to_owned(),
                row["target"]["path"]
                    .as_str()
                    .ok_or("missing target")?
                    .to_owned(),
            )
        } else if row["scope"] == "entry_prefix_only" && row["classification"] != "not_checked" {
            (
                Path::new(row["evidence"].as_str().ok_or("missing prefix evidence")?).to_owned(),
                row["generated_target"]["path"]
                    .as_str()
                    .ok_or("missing prefix target")?
                    .to_owned(),
            )
        } else {
            continue;
        };
        let queries = out.join(format!("query-{i}"));
        crate::obligation::run(
            &case.join("check-vir/crate.vir"),
            &function,
            &case.join("package"),
            z3,
            &queries,
        )?;
        let query: Value = serde_json::from_slice(&fs::read(queries.join("report.json"))?)?;
        row["attribution"] = json!({"status":query["status"],"evidence":queries.join("report.json"),"reason":query["reason"],
            "interpretation":"Queries apply within their recorded fragment; defect attribution and original program reachability remain separate"});
    }
    report["status"] = json!("queries_completed");
    write_json(&path, &report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proof_failure_does_not_establish_a_contract_gap() {
        assert_eq!(classification("not_proved", true), "undetermined");
        assert_eq!(
            classification("proved", false),
            "correspondence_unestablished"
        );
        assert_eq!(
            classification("proved", true),
            "proved_under_existing_context"
        );
    }
}
