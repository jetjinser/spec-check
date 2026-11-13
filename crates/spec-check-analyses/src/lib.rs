//! Pluggable spec-quality analyses.
//!
//! Every analysis is a generic pass over captured artifacts (Verus VIR
//! contracts, Rustc HIR/MIR of the crate and its tests). Analyses must not
//! reference any specific target project (Vostd-specific knowledge lives in
//! project-side inputs, never here).
//!
//! # Adding an analysis
//!
//! 1. Implement [`Analysis`]: declare the [`RequiredArtifacts`] your pass
//!    needs and implement [`Analysis::run`], returning typed [`Finding`]s.
//! 2. Register the implementation in [`registry::builtin_registry`].
//! 3. Add positive/negative controls under `tests/fixtures` so the analysis
//!    is validated against a known-good and a known-bad case.

pub mod divergence;
pub mod ledger;
mod probe_infra;
pub mod synthesize;
pub mod vacuity;

use std::collections::BTreeMap;

/// Captured artifacts an analysis may require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RequiredArtifact {
    /// Verus VIR of the target crate (contract-level analysis).
    Vir,
    /// Rustc HIR/MIR inventory of the crate and its tests.
    Inventory,
    /// Guard/owner relation records.
    GuardOwner,
}

/// One reproducible spec-quality finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable, analysis-scoped identifier (e.g. `vacuity.unsat_precondition`).
    pub kind: String,
    /// Subject of the finding (function path, contract span, ...).
    pub subject: String,
    /// Evidence class; the meaning is defined by the analysis.
    pub evidence_class: EvidenceClass,
    /// Machine-checkable reproduction hint (command or probe name).
    pub repro: String,
}

/// How strongly a finding is established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceClass {
    /// Confirmed by a checked probe with a positive control.
    Confirmed,
    /// Detected by a rule; needs a probe or human review.
    Candidate,
}

/// A pluggable analysis over captured artifacts.
pub trait Analysis {
    /// Stable name used on the CLI and in reports.
    fn name(&self) -> &'static str;
    /// One-line description.
    fn description(&self) -> &'static str;
    /// Artifacts this analysis needs from the capture.
    fn required_artifacts(&self) -> &'static [RequiredArtifact];
    /// Run against the capture. `artifacts` maps artifact kinds to
    /// filesystem paths produced by `project-capture`.
    fn run(
        &self,
        artifacts: &BTreeMap<RequiredArtifact, std::path::PathBuf>,
    ) -> Result<Vec<Finding>, AnalysisError>;
}

/// Errors analyses may surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisError(pub String);

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AnalysisError {}

/// Registry of built-in analyses, keyed by name.
pub mod registry {
    use super::Analysis;

    use std::collections::BTreeMap;

    /// Returns all built-in analyses. New analyses register here.
    pub fn builtin_registry() -> BTreeMap<&'static str, Box<dyn Analysis>> {
        BTreeMap::new()
    }
}
