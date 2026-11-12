//! E6: first-class signals for pre-registered expectations.
//!
//! The runner has always had a conservative internal `expected` verdict
//! (`ConfirmedLive` for a Tier A entry with a bounded witness, otherwise
//! `CandidateInconclusive`). E6 turns the comparison against that internal
//! expectation, and against an optional project-side pre-registration, into a
//! structured output. The tool never edits the registry; every signal carries
//! a suggestion and a model-relation gate.
//!
//! Design and pre-registration: `docs/M7e-e6-signals.md` and
//! `examples/vostd-requires/E6-EXPECTED.md`.

use crate::{Result, json};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SCHEMA: &str = "spec-check.e6-signals/1";
pub const EXPECTATIONS_SCHEMA: &str = "spec-check.expectations/1";

const VERDICTS: [&str; 4] = [
    "ConfirmedLive",
    "ConfirmedVacuous",
    "CandidateInconclusive",
    "Anomaly",
];

#[derive(Debug, Clone)]
pub struct Expectation {
    pub expected: String,
    pub basis: String,
    pub scope: String,
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Expectations {
    pub entries: BTreeMap<String, Expectation>,
}

impl Expectations {
    pub fn from_path(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&text)?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        if value["schema"].as_str() != Some(EXPECTATIONS_SCHEMA) {
            return Err(format!("expectations schema must be {EXPECTATIONS_SCHEMA}").into());
        }
        let object = value["entries"]
            .as_object()
            .ok_or("expectations needs an entries object")?;
        let mut entries = BTreeMap::new();
        for (name, entry) in object {
            if name.is_empty() {
                return Err("expectations entry name must not be empty".into());
            }
            let expected = entry["expected"]
                .as_str()
                .ok_or_else(|| format!("expectations entry `{name}` needs an expected verdict"))?;
            if !VERDICTS.contains(&expected) {
                return Err(format!(
                    "expectations entry `{name}` has unsupported verdict `{expected}`"
                )
                .into());
            }
            let basis = entry["basis"].as_str().unwrap_or_default();
            if basis.trim().is_empty() {
                return Err(format!("expectations entry `{name}` needs a non-empty basis").into());
            }
            let scope = match entry["scope"].as_str().unwrap_or("crate") {
                "crate" => "crate",
                "global" => "global",
                other => {
                    return Err(format!(
                        "expectations entry `{name}` has unsupported scope `{other}`"
                    )
                    .into());
                }
            };
            let registry = entry["registry"].as_str().map(str::to_owned);
            entries.insert(
                name.clone(),
                Expectation {
                    expected: expected.to_owned(),
                    basis: basis.to_owned(),
                    scope: scope.to_owned(),
                    registry,
                },
            );
        }
        Ok(Self { entries })
    }

    fn get(&self, name: &str) -> Option<&Expectation> {
        self.entries.get(name)
    }
}

fn relation_counts(entry: &Value) -> (u64, u64, u64, u64) {
    let relations = &entry["modeling"]["projection_relations"];
    let get = |key: &str| {
        relations[key]
            .as_u64()
            .or_else(|| relations[key.replace('-', "_")].as_u64())
            .unwrap_or(0)
    };
    (
        get("faithful"),
        get("over-approx"),
        get("under-approx"),
        get("structural_relaxed_clauses"),
    )
}

fn relation_of(entry: &Value) -> &'static str {
    if let Some(relation) = entry["modeling"]["vacuum_relation"].as_str() {
        return match relation {
            "exact" => "exact",
            "faithful" => "faithful",
            "over-approx" => "over-approx",
            "under-approx" => "under-approx",
            "undeclared" => "undeclared",
            _ => "undeclared",
        };
    }
    let (faithful, over, under, structural) = relation_counts(entry);
    if under > 0 {
        return "under-approx";
    }
    if structural > 0 {
        return "over-approx";
    }
    if faithful > 0 && over == 0 {
        return "faithful";
    }
    if faithful + over > 0 {
        return "over-approx";
    }
    let tier = entry["tier"].as_str().unwrap_or("");
    let projected = entry["projected_clauses"].as_u64().unwrap_or(0);
    let unsupported = entry["unsupported_clauses"].as_u64().unwrap_or(0);
    if tier == "A" && projected == 0 && unsupported == 0 {
        "exact"
    } else if tier == "B" {
        "over-approx"
    } else {
        "undeclared"
    }
}

fn direction(expected: &str, actual: &str) -> &'static str {
    if actual == "Anomaly" {
        "anomaly"
    } else if expected != "ConfirmedVacuous" && actual == "ConfirmedVacuous" {
        "vacuous-signal"
    } else if expected == "ConfirmedVacuous" && actual != "ConfirmedVacuous" {
        "expected-falsified"
    } else {
        "verdict-divergence"
    }
}

