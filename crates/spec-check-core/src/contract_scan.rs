//! Inventory constant spec predicates and their actual VIR contract uses.
use crate::{Result, contracts, hash, ordinary::ContractIndex, vir, write_json};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn path(function: &vir::Node) -> Option<&str> {
    function.field("name")?.field("path")?.atom()
}

fn span(node: &vir::Node) -> Option<&str> {
    let parts = node.list()?;
    matches!(parts.first()?.atom(), Some("@" | "@@"))
        .then(|| parts.get(1)?.atom())
        .flatten()
}

fn calls(
    node: &vir::Node,
    location: Option<&str>,
    result: &mut BTreeSet<(String, Option<String>)>,
) {
    let location = span(node).or(location);
    if let Some(parts) = node.list() {
        if parts.first().and_then(vir::Node::atom) == Some("CallTarget") {
            let mut targets = Vec::new();
            vir::walk_calls(node, &mut targets);
            for target in targets {
                result.insert((target, location.map(str::to_owned)));
            }
        } else {
            for part in parts {
                calls(part, location, result);
            }
        }
    }
}

// Binder identities include Rustc disambiguators; names alone cannot resolve shadowing.
#[derive(Default)]
struct Reads(BTreeMap<String, BTreeSet<(bool, String)>>);
impl Reads {
    fn walk(
        &mut self,
        node: &vir::Node,
        old: bool,
        location: Option<&str>,
    ) -> std::result::Result<(), String> {
        let location = span(node).or(location);
        let Some(parts) = node.unwrap().list() else {
            return Ok(());
        };
        let tag = parts.first().and_then(vir::Node::atom);
        let op = parts.get(1).and_then(vir::Node::atom);
        if tag == Some(">") && op == Some("Old") {
            for child in &parts[2..] {
                self.walk(child, true, location)?;
            }
            return Ok(());
        }
        if tag == Some(">") && op == Some("VarAt") {
            let Some(at) = parts.get(3).and_then(vir::Node::list) else {
                return Err("unrecognized VarAt state".into());
            };
            if at.first().and_then(vir::Node::atom) != Some("VarAt")
                || at.get(1).and_then(vir::Node::atom) != Some("Pre")
            {
                return Err("unsupported VarAt state".into());
            }
            if let Some(var) = parts.get(2) {
                self.insert(var, true, location);
            }
            return Ok(());
        }
        if tag == Some(">") && op == Some("Unary") {
            let operator = parts.get(2).and_then(vir::Node::list).unwrap_or_default();
            if operator.get(1).and_then(vir::Node::atom) == Some("MutRefFuture") {
                let source = operator
                    .get(2)
                    .and_then(vir::Node::list)
                    .unwrap_or_default();
                if source.get(1).and_then(vir::Node::atom) != Some("Final") {
                    return Err("unsupported mutable-reference future state".into());
                }
                for child in &parts[3..] {
                    self.walk(child, false, location)?;
                }
                return Ok(());
            }
        }
        if (tag == Some("Place") && op == Some("Local")) || (tag == Some(">") && op == Some("Var"))
        {
            if let Some(var) = parts.get(2) {
                self.insert(var, old, location);
            }
            return Ok(());
        }
        for child in parts {
            self.walk(child, old, location)?;
        }
        Ok(())
    }
    fn insert(&mut self, var: &vir::Node, old: bool, location: Option<&str>) {
        if var.list().and_then(|v| v.first()).and_then(vir::Node::atom) == Some("VarIdent") {
            self.0
                .entry(contracts::render(var))
                .or_default()
                .insert((old, location.unwrap_or("span unavailable").to_owned()));
        }
    }
    fn references(&self, param: &vir::Node, source: &str, section: &str) -> Vec<Value> {
        let mut names = param.field("name").into_iter().collect::<Vec<_>>();
        if let Some(alias) = param
            .field("unwrapped_info")
            .and_then(vir::Node::list)
            .and_then(|v| v.get(2))
        {
            names.push(alias);
        }
        names.into_iter().filter_map(|n|self.0.get(&contracts::render(n))).flatten().map(|(old,location)|json!({"contract_source":source,"section":section,"state":if *old {"pre"}else{"post_or_current"},"span":location})).collect()
    }
}

fn mutable_reference(typ: &vir::Node) -> bool {
    let Some(parts) = typ.unwrap().list() else {
        return false;
    };
    match parts.get(1).and_then(vir::Node::atom) {
        Some("MutRef") => true,
        Some("Decorate") => parts.last().is_some_and(mutable_reference),
        _ => false,
    }
}

