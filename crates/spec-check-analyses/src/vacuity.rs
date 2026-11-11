//! `contract-vacuity`: generic vacuity analysis over a probe manifest.
//!
//! Phase-1 scope (honest boundary): the analysis consumes a **probe
//! manifest** — a project-side JSON listing contracts as self-contained
//! verus snippets (`params` + `requires` referencing them). Automatic
//! contract extraction from captured VIR is a later milestone; the runner,
//! probe-crate generator, and judgment below are fully generic.
//!
//! Judgment criterion (validated in `examples/contract-vacuity-spike`):
//! - vacuity probe (`forall` params, negated requires) verifies
//!   → `confirmed_vacuous`
//! - liveness probe (`exists` params, requires, explicit witness) has no
//!   failure attributed → `confirmed_live`
//! - both probes fail → `candidate_inconclusive` (verifier incompleteness;
//!   never counted as a defect)
//! - both probes succeed → `anomaly` (logically impossible; inspected)
//!
//! Manifest notes: quantifier triggers must be inferable, so `requires`
//! expressions should be written as named `spec fn` applications over the
//! parameters (literal-only comparisons fail trigger inference).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Project-side probe manifest: the contracts to probe.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Optional verus source inserted right after `verus! {` (spec
    /// predicates the requires expressions reference, e.g. a named
    /// reachable-state set). Required for quantifier triggers.
    #[serde(default)]
    pub preamble: String,
    pub contracts: Vec<ContractEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ContractEntry {
    /// Stable subject identifier.
    pub name: String,
    /// Verus parameter list source, e.g. `"s: u64, owner: usize"`.
    pub params: String,
    /// Verus requires expression source referencing the parameters; write it
    /// as named spec-fn applications so quantifier triggers are inferable.
    pub requires: String,
    /// Optional self-typed witness expression (tuple matching the params).
    /// Without a witness, a satisfiable contract can only be classified
    /// `candidate_inconclusive` — witnesses are not synthesized.
    #[serde(default)]
    pub witness: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct VacuityReport {
    pub verdicts: Vec<EntryVerdict>,
    /// Verification summary of the dependency build (e.g. vstd).
    pub dependency_summary: Option<String>,
    /// Verification summary of the probe crate.
    pub probe_summary: Option<String>,
}

/// Generate the probe crate source for a manifest.
pub fn generate_probe_lib(manifest: &Manifest) -> String {
    let mut out = String::from("#![no_std]\nuse vstd::prelude::*;\n\nverus! {\n\n");
    if !manifest.preamble.is_empty() {
        out.push_str(&manifest.preamble);
        out.push_str("\n\n");
    }
    for (i, c) in manifest.contracts.iter().enumerate() {
        let safe = sanitize(&c.name);
        let args = arg_names(&c.params);
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
            },
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
            requires = c.requires,
            args = args,
            safe = safe,
            i = i,
            witness_block = witness_block,
        ));
    }
    out.push_str("} // verus!\n");
    out
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn param_names(params: &str) -> Vec<String> {
    params
        .split(',')
        .filter_map(|p| p.split(':').next())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

fn arg_names(params: &str) -> String {
    param_names(params).join(", ")
}

/// Write the probe crate and run cargo-verus. Returns per-contract verdicts.
pub fn run(
    manifest: &Manifest,
    cargo_verus: &Path,
    z3: &Path,
    vstd: &Path,
    out: &Path,
) -> Result<VacuityReport, String> {
    let src = out.join("src");
    std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
    std::fs::write(out.join("Cargo.toml"), &probe_cargo_toml(vstd)).map_err(|e| e.to_string())?;
    std::fs::write(src.join("lib.rs"), generate_probe_lib(manifest))
        .map_err(|e| e.to_string())?;

    let log = run_verus(cargo_verus, z3, out)?;
    Ok(classify(manifest, &log))
}

fn probe_cargo_toml(vstd: &Path) -> String {
    // `verify = true` is required: cargo-verus does not verify the primary
    // package without it (validated in the criterion spike).
    format!("[package]\nname = \"spec-vacuity-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\npublish = false\n\n[dependencies]\nvstd = {{ path = {:?} }}\n\n[workspace]\n\n[package.metadata.verus]\nverify = true\n", vstd.display())
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

/// Classify per-contract verdicts from the verus log.
pub fn classify(manifest: &Manifest, log: &str) -> VacuityReport {
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
        for (idx, _) in manifest.contracts.iter().enumerate() {
            for kind in ["vacuous", "live"] {
                if line.starts_with(&format!("proof fn {kind}_{idx}_")) {
                    probe_lines.push((n + 1, idx, kind));
                }
            }
        }
    }
    probe_lines.sort();

    // Attribute each "postcondition not satisfied" span line to its probe.
    let mut failed: HashMap<(usize, &'static str), ()> = HashMap::new();
    for line in log.lines() {
        if let Some(rest) = line.strip_prefix("   |         ^^^^") {
            // failure marker; the span line precedes it — handled below
            let _ = rest;
        }
    }
    let mut current_failure_line: Option<usize> = None;
    for line in log.lines() {
        if line.contains("error: postcondition not satisfied") {
            current_failure_line = None;
            continue;
        }
        if let Some(rest) = line
            .trim_start()
            .strip_prefix("--> src/lib.rs:")
        {
            if let Some(num) = rest
                .split(':')
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
            {
                current_failure_line = Some(num);
            }
            continue;
        }
        if line.contains("failed this postcondition") {
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
    }
}
