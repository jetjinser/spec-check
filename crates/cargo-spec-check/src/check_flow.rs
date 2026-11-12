//! `check` entry flow (docs/ANALYSIS-CONTRACT.md v1): capture orchestration
//! plus declarative dispatch to registered analyses.
//!
//! Flow and analyses are decoupled: analyses are dispatched by name through
//! [`registry`]. Adding an analysis means a CONFIG declaration, an adapter
//! here aligning the analysis' native signature to the uniform runner
//! interface, and one registry line — the orchestration below never changes.
//! The first adapter wraps `spec_check_analyses::vacuity::run`; the second
//! wraps `spec_check_analyses::divergence::run` (same native signature).
//!
//! Live-workspace injection semantics (shared by vacuity construction
//! probes and divergence obligation probes, docs/ANALYSIS-CONTRACT.md
//! §注入语义): both analyses declare their own `target_workspace` inside
//! their manifest and inject probe modules into that working copy for the
//! duration of their run, restoring the original bytes afterwards (guard
//! drop restores even on failure). Analyses declared in one CONFIG run
//! sequentially in declaration order; the capture step itself never
//! mutates the target workspace, so every analysis sees the same
//! byte-frozen source state the capture recorded. Findings never alter the
//! workspace persistently and never fail the pipeline.
//!
//! Exit-code contract: only pipeline failures (config, capture, tool,
//! unknown analysis) return `Err` (non-zero via `main`); findings
//! (vacuous/gap/anomaly verdicts) only shape the report.
//!
//! Adapter-layer note (wt-a boundary): `spec-check-analyses` ships a
//! `registry::builtin_registry` keyed on the `Analysis` trait, but the
//! concrete analysis `run` fns — frozen for wt-b, must not be modified —
//! do not implement that trait and carry toolchain paths instead of
//! captured artifacts. Adapting them on the wt-a side keeps
//! `crates/spec-check-analyses/` untouched.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spec_check_analyses::divergence;
use spec_check_analyses::vacuity;
use spec_check_core::project;
use spec_check_core::support::write_json;
use spec_check_core::Result;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Project-side check configuration (the `--project CONFIG` JSON).
#[derive(Deserialize)]
pub struct ProjectConfig {
    pub capture: CaptureSpec,
    pub analyses: Vec<AnalysisSpec>,
}

/// `project-capture` parameters plus the cached capture output directory.
#[derive(Deserialize)]
pub struct CaptureSpec {
    /// Capture plan JSON (`project-capture --config`): the profiles.
    pub config: PathBuf,
    /// Target workspace root manifest (`project-capture --manifest-path`).
    pub manifest_path: PathBuf,
    /// SDK directory holding `cargo-verus` (`project-capture --verus-root`).
    pub verus_root: PathBuf,
    /// Capture output directory. Fresh (recorded `changed_inputs` empty,
    /// `status` captured) captures are reused; missing or stale ones are
    /// (re)captured.
    pub out: PathBuf,
}

/// One declared analysis: registry name plus its project-side manifest.
#[derive(Deserialize)]
pub struct AnalysisSpec {
    pub name: String,
    pub manifest: PathBuf,
}

/// Toolchain and directory context handed to every analysis runner.
pub struct AnalysisContext {
    pub capture_out: PathBuf,
    pub cargo_verus: PathBuf,
    pub z3: PathBuf,
    pub vstd: PathBuf,
    pub check_out: PathBuf,
}

/// What a registered analysis must produce: machine report plus the
/// human-readable per-contract verdicts and reproduction commands.
#[derive(Serialize)]
pub struct AnalysisOutcome {
    pub name: String,
    pub manifest: PathBuf,
    pub out: PathBuf,
    pub report: Value,
    pub summary: Vec<String>,
    pub repro: Vec<String>,
}

pub type AnalysisRunner =
    fn(&AnalysisSpec, &AnalysisContext) -> std::result::Result<AnalysisOutcome, String>;

/// Analysis registry: stable CONFIG name → runner. New analyses register
/// here (one line); the orchestration in [`run`] is analysis-agnostic.
pub fn registry() -> BTreeMap<&'static str, AnalysisRunner> {
    BTreeMap::from([
        ("vacuity", adapters::vacuity as AnalysisRunner),
        ("divergence", adapters::divergence as AnalysisRunner),
    ])
}

mod adapters {
    //! Per-analysis adapters: native analysis signatures aligned to the
    //! uniform [`super::AnalysisRunner`] interface.

    use super::*;

