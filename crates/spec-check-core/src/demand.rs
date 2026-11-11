//! Source demands before translation: assertions and calls with entry contracts.
use crate::{
    Result, attribution as a, contracts, hash, ir, ordinary::ContractIndex, vir, write_json,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn span(n: &vir::Node) -> Option<&str> {
    let xs = n.list()?;
    matches!(xs.first()?.atom(), Some("@" | "@@"))
        .then(|| xs.get(1)?.atom())
        .flatten()
}

fn collect(
    n: &vir::Node,
    location: Option<&str>,
    context: &[Value],
    index: &ContractIndex<'_>,
    rows: &mut Vec<Value>,
) {
    let location = span(n).or(location);
    let mut context = context.to_vec();
    if matches!(
        a::head(n),
        Some("If" | "Loop" | "Match" | "Ghost" | "Closure" | "ExecClosure")
    ) {
        context.push(json!({"kind":a::head(n),"span":location}));
    }
    if a::head(n) == Some("AssertAssume")
        && n.field("is_assume").and_then(vir::Node::atom) == Some("false")
    {
        rows.push(json!({"kind":"assertion","span":location,"context":context,"expression":n.field("expr").map(contracts::render),"status":"requires_translation"}));
    }
    if a::head(n) == Some("Call") {
        let mut targets = Vec::new();
        if let Some(target) = n.field("target") {
            vir::walk_calls(target, &mut targets);
        }
        // Keep unresolved calls visible; no textual-name fallback.
        let contract = if let [target] = targets.as_slice() {
            index.call_contract(target)
        } else {
            json!({"reason":"call target missing or ambiguous"})
        };
        let status = match contract["requires"].as_array() {
            Some(xs) if xs.is_empty() => "no_explicit_precondition",
            Some(_) => "requires_translation",
            None => "contract_unavailable",
        };
        rows.push(json!({"kind":"call_precondition","span":location,"context":context,"targets":targets,"arguments":n.field("args").map(contracts::render),"contract":contract,"status":status}));
    }
    // Annotations wrap the same node; recurse only through its unwrapped children.
    if let Some(xs) = n.unwrap().list() {
        for child in xs {
            collect(child, location, &context, index, rows);
        }
    }
}

pub(crate) fn body_demands(body: &vir::Node, index: &ContractIndex<'_>) -> Vec<Value> {
    let mut rows = Vec::new();
    collect(body, None, &[], index, &mut rows);
    for row in &mut rows {
        if row["status"] == "requires_translation" {
            row["status"] = json!("check_target");
        }
    }
    rows
}

// Map only spans wholly contained in an unchanged source segment. A changed or
// synthetic expression stays unmapped, even if its text happens to occur elsewhere.
fn original_offsets(
    start: usize,
    end: usize,
    source_len: usize,
    edits: &[ir::Edit],
) -> Option<(usize, usize)> {
    let (mut original, mut generated) = (0usize, 0usize);
    for edit in edits {
        if edit.start < original || edit.end < edit.start || edit.end > source_len {
            return None;
        }
        let unchanged = edit.start - original;
        if start >= generated && end <= generated + unchanged && start < end {
            return Some((original + start - generated, original + end - generated));
        }
        generated += unchanged + edit.replacement.len();
        original = edit.end;
    }
    if start >= generated && end <= generated + source_len - original && start < end {
        return Some((original + start - generated, original + end - generated));
    }
    None
}

pub(crate) fn source_location(
    span: Option<&str>,
    package: &Path,
    snapshot: &Path,
    edits: &BTreeMap<PathBuf, Vec<ir::Edit>>,
) -> Value {
    let lookup = || -> Result<Value> {
        let (generated_path, coordinates) = ir::span(span.ok_or("missing span")?, snapshot)?;
        let relative = generated_path.strip_prefix(snapshot)?;
        let original_path = package.join(relative);
        let changes = edits
            .get(&original_path)
            .ok_or("source edits unavailable")?;
        let original = fs::read_to_string(&original_path)?;
        let generated = fs::read_to_string(&generated_path)?;
        let mut reconstructed = original.clone();
        for edit in changes.iter().rev() {
            if edit.end > reconstructed.len()
                || edit.start > edit.end
                || !reconstructed.is_char_boundary(edit.start)
                || !reconstructed.is_char_boundary(edit.end)
            {
                return Err("invalid recorded edit".into());
            }
            reconstructed.replace_range(edit.start..edit.end, &edit.replacement);
        }
        if reconstructed != generated {
            return Err("snapshot differs from recorded edits".into());
        }
        let (start, end) = ir::offsets(&generated, coordinates)?;
        let (old_start, old_end) = original_offsets(start, end, original.len(), changes)
            .ok_or("span overlaps an edit or is empty")?;
        if generated.get(start..end) != original.get(old_start..old_end) {
            return Err("unchanged source text differs".into());
        }
        let position = |offset: usize| {
            let prefix = &original[..offset];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
            let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            (line, column)
        };
        let (sl, sc) = position(old_start);
        let (el, ec) = position(old_end);
        Ok(
            json!({"status":"mapped_unchanged","path":original_path,"start":[sl,sc],"end":[el,ec],"method":"recorded source edits and exact text"}),
        )
    };
    lookup().unwrap_or_else(|error| json!({"status":"unavailable","reason":error.to_string()}))
}

pub fn run(input: &Path, out: &Path) -> Result<()> {
    let bytes = fs::read(input)?;
    let functions = vir::functions(std::str::from_utf8(&bytes)?)?;
    let index = ContractIndex::new(&functions);
    let mut callers = Vec::new();
    for f in &functions {
        if f.field("mode").and_then(vir::Node::atom) != Some("Exec") {
            continue;
        }
        let mut demands = Vec::new();
        if let Some(body) = f.field("body") {
            collect(body, span(f), &[], &index, &mut demands);
        }
        callers.push(json!({"function":f.field("name").and_then(|n|n.field("path")).and_then(vir::Node::atom),"span":span(f),"body_available":f.field("body").is_some_and(|n|n.atom()!=Some("None")),"demands":demands}));
    }
    if fs::read(input)? != bytes {
        return Err("VIR changed during inventory".into());
    }
    fs::create_dir(out)?;
    write_json(
        &out.join("report.json"),
        &json!({"vir":input,"vir_sha256":hash(&bytes)?,"callers":callers,"scope":"All exec bodies present in supplied VIR. Context records nesting only; arguments and predicates are not yet instantiated or proved. Missing bodies and contracts retained. Assertions include proof assertions; runtime/test classification requires compiler linkage.","defects_confirmed":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_span_mapping_rejects_inserted_or_crossing_text() {
        let edits = vec![ir::Edit {
            start: 2,
            end: 3,
            replacement: "XYZ".into(),
            origin: "test".into(),
        }];
        assert_eq!(original_offsets(0, 2, 6, &edits), Some((0, 2)));
        assert_eq!(original_offsets(5, 8, 6, &edits), Some((3, 6)));
        assert_eq!(original_offsets(2, 5, 6, &edits), None);
        assert_eq!(original_offsets(1, 6, 6, &edits), None);
    }
    #[test]
    fn source_mapping_rejects_unrecorded_snapshot_changes() {
        let root = std::env::temp_dir().join(format!("demand-spans-{}", std::process::id()));
        let original = root.join("original");
        let snapshot = root.join("snapshot");
        fs::create_dir_all(&original).unwrap();
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(original.join("f.rs"), "one\ntwo\n").unwrap();
        fs::write(snapshot.join("f.rs"), "one\ntwo\nextra\n").unwrap();
        let edits = BTreeMap::from([(original.join("f.rs"), vec![])]);
        let span = format!("{}:2:1: 2:4 (#0)", snapshot.join("f.rs").display());
        assert_eq!(
            source_location(Some(&span), &original, &snapshot, &edits)["status"],
            "unavailable"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "requires GOAL3_DEMAND_CASE from an actual ordinary-check run"]
    fn real_scheduler_span_uses_recorded_edits() {
        let case = PathBuf::from(std::env::var_os("GOAL3_DEMAND_CASE").unwrap());
        let package = PathBuf::from(std::env::var_os("GOAL3_DEMAND_PACKAGE").unwrap());
        let snapshot = case.join("package");
        let edits: BTreeMap<PathBuf, Vec<ir::Edit>> =
            serde_json::from_slice(&fs::read(case.join("context-edits.json")).unwrap()).unwrap();
        let demands: Value =
            serde_json::from_slice(&fs::read(case.join("demand-sites.json")).unwrap()).unwrap();
        let call = demands["sites"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| {
                row["targets"].as_array().is_some_and(|targets| {
                    targets
                        .iter()
                        .any(|t| t.as_str() == Some("core::option::impl&%0::unwrap"))
                })
            })
            .unwrap();
        let mapped = source_location(call["span"].as_str(), &package, &snapshot, &edits);
        assert_eq!(mapped["status"], "mapped_unchanged");
        assert_eq!(mapped["start"], json!([347, 9]));
        assert_eq!(mapped["end"], json!([347, 38]));
        assert_eq!(
            source_location(call["span"].as_str(), &package, &snapshot, &BTreeMap::new())["status"],
            "unavailable"
        );
    }
    #[test]
    fn inherited_requirement_and_branch_assertion_are_retained() {
        let fs = vir::functions(r#"
(Function :name (Fun :path trait_method) :require ((@ "api.rs:1:1: 1:2 (#0)" (> Const (Constant Bool true)))) :ensure (tuple () ()) :kind (FunctionKind TraitMethodDecl))
(Function :name (Fun :path implementation) :require () :ensure (tuple () ()) :kind (FunctionKind TraitMethodImpl :method (Fun :path trait_method)))
(Function :name (Fun :path client) :mode Exec :body (@ "client.rs:1:1: 1:2 (#0)" (> If (> Const (Constant Bool true)) (@ "client.rs:2:1: 2:2 (#0)" (> Call :target (CallTarget Fun (CallTargetKind Static) (Fun :path implementation) () () None) :args ())) (@ "client.rs:3:1: 3:2 (#0)" (> AssertAssume :is_assume false :expr (> Const (Constant Bool false)))))))
"#).unwrap();
        let mut rows = Vec::new();
        collect(
            fs[2].field("body").unwrap(),
            None,
            &[],
            &ContractIndex::new(&fs),
            &mut rows,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["status"], "requires_translation");
        assert_eq!(rows[0]["contract"]["requires"].as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["span"], "client.rs:2:1: 2:2 (#0)");
        assert_eq!(rows[1]["kind"], "assertion");
        assert_eq!(rows[1]["context"][0]["kind"], "If");
    }
}
