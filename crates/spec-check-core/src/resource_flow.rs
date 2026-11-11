//! Normal-path ownership flow for directly returned permission carriers.
use crate::{
    Result,
    ir::Record,
    permission_guidance::{Carrier, direct_carrier},
    resource,
    vir::Node,
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};

pub(crate) fn verify_consumers(
    flow: &Value,
    functions: &[Node],
    carriers: &[Carrier],
    inv: &crate::capture::Invocation,
    args: &[String],
    sdk: &Path,
    out: &Path,
) -> Result<Value> {
    let mut targets = BTreeMap::new();
    let mut handoffs = BTreeMap::new();
    for path in flow["paths"].as_array().ok_or("missing ownership paths")? {
        for outcome in path["outcomes"]
            .as_array()
            .ok_or("missing ownership outcomes")?
        {
            if outcome["status"] != "moved_to_native_body_consumer" {
                continue;
            }
            let call: Record = serde_json::from_value(outcome["call"].clone())?;
            let f = resource::signature(&call, functions, &inv.cwd)?;
            let name = f
                .field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
                .ok_or("missing consumer identity")?;
            let receiver = f
                .field("params")
                .and_then(Node::list)
                .and_then(|ps| ps.first())
                .ok_or("missing consumer receiver")?;
            let is_method = receiver
                .field("name")
                .and_then(Node::list)
                .and_then(|xs| xs.get(1))
                .and_then(Node::atom)
                == Some("self");
            if !is_method {
                return Err("resource consumer proof currently requires an owned receiver".into());
            }
            let carrier = path["carrier"].as_str().ok_or("missing consumer type")?;
            let carrier_info = carriers
                .iter()
                .find(|c| c.path == carrier)
                .ok_or("missing carrier metadata")?;
            handoffs.insert(
                name.to_owned(),
                match crate::handoff::trace(f, carrier_info, functions) {
                    Ok(trace) => json!({"status":"traced","trace":trace}),
                    Err(error) => json!({"status":"unsupported","reason":error.to_string()}),
                },
            );
            let selector = format!(
                "{}::{}",
                carrier.rsplit("::").next().unwrap(),
                name.rsplit("::").next().unwrap()
            );
            let module = f
                .field("owning_module")
                .and_then(Node::atom)
                .ok_or("missing consumer module")?;
            targets.insert(name.to_owned(), (selector, module.to_owned()));
        }
    }
    let mut pending = targets.keys().cloned().collect::<Vec<_>>();
    while let Some(target) = pending.pop() {
        let f = functions
            .iter()
            .find(|f| {
                f.field("name")
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    == Some(&target)
            })
            .ok_or("missing consumer body")?;
        let mut calls = Vec::new();
        crate::vir::walk_calls(f.field("body").ok_or("missing consumer body")?, &mut calls);
        for callee in calls {
            if targets.contains_key(&callee) {
                continue;
            }
            let Some(f) = functions.iter().find(|f| {
                f.field("name")
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    == Some(&callee)
            }) else {
                continue;
            };
            if f.field("mode").and_then(Node::atom) != Some("Exec")
                || !f.field("params").and_then(Node::list).is_some_and(|ps| {
                    ps.iter().any(|p| {
                        p.field("typ")
                            .is_some_and(crate::permission_guidance::tracked_permission)
                    })
                })
            {
                continue;
            }
            if f.field("attrs")
                .and_then(|a| a.field("is_external_body"))
                .and_then(Node::atom)
                != Some("false")
                || f.field("body").is_none_or(|b| b.list().is_none())
            {
                return Err("permission handoff depends on an unchecked native body".into());
            }
            let module = f
                .field("owning_module")
                .and_then(Node::atom)
                .ok_or("missing handoff module")?;
            targets.insert(
                callee.clone(),
                (
                    callee.rsplit("::").next().unwrap().to_owned(),
                    module.to_owned(),
                ),
            );
            pending.push(callee);
            if targets.len() > 32 {
                return Err("permission handoff proof budget exceeded".into());
            }
        }
    }
    let mut checks = Vec::new();
    for (i, (target, (selector, module))) in targets.into_iter().enumerate() {
        let mut selected = args.to_vec();
        let module = module.split("::").skip(1).collect::<Vec<_>>().join("::");
        if module.is_empty() {
            selected.push("--verify-root".into());
        } else {
            selected.extend(["--verify-only-module".into(), module]);
        }
        selected.extend(["--verify-function".into(), selector]);
        let checked =
            crate::runner::prove(sdk, inv, &selected, out, &format!("resource-consumer-{i}"))?;
        checks.push(json!({"native_function":target,"body_verified":checked["status"]=="proved" && checked["summary"]==json!([1,0]),"verification":checked}));
    }
    let verified = checks
        .iter()
        .filter(|c| c["body_verified"] == true)
        .filter_map(|c| c["native_function"].as_str())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for handoff in handoffs.values_mut() {
        if let Some(trace) = handoff.get("trace") {
            let mut names = BTreeSet::new();
            crate::handoff::functions(trace, &mut names);
            handoff["native_body_proofs_complete"] =
                json!(!names.is_empty() && names.is_subset(&verified));
            if handoff["native_body_proofs_complete"] == true {
                handoff["status"] = json!("traced_and_body_checked");
            }
        }
    }
    Ok(
        json!({"checks":checks,"permission_handoffs":handoffs,"scope":"Body proofs and straight-line field-to-atomic-invariant transfer evidence are separate. Existing helper contracts and primitive dependencies remain in force; conditional transfers, implicit destruction and unwind cleanup are unsupported."}),
    )
}

