//! `ledger`: attach machine-analysis evidence to a research ledger
//! (read-only) and emit a reviewed patch proposal.
//!
//! The tool owns no project knowledge. It consumes generic report shapes
//! (`verdicts[]` with `name`/`verdict`/`detail`), an optional candidate index
//! (`id`/`function`/`dedup_key`/`entry_name`), and a registry with
//! `findings[]` (`id`/`kind`/`subject`/`dedup_key`/`interfaces`/`evidence[]`).
//! Project-side aliases and human disambiguation live in the optional
//! `--map` file.
//!
//! Safety boundary: the registry is never written. The patch never proposes
//! a `machine_status` transition (`status_suggestion` is always `null`);
//! status changes and registry application remain human-only. Unmatched and
//! ambiguous evidence is reported as such, never guessed.
//!
//! Design: `docs/M7d-ledger-automation.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const REPORT_SCHEMA: &str = "spec-check.ledger-report/1";
pub const PATCH_SCHEMA: &str = "spec-check.ledger-patch/1";
pub const MAP_SCHEMA: &str = "spec-check.ledger-map/1";

// ---------------------------------------------------------------------------
// Registry input (read-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Registry {
    #[serde(default)]
    pub schema: Value,
    #[serde(default)]
    pub findings: Vec<RegistryFinding>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RegistryFinding {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub relation: String,
    #[serde(default)]
    pub dedup_key: String,
    #[serde(default)]
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub discovered_by: Vec<String>,
    #[serde(default)]
    pub machine_status: String,
    #[serde(default)]
    pub evidence: Vec<RegistryEvidence>,
    #[serde(default)]
    pub human_review: Value,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RegistryEvidence {
    #[serde(default)]
    pub kind: String,
    #[serde(rename = "ref", default)]
    pub reference: String,
    #[serde(default)]
    pub detail: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Optional project-side map
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct LedgerMap {
    #[serde(default)]
    #[allow(dead_code)]
    schema: Option<String>,
    #[serde(default)]
    rules: Vec<MapRule>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MapRule {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    report_format: Option<String>,
    #[serde(default)]
    registry_id: Option<String>,
    #[serde(default)]
    dedup_key: Option<String>,
    #[serde(default)]
    evidence_kind: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
}

// ---------------------------------------------------------------------------
// Candidate index (enrichment only, never evidence)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct CandidateHint {
    id: String,
    entry_name: String,
    function: String,
    dedup_key: Option<String>,
}

#[derive(Debug, Default)]
struct CandidateIndex {
    by_entry: BTreeMap<String, CandidateHint>,
    by_id: BTreeMap<String, CandidateHint>,
    by_number: BTreeMap<String, CandidateHint>,
}

impl CandidateIndex {
    fn insert(&mut self, hint: CandidateHint) {
        if !hint.entry_name.is_empty() {
            self.by_entry
                .entry(hint.entry_name.to_ascii_lowercase())
                .or_insert_with(|| hint.clone());
        }
        if !hint.id.is_empty() {
            let id = hint.id.to_ascii_lowercase();
            self.by_id.entry(id.clone()).or_insert_with(|| hint.clone());
            if let Some(n) = numeric_id(&id) {
                self.by_number.entry(n).or_insert(hint);
            }
        }
    }

    fn lookup(&self, entry: &str) -> Option<CandidateHint> {
        let e = entry.to_ascii_lowercase();
        if let Some(h) = self.by_entry.get(&e) {
            return Some(h.clone());
        }
        if let Some(n) = leading_number(&e)
            && let Some(h) = self.by_number.get(&n)
        {
            return Some(h.clone());
        }
        self.by_id.get(&e).cloned()
    }
}

// ---------------------------------------------------------------------------
// Records and output schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VerdictRecord {
    name: String,
    verdict: String,
    detail: String,
}

#[derive(Debug, Clone)]
struct ParsedReport {
    label: String,
    format: String,
    verdicts: Vec<VerdictRecord>,
}

#[derive(Debug, Clone, Serialize)]
struct ProposedEvidence {
    kind: String,
    #[serde(rename = "ref")]
    reference: String,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProposedEntry {
    id: Option<String>,
    kind: String,
    subject: String,
    relation: String,
    dedup_key: String,
    interfaces: Vec<String>,
    discovered_by: Vec<String>,
    machine_status: String,
    evidence: Vec<ProposedEvidence>,
    human_review: Value,
}

#[derive(Debug, Clone, Serialize)]
struct CandidateOut {
    registry_id: String,
    dedup_key: String,
    score: u32,
    tier: String,
}

#[derive(Debug, Clone, Serialize)]
struct Record {
    pub source: String,
    pub source_format: String,
    pub entry: String,
    pub verdict: String,
    pub evidence_kind: String,
    pub actionable: bool,
    pub control: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_dedup_key: Option<String>,
    pub detail: String,
    pub match_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_score: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_dedup_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_candidates: Vec<CandidateOut>,
    pub action: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_into: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_evidence: Option<ProposedEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_entry: Option<ProposedEntry>,
    pub status_suggestion: Option<Value>,
    pub requires_human: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LedgerSummary {
    pub records: usize,
    pub actionable: usize,
    pub matched: usize,
    pub ambiguous: usize,
    pub unmatched: usize,
    pub append_evidence: usize,
    pub new_entry: usize,
    pub noop: usize,
    pub action_unmatched: usize,
    pub registry_findings: usize,
    pub appended_findings: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RegistryMeta {
    path: String,
    findings: usize,
    bytes: usize,
    unchanged_after_run: bool,
}

#[derive(Debug, Clone, Serialize)]
struct InputSummary {
    source: String,
    format: String,
    entries: usize,
    actionable: usize,
}

#[derive(Debug, Clone, Serialize)]
struct LedgerReport {
    schema: &'static str,
    registry: RegistryMeta,
    inputs: Vec<InputSummary>,
    diagnostics: Vec<String>,
    records: Vec<Record>,
    summary: LedgerSummary,
}

#[derive(Debug, Clone, Serialize)]
struct RegistryPatch {
    schema: &'static str,
    requires_human: bool,
    note: &'static str,
    registry: RegistryMeta,
    summary: LedgerSummary,
    proposals: Vec<Record>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run(
    registry_path: &Path,
    report_paths: &[PathBuf],
    candidate_paths: &[PathBuf],
    map_path: Option<&Path>,
    out: &Path,
) -> Result<LedgerSummary, String> {
    let registry_bytes = fs::read(registry_path)
        .map_err(|e| format!("read registry {}: {e}", registry_path.display()))?;
    let registry: Registry = serde_json::from_slice(&registry_bytes)
        .map_err(|e| format!("parse registry {}: {e}", registry_path.display()))?;

    let map = match map_path {
        Some(p) => {
            let bytes = fs::read(p).map_err(|e| format!("read map {}: {e}", p.display()))?;
            let map: LedgerMap = serde_json::from_slice(&bytes)
                .map_err(|e| format!("parse map {}: {e}", p.display()))?;
            validate_map(&map, &registry, p)?;
            map
        }
        None => LedgerMap::default(),
    };

    let mut candidates = CandidateIndex::default();
    let mut candidate_paths = candidate_paths.to_vec();
    candidate_paths.sort();
    for p in &candidate_paths {
        let value = read_json(p)?;
        ingest_candidates(&value, &mut candidates)
            .map_err(|e| format!("candidates {}: {e}", p.display()))?;
    }

    let mut report_paths = report_paths.to_vec();
    report_paths.sort();
    let mut parsed = Vec::new();
    let mut diagnostics = Vec::new();
    for p in &report_paths {
        let value = read_json(p)?;
        let label = p.display().to_string();
        parse_report_value(&value, &label, &mut parsed, &mut diagnostics)
            .map_err(|e| format!("report {}: {e}", p.display()))?;
    }
    diagnostics.sort();
    diagnostics.dedup();

    let mut records = Vec::new();
    for report in &parsed {
        for verdict in &report.verdicts {
            let mut rec = Record {
                source: report.label.clone(),
                source_format: report.format.clone(),
                entry: verdict.name.clone(),
                verdict: verdict.verdict.clone(),
                evidence_kind: evidence_kind(&verdict.verdict, &verdict.detail).to_string(),
                actionable: is_actionable(&verdict.verdict),
                control: is_control(&verdict.name),
                function: None,
                candidate_dedup_key: None,
                detail: verdict.detail.clone(),
                match_status: "unmatched".into(),
                match_tier: None,
                match_score: None,
                registry_id: None,
                registry_dedup_key: None,
                match_candidates: Vec::new(),
                action: "unmatched".into(),
                reason: "no-match".into(),
                merged_into: None,
                sources: Vec::new(),
                proposed_evidence: None,
                proposed_entry: None,
                status_suggestion: None,
                requires_human: false,
            };
            if let Some(hint) = candidates.lookup(&rec.entry) {
                if !hint.function.is_empty() {
                    rec.function = Some(hint.function);
                }
                rec.candidate_dedup_key = hint.dedup_key;
            }
            records.push(rec);
        }
    }
    records
        .sort_by(|a, b| (&a.source, &a.entry, &a.verdict).cmp(&(&b.source, &b.entry, &b.verdict)));

    let indexes: Vec<FindingIndex> = registry.findings.iter().map(FindingIndex::new).collect();
    let mut group_index: BTreeMap<String, usize> = BTreeMap::new();

    for i in 0..records.len() {
        let decision = decide(&records[i], &registry.findings, &indexes, &map);
        apply_match(&mut records[i], &decision, &registry.findings);
        if let Some(kind) = &decision.evidence_kind_override {
            records[i].evidence_kind = kind.clone();
        }
        let plan = plan_action(&records[i], &decision, &registry.findings);

        match plan.action.as_str() {
            "append_evidence" | "new_entry" => {
                if let Some(&pi) = group_index.get(&plan.group) {
                    let source = records[i].source.clone();
                    let mut proposal = records[pi].clone();
                    if !proposal.sources.contains(&source) {
                        proposal.sources.push(source);
                    }
                    refresh_proposal(&mut proposal);
                    records[pi] = proposal;
                    records[i].action = "noop".into();
                    records[i].reason = "merged-into-proposal".into();
                    records[i].merged_into = Some(plan.group.clone());
                    records[i].requires_human = false;
                } else {
                    group_index.insert(plan.group.clone(), i);
                    records[i].action = plan.action.clone();
                    records[i].reason = plan.reason.clone();
                    records[i].sources = vec![records[i].source.clone()];
                    records[i].requires_human = true;
                    if plan.action == "new_entry" {
                        records[i].proposed_entry = Some(build_entry(&records[i]));
                    }
                    records[i].proposed_evidence = Some(build_evidence(&records[i]));
                }
            }
            _ => {
                records[i].action = plan.action;
                records[i].reason = plan.reason;
                records[i].requires_human = false;
            }
        }
    }
    for rec in &mut records {
        rec.status_suggestion = None;
    }

    let summary = summarize(&records, registry.findings.len());
    let inputs = summarize_inputs(&parsed);

    let after = fs::read(registry_path)
        .map_err(|e| format!("re-read registry {}: {e}", registry_path.display()))?;
    if after != registry_bytes {
        return Err(format!(
            "registry {} changed during the run; refusing to emit a patch",
            registry_path.display()
        ));
    }
    let meta = RegistryMeta {
        path: registry_path.display().to_string(),
        findings: registry.findings.len(),
        bytes: registry_bytes.len(),
        unchanged_after_run: true,
    };

    let report = LedgerReport {
        schema: REPORT_SCHEMA,
        registry: meta.clone(),
        inputs,
        diagnostics,
        records: records.clone(),
        summary: summary.clone(),
    };
    let mut proposals = records.clone();
    proposals.sort_by(|a, b| {
        (
            action_rank(&a.action),
            a.registry_id.as_deref().unwrap_or(""),
            &a.source,
            &a.entry,
        )
            .cmp(&(
                action_rank(&b.action),
                b.registry_id.as_deref().unwrap_or(""),
                &b.source,
                &b.entry,
            ))
    });
    let patch = RegistryPatch {
        schema: PATCH_SCHEMA,
        requires_human: true,
        note: "append evidence only; machine_status transitions and registry application are human-only",
        registry: meta,
        summary: summary.clone(),
        proposals,
    };

    fs::create_dir_all(out).map_err(|e| format!("create out dir {}: {e}", out.display()))?;
    write_json(&out.join("ledger-report.json"), &report)?;
    write_json(&out.join("registry-patch.json"), &patch)?;
    fs::write(
        out.join("DASHBOARD.md"),
        dashboard(&records, &registry, &parsed, &summary, &report.registry),
    )
    .map_err(|e| format!("write {}: {e}", out.join("DASHBOARD.md").display()))?;

    Ok(summary)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Report parsing
// ---------------------------------------------------------------------------

fn parse_report_value(
    value: &Value,
    label: &str,
    out: &mut Vec<ParsedReport>,
    diagnostics: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(verdicts) = value.get("verdicts").and_then(Value::as_array) {
        let mut parsed = Vec::new();
        for v in verdicts {
            let name = v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let verdict = v
                .get("verdict")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if name.is_empty() || verdict.is_empty() {
                diagnostics.push(format!(
                    "{label}: verdict record missing name/verdict (ignored)"
                ));
                continue;
            }
            let detail = v
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            parsed.push(VerdictRecord {
                name,
                verdict,
                detail,
            });
        }
        out.push(ParsedReport {
            label: label.to_string(),
            format: classify_verdicts(&parsed).to_string(),
            verdicts: parsed,
        });
        return Ok(());
    }
    if let Some(analyses) = value.get("analyses").and_then(Value::as_array) {
        for analysis in analyses {
            let Some(report) = analysis.get("report") else {
                continue;
            };
            let name = analysis
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("analysis");
            parse_report_value(report, &format!("{label}#{name}"), out, diagnostics)?;
        }
        return Ok(());
    }
    if value.get("entries").is_some() || value.is_array() {
        diagnostics.push(format!(
            "{label}: candidate/synthesis source (no verdicts; use --candidates)"
        ));
        return Ok(());
    }
    diagnostics.push(format!("{label}: unrecognized report shape (ignored)"));
    Ok(())
}

fn classify_verdicts(verdicts: &[VerdictRecord]) -> &'static str {
    let has = |s: &str| verdicts.iter().any(|v| v.verdict == s);
    if has("ContractGapCandidate") || has("ObligationSupported") || has("Inconclusive") {
        "divergence-report"
    } else if has("ConfirmedVacuous") || has("ConfirmedLive") || has("CandidateInconclusive") {
        "vacuity-report"
    } else {
        "probe-report"
    }
}

fn ingest_candidates(value: &Value, index: &mut CandidateIndex) -> Result<(), String> {
    if let Some(entries) = value.get("entries").and_then(Value::as_array) {
        for e in entries {
            index.insert(CandidateHint {
                id: string_field(e, "candidate_id"),
                entry_name: string_field(e, "entry_name"),
                function: string_field(e, "function"),
                dedup_key: opt_string_field(e, "dedup_key"),
            });
        }
        return Ok(());
    }
    if let Some(items) = value.as_array() {
        for e in items {
            index.insert(CandidateHint {
                id: string_field(e, "id"),
                entry_name: string_field(e, "entry_name"),
                function: string_field(e, "function"),
                dedup_key: opt_string_field(e, "dedup_key"),
            });
        }
        return Ok(());
    }
    Err("unrecognized candidate shape".into())
}

fn string_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn opt_string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

// ---------------------------------------------------------------------------
// Evidence normalization
// ---------------------------------------------------------------------------

fn evidence_kind(verdict: &str, detail: &str) -> &'static str {
    match verdict {
        "ContractGapCandidate" => "obligation_fail",
        "ObligationSupported" => "obligation_supported",
        "ConfirmedVacuous" if detail.trim_start().starts_with("construction:") => {
            "construction_probe"
        }
        "ConfirmedVacuous" => "probe_run",
        "ConfirmedLive" => "probe_run",
        "CandidateInconclusive" | "Inconclusive" => "probe_inconclusive",
        "Anomaly" => "anomaly",
        _ => "machine_evidence",
    }
}

fn is_actionable(verdict: &str) -> bool {
    matches!(verdict, "ContractGapCandidate" | "ConfirmedVacuous")
}

fn is_control(entry: &str) -> bool {
    let e = entry.to_ascii_lowercase();
    e.contains("_control") || e.contains("cross_negative") || e.contains("negative_control")
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct FindingIndex {
    tokens: BTreeSet<String>,
    anchors: BTreeSet<String>,
    explicit: Vec<(BTreeSet<String>, BTreeSet<String>)>,
    subject: Vec<(BTreeSet<String>, BTreeSet<String>)>,
}

impl FindingIndex {
    fn new(f: &RegistryFinding) -> Self {
        let mut tokens = BTreeSet::new();
        let mut anchors = BTreeSet::new();
        let mut add_text = |text: &str| {
            tokens.extend(ident_tokens(text));
            anchors.extend(identifier_words(text));
        };
        add_text(&f.kind);
        add_text(&f.subject);
        add_text(&f.relation);
        add_text(&f.dedup_key);
        add_text(&f.machine_status);
        for s in &f.interfaces {
            add_text(s);
        }
        for d in &f.discovered_by {
            add_text(d);
        }
        for e in &f.evidence {
            add_text(&e.kind);
            add_text(&e.reference);
            add_text(&e.detail);
        }

        let mut explicit = BTreeSet::new();
        for i in &f.interfaces {
            if let Some(pair) = interface_tokens(i) {
                explicit.insert(pair);
            }
        }
        let mut subject = BTreeSet::new();
        for pair in subject_interfaces(&f.subject) {
            subject.insert(pair);
        }

        Self {
            tokens,
            anchors,
            explicit: explicit.into_iter().collect(),
            subject: subject.into_iter().collect(),
        }
    }

    fn anchor_hits(&self, entry: &str) -> bool {
        let e = entry.to_ascii_lowercase();
        if self.anchors.contains(&e) {
            return true;
        }
        e.len() >= 8 && self.anchors.iter().any(|a| a.contains(&e))
    }

    fn explicit_matches(&self, recv: &BTreeSet<String>, method: &BTreeSet<String>) -> bool {
        self.explicit.iter().any(|(r, m)| r == recv && m == method)
    }

    fn subject_matches(&self, entry_tokens: &BTreeSet<String>) -> bool {
        self.subject.iter().any(|(recv, method)| {
            !recv.is_empty()
                && recv.is_subset(entry_tokens)
                && method.is_subset(entry_tokens)
                && entry_tokens.is_subset(&recv.union(method).cloned().collect())
        })
    }
}

#[derive(Debug, Default)]
struct Decision {
    status: &'static str,
    tier: Option<&'static str>,
    score: Option<u32>,
    finding: Option<usize>,
    candidates: Vec<CandidateOut>,
    evidence_kind_override: Option<String>,
}

fn decide(
    rec: &Record,
    findings: &[RegistryFinding],
    indexes: &[FindingIndex],
    map: &LedgerMap,
) -> Decision {
    if let Some(d) = map_match(rec, findings, map) {
        return d;
    }

    let entry_tokens = semantic_entry_tokens(&rec.entry);
    let function_pair = rec.function.as_deref().and_then(interface_tokens);
    let mut scored: Vec<(usize, u32, &'static str)> = Vec::new();

    for (i, f) in indexes.iter().enumerate() {
        let mut best = 0u32;
        let mut tier = "";
        if f.anchor_hits(&rec.entry) {
            best = 1000;
            tier = "entry-anchor";
        }
        if let Some(key) = &rec.candidate_dedup_key
            && !key.is_empty()
            && key == &findings[i].dedup_key
            && best < 950
        {
            best = 950;
            tier = "dedup-key";
        }
        if let Some((recv, method)) = &function_pair
            && f.explicit_matches(recv, method)
            && best < 800
        {
            best = 800;
            tier = "function-interface";
        }
        if f.subject_matches(&entry_tokens) && best < 790 {
            best = 790;
            tier = "function-interface";
        }
        if entry_tokens.len() >= 3 && entry_tokens.is_subset(&f.tokens) && best < 600 {
            best = 600;
            tier = "semantic-tokens";
        }
        if best > 0 {
            scored.push((i, best, tier));
        }
    }

    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if scored.is_empty() {
        return Decision {
            status: "unmatched",
            ..Decision::default()
        };
    }
    let top_score = scored[0].1;
    let tied = scored.iter().filter(|(_, s, _)| *s == top_score).count();
    let candidates = scored
        .iter()
        .map(|(i, score, tier)| CandidateOut {
            registry_id: findings[*i].id.clone(),
            dedup_key: findings[*i].dedup_key.clone(),
            score: *score,
            tier: tier.to_string(),
        })
        .collect();
    if tied > 1 {
        return Decision {
            status: "ambiguous",
            candidates,
            ..Decision::default()
        };
    }
    let (i, score, tier) = scored[0];
    Decision {
        status: "matched",
        tier: Some(tier),
        score: Some(score),
        finding: Some(i),
        candidates,
        evidence_kind_override: None,
    }
}

fn map_match(rec: &Record, findings: &[RegistryFinding], map: &LedgerMap) -> Option<Decision> {
    for rule in &map.rules {
        if !rule_matches(rule, rec) {
            continue;
        }
        let id = rule.registry_id.as_deref();
        let dk = rule.dedup_key.as_deref();
        if id.is_none() && dk.is_none() {
            continue;
        }
        let idx = findings
            .iter()
            .position(|f| id.is_none_or(|x| f.id == x) && dk.is_none_or(|x| f.dedup_key == x))?;
        let f = &findings[idx];
        return Some(Decision {
            status: "matched",
            tier: Some("map"),
            score: Some(1000),
            finding: Some(idx),
            candidates: vec![CandidateOut {
                registry_id: f.id.clone(),
                dedup_key: f.dedup_key.clone(),
                score: 1000,
                tier: "map".into(),
            }],
            evidence_kind_override: rule.evidence_kind.clone(),
        });
    }
    None
}

fn rule_matches(rule: &MapRule, rec: &Record) -> bool {
    rule.source
        .as_deref()
        .is_none_or(|p| glob_match(p, &rec.source))
        && rule
            .entry
            .as_deref()
            .is_none_or(|p| glob_match(p, &rec.entry))
        && rule
            .report_format
            .as_deref()
            .is_none_or(|p| glob_match(p, &rec.source_format))
}

fn validate_map(map: &LedgerMap, registry: &Registry, path: &Path) -> Result<(), String> {
    for (i, rule) in map.rules.iter().enumerate() {
        if let Some(id) = &rule.registry_id
            && !registry.findings.iter().any(|f| &f.id == id)
        {
            return Err(format!(
                "map {} rule {}: unknown registry_id {id}",
                path.display(),
                i
            ));
        }
        if let Some(dk) = &rule.dedup_key
            && !registry.findings.iter().any(|f| &f.dedup_key == dk)
        {
            return Err(format!(
                "map {} rule {}: unknown dedup_key {dk}",
                path.display(),
                i
            ));
        }
        if let (Some(id), Some(dk)) = (&rule.registry_id, &rule.dedup_key)
            && !registry
                .findings
                .iter()
                .any(|f| &f.id == id && &f.dedup_key == dk)
        {
            return Err(format!(
                "map {} rule {}: registry_id {id} and dedup_key {dk} refer to different findings",
                path.display(),
                i
            ));
        }
    }
    Ok(())
}

fn apply_match(rec: &mut Record, decision: &Decision, findings: &[RegistryFinding]) {
    rec.match_status = decision.status.to_string();
    rec.match_tier = decision.tier.map(str::to_string);
    rec.match_score = decision.score;
    rec.match_candidates = decision.candidates.clone();
    if let Some(i) = decision.finding {
        rec.registry_id = Some(findings[i].id.clone());
        rec.registry_dedup_key = Some(findings[i].dedup_key.clone());
    }
}

// ---------------------------------------------------------------------------
// Plan and patch construction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Plan {
    action: String,
    reason: String,
    group: String,
}

fn plan_action(rec: &Record, decision: &Decision, findings: &[RegistryFinding]) -> Plan {
    let noop = |reason: &str| Plan {
        action: "noop".into(),
        reason: reason.into(),
        group: String::new(),
    };
    let unmatched = |reason: &str| Plan {
        action: "unmatched".into(),
        reason: reason.into(),
        group: String::new(),
    };

    if rec.control {
        return if rec.match_status == "matched" {
            noop("control-entry")
        } else if rec.match_status == "ambiguous" {
            unmatched("ambiguous-match")
        } else {
            unmatched("control-entry")
        };
    }
    if !rec.actionable {
        return if rec.match_status == "matched" {
            noop("verdict-not-actionable")
        } else if rec.match_status == "ambiguous" {
            unmatched("ambiguous-match")
        } else {
            unmatched("no-match")
        };
    }
    if rec.match_status == "ambiguous" {
        return unmatched("ambiguous-match");
    }
    if rec.match_status == "unmatched" {
        return Plan {
            action: "new_entry".into(),
            reason: "no-existing-entry".into(),
            group: format!(
                "new:{}:{}",
                rec.evidence_kind,
                semantic_entry_key(&rec.entry)
            ),
        };
    }
    let i = decision.finding.expect("matched decision has a finding");
    if already_recorded(&findings[i], rec) {
        return noop("evidence-already-present");
    }
    Plan {
        action: "append_evidence".into(),
        reason: "new-source-evidence".into(),
        group: format!(
            "append:{}:{}:{}",
            findings[i].id,
            rec.evidence_kind,
            semantic_entry_key(&rec.entry)
        ),
    }
}

fn already_recorded(f: &RegistryFinding, rec: &Record) -> bool {
    f.evidence.iter().any(|e| {
        evidence_mentions_entry(e, &rec.entry) || ref_matches_source(&e.reference, &rec.source)
    })
}

fn evidence_mentions_entry(e: &RegistryEvidence, entry: &str) -> bool {
    let entry = entry.to_ascii_lowercase();
    if entry.len() < 8 {
        return false;
    }
    let text = format!("{} {}", e.reference, e.detail).to_ascii_lowercase();
    text.contains(&entry)
}

fn ref_matches_source(reference: &str, source: &str) -> bool {
    let source = source
        .split('#')
        .next()
        .unwrap_or(source)
        .replace('\\', "/");
    let reference = reference.replace('\\', "/");
    let mut probes = vec![source.clone()];
    if let Some(parent) = Path::new(&source).parent() {
        probes.push(parent.display().to_string());
        if let Some(name) = parent.file_name() {
            probes.push(name.to_string_lossy().into_owned());
        }
    }
    probes
        .into_iter()
        .any(|p| p.len() >= 8 && reference.contains(&p))
}

fn build_evidence(rec: &Record) -> ProposedEvidence {
    let sources = if rec.sources.is_empty() {
        vec![rec.source.clone()]
    } else {
        rec.sources.clone()
    };
    let detail = if rec.detail.trim().is_empty() {
        format!("{} {}", rec.entry, rec.verdict)
    } else {
        format!("{} {}; {}", rec.entry, rec.verdict, rec.detail.trim())
    };
    ProposedEvidence {
        kind: rec.evidence_kind.clone(),
        reference: format!("spec-check ledger inputs: {}", sources.join("; ")),
        detail,
    }
}

fn build_entry(rec: &Record) -> ProposedEntry {
    let subject = rec
        .function
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| rec.entry.clone());
    let interfaces = if let Some(f) = rec.function.as_ref().filter(|s| !s.is_empty()) {
        vec![f.clone()]
    } else {
        vec![rec.entry.clone()]
    };
    let (kind, status) = if rec.verdict == "ContractGapCandidate" {
        ("contract_gap", "obligation_fail")
    } else if rec.evidence_kind == "construction_probe" {
        (
            "vacuous_precondition",
            "construction_probe_confirmed_vacuous",
        )
    } else {
        ("vacuous_precondition", "candidate_unverified")
    };
    let relation = if rec.detail.trim().is_empty() {
        format!("machine evidence: {} {}", rec.entry, rec.verdict)
    } else {
        format!(
            "machine evidence: {} {}; {}",
            rec.entry,
            rec.verdict,
            rec.detail.trim()
        )
    };
    ProposedEntry {
        id: None,
        kind: kind.into(),
        subject: subject.clone(),
        relation,
        dedup_key: format!("ledger:{}", slug(&subject)),
        interfaces,
        discovered_by: vec!["ledger_automation".into()],
        machine_status: status.into(),
        evidence: vec![build_evidence(rec)],
        human_review: Value::Null,
    }
}

fn refresh_proposal(rec: &mut Record) {
    let evidence = build_evidence(rec);
    rec.proposed_evidence = Some(evidence.clone());
    if rec.action == "new_entry"
        && let Some(entry) = rec.proposed_entry.as_mut()
    {
        entry.evidence = vec![evidence];
    }
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

fn summarize(records: &[Record], registry_findings: usize) -> LedgerSummary {
    let mut s = LedgerSummary {
        records: records.len(),
        registry_findings,
        ..LedgerSummary::default()
    };
    for r in records {
        if r.actionable {
            s.actionable += 1;
        }
        match r.match_status.as_str() {
            "matched" => s.matched += 1,
            "ambiguous" => s.ambiguous += 1,
            _ => s.unmatched += 1,
        }
        match r.action.as_str() {
            "append_evidence" => s.append_evidence += 1,
            "new_entry" => s.new_entry += 1,
            "noop" => s.noop += 1,
            "unmatched" => s.action_unmatched += 1,
            _ => {}
        }
    }
    let mut appended: BTreeSet<&str> = BTreeSet::new();
    for r in records {
        if r.action == "append_evidence"
            && let Some(id) = &r.registry_id
        {
            appended.insert(id);
        }
    }
    s.appended_findings = appended.len();
    s
}

fn summarize_inputs(parsed: &[ParsedReport]) -> Vec<InputSummary> {
    let mut out: Vec<InputSummary> = parsed
        .iter()
        .map(|r| InputSummary {
            source: r.label.clone(),
            format: r.format.clone(),
            entries: r.verdicts.len(),
            actionable: r
                .verdicts
                .iter()
                .filter(|v| is_actionable(&v.verdict))
                .count(),
        })
        .collect();
    out.sort_by(|a, b| a.source.cmp(&b.source).then(a.format.cmp(&b.format)));
    out
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

fn dashboard(
    records: &[Record],
    registry: &Registry,
    parsed: &[ParsedReport],
    summary: &LedgerSummary,
    meta: &RegistryMeta,
) -> String {
    let mut l = String::new();
    l.push_str("# Ledger automation dashboard\n\n");
    l.push_str("Generated by `cargo-spec-check ledger` (machine output; no hand edits).\n\n");
    l.push_str("## Registry\n\n");
    l.push_str(&format!("- path: `{}`\n", meta.path));
    l.push_str(&format!("- findings: {}\n", meta.findings));
    l.push_str(&format!("- bytes: {}\n", meta.bytes));
    l.push_str(&format!(
        "- unchanged after run: {}\n\n",
        meta.unchanged_after_run
    ));

    l.push_str("## Inputs\n\n| source | format | entries | actionable |\n|---|---|---:|---:|\n");
    for i in summarize_inputs(parsed) {
        l.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            md_cell(&i.source),
            i.format,
            i.entries,
            i.actionable
        ));
    }

    l.push_str("\n## Patch summary\n\n| action | count |\n|---|---:|\n");
    l.push_str(&format!(
        "| append_evidence | {} |\n",
        summary.append_evidence
    ));
    l.push_str(&format!("| new_entry | {} |\n", summary.new_entry));
    l.push_str(&format!("| noop | {} |\n", summary.noop));
    l.push_str(&format!("| unmatched | {} |\n", summary.action_unmatched));
    l.push_str(&format!("| records | {} |\n", summary.records));
    l.push_str(&format!(
        "| actionable records | {} |\n| matched / ambiguous / unmatched | {} / {} / {} |\n\n",
        summary.actionable, summary.matched, summary.ambiguous, summary.unmatched
    ));

    l.push_str("## Evidence by verdict / kind\n\n| verdict | kind | records | actionable |\n|---|---|---:|---:|\n");
    let mut by_verdict: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    for r in records {
        let e = by_verdict
            .entry((r.verdict.clone(), r.evidence_kind.clone()))
            .or_insert((0, 0));
        e.0 += 1;
        if r.actionable {
            e.1 += 1;
        }
    }
    for ((verdict, kind), (count, actionable)) in by_verdict {
        l.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            verdict, kind, count, actionable
        ));
    }

    l.push_str("\n## Registry coverage\n\n");
    l.push_str(
        "| id | kind | machine_status | interfaces | evidence | matched | append | noop |\n",
    );
    l.push_str("|---|---|---|---:|---:|---:|---:|---:|\n");
    for f in &registry.findings {
        let mut matched = 0;
        let mut append = 0;
        let mut noop = 0;
        for r in records {
            if r.registry_id.as_deref() == Some(f.id.as_str()) {
                matched += 1;
                match r.action.as_str() {
                    "append_evidence" => append += 1,
                    "noop" => noop += 1,
                    _ => {}
                }
            }
        }
        l.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            f.id,
            f.kind,
            f.machine_status,
            f.interfaces.len(),
            f.evidence.len(),
            matched,
            append,
            noop
        ));
    }

    l.push_str("\n## Function coverage\n\n| function / entry | records | matched | append | noop | unmatched |\n|---|---:|---:|---:|---:|---:|\n");
    let mut by_fn: BTreeMap<String, [usize; 5]> = BTreeMap::new();
    for r in records {
        let key = r
            .function
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| semantic_entry_key(&r.entry));
        let e = by_fn.entry(key).or_default();
        e[0] += 1;
        if r.match_status == "matched" {
            e[1] += 1;
        }
        match r.action.as_str() {
            "append_evidence" => e[2] += 1,
            "noop" => e[3] += 1,
            "unmatched" => e[4] += 1,
            _ => {}
        }
    }
    for (key, c) in by_fn {
        l.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            md_cell(&key),
            c[0],
            c[1],
            c[2],
            c[3],
            c[4]
        ));
    }

    l.push_str("\n## Unmatched / ambiguous evidence\n\n| source | entry | verdict | reason | candidates |\n|---|---|---|---|---|\n");
    let mut any = false;
    for r in records {
        if r.action != "unmatched" {
            continue;
        }
        any = true;
        let candidates = if r.match_candidates.is_empty() {
            "—".to_string()
        } else {
            r.match_candidates
                .iter()
                .map(|c| format!("{}({})", c.registry_id, c.score))
                .collect::<Vec<_>>()
                .join(", ")
        };
        l.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            md_cell(&r.source),
            md_cell(&r.entry),
            r.verdict,
            md_cell(&r.reason),
            md_cell(&candidates)
        ));
    }
    if !any {
        l.push_str("| — | — | — | none | — |\n");
    }

    l.push_str("\n## Safety\n\n");
    l.push_str("- The registry is read-only; `registry-patch.json` is a proposal.\n");
    l.push_str("- `status_suggestion` is always `null`: status transitions are human-only.\n");
    l.push_str("- Unmatched, ambiguous, and control evidence is never promoted to a finding.\n");
    l
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

