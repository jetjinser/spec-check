//! Candidate owner guarantees from a forwarding Option callback. Verify before use.
use crate::{
    Result, ir,
    vir::{self, Node},
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Candidate {
    pub file: PathBuf,
    pub edits: Vec<ir::Edit>,
    pub report: Value,
}

/// Select only resolved calls and retain the compiler owner for correspondence.
pub fn called(
    bodies: &std::collections::BTreeMap<String, Vec<ir::Record>>,
    functions: &[Node],
    inventory: &Value,
    cwd: &Path,
) -> Result<Vec<(String, Candidate)>> {
    let mut selected = std::collections::BTreeMap::new();
    for call in bodies.values().flatten().filter(|r| r["kind"] == "call") {
        let Ok(f) = crate::resource::signature(call, functions, cwd) else {
            continue;
        };
        let identity = path(f).ok_or("missing called wrapper identity")?;
        if selected.contains_key(identity) {
            continue;
        }
        let Some(candidate) = propose(f, functions, cwd)? else {
            continue;
        };
        let (file, coordinates) = ir::span(span(f)?, cwd)?;
        let file = fs::canonicalize(file)?;
        let owners = inventory["records"]
            .as_array()
            .ok_or("missing compiler inventory")?
            .iter()
            .filter(|r| r["kind"] == "owner")
            .filter(|r| {
                r["span"]
                    .as_str()
                    .and_then(|s| ir::span(s, cwd).ok())
                    .is_some_and(|(p, c)| {
                        fs::canonicalize(p).is_ok_and(|p| p == file) && c[..2] == coordinates[..2]
                    })
            })
            .filter_map(|r| r["path"].as_str())
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err("called wrapper has no unique compiler source identity".into());
        };
        selected.insert(identity.to_owned(), ((*owner).to_owned(), candidate));
    }
    if selected.len() > 1 {
        return Err(
            "multiple library summary candidates require independent dependency checks".into(),
        );
    }
    Ok(selected.into_values().collect())
}
fn path(f: &Node) -> Option<&str> {
    f.field("name")?.field("path")?.atom()
}
fn part(n: &Node, i: usize) -> Option<&Node> {
    n.unwrap().list()?.get(i)
}
fn head(n: &Node, name: &str) -> bool {
    part(n, 0).and_then(Node::atom) == Some(">") && part(n, 1).and_then(Node::atom) == Some(name)
}
fn tail(n: &Node) -> Option<&Node> {
    if head(n, "Block") && part(n, 2)?.list()?.is_empty() {
        part(n, 3)
    } else {
        None
    }
}
fn target(call: &Node) -> Option<&str> {
    if !head(call, "Call") {
        return None;
    }
    let xs = call.field("target")?.list()?;
    if xs.get(2)?.list()?.get(1)?.atom() != Some("Static") {
        return None;
    }
    xs.get(3)?.field("path")?.atom()
}
fn span(n: &Node) -> Result<&str> {
    n.list()
        .and_then(|v| v.get(1))
        .and_then(Node::atom)
        .ok_or_else(|| "missing owner-summary source span".into())
}
fn read(n: &Node, cwd: &Path) -> Result<(PathBuf, usize, usize, String)> {
    let origin = span(n)?;
    if !origin.ends_with("(#0)") {
        return Err("macro-generated owner summary unsupported".into());
    }
    let (file, coords) = ir::span(origin, cwd)?;
    let source = fs::read_to_string(&file)?;
    let (a, b) = ir::offsets(&source, coords)?;
    Ok((file, a, b, source[a..b].to_owned()))
}
fn receiver(f: &Node) -> Option<(&Node, &str)> {
    let [p] = f.field("params")?.list()? else {
        return None;
    };
    if p.field("mode")?.atom() != Some("Exec") || part(p.field("name")?, 1)?.atom() != Some("self")
    {
        return None;
    }
    let typ = p.field("typ")?;
    let xs = typ.list()?;
    if xs.get(1)?.atom() != Some("Decorate") || part(xs.get(2)?, 1)?.atom() != Some("Ref") {
        return None;
    }
    let dt = xs.get(4)?.list()?.get(2)?.list()?.get(2)?.atom()?;
    Some((typ, dt))
}

