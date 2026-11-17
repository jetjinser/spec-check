//! `mutation`: contract-mutation adequacy analysis.
//!
//! The analysis applies source mutations to a target workspace, runs Verus on
//! each mutant, and reports which mutants still verify. A verified survivor is
//! a **contract-gap candidate**: the contract admits a body that differs from
//! the original. It is not, by itself, an implementation bug or a runtime
//! counterexample.
//!
//! This is deliberately a thin, conservative runner. Mutants are supplied by a
//! frozen project-side manifest (exact old/new snippets); the analysis does not
//! invent mutations, does not decide intent, and does not classify survivors as
//! confirmed defects. Every mutant first runs focused verification; a
//! focus-verified mutant can then be confirmed with a full crate check.
//!
//! The input workspace is restored after every run, including on errors.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

pub const MANIFEST_SCHEMA_VERSION: &str = "mutation.manifest.v1";
pub const REPORT_SCHEMA_VERSION: &str = "mutation.report.v1";

/// Project-side mutation manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    /// Version of the manifest contract. Older manifests default to v1.
    #[serde(default = "default_manifest_schema_version")]
    pub schema_version: String,
    /// Workspace root containing the package.
    pub target_workspace: PathBuf,
    /// Source file to mutate, relative to `target_workspace`.
    pub source: PathBuf,
    /// Cargo package name passed to `cargo-verus -p`.
    pub package: String,
    /// Verus module selector for focused verification.
    pub module: String,
    /// Optional target triple (for example `x86_64-unknown-none`).
    #[serde(default)]
    pub target: Option<String>,
    /// Cargo target directory. Required so runs reuse one cache and do not
    /// write build output into the source tree.
    pub target_dir: PathBuf,
    /// Run a full crate check after a focused survivor. Defaults to true.
    #[serde(default = "default_true")]
    pub full_confirm: bool,
    /// Optional target dir for full-crate confirmation. Using a separate dir
    /// prevents a cached focused run from masking the full verification
    /// summary. Defaults to `<target_dir>/full`.
    #[serde(default)]
    pub full_target_dir: Option<PathBuf>,
    /// Per-Verus-run wall timeout in seconds. Defaults to 900.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    pub mutations: Vec<Mutation>,
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> u64 {
    900
}

fn default_manifest_schema_version() -> String {
    MANIFEST_SCHEMA_VERSION.to_string()
}