fn return_shape(typ: &vir::Node) -> &'static str {
    let Some(parts) = typ.unwrap().list() else {
        return "unknown";
    };
    match parts.get(1).and_then(vir::Node::atom) {
        Some("TypParam" | "Projection") => "unknown_generic",
        Some("Never") => "diverging",
        Some("Datatype")
            if parts.get(2).and_then(vir::Node::list).is_some_and(|v| {
                v.len() == 3
                    && v[0].atom() == Some("Dt")
                    && v[1].atom() == Some("Tuple")
                    && v[2].atom() == Some("0")
            }) =>
        {
            "unit"
        }
        Some(
            "Datatype" | "Int" | "Bool" | "MutRef" | "Decorate" | "Primitive" | "FnDef" | "Lambda",
        ) => "non_unit",
        _ => "unknown",
    }
}

fn coverage(function: &vir::Node, index: &ContractIndex<'_>, contract: &Value) -> Value {
    let fail = |reason: &str| json!({"status":"unavailable","reason":reason});
    if !matches!(
        function.field("mode").and_then(vir::Node::atom),
        Some("Exec" | "Proof")
    ) {
        return json!({"status":"not_applicable","reason":"Spec definitions supply their body as a logical definition."});
    }
    if function
        .field("item_kind")
        .and_then(vir::Node::list)
        .and_then(|v| v.get(1))
        .and_then(vir::Node::atom)
        .is_some_and(|kind| kind != "Function")
    {
        return json!({"status":"not_applicable","reason":"Constant/static initializer rather than a function item."});
    }
    let Some(params) = function.field("params").and_then(vir::Node::list) else {
        return fail("parameter binders unavailable");
    };
    let Some(ret) = function.field("inner_ret") else {
        return fail("return binder unavailable");
    };
    if params.iter().chain(std::iter::once(ret)).any(|p| {
        p.field("name")
            .and_then(vir::Node::list)
            .and_then(|v| v.first())
            .and_then(vir::Node::atom)
            != Some("VarIdent")
    }) {
        return fail("parameter or return variable identity unavailable");
    }
    let Some(ret_type) = ret.field("typ") else {
        return fail("return type unavailable");
    };
    let Some(origins) = contract["require_origins"].as_array() else {
        return fail("inherited contract unavailable or ambiguous");
    };
    let mut parameter_refs = vec![Vec::new(); params.len()];
    let mut return_refs = Vec::new();
    let mut returns_sources = Vec::new();
    let mut entry_false = false;
    for origin in origins {
        let Some(source) = origin["function"].as_str() else {
            return fail("contract source identity unavailable");
        };
        let Some(definition) = index.lookup(source) else {
            return fail("contract source missing or ambiguous");
        };
        let Some(source_params) = definition.field("params").and_then(vir::Node::list) else {
            return fail("inherited parameter binders unavailable");
        };
        if source_params.len() != params.len() {
            return fail("inherited parameter positions do not correspond");
        };
        let Some(source_ret) = definition.field("inner_ret") else {
            return fail("inherited return binder unavailable");
        };
        let Some(ensures) = definition.field("ensure").and_then(vir::Node::list) else {
            return fail("ensures representation unavailable");
        };
        if ensures.len() != 3
            || ensures[0].atom() != Some("tuple")
            || ensures[1].list().is_none()
            || ensures[2].list().is_none()
        {
            return fail("unsupported ensures representation");
        };
        for section in ["require", "ensure", "returns"] {
            let Some(expression) = definition.field(section) else {
                return fail("contract expression unavailable");
            };
            if section == "returns" && expression.atom() == Some("None") {
                continue;
            }
            if section == "returns" {
                returns_sources.push(source.to_owned());
            }
            if section == "require" {
                entry_false |= expression
                    .list()
                    .into_iter()
                    .flatten()
                    .any(|node| index_bool(node) == Some(false));
            }
            let mut reads = Reads::default();
            // The tuple contains regular and trait-default ensures; both are contracts.
            if let Err(reason) = reads.walk(expression, section == "require", None) {
                return fail(&reason);
            }
            for (i, param) in source_params.iter().enumerate() {
                parameter_refs[i].extend(reads.references(param, source, section));
            }
            return_refs.extend(reads.references(source_ret, source, section));
            if let Some(outer) = definition.field("outer_ret")
                && outer.field("name") != source_ret.field("name")
            {
                return_refs.extend(reads.references(outer, source, section));
            }
        }
    }
    let mut signals = Vec::new();
    let mut parameters = Vec::new();
    for (param, references) in params.iter().zip(parameter_refs) {
        let Some(typ) = param.field("typ") else {
            return fail("parameter type unavailable");
        };
        let mut_ref = mutable_reference(typ);
        let post = references
            .iter()
            .any(|r| r["section"] != "require" && r["state"] != "pre");
        let pre_only = !post && references.iter().any(|r| r["section"] != "require");
        let name = param.field("name").map(contracts::render);
        if mut_ref && !post && !entry_false {
            signals.push(json!({"kind":"mutable_final_state_unmentioned","parameter":name,"parameter_span":span(param),"post_mentions_old_state_only":pre_only,"return_alias_analysis":"Not inferred; returned references and implicit type/resource invariants may supply other guarantees."}));
        }
        parameters.push(json!({"name":name,"span":span(param),"mutable_reference":if return_shape(typ)=="unknown_generic" {None}else{Some(mut_ref)},"type":contracts::render(typ),"post_state_occurrence":post,"references":references}));
    }
    let shape = return_shape(ret_type);
    let mentioned =
        !returns_sources.is_empty() || return_refs.iter().any(|r| r["section"] == "ensure");
    if shape == "non_unit" && !mentioned && !entry_false {
        signals.push(json!({"kind":"return_value_unmentioned","return_span":span(ret)}));
    }
    json!({"status":"recorded","scope":"Syntactic variable occurrences using exact VIR binders and positional trait inheritance. Predicate arguments count as occurrences; no guarantee strength or alias/frame completeness is inferred.","literal_false_precondition":entry_false,"signals_suppressed":if entry_false{Some("Literal false entry condition; unavailable entry is recorded separately from omission candidates.")}else{None},"parameters":parameters,"return":{"shape":shape,"type":contracts::render(ret_type),"postcondition_occurrence":mentioned,"returns_sources":returns_sources,"references":return_refs},"signals":signals})
}

