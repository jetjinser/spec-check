use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "cargo-spec-check-cli-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("src")).unwrap();
    path
}

fn manifest(dir: &Path) -> String {
    format!(
        r#"{{
  "target_workspace": {dir:?},
  "source": "src/lib.rs",
  "package": "p",
  "module": "m",
  "target": null,
  "target_dir": {target:?},
  "full_confirm": true,
  "full_target_dir": null,
  "timeout_seconds": 1,
  "mutations": [
    {{"id": "m1", "description": "comparison_flip: f at line 1", "old": "0", "new": "1"}}
  ]
}}"#,
        dir = dir.display().to_string(),
        target = dir.join("target").display().to_string(),
    )
}

#[test]
fn help_lists_mutation_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-spec-check"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("mutation-generate"));
    assert!(stdout.contains("mutation-triage"));
    assert!(stdout.contains("--dry-run"));
}

#[test]
fn dry_run_accepts_primary_and_alias() {
    let dir = temp_dir("dry-run");
    fs::write(dir.join("src/lib.rs"), "fn f() { let x = 0; }\n").unwrap();
    let manifest_path = dir.join("manifest.json");
    fs::write(&manifest_path, manifest(&dir)).unwrap();
    for command in ["mutation", "analyses-mutation"] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-spec-check"))
            .args([
                command,
                "--manifest",
                manifest_path.to_str().unwrap(),
                "--dry-run",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            command,
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("valid mutants"));
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn nested_generate_writes_manifest() {
    let dir = temp_dir("generate");
    fs::write(dir.join("src/lib.rs"), "fn f() { return 0; }\n").unwrap();
    let config_path = dir.join("config.json");
    fs::write(
        &config_path,
        format!(
            r#"{{
  "target_workspace": {dir:?},
  "source": "src/lib.rs",
  "package": "p",
  "module": "m",
  "target": null,
  "target_dir": {target:?},
  "full_confirm": false,
  "functions": ["f"],
  "operators": ["return_constant"],
  "max_mutants": 10,
  "timeout_seconds": 1
}}"#,
            dir = dir.display().to_string(),
            target = dir.join("target").display().to_string(),
        ),
    )
    .unwrap();
    let out = dir.join("manifest.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-spec-check"))
        .args([
            "mutation",
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = fs::read_to_string(out).unwrap();
    assert!(manifest.contains("G-001-return_constant"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn quickstart_runs_without_verus() {
    let dir = temp_dir("quickstart");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/mutation-quickstart/fixture");
    let config = dir.join("config.json");
    fs::write(
        &config,
        format!(
            r#"{{
  "target_workspace": {fixture:?},
  "source": "src/lib.rs",
  "package": "quickstart",
  "module": "quickstart",
  "target": null,
  "target_dir": {target:?},
  "full_confirm": true,
  "functions": ["f"],
  "operators": ["return_constant"],
  "max_mutants": 5,
  "timeout_seconds": 30
}}"#,
            fixture = fixture.display().to_string(),
            target = dir.join("target").display().to_string(),
        ),
    )
    .unwrap();
    let manifest = dir.join("manifest.json");
    let run = dir.join("run");
    let triage = dir.join("triage.json");
    let fake = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/mutation-quickstart/fake-verus.sh");
    let run_bin = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-spec-check"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_bin(&[
        "mutation",
        "generate",
        "--config",
        config.to_str().unwrap(),
        "--out",
        manifest.to_str().unwrap(),
    ]);
    run_bin(&[
        "mutation",
        "run",
        "--manifest",
        manifest.to_str().unwrap(),
        "--cargo-verus",
        fake.to_str().unwrap(),
        "--z3",
        "/nonexistent/z3",
        "--vstd",
        fixture.to_str().unwrap(),
        "--out",
        run.to_str().unwrap(),
    ]);
    run_bin(&[
        "mutation",
        "triage",
        "--manifest",
        manifest.to_str().unwrap(),
        "--results",
        run.join("results.jsonl").to_str().unwrap(),
        "--source",
        fixture.join("src/lib.rs").to_str().unwrap(),
        "--out",
        triage.to_str().unwrap(),
    ]);
    let generated = fs::read_to_string(&manifest).unwrap();
    assert!(generated.contains("\"schema_version\": \"mutation.manifest.v1\""));
    let run_report = fs::read_to_string(run.join("report.json")).unwrap();
    assert!(run_report.contains("\"schema_version\": \"mutation.report.v1\""));
    let report = fs::read_to_string(triage).unwrap();
    assert!(report.contains("\"survivors\": 1"));
    assert!(report.contains("\"schema_version\": \"mutation.triage.v1\""));

    let strict_out = dir.join("strict");
    let strict = Command::new(env!("CARGO_BIN_EXE_cargo-spec-check"))
        .args([
            "mutation",
            "run",
            "--manifest",
            manifest.to_str().unwrap(),
            "--cargo-verus",
            fake.to_str().unwrap(),
            "--z3",
            "/nonexistent/z3",
            "--vstd",
            fixture.to_str().unwrap(),
            "--out",
            strict_out.to_str().unwrap(),
            "--mode",
            "strict",
        ])
        .output()
        .unwrap();
    assert!(!strict.status.success(), "strict mode must flag survivors");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn schema_files_are_json_contracts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    for name in [
        "mutation.manifest.v1.schema.json",
        "mutation.report.v1.schema.json",
        "mutation.triage.v1.schema.json",
    ] {
        let text = fs::read_to_string(root.join(name)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(value.get("$schema").is_some(), "{name}");
        assert!(value.get("required").is_some(), "{name}");
    }
}

#[test]
fn triage_writes_structured_report() {
    let dir = temp_dir("triage");
    fs::write(dir.join("src/lib.rs"), "fn f() { let x = 0; }\n").unwrap();
    let manifest_path = dir.join("manifest.json");
    fs::write(&manifest_path, manifest(&dir)).unwrap();
    let results_path = dir.join("results.jsonl");
    fs::write(
        &results_path,
        r#"{"id":"m1","description":"comparison_flip: f at line 1","final_status":"verified_survivor","focus":{"log":"logs/m1-focus.log"},"full":null}
"#,
    )
    .unwrap();
    let out = dir.join("triage.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-spec-check"))
        .args([
            "mutation",
            "triage",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--results",
            results_path.to_str().unwrap(),
            "--source",
            dir.join("src/lib.rs").to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read_to_string(out).unwrap();
    assert!(report.contains("\"survivors\": 1"));
    assert!(report.contains("\"entries\""));
    let _ = fs::remove_dir_all(dir);
}
