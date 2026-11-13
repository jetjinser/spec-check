//! `contract-vacuity`: generic vacuity analysis over a probe manifest.
//!
//! Interface contract: `docs/ANALYSIS-CONTRACT.md` (manifest v1.1 + two-phase
//! construction-probe runner + per-probe-kind classification).
//!
//! Phase-1 scope (honest boundary): the analysis consumes a **probe
//! manifest** — a project-side JSON listing contracts as self-contained
//! verus snippets. Automatic contract extraction from captured VIR is a
//! later milestone; the runner, probe generator, and judgments below are
//! fully generic.
//!
//! Two probe kinds (manifest v1.1; absent `probe` field = `quantifier`,
//! back-compatible with v1.0):
//!
//! - **quantifier** entries run in a standalone probe crate (existing
//!   behavior): vacuity probe (`forall` params, negated requires) verifies
//!   → `confirmed_vacuous`; liveness probe (`exists` + witness) has no
//!   attributed failure → `confirmed_live`; both fail →
//!   `candidate_inconclusive`; both succeed → `anomaly`.
//! - **construction** entries are injected into a target workspace
//!   (`target_workspace.inject_dir/<name>.rs`, gated `#[cfg(gate_cfg)]`),
//!   pre-compiled by a rustc-check loop (`cargo-verus check --no-verify`
//!   with the gate cfg forwarded to the Verus driver), then the target
//!   workspace is built with `cargo-verus build` and the log is parsed.
//!   The generated module holds `model_preamble` (model verus source after
//!   `verus! {`) plus a synthesized entry fn `entry_<i>_<safe>` whose body
//!   is the manifest `construct` (the caller-face construction attempt),
//!   and — iff a `witness` is given — a holder-face control fn
//!   `live_<i>_<safe}` with `requires expect_fail_requires`. With
//!   `face_requires = true` the entry fn itself carries
//!   `requires expect_fail_requires` (holder-face entries).
//!   Judgment: a `precondition not satisfied` error attributed by span to
//!   the entry fn region → `confirmed_vacuous` (fabrication channel
//!   rejected + channel-exhaustion argument, documented — model-relative);
//!   entry fn verifies with no holder control → `confirmed_live`; both
//!   probes verified, or contradictory outcomes → `anomaly`; everything
//!   else → `candidate_inconclusive` (never counted as a defect).
//!
//! Injection replay/removal: every created/modified target file is
//! snapshotted under `<out>/injections/` (original bytes + injected bytes +
//! `replay.json`); after the run the workspace is restored to its original
//! state.
//!
//! Manifest notes: quantifier triggers must be inferable, so quantifier
//! `requires` expressions should be written as named `spec fn`
//! applications over the parameters. Schema clarifications added by this
//! implementation (reported against the contract doc): optional
//! `target_workspace.target` (cross-compilation triple; ostd-style crates
//! do not build for the host), optional `target_workspace.extra_cfgs`
//! (additional forwarded rustc cfgs, e.g. exclusion gates such as
//! `ktest_capture_gap`), and optional per-entry `face_requires` (the
//! contract table distinguishes fabrication and holder faces but the
//! schema had no field selecting which one the entry fn is).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::probe_infra::{
    CEvent, CMarker, fn_region, forwarded_cfgs, indent_block, log_tail, param_names,
    parse_target_log, run_captured, sanitize, target_command,
};

// Public API surface kept at the historical paths (manifests and archives
// reference `vacuity::TargetWorkspace`, `vacuity::ProbeInjection`, and
// `vacuity::ProbeModule`); the implementations live in `probe_infra`.
pub use crate::probe_infra::{ProbeInjection, ProbeModule, TargetWorkspace};

/// Which probe family an entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ProbeKind {
    /// Standalone quantifier probe crate (default; v1.0 behavior).
    #[default]
    Quantifier,
    /// Construction probe injected into `target_workspace`.
    Construction,
}

