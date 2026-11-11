//! Inventory direct conditional return guarantees; candidates require solver validation.
use crate::{
    Result, attribution as a, contracts, hash, ir,
    vir::{self, Node},
    write_json,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn normalized(n: &Node) -> Node {
    match n.unwrap() {
        Node::Atom(s) => Node::Atom(s.clone()),
        Node::List(xs) => Node::List(xs.iter().map(normalized).collect()),
    }
}
fn contains(n: &Node, needle: &Node) -> bool {
    normalized(n) == normalized(needle)
        || n.list()
            .is_some_and(|xs| xs.iter().any(|x| contains(x, needle)))
}
fn logical<'a>(n: &'a Node, op: &str) -> Option<&'a [Node]> {
    let xs = a::items(n);
    (a::head(n) == Some("Logical") && xs.get(2).is_some_and(|x| a::atom(x, 1) == Some(op)))
        .then_some(xs.get(3..).unwrap_or_default())
}
fn conjuncts<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    let n = unblock(n);
    if let Some(xs) = logical(n, "And") {
        for x in xs {
            conjuncts(x, out);
        }
    } else {
        out.push(n);
    }
}
fn location(n: &Node) -> Option<&str> {
    n.list().and_then(|xs| {
        matches!(xs.first()?.atom(), Some("@" | "@@"))
            .then(|| xs.get(1)?.atom())
            .flatten()
    })
}
fn unblock(mut n: &Node) -> &Node {
    while a::head(n) == Some("Block") {
        let xs = a::items(n);
        if xs.len() != 4 || !xs[2].list().is_some_and(|s| s.is_empty()) {
            break;
        }
        n = &xs[3];
    }
    n
}
fn literal_true(n: &Node) -> bool {
    let xs = a::items(unblock(n));
    a::head(unblock(n)) == Some("Const")
        && xs
            .get(2)
            .is_some_and(|c| a::atom(c, 1) == Some("Bool") && a::atom(c, 2) == Some("true"))
}
fn bindings(
    pattern: &Node,
    body: &Node,
    selectors: &[Value],
    out: &mut Vec<Value>,
) -> std::result::Result<(), &'static str> {
    let xs = a::items(pattern);
    if a::head(pattern) != Some("Pattern") {
        return Err("unrecognized pattern");
    }
    match a::atom(pattern, 1) {
        Some("Wildcard") => Ok(()),
        Some("Var") => {
            let binding = xs.get(2).ok_or("missing binding")?;
            let name = binding.field("name").ok_or("missing binder identity")?;
            out.push(
                json!({"binder":contracts::render(name),"selectors":selectors,
                "by_ref":binding.field("by_ref").map(contracts::render),
                "type":binding.field("typ").map(contracts::render),
                "used_in_consequence":contains(body,name)}),
            );
            Ok(())
        }
        Some("Constructor") if xs.len() == 5 => {
            let fields = xs[4].list().ok_or("missing constructor fields")?;
            for field in fields {
                let edge = a::items(field);
                if edge.len() != 3 || edge[0].atom() != Some("->") {
                    return Err("unknown pattern field");
                }
                let mut path = selectors.to_vec();
                path.push(json!({"datatype":contracts::render(&xs[2]),"variant":xs[3].atom(),"field":edge[1].atom()}));
                bindings(&edge[2], body, &path, out)?;
            }
            Ok(())
        }
        _ => Err("pattern binding form not analyzed"),
    }
}
fn scan(functions: &[Node]) -> Value {
    let mut inventory = Vec::new();
    let mut candidates = Vec::new();
    for f in functions {
        if f.field("mode").and_then(Node::atom) != Some("Exec") {
            continue;
        }
        let path = f
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(Node::atom);
        let mut direct = Vec::new();
        // Both VIR postcondition lists retain expressions; neither contains return binders.
        let ensures = f.field("ensure").and_then(Node::list);
        let ret = f.field("outer_ret").and_then(|n| n.field("name"));
        if let Some(es) = ensures {
            for group in es.iter().skip(1) {
                if let Some(expressions) = group.list() {
                    for e in expressions {
                        conjuncts(e, &mut direct);
                    }
                }
            }
        }
        let mut guaranteed = Vec::new();
        for e in &direct {
            if logical(e, "Implies").is_none() {
                conjuncts(e, &mut guaranteed);
            }
        }
        let before = candidates.len();
        let mut unanalysed = Vec::new();
        for e in &direct {
            if a::head(e) == Some("Match") {
                let xs = a::items(e);
                if !ret.is_some_and(|r| contains(e, r)) {
                    continue;
                }
                if let Some(arms) = xs.get(3).and_then(Node::list) {
                    let mut prior = Vec::new();
                    for arm in arms {
                        if let (Some(pattern), Some(guard), Some(body)) =
                            (arm.field("pattern"), arm.field("guard"), arm.field("body"))
                        {
                            if !literal_true(body) {
                                let mut aliases = Vec::new();
                                let binding_error =
                                    bindings(pattern, body, &[], &mut aliases).err();
                                if let Some(reason) = binding_error {
                                    aliases.clear();
                                    unanalysed.push(json!({"span":location(pattern),"form":"MatchPattern","reason":reason}));
                                }
                                candidates.push(json!({"kind":"ordered_match_arm","function":path,"span":location(f),
                                    "guarantee_span":location(e),"condition":{"scrutinee":contracts::render(&xs[2]),
                                        "pattern":contracts::render(pattern),"guard":contracts::render(guard),"preceding_arms":prior},
                                    "bindings":aliases,"binding_error":binding_error,"consequence":contracts::render(body),
                                    "source_expression":contracts::render(e),"status":"candidate",
                                    "reason":"Return-dependent ordered Match branch; patterns and guards are retained without strengthening or logical rewriting. No semantic sufficiency or defect verdict."}));
                            }
                            prior.push(json!({"pattern":contracts::render(pattern),"guard":contracts::render(guard)}));
                        } else {
                            unanalysed.push(json!({"span":location(arm),"form":"MatchArm","reason":"missing pattern, guard or body"}));
                            prior.push(json!({"unanalysed_arm":contracts::render(arm)}));
                        }
                    }
                } else {
                    unanalysed.push(
                        json!({"span":location(e),"form":"Match","reason":"missing arm list"}),
                    );
                }
                continue;
            }
            if matches!(a::head(e), Some("If" | "Quant" | "Bind" | "Let" | "Block")) {
                unanalysed.push(json!({"span":location(e),"form":a::head(e),"reason":"nested condition or binding scope not analyzed"}));
            }
            let Some([condition, consequence]) = logical(e, "Implies") else {
                continue;
            };
            let Some(ret) = ret else {
                continue;
            };
            if !contains(consequence, ret) && !contains(condition, ret) {
                continue;
            }
            let mut conditions = Vec::new();
            conjuncts(condition, &mut conditions);
            let missing: Vec<_> = conditions
                .into_iter()
                .filter(|c| !guaranteed.iter().any(|g| normalized(c) == normalized(g)))
                .collect();
            if missing.is_empty() {
                continue;
            }
            candidates.push(json!({"kind":"implication","function":path,"span":location(f),"guarantee_span":location(e),
                "condition":contracts::render(condition),"consequence":contracts::render(consequence),
                "not_directly_guaranteed":missing.iter().map(|n| contracts::render(n)).collect::<Vec<_>>(),
                "status":"candidate","reason":"Condition is not an identical unconditional postcondition conjunct; semantic implication, preconditions and inherited contracts have not been checked."}));
        }
        if candidates.len() == before
            && f.field("outer_ret")
                .and_then(|r| r.field("typ"))
                .and_then(machine_integer)
                .is_some()
            && f.field("outer_ret")
                .and_then(|r| r.field("mode"))
                .and_then(Node::atom)
                != Some("Proof")
        {
            candidates.push(json!({"kind":"machine_integer_return","function":path,"span":location(f),
                "return_type":f.field("outer_ret").and_then(|r|r.field("typ")).map(contracts::render),
                "status":"candidate","reason":"Finite machine-integer return alternatives; the complete native contract decides acceptance, including unconditional or absent postconditions."}));
        }
        inventory.push(json!({"function":path,"span":location(f),"direct_ensures_available":ensures.is_some(),"conditional_candidates":candidates.len()-before,"unanalysed_guarantees":unanalysed}));
    }
    json!({"scope":"Machine-integer returns, top-level implications and ordered Match branches depending on the return binder. Payload bindings retain complete identities, constructor selectors and reference modes. Nested conditions and inherited contracts remain unanalysed. Candidates are inspection signals; no defect or satisfiability conclusion.","functions":inventory,"candidates":candidates,"defects_confirmed":0})
}
pub fn run(input: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("conditional-scan output directory already exists".into());
    }
    let input = input.canonicalize()?;
    let source = fs::read_to_string(&input)?;
    let functions = vir::functions(&source)?;
    if functions.is_empty() {
        return Err("input contains no typed VIR functions".into());
    }
    let mut report = scan(&functions);
    report["input"] = json!({"path":input,"sha256":hash(source.as_bytes())?,"provenance":"Supplied VIR; source/build correspondence must be established separately."});
    fs::create_dir_all(out)?;
    write_json(&out.join("report.json"), &report)
}

