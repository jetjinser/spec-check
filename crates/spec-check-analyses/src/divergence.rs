//! `test-contract-divergence`: obligation-contrast analysis over a probe
//! manifest (the complement of [`crate::vacuity`]).
//!
//! Vacuity asks whether the **requires** side of a contract is satisfiable;
//! divergence asks whether the **ensures** side is sufficient to support a
//! client obligation. A proxy client — an exec fn that uses the real API
//! (locally constructed receiver → call → obligation assertions) — is
//! injected into the target workspace and verified **under the original
//! contracts**:
//!
//! - the obligation proof fails while the target workspace's own
//!   verification (baseline) is clean → `ContractGapCandidate`: the
//!   contract does not let a verifier derive the client obligation. This is
//!   a proof-theoretic statement about the contract; it is **not** a runtime
//!   counterexample and does not establish an implementation defect.
//! - the obligation verifies (baseline clean) → `ObligationSupported`.
//! - probe compile errors, or the target verification not running →
//!   `Inconclusive` (an obligation that cannot be expressed is not a gap).
//! - contradictory evidence (baseline failures, failures outside the
//!   obligation fn) → `Anomaly` (manual inspection required).
//!
//! Interface: manifest with the same `target_workspace` shape as vacuity
//! v1.1; per-entry `obligation` (client fn body source) + optional
//! `model_preamble` (imports / helper verus source) + optional machine-
//! checkable pre-registration `expect` ("supported" | "gap"). Obligations
//! must be expressible on the frozen contract surface of the target —
//! obligation predicates introduced by project-side candidate patches are
//! project inputs, not tool capabilities.
//!
//! Injection, two-phase runner (rustc-check loop, then verified build),
//! replay copies, workspace restore, and the log model are shared with
//! vacuity via `crate::probe_infra`. All entries of a manifest are injected
//! into one build, so the baseline (target-crate errors outside the probe
//! modules) and the obligations are evidenced by the same log.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::probe_infra::{
    CEvent, CMarker, ProbeInjection, ProbeModule, TargetWorkspace, fn_region, forwarded_cfgs,
    indent_block, parse_target_log, run_captured, sanitize, target_command, write_injection,
};

/// Project-side obligation manifest: the client obligations to probe.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    /// The target workspace to inject the obligation clients into.
    pub target_workspace: TargetWorkspace,
    pub contracts: Vec<ObligationEntry>,
}

/// One obligation client entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObligationEntry {
    /// Stable subject identifier.
    pub name: String,
    /// Verus parameter list of the client fn (usually empty: obligations
    /// are self-contained, receivers constructed locally).
    #[serde(default)]
    pub params: String,
    /// Verus source emitted as the body of the client fn: the real API
    /// call plus the obligation assertions.
    pub obligation: String,
    /// Optional verus source (use items, helper spec fns) inserted after
    /// `verus! {` in the injected module.
    #[serde(default)]
    pub model_preamble: Option<String>,
    /// Machine-checkable pre-registration ("supported" | "gap"), recorded
    /// before the run. Reported as `expect_match`; never affects judgment.
    #[serde(default)]
    pub expect: Option<Expectation>,
}

