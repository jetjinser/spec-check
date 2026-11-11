//! Enable complete ordinary caller bodies in snapshots of a captured project.
use crate::{Result, capture::Invocation, client, hash, ir, link, runner, vir, write_json};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn success_demands(inventory: &Value, owner: &str) -> Vec<Value> {
    inventory["records"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| {
            if record["kind"] != "mir_call" || record["owner"] != owner {
                return None;
            }
            let target = record["target"].as_str()?;
            let identity = record["target_id"].as_str()?;
            if !identity.contains(" ~ core[") && !identity.contains(" ~ std[") {
                return None;
            }
            let required = match target {
                "core::option::Option::<T>::unwrap"
                | "core::option::Option::<T>::expect"
                | "std::option::Option::<T>::unwrap"
                | "std::option::Option::<T>::expect" => "Some",
                "core::result::Result::<T, E>::unwrap"
                | "core::result::Result::<T, E>::expect"
                | "std::result::Result::<T, E>::unwrap"
                | "std::result::Result::<T, E>::expect" => "Ok",
                _ => return None,
            };
            Some(
                json!({"kind":"success_requirement","required_variant":required,
            "source_span":record["span"],"block":record["block"],"resolved_target":target,
            "target_identity":identity,"status":"discovered_not_independently_checked"}),
            )
        })
        .collect()
}

fn call_associations(relations: &[Value], owner: &str) -> Vec<Value> {
    relations.iter().filter_map(|relation| {
        let call = &relation["call"];
        if call["owner"] != owner {
            return None;
        }
        let association = &relation["association"];
        let status = association["status"].as_str().unwrap_or("missing_status");
        let requirements = if status == "linked" {
            let mut sources = Vec::new();
            let primary = &association["contract"];
            sources.push(json!({"kind":"resolved_interface","function_span":primary["function_span"],"requires":primary["requires"]}));
            for contract in association["trait_contracts"].as_array().into_iter().flatten() {
                sources.push(json!({"kind":"trait_declaration","function_span":contract["function_span"],"requires":contract["requires"]}));
            }
            Some(sources)
        } else {
            None
        };
        Some(json!({"source_span":call["span"],"block":call["block"],
            "compiler_target":call["target"],"compiler_target_id":call["target_id"],
            "association_status":status,"interface":association["interface"],
            "requirements":requirements,"status":"discovered_not_independently_checked"}))
    }).collect()
}

fn backend_limitations(verification: &Value) -> Vec<Value> {
    verification["diagnostics"].as_array().into_iter().flatten().filter_map(|diagnostic| {
        if diagnostic["level"] != "error" { return None; }
        let message = diagnostic["message"].as_str()?;
        if message.starts_with("aborting due to") { return None; }
        let kind = if message.starts_with("The verifier does not yet support the following Rust feature:") {
            "unsupported_rust_feature"
        } else if message.starts_with("trait ") && message.ends_with(" not declared to Verus") {
            "undeclared_trait"
        } else if message.contains(" is not supported (note: you may be able to add a Verus specification") {
            "unsupported_api"
        } else {
            "other_backend_error"
        };
        Some(json!({"kind":kind,"message":message,"spans":diagnostic["spans"],"basis":"Verus diagnostic text"}))
    }).collect()
}

// Record the narrowly checkable case in which a call returns directly into an
// argument of the next call, without intervening executable statements.
fn direct_call_flows(records: &[ir::Record]) -> Result<Value> {
    let normal = regex::Regex::new(r"\[return: (bb\d+)(?:,|\])")?;
    let local = regex::Regex::new(r"^_\d+$")?;
    let mut flows = Vec::new();
    for producer in records.iter().filter(|r| r["kind"] == "call") {
        let place = &producer["destination"];
        if !local.is_match(place) {
            continue;
        }
        let terms = records
            .iter()
            .filter(|r| r["kind"] == "term" && r["block"] == producer["block"])
            .collect::<Vec<_>>();
        let [term] = terms.as_slice() else {
            continue;
        };
        let Some(next) = normal.captures(&term["operation"]) else {
            continue;
        };
        let block = &next[1];
        let harmless = records
            .iter()
            .filter(|r| r["kind"] == "stmt" && r["block"] == block)
            .all(|s| {
                let op = &s["operation"];
                (op.starts_with("StorageLive(") || op.starts_with("StorageDead("))
                    && op != &format!("StorageDead({place})")
                    && op != &format!("StorageLive({place})")
            });
        if !harmless {
            continue;
        }
        let consumers = records
            .iter()
            .filter(|r| r["kind"] == "call" && r["block"] == block)
            .collect::<Vec<_>>();
        let [consumer] = consumers.as_slice() else {
            continue;
        };
        let Ok(arguments) = crate::resource::arguments(consumer) else {
            continue;
        };
        let positions = arguments
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| {
                (arg == &format!("move {place}") || arg == &format!("copy {place}")).then_some(i)
            })
            .collect::<Vec<_>>();
        if !positions.is_empty() {
            flows.push(json!({"producer":producer,"producer_span":term["span"],"consumer":consumer,
                "consumer_span":records.iter().find(|r| r["kind"] == "term" && r["block"] == block).map(|r| &r["span"]),
                "argument_indices":positions,"place":place,"status":"compiler_value_flow_only"}));
        }
    }
    Ok(
        json!({"scope":"Direct normal-return edge and unchanged MIR local; other flows and paths are not analyzed. This is value-flow evidence, not a proof of contract implication or reachability.","flows":flows}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    sources: Vec<PathBuf>,
}

fn context_paths(selected: &str, callers: &[Value], inventory: &Value) -> Result<BTreeSet<String>> {
    let records = inventory["records"]
        .as_array()
        .ok_or("missing compiler records")?;
    let owners = records
        .iter()
        .filter(|r| r["kind"] == "owner")
        .filter_map(|r| Some((r["owner"].as_str()?, r["path"].as_str()?)))
        .collect::<BTreeMap<_, _>>();
    let ordinary = callers
        .iter()
        .filter(|c| c["route"] == "requires_generated_client")
        .filter_map(|c| Some((c["owner"]["owner"].as_str()?, c["owner"]["path"].as_str()?)))
        .collect::<BTreeMap<_, _>>();
    let mut paths = BTreeSet::from([selected.to_owned()]);
    loop {
        let mut extra = BTreeSet::new();
        for call in records.iter().filter(|r| r["kind"] == "mir_call") {
            let Some(owner) = call["owner"].as_str().and_then(|id| owners.get(id)) else {
                continue;
            };
            if paths
                .iter()
                .any(|p| *owner == p || owner.starts_with(&format!("{p}::")))
                && let Some(path) = call["target_id"].as_str().and_then(|id| ordinary.get(id))
            {
                extra.insert((*path).to_owned());
            }
        }
        let old = paths.len();
        paths.extend(extra);
        if paths.len() == old {
            return Ok(paths);
        }
    }
}

pub(crate) fn call_contract(functions: &[vir::Node], path: &str) -> Value {
    ContractIndex::new(functions).call_contract(path)
}

pub(crate) struct ContractIndex<'a> {
    functions: BTreeMap<&'a str, Vec<&'a vir::Node>>,
}

impl<'a> ContractIndex<'a> {
    pub(crate) fn new(functions: &'a [vir::Node]) -> Self {
        let mut index = BTreeMap::<_, Vec<_>>::new();
        for function in functions {
            if let Some(path) = function
                .field("name")
                .and_then(|n| n.field("path"))
                .and_then(vir::Node::atom)
            {
                index.entry(path).or_default().push(function);
            }
        }
        Self { functions: index }
    }

    pub(crate) fn lookup(&self, path: &str) -> Option<&'a vir::Node> {
        let functions = self.functions.get(path)?;
        (functions.len() == 1).then_some(functions[0])
    }

    pub(crate) fn call_contract(&self, path: &str) -> Value {
        let mut sources = Vec::new();
        let mut constant_predicates = Vec::new();
        let implementation = self.lookup(path).and_then(|f| f.field("kind"));
        let mut next = Some(path);
        let mut seen = BTreeSet::new();
        while let Some(path) = next {
            if !seen.insert(path) {
                return json!({"path":path,"requires":null,"reason":"cyclic trait contract"});
            }
            let Some(function) = self.lookup(path) else {
                return json!({"path":path,"requires":null,"reason":"native contract missing or ambiguous"});
            };
            if let Some(ensure) = function.field("ensure") {
                let mut predicates = Vec::new();
                vir::walk_calls(ensure, &mut predicates);
                predicates.sort();
                predicates.dedup();
                for predicate in predicates {
                    let resolved = self.constant_predicate(&predicate, implementation);
                    if let Some(mut resolved) = resolved {
                        resolved["contract_source"] = json!(path);
                        constant_predicates.push(resolved);
                    }
                }
            }
            sources.push(json!({
            "function":path,
            "requires":function.field("require").and_then(vir::Node::list).map(|xs|xs.iter().map(crate::contracts::render).collect::<Vec<_>>()),
            "ensures":function.field("ensure").and_then(vir::Node::list).filter(|xs|xs.len()==3 && xs[0].atom()==Some("tuple")).and_then(|xs|xs[1].list()).map(|xs|xs.iter().map(crate::contracts::render).collect::<Vec<_>>()),
            "ensure_representation":function.field("ensure").map(crate::contracts::render),
            "returns":function.field("returns").map(crate::contracts::render),
            "parameters":function.field("params").map(crate::contracts::render),
            "outer_return":function.field("outer_ret").map(crate::contracts::render),
            "inner_return":function.field("inner_ret").map(crate::contracts::render),
            "external_body":function.field("attrs").and_then(|a|a.field("is_external_body")).and_then(vir::Node::atom)
        }));
            next = function
                .field("kind")
                .and_then(|k| k.field("method"))
                .and_then(|m| m.field("path"))
                .and_then(vir::Node::atom);
        }
        let requirements = sources
            .iter()
            .filter_map(|s| s["requires"].as_array())
            .flatten()
            .collect::<Vec<_>>();
        let guarantees = sources
            .iter()
            .filter_map(|s| s["ensures"].as_array())
            .flatten()
            .collect::<Vec<_>>();
        json!({"path":path,"requires":requirements,"ensures":guarantees,
        "returns":sources.iter().filter_map(|s|s.get("returns")).collect::<Vec<_>>(),"require_origins":sources,
        "constant_postcondition_predicates":constant_predicates})
    }

    pub(crate) fn constant_predicate(
        &self,
        predicate: &str,
        implementation: Option<&vir::Node>,
    ) -> Option<Value> {
        constant_post_predicate(self, predicate, implementation)
    }
}