pub(crate) fn audit(
    records: &[Record],
    functions: &[Node],
    carriers: &[Carrier],
    cwd: &Path,
) -> Result<Value> {
    let blocks = Regex::new(r"bb\d+")?;
    let normal = Regex::new(r"\[return: (bb\d+)")?;
    let movement = Regex::new(r"^(_\d+) = move (_\d+)$")?;
    let mut paths = Vec::new();
    for acquisition in records.iter().filter(|r| r["kind"] == "call") {
        let Ok(producer) = resource::signature(acquisition, functions, cwd) else {
            continue;
        };
        let Some(carrier) = producer
            .field("outer_ret")
            .and_then(|r| r.field("typ"))
            .and_then(|t| direct_carrier(t, carriers))
        else {
            continue;
        };
        let term = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == acquisition["block"])
            .ok_or("missing acquisition terminator")?;
        let Some(entry) = normal.captures(&term["operation"]) else {
            continue;
        };
        let mut todo = VecDeque::from([(entry[1].to_owned(), acquisition["destination"].clone())]);
        let mut seen = BTreeSet::new();
        let mut outcomes = Vec::new();
        while let Some((block, mut place)) = todo.pop_front() {
            if !seen.insert((block.clone(), place.clone())) {
                continue;
            }
            if seen.len() > 256 {
                return Err("resource flow state budget exceeded".into());
            }
            let mut statements = records
                .iter()
                .filter(|r| r["kind"] == "stmt" && r["block"] == block)
                .collect::<Vec<_>>();
            statements.sort_by_key(|r| r["statement_index"].parse::<usize>().unwrap());
            for statement in statements {
                if let Some(m) = movement.captures(&statement["operation"])
                    && m[2] == place
                {
                    place = m[1].to_owned();
                } else if statement["operation"] == format!("StorageDead({place})")
                    || statement["operation"].starts_with(&format!("{place} = "))
                {
                    place = "<lost>".into();
                }
            }
            let term = records
                .iter()
                .find(|r| r["kind"] == "term" && r["block"] == block)
                .ok_or("missing resource flow terminator")?;
            if term["operation"] == "return" {
                outcomes.push(json!({"status":if place=="_0" {"returned_to_caller"} else {"unconsumed_on_normal_exit"},"place":place,"exit":term}));
                continue;
            }
            if let Some(call) = records
                .iter()
                .find(|r| r["kind"] == "call" && r["block"] == block)
            {
                let args = resource::arguments(call)?;
                if let Some(index) = args.iter().position(|a| a == &format!("move {place}")) {
                    let resolved = resource::signature(call, functions, cwd);
                    let consumes = resolved.as_ref().ok().is_some_and(|f| {
                        f.field("params")
                            .and_then(Node::list)
                            .and_then(|ps| ps.get(index))
                            .and_then(|p| p.field("typ"))
                            .and_then(|t| direct_carrier(t, carriers))
                            == Some(carrier)
                            && f.field("attrs")
                                .and_then(|a| a.field("is_external_body"))
                                .and_then(Node::atom)
                                == Some("false")
                            && f.field("body").is_some_and(|b| b.list().is_some())
                            && f.field("outer_ret")
                                .and_then(|r| r.field("typ"))
                                .is_some_and(|t| !mentions(t, carrier))
                    });
                    outcomes.push(json!({"status":if consumes {"moved_to_native_body_consumer"} else {"unsupported_resource_escape"},"call":call}));
                    continue;
                }
                // A call's destination is initialized only on its normal return edge.
                if let Some(ret) = normal.captures(&term["operation"]) {
                    if call["destination"] == place {
                        outcomes.push(json!({"status":"unsupported_overwrite","call":call}));
                    } else {
                        todo.push_back((ret[1].to_owned(), place));
                    }
                }
                continue;
            }
            if term["operation"].starts_with("drop(") {
                outcomes
                    .push(json!({"status":"unsupported_implicit_destructor","terminator":term}));
                continue;
            }
            if let Some(edge) = records
                .iter()
                .find(|r| r["kind"] == "edge" && r["block"] == block)
            {
                for next in blocks.find_iter(&edge["successors"]) {
                    todo.push_back((next.as_str().into(), place.clone()));
                }
            }
        }
        let closed = !outcomes.is_empty()
            && outcomes.iter().all(|r| {
                matches!(
                    r["status"].as_str(),
                    Some("returned_to_caller" | "moved_to_native_body_consumer")
                )
            });
        paths.push(json!({"acquisition":acquisition,"carrier":carrier,"ownership_flow_closed":closed,"outcomes":outcomes}));
    }
    Ok(
        json!({"scope":"Direct carrier returns and local moves on normal paths. Consumption means a move into a source-bound native body, subject to its library proof; physical release, nested carriers, implicit destructors and unwind cleanup are not established by this audit.","paths":paths}),
    )
}
fn mentions(n: &Node, path: &str) -> bool {
    n.atom() == Some(path)
        || n.list()
            .is_some_and(|xs| xs.iter().any(|n| mentions(n, path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{capture::Invocation, client, ir, permission_guidance, vir, write_json};
    use std::{fs, path::PathBuf};
    #[test]
    #[ignore = "requires FLOW_CASE, FLOW_CONTEXT, FLOW_MUTATED and FLOW_OUT from actual compiler runs"]
    fn actual_source_omission_has_no_consumer() {
        let get = |n: &str| PathBuf::from(std::env::var(format!("FLOW_{n}")).unwrap());
        let case = get("CASE");
        let mutation = get("MUTATED");
        let inv: Invocation = serde_json::from_slice(&fs::read(get("CONTEXT")).unwrap()).unwrap();
        let text = fs::read_to_string(case.join("check-vir/crate.vir")).unwrap();
        let functions = vir::functions(&text).unwrap();
        let name = &inv
            .args
            .windows(2)
            .find(|w| w[0] == "--crate-name")
            .unwrap()[1];
        let carriers = permission_guidance::catalogue(&text, name).unwrap();
        let selected = fs::read_to_string(case.join("selection.txt")).unwrap();
        let before =
            client::borrow_records(&fs::read_to_string(case.join("generated.stderr")).unwrap())
                .unwrap();
        let mut after =
            client::borrow_records(&fs::read_to_string(mutation.join("mutated.stderr")).unwrap())
                .unwrap();
        let old = case.join("package");
        let new = mutation.join("package");
        // The control changes the caller only. Check complete native callee
        // source ranges before mapping its cloned paths to the captured VIR.
        for call in after
            .get_mut(&selected)
            .unwrap()
            .iter_mut()
            .filter(|r| r["kind"] == "call")
        {
            call.insert(
                "definition_span".into(),
                call["definition_span"].replace(new.to_str().unwrap(), old.to_str().unwrap()),
            );
            if let Ok(f) = resource::signature(call, &functions, &inv.cwd) {
                let origin = f.list().unwrap()[1].atom().unwrap();
                let (file, coords) = ir::span(origin, &inv.cwd).unwrap();
                if let Ok(relative) = file.strip_prefix(&old) {
                    let original = fs::read_to_string(&file).unwrap();
                    let changed = fs::read_to_string(new.join(relative)).unwrap();
                    let (a, b) = ir::offsets(&original, coords).unwrap();
                    let (x, y) = ir::offsets(&changed, coords).unwrap();
                    assert_eq!(&original[a..b], &changed[x..y]);
                }
            }
        }
        let positive = audit(&before[&selected], &functions, &carriers, &inv.cwd).unwrap();
        let negative = audit(&after[&selected], &functions, &carriers, &inv.cwd).unwrap();
        assert_eq!(positive["paths"].as_array().unwrap().len(), 1);
        assert_eq!(
            positive["paths"][0]["ownership_flow_closed"], true,
            "{positive}"
        );
        assert_eq!(negative["paths"].as_array().unwrap().len(), 1);
        assert_eq!(
            negative["paths"][0]["ownership_flow_closed"], false,
            "{negative}"
        );
        let call: Record =
            serde_json::from_value(positive["paths"][0]["outcomes"][0]["call"].clone()).unwrap();
        let consumer = resource::signature(&call, &functions, &inv.cwd).unwrap();
        let carrier = carriers
            .iter()
            .find(|c| Some(c.path.as_str()) == positive["paths"][0]["carrier"].as_str())
            .unwrap();
        let handoff = crate::handoff::trace(consumer, carrier, &functions).unwrap();
        let mut names = BTreeSet::new();
        crate::handoff::functions(&handoff, &mut names);
        assert_eq!(names.len(), 3, "{handoff}");
        fn omit_option_assignment(n: &mut Node) -> bool {
            if let Node::List(xs) = n {
                if xs.first().and_then(Node::atom) == Some(">")
                    && xs.get(1).and_then(Node::atom) == Some("Assign")
                    && let Some(i) = xs.iter().position(|n| n.atom() == Some(":rhs"))
                {
                    let mut rhs = &mut xs[i + 1];
                    loop {
                        if rhs
                            .list()
                            .and_then(|xs| xs.first())
                            .and_then(Node::atom)
                            .is_some_and(|s| matches!(s, "@" | "@@"))
                        {
                            let Node::List(xs) = rhs else { unreachable!() };
                            rhs = &mut xs[2];
                        } else {
                            break;
                        }
                    }
                    if let Node::List(ctor) = rhs
                        && ctor.get(1).and_then(Node::atom) == Some("Ctor")
                        && ctor.get(3).and_then(Node::atom) == Some("Some")
                    {
                        ctor[3] = Node::Atom("None".into());
                        ctor[4] = Node::List(Vec::new());
                        return true;
                    }
                }
                return xs.iter_mut().any(omit_option_assignment);
            }
            false
        }
        let mut changed = functions.clone();
        assert!(
            changed
                .iter_mut()
                .filter(|f| f
                    .field("name")
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    .is_some_and(|p| names.contains(p)))
                .any(omit_option_assignment)
        );
        assert!(crate::handoff::trace(consumer, carrier, &changed).is_err());
        if let Ok(path) = std::env::var("FLOW_NEGATIVE_VIR") {
            let negative_functions = vir::functions(&fs::read_to_string(path).unwrap()).unwrap();
            let name = consumer
                .field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
                .unwrap();
            let negative_consumer = negative_functions
                .iter()
                .find(|f| {
                    f.field("name")
                        .and_then(|n| n.field("path"))
                        .and_then(Node::atom)
                        == Some(name)
                })
                .unwrap();
            assert!(
                crate::handoff::trace(negative_consumer, carrier, &negative_functions).is_err()
            );
        }
        if let Ok(path) = std::env::var("FLOW_HANDOFF_OUT") {
            write_json(Path::new(&path), &handoff).unwrap();
        }
        if let Ok(sdk) = std::env::var("FLOW_SDK") {
            let out = get("CONSUMER_OUT");
            fs::create_dir(&out).unwrap();
            let command: Value =
                serde_json::from_slice(&fs::read(case.join("generated.command.json")).unwrap())
                    .unwrap();
            let args = command["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            let args = crate::runner::clean(&args, &out.join("output"), None, &inv.cwd).unwrap();
            let checked = verify_consumers(
                &positive,
                &functions,
                &carriers,
                &inv,
                &args,
                Path::new(&sdk),
                &out,
            )
            .unwrap();
            write_json(&out.join("report.json"), &checked).unwrap();
            assert!(!checked["checks"].as_array().unwrap().is_empty());
            assert!(
                checked["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["body_verified"] == true),
                "{checked}"
            );
        }
        write_json(&get("OUT"),&json!({"original":positive,"source_omission":negative,"scope":"Actual recompiled omission control. Cloned native callees checked byte-for-byte before source identity normalization."})).unwrap();
    }
}