    pub(super) fn vacuity(
        spec: &AnalysisSpec,
        ctx: &AnalysisContext,
    ) -> std::result::Result<AnalysisOutcome, String> {
        let raw = fs::read(&spec.manifest).map_err(|e| e.to_string())?;
        let manifest: vacuity::Manifest =
            serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", spec.manifest.display()))?;
        let out = ctx.check_out.join("analyses").join(&spec.name);
        let report = vacuity::run(&manifest, &ctx.cargo_verus, &ctx.z3, &ctx.vstd, &out)?;
        write_json(&out.join("report.json"), &report).map_err(|e| e.to_string())?;
        let outcome = AnalysisOutcome {
            name: spec.name.clone(),
            manifest: spec.manifest.clone(),
            out,
            report: serde_json::to_value(&report).map_err(|e| e.to_string())?,
            summary: report
                .verdicts
                .iter()
                .map(|v| {
                    format!(
                        "{}: {:?}  ({}) {}",
                        v.name,
                        v.verdict,
                        v.detail,
                        super::verdict_hint(&format!("{:?}", v.verdict))
                    )
                })
                .collect(),
            repro: vec![format!(
                "cargo-spec-check analyses-vacuity --manifest {} --cargo-verus {} --z3 {} --vstd {} --out NEW_DIRECTORY",
                spec.manifest.display(),
                ctx.cargo_verus.display(),
                ctx.z3.display(),
                ctx.vstd.display(),
            )],
        };
        Ok(outcome)
    }

    pub(super) fn divergence(
        spec: &AnalysisSpec,
        ctx: &AnalysisContext,
    ) -> std::result::Result<AnalysisOutcome, String> {
        let raw = fs::read(&spec.manifest).map_err(|e| e.to_string())?;
        let manifest: divergence::Manifest =
            serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", spec.manifest.display()))?;
        let out = ctx.check_out.join("analyses").join(&spec.name);
        let report = divergence::run(&manifest, &ctx.cargo_verus, &ctx.z3, &ctx.vstd, &out)?;
        write_json(&out.join("report.json"), &report).map_err(|e| e.to_string())?;
        let outcome = AnalysisOutcome {
            name: spec.name.clone(),
            manifest: spec.manifest.clone(),
            out,
            report: serde_json::to_value(&report).map_err(|e| e.to_string())?,
            summary: report
                .verdicts
                .iter()
                .map(|v| {
                    format!(
                        "{}: {:?}  ({}) {}",
                        v.name,
                        v.verdict,
                        v.detail,
                        super::verdict_hint(&format!("{:?}", v.verdict)))
                })
                .collect(),
            repro: vec![format!(
                "cargo-spec-check analyses-divergence --manifest {} --cargo-verus {} --z3 {} --vstd {} --out NEW_DIRECTORY",
                spec.manifest.display(),
                ctx.cargo_verus.display(),
                ctx.z3.display(),
                ctx.vstd.display(),
            )],
        };
        Ok(outcome)
    }
}

/// One-line interpretation of a contract verdict, per the contract's
/// classification rules. Input is the variant name, e.g. `ConfirmedVacuous`.
pub fn verdict_hint(verdict: &str) -> &'static str {
    match verdict {
        "ConfirmedVacuous" => "confirmed vacuous: precondition unsatisfiable",
        "ConfirmedLive" => "confirmed live: precondition satisfiable",
        "CandidateInconclusive" => "candidate: verifier inconclusive; human review, not a defect",
        "ObligationSupported" => "obligation supported: contract derives the client obligation",
        "ContractGapCandidate" => {
            "contract gap candidate: obligation not derivable under the original contracts; \
             proof-theoretic, not a runtime counterexample"
        },
        "Inconclusive" => "inconclusive: obligation not decidable; human review, not a defect",
        "Anomaly" => "anomaly: contradictory probes; manual review required",
        _ => "",
    }
}

/// Capture freshness per the contract: an existing output directory whose
/// recorded report says `captured` with empty `changed_inputs` is reused;
/// anything else is stale and must be re-captured.
#[derive(Debug, PartialEq, Eq)]
pub enum CaptureFreshness {
    Fresh,
    Missing,
    Stale(String),
}

pub fn capture_freshness(out: &Path) -> Result<CaptureFreshness> {
    if !out.exists() {
        return Ok(CaptureFreshness::Missing);
    }
    if !out.is_dir() {
        return Ok(CaptureFreshness::Stale("not a directory".into()));
    }
    let report = out.join("report.json");
    if !report.is_file() {
        return Ok(CaptureFreshness::Stale("report.json missing".into()));
    }
    let raw = fs::read(&report).map_err(|e| e.to_string())?;
    let value: Value = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Ok(CaptureFreshness::Stale(format!(
                "report.json unreadable: {e}"
            )))
        },
    };
    if value["status"] != json!("captured") {
        return Ok(CaptureFreshness::Stale(format!(
            "capture status {}",
            value["status"]
        )));
    }
    let changed = value["changed_inputs"].as_array().map(Vec::len).unwrap_or(0);
    if changed > 0 {
        return Ok(CaptureFreshness::Stale(format!("{changed} changed inputs")));
    }
    Ok(CaptureFreshness::Fresh)
}

