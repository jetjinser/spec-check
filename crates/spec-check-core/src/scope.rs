//! Reconcile requested source files, compiler registrations and actual check outcomes.
use crate::{Result, capture::Invocation, hash, ir, link, project, runner, write_json};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    build: PathBuf,
    sources: Vec<PathBuf>,
    #[serde(default = "wall_seconds")]
    wall_seconds: u64,
    #[serde(default)]
    resources: bool,
    #[serde(default)]
    resource_api_moves: Option<PathBuf>,
    #[serde(default)]
    source_tests: bool,
    #[serde(default)]
    source_functions: bool,
}
// Require agreement between an explicit source placeholder and a whole-body MIR panic.
fn implementation_availability(inventory: &Value, owner: &Value, cwd: &Path) -> Value {
    let records = array(&inventory["records"])
        .filter(|r| r["owner"] == owner["owner"])
        .collect::<Vec<_>>();
    let terms = records
        .iter()
        .filter(|r| r["kind"] == "mir_term")
        .collect::<Vec<_>>();
    let one_block = records
        .iter()
        .any(|r| r["kind"] == "mir_body" && r["blocks"] == "1");
    if one_block
        && terms.len() == 1
        && terms[0]["operation"].as_str().is_some_and(|op| {
            op.contains("core::panicking::") && op.ends_with("-> unwind unreachable")
        })
    {
        for call in records.iter().filter(|r| r["kind"] == "call") {
            let snippet = (|| -> Result<String> {
                let (file, coordinates) = ir::span(text(&call["span"])?, cwd)?;
                let source = fs::read_to_string(file)?;
                let (start, end) = ir::offsets(&source, coordinates)?;
                Ok(source[start..end].trim().to_owned())
            })();
            if let Ok(snippet) = snippet
                && matches!(snippet.as_str(), "unimplemented!()" | "todo!()")
            {
                return json!({"status":"explicit_placeholder","source":call["span"],"expression":snippet,"mir":terms[0],"scope":"Entire compiled body terminates in the explicit source placeholder; no author intent or reachability inferred"});
            }
        }
    }
    json!({"status":"not_identified_as_placeholder"})
}

fn wall_seconds() -> u64 {
    900
}
fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn optional(path: &Path) -> Result<Value> {
    if path.exists() {
        read(path)
    } else {
        Ok(Value::Null)
    }
}
fn array(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}
fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| "missing report identity".into())
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
fn expand(root: &Path, scopes: &[PathBuf]) -> Result<BTreeSet<PathBuf>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
        if path.is_symlink() || (path.exists() && !path.canonicalize()?.starts_with(root)) {
            return Err("source scope must not escape the workspace or follow symlinks".into());
        }
        if path.is_dir() {
            for e in fs::read_dir(path)? {
                let e = e?;
                if e.file_name() != "target" && e.file_name() != ".git" {
                    visit(root, &e.path(), files)?;
                }
            }
        } else if path.extension().is_some_and(|s| s == "rs") {
            files.insert(path.to_owned());
        } else if !path.exists() {
            return Err(format!("missing source scope: {}", path.display()).into());
        }
        Ok(())
    }
    let mut files = BTreeSet::new();
    for path in scopes {
        if !relative(path) {
            return Err("source scopes must be workspace-relative paths".into());
        }
        visit(root, &root.join(path), &mut files)?;
    }
    if files.is_empty() {
        return Err("source scope has no Rust files".into());
    }
    Ok(files)
}
fn stage(command: &mut Command, out: &Path, name: &str, seconds: u64) -> Result<Value> {
    let v = match runner::execute_limited(command, out, name, Duration::from_secs(seconds)) {
        Ok((r, timed)) => {
            json!({"status":if timed{"timeout"}else if r.status.success(){"ok"}else{"failed"},"exit_code":r.status.code()})
        }
        Err(e) => json!({"status":"execution_failed","error":e.to_string()}),
    };
    let mut v = v;
    v["command"] = json!(out.join(format!("{name}.command.json")));
    v["stdout"] = json!(out.join(format!("{name}.stdout")));
    v["stderr"] = json!(out.join(format!("{name}.stderr")));
    write_json(&out.join(format!("{name}.json")), &v)?;
    Ok(v)
}
fn key(profile: &str, unit: &str, owner: &Value) -> Result<String> {
    Ok(format!(
        "{profile}/{unit}/{}",
        link::stable(text(&owner["owner"])?)
    ))
}

fn source_test_slots(capture: &Path, sources: &BTreeSet<PathBuf>) -> Result<Vec<Value>> {
    let captured = read(&capture.join("report.json"))?;
    let mut slots = Vec::new();
    for profile in array(&captured["profiles"]) {
        for unit in array(&profile["units"]).filter(|u| u["status"] == "captured") {
            let location = capture
                .join(text(&profile["profile"])?)
                .join(text(&unit["evidence"])?);
            let base = location.parent().ok_or("missing test unit")?;
            let context = base.join("context.json");
            let inv: Invocation = serde_json::from_slice(&fs::read(&context)?)?;
            let package = Path::new(
                inv.env
                    .get("CARGO_MANIFEST_DIR")
                    .ok_or("missing test package")?,
            )
            .canonicalize()?;
            let identity = read(&base.join("source-identity.json"))?;
            let members = array(&identity["inputs"])
                .filter_map(|f| f["canonical_path"].as_str())
                .map(PathBuf::from)
                .collect::<BTreeSet<_>>();
            for source in sources
                .iter()
                .filter(|p| p.starts_with(&package) && members.contains(*p))
            {
                slots.push(json!({"profile":profile["profile"],"unit":base.file_name().and_then(|s|s.to_str()),"context":context,"source":source}));
            }
        }
    }
    Ok(slots)
}