fn index_bool(node: &vir::Node) -> Option<bool> {
    let values = node.unwrap().list()?;
    if values.len() != 3 || values[0].atom() != Some(">") || values[1].atom() != Some("Const") {
        return None;
    }
    let value = values[2].list()?;
    if value.len() != 3 || value[0].atom() != Some("Constant") || value[1].atom() != Some("Bool") {
        return None;
    }
    match value[2].atom()? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn scan(functions: &[vir::Node]) -> Value {
    let index = ContractIndex::new(functions);
    let mut definitions = BTreeMap::new();
    let mut uses = BTreeMap::<String, Vec<Value>>::new();
    let mut inventory = Vec::new();
    for function in functions {
        let Some(name) = path(function) else {
            inventory.push(json!({"status":"unavailable","reason":"VIR function has no definition path","span":span(function)}));
            continue;
        };
        if let Some(mut predicate) = index.constant_predicate(name, None) {
            predicate["definition_span"] = json!(span(function));
            definitions.insert(name.to_owned(), predicate);
        }
        for section in ["require", "ensure", "returns", "body"] {
            if let Some(expression) = function.field(section) {
                let mut found = BTreeSet::new();
                calls(expression, None, &mut found);
                for (target, call_span) in found {
                    uses.entry(target).or_default().push(json!({"function":name,"function_span":span(function),"section":section,"call_span":call_span}));
                }
            }
        }
        let contract = index.call_contract(name);
        let parameter_coverage = coverage(function, &index, &contract);
        inventory.push(json!({"function":name,"span":span(function),"mode":function.field("mode").and_then(vir::Node::atom),"contract_status":if contract["requires"].is_array(){"recorded"}else{"unavailable"},"reason":contract["reason"],"contract_sources":contract["require_origins"].as_array().map(|xs|xs.iter().map(|x|x["function"].clone()).collect::<Vec<_>>()),"constant_postcondition_predicates":contract["constant_postcondition_predicates"],"parameter_coverage":parameter_coverage,"external_body":function.field("attrs").and_then(|a|a.field("is_external_body")).and_then(vir::Node::atom),"item_kind":function.field("item_kind").map(contracts::render)}));
        for signal in contract["constant_postcondition_predicates"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(resolved) = signal["resolved_to"].as_str() else {
                continue;
            };
            uses.entry(resolved.to_owned()).or_default().push(json!({"function":name,"function_span":span(function),"section":"inherited_or_direct_postcondition","predicate":signal["predicate"],"contract_source":signal["contract_source"]}));
        }
    }
    let mut candidates = Vec::new();
    let mut constants = Vec::new();
    for (name, mut predicate) in definitions {
        let references = uses.remove(&name).unwrap_or_default();
        let contract_uses = references
            .iter()
            .filter(|r| r["section"] != "body")
            .cloned()
            .collect::<Vec<_>>();
        predicate["uses"] = json!(references);
        predicate["contract_uses"] = json!(contract_uses);
        if !contract_uses.is_empty() {
            candidates.push(predicate.clone());
        }
        constants.push(predicate);
    }
    let unavailable = inventory
        .iter()
        .filter(|f| f["contract_status"] == "unavailable" || f["status"] == "unavailable")
        .count();
    let source_spans = inventory
        .iter()
        .filter_map(|f| {
            Some((
                f["function"].as_str()?.to_owned(),
                f["span"].as_str()?.to_owned(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    for function in &mut inventory {
        let related = function["function"]
            .as_str()
            .and_then(|p| p.rsplit_once("::"))
            .and_then(|(owner, name)| {
                Some(format!(
                    "{owner}::{}",
                    name.strip_prefix("_VERUS_VERIFIED_")?
                ))
            })
            .filter(|p| {
                source_spans
                    .get(p)
                    .is_some_and(|s| function["span"].as_str() == Some(s.as_str()))
            });
        function["same_span_auxiliary_for"] = json!(related);
    }
    let mut coverage_summary = BTreeMap::<String, usize>::new();
    let mut omissions = Vec::new();
    for function in &inventory {
        let coverage = &function["parameter_coverage"];
        *coverage_summary
            .entry(
                coverage["status"]
                    .as_str()
                    .unwrap_or("unavailable")
                    .to_owned(),
            )
            .or_default() += 1;
        for signal in coverage["signals"].as_array().into_iter().flatten() {
            let mut signal = signal.clone();
            signal["function"] = function["function"].clone();
            signal["span"] = function["span"].clone();
            signal["same_span_auxiliary_for"] = function["same_span_auxiliary_for"].clone();
            signal["external_body"] = function["external_body"].clone();
            omissions.push(signal);
        }
    }
    let mut omission_summary = BTreeMap::<String, usize>::new();
    for signal in &omissions {
        *omission_summary
            .entry(signal["kind"].as_str().unwrap_or("unknown").to_owned())
            .or_default() += 1;
    }
    json!({"schema":2,"report_complete":true,"omission_summary":omission_summary,"coverage_summary":coverage_summary,"omission_candidates":omissions,"coverage_limits":"Occurrences are syntactic signals. Generic return shapes, unavailable inherited binders, implicit type invariants and returned-reference aliases retain their stated limitations. No omission is classified as a defect.","scope":"Every Function item in the supplied typed VIR; constant bool bodies and empty blocks only. Uses are VIR calls, including inherited trait postconditions resolved in their implementation context. No source build or solver is run.","assessment":"Review signals requiring a concrete client need and independent verification before any defect conclusion.","functions_total":functions.len(),"contracts_unavailable":unavailable,"constant_predicates":constants,"review_candidates":candidates,"functions":inventory})
}

pub fn run(input: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("contract-scan output directory already exists".into());
    }
    let input = input.canonicalize()?;
    let text = fs::read_to_string(&input)?;
    let functions = vir::functions(&text)?;
    if functions.is_empty() {
        return Err("input contains no typed VIR Function items".into());
    }
    let mut report = scan(&functions);
    report["input"] = json!({"path":input,"sha256":hash(text.as_bytes())?,"provenance":"Caller-supplied VIR; correspondence with a source revision/build has not been established by this command."});
    report["checker_sha256"] = json!(hash(&fs::read(std::env::current_exe()?)?)?);
    fs::create_dir_all(out)?;
    write_json(&out.join("report.json"), &report)?;
    let mut lines = vec![
        format!(
            "# Constant contract predicate review\n\n{} functions; {} constant predicates; {} review candidates; {} unavailable contracts.\n",
            functions.len(),
            report["constant_predicates"].as_array().map_or(0, Vec::len),
            report["review_candidates"].as_array().map_or(0, Vec::len),
            report["contracts_unavailable"]
        ),
        report["assessment"].as_str().unwrap_or_default().to_owned(),
    ];
    lines.push(format!("\nParameter/return occurrence coverage: {}. Omission signals: {}. Full per-function references and inherited contract sources are in report.json. These syntactic signals do not establish missing semantic guarantees.", report["coverage_summary"], report["omission_summary"]));
    for candidate in report["review_candidates"].as_array().into_iter().flatten() {
        lines.push(format!(
            "\n- `{}` = `{}` ({})",
            candidate["resolved_to"].as_str().unwrap_or_default(),
            candidate["value"],
            candidate["span"].as_str().unwrap_or("span unavailable")
        ));
        for usage in candidate["contract_uses"].as_array().into_iter().flatten() {
            lines.push(format!(
                "  - `{}` — {}",
                usage["function"].as_str().unwrap_or_default(),
                usage["section"].as_str().unwrap_or_default()
            ));
        }
    }
    fs::write(out.join("README.md"), lines.join("\n"))?;
    println!(
        "{} functions; {} constant-predicate review candidates; {} unavailable contracts; {} omission signals",
        functions.len(),
        report["review_candidates"].as_array().map_or(0, Vec::len),
        report["contracts_unavailable"],
        report["omission_candidates"].as_array().map_or(0, Vec::len)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage_fixture(
        name: &str,
        parameter_id: usize,
        return_id: usize,
        ensure: &str,
        returns: &str,
        kind: &str,
        return_type: &str,
    ) -> String {
        format!(
            r#"(Function :name (Fun :path {name}) :kind {kind} :mode Exec
          :params ((Param :name (VarIdent "x" (VarIdentDisambiguate RustcId {parameter_id})) :typ (Typ MutRef (Typ Int (IntRange USize)))))
          :inner_ret (Param :name (VarIdent "result" (VarIdentDisambiguate RustcId {return_id})) :typ {return_type})
          :require () :ensure (tuple ({ensure}) ()) :returns {returns})"#
        )
    }

    #[test]
    fn final_state_occurrences_distinguish_old_shadowing_and_helper_arguments() {
        let local =
            r#"(> ReadPlace (Place Local (VarIdent "x" (VarIdentDisambiguate RustcId 1))))"#;
        for (ensure, expected) in [
            (format!("(> Old {local})"), true),
            (local.replace("RustcId 1", "RustcId 9"), true),
            (format!("(> Call :args ({local}))"), false),
            (
                format!("(> Unary (UnaryOp MutRefFuture (MutRefFutureSourceName Final)) {local})"),
                false,
            ),
        ] {
            let text = coverage_fixture(
                "p::f",
                1,
                2,
                &ensure,
                "None",
                "(FunctionKind Static)",
                "(Typ Datatype (Dt Tuple 0) () ())",
            );
            let functions = vir::functions(&text).unwrap();
            let report = scan(&functions);
            assert_eq!(
                report["omission_candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["kind"] == "mutable_final_state_unmentioned"),
                expected,
                "{ensure}"
            );
        }
    }

    #[test]
    fn returns_and_generic_shapes_do_not_become_missing_return_reports() {
        for (ensure, returns, typ, expected) in [
            ("", "None", "(Typ Int (IntRange USize))", true),
            (
                r#"(> Var (VarIdent "result" (VarIdentDisambiguate RustcId 2)))"#,
                "None",
                "(Typ Int (IntRange USize))",
                false,
            ),
            (
                "",
                "(> Const (Constant Int 4))",
                "(Typ Int (IntRange USize))",
                false,
            ),
            ("", "None", r#"(Typ TypParam "T")"#, false),
        ] {
            let text =
                coverage_fixture("p::f", 1, 2, ensure, returns, "(FunctionKind Static)", typ);
            let report = scan(&vir::functions(&text).unwrap());
            assert_eq!(
                report["omission_candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["kind"] == "return_value_unmentioned"),
                expected
            );
        }
    }

    #[test]
    fn trait_default_ensures_are_included() {
        let text = coverage_fixture(
            "p::f",
            1,
            2,
            "",
            "None",
            "(FunctionKind Static)",
            "(Typ Int (IntRange USize))",
        )
        .replace(
            ":ensure (tuple () ())",
            r#":ensure (tuple () ((> Var (VarIdent "result" (VarIdentDisambiguate RustcId 2)))))"#,
        );
        let report = scan(&vir::functions(&text).unwrap());
        assert_eq!(
            report["functions"][0]["parameter_coverage"]["return"]["postcondition_occurrence"],
            true
        );
        assert!(
            !report["omission_candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["kind"] == "return_value_unmentioned")
        );
    }

    #[test]
    fn inherited_parameter_and_result_binders_are_mapped_by_position() {
        let declaration = coverage_fixture(
            "p::T::f",
            21,
            22,
            r#"(> Call :args ((> ReadPlace (Place Local (VarIdent "x" (VarIdentDisambiguate RustcId 21)))) (> Var (VarIdent "result" (VarIdentDisambiguate RustcId 22)))))"#,
            "None",
            "(FunctionKind TraitMethodDecl)",
            "(Typ Int (IntRange USize))",
        );
        let implementation = coverage_fixture(
            "p::impl::f",
            1,
            2,
            "",
            "None",
            "(FunctionKind TraitMethodImpl :method (Fun :path p::T::f))",
            "(Typ Int (IntRange USize))",
        );
        let report = scan(&vir::functions(&(declaration + &implementation)).unwrap());
        assert_eq!(report["coverage_summary"]["recorded"], 2);
        assert!(report["omission_candidates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn unsupported_temporal_reads_are_retained_and_false_entries_are_suppressed() {
        let text = coverage_fixture(
            "p::f",
            1,
            2,
            r#"(> VarAt (VarIdent "x" (VarIdentDisambiguate RustcId 1)) (VarAt Unknown))"#,
            "None",
            "(FunctionKind Static)",
            "(Typ Int (IntRange USize))",
        );
        let report = scan(&vir::functions(&text).unwrap());
        assert_eq!(report["coverage_summary"]["unavailable"], 1);
        let text = coverage_fixture(
            "p::f",
            1,
            2,
            "",
            "None",
            "(FunctionKind Static)",
            "(Typ Int (IntRange USize))",
        )
        .replace(":require ()", ":require ((> Const (Constant Bool false)))");
        let report = scan(&vir::functions(&text).unwrap());
        assert_eq!(
            report["functions"][0]["parameter_coverage"]["literal_false_precondition"],
            true
        );
        assert!(report["omission_candidates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn resolves_trait_contracts_and_retains_body_only_constants() {
        let functions = vir::functions(r#"
(Function :name (Fun :path p::T::relation) :kind (FunctionKind TraitMethodDecl) :mode Spec :body None :attrs (Attrs :is_external_body false))
(Function :name (Fun :path p::T::borrow) :kind (FunctionKind TraitMethodDecl) :ensure (CallTarget Static () (Fun :path p::T::relation)))
(Function :name (Fun :path p::impl::relation) :kind (FunctionKind TraitMethodImpl :trait_path p::T :impl_path p::impl :method (Fun :path p::T::relation)) :mode Spec :attrs (Attrs :is_external_body false) :body (@ "file.rs:1" (> Const (Constant Bool true))))
(Function :name (Fun :path p::impl::borrow) :kind (FunctionKind TraitMethodImpl :trait_path p::T :impl_path p::impl :method (Fun :path p::T::borrow)))
(Function :name (Fun :path p::never) :mode Spec :kind (FunctionKind Static) :attrs (Attrs :is_external_body false) :body (@ "file.rs:3" (> Block () (@ "file.rs:3" (> Const (Constant Bool false))))))
(Function :name (Fun :path p::use) :require ((@ "file.rs:4" (CallTarget Static () (Fun :path p::never)))))
(Function :name (Fun :path p::unused) :mode Spec :kind (FunctionKind Static) :attrs (Attrs :is_external_body false) :body (@ "file.rs:5" (> Const (Constant Bool true))))
(Function :name (Fun :path p::opaque) :mode Spec :kind (FunctionKind Static) :attrs (Attrs :is_external_body true) :body (@ "file.rs:6" (> Const (Constant Bool true))))
"#).unwrap();
        let report = scan(&functions);
        assert_eq!(report["functions_total"], 8);
        assert_eq!(report["constant_predicates"].as_array().unwrap().len(), 3);
        let candidates = report["review_candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        let relation = candidates
            .iter()
            .find(|c| c["resolved_to"] == "p::impl::relation")
            .unwrap();
        assert_eq!(relation["value"], true);
        assert!(
            relation["contract_uses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u["function"] == "p::impl::borrow")
        );
        let never = candidates
            .iter()
            .find(|c| c["resolved_to"] == "p::never")
            .unwrap();
        assert_eq!(never["value"], false);
        assert_eq!(never["contract_uses"][0]["call_span"], "file.rs:4");
    }
}
