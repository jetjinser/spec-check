//! Checked membership candidates from a single set insertion/removal contract.
use crate::{
    Result,
    guidance::source,
    hash,
    ir::{Edit, Record},
    state_semantics as sem,
    vir::Node,
};
use regex::Regex;
use serde_json::{Value, json};
use std::{fs, path::Path};

struct Transition<'a> {
    function: &'a Node,
    present: bool,
    clause: &'a Node,
}

fn declared_call(node: &Node) -> Option<&str> {
    node.field("target")?.list()?.get(3)?.field("path")?.atom()
}

fn one_local(node: &Node) -> Option<&Node> {
    let names = sem::nodes(node, "VarIdent");
    let first = *names.first()?;
    names.iter().all(|n| *n == first).then_some(first)
}

fn invariant(node: &Node, receiver: &Node, final_state: bool) -> bool {
    declared_call(node) == Some("vstd_extra::ownership::Inv::inv")
        && sem::call_args(node).len() == 1
        && one_local(&sem::call_args(node)[0]) == Some(receiver)
        && (if final_state {
            sem::has_final_self(node, receiver)
        } else {
            !sem::has_final_self(node, receiver) && !sem::has_old_self(node, receiver)
        })
}

fn discover(functions: &[Node]) -> Vec<Transition<'_>> {
    let mut result = Vec::new();
    for f in functions {
        if f.field("mode").and_then(Node::atom) != Some("Exec")
            || f.field("body").and_then(Node::list).is_none()
            || f.field("attrs")
                .and_then(|n| n.field("is_external_body"))
                .and_then(Node::atom)
                != Some("false")
        {
            continue;
        }
        let Some(params) = f.field("params").and_then(Node::list) else {
            continue;
        };
        let [state, key] = params else { continue };
        let (Some(state), Some(key)) = (state.field("name"), key.field("name")) else {
            continue;
        };
        let Some(requires) = f.field("require").and_then(Node::list) else {
            continue;
        };
        if !requires.iter().any(|n| invariant(n, state, false)) {
            continue;
        }
        let Some(clauses) = f
            .field("ensure")
            .and_then(Node::list)
            .and_then(|n| n.get(1))
            .and_then(Node::list)
        else {
            continue;
        };
        if !clauses.iter().any(|n| invariant(n, state, true)) {
            continue;
        }
        for clause in clauses {
            let Some((left, right)) = sem::binary_eq(clause) else {
                continue;
            };
            let present = match sem::call_path(right) {
                Some("vstd::set::Set::insert") => true,
                Some("vstd::set::Set::remove") => false,
                _ => continue,
            };
            let args = sem::call_args(right);
            if args.len() != 2
                || !sem::final_view(left, state)
                || !sem::old_view(&args[0], state)
                || declared_call(left) != Some("vstd::view::View::view")
                || declared_call(&args[0]) != Some("vstd::view::View::view")
                || declared_call(&args[1]) != Some("vstd::view::View::view")
                || sem::local_read(&args[1]) != Some(key)
                || sem::has_old_self(&args[1], key)
                || sem::has_final_self(&args[1], key)
            {
                continue;
            }
            result.push(Transition {
                function: f,
                present,
                clause,
            });
        }
    }
    result
}

