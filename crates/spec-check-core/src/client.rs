//! Run source-bound caller checks using the project capture and interface handoff.
use crate::ir;
use crate::{Result, capture::Invocation, contracts, hash, link, runner, vir, write_json};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub(crate) fn borrow_records(text: &str) -> Result<BTreeMap<String, Vec<ir::Record>>> {
    let mut result = BTreeMap::new();
    let mut current = None;
    let mut buffer = String::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("CALLER_BEGIN\t") {
            if current.replace(path.to_owned()).is_some() {
                return Err("nested caller extraction".into());
            }
            buffer.clear();
        } else if let Some(path) = line.strip_prefix("CALLER_END\t") {
            if current.as_deref() != Some(path) {
                return Err("caller extraction boundary mismatch".into());
            }
            if result
                .insert(path.to_owned(), ir::parse(&buffer)?)
                .is_some()
            {
                return Err("duplicate caller extraction".into());
            }
            current = None;
        } else if current.is_some() && line.starts_with("CLIENT_") {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    if current.is_some() {
        return Err("incomplete caller extraction".into());
    }
    Ok(result)
}

pub(crate) fn extract_callers(
    probe: &Path,
    inv: &Invocation,
    args: &[String],
    selected: &[&str],
    out: &Path,
    name: &str,
) -> Result<(BTreeMap<String, Vec<ir::Record>>, Value)> {
    let mut pending = vec![selected.to_vec()];
    let mut bodies = BTreeMap::new();
    let mut callers = BTreeMap::new();
    let mut attempts = Vec::new();
    while let Some(group) = pending.pop() {
        let label = if attempts.is_empty() {
            name.to_owned()
        } else {
            format!("{name}-part-{}", attempts.len())
        };
        let selection = out.join(format!("{label}.selection.txt"));
        fs::write(&selection, group.join("\n"))?;
        let mut context = inv.clone();
        context.env.insert(
            "CLIENT_PROJECT_BORROW_PATHS".into(),
            selection.display().to_string(),
        );
        context
            .env
            .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
        context
            .env
            .insert("CLIENT_PROJECT_PREFIX".into(), "1".into());
        let work = || -> Result<BTreeMap<String, Vec<ir::Record>>> {
            runner::extract_context(probe, &context, args, "*", out, &label, true)?;
            let records =
                borrow_records(&fs::read_to_string(out.join(format!("{label}.stderr")))?)?;
            if group.iter().any(|p| !records.contains_key(*p)) {
                return Err("selected caller missing from successful extraction".into());
            }
            Ok(records)
        };
        match work() {
            Ok(records) => {
                for path in &group {
                    callers.insert(*path, json!({"status":"typed","evidence":out.join(format!("{label}.json")),"stderr":out.join(format!("{label}.stderr"))}));
                }
                for (path, records) in records {
                    if let Some(previous) = bodies.insert(path, records.clone())
                        && previous != records
                    {
                        return Err("conflicting repeated caller extraction".into());
                    }
                }
                attempts.push(json!({"label":label,"callers":group,"status":"typed"}));
            }
            Err(error) => {
                let failure = json!({"status":"not_analyzed","reason":error.to_string(),"evidence":out.join(format!("{label}.json")),"stderr":out.join(format!("{label}.stderr"))});
                attempts.push(json!({"label":label,"callers":group,"failure":failure}));
                if group.len() > 1 {
                    let middle = group.len() / 2;
                    pending.push(group[middle..].to_vec());
                    pending.push(group[..middle].to_vec());
                } else if let Some(path) = group.first() {
                    callers.insert(*path, failure);
                }
            }
        }
    }
    let report = json!({"callers":callers,"attempts":attempts,"scope":"Failed batches are partitioned to caller boundaries; failed or incomplete extractions never contribute typed bodies."});
    write_json(&out.join(format!("{name}-callers.json")), &report)?;
    Ok((bodies, report))
}

fn identity_records(
    before: &[ir::Record],
    after: &[ir::Record],
    package: &Path,
    snapshot: &Path,
) -> Result<bool> {
    let spans = regex::Regex::new(r#"([^\s"{},\[\](),@]+\.rs)(:\d+:\d+: \d+:\d+)(?: \(#\d+\))?"#)?;
    let original = format!("{}/", package.display());
    let copied = format!("{}/", snapshot.display());
    let relative = format!(
        "{}/",
        package
            .file_name()
            .ok_or("package name missing")?
            .to_string_lossy()
    );
    let norm = |record: &ir::Record| {
        record
            .iter()
            .map(|(k, v)| {
                let value = spans.replace_all(v, |c: &regex::Captures<'_>| {
                    let path = c[1]
                        .strip_prefix(&original)
                        .or_else(|| c[1].strip_prefix(&copied))
                        .or_else(|| c[1].strip_prefix(&relative));
                    let file = path
                        .map(|p| format!("PACKAGE/{p}"))
                        .unwrap_or_else(|| c[1].to_owned());
                    format!("{}{}", file, &c[2])
                });
                (k.clone(), link::stable(&value))
            })
            .collect::<ir::Record>()
    };
    Ok(before.len() == after.len() && before.iter().zip(after).all(|(a, b)| norm(a) == norm(b)))
}

fn identity_group(
    before: &BTreeMap<String, Vec<ir::Record>>,
    after: &BTreeMap<String, Vec<ir::Record>>,
    path: &str,
    package: &Path,
    snapshot: &Path,
) -> Result<bool> {
    let prefix = format!("{path}::");
    let bodies = |m: &BTreeMap<String, Vec<ir::Record>>| {
        m.keys()
            .filter(|k| k.as_str() == path || k.starts_with(&prefix))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    };
    let expected = bodies(before);
    if !expected.contains(path) || expected != bodies(after) {
        return Ok(false);
    }
    for key in expected {
        if !identity_records(&before[&key], &after[&key], package, snapshot)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn full_verification_context(inv: &Invocation) -> Result<()> {
    if !inv.unsupported_env.is_empty() {
        return Err("incomplete compiler environment".into());
    }
    if inv.args.iter().chain(inv.env.values()).any(|v| {
        ["--verify-only", "--verify-function", "--verify-module"]
            .iter()
            .any(|f| v.contains(f))
    }) {
        return Err("partial verification scope cannot establish complete caller checks".into());
    }
    let package = inv
        .env
        .get("CARGO_PKG_NAME")
        .ok_or("missing package identity")?;
    let prefix = format!("__VERUS_DRIVER_VERIFY_{package}-");
    if !inv
        .env
        .iter()
        .any(|(k, v)| k.trim_start().starts_with(&prefix) && v == "1")
    {
        return Err("captured package verification was not enabled".into());
    }
    Ok(())
}

pub fn run(links: &Path, sdk: &Path, out: &Path) -> Result<()> {
    run_scoped(links, sdk, out, None)
}
pub fn run_scoped(links: &Path, sdk: &Path, out: &Path, sources: Option<&Path>) -> Result<()> {
    run_mode(links, sdk, out, sources, false)
}

pub fn census(links: &Path, sdk: &Path, out: &Path) -> Result<()> {
    run_mode(links, sdk, out, None, true)
}

fn run_mode(
    links: &Path,
    sdk: &Path,
    out: &Path,
    sources: Option<&Path>,
    census_only: bool,
) -> Result<()> {
    let sources: Option<std::collections::BTreeSet<std::path::PathBuf>> = sources
        .map(|p| -> Result<_> {
            let v: Value = serde_json::from_slice(&fs::read(p)?)?;
            v["sources"]
                .as_array()
                .ok_or("missing source scope")?
                .iter()
                .map(|s| Ok(Path::new(s.as_str().ok_or("invalid source path")?).canonicalize()?))
                .collect()
        })
        .transpose()?;
    link::validate(links)?;
    fs::create_dir(out)?;
    let out = fs::canonicalize(out)?;
    fs::copy(std::env::current_exe()?, out.join("check-tool"))?;
    fs::write(out.join("client-rule.rs"), include_str!("client.rs"))?;
    write_json(
        &out.join("tool-identity.json"),
        &json!({"executed_path":std::env::current_exe()?,"captured_tool":out.join("check-tool"),"sha256":hash(&fs::read(out.join("check-tool"))?)?}),
    )?;

    let origin: Value = serde_json::from_slice(&fs::read(links.join("origin.json"))?)?;
    let capture = Path::new(origin["capture"].as_str().ok_or("missing capture")?);
    let report: Value = serde_json::from_slice(&fs::read(links.join("report.json"))?)?;
    let inputs: BTreeMap<std::path::PathBuf, String> =
        serde_json::from_slice(&fs::read(capture.join("inputs.json"))?)?;
    for tool in ["verus", "rust_verify", "cargo-verus"] {
        let digest = hash(&fs::read(sdk.join(tool))?)?;
        if !inputs
            .iter()
            .any(|(p, h)| p.file_name().is_some_and(|n| n == tool) && h == &digest)
        {
            return Err(format!("SDK {tool} differs from captured toolchain").into());
        }
    }
    let mut profiles = Vec::new();
    for profile in report["profiles"].as_array().ok_or("missing profiles")? {
        let name = profile["profile"].as_str().ok_or("missing profile name")?;
        let mut units = Vec::new();
        for unit in profile["units"].as_array().ok_or("missing units")? {
            let evidence = Path::new(unit["evidence"].as_str().ok_or("missing relations")?);
            let base = evidence.parent().ok_or("missing unit directory")?;
            let original = capture
                .join(name)
                .join(base.file_name().ok_or("missing unit name")?);
            let inv: Invocation =
                serde_json::from_slice(&fs::read(original.join("context.json"))?)?;
            full_verification_context(&inv)?;
            let package = Path::new(
                inv.env
                    .get("CARGO_MANIFEST_DIR")
                    .ok_or("missing Cargo package")?,
            );
            let native: Value =
                serde_json::from_slice(&fs::read(original.join("verification-input.json"))?)?;
            let text = fs::read_to_string(native["vir"].as_str().ok_or("missing native VIR")?)?;
            let functions = vir::functions(&text)?;
            let trust=functions.iter().filter(|f| f.field("attrs").and_then(|a|a.field("is_external_body")).and_then(vir::Node::atom)==Some("true") || f.field("proxy").is_some_and(|p|p.atom()!=Some("None"))).map(|f|json!({"interface":f.field("name").and_then(|n|n.field("path")).and_then(vir::Node::atom),"proxy":f.field("proxy").map(contracts::render),"external_body":f.field("attrs").and_then(|a|a.field("is_external_body")).and_then(vir::Node::atom)})).collect::<Vec<_>>();
            let owner_paths: Vec<Vec<String>> =
                serde_json::from_slice(&fs::read(base.join("owners.json"))?)?;
            let inventory: Value = serde_json::from_slice(&fs::read(
                original.join("compile/inventory-records.json"),
            )?)?;
            let dest = out.join(name).join(base.file_name().unwrap());
            fs::create_dir_all(&dest)?;
            write_json(&dest.join("owner-identities.json"), &owner_paths)?;
            let dependencies: Value =
                serde_json::from_slice(&fs::read(original.join("dependency-inputs.json"))?)?;
            write_json(
                &dest.join("trusted-context.json"),
                &json!({"scope":"Original verification context; no solver dependency slice claimed","existing_models":trust,"compiled_dependencies":dependencies,"new_trusted_declarations":0}),
            )?;

            let mut callers = Vec::new();
            for owner in inventory["records"]
                .as_array()
                .ok_or("missing inventory")?
                .iter()
                .filter(|r| {
                    r["kind"] == "owner"
                        && matches!(r["definition_kind"].as_str(), Some("Fn" | "AssocFn"))
                })
            {
                if let Some(sources) = &sources
                    && ir::span(
                        owner["span"].as_str().ok_or("missing owner span")?,
                        &inv.cwd,
                    )
                    .ok()
                    .and_then(|(p, _)| p.canonicalize().ok())
                    .is_some_and(|p| !sources.contains(&p))
                {
                    continue;
                }
                let mapping = owner_paths.iter().find(|r| {
                    r.get(1).is_some_and(|v| {
                        link::stable(v) == link::stable(owner["owner"].as_str().unwrap())
                    })
                });
                let path = mapping.and_then(|r| r.get(2));
                let found = functions
                    .iter()
                    .filter(|f| {
                        f.field("name")
                            .and_then(|v| v.field("path"))
                            .and_then(vir::Node::atom)
                            == path.map(String::as_str)
                    })
                    .collect::<Vec<_>>();
                let target = if let [f] = found.as_slice() {
                    if f.field("body").is_some_and(|b| b.atom() != Some("None")) {
                        Some(*f)
                    } else {
                        link::projected_interface(f, &functions, &inv.cwd).ok()
                    }
                } else {
                    None
                };
                let non_exec = found
                    .iter()
                    .any(|f| f.field("mode").and_then(vir::Node::atom) != Some("Exec"));
                let target = target.filter(|f| {
                    f.field("mode").and_then(vir::Node::atom) == Some("Exec")
                        && f.field("attrs")
                            .and_then(|a| a.field("is_external_body"))
                            .and_then(vir::Node::atom)
                            == Some("false")
                        && f.field("body").is_some_and(|b| b.list().is_some())
                });
                let context = target
                    .map(|f| {
                        let p = f
                            .field("name")
                            .unwrap()
                            .field("path")
                            .unwrap()
                            .atom()
                            .unwrap();
                        contracts::summarize_compact(&functions, p, &inv.cwd)
                    })
                    .transpose();
                let (context, context_error) = match context {
                    Ok(value) => (value, None),
                    Err(error) => (None, Some(error.to_string())),
                };
                callers.push(json!({"owner":owner,"compiler_identity":mapping,"native_context":context,
                    "definition":{"mode":if found.len()==1 {found[0].field("mode").and_then(vir::Node::atom)} else {None},"native_body_available":target.is_some(),
                        "external_body":if found.len()==1 {found[0].field("attrs").and_then(|a|a.field("is_external_body")).and_then(vir::Node::atom)} else {None},
                        "representation":if found.is_empty(){"rust_body"}else{"vir_definition"}},
                    "context_error":context_error,
                    "route":if context_error.is_some(){"native_context_unavailable"}else if target.is_some(){"existing_native_client"}else if non_exec {"non_exec_definition"}else if !found.is_empty(){"native_entry_not_connected"}else{"requires_generated_client"},
                    "new_entry_assumptions":0,
                    "context_sources":{"entry":"native_context.function requires and parameters, when present","initialization_and_control":"original source and MIR retained","preceding_calls":"original body, checked by Verus","proof_guidance":"existing proof code retained; no native client hints added"},
                    "trusted_context":dest.join("trusted-context.json")}));
            }
            write_json(&dest.join("callers.json"), &callers)?;
            if callers.is_empty() {
                units.push(json!({"native_input":native,"callers":dest.join("callers.json"),"counts":{},"check":{"route":"no_scoped_callers"}}));
                continue;
            }
            let is_tests = inv.args.iter().any(|a| a == "--test");
            let check = if census_only {
                json!({"route":"catalogue_only","verification_executed":false,"new_proof_coverage":false})
            } else if is_tests {
                let artifacts: Vec<Value> =
                    serde_json::from_slice(&fs::read(capture.join(name).join("artifacts.json"))?)?;
                let selected = artifacts
                    .into_iter()
                    .filter(|a| {
                        a["executable"].is_string()
                            && a["manifest_path"]
                                .as_str()
                                .is_some_and(|m| Path::new(m) == package.join("Cargo.toml"))
                    })
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    return Err("no captured test artifact for caller checks".into());
                }
                let checks = dest.join("checks");
                fs::create_dir(&checks)?;
                json!({"route":"generated_tests","results":runner::check(sdk,package,&checks,&selected,std::slice::from_ref(&inv),runner::Options{verified:true,host:true,loop_guidance:true})?})
            } else {
                let snapshot = dest.join("package");
                runner::copy_package(package, &snapshot)?;
                let roots = inv
                    .args
                    .iter()
                    .filter_map(|a| {
                        let p = inv.cwd.join(a);
                        (a.ends_with(".rs") && p.is_file()).then_some(p)
                    })
                    .collect::<Vec<_>>();
                let [source] = roots.as_slice() else {
                    return Err("single crate source required".into());
                };
                let source = fs::canonicalize(source)?;
                let package = fs::canonicalize(package)?;
                let copied = snapshot.join(source.strip_prefix(&package)?);
                let mut original_sources = BTreeMap::new();
                let mut copied_sources = BTreeMap::new();
                runner::source_hashes(&package, &mut original_sources)?;
                runner::source_hashes(&snapshot, &mut copied_sources)?;
                let identity_edits = original_sources
                    .keys()
                    .cloned()
                    .map(|path| (path, Vec::new()))
                    .collect::<BTreeMap<_, _>>();
                let relative = |files: BTreeMap<std::path::PathBuf, String>,
                                root: &Path|
                 -> Result<BTreeMap<std::path::PathBuf, String>> {
                    files
                        .into_iter()
                        .map(|(p, h)| Ok((p.strip_prefix(root)?.to_path_buf(), h)))
                        .collect()
                };
                if relative(original_sources, &package)? != relative(copied_sources, &snapshot)? {
                    return Err("native client source copy differs".into());
                }

                let args = runner::clean(
                    &inv.args,
                    &dest.join("verifier-output"),
                    Some((&source, &copied)),
                    &inv.cwd,
                )?;
                let selected = callers
                    .iter()
                    .filter(|c| c["route"] == "existing_native_client")
                    .map(|c| c["owner"]["path"].as_str().unwrap())
                    .collect::<Vec<_>>();
                let selection = dest.join("borrow-selection.txt");
                fs::write(&selection, selected.join("\n"))?;
                let probe = runner::build_probe(&inv, &dest)?.ok_or("borrow probe build failed")?;
                let mut context = inv.clone();
                context.env.insert(
                    "CLIENT_PROJECT_BORROW_PATHS".into(),
                    selection.to_string_lossy().into_owned(),
                );
                context
                    .env
                    .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
                context
                    .env
                    .insert("CLIENT_PROJECT_PREFIX".into(), "1".into());
                let original_args =
                    runner::clean(&inv.args, &dest.join("original-output"), None, &inv.cwd)?;
                let (before, original_extraction) = extract_callers(
                    &probe,
                    &context,
                    &original_args,
                    &selected,
                    &dest,
                    "original-borrow",
                )?;
                let (after, generated_extraction) = extract_callers(
                    &probe,
                    &context,
                    &args,
                    &selected,
                    &dest,
                    "generated-borrow",
                )?;
                let mut matches = BTreeMap::new();
                for path in selected {
                    let same = identity_group(&before, &after, path, &package, &snapshot)?;
                    matches.insert(path, same);
                }
                write_json(&dest.join("correspondence.json"), &matches)?;
                let mut verification_args = args.clone();
                if sources.is_some() {
                    let mut modules = std::collections::BTreeSet::new();
                    for caller in callers
                        .iter()
                        .filter(|c| c["route"] == "existing_native_client")
                    {
                        let target = caller["native_context"]["target"]
                            .as_str()
                            .ok_or("missing target")?;
                        let f = functions
                            .iter()
                            .find(|f| {
                                f.field("name")
                                    .and_then(|n| n.field("path"))
                                    .and_then(vir::Node::atom)
                                    == Some(target)
                            })
                            .ok_or("missing native target")?;
                        let module = f
                            .field("owning_module")
                            .and_then(vir::Node::atom)
                            .ok_or("missing native owning module")?;
                        modules.insert(module.split("::").skip(1).collect::<Vec<_>>().join("::"));
                    }
                    for module in modules {
                        if module.is_empty() {
                            verification_args.push("--verify-root".into());
                        } else {
                            verification_args.extend(["--verify-only-module".into(), module]);
                        }
                    }
                }
                let verification =
                    runner::prove(sdk, &inv, &verification_args, &dest, "native-check")?;
                let generated = vir::functions(&fs::read_to_string(
                    dest.join("native-check-vir/crate.vir"),
                )?)?;
                let contract_index = crate::ordinary::ContractIndex::new(&generated);
                let mut checked_targets = BTreeMap::new();
                let mut demand_sites = BTreeMap::new();
                for caller in callers
                    .iter()
                    .filter(|c| c["route"] == "existing_native_client")
                {
                    let target = caller["native_context"]["target"].as_str().unwrap();
                    let found = generated
                        .iter()
                        .filter(|f| {
                            f.field("name")
                                .and_then(|n| n.field("path"))
                                .and_then(vir::Node::atom)
                                == Some(target)
                        })
                        .collect::<Vec<_>>();
                    let checked = matches!(found.as_slice(),[f] if f.field("mode").and_then(vir::Node::atom)==Some("Exec") && f.field("attrs").and_then(|a|a.field("is_external_body")).and_then(vir::Node::atom)==Some("false") && f.field("body").is_some_and(|b|b.list().is_some()));
                    if checked {
                        let mut sites = crate::demand::body_demands(
                            found[0].field("body").unwrap(),
                            &contract_index,
                        );
                        for site in &mut sites {
                            site["original_source"] = crate::demand::source_location(
                                site["span"].as_str(),
                                &package,
                                &snapshot,
                                &identity_edits,
                            );
                        }
                        demand_sites.insert(caller["owner"]["path"].as_str().unwrap(), sites);
                    }
                    checked_targets.insert(caller["owner"]["path"].as_str().unwrap(), checked);
                }
                write_json(&dest.join("checked-targets.json"), &checked_targets)?;
                write_json(
                    &dest.join("native-demand-sites.json"),
                    &json!({
                        "callers": demand_sites,
                        "scope": "Existing verified caller bodies in the generated identity copy; not new proof coverage. Context is syntactic nesting, not a proved path condition. Verification is reported for the selected modules, not independently for each site."
                    }),
                )?;
                let complete = !matches.is_empty()
                    && matches.values().all(|v| *v)
                    && checked_targets.values().all(|v| *v);
                let relations: Value = serde_json::from_slice(&fs::read(evidence)?)?;
                let prefix_checks =
                    crate::prefix::run(&inv, sdk, &dest, &relations, &callers, &functions, &probe)?;
                json!({"route":"existing_native_clients","status":if complete{"checked"}else{"correspondence_incomplete"},"verification":verification,
                    "prefix_checks":prefix_checks,"checked_targets":dest.join("checked-targets.json"),"demand_sites":dest.join("native-demand-sites.json"),"correspondence":dest.join("correspondence.json"),"new_proof_coverage":false,"source_transformation":"identity package copy","extractions":{"original":original_extraction,"generated":generated_extraction}})
            };
            let counts = callers
                .iter()
                .fold(BTreeMap::<String, usize>::new(), |mut m, c| {
                    *m.entry(c["route"].as_str().unwrap().into()).or_default() += 1;
                    m
                });
            units.push(json!({"native_input":native,"callers":dest.join("callers.json"),"counts":counts,"check":check}));
        }
        profiles.push(json!({"profile":name,"units":units}));
    }
    link::validate(links)?;
    write_json(
        &out.join("report.json"),
        &json!({"status":if census_only {"catalogued"} else {"checks_run"},"profiles":profiles,"new_entry_assumptions":0,
        "links":fs::canonicalize(links)?,"capture":capture,
        "limits":["Native route preserves existing contracts and ghost annotations; no general synthesis for unannotated resource callers","Full checks for unregistered callers without a native body remain unavailable; entry-prefix checks have their own limited scope","Kernel ktest has not run; source and pinned compiler correspondence do not prove general Rust semantic refinement"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires PROJECT_CENSUS from a real complete project catalogue"]
    fn census_keeps_compiler_functions_without_claiming_checks() {
        let root = std::path::PathBuf::from(std::env::var("PROJECT_CENSUS").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(root.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["status"], "catalogued");
        let capture = Path::new(report["capture"].as_str().unwrap());
        for profile in report["profiles"].as_array().unwrap() {
            for unit in profile["units"].as_array().unwrap() {
                let file = Path::new(unit["callers"].as_str().unwrap());
                let callers: Vec<Value> = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
                let inventory: Value = serde_json::from_slice(
                    &fs::read(
                        capture
                            .join(profile["profile"].as_str().unwrap())
                            .join(file.parent().unwrap().file_name().unwrap())
                            .join("compile/inventory-records.json"),
                    )
                    .unwrap(),
                )
                .unwrap();
                let expected = inventory["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| {
                        r["kind"] == "owner"
                            && matches!(r["definition_kind"].as_str(), Some("Fn" | "AssocFn"))
                    })
                    .map(|r| r["owner"].as_str().unwrap())
                    .collect::<std::collections::BTreeSet<_>>();
                let actual = callers
                    .iter()
                    .map(|r| r["owner"]["owner"].as_str().unwrap())
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(actual, expected);
                assert_eq!(actual.len(), callers.len());
                assert_eq!(unit["check"]["verification_executed"], false);
                assert_eq!(unit["check"]["new_proof_coverage"], false);
                assert!(
                    !file
                        .parent()
                        .unwrap()
                        .join("verification.command.json")
                        .exists()
                );
                assert!(!file.parent().unwrap().join("checked-targets.json").exists());
            }
        }
    }

    #[test]
    #[ignore = "requires ISOLATION_CONTEXT, ISOLATION_SELECTION, ISOLATION_PROBE and ISOLATION_OUT from Goal 26"]
    fn a_real_projection_failure_does_not_block_another_caller() {
        let inv: Invocation =
            serde_json::from_slice(&fs::read(std::env::var("ISOLATION_CONTEXT").unwrap()).unwrap())
                .unwrap();
        let selection = fs::read_to_string(std::env::var("ISOLATION_SELECTION").unwrap()).unwrap();
        let bad = selection
            .lines()
            .find(|p| p.contains("non_null::either::") && p.ends_with("::raw_as_ref"))
            .unwrap();
        let good = selection
            .lines()
            .find(|p| p.contains("rwarc::") && p.ends_with("::read"))
            .unwrap();
        let out = std::path::PathBuf::from(std::env::var("ISOLATION_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let args = runner::clean(&inv.args, &out.join("compiler-output"), None, &inv.cwd).unwrap();
        let probe = std::path::PathBuf::from(std::env::var("ISOLATION_PROBE").unwrap());
        let (bodies, report) =
            extract_callers(&probe, &inv, &args, &[bad, good], &out, "isolation").unwrap();
        assert_eq!(report["callers"][bad]["status"], "not_analyzed");
        assert_eq!(report["callers"][good]["status"], "typed");
        assert!(!bodies.contains_key(bad) && bodies.contains_key(good));
        assert_eq!(report["attempts"].as_array().unwrap().len(), 3);
        let stderr =
            fs::read_to_string(report["callers"][bad]["stderr"].as_str().unwrap()).unwrap();
        assert!(stderr.contains("Failed to normalize Alias"));
        assert_eq!(report["attempts"][2]["status"], "typed");
    }
    fn record(kind: &str, field: &str, value: &str) -> ir::Record {
        BTreeMap::from([("kind".into(), kind.into()), (field.into(), value.into())])
    }
    #[test]
    fn identity_keeps_paths_calls_assertions_and_consumption() {
        let original = Path::new("/work/ostd");
        let snapshot = Path::new("/tmp/check/package");
        let rows = vec![
            record("decl", "span", "ostd/src/lib.rs:1:1: 2:1 (#7)"),
            record(
                "term",
                "operation",
                "switchInt(copy _1) -> [0: bb1, otherwise: bb2]",
            ),
            record("call", "target", "assertion_check"),
            record("call", "target", "api::write"),
            record("call", "target", "guard::drop"),
        ];
        let mut copy = rows.clone();
        copy[0].insert(
            "span".into(),
            "/tmp/check/package/src/lib.rs:1:1: 2:1 (#92)".into(),
        );
        assert!(identity_records(&rows, &copy, original, snapshot).unwrap());
        for omitted in 1..rows.len() {
            let mut wrong = copy.clone();
            wrong.remove(omitted);
            assert!(!identity_records(&rows, &wrong, original, snapshot).unwrap());
        }
        let mut wrong = copy.clone();
        wrong[1].insert("operation".into(), "goto -> bb1".into());
        assert!(!identity_records(&rows, &wrong, original, snapshot).unwrap());
        let a = [record(
            "stmt",
            "operation",
            "_7 = {closure@ostd/src/lib.rs:1:2: 1:8}",
        )];
        let b = [record(
            "stmt",
            "operation",
            "_7 = {closure@/tmp/check/package/src/lib.rs:1:2: 1:8}",
        )];
        assert!(identity_records(&a, &b, original, snapshot).unwrap());
        // file!() values are executable constants, not source-location metadata.
        let a = [record(
            "stmt",
            "operation",
            "_1 = const \"ostd/src/lib.rs\"",
        )];
        let b = [record(
            "stmt",
            "operation",
            "_1 = const \"/tmp/check/package/src/lib.rs\"",
        )];
        assert!(!identity_records(&a, &b, original, snapshot).unwrap());
    }
    #[test]
    fn nested_body_omissions_are_rejected() {
        let before = BTreeMap::from([
            ("f".into(), vec![record("term", "operation", "return")]),
            (
                "f::{closure#0}".into(),
                vec![record("term", "operation", "return")],
            ),
        ]);
        let mut after = before.clone();
        assert!(identity_group(&before, &after, "f", Path::new("/p"), Path::new("/q")).unwrap());
        after.remove("f::{closure#0}");
        assert!(!identity_group(&before, &after, "f", Path::new("/p"), Path::new("/q")).unwrap());
    }

    #[test]
    fn partial_or_disabled_verification_cannot_check_all_callers() {
        let mut inv:Invocation=serde_json::from_value(json!({"cwd":"/p","rustc":"rustc","args":[],"env":{"CARGO_PKG_NAME":"p","__VERUS_DRIVER_VERIFY_p-0.1.0-id":"1"},"unsupported_env":[]})).unwrap();
        assert!(full_verification_context(&inv).is_ok());
        inv.args.push("--verify-function".into());
        assert!(full_verification_context(&inv).is_err());
        inv.args.clear();
        inv.env
            .insert("__VERUS_DRIVER_VERIFY_p-0.1.0-id".into(), "0".into());
        assert!(full_verification_context(&inv).is_err());
    }

    #[test]
    fn incomplete_and_duplicated_borrow_groups_are_rejected() {
        assert!(borrow_records("CALLER_BEGIN\tf\n").is_err());
        assert!(borrow_records("CALLER_END\tf\n").is_err());
        assert!(borrow_records("CALLER_BEGIN\tf\nCALLER_BEGIN\tg\n").is_err());
        assert!(
            borrow_records("CALLER_BEGIN\tf\nCALLER_END\tf\nCALLER_BEGIN\tf\nCALLER_END\tf\n")
                .is_err()
        );
    }
}

#[cfg(test)]
mod real_controls {
    use super::*;
    #[test]
    #[ignore = "requires PROJECT_CLIENT_RUN, PROJECT_CLIENT_CAPTURE and new PROJECT_CLIENT_CONTROLS"]
    fn omitted_assertion_path_call_and_resource_are_rejected() {
        let run = std::path::PathBuf::from(std::env::var("PROJECT_CLIENT_RUN").unwrap());
        let capture = std::path::PathBuf::from(std::env::var("PROJECT_CLIENT_CAPTURE").unwrap());
        let out = std::path::PathBuf::from(std::env::var("PROJECT_CLIENT_CONTROLS").unwrap());
        fs::create_dir(&out).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        let plan: Value = serde_json::from_str(include_str!(
            "../assets/experiments/test-spec/client-check-plan.json"
        ))
        .unwrap();
        let branch = plan["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["coverage"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "no_assertion")
            })
            .unwrap();
        let resource = plan["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["coverage"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "existing_tracked_entry")
            })
            .unwrap();
        let mut outcomes = Vec::new();
        for (name, profile, selector, from, to) in [
            (
                "path",
                "kernel-library",
                branch["owner"]["path"].as_str().unwrap(),
                "match self.typ()",
                "match MemoryRegionType::Usable",
            ),
            (
                "call",
                "kernel-library",
                branch["owner"]["path"].as_str().unwrap(),
                ".align_up(PAGE_SIZE)",
                ".wrapping_add(PAGE_SIZE)",
            ),
            (
                "resource_consumption",
                "kernel-library",
                resource["owner"]["path"].as_str().unwrap(),
                "lock_guard.drop();",
                "/* omitted explicit resource return */",
            ),
            (
                "ghost_resource_consumption",
                "kernel-library",
                resource["owner"]["path"].as_str().unwrap(),
                "resource.remaining.combine(allocated.tracked_into_subset());",
                "/* omitted tracked resource consumption */",
            ),
            (
                "assertion",
                "host-tests",
                "test::test_align_down",
                "assert!(n.align_down(a) == n2);",
                "/* omitted assertion */",
            ),
        ] {
            let unit = &report["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["profile"] == profile)
                .unwrap()["units"][0];
            let base = Path::new(unit["callers"].as_str().unwrap())
                .parent()
                .unwrap();
            let inv: Invocation = serde_json::from_slice(
                &fs::read(capture.join(profile).join("unit-0/context.json")).unwrap(),
            )
            .unwrap();
            let package = Path::new(&inv.env["CARGO_MANIFEST_DIR"])
                .canonicalize()
                .unwrap();
            let (input, before, editing) = if profile == "host-tests" {
                let sample = unit["check"]["results"][0]["samples"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["function"] == selector)
                    .unwrap();
                let case = Path::new(sample["contracts"].as_str().unwrap())
                    .parent()
                    .unwrap();
                (
                    case.join("package"),
                    ir::parse(&fs::read_to_string(case.join("original.stderr")).unwrap()).unwrap(),
                    ir::parse(&fs::read_to_string(case.join("checked.stderr")).unwrap()).unwrap(),
                )
            } else {
                let groups = borrow_records(
                    &fs::read_to_string(base.join("original-borrow.stderr")).unwrap(),
                )
                .unwrap();
                (
                    base.join("package"),
                    groups[selector].clone(),
                    groups[selector].clone(),
                )
            };
            let case = out.join(name);
            fs::create_dir(&case).unwrap();
            let snapshot = case.join("package");
            runner::copy_package(&input, &snapshot).unwrap();
            let decl = before.iter().find(|r| r["kind"] == "decl").unwrap();
            let (file, _) = ir::span(&decl["span"], &inv.cwd).unwrap();
            let relative = file
                .canonicalize()
                .unwrap()
                .strip_prefix(&package)
                .unwrap()
                .to_path_buf();
            let path = snapshot.join(relative);
            let code = fs::read_to_string(&path).unwrap();
            let body = editing.iter().find(|r| r["kind"] == "body").unwrap();
            let (_, coords) = ir::span(&body["span"], &inv.cwd).unwrap();
            let (start, end) = ir::offsets(&code, coords).unwrap();
            let snippet = &code[start..end];
            assert!(
                snippet.contains(from),
                "control does not apply to selected caller: {name}"
            );
            fs::write(
                &path,
                format!(
                    "{}{}{}",
                    &code[..start],
                    snippet.replacen(from, to, 1),
                    &code[end..]
                ),
            )
            .unwrap();
            let root = inv
                .args
                .iter()
                .map(|a| inv.cwd.join(a))
                .find(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
                .unwrap()
                .canonicalize()
                .unwrap();
            let copied = snapshot.join(root.strip_prefix(&package).unwrap());
            let selection = case.join("selection.txt");
            fs::write(&selection, selector).unwrap();
            let mut context = inv.clone();
            context
                .env
                .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
            context.env.insert(
                "CLIENT_PROJECT_BORROW_PATHS".into(),
                selection.to_string_lossy().into_owned(),
            );
            let probe = runner::build_probe(&context, &case).unwrap().unwrap();
            let args = runner::clean(
                &inv.args,
                &case.join("metadata"),
                Some((&root, &copied)),
                &inv.cwd,
            )
            .unwrap();
            runner::extract_context(&probe, &context, &args, "*", &case, "mutant", true).unwrap();
            let groups =
                borrow_records(&fs::read_to_string(case.join("mutant.stderr")).unwrap()).unwrap();
            let after = &groups[selector];
            let accepted = if profile == "host-tests" {
                ir::compare(&before, after, None).unwrap_or(false)
            } else {
                identity_records(&before, after, &package, &snapshot).unwrap()
            };
            assert!(!accepted, "omitted {name} accepted");
            let physical = |rows: &[ir::Record]| {
                rows.iter()
                    .filter(|r| {
                        matches!(
                            r["kind"].as_str(),
                            "local"
                                | "stmt"
                                | "term"
                                | "call"
                                | "edge"
                                | "loan"
                                | "loan_issued"
                                | "loan_out_of_scope"
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let physical_equal =
                identity_records(&physical(&before), &physical(after), &package, &snapshot)
                    .unwrap();
            if name == "ghost_resource_consumption" {
                assert!(
                    physical_equal,
                    "ghost-only omission unexpectedly changed runtime MIR"
                );
            }

            outcomes.push(json!({"omitted":name,"caller":selector,"rustc_typechecked":true,"correspondence_rejected":true,"runtime_mir_equal":physical_equal,"runtime_executed":false}));
        }
        write_json(&out.join("results.json"), &outcomes).unwrap();
    }
}
