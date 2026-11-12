//! M7e: census-driven screening of `requires` clauses for unsatisfiable
//! (vacuous) preconditions.
//!
//! Input is a census report produced by [`crate::census`]. The screening
//! makes three decisions per function and is deliberately conservative:
//!
//! - **Tier A**: every `requires` clause is mechanically transcribable over
//!   the referenced scalar parameters (and lifted constants). One quantifier
//!   probe per function is generated; a bounded witness search decides
//!   whether the precondition is satisfiable in the probe model.
//! - **Tier B**: the function has no tracked/ghost/reference/generic blocker,
//!   but at least one clause is only expressible as a named `uninterp spec
//!   fn`. Probes are still generated, but they are expected to be
//!   `CandidateInconclusive` (Verus is incomplete over uninterpreted
//!   predicates); they are never counted as defects.
//! - **Tier C**: the requires references a tracked/ghost parameter, a
//!   reference parameter, or the function has type parameters. No probe is
//!   generated and the reason is recorded. Tier C is **not** evidence of
//!   absence of defects.
//!
//! Functions without any `requires` clauses are outside the target set: the
//! precondition is the empty conjunction and is trivially satisfiable
//! (`no_requires`, not a screening gap).
//!
//! Project-specific knowledge (which functions exist, their types, their
//! clause text) is entirely in the census input. The rules here are generic.

use crate::{Result, write_json};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const SCHEMA: &str = "spec-check.requires-screen/1";
pub const CONFIG_SCHEMA: &str = "spec-check.requires-config/1";

/// Project-side Tier C modeling configuration. The default is exactly the
/// M7e behavior: no instantiation, no reference/ghost abstraction.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub generic_instances: Vec<GenericRule>,
    pub reference_opaque: bool,
    pub ghost_opaque: bool,
}

#[derive(Debug, Clone)]
pub struct GenericRule {
    pub pattern: String,
    pub types: BTreeMap<String, String>,
}

impl Config {
    pub fn from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&text)?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        if value["schema"].as_str() != Some(CONFIG_SCHEMA) {
            return Err(format!("config schema must be {CONFIG_SCHEMA}").into());
        }
        let mut config = Config::default();
        for rule in value["generic_instances"].as_array().into_iter().flatten() {
            let pattern = rule["match"]
                .as_str()
                .ok_or("generic_instances entry needs a match pattern")?;
            let mut types = BTreeMap::new();
            for (name, typ) in rule["types"]
                .as_object()
                .ok_or("generic_instances entry needs a types object")?
            {
                types.insert(
                    name.clone(),
                    typ.as_str()
                        .ok_or("generic instance type must be a string")?
                        .to_owned(),
                );
            }
            config.generic_instances.push(GenericRule {
                pattern: pattern.to_owned(),
                types,
            });
        }
        config.reference_opaque = match value["reference"]["mode"].as_str().unwrap_or("off") {
            "off" => false,
            "opaque" => true,
            other => return Err(format!("unsupported reference mode: {other}").into()),
        };
        config.ghost_opaque = match value["ghost"]["mode"].as_str().unwrap_or("off") {
            "off" => false,
            "opaque" => true,
            other => return Err(format!("unsupported ghost mode: {other}").into()),
        };
        Ok(config)
    }

    fn generic_rule(&self, path: &str) -> Option<&GenericRule> {
        self.generic_instances
            .iter()
            .find(|rule| glob_match(&rule.pattern, path))
    }

    fn is_default(&self) -> bool {
        self.generic_instances.is_empty() && !self.reference_opaque && !self.ghost_opaque
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.split_once('*') {
        None => pattern == text,
        Some((prefix, suffix)) => {
            text.len() >= prefix.len() + suffix.len()
                && text.starts_with(prefix)
                && text.ends_with(suffix)
        }
    }
}

/// Selected tiers and deterministic batching options.
#[derive(Debug, Clone)]
pub struct Options {
    pub tiers: BTreeSet<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
    pub batch: Option<usize>,
    pub batch_size: usize,
    pub config: Config,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tiers: ["A", "B"].into_iter().map(str::to_owned).collect(),
            filter: None,
            limit: None,
            batch: None,
            batch_size: 50,
            config: Config::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    A,
    B,
    C,
    None,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::None => "none",
        }
    }
}

struct ParamInfo {
    name: String,
    ty: String,
    kind: String,
}

struct ParamModel {
    probe: String,
    int: bool,
}

struct Decision {
    path: String,
    tier: Tier,
    reasons: Vec<String>,
    entry: Option<Value>,
    expected: Option<&'static str>,
    expected_note: Option<String>,
    clauses: usize,
    mechanical: usize,
    unsupported: usize,
    modeling: Value,
}

