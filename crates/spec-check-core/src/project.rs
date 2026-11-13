//! Capture actual Cargo profiles and inventory typed bodies without client selection.
use crate::{Result, capture, execute, hash, runner, write_json};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, env, fs, path::Path, process::Command};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    profiles: Vec<Profile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    name: String,
    package: String,
    target: String,
    tests: bool,
    features: Vec<String>,
    default_features: bool,
    #[serde(default)]
    cfgs: Vec<String>,
    #[serde(default)]
    rlimit: Option<u32>,
}

pub fn run(config: &Path, manifest: &Path, sdk: &Path, out: &Path) -> Result<()> {
    let config = fs::canonicalize(config)?;
    let manifest = fs::canonicalize(manifest)?;
    let sdk = fs::canonicalize(sdk)?;
    let plan: Config = serde_json::from_slice(&fs::read(&config)?)?;
    let mut names = std::collections::BTreeSet::new();
    if plan.profiles.is_empty()
        || plan.profiles.iter().any(|p| {
            p.name.is_empty()
                || !p
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
                || !names.insert(&p.name)
        })
    {
        return Err("profile names must be nonempty, unique and path-safe".into());
    }
    for profile in &plan.profiles {
        capture::configured_args(&[], &profile.cfgs, true)?;
    }
    fs::create_dir(out)?;
    let out = fs::canonicalize(out)?;
    let metadata = execute(
        Command::new("cargo")
            .args([
                "metadata",
                "--offline",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(&manifest),
        &out,
        "metadata",
    )?;
    if !metadata.status.success() {
        return Err("Cargo metadata failed; see metadata.stderr".into());
    }
    let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
    let workspace = Path::new(
        metadata["workspace_root"]
            .as_str()
            .ok_or("workspace missing")?,
    );
    if out.starts_with(workspace) {
        return Err("capture output must be outside the source workspace".into());
    }
    let driver = sdk
        .parent()
        .and_then(Path::parent)
        .ok_or("SDK source missing")?
        .join("rust_verify/src/driver.rs");
    let driver_text = fs::read_to_string(&driver)?;
    if !driver_text
        .contains("rustc_args.extend([\"--cfg\", \"verus_only\", \"--cfg\", \"verus_keep_ghost\"]")
        || !driver_text.contains("rustc_args_verify.extend([\"--cfg\", \"verus_keep_ghost_body\"]")
    {
        return Err("unsupported Verus driver configuration; inspect its compilation and verification phases".into());
    }
    let mut sources = BTreeMap::new();
    let mut source_roots = std::collections::BTreeSet::new();
    for package in metadata["packages"].as_array().ok_or("packages missing")? {
        let path = Path::new(
            package["manifest_path"]
                .as_str()
                .ok_or("package manifest missing")?,
        );
        let directory = path.parent().ok_or("package directory missing")?;
        source_roots.insert(directory.to_path_buf());
        runner::source_hashes(directory, &mut sources)?;
    }
    sources.insert(config.clone(), hash(&fs::read(&config)?)?);
    sources.insert(driver.clone(), hash(driver_text.as_bytes())?);
    sources.insert(manifest.clone(), hash(&fs::read(&manifest)?)?);
    sources.insert(
        workspace.join("Cargo.toml"),
        hash(&fs::read(workspace.join("Cargo.toml"))?)?,
    );
    let mut absent_inputs = Vec::new();
    for path in [
        workspace.join("Cargo.lock"),
        workspace.join(".cargo/config.toml"),
        workspace.join(".cargo/config"),
    ] {
        if path.is_file() {
            sources.insert(path.clone(), hash(&fs::read(path)?)?);
        } else {
            absent_inputs.push(path);
        }
    }
    if workspace.join(".cargo").is_dir() {
        source_roots.insert(workspace.join(".cargo"));
    }
    write_json(&out.join("absent-inputs.json"), &absent_inputs)?;
    let captured_tool = out.join("capture-tool");
    fs::copy(env::current_exe()?, &captured_tool)?;
    for file in [
        captured_tool.clone(),
        sdk.join("cargo-verus"),
        sdk.join("rust_verify"),
        sdk.join("verus"),
    ] {
        sources.insert(file.clone(), hash(&fs::read(file)?)?);
    }
    write_json(&out.join("inputs.json"), &sources)?;
    write_json(&out.join("source-roots.json"), &source_roots)?;
    fs::copy(&config, out.join("configuration.json"))?;
    let mut reports = Vec::new();
    let verification_summary =
        regex::Regex::new(r"verification results:: \d+ verified, (\d+) errors")?;
    for profile in plan.profiles {
        let case = out.join(&profile.name);
        fs::create_dir(&case)?;
        let captures = case.join("capture");
        fs::create_dir(&captures)?;
        let parent = env::vars()
            .map(|(k, v)| Ok((k, hash(v.as_bytes())?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        write_json(&captures.join("parent-env.json"), &parent)?;
        let cargo = env::var("CARGO").unwrap_or("cargo".into());
        let mut command = Command::new(sdk.join("cargo-verus"));
        command
            .current_dir(workspace)
            .args(["build", "--offline", "--locked", "--manifest-path"])
            .arg(&manifest)
            .args(["-p", &profile.package, "--target-dir"])
            .arg(case.join("cargo-target"))
            .env("CARGO", &captured_tool)
            .env("SPEC_CHECK_CARGO", cargo)
            .env("SPEC_CHECK_PACKAGE", &profile.package)
            .env("SPEC_CHECK_CFGS", serde_json::to_string(&profile.cfgs)?)
            .env(capture::DIR, &captures)
            .env("SPEC_CHECK_CAPTURE_ALL", "1")
            .env("SPEC_CHECK_PROJECT_LOG", "1")
            .env("RUSTC_BOOTSTRAP", "1")
            .env("VERUS_USE_RUSTUP", "0")
            .env(
                "VERUS_Z3_PATH",
                sdk.parent()
                    .and_then(Path::parent)
                    .ok_or("SDK source missing")?
                    .join("z3"),
            );
        if !profile.default_features {
            command.arg("--no-default-features");
        }
        if !profile.features.is_empty() {
            command.args(["--features", &profile.features.join(",")]);
        }
        command
            .args(["--target", &profile.target])
            .arg(if profile.tests { "--tests" } else { "--lib" })
            .arg("--message-format=json");
        if let Some(limit) = profile.rlimit {
            if limit == 0 {
                return Err("rlimit must be positive".into());
            }
            command.args(["--", "--rlimit", &limit.to_string()]);
        }
        let build = execute(&mut command, &case, "build")?;
        let mut artifacts = Vec::new();
        for line in String::from_utf8_lossy(&build.stdout).lines() {
            if let Ok(item) = serde_json::from_str::<Value>(line)
                && item["reason"] == "compiler-artifact"
            {
                for name in item["filenames"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    let path = std::path::PathBuf::from(name);
                    if path.is_file() {
                        sources.insert(path.clone(), hash(&fs::read(path)?)?);
                    }
                }
                artifacts.push(item);
            }
        }
        write_json(&case.join("artifacts.json"), &artifacts)?;
        if !build.status.success() {
            let output = format!(
                "{}{}",
                String::from_utf8_lossy(&build.stdout),
                String::from_utf8_lossy(&build.stderr)
            );
            let verification_failed = verification_summary
                .captures_iter(&output)
                .any(|c| c[1].parse::<u64>().is_ok_and(|n| n > 0));
            reports.push(json!({"profile":profile.name,"package":profile.package,"target":profile.target,
                "build_success":false,"build_exit_code":build.status.code(),"build_obstacles":crate::resource_project::compiler_obstacles(&output),"build_stdout":case.join("build.stdout"),"build_failure_kind":if verification_failed{"verification_failed"}else{"cargo_build_failed"},"units":[],"registry":[],
                "registry_correspondence":false,"runtime":"not executed","evidence":case.join("build.stderr")}));
            continue;
        }
        let mut registry = Vec::new();
        let selected = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == profile.package)
            .ok_or("profile package missing")?;
        for artifact in &artifacts {
            if artifact["package_id"] == selected["id"]
                && artifact["profile"]["test"] == true
                && let Some(exe) = artifact["executable"].as_str()
            {
                let list = execute(
                    Command::new(exe).args(["--list", "--format", "terse"]),
                    &case,
                    &format!("registry-{}", registry.len()),
                )?;
                registry.push(json!({"executable":exe,"exit_code":list.status.code(),"tests":String::from_utf8_lossy(&list.stdout).lines().filter_map(|s|s.strip_suffix(": test")).collect::<Vec<_>>() }));
            }
        }
        write_json(&case.join("registry.json"), &registry)?;
        let mut invocations = Vec::new();
        for entry in fs::read_dir(&captures)? {
            let path = entry?.path();
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("invocation-")
            {
                invocations.push(serde_json::from_slice::<capture::Invocation>(&fs::read(
                    path,
                )?)?);
            }
        }
        let mut units = Vec::new();
        let mut compiler_tests = Vec::new();
        for (i, inv) in invocations.iter().enumerate() {
            let unit = case.join(format!("unit-{i}"));
            fs::create_dir(&unit)?;
            write_json(&unit.join("context.json"), inv)?;
            let mut imports = Vec::new();
            for pair in inv.args.windows(2).filter(|p| p[0] == "--extern") {
                if let Some((name, path)) = pair[1].split_once('=') {
                    let artifact = inv.cwd.join(path);
                    for input in [artifact.clone(), artifact.with_extension("vir")] {
                        if input.is_file() {
                            let digest = hash(&fs::read(&input)?)?;
                            sources.insert(input.clone(), digest.clone());
                            imports.push(json!({"crate":name,"path":input,"sha256":digest}));
                        }
                    }
                }
            }
            write_json(&unit.join("dependency-inputs.json"), &imports)?;

            let crate_name = inv
                .args
                .windows(2)
                .find(|v| v[0] == "--crate-name")
                .ok_or("captured crate name missing")?[1]
                .clone();
            let suffix = inv
                .args
                .windows(2)
                .find(|v| v[0] == "-C" && v[1].starts_with("extra-filename="))
                .map(|v| v[1].trim_start_matches("extra-filename="))
                .unwrap_or("");
            let directory = &inv
                .args
                .windows(2)
                .find(|v| v[0] == "--out-dir")
                .ok_or("captured output directory missing")?[1];
            let depinfo = inv
                .cwd
                .join(directory)
                .join(format!("{crate_name}{suffix}.d"));
            let dependencies = runner::depfiles(&fs::read_to_string(&depinfo)?, &inv.cwd)?;
            let mut identities = Vec::new();
            for path in dependencies {
                let canonical = fs::canonicalize(&path)?;
                let digest = hash(&fs::read(&path)?)?;
                identities
                    .push(json!({"compiler_path":path,"canonical_path":canonical,"sha256":digest}));
                sources.insert(path, digest);
            }
            sources.insert(depinfo.clone(), hash(&fs::read(&depinfo)?)?);
            write_json(
                &unit.join("source-identity.json"),
                &json!({"dep_info":depinfo,"inputs":identities}),
            )?;

            if !inv.unsupported_env.is_empty() {
                units.push(json!({"status":"unsupported_environment","variables":inv.unsupported_env,"evidence":format!("unit-{i}")}));
                continue;
            }
            let flags = inv
                .env
                .iter()
                .find(|(k, _)| k.trim() == "__VERUS_DRIVER_ARGS__")
                .ok_or("native driver arguments missing")?
                .1;
            let parts = flags.split("__VERUS_DRIVER_ARGS_SEP__").collect::<Vec<_>>();
            let logs = parts
                .windows(2)
                .find(|v| v[0] == "--log-dir")
                .ok_or("native VIR directory missing")?[1];
            let vir_file = Path::new(logs).join("crate.vir");
            if !vir_file.is_file() || fs::metadata(&vir_file)?.len() == 0 {
                units.push(json!({"status":"native_verification_input_missing","evidence":format!("unit-{i}"),"expected":vir_file}));
                continue;
            }
            sources.insert(vir_file.clone(), hash(&fs::read(&vir_file)?)?);
            write_json(
                &unit.join("verification-input.json"),
                &json!({"source":"Actual Verus driver from the captured Cargo invocation","vir":vir_file,"sha256":sources[&vir_file],"cfg":["verus_only","verus_keep_ghost","verus_keep_ghost_body"],"driver_source":driver}),
            )?;
            let Some(probe) = runner::build_probe(inv, &unit)? else {
                units.push(json!({"status":"probe_build_failed","evidence":format!("unit-{i}")}));
                continue;
            };
            let mut replay = inv.clone();
            replay
                .env
                .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
            {
                let mode = "compile";
                let location = unit.join(mode);
                fs::create_dir(&location)?;
                let args =
                    runner::clean(&inv.args, &location.join("compiler-output"), None, &inv.cwd)?;
                let result = runner::extract_context(
                    &probe,
                    &replay,
                    &args,
                    "*",
                    &location,
                    "inventory",
                    true,
                );
                let stderr = fs::read_to_string(location.join("inventory.stderr"))?;
                let parsed = inventory(&stderr);
                let counts = parsed.as_ref().ok().map(|p| p["counts"].clone());
                if mode == "compile"
                    && let Ok(parsed) = &parsed
                {
                    compiler_tests.extend(
                        parsed["records"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|r| r["kind"] == "test")
                            .map(|r| r["name"].as_str().unwrap().to_owned()),
                    );
                }
                let status = if result.is_ok() && parsed.is_ok() {
                    "captured"
                } else {
                    "inventory_failed"
                };
                let error = result
                    .err()
                    .map(|e| e.to_string())
                    .or_else(|| parsed.as_ref().err().map(|e| e.to_string()));
                if let Ok(parsed) = parsed {
                    write_json(&location.join("inventory-records.json"), &parsed)?;
                }
                units.push(json!({"status":status,"error":error,"mode":mode,"evidence":format!("unit-{i}/{mode}"),"counts":counts,"test_harness":inv.args.iter().any(|s|s=="--test"),"driver_configuration":{"source":driver,"base_cfg":["verus_only","verus_keep_ghost"],"keep_ghost_body":false},"inventory_stage":"macro expansion and typeck, before borrow checking; native Cargo build checked separately"}));
            }
        }
        let frequencies = |names: Vec<String>| {
            let mut counts = BTreeMap::<String, usize>::new();
            for name in names {
                *counts.entry(name).or_default() += 1;
            }
            counts
        };
        let registered = registry
            .iter()
            .flat_map(|r| {
                r["tests"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap().to_owned())
            })
            .collect::<Vec<_>>();
        let registry_correspondence = frequencies(registered) == frequencies(compiler_tests)
            && registry.iter().all(|r| r["exit_code"] == 0);
        reports.push(
            json!({"profile":profile.name,"package":profile.package,"target":profile.target,
            "build_success":build.status.success(),"registry_correspondence":registry_correspondence,"units":units,"registry":registry,"runtime":"not executed",
            "ktest":"no runtime execution; registration requires its actual build configuration"}),
        );
    }
    let mut changed = Vec::new();
    for (path, expected) in &sources {
        if fs::read(path).ok().and_then(|b| hash(&b).ok()).as_ref() != Some(expected) {
            changed.push(path.clone());
        }
    }
    for directory in &source_roots {
        let mut current = BTreeMap::new();
        runner::source_hashes(directory, &mut current)?;
        for path in current.keys() {
            if !sources.contains_key(path) {
                changed.push(path.clone());
            }
        }
    }
    write_json(&out.join("inputs.json"), &sources)?;
    changed.extend(absent_inputs.iter().filter(|p| p.exists()).cloned());
    let complete = changed.is_empty()
        && reports.iter().all(|r| {
            r["build_success"] == true
                && r["registry_correspondence"] == true
                && r["units"]
                    .as_array()
                    .is_some_and(|u| !u.is_empty() && u.iter().all(|v| v["status"] == "captured"))
        });
    write_json(
        &out.join("report.json"),
        &json!({"status":if complete{"captured"}else{"incomplete"},"profiles":reports,"changed_inputs":changed,"per_client_configuration":0,"configuration_sha256":hash(&fs::read(&config)?)?}),
    )?;
    if !complete {
        return Err("project capture incomplete; see report.json and profile diagnostics".into());
    }
    Ok(())
}

pub(crate) fn inventory(text: &str) -> Result<Value> {
    let mut rows = Vec::new();
    let mut owners = std::collections::BTreeSet::new();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut completed = None;
    for line in text.lines().filter(|s| s.starts_with("PROJECT_")) {
        if let Some(count) = line.strip_prefix("PROJECT_COMPLETE\t") {
            if completed.replace(count.parse::<usize>()?).is_some() {
                return Err("duplicate inventory completion marker".into());
            }
            continue;
        }
        let mut values = line.split('\t');
        let kind = values
            .next()
            .unwrap()
            .trim_start_matches("PROJECT_")
            .to_lowercase();
        let names: &[&str] = match kind.as_str() {
            "owner" => &["owner", "path", "definition_kind", "span", "attributes"],
            "call" => &["owner", "hir_id", "span", "call_kind", "target"],
            "assert" => &["owner", "span", "macro"],
            "control" => &["owner", "expression_kind", "span"],
            "test" => &["owner", "name", "span"],
            "mir_body" => &["owner", "blocks"],
            "mir_local" => &["owner", "place", "type", "interface_type", "datatype_paths"],
            "mir_term" => &["owner", "block", "span", "statement_count", "operation"],
            "mir_stmt" => &["owner", "block", "index", "span", "operation"],
            "mir_call" => &[
                "owner",
                "block",
                "span",
                "target",
                "target_id",
                "definition_span",
                "generic_args",
                "arguments",
                "destination",
            ],
            "mir_drop" => &["owner", "block", "span", "place", "type"],
            "mir_unavailable" => &["owner", "reason"],
            "mir_unsupported" => &["owner", "block", "span", "reason"],
            "file" => &["file"],
            "excluded" => &["owner", "reason"],
            _ => return Err(format!("unknown inventory record {kind}").into()),
        };
        let values = values.collect::<Vec<_>>();
        if names.len() != values.len() {
            return Err(format!("malformed inventory record {kind}").into());
        }
        let mut record = BTreeMap::from([("kind".to_owned(), kind.clone())]);
        record.extend(
            names
                .iter()
                .zip(values)
                .map(|(k, v)| (k.to_string(), v.to_owned())),
        );
        if kind == "owner" && !owners.insert(record["owner"].clone()) {
            return Err("duplicate compiler owner identity".into());
        }
        *counts.entry(kind).or_default() += 1;
        rows.push(record);
    }
    if completed.is_some_and(|n| n != owners.len())
        || (owners.is_empty()
            && (completed != Some(0) || !rows.iter().any(|r| r["kind"] == "file")))
    {
        return Err("inventory lacks complete compiler body enumeration".into());
    }
    for row in &rows {
        if row.contains_key("owner") && !owners.contains(&row["owner"]) {
            return Err("record references an absent owner".into());
        }
    }
    let mut locals = std::collections::BTreeSet::new();
    for row in rows.iter().filter(|r| r["kind"] == "mir_local") {
        if !locals.insert((&row["owner"], &row["place"])) {
            return Err("duplicate compiler local identity".into());
        }
    }
    let wanted = rows
        .iter()
        .filter(|r| {
            r["kind"] == "owner"
                && ["Fn", "AssocFn", "Closure"].contains(&r["definition_kind"].as_str())
        })
        .map(|r| r["owner"].clone())
        .collect::<std::collections::BTreeSet<_>>();
    let present = rows
        .iter()
        .filter(|r| ["mir_body", "mir_unavailable"].contains(&r["kind"].as_str()))
        .map(|r| r["owner"].clone())
        .collect::<std::collections::BTreeSet<_>>();
    if wanted != present {
        return Err("MIR coverage differs from compiler function and closure owners".into());
    }
    let mut block_ids = BTreeMap::<&str, std::collections::BTreeMap<&str, usize>>::new();
    for term in rows.iter().filter(|r| r["kind"] == "mir_term") {
        block_ids
            .entry(&term["owner"])
            .or_default()
            .insert(&term["block"], term["statement_count"].parse::<usize>()?);
    }
    for body in rows.iter().filter(|r| r["kind"] == "mir_body") {
        if block_ids.get(body["owner"].as_str()).map(|ids| ids.len())
            != Some(body["blocks"].parse::<usize>()?)
        {
            return Err("MIR block inventory incomplete".into());
        }
    }
    let mut block_statement_ids =
        BTreeMap::<(&str, &str), std::collections::BTreeSet<usize>>::new();
    let mut statement_ids = std::collections::BTreeSet::new();
    for statement in rows.iter().filter(|r| r["kind"] == "mir_stmt") {
        let key = (statement["owner"].as_str(), statement["block"].as_str());
        let Some(expected) = block_ids.get(key.0).and_then(|blocks| blocks.get(key.1)) else {
            return Err("MIR statement references an absent basic block".into());
        };
        let index = statement["index"].parse::<usize>()?;
        if !statement_ids.insert((key.0, key.1, index)) {
            return Err("duplicate MIR statement identity".into());
        }
        if index >= *expected {
            return Err("MIR statement index exceeds block statement count".into());
        }
        block_statement_ids.entry(key).or_default().insert(index);
    }
    for (owner, blocks) in block_ids {
        for (block, expected) in blocks {
            let found = block_statement_ids
                .get(&(owner, block))
                .map(|indices| indices.len())
                .unwrap_or(0);
            if found != expected {
                return Err("MIR statement inventory incomplete".into());
            }
            if (0..expected).any(|index| {
                !block_statement_ids
                    .get(&(owner, block))
                    .is_some_and(|indices| indices.contains(&index))
            }) {
                return Err("MIR statement indices are not contiguous".into());
            }
        }
    }
    Ok(
        json!({"records":rows,"counts":counts,"identity":"compiler local definition ID, scoped to the captured compilation; paths may repeat"}),
    )
}

pub(crate) fn validate_inputs(capture: &Path, config: &Path) -> Result<()> {
    let report: Value = serde_json::from_slice(&fs::read(capture.join("report.json"))?)?;
    if report["configuration_sha256"] != hash(&fs::read(config)?)? {
        return Err("project configuration changed".into());
    }
    let absent: Vec<std::path::PathBuf> =
        serde_json::from_slice(&fs::read(capture.join("absent-inputs.json"))?)?;
    if absent.iter().any(|p| p.exists()) {
        return Err("previously absent build configuration appeared".into());
    }
    let expected: BTreeMap<std::path::PathBuf, String> =
        serde_json::from_slice(&fs::read(capture.join("inputs.json"))?)?;
    for (path, digest) in expected {
        if fs::read(&path).ok().and_then(|b| hash(&b).ok()).as_ref() != Some(&digest) {
            return Err(format!("captured input changed or missing: {}", path.display()).into());
        }
    }
    let roots: Vec<std::path::PathBuf> =
        serde_json::from_slice(&fs::read(capture.join("source-roots.json"))?)?;
    let expected: BTreeMap<std::path::PathBuf, String> =
        serde_json::from_slice(&fs::read(capture.join("inputs.json"))?)?;
    for root in roots {
        let mut current = BTreeMap::new();
        runner::source_hashes(&root, &mut current)?;
        if current.keys().any(|path| !expected.contains_key(path)) {
            return Err(format!("new source input under {}", root.display()).into());
        }
    }
    Ok(())
}
pub fn validate(capture: &Path, config: &Path) -> Result<()> {
    validate_inputs(capture, config)?;
    let report: Value = serde_json::from_slice(&fs::read(capture.join("report.json"))?)?;
    if report["status"] != "captured" {
        return Err("capture was not complete".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    #[ignore = "requires PROJECT_TYPE_CONTEXT and fresh PROJECT_TYPE_OUT"]
    fn real_typed_local_inventory() {
        let inv: capture::Invocation = serde_json::from_slice(
            &fs::read(env::var_os("PROJECT_TYPE_CONTEXT").unwrap()).unwrap(),
        )
        .unwrap();
        let out = PathBuf::from(env::var_os("PROJECT_TYPE_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let probe = runner::build_probe(&inv, &out).unwrap().unwrap();
        let args = runner::clean(&inv.args, &out.join("metadata"), None, &inv.cwd).unwrap();
        let mut context = inv.clone();
        context
            .env
            .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
        runner::extract_context(&probe, &context, &args, "*", &out, "inventory", true).unwrap();
        let report = inventory(&fs::read_to_string(out.join("inventory.stderr")).unwrap()).unwrap();
        assert!(report["counts"]["mir_local"].as_u64().unwrap() > 0);
        let records = report["records"].as_array().unwrap();
        let bodies = records
            .iter()
            .filter(|r| r["kind"] == "mir_body")
            .map(|r| r["owner"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        let returns = records
            .iter()
            .filter(|r| r["kind"] == "mir_local" && r["place"] == "_0")
            .map(|r| r["owner"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(bodies, returns);
        write_json(&out.join("inventory-records.json"), &report).unwrap();
    }
    #[test]
    fn compiler_ids_keep_same_named_owners_distinct() {
        let text = "PROJECT_OWNER\t1\tf\tFn\tfile:1\t[]\nPROJECT_OWNER\t2\tf\tConst\tfile:1\t[]\nPROJECT_EXCLUDED\t2\tconstant\nPROJECT_CALL\t1\texpr\tfile:2\tcall\tcallee\nPROJECT_MIR_BODY\t1\t1\nPROJECT_MIR_TERM\t1\tbb0\tfile:2\t0\treturn";
        assert_eq!(inventory(text).unwrap()["counts"]["owner"], 2);
        assert!(inventory(&text.replace("PROJECT_OWNER\t2", "PROJECT_OWNER\t1")).is_err());
        assert!(inventory(&text.replace("PROJECT_CALL\t1", "PROJECT_CALL\t3")).is_err());
        assert!(inventory("PROJECT_NEW\tunknown").is_err());
        let local = "\nPROJECT_MIR_LOCAL\t1\t_0\t()\ttuple[]\t";
        assert_eq!(
            inventory(&(text.to_owned() + local)).unwrap()["counts"]["mir_local"],
            1
        );
        assert!(inventory(&(text.to_owned() + local + local)).is_err());
        let empty = "PROJECT_FILE\tempty.rs\nPROJECT_COMPLETE\t0";
        assert_eq!(
            inventory(empty).unwrap()["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(inventory("PROJECT_FILE\tempty.rs").is_err());
        assert!(inventory("PROJECT_COMPLETE\t0").is_err());
        assert!(inventory(&(empty.to_owned() + "\nPROJECT_COMPLETE\t0")).is_err());
        assert!(inventory(&(text.to_owned() + "\nPROJECT_COMPLETE\t1")).is_err());
    }

    #[test]
    fn mir_statement_records_are_scoped_to_real_blocks_and_have_unique_indices() {
        let text = "PROJECT_OWNER\t1\tf\tFn\tfile:1\t[]\nPROJECT_MIR_BODY\t1\t1\nPROJECT_MIR_STMT\t1\tbb0\t0\tfile:1\tAssign(_4, Discriminant(_3))\nPROJECT_MIR_TERM\t1\tbb0\tfile:1\t1\treturn";
        assert_eq!(inventory(text).unwrap()["counts"]["mir_stmt"], 1);
        assert!(
            inventory(&(text.to_owned() + "\nPROJECT_MIR_STMT\t1\tbb1\t0\tfile:1\tNop")).is_err()
        );
        assert!(
            inventory(&(text.to_owned() + "\nPROJECT_MIR_STMT\t1\tbb0\t0\tfile:1\tNop")).is_err()
        );
        assert!(
            inventory(&text.replace(
                "PROJECT_MIR_STMT\t1\tbb0\t0\tfile:1\tAssign(_4, Discriminant(_3))\n",
                ""
            ))
            .is_err()
        );
    }

    #[test]
    fn changed_configuration_modified_and_new_sources_are_rejected() {
        let dir = env::temp_dir().join(format!(
            "project-inputs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let src = dir.join("source");
        fs::create_dir(&src).unwrap();
        let file = src.join("lib.rs");
        fs::write(&file, "original").unwrap();
        let config = dir.join("config.json");
        fs::write(&config, "features=[]").unwrap();
        write_json(&dir.join("report.json"),&json!({"status":"captured","configuration_sha256":hash(&fs::read(&config).unwrap()).unwrap()})).unwrap();
        write_json(
            &dir.join("inputs.json"),
            &BTreeMap::from([(file.clone(), hash(b"original").unwrap())]),
        )
        .unwrap();
        write_json(&dir.join("source-roots.json"), &vec![src.clone()]).unwrap();
        write_json(&dir.join("absent-inputs.json"), &Vec::<String>::new()).unwrap();
        validate(&dir, &config).unwrap();
        fs::write(&config, "features=[extra]").unwrap();
        assert!(validate(&dir, &config).is_err());
        fs::write(&config, "features=[]").unwrap();
        fs::write(&file, "changed").unwrap();
        assert!(validate(&dir, &config).is_err());
        fs::write(&file, "original").unwrap();
        fs::write(src.join("added.rs"), "new").unwrap();
        assert!(validate(&dir, &config).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