// Trait implementations inherit entry conditions even when their own list is empty.
fn check_entry_contract(functions: &[vir::Node], path: &str) -> Result<()> {
    let contract = call_contract(functions, path);
    let requires = contract["requires"]
        .as_array()
        .ok_or("missing or ambiguous generated entry contract")?;
    if !requires.is_empty() {
        return Err("generated entry conditions, including inherited requires, need provenance before accepting this caller".into());
    }
    Ok(())
}

fn constant_post_predicate(
    index: &ContractIndex<'_>,
    predicate: &str,
    implementation: Option<&vir::Node>,
) -> Option<Value> {
    let lookup = |path: &str| index.lookup(path);
    let declaration = lookup(predicate)?;
    let kind = declaration.field("kind")?;
    let mut path = predicate.to_owned();
    let function = if kind.list()?.get(1)?.atom() == Some("TraitMethodDecl") {
        let implementation = implementation?;
        let trait_path = implementation.field("trait_path")?.atom()?;
        let (owner, method) = predicate.rsplit_once("::")?;
        if owner != trait_path {
            return None;
        }
        path = format!("{}::{method}", implementation.field("impl_path")?.atom()?);
        let function = lookup(&path)?;
        let kind = function.field("kind")?;
        if kind.field("method")?.field("path")?.atom()? != predicate
            || kind.field("trait_path")?.atom()? != trait_path
        {
            return None;
        }
        function
    } else {
        declaration
    };
    if function.field("mode")?.atom()? != "Spec"
        || function.field("attrs")?.field("is_external_body")?.atom()? != "false"
    {
        return None;
    }
    let body = function.field("body")?;
    let value = constant_bool(body)?;
    Some(
        json!({"predicate":predicate,"resolved_to":path,"value":value,"span":body.list().and_then(|v|v.get(1)).and_then(vir::Node::atom),
        "body":crate::contracts::render(body),"assessment":"Constant predicate occurrence in an existing postcondition; significance depends on the surrounding condition and client demand."}),
    )
}

