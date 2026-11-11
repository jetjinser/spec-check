//! Candidate state parameterization; acceptance still requires correspondence evidence.
use crate::{Result, hash, ir, std_body};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

const STATE: &str = "__spec_callback_state";
const EACH: &str = "__spec_each_state";
const DISPATCH: &str = "\nfn __spec_each_state<I: Iterator, S, F: Fn(&mut S, I::Item)>(iter: I, state: &mut S, callback: F) {\n    for item in iter { callback(state, item); }\n}\n";

pub struct Proposal {
    pub source: String,
    pub report: Value,
}

fn range(span: &str, cwd: &Path, file: &Path, source: &str) -> Result<(usize, usize)> {
    let (path, coordinates) = ir::span(span, cwd)?;
    if path != file {
        return Err("callback spans different source files".into());
    }
    ir::offsets(source, coordinates)
}

fn call_contexts(
    target: &str,
    bodies: &BTreeMap<String, Vec<ir::Record>>,
    cwd: &Path,
) -> Result<Value> {
    let callee = &bodies[target];
    let Some(count) = callee.iter().find(|r| r["kind"] == "parameters") else {
        return Ok(json!({"status":"parameter_metadata_unavailable"}));
    };
    let count = count["count"].parse::<usize>()?;
    let mut parameters = callee
        .iter()
        .filter(|r| r["kind"] == "parameter")
        .collect::<Vec<_>>();
    parameters.sort_by_key(|r| r["index"].parse::<usize>().unwrap_or(usize::MAX));
    if parameters.len() != count
        || parameters
            .iter()
            .enumerate()
            .any(|(i, r)| r["index"] != i.to_string())
    {
        return Err("helper parameter identities are incomplete".into());
    }
    let declaration = callee
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("helper declaration missing")?;
    let mut contexts = Vec::new();
    for (owner, records) in bodies {
        for call in records
            .iter()
            .filter(|r| r["kind"] == "call" && r["target"] == target)
        {
            if call["definition_span"] != declaration["span"] {
                return Err("helper call declaration differs from extracted body".into());
            }
            let term = records
                .iter()
                .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                .ok_or("helper call terminator missing")?;
            let mut arguments = records
                .iter()
                .filter(|r| r["kind"] == "source_argument" && r["call_span"] == term["span"])
                .collect::<Vec<_>>();
            arguments.sort_by_key(|r| r["index"].parse::<usize>().unwrap_or(usize::MAX));
            if arguments.len() != count
                || arguments
                    .iter()
                    .enumerate()
                    .any(|(i, r)| r["index"] != i.to_string())
            {
                contexts.push(
                    json!({"owner":owner,"call":call,"status":"argument_metadata_unavailable"}),
                );
                continue;
            }
            let mut bindings = Vec::new();
            for (parameter, argument) in parameters.iter().zip(arguments) {
                let (file, coordinates) = ir::span(&argument["span"], cwd)?;
                let source = fs::read_to_string(&file)?;
                let (a, b) = ir::offsets(&source, coordinates)?;
                let names = callee
                    .iter()
                    .filter(|r| r["kind"] == "binding" && r["place"] == parameter["place"])
                    .collect::<Vec<_>>();
                bindings.push(json!({"parameter":parameter,"bindings":names,"argument":argument,"expression":&source[a..b],"source_sha256":hash(source.as_bytes())?}));
            }
            contexts.push(json!({"owner":owner,"call":call,"span":term["span"],"arguments":bindings,"status":"recorded"}));
        }
    }
    Ok(
        json!({"status":"recorded","contexts":contexts,"scope":"Resolved calls and source arguments only; no helper precondition is assumed or inferred."}),
    )
}

