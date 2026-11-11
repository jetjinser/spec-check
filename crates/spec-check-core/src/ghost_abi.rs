//! Compiler-only ghost ABI projection. Placeholders must never be proof inputs.
use crate::{
    Result,
    borrowed_resource::{name, one, params, ret},
    hash, ir, resource,
    vir::{self, Node},
    write_json,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub fn run(hir: &Path, native: &Path, moves: &Path, cwd: &Path, out: &Path) -> Result<()> {
    fs::create_dir(out)?;
    let rows: Vec<Vec<String>> = serde_json::from_slice(&fs::read(hir)?)?;
    let native_text = fs::read_to_string(native)?;
    let functions = vir::functions(&native_text)?;
    let moves: BTreeMap<String, String> = serde_json::from_slice(&fs::read(moves)?)?;
    for (old, new) in &moves {
        if old.rsplit_once("::").map(|(_, m)| m) != new.rsplit_once("::").map(|(_, m)| m)
            || functions.iter().any(|f| name(f).is_ok_and(|n| n == old))
        {
            return Err("API moves require an absent old method and unchanged method name".into());
        }
        let candidates = functions
            .iter()
            .filter(|f| name(f).is_ok_and(|n| n == new))
            .collect::<Vec<_>>();
        if !matches!(candidates.as_slice(),[f] if f.field("kind").and_then(Node::list).and_then(|v|v.get(1)).and_then(Node::atom)==Some("TraitMethodDecl"))
        {
            return Err("API move target must be a unique native trait declaration".into());
        }
    }
    let mut owners = BTreeMap::<String, Vec<Vec<String>>>::new();
    let mut current = None;
    for row in &rows {
        let tag = row.first().ok_or("empty source record")?;
        if tag == "SOURCE_OWNER" {
            if row.len() != 3 || current.is_some() || owners.contains_key(&row[1]) {
                return Err("invalid source owner boundary".into());
            }
            current = Some(row[1].clone());
            owners.insert(row[1].clone(), vec![row.clone()]);
        } else if tag == "SOURCE_END" {
            if row.len() != 2 || current.as_deref() != Some(&row[1]) {
                return Err("source owner boundary mismatch".into());
            }
            current = None;
        } else if let Some(owner) = &current {
            let size = match tag.as_str() {
                "SOURCE_SELF" | "SOURCE_BODY" => 3,
                "SOURCE_ARGUMENT" | "SOURCE_QUALIFIED" => 4,
                "SOURCE_BINDING" | "SOURCE_COMPONENT" | "SOURCE_TARGET" | "SOURCE_CALL"
                | "SOURCE_METHOD" => 5,
                _ => return Err("unsupported source record in ABI capture".into()),
            };
            if row.len() != size {
                return Err("invalid ABI source record arity".into());
            }
            owners.get_mut(owner).unwrap().push(row.clone());
        } else {
            return Err("source record outside owner".into());
        }
    }
    if current.is_some() || owners.is_empty() {
        return Err("incomplete or empty source capture".into());
    }
    let mut proposals = owners
        .iter()
        .map(|(owner, records)| (owner.clone(), project(records, &functions, &moves, cwd)))
        .collect::<BTreeMap<_, _>>();
    let projected = proposals
        .iter()
        .filter(|(_, result)| result.is_ok())
        .map(|(owner, _)| owner.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut results = Vec::new();
    for (i, (owner, records)) in owners.iter().enumerate() {
        let entry = &records[0];
        let (file, _) = ir::span(&entry[2], cwd)?;
        let source = fs::read_to_string(&file)?;
        let result = proposals.remove(owner).ok_or("missing ABI proposal")?;
        let item = match result {
            Err(error) => {
                json!({"owner":owner,"status":"unsupported","reason":error.to_string(),"source":file,"source_sha256":hash(source.as_bytes())?})
            }
            Ok((mut edits, evidence)) => {
                // Retain supporting definitions that need no ABI projection.
                for (other, records) in &owners {
                    if other == owner || !projected.contains(other) {
                        continue;
                    }
                    let (path, c) = ir::span(&records[0][2], cwd)?;
                    if path != file {
                        return Err("source projection currently requires one file".into());
                    }
                    let (a, _) = ir::offsets(&source, c)?;
                    edits.push(ir::Edit {
                        start: a,
                        end: a,
                        replacement: "#[cfg(any())]\n".into(),
                        origin: records[0][2].clone(),
                    });
                }
                edits.sort_by_key(|e| e.start);
                if edits.windows(2).any(|w| w[0].end > w[1].start) {
                    return Err("overlapping ghost ABI edits".into());
                }
                let mut changed = source.clone();
                for edit in edits.iter().rev() {
                    changed.replace_range(edit.start..edit.end, &edit.replacement);
                }
                let output = out.join(format!("caller-{i}.rs"));
                fs::write(&output, &changed)?;
                json!({"owner":owner,"status":"generated_unchecked","source":file,"source_sha256":hash(source.as_bytes())?,"output":output,"output_sha256":hash(changed.as_bytes())?,"observed_hir_calls":records.iter().filter(|r|r[0]=="SOURCE_CALL").count(),"edits":edits,"calls":evidence})
            }
        };
        results.push(item);
    }
    write_json(
        &out.join("report.json"),
        &json!({"status":"compiler_projection_only","inputs":results,"hir_sha256":hash(&fs::read(hir)?)?,"vir_sha256":hash(native_text.as_bytes())?,"project_api_moves":moves,"manual_token_mappings":0,"scope":"Current ghost ABI prepared from native signatures. API moves are explicit project configuration. Every input still requires successful compilation and permission connection; placeholders are forbidden in checked Verus clients."}),
    )
}

fn project(
    rows: &[Vec<String>],
    functions: &[Node],
    moves: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<(Vec<ir::Edit>, Vec<Value>)> {
    let (file, _) = ir::span(&rows[0][2], cwd)?;
    let source = fs::read_to_string(&file)?;
    let range = |span: &str| -> Result<(usize, usize)> {
        let (p, c) = ir::span(span, cwd)?;
        if p != file {
            return Err("ghost ABI edit outside caller file".into());
        }
        ir::offsets(&source, c)
    };
    let local_name = regex::Regex::new(r"^[A-Za-z_]\w*$")?;
    let mut edits = Vec::new();
    let mut evidence = Vec::new();
    for call in rows
        .iter()
        .filter(|r| r.first().is_some_and(|v| v == "SOURCE_CALL"))
    {
        if call.len() != 5 {
            return Err("invalid source call record".into());
        }
        let qualified = rows
            .iter()
            .filter(|r| {
                r.first().is_some_and(|v| v == "SOURCE_QUALIFIED") && r.get(1) == Some(&call[1])
            })
            .collect::<Vec<_>>();
        let qualified = match qualified.as_slice() {
            [] => None,
            [q] => Some(*q),
            _ => return Err("ambiguous qualified source identity".into()),
        };
        let old = if let Some(q) = qualified {
            if q.len() != 4 {
                return Err("invalid qualified source call".into());
            }
            format!("{}::{}", q[2], q[3])
        } else {
            call[3].clone()
        };
        let declared = functions
            .iter()
            .filter(|f| name(f).is_ok_and(|n| n == call[3]))
            .collect::<Vec<_>>();
        let migrated = declared.is_empty() && moves.contains_key(&old);
        let target = if migrated {
            moves[&old].as_str()
        } else {
            call[3].as_str()
        };
        let candidates = functions
            .iter()
            .filter(|f| name(f).is_ok_and(|n| n == target))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }
        let [f] = candidates.as_slice() else {
            return Err("ambiguous native ABI declaration".into());
        };
        if f.field("mode").and_then(Node::atom) != Some("Exec") {
            continue;
        }
        let native_span = f
            .list()
            .and_then(|v| v.get(1))
            .and_then(Node::atom)
            .ok_or("native ABI source unavailable")?;
        let (native_file, native_coords) = ir::span(native_span, cwd)?;
        if !migrated {
            let resolved = one(rows, "SOURCE_TARGET", &call[1])?;
            let (p, c) = ir::span(
                resolved.get(4).ok_or("source target location missing")?,
                cwd,
            )?;
            if c[..2] != native_coords[..2] || fs::read(p)? != fs::read(&native_file)? {
                return Err("ABI signature source mismatch".into());
            }
        } else if f
            .field("kind")
            .and_then(Node::list)
            .and_then(|v| v.get(1))
            .and_then(Node::atom)
            != Some("TraitMethodDecl")
        {
            return Err("API migration must target a native trait declaration".into());
        }
        let ps = params(f)?;
        let supplied = call[4].parse::<usize>()?;
        let tracked = ps
            .iter()
            .filter(|t| resource::decoration(t, "Tracked").is_some())
            .count();
        if supplied != ps.len() - tracked {
            return Err("source arguments do not match the ghost-erased ABI".into());
        }
        let mut arguments = Vec::new();
        let mut input = 0;
        for p in ps {
            if resource::decoration(p, "Tracked").is_some() {
                arguments.push("vstd::prelude::Tracked::assume_new()".to_owned());
            } else {
                let args = rows
                    .iter()
                    .filter(|r| {
                        r.first().is_some_and(|v| v == "SOURCE_ARGUMENT")
                            && r.get(1) == Some(&call[1])
                            && r.get(2) == Some(&input.to_string())
                    })
                    .collect::<Vec<_>>();
                let [arg] = args.as_slice() else {
                    return Err("source argument identity missing".into());
                };
                let (a, b) = range(arg.get(3).ok_or("argument span missing")?)?;
                arguments.push(source[a..b].to_owned());
                input += 1;
            }
        }
        let tuple = resource::tuple_fields(ret(f)?);
        let tuple_projection = matches!(tuple,Some([_,permission]) if resource::decoration(permission,"Tracked").is_some());
        if tracked == 0 && !tuple_projection && !migrated {
            continue;
        }
        let (a, b) = range(&call[1])?;
        let (s, e) = range(&call[2])?;
        if !(a <= s && s < e && e <= b)
            || !source[e..b].trim_start().starts_with('(')
            || !source[a..b].trim_end().ends_with(')')
        {
            return Err("source call shape unsupported".into());
        }
        let callee = if migrated {
            let self_type = one(rows, "SOURCE_SELF", &call[1])?;
            let (s, e) = range(&self_type[2])?;
            let (trait_path, method) = target
                .rsplit_once("::")
                .ok_or("migration target has no method")?;
            let (crate_name, _) = trait_path
                .split_once("::")
                .ok_or("migration target has no crate")?;
            if !old.starts_with(&format!("{crate_name}::")) {
                return Err("cross-crate API migration unsupported".into());
            }
            format!(
                "<{} as crate{}>::{method}",
                &source[s..e],
                &trait_path[crate_name.len()..]
            )
        } else {
            source[s..e].to_owned()
        };
        if tracked != 0 || migrated {
            edits.push(ir::Edit {
                start: a,
                end: b,
                replacement: format!("{callee}({})", arguments.join(", ")),
                origin: call[1].clone(),
            });
        }
        if tuple_projection {
            let bindings = rows
                .iter()
                .filter(|r| {
                    r.first().is_some_and(|v| v == "SOURCE_BINDING")
                        && r.get(4) == Some(&format!("Some({})", call[1]))
                })
                .collect::<Vec<_>>();
            let [binding] = bindings.as_slice() else {
                return Err("ghost return must initialize a direct binding".into());
            };
            if binding[3] != "None" {
                return Err("annotated ghost return binding unsupported".into());
            }
            let (s, e) = range(&binding[2])?;
            if !local_name.is_match(&source[s..e]) {
                return Err("ghost return needs a simple binding".into());
            }
            edits.push(ir::Edit {
                start: s,
                end: e,
                replacement: format!("({}, _)", &source[s..e]),
                origin: binding[2].clone(),
            });
        }
        evidence.push(json!({"source_call":call[1],"original_identity":old,"native_identity":target,"native_source":native_span,"native_source_sha256":hash(&fs::read(native_file)?)?,"project_migration":migrated,"tracked_placeholders":tracked,"tuple_projection":tuple_projection}));
    }
    if evidence.is_empty() {
        return Err("no supported ghost ABI changes".into());
    }
    edits.sort_by_key(|e| e.start);
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return Err("nested ghost ABI rewrites unsupported".into());
    }
    Ok((edits, evidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires actual partial source capture and native VIR in GHOST_ABI_HIR, GHOST_ABI_VIR and GHOST_ABI_MOVES"]
    fn actual_source_projection_and_identity_controls() {
        let rows: Vec<Vec<String>> =
            serde_json::from_slice(&fs::read(std::env::var("GHOST_ABI_HIR").unwrap()).unwrap())
                .unwrap();
        let functions =
            vir::functions(&fs::read_to_string(std::env::var("GHOST_ABI_VIR").unwrap()).unwrap())
                .unwrap();
        let moves: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(std::env::var("GHOST_ABI_MOVES").unwrap()).unwrap())
                .unwrap();
        let mut records = Vec::new();
        let mut inputs = 0;
        for row in rows {
            if row[0] == "SOURCE_OWNER" {
                records.clear();
            }
            if row[0] == "SOURCE_END" {
                let (edits, calls) = project(&records, &functions, &moves, Path::new("/")).unwrap();
                assert_eq!(calls.len(), 4);
                assert_eq!(
                    calls
                        .iter()
                        .filter(|c| c["project_migration"] == true)
                        .count(),
                    2
                );
                assert_eq!(
                    calls
                        .iter()
                        .filter_map(|c| c["tracked_placeholders"].as_u64())
                        .sum::<u64>(),
                    2
                );
                assert_eq!(
                    calls
                        .iter()
                        .filter(|c| c["tuple_projection"] == true)
                        .count(),
                    2
                );
                assert_eq!(
                    edits
                        .iter()
                        .filter(|e| e.replacement.contains("Tracked::assume_new()"))
                        .count(),
                    2
                );
                let missing = records
                    .iter()
                    .filter(|r| r[0] != "SOURCE_TARGET")
                    .cloned()
                    .collect::<Vec<_>>();
                assert!(project(&missing, &functions, &moves, Path::new("/")).is_err());
                let mut duplicate = records.clone();
                duplicate.push(
                    records
                        .iter()
                        .find(|r| r[0] == "SOURCE_QUALIFIED")
                        .unwrap()
                        .clone(),
                );
                assert!(project(&duplicate, &functions, &moves, Path::new("/")).is_err());
                let mut wrong = records.clone();
                for r in wrong
                    .iter_mut()
                    .filter(|r| r[0] == "SOURCE_CALL" && r[3].ends_with("::from_raw"))
                {
                    r[4] = "2".into();
                }
                assert!(project(&wrong, &functions, &moves, Path::new("/")).is_err());
                inputs += 1;
            } else {
                records.push(row);
            }
        }
        assert_eq!(inputs, 2);
    }
}