/// Build the manifest and screening report from a census JSON document.
pub fn build(census: &Value, options: &Options) -> Result<Value> {
    let functions = census["functions"]
        .as_array()
        .ok_or("census has no functions array")?;
    let mut decisions = Vec::with_capacity(functions.len());
    let mut baselines = Vec::with_capacity(functions.len());
    let mut name_used = BTreeSet::new();
    let mut baseline_name_used = BTreeSet::new();

    for (index, function) in functions.iter().enumerate() {
        let baseline = classify(function, index, &mut baseline_name_used, &Config::default());
        baselines.push((baseline.tier, baseline.reasons.clone()));
        let decision = classify(function, index, &mut name_used, &options.config);
        decisions.push(decision);
    }

    // Stable global entry names are assigned in census order; batching and
    // filtering only select among them.
    let mut candidates = Vec::new();
    for decision in &decisions {
        if decision.entry.is_none() {
            continue;
        }
        if !options.tiers.contains(decision.tier.as_str()) {
            continue;
        }
        if let Some(filter) = &options.filter {
            if !decision.path.contains(filter) {
                continue;
            }
        }
        candidates.push(decision.path.clone());
    }

    let total_candidates = candidates.len();
    let selected = if let Some(batch) = options.batch {
        let start = batch.saturating_mul(options.batch_size);
        candidates
            .iter()
            .skip(start)
            .take(options.batch_size)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        candidates.clone()
    };
    let selected = if let Some(limit) = options.limit {
        selected.into_iter().take(limit).collect::<Vec<_>>()
    } else {
        selected
    };
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();

    let mut contracts = Vec::new();
    let mut preamble = String::from(
        "// Generated by spec-check screen-requires (M7e).\n\
         // Requires clauses are wrapped as named spec fns (quantifier-trigger\n\
         // discipline). Mechanical clauses are open spec bodies; clauses over\n\
         // ghost structure are uninterp spec fns (constraint structure kept by\n\
         // name, body abstracted). Constants referenced by requires are lifted\n\
         // to probe parameters (documented abstraction).\n\n",
    );
    let mut entries = Vec::new();
    for decision in &decisions {
        if !selected_set.contains(&decision.path) {
            continue;
        }
        let entry = decision
            .entry
            .as_ref()
            .expect("selected decision has entry");
        let name = entry["name"].as_str().unwrap_or_default();
        if let Some(fns) = entry["preamble"].as_str() {
            preamble.push_str(fns);
            preamble.push('\n');
        }
        let mut contract = json!({
            "name": name,
            "probe": "quantifier",
            "params": entry["params"],
            "requires": entry["requires"],
            "origin": entry["origin"],
        });
        if let Some(witness) = entry.get("witness").filter(|value| !value.is_null()) {
            contract["witness"] = witness.clone();
        }
        contracts.push(contract);
        entries.push(json!({
            "name": name,
            "path": decision.path,
            "tier": decision.tier.as_str(),
            "expected": decision.expected,
            "expected_note": decision.expected_note,
            "params": entry["params"],
            "requires": entry["requires"],
            "witness": entry.get("witness"),
            "clauses": decision.clauses,
            "mechanical_clauses": decision.mechanical,
            "unsupported_clauses": decision.unsupported,
            "origin": entry["origin"],
            "modeling": decision.modeling,
        }));
    }

    let manifest = json!({
        "preamble": preamble,
        "contracts": contracts,
    });

    let mut tier_counts = BTreeMap::new();
    let mut baseline_counts = BTreeMap::new();
    let mut upgraded = [0usize; 3];
    let mut reason_counts = BTreeMap::new();
    let mut clause_counts = [0usize; 3];
    let mut function_rows = Vec::with_capacity(decisions.len());
    for (decision, (baseline_tier, baseline_reasons)) in decisions.iter().zip(&baselines) {
        *tier_counts.entry(decision.tier.as_str()).or_insert(0usize) += 1;
        *baseline_counts
            .entry(baseline_tier.as_str())
            .or_insert(0usize) += 1;
        if *baseline_tier == Tier::C && decision.tier != Tier::None {
            match decision.tier {
                Tier::A => upgraded[0] += 1,
                Tier::B => upgraded[1] += 1,
                Tier::C => upgraded[2] += 1,
                Tier::None => {}
            }
        }
        let kinds = decision
            .reasons
            .iter()
            .map(|reason| reason.split(':').next().unwrap_or(reason).to_owned())
            .collect::<BTreeSet<_>>();
        for kind in kinds {
            *reason_counts.entry(kind).or_insert(0usize) += 1;
        }
        match decision.tier {
            Tier::A => clause_counts[0] += decision.clauses,
            Tier::B => clause_counts[1] += decision.clauses,
            Tier::C => clause_counts[2] += decision.clauses,
            Tier::None => {}
        }
        function_rows.push(json!({
            "path": decision.path,
            "tier": decision.tier.as_str(),
            "reasons": decision.reasons,
            "baseline_tier": baseline_tier.as_str(),
            "baseline_reasons": baseline_reasons,
            "clauses": decision.clauses,
            "mechanical_clauses": decision.mechanical,
            "unsupported_clauses": decision.unsupported,
            "entry": decision.entry.as_ref().map(|entry| entry["name"].clone()),
            "expected": decision.expected,
            "selected": selected_set.contains(&decision.path),
            "modeling": decision.modeling,
        }));
    }

    let report = json!({
        "schema": SCHEMA,
        "census": {
            "schema": census["schema"],
            "crate": census["crate"],
            "functions": functions.len(),
        },
        "options": {
            "tiers": options.tiers,
            "filter": options.filter,
            "limit": options.limit,
            "batch": options.batch,
            "batch_size": options.batch_size,
            "config": {
                "is_default": options.config.is_default(),
                "generic_instances": options.config.generic_instances.len(),
                "reference": if options.config.reference_opaque { "opaque" } else { "off" },
                "ghost": if options.config.ghost_opaque { "opaque" } else { "off" },
            },
        },
        "counts": {
            "functions": functions.len(),
            "with_requires": decisions.iter().filter(|d| d.tier != Tier::None).count(),
            "no_requires": decisions.iter().filter(|d| d.tier == Tier::None).count(),
            "tier_a": tier_counts.get("A").copied().unwrap_or(0),
            "tier_b": tier_counts.get("B").copied().unwrap_or(0),
            "tier_c": tier_counts.get("C").copied().unwrap_or(0),
            "tier_c_reasons": reason_counts,
            "clauses": {
                "tier_a": clause_counts[0],
                "tier_b": clause_counts[1],
                "tier_c": clause_counts[2],
            },
            "baseline": {
                "tier_a": baseline_counts.get("A").copied().unwrap_or(0),
                "tier_b": baseline_counts.get("B").copied().unwrap_or(0),
                "tier_c": baseline_counts.get("C").copied().unwrap_or(0),
            },
            "upgraded": {
                "to_a": upgraded[0],
                "to_b": upgraded[1],
                "still_c": upgraded[2],
            },
            "candidates": total_candidates,
            "selected": entries.len(),
        },
        "entries": entries,
        "functions": function_rows,
    });

    Ok(json!({"manifest": manifest, "report": report}))
}

/// Write `manifest.json` and `screening.json` under `out`.
pub fn run(census_path: &Path, out: &Path, options: &Options) -> Result<Value> {
    let text = std::fs::read_to_string(census_path)?;
    let census: Value = serde_json::from_str(&text)?;
    let built = build(&census, options)?;
    std::fs::create_dir_all(out)?;
    write_json(&out.join("manifest.json"), &built["manifest"])?;
    write_json(&out.join("screening.json"), &built["report"])?;
    Ok(built)
}