fn vacuum_evidence(relation: &str) -> bool {
    matches!(relation, "exact" | "faithful" | "over-approx")
}

fn severity(direction: &str, relation: &str) -> &'static str {
    match direction {
        "vacuous-signal" if vacuum_evidence(relation) => "candidate",
        "anomaly" => "candidate",
        _ => "info",
    }
}

fn suggested_action(direction: &str, severity: &str, registry: Option<&str>) -> String {
    match direction {
        "vacuous-signal" if severity == "candidate" => match registry {
            Some(registry) => format!("confirm-registry:{registry}"),
            None => "replicate-and-register".to_owned(),
        },
        "expected-falsified" => "update-or-retire-expectation".to_owned(),
        "anomaly" => "investigate-anomaly".to_owned(),
        _ => "review-expectation".to_owned(),
    }
}

/// Compare actual verdicts with the tool-internal expectations and, when
/// supplied, with the project-side pre-registration. `screening` is the
/// report produced by [`crate::requires::build`]; `verdicts` is either the
/// runner report or the `verdicts.json` object with an `entries` array.
pub fn evaluate(
    screening: &Value,
    verdicts: &Value,
    expectations: Option<&Expectations>,
) -> Result<Value> {
    let entries = screening["entries"]
        .as_array()
        .ok_or("screening report has no entries array")?;
    let entry_by_name = entries
        .iter()
        .filter_map(|entry| Some((entry["name"].as_str()?.to_owned(), entry)))
        .collect::<BTreeMap<_, _>>();
    let actuals = verdicts["entries"]
        .as_array()
        .ok_or("verdict report has no entries array")?;

    let mut signals = Vec::new();
    let mut by_source = BTreeMap::new();
    let mut by_direction = BTreeMap::new();
    let mut by_relation = BTreeMap::new();
    let mut missing = 0usize;
    let mut candidates = 0usize;
    let mut info = 0usize;

    for verdict in actuals {
        let Some(name) = verdict["name"].as_str() else {
            continue;
        };
        let Some(entry) = entry_by_name.get(name) else {
            continue;
        };
        let actual = verdict["verdict"].as_str().unwrap_or("Unknown");
        let tool_expected = entry["expected"]
            .as_str()
            .or_else(|| verdict["expected"].as_str())
            .unwrap_or("CandidateInconclusive");
        let pre = expectations.and_then(|expectations| expectations.get(name));
        if pre.is_none() {
            missing += 1;
        }
        let preregistered_match = pre.map(|pre| pre.expected == actual);
        let tool_match = tool_expected == actual;
        let (expected_source, expected) = if let Some(pre) = pre {
            if pre.expected != actual {
                ("pre-registered", pre.expected.as_str())
            } else if !tool_match {
                ("tool", tool_expected)
            } else {
                continue;
            }
        } else if !tool_match {
            ("tool", tool_expected)
        } else {
            continue;
        };

        let relation = relation_of(entry);
        let direction = direction(expected, actual);
        let severity = severity(direction, relation);
        let vacuum_evidence = direction == "vacuous-signal" && vacuum_evidence(relation);
        let registry = pre.and_then(|pre| pre.registry.as_deref());
        let action = suggested_action(direction, severity, registry);
        if severity == "candidate" {
            candidates += 1;
        } else {
            info += 1;
        }
        *by_source
            .entry(expected_source.to_owned())
            .or_insert(0usize) += 1;
        *by_direction.entry(direction.to_owned()).or_insert(0usize) += 1;
        *by_relation.entry(relation.to_owned()).or_insert(0usize) += 1;

        signals.push(json!({
            "entry": name,
            "function": entry["path"],
            "tier": entry["tier"],
            "expected_source": expected_source,
            "expected": expected,
            "actual": actual,
            "direction": direction,
            "severity": severity,
            "preregistered_expected": pre.map(|pre| pre.expected.clone()),
            "preregistered_match": preregistered_match,
            "tool_expected": tool_expected,
            "tool_match": tool_match,
            "basis": pre.map(|pre| pre.basis.clone()).unwrap_or_else(|| "tool internal expected".to_owned()),
            "model_relation": relation,
            "scope": pre.map(|pre| pre.scope.as_str()).unwrap_or("crate"),
            "vacuum_evidence": vacuum_evidence,
            "suggested_registry_action": action,
            "registry": registry,
            "evidence": {
                "span": entry["origin"]["source"]["span"],
                "witness": entry["witness"],
                "requires": entry["requires"],
                "detail": verdict["detail"],
                "projection_relations": entry["modeling"]["projection_relations"],
            },
        }));
    }
    signals.sort_by(|left, right| {
        left["entry"]
            .as_str()
            .unwrap_or("")
            .cmp(right["entry"].as_str().unwrap_or(""))
    });

    let matched = expectations
        .map(|expectations| {
            expectations
                .entries
                .keys()
                .filter(|name| entry_by_name.contains_key(name.as_str()))
                .count()
        })
        .unwrap_or(0);
    let provided_entries = expectations
        .map(|expectations| expectations.entries.len())
        .unwrap_or(0);
    let unmatched = provided_entries.saturating_sub(matched);

    Ok(json!({
        "schema": SCHEMA,
        "expectations": {
            "provided": expectations.is_some(),
            "entries": provided_entries,
            "matched": matched,
            "missing": missing,
            "unmatched": unmatched,
        },
        "counts": {
            "signals": signals.len(),
            "candidates": candidates,
            "info": info,
            "by_direction": by_direction,
            "by_expected_source": by_source,
            "by_model_relation": by_relation,
        },
        "signals": signals,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, tier: &str, expected: &str, relation: Value) -> Value {
        json!({
            "name": name,
            "path": format!("p::{name}"),
            "tier": tier,
            "expected": expected,
            "witness": null,
            "requires": "p0_k0()",
            "projected_clauses": 1,
            "unsupported_clauses": 0,
            "origin": {"source": {"span": "p.rs:1:1: 1:2 (#0)"}},
            "modeling": {
                "projection_relations": relation
            }
        })
    }

    fn verdict(name: &str, verdict: &str, detail: &str) -> Value {
        json!({"name": name, "verdict": verdict, "detail": detail})
    }

    fn screening(entries: Vec<Value>) -> Value {
        json!({"entries": entries})
    }

    fn expectations(entries: Value) -> Expectations {
        Expectations::from_value(&json!({
            "schema": EXPECTATIONS_SCHEMA,
            "entries": entries
        }))
        .unwrap()
    }

    #[test]
    fn tool_divergence_is_a_vacuous_signal_with_pre_registration_confirmation() {
        let entries = screening(vec![entry(
            "r1",
            "A",
            "CandidateInconclusive",
            json!({"faithful": 1, "over-approx": 0, "under-approx": 0, "structural_relaxed_clauses": 0}),
        )]);
        let verdicts =
            json!({"entries": [verdict("r1", "ConfirmedVacuous", "vacuous_probe_failed=false")]});
        let pre = expectations(json!({
            "r1": {
                "expected": "ConfirmedVacuous",
                "basis": "P5",
                "scope": "crate",
                "registry": "V-003"
            }
        }));
        let report = evaluate(&entries, &verdicts, Some(&pre)).unwrap();
        let signal = &report["signals"][0];
        assert_eq!(signal["direction"], "vacuous-signal");
        assert_eq!(signal["expected_source"], "tool");
        assert_eq!(signal["expected"], "CandidateInconclusive");
        assert_eq!(signal["preregistered_expected"], "ConfirmedVacuous");
        assert_eq!(signal["preregistered_match"], true);
        assert_eq!(signal["severity"], "candidate");
        assert_eq!(signal["model_relation"], "faithful");
        assert_eq!(
            signal["suggested_registry_action"],
            "confirm-registry:V-003"
        );
        assert_eq!(report["counts"]["signals"], 1);
        assert_eq!(report["counts"]["candidates"], 1);
    }

    #[test]
    fn expected_vacuous_to_live_is_an_info_falsification() {
        let entries = screening(vec![entry(
            "r1",
            "A",
            "CandidateInconclusive",
            json!({"faithful": 1, "over-approx": 0, "under-approx": 0, "structural_relaxed_clauses": 0}),
        )]);
        let verdicts = json!({"entries": [verdict("r1", "ConfirmedLive", "")]});
        let pre = expectations(json!({
            "r1": {"expected": "ConfirmedVacuous", "basis": "P5", "scope": "crate"}
        }));
        let report = evaluate(&entries, &verdicts, Some(&pre)).unwrap();
        let signal = &report["signals"][0];
        assert_eq!(signal["direction"], "expected-falsified");
        assert_eq!(signal["expected_source"], "pre-registered");
        assert_eq!(signal["severity"], "info");
        assert_eq!(
            signal["suggested_registry_action"],
            "update-or-retire-expectation"
        );
        assert_eq!(report["counts"]["candidates"], 0);
    }

    #[test]
    fn under_approx_vacuity_is_downgraded_to_info() {
        let entries = screening(vec![entry(
            "r1",
            "A",
            "CandidateInconclusive",
            json!({"faithful": 0, "over-approx": 0, "under-approx": 1, "structural_relaxed_clauses": 0}),
        )]);
        let verdicts = json!({"entries": [verdict("r1", "ConfirmedVacuous", "")]});
        let report = evaluate(&entries, &verdicts, None).unwrap();
        let signal = &report["signals"][0];
        assert_eq!(signal["model_relation"], "under-approx");
        assert_eq!(signal["vacuum_evidence"], false);
        assert_eq!(signal["severity"], "info");
        assert_eq!(signal["suggested_registry_action"], "review-expectation");
    }

    #[test]
    fn structural_relaxation_is_over_approx_evidence() {
        let entries = screening(vec![entry(
            "r1",
            "B",
            "CandidateInconclusive",
            json!({"faithful": 0, "over-approx": 1, "under-approx": 0, "structural_relaxed_clauses": 1}),
        )]);
        let verdicts = json!({"entries": [verdict("r1", "ConfirmedVacuous", "")]});
        let report = evaluate(&entries, &verdicts, None).unwrap();
        let signal = &report["signals"][0];
        assert_eq!(signal["model_relation"], "over-approx");
        assert_eq!(signal["vacuum_evidence"], true);
        assert_eq!(signal["severity"], "candidate");
        assert_eq!(
            signal["suggested_registry_action"],
            "replicate-and-register"
        );
    }

    #[test]
    fn missing_expectation_still_reports_tool_divergence() {
        let entries = screening(vec![entry(
            "r1",
            "A",
            "CandidateInconclusive",
            json!({"faithful": 1, "over-approx": 0, "under-approx": 0, "structural_relaxed_clauses": 0}),
        )]);
        let verdicts = json!({"entries": [verdict("r1", "ConfirmedVacuous", "")]});
        let report = evaluate(&entries, &verdicts, None).unwrap();
        assert_eq!(report["expectations"]["missing"], 1);
        assert_eq!(report["expectations"]["provided"], false);
        assert_eq!(report["signals"][0]["preregistered_expected"], Value::Null);
        assert_eq!(report["signals"][0]["expected_source"], "tool");
    }

    #[test]
    fn no_divergence_is_no_signal() {
        let entries = screening(vec![entry(
            "r1",
            "A",
            "ConfirmedLive",
            json!({"faithful": 0, "over-approx": 0, "under-approx": 0, "structural_relaxed_clauses": 0}),
        )]);
        let verdicts = json!({"entries": [verdict("r1", "ConfirmedLive", "")]});
        let pre = expectations(json!({
            "r1": {"expected": "ConfirmedLive", "basis": "shape"}
        }));
        let report = evaluate(&entries, &verdicts, Some(&pre)).unwrap();
        assert_eq!(report["counts"]["signals"], 0);
        assert_eq!(report["expectations"]["matched"], 1);
        assert_eq!(report["expectations"]["missing"], 0);
    }

    #[test]
    fn evaluation_is_deterministic() {
        let entries = screening(vec![
            entry(
                "r2",
                "A",
                "CandidateInconclusive",
                json!({"faithful": 1, "over-approx": 0, "under-approx": 0, "structural_relaxed_clauses": 0}),
            ),
            entry(
                "r1",
                "B",
                "CandidateInconclusive",
                json!({"faithful": 0, "over-approx": 1, "under-approx": 0, "structural_relaxed_clauses": 0}),
            ),
        ]);
        let verdicts = json!({"entries": [
            verdict("r2", "ConfirmedVacuous", ""),
            verdict("r1", "ConfirmedVacuous", "")
        ]});
        let first = evaluate(&entries, &verdicts, None).unwrap();
        let second = evaluate(&entries, &verdicts, None).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["signals"][0]["entry"], "r1");
        assert_eq!(first["signals"][1]["entry"], "r2");
    }

    #[test]
    fn expectations_loader_is_strict() {
        let bad_schema = Expectations::from_value(&json!({
            "schema": "other",
            "entries": {}
        }));
        assert!(bad_schema.is_err());
        let bad_verdict = Expectations::from_value(&json!({
            "schema": EXPECTATIONS_SCHEMA,
            "entries": {"r1": {"expected": "Vacuous", "basis": "x"}}
        }));
        assert!(bad_verdict.is_err());
        let missing_basis = Expectations::from_value(&json!({
            "schema": EXPECTATIONS_SCHEMA,
            "entries": {"r1": {"expected": "ConfirmedVacuous"}}
        }));
        assert!(missing_basis.is_err());
    }
}