pub fn propose(bodies: &BTreeMap<String, Vec<ir::Record>>, cwd: &Path) -> Result<Vec<Proposal>> {
    let mut proposals = Vec::new();
    for (owner, records) in bodies {
        if !records
            .iter()
            .any(|r| r["kind"] == "closure_kind" && r["closure_kind"] == "FnMut")
        {
            continue;
        }
        let captures = records
            .iter()
            .filter(|r| r["kind"] == "capture")
            .collect::<Vec<_>>();
        let [capture] = captures.as_slice() else {
            continue;
        };
        if capture["capture_kind"] != "ByRef(Mutable)" || capture["projection_count"] != "0" {
            continue;
        }
        let Some((parent, _)) = owner.rsplit_once("::{closure#") else {
            continue;
        };
        let parent_name = parent;
        let Some(parent) = bodies.get(parent) else {
            continue;
        };
        if parent
            .iter()
            .any(|r| matches!(r["kind"].as_str(), "container" | "closure_kind"))
        {
            continue;
        }
        let declaration = records
            .iter()
            .find(|r| r["kind"] == "decl")
            .ok_or("callback declaration missing")?;
        let (file, coordinates) = ir::span(&declaration["span"], cwd)?;
        let source = fs::read_to_string(&file)?;
        if source.contains(STATE) || source.contains(EACH) {
            return Err("callback helper name collision".into());
        }
        let (head_start, head_end) = ir::offsets(&source, coordinates)?;
        let header = &source[head_start..head_end];
        if !header.starts_with('|') || !header.ends_with('|') {
            continue;
        }
        let uses = records
            .iter()
            .filter(|r| r["kind"] == "capture_use")
            .collect::<Vec<_>>();
        if uses.is_empty()
            || uses
                .iter()
                .any(|r| r["binding_id"] != capture["binding_id"])
        {
            continue;
        }
        let (state_start, state_end) = range(&uses[0]["span"], cwd, &file, &source)?;
        let state = &source[state_start..state_end];
        if state != capture["binding"] && state != format!("r#{}", capture["binding"]) {
            continue;
        }
        for call in parent.iter().filter(|r| {
            r["kind"] == "call"
                && matches!(
                    r["target"].as_str(),
                    "core::iter::Iterator::for_each" | "std::iter::Iterator::for_each"
                )
        }) {
            let Some(instance) = parent
                .iter()
                .find(|r| r["kind"] == "instance" && r["block"] == call["block"])
            else {
                continue;
            };
            if instance["target"] != call["target"]
                || instance["definition_span"] != call["definition_span"]
            {
                continue;
            }
            let term = parent
                .iter()
                .find(|r| r["kind"] == "term" && r["block"] == call["block"])
                .ok_or("for_each terminator missing")?;
            let mut args = parent
                .iter()
                .filter(|r| r["kind"] == "source_argument" && r["call_span"] == term["span"])
                .collect::<Vec<_>>();
            args.sort_by_key(|r| &r["index"]);
            let [iter, callback] = args.as_slice() else {
                continue;
            };
            if iter["index"] != "0" || callback["index"] != "1" {
                return Err("for_each source argument identities changed".into());
            }
            let (a, b) = range(&callback["span"], cwd, &file, &source)?;
            if a != head_start || head_end > b {
                continue;
            }
            let (_, signature, body, body_hash) =
                std_body::pinned_body(&call["definition_span"], cwd)?;
            if signature != "fnfor_each<F>(self,f:F)whereSelf:Sized,F:FnMut(Self::Item),"
                || body
                    != "{#[inline]fncall<T>(mutf:implFnMut(T))->implFnMut((),T){move|(),item|f(item)}self.fold((),call(f));}"
            {
                return Err("pinned for_each source changed".into());
            }
            let mut edits = Vec::new();
            for usage in &uses {
                let (start, end) = range(&usage["span"], cwd, &file, &source)?;
                if start < head_end || end > b || source[start..end] != *state {
                    return Err("capture use outside the callback body".into());
                }
                edits.push((start - a, end - a, format!("(*{STATE})")));
            }
            let separator = if header[1..header.len() - 1].trim().is_empty() {
                ""
            } else {
                ", "
            };
            edits.push((1, 1, format!("{STATE}: &mut _{separator}")));
            edits.sort_by_key(|e| e.0);
            if edits.windows(2).any(|w| w[0].1 > w[1].0) {
                return Err("overlapping capture edits".into());
            }
            let mut lifted = source[a..b].to_owned();
            for (start, end, replacement) in edits.into_iter().rev() {
                lifted.replace_range(start..end, &replacement);
            }
            let (iter_start, iter_end) = range(&iter["span"], cwd, &file, &source)?;
            let (call_start, call_end) = range(&term["span"], cwd, &file, &source)?;
            if !(call_start <= iter_start
                && iter_start < iter_end
                && iter_end <= a
                && b <= call_end)
            {
                return Err("for_each source arguments escape the call".into());
            }
            let parent_body = parent
                .iter()
                .find(|r| r["kind"] == "body")
                .ok_or("enclosing body missing")?;
            let (_, anchor) = range(&parent_body["span"], cwd, &file, &source)?;
            if call_end > anchor {
                return Err("callback helper outside enclosing function".into());
            }
            let replacement = format!(
                "self::{EACH}({}, &mut {state}, {lifted})",
                &source[iter_start..iter_end]
            );
            let mut candidate = source.clone();
            candidate.insert_str(anchor, DISPATCH);
            candidate.replace_range(call_start..call_end, &replacement);
            let generated_hash = hash(candidate.as_bytes())?;
            proposals.push(Proposal { source: candidate, report: json!({"owner":owner,"original_source":file,"source_sha256":hash(source.as_bytes())?,"generated_sha256":generated_hash,"call_span":term["span"],"capture":capture,"source_uses":uses,"pinned_for_each_sha256":body_hash,"resolved_instance":instance,"enclosing_call_contexts":call_contexts(parent_name,bodies,cwd)?,"status":"candidate_requires_correspondence","scope":"Explicit-state callback and iterator dispatcher candidate; no trusted spec or proof acceptance is added."}) });
        }
    }
    Ok(proposals)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires LIBRARY_GUIDANCE_MIR from actual original/candidate extraction"]
    fn actual_library_guidance_retains_runtime() {
        let base = std::path::PathBuf::from(std::env::var("LIBRARY_GUIDANCE_MIR").unwrap());
        let read = |name: &str| {
            crate::client::borrow_records(
                &fs::read_to_string(base.join(name).join("stderr")).unwrap(),
            )
            .unwrap()
        };
        let before = read("original");
        let after = read("candidate");
        let roots = std::collections::BTreeSet::from(["range_difference".to_owned()]);
        let report = crate::ordinary::correspondence_report(&before, &after, &roots);
        crate::write_json(&base.join("correspondence.json"), &report).unwrap();
        assert_eq!(report["matches"], true);
    }

    #[test]
    #[ignore = "requires CALL_CONTEXT_RUN from the actual nine-test integration run"]
    fn actual_helper_calls_preserve_argument_identities() {
        let base = std::path::PathBuf::from(std::env::var("CALL_CONTEXT_RUN").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(base.join("report.json")).unwrap()).unwrap();
        let mut count = 0;
        for result in report["results"].as_array().unwrap() {
            let Some(path) = result["callback_proposals"].as_str() else {
                continue;
            };
            let proposal: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            for proposal in proposal["proposals"].as_array().unwrap() {
                let context = &proposal["derivation"]["enclosing_call_contexts"];
                for context in context["contexts"].as_array().unwrap() {
                    assert_eq!(context["status"], "recorded");
                    assert_eq!(context["arguments"].as_array().unwrap().len(), 3);
                    count += 1;
                    let bodies = crate::client::borrow_records(
                        &fs::read_to_string(
                            Path::new(path).parent().unwrap().join("original.stderr"),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    let target = context["call"]["target"].as_str().unwrap();
                    let command: Value = serde_json::from_slice(
                        &fs::read(
                            Path::new(path)
                                .parent()
                                .unwrap()
                                .join("original.command.json"),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    let cwd = Path::new(command["cwd"].as_str().unwrap());
                    let mut changed = bodies.clone();
                    let parameter = changed
                        .get_mut(target)
                        .unwrap()
                        .iter_mut()
                        .find(|r| r["kind"] == "parameter")
                        .unwrap();
                    parameter.insert("index".into(), "99".into());
                    assert!(call_contexts(target, &changed, cwd).is_err());
                    let mut changed = bodies.clone();
                    let call = changed
                        .get_mut(context["owner"].as_str().unwrap())
                        .unwrap()
                        .iter_mut()
                        .find(|r| r["kind"] == "call" && r["target"] == target)
                        .unwrap();
                    call.insert("definition_span".into(), "wrong declaration".into());
                    assert!(call_contexts(target, &changed, cwd).is_err());
                }
            }
        }
        assert_eq!(count, 9);
    }

    #[test]
    #[ignore = "requires CALLBACK_BODIES and CALLBACK_TRANSFORMED_BODIES from the actual candidate"]
    fn runtime_candidate_still_requires_a_correspondence_proof() {
        let read = |name| {
            crate::client::borrow_records(
                &fs::read_to_string(std::env::var(name).unwrap()).unwrap(),
            )
            .unwrap()
        };
        let before = read("CALLBACK_BODIES");
        let mut after = read("CALLBACK_TRANSFORMED_BODIES");
        after.retain(|owner, _| before.contains_key(owner));
        assert!(before.keys().eq(after.keys()));
        let roots = before
            .keys()
            .filter(|owner| !owner.contains("::{closure#"))
            .cloned()
            .collect();
        let report = crate::ordinary::correspondence_report(&before, &after, &roots);
        assert_eq!(report["matches"], false);
        if let Ok(out) = std::env::var("CALLBACK_CHECK_OUT") {
            crate::write_json(Path::new(&out), &report).unwrap();
        }
    }

    #[test]
    #[ignore = "requires CALLBACK_BODIES, CALLBACK_CWD and new CALLBACK_OUT from actual Rustc extraction"]
    fn actual_stateful_callback_proposal_uses_resolved_capture() {
        let raw = fs::read_to_string(std::env::var("CALLBACK_BODIES").unwrap()).unwrap();
        let bodies = crate::client::borrow_records(&raw).unwrap();
        let proposals =
            propose(&bodies, Path::new(&std::env::var("CALLBACK_CWD").unwrap())).unwrap();
        assert_eq!(proposals.len(), 1);
        let out = std::path::PathBuf::from(std::env::var("CALLBACK_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let proposal = &proposals[0];
        assert_eq!(
            proposal.report["status"],
            "candidate_requires_correspondence"
        );
        assert_eq!(proposal.report["capture"]["binding"], "res");
        assert!(proposal.source.contains("&mut res"));
        assert!(proposal.source.contains("(*__spec_callback_state).next()"));
        for (field, value) in [
            ("capture_kind", "ByValue"),
            ("projection_count", "1"),
            ("binding_id", "unrelated"),
        ] {
            let mut changed = bodies.clone();
            let capture = changed
                .values_mut()
                .flat_map(|r| r.iter_mut())
                .find(|r| r["kind"] == "capture")
                .unwrap();
            capture.insert(field.into(), value.into());
            assert!(
                propose(&changed, Path::new(&std::env::var("CALLBACK_CWD").unwrap()))
                    .unwrap()
                    .is_empty()
            );
        }
        for replacement in [None, Some("custom::Iterator::for_each")] {
            let mut changed = bodies.clone();
            for records in changed.values_mut() {
                if let Some(target) = replacement {
                    for record in records.iter_mut().filter(|r| r["kind"] == "instance") {
                        record.insert("target".into(), target.into());
                    }
                } else {
                    records.retain(|r| r["kind"] != "instance");
                }
            }
            assert!(
                propose(&changed, Path::new(&std::env::var("CALLBACK_CWD").unwrap()))
                    .unwrap()
                    .is_empty()
            );
        }
        fs::write(out.join("candidate.rs"), &proposal.source).unwrap();
        crate::write_json(&out.join("report.json"), &proposal.report).unwrap();
    }
}