pub fn propose(wrapper: &Node, functions: &[Node], cwd: &Path) -> Result<Option<Candidate>> {
    let Some((receiver_type, receiver_path)) = receiver(wrapper) else {
        return Ok(None);
    };
    if wrapper.field("mode").and_then(Node::atom) != Some("Exec")
        || wrapper
            .field("attrs")
            .and_then(|a| a.field("is_external_body"))
            .and_then(Node::atom)
            != Some("false")
        || wrapper
            .field("require")
            .and_then(Node::list)
            .is_none_or(|v| !v.is_empty())
        || wrapper
            .field("ensure")
            .and_then(|v| part(v, 1))
            .and_then(Node::list)
            .is_none_or(|v| !v.is_empty())
        || wrapper.field("returns").and_then(Node::atom) != Some("None")
    {
        return Ok(None);
    }
    let Some(outer) = wrapper.field("body").and_then(tail) else {
        return Ok(None);
    };
    let Some(combiner_path) = target(outer) else {
        return Ok(None);
    };
    let Some(args) = outer.field("args").and_then(Node::list) else {
        return Ok(None);
    };
    let [_, callback_arg] = args else {
        return Ok(None);
    };
    let Some(callback) = tail(callback_arg) else {
        return Ok(None);
    };
    if !head(callback, "NonSpecClosure")
        || callback
            .field("params")
            .and_then(Node::list)
            .is_none_or(|v| !v.is_empty())
    {
        return Ok(None);
    }
    let Some(call) = callback.field("body").and_then(tail) else {
        return Ok(None);
    };
    let Some(producer_path) = target(call) else {
        return Ok(None);
    };
    let producer = functions
        .iter()
        .find(|f| path(f) == Some(producer_path))
        .ok_or("missing callback producer")?;
    let combiner = functions
        .iter()
        .find(|f| path(f) == Some(combiner_path))
        .ok_or("missing callback consumer")?;
    if receiver(producer).map(|(t, _)| t) != Some(receiver_type)
        || producer
            .field("require")
            .and_then(Node::list)
            .is_none_or(|v| !v.is_empty())
    {
        return Ok(None);
    }
    let Some([argument]) = call.field("args").and_then(Node::list) else {
        return Ok(None);
    };
    let (argument_file, _, _, argument_text) = read(argument, cwd)?;
    if argument_text != "self" {
        return Ok(None);
    }
    let Some(ret) = producer.field("outer_ret") else {
        return Ok(None);
    };
    let Some(return_type) = ret.field("typ").and_then(Node::list) else {
        return Ok(None);
    };
    if return_type
        .get(2)
        .and_then(Node::list)
        .and_then(|v| v.get(2))
        .and_then(Node::atom)
        != Some("core::option::Option")
    {
        return Ok(None);
    }
    let Some([payload]) = return_type.get(3).and_then(Node::list) else {
        return Ok(None);
    };
    if wrapper.field("outer_ret").and_then(|r| r.field("typ")) != Some(payload) {
        return Ok(None);
    }
    let Some(ret_name) = ret
        .field("name")
        .and_then(|n| part(n, 1))
        .and_then(Node::atom)
    else {
        return Ok(None);
    };
    let Some([clause]) = producer
        .field("ensure")
        .and_then(|n| part(n, 1))
        .and_then(Node::list)
    else {
        return Ok(None);
    };
    if !head(clause, "Logical")
        || part(clause, 2)
            .and_then(|n| part(n, 1))
            .and_then(Node::atom)
            != Some("Implies")
    {
        return Ok(None);
    }
    let guard = part(clause, 3).ok_or("missing Some guard")?;
    let consequent = part(clause, 4).ok_or("missing callback consequence")?;
    let (file, a, _, _) = read(wrapper, cwd)?;
    let (guard_file, _, _, guard_text) = read(guard, cwd)?;
    let (post_file, _, _, post) = read(consequent, cwd)?;
    if file != argument_file
        || file != guard_file
        || file != post_file
        || guard_text != format!("{ret_name} is Some")
    {
        return Ok(None);
    }
    // Restrict copied syntax to a single owner relation. Nested binders and other
    // result occurrences need a separate substitution rule.
    let prefix = format!("{ret_name}->0");
    let Some(suffix) = post.strip_prefix(&prefix) else {
        return Ok(None);
    };
    if !Regex::new(r"^\.[A-Za-z_][A-Za-z_0-9]*\(\*self\)$")?.is_match(suffix) {
        return Ok(None);
    }
    let (callback_file, s, e, _) = read(callback_arg, cwd)?;
    let (return_file, _, _, return_text) = read(ret, cwd)?;
    if callback_file != file || return_file != file {
        return Ok(None);
    }
    let (_, _, _, body) = read(call, cwd)?;
    let source = fs::read_to_string(&file)?;
    if source.contains("__spec_owner_") {
        return Err("owner-summary names already present".into());
    }
    let value = "__spec_owner_value";
    let option = "__spec_owner_option";
    let result_post = format!("{value}{suffix}");
    let callback_post = format!("{option} is Some ==> {option}->0{suffix}");
    let method = path(wrapper)
        .ok_or("missing wrapper identity")?
        .rsplit("::")
        .next()
        .unwrap();
    let selector = format!("{}::{method}", receiver_path.rsplit("::").next().unwrap());
    let edits = vec![
        ir::Edit {
            start: a,
            end: a,
            replacement: format!("#[verus_spec({value} => ensures {result_post},)]\n    "),
            origin: span(wrapper)?.into(),
        },
        ir::Edit {
            start: s,
            end: e,
            replacement: format!(
                "|| -> ({option}: {return_text}) ensures {callback_post}, {{ {body} }}"
            ),
            origin: span(callback_arg)?.into(),
        },
    ];
    Ok(Some(Candidate {
        file,
        edits,
        report: json!({"rule":"forwarded_option_owner_relation","wrapper":path(wrapper),"producer":producer_path,
        "combiner":combiner_path,"combiner_guarantees":combiner.field("ensure").map(crate::contracts::render),
        "producer_guarantee":crate::contracts::render(clause),"postcondition":result_post,"callback_postcondition":callback_post,
        "selector":selector,"module":wrapper.field("owning_module").and_then(Node::atom),
        "entry_permissions":0,"new_trusted_declarations":0,"implementation_verification_required":true,"runtime_correspondence_required":true}),
    }))
}

