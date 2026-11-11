//! Run the resource route for every registered caller in a project snapshot.
use crate::{
    Result, borrowed_resource, capture::Invocation, client, ghost_abi, hash, ir, runner, vir,
    write_json,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    context: PathBuf,
    package: PathBuf,
    #[serde(default)]
    hir: Option<PathBuf>,
    #[serde(default)]
    sources: Vec<PathBuf>,
    vir: PathBuf,
    api_moves: PathBuf,
}

fn source_scope(package: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    paths
        .iter()
        .map(|path| {
            if path.as_os_str().is_empty()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("resource sources must be package-relative paths".into());
            }
            let absolute = package.join(path).canonicalize()?;
            if !absolute.starts_with(package) {
                return Err("resource source escapes package".into());
            }
            Ok(absolute)
        })
        .collect()
}

fn scoped_records(
    rows: &[Vec<String>],
    sources: &[PathBuf],
    cwd: &Path,
) -> Result<(Vec<Vec<String>>, Vec<Value>)> {
    let mut output = Vec::new();
    let mut unresolved = Vec::new();
    let mut current: Option<(&str, bool)> = None;
    for row in rows {
        match row.first().map(String::as_str) {
            Some("SOURCE_OWNER") if row.len() == 3 && current.is_none() => {
                let selected = match ir::span(&row[2], cwd)
                    .and_then(|(file, _)| Ok(file.canonicalize()?))
                {
                    Ok(file) => sources
                        .iter()
                        .any(|s| file == *s || (s.is_dir() && file.starts_with(s))),
                    Err(error) => {
                        unresolved.push(json!({"owner":row[1],"span":row[2],"status":"scope_unresolved","reason":error.to_string()}));
                        false
                    }
                };
                current = Some((&row[1], selected));
            }
            Some("SOURCE_END") if row.len() == 2 => {
                let (owner, selected) = current.take().ok_or("source end without owner")?;
                if owner != row[1] {
                    return Err("source discovery owner mismatch".into());
                }
                if selected {
                    output.push(row.clone());
                }
                continue;
            }
            Some("SOURCE_OWNER" | "SOURCE_END") => {
                return Err("invalid source owner boundary".into());
            }
            _ if current.is_none() => return Err("source record outside owner".into()),
            _ => {}
        }
        if current.is_some_and(|(_, selected)| selected) {
            output.push(row.clone());
        }
    }
    if current.is_some() {
        return Err("incomplete source discovery".into());
    }
    Ok((output, unresolved))
}