/// Pre-registered expectation for an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expectation {
    /// The contract is expected to support the obligation.
    Supported,
    /// The contract is expected to fail to support the obligation.
    Gap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    ObligationSupported,
    ContractGapCandidate,
    Inconclusive,
    Anomaly,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryVerdict {
    pub name: String,
    pub verdict: Verdict,
    /// Pre-registered expectation, if any.
    pub expect: Option<Expectation>,
    /// Whether the verdict matched the pre-registered expectation
    /// (`None` when no expectation was registered).
    pub expect_match: Option<bool>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct DivergenceReport {
    pub verdicts: Vec<EntryVerdict>,
    /// Verification summary of the dependency build (e.g. vstd).
    pub dependency_summary: Option<String>,
    /// Verification summary of the target workspace crate.
    pub target_summary: Option<String>,
    /// Whether the target-crate baseline (verification outside the probe
    /// modules) was clean.
    pub baseline_clean: bool,
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.target_workspace.inject_dir.file_name().is_none() {
        return Err("target_workspace.inject_dir has no file name".into());
    }
    for c in &manifest.contracts {
        if c.obligation.trim().is_empty() {
            return Err(format!("obligation entry {} missing obligation", c.name));
        }
    }
    Ok(())
}

/// Generate the injected obligation-client module set. Pure: the classifier
/// regenerates this to map log spans.
pub fn generate_obligation_injection(manifest: &Manifest) -> Result<ProbeInjection, String> {
    let tw = &manifest.target_workspace;
    let module_name = tw
        .inject_dir
        .file_name()
        .map(|n| sanitize(&n.to_string_lossy()))
        .ok_or("target_workspace.inject_dir has no file name")?;
    let gate = &tw.gate_cfg;
    let mut modules = Vec::new();
    let mut mod_decls = String::new();
    for (i, c) in manifest.contracts.iter().enumerate() {
        let safe = sanitize(&c.name);
        if modules.iter().any(|m: &ProbeModule| m.safe_name == safe) {
            return Err(format!(
                "obligation entry name collides after sanitize: {}",
                c.name
            ));
        }

        let mut src = String::new();
        src.push_str(&format!(
            "// Generated by spec-check test-contract-divergence (obligation client probe).\n\
             // Entry: {} — injected under cfg({}); removed after the run; replay\n\
             // copy kept in the analysis output directory.\n\
             #![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n\n\
             use vstd::prelude::*;\n\n\
             verus! {{\n\n",
            c.name, gate,
        ));
        if let Some(mp) = &c.model_preamble
            && !mp.is_empty()
        {
            src.push_str(mp);
            src.push_str("\n\n");
        }
        src.push_str(&format!(
            "// ---- obligation client — do not edit ----\n\
             pub fn obligation_{i}_{safe}({params}) {{\n{obligation}\n}}\n\n",
            i = i,
            safe = safe,
            params = c.params,
            obligation = indent_block(&c.obligation),
        ));
        src.push_str("} // verus!\n");

        let lines: Vec<&str> = src.lines().collect();
        let entry_region = fn_region(&lines, &format!("pub fn obligation_{i}_{safe}"))
            .ok_or_else(|| format!("internal: obligation fn region not found for {}", c.name))?;

        mod_decls.push_str(&format!("#[cfg({gate})]\npub mod {safe};\n\n", gate = gate));

        modules.push(ProbeModule {
            safe_name: safe,
            index: i,
            source: src,
            entry_region,
            live_region: None,
        });
    }
    if modules.is_empty() {
        return Err("no obligation entries in manifest".into());
    }
    let mod_rs = format!(
        "// Generated by spec-check test-contract-divergence (obligation client probes).\n\
         // Removed after the run; replay copy kept in the analysis output directory.\n\
         #![allow(dead_code, unused_imports)]\n\n\
         {mod_decls}"
    );
    Ok(ProbeInjection {
        lib_decl: format!("#[cfg({gate})] mod {module_name};", gate = gate),
        module_name,
        mod_rs,
        modules,
    })
}

/// Run the two-phase probe build and return per-obligation verdicts.
pub fn run(
    manifest: &Manifest,
    cargo_verus: &Path,
    z3: &Path,
    vstd: &Path,
    out: &Path,
) -> Result<DivergenceReport, String> {
    let _ = vstd; // shared signature with vacuity::run; obligations build in the target workspace
    validate_manifest(manifest)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let injection = generate_obligation_injection(manifest)?;
    let (guard, target) = write_injection(
        &manifest.target_workspace,
        out,
        &injection,
        "test-contract-divergence (obligation client probes)",
    )?;

    let tw = &manifest.target_workspace;
    let cfg_args = forwarded_cfgs(tw);
    let mut check_args = vec!["--no-verify"];
    check_args.extend(cfg_args.iter().map(|s| s.as_str()));
    let build_args = cfg_args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let target_triple = tw.target.as_ref();
    let precheck_log = out.join("precheck-build.log");
    let build_log = out.join("target-build.log");

    let run_result = (|| -> Result<DivergenceReport, String> {
        // Phase 1: rustc-check loop. A non-compiling obligation client is
        // an Inconclusive entry (inexpressible obligation), not a defect —
        // report it per entry instead of failing the whole run.
        let mut phase1 = target_command(
            cargo_verus,
            "check",
            &target.workspace_root,
            &target.crate_name,
            &out.join("target-check"),
            target_triple,
            &check_args,
            z3,
        );
        let (status, log1) = run_captured(&mut phase1, &precheck_log)?;
        if !status.success() {
            let mut report = classify(manifest, "");
            for v in &mut report.verdicts {
                v.verdict = Verdict::Inconclusive;
                v.detail = format!(
                    "obligation client did not compile in the precheck loop; see {}; \
                     first failure: {}",
                    precheck_log.display(),
                    first_error_line(&log1),
                );
            }
            return Ok(report);
        }

        // Phase 2: verified cargo-verus build of the target workspace.
        // A nonzero exit is expected when an obligation fails — the log,
        // not the exit code, is the evidence.
        let mut phase2 = target_command(
            cargo_verus,
            "build",
            &target.workspace_root,
            &target.crate_name,
            &out.join("target-verify"),
            target_triple,
            &build_args,
            z3,
        );
        let (_, log2) = run_captured(&mut phase2, &build_log)?;
        Ok(classify(manifest, &log2))
    })();

    drop(guard); // restore workspace regardless of outcome
    run_result
}

fn first_error_line(log: &str) -> String {
    log.lines()
        .find(|l| l.starts_with("error"))
        .unwrap_or("no error line captured")
        .to_string()
}

fn module_events<'a>(events: &'a [CEvent], module_suffix: &str) -> Vec<&'a CEvent> {
    events
        .iter()
        .filter(|e| {
            e.file.ends_with(module_suffix) || e.file == module_suffix.trim_start_matches('/')
        })
        .collect()
}