fn machine_integer(typ: &Node) -> Option<bool> {
    if a::atom(typ, 1) != Some("Int") {
        return None;
    }
    let range = a::items(typ).get(2)?;
    match a::atom(range, 1) {
        Some("USize") => Some(false),
        Some("ISize") => Some(true),
        Some(sign @ ("U" | "I"))
            if matches!(a::atom(range, 2), Some("8" | "16" | "32" | "64" | "128")) =>
        {
            Some(sign == "I")
        }
        _ => None,
    }
}
fn identifier(name: &str) -> Option<String> {
    if name.is_empty()
        || matches!(name, "_" | "crate" | "self" | "Self" | "super")
        || !name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
    {
        return None;
    }
    Some(format!("r#{name}"))
}
fn datatype_path(typ: &Node) -> Option<&str> {
    if a::atom(typ, 1) != Some("Datatype") {
        return None;
    }
    let dt = a::items(typ).get(2)?;
    (a::atom(dt, 1) == Some("Path"))
        .then(|| a::atom(dt, 2))
        .flatten()
}
fn substitute(typ: &Node, names: &[&str], args: &[Node]) -> Node {
    if a::atom(typ, 0) == Some("Typ")
        && a::atom(typ, 1) == Some("TypParam")
        && let Some(i) = names.iter().position(|name| Some(*name) == a::atom(typ, 2))
    {
        return args[i].clone();
    }
    match typ.unwrap() {
        Node::Atom(s) => Node::Atom(s.clone()),
        Node::List(xs) => Node::List(xs.iter().map(|n| substitute(n, names, args)).collect()),
    }
}
fn values(typ: &Node, datatypes: &[Node], krate: &str, depth: usize) -> Vec<String> {
    if depth == 0 {
        return Vec::new();
    }
    if a::atom(typ, 1) == Some("Bool") {
        return vec!["false".into(), "true".into()];
    }
    if machine_integer(typ).is_some() {
        return vec!["0".into(), "1".into()];
    }
    let xs = a::items(typ);
    if a::atom(typ, 1) == Some("Decorate")
        && xs.get(2).is_some_and(|n| a::atom(n, 1) == Some("Ref"))
        && xs.get(4).is_some_and(|n| {
            a::atom(n, 1) == Some("Primitive")
                && a::items(n)
                    .get(2)
                    .is_some_and(|p| a::atom(p, 1) == Some("StrSlice"))
        })
    {
        return vec!["\"\"".into()];
    }
    if a::atom(typ, 1) == Some("Datatype")
        && xs
            .get(2)
            .is_some_and(|n| a::atom(n, 1) == Some("Tuple") && a::atom(n, 2) == Some("0"))
    {
        return vec!["()".into()];
    }
    let Some(path) = datatype_path(typ) else {
        return Vec::new();
    };
    if path == "core::option::Option" {
        return vec!["::core::option::Option::None".into()];
    }
    let matching = datatypes
        .iter()
        .filter(|d| {
            d.field("name")
                .is_some_and(|n| a::atom(n, 1) == Some("Path") && a::atom(n, 2) == Some(path))
        })
        .collect::<Vec<_>>();
    let [datatype] = matching.as_slice() else {
        return Vec::new();
    };
    if datatype.field("mode").and_then(Node::atom) != Some("Exec")
        || datatype.field("proxy").and_then(Node::atom) != Some("None")
        || datatype
            .field("transparency")
            .is_some_and(|n| a::atom(n, 1) == Some("Never"))
    {
        return Vec::new();
    }
    let names = datatype
        .field("typ_params")
        .and_then(Node::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|n| a::atom(n, 1))
        .collect::<Vec<_>>();
    let args = xs.get(3).and_then(Node::list).unwrap_or_default();
    if names.len() != args.len() {
        return Vec::new();
    }
    let mut segments = path.split("::");
    let first = segments.next().unwrap_or_default();
    let prefix = if first == krate {
        Some("crate".to_owned())
    } else {
        identifier(first).map(|s| format!("::{s}"))
    };
    let Some(mut prefix) = prefix else {
        return Vec::new();
    };
    for part in segments {
        let Some(part) = identifier(part) else {
            return Vec::new();
        };
        prefix.push_str(&format!("::{part}"));
    }
    let mut result = Vec::new();
    'variant: for variant in datatype
        .field("variants")
        .and_then(Node::list)
        .unwrap_or_default()
    {
        let Some(name) = variant.field("name").and_then(Node::atom) else {
            continue;
        };
        let Some(variant_name) = identifier(name) else {
            continue;
        };
        let constructor = if name == path.rsplit("::").next().unwrap_or_default() {
            prefix.clone()
        } else {
            format!("{prefix}::{variant_name}")
        };
        let Some(fields) = variant.field("fields").and_then(Node::list) else {
            continue;
        };
        if fields.len() > 8 {
            continue;
        }
        let style = variant.field("ctor_style").and_then(|n| a::atom(n, 1));
        let mut products = vec![Vec::<String>::new()];
        for (i, field) in fields.iter().enumerate() {
            let edge = a::items(field);
            if edge.len() != 3 || edge[0].atom() != Some("->") {
                continue 'variant;
            }
            let data = a::items(&edge[2]);
            if data.len() != 4 || data[2].atom() != Some("Exec") {
                continue 'variant;
            }
            let choices = values(
                &substitute(&data[1], &names, args),
                datatypes,
                krate,
                depth - 1,
            );
            if choices.is_empty() {
                continue 'variant;
            }
            let label = if style == Some("Braces") {
                let Some(label) = edge[1].atom().and_then(identifier) else {
                    continue 'variant;
                };
                Some(label)
            } else {
                if edge[1].atom().and_then(|s| s.parse::<usize>().ok()) != Some(i) {
                    continue 'variant;
                }
                None
            };
            let mut next = Vec::new();
            for product in &products {
                for value in &choices {
                    let mut product = product.clone();
                    product.push(
                        label
                            .as_ref()
                            .map_or_else(|| value.clone(), |label| format!("{label}: {value}")),
                    );
                    next.push(product);
                    if next.len() == 2 {
                        break;
                    }
                }
                if next.len() == 2 {
                    break;
                }
            }
            products = next;
        }
        for product in products {
            let expression = match style {
                Some("Const") if fields.is_empty() => constructor.clone(),
                Some("Braces") => format!("{constructor} {{ {} }}", product.join(", ")),
                Some("Parens") => format!("{constructor}({})", product.join(", ")),
                _ => continue,
            };
            result.push(expression);
            if result.len() == 2 {
                return result;
            }
        }
    }
    result
}

