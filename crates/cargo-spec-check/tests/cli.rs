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
            "mutation-triage",
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