/// Project-side probe manifest: the contracts to probe.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Optional verus source inserted right after `verus! {` (spec
    /// predicates the quantifier requires expressions reference). Required
    /// for quantifier triggers.
    #[serde(default)]
    pub preamble: String,
    /// Required by construction entries: the target workspace to inject
    /// into. Absent = pure quantifier manifest (v1.0).
    #[serde(default)]
    pub target_workspace: Option<TargetWorkspace>,
    pub contracts: Vec<ContractEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ContractEntry {
    /// Stable subject identifier.
    pub name: String,
    /// Probe kind (default `quantifier`).
    #[serde(default)]
    pub probe: ProbeKind,
    /// Verus parameter list source, e.g. `"s: u64, owner: usize"`.
    pub params: String,
    /// quantifier: Verus requires expression source referencing the
    /// parameters; write it as named spec-fn applications so quantifier
    /// triggers are inferable.
    #[serde(default)]
    pub requires: Option<String>,
    /// quantifier: optional self-typed witness expression (tuple matching
    /// the params). construction: optional holder-face control body.
    /// Without a quantifier witness, a satisfiable contract can only be
    /// classified `candidate_inconclusive` — witnesses are not synthesized.
    #[serde(default)]
    pub witness: Option<String>,
    /// construction: model verus source (uses, uninterp/spec fns, structs,
    /// the contract-face stub) inserted after `verus! {` in the injected
    /// module.
    #[serde(default)]
    pub model_preamble: Option<String>,
    /// construction: the precondition text expected to be unsatisfiable
    /// through the fabrication channel; used as the holder-face requires
    /// and for verdict detail.
    #[serde(default)]
    pub expect_fail_requires: Option<String>,
    /// construction: verus source emitted as the body of the entry fn —
    /// the caller-face construction attempt (e.g. the only writable
    /// fabrication channel, or the holder call for `face_requires`
    /// entries).
    #[serde(default)]
    pub construct: Option<String>,
    /// construction: emit `requires expect_fail_requires` on the entry fn
    /// itself (holder-face entries whose construct call is satisfied by the
    /// call-face parameters). Default false: the entry fn is the
    /// fabrication face and must not carry the expected-fail requires.
    #[serde(default)]
    pub face_requires: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryVerdict {
    pub name: String,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    ConfirmedVacuous,
    ConfirmedLive,
    CandidateInconclusive,
    Anomaly,
}

#[derive(Debug, Serialize)]
pub struct VacuityReport {
    pub verdicts: Vec<EntryVerdict>,
    /// Verification summary of the dependency build (e.g. vstd).
    pub dependency_summary: Option<String>,
    /// Verification summary of the probe crate.
    pub probe_summary: Option<String>,
    /// Verification summary of the target workspace crate (construction).
    pub target_summary: Option<String>,
}

/// Generate the standalone probe crate source for a manifest (quantifier
/// entries only; construction entries are injected into the target
/// workspace instead).
pub fn generate_probe_lib(manifest: &Manifest) -> String {
    let mut out = String::from("#![no_std]\nuse vstd::prelude::*;\n\nverus! {\n\n");
    if !manifest.preamble.is_empty() {
        out.push_str(&manifest.preamble);
        out.push_str("\n\n");
    }
    for (i, c) in manifest.contracts.iter().enumerate() {
        if c.probe != ProbeKind::Quantifier {
            continue;
        }
        let safe = sanitize(&c.name);
        let args = arg_names(&c.params);
        let requires = c.requires.as_deref().unwrap_or("");
        let witness_block = match &c.witness {
            Some(w) => {
                let names = param_names(&c.params);
                // Multi-parameter bindings need a tuple pattern: `let (a, b) = (..);`
                let binding = if names.len() == 1 {
                    names[0].clone()
                } else {
                    format!("({})", names.join(", "))
                };
                format!(
                    "    let {binding} = {w};\n    assert(pre_{i}_{safe}({args}));\n",
                    w = w,
                    args = args,
                    safe = safe,
                    i = i,
                )
            }
            None => String::new(),
        };
        out.push_str(&format!(
            "// contract: {name}\n\
             pub open spec fn pre_{i}_{safe}({params}) -> bool {{\n    {requires}\n}}\n\n\
             pub fn entry_{i}_{safe}({params})\n    requires\n        pre_{i}_{safe}({args}),\n{{\n}}\n\n\
             proof fn vacuous_{i}_{safe}()\n    ensures\n        forall|{params}| !pre_{i}_{safe}({args}),\n{{\n}}\n\n\
             proof fn live_{i}_{safe}()\n    ensures\n        exists|{params}| pre_{i}_{safe}({args}),\n{{\n{witness_block}}}\n\n",
            name = c.name,
            params = c.params,
            requires = requires,
            args = args,
            safe = safe,
            i = i,
            witness_block = witness_block,
        ));
    }
    out.push_str("} // verus!\n");
    out
}

fn arg_names(params: &str) -> String {
    param_names(params).join(", ")
}

// ---------------------------------------------------------------------------
// Construction-probe injection generation
// ---------------------------------------------------------------------------

/// Generate the injected probe module set for a manifest's construction
/// entries. Pure: the classifier regenerates this to map log spans.
pub fn generate_probe_injection(manifest: &Manifest) -> Result<ProbeInjection, String> {
    let tw = manifest
        .target_workspace
        .as_ref()
        .ok_or("construction entries require target_workspace")?;
    let module_name = tw
        .inject_dir
        .file_name()
        .map(|n| sanitize(&n.to_string_lossy()))
        .ok_or("target_workspace.inject_dir has no file name")?;
    let gate = &tw.gate_cfg;
    let mut modules = Vec::new();
    let mut mod_decls = String::new();
    for (i, c) in manifest.contracts.iter().enumerate() {
        if c.probe != ProbeKind::Construction {
            continue;
        }
        let safe = sanitize(&c.name);
        if modules.iter().any(|m: &ProbeModule| m.safe_name == safe) {
            return Err(format!(
                "construction entry name collides after sanitize: {}",
                c.name
            ));
        }
        let construct = c
            .construct
            .as_deref()
            .ok_or_else(|| format!("construction entry {} missing construct", c.name))?;
        let efr = c
            .expect_fail_requires
            .as_deref()
            .ok_or_else(|| format!("construction entry {} missing expect_fail_requires", c.name))?;

        let mut src = String::new();
        src.push_str(&format!(
            "// Generated by spec-check contract-vacuity (construction probe).\n\
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
            "// ---- probe entry fn (construct face) — do not edit ----\n\
             pub fn entry_{i}_{safe}({params})",
            i = i,
            safe = safe,
            params = c.params,
        ));
        if c.face_requires {
            src.push_str(&format!(
                "\n    requires\n        {efr},\n{{\n{construct}\n}}\n\n",
                efr = efr,
                construct = indent_block(construct),
            ));
        } else {
            src.push_str(&format!(
                " {{\n{construct}\n}}\n\n",
                construct = indent_block(construct),
            ));
        }
        let entry_decl = format!("pub fn entry_{i}_{safe}");

        let live_region;
        if let Some(w) = &c.witness {
            src.push_str(&format!(
                "// ---- probe live fn (holder face control) — do not edit ----\n\
                 pub fn live_{i}_{safe}({params})\n\
                 \x20   requires\n\
                 \x20       {efr},\n\
                 {{\n{witness}\n}}\n\n",
                i = i,
                safe = safe,
                params = c.params,
                efr = efr,
                witness = indent_block(w),
            ));
            live_region = Some(format!("pub fn live_{i}_{safe}"));
        } else {
            live_region = None;
        }
        src.push_str("} // verus!\n");

        let lines: Vec<&str> = src.lines().collect();
        let entry_region = fn_region(&lines, &entry_decl)
            .ok_or_else(|| format!("internal: entry fn region not found for {}", c.name))?;
        let live = live_region.and_then(|d| fn_region(&lines, &d));

        mod_decls.push_str(&format!("#[cfg({gate})]\npub mod {safe};\n\n"));

        modules.push(ProbeModule {
            safe_name: safe,
            index: i,
            source: src,
            entry_region,
            live_region: live,
        });
    }
    if modules.is_empty() {
        return Err("no construction entries in manifest".into());
    }
    let mod_rs = format!(
        "// Generated by spec-check contract-vacuity (construction probes).\n\
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

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Write the probe crate and run cargo-verus. Returns per-contract verdicts.
pub fn run(
    manifest: &Manifest,
    cargo_verus: &Path,
    z3: &Path,
    vstd: &Path,
    out: &Path,
) -> Result<VacuityReport, String> {
    validate_manifest(manifest)?;
    let has_construction = manifest
        .contracts
        .iter()
        .any(|c| c.probe == ProbeKind::Construction);
    let has_quantifier = manifest
        .contracts
        .iter()
        .any(|c| c.probe == ProbeKind::Quantifier);
    if has_construction && manifest.target_workspace.is_none() {
        return Err("construction entries require target_workspace".into());
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;

    let mut verdicts: Vec<Option<EntryVerdict>> = manifest.contracts.iter().map(|_| None).collect();
    let mut dependency_summary = None;
    let mut probe_summary = None;
    let mut target_summary = None;

    if has_quantifier {
        let src = out.join("src");
        std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
        std::fs::write(out.join("Cargo.toml"), probe_cargo_toml(vstd))
            .map_err(|e| e.to_string())?;
        std::fs::write(src.join("lib.rs"), generate_probe_lib(manifest))
            .map_err(|e| e.to_string())?;

        let log = run_verus(cargo_verus, z3, out)?;
        let report = classify_quantifier(manifest, &log);
        for v in report.verdicts {
            if let Some(idx) = manifest.contracts.iter().position(|c| c.name == v.name) {
                verdicts[idx] = Some(v);
            }
        }
        dependency_summary = report.dependency_summary;
        probe_summary = report.probe_summary;
    }

    if has_construction {
        let tw = manifest.target_workspace.as_ref().unwrap();
        let report = run_construction(manifest, tw, cargo_verus, z3, out)?;
        for v in report.verdicts {
            if let Some(idx) = manifest.contracts.iter().position(|c| c.name == v.name) {
                verdicts[idx] = Some(v);
            }
        }
        target_summary = report.target_summary;
    }

    let verdicts = verdicts
        .into_iter()
        .zip(manifest.contracts.iter())
        .map(|(v, c)| {
            v.unwrap_or(EntryVerdict {
                name: c.name.clone(),
                verdict: Verdict::CandidateInconclusive,
                detail: "entry was not evaluated".into(),
            })
        })
        .collect();
    Ok(VacuityReport {
        verdicts,
        dependency_summary,
        probe_summary,
        target_summary,
    })
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    for c in &manifest.contracts {
        match c.probe {
            ProbeKind::Quantifier => {
                if c.params.trim().is_empty() {
                    return Err(format!("quantifier entry {} missing params", c.name));
                }
                if c.requires.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!("quantifier entry {} missing requires", c.name));
                }
            }
            ProbeKind::Construction => {
                if c.expect_fail_requires
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    return Err(format!(
                        "construction entry {} missing expect_fail_requires",
                        c.name
                    ));
                }
                if c.construct.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(format!("construction entry {} missing construct", c.name));
                }
            }
        }
    }
    Ok(())
}

