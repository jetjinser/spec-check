//! `obligation-synthesizer`: render an executable divergence obligation
//! manifest from candidate findings plus project-side type recipes.
//!
//! The synthesizer is a pure transformation (no target-workspace access, no
//! verification). It owns only generic semantics:
//!
//! - four obligation families with fixed Rust/Verus skeletons
//!   (`value-readback`, `is_none-demand`, `guard-identity`,
//!   `final-relation`; `other` has no template and is skipped);
//! - `{recv}` / `{out}` / `{handle}` placeholders and per-family validation;
//! - recipe schema (type constructor / imports / setup; interface call,
//!   snapshot, probes, assertion, optional expectation override);
//! - dedup by the candidate `dedup_key`, deterministic entry naming, and
//!   selection by id / function / entry name;
//! - pre-registration inheritance (candidate `prereg_expected`) with an
//!   explicit, audited recipe override.
//!
//! Project-specific knowledge (how to construct a target type, which
//! accessor reads a value) lives in the recipe file passed on the CLI, never
//! in this module. Design: `docs/M7c-obligation-synthesizer.md`.
//!
//! Candidates without a recipe are **skipped** and recorded in the synthesis
//! report; they never enter the manifest as placeholder obligations (a
//! non-compiling probe would turn the whole divergence build's entries into
//! `Inconclusive`). Recipe configuration errors fail the whole synthesis.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::divergence::{Expectation, Manifest as DivergenceManifest, ObligationEntry};
use crate::probe_infra::{sanitize, TargetWorkspace};

// ---------------------------------------------------------------------------
// Candidate input (one record per upstream finding; unknown fields ignored)
// ---------------------------------------------------------------------------

/// One upstream obligation candidate.
#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub function: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub constructibility: String,
    #[serde(default)]
    pub constructibility_reason: Option<String>,
    #[serde(default)]
    pub constructibility_note: Option<String>,
    /// Run-before hypothesis ("gap" | "supported" | "inconclusive").
    #[serde(default)]
    pub prereg_expected: Option<String>,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub registry_overlap: Option<String>,
    /// Draft only; never executed directly (it may not be valid Verus).
    #[serde(default)]
    pub obligation_draft: Option<String>,
}

// ---------------------------------------------------------------------------
// Recipe input (project-side configuration)
// ---------------------------------------------------------------------------

/// Project-side recipe file.
#[derive(Debug, Clone, Deserialize)]
pub struct Recipes {
    /// Target workspace, passed through to the divergence manifest unchanged.
    pub target_workspace: TargetWorkspace,
    #[serde(default)]
    pub types: BTreeMap<String, TypeRecipe>,
    #[serde(default)]
    pub interfaces: BTreeMap<String, InterfaceRecipe>,
}

/// How to build a receiver of one type family.
#[derive(Debug, Clone, Deserialize)]
pub struct TypeRecipe {
    /// `use` lines emitted into the probe module preamble.
    #[serde(default)]
    pub imports: Vec<String>,
    /// Statements emitted before the receiver binding (slice/array setup).
    #[serde(default)]
    pub setup: Option<String>,
    /// Receiver construction expression (`let recv = <this>;`). Empty means
    /// the interface has no receiver (static functions).
    #[serde(default)]
    pub construct: String,
    /// Attribute emitted on the receiver binding (project-side ghost-output
    /// plumbing, e.g. a Verus `#[verus_spec(with => Tracked(...))]` output).
    #[serde(default)]
    pub bind_attr: Option<String>,
    /// Extra verus source emitted after `verus! {` (helper specs/structs).
    #[serde(default)]
    pub model_preamble: Option<String>,
}

/// One interface obligation recipe.
#[derive(Debug, Clone, Deserialize)]
pub struct InterfaceRecipe {
    /// Type recipe key used for construction and imports.
    #[serde(default)]
    pub receiver_type: Option<String>,
    /// Bind the receiver as `let mut recv = ...` (methods taking `&mut self`).
    #[serde(default)]
    pub receiver_mut: bool,
    /// Cross-check against the candidate family (optional).
    #[serde(default)]
    pub family: Option<String>,
    /// Per-interface constructor override (alternate constructors of one
    /// type recipe).
    #[serde(default)]
    pub construct: Option<String>,
    /// Statements between receiver binding and the call.
    #[serde(default)]
    pub bridge: Option<String>,
    /// Call expression (`value-readback`/`is_none-demand`/`guard-identity`
    /// require it; `final-relation` may leave it empty for constructor
    /// post-state obligations).
    #[serde(default)]
    pub call: String,
    /// How the call result is bound: `let` (default) or `if-let-some`
    /// (fallible/optional acquisition returns).
    #[serde(default)]
    pub bind: Option<String>,
    /// Pre-call snapshot statements (`final-relation` only).
    #[serde(default)]
    pub snapshot: Option<String>,
    /// Extra exec statements after the call binding (e.g. `let flag =
    /// {out}.is_none();`). Avoids exec calls inside `assert` arguments.
    #[serde(default)]
    pub probes: Vec<String>,
    /// Assertion expression.
    #[serde(rename = "assert")]
    pub assertion: String,
    /// Keep the produced handle alive after the assertion.
    #[serde(default)]
    pub hold: bool,
    /// Optional pre-registration override ("gap" | "supported" | "none").
    #[serde(default)]
    pub expect: Option<String>,
    /// Required with `expect`; recorded in the report for audit.
    #[serde(default)]
    pub expect_basis: Option<String>,
    /// Extra per-interface preamble (helper spec source).
    #[serde(default)]
    pub model_preamble: Option<String>,
}