fn span_note(events: &[&CEvent]) -> String {
    events
        .first()
        .map(|e| format!("{}:{}", e.file, e.line))
        .unwrap_or_else(|| "-".into())
}

/// Classify obligation entries from the target-workspace verus build log.
pub fn classify(manifest: &Manifest, log: &str) -> DivergenceReport {
    let injection = match generate_obligation_injection(manifest) {
        Ok(inj) => inj,
        Err(e) => {
            return DivergenceReport {
                verdicts: manifest
                    .contracts
                    .iter()
                    .map(|c| EntryVerdict {
                        name: c.name.clone(),
                        verdict: Verdict::Inconclusive,
                        expect: c.expect,
                        expect_match: None,
                        detail: format!("injection generation failed: {e}"),
                    })
                    .collect(),
                dependency_summary: None,
                target_summary: None,
                baseline_clean: false,
            };
        }
    };
    let crate_name = manifest
        .target_workspace
        .inject_dir
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .unwrap_or_default();
    let module_dir_prefix = format!("{}/", crate_name);

    let (events, summaries, crate_sections) = parse_target_log(log);
    let target_ran = crate_sections
        .get(&crate_name)
        .map(|sec| summaries.iter().any(|(s, _)| s == sec))
        .unwrap_or(false);
    let target_summary = crate_sections
        .get(&crate_name)
        .and_then(|sec| {
            summaries
                .iter()
                .filter(|(s, _)| s == sec)
                .map(|(_, t)| t.clone())
                .next_back()
        })
        .or_else(|| summaries.last().map(|(_, t)| t.clone()));
    let dependency_summary = if summaries.len() > 1 {
        Some(summaries[0].1.clone())
    } else {
        None
    };

    // Baseline: target-crate error events outside the probe modules.
    let probe_suffixes: Vec<String> = injection
        .modules
        .iter()
        .map(|m| format!("/{}.rs", m.safe_name))
        .collect();
    let baseline_events: Vec<&CEvent> = events
        .iter()
        .filter(|e| e.file.starts_with(&module_dir_prefix))
        .filter(|e| !probe_suffixes.iter().any(|s| e.file.ends_with(s.as_str())))
        .collect();
    let baseline_failed = !baseline_events.is_empty();

    let mut verdicts = Vec::new();
    for m in &injection.modules {
        let c = &manifest.contracts[m.index];
        let suffix = format!("/{}.rs", m.safe_name);
        let all_module = module_events(&events, &suffix);
        let compile_errors: Vec<&CEvent> = all_module
            .iter()
            .copied()
            .filter(|e| e.marker == CMarker::Other)
            .collect();
        let obligation_events: Vec<&CEvent> = all_module
            .iter()
            .copied()
            .filter(|e| e.line >= m.entry_region.0 && e.line <= m.entry_region.1)
            .collect();
        let obligation_failed = obligation_events.iter().any(|e| e.marker != CMarker::Other);
        let outside_obligation_failed = all_module.iter().any(|e| {
            e.marker != CMarker::Other && (e.line < m.entry_region.0 || e.line > m.entry_region.1)
        });

        let (verdict, reason) = if !target_ran {
            (
                Verdict::Inconclusive,
                format!(
                    "target verification did not run (crate section has no verification summary); \
                     compile errors in module: {}",
                    span_note(&compile_errors),
                ),
            )
        } else if !compile_errors.is_empty() {
            (
                Verdict::Inconclusive,
                format!(
                    "obligation client compile error (inexpressible obligation is not a contract gap); \
                     first at {}",
                    span_note(&compile_errors),
                ),
            )
        } else if baseline_failed {
            (
                Verdict::Anomaly,
                format!(
                    "baseline verification failed (target errors outside probe modules, e.g. {}); \
                     manual inspection required",
                    span_note(&baseline_events),
                ),
            )
        } else if obligation_failed && !outside_obligation_failed {
            (
                Verdict::ContractGapCandidate,
                format!(
                    "obligation failed under the original contracts (failure attributed to the \
                     obligation fn at {}); baseline clean — the contract does not derive the \
                     client obligation (proof-theoretic gap candidate, not a runtime counterexample)",
                    span_note(&obligation_events),
                ),
            )
        } else if !obligation_failed && !outside_obligation_failed {
            (
                Verdict::ObligationSupported,
                "obligation verified under the original contracts; baseline clean".into(),
            )
        } else if !obligation_failed && outside_obligation_failed {
            (
                Verdict::Inconclusive,
                format!(
                    "obligation fn clean but the probe module has failures outside it at {} \
                     (harness not clean); manual inspection required",
                    span_note(
                        &all_module
                            .iter()
                            .copied()
                            .filter(|e| e.marker != CMarker::Other)
                            .collect::<Vec<_>>(),
                    ),
                ),
            )
        } else {
            (
                Verdict::Anomaly,
                format!(
                    "obligation failed at {} while the probe module has additional failures at {} — \
                     contradictory evidence, manual inspection required",
                    span_note(&obligation_events),
                    span_note(
                        &all_module
                            .iter()
                            .copied()
                            .filter(|e| e.marker != CMarker::Other)
                            .collect::<Vec<_>>(),
                    ),
                ),
            )
        };

        let expect_match = c.expect.map(|e| {
            matches!(
                (e, verdict),
                (Expectation::Supported, Verdict::ObligationSupported)
                    | (Expectation::Gap, Verdict::ContractGapCandidate)
            )
        });
        verdicts.push(EntryVerdict {
            name: c.name.clone(),
            verdict,
            expect: c.expect,
            expect_match,
            detail: reason,
        });
    }

    DivergenceReport {
        verdicts,
        dependency_summary,
        target_summary,
        baseline_clean: target_ran && !baseline_failed,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        serde_json::from_str(
            r#"{
              "target_workspace": {
                "manifest_path": "/tmp/target/Cargo.toml",
                "inject_dir": "ostd/src/spec_check_probes",
                "gate_cfg": "ktest"
              },
              "contracts": [
                {
                  "name": "guard_identity",
                  "params": "",
                  "obligation": "let m = make();\nlet g = m.lock();\nassert(g.rel(&m));",
                  "model_preamble": "use crate::sync::Mutex;",
                  "expect": "gap"
                },
                {
                  "name": "supported_rel",
                  "params": "",
                  "obligation": "let m = make();\nif let Some(g) = m.try_lock() { assert(g.rel(&m)); }"
                },
                {
                  "name": "1bad_name",
                  "obligation": "typoed_call();"
                }
              ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn obligation_entry_defaults_and_generation() {
        let m = manifest();
        assert_eq!(m.contracts[0].expect, Some(Expectation::Gap));
        assert_eq!(m.contracts[1].expect, None);
        assert_eq!(m.contracts[2].obligation, "typoed_call();");

        let inj = generate_obligation_injection(&m).unwrap();
        assert_eq!(inj.module_name, "spec_check_probes");
        assert_eq!(inj.lib_decl, "#[cfg(ktest)] mod spec_check_probes;");
        assert_eq!(inj.modules.len(), 3);
        assert!(
            inj.mod_rs
                .contains("#[cfg(ktest)]\npub mod guard_identity;")
        );
        // leading digit sanitized
        assert_eq!(inj.modules[2].safe_name, "p_1bad_name");

        let src = &inj.modules[0].source;
        assert!(src.contains("use crate::sync::Mutex;"));
        assert!(src.contains("pub fn obligation_0_guard_identity() {"));
        assert!(src.contains("assert(g.rel(&m));"));
        let lines: Vec<&str> = src.lines().collect();
        assert_eq!(
            lines[inj.modules[0].entry_region.0 - 1],
            "pub fn obligation_0_guard_identity() {"
        );
        assert_eq!(lines[inj.modules[0].entry_region.1 - 1], "}");
        assert!(inj.modules[0].live_region.is_none());
    }

    fn log_for(m: &Manifest, events: &[(usize, &str, usize)], summary: Option<&str>) -> String {
        let inj = generate_obligation_injection(m).unwrap();
        let mut out = String::from(
            "   Compiling vstd v0.1.0 (/tmp/tools/vstd)\nverification results:: 2060 verified, 0 errors\n   Compiling ostd v0.1.0 (/tmp/target/ostd)\n",
        );
        for (module, kind, line) in events {
            let file = format!(
                "ostd/src/spec_check_probes/{}.rs",
                inj.modules[*module].safe_name
            );
            let marker = match *kind {
                "assert" => "error: assertion failed",
                "pre" => "error: precondition not satisfied",
                "post" => "error: postcondition not satisfied",
                _ => "error[E0433]: cannot find module `nope`",
            };
            out.push_str(&format!("{marker}\n   --> {file}:{line}:5\n"));
        }
        if let Some(s) = summary {
            out.push_str(&format!("verification results:: {s}\n"));
        }
        out
    }

    #[test]
    fn validation_rejects_missing_obligation() {
        let m: Manifest = serde_json::from_str(
            r#"{
              "target_workspace": {
                "manifest_path": "/tmp/target/Cargo.toml",
                "inject_dir": "ostd/src/spec_check_probes",
                "gate_cfg": "ktest"
              },
              "contracts": [{"name": "empty", "params": "", "obligation": "  "}]
            }"#,
        )
        .unwrap();
        assert!(validate_manifest(&m).is_err());
        let err = run(
            &m,
            Path::new("cargo-verus"),
            Path::new("z3"),
            Path::new("vstd"),
            &std::env::temp_dir().join("spec-check-divergence-test-out"),
        )
        .unwrap_err();
        assert!(err.contains("missing obligation"));
    }

    #[test]
    fn classification_matrix() {
        let m = manifest();
        let inj = generate_obligation_injection(&m).unwrap();
        let gap_line = inj.modules[0].entry_region.0 + 1;

        // gap + supported + baseline clean
        let log = log_for(
            &m,
            &[(0, "assert", gap_line)],
            Some("1500 verified, 1 errors"),
        );
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::ContractGapCandidate);
        assert_eq!(r.verdicts[0].expect_match, Some(true));
        assert!(
            r.verdicts[0]
                .detail
                .contains(&format!("guard_identity.rs:{gap_line}"))
        );
        assert_eq!(r.verdicts[1].verdict, Verdict::ObligationSupported);
        assert_eq!(r.verdicts[1].expect_match, None);
        assert!(r.baseline_clean);
        assert_eq!(r.target_summary.as_deref(), Some("1500 verified, 1 errors"));
        assert_eq!(
            r.dependency_summary.as_deref(),
            Some("2060 verified, 0 errors")
        );

        // baseline failure (target-crate error outside probe modules) → anomaly
        let log = format!(
            "{}\nerror: postcondition not satisfied\n   --> ostd/src/sync/mutex.rs:80:9\n\nverification results:: 1499 verified, 2 errors\n",
            log_for(&m, &[(0, "assert", gap_line)], None)
        );
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::Anomaly);
        assert!(r.verdicts[0].detail.contains("baseline"));
        assert_eq!(r.verdicts[1].verdict, Verdict::Anomaly);
        assert!(!r.baseline_clean);

        // compile error in one probe module (E0433 = CMarker::Other)
        let log = log_for(&m, &[(2, "compile", 3)], Some("1499 verified, 1 errors"));
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[2].verdict, Verdict::Inconclusive);
        assert!(r.verdicts[2].detail.contains("compile error"));

        // no summary in the target section → not run → inconclusive
        let log = log_for(&m, &[], None);
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::Inconclusive);
        assert!(r.verdicts[0].detail.contains("did not run"));

        // obligation clean but failure elsewhere in its module → inconclusive
        let preamble_line = inj.modules[0].entry_region.0 - 2;
        let log = log_for(
            &m,
            &[(0, "post", preamble_line)],
            Some("1499 verified, 1 errors"),
        );
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::Inconclusive);
        assert!(r.verdicts[0].detail.contains("harness not clean"));

        // obligation + additional failure in module → anomaly
        let log = log_for(
            &m,
            &[(0, "assert", gap_line), (0, "post", preamble_line)],
            Some("1498 verified, 2 errors"),
        );
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::Anomaly);

        // supported expectation mismatch is reported, not rewritten
        let mut m2 = manifest();
        m2.contracts[0].expect = Some(Expectation::Supported);
        let log = log_for(
            &m2,
            &[(0, "assert", gap_line)],
            Some("1500 verified, 1 errors"),
        );
        let r = classify(&m2, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::ContractGapCandidate);
        assert_eq!(r.verdicts[0].expect_match, Some(false));

        // rustc events outside the target crate are not baseline failures
        let log = "   Compiling vstd v0.1.0 (/tmp/tools/vstd)\nerror[E0433]: cannot find module `nope`\n   --> vstd/src/some.rs:3:5\n   Compiling ostd v0.1.0 (/tmp/target/ostd)\nverification results:: 1501 verified, 0 errors\n".to_string();
        let r = classify(&m, &log);
        assert!(r.baseline_clean);
        assert_eq!(r.verdicts[0].verdict, Verdict::ObligationSupported);
        assert_eq!(r.verdicts[1].verdict, Verdict::ObligationSupported);
    }
}