fn helper_names(plan: &Value) -> Result<BTreeSet<String>> {
    let full_model = plan["standard_body_copies"]
        .as_array()
        .and_then(|entries| entries.iter().find(|v| v["output"] == "checked.rs"))
        .map(|v| &v["evidence"])
        .ok_or("checked source preparation missing")?;
    let mut helper_names = full_model["copies"]
        .as_array()
        .ok_or("standard copy identities missing")?
        .iter()
        .map(|v| {
            v["helper"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "standard helper identity missing".into())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if let Some(failure) = full_model["failure_helper"].as_str() {
        helper_names.insert(failure.into());
    }
    Ok(helper_names)
}

pub(crate) fn validate_complete(result: &Value, inv: &Invocation) -> Result<()> {
    let read = |path: &Path| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(path)?)?) };
    let case = Path::new(
        result["evidence"]
            .as_str()
            .ok_or("resource evidence missing")?,
    );
    if result["status"] != "whole_body_checked" || result["complete_target_check"] != true {
        return Err("resource caller has no complete check".into());
    }
    let plan_dir = case.join("checked-plan");
    let plan = read(&plan_dir.join("plan.json"))?;
    for (name, expected) in plan["generated_sources"]
        .as_object()
        .ok_or("missing source fingerprints")?
    {
        if hash(&fs::read(plan_dir.join(name))?)? != *expected {
            return Err("resource generated source changed".into());
        }
    }
    if plan["correspondence"]["matches"] != true
        || result["resource_correspondence"] != plan["correspondence"]
        || read(&plan_dir.join("before-normalized.json"))?
            != read(&plan_dir.join("after-normalized.json"))?
        || read(&case.join("std-correspondence.json"))?["matches"] != true
        || read(&case.join("std-correspondence.json"))? != result["std_correspondence"]
        || read(&case.join("std-before.json"))? != read(&case.join("std-after.json"))?
    {
        return Err("resource caller lacks both conversion checks".into());
    }
    let owner = result["owner"].as_str().ok_or("resource owner missing")?;
    let module = owner.rsplit_once("::").ok_or("resource module missing")?.0;
    let helpers = helper_names(&plan)?;
    for (index, helper) in helpers.iter().enumerate() {
        let name = format!("helper-{index}");
        let proof = runner::recorded_proof(case, &name)?;
        let command = read(&case.join(format!("{name}.command.json")))?;
        if proof["status"] != "proved"
            || proof["summary"] != json!([1, 0])
            || !command["args"].as_array().is_some_and(|a| {
                a.windows(2)
                    .any(|w| w[0] == "--verify-function" && w[1] == *helper)
            })
        {
            return Err("resource helper not independently verified".into());
        }
    }
    for name in ["full", "resource"] {
        let proof = runner::recorded_proof(case, name)?;
        let command = read(&case.join(format!("{name}.command.json")))?;
        let args = command["args"]
            .as_array()
            .ok_or("missing resource proof command")?;
        if args.iter().any(|a| {
            matches!(
                a.as_str(),
                Some("--no-verify" | "--no-lifetime" | "--verify-function")
            )
        }) || !args
            .windows(2)
            .any(|w| w[0] == "--verify-only-module" && w[1] == module)
        {
            return Err("resource proof scope changed".into());
        }
        if name == "resource" {
            if proof != result["resource_control"] || proof["status"] != "proved" {
                return Err("resource construction not verified".into());
            }
        } else if proof != result["verification"]
            || !matches!(proof["status"].as_str(), Some("proved" | "not_proved"))
            || (proof["status"] == "not_proved" && proof["summary"][1] != 1)
            || !proof["summary"].as_array().is_some_and(|a| {
                a.iter().filter_map(Value::as_u64).sum::<u64>() == helpers.len() as u64 + 1
            })
        {
            return Err("resource caller verification incomplete".into());
        }
    }
    let target_path = result["target"]["path"]
        .as_str()
        .ok_or("resource target missing")?;
    if target_path.split_once("::").map(|(_, path)| path) != Some(owner) {
        return Err("resource target differs from registered caller".into());
    }
    let native = fs::read_to_string(case.join("full-vir/crate.vir"))?;
    let target =
        vir::checked_target_in(&native, target_path, &case.join("full-package"), &inv.cwd)?;
    if target.as_ref() != Some(&result["target"]) || result["target"]["existing_preconditions"] != 0
    {
        return Err("resource target body or entry conditions changed".into());
    }
    let (source, _) = ir::span(
        result["target"]["span"]
            .as_str()
            .ok_or("resource target span missing")?,
        &inv.cwd,
    )?;
    if hash(&fs::read(source)?)? != plan["generated_sources"]["checked.rs"] {
        return Err("resource proof source differs from generated source".into());
    }
    Ok(())
}