// ---------------------------------------------------------------------------
// Families and skeletons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    ValueReadback,
    FinalRelation,
    GuardIdentity,
    IsNoneDemand,
    Other,
}

impl Family {
    pub fn parse(s: &str) -> Option<Family> {
        match s {
            "value-readback" => Some(Family::ValueReadback),
            "final-relation" => Some(Family::FinalRelation),
            "guard-identity" => Some(Family::GuardIdentity),
            "is_none-demand" => Some(Family::IsNoneDemand),
            "other" => Some(Family::Other),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Family::ValueReadback => "value-readback",
            Family::FinalRelation => "final-relation",
            Family::GuardIdentity => "guard-identity",
            Family::IsNoneDemand => "is_none-demand",
            Family::Other => "other",
        }
    }

    fn has_template(self) -> bool {
        !matches!(self, Family::Other)
    }
}

/// Rendered probe body plus its preamble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub obligation: String,
    pub model_preamble: Option<String>,
}

fn substitute(template: &str, pairs: &[(&str, &str)]) -> Result<String, String> {
    let mut out = template.to_string();
    for (placeholder, value) in pairs {
        out = out.replace(placeholder, value);
    }
    for leftover in ["{recv}", "{out}", "{handle}"] {
        if out.contains(leftover) {
            return Err(format!(
                "placeholder {leftover} is not available in this obligation skeleton"
            ));
        }
    }
    Ok(out)
}

fn push_bind_attr(lines: &mut Vec<String>, ty: Option<&TypeRecipe>, has_receiver: bool) {
    if !has_receiver {
        return;
    }
    if let Some(attr) = ty.and_then(|t| t.bind_attr.as_deref()) {
        let attr = attr.trim();
        if !attr.is_empty() {
            lines.push(attr.to_string());
        }
    }
}

fn push_block(lines: &mut Vec<String>, block: Option<&String>) {
    if let Some(block) = block {
        for line in block.lines() {
            let line = line.trim_end();
            if !line.trim().is_empty() {
                lines.push(line.to_string());
            }
        }
    }
}