fn verdict_counts(outcomes: &[AnalysisOutcome]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for outcome in outcomes {
        for verdict in outcome.report["verdicts"].as_array().into_iter().flatten() {
            if let Some(name) = verdict["verdict"].as_str() {
                *counts.entry(name.to_owned()).or_default() += 1;
            }
        }
    }
    counts
}

/// Run the full `check` pipeline: capture (cached) then declared analyses.
pub fn run(
    config_path: &Path,
    cargo_verus: &Path,
    z3: &Path,
    vstd: &Path,
    out: &Path,
    json_mode: bool,
) -> Result<()> {
    let config_path = fs::canonicalize(config_path)?;
    let config: ProjectConfig = serde_json::from_slice(&fs::read(&config_path)?)?;
    let base = config_path
        .parent()
        .ok_or("project config has no parent directory")?
        .to_path_buf();
    let resolve = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base.join(p)
        }
    };
    let capture = CaptureSpec {
        config: resolve(&config.capture.config),
        manifest_path: resolve(&config.capture.manifest_path),
        verus_root: resolve(&config.capture.verus_root),
        out: resolve(&config.capture.out),
    };
    for (label, path) in [
        ("capture.config", &capture.config),
        ("capture.manifest_path", &capture.manifest_path),
        ("capture.verus_root", &capture.verus_root),
    ] {
        if !path.exists() {
            return Err(format!("{label} does not exist: {}", path.display()).into());
        }
    }
    for (label, path, expect_dir) in [
        ("--cargo-verus", cargo_verus, false),
        ("--z3", z3, false),
        ("--vstd", vstd, true),
    ] {
        let present = if expect_dir {
            path.is_dir()
        } else {
            path.is_file()
        };
        if !present {
            return Err(format!("{label} does not exist: {}", path.display()).into());
        }
    }
    fs::create_dir_all(out)?;
    let check_out = fs::canonicalize(out)?;

    let (reused, recaptured_stale_reason) = match capture_freshness(&capture.out)? {
        CaptureFreshness::Fresh => (true, None),
        CaptureFreshness::Missing => {
            project::run(
                &capture.config,
                &capture.manifest_path,
                &capture.verus_root,
                &capture.out,
            )?;
            (false, None)
        },
        CaptureFreshness::Stale(reason) => {
            fs::remove_dir_all(&capture.out)?;
            project::run(
                &capture.config,
                &capture.manifest_path,
                &capture.verus_root,
                &capture.out,
            )?;
            (false, Some(reason))
        },
    };

    let registry = registry();
    let ctx = AnalysisContext {
        capture_out: capture.out.clone(),
        cargo_verus: cargo_verus.to_path_buf(),
        z3: z3.to_path_buf(),
        vstd: vstd.to_path_buf(),
        check_out: check_out.clone(),
    };
    let mut outcomes = Vec::new();
    for spec in &config.analyses {
        let runner = registry
            .get(spec.name.as_str())
            .ok_or_else(|| {
                format!(
                    "unknown analysis '{}'; registered: {}",
                    spec.name,
                    registry
                        .keys()
                        .map(|s| s.to_owned())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        let manifest = resolve(&spec.manifest);
        if !manifest.is_file() {
            return Err(format!(
                "analysis {} manifest does not exist: {}",
                spec.name,
                manifest.display()
            )
            .into());
        }
        let spec = AnalysisSpec {
            name: spec.name.clone(),
            manifest,
        };
        outcomes.push(runner(&spec, &ctx).map_err(|e| format!("analysis {} failed: {e}", spec.name))?);
    }

    let report = json!({
        "tool": "cargo-spec-check check",
        "contract": "ANALYSIS-CONTRACT v1",
        "project_config": config_path,
        "capture": {
            "out": capture.out,
            "reused": reused,
            "recaptured_stale_reason": recaptured_stale_reason,
            "config": capture.config,
            "manifest_path": capture.manifest_path,
            "verus_root": capture.verus_root,
        },
        "analyses": outcomes,
        "summary": {
            "analyses": outcomes.len(),
            "verdicts": verdict_counts(&outcomes),
        },
    });
    write_json(&check_out.join("report.json"), &report)?;
    if json_mode {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!(
            "== capture: {}{}",
            if reused {
                "reused fresh"
            } else {
                "ran project-capture"
            },
            match recaptured_stale_reason {
                Some(reason) => format!(" (stale: {reason}; re-captured)"),
                None => String::new(),
            }
        );
        println!("== capture directory: {}", capture.out.display());
        for outcome in &outcomes {
            println!("== analysis {} (manifest {})", outcome.name, outcome.manifest.display());
            for line in &outcome.summary {
                println!("  {line}");
            }
            for repro in &outcome.repro {
                println!("  repro: {repro}");
            }
        }
        println!(
            "== verdict totals: {}",
            verdict_counts(&outcomes)
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("report: {}", check_out.join("report.json").display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spec-check-flow-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn capture_freshness_follows_recorded_state() {
        let absent = scratch("freshness-absent");
        fs::remove_dir(&absent).unwrap();
        assert_eq!(capture_freshness(&absent).unwrap(), CaptureFreshness::Missing);
        let dir = scratch("freshness");
        assert!(matches!(
            capture_freshness(&dir).unwrap(),
            CaptureFreshness::Stale(_)
        ));
        let report = dir.join("report.json");
        fs::write(
            &report,
            json!({"status":"captured","changed_inputs":[]}).to_string(),
        )
        .unwrap();
        assert_eq!(capture_freshness(&dir).unwrap(), CaptureFreshness::Fresh);
        fs::write(
            &report,
            json!({"status":"incomplete","changed_inputs":[]}).to_string(),
        )
        .unwrap();
        assert!(matches!(
            capture_freshness(&dir).unwrap(),
            CaptureFreshness::Stale(_)
        ));
        fs::write(
            &report,
            json!({"status":"captured","changed_inputs":["/x"]}).to_string(),
        )
        .unwrap();
        assert!(matches!(
            capture_freshness(&dir).unwrap(),
            CaptureFreshness::Stale(_)
        ));
        fs::write(&report, "not json").unwrap();
        assert!(matches!(
            capture_freshness(&dir).unwrap(),
            CaptureFreshness::Stale(_)
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn project_config_parses_declarations_and_relative_paths() {
        let config: ProjectConfig = serde_json::from_str(
            r#"{
              "capture": {
                "config": "capture.json",
                "manifest_path": "crate/Cargo.toml",
                "verus_root": "/abs/sdk",
                "out": "/abs/capture-out"
              },
              "analyses": [
                {"name": "vacuity", "manifest": "manifest.json"},
                {"name": "divergence", "manifest": "divergence-manifest.json"}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(config.analyses.len(), 2);
        assert_eq!(config.analyses[1].name, "divergence");
        assert_eq!(config.capture.config, PathBuf::from("capture.json"));
    }

    #[test]
    fn registry_resolves_vacuity_and_rejects_unknown_names() {
        let registry = registry();
        assert!(registry.contains_key("vacuity"));
        assert!(registry.contains_key("divergence"));
        assert!(registry.get("construction").is_none());
    }

    #[test]
    fn verdict_hint_covers_all_registered_analysis_verdicts() {
        for verdict in [
            "ConfirmedVacuous",
            "ConfirmedLive",
            "CandidateInconclusive",
            "ObligationSupported",
            "ContractGapCandidate",
            "Inconclusive",
            "Anomaly",
        ] {
            assert!(!verdict_hint(verdict).is_empty(), "missing hint: {verdict}");
        }
        assert!(verdict_hint("SomethingElse").is_empty());
    }

    #[test]
    fn verdict_counts_only_tally_verdict_shaped_reports() {
        let vacuity_shaped = AnalysisOutcome {
            name: "vacuity".into(),
            manifest: PathBuf::from("m"),
            out: PathBuf::from("o"),
            report: json!({"verdicts": [
                {"name": "a", "verdict": "ConfirmedVacuous", "detail": ""},
                {"name": "b", "verdict": "ConfirmedLive", "detail": ""},
                {"name": "c", "verdict": "ConfirmedVacuous", "detail": ""}
            ]}),
            summary: vec![],
            repro: vec![],
        };
        let other = AnalysisOutcome {
            name: "future".into(),
            manifest: PathBuf::from("m"),
            out: PathBuf::from("o"),
            report: json!({"findings": []}),
            summary: vec![],
            repro: vec![],
        };
        let counts = verdict_counts(&[vacuity_shaped, other]);
        assert_eq!(counts.get("ConfirmedVacuous"), Some(&2));
        assert_eq!(counts.get("ConfirmedLive"), Some(&1));
        assert_eq!(counts.len(), 2);
    }
}