/// One exact-snippet mutation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Mutation {
    pub id: String,
    #[serde(default)]
    pub description: String,
    /// The snippet must occur exactly once in the pristine source.
    pub old: String,
    /// Replacement snippet.
    pub new: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Verified,
    Rejected,
    CompileError,
    TimedOut,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub status: Status,
    pub verified: Option<usize>,
    pub errors: Option<usize>,
    pub returncode: Option<i32>,
    pub elapsed_seconds: f64,
    pub timed_out: bool,
    /// `fresh` when a verification summary was observed; `cached` when cargo
    /// finished without one; `unknown` otherwise.
    pub cache_status: String,
    pub log: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutationResult {
    pub id: String,
    pub description: String,
    pub focus: RunResult,
    pub full: Option<RunResult>,
    /// `verified_survivor`, `rejected`, `compile_error`, ...
    pub final_status: String,
    pub first_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub mutants: usize,
    pub verified_survivors: usize,
    pub rejected: usize,
    pub compile_errors: usize,
    pub timeouts: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutationReport {
    pub schema_version: String,
    pub target_workspace: PathBuf,
    pub source: PathBuf,
    pub package: String,
    pub module: String,
    pub baseline_focus: RunResult,
    pub results: Vec<MutationResult>,
    pub summary: Summary,
}

struct SourceGuard {
    path: PathBuf,
    original: Vec<u8>,
}

impl Drop for SourceGuard {
    fn drop(&mut self) {
        let _ = fs::write(&self.path, &self.original);
    }
}

fn validate(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "unsupported manifest schema_version {:?} (expected {})",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    if manifest.mutations.is_empty() {
        return Err("no mutations in manifest".into());
    }
    let mut ids = BTreeSet::new();
    for m in &manifest.mutations {
        if m.id.trim().is_empty() {
            return Err("mutation id is empty".into());
        }
        if !ids.insert(m.id.as_str()) {
            return Err(format!("duplicate mutation id: {}", m.id));
        }
        if m.old.is_empty() {
            return Err(format!("{}: old snippet is empty", m.id));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub source: PathBuf,
    pub target_workspace: PathBuf,
    pub mutants: usize,
    pub warnings: Vec<String>,
}

/// Validate a manifest without running Verus or modifying the source.
pub fn validate_manifest(manifest: &Manifest) -> Result<ValidationReport, String> {
    validate(manifest)?;
    let source = manifest.target_workspace.join(&manifest.source);
    if !manifest.target_workspace.is_dir() {
        return Err(format!(
            "target workspace is not a directory: {}",
            manifest.target_workspace.display()
        ));
    }
    let pristine = fs::read_to_string(&source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut warnings = Vec::new();
    for mutation in &manifest.mutations {
        apply_once(&pristine, mutation)?;
        if mutation.new == mutation.old {
            warnings.push(format!("{}: new snippet equals old snippet", mutation.id));
        }
    }
    Ok(ValidationReport {
        source,
        target_workspace: manifest.target_workspace.clone(),
        mutants: manifest.mutations.len(),
        warnings,
    })
}

/// Apply one mutation to `pristine`, requiring exactly one occurrence.
pub fn apply_once(pristine: &str, mutation: &Mutation) -> Result<String, String> {
    let count = pristine.matches(&mutation.old).count();
    if count != 1 {
        return Err(format!(
            "{}: old snippet occurs {} times (expected exactly 1)",
            mutation.id, count
        ));
    }
    Ok(pristine.replacen(&mutation.old, &mutation.new, 1))
}

/// Parse the last `verification results:: N verified, M errors` line.
pub fn parse_verification(text: &str) -> Option<(usize, usize)> {
    let line = text
        .lines()
        .rev()
        .find(|line| line.contains("verification results::"))?;
    let rest = line.split("verification results::").nth(1)?.trim();
    let mut parts = rest.split_whitespace();
    let verified = parts.next()?.parse().ok()?;
    let _ = parts.next()?; // "verified,"
    let errors = parts.next()?.trim_end_matches(',').parse().ok()?;
    Some((verified, errors))
}

fn classify(
    text: &str,
    _returncode: Option<i32>,
    timed_out: bool,
) -> (Status, Option<usize>, Option<usize>) {
    if timed_out {
        (Status::TimedOut, None, None)
    } else if let Some((verified, errors)) = parse_verification(text) {
        if errors == 0 {
            (Status::Verified, Some(verified), Some(0))
        } else {
            (Status::Rejected, Some(verified), Some(errors))
        }
    } else if text.contains("error[E") || text.contains("could not compile") {
        (Status::CompileError, None, None)
    } else {
        (Status::Unknown, None, None)
    }
}

fn cache_status(text: &str, status: Status, returncode: Option<i32>) -> String {
    if status == Status::TimedOut {
        return "timeout".into();
    }
    if parse_verification(text).is_some() {
        return "fresh".into();
    }
    if returncode == Some(0) && text.contains("Finished") {
        return "cached".into();
    }
    "unknown".into()
}

fn first_error(text: &str) -> Option<String> {
    text.lines()
        .find(|line| {
            line.contains("error[E")
                || line.contains("error: postcondition")
                || line.contains("error: invariant")
                || line.contains("error: assertion")
                || line.contains("could not compile")
        })
        .map(|line| line.trim().chars().take(300).collect())
}

fn target_dir_for(manifest: &Manifest, focus: bool) -> PathBuf {
    if focus {
        return manifest.target_dir.clone();
    }
    manifest
        .full_target_dir
        .clone()
        .unwrap_or_else(|| manifest.target_dir.join("full"))
}

fn run_one(
    manifest: &Manifest,
    cargo_verus: &Path,
    z3: &Path,
    out: &Path,
    mutation_id: &str,
    focus: bool,
) -> Result<RunResult, String> {
    let mode = if focus { "focus" } else { "check" };
    let mut verus_args: Vec<&str> = Vec::new();
    if focus {
        verus_args.extend(["--verify-module", manifest.module.as_str()]);
    }
    let target_dir = target_dir_for(manifest, focus);
    let mut cmd = crate::probe_infra::target_command(
        cargo_verus,
        mode,
        &manifest.target_workspace,
        &manifest.package,
        &target_dir,
        manifest.target.as_ref(),
        &verus_args,
        z3,
    );
    let log_name = format!("{mutation_id}-{}.log", if focus { "focus" } else { "full" });
    let log_path = out.join("logs").join(&log_name);
    let started = Instant::now();
    let timeout = std::time::Duration::from_secs(manifest.timeout_seconds.max(1));
    let (exit_status, log, timed_out) =
        crate::probe_infra::run_captured_timeout(&mut cmd, &log_path, timeout)
            .map_err(|e| format!("run {log_name}: {e}"))?;
    let elapsed = started.elapsed().as_secs_f64();
    let returncode = exit_status.and_then(|s| s.code());
    let (status, verified, errors) = classify(&log, returncode, timed_out);
    Ok(RunResult {
        status,
        verified,
        errors,
        returncode,
        elapsed_seconds: (elapsed * 10.0).round() / 10.0,
        timed_out,
        cache_status: cache_status(&log, status, returncode),
        log: format!("logs/{log_name}"),
    })
}

/// Run the mutation manifest. The source file is restored before returning,
/// even when an individual mutant fails.
pub fn run(
    manifest: &Manifest,
    cargo_verus: &Path,
    z3: &Path,
    output: &Path,
) -> Result<MutationReport, String> {
    validate(manifest)?;
    fs::create_dir_all(output.join("logs")).map_err(|e| e.to_string())?;
    let source = manifest.target_workspace.join(&manifest.source);
    let original = fs::read(&source).map_err(|e| format!("{}: {e}", source.display()))?;
    let pristine = String::from_utf8(original.clone())
        .map_err(|e| format!("{} is not UTF-8: {e}", source.display()))?;
    let _guard = SourceGuard {
        path: source.clone(),
        original,
    };

    let baseline_focus = run_one(manifest, cargo_verus, z3, output, "baseline", true)?;
    let mut results = Vec::new();
    for mutation in &manifest.mutations {
        let mutated = apply_once(&pristine, mutation)?;
        fs::write(&source, mutated.as_bytes()).map_err(|e| e.to_string())?;
        let focus = run_one(manifest, cargo_verus, z3, output, &mutation.id, true)?;
        let (full, final_status) = if focus.status == Status::Verified && manifest.full_confirm {
            let full = run_one(manifest, cargo_verus, z3, output, &mutation.id, false)?;
            let status = if full.status == Status::Verified {
                "verified_survivor"
            } else if full.status == Status::Rejected {
                "focus_verified_full_rejected"
            } else if full.status == Status::TimedOut {
                "full_timeout"
            } else {
                "full_unknown"
            };
            (Some(full), status.to_string())
        } else {
            (
                None,
                match focus.status {
                    Status::Verified => "verified_survivor".to_string(),
                    Status::Rejected => "rejected".to_string(),
                    Status::CompileError => "compile_error".to_string(),
                    Status::TimedOut => "timeout".to_string(),
                    Status::Unknown => "unknown".to_string(),
                },
            )
        };
        let chosen_log = if let Some(full) = &full {
            &full.log
        } else {
            &focus.log
        };
        let text = fs::read_to_string(output.join(chosen_log)).unwrap_or_default();
        results.push(MutationResult {
            id: mutation.id.clone(),
            description: mutation.description.clone(),
            focus,
            full,
            final_status,
            first_error: first_error(&text),
        });
    }

    let verified_survivors = results
        .iter()
        .filter(|r| r.final_status == "verified_survivor")
        .count();
    let rejected = results
        .iter()
        .filter(|r| r.final_status == "rejected")
        .count();
    let compile_errors = results
        .iter()
        .filter(|r| r.final_status == "compile_error")
        .count();
    let timeouts = results
        .iter()
        .filter(|r| r.final_status == "timeout" || r.final_status == "full_timeout")
        .count();
    let unknown = results.len() - verified_survivors - rejected - compile_errors - timeouts;
    let mut jsonl = String::new();
    for result in &results {
        jsonl.push_str(&serde_json::to_string(result).map_err(|e| e.to_string())?);
        jsonl.push('\n');
    }
    fs::write(output.join("results.jsonl"), jsonl).map_err(|e| e.to_string())?;
    Ok(MutationReport {
        schema_version: REPORT_SCHEMA_VERSION.to_string(),
        target_workspace: manifest.target_workspace.clone(),
        source: manifest.source.clone(),
        package: manifest.package.clone(),
        module: manifest.module.clone(),
        baseline_focus,
        summary: Summary {
            mutants: results.len(),
            verified_survivors,
            rejected,
            compile_errors,
            timeouts,
            unknown,
        },
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mutation() -> Mutation {
        Mutation {
            id: "m1".into(),
            description: "test".into(),
            old: "a + b".into(),
            new: "a - b".into(),
        }
    }

    #[test]
    fn apply_requires_one_occurrence() {
        assert_eq!(apply_once("x = a + b;", &mutation()).unwrap(), "x = a - b;");
        assert!(apply_once("a + b; a + b;", &mutation()).is_err());
        assert!(apply_once("a + c;", &mutation()).is_err());
    }

    #[test]
    fn legacy_manifest_defaults_to_v1() {
        let raw = r#"{
            "target_workspace": "/ws",
            "source": "src/lib.rs",
            "package": "p",
            "module": "m",
            "target_dir": "/target",
            "mutations": [{"id": "m1", "old": "0", "new": "1"}]
        }"#;
        let manifest: Manifest = serde_json::from_str(raw).unwrap();
        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert!(manifest.full_confirm);
        assert_eq!(manifest.timeout_seconds, 900);
    }

    #[test]
    fn rejects_unknown_manifest_schema() {
        let mut manifest = Manifest {
            schema_version: "mutation.manifest.v99".into(),
            target_workspace: PathBuf::from("/ws"),
            source: "src/lib.rs".into(),
            package: "p".into(),
            module: "m".into(),
            target: None,
            target_dir: PathBuf::from("/target"),
            full_confirm: false,
            full_target_dir: None,
            timeout_seconds: 1,
            mutations: vec![mutation()],
        };
        assert!(validate(&manifest).is_err());
        manifest.schema_version = MANIFEST_SCHEMA_VERSION.into();
    }

    #[test]
    fn classify_timeout_and_cache_status() {
        let (status, _, _) = classify("no summary", Some(0), true);
        assert!(matches!(status, Status::TimedOut));
        assert_eq!(
            cache_status("Finished `dev` profile", Status::Unknown, Some(0)),
            "cached"
        );
        assert_eq!(
            cache_status(
                "verification results:: 1 verified, 0 errors",
                Status::Verified,
                Some(0)
            ),
            "fresh"
        );
    }

    #[test]
    fn parses_last_verification_line() {
        let text = "verification results:: 12 verified, 1 errors\n\
                    verification results:: 13 verified, 0 errors";
        assert_eq!(parse_verification(text), Some((13, 0)));
    }

    #[test]
    fn classifies_verified_and_rejected() {
        let (s, v, e) = classify(
            "verification results:: 13 verified, 0 errors",
            Some(0),
            false,
        );
        assert!(matches!(s, Status::Verified));
        assert_eq!((v, e), (Some(13), Some(0)));
        let (s, _, e) = classify(
            "verification results:: 12 verified, 1 errors",
            Some(1),
            false,
        );
        assert!(matches!(s, Status::Rejected));
        assert_eq!(e, Some(1));
    }

    #[test]
    fn full_confirmation_uses_a_separate_target_dir() {
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            target_workspace: PathBuf::from("/ws"),
            source: "src/lib.rs".into(),
            package: "p".into(),
            module: "m".into(),
            target: None,
            target_dir: PathBuf::from("/target"),
            full_confirm: true,
            full_target_dir: None,
            timeout_seconds: 900,
            mutations: vec![mutation()],
        };
        assert_eq!(target_dir_for(&manifest, true), PathBuf::from("/target"));
        assert_eq!(
            target_dir_for(&manifest, false),
            PathBuf::from("/target/full")
        );
        let mut explicit = manifest.clone();
        explicit.full_target_dir = Some(PathBuf::from("/full"));
        assert_eq!(target_dir_for(&explicit, false), PathBuf::from("/full"));
    }

    #[cfg(unix)]
    #[test]
    fn run_marks_timeouts_without_a_survivor() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "spec-check-mutation-timeout-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let workspace = root.join("workspace");
        let source = workspace.join("src/lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "fn f() { let x = 0; }\n").unwrap();
        let fake = root.join("cargo-verus");
        std::fs::write(&fake, "#!/bin/sh\nsleep 5\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            target_workspace: workspace,
            source: "src/lib.rs".into(),
            package: "p".into(),
            module: "m".into(),
            target: None,
            target_dir: root.join("target"),
            full_confirm: true,
            full_target_dir: None,
            timeout_seconds: 1,
            mutations: vec![Mutation {
                id: "m1".into(),
                description: "zero to one".into(),
                old: "0".into(),
                new: "1".into(),
            }],
        };
        let report = run(
            &manifest,
            &fake,
            Path::new("/nonexistent/z3"),
            &root.join("out"),
        )
        .expect("timeout run");
        assert!(matches!(report.baseline_focus.status, Status::TimedOut));
        assert!(matches!(report.results[0].focus.status, Status::TimedOut));
        assert_eq!(report.summary.timeouts, 1);
        assert_eq!(report.summary.verified_survivors, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn run_restores_source_and_confirms_survivor() {
        use std::os::unix::fs::PermissionsExt;

        let root =
            std::env::temp_dir().join(format!("spec-check-mutation-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let workspace = root.join("workspace");
        let source = workspace.join("src/lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        let pristine = "fn f() { let x = 0; }\n";
        std::fs::write(&source, pristine).unwrap();

        let fake = root.join("cargo-verus");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho 'verification results:: 5 verified, 0 errors'\nexit 0\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            target_workspace: workspace,
            source: "src/lib.rs".into(),
            package: "p".into(),
            module: "m".into(),
            target: None,
            target_dir: root.join("target"),
            full_confirm: true,
            full_target_dir: None,
            timeout_seconds: 900,
            mutations: vec![Mutation {
                id: "m1".into(),
                description: "zero to one".into(),
                old: "0".into(),
                new: "1".into(),
            }],
        };
        let report = run(
            &manifest,
            &fake,
            Path::new("/nonexistent/z3"),
            &root.join("out"),
        )
        .expect("mutation run");
        assert_eq!(report.summary.verified_survivors, 1);
        assert_eq!(report.results[0].final_status, "verified_survivor");
        assert!(report.results[0].full.is_some());
        assert_eq!(std::fs::read_to_string(&source).unwrap(), pristine);
        let _ = std::fs::remove_dir_all(&root);
    }
}