fn classify(
    function: &Value,
    index: usize,
    name_used: &mut BTreeSet<String>,
    config: &Config,
) -> Decision {
    let path = function["path"].as_str().unwrap_or("<unknown>").to_owned();
    let clauses = function["requires"].as_array().cloned().unwrap_or_default();
    let raw_params = function["params"].as_array().cloned().unwrap_or_default();
    let generic_names = function["generics"]["names"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let generic_count = function["generics"]["count"].as_u64().unwrap_or(0);
    let mechanical = clauses
        .iter()
        .filter(|clause| clause["class"].as_str() == Some("mechanical"))
        .count();
    let unsupported = clauses.len().saturating_sub(mechanical);
    let origin = json!({
        "external_body": function["external_body"],
        "kind": function["kind"],
        "visibility": function["visibility"],
        "trait": function["trait"],
        "source": function["source"],
        "mode": function["mode"],
        "generics": function["generics"],
    });
    let mut modeling = json!({
        "generic_instantiation": Value::Null,
        "reference_mapping": [],
        "ghost_mapping": [],
    });

    if clauses.is_empty() {
        return Decision {
            path,
            tier: Tier::None,
            reasons: Vec::new(),
            entry: None,
            expected: None,
            expected_note: None,
            clauses: 0,
            mechanical: 0,
            unsupported: 0,
            modeling,
        };
    }

    let mut reasons = Vec::new();
    let mut params = raw_params
        .iter()
        .filter_map(|param| {
            let name = param["name"].as_str()?.to_owned();
            Some(ParamInfo {
                name,
                ty: param["type"].as_str().unwrap_or("").to_owned(),
                kind: param["kind"].as_str().unwrap_or("opaque").to_owned(),
            })
        })
        .collect::<Vec<_>>();

    // G rules: project-side instantiation, then re-derive parameter kinds.
    if generic_count > 0 && generic_names.is_empty() {
        // Legacy/foreign census without names: keep the M7e blocker.
        reasons.push(format!("generic:{}", function["generics"]["names"]));
    } else if !generic_names.is_empty() {
        match config.generic_rule(&path) {
            Some(rule) => {
                let mut types = BTreeMap::new();
                let mut missing = Vec::new();
                for name in &generic_names {
                    if let Some(ty) = rule.types.get(name).or_else(|| rule.types.get("*")) {
                        types.insert(name.clone(), ty.clone());
                    } else {
                        missing.push(name.clone());
                    }
                }
                if missing.is_empty() {
                    for param in &mut params {
                        param.ty = substitute_type(&param.ty, &types);
                        param.kind = type_kind_of(&param.ty);
                    }
                    modeling["generic_instantiation"] = json!({
                        "matched": rule.pattern,
                        "types": types,
                    });
                } else {
                    reasons.push(format!("generic_uninstantiated:{}", json!(missing)));
                }
            }
            None => reasons.push(format!("generic:{}", function["generics"]["names"])),
        }
    }

    // R/H rules: only parameters referenced by a clause can constrain it.
    let referenced = referenced_names(&clauses, &raw_params);
    let mut reference_mapping = Vec::new();
    let mut ghost_mapping = Vec::new();
    for param in &params {
        if !referenced.contains(&param.name) {
            continue;
        }
        match param.kind.as_str() {
            "ghost" => {
                if config.ghost_opaque && param_is_mappable(&param.name, &clauses) {
                    ghost_mapping.push(mapping_json(param, &clauses));
                } else {
                    let kind = if config.ghost_opaque {
                        "ghost_param_unmapped"
                    } else {
                        "ghost_param"
                    };
                    reasons.push(format!("{kind}:{}", param.name));
                }
            }
            "reference" => {
                if config.reference_opaque && param_is_mappable(&param.name, &clauses) {
                    reference_mapping.push(mapping_json(param, &clauses));
                } else {
                    let kind = if config.reference_opaque {
                        "reference_param_unmapped"
                    } else {
                        "reference_param"
                    };
                    reasons.push(format!("{kind}:{}", param.name));
                }
            }
            _ => {}
        }
    }
    modeling["reference_mapping"] = json!(reference_mapping);
    modeling["ghost_mapping"] = json!(ghost_mapping);

    let tier = if !reasons.is_empty() {
        Tier::C
    } else if unsupported > 0 {
        Tier::B
    } else {
        Tier::A
    };

    if tier == Tier::C {
        return Decision {
            path,
            tier,
            reasons,
            entry: None,
            expected: None,
            expected_note: None,
            clauses: clauses.len(),
            mechanical,
            unsupported,
            modeling,
        };
    }

    let model = param_models(&params, &referenced);
    let mut spec_fns = String::new();
    let mut terms = Vec::new();
    let mut constraints = Vec::new();
    let mut param_order: Vec<String> = Vec::new();
    let mut const_names: BTreeMap<String, String> = BTreeMap::new();
    let mut used_names = model
        .values()
        .map(|p| p.probe.clone())
        .collect::<BTreeSet<_>>();

    for (clause_index, clause) in clauses.iter().enumerate() {
        match clause["class"].as_str() {
            Some("mechanical") => {
                let ast = &clause["ast"];
                let rewritten = rewrite_consts(ast, &mut const_names, &mut used_names);
                let mut vars = BTreeSet::new();
                collect_ast_vars(&rewritten, &mut vars);
                let args = ordered_args(&vars, &params, &model, &const_names, &mut param_order);
                render_mech_fn(
                    &mut spec_fns,
                    index,
                    clause_index,
                    &args,
                    &rewritten,
                    &model,
                );
                terms.push(format!(
                    "p{index}_k{clause_index}({})",
                    args.iter()
                        .map(|a| a.1.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                constraints.push(rewritten);
            }
            _ => {
                let args = unsupported_args(
                    clause,
                    &params,
                    &model,
                    &mut used_names,
                    &mut const_names,
                    &mut param_order,
                );
                let call_args = args
                    .iter()
                    .map(|a| a.1.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                spec_fns.push_str(&format!(
                    "pub uninterp spec fn p{index}_k{clause_index}({}) -> bool;\n\n",
                    args.iter()
                        .map(|(name, _)| {
                            let int = model.get(name).map_or(true, |entry| entry.int);
                            format!("{name}: {}", if int { "usize" } else { "bool" })
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                terms.push(format!("p{index}_k{clause_index}({call_args})"));
            }
        }
    }

    if param_order.is_empty() {
        param_order.push("_unused".into());
        used_names.insert("_unused".into());
    }
    let mut param_types: BTreeMap<String, bool> = BTreeMap::new();
    for model in model.values() {
        param_types.insert(model.probe.clone(), model.int);
    }
    for name in &param_order {
        param_types.entry(name.clone()).or_insert(true);
    }

    let witness = if tier == Tier::A {
        search_witness(&constraints, &param_order, &param_types)
    } else {
        None
    };
    let witness_source = witness
        .as_ref()
        .map(|values| render_witness(values, &param_order, &param_types));
    let expected = if tier == Tier::A && witness_source.is_some() {
        "ConfirmedLive"
    } else {
        "CandidateInconclusive"
    };
    let expected_note = if tier == Tier::A && witness_source.is_none() {
        Some(
            "no bounded witness; a ConfirmedVacuous verdict here is an E6 true positive that must be replicated"
                .into(),
        )
    } else if tier == Tier::B {
        Some("uninterp clauses: verifier incompleteness expected; not a defect".into())
    } else {
        None
    };

    let params_source = param_order
        .iter()
        .map(|name| {
            let typ = if param_types.get(name).copied().unwrap_or(true) {
                "usize"
            } else {
                "bool"
            };
            format!("{name}: {typ}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let short = path
        .rsplit("::")
        .next()
        .unwrap_or("entry")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let short = short.trim_matches('_');
    let mut name = format!("r{index:04}_{short}");
    while name_used.contains(&name) {
        name.push('_');
    }
    name_used.insert(name.clone());

    let entry = json!({
        "name": name,
        "params": params_source,
        "requires": if terms.is_empty() { "true".to_owned() } else { terms.join(" && ") },
        "witness": witness_source,
        "preamble": spec_fns,
        "origin": origin,
    });

    Decision {
        path,
        tier,
        reasons,
        entry: Some(entry),
        expected: Some(expected),
        expected_note,
        clauses: clauses.len(),
        mechanical,
        unsupported,
        modeling,
    }
}

fn sanitize_ident(name: &str) -> String {
    let mut out = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        out.push('p');
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, 'p');
    }
    out
}

fn type_kind_of(ty: &str) -> String {
    let ty = ty.trim();
    if ty.starts_with('&') {
        "reference".to_owned()
    } else if ty.starts_with("Tracked<") || ty.starts_with("Ghost<") {
        "ghost".to_owned()
    } else if ty == "bool" {
        "bool".to_owned()
    } else if is_int_type(ty) {
        "int".to_owned()
    } else {
        "opaque".to_owned()
    }
}

fn is_int_type(ty: &str) -> bool {
    matches!(ty, "int" | "nat" | "char" | "usize" | "isize")
        || ty
            .strip_prefix(['u', 'i'])
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

fn replace_ident(text: &str, name: &str, replacement: &str) -> String {
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '%';
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if token == name {
            out.push_str(replacement);
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if identifier(c) {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn substitute_type(ty: &str, types: &BTreeMap<String, String>) -> String {
    let mut out = ty.to_owned();
    let mut names = types.keys().collect::<Vec<_>>();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    for name in names {
        let replacement = &types[name];
        out = out.replace(&format!("Typ TypParam {name}"), replacement);
        out = replace_ident(&out, name, replacement);
    }
    out
}

fn param_uses(clause: &Value, name: &str) -> Vec<Value> {
    clause["param_uses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["name"].as_str() == Some(name))
        .cloned()
        .collect()
}

fn param_is_mappable(name: &str, clauses: &[Value]) -> bool {
    let mut seen = false;
    for clause in clauses {
        let occurrences = clause["param_occurrences"][name].as_u64().unwrap_or(0) as usize;
        let uses = param_uses(clause, name);
        if occurrences == 0 && uses.is_empty() {
            continue;
        }
        if occurrences == 0 || uses.len() != occurrences {
            return false;
        }
        for entry in &uses {
            match entry["kind"].as_str() {
                Some("receiver_root" | "field_root" | "deref_root") => {}
                _ => return false,
            }
        }
        seen = true;
    }
    seen
}

fn mapping_json(param: &ParamInfo, clauses: &[Value]) -> Value {
    let uses = clauses
        .iter()
        .flat_map(|clause| param_uses(clause, &param.name))
        .map(|entry| {
            json!({
                "kind": entry["kind"],
                "callee": entry["callee"],
                "field": entry["field"],
                "argc": entry["argc"],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "param": param.name,
        "id": format!("{}_id", sanitize_ident(&param.name)),
        "uses": uses,
    })
}

fn referenced_names(clauses: &[Value], params: &[Value]) -> BTreeSet<String> {
    let param_names = params
        .iter()
        .filter_map(|param| param["name"].as_str())
        .collect::<BTreeSet<_>>();
    let mut referenced = BTreeSet::new();
    for clause in clauses {
        if let Some(vars) = clause["vars"].as_array() {
            for var in vars.iter().filter_map(Value::as_str) {
                if param_names.contains(var) {
                    referenced.insert(var.to_owned());
                }
            }
        }
    }
    referenced
}

fn param_models(
    params: &[ParamInfo],
    referenced: &BTreeSet<String>,
) -> BTreeMap<String, ParamModel> {
    params
        .iter()
        .filter_map(|param| {
            if !referenced.contains(&param.name) {
                return None;
            }
            let int = param.kind != "bool";
            let probe = if param.kind == "int" || param.kind == "bool" {
                sanitize_ident(&param.name)
            } else {
                format!("{}_id", sanitize_ident(&param.name))
            };
            Some((param.name.clone(), ParamModel { probe, int }))
        })
        .collect()
}

fn rewrite_consts(
    ast: &Value,
    const_names: &mut BTreeMap<String, String>,
    used_names: &mut BTreeSet<String>,
) -> Value {
    match ast["op"].as_str() {
        Some("const") => {
            let path = ast["path"].as_str().unwrap_or("const");
            let name = lift_const(path, const_names, used_names);
            json!({"op": "var", "name": name})
        }
        Some("not") => {
            json!({"op": "not", "arg": rewrite_consts(&ast["arg"], const_names, used_names)})
        }
        Some("neg") => {
            json!({"op": "neg", "arg": rewrite_consts(&ast["arg"], const_names, used_names)})
        }
        Some("bin") => json!({
            "op": "bin",
            "sym": ast["sym"],
            "lhs": rewrite_consts(&ast["lhs"], const_names, used_names),
            "rhs": rewrite_consts(&ast["rhs"], const_names, used_names),
        }),
        _ => ast.clone(),
    }
}

fn lift_const(
    path: &str,
    const_names: &mut BTreeMap<String, String>,
    used_names: &mut BTreeSet<String>,
) -> String {
    if let Some(name) = const_names.get(path) {
        return name.clone();
    }
    let base = path
        .rsplit("::")
        .next()
        .unwrap_or("c")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let base = if base.is_empty() {
        "c".to_owned()
    } else {
        base
    };
    let mut name = base.clone();
    let mut suffix = 2;
    while used_names.contains(&name) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    used_names.insert(name.clone());
    const_names.insert(path.to_owned(), name.clone());
    name
}

fn collect_ast_vars(ast: &Value, out: &mut BTreeSet<String>) {
    match ast["op"].as_str() {
        Some("var") => {
            if let Some(name) = ast["name"].as_str() {
                out.insert(name.to_owned());
            }
        }
        Some("not") | Some("neg") => collect_ast_vars(&ast["arg"], out),
        Some("bin") => {
            collect_ast_vars(&ast["lhs"], out);
            collect_ast_vars(&ast["rhs"], out);
        }
        _ => {}
    }
}

fn ordered_args(
    vars: &BTreeSet<String>,
    params: &[ParamInfo],
    model: &BTreeMap<String, ParamModel>,
    const_names: &BTreeMap<String, String>,
    param_order: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut args = Vec::new();
    for param in params {
        let original = param.name.as_str();
        if let Some(entry) = model.get(original) {
            if vars.contains(&entry.probe) || vars.contains(original) {
                args.push((entry.probe.clone(), entry.probe.clone()));
                if !param_order.contains(&entry.probe) {
                    param_order.push(entry.probe.clone());
                }
            }
        }
    }
    for (path, name) in const_names {
        if vars.contains(name) {
            args.push((name.clone(), name.clone()));
            if !param_order.contains(name) {
                param_order.push(name.clone());
            }
            let _ = path;
        }
    }
    args
}

fn unsupported_args(
    clause: &Value,
    params: &[ParamInfo],
    model: &BTreeMap<String, ParamModel>,
    used_names: &mut BTreeSet<String>,
    const_names: &mut BTreeMap<String, String>,
    param_order: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut vars = BTreeSet::new();
    for var in clause["vars"].as_array().into_iter().flatten() {
        if let Some(var) = var.as_str() {
            vars.insert(var.to_owned());
        }
    }
    let mut args = Vec::new();
    for param in params {
        let original = param.name.as_str();
        if vars.contains(original) {
            if let Some(entry) = model.get(original) {
                args.push((entry.probe.clone(), entry.probe.clone()));
                if !param_order.contains(&entry.probe) {
                    param_order.push(entry.probe.clone());
                }
            } else {
                // Free variable that is not a modelable parameter: keep an
                // opaque id with a deterministic name.
                let base = format!("{}_id", sanitize_ident(original));
                let mut name = base.clone();
                let mut suffix = 2;
                while used_names.contains(&name) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                used_names.insert(name.clone());
                args.push((name.clone(), name.clone()));
                if !param_order.contains(&name) {
                    param_order.push(name);
                }
            }
        }
    }
    for path in clause["consts"].as_array().into_iter().flatten() {
        if let Some(path) = path.as_str() {
            let name = lift_const(path, const_names, used_names);
            args.push((name.clone(), name.clone()));
            if !param_order.contains(&name) {
                param_order.push(name);
            }
        }
    }
    args
}

fn render_mech_fn(
    out: &mut String,
    index: usize,
    clause_index: usize,
    args: &[(String, String)],
    ast: &Value,
    model: &BTreeMap<String, ParamModel>,
) {
    let arglist = args
        .iter()
        .map(|(name, _)| {
            let int = model.get(name).map_or(true, |entry| entry.int);
            format!("{name}: {}", if int { "usize" } else { "bool" })
        })
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!(
        "pub open spec fn p{index}_k{clause_index}({arglist}) -> bool {{\n    {}\n}}\n\n",
        render_ast(ast)
    ));
}

fn render_ast(ast: &Value) -> String {
    match ast["op"].as_str() {
        Some("int") => ast["value"].to_string(),
        Some("bool") => ast["value"].to_string(),
        Some("var") => ast["name"].as_str().unwrap_or("_").to_owned(),
        Some("const") => ast["path"].as_str().unwrap_or("_").to_owned(),
        Some("not") => format!("!({})", render_ast(&ast["arg"])),
        Some("neg") => format!("-({})", render_ast(&ast["arg"])),
        Some("bin") => format!(
            "({} {} {})",
            render_ast(&ast["lhs"]),
            ast["sym"].as_str().unwrap_or("&&"),
            render_ast(&ast["rhs"])
        ),
        _ => "true".into(),
    }
}

fn search_witness(
    constraints: &[Value],
    param_order: &[String],
    param_types: &BTreeMap<String, bool>,
) -> Option<Vec<Value>> {
    let constraint_vars = constraints
        .iter()
        .map(|ast| {
            let mut vars = BTreeSet::new();
            collect_ast_vars(ast, &mut vars);
            vars
        })
        .collect::<Vec<_>>();
    let domains: Vec<Vec<Value>> = param_order
        .iter()
        .map(|name| {
            if param_types.get(name).copied().unwrap_or(true) {
                [
                    0i64, 1, 2, 3, 4, 5, 6, 7, 8, 15, 16, 31, 32, 63, 64, 255, 256, 4096,
                ]
                .into_iter()
                .map(|value| json!(value))
                .collect()
            } else {
                vec![json!(false), json!(true)]
            }
        })
        .collect();

    let mut env: BTreeMap<String, Value> = BTreeMap::new();
    let mut nodes = 0usize;
    const NODE_CAP: usize = 200_000;

    fn evaluate(ast: &Value, env: &BTreeMap<String, Value>) -> Option<Value> {
        match ast["op"].as_str()? {
            "int" | "bool" => Some(ast["value"].clone()),
            "var" => env.get(ast["name"].as_str()?).cloned(),
            "const" => None,
            "not" => Some(json!(!evaluate(&ast["arg"], env)?.as_bool()?)),
            "neg" => Some(json!(-evaluate(&ast["arg"], env)?.as_i64()?)),
            "bin" => {
                let lhs = evaluate(&ast["lhs"], env)?;
                let rhs = evaluate(&ast["rhs"], env)?;
                let sym = ast["sym"].as_str()?;
                Some(match sym {
                    "+" => json!(lhs.as_i64()? + rhs.as_i64()?),
                    "-" => json!(lhs.as_i64()? - rhs.as_i64()?),
                    "*" => json!(lhs.as_i64()? * rhs.as_i64()?),
                    "==" => json!(lhs == rhs),
                    "!=" => json!(lhs != rhs),
                    "<" => json!(lhs.as_i64()? < rhs.as_i64()?),
                    "<=" => json!(lhs.as_i64()? <= rhs.as_i64()?),
                    ">" => json!(lhs.as_i64()? > rhs.as_i64()?),
                    ">=" => json!(lhs.as_i64()? >= rhs.as_i64()?),
                    "&&" => json!(lhs.as_bool()? && rhs.as_bool()?),
                    "||" => json!(lhs.as_bool()? || rhs.as_bool()?),
                    "==>" => json!(!lhs.as_bool()? || rhs.as_bool()?),
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    fn feasible(
        constraints: &[Value],
        constraint_vars: &[BTreeSet<String>],
        env: &BTreeMap<String, Value>,
    ) -> bool {
        for (ast, vars) in constraints.iter().zip(constraint_vars) {
            if vars.iter().all(|name| env.contains_key(name)) {
                match evaluate(ast, env) {
                    Some(value) if value.as_bool() == Some(true) => {}
                    _ => return false,
                }
            }
        }
        true
    }

    fn dfs(
        index: usize,
        param_order: &[String],
        domains: &[Vec<Value>],
        constraints: &[Value],
        constraint_vars: &[BTreeSet<String>],
        env: &mut BTreeMap<String, Value>,
        nodes: &mut usize,
    ) -> Option<Vec<Value>> {
        if *nodes > NODE_CAP {
            return None;
        }
        *nodes += 1;
        if index == param_order.len() {
            return Some(param_order.iter().map(|name| env[name].clone()).collect());
        }
        let name = &param_order[index];
        for value in &domains[index] {
            env.insert(name.clone(), value.clone());
            if feasible(constraints, constraint_vars, env) {
                if let Some(values) = dfs(
                    index + 1,
                    param_order,
                    domains,
                    constraints,
                    constraint_vars,
                    env,
                    nodes,
                ) {
                    return Some(values);
                }
            }
            env.remove(name);
        }
        None
    }

    dfs(
        0,
        param_order,
        &domains,
        constraints,
        &constraint_vars,
        &mut env,
        &mut nodes,
    )
}

fn render_witness(
    values: &[Value],
    param_order: &[String],
    param_types: &BTreeMap<String, bool>,
) -> String {
    let parts = values
        .iter()
        .zip(param_order)
        .map(|(value, name)| {
            if param_types.get(name).copied().unwrap_or(true) {
                format!("{}usize", value.as_i64().unwrap_or(0))
            } else {
                value.as_bool().unwrap_or(false).to_string()
            }
        })
        .collect::<Vec<_>>();
    if parts.len() == 1 {
        parts[0].clone()
    } else {
        format!("({})", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, kind: &str) -> Value {
        json!({"name": name, "type": kind, "kind": kind, "mode": "exec"})
    }

    fn clause(index: usize, class: &str, ast: Value, vars: &[&str]) -> Value {
        json!({
            "index": index,
            "span": null,
            "file": null,
            "line": null,
            "text": null,
            "class": class,
            "ast": if class == "mechanical" { ast } else { Value::Null },
            "ast_vars": [],
            "ast_consts": [],
            "vars": vars,
            "consts": [],
            "calls": [],
            "features": {},
        })
    }

    fn function(path: &str, params: Vec<Value>, generics: usize, clauses: Vec<Value>) -> Value {
        json!({
            "path": path,
            "name": path.rsplit("::").next().unwrap_or(path),
            "mode": "exec",
            "kind": "static",
            "visibility": {"kind": "pub", "restricted_to": null},
            "source": {"file": null, "line": null, "span": null},
            "external_body": false,
            "trait": null,
            "generics": {"count": generics, "names": []},
            "params": params,
            "requires": clauses,
        })
    }

    fn census(functions: Vec<Value>) -> Value {
        json!({"schema": "spec-check.census/1", "crate": "p", "functions": functions})
    }

    #[test]
    fn tiers_follow_the_documented_blockers() {
        let lt = json!({
            "op": "bin", "sym": "<",
            "lhs": {"op": "var", "name": "x"},
            "rhs": {"op": "var", "name": "y"}
        });
        let unsupported = clause(0, "unsupported", Value::Null, &["x"]);
        let data = census(vec![
            function(
                "p::a",
                vec![param("x", "int"), param("y", "int")],
                0,
                vec![clause(0, "mechanical", lt.clone(), &["x", "y"])],
            ),
            function(
                "p::b",
                vec![param("x", "int")],
                0,
                vec![unsupported.clone()],
            ),
            function(
                "p::c",
                vec![param("x", "int")],
                1,
                vec![clause(0, "mechanical", lt.clone(), &["x"])],
            ),
            function(
                "p::d",
                vec![param("g", "ghost")],
                0,
                vec![clause(0, "mechanical", lt, &["g"])],
            ),
            function("p::e", vec![param("r", "reference")], 0, vec![]),
            function("p::f", vec![param("x", "int")], 0, vec![]),
        ]);
        let built = build(&data, &Options::default()).unwrap();
        let rows = built["report"]["functions"].as_array().unwrap();
        let tiers = rows
            .iter()
            .map(|row| row["tier"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(tiers, vec!["A", "B", "C", "C", "none", "none"]);
        assert_eq!(built["report"]["counts"]["tier_a"], 1);
        assert_eq!(built["report"]["counts"]["tier_b"], 1);
        assert_eq!(built["report"]["counts"]["tier_c"], 2);
    }

    #[test]
    fn mechanical_clauses_are_wrapped_and_witnessed() {
        let ast = json!({
            "op": "bin", "sym": "<=",
            "lhs": {"op": "var", "name": "x"},
            "rhs": {"op": "int", "value": 3}
        });
        let data = census(vec![function(
            "p::f",
            vec![param("x", "int"), param("unused", "int")],
            0,
            vec![clause(0, "mechanical", ast, &["x"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "x: usize");
        assert_eq!(contract["requires"], "p0_k0(x)");
        assert_eq!(contract["witness"], "0usize");
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("pub open spec fn p0_k0(x: usize) -> bool")
        );
        assert_eq!(built["report"]["entries"][0]["expected"], "ConfirmedLive");
    }

    #[test]
    fn unsatisfiable_scalar_clause_has_no_witness_and_expects_vacuous() {
        let ast = json!({
            "op": "bin", "sym": "&&",
            "lhs": {"op": "bin", "sym": ">", "lhs": {"op": "var", "name": "x"}, "rhs": {"op": "int", "value": 7}},
            "rhs": {"op": "bin", "sym": "<", "lhs": {"op": "var", "name": "x"}, "rhs": {"op": "int", "value": 8}}
        });
        let data = census(vec![function(
            "p::f",
            vec![param("x", "int")],
            0,
            vec![clause(0, "mechanical", ast, &["x"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert!(built["manifest"]["contracts"][0].get("witness").is_none());
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
        assert!(
            built["report"]["entries"][0]["expected_note"]
                .as_str()
                .unwrap()
                .contains("E6")
        );
    }

    #[test]
    fn constant_clauses_are_lifted_to_parameters() {
        let ast = json!({
            "op": "bin", "sym": "<=",
            "lhs": {"op": "var", "name": "x"},
            "rhs": {"op": "const", "path": "p::MAX_LEN"}
        });
        let data = census(vec![function(
            "p::f",
            vec![param("x", "int")],
            0,
            vec![clause(0, "mechanical", ast, &["x"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "x: usize, max_len: usize");
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("p0_k0(x: usize, max_len: usize)")
        );
        assert_eq!(contract["witness"], "(0usize, 0usize)");
    }

    #[test]
    fn unsupported_clause_becomes_zero_arg_uninterp() {
        let data = census(vec![function(
            "p::f",
            vec![param("self", "opaque")],
            0,
            vec![clause(0, "unsupported", Value::Null, &[])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("pub uninterp spec fn p0_k0() -> bool;")
        );
        assert_eq!(built["manifest"]["contracts"][0]["requires"], "p0_k0()");
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
    }

    #[test]
    fn unsupported_clause_binds_opaque_parameters_as_ids() {
        let data = census(vec![function(
            "p::f",
            vec![param("left", "opaque"), param("right", "opaque")],
            0,
            vec![clause(0, "unsupported", Value::Null, &["left", "right"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "left_id: usize, right_id: usize");
        assert_eq!(contract["requires"], "p0_k0(left_id, right_id)");
        let preamble = built["manifest"]["preamble"].as_str().unwrap();
        assert!(
            preamble
                .contains("pub uninterp spec fn p0_k0(left_id: usize, right_id: usize) -> bool;"),
            "{preamble}"
        );
    }

    #[test]
    fn boolean_parameters_get_bool_domains_and_witnesses() {
        let data = census(vec![function(
            "p::f",
            vec![param("b", "bool")],
            0,
            vec![clause(
                0,
                "mechanical",
                json!({"op": "var", "name": "b"}),
                &["b"],
            )],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert_eq!(built["manifest"]["contracts"][0]["params"], "b: bool");
        assert_eq!(built["manifest"]["contracts"][0]["witness"], "true");
        assert_eq!(built["report"]["entries"][0]["expected"], "ConfirmedLive");
    }

    #[test]
    fn batching_is_deterministic_and_selects_contiguous_ranges() {
        let ast = json!({"op": "bool", "value": true});
        let functions = (0..5)
            .map(|index| {
                function(
                    &format!("p::f{index}"),
                    vec![param("x", "int")],
                    0,
                    vec![clause(0, "mechanical", ast.clone(), &["x"])],
                )
            })
            .collect();
        let data = census(functions);
        let options = Options {
            batch: Some(1),
            batch_size: 2,
            ..Options::default()
        };
        let built = build(&data, &options).unwrap();
        let entries = built["report"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["path"], "p::f2");
        assert_eq!(entries[1]["path"], "p::f3");
    }

    #[test]
    fn filter_and_limit_apply_after_tier_selection() {
        let ast = json!({"op": "bool", "value": true});
        let functions = (0..3)
            .map(|index| {
                function(
                    &format!("p::keep{index}"),
                    vec![param("x", "int")],
                    0,
                    vec![clause(0, "mechanical", ast.clone(), &["x"])],
                )
            })
            .chain([function(
                "p::drop",
                vec![param("x", "int")],
                0,
                vec![clause(0, "mechanical", ast.clone(), &["x"])],
            )])
            .collect();
        let data = census(functions);
        let options = Options {
            filter: Some("keep".into()),
            limit: Some(2),
            ..Options::default()
        };
        let built = build(&data, &options).unwrap();
        assert_eq!(built["report"]["entries"].as_array().unwrap().len(), 2);
        assert_eq!(built["report"]["counts"]["candidates"], 3);
    }

    fn typed_function(
        path: &str,
        generics: &[&str],
        params: Vec<Value>,
        clauses: Vec<Value>,
    ) -> Value {
        json!({
            "path": path,
            "name": path.rsplit("::").next().unwrap_or(path),
            "mode": "exec",
            "kind": "static",
            "visibility": {"kind": "pub", "restricted_to": null},
            "source": {"file": null, "line": null, "span": null},
            "external_body": false,
            "trait": null,
            "generics": {"count": generics.len(), "names": generics},
            "params": params,
            "requires": clauses,
        })
    }

    fn config(value: Value) -> Config {
        Config::from_value(&value).unwrap()
    }

    fn uses_clause(kind: &str, name: &str, occurrences: usize) -> Value {
        let mut clause = clause(0, "unsupported", Value::Null, &[name]);
        clause["param_occurrences"] = json!({name: occurrences});
        clause["param_uses"] =
            json!([{"name": name, "kind": kind, "callee": "p::inv", "field": null, "argc": 1}]);
        clause
    }

    #[test]
    fn generic_instantiation_reclassifies_and_unmapped_names_stay_c() {
        let lt = json!({
            "op": "bin", "sym": "<=",
            "lhs": {"op": "var", "name": "n"},
            "rhs": {"op": "int", "value": 3}
        });
        let data = census(vec![
            typed_function(
                "p::lemma",
                &["E"],
                vec![param("s", "opaque"), param("n", "int")],
                vec![clause(0, "mechanical", lt.clone(), &["n"])],
            ),
            typed_function(
                "p::other",
                &["E"],
                vec![param("n", "int")],
                vec![clause(0, "mechanical", lt, &["n"])],
            ),
        ]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "generic_instances": [{"match": "p::lemma", "types": {"E": "u64"}}]
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let rows = built["report"]["functions"].as_array().unwrap();
        assert_eq!(rows[0]["tier"], "A");
        assert_eq!(rows[0]["baseline_tier"], "C");
        assert_eq!(
            rows[0]["modeling"]["generic_instantiation"]["matched"],
            "p::lemma"
        );
        assert_eq!(built["report"]["counts"]["baseline"]["tier_c"], 2);
        assert_eq!(built["report"]["counts"]["upgraded"]["to_a"], 1);
        assert_eq!(rows[1]["tier"], "C");
        assert!(
            rows[1]["reasons"][0]
                .as_str()
                .unwrap()
                .starts_with("generic:")
        );
        assert_eq!(built["manifest"]["contracts"][0]["witness"], "0usize");
    }

    #[test]
    fn generic_instantiation_defaults_all_parameters_to_star() {
        let ast = json!({
            "op": "bin", "sym": "==",
            "lhs": {"op": "var", "name": "n"},
            "rhs": {"op": "int", "value": 0}
        });
        let data = census(vec![typed_function(
            "p::f",
            &["T", "U"],
            vec![param("n", "int")],
            vec![clause(0, "mechanical", ast, &["n"])],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "generic_instances": [{"match": "*", "types": {"*": "u64"}}]
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(built["report"]["functions"][0]["tier"], "A");
    }

    #[test]
    fn reference_receiver_use_is_abstracted_to_an_id_at_tier_b() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference"), param("cpu", "opaque")],
            vec![uses_clause("receiver_root", "self", 1)],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"}
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "B");
        assert_eq!(row["baseline_tier"], "C");
        assert_eq!(row["modeling"]["reference_mapping"][0]["id"], "self_id");
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "self_id: usize");
        assert_eq!(contract["requires"], "p0_k0(self_id)");
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
    }

    #[test]
    fn reference_direct_use_stays_tier_c_with_an_unmapped_reason() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference")],
            vec![uses_clause("plain_root", "self", 1)],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"}
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "C");
        assert_eq!(row["reasons"][0], "reference_param_unmapped:self");
        assert!(
            built["manifest"]["contracts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn ghost_receiver_is_abstracted_only_when_the_policy_is_on() {
        let data = census(vec![typed_function(
            "p::perm",
            &[],
            vec![param("tracked", "ghost")],
            vec![uses_clause("receiver_root", "tracked", 1)],
        )]);
        let off = build(&data, &Options::default()).unwrap();
        assert_eq!(off["report"]["functions"][0]["tier"], "C");
        assert_eq!(
            off["report"]["functions"][0]["reasons"][0],
            "ghost_param:tracked"
        );
        let on = build(
            &data,
            &Options {
                config: config(json!({
                    "schema": CONFIG_SCHEMA,
                    "ghost": {"mode": "opaque"}
                })),
                ..Options::default()
            },
        )
        .unwrap();
        let row = &on["report"]["functions"][0];
        assert_eq!(row["tier"], "B");
        assert_eq!(row["modeling"]["ghost_mapping"][0]["id"], "tracked_id");
        assert_eq!(
            on["manifest"]["contracts"][0]["params"],
            "tracked_id: usize"
        );
    }

    #[test]
    fn tier_recomputation_is_deterministic() {
        let data = census(vec![
            typed_function(
                "p::g",
                &["T"],
                vec![param("self", "reference")],
                vec![uses_clause("receiver_root", "self", 1)],
            ),
            typed_function(
                "p::h",
                &[],
                vec![param("self", "reference")],
                vec![uses_clause("plain_root", "self", 1)],
            ),
        ]);
        let options = Options {
            config: config(json!({
                "schema": CONFIG_SCHEMA,
                "generic_instances": [{"match": "*", "types": {"*": "u64"}}],
                "reference": {"mode": "opaque"},
                "ghost": {"mode": "opaque"}
            })),
            ..Options::default()
        };
        let first = build(&data, &options).unwrap();
        let second = build(&data, &options).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn empty_param_mechanical_entry_uses_dummy_parameter() {
        let data = census(vec![function(
            "p::f",
            vec![],
            0,
            vec![clause(
                0,
                "mechanical",
                json!({"op": "bool", "value": false}),
                &[],
            )],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert_eq!(
            built["manifest"]["contracts"][0]["params"],
            "_unused: usize"
        );
        assert!(built["manifest"]["contracts"][0].get("witness").is_none());
    }
}