pub(crate) fn alternatives(f: &Node, datatypes: &[Node]) -> Result<Vec<String>> {
    let typ = f
        .field("outer_ret")
        .and_then(|r| r.field("typ"))
        .ok_or("missing return type")?;
    if let Some(signed) = machine_integer(typ) {
        let mut result = vec!["0".into(), "1".into()];
        if signed {
            result.push("-1".into());
        }
        return Ok(result);
    }
    if a::atom(typ, 1) == Some("Bool") {
        return Ok(vec!["false".into(), "true".into()]);
    }
    let datatype = a::items(typ).get(2);
    let path = datatype.and_then(|d| {
        if a::atom(d, 1) == Some("Path") {
            a::atom(d, 2)
        } else {
            None
        }
    });
    if a::atom(typ, 1) == Some("Datatype") && path == Some("core::option::Option") {
        return Ok(vec!["::core::option::Option::None".into()]);
    }
    if a::atom(typ, 1) != Some("Datatype") || path != Some("core::result::Result") {
        return Err("return type outside machine-integer/Bool/Option/Result mutation rules".into());
    }
    let error = a::items(typ)
        .get(3)
        .and_then(Node::list)
        .and_then(|a| a.get(1))
        .ok_or("missing Result error type")?;
    let krate = f
        .field("name")
        .and_then(|n| n.field("path"))
        .and_then(Node::atom)
        .and_then(|s| s.split("::").next())
        .unwrap_or("");
    let mut expressions = values(error, datatypes, krate, 4)
        .into_iter()
        .map(|e| format!("::core::result::Result::Err({e})"))
        .collect::<Vec<_>>();
    for p in f.field("params").and_then(Node::list).unwrap_or_default() {
        if p.field("mode").and_then(Node::atom) != Some("Exec")
            || !p
                .field("typ")
                .is_some_and(|t| normalized(t) == normalized(error))
        {
            continue;
        }
        let Some(name) = p.field("name").and_then(|n| a::atom(n, 1)) else {
            continue;
        };
        if name == "_"
            || name.is_empty()
            || !name
                .chars()
                .enumerate()
                .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
        {
            continue;
        }
        let name = if name == "self" {
            name.to_owned()
        } else {
            format!("r#{name}")
        };
        expressions.push(format!("::core::result::Result::Err({name})"));
    }
    if expressions.is_empty() {
        return Err(
            "Result error has no bounded type-directed value or matching executable value parameter".into(),
        );
    }
    Ok(expressions)
}