pub fn catalogue(text: &str, cwd: &Path) -> Result<Value> {
    let functions = vir::functions(text)?;
    let mut candidates = Vec::new();
    for f in &functions {
        match propose(f,&functions,cwd) {
            Ok(Some(c))=>candidates.push(json!({"status":"candidate_unverified","source":c.file,"edits":c.edits,"derivation":c.report})),
            Ok(None)=>{},
            Err(e)=>candidates.push(json!({"status":"unsupported","wrapper":path(f),"reason":e.to_string()})),
        }
    }
    Ok(
        json!({"candidates":candidates,"scope":"Proposal only. No guarantee may be consumed before implementation verification and runtime correspondence."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires OWNER_POST_VIR, OWNER_POST_CONTEXT, OWNER_POST_RECORDS and OWNER_POST_INVENTORY"]
    fn called_wrapper_selection_uses_definition_identity() {
        let get = |name: &str| std::env::var(format!("OWNER_POST_{name}")).unwrap();
        let functions = vir::functions(&fs::read_to_string(get("VIR")).unwrap()).unwrap();
        let inv: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(get("CONTEXT")).unwrap()).unwrap();
        let inventory: Value =
            serde_json::from_slice(&fs::read(get("INVENTORY")).unwrap()).unwrap();
        let bodies =
            crate::client::borrow_records(&fs::read_to_string(get("RECORDS")).unwrap()).unwrap();
        let selected = called(&bodies, &functions, &inventory, &inv.cwd).unwrap();
        assert_eq!(selected.len(), 1);
        let crate_name = inv
            .args
            .windows(2)
            .find(|w| w[0] == "--crate-name")
            .unwrap()[1]
            .as_str();
        let carriers = crate::permission_guidance::catalogue(
            &fs::read_to_string(get("VIR")).unwrap(),
            crate_name,
        )
        .unwrap();
        let usages = crate::permission_guidance::usage_inventory(
            &carriers, &functions, &inventory, &inv.cwd,
        )
        .unwrap();
        assert!(!usages["calls"].as_array().unwrap().is_empty());
        if let Ok(out) = std::env::var("OWNER_POST_USAGE_OUT") {
            crate::write_json(Path::new(&out), &usages).unwrap();
        }
        let (owner, candidate) = &selected[0];
        let caller = bodies
            .values()
            .find(|body| {
                body.iter()
                    .any(|r| r["kind"] == "call" && r["target"] == *owner)
            })
            .unwrap();
        let flow = crate::resource_flow::audit(caller, &functions, &carriers, &inv.cwd).unwrap();
        assert_eq!(flow["paths"].as_array().unwrap().len(), 1);
        assert_eq!(flow["paths"][0]["ownership_flow_closed"], true, "{flow}");
        let consumer_block = flow["paths"][0]["outcomes"][0]["call"]["block"]
            .as_str()
            .unwrap();
        let mut omission = caller.clone();
        omission.retain(|r| !(r["kind"] == "call" && r["block"] == consumer_block));
        let missing =
            crate::resource_flow::audit(&omission, &functions, &carriers, &inv.cwd).unwrap();
        assert_eq!(
            missing["paths"][0]["ownership_flow_closed"], false,
            "{missing}"
        );
        if let Ok(out) = std::env::var("OWNER_POST_FLOW_OUT") {
            crate::write_json(
                Path::new(&out),
                &json!({"original":flow,"omitted_consumer_record_control":missing}),
            )
            .unwrap();
        }
        assert_eq!(candidate.report["entry_permissions"], 0);
        let mut renamed = bodies.clone();
        for call in renamed
            .values_mut()
            .flatten()
            .filter(|r| r["kind"] == "call")
        {
            call.insert("target".into(), "untrusted_display_name".into());
        }
        assert_eq!(
            called(&renamed, &functions, &inventory, &inv.cwd).unwrap()[0].0,
            *owner
        );
        let mut missing_owner = inventory.clone();
        missing_owner["records"]
            .as_array_mut()
            .unwrap()
            .retain(|r| r["path"] != *owner);
        assert!(called(&bodies, &functions, &missing_owner, &inv.cwd).is_err());
        for call in renamed
            .values_mut()
            .flatten()
            .filter(|r| r["kind"] == "call")
        {
            call.insert("definition_span".into(), "unresolved".into());
        }
        assert!(
            called(&renamed, &functions, &inventory, &inv.cwd)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    #[ignore = "requires OWNER_POST_VIR, OWNER_POST_FUNCTION, OWNER_POST_CONTEXT, OWNER_POST_SDK and OWNER_POST_OUT"]
    fn actual_wrapper_implementation_checks_candidate() {
        let get = |name: &str| std::env::var(format!("OWNER_POST_{name}")).unwrap();
        let text = fs::read_to_string(get("VIR")).unwrap();
        let functions = vir::functions(&text).unwrap();
        let function = get("FUNCTION");
        let f = functions
            .iter()
            .find(|f| path(f) == Some(&function))
            .unwrap();
        let inv: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(get("CONTEXT")).unwrap()).unwrap();
        let mut candidate = propose(f, &functions, &inv.cwd).unwrap().unwrap();
        let out = PathBuf::from(get("OUT"));
        fs::create_dir(&out).unwrap();
        let original = candidate
            .file
            .ancestors()
            .find(|p| p.join("Cargo.toml").exists())
            .unwrap();
        let snapshot = out.join("package");
        crate::runner::copy_package(original, &snapshot).unwrap();
        let negative = std::env::var_os("OWNER_POST_WRONG_RELATION").is_some();
        if negative {
            let post = candidate.report["postcondition"].as_str().unwrap();
            candidate.edits[0].replacement = candidate.edits[0]
                .replacement
                .replace(post, &format!("!({post})"));
        }
        let mut source = fs::read_to_string(&candidate.file).unwrap();
        candidate.edits.sort_by_key(|e| e.start);
        for e in candidate.edits.iter().rev() {
            source.replace_range(e.start..e.end, &e.replacement);
        }
        fs::write(
            snapshot.join(candidate.file.strip_prefix(original).unwrap()),
            source,
        )
        .unwrap();
        let roots = inv
            .args
            .iter()
            .map(|a| inv.cwd.join(a))
            .filter(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
            .collect::<Vec<_>>();
        let [root] = roots.as_slice() else {
            panic!("expected one captured root")
        };
        let package = Path::new(&inv.env["CARGO_MANIFEST_DIR"]);
        let copied = snapshot.join(root.strip_prefix(package).unwrap());
        let mut args = crate::runner::clean(
            &inv.args,
            &out.join("output"),
            Some((root, &copied)),
            &inv.cwd,
        )
        .unwrap();
        let module = candidate.report["module"]
            .as_str()
            .unwrap()
            .split("::")
            .skip(1)
            .collect::<Vec<_>>()
            .join("::");
        args.extend([
            "--verify-only-module".into(),
            module,
            "--verify-function".into(),
            candidate.report["selector"].as_str().unwrap().into(),
        ]);
        let checked =
            crate::runner::prove(Path::new(&get("SDK")), &inv, &args, &out, "implementation")
                .unwrap();
        assert_eq!(
            checked["status"],
            if negative { "not_proved" } else { "proved" },
            "{checked}"
        );
        assert!(
            checked["summary"][0].as_u64().unwrap() + checked["summary"][1].as_u64().unwrap() > 0
        );
        crate::write_json(&out.join("candidate.json"), &candidate.report).unwrap();
        crate::write_json(&out.join("edits.json"), &candidate.edits).unwrap();
        if !negative && let Ok(owner) = std::env::var("OWNER_POST_COMPILER_PATH") {
            let probe = crate::runner::build_probe(&inv, &out).unwrap().unwrap();
            let selection = out.join("selection.txt");
            fs::write(&selection, &owner).unwrap();
            let mut context = inv.clone();
            context.env.insert(
                "CLIENT_PROJECT_BORROW_PATHS".into(),
                selection.to_string_lossy().into_owned(),
            );
            context
                .env
                .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
            let before_root = original.join(root.strip_prefix(package).unwrap());
            let before_args = crate::runner::clean(
                &inv.args,
                &out.join("before-output"),
                Some((root, &before_root)),
                &inv.cwd,
            )
            .unwrap();
            let after_args = crate::runner::clean(
                &inv.args,
                &out.join("after-output"),
                Some((root, &copied)),
                &inv.cwd,
            )
            .unwrap();
            for (name, args) in [("before", before_args), ("after", after_args)] {
                crate::runner::extract_context(&probe, &context, &args, "*", &out, name, true)
                    .unwrap();
            }
            let before = crate::client::borrow_records(
                &fs::read_to_string(out.join("before.stderr")).unwrap(),
            )
            .unwrap();
            let after = crate::client::borrow_records(
                &fs::read_to_string(out.join("after.stderr")).unwrap(),
            )
            .unwrap();
            let paths = std::collections::BTreeSet::from([owner]);
            let report = crate::ordinary::correspondence_report(&before, &after, &paths);
            crate::write_json(&out.join("correspondence.json"), &report).unwrap();
            assert_eq!(report["matches"], true, "{report}");
        }
    }
}