fn diagnostics(raw: &str) -> Vec<Value> {
    raw.lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .map(|d| {
            if d["reason"] == "compiler-message" {
                d["message"].clone()
            } else {
                d
            }
        })
        .filter(|d| d["level"] == "error")
        .collect()
}
pub(crate) fn compiler_obstacles(raw: &str) -> Vec<Value> {
    diagnostics(raw).into_iter().filter_map(|diagnostic| {
        let message = diagnostic["message"].as_str().unwrap_or("");
        if message.starts_with("aborting ") { return None; }
        let code = diagnostic["code"]["code"].as_str();
        let kind = match code {
            Some("E0614") => "deref_unavailable",
            Some("E0277") => "trait_bound_unsatisfied",
            Some("E0432" | "E0433" | "E0576" | "E0599") => "interface_unavailable",
            Some("E0061" | "E0050" | "E0053") => "interface_mismatch",
            _ if message.contains("cannot find attribute") => "macro_unavailable",
            _ if runner::unsupported_message(message) => "backend_unsupported",
            _ => "compiler_error",
        };
        Some(json!({"kind":kind,"code":code,"message":message,"spans":diagnostic["spans"],
            "scope":"Compiler fact for this configuration; does not assign fault to the caller or library."}))
    }).collect()
}
fn source_capture(
    probe: &Path,
    inv: &Invocation,
    args: &[String],
    out: &Path,
    name: &str,
) -> Result<Vec<Vec<String>>> {
    let mut command = runner::probe_command(probe, inv, args, "*", true);
    command.env("CLIENT_PROJECT_SOURCE_ONLY", "1");
    let result = runner::compile(&mut command, out, name, &inv.args)?;
    let raw = String::from_utf8_lossy(&result.stderr);
    let rows = raw
        .lines()
        .filter(|s| s.starts_with("SOURCE_"))
        .map(|s| s.split('\t').map(str::to_owned).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    write_json(&out.join(format!("{name}.json")), &rows)?;
    write_json(
        &out.join(format!("{name}-status.json")),
        &json!({"status":"source_only","compiler_exit_code":result.status.code(),"diagnostics":diagnostics(&raw),"compiler_obstacles":compiler_obstacles(&raw),"verification_coverage":false}),
    )?;
    Ok(rows)
}
fn typed(probe: &Path, inv: &Invocation, args: &[String], out: &Path, name: &str) -> Result<Value> {
    let mut command = runner::probe_command(probe, inv, args, "*", true);
    command.env_remove("CLIENT_PROJECT_SOURCE_ONLY");
    let result = runner::compile(&mut command, out, name, &inv.args)?;
    let raw = String::from_utf8_lossy(&result.stderr);
    let value = json!({"status":if result.status.success(){"typed"}else{"build_failed"},"compiler_exit_code":result.status.code(),"diagnostics":diagnostics(&raw),"compiler_obstacles":compiler_obstacles(&raw)});
    write_json(&out.join(format!("{name}.json")), &value)?;
    Ok(value)
}

pub fn run(config: &Path, sdk: &Path, out: &Path) -> Result<()> {
    let config = fs::canonicalize(config)?;
    let base = config.parent().ok_or("configuration directory missing")?;
    let resolve = |p: PathBuf| -> Result<PathBuf> {
        Ok(fs::canonicalize(if p.is_absolute() {
            p
        } else {
            base.join(p)
        })?)
    };
    let config_bytes = fs::read(&config)?;
    let config_value: Config = serde_json::from_slice(&config_bytes)?;
    let context = resolve(config_value.context)?;
    let package = resolve(config_value.package)?;
    if config_value.hir.is_some() == !config_value.sources.is_empty() {
        return Err("provide either legacy hir registration or package-relative sources".into());
    }
    let hir = config_value.hir.map(resolve).transpose()?;
    let sources = source_scope(&package, &config_value.sources)?;
    let native = resolve(config_value.vir)?;
    let moves = resolve(config_value.api_moves)?;
    let sdk = fs::canonicalize(sdk)?;
    let driver = sdk
        .parent()
        .and_then(Path::parent)
        .ok_or("SDK layout")?
        .join("rust_verify/src/driver.rs");
    let driver_text = fs::read_to_string(&driver)?;
    if !driver_text
        .contains("rustc_args.extend([\"--cfg\", \"verus_only\", \"--cfg\", \"verus_keep_ghost\"]")
        || !driver_text.contains("rustc_args_verify.extend([\"--cfg\", \"verus_keep_ghost_body\"]")
    {
        return Err(
            "resource route requires the captured SDK compilation/verification phase split".into(),
        );
    }
    fs::create_dir(out)?;
    let out = fs::canonicalize(out)?;
    if out.starts_with(&package) {
        return Err("resource output must be outside the input snapshot".into());
    }
    let mut inv: Invocation = serde_json::from_slice(&fs::read(&context)?)?;
    let mut fingerprints = BTreeMap::new();
    let run_tool = out.join("run-tool");
    fs::copy(std::env::current_exe()?, &run_tool)?;
    fingerprints.insert(run_tool, hash(&fs::read(out.join("run-tool"))?)?);
    runner::source_hashes(&package, &mut fingerprints)?;
    for path in [&context, &native, &moves, &config]
        .into_iter()
        .chain(hir.iter())
    {
        fingerprints.insert(path.clone(), hash(&fs::read(path)?)?);
    }
    fingerprints.insert(driver.clone(), hash(driver_text.as_bytes())?);
    for tool in ["verus", "rust_verify"] {
        let p = sdk.join(tool);
        fingerprints.insert(p.clone(), hash(&fs::read(p)?)?);
    }
    write_json(&out.join("inputs.json"), &fingerprints)?;
    let registration: Vec<Vec<String>> = hir
        .as_ref()
        .map(|p| -> Result<_> { Ok(serde_json::from_slice(&fs::read(p)?)?) })
        .transpose()?
        .unwrap_or_default();
    let mut registered = registration
        .iter()
        .filter(|r| r.first().is_some_and(|v| v == "SOURCE_OWNER"))
        .map(|r| {
            if r.len() != 3 {
                return Err("invalid registered source owner".into());
            }
            Ok(r[1].clone())
        })
        .collect::<Result<Vec<_>>>()?;
    let mut expected = registered.iter().cloned().collect::<BTreeSet<_>>();
    if hir.is_some() && (registered.is_empty() || expected.len() != registered.len()) {
        return Err("empty or duplicate resource registration".into());
    }
    let roots = inv
        .args
        .iter()
        .filter(|s| !s.starts_with('-') && s.ends_with(".rs"))
        .map(|p| {
            let p = Path::new(p);
            if p.is_absolute() {
                p.to_owned()
            } else {
                inv.cwd.join(p)
            }
        })
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        return Err("one captured Rust crate root required".into());
    };
    let package_root = package
        .join("src")
        .join(root.file_name().ok_or("crate root filename missing")?);
    if fs::read(root)? != fs::read(&package_root)? {
        return Err("project snapshot crate root differs from capture".into());
    }
    let relative_root = package_root.strip_prefix(&package)?.to_owned();
    inv.env.insert("VERUS_USE_RUSTUP".into(), "0".into());
    inv.env.insert(
        "VERUS_Z3_PATH".into(),
        sdk.parent()
            .and_then(Path::parent)
            .ok_or("SDK layout")?
            .join("z3")
            .display()
            .to_string(),
    );
    inv.env.insert("CLIENT_RESOURCE_LAYOUT".into(), "1".into());
    inv.env
        .insert("CLIENT_RESOLVE_INSTANCES".into(), "1".into());
    let selection = out.join("registered.txt");
    fs::write(&selection, registered.join("\n"))?;
    inv.env.remove("CLIENT_PROJECT_BORROW_PATHS");
    if hir.is_some() {
        inv.env.insert(
            "CLIENT_PROJECT_BORROW_PATHS".into(),
            selection.display().to_string(),
        );
    }
    let probe = runner::build_probe(&inv, &out)?.ok_or("Rustc probe unavailable")?;
    let initial_args = runner::clean(
        &inv.args,
        &out.join("source-output"),
        Some((root, &package_root)),
        &inv.cwd,
    )?;
    let mut rows = source_capture(&probe, &inv, &initial_args, &out, "source")?;
    let mut unresolved = Vec::new();
    if hir.is_none() {
        (rows, unresolved) = scoped_records(&rows, &sources, &inv.cwd)?;
        write_json(&out.join("unresolved-sources.json"), &unresolved)?;
        registered = rows
            .iter()
            .filter(|r| r[0] == "SOURCE_OWNER")
            .map(|r| r[1].clone())
            .collect();
        expected = registered.iter().cloned().collect();
        if registered.is_empty() || expected.len() != registered.len() {
            return Err(
                "source scope has no unique discovered functions; see source-status.json".into(),
            );
        }
        write_json(&out.join("source.json"), &rows)?;
    }
    write_json(&out.join("registered.json"), &registered)?;
    let actual = rows
        .iter()
        .filter(|r| r.first().is_some_and(|v| v == "SOURCE_OWNER"))
        .filter_map(|r| r.get(1).cloned())
        .collect::<BTreeSet<_>>();
    let mut results = Vec::new();
    if actual != expected {
        for owner in &registered {
            results.push(json!({"owner":owner,"status":"source_capture_failed","source":out.join("source-status.json")}));
        }
    } else {
        let abi = out.join("abi");
        ghost_abi::run(&out.join("source.json"), &native, &moves, &inv.cwd, &abi)?;
        let projected: Value = serde_json::from_slice(&fs::read(abi.join("report.json"))?)?;
        for (i, item) in projected["inputs"]
            .as_array()
            .ok_or("ABI input report missing")?
            .iter()
            .enumerate()
        {
            let case = out.join(format!("caller-{i}"));
            fs::create_dir(&case)?;
            let value = match check_case(
                item,
                &package,
                &relative_root,
                root,
                &native,
                &sdk,
                &probe,
                &inv,
                &case,
            ) {
                Ok(v) => v,
                Err(error) => json!({"status":"execution_failed","reason":error.to_string()}),
            };
            let mut value = value;
            value["owner"] = item["owner"].clone();
            value["evidence"] = json!(case);
            value["interface_migrations"] = json!(
                item["calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["project_migration"] == true)
                    .collect::<Vec<_>>()
            );
            if value["status"] == "whole_body_checked"
                && let Err(error) = validate_complete(&value, &inv)
            {
                value["status"] = json!("execution_failed");
                value["stage"] = json!("evidence_validation");
                value["complete_target_check"] = json!(false);
                value["reason"] = json!(error.to_string());
            }
            write_json(&case.join("report.json"), &value)?;
            results.push(value);
        }
    }
    let reported = results
        .iter()
        .filter_map(|r| r["owner"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    if reported != expected || results.len() != registered.len() {
        return Err("resource report registration mismatch".into());
    }
    let mut current = BTreeMap::new();
    runner::source_hashes(&package, &mut current)?;
    for path in fingerprints.keys().filter(|p| !p.starts_with(&package)) {
        current.insert(path.clone(), hash(&fs::read(path)?)?);
    }
    if current != fingerprints {
        return Err("resource inputs changed during checking".into());
    }
    write_json(
        &out.join("report.json"),
        &json!({"status":"reported","inputs":results,"registered":registered.len(),"unresolved_inputs":unresolved,"registration":{"mode":if hir.is_some(){"legacy_hir"}else{"source_discovery"},"sources":sources,"source_capture":out.join("source-status.json")},"input_fingerprints_unchanged":true,"manual_token_mappings":0,"new_entry_assumptions":0,"runtime_executed":false,"compiler_configuration":{"cfg":["verus_only","verus_keep_ghost"],"verus_keep_ghost_body":false,"source":driver},"scope":"Provided project snapshot and registered module-level zero-argument resource callers. SDK compilation MIR retains ghost declarations and erases proof bodies; this is separate from runtime execution. Existing API contracts and explicit project API moves retained. Standard/assertion conversion is reported separately; resource-only correspondence cannot establish full target coverage."}),
    )
}

#[allow(clippy::too_many_arguments)]
fn check_case(
    item: &Value,
    package: &Path,
    relative_root: &Path,
    original_root: &Path,
    native: &Path,
    sdk: &Path,
    probe: &Path,
    inv: &Invocation,
    case: &Path,
) -> Result<Value> {
    if item["status"] != "generated_unchecked" {
        return Ok(item.clone());
    }
    let owner = item["owner"].as_str().ok_or("ABI owner missing")?;
    let source = Path::new(item["source"].as_str().ok_or("ABI source missing")?);
    let relative_source = source.strip_prefix(package)?;
    let generated = Path::new(item["output"].as_str().ok_or("ABI output missing")?);
    if hash(&fs::read(source)?)? != item["source_sha256"]
        || hash(&fs::read(generated)?)? != item["output_sha256"]
    {
        return Err("ABI source or output changed".into());
    }
    let snapshot = case.join("package");
    runner::copy_package(package, &snapshot)?;
    fs::copy(generated, snapshot.join(relative_source))?;
    let selection = case.join("selection.txt");
    fs::write(&selection, owner)?;
    let mut inv = inv.clone();
    inv.env.insert(
        "CLIENT_PROJECT_BORROW_PATHS".into(),
        selection.display().to_string(),
    );
    let args = runner::clean(
        &inv.args,
        &case.join("compiler-output"),
        Some((original_root, &snapshot.join(relative_root))),
        &inv.cwd,
    )?;
    let rows = source_capture(probe, &inv, &args, case, "source")?;
    if !rows.iter().any(|r| {
        r.first().is_some_and(|v| v == "SOURCE_OWNER") && r.get(1).is_some_and(|p| p == owner)
    }) {
        return Ok(json!({"status":"source_capture_failed"}));
    }
    let compilation = typed(probe, &inv, &args, case, "typed")?;
    if compilation["status"] != "typed" {
        return Ok(json!({"status":"build_failed","compilation":compilation}));
    }
    let bodies = client::borrow_records(&fs::read_to_string(case.join("typed.stderr"))?)?;
    if !bodies.contains_key(owner) {
        return Err("successful compiler omitted registered caller".into());
    }
    let plan = case.join("plan");
    if let Err(error) = borrowed_resource::run(
        &case.join("typed.stderr"),
        &case.join("source.json"),
        native,
        &inv.cwd,
        &plan,
        None,
    ) {
        return Ok(
            json!({"status":"unsupported","stage":"resource_connection","reason":error.to_string()}),
        );
    }
    let connected = case.join("connected-package");
    runner::copy_package(&snapshot, &connected)?;
    fs::copy(plan.join("connected.rs"), connected.join(relative_source))?;
    let connected_args = runner::clean(
        &inv.args,
        &case.join("connected-output"),
        Some((original_root, &connected.join(relative_root))),
        &inv.cwd,
    )?;
    let compilation = typed(probe, &inv, &connected_args, case, "connected")?;
    if compilation["status"] != "typed" {
        return Ok(json!({"status":"build_failed","stage":"connected","compilation":compilation}));
    }
    let checked_plan = case.join("checked-plan");
    borrowed_resource::run(
        &case.join("typed.stderr"),
        &case.join("source.json"),
        native,
        &inv.cwd,
        &checked_plan,
        Some(&case.join("connected.stderr")),
    )?;
    let plan_report: Value = serde_json::from_slice(&fs::read(checked_plan.join("plan.json"))?)?;
    let helper_names = helper_names(&plan_report)?;
    let module = owner.rsplit_once("::").ok_or("caller module missing")?.0;
    let mut checks = BTreeMap::new();
    let mut full_correspondence = Value::Null;
    for (name, file) in [("full", "checked.rs"), ("resource", "resource-control.rs")] {
        let proof_package = case.join(format!("{name}-package"));
        runner::copy_package(&snapshot, &proof_package)?;
        if hash(&fs::read(checked_plan.join(file))?)? != plan_report["generated_sources"][file] {
            return Err("generated source changed before verification".into());
        }
        fs::copy(checked_plan.join(file), proof_package.join(relative_source))?;
        let mut args = runner::clean(
            &inv.args,
            &case.join(format!("{name}-output")),
            Some((original_root, &proof_package.join(relative_root))),
            &inv.cwd,
        )?;
        if name == "full" {
            let compilation = typed(probe, &inv, &args, case, "full-compiled")?;
            if compilation["status"] != "typed" {
                return Ok(
                    json!({"status":"build_failed","stage":"checked","compilation":compilation}),
                );
            }
            let before =
                client::borrow_records(&fs::read_to_string(case.join("connected.stderr"))?)?;
            let after =
                client::borrow_records(&fs::read_to_string(case.join("full-compiled.stderr"))?)?;
            let before = crate::std_body::pointer_correspondence_input(
                before.get(owner).ok_or("connected caller missing")?,
                false,
            )?;
            let after = crate::std_body::pointer_correspondence_input(
                after.get(owner).ok_or("checked caller missing")?,
                true,
            )?;
            let before = ir::remap(before, &BTreeMap::new(), true)?;
            let after = ir::remap(after, &BTreeMap::new(), true)?;
            let failure = format!("{module}::checked_eq_failure");
            let comparison = ir::compare(&before, &after, Some(&failure));
            full_correspondence = json!({"matches":comparison.as_ref().is_ok_and(|v|*v),"reason":comparison.err().map(|e|e.to_string()),"scope":"Pinned standard-body copies and abstract assertion-failure bridge; panic payloads and unwind semantics excluded."});
            write_json(
                &case.join("std-before.json"),
                &ir::normalize(&before, Some("core::panicking::assert_failed"))?,
            )?;
            write_json(
                &case.join("std-after.json"),
                &ir::normalize(&after, Some(&failure))?,
            )?;
            write_json(&case.join("std-correspondence.json"), &full_correspondence)?;
        }
        args.extend([
            "-Zcrate-attr=feature(panic_internals)".into(),
            "--verify-only-module".into(),
            module.into(),
        ]);
        if name == "full" {
            let mut helper_checks = BTreeMap::new();
            for (index, helper) in helper_names.iter().enumerate() {
                let mut helper_args = args.clone();
                helper_args.extend(["--verify-function".into(), helper.clone()]);
                helper_checks.insert(
                    helper.clone(),
                    runner::prove(sdk, &inv, &helper_args, case, &format!("helper-{index}"))?,
                );
            }
            let proved = helper_checks
                .values()
                .all(|v| v["status"] == "proved" && v["summary"] == json!([1, 0]));
            checks.insert("helpers",json!({"status":if proved{"proved"}else{"not_proved"},"summary":if proved{json!([helper_names.len(),0])}else{Value::Null},"functions":helper_checks}));
        }
        let result = runner::prove(sdk, &inv, &args, case, name)?;
        if hash(&fs::read(proof_package.join(relative_source))?)?
            != plan_report["generated_sources"][file]
        {
            return Err("compiled source changed during verification".into());
        }
        checks.insert(name, result);
    }
    let crate_name = inv
        .args
        .windows(2)
        .find(|w| w[0] == "--crate-name")
        .map(|w| w[1].as_str())
        .ok_or("crate identity missing")?;
    let text = fs::read_to_string(case.join("full-vir/crate.vir"))?;
    let target = vir::checked_target_in(
        &text,
        &format!("{crate_name}::{owner}"),
        &case.join("full-package"),
        &inv.cwd,
    )?;
    let native_functions = vir::functions(&text)?;
    let call_targets = target
        .as_ref()
        .and_then(|t| t["callees"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|path| crate::ordinary::call_contract(&native_functions, path))
        .collect::<Vec<_>>();
    let source_assertions = bodies[owner]
        .iter()
        .filter(|r| r["kind"] == "assert_operand")
        .map(|r| r["assertion_span"].clone())
        .collect::<BTreeSet<_>>();
    let explicit_targets = source_assertions.len()
        + call_targets
            .iter()
            .filter_map(|c| c["requires"].as_array())
            .map(Vec::len)
            .sum::<usize>();
    let complete = full_correspondence["matches"] == true
        && plan_report["correspondence"]["matches"] == true
        && target
            .as_ref()
            .is_some_and(|t| t["existing_preconditions"] == 0)
        && checks["helpers"]["status"] == "proved"
        && checks["helpers"]["summary"] == json!([helper_names.len(), 0])
        && checks["resource"]["status"] == "proved"
        && matches!(
            checks["full"]["status"].as_str(),
            Some("proved" | "not_proved")
        )
        && (checks["full"]["status"] == "proved"
            || checks["full"]["summary"].get(1).and_then(Value::as_u64) == Some(1))
        && checks["full"]["summary"].as_array().is_some_and(|v| {
            v.iter().filter_map(Value::as_u64).sum::<u64>() == helper_names.len() as u64 + 1
        });
    let generated_source = case.join("full-package").join(relative_source);
    let compiled = client::borrow_records(&fs::read_to_string(case.join("full-compiled.stderr"))?)?;
    let review = crate::review::summarize(
        &checks["full"],
        &text,
        &inv.cwd,
        &generated_source,
        &generated_source,
        &[],
        compiled
            .get(owner)
            .ok_or("checked caller records missing")?,
    )?;
    let review = json!({"location_scope":"Generated caller source; original assertion spans are recorded separately in source_assertions", "obligations":review});
    Ok(
        json!({"review":review,"status":if complete{"whole_body_checked"}else{"resource_route_checked"},"compilation":compilation,"resource_correspondence":plan_report["correspondence"],"std_correspondence":full_correspondence,"plan":checked_plan,"target":target,"source_assertions":source_assertions,"call_targets":call_targets,"explicit_target_count":explicit_targets,"verification":checks["full"],"helpers":checks["helpers"],"resource_control":checks["resource"],"complete_target_check":complete,"logical_satisfiability":"not_checked","resource_construction":if checks["resource"]["status"]=="proved"{"proved_under_existing_api_contracts"}else{"not_proved"},"runtime_reachability":"not_executed"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires KTEST_BUILD_OUTPUT from the original-project cfg(ktest) build"]
    fn cargo_failures_keep_imports_and_uncoded_macro_errors() {
        let raw = fs::read_to_string(std::env::var("KTEST_BUILD_OUTPUT").unwrap()).unwrap();
        let obstacles = compiler_obstacles(&raw);
        assert_eq!(obstacles.len(), 166);
        assert!(obstacles.iter().any(|r| r["code"] == "E0432"
            && r["kind"] == "interface_unavailable"
            && r["message"] == "unresolved import `crate::prelude`"));
        assert!(
            obstacles
                .iter()
                .any(|r| r["code"].is_null() && r["kind"] == "macro_unavailable")
        );
        assert!(obstacles.iter().all(|r| r["spans"].is_array()));
    }

    #[test]
    #[ignore = "requires RESOURCE_PROJECT_RUN containing the real stale-interface and BoxRef compiler failures"]
    fn actual_compiler_failures_keep_interface_and_type_causes() {
        let run = PathBuf::from(std::env::var("RESOURCE_PROJECT_RUN").unwrap());
        let source = fs::read_to_string(run.join("source.stderr")).unwrap();
        assert!(
            compiler_obstacles(&source)
                .iter()
                .any(|r| r["code"] == "E0576" && r["kind"] == "interface_unavailable")
        );
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        let failed = report["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["status"] == "build_failed")
            .unwrap();
        let raw = fs::read_to_string(
            Path::new(failed["evidence"].as_str().unwrap()).join("typed.stderr"),
        )
        .unwrap();
        assert!(
            compiler_obstacles(&raw)
                .iter()
                .any(|r| r["code"] == "E0614" && r["kind"] == "deref_unavailable")
        );
    }

    #[test]
    #[ignore = "requires RESOURCE_PROJECT_RUN and RESOURCE_PROJECT_CONTEXT from a completed real resource run"]
    fn completed_resource_evidence_rejects_changed_claims() {
        let run = PathBuf::from(std::env::var("RESOURCE_PROJECT_RUN").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        let inv: Invocation = serde_json::from_slice(
            &fs::read(std::env::var("RESOURCE_PROJECT_CONTEXT").unwrap()).unwrap(),
        )
        .unwrap();
        let original = report["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["status"] == "whole_body_checked")
            .unwrap();
        validate_complete(original, &inv).unwrap();
        let mut changed = original.clone();
        changed["verification"]["summary"] = json!([5, 0]);
        assert!(validate_complete(&changed, &inv).is_err());
        let mut changed = original.clone();
        changed["std_correspondence"]["matches"] = json!(false);
        assert!(validate_complete(&changed, &inv).is_err());
        let mut changed = original.clone();
        changed["owner"] = json!("another::caller");
        assert!(validate_complete(&changed, &inv).is_err());
    }

    #[test]
    fn discovered_scope_keeps_whole_owners_and_rejects_broken_boundaries() {
        let root = std::env::temp_dir().join(format!("resource-discovery-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let selected = root.join("selected.rs");
        let other = root.join("other.rs");
        fs::write(&selected, "fn selected() {}\n").unwrap();
        fs::write(&other, "fn other() {}\n").unwrap();
        let mut rows = Vec::new();
        for (name, file) in [("selected", &selected), ("other", &other)] {
            rows.push(vec![
                "SOURCE_OWNER".into(),
                name.into(),
                format!("{}:1:1: 1:17 (#0)", file.display()),
            ]);
            rows.push(vec![
                "SOURCE_BODY".into(),
                name.into(),
                format!("{}:1:15: 1:17 (#0)", file.display()),
            ]);
            rows.push(vec!["SOURCE_END".into(), name.into()]);
        }
        assert_eq!(
            scoped_records(&rows, &[selected.canonicalize().unwrap()], &root)
                .unwrap()
                .0,
            rows[..3]
        );
        let mut broken = rows.clone();
        broken[2][1] = "wrong".into();
        assert!(scoped_records(&broken, std::slice::from_ref(&root), &root).is_err());
        assert!(scoped_records(&rows[..2], std::slice::from_ref(&root), &root).is_err());
        rows.extend([
            vec![
                "SOURCE_OWNER".into(),
                "generated_main".into(),
                "no-location (#0)".into(),
            ],
            vec!["SOURCE_END".into(), "generated_main".into()],
        ]);
        let (retained, unresolved) =
            scoped_records(&rows, std::slice::from_ref(&root), &root).unwrap();
        assert_eq!(retained.len(), 6);
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0]["owner"], "generated_main");
        assert!(source_scope(&root, &[PathBuf::from("../outside.rs")]).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires actual project case in RESOURCE_STD_CASE and new RESOURCE_STD_OUT"]
    fn replay_standard_correspondence() {
        let case = PathBuf::from(std::env::var("RESOURCE_STD_CASE").unwrap());
        let out = PathBuf::from(std::env::var("RESOURCE_STD_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let rows: Vec<Vec<String>> =
            serde_json::from_slice(&fs::read(case.join("source.json")).unwrap()).unwrap();
        let owner = &rows.iter().find(|r| r[0] == "SOURCE_OWNER").unwrap()[1];
        let module = owner.rsplit_once("::").unwrap().0;
        let before =
            client::borrow_records(&fs::read_to_string(case.join("connected.stderr")).unwrap())
                .unwrap();
        let after =
            client::borrow_records(&fs::read_to_string(case.join("full-compiled.stderr")).unwrap())
                .unwrap();
        let before = crate::std_body::pointer_correspondence_input(&before[owner], false).unwrap();
        let after = crate::std_body::pointer_correspondence_input(&after[owner], true).unwrap();
        let before = ir::remap(before, &BTreeMap::new(), true).unwrap();
        let after = ir::remap(after, &BTreeMap::new(), true).unwrap();
        let failure = format!("{module}::checked_eq_failure");
        write_json(
            &out.join("before.json"),
            &ir::normalize(&before, Some("core::panicking::assert_failed")).unwrap(),
        )
        .unwrap();
        write_json(
            &out.join("after.json"),
            &ir::normalize(&after, Some(&failure)).unwrap(),
        )
        .unwrap();
        assert!(ir::compare(&before, &after, Some(&failure)).unwrap());
        let mut changed = after.clone();
        let literal = changed
            .iter_mut()
            .find(|r| r["kind"] == "stmt" && r["operation"].contains("const 1_usize"))
            .unwrap();
        literal.insert(
            "operation".into(),
            literal["operation"].replace("const 1_usize", "const 2_usize"),
        );
        assert!(!ir::compare(&before, &changed, Some(&failure)).unwrap());
        let mut changed = after.clone();
        let call = changed
            .iter_mut()
            .find(|r| r["kind"] == "call" && r["target"] == "checked_pointer_eq")
            .unwrap();
        call.insert("target".into(), "unrelated_pointer_operation".into());
        assert!(!ir::compare(&before, &changed, Some(&failure)).unwrap());
    }
}