pub fn propose(
    records: &[Record],
    functions: &[Node],
    cwd: &Path,
    pkg: &Path,
) -> Result<(Vec<Edit>, Vec<Value>)> {
    let patterns = discover(functions);
    let header = Regex::new(
        r"^for\s+(?:_|[A-Za-z_]\w*)\s+in\s+0(?:usize)?\s*\.\.\s*(\d+(?:usize)?|[A-Za-z_]\w*)\s*\{",
    )?;
    let method = Regex::new(r"^([A-Za-z_]\w*)\.[A-Za-z_]\w*\(\s*([A-Za-z_]\w*)\s*\)$")?;
    let arg = Regex::new(r"Spanned \{ node: (?:move|copy) (_\d+), span: [^}]+\}")?;
    let mut candidates = Vec::new();
    for loop_ in records
        .iter()
        .filter(|r| r["kind"] == "source_expr" && r["expression_kind"] == "loop")
    {
        let (file, start, end, text) = source(&loop_["span"], cwd)?;
        let Some(h) = header.captures(&text) else {
            continue;
        };
        let bound = &h[1];
        let full = fs::read_to_string(&file)?;
        if !file.starts_with(pkg) || full[..start].trim_end().ends_with(']') {
            continue;
        }
        if !records.iter().any(|r| {
            r["kind"] == "source_argument"
                && r["call_span"] == loop_["span"]
                && r["type"] == "std::ops::Range<usize>"
        }) {
            continue;
        }
        for transition in &patterns {
            let span =
                sem::span(transition.function).ok_or("state transition source span missing")?;
            for call in records.iter().filter(|r| r["kind"] == "call") {
                if crate::ir::span(&call["definition_span"], cwd).ok()
                    != crate::ir::span(span, cwd).ok()
                {
                    continue;
                }
                let Some(term) = records
                    .iter()
                    .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                else {
                    continue;
                };
                let (p, a, b, call_text) = source(&term["span"], cwd)?;
                if p != file || a < start || b > end {
                    continue;
                }
                let Some(m) = method.captures(&call_text) else {
                    continue;
                };
                let (receiver, key) = (&m[1], &m[2]);
                let bindings = |name: &str| {
                    records
                        .iter()
                        .filter(|r| r["kind"] == "binding" && r["name"] == name)
                        .collect::<Vec<_>>()
                };
                let states = bindings(receiver);
                let keys = bindings(key);
                let ([state], [key_binding]) = (states.as_slice(), keys.as_slice()) else {
                    continue;
                };
                if source(&key_binding["span"], cwd)?.3.trim() != key {
                    continue;
                }

                let arguments = arg
                    .captures_iter(&call["arguments"])
                    .map(|c| c[1].to_owned())
                    .collect::<Vec<_>>();
                if arguments.len() != 2 {
                    continue;
                }
                let loans = records
                    .iter()
                    .filter(|r| {
                        r["kind"] == "loan"
                            && r["assigned_place"] == arguments[0]
                            && r["borrow_kind"].starts_with("Mut ")
                            && r["borrowed_place"] == state["place"]
                    })
                    .collect::<Vec<_>>();
                if loans.len() != 1 {
                    continue;
                }
                let key_copy = Regex::new(&format!(
                    r"^{} = copy {}$",
                    regex::escape(&arguments[1]),
                    regex::escape(&key_binding["place"])
                ))?;
                if !records.iter().any(|r| {
                    r["kind"] == "stmt"
                        && r["block"] == call["block"]
                        && key_copy.is_match(&r["operation"])
                }) {
                    continue;
                }
                let write = Regex::new(&format!(
                    r"^\(?{}\b.*=",
                    regex::escape(&key_binding["place"])
                ))?;
                if records
                    .iter()
                    .any(|r| r["kind"] == "stmt" && write.is_match(&r["operation"]))
                    || records.iter().any(|r| {
                        r["kind"] == "loan"
                            && r["borrowed_place"] == key_binding["place"]
                            && r["borrow_kind"].starts_with("Mut ")
                    })
                {
                    continue;
                }
                let mutators = records
                    .iter()
                    .filter(|r| {
                        r["kind"] == "loan"
                            && r["borrow_kind"].starts_with("Mut ")
                            && r["borrowed_place"] == state["place"]
                    })
                    .filter(|r| r["assigned_place"] != arguments[0])
                    .count();
                if mutators != 0 {
                    continue;
                }
                let constructors = records
                    .iter()
                    .filter(|r| r["kind"] == "call" && r["destination"] == state["place"])
                    .collect::<Vec<_>>();
                if constructors.len() != 1 {
                    continue;
                }
                let ctor = constructors[0];
                let Some(ctor_term) = records
                    .iter()
                    .find(|r| r["kind"] == "term" && r["block"] == ctor["block"])
                else {
                    continue;
                };
                if source(&ctor_term["span"], cwd)?.2 >= start {
                    continue;
                }
                let iter = format!("__spec_set_iter_{start}");
                if full.contains(&iter) {
                    return Err("set guidance identifier collision".into());
                }
                let membership = format!("({receiver})@.contains(({key})@)");
                let clauses = vec![
                    format!("vstd_extra::ownership::Inv::inv({receiver})"),
                    format!(
                        "{iter}.index() > 0 ==> {}{membership}",
                        if transition.present { "" } else { "!" }
                    ),
                ];
                let edit = Edit {
                    start,
                    end: start,
                    replacement: format!(
                        "#[vstd::prelude::verus_spec({iter} => invariant {},)]\n",
                        clauses.join(", ")
                    ),
                    origin: loop_["span"].clone(),
                };
                candidates.push((edit,json!({"kind":"set_membership_state","span":loop_["span"],"bound":bound,
                    "receiver":receiver,"key":key,"receiver_place":state["place"],"key_place":key_binding["place"],
                    "transition":transition.function.field("name").and_then(|n|n.field("path")).and_then(Node::atom),
                    "transition_span":span,"effect":if transition.present {"insert"}else{"remove"},
                    "clause_span":sem::span(transition.clause),"constructor":ctor,
                    "clauses":clauses,"source_sha256":hash(full.as_bytes())?,
                    "scope":"Candidate invariants from an unconditional set update and preserved Inv; Verus checks initialization, preservation and original assertions"})));
            }
        }
    }
    if candidates.len() != 1 {
        return Ok((vec![], vec![]));
    }
    let (edit, report) = candidates.remove(0);
    Ok((vec![edit], vec![report]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a CpuSet development baseline"]
    fn real_set_transition_candidate() {
        let root = std::path::PathBuf::from(std::env::var("SPEC_SET_TARGET").unwrap());
        let context: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(root.join("context.json")).unwrap()).unwrap();
        let raw: Value =
            serde_json::from_slice(&fs::read(root.join("sample-1/original.json")).unwrap())
                .unwrap();
        let records: Vec<Record> = serde_json::from_value(raw["records"].clone()).unwrap();
        let functions = crate::vir::functions(
            &fs::read_to_string(root.join("baseline-vir/crate.vir")).unwrap(),
        )
        .unwrap();
        let (edits, reports) = propose(&records, &functions, &context.cwd, &context.cwd).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(reports[0]["effect"], "insert");
        let mut aliased = records.clone();
        let key_place = reports[0]["key_place"].as_str().unwrap();
        aliased.push(Record::from([
            ("kind".into(), "loan".into()),
            ("borrow_kind".into(), "Mut { kind: Default }".into()),
            ("borrowed_place".into(), key_place.into()),
            ("assigned_place".into(), "_999".into()),
        ]));
        assert!(
            propose(&aliased, &functions, &context.cwd, &context.cwd)
                .unwrap()
                .0
                .is_empty()
        );
        let mut weak = functions.clone();
        let path = reports[0]["transition"].as_str().unwrap();
        let selected = weak
            .iter_mut()
            .find(|f| {
                f.field("name")
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    == Some(path)
            })
            .unwrap();
        fn field_mut<'a>(n: &'a mut Node, key: &str) -> &'a mut Node {
            let Node::List(v) = n else { panic!("list") };
            if matches!(v[0].atom(), Some("@" | "@@")) {
                return field_mut(&mut v[2], key);
            }
            let at = v.iter().position(|n| n.atom() == Some(key)).unwrap();
            &mut v[at + 1]
        }
        let Node::List(ensure) = field_mut(selected, ":ensure") else {
            panic!("ensure")
        };
        let Node::List(clauses) = &mut ensure[1] else {
            panic!("clauses")
        };
        clauses.remove(0);
        assert!(!discover(&weak).iter().any(|p| {
            p.function
                .field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
                == Some(path)
        }));
        let out = std::path::PathBuf::from(std::env::var("SPEC_SET_OUT").unwrap());
        fs::create_dir_all(&out).unwrap();
        fs::write(
            out.join("edits.json"),
            serde_json::to_vec_pretty(&edits).unwrap(),
        )
        .unwrap();
        fs::write(
            out.join("report.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
}