fn preamble_of(ty: Option<&TypeRecipe>, ir: &InterfaceRecipe) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ty) = ty {
        for import in &ty.imports {
            let import = import.trim();
            if !import.is_empty() {
                parts.push(import.to_string());
            }
        }
        if let Some(mp) = &ty.model_preamble {
            if !mp.trim().is_empty() {
                parts.push(mp.trim_end().to_string());
            }
        }
    }
    if let Some(mp) = &ir.model_preamble {
        if !mp.trim().is_empty() {
            parts.push(mp.trim_end().to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// Render one obligation from its recipe. Pure and deterministic.
pub fn render(
    family: Family,
    ty: Option<&TypeRecipe>,
    ir: &InterfaceRecipe,
) -> Result<Rendered, String> {
    if ir.assertion.trim().is_empty() {
        return Err("interface recipe has an empty `assert` expression".into());
    }
    let construct = ir
        .construct
        .as_deref()
        .map(str::trim)
        .or_else(|| ty.map(|t| t.construct.trim()))
        .unwrap_or("");
    let has_receiver = !construct.is_empty();
    if has_receiver && construct.ends_with(';') {
        return Err("type recipe `construct` must be an expression, not a statement".into());
    }
    let mut lines: Vec<String> = Vec::new();
    if let Some(ty) = ty {
        push_block(&mut lines, ty.setup.as_ref());
    }

    let call = ir.call.trim();

    match family {
        Family::ValueReadback | Family::IsNoneDemand | Family::GuardIdentity => {
            if call.is_empty() {
                return Err(format!(
                    "family {} requires a non-empty `call`",
                    family.as_str()
                ));
            }
            push_bind_attr(&mut lines, ty, has_receiver);
            if has_receiver {
                let mut_kw = if ir.receiver_mut { "mut " } else { "" };
                lines.push(format!("let {mut_kw}recv = {construct};"));
            }
            let binding = if family == Family::GuardIdentity {
                "handle"
            } else {
                "out"
            };
            let pairs: Vec<(&str, &str)> = if has_receiver {
                vec![("{recv}", "recv")]
            } else {
                Vec::new()
            };
            push_block(&mut lines, ir.bridge.as_ref());
            // bridge may use {recv}
            let bridged: Vec<String> = lines
                .iter()
                .map(|line| substitute(line, &pairs))
                .collect::<Result<_, _>>()?;
            lines = bridged;
            let call = substitute(call, &pairs)?;
            let binding_mode = ir.bind.as_deref().unwrap_or("let");
            if !matches!(binding_mode, "let" | "if-let-some") {
                return Err(format!(
                    "unknown `bind` mode {binding_mode:?} (expected `let` or `if-let-some`)"
                ));
            }
            let call_pairs = {
                let mut p = pairs.clone();
                p.push(("{out}", "out"));
                p.push(("{handle}", "handle"));
                p
            };
            if family == Family::GuardIdentity
                && !ir.assertion.contains("{handle}")
                && !ir.probes.iter().any(|p| p.contains("{handle}"))
            {
                return Err(
                    "guard-identity obligation never references {handle} (no relation checked)"
                        .into(),
                );
            }
            let mut body: Vec<String> = Vec::new();
            for probe in &ir.probes {
                body.push(substitute(probe, &call_pairs)?);
            }
            body.push(format!(
                "assert({});",
                substitute(&ir.assertion, &call_pairs)?
            ));
            if ir.hold {
                body.push(format!("let _held = {binding};"));
            }
            if binding_mode == "if-let-some" {
                lines.push(format!("if let Some({binding}) = {call} {{"));
                for line in body {
                    lines.push(format!("    {line}"));
                }
                lines.push("}".into());
            } else {
                lines.push(format!("let {binding} = {call};"));
                lines.extend(body);
            }
        },
        Family::FinalRelation => {
            push_bind_attr(&mut lines, ty, has_receiver);
            if has_receiver {
                lines.push(format!("let mut recv = {construct};"));
            }
            let pairs: Vec<(&str, &str)> = if has_receiver {
                vec![("{recv}", "recv")]
            } else {
                Vec::new()
            };
            for block in [ir.bridge.as_ref(), ir.snapshot.as_ref()] {
                if let Some(block) = block {
                    for line in block.lines() {
                        if !line.trim().is_empty() {
                            lines.push(substitute(line, &pairs)?);
                        }
                    }
                }
            }
            if !call.is_empty() {
                lines.push(format!("{};", substitute(call, &pairs)?));
            }
            for probe in &ir.probes {
                lines.push(substitute(probe, &pairs)?);
            }
            if has_receiver && !ir.assertion.contains("{recv}") {
                return Err(
                    "final-relation obligation does not reference {recv} (not a receiver post-state)"
                        .into(),
                );
            }
            lines.push(format!("assert({});", substitute(&ir.assertion, &pairs)?));
        },
        Family::Other => {
            return Err("family `other` has no obligation skeleton".into());
        },
    }

    let obligation = lines.join("\n");
    if obligation.trim().is_empty() {
        return Err("rendered obligation is empty".into());
    }
    Ok(Rendered {
        obligation,
        model_preamble: preamble_of(ty, ir),
    })
}

// ---------------------------------------------------------------------------
// Synthesis
// ---------------------------------------------------------------------------

/// Per-candidate disposition in the synthesis report.
#[derive(Debug, Clone, Serialize)]
pub struct EntryAudit {
    pub candidate_id: String,
    pub function: String,
    pub family: String,
    pub constructibility: String,
    pub registry_overlap: String,
    pub dedup_key: String,
    pub entry_name: String,
    pub receiver_type: Option<String>,
    pub expect: Option<String>,
    pub expect_source: String,
    pub expect_basis: Option<String>,
    pub obligation: String,
    pub obligation_draft: Option<String>,
}

/// A skipped candidate with the reason it did not enter the manifest.
#[derive(Debug, Clone, Serialize)]
pub struct SkipAudit {
    pub candidate_id: String,
    pub function: String,
    pub family: String,
    pub constructibility: String,
    pub reason: String,
    pub detail: Option<String>,
}

/// Synthesis report (written next to the manifest; not consumed by the runner).
#[derive(Debug, Clone, Serialize)]
pub struct SynthesisReport {
    pub candidates_total: usize,
    pub candidates_selected: usize,
    pub synthesized: usize,
    pub skipped: usize,
    pub entries: Vec<EntryAudit>,
    pub skipped_candidates: Vec<SkipAudit>,
}

#[derive(Debug, Clone)]
pub struct Synthesis {
    pub manifest: DivergenceManifest,
    pub report: SynthesisReport,
}

/// Remove angle-bracket groups from a function path, keeping `Type::method`.
/// `<LinkedList<M> as Default>::default` becomes ` as Default::default`; the
/// canonical form is only used for selection matching and entry naming.
pub fn canonical_function(function: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in function.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {},
        }
    }
    out.trim().to_string()
}

/// Stable entry name: candidate number plus canonical function path.
pub fn entry_name(candidate: &Candidate) -> String {
    let digits: String = candidate
        .id
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect();
    let stem = canonical_function(&candidate.function)
        .replace("::", "_")
        .to_lowercase();
    let stem = sanitize(&stem);
    if digits.is_empty() {
        format!("c_{stem}")
    } else {
        format!("c{digits}_{stem}")
    }
}

fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn select_matches(token: &str, candidate: &Candidate) -> bool {
    let canonical = canonical_function(&candidate.function);
    token == candidate.id
        || token.eq_ignore_ascii_case(&candidate.function)
        || token.eq_ignore_ascii_case(&canonical)
        || token.eq_ignore_ascii_case(&entry_name(candidate))
        || (!token.is_empty() && normalize_token(token) == normalize_token(&candidate.id))
}

fn resolve_expect(
    candidate: &Candidate,
    ir: &InterfaceRecipe,
) -> Result<(Option<Expectation>, &'static str, Option<String>), String> {
    if let Some(raw) = &ir.expect {
        let basis = ir
            .expect_basis
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                format!(
                    "interface recipe for {} overrides `expect` without `expect_basis`",
                    candidate.function
                )
            })?;
        let expect = match raw.as_str() {
            "gap" => Some(Expectation::Gap),
            "supported" => Some(Expectation::Supported),
            "none" => None,
            other => {
                return Err(format!(
                    "interface recipe for {} has unknown expect {other:?}",
                    candidate.function
                ));
            },
        };
        return Ok((expect, "recipe-override", Some(basis.to_string())));
    }
    match candidate.prereg_expected.as_deref() {
        Some("gap") => Ok((Some(Expectation::Gap), "candidate-prereg", None)),
        Some("supported") => Ok((Some(Expectation::Supported), "candidate-prereg", None)),
        Some("inconclusive") | None => Ok((None, "none", None)),
        Some(other) => Err(format!(
            "candidate {} has unknown prereg_expected {other:?}",
            candidate.id
        )),
    }
}

