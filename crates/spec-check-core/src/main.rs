mod assertion_rule;
mod attribution;
mod borrowed_resource;
mod call_guidance;
mod callback;
mod capture;
mod census;
mod client;
mod conditional;
mod conditional_check;
mod conditional_project;
mod contract_scan;
mod contracts;
mod demand;
mod demand_check;
mod evidence;
mod filter_guidance;
mod finite;
mod ghost_abi;
mod guard_owner;
mod guidance;
mod handoff;
mod helper;
mod ir;
mod link;
mod literal;
mod obligation;
mod option_alternative;
mod ordinary;
mod owner_post;
mod owner_report_enrich;
mod permission_guidance;
mod prefix;
mod project;
mod reader;
mod resource;
mod resource_flow;
mod resource_project;
mod return_forwarding;
mod review;
mod runner;
mod scope;
mod set_state;
mod state;
mod state_semantics;
mod std_body;
mod test_census;
mod vir;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn hash(bytes: &[u8]) -> Result<String> {
    let mut p = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    p.stdin
        .take()
        .ok_or("missing hash stdin")?
        .write_all(bytes)?;
    let r = p.wait_with_output()?;
    if !r.status.success() {
        return Err("sha256sum failed".into());
    }
    Ok(String::from_utf8(r.stdout)?[..64].to_owned())
}
fn execute(command: &mut Command, out: &Path, name: &str) -> Result<std::process::Output> {
    write_json(
        &out.join(format!("{name}.command.json")),
        &json!({"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|s|s.to_string_lossy()).collect::<Vec<_>>(),"cwd":command.get_current_dir()}),
    )?;
    let r = command.output()?;
    fs::write(out.join(format!("{name}.stdout")), &r.stdout)?;
    fs::write(out.join(format!("{name}.stderr")), &r.stderr)?;
    Ok(r)
}
fn check() -> Result<()> {
    let binary = env::current_exe()?;
    let binary_hash = hash(&fs::read(&binary)?)?;
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    let mut options = BTreeMap::new();
    if args.first().is_some_and(|s| s == "spec-check") {
        args.remove(0);
    }
    if args.first().is_some_and(|s| s == "option-alternatives") {
        let keys = ["--input", "--context", "--verus-root", "--out"];
        if args.len() != 9 || keys.iter().enumerate().any(|(i, k)| args[1 + i * 2] != *k) {
            return Err("option-alternatives --input DIRECTORY --context FILE --verus-root SDK --out NEW_DIRECTORY".into());
        }
        return option_alternative::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
        );
    }
    if args.first().is_some_and(|s| s == "demand-check") {
        if args.len() != 7
            || args[1] != "--config"
            || args[3] != "--verus-root"
            || args[5] != "--out"
        {
            return Err(
                "demand-check --config FILE --verus-root DIRECTORY --out NEW_DIRECTORY".into(),
            );
        }
        return demand_check::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "demand-census") {
        if args.len() != 5 || args[1] != "--vir" || args[3] != "--out" {
            return Err("demand-census --vir VIR --out NEW_DIRECTORY".into());
        }
        return demand::run(Path::new(&args[2]), Path::new(&args[4]));
    }
    if args.first().is_some_and(|s| s == "conditional-check") {
        if args.len() != 7
            || args[1] != "--config"
            || args[3] != "--verus-root"
            || args[5] != "--out"
        {
            return Err(
                "conditional-check --config FILE --verus-root SDK --out NEW_DIRECTORY".into(),
            );
        }
        return conditional_check::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "conditional-project") {
        if args.len() != 9
            || args[1] != "--config"
            || args[3] != "--manifest-path"
            || args[5] != "--verus-root"
            || args[7] != "--out"
        {
            return Err("conditional-project --config JSON --manifest-path TOML --verus-root SDK --out NEW_DIRECTORY".into());
        }
        return conditional_project::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
        );
    }
    if args.first().is_some_and(|s| s == "conditional-generate") {
        if args.len() != 7 || args[1] != "--vir" || args[3] != "--source-root" || args[5] != "--out"
        {
            return Err(
                "conditional-generate --vir VIR --source-root DIRECTORY --out NEW_DIRECTORY".into(),
            );
        }
        return conditional::generate(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "conditional-scan") {
        if args.len() != 5 || args[1] != "--vir" || args[3] != "--out" {
            return Err("conditional-scan --vir VIR --out NEW_DIRECTORY".into());
        }
        return conditional::run(Path::new(&args[2]), Path::new(&args[4]));
    }
    if args.first().is_some_and(|s| s == "census") {
        let usage = "census --vir FILE --crate NAME --out DIRECTORY [--source-root DIRECTORY]";
        let mut values = BTreeMap::new();
        let mut iter = args.iter().skip(1);
        while let Some(key) = iter.next() {
            if !matches!(
                key.as_str(),
                "--vir" | "--crate" | "--out" | "--source-root"
            ) {
                return Err(usage.into());
            }
            let value = iter.next().ok_or(usage)?.clone();
            if values.insert(key.clone(), value).is_some() {
                return Err(usage.into());
            }
        }
        let get = |k: &str| values.get(k).map(String::as_str).ok_or("missing option");
        let report = census::run(
            Path::new(get("--vir")?),
            get("--crate")?,
            values.get("--source-root").map(Path::new),
            Path::new(get("--out")?),
        )?;
        let counts = &report["counts"];
        println!(
            "census: {} functions ({} exec, {} proof), {} with postcondition ({} outputs, {} findings), {} without ({} outputs, {} findings)",
            counts["functions"],
            counts["exec"],
            counts["proof"],
            counts["with_postcondition"]["functions"],
            counts["with_postcondition"]["outputs"],
            counts["with_postcondition"]["findings"],
            counts["without_postcondition"]["functions"],
            counts["without_postcondition"]["outputs"],
            counts["without_postcondition"]["findings"],
        );
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "contract-scan") {
        if args.len() != 5 || args[1] != "--vir" || args[3] != "--out" {
            return Err("contract-scan --vir VIR --out NEW_DIRECTORY".into());
        }
        return contract_scan::run(Path::new(&args[2]), Path::new(&args[4]));
    }
    if args.first().is_some_and(|s| s == "guard-owner-scan") {
        if args.len() != 5 || args[1] != "--vir" || args[3] != "--out" {
            return Err("guard-owner-scan --vir VIR --out NEW_DIRECTORY".into());
        }
        return guard_owner::run(Path::new(&args[2]), Path::new(&args[4]));
    }
    if args.first().is_some_and(|s| s == "owner-closure-scan") {
        if args.len() != 7
            || args[1] != "--vir"
            || args[3] != "--owner-report"
            || args[5] != "--out"
        {
            return Err(
                "owner-closure-scan --vir VIR --owner-report REPORT --out NEW_DIRECTORY".into(),
            );
        }
        return guard_owner::closure_forwarder_scan(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "owner-report-enrich") {
        if args.len() != 7 || args[1] != "--vir" || args[3] != "--report" || args[5] != "--out" {
            return Err(
                "owner-report-enrich --vir VIR --report SCAN_REPORT --out NEW_DIRECTORY".into(),
            );
        }
        return owner_report_enrich::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "scope-validate") {
        if args.len() != 3 || args[1] != "--input" {
            return Err("scope-validate --input DIRECTORY".into());
        }
        return scope::validate(Path::new(&args[2]));
    }
    if args.first().is_some_and(|s| s == "scope-check") {
        let keys = ["--config", "--manifest-path", "--verus-root", "--out"];
        if !matches!(args.len(), 9 | 11)
            || keys.iter().enumerate().any(|(i, k)| args[1 + i * 2] != *k)
            || (args.len() == 11 && args[9] != "--capture")
        {
            return Err("scope-check --config JSON --manifest-path TOML --verus-root SDK --out NEW_DIRECTORY [--capture DIRECTORY]".into());
        }
        return scope::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
            args.get(10).map(Path::new),
        );
    }
    if args.first().is_some_and(|s| s == "resource-project-check") {
        if args.len() != 7
            || args[1] != "--config"
            || args[3] != "--verus-root"
            || args[5] != "--out"
        {
            return Err(
                "resource-project-check --config JSON --verus-root SDK --out NEW_DIRECTORY".into(),
            );
        }
        return resource_project::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "ghost-abi-project") {
        let keys = ["--hir", "--vir", "--moves", "--cwd", "--out"];
        if args.len() != 11 || keys.iter().enumerate().any(|(i, k)| args[1 + i * 2] != *k) {
            return Err("ghost-abi-project --hir JSON --vir VIR --moves JSON --cwd PATH --out NEW_DIRECTORY".into());
        }
        return ghost_abi::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
            Path::new(&args[10]),
        );
    }
    if args.first().is_some_and(|s| s == "borrowed-resource-plan") {
        let keys = ["--records", "--hir", "--vir", "--cwd", "--out"];
        if !matches!(args.len(), 11 | 13)
            || keys.iter().enumerate().any(|(i, k)| args[1 + i * 2] != *k)
            || (args.len() == 13 && args[11] != "--after")
        {
            return Err("borrowed-resource-plan --records RAW --hir JSON --vir VIR --cwd PATH --out NEW_DIRECTORY [--after RAW]".into());
        }
        return borrowed_resource::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
            Path::new(&args[10]),
            args.get(12).map(Path::new),
        );
    }
    if args.first().is_some_and(|s| s == "resource-connect") {
        let keys = [
            "--context",
            "--source",
            "--function",
            "--verus-root",
            "--out",
        ];
        if args.len() != 11
            || keys
                .iter()
                .enumerate()
                .any(|(i, key)| args[1 + i * 2] != *key)
        {
            return Err("resource-connect --context JSON --source RS --function NAME --verus-root SDK --out NEW_DIRECTORY".into());
        }
        return resource::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            &args[6],
            Path::new(&args[8]),
            Path::new(&args[10]),
        );
    }
    if args.first().is_some_and(|s| s == "test-isolate") {
        if !matches!(args.len(), 5 | 7)
            || args[1] != "--census"
            || args[3] != "--out"
            || (args.len() == 7 && args[5] != "--verus-root")
        {
            return Err(
                "test-isolate --census DIRECTORY --out NEW_DIRECTORY [--verus-root SDK]".into(),
            );
        }
        return test_census::isolate(
            Path::new(&args[2]),
            Path::new(&args[4]),
            args.get(6).map(Path::new),
        );
    }
    if args.first().is_some_and(|s| s == "test-census") {
        if !matches!(args.len(), 7 | 8)
            || args[1] != "--context"
            || args[3] != "--source"
            || args[5] != "--out"
            || (args.len() == 8 && args[7] != "--functions")
        {
            return Err(
                "test-census --context JSON --source RS --out NEW_DIRECTORY [--functions]".into(),
            );
        }
        return test_census::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            args.len() == 8,
        );
    }
    if args.first().is_some_and(|s| s == "project-census") {
        if args.len() != 7
            || args[1] != "--links"
            || args[3] != "--verus-root"
            || args[5] != "--out"
        {
            return Err(
                "project-census --links DIRECTORY --verus-root SDK --out NEW_DIRECTORY".into(),
            );
        }
        return client::census(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "project-check") {
        if !matches!(args.len(), 7 | 9)
            || args[1] != "--links"
            || args[3] != "--verus-root"
            || args[5] != "--out"
            || (args.len() == 9 && args[7] != "--sources")
        {
            return Err("project-check --links DIRECTORY --verus-root SDK --out NEW_DIRECTORY [--sources JSON]".into());
        }
        return if let Some(source) = args.get(8) {
            client::run_scoped(
                Path::new(&args[2]),
                Path::new(&args[4]),
                Path::new(&args[6]),
                Some(Path::new(source)),
            )
        } else {
            client::run(
                Path::new(&args[2]),
                Path::new(&args[4]),
                Path::new(&args[6]),
            )
        };
    }
    if args.first().is_some_and(|s| s == "ordinary-check") {
        let keys = ["--input", "--config", "--verus-root", "--out"];
        if args.len() != 9 || keys.iter().enumerate().any(|(i, k)| args[1 + i * 2] != *k) {
            return Err("ordinary-check --input DIRECTORY --config JSON --verus-root SDK --out NEW_DIRECTORY".into());
        }
        return ordinary::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
        );
    }
    if args.first().is_some_and(|s| s == "ordinary-diagnose") {
        if args.len() != 7 || args[1] != "--input" || args[3] != "--z3" || args[5] != "--out" {
            return Err(
                "ordinary-diagnose --input DIRECTORY --z3 SOLVER --out NEW_DIRECTORY".into(),
            );
        }
        return evidence::ordinary(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "project-evidence") {
        if args.len() != 5 || args[1] != "--input" || args[3] != "--out" {
            return Err("project-evidence --input DIRECTORY --out NEW_DIRECTORY".into());
        }
        return evidence::run(Path::new(&args[2]), Path::new(&args[4]));
    }
    if args.first().is_some_and(|s| s == "client-query") {
        let keys = ["--vir", "--function", "--source-root", "--z3", "--out"];
        if args.len() != 11 || keys.iter().enumerate().any(|(i, k)| args[1 + i * 2] != *k) {
            return Err("client-query --vir VIR --function PATH --source-root DIRECTORY --z3 SOLVER --out NEW_DIRECTORY".into());
        }
        return obligation::run(
            Path::new(&args[2]),
            &args[4],
            Path::new(&args[6]),
            Path::new(&args[8]),
            Path::new(&args[10]),
        );
    }
    if args.first().is_some_and(|s| s == "project-diagnose") {
        if args.len() != 7 || args[1] != "--input" || args[3] != "--z3" || args[5] != "--out" {
            return Err(
                "project-diagnose --input DIRECTORY --z3 SOLVER --out NEW_DIRECTORY".into(),
            );
        }
        return evidence::diagnose(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "project-link-validate") {
        if args.len() != 3 || args[1] != "--input" {
            return Err("project-link-validate --input DIRECTORY".into());
        }
        return link::validate(Path::new(&args[2]));
    }
    if args.first().is_some_and(|s| s == "project-link") {
        if args.len() != 7 || args[1] != "--capture" || args[3] != "--config" || args[5] != "--out"
        {
            return Err(
                "project-link --capture DIRECTORY --config JSON --out NEW_DIRECTORY".into(),
            );
        }
        return link::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.first().is_some_and(|s| s == "project-validate") {
        if args.len() != 5 || args[1] != "--capture" || args[3] != "--config" {
            return Err("project-validate --capture DIRECTORY --config JSON".into());
        }
        return project::validate(Path::new(&args[2]), Path::new(&args[4]));
    }
    if args.first().is_some_and(|s| s == "project-capture") {
        if args.len() != 9
            || args[1] != "--config"
            || args[3] != "--manifest-path"
            || args[5] != "--verus-root"
            || args[7] != "--out"
        {
            return Err("project-capture --config JSON --manifest-path TOML --verus-root SDK --out NEW_DIRECTORY".into());
        }
        return project::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
        );
    }
    if args
        .first()
        .is_some_and(|s| s == "conditional-resource-plan")
    {
        if args.len() != 9
            || args[1] != "--records"
            || args[3] != "--vir"
            || args[5] != "--cwd"
            || args[7] != "--out"
        {
            return Err(
                "conditional-resource-plan --records RAW --vir VIR --cwd PATH --out NEW_DIRECTORY"
                    .into(),
            );
        }
        return resource::conditional(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
            Path::new(&args[8]),
        );
    }
    if args.first().is_some_and(|s| s == "contract-query") {
        if args.len() != 7 || args[1] != "--request" || args[3] != "--z3" || args[5] != "--out" {
            return Err("contract-query --request JSON --z3 PATH --out NEW_DIRECTORY".into());
        }
        return attribution::run(
            Path::new(&args[2]),
            Path::new(&args[4]),
            Path::new(&args[6]),
        );
    }
    if args.is_empty() || args.iter().any(|x| x == "--help") {
        println!(
            "cargo-spec-check --manifest-path PATH --package NAME --verus-root PATH --out PATH [--target TRIPLE] [--features NAMES] [--no-default-features] [--proof-guidance]"
        );
        return Ok(());
    }
    let mut iter = args.iter();
    while let Some(key) = iter.next() {
        if ![
            "--manifest-path",
            "--package",
            "--verus-root",
            "--out",
            "--target",
            "--features",
            "--no-default-features",
            "--loop-guidance",
            "--proof-guidance",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unknown option {key}").into());
        }
        let value = if key == "--no-default-features"
            || key == "--loop-guidance"
            || key == "--proof-guidance"
        {
            "true".to_owned()
        } else {
            iter.next().ok_or("missing option value")?.clone()
        };
        if options.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate option {key}").into());
        }
    }
    let get = |k: &str| {
        options
            .get(k)
            .cloned()
            .ok_or_else(|| format!("missing {k}"))
    };
    let manifest = fs::canonicalize(get("--manifest-path")?)?;
    let package = get("--package")?;
    let root = fs::canonicalize(get("--verus-root")?)?;
    let out = PathBuf::from(get("--out")?);
    fs::create_dir(&out)?;
    let out = fs::canonicalize(out)?;
    let cargo = env::var("CARGO").unwrap_or("cargo".into());
    let metadata = execute(
        Command::new(&cargo)
            .current_dir(manifest.parent().unwrap())
            .args([
                "metadata",
                "--offline",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
            ])
            .arg(&manifest),
        &out,
        "metadata",
    )?;
    if !metadata.status.success() {
        write_json(
            &out.join("report.json"),
            &json!({
                "status":"metadata_failed", "build_success":false,
                "package":package, "package_manifest":manifest,
                "failure_stage":"metadata", "diagnostic":"metadata.stderr"
            }),
        )?;
        reader::write(&out)?;
        return Err("Cargo metadata failed; see REVIEW.md and metadata.stderr".into());
    }
    let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
    write_json(
        &out.join("request.json"),
        &json!({"options":options,"tool_sha256":binary_hash,"cargo":cargo}),
    )?;
    let selected = metadata["packages"]
        .as_array()
        .ok_or("missing packages")?
        .iter()
        .filter(|p| p["name"] == package)
        .collect::<Vec<_>>();
    if selected.len() != 1 {
        return Err("package selection is missing or ambiguous".into());
    }
    let package_manifest = PathBuf::from(
        selected[0]["manifest_path"]
            .as_str()
            .ok_or("missing manifest")?,
    );
    if out.starts_with(package_manifest.parent().unwrap()) {
        return Err("output must be outside the selected package".into());
    }
    let mut before = BTreeMap::new();
    runner::source_hashes(
        package_manifest
            .parent()
            .ok_or("missing package directory")?,
        &mut before,
    )?;
    let workspace = PathBuf::from(
        metadata["workspace_root"]
            .as_str()
            .ok_or("missing workspace")?,
    );
    let rustc = Command::new("rustc").arg("-vV").output()?;
    fs::write(out.join("rustc.txt"), &rustc.stdout)?;
    let version = String::from_utf8(rustc.stdout)?;
    let host = version
        .lines()
        .find_map(|s| s.strip_prefix("host: "))
        .ok_or("missing compiler host")?;
    let target = options.get("--target").map(String::as_str).unwrap_or(host);
    let captures = out.join("capture");
    fs::create_dir(&captures)?;
    let mut parent = BTreeMap::new();
    for (k, v) in env::vars() {
        parent.insert(k, hash(v.as_bytes())?);
    }
    write_json(&captures.join("parent-env.json"), &parent)?;
    let mut command = Command::new(root.join("cargo-verus"));
    command
        .current_dir(manifest.parent().unwrap())
        .args(["build", "--manifest-path"])
        .arg(&manifest)
        .args(["-p", &package, "--target-dir"])
        .arg(out.join("cargo-target"));
    if let Some(features) = options.get("--features") {
        command.args(["--features", features]);
    }
    if options.contains_key("--no-default-features") {
        command.arg("--no-default-features");
    }
    let build = execute(
        command
            .args(["--offline", "--target", target, "--tests"])
            .arg("--message-format=json")
            .env("CARGO", env::current_exe()?)
            .env("SPEC_CHECK_CARGO", &cargo)
            .env(capture::DIR, &captures)
            .env("SPEC_CHECK_PACKAGE", &package)
            .env("RUSTC_BOOTSTRAP", "1")
            .env("VERUS_USE_RUSTUP", "0")
            .env(
                "VERUS_Z3_PATH",
                root.parent()
                    .and_then(Path::parent)
                    .ok_or("invalid Verus root")?
                    .join("z3"),
            ),
        &out,
        "build",
    )?;
    let mut after = BTreeMap::new();
    runner::source_hashes(package_manifest.parent().unwrap(), &mut after)?;
    if before != after {
        return Err("package inputs changed during Cargo build".into());
    }
    let artifacts = String::from_utf8_lossy(&build.stdout)
        .lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|v| {
            v["reason"] == "compiler-artifact"
                && v["package_id"] == selected[0]["id"]
                && v["profile"]["test"] == true
                && v["executable"].is_string()
        })
        .collect::<Vec<_>>();
    let mut invocations = Vec::new();
    for entry in fs::read_dir(&captures)? {
        let p = entry?.path();
        if p.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("invocation-")
        {
            invocations.push(serde_json::from_slice::<capture::Invocation>(&fs::read(
                p,
            )?)?);
        }
    }
    for message in String::from_utf8_lossy(&build.stdout)
        .lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|v| v["reason"] == "build-script-executed" && v["package_id"] == selected[0]["id"])
    {
        for pair in message["env"].as_array().into_iter().flatten() {
            let key = pair[0]
                .as_str()
                .ok_or("invalid build-script environment name")?;
            let value = pair[1]
                .as_str()
                .ok_or("invalid build-script environment value")?;
            for inv in &mut invocations {
                if inv.env_hashes.get(key) == Some(&Some(hash(value.as_bytes())?)) {
                    inv.env.insert(key.into(), value.into());
                    inv.env_origins
                        .insert(key.into(), "cargo-build-script".into());
                    inv.unsupported_env.retain(|k| k != key);
                }
            }
        }
    }
    write_json(&out.join("cargo-context.json"), &invocations)?;
    let mut workspace_inputs = BTreeMap::new();
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        ".cargo/config.toml",
        ".cargo/config",
        "rust-toolchain.toml",
    ] {
        let path = workspace.join(file);
        if path.is_file() {
            workspace_inputs.insert(path.clone(), hash(&fs::read(path)?)?);
        }
    }
    write_json(&out.join("workspace-inputs.json"), &workspace_inputs)?;
    write_json(
        &out.join("report.json"),
        &json!({"status":if build.status.success(){"build_only"}else{"build_failed"},"build_success":build.status.success(),"package":package,"package_manifest":package_manifest,"artifacts":artifacts,"invocations":invocations}),
    )?;
    if !build.status.success() {
        reader::write(&out)?;
        return Err("Cargo build failed; see REVIEW.md, build.stdout and build.stderr".into());
    }
    if build.status.success() {
        let verified = selected[0]["metadata"]["verus"]["verify"] == true;
        let reports = runner::check(
            &root,
            package_manifest
                .parent()
                .ok_or("missing package directory")?,
            &out,
            &artifacts,
            &invocations,
            runner::Options {
                verified,
                host: target == host,
                loop_guidance: (options.contains_key("--loop-guidance")
                    || options.contains_key("--proof-guidance")),
            },
        )?;
        if hash(&fs::read(&binary)?)? != binary_hash {
            return Err("tool binary changed during execution".into());
        }
        for (path, expected) in &workspace_inputs {
            if hash(&fs::read(path)?)? != *expected {
                return Err("workspace configuration changed during check".into());
            }
        }
        let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
        let mut runtimes: BTreeMap<String, usize> = BTreeMap::new();
        for sample in reports
            .iter()
            .filter_map(|t| t["samples"].as_array())
            .flatten()
        {
            *statuses
                .entry(sample["status"].as_str().unwrap_or("not_analyzed").into())
                .or_default() += 1;
            *runtimes
                .entry(
                    sample["runtime"]["status"]
                        .as_str()
                        .unwrap_or("not_run")
                        .into(),
                )
                .or_default() += 1;
        }
        let limits = [
            "Only Cargo packages explicitly enabled for Verus; normal-return tests; usize and standard Vec assert_eq conversion",
            "Optional guidance covers source-bound iterator guarantees, immutable usize tables (up to 16 cases), opaque spec unfolding (fuel 4) and one monotone boolean-sequence prefix loop; general state/resource inference remains unsupported",
            "Imported dependency contracts and Rustc/Verus erasure configuration are trusted; no general translation theorem",
            "Expected panic, unresolved build-environment overrides and source-snapshot escapes are reported separately",
            "Packages without Verus metadata can expose original typed Rustc records while retaining unsupported_context; no contract proof or translation correspondence is claimed for those records",
        ];
        write_json(
            &out.join("report.json"),
            &json!({"status":"completed","build_success":true,"package":package,"package_manifest":package_manifest,"targets":reports,
            "metrics":{"targets":reports.len(),"targets_without_census":reports.iter().filter(|t|!t["samples"].is_array()).count(),"statuses":statuses,"runtime":runtimes,
            "typed_only_samples":reports.iter().filter_map(|t|t["samples"].as_array()).flatten().filter(|s|s["typed_extraction"]["status"]=="typed").count()},
            "source_selection":"Cargo metadata and actual compiler invocations; no source or test list supplied","limits":limits}),
        )?;
        let mut lines=vec![format!("# Cargo 测试契约检查 — {package}"),String::new(),"[逐项审阅入口](REVIEW.md)".into(),String::new(),
            "测试清单与构建参数来自实际 Cargo 构建。Rust 入口直接调用 Rustc 与 Verus，保留原文件及模块关系；没有新增测试入口前提。".into(),String::new(),
            "| 目标 | 测试 | 原测试运行 | 检查 |".into(),"| --- | --- | --- | --- |".into()];
        for target in &reports {
            if let Some(samples) = target["samples"].as_array() {
                for sample in samples {
                    lines.push(format!(
                        "| {} | {} | {} | {} |",
                        target["target"]["name"].as_str().unwrap_or("?"),
                        sample["function"].as_str().unwrap_or("?"),
                        sample["runtime"]["status"].as_str().unwrap_or("?"),
                        sample["status"].as_str().unwrap_or("?")
                    ));
                }
            } else {
                lines.push(format!(
                    "| {} | 清单未建立 | 未运行 | {} |",
                    target["target"]["name"].as_str().unwrap_or("?"),
                    target["status"].as_str().unwrap_or("?")
                ));
            }
        }
        lines.extend([String::new(),"报告中的 proved 要求目标测试函数确实进入 Verus 检查、所在目标的 baseline 验证通过，并通过转换对应检查。已有入口前提、预期 panic、忽略执行和构建上下文缺口分别报告。".into(),
            "导入契约沿用实际构建产物；依赖实现未在本入口重新全量验证。更完整的资源推导与实验性证明指导仍需继续开发。逐项原位置、实际调用目标、诊断、cfg 和指纹见 report.json、context.json 及 inputs.json。".into()]);
        fs::write(out.join("README.md"), lines.join("\n") + "\n")?;
    }
    reader::write(&out)?;
    println!(
        "{} test artifacts, {} captured compilations; {}",
        artifacts.len(),
        invocations.len(),
        out.join("report.json").display()
    );
    Ok(())
}
fn main() {
    let result = if env::var_os(capture::DIR).is_some() {
        capture::wrapper(&env::args().skip(1).collect::<Vec<_>>())
    } else {
        check().map(|_| 0)
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("cargo-spec-check: {e}");
            std::process::exit(2)
        }
    }
}
