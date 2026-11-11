//! Check an effect-free entry call without inventing caller preconditions.
use crate::{
    Result,
    capture::Invocation,
    contracts, hash, ir, link, runner,
    vir::{self, Node},
    write_json,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

fn path(f: &Node) -> Option<&str> {
    f.field("name")?.field("path")?.atom()
}
fn scalar_predicate(n: &Node, params: &BTreeSet<String>, functions: &[Node]) -> bool {
    let Some(xs) = n.unwrap().list() else {
        return true;
    };
    if xs.first().and_then(Node::atom) == Some(">") {
        let supported = match xs.get(1).and_then(Node::atom) {
            Some("Const" | "Binary") => true,
            Some("ReadPlace") => xs.get(2).and_then(|p| p.unwrap().list()).is_some_and(|p| {
                p.get(1).and_then(Node::atom) == Some("Local")
                    && p.get(2)
                        .is_some_and(|v| params.contains(&contracts::render(v)))
            }),
            Some("ConstVar") => xs
                .get(2)
                .and_then(|f| f.field("path"))
                .and_then(Node::atom)
                .is_some_and(|p| {
                    functions.iter().any(|f| {
                        path(f) == Some(p)
                            && f.field("item_kind")
                                .and_then(Node::list)
                                .is_some_and(|k| k.get(1).and_then(Node::atom) == Some("Const"))
                    })
                }),
            Some("UnaryOpr") => xs
                .get(2)
                .and_then(Node::list)
                .is_some_and(|v| v.get(1).and_then(Node::atom) == Some("IntegerTypeBound")),
            _ => false,
        };
        if !supported {
            return false;
        }
    }
    xs.iter().all(|x| scalar_predicate(x, params, functions))
}

pub(crate) fn run(
    inv: &Invocation,
    sdk: &Path,
    unit: &Path,
    relations: &Value,
    callers: &[Value],
    functions: &[Node],
    probe: &Path,
) -> Result<Value> {
    let original = fs::read_to_string(unit.join("original-borrow.stderr"))?;
    let package =
        Path::new(inv.env.get("CARGO_MANIFEST_DIR").ok_or("missing package")?).canonicalize()?;
    let root = inv
        .args
        .iter()
        .map(|a| inv.cwd.join(a))
        .find(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
        .ok_or("missing crate root")?
        .canonicalize()?;
    let owners: Vec<Vec<String>> =
        serde_json::from_slice(&fs::read(unit.join("owner-identities.json"))?)?;
    let mut checks = Vec::new();
    for line in original
        .lines()
        .filter(|l| l.starts_with("CALLER_PREFIX\t"))
    {
        let row = line.split('\t').collect::<Vec<_>>();
        if row.len() != 9 {
            return Err("invalid compiler prefix".into());
        }
        let Some(caller) = callers.iter().find(|c| {
            link::stable(c["owner"]["owner"].as_str().unwrap()) == link::stable(row[1])
                && c["route"] == "requires_generated_client"
        }) else {
            continue;
        };
        let own = owners
            .iter()
            .find(|o| link::stable(&o[1]) == link::stable(row[1]))
            .ok_or("missing caller identity")?;
        if row[8] == "true"
            || functions.iter().any(|f| path(f) == Some(&own[2]))
            || (row[7] != "-"
                && (row[7] == "UNRESOLVED_TRAIT"
                    || functions.iter().any(|f| path(f) == Some(row[7]))))
        {
            checks.push(
                json!({"caller":caller["owner"],"status":"entry_contract_requires_connection"}),
            );
            continue;
        }
        let Some(relation) = relations
            .as_array()
            .ok_or("missing relations")?
            .iter()
            .find(|r| {
                link::stable(r["call"]["owner"].as_str().unwrap()) == link::stable(row[1])
                    && r["call"]["block"] == "bb0"
            })
        else {
            return Err("prefix not retained in interface handoff".into());
        };
        let binding = &relation["association"];
        if binding["status"] != "linked"
            || binding["ghost_projection"] == true
            || binding["interface"] != row[3]
            || link::stable(relation["call"]["span"].as_str().unwrap()) != link::stable(row[2])
        {
            checks.push(json!({"caller":caller["owner"],"status":"prefix_interface_unavailable"}));
            continue;
        }
        let callee = functions
            .iter()
            .find(|f| path(f) == Some(row[3]))
            .ok_or("missing callee")?;
        let requirements = callee
            .field("require")
            .and_then(Node::list)
            .ok_or("missing requires")?;
        if requirements.is_empty() {
            checks.push(json!({"caller":caller["owner"],"status":"no_precondition_target"}));
            continue;
        }
        let params = callee
            .field("params")
            .and_then(Node::list)
            .ok_or("missing parameters")?
            .iter()
            .filter_map(|p| p.field("name").map(contracts::render))
            .collect();
        if !requirements
            .iter()
            .all(|n| scalar_predicate(n, &params, functions))
        {
            checks
                .push(json!({"caller":caller["owner"],"status":"non_scalar_precondition_context"}));
            continue;
        }
        let id = format!(
            "__spec_call_{}",
            &hash(format!("{}:{}", link::stable(row[1]), row[2]).as_bytes())?[..12]
        );
        let case = unit.join(&id);
        fs::create_dir(&case)?;
        let snapshot = case.join("package");
        runner::copy_package(&package, &snapshot)?;
        let (source, _) = ir::span(row[2], &inv.cwd)?;
        let source = source.canonicalize()?;
        let file = snapshot.join(source.strip_prefix(&package)?);
        let code = fs::read_to_string(&file)?;
        if code.contains(&id) {
            return Err("generated symbol collision".into());
        }
        let inputs = if row[4].is_empty() {
            Vec::new()
        } else {
            row[4]
                .split('|')
                .map(|p| p.split_once(':').ok_or("invalid input type"))
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let arguments = if row[5].is_empty() {
            Vec::new()
        } else {
            row[5].split(',').collect::<Vec<_>>()
        };
        let declarations = inputs
            .iter()
            .map(|(n, t)| format!("__arg_{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        let actual = arguments
            .iter()
            .map(|n| format!("__arg_{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let rust_path = std::iter::once("crate".to_owned())
            .chain(row[3].split("::").skip(1).map(|s| format!("r#{s}")))
            .collect::<Vec<_>>()
            .join("::");
        let generated = format!(
            "\nvstd::prelude::verus! {{ exec fn {id}({declarations}) -> {} {{ {rust_path}({actual}) }} }}\n",
            row[6]
        );
        fs::write(&file, format!("{code}{generated}"))?;
        let copied = snapshot.join(root.strip_prefix(&package)?);
        let mut args = runner::clean(
            &inv.args,
            &case.join("metadata"),
            Some((&root, &copied)),
            &inv.cwd,
        )?;
        let mut context = inv.clone();
        context
            .env
            .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
        context
            .env
            .insert("CLIENT_PROJECT_PREFIX".into(), "1".into());
        context.env.insert("CLIENT_PROJECT_LINK".into(), "1".into());
        runner::extract_context(probe, &context, &args, "*", &case, "generated", true)?;
        let stderr = fs::read_to_string(case.join("generated.stderr"))?;
        let defs = stderr
            .lines()
            .filter_map(|l| l.strip_prefix("LINK_OWNER\t"))
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .filter(|r| r.get(1).is_some_and(|p| p.rsplit("::").next() == Some(&id)))
            .collect::<Vec<_>>();
        let [def] = defs.as_slice() else {
            return Err("generated caller identity not unique".into());
        };
        let observed = stderr
            .lines()
            .filter(|l| l.starts_with("CALLER_PREFIX\t"))
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .find(|r| link::stable(r[1]) == link::stable(def[0]))
            .ok_or("generated call no longer has entry-prefix shape")?;
        let expected_args = arguments
            .iter()
            .map(|n| {
                inputs
                    .iter()
                    .position(|(p, _)| p == n)
                    .map(|i| (i + 1).to_string())
                    .ok_or("unbound prefix argument")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
            .join(",");
        let expected_params = inputs
            .iter()
            .enumerate()
            .map(|(i, (_, t))| format!("{}:{t}", i + 1))
            .collect::<Vec<_>>()
            .join("|");
        if observed[3] != row[3]
            || observed[4] != expected_params
            || observed[5] != expected_args
            || observed[6] != row[6]
        {
            return Err("prefix target, argument or return correspondence failed".into());
        }
        let parts = def[1].split("::").collect::<Vec<_>>();
        let module = parts[1..parts.len() - 1].join("::");
        if module.is_empty() {
            args.push("--verify-root".into());
        } else {
            args.extend(["--verify-only-module".into(), module]);
        }
        args.extend(["--verify-function".into(), id.clone()]);
        let verification = runner::prove(sdk, inv, &args, &case, "check")?;
        if !case.join("check-vir/crate.vir").is_file() {
            checks.push(json!({"caller":caller["owner"],"status":"prefix_not_analyzed","verification":verification,"evidence":case}));
            continue;
        }
        let checked = vir::checked_target_in(
            &fs::read_to_string(case.join("check-vir/crate.vir"))?,
            def[1],
            &snapshot,
            &inv.cwd,
        )?
        .ok_or("generated prefix not checked")?;
        let attempted = verification["summary"]
            .as_array()
            .is_some_and(|v| v.len() == 2 && v.iter().filter_map(Value::as_u64).sum::<u64>() == 1);
        if !attempted
            || !checked["callees"]
                .as_array()
                .is_some_and(|v| v.iter().any(|p| p == &binding["interface"]))
        {
            return Err("prefix verifier scope or target not confirmed".into());
        }
        if checked["existing_preconditions"] != 0 {
            return Err("generated entry acquired preconditions".into());
        }
        checks.push(json!({"caller":caller["owner"],"status":"prefix_checked","scope":"First effect-free entry call; full caller not checked; informal safety conditions and reachability not assumed",
        "context_direction":"Overapproximation by scalar entry arguments; no exported caller or modeled trait entry contract; target restricted to scalar arithmetic and constants","original_call":relation["call"],"interface":binding["interface"],"entry_arguments":inputs,"target_requires":requirements.iter().map(contracts::render).collect::<Vec<_>>(),"correspondence":true,"added_entry_assumptions":0,"verification":verification,"target":checked,"evidence":case}));
    }
    write_json(&unit.join("prefix-checks.json"), &checks)?;
    Ok(json!({"evidence":unit.join("prefix-checks.json"),"checks":checks}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires PROJECT_PREFIX_UNIT, PROJECT_CLIENT_CAPTURE, PROJECT_INTERFACE_UNIT, PROJECT_PREFIX_OUT"]
    fn replay_real_entry_prefix() {
        let base = std::path::PathBuf::from(std::env::var("PROJECT_PREFIX_UNIT").unwrap());
        let capture = std::path::PathBuf::from(std::env::var("PROJECT_CLIENT_CAPTURE").unwrap());
        let interfaces = std::path::PathBuf::from(std::env::var("PROJECT_INTERFACE_UNIT").unwrap());
        let out = std::path::PathBuf::from(std::env::var("PROJECT_PREFIX_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let inv: Invocation =
            serde_json::from_slice(&fs::read(capture.join("context.json")).unwrap()).unwrap();
        let native: Value =
            serde_json::from_slice(&fs::read(capture.join("verification-input.json")).unwrap())
                .unwrap();
        let functions =
            vir::functions(&fs::read_to_string(native["vir"].as_str().unwrap()).unwrap()).unwrap();
        let callers: Vec<Value> =
            serde_json::from_slice(&fs::read(base.join("callers.json")).unwrap()).unwrap();
        let relations: Value =
            serde_json::from_slice(&fs::read(interfaces.join("relations.json")).unwrap()).unwrap();
        fs::copy(
            interfaces.join("owners.json"),
            out.join("owner-identities.json"),
        )
        .unwrap();
        let probe = runner::build_probe(&inv, &out).unwrap().unwrap();
        let mut context = inv.clone();
        context
            .env
            .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
        context
            .env
            .insert("CLIENT_PROJECT_PREFIX".into(), "1".into());
        let args = runner::clean(&inv.args, &out.join("original-output"), None, &inv.cwd).unwrap();
        runner::extract_context(&probe, &context, &args, "*", &out, "original-borrow", true)
            .unwrap();
        let sdk = std::path::PathBuf::from(std::env::var("PROJECT_PREFIX_SDK").unwrap());
        let result = run(&inv, &sdk, &out, &relations, &callers, &functions, &probe).unwrap();
        let checked = result["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["status"] == "prefix_checked")
            .unwrap();
        let evidence = Path::new(checked["evidence"].as_str().unwrap());
        let (source, _) = ir::span(checked["target"]["span"].as_str().unwrap(), &inv.cwd).unwrap();
        let relative = source.strip_prefix(evidence.join("package")).unwrap();
        let code = fs::read_to_string(&source).unwrap();
        let api = std::iter::once("crate".to_owned())
            .chain(
                checked["interface"]
                    .as_str()
                    .unwrap()
                    .split("::")
                    .skip(1)
                    .map(|s| format!("r#{s}")),
            )
            .collect::<Vec<_>>()
            .join("::");
        let index = checked["entry_arguments"][0][0].as_str().unwrap();
        let arg = format!("__arg_{index}");
        let call = format!("{api}({arg})");
        assert!(code.contains(&call));
        let mut controls = Vec::new();
        for (name, replacement) in [
            ("omitted_call", arg.clone()),
            ("replaced_argument", format!("{api}(0usize)")),
            (
                "omitted_path",
                format!("if {arg} == 0 {{ {call} }} else {{ 0usize }}"),
            ),
        ] {
            let case = out.join(name);
            fs::create_dir(&case).unwrap();
            let package = case.join("package");
            runner::copy_package(&evidence.join("package"), &package).unwrap();
            fs::write(package.join(relative), code.replace(&call, &replacement)).unwrap();
            let original_root = inv
                .args
                .iter()
                .map(|a| inv.cwd.join(a))
                .find(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
                .unwrap()
                .canonicalize()
                .unwrap();
            let original_package = Path::new(&inv.env["CARGO_MANIFEST_DIR"])
                .canonicalize()
                .unwrap();
            let copied = package.join(original_root.strip_prefix(&original_package).unwrap());
            let args = runner::clean(
                &inv.args,
                &case.join("metadata"),
                Some((&original_root, &copied)),
                &inv.cwd,
            )
            .unwrap();
            runner::extract_context(&probe, &context, &args, "*", &case, "mutant", true).unwrap();
            let data = fs::read_to_string(case.join("mutant.stderr")).unwrap();
            let symbol = checked["target"]["path"]
                .as_str()
                .unwrap()
                .rsplit("::")
                .next()
                .unwrap();
            assert!(
                !data
                    .lines()
                    .filter(|l| l.starts_with("CALLER_PREFIX\t"))
                    .any(|l| l.contains(symbol)),
                "mutated prefix still admitted: {name}"
            );
            controls.push(
                json!({"control":name,"rustc_typechecked":true,"prefix_shape_rejected":true}),
            );
        }
        write_json(&out.join("generation-controls.json"), &controls).unwrap();
        assert!(
            result["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["status"] == "prefix_checked")
        );
    }
}
