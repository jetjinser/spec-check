//! Check a None-returning default against an extracted abstract Option contract.
use crate::{
    Result, attribution as a,
    capture::Invocation,
    hash, ir, runner,
    vir::{self, Node},
    write_json,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

fn path(f: &Node) -> Option<&str> {
    f.field("name")?.field("path")?.atom()
}
fn normalized(n: &Node) -> Node {
    match n.unwrap() {
        Node::Atom(s) => Node::Atom(s.clone()),
        Node::List(xs) => Node::List(xs.iter().map(normalized).collect()),
    }
}
pub(crate) fn contract(f: &Node) -> Result<Vec<Node>> {
    let mut binders = f
        .field("params")
        .and_then(Node::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p.field("name"))
        .map(normalized)
        .collect::<Vec<_>>();
    if let Some(ret) = f.field("outer_ret").and_then(|r| r.field("name")) {
        binders.push(normalized(ret));
    }
    fn alpha(n: &Node, binders: &[Node]) -> Node {
        let n = normalized(n);
        if let Some(index) = binders.iter().position(|b| *b == n) {
            return Node::List(vec![
                Node::Atom("ContractBinding".into()),
                Node::Atom(index.to_string()),
            ]);
        }
        match n {
            Node::Atom(_) => n,
            Node::List(xs) => Node::List(xs.iter().map(|n| alpha(n, binders)).collect()),
        }
    }
    [
        "require",
        "ensure",
        "returns",
        "params",
        "outer_ret",
        "typ_params",
        "typ_bounds",
        "mask_spec",
        "unwind_spec",
    ]
    .iter()
    .map(|k| {
        f.field(k)
            .map(|n| alpha(n, &binders))
            .ok_or_else(|| format!("missing contract field {k}").into())
    })
    .collect()
}
fn returns_none(n: &Node) -> bool {
    let xs = a::items(n);
    match a::head(n) {
        Some("Block") if xs.len() == 4 && xs[2].list().is_some_and(|s| s.is_empty()) => {
            returns_none(&xs[3])
        }
        Some("Ctor") if xs.len() == 6 => {
            a::atom(&xs[2], 1) == Some("Path")
                && a::atom(&xs[2], 2) == Some("core::option::Option")
                && xs[3].atom() == Some("None")
                && xs[4].list().is_some_and(|s| s.is_empty())
                && xs[5].atom() == Some("None")
        }
        _ => false,
    }
}

fn eligible(f: &Node) -> bool {
    f.field("mode").and_then(Node::atom) == Some("Exec")
        && a::atom(f.field("kind").unwrap_or(f), 1) == Some("TraitMethodDecl")
        && f.field("body").is_some_and(|b| b.atom() == Some("None"))
        && f.field("attrs")
            .and_then(|n| n.field("is_external_body"))
            .and_then(Node::atom)
            == Some("false")
        && f.field("outer_ret")
            .and_then(|n| n.field("typ"))
            .is_some_and(|t| {
                a::atom(t, 1) == Some("Datatype")
                    && a::items(t).get(2).is_some_and(|dt| {
                        a::atom(dt, 1) == Some("Path")
                            && a::atom(dt, 2) == Some("core::option::Option")
                    })
            })
}

pub fn run(input: &Path, context: &Path, sdk: &Path, out: &Path) -> Result<()> {
    let prior: Value = serde_json::from_slice(&fs::read(input.join("report.json"))?)?;
    let inv: Invocation = serde_json::from_slice(&fs::read(context)?)?;
    crate::client::full_verification_context(&inv)?;
    let package =
        Path::new(inv.env.get("CARGO_MANIFEST_DIR").ok_or("missing package")?).canonicalize()?;
    let root = inv
        .args
        .iter()
        .map(|x| inv.cwd.join(x))
        .find(|p| p.extension().is_some_and(|s| s == "rs") && p.is_file())
        .ok_or("missing crate root")?
        .canonicalize()?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let mut results = Vec::new();
    for row in prior["results"]
        .as_array()
        .ok_or("missing ordinary results")?
    {
        if row["result"]["status"] != "whole_body_checked"
            || row["result"]["verification"]["status"] != "not_proved"
        {
            continue;
        }
        let case = Path::new(row["evidence"].as_str().ok_or("missing case")?);
        let text = fs::read_to_string(case.join("check-vir/crate.vir"))?;
        let functions = vir::functions(&text)?;
        let caller = row["result"]["target"]
            .as_str()
            .ok_or("missing checked caller")?;
        let caller = functions
            .iter()
            .find(|f| path(f) == Some(caller))
            .ok_or("checked caller missing")?;
        let mut calls = Vec::new();
        vir::walk_calls(
            caller.field("body").ok_or("missing caller body")?,
            &mut calls,
        );
        let calls = calls.into_iter().collect::<BTreeSet<_>>();
        for f in functions
            .iter()
            .filter(|f| path(f).is_some_and(|p| calls.contains(p)) && eligible(f))
        {
            let work = out.join(format!("case-{}", results.len()));
            fs::create_dir(&work)?;
            let attempt = || -> Result<Value> {
                let name = path(f).ok_or("callee without identity")?;
                let span = f
                    .list()
                    .and_then(|xs| xs.get(1))
                    .and_then(Node::atom)
                    .ok_or("callee without span")?;
                if !span.ends_with("(#0)") {
                    return Err("macro-expanded declaration unsupported".into());
                }
                let (file, coords) = ir::span(span, &inv.cwd)?;
                let original = case.join("package").canonicalize()?;
                let file = file.canonicalize()?;
                let code = fs::read_to_string(&file)?;
                let (a, b) = ir::offsets(&code, coords)?;
                let decl = code[a..b]
                    .strip_suffix(';')
                    .ok_or("expected abstract method declaration")?;
                let snapshot = work.join("package");
                runner::copy_package(&original, &snapshot)?;
                let changed_file = snapshot.join(file.strip_prefix(&original)?);
                let mut changed = code.clone();
                changed.replace_range(a..b, &format!("{decl} {{ ::core::option::Option::None }}"));
                fs::write(&changed_file, &changed)?;
                let mut args = runner::clean(
                    &inv.args,
                    &work.join("output"),
                    Some((&root, &snapshot.join(root.strip_prefix(&package)?))),
                    &inv.cwd,
                )?;
                let module = f
                    .field("owning_module")
                    .and_then(Node::atom)
                    .map(|m| m.trim_end_matches("::"))
                    .ok_or("missing owning module")?;
                let relative = module.split("::").skip(1).collect::<Vec<_>>().join("::");
                if relative.is_empty() {
                    args.push("--verify-root".into());
                } else {
                    args.extend(["--verify-only-module".into(), relative]);
                }
                args.extend([
                    "--verify-function".into(),
                    name.strip_prefix(&format!("{module}::"))
                        .unwrap_or(name)
                        .into(),
                ]);
                let proof = runner::prove(sdk, &inv, &args, &work, "alternative")?;
                let checked = work.join("alternative-vir/crate.vir");
                if !checked.is_file() {
                    return Ok(json!({"status":"not_analyzed","verification":proof}));
                }
                let after_text = fs::read_to_string(&checked)?;
                let after = vir::functions(&after_text)?;
                let after = after
                    .iter()
                    .find(|g| path(g) == Some(name))
                    .ok_or("alternative identity missing")?;
                let unchanged = contract(f)? == contract(after)?;
                let target = vir::checked_target_in(&after_text, name, &snapshot, &inv.cwd)?;
                let literal_none = after.field("body").is_some_and(returns_none);
                let accepted = unchanged
                    && literal_none
                    && target.is_some()
                    && proof["summary"] == json!([1, 0]);
                let negative = if accepted {
                    let control = work.join("control-package");
                    runner::copy_package(&original, &control)?;
                    let mut control_code = code.clone();
                    control_code.replace_range(a..b,&format!("{decl} {{ let __result = ::core::option::Option::None; ::vstd::prelude::proof! {{ assert(__result is Some); }} __result }}"));
                    fs::write(control.join(file.strip_prefix(&original)?), control_code)?;
                    let mut control_args = args.clone();
                    for arg in &mut control_args {
                        if *arg
                            == snapshot
                                .join(root.strip_prefix(&package)?)
                                .to_string_lossy()
                        {
                            *arg = control
                                .join(root.strip_prefix(&package)?)
                                .to_string_lossy()
                                .into_owned();
                        }
                    }
                    runner::prove(sdk, &inv, &control_args, &work, "some-control")?
                } else {
                    Value::Null
                };
                let accepted = accepted && negative["summary"] == json!([0, 1]);
                if fs::read_to_string(&file)? != code {
                    return Err("input source changed".into());
                }
                Ok(
                    json!({"status":if accepted{"contract_allows_none_default"}else{"alternative_not_established"},"verification":proof,"contract_unchanged":unchanged,"literal_none_confirmed":literal_none,"negative_control":negative,"negative_control_scope":"Constructed Some assertion for sensitivity, preserving the original contract; not a proposed repair.","target":target,"source":file,"source_sha256":hash(code.as_bytes())?,"original_declaration":&code[a..b],"replacement":format!("{decl} {{ ::core::option::Option::None }}"),"interpretation":"Acceptance requires a verified None-returning default with unchanged extracted contract. Original implementation behavior, documented entry protocols and reachability remain separate; this is not a confirmed defect."}),
                )
            };
            let result = match attempt() {
                Ok(v) => v,
                Err(e) => json!({"status":"unsupported","reason":e.to_string()}),
            };
            write_json(&work.join("report.json"), &result)?;
            results.push(json!({"caller":row["caller"],"callee":path(f),"original_case":case,"result":result,"evidence":work}));
        }
    }
    write_json(
        &out.join("report.json"),
        &json!({"results":results,"rule":"Abstract Option-returning callees of unproved ordinary callers; replace declaration semicolon by a None-returning default and preserve every explicit contract field.","input_sha256":hash(&fs::read(input.join("report.json"))?)?,"per_client_hints":0,"new_confirmed_defects":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn none_witness_requires_the_native_constructor() {
        let fs = vir::functions(
            "(Function :body (> Block () (> Ctor (Dt Path core::option::Option) None () None)))",
        )
        .unwrap();
        assert!(returns_none(fs[0].field("body").unwrap()));
        let fs =
            vir::functions("(Function :body (> Ctor (Dt Path core::option::Option) Some () None))")
                .unwrap();
        assert!(!returns_none(fs[0].field("body").unwrap()));
        let fs=vir::functions("(Function :body (> Block (ignored_effect) (> Ctor (Dt Path core::option::Option) None () None)))").unwrap();
        assert!(!returns_none(fs[0].field("body").unwrap()));
    }
    #[test]
    fn declaration_to_body_renames_only_parameter_binders() {
        let t = "(Function :require () :ensure (tuple () ()) :returns None :params ((Param :name (VarIdent self (VarIdentDisambiguate NoBodyParam)) :typ (Typ Bool))) :outer_ret (Param :typ (Typ Bool)) :typ_params () :typ_bounds () :mask_spec None :unwind_spec None)";
        let a = vir::functions(t).unwrap();
        let b = vir::functions(&t.replace("NoBodyParam", "RustcId 2")).unwrap();
        assert_eq!(contract(&a[0]).unwrap(), contract(&b[0]).unwrap());
    }
    #[test]
    fn preservation_ignores_locations_but_retains_constraints() {
        let text = r#"(Function :require ((@ "a.rs:1:1: 1:2 (#0)" (> Const (Constant Bool true)))) :ensure (tuple () ()) :returns None :params () :outer_ret (Param :typ (Typ Bool)) :typ_params () :typ_bounds () :mask_spec None :unwind_spec None)"#;
        let a = vir::functions(text).unwrap();
        let b = vir::functions(&text.replace("a.rs:1:1: 1:2", "b.rs:2:1: 2:2")).unwrap();
        assert_eq!(contract(&a[0]).unwrap(), contract(&b[0]).unwrap());
        let b = vir::functions(&text.replace("Bool true", "Bool false")).unwrap();
        assert_ne!(contract(&a[0]).unwrap(), contract(&b[0]).unwrap());
        let b = vir::functions(&text.replace(":params ()", ":params ((Param :typ (Typ Bool)))"))
            .unwrap();
        assert_ne!(contract(&a[0]).unwrap(), contract(&b[0]).unwrap());
    }
}