fn constant_bool(node: &vir::Node) -> Option<bool> {
    let parts = node.unwrap().list()?;
    if parts.first()?.atom()? != ">" {
        return None;
    }
    match parts.get(1)?.atom()? {
        "Block" if parts.len() == 4 && parts[2].list()?.is_empty() => constant_bool(&parts[3]),
        "Const" if parts.len() == 3 => {
            let value = parts[2].list()?;
            if value.len() != 3 || value[0].atom()? != "Constant" || value[1].atom()? != "Bool" {
                return None;
            }
            match value[2].atom()? {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}

fn callback_state(bodies: &BTreeMap<String, Vec<ir::Record>>) -> Result<Value> {
    let mut callbacks = Vec::new();
    for (owner, records) in bodies {
        let kinds = records
            .iter()
            .filter(|r| r["kind"] == "closure_kind")
            .collect::<Vec<_>>();
        let captures = records
            .iter()
            .filter(|r| r["kind"] == "capture")
            .collect::<Vec<_>>();
        if kinds.is_empty() && captures.is_empty() {
            if owner.contains("::{closure#") {
                callbacks.push(json!({"owner":owner,"status":"unavailable","reason":"Compiler closure capture metadata missing"}));
            }
            continue;
        }
        if kinds.len() != 1
            || captures
                .iter()
                .enumerate()
                .any(|(i, r)| r["index"].parse::<usize>() != Ok(i))
        {
            return Err("incomplete or duplicated compiler closure capture records".into());
        }
        let uses = records
            .iter()
            .filter(|r| r["kind"] == "capture_use")
            .collect::<Vec<_>>();
        if uses
            .iter()
            .any(|r| !captures.iter().any(|c| c["binding_id"] == r["binding_id"]))
        {
            return Err("capture source use has no compiler-resolved captured binding".into());
        }
        callbacks.push(json!({"owner":owner,"status":"recorded","closure_kind":kinds[0]["closure_kind"],"captures":captures,"source_uses":uses}));
    }
    Ok(
        json!({"callbacks":callbacks,"scope":"Compiler-resolved captures and closure kinds; state lifting and source equivalence have not been inferred from these facts."}),
    )
}

pub(crate) fn correspondence_report(
    before: &BTreeMap<String, Vec<ir::Record>>,
    after: &BTreeMap<String, Vec<ir::Record>>,
    paths: &BTreeSet<String>,
) -> Value {
    if !paths.iter().all(|p| before.contains_key(p)) || !before.keys().eq(after.keys()) {
        return json!({"matches":false,"reason":"missing or changed body identities"});
    }
    let mut checks = BTreeMap::new();
    for (path, records) in before {
        let converted_assertion = records.iter().any(|r| {
            r["kind"] == "assert_operand"
                && matches!(r["macro"].as_str(), "std::assert_eq" | "core::assert_eq")
        });
        let failures = after[path]
            .iter()
            .filter(|r| r["kind"] == "call")
            .filter_map(|r| r.get("target"))
            .filter(|target| {
                target.as_str() == "checked_eq_failure" || target.ends_with("::checked_eq_failure")
            })
            .collect::<BTreeSet<_>>();
        let failure = if converted_assertion && failures.len() == 1 {
            failures.first().map(|s| s.as_str())
        } else {
            None
        };
        let opaque_equal = match (
            ir::assertion_opaque_signature(
                records,
                failure.map(|_| "core::panicking::assert_failed"),
            ),
            ir::assertion_opaque_signature(&after[path], failure),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        let report = if !opaque_equal {
            json!({"matches":false,"reason":"literal or nominal operand identity changed"})
        } else if ir::compare(records, &after[path], failure).unwrap_or(false) {
            json!({"matches":true,"rule":if failure.is_some(){"pinned_mir_borrows_and_assertion_failure"}else{"pinned_mir_and_borrows"}})
        } else if failure.is_some() {
            json!({"matches":false,"reason":"assertion conversion changed runtime MIR or borrow events"})
        } else {
            match crate::return_forwarding::compare(records, &after[path]) {
                Ok(mut report) => {
                    report["rule"] = json!("immediate_return_forwarding");
                    report
                }
                Err(error) => json!({"matches":false,"reason":error.to_string()}),
            }
        };
        checks.insert(path, report);
    }
    json!({"matches":checks.values().all(|r|r["matches"]==true),"checks":checks,
        "scope":"Complete runtime MIR and borrow events, with recorded immediate-return forwarding when needed"})
}

fn correspondence_with_std(
    before: &BTreeMap<String, Vec<ir::Record>>,
    after: &BTreeMap<String, Vec<ir::Record>>,
    paths: &BTreeSet<String>,
    copies: &BTreeMap<String, Value>,
) -> Result<Value> {
    let mut original = before.clone();
    let mut generated = after.clone();
    for path in copies.keys() {
        original.insert(
            path.clone(),
            crate::std_body::correspondence_input(
                before.get(path).ok_or("missing std source body")?,
                false,
            )?,
        );
        generated.insert(
            path.clone(),
            crate::std_body::correspondence_input(
                after.get(path).ok_or("missing std generated body")?,
                true,
            )?,
        );
    }
    Ok(correspondence_report(&original, &generated, paths))
}

#[cfg(test)]
fn corresponding(
    before: &BTreeMap<String, Vec<ir::Record>>,
    after: &BTreeMap<String, Vec<ir::Record>>,
    paths: &BTreeSet<String>,
) -> Result<bool> {
    Ok(correspondence_report(before, after, paths)["matches"] == true)
}

fn invariant_catalogue(text: &str, crate_name: &str) -> Result<Vec<(String, String)>> {
    let functions = vir::functions(text)?;
    let mut result = Vec::new();
    for f in &functions {
        if f.field("attrs")
            .and_then(|a| a.field("is_type_invariant_fn"))
            .and_then(vir::Node::atom)
            != Some("true")
        {
            continue;
        }
        let Some(params) = f.field("params").and_then(vir::Node::list) else {
            continue;
        };
        let [param] = params else {
            continue;
        };
        let Some(typ) = param.field("typ").and_then(vir::Node::list) else {
            continue;
        };
        if typ.get(1).and_then(vir::Node::atom) != Some("Datatype") {
            continue;
        }
        let Some(path) = typ
            .get(2)
            .and_then(vir::Node::list)
            .and_then(|n| n.get(2))
            .and_then(vir::Node::atom)
        else {
            continue;
        };
        let Some(function) = f
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(vir::Node::atom)
        else {
            continue;
        };
        result.push((
            path.strip_prefix(&format!("{crate_name}::"))
                .unwrap_or(path)
                .to_owned(),
            function.to_owned(),
        ));
    }
    Ok(result)
}

fn invariant_facts(
    records: &[ir::Record],
    catalogue: &[(String, String)],
    cwd: &Path,
) -> Result<Vec<(PathBuf, ir::Edit, Value)>> {
    let mut result = Vec::new();
    for binding in records.iter().filter(|r| r["kind"] == "binding") {
        let matches = catalogue
            .iter()
            .filter(|(typ, _)| {
                binding["type"] == *typ || binding["type"].starts_with(&format!("{typ}<"))
            })
            .collect::<Vec<_>>();
        let [(typ, function)] = matches.as_slice() else {
            continue;
        };
        let name = &binding["name"];
        if !name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
            || name.is_empty()
            || name == "_"
        {
            continue;
        }
        let (file, coords) = ir::span(&binding["span"], cwd)?;
        let source = fs::read_to_string(&file)?;
        let (start, end) = ir::offsets(&source, coords)?;
        let line = source[..start].rfind('\n').map_or(0, |i| i + 1);
        if source[line..start].trim() != "let"
            || ![name.as_str(), &format!("mut {name}")].contains(&source[start..end].trim())
        {
            continue;
        }
        for call in records
            .iter()
            .filter(|r| r["kind"] == "call" && r["destination"] == binding["place"])
        {
            let Some(term) = records
                .iter()
                .find(|r| r["kind"] == "term" && r["block"] == call["block"])
            else {
                continue;
            };
            let (path, coords) = ir::span(&term["span"], cwd)?;
            if path != file {
                continue;
            }
            let (a, b) = ir::offsets(&source, coords)?;
            if a < end || source[end..a].trim() != "=" {
                continue;
            }
            let semi = b + source[b..].len() - source[b..].trim_start().len();
            if source.as_bytes().get(semi) != Some(&b';') {
                continue;
            }
            let edit = ir::Edit {
                start: semi + 1,
                end: semi + 1,
                replacement: format!(
                    "\n        vstd::prelude::proof! {{ vstd::prelude::use_type_invariant(&{name}); }}"
                ),
                origin: binding["span"].clone(),
            };
            result.push((file.clone(),edit,json!({"binding":name,"datatype":typ,"invariant":function,"initializer":call,"source":binding["span"],"new_entry_assumptions":0})));
        }
    }
    Ok(result)
}

pub fn run(input: &Path, config: &Path, sdk: &Path, out: &Path) -> Result<()> {
    let config: Config = serde_json::from_slice(&fs::read(config)?)?;
    if config.sources.iter().any(|p| {
        p.is_absolute()
            || p.components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
    }) {
        return Err("scope sources must be package-relative paths".into());
    }
    let prior: Value = serde_json::from_slice(&fs::read(input.join("report.json"))?)?;
    link::validate(Path::new(prior["links"].as_str().ok_or("missing links")?))?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    write_json(
        &out.join("scope.json"),
        &json!({"sources":config.sources,"input":input,"checker_sha256":hash(&fs::read(std::env::current_exe()?)?)?}),
    )?;
    let capture = Path::new(prior["capture"].as_str().ok_or("missing capture")?);
    let inputs: BTreeMap<PathBuf, String> =
        serde_json::from_slice(&fs::read(capture.join("inputs.json"))?)?;
    for tool in ["verus", "rust_verify", "cargo-verus"] {
        let digest = hash(&fs::read(sdk.join(tool))?)?;
        if !inputs
            .iter()
            .any(|(p, h)| p.file_name().is_some_and(|n| n == tool) && h == &digest)
        {
            return Err("SDK differs from the captured project toolchain".into());
        }
    }
    let mut results = Vec::new();
    let mut unresolved_sources = Vec::new();
    let mut registered = BTreeSet::new();
    for profile in prior["profiles"].as_array().ok_or("missing profiles")? {
        for unit in profile["units"].as_array().ok_or("missing units")? {
            let callers = Path::new(unit["callers"].as_str().ok_or("missing callers")?);
            let name = callers
                .parent()
                .and_then(Path::file_name)
                .ok_or("missing unit name")?;
            let original = capture
                .join(profile["profile"].as_str().ok_or("missing profile")?)
                .join(name);
            let inv: Invocation =
                serde_json::from_slice(&fs::read(original.join("context.json"))?)?;
            client::full_verification_context(&inv)?;
            let package = Path::new(inv.env.get("CARGO_MANIFEST_DIR").ok_or("missing package")?)
                .canonicalize()?;
            let callers: Vec<Value> = serde_json::from_slice(&fs::read(callers)?)?;
            let inventory: Value = serde_json::from_slice(&fs::read(
                original.join("compile/inventory-records.json"),
            )?)?;
            inventory["records"]
                .as_array()
                .ok_or("missing compiler records")?;
            let relation_path = Path::new(prior["links"].as_str().ok_or("missing links")?)
                .join(profile["profile"].as_str().ok_or("missing profile")?)
                .join(name)
                .join("relations.json");
            let relations: Vec<Value> = serde_json::from_slice(&fs::read(relation_path)?)?;
            let mut candidates = Vec::new();
            for c in &callers {
                let source = ir::span(
                    c["owner"]["span"].as_str().ok_or("missing owner span")?,
                    &inv.cwd,
                );
                let (source, _) = match source {
                    Ok(s) => s,
                    Err(e) => {
                        unresolved_sources
                            .push(json!({"caller":c["owner"],"reason":e.to_string()}));
                        continue;
                    }
                };
                let Ok(relative) = source.strip_prefix(&package) else {
                    continue;
                };
                if !config.sources.contains(&relative.to_owned()) {
                    continue;
                }
                registered.insert(relative.to_owned());
                if c["route"] == "requires_generated_client" {
                    candidates.push(c);
                } else {
                    let owner = c["owner"]["owner"]
                        .as_str()
                        .ok_or("missing owner identity")?;
                    results.push(json!({"caller":c["owner"],"status":"existing_route","route":c["route"],"success_demands":success_demands(&inventory,owner),"call_associations":call_associations(&relations,owner),"new_proof_coverage":false}));
                }
            }
            if candidates.is_empty() {
                continue;
            }
            let base = out.join(profile["profile"].as_str().unwrap()).join(name);
            fs::create_dir_all(&base)?;
            let probe = runner::build_probe(&inv, &base)?.ok_or("Rustc probe unavailable")?;
            let native: Value =
                serde_json::from_slice(&fs::read(original.join("verification-input.json"))?)?;
            let crate_name = inv
                .args
                .windows(2)
                .find(|w| w[0] == "--crate-name")
                .map(|w| w[1].as_str())
                .ok_or("missing crate name")?;
            let native_text =
                fs::read_to_string(native["vir"].as_str().ok_or("missing native VIR")?)?;
            let catalogue = invariant_catalogue(&native_text, crate_name)?;
            let permissions = crate::permission_guidance::catalogue(&native_text, crate_name)?;
            let native_functions = vir::functions(&native_text)?;
            write_json(
                &base.join("resource-uses.json"),
                &crate::permission_guidance::usage_inventory(
                    &permissions,
                    &native_functions,
                    &inventory,
                    &inv.cwd,
                )?,
            )?;
            write_json(
                &base.join("library-post-candidates.json"),
                &crate::owner_post::catalogue(&native_text, &inv.cwd)?,
            )?;
            for (i, caller) in candidates.iter().enumerate() {
                let case = base.join(format!("caller-{i}"));
                fs::create_dir(&case)?;
                let mut phase = "conversion";
                let mut work = || -> Result<Value> {
                    if !caller["owner"]["span"].as_str().unwrap().ends_with("(#0)") {
                        return Err(
                            "macro-generated source requires a separate conversion rule".into()
                        );
                    }
                    let selected = caller["owner"]["path"]
                        .as_str()
                        .ok_or("missing caller path")?;
                    let selection = case.join("selection.txt");
                    let paths = context_paths(selected, &callers, &inventory)?;
                    fs::write(&selection, selected)?;
                    let context_selection = case.join("context-selection.txt");
                    fs::write(
                        &context_selection,
                        paths.iter().cloned().collect::<Vec<_>>().join("\n"),
                    )?;
                    let mut context = inv.clone();
                    context.env.insert(
                        "CLIENT_PROJECT_BORROW_PATHS".into(),
                        context_selection.to_string_lossy().into_owned(),
                    );
                    context
                        .env
                        .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
                    context.env.insert("CLIENT_PROJECT_LINK".into(), "1".into());
                    context
                        .env
                        .insert("CLIENT_RESOLVE_INSTANCES".into(), "1".into());
                    let args =
                        runner::clean(&inv.args, &case.join("original-output"), None, &inv.cwd)?;
                    phase = "original_extraction";
                    runner::extract_context(&probe, &context, &args, "*", &case, "original", true)?;
                    let mut before =
                        client::borrow_records(&fs::read_to_string(case.join("original.stderr"))?)?;
                    let library_posts = crate::owner_post::called(
                        &before,
                        &native_functions,
                        &inventory,
                        &inv.cwd,
                    )?;
                    let mut correspondence_paths = paths.clone();
                    for (owner, _) in &library_posts {
                        correspondence_paths.insert(owner.clone());
                    }
                    if !library_posts.is_empty() {
                        fs::write(
                            &context_selection,
                            correspondence_paths
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )?;
                        runner::extract_context(
                            &probe,
                            &context,
                            &args,
                            "*",
                            &case,
                            "library-original",
                            true,
                        )?;
                        before = client::borrow_records(&fs::read_to_string(
                            case.join("library-original.stderr"),
                        )?)?;
                    }
                    write_json(&case.join("callback-state.json"), &callback_state(&before)?)?;
                    let proposals = match crate::callback::propose(&before, &inv.cwd) {
                        Ok(proposals) => {
                            let mut reports = Vec::new();
                            for (index, proposal) in proposals.into_iter().enumerate() {
                                let source = case.join(format!("callback-candidate-{index}.rs"));
                                fs::write(&source, proposal.source)?;
                                reports.push(json!({"source":source,"derivation":proposal.report}));
                            }
                            json!({"status":"candidates_only","proposals":reports,"scope":"One mutable whole-variable capture in a direct standard for_each callback; correspondence is pending and the original client remains the checked input."})
                        }
                        Err(error) => json!({"status":"unsupported","reason":error.to_string()}),
                    };
                    write_json(&case.join("callback-proposals.json"), &proposals)?;
                    let records = before
                        .get(selected)
                        .ok_or("selected caller was not extracted")?;
                    write_json(
                        &case.join("call-value-flow.json"),
                        &direct_call_flows(records)?,
                    )?;
                    let resource_flow = crate::resource_flow::audit(
                        records,
                        &native_functions,
                        &permissions,
                        &inv.cwd,
                    )?;
                    write_json(&case.join("resource-flow.json"), &resource_flow)?;
                    phase = "conversion";
                    let (file, _, _) = ir::lower_client(records, &inv.cwd)?;
                    let mut files = BTreeMap::<PathBuf, Vec<ir::Edit>>::new();
                    for (_, candidate) in &library_posts {
                        files
                            .entry(candidate.file.clone())
                            .or_default()
                            .extend(candidate.edits.clone());
                    }
                    write_json(
                        &case.join("library-posts.json"),
                        &library_posts
                            .iter()
                            .map(|(owner, c)| json!({"compiler_owner":owner,"derivation":c.report}))
                            .collect::<Vec<_>>(),
                    )?;
                    let mut std_copies = BTreeMap::new();
                    let mut filter_report = BTreeMap::new();
                    let mut invariant_report = Vec::new();
                    let mut permission_report = Vec::new();
                    for path in &paths {
                        let body = before.get(path).ok_or("local helper was not extracted")?;
                        let (file, _, edits) = ir::lower_client(body, &inv.cwd)?;
                        files.entry(file).or_default().extend(edits);
                        if let Some(copy) = crate::std_body::propose(body, &inv.cwd)? {
                            files.entry(copy.file).or_default().extend(copy.edits);
                            std_copies.insert(path.clone(), copy.evidence);
                        }
                        if let Some(guidance) = crate::filter_guidance::propose(body, &inv.cwd)? {
                            files
                                .entry(guidance.file)
                                .or_default()
                                .extend(guidance.edits);
                            filter_report.insert(path.clone(), guidance.evidence);
                        }
                        for (file, edit, report) in invariant_facts(body, &catalogue, &inv.cwd)? {
                            files.entry(file).or_default().push(edit);
                            invariant_report.push(report);
                        }
                        match crate::permission_guidance::propose(body, &permissions, &inv.cwd, path) {
                            Ok((extra,reports)) => {
                                for (file,edit) in extra {files.entry(file).or_default().push(edit);}
                                permission_report.extend(reports);
                            }
                            Err(error) => permission_report.push(json!({"status":"unsupported","body":path,"reason":error.to_string()})),
                        }
                    }
                    write_json(&case.join("invariant-facts.json"), &invariant_report)?;
                    write_json(&case.join("permission-guidance.json"), &permission_report)?;
                    write_json(&case.join("std-bodies.json"), &std_copies)?;
                    write_json(&case.join("filter-guidance.json"), &filter_report)?;
                    for body in before.values() {
                        for datatype in body
                            .iter()
                            .filter(|r| r["kind"] == "datatype" && r["verified"] == "false")
                        {
                            if !datatype["span"].ends_with("(#0)") {
                                return Err(
                                    "macro-generated datatype requires its own conversion".into()
                                );
                            }
                            let (file, coords) = ir::span(&datatype["span"], &inv.cwd)?;
                            let (start, _) = ir::offsets(&fs::read_to_string(&file)?, coords)?;
                            files.entry(file).or_default().push(ir::Edit {
                                start,
                                end: start,
                                replacement: "#[vstd::prelude::verus_verify]\n".into(),
                                origin: datatype["span"].clone(),
                            });
                        }
                    }
                    for (nested, body) in &before {
                        if !nested.contains("::{closure#")
                            || !body.iter().any(|r| r["kind"] == "assert_operand")
                        {
                            continue;
                        }
                        if let Some(parent) = paths
                            .iter()
                            .filter(|p| nested.starts_with(&format!("{p}::")))
                            .max_by_key(|p| p.len())
                        {
                            let (file, _, edits) =
                                ir::lower_closure_assertions(body, &before[parent], &inv.cwd)?;
                            files.entry(file).or_default().extend(edits);
                        }
                    }
                    let edits = files.get_mut(&file).ok_or("missing primary source edits")?;
                    let guidance = if records
                        .iter()
                        .any(|r| r["kind"] == "source_expr" && r["expression_kind"] == "loop")
                    {
                        let native: Value = serde_json::from_slice(&fs::read(
                            original.join("verification-input.json"),
                        )?)?;
                        let text = fs::read_to_string(
                            native["vir"].as_str().ok_or("missing captured VIR")?,
                        )?;
                        match crate::guidance::propose(records, &text, &inv.cwd, &package) {
                            Ok((extra, report)) => {
                                edits.extend(extra);
                                edits.sort_by_key(|e| e.start);
                                if edits.windows(2).any(|w| w[0].end > w[1].start) {
                                    return Err("guidance overlaps client conversion".into());
                                }
                                report
                            }
                            Err(e) => json!({"status":"unsupported","reason":e.to_string()}),
                        }
                    } else {
                        json!({"status":"not_needed"})
                    };
                    write_json(&case.join("loop-guidance.json"), &guidance)?;
                    let snapshot = case.join("package");
                    runner::copy_package(&package, &snapshot)?;
                    for (file, edits) in &mut files {
                        edits.sort_by(|a, b| {
                            (a.start, a.end, &a.replacement).cmp(&(b.start, b.end, &b.replacement))
                        });
                        edits.dedup_by(|a, b| {
                            a.start == b.start && a.end == b.end && a.replacement == b.replacement
                        });
                        if edits.windows(2).any(|w| w[0].end > w[1].start) {
                            return Err("overlapping context edits".into());
                        }
                        let mut source = fs::read_to_string(file)?;
                        for e in edits.iter().rev() {
                            source.replace_range(e.start..e.end, &e.replacement);
                        }
                        fs::write(snapshot.join(file.strip_prefix(&package)?), source)?;
                    }
                    write_json(&case.join("edits.json"), &files[&file])?;
                    write_json(&case.join("context-edits.json"), &files)?;
                    let roots = inv
                        .args
                        .iter()
                        .map(|a| inv.cwd.join(a))
                        .filter(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
                        .collect::<Vec<_>>();
                    let [root] = roots.as_slice() else {
                        return Err("single crate root required".into());
                    };
                    let root = root.canonicalize()?;
                    let copied = snapshot.join(root.strip_prefix(&package)?);
                    let mut generated_args = runner::clean(
                        &inv.args,
                        &case.join("generated-output"),
                        Some((&root, &copied)),
                        &inv.cwd,
                    )?;
                    if permission_report
                        .iter()
                        .any(|r| r["status"] == "generated_candidate_requires_verification")
                    {
                        generated_args.push("-Zcrate-attr=feature(panic_internals)".into());
                    }
                    phase = "generated_extraction";
                    runner::extract_context(
                        &probe,
                        &context,
                        &generated_args,
                        "*",
                        &case,
                        "generated",
                        true,
                    )?;
                    let after = client::borrow_records(&fs::read_to_string(
                        case.join("generated.stderr"),
                    )?)?;
                    let mut correspondence = correspondence_with_std(
                        &before,
                        &after,
                        &correspondence_paths,
                        &std_copies,
                    )?;
                    correspondence["std_body_copies"] = json!(case.join("std-bodies.json"));
                    let matches = correspondence["matches"] == true;
                    write_json(&case.join("correspondence.json"), &correspondence)?;
                    if !matches {
                        return Err("ordinary client correspondence failed".into());
                    }
                    phase = "resource_consumer_verification";
                    write_json(
                        &case.join("resource-consumers.json"),
                        &crate::resource_flow::verify_consumers(
                            &resource_flow,
                            &native_functions,
                            &permissions,
                            &inv,
                            &generated_args,
                            sdk,
                            &case,
                        )?,
                    )?;
                    for (i, (_, candidate)) in library_posts.iter().enumerate() {
                        let mut library_args = generated_args.clone();
                        let module = candidate.report["module"]
                            .as_str()
                            .ok_or("missing library module")?
                            .split("::")
                            .skip(1)
                            .collect::<Vec<_>>()
                            .join("::");
                        if module.is_empty() {
                            library_args.push("--verify-root".into());
                        } else {
                            library_args.extend(["--verify-only-module".into(), module]);
                        }
                        library_args.extend([
                            "--verify-function".into(),
                            candidate.report["selector"]
                                .as_str()
                                .ok_or("missing library selector")?
                                .into(),
                        ]);
                        phase = "library_summary_verification";
                        let checked = runner::prove(
                            sdk,
                            &inv,
                            &library_args,
                            &case,
                            &format!("library-post-{i}"),
                        )?;
                        if checked["status"] != "proved" || checked["summary"] != json!([1, 0]) {
                            return Err("generated library guarantee did not verify".into());
                        }
                    }
                    let trace = fs::read_to_string(case.join("generated.stderr"))?;
                    let entries = trace
                        .lines()
                        .map(|s| s.split('\t').collect::<Vec<_>>())
                        .collect::<Vec<_>>();
                    let mut identities = BTreeMap::new();
                    let mut modules = BTreeSet::new();
                    let mut owning_modules = BTreeMap::new();
                    for path in &paths {
                        let owners = entries
                            .iter()
                            .filter(|r| {
                                r.first() == Some(&"PROJECT_OWNER")
                                    && r.get(2) == Some(&path.as_str())
                                    && matches!(r.get(3), Some(&"Fn" | &"AssocFn"))
                            })
                            .collect::<Vec<_>>();
                        let [owner] = owners.as_slice() else {
                            return Err("fresh caller identity missing or ambiguous".into());
                        };
                        let native = entries
                            .iter()
                            .filter(|r| {
                                r.first() == Some(&"LINK_OWNER") && r.get(1) == owner.get(1)
                            })
                            .collect::<Vec<_>>();
                        let [native] = native.as_slice() else {
                            return Err("fresh native identity missing or ambiguous".into());
                        };
                        let identity = *native.get(2).ok_or("missing native identity")?;
                        let owners = entries
                            .iter()
                            .filter(|r| {
                                r.first() == Some(&"LINK_MODULE") && r.get(1) == owner.get(1)
                            })
                            .collect::<Vec<_>>();
                        let [module] = owners.as_slice() else {
                            return Err("fresh owning module missing or ambiguous".into());
                        };
                        let module = *module.get(2).ok_or("missing owning module")?;
                        owning_modules.insert(identity, module);
                        modules.insert(module.split("::").skip(1).collect::<Vec<_>>().join("::"));
                        identities.insert(path.clone(), identity);
                    }
                    let identity = identities[selected];
                    let mut args = generated_args;
                    for module in modules {
                        if module.is_empty() {
                            args.push("--verify-root".into());
                        } else {
                            args.extend(["--verify-only-module".into(), module]);
                        }
                    }
                    phase = "verification";
                    crate::std_body::validate(&std_copies, &package, &snapshot)?;
                    if !std_copies.is_empty() {
                        let mut helper_args = args.clone();
                        helper_args
                            .extend(["--verify-function".into(), crate::std_body::HELPER.into()]);
                        let checked =
                            runner::prove(sdk, &inv, &helper_args, &case, "std-body-check")?;
                        if checked["status"] != "proved" || checked["summary"] != json!([1, 0]) {
                            return Err(
                                "copied standard-library implementation did not verify".into()
                            );
                        }
                    }
                    let verification = runner::prove(sdk, &inv, &args, &case, "check")?;
                    if !case.join("check-vir/crate.vir").is_file()
                        || !matches!(
                            verification["status"].as_str(),
                            Some("proved" | "not_proved")
                        )
                        || verification["summary"]
                            .as_array()
                            .is_none_or(|s| s.iter().filter_map(Value::as_u64).sum::<u64>() == 0)
                    {
                        return Ok(
                            json!({"status":"backend_not_analyzed","limitations":backend_limitations(&verification),"verification":verification,"new_proof_coverage":false,"correspondence":case.join("correspondence.json")}),
                        );
                    }
                    let text = fs::read_to_string(case.join("check-vir/crate.vir"))?;
                    let records = before.values().flatten().cloned().collect::<Vec<_>>();
                    let quality = crate::review::summarize_context(
                        &verification,
                        &text,
                        &inv.cwd,
                        (&package, &snapshot),
                        &files,
                        &records,
                    )?;
                    write_json(&case.join("failure-evidence.json"), &quality)?;
                    let functions = vir::functions(&text)?;
                    let contract_index = ContractIndex::new(&functions);
                    let mut calls = Vec::new();
                    let mut demand_sites = Vec::new();
                    for native in identities.values() {
                        let targets = functions
                            .iter()
                            .filter(|f| {
                                f.field("name")
                                    .and_then(|n| n.field("path"))
                                    .and_then(vir::Node::atom)
                                    == Some(*native)
                            })
                            .collect::<Vec<_>>();
                        let [target] = targets.as_slice() else {
                            return Err("generated verification target missing or ambiguous".into());
                        };
                        if target
                            .field("owning_module")
                            .and_then(vir::Node::atom)
                            .map(|p| p.trim_end_matches("::"))
                            != owning_modules
                                .get(*native)
                                .copied()
                                .map(|p| p.trim_end_matches("::"))
                        {
                            return Err("VIR owning module differs from compiler module".into());
                        }
                        if target
                            .field("attrs")
                            .and_then(|a| a.field("is_external_body"))
                            .and_then(vir::Node::atom)
                            != Some("false")
                            || target.field("body").is_none_or(|b| b.list().is_none())
                        {
                            return Err("generated target has no checked body".into());
                        }
                        check_entry_contract(&functions, native)?;
                        for mut demand in crate::demand::body_demands(
                            target.field("body").unwrap(),
                            &contract_index,
                        ) {
                            demand["checked_function"] = json!(native);
                            demand["original_source"] = crate::demand::source_location(
                                demand["span"].as_str(),
                                &package,
                                &snapshot,
                                &files,
                            );
                            demand_sites.push(demand);
                        }
                        vir::walk_calls(target.field("body").unwrap(), &mut calls);
                    }
                    let call_targets = calls
                        .iter()
                        .map(|p| call_contract(&functions, p))
                        .collect::<Vec<_>>();
                    let source_assertions = before
                        .iter()
                        .filter(|(owner, _)| {
                            paths
                                .iter()
                                .any(|p| *owner == p || owner.starts_with(&format!("{p}::")))
                        })
                        .flat_map(|(_, body)| body)
                        .filter(|r| r["kind"] == "assert_operand")
                        .map(|r| r["assertion_span"].clone())
                        .collect::<BTreeSet<_>>();
                    let explicit_targets = source_assertions.len()
                        + call_targets
                            .iter()
                            .filter_map(|c| c["requires"].as_array())
                            .map(Vec::len)
                            .sum::<usize>();
                    write_json(
                        &case.join("demand-sites.json"),
                        &json!({
                            "sites": demand_sites,
                        "span_origin": "generated snapshot; original_source is mapped only through unchanged source segments",
                            "path_context": "syntactic nesting, not a proved path condition",
                            "verification": "the whole caller is checked; no per-site proof result is inferred from a whole-body result"
                        }),
                    )?;
                    Ok(
                        json!({"status":"whole_body_checked","verification":verification,"target":identity,"checked_context":identities,"verification_modules":owning_modules,"call_targets":call_targets,"demand_sites":case.join("demand-sites.json"),"source_assertions":source_assertions,"explicit_target_count":explicit_targets,"entry_conditions":[],"new_entry_assumptions":0,"loop_guidance":guidance,"invariant_facts":case.join("invariant-facts.json"),"permission_guidance":case.join("permission-guidance.json"),"correspondence":case.join("correspondence.json"),"edits":case.join("edits.json"),"quality_evidence":case.join("failure-evidence.json"),"callback_state":case.join("callback-state.json"),"filter_guidance":case.join("filter-guidance.json"),"new_body_verification":true,"new_proof_coverage":explicit_targets>0,"resource_flow":case.join("resource-flow.json"),"resource_consumers":case.join("resource-consumers.json"),"library_guarantees":case.join("library-posts.json"),"contract_context":if library_posts.is_empty(){"original"}else{"original_with_implementation_verified_summaries"},"scope":"Whole ordinary caller body; library summary proofs and module verification retained separately; zero explicit targets do not count as new property coverage"}),
                    )
                };
                let result = match work() {
                    Ok(v) => v,
                    Err(e) => {
                        json!({"status":"not_completed","phase":phase,"error_kind":if e.is::<std::io::Error>(){"execution_failed"}else{"unsupported"},"reason":e.to_string(),"new_proof_coverage":false})
                    }
                };
                let owner = caller["owner"]["owner"]
                    .as_str()
                    .ok_or("missing owner identity")?;
                results.push(json!({"caller":caller["owner"],"result":result,"success_demands":success_demands(&inventory,owner),"call_associations":call_associations(&relations,owner),"evidence":case,"call_value_flow":if case.join("call-value-flow.json").is_file(){Some(case.join("call-value-flow.json"))}else{None},"resource_flow":if case.join("resource-flow.json").is_file(){Some(case.join("resource-flow.json"))}else{None},"permission_guidance":if case.join("permission-guidance.json").is_file(){Some(case.join("permission-guidance.json"))}else{None},"callback_state":if case.join("callback-state.json").is_file(){Some(case.join("callback-state.json"))}else{None},"callback_proposals":if case.join("callback-proposals.json").is_file(){Some(case.join("callback-proposals.json"))}else{None},"filter_guidance":if case.join("filter-guidance.json").is_file(){Some(case.join("filter-guidance.json"))}else{None}}));
            }
        }
    }
    let missing = config
        .sources
        .iter()
        .filter(|p| !registered.contains(*p))
        .collect::<Vec<_>>();
    link::validate(Path::new(prior["links"].as_str().unwrap()))?;
    write_json(
        &out.join("report.json"),
        &json!({"results":results,"unregistered_sources":missing,"unresolved_source_owners":unresolved_sources,"scope":"Project source scope only; no client-specific hints or entry requires accepted"}),
    )
}

#[cfg(test)]
mod controls {
    use super::*;
    #[test]
    #[ignore = "requires GOAL3_CALLER_RECORDS from actual Rustc extraction"]
    fn real_direct_call_flow_rejects_intervening_write() {
        let bodies = client::borrow_records(
            &fs::read_to_string(std::env::var_os("GOAL3_CALLER_RECORDS").unwrap()).unwrap(),
        )
        .unwrap();
        let mut body = bodies["task::scheduler::LocalRunQueue::pick_next"].clone();
        let report = direct_call_flows(&body).unwrap();
        let flow = report["flows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["consumer"]["target"] == "core::option::Option::<T>::unwrap")
            .unwrap();
        assert_eq!(
            flow["producer"]["target"],
            "task::scheduler::LocalRunQueue::try_pick_next"
        );
        assert_eq!(flow["argument_indices"], json!([0]));
        body.push(BTreeMap::from([
            ("kind".into(), "stmt".into()),
            (
                "block".into(),
                flow["consumer"]["block"].as_str().unwrap().into(),
            ),
            (
                "operation".into(),
                format!("{} = const None", flow["place"].as_str().unwrap()),
            ),
        ]));
        assert!(
            direct_call_flows(&body).unwrap()["flows"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn backend_limitations_keep_unknown_errors_and_do_not_call_them_defects() {
        let verification = json!({"diagnostics":[
            {"level":"error","message":"trait example::T not declared to Verus","spans":[]},
            {"level":"error","message":"The verifier does not yet support the following Rust feature: unsizing","spans":[]},
            {"level":"error","message":"something else","spans":[]},
            {"level":"error","message":"aborting due to 3 previous errors","spans":[]}
        ]});
        let limitations = backend_limitations(&verification);
        assert_eq!(limitations.len(), 3);
        assert_eq!(limitations[0]["kind"], "undeclared_trait");
        assert_eq!(limitations[1]["kind"], "unsupported_rust_feature");
        assert_eq!(limitations[2]["kind"], "other_backend_error");
    }
    #[test]
    #[ignore = "requires GOAL3_BACKEND_RESULT from the real scheduler check"]
    fn real_backend_errors_are_separated() {
        let verification: Value = serde_json::from_slice(
            &fs::read(std::env::var_os("GOAL3_BACKEND_RESULT").unwrap()).unwrap(),
        )
        .unwrap();
        let limitations = backend_limitations(&verification);
        assert!(limitations.iter().any(|x| x["kind"] == "undeclared_trait"));
        assert!(limitations.iter().any(|x| x["kind"] == "unsupported_api"));
    }
    #[test]
    fn call_associations_keep_missing_contracts_and_trait_requirements_separate() {
        let relations = vec![
            json!({"call":{"owner":"a","span":"client.rs:1","target":"api"},"association":{"status":"linked","contract":{"function_span":"api.rs:1","requires":[{"vir":"P"}]},"trait_contracts":[{"function_span":"trait.rs:1","requires":[{"vir":"Q"}]}]}}),
            json!({"call":{"owner":"a","span":"client.rs:2","target":"unknown"},"association":{"status":"no_matching_interface"}}),
            json!({"call":{"owner":"b","target":"other"},"association":{"status":"linked"}}),
        ];
        let rows = call_associations(&relations, "a");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["requirements"][0]["requires"][0]["vir"], "P");
        assert_eq!(rows[0]["requirements"][1]["requires"][0]["vir"], "Q");
        assert_eq!(rows[1]["association_status"], "no_matching_interface");
        assert!(rows[1]["requirements"].is_null());
    }
    #[test]
    #[ignore = "requires GOAL3_RELATIONS from the real scheduler capture"]
    fn real_call_associations_preserve_linked_and_unresolved_targets() {
        let relations: Vec<Value> = serde_json::from_slice(
            &fs::read(std::env::var_os("GOAL3_RELATIONS").unwrap()).unwrap(),
        )
        .unwrap();
        let owner = relations
            .iter()
            .find_map(|r| {
                let owner = r["call"]["owner"].as_str()?;
                owner.contains("LocalRunQueue::pick_next)").then_some(owner)
            })
            .unwrap();
        let rows = call_associations(&relations, owner);
        assert!(rows.iter().any(|r| {
            r["compiler_target"] == "core::option::Option::<T>::unwrap"
                && r["association_status"] == "linked"
                && r["requirements"][0]["requires"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
        }));
        assert!(rows.iter().any(|r| r["compiler_target"]
            == "task::scheduler::LocalRunQueue::try_pick_next"
            && r["association_status"] == "unresolved_compiler_target"
            && r["requirements"].is_null()));
    }
    #[test]
    fn success_demand_requires_resolved_standard_target() {
        let rows = json!({"records":[
            {"kind":"mir_call","owner":"a","target":"core::result::Result::<T, E>::expect","target_id":"DefId(1:4 ~ core[abc]::result::expect)","span":"file.rs:2:1: 2:8 (#0)","block":"bb2"},
            {"kind":"mir_call","owner":"a","target":"core::option::Option::<T>::unwrap","target_id":"DefId(1:5 ~ core[abc]::option::unwrap)","span":"file.rs:3:1: 3:8 (#0)","block":"bb3"},
            {"kind":"mir_call","owner":"a","target":"core::option::Option::<T>::unwrap","target_id":"DefId(0:5 ~ client[abc]::option::unwrap)","span":"file.rs:4:1: 4:8 (#0)"},
            {"kind":"mir_call","owner":"other","target":"core::option::Option::<T>::expect","target_id":"DefId(1:5 ~ core[abc]::option::expect)"}
        ]});
        let demands = success_demands(&rows, "a");
        assert_eq!(demands.len(), 2);
        assert_eq!(demands[0]["required_variant"], "Ok");
        assert_eq!(demands[1]["required_variant"], "Some");
        assert_eq!(demands[0]["status"], "discovered_not_independently_checked");
    }
    #[test]
    #[ignore = "requires GOAL3_INVENTORY from a real Vostd compiler capture"]
    fn real_expect_is_discovered_in_compiler_inventory() {
        let inventory: Value = serde_json::from_slice(
            &fs::read(std::env::var_os("GOAL3_INVENTORY").unwrap()).unwrap(),
        )
        .unwrap();
        let record = inventory["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["kind"] == "mir_call"
                    && r["target"] == "core::result::Result::<T, E>::expect"
                    && r["span"]
                        .as_str()
                        .is_some_and(|s| s.contains("page_table/node/mod.rs:278:"))
            })
            .unwrap();
        let demands = success_demands(&inventory, record["owner"].as_str().unwrap());
        assert!(
            demands
                .iter()
                .any(|d| d["source_span"] == record["span"] && d["required_variant"] == "Ok")
        );
    }
    #[test]
    fn compiler_capture_facts_keep_mutable_and_owned_state_distinct() {
        let data: Value = serde_json::from_str(include_str!(
            "../assets/experiments/test-spec/goal31-capture-facts.json"
        ))
        .unwrap();
        let mut bodies =
            client::borrow_records(data["cases"]["controls"]["records"].as_str().unwrap()).unwrap();
        let report = callback_state(&bodies).unwrap();
        let callbacks = report["callbacks"].as_array().unwrap();
        assert_eq!(callbacks.len(), 4);
        for (name, kind, capture) in [
            ("shared", "Fn", Some("ByRef(Immutable)")),
            ("borrowed_state", "FnMut", Some("ByRef(Mutable)")),
            ("owned_state", "FnMut", Some("ByValue")),
            ("explicit_state", "Fn", None),
        ] {
            let row = callbacks
                .iter()
                .find(|r| {
                    r["owner"]
                        .as_str()
                        .unwrap()
                        .starts_with(&format!("{name}::"))
                })
                .unwrap();
            assert_eq!(row["closure_kind"], kind);
            assert_eq!(
                row["captures"]
                    .as_array()
                    .unwrap()
                    .first()
                    .map(|c| c["capture_kind"].as_str().unwrap()),
                capture
            );
        }
        let range = callback_state(
            &client::borrow_records(data["cases"]["range"]["records"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(range["callbacks"][0]["captures"][0]["binding"], "res");
        assert_eq!(
            range["callbacks"][0]["captures"][0]["capture_kind"],
            "ByRef(Mutable)"
        );
        assert_eq!(
            range["callbacks"][0]["captures"][0]["binding_id"],
            range["callbacks"][0]["source_uses"][0]["binding_id"]
        );
        assert_eq!(
            range["callbacks"][0]["captures"][0]["projection_count"],
            "0"
        );
        let records = bodies.get_mut("shared::{closure#0}").unwrap();
        let duplicate = records
            .iter()
            .find(|r| r["kind"] == "capture")
            .unwrap()
            .clone();
        records.push(duplicate);
        assert!(callback_state(&bodies).is_err());
    }

    #[test]
    #[ignore = "requires CLOSURE_CORRESPONDENCE_CASE from the actual closure control"]
    fn actual_closure_assertion_keeps_values_and_failure_effects() {
        let case = PathBuf::from(std::env::var("CLOSURE_CORRESPONDENCE_CASE").unwrap());
        let before =
            client::borrow_records(&fs::read_to_string(case.join("original.stderr")).unwrap())
                .unwrap();
        let mut after =
            client::borrow_records(&fs::read_to_string(case.join("generated.stderr")).unwrap())
                .unwrap();
        let paths = BTreeSet::from(["closure_control".to_owned()]);
        assert_eq!(
            correspondence_report(&before, &after, &paths)["matches"],
            true
        );
        let nested = after.get_mut("closure_control::{closure#0}").unwrap();
        let operation = nested
            .iter_mut()
            .find(|r| r.get("operation").is_some_and(|v| v.contains(" = Eq(")))
            .expect("actual closure comparison");
        operation.insert(
            "operation".into(),
            operation["operation"].replace(" = Eq(", " = Ne("),
        );
        assert_eq!(
            correspondence_report(&before, &after, &paths)["matches"],
            false
        );
        let mut after =
            client::borrow_records(&fs::read_to_string(case.join("generated.stderr")).unwrap())
                .unwrap();
        let capture = after
            .get_mut("closure_control")
            .unwrap()
            .iter_mut()
            .find(|r| {
                r.get("operation")
                    .is_some_and(|v| v.contains("const 4_usize"))
            })
            .unwrap();
        capture.insert(
            "operation".into(),
            capture["operation"].replace("const 4_usize", "const 5_usize"),
        );
        assert_eq!(
            correspondence_report(&before, &after, &paths)["matches"],
            false
        );
    }

    #[test]
    #[ignore = "requires POST_PREDICATE_ORIGINAL and POST_PREDICATE_CANDIDATE VIR from the real Either relation controls"]
    fn real_trait_postconditions_resolve_constant_predicates() {
        let original = vir::functions(
            &fs::read_to_string(std::env::var("POST_PREDICATE_ORIGINAL").unwrap()).unwrap(),
        )
        .unwrap();
        let candidate = vir::functions(
            &fs::read_to_string(std::env::var("POST_PREDICATE_CANDIDATE").unwrap()).unwrap(),
        )
        .unwrap();
        for method in ["raw_as_ref", "ref_as_raw"] {
            let path = format!("ostd::sync::rcu::non_null::either::impl&%1::{method}");
            let before = call_contract(&original, &path);
            let signals = before["constant_postcondition_predicates"]
                .as_array()
                .unwrap();
            assert_eq!(signals.len(), 1);
            assert_eq!(signals[0]["value"], true);
            assert_eq!(
                signals[0]["resolved_to"],
                "ostd::sync::rcu::non_null::either::impl&%1::ref_rel_perm"
            );
            let after = call_contract(&candidate, &path);
            assert!(
                after["constant_postcondition_predicates"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn bookkeeping_renaming_cannot_hide_literal_changes() {
        let before = ir::parse("CLIENT_LOCAL\t_0\t()\nCLIENT_LOCAL\t_1\t()\nCLIENT_LOCAL\t_2\t&str\nCLIENT_STMT\tbb0\t0\tunused\t_2 = const \"_2\"\n").unwrap();
        let after = ir::parse("CLIENT_LOCAL\t_0\t()\nCLIENT_LOCAL\t_1\t&str\nCLIENT_STMT\tbb0\t0\tunused\t_1 = const \"_1\"\n").unwrap();
        assert!(ir::compare(&before, &after, None).unwrap());
        let paths = BTreeSet::from(["client".to_owned()]);
        assert!(
            !corresponding(
                &BTreeMap::from([("client".into(), before)]),
                &BTreeMap::from([("client".into(), after)]),
                &paths
            )
            .unwrap()
        );
    }
    #[test]
    fn invariant_catalogue_uses_verifier_metadata() {
        let text = "(Function :name (Fun :path pkg::predicate) :attrs (Attrs :is_type_invariant_fn true) :params ((Param :typ (Typ Datatype (Path :path pkg::M))))) (Function :name (Fun :path pkg::type_inv) :attrs (Attrs :is_type_invariant_fn false) :params ((Param :typ (Typ Datatype (Path :path pkg::Other)))))";
        assert_eq!(
            invariant_catalogue(text, "pkg").unwrap(),
            vec![("M".to_owned(), "pkg::predicate".to_owned())]
        );
    }
    #[test]
    fn invariant_guidance_requires_local_initialization() {
        let path = std::env::temp_dir().join(format!("invariant-local-{}.rs", std::process::id()));
        fs::write(
            &path,
            "fn f(mut x: M) {\n    x = make();\n    let y = make();\n}\n",
        )
        .unwrap();
        let records = ir::parse(&format!(
            "CLIENT_BINDING\tx\t_1\t{0}:1:6: 1:11 (#0)\tM\nCLIENT_BINDING\ty\t_2\t{0}:3:9: 3:10 (#0)\tM\nCLIENT_CALL\tbb0\tmake\tunused\t[]\t_1\t[]\nCLIENT_TERM\tbb0\t{0}:2:9: 2:15 (#0)\tcall\nCLIENT_CALL\tbb1\tmake\tunused\t[]\t_2\t[]\nCLIENT_TERM\tbb1\t{0}:3:13: 3:19 (#0)\tcall\n", path.display()
        )).unwrap();
        let facts = invariant_facts(
            &records,
            &[("M".into(), "pkg::predicate".into())],
            Path::new("/"),
        )
        .unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].2["binding"], "y");
        assert_eq!(facts[0].2["new_entry_assumptions"], 0);
    }
    #[test]
    fn inherited_requirements_and_missing_helpers_are_visible() {
        let functions = vir::functions("(Function :name (Fun :path Impl::f) :kind (FunctionKind TraitMethodImpl :method (Fun :path Trait::f)) :require () :ensure (tuple (local_guarantee) ())) (Function :name (Fun :path Trait::f) :kind (FunctionKind TraitMethodDecl) :require (condition) :ensure (tuple (trait_guarantee) ()))").unwrap();
        let contract = call_contract(&functions, "Impl::f");
        assert_eq!(contract["requires"], json!(["\"condition\""]));
        assert_eq!(contract["require_origins"][1]["function"], "Trait::f");
        assert!(check_entry_contract(&functions, "Impl::f").is_err());
        assert!(check_entry_contract(&functions, "Missing::f").is_err());
        let entry_free = vir::functions("(Function :name (Fun :path free) :kind (FunctionKind Static) :require () :ensure (tuple () ()))").unwrap();
        assert!(check_entry_contract(&entry_free, "free").is_ok());
        assert_eq!(
            contract["ensures"],
            json!(["\"local_guarantee\"", "\"trait_guarantee\""])
        );
        assert!(call_contract(&functions, "Missing::f")["requires"].is_null());
        let before = BTreeMap::from([("client".into(), vec![]), ("helper".into(), vec![])]);
        let paths = before.keys().cloned().collect();
        assert!(corresponding(&before, &before, &paths).unwrap());
        let mut after = before.clone();
        after.remove("helper");
        assert!(!corresponding(&before, &after, &paths).unwrap());
    }
    #[test]
    fn returns_guarantee_is_preserved_separately_from_ensures() {
        let functions = vir::functions("(Function :name (Fun :path Lib::len) :kind (FunctionKind Static) :require () :ensure (tuple () ()) :returns (Some exact_length))").unwrap();
        let contract = call_contract(&functions, "Lib::len");
        assert_eq!(contract["ensures"], json!([]));
        assert_eq!(contract["returns"], json!(["(\"Some\" \"exact_length\")"]));
    }
    #[test]
    #[ignore = "requires ORDINARY_REPLAY_PLAN and ORDINARY_REPLAY_OUT; reuses typed compiler records"]
    fn replay_typed_correspondence() {
        let plan: Value = serde_json::from_slice(
            &fs::read(std::env::var_os("ORDINARY_REPLAY_PLAN").unwrap()).unwrap(),
        )
        .unwrap();
        let mut reports = Vec::new();
        for row in plan.as_array().unwrap() {
            let before = Path::new(row["before"].as_str().unwrap());
            let after = Path::new(row["after"].as_str().unwrap());
            let original = client::borrow_records(&fs::read_to_string(before).unwrap()).unwrap();
            let generated = client::borrow_records(&fs::read_to_string(after).unwrap()).unwrap();
            let paths = fs::read_to_string(row["selection"].as_str().unwrap())
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect();
            let report = correspondence_report(&original, &generated, &paths);
            assert_eq!(
                report["matches"], row["expected"],
                "{}: {report}",
                row["name"]
            );
            reports.push(json!({"input":row,"before_sha256":hash(&fs::read(before).unwrap()).unwrap(),"after_sha256":hash(&fs::read(after).unwrap()).unwrap(),"report":report}));
        }
        write_json(
            Path::new(&std::env::var_os("ORDINARY_REPLAY_OUT").unwrap()),
            &reports,
        )
        .unwrap();
    }
    #[test]
    #[ignore = "requires ORDINARY_CASE, ORDINARY_CONTEXT, ORDINARY_CONTROL and ORDINARY_MUTATION"]
    fn typed_omissions_and_changed_arguments_are_rejected() {
        let case = PathBuf::from(std::env::var_os("ORDINARY_CASE").unwrap());
        let out = PathBuf::from(std::env::var_os("ORDINARY_CONTROL").unwrap());
        let inv: Invocation = serde_json::from_slice(
            &fs::read(std::env::var_os("ORDINARY_CONTEXT").unwrap()).unwrap(),
        )
        .unwrap();
        let mutation = std::env::var("ORDINARY_MUTATION").unwrap();
        let guided_body = if mutation.starts_with("resource_guidance") {
            fs::read_to_string(std::env::var_os("ORDINARY_GUIDED_BODY").unwrap()).unwrap()
        } else {
            String::new()
        };
        fs::create_dir(&out).unwrap();
        let selected = fs::read_to_string(case.join("selection.txt")).unwrap();
        let selection = if case.join("context-selection.txt").is_file() {
            case.join("context-selection.txt")
        } else {
            case.join("selection.txt")
        };
        let paths = fs::read_to_string(&selection)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        let original_records = if case.join("library-original.stderr").is_file() {
            case.join("library-original.stderr")
        } else {
            case.join("original.stderr")
        };
        let before =
            client::borrow_records(&fs::read_to_string(original_records).unwrap()).unwrap();
        let generated =
            client::borrow_records(&fs::read_to_string(case.join("generated.stderr")).unwrap())
                .unwrap();
        let copies: BTreeMap<String, Value> = if case.join("std-bodies.json").exists() {
            serde_json::from_slice(&fs::read(case.join("std-bodies.json")).unwrap()).unwrap()
        } else {
            BTreeMap::new()
        };
        assert_eq!(
            correspondence_with_std(&before, &generated, &paths, &copies).unwrap()["matches"],
            true
        );
        let records = &generated[&selected];
        let (span, replacement) = match mutation.as_str() {
            "optional_unwrap" => {
                let call = records
                    .iter()
                    .find(|r| r["kind"] == "call" && r["target"].ends_with("::unwrap"))
                    .unwrap();
                (
                    records
                        .iter()
                        .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                        .unwrap()["span"]
                        .clone(),
                    "match self.try_pick_next() { Some(value) => value, None => panic!() }",
                )
            }
            "optional_branch" => (
                records
                    .iter()
                    .find(|r| r["kind"] == "source_expr" && r["expression_kind"] == "if")
                    .unwrap()["span"]
                    .clone(),
                "None",
            ),
            "std_default" | "std_callback" | "std_call" => {
                let call = records
                    .iter()
                    .find(|r| r["kind"] == "call" && r["target"].ends_with(crate::std_body::HELPER))
                    .unwrap();
                let span = &records
                    .iter()
                    .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                    .unwrap()["span"];
                if mutation == "std_call" {
                    (span.clone(), "0usize")
                } else {
                    let index = if mutation == "std_default" { "1" } else { "2" };
                    let arg = records
                        .iter()
                        .find(|r| {
                            r["kind"] == "source_argument"
                                && r["call_span"] == *span
                                && r["index"] == index
                        })
                        .unwrap();
                    (
                        arg["span"].clone(),
                        if mutation == "std_default" {
                            "0usize"
                        } else {
                            "|i| i + 1"
                        },
                    )
                }
            }
            "optional_result" => {
                let call = records
                    .iter()
                    .find(|r| r["kind"] == "call" && r["target"].ends_with("::try_pick_next"))
                    .unwrap();
                (
                    records
                        .iter()
                        .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                        .unwrap()["span"]
                        .clone(),
                    "None",
                )
            }
            "resource_guidance" | "resource_guidance_changed" => (
                records.iter().find(|r| r["kind"] == "body").unwrap()["span"].clone(),
                guided_body.as_str(),
            ),
            "argument" => (
                records
                    .iter()
                    .find(|r| {
                        r["kind"] == "source_argument"
                            && matches!(r["type"].as_str(), "u32" | "usize")
                    })
                    .unwrap()["span"]
                    .clone(),
                "0",
            ),
            "assertion" => (
                records
                    .iter()
                    .find(|r| r["kind"] == "assert_operand")
                    .unwrap()["assertion_span"]
                    .clone(),
                "()",
            ),
            "false_assertion" => (
                records
                    .iter()
                    .find(|r| r["kind"] == "assert_operand")
                    .unwrap()["span"]
                    .clone(),
                "false",
            ),
            "path" => (
                records
                    .iter()
                    .find(|r| r["kind"] == "source_expr" && r["expression_kind"] == "loop")
                    .unwrap()["span"]
                    .clone(),
                "{}",
            ),
            "branch" => (
                records
                    .iter()
                    .find(|r| {
                        r["kind"] == "source_expr"
                            && r["expression_kind"] == "if"
                            && r["snippet"].starts_with("Some(\"if ")
                    })
                    .unwrap()["span"]
                    .clone(),
                "{}",
            ),
            "mutation_call" | "resource_release" => {
                let suffix = if mutation == "resource_release" {
                    "::drop"
                } else {
                    "::set"
                };
                let call = records
                    .iter()
                    .find(|r| r["kind"] == "call" && r["target"].ends_with(suffix))
                    .unwrap();
                (
                    records
                        .iter()
                        .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                        .unwrap()["span"]
                        .clone(),
                    "()",
                )
            }
            "helper" => (
                generated
                    .iter()
                    .find(|(p, _)| *p != &selected && paths.contains(*p))
                    .unwrap()
                    .1
                    .iter()
                    .find(|r| r["kind"] == "body")
                    .unwrap()["span"]
                    .clone(),
                "{ false }",
            ),
            "call" => {
                let assertion = records
                    .iter()
                    .find(|r| r["kind"] == "assert_operand")
                    .unwrap();
                let (path, coords) = ir::span(&assertion["span"], &inv.cwd).unwrap();
                let text = fs::read_to_string(&path).unwrap();
                let (a, b) = ir::offsets(&text, coords).unwrap();
                let call = records
                    .iter()
                    .find(|r| {
                        if r["kind"] != "source_expr" || r["expression_kind"] != "method_call" {
                            return false;
                        }
                        let (p, c) = ir::span(&r["span"], &inv.cwd).unwrap();
                        let (s, e) = ir::offsets(&text, c).unwrap();
                        p == path && a <= s && e <= b
                    })
                    .unwrap();
                (call["span"].clone(), "0usize")
            }
            _ => panic!("unknown mutation"),
        };
        let package = case.join("package");
        let snapshot = out.join("package");
        runner::copy_package(&package, &snapshot).unwrap();
        let (file, coords) = ir::span(&span, &inv.cwd).unwrap();
        let file = file.canonicalize().unwrap();
        let mut source = fs::read_to_string(&file).unwrap();
        let (a, b) = ir::offsets(&source, coords).unwrap();
        source.replace_range(a..b, replacement);
        fs::write(snapshot.join(file.strip_prefix(&package).unwrap()), source).unwrap();
        let command: Value =
            serde_json::from_slice(&fs::read(case.join("generated.command.json")).unwrap())
                .unwrap();
        let roots = inv
            .args
            .iter()
            .map(|p| inv.cwd.join(p))
            .filter(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
            .collect::<Vec<_>>();
        let [root] = roots.as_slice() else {
            panic!("expected one generated root");
        };
        let original_package = Path::new(inv.env.get("CARGO_MANIFEST_DIR").unwrap());
        let copied = snapshot.join(root.strip_prefix(original_package).unwrap());
        let mut args = runner::clean(
            &inv.args,
            &out.join("metadata"),
            Some((root, &copied)),
            &inv.cwd,
        )
        .unwrap();
        if let Ok(feature) = std::env::var("ORDINARY_CONTROL_FEATURE") {
            assert!(feature.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            args.push(format!("-Zcrate-attr=feature({feature})"));
        }
        let mut context = inv.clone();
        context.env.insert(
            "CLIENT_PROJECT_BORROW_PATHS".into(),
            selection.to_string_lossy().into_owned(),
        );
        context
            .env
            .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
        let probe = Path::new(command["program"].as_str().unwrap());
        runner::extract_context(probe, &context, &args, "*", &out, "mutated", true).unwrap();
        let changed =
            client::borrow_records(&fs::read_to_string(out.join("mutated.stderr")).unwrap())
                .unwrap();
        let report = correspondence_with_std(&before, &changed, &paths, &copies).unwrap();
        write_json(&out.join("correspondence.json"), &report).unwrap();
        let comparison: Result<bool> = Ok(report["matches"] == true);
        let rejected = !matches!(comparison, Ok(true));
        let rejection = match comparison {
            Ok(true) if mutation == "resource_guidance" => "none".to_owned(),
            Ok(false) => "records differ".to_owned(),
            Err(error) => error.to_string(),
            Ok(true) => panic!("changed client passed correspondence"),
        };
        if mutation == "resource_guidance" {
            write_json(
                &out.join("guidance-comparison.json"),
                &json!({"correspondence":!rejected,"reason":rejection,"automatic_guidance":false}),
            )
            .unwrap();
            write_json(
                &out.join("before-normalized.json"),
                &ir::normalize(&before[&selected], None).unwrap(),
            )
            .unwrap();
            write_json(
                &out.join("after-normalized.json"),
                &ir::normalize(&changed[&selected], None).unwrap(),
            )
            .unwrap();
            assert!(
                !rejected,
                "proof guidance changed runtime records: {rejection}"
            );
        }
        let caller_unchanged =
            ir::compare(&before[&selected], &changed[&selected], None).unwrap_or(false);
        if mutation == "helper" {
            assert!(caller_unchanged);
        }
        let mut native = Value::Null;
        if mutation == "false_assertion" {
            let sdk = PathBuf::from(std::env::var_os("ORDINARY_SDK").unwrap());
            let mut proof_args = args.clone();
            let proof: Value =
                serde_json::from_slice(&fs::read(case.join("check.command.json")).unwrap())
                    .unwrap();
            let original = proof["args"].as_array().unwrap();
            for (i, arg) in original.iter().enumerate() {
                if arg == "--verify-root" {
                    proof_args.push("--verify-root".into());
                }
                if arg == "--verify-only-module" {
                    proof_args.extend([
                        "--verify-only-module".into(),
                        original[i + 1].as_str().unwrap().into(),
                    ]);
                }
            }
            native = runner::prove(&sdk, &inv, &proof_args, &out, "counter-check").unwrap();
            assert_eq!(native["status"], "not_proved");
            assert!(native["summary"][1].as_u64().unwrap() > 0);
        }
        write_json(&out.join("result.json"),&json!({"mutation":mutation,"original_correspondence":true,"mutant_typechecked":true,"correspondence_rejected":rejected,"rejection":rejection,"caller_mir_unchanged":caller_unchanged,"runtime_executed":false,"native_countercheck":native})).unwrap();
    }
}