fn run_construction(
    manifest: &Manifest,
    tw: &TargetWorkspace,
    cargo_verus: &Path,
    z3: &Path,
    out: &Path,
) -> Result<VacuityReport, String> {
    let injection = generate_probe_injection(manifest)?;
    let (guard, target) = crate::probe_infra::write_injection(
        tw,
        out,
        &injection,
        "contract-vacuity (construction probes)",
    )?;

    let cfg_args = forwarded_cfgs(tw);
    let mut check_args = vec!["--no-verify"];
    check_args.extend(cfg_args.iter().map(|s| s.as_str()));
    let build_args = cfg_args.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let target_triple = tw.target.as_ref();
    let precheck_log = out.join("precheck-build.log");
    let build_log = out.join("target-build.log");

    let run_result = (|| -> Result<VacuityReport, String> {
        // Phase 1: rustc-check loop (no verification) — catches probe
        // module syntax/name errors before the expensive verified build.
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
            return Err(format!(
                "precompile check loop failed; see {}\n--- tail ---\n{}",
                precheck_log.display(),
                log_tail(&log1)
            ));
        }

        // Phase 2: verified cargo-verus build of the target workspace.
        // A nonzero exit is expected when a fabrication probe is rejected —
        // the log, not the exit code, is the evidence.
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
        Ok(classify_construction(manifest, &log2))
    })();

    drop(guard); // restore workspace regardless of outcome
    run_result
}