fn resolve_interface<'a>(
    recipes: &'a Recipes,
    candidate: &'a Candidate,
) -> Option<(&'a str, &'a InterfaceRecipe)> {
    if let Some(ir) = recipes.interfaces.get(&candidate.function) {
        return Some((candidate.function.as_str(), ir));
    }
    let canonical = canonical_function(&candidate.function);
    for (key, ir) in &recipes.interfaces {
        if canonical_function(key) == canonical {
            return Some((key.as_str(), ir));
        }
    }
    None
}

/// Pure synthesis: candidates + recipes + selection -> manifest + report.
pub fn synthesize(
    candidates: &[Candidate],
    recipes: &Recipes,
    select: &[String],
) -> Result<Synthesis, String> {
    let tokens: Vec<&str> = select
        .iter()
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut matched_tokens = vec![false; tokens.len()];

    let mut contracts = Vec::new();
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut seen_dedup: BTreeMap<String, String> = BTreeMap::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut selected_count = 0usize;

    for candidate in candidates {
        if !tokens.is_empty() {
            let mut hit = false;
            for (i, token) in tokens.iter().enumerate() {
                if select_matches(token, candidate) {
                    matched_tokens[i] = true;
                    hit = true;
                }
            }
            if !hit {
                skipped.push(SkipAudit {
                    candidate_id: candidate.id.clone(),
                    function: candidate.function.clone(),
                    family: candidate.family.clone(),
                    constructibility: candidate.constructibility.clone(),
                    reason: "not-selected".into(),
                    detail: None,
                });
                continue;
            }
        }
        selected_count += 1;

        let Some(family) = Family::parse(&candidate.family) else {
            return Err(format!(
                "candidate {} has unknown family {:?}",
                candidate.id, candidate.family
            ));
        };
        if !family.has_template() {
            skipped.push(SkipAudit {
                candidate_id: candidate.id.clone(),
                function: candidate.function.clone(),
                family: candidate.family.clone(),
                constructibility: candidate.constructibility.clone(),
                reason: "no-template-family".into(),
                detail: Some("family `other` has no generic skeleton".into()),
            });
            continue;
        }

        let dedup_key = candidate
            .dedup_key
            .clone()
            .unwrap_or_else(|| format!("id:{}", candidate.id));
        if let Some(first) = seen_dedup.get(&dedup_key) {
            skipped.push(SkipAudit {
                candidate_id: candidate.id.clone(),
                function: candidate.function.clone(),
                family: candidate.family.clone(),
                constructibility: candidate.constructibility.clone(),
                reason: "dedup".into(),
                detail: Some(format!("dedup_key {dedup_key:?} already synthesized by {first}")),
            });
            continue;
        }

        let Some((_recipe_key, ir)) = resolve_interface(recipes, candidate) else {
            let (reason, detail) = match candidate.constructibility.as_str() {
                "needs-recipe" => (
                    "needs-recipe",
                    candidate
                        .constructibility_note
                        .clone()
                        .or_else(|| candidate.constructibility_reason.clone()),
                ),
                "blocked" => (
                    "blocked",
                    candidate
                        .constructibility_reason
                        .clone()
                        .or_else(|| candidate.constructibility_note.clone()),
                ),
                _ => (
                    "no-interface-recipe",
                    Some("no interface recipe for this function".into()),
                ),
            };
            skipped.push(SkipAudit {
                candidate_id: candidate.id.clone(),
                function: candidate.function.clone(),
                family: candidate.family.clone(),
                constructibility: candidate.constructibility.clone(),
                reason: reason.into(),
                detail,
            });
            continue;
        };

        if let Some(recipe_family) = &ir.family {
            let Some(declared) = Family::parse(recipe_family) else {
                return Err(format!(
                    "interface recipe for {} has unknown family {:?}",
                    candidate.function, recipe_family
                ));
            };
            if declared != family {
                return Err(format!(
                    "interface recipe family {:?} does not match candidate {} family {:?}",
                    recipe_family, candidate.id, candidate.family
                ));
            }
        }

        let ty = match &ir.receiver_type {
            Some(name) => Some(
                recipes
                    .types
                    .get(name)
                    .ok_or_else(|| {
                        format!(
                            "interface recipe for {} references unknown type recipe {name:?}",
                            candidate.function
                        )
                    })?,
            ),
            None => None,
        };

        let rendered = render(family, ty, ir)?;
        let name = entry_name(candidate);
        if !seen_names.insert(name.clone()) {
            return Err(format!("duplicate synthesized entry name {name:?}"));
        }
        let (expect, expect_source, expect_basis) = resolve_expect(candidate, ir)?;

        entries.push(EntryAudit {
            candidate_id: candidate.id.clone(),
            function: candidate.function.clone(),
            family: candidate.family.clone(),
            constructibility: candidate.constructibility.clone(),
            registry_overlap: candidate
                .registry_overlap
                .clone()
                .unwrap_or_else(|| "none".into()),
            dedup_key: dedup_key.clone(),
            entry_name: name.clone(),
            receiver_type: ir.receiver_type.clone(),
            expect: expect.map(|e| match e {
                Expectation::Gap => "gap".into(),
                Expectation::Supported => "supported".into(),
            }),
            expect_source: expect_source.into(),
            expect_basis,
            obligation: rendered.obligation.clone(),
            obligation_draft: candidate.obligation_draft.clone(),
        });
        contracts.push(ObligationEntry {
            name,
            params: String::new(),
            obligation: rendered.obligation,
            model_preamble: rendered.model_preamble,
            expect,
        });
        seen_dedup.insert(dedup_key, candidate.id.clone());
    }

    let unmatched: Vec<&str> = tokens
        .iter()
        .zip(matched_tokens.iter())
        .filter(|(_, matched)| !**matched)
        .map(|(token, _)| *token)
        .collect();
    if !unmatched.is_empty() {
        return Err(format!(
            "--select token(s) matched no candidate: {}",
            unmatched.join(", ")
        ));
    }
    if contracts.is_empty() {
        return Err("synthesis produced no obligation entries".into());
    }

    let report = SynthesisReport {
        candidates_total: candidates.len(),
        candidates_selected: selected_count,
        synthesized: entries.len(),
        skipped: skipped.len(),
        entries,
        skipped_candidates: skipped,
    };
    Ok(Synthesis {
        manifest: DivergenceManifest {
            target_workspace: recipes.target_workspace.clone(),
            contracts,
        },
        report,
    })
}