fn source_test_checks(
    plan: &Config,
    capture: &Path,
    sources: &BTreeSet<PathBuf>,
    sdk: &Path,
    out: &Path,
) -> Result<()> {
    fs::create_dir(out)?;
    let tool = out
        .parent()
        .ok_or("missing test output parent")?
        .join("run-tool");
    let mut results = Vec::new();
    for slot in source_test_slots(capture, sources)? {
        let case = out.join(format!("case-{}", results.len()));
        fs::create_dir(&case)?;
        let census = case.join("census");
        let discovery = stage(
            Command::new(&tool)
                .arg("test-census")
                .arg("--context")
                .arg(text(&slot["context"])?)
                .arg("--source")
                .arg(text(&slot["source"])?)
                .arg("--out")
                .arg(&census)
                .args(plan.source_functions.then_some("--functions")),
            &case,
            "discover",
            plan.wall_seconds,
        )?;
        let parsed = optional(&census.join("report.json"))?;
        let checks = case.join("checks");
        let execution =
            if discovery["status"] == "ok" && array(&parsed["test_candidates"]).next().is_some() {
                stage(
                    Command::new(&tool)
                        .arg("test-isolate")
                        .arg("--census")
                        .arg(&census)
                        .arg("--out")
                        .arg(&checks)
                        .arg("--verus-root")
                        .arg(sdk),
                    &case,
                    "check",
                    plan.wall_seconds,
                )?
            } else {
                json!({"status":"not_requested"})
            };
        results.push(json!({"input":slot,"discovery":discovery,"census":census,"execution":execution,"checks":checks}));
    }
    write_json(
        &out.join("report.json"),
        &json!({"results":results,"scope":"Explicit source function/test candidates before cfg expansion, as configured; separate from the compiled test registry. Activation snapshots and isolated candidates retain their own scope. Kernel tests are not executed."}),
    )
}
fn ordinary_status(r: &Value) -> &'static str {
    if r["status"] == "not_completed" {
        return if r["error_kind"] == "execution_failed" {
            "execution_failed"
        } else {
            "unsupported"
        };
    }
    if r["status"] == "whole_body_checked" {
        let v = &r["verification"];
        if v["exit_code"].is_null() {
            return "execution_failed";
        }
        if v["summary"]
            .as_array()
            .is_none_or(|a| a.len() != 2 || a.iter().filter_map(Value::as_u64).sum::<u64>() == 0)
        {
            return "execution_failed";
        }
        return match v["status"].as_str() {
            Some("proved") if v["exit_code"] == 0 => "proved",
            Some("not_proved") => "not_proved",
            _ => "execution_failed",
        };
    }
    let v = &r["verification"];
    if v["status"] == "timeout" {
        "timeout"
    } else if v["status"] == "unknown" {
        "verification_unknown"
    } else if v["exit_code"].is_null() || v["status"] == "tool_failure" {
        "execution_failed"
    } else if array(&v["diagnostics"]).any(|d| d["level"] == "error") {
        "unsupported"
    } else {
        "execution_failed"
    }
}
fn prefer_ordinary(result: &Value, checked: Option<&Value>) -> bool {
    matches!(ordinary_status(result), "proved" | "not_proved")
        || checked.is_none_or(|r| !matches!(checked_evidence_status(r), "proved" | "not_proved"))
}
fn resource_fills_gap(ordinary: Option<&Value>, native: Option<&Value>) -> bool {
    ordinary.is_none_or(|r| !matches!(ordinary_status(&r["result"]), "proved" | "not_proved"))
        && native.is_none_or(|r| !matches!(checked_evidence_status(r), "proved" | "not_proved"))
}
fn captured_context_matches(capture: &Path, root: &Path, sdk: &Path) -> Result<bool> {
    let metadata = read(&capture.join("metadata.stdout"))?;
    if Path::new(text(&metadata["workspace_root"])?).canonicalize()? != root {
        return Ok(false);
    }
    let report = read(&capture.join("report.json"))?;
    for p in array(&report["profiles"]) {
        for u in array(&p["units"]).filter(|u| u["status"] == "captured") {
            let unit = capture
                .join(text(&p["profile"])?)
                .join(text(&u["evidence"])?);
            if fs::read(
                unit.parent()
                    .ok_or("missing captured unit")?
                    .join("probe.rs"),
            )? != include_bytes!("../assets/experiments/rustc-ir/mir_probe.rs")
            {
                return Ok(false);
            }
        }
    }
    let inputs: BTreeMap<PathBuf, String> =
        serde_json::from_slice(&fs::read(capture.join("inputs.json"))?)?;
    for name in ["verus", "rust_verify", "cargo-verus"] {
        let digest = hash(&fs::read(sdk.join(name))?)?;
        if !inputs
            .iter()
            .any(|(p, h)| p.file_name().is_some_and(|n| n == name) && *h == digest)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn checked_evidence_status(r: &Value) -> &'static str {
    if r["scope"] == "not_checked" {
        return if r["route"] == "non_exec_definition" {
            "not_exec"
        } else {
            "unsupported"
        };
    }
    if ["original", "generated"]
        .iter()
        .any(|stage| r["extraction"][stage]["status"] == "not_analyzed")
    {
        return "extraction_failed";
    }
    if r["conversion"]["correspondence"] != true {
        return "correspondence_failed";
    }
    if r["verification"]["exit_code"].is_null() {
        return "execution_failed";
    }
    if matches!(
        r["verification_status"].as_str(),
        Some("proved" | "not_proved")
    ) && r["verification"]["summary"]
        .as_array()
        .is_none_or(|a| a.len() != 2 || a.iter().filter_map(Value::as_u64).sum::<u64>() == 0)
    {
        return "execution_failed";
    }
    match r["verification_status"].as_str() {
        Some("proved") => "proved",
        Some("not_proved") => "not_proved",
        Some("timeout") => "timeout",
        Some("unsupported") => "unsupported",
        _ => "execution_failed",
    }
}

// Rebuild the denominator from captured compiler records, independently of checker output.
fn assemble(meta: &Value) -> Result<Value> {
    let root = Path::new(text(&meta["workspace"])?);
    let config: Config = serde_json::from_slice(&fs::read(text(&meta["config"])?)?)?;
    if Path::new(text(&meta["config"])?)
        .parent()
        .unwrap()
        .join(&config.build)
        .canonicalize()?
        != Path::new(text(&meta["build_config"])?).canonicalize()?
    {
        return Err("scope build configuration identity changed".into());
    }
    let sources = expand(root, &config.sources)?;
    let capture = Path::new(text(&meta["capture"])?);
    let captured = optional(&capture.join("report.json"))?;
    let build = read(Path::new(text(&meta["build_config"])?))?;
    let checked = Path::new(text(&meta["checks"])?);
    let ordinary = optional(&Path::new(text(&meta["ordinary"])?).join("report.json"))?;
    let dossiers = optional(&Path::new(text(&meta["dossiers"])?).join("report.json"))?;
    let resource_routes = match meta["resources"].as_str() {
        Some(path) => optional(&Path::new(path).join("report.json"))?,
        None => Value::Null,
    };
    let mut source_test_routes = Value::Null;
    if (config.source_tests || config.source_functions)
        && meta["stages"]["source_tests"]["status"] == "ok"
    {
        let directory = Path::new(text(&meta["source_tests"])?);
        let attempts = read(&directory.join("report.json"))?;
        let expected = source_test_slots(capture, &sources)?;
        let rows = array(&attempts["results"]).collect::<Vec<_>>();
        if rows.len() != expected.len() || rows.iter().zip(&expected).any(|(r, e)| r["input"] != *e)
        {
            return Err("source test report omits or changes project inputs".into());
        }
        let mut outcomes = Vec::new();
        let observed_sources = expected
            .iter()
            .map(|e| text(&e["source"]).map(PathBuf::from))
            .collect::<Result<BTreeSet<_>>>()?;
        let unscanned = sources.difference(&observed_sources).collect::<Vec<_>>();
        let mut complete = unscanned.is_empty()
            && array(&captured["profiles"]).all(|p| p["build_success"] == true);
        for row in rows {
            let census = if row["discovery"]["status"] == "ok" {
                let census = crate::test_census::validate(Path::new(text(&row["census"])?))?;
                if (census["selection"] == "functions") != config.source_functions {
                    return Err(
                        "source candidate selection differs from project configuration".into(),
                    );
                }
                if census["source"] != row["input"]["source"]
                    || census["context"] != row["input"]["context"]
                {
                    return Err("source test census belongs to a different project input".into());
                }
                census
            } else {
                optional(&Path::new(text(&row["census"])?).join("report.json"))?
            };
            let checks = optional(&Path::new(text(&row["checks"])?).join("report.json"))?;
            let candidates = array(&census["test_candidates"]).collect::<Vec<_>>();
            let checked = array(&checks["cases"]).collect::<Vec<_>>();
            if checked.len() > candidates.len()
                || candidates
                    .iter()
                    .zip(&checked)
                    .any(|(a, b)| a["path"] != b["test"])
                || (row["execution"]["status"] == "ok" && candidates.len() != checked.len())
            {
                return Err("source test checks omit or reorder AST candidates".into());
            }
            let discovery_complete =
                row["discovery"]["status"] == "ok" && census["parse_complete"] == true;
            complete &= discovery_complete;
            for (index, candidate) in candidates.iter().enumerate() {
                let check = checked.get(index).copied().unwrap_or(&Value::Null);
                let body_complete = row["execution"]["status"] == "ok"
                    && check["result"]["check"]["body_check_complete"] == true;
                let covered = row["execution"]["status"] == "ok"
                    && check["result"]["check"]["complete_property_coverage"] == true;
                complete &= body_complete || covered;
                outcomes.push(json!({"input":row["input"],"test":candidate["path"],"body_check_complete":body_complete,"complete_property_coverage":covered,"result":check,"execution":row["execution"],"evidence":row["checks"]}));
            }
        }
        source_test_routes = json!({"attempts":attempts,"candidates":outcomes,"unscanned_sources":unscanned,"analysis_complete":complete,"counting":"Supplemental AST candidates; overlaps with compiled registrations are not added to the main function count. Unscanned sources have an unknown candidate denominator."});
    }
    let mut resource_rows = BTreeMap::new();
    for group in array(&resource_routes["results"]) {
        for input in array(&group["result"]["inputs"]) {
            let key = (
                text(&group["profile"])?,
                text(&group["unit"])?,
                text(&input["owner"])?,
            );
            if resource_rows.insert(key, input).is_some() {
                return Err("duplicate resource caller in project scope".into());
            }
        }
    }
    let mut checked_rows = BTreeMap::new();
    for r in array(&dossiers["results"]).filter(|r| r["scope"] != "entry_prefix_only") {
        let callers = Path::new(text(&r["evidence"]["callers"])?);
        let unit = callers
            .parent()
            .and_then(Path::file_name)
            .ok_or("missing unit identity")?
            .to_str()
            .ok_or("invalid unit")?;
        if checked_rows
            .insert(key(text(&r["profile"])?, unit, &r["caller"])?, r)
            .is_some()
        {
            return Err("duplicate checked caller".into());
        }
    }
    let mut ordinary_rows = BTreeMap::new();
    for r in array(&ordinary["results"]).filter(|r| r.get("result").is_some()) {
        let case = Path::new(text(&r["evidence"])?);
        let unit = case.parent().ok_or("missing ordinary unit")?;
        let profile = unit
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .ok_or("missing ordinary profile")?;
        if ordinary_rows
            .insert(
                key(
                    profile,
                    unit.file_name().unwrap().to_str().unwrap(),
                    &r["caller"],
                )?,
                r,
            )
            .is_some()
        {
            return Err("duplicate ordinary caller".into());
        }
    }
    let mut files = Vec::new();
    let mut functions = Vec::new();
    let mut contract_review = BTreeMap::<String, Value>::new();
    let mut expected = BTreeSet::new();
    let mut complete_inventory = true;
    let mut total_functions = 0;
    let mut total_calls = 0;
    let mut total_owners = 0;
    let trusted = meta["capture_trusted"] == true;
    if trusted {
        let expected: BTreeSet<_> = array(&build["profiles"])
            .map(|p| p["name"].as_str())
            .collect();
        let observed: BTreeSet<_> = array(&captured["profiles"])
            .map(|p| p["profile"].as_str())
            .collect();
        if expected != observed || observed.len() != array(&captured["profiles"]).count() {
            return Err("capture omitted or duplicated a configured profile".into());
        }
    }
    for profile in array(&build["profiles"]) {
        let name = text(&profile["name"])?;
        let p = array(&captured["profiles"]).find(|p| p["profile"] == name);
        let mut members = BTreeSet::new();
        let mut inventory_complete = true;
        let mut units = 0;
        if trusted && let Some(p) = p {
            for u in array(&p["units"]) {
                if u["status"] != "captured" {
                    inventory_complete = false;
                    continue;
                }
                units += 1;
                let location = capture.join(name).join(text(&u["evidence"])?);
                let unit_dir = location.parent().ok_or("missing unit directory")?;
                let unit = unit_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("missing unit name")?;
                let inv: Invocation =
                    serde_json::from_slice(&fs::read(unit_dir.join("context.json"))?)?;
                let identity = read(&unit_dir.join("source-identity.json"))?;
                for f in array(&identity["inputs"]) {
                    members.insert(Path::new(text(&f["canonical_path"])?).canonicalize()?);
                }
                let inventory = read(&location.join("inventory-records.json"))?;
                if inventory
                    != project::inventory(&fs::read_to_string(location.join("inventory.stderr"))?)?
                {
                    return Err("compiler inventory differs from raw extraction".into());
                }
                total_calls += array(&inventory["records"])
                    .filter(|r| r["kind"] == "mir_call")
                    .count();
                total_owners += array(&inventory["records"])
                    .filter(|r| r["kind"] == "owner")
                    .count();
                let caller_list = optional(&checked.join(name).join(unit).join("callers.json"))?;
                let native_functions =
                    read(&unit_dir.join("verification-input.json")).and_then(|input| {
                        crate::vir::functions(&fs::read_to_string(text(&input["vir"])?)?)
                    });
                for owner in array(&inventory["records"]).filter(|r| {
                    r["kind"] == "owner"
                        && matches!(r["definition_kind"].as_str(), Some("Fn" | "AssocFn"))
                }) {
                    total_functions += 1;
                    let source = ir::span(text(&owner["span"])?, &inv.cwd)
                        .ok()
                        .and_then(|(p, _)| p.canonicalize().ok());
                    if source.as_ref().is_some_and(|s| !sources.contains(s)) {
                        continue;
                    }
                    let id = key(name, unit, owner)?;
                    if !expected.insert(id.clone()) {
                        return Err("duplicate compiler caller".into());
                    }
                    let caller = array(&caller_list).find(|c| c["owner"] == *owner);
                    let contract = caller
                        .and_then(|c| c["compiler_identity"].get(2))
                        .and_then(Value::as_str)
                        .and_then(|path| {
                            native_functions
                                .as_ref()
                                .ok()
                                .map(|functions| crate::ordinary::call_contract(functions, path))
                        });
                    let contract_scan = if let Err(error) = &native_functions {
                        json!({"status":"unavailable","reason":error.to_string()})
                    } else if contract.as_ref().is_some_and(|c| !c["requires"].is_null()) {
                        json!({"status":"scanned"})
                    } else {
                        json!({"status":"unmapped_definition"})
                    };
                    let signals = contract
                        .as_ref()
                        .map(|c| c["constant_postcondition_predicates"].clone())
                        .unwrap_or(json!([]));
                    for signal in array(&signals) {
                        let entry = contract_review
                            .entry(text(&signal["resolved_to"])?.to_owned())
                            .or_insert_with(|| json!({"predicate":signal,"postcondition_uses":[]}));
                        entry["postcondition_uses"].as_array_mut().unwrap().push(json!({"id":id,"owner":owner["path"],"profile":name,"unit":unit,"source":source}));
                    }
                    let ordinary_row = ordinary_rows.get(&id);
                    let evidence = checked_rows.get(&id);
                    let resource_attempt = resource_rows.get(&(name, unit, text(&owner["path"])?));
                    let (status, basis, baseline, targets) = if source.is_none() {
                        (
                            "scope_unresolved",
                            json!({"reason":"Compiler owner location could not be assigned to a source file"}),
                            false,
                            Value::Null,
                        )
                    } else if caller.is_some_and(|c| c["route"] == "non_exec_definition") {
                        (
                            "not_exec",
                            json!({"reason":"VIR classifies this definition as spec or proof"}),
                            false,
                            Value::Null,
                        )
                    } else if let Some(r) = resource_attempt.filter(|r| {
                        r["status"] == "whole_body_checked"
                            && resource_fills_gap(ordinary_row.copied(), evidence.copied())
                    }) {
                        crate::resource_project::validate_complete(r, &inv)?;
                        (
                            ordinary_status(r),
                            json!({"resource":r["evidence"],"proof_scope":"Whole resource caller body with independent helpers and construction control", "diagnostics":r["verification"]["diagnostics"],"ordinary_attempt":ordinary_row}),
                            false,
                            r["explicit_target_count"].clone(),
                        )
                    } else if let Some(r) =
                        ordinary_row.filter(|r| prefer_ordinary(&r["result"], evidence.copied()))
                    {
                        let result = &r["result"];
                        let status = ordinary_status(result);
                        if matches!(status, "proved" | "not_proved")
                            && read(Path::new(text(&result["correspondence"])?))?["matches"] != true
                        {
                            return Err("complete ordinary check lacks correspondence".into());
                        }
                        (
                            status,
                            json!({"ordinary":r["evidence"],"callback_state":r["callback_state"],"callback_proposals":r["callback_proposals"],"filter_guidance":r["filter_guidance"],"result":result["status"],"phase":result["phase"],"proof_scope":"verification module; failure is not attributed to every method","reason":result["reason"],"diagnostics":result["verification"]["diagnostics"]}),
                            false,
                            result["explicit_target_count"].clone(),
                        )
                    } else if ordinary_row.is_none()
                        && caller.is_some_and(|c| c["route"] == "requires_generated_client")
                        && meta["stages"]["ordinary"]["status"] == "ok"
                    {
                        return Err(
                            format!("ordinary report omitted registered caller {id}").into()
                        );
                    } else if caller.is_some_and(|c| c["route"] == "requires_generated_client")
                        && evidence.is_none_or(|r| r["scope"] == "not_checked")
                        && matches!(
                            meta["stages"]["ordinary"]["status"].as_str(),
                            Some("failed" | "execution_failed" | "timeout")
                        )
                    {
                        (
                            if meta["stages"]["ordinary"]["status"] == "timeout" {
                                "timeout"
                            } else {
                                "execution_failed"
                            },
                            json!({"reason":"Ordinary checker stage did not complete","stage":meta["stages"]["ordinary"]}),
                            false,
                            Value::Null,
                        )
                    } else if let Some(r) = evidence {
                        let status = checked_evidence_status(r);
                        (
                            status,
                            json!({"dossier":meta["dossiers"],"scope":r["scope"],"route":r["route"],"extraction":r["extraction"],"proof_scope":"verification module or test artifact; failure is not attributed to every method","diagnostics":r["failure_locations"],"ordinary_attempt":ordinary_row.map(|r|json!({"evidence":r["evidence"],"status":ordinary_status(&r["result"]),"result":r["result"]["status"]}))}),
                            r["scope"] == "existing_native_baseline",
                            Value::Null,
                        )
                    } else {
                        if meta["stages"]["dossiers"]["status"] == "ok" {
                            return Err(format!("report omitted registered caller {id}").into());
                        }
                        (
                            "execution_failed",
                            json!({"reason":"No completed caller result; see pipeline stage failures","stages":meta["stages"]}),
                            false,
                            Value::Null,
                        )
                    };
                    let implementation =
                        if caller.is_some_and(|c| c["route"] == "non_exec_definition") {
                            json!({"status":"not_applicable_spec_or_proof"})
                        } else {
                            implementation_availability(&inventory, owner, &inv.cwd)
                        };
                    let status = if status == "unsupported"
                        && !baseline
                        && implementation["status"] == "explicit_placeholder"
                    {
                        "implementation_missing"
                    } else {
                        status
                    };
                    functions.push(json!({"implementation":implementation,"id":id,"profile":name,"unit":unit,"owner":owner,"source":source,"status":status,"baseline":baseline,"definition":caller.map(|c|&c["definition"]),"route":caller.map(|c|&c["route"]),"explicit_target_count":targets,"evidence":basis,"contract_scan":contract_scan,"contract_signals":signals,"resource_attempt":resource_attempt,"inventory":location.join("inventory-records.json")}));
                }
            }
        }
        let successful = p.is_some_and(|p| p["build_success"] == true);
        complete_inventory &= trusted && successful && inventory_complete && units > 0;
        for source in &sources {
            let status = if !trusted {
                "execution_failed"
            } else if p.is_some_and(|p| p["build_success"] == false) {
                if p.is_some_and(|p| p["build_failure_kind"] == "verification_failed") {
                    "build_verification_failed"
                } else {
                    "build_failed"
                }
            } else if !successful || !inventory_complete || units == 0 {
                "inventory_unavailable"
            } else if members.contains(source) {
                "compiler_input"
            } else {
                "not_in_build"
            };
            let mut counts = BTreeMap::<String, usize>::new();
            for f in functions
                .iter()
                .filter(|f| f["profile"] == name && f["source"].as_str() == source.to_str())
            {
                *counts.entry(text(&f["status"])?.into()).or_default() += 1;
            }
            files.push(json!({"profile":name,"source":source,"status":status,"functions":counts,"registration_known":trusted && successful && inventory_complete && units>0,"evidence":capture.join(name),"note":if status=="not_in_build"{"No membership in actual compiler inputs for this profile; reason not inferred"}else{"File membership and typed function coverage are separate; runtime outcomes, if available, are in checks/report.json"}}));
        }
    }
    for function in &mut functions {
        let matches = array(&source_test_routes["candidates"])
            .filter(|c| {
                c["input"]["profile"] == function["profile"]
                    && c["input"]["unit"] == function["unit"]
                    && c["input"]["source"] == function["source"]
                    && c["test"] == function["owner"]["path"]
                    && c["body_check_complete"] == true
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            let candidate = matches[0];
            function["source_attempt"] = candidate.clone();
            if function["baseline"] != true
                && !matches!(function["status"].as_str(), Some("proved" | "not_proved"))
            {
                function["evidence"] = json!({"source_body":candidate["result"]["case"],"previous_attempt":function["evidence"],"input_scope":candidate["result"]["result"]["check"]["input_scope"]});
                function["status"] =
                    candidate["result"]["result"]["check"]["verification"]["status"].clone();
                function["explicit_target_count"]=candidate["result"]["result"]["check"]["annotation_correspondence"]["explicit_assertions"].clone();
            }
        }
    }
    for file in &mut files {
        let mut counts = BTreeMap::<String, usize>::new();
        for function in functions
            .iter()
            .filter(|f| f["profile"] == file["profile"] && f["source"] == file["source"])
        {
            *counts.entry(text(&function["status"])?.into()).or_default() += 1;
        }
        file["functions"] = json!(counts);
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for row in &functions {
        *counts.entry(text(&row["status"])?.into()).or_default() += 1;
    }
    let unresolved = functions.iter().filter(|r| r["source"].is_null()).count();
    let analysis_complete = (!(config.source_tests || config.source_functions)
        || source_test_routes["analysis_complete"] == true)
        && unresolved == 0
        && files.iter().all(|r| r["status"] == "compiler_input")
        && functions.iter().all(|r| {
            matches!(
                r["status"].as_str(),
                Some("proved" | "not_proved" | "not_exec")
            )
        });
    Ok(
        json!({"report_complete":true,"analysis_complete":analysis_complete,"requested_sources":sources,"sources":files,"functions":functions,"function_statuses":counts,
        "build_failures":array(&captured["profiles"]).filter(|p|p["build_success"] == false).map(|p|json!({"profile":p["profile"],"kind":p["build_failure_kind"],"obstacles":p["build_obstacles"],"stdout":p["build_stdout"],"stderr":p["evidence"]})).collect::<Vec<_>>(),
        "compiler_totals":{"inventory_complete":complete_inventory,"functions":if complete_inventory{Some(total_functions)}else{None},"functions_observed":total_functions,"owners_including_nested":total_owners,"mir_calls":total_calls},
        "contract_review_items":contract_review.into_values().collect::<Vec<_>>(),"resource_routes":resource_routes,"source_test_routes":source_test_routes,"scope_function_count":expected.len()-unresolved,"scope_unresolved_count":unresolved,"out_of_scope_function_count":total_functions-expected.len(),"runtime":"Host test execution, if performed by project-check, is recorded in checks/report.json; kernel ktest is not executed","quality_conclusion":"Constant predicate occurrences are review signals; no spec defect is inferred from these signals alone, failed builds, unsupported code or unproved obligations"}),
    )
}
fn fingerprint(path: &Path, map: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    map.insert(path.to_owned(), hash(&fs::read(path)?)?);
    Ok(())
}
fn artifacts(dir: &Path, map: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        let name = p.file_name().unwrap().to_string_lossy();
        if p.is_dir() {
            if ![
                "cargo-target",
                "compiler-output",
                "verifier-output",
                "original-output",
                "generated-output",
                "target",
            ]
            .contains(&name.as_ref())
            {
                artifacts(&p, map)?;
            }
        } else if p.extension().is_some_and(|s| {
            ["json", "stderr", "stdout", "vir", "rs", "md", "txt"]
                .iter()
                .any(|x| s == *x)
        }) && name != "evidence.json"
        {
            fingerprint(&p, map)?;
        }
    }
    Ok(())
}

fn resource_checks(
    plan: &Config,
    config: &Path,
    capture: &Path,
    sources: &BTreeSet<PathBuf>,
    sdk: &Path,
    out: &Path,
) -> Result<()> {
    fs::create_dir(out)?;
    let tool = out
        .parent()
        .ok_or("missing resource output parent")?
        .join("run-tool");
    let moves = if let Some(path) = &plan.resource_api_moves {
        config.parent().unwrap().join(path).canonicalize()?
    } else {
        let path = out.join("api-moves.json");
        write_json(&path, &json!({}))?;
        path
    };
    let report = read(&capture.join("report.json"))?;
    let mut results = Vec::new();
    for profile in array(&report["profiles"]) {
        for unit in array(&profile["units"]).filter(|u| u["status"] == "captured") {
            let location = capture
                .join(text(&profile["profile"])?)
                .join(text(&unit["evidence"])?);
            let base = location.parent().ok_or("missing resource unit")?;
            let context = base.join("context.json");
            let inv: Invocation = serde_json::from_slice(&fs::read(&context)?)?;
            let package = Path::new(
                inv.env
                    .get("CARGO_MANIFEST_DIR")
                    .ok_or("missing resource package")?,
            )
            .canonicalize()?;
            let native = read(&base.join("verification-input.json"))?;
            let identity = read(&base.join("source-identity.json"))?;
            let members = array(&identity["inputs"])
                .filter_map(|f| f["canonical_path"].as_str())
                .map(PathBuf::from)
                .collect::<BTreeSet<_>>();
            for source in sources
                .iter()
                .filter(|p| members.contains(*p) && p.starts_with(&package))
            {
                let case = out.join(format!("case-{}", results.len()));
                fs::create_dir(&case)?;
                let config = case.join("config.json");
                write_json(
                    &config,
                    &json!({"context":context,"package":package,"vir":native["vir"],"api_moves":moves,"sources":[source.strip_prefix(&package)?]}),
                )?;
                let run = case.join("run");
                let execution = stage(
                    Command::new(&tool)
                        .arg("resource-project-check")
                        .arg("--config")
                        .arg(&config)
                        .arg("--verus-root")
                        .arg(sdk)
                        .arg("--out")
                        .arg(&run),
                    &case,
                    "check",
                    plan.wall_seconds,
                )?;
                results.push(json!({"profile":profile["profile"],"unit":base.file_name().and_then(|s|s.to_str()),"source":source,
                    "origin":"captured_project","adaptations":[],"execution":execution,"evidence":run,
                    "result":optional(&run.join("report.json"))?}));
            }
        }
    }
    write_json(
        &out.join("report.json"),
        &json!({"results":results,
        "counting":"Resource attempts supplement the compiler inventory; do not add their registrations to ordinary or native baseline counts.",
        "runtime":"not_executed"}),
    )
}

pub fn run(
    config: &Path,
    manifest: &Path,
    sdk: &Path,
    out: &Path,
    reuse: Option<&Path>,
) -> Result<()> {
    let config = config.canonicalize()?;
    let manifest = manifest.canonicalize()?;
    let plan: Config = serde_json::from_slice(&fs::read(&config)?)?;
    if plan.resource_api_moves.is_some() && !plan.resources {
        return Err("resource_api_moves requires resources=true".into());
    }
    if plan.wall_seconds == 0 || plan.wall_seconds > 3600 {
        return Err("wall_seconds must be 1..3600 per stage".into());
    }
    let build = config.parent().unwrap().join(&plan.build).canonicalize()?;
    let sdk = sdk.canonicalize()?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let tool = out.join("run-tool");
    fs::copy(std::env::current_exe()?, &tool)?;
    let mut inputs = BTreeMap::new();
    for p in [&config, &build, &manifest, &tool] {
        fingerprint(p, &mut inputs)?;
    }
    if let Some(path) = &plan.resource_api_moves {
        fingerprint(
            &config.parent().unwrap().join(path).canonicalize()?,
            &mut inputs,
        )?;
    }
    let mut absent = Vec::new();
    let driver = sdk
        .parent()
        .and_then(Path::parent)
        .ok_or("missing SDK source directory")?
        .join("rust_verify/src/driver.rs");
    for p in [
        sdk.join("verus"),
        sdk.join("rust_verify"),
        sdk.join("cargo-verus"),
        driver,
    ] {
        if p.is_file() {
            fingerprint(&p, &mut inputs)?;
        } else {
            absent.push(p);
        }
    }
    let mut stages = BTreeMap::new();
    stages.insert(
        "metadata",
        stage(
            Command::new("cargo")
                .args([
                    "metadata",
                    "--no-deps",
                    "--offline",
                    "--format-version",
                    "1",
                    "--manifest-path",
                ])
                .arg(&manifest),
            &out,
            "metadata-stage",
            plan.wall_seconds,
        )?,
    );
    let metadata = serde_json::from_slice::<Value>(&fs::read(out.join("metadata-stage.stdout"))?)
        .unwrap_or(Value::Null);
    let root = metadata["workspace_root"]
        .as_str()
        .map(Path::new)
        .unwrap_or(manifest.parent().unwrap())
        .canonicalize()?;
    if out.starts_with(&root) {
        return Err("report output must be outside the source workspace".into());
    }
    let sources = expand(&root, &plan.sources)?;
    for p in &sources {
        fingerprint(p, &mut inputs)?;
    }
    let selected_sources = out.join("sources.json");
    write_json(&selected_sources, &json!({"sources":sources}))?;
    let capture = reuse
        .map(Path::canonicalize)
        .transpose()?
        .unwrap_or_else(|| out.join("capture"));
    if reuse.is_some() {
        stages.insert(
            "capture",
            stage(
                Command::new(&tool)
                    .arg("project-validate")
                    .arg("--capture")
                    .arg(&capture)
                    .arg("--config")
                    .arg(&build),
                &out,
                "capture-stage",
                plan.wall_seconds,
            )?,
        );
    } else {
        stages.insert(
            "capture",
            stage(
                Command::new(&tool)
                    .arg("project-capture")
                    .arg("--config")
                    .arg(&build)
                    .arg("--manifest-path")
                    .arg(&manifest)
                    .arg("--verus-root")
                    .arg(&sdk)
                    .arg("--out")
                    .arg(&capture),
                &out,
                "capture-stage",
                plan.wall_seconds,
            )?,
        );
    }
    let trusted = capture.join("inputs.json").is_file()
        && project::validate_inputs(&capture, &build).is_ok()
        && captured_context_matches(&capture, &root, &sdk).unwrap_or(false);
    let capture_report = optional(&capture.join("report.json"))?;
    let links = out.join("links");
    let checks = out.join("checks");
    let dossiers = out.join("dossiers");
    let ordinary = out.join("ordinary");
    let resources = out.join("resources");
    let source_tests = out.join("source-tests");
    if trusted && capture_report["status"] == "captured" && stages["capture"]["status"] == "ok" {
        stages.insert(
            "links",
            stage(
                Command::new(&tool)
                    .arg("project-link")
                    .arg("--capture")
                    .arg(&capture)
                    .arg("--config")
                    .arg(&build)
                    .arg("--out")
                    .arg(&links),
                &out,
                "link-stage",
                plan.wall_seconds,
            )?,
        );
        if stages["links"]["status"] == "ok" {
            stages.insert(
                "checks",
                stage(
                    Command::new(&tool)
                        .arg("project-check")
                        .arg("--links")
                        .arg(&links)
                        .arg("--verus-root")
                        .arg(&sdk)
                        .arg("--out")
                        .arg(&checks)
                        .arg("--sources")
                        .arg(&selected_sources),
                    &out,
                    "check-stage",
                    plan.wall_seconds,
                )?,
            );
            if stages["checks"]["status"] == "ok" {
                stages.insert(
                    "dossiers",
                    stage(
                        Command::new(&tool)
                            .arg("project-evidence")
                            .arg("--input")
                            .arg(&checks)
                            .arg("--out")
                            .arg(&dossiers),
                        &out,
                        "dossier-stage",
                        plan.wall_seconds,
                    )?,
                );
                let mut relative_sources = BTreeSet::new();
                for package in array(&metadata["packages"]) {
                    let dir = Path::new(text(&package["manifest_path"])?)
                        .parent()
                        .unwrap()
                        .canonicalize()?;
                    for source in &sources {
                        if let Ok(p) = source.strip_prefix(&dir) {
                            relative_sources.insert(p);
                        }
                    }
                }
                let scope = out.join("ordinary-config.json");
                write_json(&scope, &json!({"sources":relative_sources}))?;
                stages.insert(
                    "ordinary",
                    stage(
                        Command::new(&tool)
                            .arg("ordinary-check")
                            .arg("--input")
                            .arg(&checks)
                            .arg("--config")
                            .arg(&scope)
                            .arg("--verus-root")
                            .arg(&sdk)
                            .arg("--out")
                            .arg(&ordinary),
                        &out,
                        "ordinary-stage",
                        plan.wall_seconds,
                    )?,
                );
            }
        }
    }
    if plan.resources && trusted && stages["capture"]["status"] == "ok" {
        let result = match resource_checks(&plan, &config, &capture, &sources, &sdk, &resources) {
            Ok(()) => json!({"status":"ok","evidence":resources.join("report.json")}),
            Err(error) => json!({"status":"execution_failed","error":error.to_string()}),
        };
        stages.insert("resources", result);
    }
    if (plan.source_tests || plan.source_functions)
        && trusted
        && stages["capture"]["status"] == "ok"
    {
        let result = match source_test_checks(&plan, &capture, &sources, &sdk, &source_tests) {
            Ok(()) => json!({"status":"ok","evidence":source_tests.join("report.json")}),
            Err(error) => json!({"status":"execution_failed","error":error.to_string()}),
        };
        stages.insert("source_tests", result);
    }
    let meta = json!({"workspace":root,"config":config,"build_config":build,"capture":capture,"capture_trusted":trusted,"checks":checks,"ordinary":ordinary,"resources":resources,"source_tests":source_tests,"dossiers":dossiers,"stages":stages,"reused_capture":reuse.is_some(),"absent_inputs":absent,"checker_sha256":hash(&fs::read(&tool)?)?});
    let mut report = match assemble(&meta) {
        Ok(v) => v,
        Err(e) => json!({"report_complete":false,"error":e.to_string()}),
    };
    report["context"] = meta;
    if reuse.is_some() && (!trusted || stages["capture"]["status"] != "ok") {
        report["report_complete"] = json!(false);
        report["error"] =
            json!("Reused capture rejected; old registrations and results are not trusted");
    }
    for (p, h) in &inputs {
        if hash(&fs::read(p)?)? != *h {
            report["report_complete"] = json!(false);
            report["error"] = json!("Inputs changed during analysis");
        }
    }
    if trusted && project::validate_inputs(&capture, &build).is_err() {
        report["report_complete"] = json!(false);
        report["error"] = json!("Captured inputs changed during analysis");
    }
    write_json(&out.join("report.json"), &report)?;
    let mut summary = format!(
        "# 项目检查范围\n\n报告清单完整：{}。范围内已捕获注册函数 {} 项，源码归属未确定 {} 项。\n\n",
        report["report_complete"], report["scope_function_count"], report["scope_unresolved_count"]
    );
    summary.push_str(&format!(
        "全部输入完成分析：{}。报告完整性与分析覆盖分别判定。\n\n",
        report["analysis_complete"]
    ));
    if plan.source_tests || plan.source_functions {
        if report["source_test_routes"].is_object() {
            let candidates = &report["source_test_routes"]["candidates"];
            summary.push_str(&format!("## 源码调用方检查\n\n已识别函数候选 {} 项，完成目标检查 {} 项。候选与编译器注册清单可能重叠，分别统计。\n\n| 函数 | 完成目标检查 | 结果 |\n|---|---|---|\n",array(candidates).count(),array(candidates).filter(|r|r["complete_property_coverage"] == true).count()));
            for candidate in array(candidates) {
                summary.push_str(&format!(
                    "| {} | {} | {} |\n",
                    text(&candidate["test"])?,
                    candidate["complete_property_coverage"],
                    candidate["result"]["result"]["check"]["verification"]["status"]
                ));
            }
            summary.push_str("\n模块编译失败、未扫描源码和阶段失败保留在 source_test_routes 中。测试激活副本与独立候选的适用范围分别记录。\n\n");
            summary.push_str("验证结果包含所选函数及其内部定义的函数，失败位置见逐项诊断。以下仅列有自身断言的完整检查，归因仍待对照确认。\n\n| 所选函数 | 失败类型 | 原源码位置 | 相关契约 |\n|---|---|---|---|\n");
            for candidate in array(candidates).filter(|r| r["complete_property_coverage"] == true) {
                for obligation in
                    array(&candidate["result"]["result"]["check"]["review"]["obligations"])
                {
                    let locations = array(&obligation["client_locations"])
                        .filter(|s| s["primary"] == true)
                        .map(|s| {
                            format!(
                                "{}:{}",
                                s["original"]["file"].as_str().unwrap_or("unknown"),
                                s["original"]["line"]
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let contracts = array(&obligation["related_contracts"])
                        .filter_map(|c| c["function"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    summary.push_str(&format!(
                        "| {} | {} | {} | {} |\n",
                        text(&candidate["test"])?,
                        text(&obligation["kind"])?,
                        locations,
                        contracts
                    ));
                }
            }
            summary.push('\n');
        } else {
            summary.push_str("源码测试检查未完成，候选数量尚未确定；阶段诊断见 report.json。\n\n");
        }
    }
    if array(&report["contract_review_items"]).next().is_some() {
        summary.push_str("## 契约审阅信号\n\n这些后置条件引用了常量 predicate，需结合调用需求审阅。\n\n| 定义 | 值 | 使用该关系的接口 |\n|---|---|---|\n");
        for item in array(&report["contract_review_items"]) {
            let uses = array(&item["postcondition_uses"])
                .map(|r| format!("`{}`", r["owner"].as_str().unwrap_or("unknown")))
                .collect::<Vec<_>>()
                .join(", ");
            summary.push_str(&format!(
                "| `{}` | `{}` | {} |\n",
                item["predicate"]["resolved_to"]
                    .as_str()
                    .unwrap_or("unknown"),
                item["predicate"]["value"],
                uses
            ));
        }
        summary.push_str("\n定义位置与完整条件保留在 report.json；这些信号不直接计为缺陷。\n\n");
    }
    summary.push_str("| 构建 profile | 源码 | 构建范围状态 | 函数结果 |\n|---|---|---|---|\n");
    for r in array(&report["sources"]) {
        let path = Path::new(text(&r["source"])?);
        summary.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            text(&r["profile"])?,
            path.strip_prefix(&root).unwrap_or(path).display(),
            text(&r["status"])?,
            r["functions"]
        ));
    }
    summary.push_str(&format!("\n函数结果：{}。\n\n`not_in_build` 表示未进入该 profile 的编译输入；`build_failed` 表示 profile 构建失败，源码是否参与尚未知；`build_verification_failed` 表示构建中的 Verus 验证失败；`not_exec` 为 spec／proof 定义，单列为非执行对象；`unsupported` 表示检查能力不支持；`execution_failed` 和 `timeout` 保留工具执行失败。`proved`／`not_proved` 要求完整检查和转换核对，结果来自所在验证范围，失败尚未逐函数归因。已有 Verus 基线单列；无显式目标的函数体检查不代表新的性质覆盖。\n\nHost 测试若由 project-check 执行，原始结果保存在 checks/report.json；内核 ktest 未运行。构建失败、不支持和未证明均不直接推导为 spec 缺陷。源码位置、诊断、阶段命令与证据路径见 report.json。\n",report["function_statuses"]));
    fs::write(out.join("README.md"), summary)?;
    let mut evidence = inputs;
    artifacts(&out, &mut evidence)?;
    if reuse.is_some() {
        artifacts(&capture, &mut evidence)?;
    }
    write_json(&out.join("evidence.json"), &evidence)?;
    if report["report_complete"] != true {
        return Err("scope report incomplete or stale; see report.json".into());
    }
    validate(&out)?;
    println!(
        "Report complete; {} registered functions in scope; {}",
        report["scope_function_count"],
        out.join("report.json").display()
    );
    Ok(())
}
pub fn validate(out: &Path) -> Result<()> {
    let expected: BTreeMap<PathBuf, String> =
        serde_json::from_slice(&fs::read(out.join("evidence.json"))?)?;
    if expected.is_empty() || !expected.contains_key(&out.canonicalize()?.join("report.json")) {
        return Err("report evidence index is incomplete".into());
    }
    for (p, h) in &expected {
        if hash(&fs::read(p)?)? != *h {
            return Err(format!("Stale or changed evidence: {}", p.display()).into());
        }
    }
    let report = read(&out.join("report.json"))?;
    if report["report_complete"] != true {
        return Err("scope report was incomplete".into());
    }
    let m = &report["context"];
    if m["checker_sha256"] != hash(&fs::read(std::env::current_exe()?)?)? {
        return Err("scope report belongs to a different checker; rerun scope-check".into());
    }
    if array(&m["absent_inputs"]).any(|p| p.as_str().is_none_or(|s| Path::new(s).exists())) {
        return Err("previously absent toolchain input appeared".into());
    }
    if m["capture_trusted"] == true {
        project::validate_inputs(
            Path::new(text(&m["capture"])?),
            Path::new(text(&m["build_config"])?),
        )?;
    }
    let mut current = assemble(m)?;
    current["context"] = m.clone();
    if current != report {
        return Err("scope report omits or changes expected inputs or outcomes".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires SOURCE_TEST_SCOPE_RUN, SOURCE_TEST_OTHER_CAPTURE and fresh SOURCE_TEST_SCOPE_CONTROLS from actual captures"]
    fn source_tests_share_scope_without_hiding_omitted_inputs() {
        use super::*;
        let run = PathBuf::from(std::env::var("SOURCE_TEST_SCOPE_RUN").unwrap());
        let report = read(&run.join("report.json")).unwrap();
        let mut expected = report.clone();
        expected.as_object_mut().unwrap().remove("context");
        assert_eq!(assemble(&report["context"]).unwrap(), expected);
        let routes = &report["source_test_routes"];
        assert_eq!(array(&routes["candidates"]).count(), 2);
        assert!(array(&routes["candidates"]).all(|c| c["complete_property_coverage"] == true));
        assert_eq!(routes["analysis_complete"], true);
        assert_eq!(report["compiler_totals"]["functions"], 14);
        assert_eq!(report["scope_function_count"], 13);
        assert_eq!(report["scope_unresolved_count"], 1);
        let other = PathBuf::from(std::env::var("SOURCE_TEST_OTHER_CAPTURE").unwrap());
        let source = PathBuf::from(routes["candidates"][0]["input"]["source"].as_str().unwrap());
        assert!(
            source_test_slots(&other, &BTreeSet::from([source]))
                .unwrap()
                .is_empty()
        );
        let controls = PathBuf::from(std::env::var("SOURCE_TEST_SCOPE_CONTROLS").unwrap());
        fs::create_dir(&controls).unwrap();
        let mut meta = report["context"].clone();
        meta["source_tests"] = json!(controls);
        let mut attempts = routes["attempts"].clone();
        let original_attempt = attempts.clone();
        attempts["results"].as_array_mut().unwrap().clear();
        write_json(&controls.join("report.json"), &attempts).unwrap();
        assert!(
            assemble(&meta)
                .unwrap_err()
                .to_string()
                .contains("omits or changes project inputs")
        );
        let mut attempts = original_attempt;
        let census = PathBuf::from(attempts["results"][0]["census"].as_str().unwrap());
        let copy = controls.join("census");
        fs::create_dir(&copy).unwrap();
        fs::copy(census.join("parse.stderr"), copy.join("parse.stderr")).unwrap();
        let mut omitted = read(&census.join("report.json")).unwrap();
        omitted["test_candidates"].as_array_mut().unwrap().pop();
        write_json(&copy.join("report.json"), &omitted).unwrap();
        attempts["results"][0]["census"] = json!(copy);
        write_json(&controls.join("report.json"), &attempts).unwrap();
        assert!(
            assemble(&meta)
                .unwrap_err()
                .to_string()
                .contains("raw records")
        );
        attempts["results"][0]["census"] = json!(census);
        let checks = PathBuf::from(attempts["results"][0]["checks"].as_str().unwrap());
        let copy = controls.join("checks");
        fs::create_dir(&copy).unwrap();
        let mut omitted = read(&checks.join("report.json")).unwrap();
        omitted["cases"].as_array_mut().unwrap().pop();
        write_json(&copy.join("report.json"), &omitted).unwrap();
        attempts["results"][0]["checks"] = json!(copy);
        write_json(&controls.join("report.json"), &attempts).unwrap();
        assert!(
            assemble(&meta)
                .unwrap_err()
                .to_string()
                .contains("omit or reorder AST candidates")
        );
        write_json(&controls.join("controls.json"), &json!({"whole_input_omission_rejected":true,"ast_candidate_omission_rejected":true,"check_omission_rejected":true,"different_crate_source_ignored":true,"compiled_function_denominator":14,"functions_assigned_to_scope":13,"scope_unresolved":1,"supplemental_source_candidates":2})).unwrap();
    }
    #[test]
    #[ignore = "requires RANGE_PROFILE_SCOPE and fresh RANGE_SCOPE_REASSEMBLED"]
    fn complete_generated_tests_survive_an_unsupported_ordinary_attempt() {
        let previous: serde_json::Value = super::read(
            &std::path::PathBuf::from(std::env::var("RANGE_PROFILE_SCOPE").unwrap())
                .join("report.json"),
        )
        .unwrap();
        let report = super::assemble(&previous["context"]).unwrap();
        let tests = report["functions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                r["owner"]["path"]
                    .as_str()
                    .is_some_and(|p| p.starts_with("test::range_difference_"))
            })
            .collect::<Vec<_>>();
        assert_eq!(tests.len(), 9);
        for test in tests {
            assert_eq!(test["status"], "not_proved");
            assert_eq!(
                test["evidence"]["ordinary_attempt"]["status"],
                "unsupported"
            );
        }
        assert_eq!(report["function_statuses"]["unsupported"], 2);
        assert_eq!(report["scope_unresolved_count"], 1);
        let out = std::path::PathBuf::from(std::env::var("RANGE_SCOPE_REASSEMBLED").unwrap());
        std::fs::create_dir(&out).unwrap();
        super::write_json(&out.join("report.json"), &report).unwrap();
    }
    #[test]
    #[ignore = "requires PLACEHOLDER_SCOPE from the frozen WaitQueue evaluation"]
    fn real_missing_implementations_are_separate_from_tool_limits() {
        let run = PathBuf::from(std::env::var("PLACEHOLDER_SCOPE").unwrap());
        let old = read(&run.join("report.json")).unwrap();
        let report = assemble(&old["context"]).unwrap();
        assert_eq!(report["scope_function_count"], old["scope_function_count"]);
        assert_eq!(report["function_statuses"]["implementation_missing"], 2);
        assert_eq!(report["function_statuses"]["proved"], 14);
        assert_eq!(report["function_statuses"]["not_exec"], 6);
        assert!(report["function_statuses"]["unsupported"].is_null());
        for row in array(&report["functions"]).filter(|r| r["status"] == "implementation_missing") {
            assert_eq!(row["implementation"]["status"], "explicit_placeholder");
            assert_eq!(row["baseline"], false);
            assert_eq!(row["evidence"]["scope"], "not_checked");
        }
    }
    use super::*;
    #[test]
    #[ignore = "requires IMPLEMENTATION_INVENTORY and IMPLEMENTATION_CWD from actual compiler capture"]
    fn actual_placeholder_requires_source_and_whole_body_agreement() {
        let mut inventory = read(Path::new(
            &std::env::var("IMPLEMENTATION_INVENTORY").unwrap(),
        ))
        .unwrap();
        let cwd = PathBuf::from(std::env::var("IMPLEMENTATION_CWD").unwrap());
        let owner = array(&inventory["records"])
            .find(|r| r["kind"] == "owner" && r["path"] == "arch::irq::disable_local")
            .unwrap()
            .clone();
        assert_eq!(
            implementation_availability(&inventory, &owner, &cwd)["status"],
            "explicit_placeholder"
        );
        for record in inventory["records"].as_array_mut().unwrap() {
            if record["owner"] == owner["owner"] && record["kind"] == "mir_body" {
                record["blocks"] = json!("2");
            }
        }
        assert_eq!(
            implementation_availability(&inventory, &owner, &cwd)["status"],
            "not_identified_as_placeholder"
        );
    }

    #[test]
    #[ignore = "requires CONTRACT_SCOPE_RUN from the real kernel scope and a new CONTRACT_SCOPE_OUT"]
    fn actual_project_contract_scan_preserves_the_denominator() {
        let run = PathBuf::from(std::env::var("CONTRACT_SCOPE_RUN").unwrap());
        let out = PathBuf::from(std::env::var("CONTRACT_SCOPE_OUT").unwrap());
        let previous = read(&run.join("report.json")).unwrap();
        let frozen_tool = run.join("run-tool");
        let frozen_hash = hash(&fs::read(&frozen_tool).unwrap()).unwrap();
        assert_eq!(previous["context"]["checker_sha256"], frozen_hash);
        let evidence: BTreeMap<PathBuf, String> =
            serde_json::from_slice(&fs::read(run.join("evidence.json")).unwrap()).unwrap();
        let mut tool_aliases = Vec::new();
        for (path, expected) in evidence {
            if fs::read(&path)
                .ok()
                .and_then(|bytes| hash(&bytes).ok())
                .as_ref()
                != Some(&expected)
            {
                // Old reports also bound the mutable build output; its exact snapshot survives.
                assert_eq!(
                    expected,
                    frozen_hash,
                    "changed non-tool input: {}",
                    path.display()
                );
                tool_aliases
                    .push(json!({"original":path,"retained_bytes":frozen_tool,"sha256":expected}));
            }
        }
        let report = assemble(&previous["context"]).unwrap();
        assert_eq!(
            report["scope_function_count"],
            previous["scope_function_count"]
        );
        assert_eq!(report["function_statuses"], previous["function_statuses"]);
        let item = array(&report["contract_review_items"])
            .find(|r| {
                r["predicate"]["resolved_to"]
                    == "ostd::sync::rcu::non_null::either::impl&%1::ref_rel_perm"
            })
            .unwrap();
        assert_eq!(item["predicate"]["value"], true);
        assert_eq!(item["postcondition_uses"].as_array().unwrap().len(), 2);
        fs::create_dir(&out).unwrap();
        write_json(&out.join("report.json"), &json!({"source_report":run,"historical_tool_aliases":tool_aliases,"scanner_sha256":hash(&fs::read(std::env::current_exe().unwrap()).unwrap()).unwrap(),"report":report})).unwrap();
    }

    #[test]
    fn resource_fallback_preserves_complete_existing_checks() {
        let ordinary = json!({"result":{"status":"whole_body_checked","verification":{"status":"proved","exit_code":0,"summary":[1,0]}}});
        assert!(!resource_fills_gap(Some(&ordinary), None));
        let mut failed = ordinary.clone();
        failed["result"]["verification"] =
            json!({"status":"not_proved","exit_code":1,"summary":[0,1]});
        assert!(!resource_fills_gap(Some(&failed), None));
        failed["result"]["verification"]["summary"] = json!([0, 0]);
        assert!(resource_fills_gap(Some(&failed), None));
        let native = json!({"scope":"existing_native_baseline","conversion":{"correspondence":true},"verification_status":"proved","verification":{"exit_code":0,"summary":[1,0]}});
        assert!(!resource_fills_gap(None, Some(&native)));
    }

    #[test]
    #[ignore = "requires PARTIAL_SCOPE_RUN, PARTIAL_EXTRACTION and PARTIAL_OUT; combines actual positive and failed extraction records"]
    fn partial_extraction_keeps_other_verified_callers() {
        let run = PathBuf::from(std::env::var("PARTIAL_SCOPE_RUN").unwrap());
        let failed = read(Path::new(&std::env::var("PARTIAL_EXTRACTION").unwrap())).unwrap();
        let out = PathBuf::from(std::env::var("PARTIAL_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let (bad, failure) = failed["callers"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, v)| v["status"] == "not_analyzed")
            .unwrap();
        let good = failed["callers"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, v)| v["status"] == "typed")
            .unwrap()
            .0;
        let checks = out.join("checks");
        let unit = checks.join("kernel-library/unit-0");
        fs::create_dir_all(&unit).unwrap();
        let mut report = read(&run.join("checks/report.json")).unwrap();
        let row = &mut report["profiles"][0]["units"][0];
        fs::copy(row["callers"].as_str().unwrap(), unit.join("callers.json")).unwrap();
        let check = &mut row["check"];
        let mut correspondence =
            read(Path::new(check["correspondence"].as_str().unwrap())).unwrap();
        assert_eq!(correspondence[bad], true);
        assert_eq!(correspondence[good], true);
        correspondence[bad] = json!(false);
        write_json(&unit.join("correspondence.json"), &correspondence).unwrap();
        check["correspondence"] = json!(unit.join("correspondence.json"));
        check["extractions"]["original"]["callers"][bad] = failure.clone();
        check["status"] = json!("correspondence_incomplete");
        write_json(&checks.join("report.json"), &report).unwrap();
        let dossiers = out.join("dossiers");
        crate::evidence::run(&checks, &dossiers).unwrap();
        let mut meta = read(&run.join("report.json")).unwrap()["context"].clone();
        meta["checks"] = json!(checks);
        meta["dossiers"] = json!(dossiers);
        let mixed = assemble(&meta).unwrap();
        let original = read(&run.join("report.json")).unwrap();
        let identities = |r: &Value| {
            array(&r["functions"])
                .map(|f| f["id"].as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(identities(&mixed), identities(&original));
        let find = |path: &str| {
            array(&mixed["functions"])
                .find(|r| r["owner"]["path"] == path)
                .unwrap()
        };
        assert_eq!(find(bad)["status"], "extraction_failed");
        assert_eq!(find(good)["status"], "proved");
        assert_eq!(mixed["function_statuses"]["proved"], 58);
        assert_eq!(mixed["function_statuses"]["extraction_failed"], 1);
        write_json(&out.join("report.json"), &mixed).unwrap();
        write_json(&out.join("control.json"), &json!({"scope":"Controlled combination of actual failed old-probe extraction and current verified context; no new runtime or defect claim","positive":run,"negative":std::env::var("PARTIAL_EXTRACTION").unwrap(),"failed_caller":bad,"successful_caller":good})).unwrap();
    }
    #[test]
    fn no_verifier_summary_or_correspondence_cannot_be_a_complete_proof() {
        let mut r = json!({"status":"whole_body_checked","verification":{"status":"proved","exit_code":0,"summary":[0,0]}});
        assert_eq!(ordinary_status(&r), "execution_failed");
        r["verification"]["summary"] = json!([2, 0]);
        assert_eq!(ordinary_status(&r), "proved");
        r["verification"]["status"] = json!("not_proved");
        r["verification"]["summary"] = json!([1, 1]);
        assert_eq!(ordinary_status(&r), "not_proved");
        r["verification"]["exit_code"] = Value::Null;
        assert_eq!(ordinary_status(&r), "execution_failed");
        r["status"] = json!("backend_not_analyzed");
        r["verification"] = json!({"status":"unsupported","exit_code":1,"diagnostics":[{"level":"error","message":"unsupported cast"}]});
        assert_eq!(ordinary_status(&r), "unsupported");
        r["verification"]["exit_code"] = Value::Null;
        assert_eq!(ordinary_status(&r), "execution_failed");
        r["verification"]["exit_code"] = json!(1);
        r["verification"]["status"] = json!("unknown");
        assert_eq!(ordinary_status(&r), "verification_unknown");
        r["verification"]["status"] = json!("tool_failure");
        assert_eq!(ordinary_status(&r), "execution_failed");
        r["verification"]["status"] = json!("timeout");
        assert_eq!(ordinary_status(&r), "timeout");
        assert_eq!(
            checked_evidence_status(
                &json!({"scope":"generated_test","verification_status":"proved","conversion":{"correspondence":false}})
            ),
            "correspondence_failed"
        );
    }
    #[test]
    fn project_scope_cannot_escape_or_supply_client_assumptions() {
        assert!(!relative(Path::new("../src")));
        assert!(!relative(Path::new("/tmp/src")));
        assert!(!relative(Path::new("")));
        assert!(
            serde_json::from_value::<Config>(
                json!({"build":"build.json","sources":["src"],"requires":"true"})
            )
            .is_err()
        );
    }
}