/// Emit isolated source mutations; acceptance still requires an independent Verus run.
pub fn generate(input: &Path, source_root: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("conditional-generate output already exists".into());
    }
    let text = fs::read_to_string(input)?;
    let functions = vir::functions(&text)?;
    let datatypes = vir::datatypes(&text)?;
    let census = scan(&functions);
    let selected = census["candidates"]
        .as_array()
        .ok_or("missing census")?
        .iter()
        .filter_map(|r| r["function"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let root = source_root.canonicalize()?;
    fs::create_dir_all(out)?;
    let mut rows = Vec::new();
    for f in &functions {
        let Some(name) = f
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(Node::atom)
        else {
            continue;
        };
        if !selected.contains(name) {
            continue;
        }
        let attempt = || -> Result<Vec<Value>> {
            if f.field("attrs")
                .and_then(|n| n.field("is_external_body"))
                .and_then(Node::atom)
                == Some("true")
            {
                return Err("trusted external body is excluded".into());
            }
            let replacements = alternatives(f, &datatypes)?;
            let body = f.field("body").ok_or("missing body")?;
            let span = location(body).ok_or("body unavailable or lacks source location")?;
            if !span.ends_with("(#0)") {
                return Err("macro-expanded body excluded".into());
            }
            let (file, coords) = ir::span(span, &root)?;
            let file = file.canonicalize()?;
            let relative = file.strip_prefix(&root)?;
            let source = fs::read_to_string(&file)?;
            let (start, end) = ir::offsets(&source, coords)?;
            if !source[start..end].starts_with('{') || !source[start..end].ends_with('}') {
                return Err("source span does not delimit a complete function body".into());
            }
            let mut mutations = Vec::new();
            for expression in replacements {
                let folder = out.join(format!("mutation-{}-{}", rows.len(), mutations.len()));
                fs::create_dir(&folder)?;
                let mut changed = source.clone();
                changed.replace_range(start..end, &format!("{{ {expression} }}"));
                fs::write(folder.join("source.rs"), changed)?;
                mutations.push(json!({"expression":expression,"artifact":folder.join("source.rs"),"source_relative":relative,
                    "source_sha256":hash(source.as_bytes())?,"body_span":span,"body_edit":[start,end],
                    "signature_and_contract":"Source outside the function body is byte-identical; actual expanded contract must be checked after compilation.",
                    "status":"generated_unchecked","runtime":"not_run"}));
            }
            Ok(mutations)
        };
        let result = match attempt() {
            Ok(m) => json!({"status":"generated_unchecked","mutations":m}),
            Err(e) => json!({"status":"unsupported","reason":e.to_string()}),
        };
        rows.push(json!({"function":name,"result":result}));
    }
    write_json(
        &out.join("report.json"),
        &json!({"vir_sha256":hash(text.as_bytes())?,"source_root":root,
        "rule":"Replace only the body with machine-integer/Bool constants, Option::None or Result::Err constructed from typed fields/parameters. Constructor depth 4, at most 8 fields and 2 values per recursive type; no assumptions or contract edits.",
        "per_function_hints":0,"results":rows,"confirmed_defects":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(ensure: &str) -> Value {
        let text = format!(
            "(Function :name (Fun :path p::f) :mode Exec :outer_ret (Param :name (VarIdent r 1)) :ensure (tuple ({ensure}) ()))"
        );
        scan(&vir::functions(&text).unwrap())
    }
    #[test]
    fn tracks_exact_return_and_requires_unconditional_guard() {
        let implication =
            "(> Logical (LogicalOp Implies) (> Var (VarIdent law 2)) (> Var (VarIdent r 1)))";
        assert_eq!(
            fixture(implication)["candidates"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            fixture(&format!("(> Var (VarIdent law 2)) {implication}"))["candidates"],
            json!([])
        );
        assert_eq!(
            fixture(&implication.replace("r 1", "r 9"))["candidates"],
            json!([])
        );
    }
    #[test]
    fn conditional_guard_is_not_unconditional_evidence() {
        let text = "(> Logical (LogicalOp Implies) (> Var (VarIdent law 2)) (> Var (VarIdent r 1))) (> Logical (LogicalOp Implies) (> Var (VarIdent other 3)) (> Var (VarIdent law 2)))";
        assert_eq!(fixture(text)["candidates"].as_array().unwrap().len(), 1);
    }
    fn matched(payload_use: &str) -> Value {
        fixture(&format!(
            "(> Match (Place Local (VarIdent r 1)) ((Arm :pattern (Pattern Constructor (Dt Path core::option::Option) Some ((-> 0 (Pattern Var (PatternBinding :name (VarIdent r 9) :by_ref (ByRef No) :typ (Typ Int (IntRange USize))))))) :guard (> Const (Constant Bool true)) :body (> Block () {payload_use})) (Arm :pattern (Pattern Wildcard) :guard (> Const (Constant Bool true)) :body (> Block () (> Const (Constant Bool true))))))"
        ))
    }
    #[test]
    fn some_payload_binding_retains_identity_and_selector() {
        let r = matched("(> Var (VarIdent r 9))");
        let c = &r["candidates"][0];
        assert_eq!(r["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(c["kind"], "ordered_match_arm");
        assert_eq!(c["bindings"][0]["selectors"][0]["variant"], "Some");
        assert_eq!(c["bindings"][0]["selectors"][0]["field"], "0");
        assert_eq!(c["bindings"][0]["used_in_consequence"], true);
        let other = matched("(> Var (VarIdent r 3))");
        assert_eq!(
            other["candidates"][0]["bindings"][0]["used_in_consequence"],
            false
        );
    }
    #[test]
    fn match_preserves_preceding_true_arms_and_guards() {
        let r = fixture(
            "(> Match (Place Local (VarIdent r 1)) ((Arm :pattern (Pattern Wildcard) :guard (> Var (VarIdent enabled 8)) :body (> Const (Constant Bool true))) (Arm :pattern (Pattern Wildcard) :guard (> Const (Constant Bool true)) :body (> Const (Constant Bool false)))))",
        );
        let prior = &r["candidates"][0]["condition"]["preceding_arms"];
        assert_eq!(prior.as_array().unwrap().len(), 1);
        assert!(prior[0]["guard"].as_str().unwrap().contains("enabled"));
    }
    #[test]
    fn unanalysed_forms_are_visible_and_return_guards_are_candidates() {
        let r = fixture(
            "(> If (> Const (Constant Bool true)) (> Var (VarIdent r 1)) (> Const (Constant Bool false)))",
        );
        assert_eq!(r["functions"][0]["unanalysed_guarantees"][0]["form"], "If");
        let r = fixture(
            "(> Logical (LogicalOp Implies) (> Var (VarIdent r 1)) (> Var (VarIdent updated 7)))",
        );
        assert_eq!(r["candidates"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn result_error_comes_only_from_unit_or_exact_exec_parameter_type() {
        let text = "(Function :outer_ret (Param :typ (Typ Datatype (Dt Path core::result::Result) ((Typ Bool) (Typ Datatype (Dt Path p::Error) () ())) ())) :params ((Param :name (VarIdent error 3) :mode Exec :typ (Typ Datatype (Dt Path p::Error) () ()))))";
        let fs = vir::functions(text).unwrap();
        assert_eq!(
            alternatives(&fs[0], &[]).unwrap(),
            vec!["::core::result::Result::Err(r#error)"]
        );
        for changed in [
            text.replace(":mode Exec", ":mode Proof"),
            text.replace("(VarIdent error 3)", "(VarIdent _ 3)"),
        ] {
            assert!(alternatives(&vir::functions(&changed).unwrap()[0], &[]).is_err());
        }
        let unit = "(Function :outer_ret (Param :typ (Typ Datatype (Dt Path core::result::Result) ((Typ Bool) (Typ Datatype (Dt Tuple 0) () ())) ())) :params ())";
        assert_eq!(
            alternatives(&vir::functions(unit).unwrap()[0], &[]).unwrap(),
            vec!["::core::result::Result::Err(())"]
        );
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;
    #[test]
    fn machine_values_are_signed_and_executable() {
        for (range, expected) in [("USize", vec!["0", "1"]), ("I 32", vec!["0", "1", "-1"])] {
            let f = vir::functions(&format!(
                "(Function :mode Exec :outer_ret (Param :typ (Typ Int (IntRange {range}))))"
            ))
            .unwrap();
            assert_eq!(alternatives(&f[0], &[]).unwrap(), expected);
            assert_eq!(scan(&f)["candidates"][0]["kind"], "machine_integer_return");
        }
        for range in ["Nat", "Int", "Char"] {
            let f = vir::functions(&format!(
                "(Function :outer_ret (Param :typ (Typ Int (IntRange {range}))))"
            ))
            .unwrap();
            assert!(alternatives(&f[0], &[]).is_err());
        }
    }
    #[test]
    fn generic_error_fields_are_substituted_and_bounded() {
        let text = "(Datatype :name (Dt Path p::Error) :mode Exec :proxy None :transparency (DatatypeTransparency WhenVisible) :typ_params ((tuple T Accept)) :variants ((Variant :name Error :fields ((-> value (tuple (Typ TypParam T) Exec (Visibility :restricted_to None))) (-> trace (tuple (Typ Datatype (Dt Path core::option::Option) () ()) Exec (Visibility :restricted_to None)))) :ctor_style (CtorPrintStyle Braces)))) (Function :name (Fun :path p::f) :outer_ret (Param :typ (Typ Datatype (Dt Path core::result::Result) ((Typ Bool) (Typ Datatype (Dt Path p::Error) ((Typ Int (IntRange U 8))) ())) ())))";
        let fs = vir::functions(text).unwrap();
        let types = vir::datatypes(text).unwrap();
        let values = alternatives(&fs[0], &types).unwrap();
        assert_eq!(values.len(), 2);
        assert!(
            values[0]
                .contains("crate::r#Error { r#value: 0, r#trace: ::core::option::Option::None }")
        );
        assert!(values[1].contains("r#value: 1"));
        for changed in [
            text.replace("(Typ TypParam T) Exec", "(Typ TypParam T) Proof"),
            text.replace(":proxy None", ":proxy (Some alias)"),
            text.replace("WhenVisible", "Never"),
        ] {
            assert!(
                alternatives(
                    &vir::functions(&changed).unwrap()[0],
                    &vir::datatypes(&changed).unwrap()
                )
                .is_err()
            );
        }
    }
    #[test]
    fn recursive_errors_do_not_invent_resources() {
        let text = "(Datatype :name (Dt Path p::Loop) :mode Exec :proxy None :typ_params () :variants ((Variant :name Next :fields ((-> 0 (tuple (Typ Datatype (Dt Path p::Loop) () ()) Exec (Visibility :restricted_to None)))) :ctor_style (CtorPrintStyle Parens)))) (Function :name (Fun :path p::f) :outer_ret (Param :typ (Typ Datatype (Dt Path core::result::Result) ((Typ Bool) (Typ Datatype (Dt Path p::Loop) () ())) ())))";
        assert!(
            alternatives(
                &vir::functions(text).unwrap()[0],
                &vir::datatypes(text).unwrap()
            )
            .is_err()
        );
    }
}