/// Read inputs, synthesize, and write the manifest + report.
///
/// The report path defaults to `<out stem>.report.json` (e.g.
/// `manifest.json` -> `manifest.report.json`).
pub fn run(
    candidates_path: &Path,
    recipes_path: &Path,
    out_path: &Path,
    report_path: Option<&Path>,
    select: &[String],
) -> Result<SynthesisReport, String> {
    let candidates: Vec<Candidate> = serde_json::from_slice(
        &std::fs::read(candidates_path).map_err(|e| format!("{}: {e}", candidates_path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", candidates_path.display()))?;
    let recipes: Recipes = serde_json::from_slice(
        &std::fs::read(recipes_path).map_err(|e| format!("{}: {e}", recipes_path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", recipes_path.display()))?;

    let synthesis = synthesize(&candidates, &recipes, select)?;
    let manifest_json = serde_json::to_string_pretty(&synthesis.manifest)
        .map_err(|e| format!("serialize manifest: {e}"))?;
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(out_path, format!("{manifest_json}\n"))
        .map_err(|e| format!("{}: {e}", out_path.display()))?;

    let derived_report;
    let report_path = match report_path {
        Some(path) => path,
        None => {
            let stem = out_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "synthesis".into());
            derived_report = out_path.with_file_name(format!("{stem}.report.json"));
            &derived_report
        },
    };
    let report_json = serde_json::to_string_pretty(&synthesis.report)
        .map_err(|e| format!("serialize report: {e}"))?;
    std::fs::write(report_path, format!("{report_json}\n"))
        .map_err(|e| format!("{}: {e}", report_path.display()))?;

    Ok(synthesis.report)
}

// ---------------------------------------------------------------------------
// Tests (generic names only: no target-project knowledge in this crate)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates_json() -> Vec<Candidate> {
        serde_json::from_str(
            r#"[
              {
                "id": "C-001",
                "function": "Widget<'_, Mode>::limit",
                "family": "final-relation",
                "constructibility": "constructible",
                "prereg_expected": "gap",
                "dedup_key": "fr:widget-limit",
                "obligation_draft": "let mut w = Widget::from(&buf[..]);\nlet _ = w.limit(4);\nassert(final(w).remain() <= 4);"
              },
              {
                "id": "C-002",
                "function": "Widget<'_, Mode>::has_remain",
                "family": "is_none-demand",
                "constructibility": "constructible",
                "prereg_expected": "gap",
                "dedup_key": "demand:widget-has-remain"
              },
              {
                "id": "C-003",
                "function": "Cache<T>::get",
                "family": "value-readback",
                "constructibility": "constructible",
                "prereg_expected": "supported",
                "dedup_key": "vr:cache-get"
              },
              {
                "id": "C-004",
                "function": "Cache<T>::clone",
                "family": "other",
                "constructibility": "blocked",
                "prereg_expected": "inconclusive",
                "dedup_key": "other:cache-clone"
              },
              {
                "id": "C-005",
                "function": "Pool<T>::alloc",
                "family": "value-readback",
                "constructibility": "needs-recipe",
                "constructibility_reason": "needs-type-family-recipe",
                "constructibility_note": "needs a tracked owner",
                "prereg_expected": "gap",
                "dedup_key": "vr:pool-alloc"
              },
              {
                "id": "C-006",
                "function": "Widget<'_, Mode>::limit_dup",
                "family": "final-relation",
                "constructibility": "constructible",
                "prereg_expected": "gap",
                "dedup_key": "fr:widget-limit"
              }
            ]"#,
        )
        .unwrap()
    }

    fn recipes_json() -> Recipes {
        serde_json::from_str(
            r#"{
              "target_workspace": {
                "manifest_path": "/tmp/target/Cargo.toml",
                "inject_dir": "demo/src/probes",
                "gate_cfg": "ktest",
                "extra_cfgs": ["capture_gap"],
                "target": "x86_64-unknown-none"
              },
              "types": {
                "Widget": {
                  "imports": ["use demo::Widget;"],
                  "setup": "let buf = [0u8; 8];",
                  "construct": "Widget::from(&buf[..])"
                },
                "Cache": {
                  "imports": ["use demo::Cache;"],
                  "construct": "Cache::<u64>::new(5u64)"
                }
              },
              "interfaces": {
                "Widget<'_, Mode>::limit": {
                  "receiver_type": "Widget",
                  "family": "final-relation",
                  "call": "{recv}.limit(4)",
                  "snapshot": "let before = {recv}.cursor.vaddr;",
                  "assert": "{recv}.remain_spec() <= 4",
                  "expect": "supported",
                  "expect_basis": "test-only override"
                },
                "Widget<'_, Mode>::has_remain": {
                  "receiver_type": "Widget",
                  "call": "{recv}.has_remain()",
                  "assert": "{out}"
                },
                "Cache<T>::get": {
                  "receiver_type": "Cache",
                  "call": "{recv}.get()",
                  "probes": ["let flag = {out}.is_some();"],
                  "assert": "flag",
                  "hold": true
                }
              }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn renders_final_relation_with_snapshot() {
        let recipes = recipes_json();
        let ty = recipes.types.get("Widget").unwrap();
        let ir = recipes
            .interfaces
            .get("Widget<'_, Mode>::limit")
            .unwrap();
        let rendered = render(Family::FinalRelation, Some(ty), ir).unwrap();
        assert_eq!(
            rendered.obligation,
            "let buf = [0u8; 8];\nlet mut recv = Widget::from(&buf[..]);\nlet before = recv.cursor.vaddr;\nrecv.limit(4);\nassert(recv.remain_spec() <= 4);"
        );
        assert_eq!(rendered.model_preamble.as_deref(), Some("use demo::Widget;"));
    }

    #[test]
    fn renders_value_readback_with_probes_and_hold() {
        let recipes = recipes_json();
        let ty = recipes.types.get("Cache").unwrap();
        let ir = recipes.interfaces.get("Cache<T>::get").unwrap();
        let rendered = render(Family::ValueReadback, Some(ty), ir).unwrap();
        assert_eq!(
            rendered.obligation,
            "let recv = Cache::<u64>::new(5u64);\nlet out = recv.get();\nlet flag = out.is_some();\nassert(flag);\nlet _held = out;"
        );
    }

    #[test]
    fn renders_guard_identity_and_requires_handle_relation() {
        let recipes = recipes_json();
        let ty = recipes.types.get("Cache").unwrap();
        let ir: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "receiver_type": "Cache",
            "call": "{recv}.lock()",
            "assert": "{handle}.rel_cache({recv})"
        }))
        .unwrap();
        let rendered = render(Family::GuardIdentity, Some(ty), &ir).unwrap();
        assert!(rendered.obligation.contains("let handle = recv.lock();"));
        assert!(rendered.obligation.contains("assert(handle.rel_cache(recv));"));

        let bad: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "receiver_type": "Cache",
            "call": "{recv}.lock()",
            "assert": "true"
        }))
        .unwrap();
        let err = render(Family::GuardIdentity, Some(ty), &bad).unwrap_err();
        assert!(err.contains("{handle}"));
    }

    #[test]
    fn renders_optional_acquisition_with_if_let() {
        let recipes = recipes_json();
        let ty = recipes.types.get("Cache").unwrap();
        let ir: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "receiver_type": "Cache",
            "call": "{recv}.try_get()",
            "bind": "if-let-some",
            "assert": "{out}.view() == 5u64",
            "hold": true
        }))
        .unwrap();
        let rendered = render(Family::ValueReadback, Some(ty), &ir).unwrap();
        assert_eq!(
            rendered.obligation,
            "let recv = Cache::<u64>::new(5u64);\nif let Some(out) = recv.try_get() {\n    assert(out.view() == 5u64);\n    let _held = out;\n}"
        );
    }

    #[test]
    fn family_mismatch_is_an_error() {
        let candidates = candidates_json();
        let mut recipes = recipes_json();
        recipes
            .interfaces
            .get_mut("Widget<'_, Mode>::limit")
            .unwrap()
            .family = Some("value-readback".into());
        let err = synthesize(&candidates, &recipes, &[]).unwrap_err();
        assert!(err.contains("does not match candidate"));
    }

    #[test]
    fn missing_recipe_degrades_to_skip_with_reason() {
        let candidates = candidates_json();
        let recipes = recipes_json();
        let synthesis = synthesize(&candidates, &recipes, &[]).unwrap();
        assert_eq!(synthesis.manifest.contracts.len(), 3);
        let reasons: BTreeMap<&str, &str> = synthesis
            .report
            .skipped_candidates
            .iter()
            .map(|s| (s.candidate_id.as_str(), s.reason.as_str()))
            .collect();
        assert_eq!(reasons.get("C-003"), None);
        assert_eq!(reasons["C-004"], "no-template-family");
        assert_eq!(reasons["C-005"], "needs-recipe");
        assert_eq!(reasons["C-006"], "dedup");
        assert!(synthesis
            .report
            .skipped_candidates
            .iter()
            .find(|s| s.candidate_id == "C-005")
            .unwrap()
            .detail
            .as_deref()
            .unwrap()
            .contains("tracked owner"));
    }

    #[test]
    fn expect_inheritance_and_override_are_audited() {
        let candidates = candidates_json();
        let recipes = recipes_json();
        let synthesis = synthesize(&candidates, &recipes, &[]).unwrap();
        let limit = &synthesis.report.entries[0];
        assert_eq!(limit.candidate_id, "C-001");
        // recipe override wins over candidate prereg "gap"
        assert_eq!(limit.expect.as_deref(), Some("supported"));
        assert_eq!(limit.expect_source, "recipe-override");
        assert_eq!(limit.expect_basis.as_deref(), Some("test-only override"));

        let demand = &synthesis.report.entries[1];
        assert_eq!(demand.expect.as_deref(), Some("gap"));
        assert_eq!(demand.expect_source, "candidate-prereg");
        assert!(demand.expect_basis.is_none());

        // inconclusive prereg is carried as "no machine expectation"
        assert_eq!(synthesis.report.entries.len(), 3);
        assert_eq!(
            synthesis.report.entries[2].expect.as_deref(),
            Some("supported")
        );
    }

    #[test]
    fn expect_override_requires_basis() {
        let candidates = candidates_json();
        let mut recipes = recipes_json();
        recipes
            .interfaces
            .get_mut("Widget<'_, Mode>::limit")
            .unwrap()
            .expect_basis = None;
        let err = synthesize(&candidates, &recipes, &[]).unwrap_err();
        assert!(err.contains("expect_basis"));
    }

    #[test]
    fn selection_by_id_function_and_entry_name() {
        let candidates = candidates_json();
        let recipes = recipes_json();
        let by_id = synthesize(&candidates, &recipes, &["c001".into()]).unwrap();
        assert_eq!(by_id.manifest.contracts.len(), 1);
        assert_eq!(by_id.manifest.contracts[0].name, "c001_widget_limit");

        let by_function =
            synthesize(&candidates, &recipes, &["Cache<T>::get".into()]).unwrap();
        assert_eq!(by_function.manifest.contracts.len(), 1);
        assert_eq!(by_function.manifest.contracts[0].name, "c003_cache_get");

        let by_entry = synthesize(
            &candidates,
            &recipes,
            &["c002_widget_has_remain".into()],
        )
        .unwrap();
        assert_eq!(by_entry.manifest.contracts.len(), 1);

        let err = synthesize(&candidates, &recipes, &["C-404".into()]).unwrap_err();
        assert!(err.contains("matched no candidate"));
    }

    #[test]
    fn deterministic_output_and_placeholder_validation() {
        let candidates = candidates_json();
        let recipes = recipes_json();
        let a = synthesize(&candidates, &recipes, &[]).unwrap();
        let b = synthesize(&candidates, &recipes, &[]).unwrap();
        assert_eq!(
            serde_json::to_string(&a.manifest).unwrap(),
            serde_json::to_string(&b.manifest).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&a.report).unwrap(),
            serde_json::to_string(&b.report).unwrap()
        );

        let ty = recipes.types.get("Widget").unwrap();
        let ir: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "receiver_type": "Widget",
            "call": "{recv}.limit(4)",
            "assert": "{recv} == {out}"
        }))
        .unwrap();
        let err = render(Family::FinalRelation, Some(ty), &ir).unwrap_err();
        assert!(err.contains("{out}"));
    }

    #[test]
    fn mutable_receiver_binding_is_supported() {
        let ty: TypeRecipe = serde_json::from_value(serde_json::json!({
            "construct": "Cache::<u64>::new(5u64)"
        }))
        .unwrap();
        let ir: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "receiver_type": "Cache",
            "receiver_mut": true,
            "call": "{recv}.update()",
            "assert": "true"
        }))
        .unwrap();
        let rendered = render(Family::ValueReadback, Some(&ty), &ir).unwrap();
        assert!(rendered.obligation.contains("let mut recv = Cache::<u64>::new(5u64);"));
    }

    #[test]
    fn receiver_binding_attribute_is_emitted() {
        let ty: TypeRecipe = serde_json::from_value(serde_json::json!({
            "construct": "Widget::from(&buf[..])",
            "bind_attr": "#[verus_spec(with => Tracked(owner))]"
        }))
        .unwrap();
        let ir: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "call": "{recv}.limit(4)",
            "assert": "{recv}.remain_spec() <= 4"
        }))
        .unwrap();
        let rendered = render(Family::FinalRelation, Some(&ty), &ir).unwrap();
        let lines: Vec<&str> = rendered.obligation.lines().collect();
        assert_eq!(lines[0], "#[verus_spec(with => Tracked(owner))]");
        assert_eq!(lines[1], "let mut recv = Widget::from(&buf[..]);");
    }

    #[test]
    fn receiverless_interface_is_supported() {
        let ty: TypeRecipe = serde_json::from_value(serde_json::json!({
            "imports": ["use demo::Clock;"],
            "construct": ""
        }))
        .unwrap();
        let ir: InterfaceRecipe = serde_json::from_value(serde_json::json!({
            "receiver_type": "Clock",
            "call": "Clock::now()",
            "assert": "{out} >= 0"
        }))
        .unwrap();
        let rendered = render(Family::ValueReadback, Some(&ty), &ir).unwrap();
        assert_eq!(rendered.obligation, "let out = Clock::now();\nassert(out >= 0);");
        assert!(!rendered.obligation.contains("let recv"));
    }

    #[test]
    fn bogus_parse_inputs_are_rejected() {
        let candidates = candidates_json();
        let mut recipes = recipes_json();
        recipes
            .interfaces
            .get_mut("Widget<'_, Mode>::limit")
            .unwrap()
            .receiver_type = Some("Nope".into());
        let err = synthesize(&candidates, &recipes, &[]).unwrap_err();
        assert!(err.contains("unknown type recipe"));

        let mut recipes = recipes_json();
        recipes
            .interfaces
            .get_mut("Widget<'_, Mode>::has_remain")
            .unwrap()
            .call = String::new();
        let err = synthesize(&candidates, &recipes, &[]).unwrap_err();
        assert!(err.contains("requires a non-empty `call`"));
    }

    #[test]
    fn canonical_function_and_entry_naming() {
        assert_eq!(
            canonical_function("Widget<'_, Mode>::limit"),
            "Widget::limit"
        );
        assert_eq!(
            canonical_function("<List<M> as Default>::default"),
            "::default"
        );
        let candidate: Candidate = serde_json::from_value(serde_json::json!({
            "id": "C-042",
            "function": "Widget<'_, Mode>::limit"
        }))
        .unwrap();
        assert_eq!(entry_name(&candidate), "c042_widget_limit");
    }
}