fn numeric_id(id: &str) -> Option<String> {
    let digits: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        let n = digits.trim_start_matches('0');
        Some(if n.is_empty() {
            "0".into()
        } else {
            n.to_string()
        })
    }
}

fn leading_number(entry: &str) -> Option<String> {
    let (prefix, _) = entry.split_once('_')?;
    let start = prefix.find(|c: char| c.is_ascii_digit())?;
    if start == 0 || !prefix[start..].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n = prefix[start..].trim_start_matches('0');
    Some(if n.is_empty() {
        "0".into()
    } else {
        n.to_string()
    })
}

fn is_numbered_prefix(prefix: &str) -> bool {
    let start = prefix.find(|c: char| c.is_ascii_digit());
    match start {
        Some(s) if s > 0 => prefix[s..].chars().all(|c| c.is_ascii_digit()),
        _ => false,
    }
}

fn identifier_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn ident_tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    let flush = |cur: &mut String, out: &mut BTreeSet<String>| {
        if !cur.is_empty() {
            out.insert(std::mem::take(cur));
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && prev_lower {
                flush(&mut cur, &mut out);
            }
            cur.push(ch.to_ascii_lowercase());
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else {
            flush(&mut cur, &mut out);
            prev_lower = false;
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn normalize_type_segment(s: &str) -> String {
    let s = s.trim().trim_start_matches('&').trim_start_matches("mut ");
    let s = s.trim_start_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn split_type_method(s: &str) -> Option<(String, String)> {
    let (recv, method) = s.rsplit_once("::")?;
    let recv = normalize_type_segment(recv);
    let method = normalize_type_segment(method);
    if recv.is_empty() || method.is_empty() {
        None
    } else {
        Some((recv, method))
    }
}

fn interface_tokens(s: &str) -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let (recv, method) = split_type_method(s)?;
    Some((ident_tokens(&recv), ident_tokens(&method)))
}

fn subject_interfaces(subject: &str) -> Vec<(BTreeSet<String>, BTreeSet<String>)> {
    let mut out = BTreeSet::new();
    for word in subject.split_whitespace() {
        if !word.contains("::") {
            continue;
        }
        let cleaned: String = word
            .chars()
            .filter(|c| !matches!(c, '(' | ')' | ',' | ';' | '{' | '}' | '`' | '"' | '\''))
            .collect();
        let segments: Vec<&str> = cleaned.split("::").filter(|s| !s.is_empty()).collect();
        for pair in segments.windows(2) {
            let recv = normalize_type_segment(pair[0]);
            let method = normalize_type_segment(pair[1]);
            if recv.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && method
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
            {
                out.insert((ident_tokens(&recv), ident_tokens(&method)));
            }
        }
    }
    out.into_iter().collect()
}

fn strip_stage_suffix(mut s: String) -> String {
    const SUFFIXES: &[&str] = &[
        "_holder_face",
        "_quantifier_shape",
        "_construction",
        "_live_control",
        "_vacuous_control",
        "_cross_control",
        "_cross_negative",
        "_negative_control",
        "_shape",
        "_control",
        "_supported",
    ];
    loop {
        let mut changed = false;
        for suffix in SUFFIXES {
            if s.ends_with(suffix) && s.len() > suffix.len() {
                s.truncate(s.len() - suffix.len());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    s
}

fn semantic_entry_tokens(entry: &str) -> BTreeSet<String> {
    let mut s = entry.to_ascii_lowercase();
    if let Some((prefix, rest)) = s.split_once('_')
        && is_numbered_prefix(prefix)
        && !rest.is_empty()
    {
        s = rest.to_string();
    }
    ident_tokens(&strip_stage_suffix(s))
}

fn semantic_entry_key(entry: &str) -> String {
    let mut s = entry.to_ascii_lowercase();
    if let Some((prefix, rest)) = s.split_once('_')
        && is_numbered_prefix(prefix)
        && !rest.is_empty()
    {
        s = rest.to_string();
    }
    slug(&strip_stage_suffix(s))
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "entry".into() } else { out }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn action_rank(action: &str) -> u8 {
    match action {
        "append_evidence" => 0,
        "new_entry" => 1,
        "noop" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("spec-check-ledger-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_value(path: &Path, value: &Value) {
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn read_value(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn run_case(
        name: &str,
        registry: Value,
        reports: &[(&str, Value)],
        candidates: &[(&str, Value)],
        map: Option<Value>,
    ) -> (PathBuf, LedgerSummary, Value) {
        let root = temp(name);
        let registry_path = root.join("registry.json");
        write_value(&registry_path, &registry);
        let mut report_paths = Vec::new();
        for (file, value) in reports {
            let p = root.join(file);
            write_value(&p, value);
            report_paths.push(p);
        }
        let mut candidate_paths = Vec::new();
        for (file, value) in candidates {
            let p = root.join(file);
            write_value(&p, value);
            candidate_paths.push(p);
        }
        let map_path = map.map(|value| {
            let p = root.join("map.json");
            write_value(&p, &value);
            p
        });
        let out = root.join("out");
        let summary = run(
            &registry_path,
            &report_paths,
            &candidate_paths,
            map_path.as_deref(),
            &out,
        )
        .unwrap();
        let report = read_value(&out.join("ledger-report.json"));
        (root, summary, report)
    }

    fn gap(entry: &str) -> Value {
        json!({
            "verdicts": [{
                "name": entry,
                "verdict": "ContractGapCandidate",
                "detail": "obligation failed; baseline clean"
            }]
        })
    }

    fn synth_candidate(candidate_id: &str, entry: &str, function: &str) -> Value {
        json!({
            "candidates_total": 1,
            "entries": [{
                "candidate_id": candidate_id,
                "entry_name": entry,
                "function": function,
                "dedup_key": "ktest:test",
                "receiver_type": "Mutex"
            }]
        })
    }

    #[test]
    fn verdict_kind_table() {
        assert_eq!(evidence_kind("ContractGapCandidate", ""), "obligation_fail");
        assert_eq!(
            evidence_kind("ObligationSupported", ""),
            "obligation_supported"
        );
        assert_eq!(
            evidence_kind("ConfirmedVacuous", "construction: rejected"),
            "construction_probe"
        );
        assert_eq!(evidence_kind("ConfirmedVacuous", "probe"), "probe_run");
        assert_eq!(evidence_kind("ConfirmedLive", ""), "probe_run");
        assert_eq!(evidence_kind("Inconclusive", ""), "probe_inconclusive");
        assert!(is_actionable("ContractGapCandidate"));
        assert!(is_actionable("ConfirmedVacuous"));
        assert!(!is_actionable("ConfirmedLive"));
        assert!(is_control("arith_oob_vacuous_control"));
        assert!(is_control("spinlock_lock_constant_cross_negative"));
    }

    #[test]
    fn append_matches_interface_and_merges_sources() {
        let registry = json!({
            "schema": 1,
            "findings": [{
                "id": "F-013",
                "kind": "contract_gap",
                "subject": "ostd::sync::mutex::Mutex::try_lock",
                "relation": "is_none demand",
                "dedup_key": "mutex-try-lock-is-none-demand",
                "machine_status": "obligation_fail",
                "evidence": [{"kind": "obligation_fail", "ref": "old/", "detail": "original"}]
            }]
        });
        let (_root, summary, report) = run_case(
            "append",
            registry,
            &[
                ("run1.json", gap("c131_mutex_try_lock")),
                ("run2.json", gap("c131_mutex_try_lock")),
            ],
            &[(
                "candidates.json",
                synth_candidate("K131", "c131_mutex_try_lock", "Mutex::try_lock"),
            )],
            None,
        );
        assert_eq!(summary.append_evidence, 1);
        assert_eq!(summary.new_entry, 0);
        assert_eq!(summary.noop, 1);
        assert_eq!(summary.appended_findings, 1);
        let proposals = report["records"].as_array().unwrap();
        let append = proposals
            .iter()
            .find(|r| r["action"] == "append_evidence")
            .unwrap();
        assert_eq!(append["registry_id"], "F-013");
        assert_eq!(append["match_tier"], "function-interface");
        assert_eq!(append["sources"].as_array().unwrap().len(), 2);
        let merged = proposals
            .iter()
            .find(|r| r["reason"] == "merged-into-proposal")
            .unwrap();
        assert_eq!(merged["action"], "noop");
    }

    #[test]
    fn noop_when_entry_already_cited() {
        let registry = json!({
            "findings": [{
                "id": "F-013",
                "kind": "contract_gap",
                "subject": "ostd::sync::mutex::Mutex::try_lock",
                "dedup_key": "mutex-try-lock-is-none-demand",
                "machine_status": "obligation_fail",
                "evidence": [{
                    "kind": "obligation_fail",
                    "ref": "run1",
                    "detail": "c131_mutex_try_lock ContractGapCandidate"
                }]
            }]
        });
        let (_root, summary, report) = run_case(
            "noop",
            registry,
            &[("run1.json", gap("c131_mutex_try_lock"))],
            &[(
                "candidates.json",
                synth_candidate("K131", "c131_mutex_try_lock", "Mutex::try_lock"),
            )],
            None,
        );
        assert_eq!(summary.append_evidence, 0);
        assert_eq!(summary.noop, 1);
        let rec = &report["records"][0];
        assert_eq!(rec["reason"], "evidence-already-present");
        assert_eq!(rec["registry_id"], "F-013");
    }

    #[test]
    fn unmatched_gap_becomes_new_entry_proposal() {
        let registry = json!({"findings": []});
        let (_root, summary, report) = run_case(
            "new-entry",
            registry,
            &[("run.json", gap("spinlock_foo_gap"))],
            &[],
            None,
        );
        assert_eq!(summary.new_entry, 1);
        assert_eq!(summary.append_evidence, 0);
        let rec = &report["records"][0];
        assert_eq!(rec["action"], "new_entry");
        assert_eq!(rec["proposed_entry"]["id"], Value::Null);
        assert_eq!(rec["proposed_entry"]["kind"], "contract_gap");
        assert_eq!(rec["proposed_entry"]["machine_status"], "obligation_fail");
        assert!(
            rec["proposed_entry"]["dedup_key"]
                .as_str()
                .unwrap()
                .starts_with("ledger:")
        );
        assert_eq!(rec["status_suggestion"], Value::Null);
    }

    #[test]
    fn non_actionable_verdict_is_unmatched_not_a_finding() {
        let registry = json!({"findings": []});
        let report_value = json!({
            "verdicts": [{
                "name": "c007_memoryregionarray_push",
                "verdict": "ObligationSupported",
                "detail": "obligation verified"
            }]
        });
        let (_root, summary, report) = run_case(
            "supported",
            registry,
            &[("run.json", report_value)],
            &[],
            None,
        );
        assert_eq!(summary.new_entry, 0);
        assert_eq!(summary.action_unmatched, 1);
        assert_eq!(report["records"][0]["reason"], "no-match");
        assert_eq!(report["records"][0]["action"], "unmatched");
    }

    #[test]
    fn ambiguous_match_needs_map() {
        let registry = json!({
            "findings": [
                {
                    "id": "F-001",
                    "kind": "contract_gap",
                    "subject": "ostd::sync::mutex::Mutex::try_lock",
                    "dedup_key": "one",
                    "machine_status": "obligation_fail",
                    "evidence": [{"kind": "obligation_fail", "ref": "a", "detail": "a"}]
                },
                {
                    "id": "F-002",
                    "kind": "contract_gap",
                    "subject": "ostd::sync::mutex::Mutex::try_lock",
                    "dedup_key": "two",
                    "machine_status": "obligation_fail",
                    "evidence": [{"kind": "obligation_fail", "ref": "b", "detail": "b"}]
                }
            ]
        });
        let (_root, summary, report) = run_case(
            "ambiguous",
            registry.clone(),
            &[("run.json", gap("c131_mutex_try_lock"))],
            &[(
                "candidates.json",
                synth_candidate("K131", "c131_mutex_try_lock", "Mutex::try_lock"),
            )],
            None,
        );
        assert_eq!(summary.ambiguous, 1);
        assert_eq!(summary.action_unmatched, 1);
        assert_eq!(report["records"][0]["reason"], "ambiguous-match");
        assert_eq!(
            report["records"][0]["match_candidates"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let map = json!({
            "schema": MAP_SCHEMA,
            "rules": [{
                "entry": "c131_*",
                "registry_id": "F-002",
                "evidence_kind": "mapped_kind"
            }]
        });
        let (_root, summary, report) = run_case(
            "ambiguous-map",
            registry,
            &[("run.json", gap("c131_mutex_try_lock"))],
            &[(
                "candidates.json",
                synth_candidate("K131", "c131_mutex_try_lock", "Mutex::try_lock"),
            )],
            Some(map),
        );
        assert_eq!(summary.ambiguous, 0);
        assert_eq!(summary.append_evidence, 1);
        assert_eq!(report["records"][0]["registry_id"], "F-002");
        assert_eq!(report["records"][0]["match_tier"], "map");
        assert_eq!(report["records"][0]["evidence_kind"], "mapped_kind");
    }

    #[test]
    fn control_is_never_proposed() {
        let registry = json!({"findings": []});
        let report_value = json!({
            "verdicts": [{
                "name": "arith_oob_vacuous_control",
                "verdict": "ConfirmedVacuous",
                "detail": "vacuous_probe_failed=false live_probe_failed=true"
            }]
        });
        let (_root, summary, report) = run_case(
            "control",
            registry,
            &[("run.json", report_value)],
            &[],
            None,
        );
        assert_eq!(summary.new_entry, 0);
        assert_eq!(summary.action_unmatched, 1);
        assert_eq!(report["records"][0]["reason"], "control-entry");
    }

    #[test]
    fn nested_check_report_and_construction_kind() {
        let registry = json!({
            "findings": [{
                "id": "V-001",
                "kind": "vacuous_precondition",
                "subject": "ostd::sync::mutex::MutexGuard::new (external stub)",
                "dedup_key": "vacuity:mutex-guard-new-stub",
                "machine_status": "construction_probe_confirmed_vacuous",
                "evidence": [{"kind": "static_rule", "ref": "audit/", "detail": "scan"}]
            }]
        });
        let report_value = json!({
            "analyses": [{
                "name": "vacuity",
                "report": {
                    "verdicts": [{
                        "name": "mutex_guard_new_construction",
                        "verdict": "ConfirmedVacuous",
                        "detail": "construction: fabrication call rejected"
                    }]
                }
            }]
        });
        let (_root, summary, report) = run_case(
            "nested",
            registry,
            &[("check.json", report_value)],
            &[],
            None,
        );
        assert_eq!(summary.append_evidence, 1);
        let rec = &report["records"][0];
        assert_eq!(rec["registry_id"], "V-001");
        assert_eq!(rec["evidence_kind"], "construction_probe");
        assert!(rec["source"].as_str().unwrap().ends_with("#vacuity"));
    }

    #[test]
    fn output_is_deterministic() {
        let registry = json!({
            "findings": [{
                "id": "F-001",
                "kind": "contract_gap",
                "subject": "Mutex::try_lock",
                "dedup_key": "mutex",
                "machine_status": "obligation_fail",
                "evidence": [{"kind": "obligation_fail", "ref": "old", "detail": "old"}]
            }]
        });
        let report_value = gap("c131_mutex_try_lock");
        let root = temp("det");
        let reg = root.join("registry.json");
        write_value(&reg, &registry);
        let rp = root.join("run.json");
        write_value(&rp, &report_value);
        let cp = root.join("candidates.json");
        write_value(
            &cp,
            &synth_candidate("K131", "c131_mutex_try_lock", "Mutex::try_lock"),
        );
        let out1 = root.join("out1");
        let out2 = root.join("out2");
        run(
            &reg,
            std::slice::from_ref(&rp),
            std::slice::from_ref(&cp),
            None,
            &out1,
        )
        .unwrap();
        run(&reg, &[rp], &[cp], None, &out2).unwrap();
        for name in ["ledger-report.json", "registry-patch.json", "DASHBOARD.md"] {
            assert_eq!(
                fs::read(out1.join(name)).unwrap(),
                fs::read(out2.join(name)).unwrap(),
                "{name} differs"
            );
        }
    }

    #[test]
    fn registry_is_not_modified() {
        let registry = json!({"findings": []});
        let root = temp("readonly");
        let reg = root.join("registry.json");
        write_value(&reg, &registry);
        let before = fs::read(&reg).unwrap();
        let rp = root.join("run.json");
        write_value(&rp, &gap("some_entry"));
        let out = root.join("out");
        run(&reg, &[rp], &[], None, &out).unwrap();
        assert_eq!(fs::read(&reg).unwrap(), before);
        let patch = read_value(&out.join("registry-patch.json"));
        assert_eq!(patch["registry"]["unchanged_after_run"], true);
        assert_eq!(patch["requires_human"], true);
        for rec in patch["proposals"].as_array().unwrap() {
            assert_eq!(rec["status_suggestion"], Value::Null);
        }
    }

    #[test]
    fn unknown_map_target_fails_closed() {
        let root = temp("bad-map");
        let reg = root.join("registry.json");
        write_value(&reg, &json!({"findings": []}));
        let rp = root.join("run.json");
        write_value(&rp, &gap("some_entry"));
        let mp = root.join("map.json");
        write_value(
            &mp,
            &json!({"rules": [{"entry": "*", "registry_id": "F-999"}]}),
        );
        let err = run(&reg, &[rp], &[], Some(&mp), &root.join("out")).unwrap_err();
        assert!(err.contains("unknown registry_id F-999"), "{err}");
    }
}