fn probe_cargo_toml(vstd: &Path) -> String {
    // `verify = true` is required: cargo-verus does not verify the primary
    // package without it (validated in the criterion spike).
    format!(
        "[package]\nname = \"spec-vacuity-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\npublish = false\n\n[dependencies]\nvstd = {{ path = {:?} }}\n\n[workspace]\n\n[package.metadata.verus]\nverify = true\n",
        vstd.display()
    )
}

fn run_verus(cargo_verus: &Path, z3: &Path, out: &Path) -> Result<String, String> {
    let output = Command::new(cargo_verus)
        .arg("build")
        .current_dir(out)
        .env("VERUS_USE_RUSTUP", "0")
        .env("VERUS_Z3_PATH", z3)
        .env("RUSTC_BOOTSTRAP", "1")
        .output()
        .map_err(|e| format!("failed to execute cargo-verus: {e}"))?;
    let mut log = String::from_utf8_lossy(&output.stdout).to_string();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    std::fs::write(out.join("probe-build.log"), &log).map_err(|e| e.to_string())?;
    Ok(log)
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify quantifier entries from a standalone probe-crate verus log.
/// Construction entries are classified by [`classify_construction`] against
/// the target-workspace build log; in this log they have no evidence and
/// receive a placeholder verdict that `run` overwrites.
pub fn classify(manifest: &Manifest, log: &str) -> VacuityReport {
    let mut report = classify_quantifier(manifest, log);
    for c in &manifest.contracts {
        if c.probe == ProbeKind::Construction && !report.verdicts.iter().any(|v| v.name == c.name) {
            report.verdicts.push(EntryVerdict {
                name: c.name.clone(),
                verdict: Verdict::CandidateInconclusive,
                detail: "construction entry has no evidence in the quantifier standalone log"
                    .into(),
            });
        }
    }
    report
}

/// Quantifier classification (existing M1 behavior, quantifier entries only).
fn classify_quantifier(manifest: &Manifest, log: &str) -> VacuityReport {
    let summaries: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("verification results::"))
        .collect();
    let probe_summary = summaries.last().map(|s| s.to_string());
    let dependency_summary = if summaries.len() > 1 {
        Some(summaries[0].to_string())
    } else {
        None
    };

    // Map each generated `proof fn` line to (contract index, probe kind).
    let generated = generate_probe_lib(manifest);
    let mut probe_lines: Vec<(usize, usize, &'static str)> = Vec::new();
    for (n, line) in generated.lines().enumerate() {
        for (idx, c) in manifest.contracts.iter().enumerate() {
            if c.probe != ProbeKind::Quantifier {
                continue;
            }
            for kind in ["vacuous", "live"] {
                if line.starts_with(&format!("proof fn {kind}_{idx}_")) {
                    probe_lines.push((n + 1, idx, kind));
                }
            }
        }
    }
    probe_lines.sort();

    // Attribute each failure marker line to its probe. Two marker kinds:
    // - `failed this postcondition` under a probe's `ensures`
    // - `assertion failed` under the witness `assert` inside a `live_` probe
    //   body: a witness that cannot be proven must NOT count as an
    //   unattributed liveness success — verus proves the rest of the poisoned
    //   path vacuously, so the assert error is the only emitted failure.
    let mut failed: HashMap<(usize, &'static str), ()> = HashMap::new();
    let mut current_failure_line: Option<usize> = None;
    for line in log.lines() {
        if line.starts_with("error:") {
            current_failure_line = None;
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("--> src/lib.rs:") {
            if let Some(num) = rest
                .split(':')
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
            {
                current_failure_line = Some(num);
            }
            continue;
        }
        let trimmed = line.trim_start();
        if line.contains('^')
            && (trimmed.contains("failed this postcondition")
                || trimmed.contains("assertion failed"))
        {
            if let Some(l) = current_failure_line {
                // nearest preceding probe declaration
                let mut best: Option<(usize, usize, &'static str)> = None;
                for p in &probe_lines {
                    if p.0 <= l {
                        best = Some(*p);
                    }
                }
                if let Some((_, idx, kind)) = best {
                    failed.insert((idx, kind), ());
                }
            }
            current_failure_line = None;
        }
    }

    let verdicts = manifest
        .contracts
        .iter()
        .enumerate()
        .filter(|(_, c)| c.probe == ProbeKind::Quantifier)
        .map(|(i, c)| {
            let vacuous_failed = failed.contains_key(&(i, "vacuous"));
            let live_failed = failed.contains_key(&(i, "live"));
            let verdict = match (vacuous_failed, live_failed) {
                (false, false) if !probe_lines.is_empty() => {
                    // both verified: the contract is vacuous AND live — impossible
                    Verdict::Anomaly
                }
                (false, true) => Verdict::ConfirmedVacuous,
                (true, false) => Verdict::ConfirmedLive,
                (true, true) => Verdict::CandidateInconclusive,
                (_, _) => Verdict::CandidateInconclusive,
            };
            EntryVerdict {
                name: c.name.clone(),
                verdict,
                detail: format!(
                    "vacuous_probe_failed={vacuous_failed} live_probe_failed={live_failed}"
                ),
            }
        })
        .collect();
    VacuityReport {
        verdicts,
        dependency_summary,
        probe_summary,
        target_summary: None,
    }
}

/// Classify construction entries from the target-workspace verus build log.
pub fn classify_construction(manifest: &Manifest, log: &str) -> VacuityReport {
    let injection = match generate_probe_injection(manifest) {
        Ok(inj) => inj,
        Err(e) => {
            return VacuityReport {
                verdicts: manifest
                    .contracts
                    .iter()
                    .filter(|c| c.probe == ProbeKind::Construction)
                    .map(|c| EntryVerdict {
                        name: c.name.clone(),
                        verdict: Verdict::CandidateInconclusive,
                        detail: format!("injection generation failed: {e}"),
                    })
                    .collect(),
                dependency_summary: None,
                probe_summary: None,
                target_summary: None,
            };
        }
    };
    let crate_name = manifest
        .target_workspace
        .as_ref()
        .and_then(|tw| tw.inject_dir.components().next())
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .unwrap_or_default();

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

    let mut verdicts = Vec::new();
    for m in &injection.modules {
        let c = &manifest.contracts[m.index];
        let suffix = format!("/{}.rs", m.safe_name);
        let in_region = |r: (usize, usize)| {
            events
                .iter()
                .filter(|e| e.file.ends_with(&suffix) || e.file == format!("{}.rs", m.safe_name))
                .filter(|e| e.line >= r.0 && e.line <= r.1)
                .collect::<Vec<_>>()
        };
        let entry_events = in_region(m.entry_region);
        let live_events = m.live_region.map(in_region).unwrap_or_default();

        let entry_outcome = if !target_ran {
            "not_run"
        } else if entry_events
            .iter()
            .any(|e| e.marker == CMarker::Precondition)
        {
            "rejected_expected"
        } else if !entry_events.is_empty() {
            "failed_other"
        } else {
            "clean"
        };
        let live_outcome = if m.live_region.is_none() {
            "absent"
        } else if !target_ran {
            "not_run"
        } else if !live_events.is_empty() {
            "failed"
        } else {
            "clean"
        };

        let span_note = |es: &[&CEvent]| {
            es.first()
                .map(|e| format!("{}:{}", e.file, e.line))
                .unwrap_or_else(|| "-".into())
        };
        let (verdict, reason) = match (entry_outcome, live_outcome) {
            ("rejected_expected", "clean") | ("rejected_expected", "absent") => (
                Verdict::ConfirmedVacuous,
                format!(
                    "construction: fabrication call rejected at expect_fail_requires \
                     (precondition not satisfied at {}; evidence: fabrication channel \
                     rejected + channel-exhaustion argument (documented, model-relative))",
                    span_note(&entry_events),
                ),
            ),
            ("rejected_expected", _) => (
                Verdict::CandidateInconclusive,
                format!(
                    "construction: fabrication rejected ({}), but holder-face control failed ({})",
                    span_note(&entry_events),
                    span_note(&live_events),
                ),
            ),
            ("clean", "absent") => (
                Verdict::ConfirmedLive,
                "construction: construct face verified (call path satisfiable)".into(),
            ),
            ("clean", "clean") | ("clean", "failed") => (
                Verdict::Anomaly,
                format!(
                    "construction: entry fn verified while holder-face control outcome is {live_outcome} — manual inspection required",
                ),
            ),
            ("failed_other", "failed") => (
                Verdict::Anomaly,
                format!(
                    "construction: both probes failed without the expected attribution \
                     (entry fn failure at {})",
                    span_note(&entry_events),
                ),
            ),
            (entry, live) => (
                Verdict::CandidateInconclusive,
                format!(
                    "construction: probes inconclusive (entry fn {entry}, holder face {live}, \
                     target verification ran: {target_ran}; entry fn failure at {})",
                    span_note(&entry_events),
                ),
            ),
        };
        verdicts.push(EntryVerdict {
            name: c.name.clone(),
            verdict,
            detail: reason,
        });
    }

    VacuityReport {
        verdicts,
        dependency_summary: None,
        probe_summary: None,
        target_summary,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn quantifier_manifest() -> Manifest {
        serde_json::from_str(
            r#"{
              "preamble": "pub uninterp spec fn f(x: u64) -> bool;\n",
              "contracts": [
                {"name": "vacuous_thing", "params": "x: u64", "requires": "f(x) && !f(x)"},
                {"name": "live_thing", "params": "x: u64", "requires": "f(x)", "witness": "0u64"}
              ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn v1_0_manifest_backcompat_and_generation() {
        let m = quantifier_manifest();
        assert!(m.target_workspace.is_none());
        for c in &m.contracts {
            assert_eq!(c.probe, ProbeKind::Quantifier);
            assert!(c.requires.is_some());
            assert!(!c.face_requires);
        }
        let lib = generate_probe_lib(&m);
        assert!(lib.contains("proof fn vacuous_0_vacuous_thing"));
        assert!(lib.contains("proof fn live_1_live_thing"));
        assert!(lib.contains("pub uninterp spec fn f(x: u64) -> bool;"));
    }

    #[test]
    fn quantifier_classification_matrix() {
        let m = quantifier_manifest();
        // No failures at all: both probes verified → anomaly (impossible).
        let r = classify(&m, "verification results:: 4 verified, 0 errors\n");
        assert_eq!(r.verdicts[0].verdict, Verdict::Anomaly);
        assert_eq!(r.verdicts[1].verdict, Verdict::Anomaly);
        // Synthetic log spans must match the generated source line numbers.
        let generated = generate_probe_lib(&m);
        let line_of = |prefix: &str| {
            generated
                .lines()
                .position(|l| l.starts_with(prefix))
                .unwrap()
                + 1
        };
        let vac_line = line_of("proof fn vacuous_0_vacuous_thing");
        let live_line = line_of("proof fn live_1_live_thing");
        // Vacuous probe failed (postcondition under vacuous_0), live clean
        // → ConfirmedVacuous.
        let log = format!(
            "\
error: postcondition not satisfied
   --> src/lib.rs:{vac_line}:9
    |
{vac_line}  |     proof fn vacuous_0_vacuous_thing()
    |         ^^^^^^^^^ failed this postcondition

verification results:: 3 verified, 1 errors
"
        );
        // Postcondition failure under vacuous_0 = the forall probe cannot
        // be proven → contract 0 is satisfiable → ConfirmedLive; contract 1
        // probes verified → anomaly.
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::ConfirmedLive);
        assert_eq!(r.verdicts[1].verdict, Verdict::Anomaly);
        // Witness assertion failure counts as live failure: contract 0
        // (vacuous failed) → ConfirmedLive; contract 1 (vacuous verified,
        // live failed) → ConfirmedVacuous.
        let log = format!(
            "\
error: postcondition not satisfied
   --> src/lib.rs:{vac_line}:9
    |     ^^^^^^^^^ failed this postcondition

error: assertion failed
   --> src/lib.rs:{live_line}:5
    |
{live_line} |         assert(pre_1_live_thing(0u64));
    |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ assertion failed

verification results:: 2 verified, 2 errors
"
        );
        let r = classify(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::ConfirmedLive);
        assert_eq!(r.verdicts[1].verdict, Verdict::ConfirmedVacuous);
    }

    fn construction_manifest() -> Manifest {
        serde_json::from_str(
            r#"{
              "target_workspace": {
                "manifest_path": "/tmp/target/Cargo.toml",
                "inject_dir": "ostd/src/spec_check_probes",
                "gate_cfg": "ktest"
              },
              "contracts": [
                {
                  "name": "fabrication",
                  "probe": "construction",
                  "params": "m: &Mutex<u64>",
                  "expect_fail_requires": "perm.id() == m.cell_id()",
                  "model_preamble": "pub struct Stub;",
                  "construct": "let t = mint(m);\nstub_call(m, t);"
                },
                {
                  "name": "holder_face",
                  "probe": "construction",
                  "params": "m: &Mutex<u64>, Tracked(perm): Tracked<PointsTo<u64>>",
                  "expect_fail_requires": "perm.id() == m.cell_id()",
                  "face_requires": true,
                  "model_preamble": "pub struct Stub;",
                  "construct": "stub_call(m, Tracked(perm));"
                },
                {
                  "name": "shape_control",
                  "params": "a: u64",
                  "requires": "g(a)",
                  "witness": "0u64"
                }
              ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn injection_generation_shapes() {
        let m = construction_manifest();
        let inj = generate_probe_injection(&m).unwrap();
        assert_eq!(inj.module_name, "spec_check_probes");
        assert_eq!(inj.lib_decl, "#[cfg(ktest)] mod spec_check_probes;");
        assert_eq!(inj.modules.len(), 2);
        assert!(inj.mod_rs.contains("#[cfg(ktest)]\npub mod fabrication;"));
        assert!(inj.mod_rs.contains("#[cfg(ktest)]\npub mod holder_face;"));

        let fab = &inj.modules[0];
        // entry fn has NO requires clause (fabrication face)
        assert!(
            !fab.source
                .contains("pub fn entry_0_fabrication(m: &Mutex<u64>)\n    requires")
        );
        assert!(
            fab.source
                .contains("pub fn entry_0_fabrication(m: &Mutex<u64>) {")
        );
        assert!(fab.source.contains("stub_call(m, t);"));
        assert!(fab.live_region.is_none());
        // region covers the whole synthesized fn
        let lines: Vec<&str> = fab.source.lines().collect();
        assert_eq!(
            lines[fab.entry_region.0 - 1],
            "pub fn entry_0_fabrication(m: &Mutex<u64>) {"
        );
        assert_eq!(lines[fab.entry_region.1 - 1], "}");

        let holder = &inj.modules[1];
        assert!(holder.source.contains("pub fn entry_1_holder_face(m: &Mutex<u64>, Tracked(perm): Tracked<PointsTo<u64>>)\n    requires\n        perm.id() == m.cell_id(),\n{"));

        // quantifier entries excluded from the standalone crate
        let lib = generate_probe_lib(&m);
        assert!(!lib.contains("fabrication"));
        assert!(lib.contains("proof fn vacuous_2_shape_control"));

        // witness-bearing construction entry generates the live control fn
        let with_witness: Manifest = serde_json::from_str(
            r#"{
              "target_workspace": {
                "manifest_path": "/tmp/target/Cargo.toml",
                "inject_dir": "ostd/src/probes",
                "gate_cfg": "ktest"
              },
              "contracts": [{
                "name": "1fabrication",
                "probe": "construction",
                "params": "m: &Mutex<u64>",
                "expect_fail_requires": "p(m)",
                "model_preamble": "",
                "construct": "attempt(m);",
                "witness": "held_call(m);"
              }]
            }"#,
        )
        .unwrap();
        let inj = generate_probe_injection(&with_witness).unwrap();
        // leading digit sanitized
        assert_eq!(inj.modules[0].safe_name, "p_1fabrication");
        let live = inj.modules[0].live_region.unwrap();
        let lines: Vec<&str> = inj.modules[0].source.lines().collect();
        assert!(lines[live.0 - 1].starts_with("pub fn live_0_p_1fabrication(m: &Mutex<u64>)"));
        assert!(inj.modules[0].source.contains("held_call(m);"));
    }

    #[test]
    fn construction_classification_matrix() {
        let m = construction_manifest();
        let inj = generate_probe_injection(&m).unwrap();
        // fabrication rejected at the entry fn span (module file), holder
        // face clean → ConfirmedVacuous.
        let fab_line = inj.modules[0].entry_region.0 + 1; // inside the entry fn body
        let log = format!(
            "\
   Compiling vstd v0.1.0 (/tmp/target/tools/verus/source/vstd)
verification results:: 2060 verified, 0 errors
   Compiling ostd v0.1.0 (/tmp/target/ostd)
error[E0624]: nothing here
  --> ostd/src/other.rs:5:1

error: precondition not satisfied
   --> ostd/src/spec_check_probes/fabrication.rs:{fab_line}:5
    |
 88 |     perm.id() == m.cell_id(),
    |     ------------------------- failed precondition

verification results:: 1500 verified, 1 errors
error: could not compile `ostd` (lib) due to 1 previous error
"
        );
        let r = classify_construction(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::ConfirmedVacuous);
        assert!(
            r.verdicts[0]
                .detail
                .contains(&format!("fabrication.rs:{fab_line}"))
        );
        assert!(r.verdicts[0].detail.contains("channel-exhaustion"));
        assert_eq!(r.verdicts[1].verdict, Verdict::ConfirmedLive);
        assert_eq!(r.target_summary.as_deref(), Some("1500 verified, 1 errors"));

        // entry fn clean + absent live → ConfirmedLive (holder-face entry).
        let log = "\
   Compiling ostd v0.1.0 (/tmp/target/ostd)
verification results:: 1501 verified, 0 errors
";
        let r = classify_construction(&m, log);
        assert_eq!(r.verdicts[0].verdict, Verdict::ConfirmedLive);
        assert_eq!(r.verdicts[1].verdict, Verdict::ConfirmedLive);

        // compile abort: no summary in the ostd section → not_run → candidate.
        let log = "\
   Compiling vstd v0.1.0 (/tmp/target/tools/verus/source/vstd)
verification results:: 2060 verified, 0 errors
   Compiling ostd v0.1.0 (/tmp/target/ostd)
error[E0433]: cannot find module `nope`
  --> ostd/src/spec_check_probes/fabrication.rs:3:5
error: could not compile `ostd` (lib) due to 1 previous error
";
        let r = classify_construction(&m, log);
        assert_eq!(r.verdicts[0].verdict, Verdict::CandidateInconclusive);
        assert!(r.verdicts[0].detail.contains("not_run"));
        assert_eq!(r.verdicts[1].verdict, Verdict::CandidateInconclusive);

        // entry fn fails at an unexpected spot (no holder control) →
        // candidate; the same failure shape with a failing live fn →
        // anomaly (both failed, attribution missing).
        let hf_line = inj.modules[1].entry_region.0 + 1; // inside holder entry body
        let log = format!(
            "\
   Compiling ostd v0.1.0 (/tmp/target/ostd)
error: postcondition not satisfied
   --> ostd/src/spec_check_probes/fabrication.rs:{fab_line}:5

error: postcondition not satisfied
   --> ostd/src/spec_check_probes/holder_face.rs:{hf_line}:5

verification results:: 1499 verified, 2 errors
"
        );
        let r = classify_construction(&m, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::CandidateInconclusive);
        assert_eq!(r.verdicts[1].verdict, Verdict::CandidateInconclusive);
        assert!(r.verdicts[0].detail.contains("failed_other"));

        // fabrication rejected but holder control failed → candidate.
        let with_witness: Manifest = serde_json::from_str(
            r#"{
              "target_workspace": {
                "manifest_path": "/tmp/target/Cargo.toml",
                "inject_dir": "ostd/src/spec_check_probes",
                "gate_cfg": "ktest"
              },
              "contracts": [{
                "name": "fabrication",
                "probe": "construction",
                "params": "m: &Mutex<u64>",
                "expect_fail_requires": "p(m)",
                "model_preamble": "",
                "construct": "attempt(m);",
                "witness": "held_call(m);"
              }]
            }"#,
        )
        .unwrap();
        let inj = generate_probe_injection(&with_witness).unwrap();
        let fab_line = inj.modules[0].entry_region.0 + 1;
        let live_line = inj.modules[0].live_region.unwrap().0 + 1;
        let log = format!(
            "\
   Compiling ostd v0.1.0 (/tmp/target/ostd)
error: precondition not satisfied
   --> ostd/src/spec_check_probes/fabrication.rs:{fab_line}:5

error: precondition not satisfied
   --> ostd/src/spec_check_probes/fabrication.rs:{live_line}:5

verification results:: 1499 verified, 2 errors
"
        );
        let r = classify_construction(&with_witness, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::CandidateInconclusive);
        assert!(r.verdicts[0].detail.contains("holder-face control failed"));

        // both probes fail without expected attribution → anomaly.
        let log = format!(
            "\
   Compiling ostd v0.1.0 (/tmp/target/ostd)
error: postcondition not satisfied
   --> ostd/src/spec_check_probes/fabrication.rs:{fab_line}:5

error: postcondition not satisfied
   --> ostd/src/spec_check_probes/fabrication.rs:{live_line}:5

verification results:: 1499 verified, 2 errors
"
        );
        let r = classify_construction(&with_witness, &log);
        assert_eq!(r.verdicts[0].verdict, Verdict::Anomaly);
    }

    #[test]
    fn run_rejects_invalid_manifests() {
        let bad: Manifest = serde_json::from_str(
            r#"{
              "contracts": [
                {"name": "c", "probe": "construction", "params": "", "construct": "x();"}
              ]
            }"#,
        )
        .unwrap();
        assert!(validate_manifest(&bad).is_err());

        let no_tw: Manifest = serde_json::from_str(
            r#"{
              "contracts": [
                {"name": "c", "probe": "construction", "params": "", "expect_fail_requires": "f", "construct": "x();"}
              ]
            }"#,
        )
        .unwrap();
        let out = std::env::temp_dir().join("spec-check-vacuity-test-out");
        let err = run(
            &no_tw,
            Path::new("cargo-verus"),
            Path::new("z3"),
            Path::new("vstd"),
            &out,
        )
        .unwrap_err();
        assert!(err.contains("target_workspace"));
    }
}
