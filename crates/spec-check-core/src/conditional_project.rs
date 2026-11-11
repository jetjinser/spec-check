//! Run native contract mutations against Cargo-discovered workspace tests.
use crate::{Result, conditional, conditional_check, execute, hash, project, runner, write_json};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path, process::Command, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    package: String,
    target: String,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default = "yes")]
    default_features: bool,
    #[serde(default)]
    cfgs: Vec<String>,
    #[serde(default)]
    rlimit: Option<u32>,
    #[serde(default = "timeout")]
    timeout_seconds: u64,
}
fn yes() -> bool {
    true
}
fn timeout() -> u64 {
    300
}
fn load(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn related(metadata: &Value, root: &str) -> Result<Vec<String>> {
    let mut ids = BTreeSet::from([root.to_owned()]);
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("Cargo dependency graph missing")?;
    loop {
        let before = ids.len();
        for node in nodes {
            if node["deps"]
                .as_array()
                .ok_or("Cargo dependencies missing")?
                .iter()
                .any(|dep| dep["pkg"].as_str().is_some_and(|id| ids.contains(id)))
            {
                ids.insert(
                    node["id"]
                        .as_str()
                        .ok_or("Cargo package identity missing")?
                        .to_owned(),
                );
            }
        }
        if ids.len() == before {
            break;
        }
    }
    Ok(metadata["packages"]
        .as_array()
        .ok_or("Cargo packages missing")?
        .iter()
        .filter(|p| {
            metadata["workspace_members"]
                .as_array()
                .is_some_and(|members| members.contains(&p["id"]))
                && p["id"].as_str().is_some_and(|id| ids.contains(id))
        })
        .map(|p| {
            p["name"]
                .as_str()
                .ok_or("Cargo package name missing")
                .map(str::to_owned)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn cargo(cfg: &Config, manifest: &Path, target_dir: &Path, packages: &[String]) -> Result<Command> {
    let mut command = Command::new("cargo");
    command
        .args([
            "test",
            "--offline",
            "--locked",
            "--tests",
            "--no-run",
            "--message-format=json",
            "--manifest-path",
        ])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target_dir)
        .args(["--target", &cfg.target])
        .env("RUSTC_BOOTSTRAP", "1");
    for package in packages {
        command.args(["-p", package]);
    }
    if !cfg.default_features {
        command.arg("--no-default-features");
    }
    if !cfg.features.is_empty() {
        command.arg("--features").arg(
            cfg.features
                .iter()
                .map(|f| format!("{}/{f}", cfg.package))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if !cfg.cfgs.is_empty() {
        if std::env::var_os("RUSTFLAGS").is_some() {
            return Err(
                "cfgs with inherited RUSTFLAGS require an explicit project configuration".into(),
            );
        }
        let mut flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
        for arg in crate::capture::configured_args(&[], &cfg.cfgs, true)? {
            if !flags.is_empty() {
                flags.push('\x1f');
            }
            flags.push_str(&arg);
        }
        command.env("CARGO_ENCODED_RUSTFLAGS", flags);
    }
    Ok(command)
}

fn suite(
    cfg: &Config,
    manifest: &Path,
    target_dir: &Path,
    packages: &[String],
    expected_build: &Value,
    out: &Path,
) -> Result<Value> {
    fs::create_dir(out)?;
    let (build, timed_out) = runner::execute_limited(
        &mut cargo(cfg, manifest, target_dir, packages)?,
        out,
        "build",
        Duration::from_secs(cfg.timeout_seconds),
    )?;
    if !build.status.success() || timed_out {
        return Ok(
            json!({"status":"build_failed","exit_code":build.status.code(),"timeout":timed_out,"runtime":"not_run"}),
        );
    }
    let artifacts = String::from_utf8_lossy(&build.stdout)
        .lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|v| v["reason"] == "compiler-artifact")
        .collect::<Vec<_>>();
    write_json(&out.join("artifacts.json"), &artifacts)?;
    if artifacts
        .iter()
        .filter(|a| a["package_id"] == expected_build["package"])
        .any(|a| a["features"] != expected_build["features"])
    {
        return Ok(
            json!({"status":"configuration_mismatch","reason":"runtime package features differ from the captured contract build","runtime":"not_run"}),
        );
    }
    let counts =
        regex::Regex::new(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored")?;
    let mut tests = Vec::new();
    for artifact in &artifacts {
        if artifact["profile"]["test"] != true {
            continue;
        }
        let Some(exe) = artifact["executable"].as_str() else {
            continue;
        };
        let index = tests.len();
        let (list, list_timeout) = runner::execute_limited(
            Command::new(exe).args(["--list", "--format", "terse"]),
            out,
            &format!("test-{index}-list"),
            Duration::from_secs(cfg.timeout_seconds),
        )?;
        let names = String::from_utf8_lossy(&list.stdout)
            .lines()
            .filter_map(|s| s.strip_suffix(": test").map(str::to_owned))
            .collect::<Vec<_>>();
        let mut row = json!({"package":artifact["package_id"],"target":artifact["target"],"executable":exe,"sha256":hash(&fs::read(exe)?)?,"tests":names});
        if !list.status.success() || list_timeout {
            row["status"] = json!("registration_failed");
            row["runtime"] = json!("not_run");
        } else {
            let (result, timed_out) = runner::execute_limited(
                &mut Command::new(exe),
                out,
                &format!("test-{index}-run"),
                Duration::from_secs(cfg.timeout_seconds),
            )?;
            let text = String::from_utf8_lossy(&result.stdout);
            let summary = counts.captures(&text).map(|c| json!({"passed":c[2].parse::<usize>().unwrap(),"failed":c[3].parse::<usize>().unwrap(),"ignored":c[4].parse::<usize>().unwrap()}));
            row["summary"] = summary.unwrap_or(Value::Null);
            row["exit_code"] = json!(result.status.code());
            row["status"] = json!(if timed_out {
                "timeout"
            } else if result.status.success() && !row["summary"].is_null() {
                "passed"
            } else if result.status.code() == Some(101)
                && row["summary"]["failed"].as_u64().is_some_and(|n| n > 0)
            {
                "test_failed"
            } else {
                "execution_failed"
            });
        }
        tests.push(row);
    }
    let status = suite_status(&tests);
    Ok(json!({"status":status,"tests":tests,"build_exit_code":build.status.code()}))
}
fn suite_status(tests: &[Value]) -> &'static str {
    if tests
        .iter()
        .any(|r| !matches!(r["status"].as_str(), Some("passed" | "test_failed")))
    {
        "execution_failed"
    } else if tests.is_empty()
        || tests
            .iter()
            .all(|r| r["tests"].as_array().is_some_and(Vec::is_empty))
    {
        "no_tests"
    } else if tests.iter().any(|r| r["status"] == "test_failed") {
        "test_failed"
    } else {
        "passed"
    }
}

fn inventory(suite: &Value) -> Value {
    let mut rows = suite["tests"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            json!([
                r["package"],
                r["target"]["name"],
                r["target"]["kind"],
                r["tests"]
            ])
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(Value::to_string);
    json!(rows)
}
fn unchanged(inputs: &Value, workspace: &Path) -> Result<()> {
    for (path, expected) in inputs.as_object().ok_or("capture input hashes missing")? {
        let path = Path::new(path);
        if path.starts_with(workspace) && hash(&fs::read(path)?)? != *expected {
            return Err(format!("runtime modified a captured input: {}", path.display()).into());
        }
    }
    Ok(())
}
fn outcome(baseline: &Value, alternative: &Value, restored: &Value) -> &'static str {
    if baseline["status"] != "passed" {
        return "baseline_not_passed";
    }
    if restored["status"] != "passed" || inventory(baseline) != inventory(restored) {
        return "restoration_not_established";
    }
    if !matches!(
        alternative["status"].as_str(),
        Some("passed" | "test_failed")
    ) {
        return "alternative_not_tested";
    }
    if inventory(baseline) != inventory(alternative) {
        return "test_inventory_changed";
    }
    match alternative["status"].as_str() {
        Some("test_failed") => "tests_reject_alternative",
        Some("passed") => "tests_accept_alternative",
        _ => "alternative_not_tested",
    }
}

pub fn run(config: &Path, manifest: &Path, sdk: &Path, destination: &Path) -> Result<()> {
    let cfg: Config = serde_json::from_slice(&fs::read(config)?)?;
    if cfg.timeout_seconds == 0 || cfg.rlimit == Some(0) {
        return Err("resource limits must be positive".into());
    }
    fs::create_dir(destination)?;
    let out = fs::canonicalize(destination)?;
    let manifest = fs::canonicalize(manifest)?;
    let mut report = json!({"complete":false,"stage":"metadata","runtime":"not_run","per_function_hints":0,"ktest":"not_run","results":[]});
    write_json(&out.join("configuration.json"), &load(config)?)?;
    write_json(&out.join("report.json"), &report)?;
    let mut work = || -> Result<Value> {
        let metadata = execute(
            Command::new("cargo")
                .args([
                    "metadata",
                    "--offline",
                    "--locked",
                    "--format-version=1",
                    "--manifest-path",
                ])
                .arg(&manifest),
            &out,
            "metadata",
        )?;
        if !metadata.status.success() {
            return Err("Cargo metadata failed".into());
        }
        let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
        let workspace = Path::new(
            metadata["workspace_root"]
                .as_str()
                .ok_or("workspace missing")?,
        );
        if out.starts_with(workspace) {
            return Err("output must be outside the source workspace".into());
        }
        runner::copy_package(workspace, &out.join("workspace"))?;
        let snapshot = out.join("workspace");
        let manifest = snapshot.join(manifest.strip_prefix(workspace)?);
        let metadata_output = execute(
            Command::new("cargo")
                .args([
                    "metadata",
                    "--offline",
                    "--locked",
                    "--format-version=1",
                    "--manifest-path",
                ])
                .arg(&manifest),
            &out,
            "snapshot-metadata",
        )?;
        if !metadata_output.status.success() {
            return Err("snapshot Cargo metadata failed".into());
        }
        let metadata: Value = serde_json::from_slice(&metadata_output.stdout)?;
        let package = metadata["packages"]
            .as_array()
            .ok_or("packages missing")?
            .iter()
            .find(|p| p["name"] == cfg.package)
            .ok_or("configured package missing")?;
        let packages = related(
            &metadata,
            package["id"].as_str().ok_or("package identity missing")?,
        )?;
        write_json(
            &out.join("test-packages.json"),
            &json!({"packages":packages,"selection":"workspace packages with a dependency path to the selected library, including the library"}),
        )?;
        let capture_config = out.join("capture-config.json");
        write_json(
            &capture_config,
            &json!({"profiles":[{"name":"library","package":cfg.package,"target":cfg.target,"tests":false,"features":cfg.features,"default_features":cfg.default_features,"cfgs":cfg.cfgs,"rlimit":cfg.rlimit}]}),
        )?;
        report["stage"] = json!("capture");
        write_json(&out.join("report.json"), &report)?;
        project::run(&capture_config, &manifest, sdk, &out.join("capture"))?;
        let capture = load(&out.join("capture/report.json"))?;
        if capture["status"] != "captured" {
            return Err("project capture incomplete".into());
        }
        let mut inputs = Vec::new();
        for entry in fs::read_dir(out.join("capture/library"))? {
            let dir = entry?.path();
            if dir.join("verification-input.json").is_file() {
                let context = load(&dir.join("context.json"))?;
                if !context["args"]
                    .as_array()
                    .ok_or("context arguments missing")?
                    .iter()
                    .any(|a| a == "--test")
                {
                    inputs.push((
                        dir.join("context.json"),
                        load(&dir.join("verification-input.json"))?,
                    ));
                }
            }
        }
        if inputs.len() != 1 {
            return Err("expected one captured library verification context".into());
        }
        let (context, verification) = &inputs[0];
        let vir = Path::new(verification["vir"].as_str().ok_or("native VIR missing")?);
        report["stage"] = json!("scan");
        write_json(&out.join("report.json"), &report)?;
        conditional::run(vir, &out.join("scan"))?;
        conditional::generate(vir, &snapshot, &out.join("generation"))?;
        let native_config = out.join("native-config.json");
        write_json(
            &native_config,
            &json!({"mutations":out.join("generation"),"vir":vir,"context":context,"capture_inputs":out.join("capture/inputs.json")}),
        )?;
        report["stage"] = json!("native");
        write_json(&out.join("report.json"), &report)?;
        conditional_check::run(&native_config, sdk, &out.join("native"))?;
        let native = load(&out.join("native/report.json"))?;
        let artifacts = load(&out.join("capture/library/artifacts.json"))?;
        let features = &artifacts
            .as_array()
            .ok_or("capture artifacts missing")?
            .iter()
            .find(|a| a["package_id"] == package["id"])
            .ok_or("library build artifact missing")?["features"];
        let expected_build = json!({"package":package["id"],"features":features});
        let mut rows = Vec::new();
        let runtime = out.join("runtime");
        fs::create_dir(&runtime)?;
        let accepted = native["results"]
            .as_array()
            .ok_or("native results missing")?
            .iter()
            .any(|r| r["result"]["status"] == "contract_accepts_alternative");
        report["stage"] = json!("runtime");
        write_json(&out.join("report.json"), &report)?;
        let baseline = if accepted {
            suite(
                &cfg,
                &manifest,
                &runtime.join("target"),
                &packages,
                &expected_build,
                &runtime.join("baseline"),
            )?
        } else {
            json!({"status":"not_run","reason":"no contract-accepted alternative"})
        };
        let captured_inputs = load(&out.join("capture/inputs.json"))?;
        unchanged(&captured_inputs, &snapshot)?;
        for row in native["results"].as_array().unwrap() {
            let mut row = row.clone();
            if row["result"]["status"] != "contract_accepts_alternative" {
                row["runtime"] = json!({"status":"not_run","reason":"alternative not established by native contract verification"});
            } else if baseline["status"] != "passed" {
                row["runtime"] = json!({"status":if baseline["status"] == "no_tests" {"no_tests"} else {"baseline_not_passed"},"baseline":baseline});
            } else {
                let case = runtime.join(format!("case-{}", rows.len()));
                fs::create_dir(&case)?;
                let mutation = &row["mutation"];
                let source = snapshot.join(
                    mutation["source_relative"]
                        .as_str()
                        .ok_or("mutation source missing")?,
                );
                let original = fs::read(&source)?;
                if hash(&original)? != mutation["source_sha256"] {
                    return Err("runtime source differs from the verified source".into());
                }
                let replacement = fs::read(
                    mutation["artifact"]
                        .as_str()
                        .ok_or("mutation artifact missing")?,
                )?;
                fs::write(&source, &replacement)?;
                let alternative = suite(
                    &cfg,
                    &manifest,
                    &runtime.join("target"),
                    &packages,
                    &expected_build,
                    &case.join("alternative"),
                );
                fs::write(&source, &original)?;
                let restored = suite(
                    &cfg,
                    &manifest,
                    &runtime.join("target"),
                    &packages,
                    &expected_build,
                    &case.join("restored"),
                );
                if fs::read(&source)? != original {
                    return Err("source restoration failed".into());
                }
                unchanged(&captured_inputs, &snapshot)?;
                let alternative = alternative.unwrap_or_else(
                    |e| json!({"status":"execution_failed","reason":e.to_string()}),
                );
                let restored = restored.unwrap_or_else(
                    |e| json!({"status":"execution_failed","reason":e.to_string()}),
                );
                row["runtime"] = json!({"status":outcome(&baseline,&alternative,&restored),"baseline":baseline,"alternative":alternative,"restored":restored,"source_restored":true,"evidence":case});
            }
            rows.push(row);
            write_json(
                &out.join("report.json"),
                &json!({"complete":false,"stage":"runtime","results":rows,"baseline":baseline,"ktest":"not_run"}),
            )?;
        }
        let witnesses = rows
            .iter()
            .filter(|r| r["runtime"]["status"] == "tests_reject_alternative")
            .count();
        Ok(
            json!({"complete":true,"stage":"complete","results":rows,"baseline":baseline,"test_packages":packages,"witnesses":witnesses,"per_function_hints":0,"ktest":"not_run"}),
        )
    };
    match work() {
        Ok(result) => write_json(&out.join("report.json"), &result),
        Err(error) => {
            if out.join("report.json").is_file() {
                report = load(&out.join("report.json"))?;
            }
            if report["results"].as_array().is_some_and(Vec::is_empty)
                && out.join("native/report.json").is_file()
            {
                let mut results = load(&out.join("native/report.json"))?["results"].clone();
                for row in results.as_array_mut().into_iter().flatten() {
                    row["runtime"] = json!({"status":"not_run","reason":error.to_string()});
                }
                report["results"] = results;
            }
            report["complete"] = json!(false);
            report["error"] = json!(error.to_string());
            write_json(&out.join("report.json"), &report)?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_selection_includes_transitive_workspace_clients() {
        let package = |name: &str| json!({"id":name,"name":name});
        let node = |name: &str, deps: &[&str]| json!({"id":name,"deps":deps.iter().map(|d|json!({"pkg":d})).collect::<Vec<_>>()});
        let metadata = json!({"packages":[package("api"),package("adapter"),package("tests"),package("unrelated"),package("external")],
            "workspace_members":["api","adapter","tests","unrelated"],
            "resolve":{"nodes":[node("tests",&["adapter"]),node("adapter",&["api"]),node("external",&["api"]),node("unrelated",&[]),node("api",&[])]}});
        assert_eq!(
            related(&metadata, "api").unwrap(),
            vec!["api", "adapter", "tests"]
        );
    }
    #[test]
    fn failed_registration_is_not_absence_of_tests() {
        assert_eq!(
            suite_status(&[json!({"status":"registration_failed","tests":[]})]),
            "execution_failed"
        );
        assert_eq!(
            suite_status(&[json!({"status":"passed","tests":[]})]),
            "no_tests"
        );
        let a = json!({"package":"a","target":{"name":"a","kind":["lib"]},"tests":["x"]});
        let b = json!({"package":"b","target":{"name":"b","kind":["lib"]},"tests":["x"]});
        assert_eq!(
            inventory(&json!({"tests":[a,b]})),
            inventory(&json!({"tests":[b,a]}))
        );
    }
    #[test]
    fn only_comparable_test_failures_are_witnesses() {
        let suite = |status: &str, name: &str| json!({"status":status,"tests":[{"package":"p","target":{"name":"tests","kind":["lib"]},"tests":[name]}]});
        let original = suite("passed", "original");
        assert_eq!(
            outcome(&original, &suite("test_failed", "original"), &original),
            "tests_reject_alternative"
        );
        assert_eq!(
            outcome(&original, &suite("test_failed", "different"), &original),
            "test_inventory_changed"
        );
        assert_eq!(
            outcome(
                &original,
                &suite("test_failed", "original"),
                &suite("test_failed", "original")
            ),
            "restoration_not_established"
        );
        assert_eq!(
            outcome(&original, &json!({"status":"build_failed"}), &original),
            "alternative_not_tested"
        );
        assert_ne!(
            outcome(
                &json!({"status":"no_tests"}),
                &suite("test_failed", "original"),
                &original
            ),
            "tests_reject_alternative"
        );
    }
}
