//! Source test candidates before cfg expansion and type checking.
use crate::{Result, capture::Invocation, hash, runner, write_json};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

fn decode(value: &str) -> Result<String> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return Err("invalid AST record encoding".into());
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(String::from_utf8(bytes)?)
}

fn items(text: &str) -> Result<Vec<Value>> {
    let mut items = BTreeMap::new();
    for line in text.lines().filter(|s| s.starts_with("PARSE_")) {
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["PARSE_ITEM", kind, path, span, start, end] => {
                let span = decode(span)?;
                if items.insert(span.clone(), json!({"kind":kind,"path":decode(path)?,"span":span,"attributes":[],"test_attributes":[],"source_bytes":[start.parse::<usize>()?,end.parse::<usize>()?]})).is_some() {
                    return Err("duplicate AST item identity".into());
                }
            }
            ["PARSE_ATTR", owner, span, role, start, end, text] => {
                let item = items
                    .get_mut(&decode(owner)?)
                    .ok_or("AST attribute lacks owner")?;
                item["attributes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"span":decode(span)?,"role":role,"text":decode(text)?,"source_bytes":[start.parse::<usize>()?,end.parse::<usize>()?]}));
            }
            ["PARSE_TEST", owner, name] => {
                let item = items
                    .get_mut(&decode(owner)?)
                    .ok_or("AST test lacks owner")?;
                item["test_attributes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(name));
            }
            ["PARSE_BODY", owner, span, start, end, text] => {
                let item = items
                    .get_mut(&decode(owner)?)
                    .ok_or("AST body lacks owner")?;
                item["body"] = json!({"span":decode(span)?,"text":decode(text)?,"source_bytes":[start.parse::<usize>()?,end.parse::<usize>()?]});
            }
            ["PARSE_COMPLETE"] => {}
            _ => return Err("unknown AST record".into()),
        }
    }
    Ok(items.into_values().collect())
}

fn bound_items(text: &str, bytes: &[u8]) -> Result<Vec<Value>> {
    let mut records = items(text)?;
    for item in &mut records {
        source_slice(bytes, item)?;
        for attr in item["attributes"].as_array().ok_or("missing attributes")? {
            if source_slice(bytes, attr)?.replace("\r\n", "\n")
                != attr["text"].as_str().ok_or("missing attribute text")?
            {
                return Err("AST attribute bytes disagree with parsed text".into());
            }
        }
        if let Some(body) = item.get_mut("body") {
            let text = source_slice(bytes, body)?;
            if text.replace("\r\n", "\n") != body["text"].as_str().ok_or("missing AST body text")? {
                return Err("AST body bytes disagree with parsed text".into());
            }
            body["source_text"] = json!(text);
        }
    }
    Ok(records)
}
fn candidates(records: &[Value], all_functions: bool) -> Vec<Value> {
    records
        .iter()
        .filter(|r| {
            r["kind"] == "fn"
                && (all_functions
                    || r["test_attributes"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty()))
        })
        .cloned()
        .collect()
}

pub(crate) fn run(context: &Path, source: &Path, out: &Path, all_functions: bool) -> Result<()> {
    let context_bytes = fs::read(context)?;
    let context_hash = hash(&context_bytes)?;
    let inv: Invocation = serde_json::from_slice(&context_bytes)?;
    let source = source.canonicalize()?;
    let source_bytes = fs::read(&source)?;
    let original = hash(&source_bytes)?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let probe = runner::build_source_probe(
        &inv,
        &out,
        include_str!("../../../experiments/rustc-ir/test_ast_probe.rs"),
        "test-ast-probe",
    )?
    .ok_or("Rustc AST probe build failed")?;
    let edition = inv
        .args
        .iter()
        .find(|a| a.starts_with("--edition="))
        .cloned()
        .or_else(|| {
            inv.args
                .windows(2)
                .find(|a| a[0] == "--edition")
                .map(|a| format!("--edition={}", a[1]))
        })
        .unwrap_or_else(|| "--edition=2015".into());
    let result = crate::execute(
        Command::new(probe)
            .arg(&source)
            .args(["--crate-type", "lib", &edition]),
        &out,
        "parse",
    )?;
    let records = bound_items(&String::from_utf8_lossy(&result.stderr), &source_bytes)?;
    let complete = result.status.success()
        && String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter(|s| *s == "PARSE_COMPLETE")
            .count()
            == 1;
    if hash(&fs::read(context)?)? != context_hash {
        return Err("compiler context changed during test census".into());
    }
    if hash(&fs::read(&source)?)? != original {
        return Err("source changed during test census".into());
    }
    let tests = if complete {
        candidates(&records, all_functions)
    } else {
        Vec::new()
    };
    write_json(
        &out.join("report.json"),
        &json!({
            "format_version":2,"selection":if all_functions{"functions"}else{"tests"},"status":if complete {"parsed"} else {"parse_failed"},"source":source,"source_sha256":original,
            "context":context.canonicalize()?,"context_sha256":context_hash,"parse_complete":complete,"items":records,"test_candidates":tests,
            "verification_executed":false,"runtime_executed":false,
            "scope":"Explicit function items in one parsed source file, selected by test attributes or all-functions mode. cfg is retained without evaluation; macro token bodies and out-of-line modules are not expanded. Candidates are not a compiled test registry or typed clients."
        }),
    )?;
    if complete {
        Ok(())
    } else {
        Err("source parse failed; see report.json".into())
    }
}

fn source_slice<'a>(bytes: &'a [u8], node: &Value) -> Result<&'a str> {
    let start = usize::try_from(
        node["source_bytes"][0]
            .as_u64()
            .ok_or("missing AST byte start")?,
    )?;
    let end = usize::try_from(
        node["source_bytes"][1]
            .as_u64()
            .ok_or("missing AST byte end")?,
    )?;
    Ok(std::str::from_utf8(
        bytes.get(start..end).ok_or("AST range outside source")?,
    )?)
}

fn annotation_correspondence(
    inv: &Invocation,
    probe: &Path,
    case: &Path,
    relative: &Path,
    runtime_source: &[u8],
    native_path: &str,
) -> Result<Value> {
    let package = case.join("package");
    let entry = inv
        .args
        .iter()
        .find(|a| a.ends_with(".rs"))
        .ok_or("missing projected crate root")?;
    let entry = inv.cwd.join(entry).canonicalize()?;
    let crate_name = inv
        .args
        .windows(2)
        .find(|a| a[0] == "--crate-name")
        .ok_or("missing crate name")?[1]
        .as_str();
    let target = native_path
        .strip_prefix(&format!("{crate_name}::"))
        .ok_or("native target crate differs")?;
    let runtime = case.join("runtime-package");
    runner::copy_package(&package, &runtime)?;
    fs::write(runtime.join(relative), runtime_source)?;
    let mut data = Vec::new();
    for (label, root) in [("runtime", &runtime), ("annotated", &package)] {
        let args = runner::clean(
            &inv.args,
            &case.join(format!("{label}-output")),
            Some((&entry, &root.join(entry.strip_prefix(&package)?))),
            &inv.cwd,
        )?;
        data.push(runner::extract_context(
            probe, inv, &args, target, case, label, true,
        )?);
    }
    let relative_hashes = |root: &Path| -> Result<BTreeMap<std::path::PathBuf, String>> {
        let mut files = BTreeMap::new();
        runner::source_hashes(root, &mut files)?;
        files
            .into_iter()
            .map(|(file, digest)| Ok((file.strip_prefix(root)?.to_path_buf(), digest)))
            .collect()
    };
    let mut expected = relative_hashes(&package)?;
    expected.insert(relative.to_path_buf(), hash(runtime_source)?);
    if relative_hashes(&runtime)? != expected {
        return Err("runtime context changed outside the proof marker".into());
    }
    let matches = crate::ir::compare(&data[0], &data[1], None)?;
    let result = json!({"status":if matches{"matched"}else{"mismatch"},"matches":matches,"target":target,
        "explicit_assertions":data[0].iter().filter(|r|r["kind"] == "assert_operand").count(),
        "scope":"Runtime MIR and borrow facts in the same isolated context before and after the proof marker; original-module adaptation remains separate.",
        "runtime_source_sha256":hash(runtime_source)?});
    write_json(&case.join("annotation-correspondence.json"), &result)?;
    Ok(result)
}

pub(crate) fn validate(census: &Path) -> Result<Value> {
    let report: Value = serde_json::from_slice(&fs::read(census.join("report.json"))?)?;
    if report["format_version"] != 2 || report["parse_complete"] != true {
        return Err("isolation requires a current complete AST census".into());
    }
    let source =
        Path::new(report["source"].as_str().ok_or("missing source identity")?).canonicalize()?;
    let bytes = fs::read(&source)?;
    if hash(&bytes)? != report["source_sha256"] {
        return Err("stale AST source".into());
    }
    let raw = fs::read_to_string(census.join("parse.stderr"))?;
    let records = bound_items(&raw, &bytes)?;
    let all_functions = match report["selection"].as_str() {
        None | Some("tests") => false,
        Some("functions") => true,
        _ => return Err("unknown source candidate selection".into()),
    };
    if raw.lines().filter(|s| *s == "PARSE_COMPLETE").count() != 1
        || json!(records) != report["items"]
        || json!(candidates(&records, all_functions)) != report["test_candidates"]
    {
        return Err("AST census differs from complete raw records".into());
    }
    let context = Path::new(
        report["context"]
            .as_str()
            .ok_or("missing compiler context")?,
    );
    let context_bytes = fs::read(context)?;
    if hash(&context_bytes)? != report["context_sha256"] {
        return Err("stale compiler context".into());
    }
    Ok(report)
}

pub(crate) fn isolate(census: &Path, out: &Path, sdk: Option<&Path>) -> Result<()> {
    let report = validate(census)?;
    let source = Path::new(report["source"].as_str().unwrap()).canonicalize()?;
    let bytes = fs::read(&source)?;
    let records = report["items"].as_array().ok_or("missing AST items")?;
    let context = Path::new(report["context"].as_str().unwrap());
    let context_bytes = fs::read(context)?;
    let inv: Invocation = serde_json::from_slice(&context_bytes)?;
    let package = Path::new(
        inv.env
            .get("CARGO_MANIFEST_DIR")
            .ok_or("missing package context")?,
    )
    .canonicalize()?;
    let relative = source.strip_prefix(&package)?;
    if out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?
        .starts_with(&package)
    {
        return Err("isolation output must be outside the source package".into());
    }

    let entries = inv
        .args
        .iter()
        .filter(|s| s.ends_with(".rs"))
        .map(|s| inv.cwd.join(s).canonicalize())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let [entry] = entries.as_slice() else {
        return Err("one crate root required".into());
    };
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let mut originals = BTreeMap::new();
    runner::source_hashes(&package, &mut originals)?;
    write_json(&out.join("original-source-hashes.json"), &originals)?;
    let mut cases = Vec::new();
    let mut mir_probe = None;
    let mut module_contexts = BTreeMap::new();
    for (index, test) in report["test_candidates"]
        .as_array()
        .ok_or("missing test list")?
        .iter()
        .enumerate()
    {
        let case = out.join(format!("case-{index}"));
        fs::create_dir(&case)?;
        let mut phase = "preparation";
        let mut work = || -> Result<Value> {
            let path = test["path"].as_str().ok_or("missing test path")?;
            let module = match path.rsplit_once("::") {
                None => "<crate>",
                Some((parent, _)) if !parent.contains("::") => parent,
                _ => return Err(
                    "isolation currently supports crate-root tests and immediate inline modules"
                        .into(),
                ),
            };
            if sdk.is_some() && !module_contexts.contains_key(module) {
                let context_dir = out.join(format!("module-{}", module_contexts.len()));
                fs::create_dir(&context_dir)?;
                let checked = (|| -> Result<Value> {
                    let root = context_dir.join("package");
                    runner::copy_package(&package, &root)?;
                    let mut activated = bytes.clone();
                    let mut edits = Vec::new();
                    for item in records {
                        let owner = item["path"].as_str().ok_or("missing AST path")?;
                        for attr in item["attributes"].as_array().ok_or("missing attributes")? {
                            let text = source_slice(&bytes, attr)?;
                            let cfg: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                            if (owner == module && matches!(cfg.as_str(), "#[cfg(ktest)]" | "#[cfg(test)]"))
                                || ((if module == "<crate>" { !owner.contains("::") } else { owner.starts_with(&format!("{module}::")) })
                                    && matches!(attr["role"].as_str(), Some("ktest" | "test")))
                            {
                                let start = attr["source_bytes"][0].as_u64().ok_or("missing byte start")? as usize;
                                for byte in &mut activated[start..start + text.len()] {
                                    if !matches!(*byte,b'\r'|b'\n') { *byte=b' '; }
                                }
                                edits.push(attr.clone());
                            }
                        }
                    }
                    fs::write(root.join(relative), &activated)?;
                    write_json(&context_dir.join("edits.json"), &edits)?;
                    let probe = runner::build_probe(&inv, &context_dir)?.ok_or("MIR probe build failed")?;
                    let args = runner::clean(&inv.args, &context_dir.join("output"), Some((entry, &root.join(entry.strip_prefix(&package)?))), &inv.cwd)?;
                    let result = runner::compile(&mut runner::probe_command(&probe, &inv, &args, "*", true), &context_dir, "compile", &inv.args)?;
                    let diagnostics = String::from_utf8_lossy(&result.stderr).lines()
                        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                        .filter(|d| d["level"] == "error")
                        .collect::<Vec<_>>();
                    let mut actual = BTreeMap::new();
                    runner::source_hashes(&root, &mut actual)?;
                    for (file, digest) in &originals {
                        let file = file.strip_prefix(&package)?;
                        let expected = if file == relative { hash(&activated)? } else { digest.clone() };
                        if actual.remove(&root.join(file)).as_ref() != Some(&expected) {
                            return Err("original module source changed outside recorded activation".into());
                        }
                    }
                    if !actual.is_empty() { return Err("unexpected source in original module context".into()); }
                    let typed = String::from_utf8_lossy(&result.stderr).lines()
                        .filter_map(|line| line.strip_prefix("CLIENT_CANDIDATE\t"))
                        .filter_map(|line| line.split_once('\t'))
                        .filter(|(_, span)| span.starts_with(&format!("{}:", root.join(relative).display())))
                        .map(|(path, span)| json!({"path":path,"span":span}))
                        .collect::<Vec<_>>();
                    Ok(json!({"status":if result.status.success(){"compiled"}else{"compile_failed"},"exit_code":result.status.code(),"typed_functions":typed,"source":root.join(relative),"source_sha256":hash(&activated)?,"diagnostics":diagnostics,"edits":context_dir.join("edits.json"),"runtime_executed":false,"scope":"Original module imports and helpers retained; only explicit test registration and module test cfg attributes removed. Compilation success alone does not close typed correspondence."}))
                })().unwrap_or_else(|error| json!({"status":"not_analyzed","reason":error.to_string()}));
                write_json(&context_dir.join("result.json"), &checked)?;
                module_contexts.insert(module.to_owned(), checked);
            }
            let attrs = test["attributes"].as_array().ok_or("missing attributes")?;
            if attrs
                .iter()
                .any(|a| matches!(a["role"].as_str(), Some("should_panic" | "ignore")))
            {
                return Err("expected-panic and ignored tests require their own policy".into());
            }
            let function = source_slice(&bytes, test)?;
            let attributes = attrs
                .iter()
                .filter(|a| !matches!(a["role"].as_str(), Some("ktest" | "test")))
                .map(|a| source_slice(&bytes, a))
                .collect::<Result<Vec<_>>>()?
                .join("\n");
            let header = format!("\nmod __spec_quality_test {{\nuse super::*;\n{attributes}\n");
            let tail = format!("{function}\n}}\n");
            let runtime_addition = format!("{header}{tail}");
            let addition = format!("{header}#[vstd::prelude::verus_verify]\n{tail}");
            let original_module = module_contexts.get(module);
            let original_target = original_module
                .filter(|m| m["status"] == "compiled")
                .and_then(|m| m["typed_functions"].as_array())
                .map(|functions| {
                    functions
                        .iter()
                        .filter(|f| {
                            let start = |node: &Value| {
                                node["span"]
                                    .as_str()
                                    .and_then(|s| crate::ir::span(s, &inv.cwd).ok())
                                    .map(|(_, c)| [c[0], c[1]])
                            };
                            start(f).is_some() && start(f) == start(test)
                        })
                        .filter_map(|f| f["path"].as_str())
                        .filter(|p| *p == path || p.ends_with(&format!("::{path}")))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let preserved = original_target.len() == 1;
            let presence = if original_module.is_some_and(|m| m["status"] == "compiled") {
                match original_target.len() {
                    0 => "not_selected_by_compiler",
                    1 => "present",
                    _ => "ambiguous",
                }
            } else {
                "unavailable"
            };
            let snapshot = case.join("package");
            runner::copy_package(&package, &snapshot)?;
            for (file, digest) in &originals {
                if hash(&fs::read(snapshot.join(file.strip_prefix(&package)?))?)? != *digest {
                    return Err("source copy differs before isolation".into());
                }
            }
            let mut runtime_source = bytes.clone();
            let candidate = if preserved {
                runtime_source = fs::read(
                    original_module.unwrap()["source"]
                        .as_str()
                        .ok_or("missing activated source")?,
                )?;
                if hash(&runtime_source)? != original_module.unwrap()["source_sha256"]
                    || source_slice(&runtime_source, test)? != function
                {
                    return Err("compiled original module changed before annotation".into());
                }
                let offset = test["source_bytes"][0]
                    .as_u64()
                    .ok_or("missing function start")? as usize;
                let mut candidate = runtime_source[..offset].to_vec();
                candidate.extend_from_slice(b"#[vstd::prelude::verus_verify]\n");
                candidate.extend_from_slice(&runtime_source[offset..]);
                candidate
            } else {
                runtime_source.extend_from_slice(runtime_addition.as_bytes());
                let mut candidate = bytes.clone();
                candidate.extend_from_slice(addition.as_bytes());
                candidate
            };
            let candidate_hash = hash(&candidate)?;
            fs::write(snapshot.join(relative), &candidate)?;
            let mut projected = inv.clone();
            for arg in &mut projected.args {
                if arg.ends_with(".rs") && inv.cwd.join(&*arg).canonicalize()? == *entry {
                    *arg = snapshot
                        .join(entry.strip_prefix(&package)?)
                        .to_string_lossy()
                        .into_owned();
                }
            }
            let check_args =
                runner::clean(&projected.args, &case.join("output"), None, &projected.cwd)?;
            write_json(&case.join("context.json"), &projected)?;
            let check = if let Some(sdk) = sdk {
                phase = "verification";
                let verification = runner::prove(sdk, &projected, &check_args, &case, "check")?;
                let suffix = if preserved {
                    format!("::{}", original_target[0])
                } else {
                    format!(
                        "::__spec_quality_test::{}",
                        path.rsplit("::").next().unwrap()
                    )
                };
                let native = fs::read_to_string(case.join("check-vir/crate.vir"))
                    .ok()
                    .map(|text| crate::vir::functions(&text))
                    .transpose()?
                    .unwrap_or_default();
                let targets = native.iter().filter(|f| f.field("name").and_then(|n|n.field("path")).and_then(crate::vir::Node::atom).is_some_and(|p|p.ends_with(&suffix)))
                    .map(|f|json!({"path":f.field("name").and_then(|n|n.field("path")).and_then(crate::vir::Node::atom),"mode":f.field("mode").and_then(crate::vir::Node::atom),"external_body":f.field("attrs").and_then(|a|a.field("is_external_body")).and_then(crate::vir::Node::atom),"body_available":f.field("body").is_some_and(|b|b.list().is_some())})).collect::<Vec<_>>();
                let conversion = if targets.len() == 1
                    && targets[0]["mode"] == "Exec"
                    && targets[0]["external_body"] == "false"
                    && targets[0]["body_available"] == true
                {
                    if mir_probe.is_none() {
                        mir_probe = runner::build_probe(&inv, &out)?;
                    }
                    if let Some(probe) = &mir_probe {
                        match annotation_correspondence(
                            &projected,
                            probe,
                            &case,
                            relative,
                            &runtime_source,
                            targets[0]["path"].as_str().unwrap(),
                        ) {
                            Ok(value) => value,
                            Err(error) => {
                                json!({"status":"not_analyzed","matches":false,"reason":error.to_string()})
                            }
                        }
                    } else {
                        json!({"status":"tool_build_failed","matches":false})
                    }
                } else {
                    json!({"status":"target_unavailable","matches":false})
                };
                let body_complete = preserved
                    && conversion["matches"] == true
                    && matches!(
                        verification["status"].as_str(),
                        Some("proved" | "not_proved")
                    );
                let complete = body_complete
                    && conversion["explicit_assertions"]
                        .as_u64()
                        .is_some_and(|n| n > 0);
                let review = if preserved {
                    (|| -> Result<Value> {
                        let text = fs::read_to_string(case.join("check-vir/crate.vir"))?;
                        let offset = test["source_bytes"][0]
                            .as_u64()
                            .ok_or("missing function start")?
                            as usize;
                        let edits = [crate::ir::Edit {
                            start: offset,
                            end: offset,
                            replacement: "#[vstd::prelude::verus_verify]\n".into(),
                            origin: test["span"].as_str().unwrap_or("").into(),
                        }];
                        crate::review::summarize(
                            &verification,
                            &text,
                            &projected.cwd,
                            &source,
                            &snapshot.join(relative),
                            &edits,
                            &[],
                        )
                    })()
                    .unwrap_or_else(
                        |error| json!({"status":"not_analyzed","reason":error.to_string()}),
                    )
                } else {
                    json!({"status":"not_analyzed","reason":"original module context unavailable"})
                };
                let index_proposal = if body_complete && verification["status"] == "not_proved" {
                    (|| -> Result<Value> {
                        let capture: Value =
                            serde_json::from_slice(&fs::read(case.join("runtime.json"))?)?;
                        let records: Vec<crate::ir::Record> =
                            serde_json::from_value(capture["records"].clone())?;
                        let Some(mut copy) =
                            crate::std_body::index_propose(&records, &projected.cwd)?
                        else {
                            return Ok(json!({"status":"not_applicable"}));
                        };
                        let offset = test["source_bytes"][0]
                            .as_u64()
                            .ok_or("missing function start")?
                            as usize;
                        copy.edits.push(crate::ir::Edit {
                            start: offset,
                            end: offset,
                            replacement: "#[vstd::prelude::verus_verify]\n".into(),
                            origin: test["span"].as_str().unwrap_or("").into(),
                        });
                        copy.edits.sort_by_key(|e| (e.start, e.end));
                        if copy.edits.windows(2).any(|w| w[0].end > w[1].start) {
                            return Err("overlapping index candidate edits".into());
                        }
                        let mut text = fs::read_to_string(&copy.file)?;
                        for edit in copy.edits.iter().rev() {
                            text.replace_range(edit.start..edit.end, &edit.replacement);
                        }
                        let file = case.join("index-candidate.rs");
                        fs::write(&file, text)?;
                        write_json(&case.join("index-candidate-edits.json"), &copy.edits)?;
                        copy.evidence["source"] = json!(file);
                        copy.evidence["edits"] = json!(case.join("index-candidate-edits.json"));
                        let trial = case.join("index-check");
                        fs::create_dir(&trial)?;
                        let root = trial.join("package");
                        runner::copy_package(&snapshot, &root)?;
                        fs::copy(&file, root.join(relative))?;
                        let mut trial_sources = BTreeMap::new();
                        runner::source_hashes(&root, &mut trial_sources)?;
                        let mut invocation = projected.clone();
                        let old_entry = snapshot.join(entry.strip_prefix(&package)?);
                        for arg in &mut invocation.args {
                            if arg.ends_with(".rs")
                                && projected.cwd.join(&*arg).canonicalize()? == old_entry
                            {
                                *arg = root
                                    .join(entry.strip_prefix(&package)?)
                                    .to_string_lossy()
                                    .into_owned();
                            }
                        }
                        let args = runner::clean(
                            &invocation.args,
                            &trial.join("output"),
                            None,
                            &invocation.cwd,
                        )?;
                        write_json(&trial.join("context.json"), &invocation)?;
                        copy.evidence["verification"] =
                            runner::prove(sdk, &invocation, &args, &trial, "check")?;
                        if let Some(probe) = &mir_probe {
                            let comparison = (|| -> Result<Value> {
                            let after = runner::extract_context(
                                probe, &invocation, &args,
                                conversion["target"].as_str().ok_or("missing original MIR target")?,
                                &trial, "runtime", true,
                            )?;
                            Ok(json!({
                                "matches":crate::ir::compare(&records, &after, None)?,
                                "body_substitution":crate::std_body::index_correspondence(&records, &after, &invocation.cwd, &root.join(relative))?,
                                "original":case.join("runtime.json"),
                                "candidate":trial.join("runtime.json"),
                                "scope":"Unmodified MIR and borrow-event comparator with original call names. The bounded copied-body mapping is reported separately."}))
                            })();
                            copy.evidence["runtime_comparison"] = comparison.unwrap_or_else(|error| json!({"matches":false,"status":"not_analyzed","reason":error.to_string()}));
                        }
                        let mut checked_sources = BTreeMap::new();
                        runner::source_hashes(&root, &mut checked_sources)?;
                        if trial_sources != checked_sources {
                            return Err("candidate source changed during verification or extraction".into());
                        }
                        let matched = copy.evidence["runtime_comparison"]["body_substitution"]["matches"] == true
                            && copy.evidence["verification"]["status"] == "proved";
                        copy.evidence["status"] = json!(if matched { "candidate_verified_with_bounded_correspondence" } else { "candidate_checked_correspondence_pending" });
                        copy.evidence["scope"] = json!("Generated candidate checked separately, with a bounded copied-body relation when supported. Original verification outcome and frozen migration outcome unchanged.");
                        Ok(copy.evidence)
                    })()
                    .unwrap_or_else(
                        |error| json!({"status":"unsupported","reason":error.to_string()}),
                    )
                } else {
                    json!({"status":"not_applicable"})
                };
                json!({"verification":verification,"verification_scope":"Selected function and nested local functions; diagnostics locate failures, totals are not per-function proof results.","review":review,"index_proposal":index_proposal,"native_targets":targets,"annotation_correspondence":conversion,"original_context_correspondence":if preserved{"source_preserved_after_recorded_activation"}else{"pending"},"body_check_complete":body_complete,"complete_property_coverage":complete,"input_scope":if preserved{"original_module_after_recorded_test_activation"}else{"isolated_context_candidate"}})
            } else {
                Value::Null
            };

            phase = "integrity";
            for (file, digest) in &originals {
                let relative_file = file.strip_prefix(&package)?;
                let expected = if relative_file == relative {
                    &candidate_hash
                } else {
                    digest
                };
                if hash(&fs::read(snapshot.join(relative_file))?)? != *expected {
                    return Err("isolated source changed during preparation or checking".into());
                }
            }
            Ok(
                json!({"status":"prepared_context_candidate","context":case.join("context.json"),"source":snapshot.join(relative),"original_test":path,"original_test_presence":presence,"original_body":test["body"],"source_sha256":candidate_hash,"addition":if preserved{Value::Null}else{json!(addition)},"runtime_addition":if preserved{Value::Null}else{json!(runtime_addition)},"function_source_sha256":hash(function.as_bytes())?,"entry_conditions_added":0,"new_trusted_declarations":0,"correspondence":"pending","verification_executed":sdk.is_some(),"check":check,"context_policy":if preserved{"Original module retained after recorded activation; only selected function gains the proof marker."}else{"Sibling module with super::* only; original test-module imports and helpers are not copied. Unresolved dependencies remain explicit conversion limitations."}}),
            )
        };
        let result = match work() {
            Ok(value) => value,
            Err(error) => {
                json!({"status":match phase {"verification"=>"execution_failed","integrity"=>"correspondence_failed",_=>"unsupported_context"},"phase":phase,"reason":error.to_string()})
            }
        };
        write_json(&case.join("preparation.json"), &result)?;
        cases.push(json!({"test":test["path"],"case":case,"result":result}));
    }
    if hash(&fs::read(&source)?)? != report["source_sha256"]
        || hash(&fs::read(context)?)? != report["context_sha256"]
    {
        return Err("isolation inputs changed".into());
    }
    write_json(
        &out.join("report.json"),
        &json!({"status":if sdk.is_some(){"candidate_checks_run"}else{"contexts_prepared"},"census":census.canonicalize()?,"original_module_contexts":module_contexts,"cases":cases,"scope":"Adapted contexts with original test functions. Type checking, translation correspondence, contract checks and runtime execution remain separate requirements."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires DOC_NATIVE_POSITIVE and DOC_NATIVE_NEGATIVE from actual Rustdoc function checks"]
    fn real_unregistered_function_assertion_accepts_true_and_rejects_false() {
        for (variable, status, summary) in [
            ("DOC_NATIVE_POSITIVE", "proved", json!([1, 0])),
            ("DOC_NATIVE_NEGATIVE", "not_proved", json!([0, 1])),
        ] {
            let run = std::path::PathBuf::from(std::env::var(variable).unwrap());
            let report: Value =
                serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
            assert_eq!(report["report_complete"], true);
            assert_eq!(report["compiler_totals"]["inventory_complete"], true);
            let candidates = report["source_test_routes"]["candidates"]
                .as_array()
                .unwrap();
            assert_eq!(candidates.len(), 2);
            let helper = candidates
                .iter()
                .find(|c| c["test"].as_str().unwrap().contains("::"))
                .unwrap();
            let source = &helper["result"]["result"];
            assert_eq!(helper["complete_property_coverage"], true);
            assert_eq!(source["check"]["verification"]["status"], status);
            assert_eq!(source["check"]["verification"]["summary"], summary);
            assert_eq!(
                source["check"]["annotation_correspondence"]["matches"],
                true
            );
            assert_eq!(source["entry_conditions_added"], 0);
            let census = Path::new(
                report["source_test_routes"]["attempts"]["results"][0]["census"]
                    .as_str()
                    .unwrap(),
            );
            let parsed = validate(census).unwrap();
            assert_eq!(parsed["selection"], "functions");
            assert!(
                parsed["test_candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["test_attributes"].as_array().unwrap().is_empty())
            );
            let case = Path::new(helper["result"]["case"].as_str().unwrap());
            let functions = crate::vir::functions(
                &fs::read_to_string(case.join("check-vir/crate.vir")).unwrap(),
            )
            .unwrap();
            let path = source["check"]["native_targets"][0]["path"]
                .as_str()
                .unwrap();
            let function = functions
                .iter()
                .find(|f| {
                    f.field("name")
                        .and_then(|n| n.field("path"))
                        .and_then(crate::vir::Node::atom)
                        == Some(path)
                })
                .unwrap();
            assert!(
                function
                    .field("require")
                    .and_then(crate::vir::Node::list)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    #[ignore = "requires ROOT_CFG_RUN and ROOT_CFG_CAPTURE from a real cfg-disabled root-test control"]
    fn real_disabled_root_test_cannot_become_an_empty_guarantee() {
        let run = std::path::PathBuf::from(std::env::var("ROOT_CFG_RUN").unwrap());
        let captured: Value = serde_json::from_slice(
            &fs::read(
                std::path::PathBuf::from(std::env::var("ROOT_CFG_CAPTURE").unwrap())
                    .join("report.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            captured["profiles"][0]["registry"][0]["tests"],
            json!(["test_align_down"])
        );
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        assert_eq!(
            report["original_module_contexts"]["<crate>"]["status"],
            "compiled"
        );
        let cases = report["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 2);
        let disabled = &cases.iter().find(|c| c["test"] == "test_align_up").unwrap()["result"];
        assert_eq!(
            disabled["original_test_presence"],
            "not_selected_by_compiler"
        );
        assert_eq!(disabled["check"]["verification"]["summary"], json!([23, 0]));
        assert_eq!(
            disabled["check"]["annotation_correspondence"]["status"],
            "target_unavailable"
        );
        assert_eq!(disabled["check"]["complete_property_coverage"], false);
        let active = &cases
            .iter()
            .find(|c| c["test"] == "test_align_down")
            .unwrap()["result"];
        assert_eq!(active["original_test_presence"], "present");
        assert_eq!(active["check"]["complete_property_coverage"], true);
    }

    #[test]
    #[ignore = "requires PRESERVED_TEST_RUN and fresh PRESERVED_TEST_CONTROLS from actual AlignExt checks"]
    fn real_original_module_preserves_test_and_rejects_changed_observation() {
        let run = std::path::PathBuf::from(std::env::var("PRESERVED_TEST_RUN").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        let census: Value = serde_json::from_slice(
            &fs::read(Path::new(report["census"].as_str().unwrap()).join("report.json")).unwrap(),
        )
        .unwrap();
        let original = fs::read(census["source"].as_str().unwrap()).unwrap();
        let modules = &report["original_module_contexts"];
        let module = modules
            .get("test")
            .or_else(|| modules.get("<crate>"))
            .unwrap();
        assert_eq!(module["status"], "compiled");
        let edits: Vec<Value> =
            serde_json::from_slice(&fs::read(module["edits"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(
            edits.len(),
            if modules.get("<crate>").is_some() {
                2
            } else {
                3
            }
        );
        let mut activated = original.clone();
        for edit in edits {
            let start = edit["source_bytes"][0].as_u64().unwrap() as usize;
            let end = edit["source_bytes"][1].as_u64().unwrap() as usize;
            activated[start..end].fill(b' ');
        }
        assert_eq!(
            activated,
            fs::read(module["source"].as_str().unwrap()).unwrap()
        );
        let cases = report["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 2);
        for (case, test) in cases
            .iter()
            .zip(census["test_candidates"].as_array().unwrap())
        {
            assert_eq!(case["test"], test["path"]);
            let result = &case["result"];
            let offset = test["source_bytes"][0].as_u64().unwrap() as usize;
            let mut expected = activated[..offset].to_vec();
            expected.extend_from_slice(b"#[vstd::prelude::verus_verify]\n");
            expected.extend_from_slice(&activated[offset..]);
            assert_eq!(
                fs::read(result["source"].as_str().unwrap()).unwrap(),
                expected
            );
            assert_eq!(result["check"]["complete_property_coverage"], true);
            assert_eq!(
                result["check"]["annotation_correspondence"]["explicit_assertions"],
                1
            );
            assert_eq!(result["check"]["verification"]["summary"], json!([24, 1]));
            assert_eq!(result["entry_conditions_added"], 0);
            assert_eq!(result["new_trusted_declarations"], 0);
        }
        let case = Path::new(cases[0]["case"].as_str().unwrap());
        let before: Value =
            serde_json::from_slice(&fs::read(case.join("runtime.json")).unwrap()).unwrap();
        let before =
            serde_json::from_value::<Vec<crate::ir::Record>>(before["records"].clone()).unwrap();
        let mut omitted = before.clone();
        let loan = omitted.iter().position(|r| r["kind"] == "loan").unwrap();
        omitted.remove(loan);
        assert!(!crate::ir::compare(&before, &omitted, None).unwrap());
        let controls = std::path::PathBuf::from(std::env::var("PRESERVED_TEST_CONTROLS").unwrap());
        fs::create_dir(&controls).unwrap();
        let mutant = controls.join("package");
        runner::copy_package(&case.join("runtime-package"), &mutant).unwrap();
        let source = mutant.join("src/lib.rs");
        let text = fs::read_to_string(&source).unwrap();
        let changed = text.replacen("let output_ns = [0usize", "let output_ns = [1usize", 1);
        assert_ne!(text, changed);
        fs::write(&source, changed).unwrap();
        let inv: Invocation =
            serde_json::from_slice(&fs::read(case.join("context.json")).unwrap()).unwrap();
        let entry = inv.args.iter().find(|a| a.ends_with(".rs")).unwrap();
        let args = runner::clean(
            &inv.args,
            &controls.join("output"),
            Some((Path::new(entry), &source)),
            &inv.cwd,
        )
        .unwrap();
        let records = runner::extract_context(
            &run.join("mir-probe"),
            &inv,
            &args,
            cases[0]["result"]["check"]["annotation_correspondence"]["target"]
                .as_str()
                .unwrap(),
            &controls,
            "changed",
            true,
        )
        .unwrap();
        assert!(!crate::ir::compare(&before, &records, None).unwrap());
        write_json(&controls.join("result.json"), &json!({"original_module_preserved":true,"changed_expected_value_rejected":true,"omitted_loan_rejected":true,"mutant_typechecked":true})).unwrap();
    }
    #[test]
    #[ignore = "requires ISOLATED_MIR_CASE and fresh ISOLATED_MIR_OUT from a checked test context"]
    fn actual_isolated_annotation_correspondence() {
        let case = std::path::PathBuf::from(std::env::var("ISOLATED_MIR_CASE").unwrap());
        let out = std::path::PathBuf::from(std::env::var("ISOLATED_MIR_OUT").unwrap());
        fs::create_dir(&out).unwrap();
        let prep: Value =
            serde_json::from_slice(&fs::read(case.join("preparation.json")).unwrap()).unwrap();
        let suite: Value =
            serde_json::from_slice(&fs::read(case.parent().unwrap().join("report.json")).unwrap())
                .unwrap();
        let census: Value = serde_json::from_slice(
            &fs::read(Path::new(suite["census"].as_str().unwrap()).join("report.json")).unwrap(),
        )
        .unwrap();
        let inv: Invocation =
            serde_json::from_slice(&fs::read(census["context"].as_str().unwrap()).unwrap())
                .unwrap();
        let root = inv
            .args
            .iter()
            .find(|s| s.ends_with(".rs"))
            .map(|s| inv.cwd.join(s).canonicalize().unwrap())
            .unwrap();
        let package = Path::new(inv.env.get("CARGO_MANIFEST_DIR").unwrap())
            .canonicalize()
            .unwrap();
        let source = Path::new(prep["source"].as_str().unwrap());
        let original_package = case.join("package");
        let relative = source.strip_prefix(&original_package).unwrap();
        let target = prep["check"]["native_targets"][0]["path"].as_str().unwrap();
        let crate_name = inv
            .args
            .windows(2)
            .find(|a| a[0] == "--crate-name")
            .unwrap()[1]
            .as_str();
        let target = target.strip_prefix(&format!("{crate_name}::")).unwrap();
        let probe = runner::build_probe(&inv, &out).unwrap().unwrap();
        let mut records = Vec::new();
        for name in ["before", "after"] {
            let snapshot = out.join(name);
            runner::copy_package(&original_package, &snapshot).unwrap();
            if name == "before" {
                let text = fs::read_to_string(snapshot.join(relative)).unwrap();
                let addition = prep["addition"].as_str().unwrap();
                assert!(text.ends_with(addition));
                let prefix = &text[..text.len() - addition.len()];
                let changed = addition.replacen("#[vstd::prelude::verus_verify]\n", "", 1);
                fs::write(snapshot.join(relative), format!("{prefix}{changed}")).unwrap();
            }
            let args = runner::clean(
                &inv.args,
                &out.join(format!("{name}-output")),
                Some((&root, &snapshot.join(root.strip_prefix(&package).unwrap()))),
                &inv.cwd,
            )
            .unwrap();
            records.push(
                runner::extract_context(&probe, &inv, &args, target, &out, name, true).unwrap(),
            );
        }
        let same = crate::ir::compare(&records[0], &records[1], None).unwrap();
        write_json(&out.join("comparison.json"),&json!({"target":target,"matches":same,"scope":"Same isolated context before and after the proof marker; original module adaptation remains separate."})).unwrap();
        assert!(same);
        let mut missing_loan = records[1].clone();
        let loan = missing_loan
            .iter()
            .position(|r| r["kind"] == "loan")
            .unwrap();
        missing_loan.remove(loan);
        assert!(!matches!(
            crate::ir::compare(&records[0], &missing_loan, None),
            Ok(true)
        ));
        let mutant = out.join("mutant");
        runner::copy_package(&out.join("after"), &mutant).unwrap();
        let text = fs::read_to_string(mutant.join(relative)).unwrap();
        let addition = prep["addition"].as_str().unwrap();
        assert!(text.ends_with(addition));
        let changed =
            addition.replacen("assert!(waker.wake_up());", "assert!(!waker.wake_up());", 1);
        assert_ne!(changed, addition);
        fs::write(
            mutant.join(relative),
            format!("{}{changed}", &text[..text.len() - addition.len()]),
        )
        .unwrap();
        let args = runner::clean(
            &inv.args,
            &out.join("mutant-output"),
            Some((&root, &mutant.join(root.strip_prefix(&package).unwrap()))),
            &inv.cwd,
        )
        .unwrap();
        let altered =
            runner::extract_context(&probe, &inv, &args, target, &out, "mutant", true).unwrap();
        assert!(!matches!(
            crate::ir::compare(&records[0], &altered, None),
            Ok(true)
        ));
        write_json(&out.join("controls.json"),&json!({"omitted_loan_rejected":true,"source_assertion_inversion_rejected":true,"mutant_compiler_exit_code":0})).unwrap();
    }

    #[test]
    #[ignore = "requires TEST_ISOLATION_RUN, TEST_ISOLATION_CONTROLS and WAKER_RETURN_CONTROL from actual runs"]
    fn real_isolation_retains_tests_and_rejects_omitted_candidates() {
        let run = std::path::PathBuf::from(std::env::var("TEST_ISOLATION_RUN").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        let census = Path::new(report["census"].as_str().unwrap());
        let parsed: Value =
            serde_json::from_slice(&fs::read(census.join("report.json")).unwrap()).unwrap();
        let original = fs::read(parsed["source"].as_str().unwrap()).unwrap();
        let cases = report["cases"].as_array().unwrap();
        assert_eq!(
            cases.len(),
            parsed["test_candidates"].as_array().unwrap().len()
        );
        assert_eq!(cases.len(), 6);
        let baseline_inv: Invocation =
            serde_json::from_slice(&fs::read(parsed["context"].as_str().unwrap()).unwrap())
                .unwrap();
        let mut matched_conversions = 0;
        let mut checks = BTreeMap::<String, usize>::new();
        for (case, test) in cases
            .iter()
            .zip(parsed["test_candidates"].as_array().unwrap())
        {
            assert_eq!(case["test"], test["path"]);
            let result = &case["result"];
            let candidate = fs::read(result["source"].as_str().unwrap()).unwrap();
            assert!(candidate.starts_with(&original));
            assert_eq!(
                &candidate[original.len()..],
                result["addition"].as_str().unwrap().as_bytes()
            );
            assert_eq!(hash(&candidate).unwrap(), result["source_sha256"]);
            assert!(
                result["addition"]
                    .as_str()
                    .unwrap()
                    .contains(source_slice(&original, test).unwrap())
            );
            assert_eq!(result["correspondence"], "pending");
            assert_eq!(result["check"]["complete_property_coverage"], false);
            if let Some(conversion) = result["check"].get("annotation_correspondence") {
                let projected: Invocation =
                    serde_json::from_slice(&fs::read(result["context"].as_str().unwrap()).unwrap())
                        .unwrap();
                assert_eq!(projected.args.len(), baseline_inv.args.len());
                for (actual, original) in projected.args.iter().zip(&baseline_inv.args) {
                    if !original.ends_with(".rs") {
                        assert_eq!(actual, original);
                    }
                }
                if conversion["status"] == "matched" {
                    assert_eq!(conversion["matches"], true);
                    matched_conversions += 1;
                }
                assert_eq!(
                    result["check"]["original_context_correspondence"],
                    "pending"
                );
            }

            *checks
                .entry(
                    result["check"]["verification"]["status"]
                        .as_str()
                        .unwrap()
                        .into(),
                )
                .or_default() += 1;
        }
        if cases.iter().any(|c| {
            c["result"]["check"]
                .get("annotation_correspondence")
                .is_some()
        }) {
            assert_eq!(matched_conversions, 2);
        }
        assert_eq!(
            checks,
            BTreeMap::from([
                ("not_analyzed".into(), 4),
                ("not_proved".into(), 1),
                ("unsupported".into(), 1)
            ])
        );
        if let Some(modules) = report.get("original_module_contexts") {
            assert_eq!(modules.as_object().unwrap().len(), 1);
            assert_eq!(modules["test"]["status"], "compile_failed");
            let diagnostics = modules["test"]["diagnostics"].as_array().unwrap();
            assert_eq!(
                diagnostics.iter().filter(|d| !d["code"].is_null()).count(),
                18
            );
            let activated = fs::read(run.join("module-0/package/src/sync/wait.rs")).unwrap();
            let edits: Vec<Value> =
                serde_json::from_slice(&fs::read(run.join("module-0/edits.json")).unwrap())
                    .unwrap();
            assert_eq!(edits.len(), 7);
            let mut expected = original.clone();
            for edit in edits {
                let start = edit["source_bytes"][0].as_u64().unwrap() as usize;
                let end = edit["source_bytes"][1].as_u64().unwrap() as usize;
                expected[start..end].fill(b' ');
            }
            assert_eq!(activated, expected);
            for test in parsed["test_candidates"].as_array().unwrap() {
                assert_eq!(
                    source_slice(&activated, test).unwrap(),
                    source_slice(&original, test).unwrap()
                );
            }
        }
        let controls = std::path::PathBuf::from(std::env::var("TEST_ISOLATION_CONTROLS").unwrap());
        fs::create_dir(&controls).unwrap();
        fs::copy(census.join("parse.stderr"), controls.join("parse.stderr")).unwrap();
        let mut omitted = parsed.clone();
        omitted["test_candidates"].as_array_mut().unwrap().pop();
        write_json(&controls.join("report.json"), &omitted).unwrap();
        assert!(
            isolate(&controls, &controls.join("rejected"), None)
                .unwrap_err()
                .to_string()
                .contains("raw records")
        );
        assert!(!controls.join("rejected").exists());
        let control = std::path::PathBuf::from(std::env::var("WAKER_RETURN_CONTROL").unwrap());
        let baseline_input: Value = serde_json::from_slice(
            &fs::read(
                Path::new(parsed["context"].as_str().unwrap())
                    .parent()
                    .unwrap()
                    .join("verification-input.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let old = crate::vir::functions(
            &fs::read_to_string(baseline_input["vir"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let new = crate::vir::functions(
            &fs::read_to_string(control.join("check-vir/crate.vir")).unwrap(),
        )
        .unwrap();
        let contract = |functions: &[crate::vir::Node]| {
            let names = functions
                .iter()
                .filter_map(|f| {
                    f.field("name")
                        .and_then(|n| n.field("path"))
                        .and_then(crate::vir::Node::atom)
                })
                .filter(|p| p.starts_with("ostd::sync::wait::") && p.ends_with("::wake_up"))
                .collect::<Vec<_>>();
            assert_eq!(names.len(), 1);
            crate::ordinary::call_contract(functions, names[0])
        };
        let a = contract(&old);
        let b = contract(&new);
        for field in ["requires", "ensures"] {
            assert_eq!(a[field], json!([]));
            assert_eq!(b[field], json!([]));
        }
        let checked: Value =
            serde_json::from_slice(&fs::read(control.join("result.json")).unwrap()).unwrap();
        assert_eq!(checked["exit_code"], 0);
        assert_eq!(checked["summaries"], json!([["19", "0"]]));
        write_json(
            &controls.join("contract-comparison.json"),
            &json!({"original":a,"return_false":b,"library_control":checked}),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires TEST_CENSUS_CASES from real AST-driver runs and controls"]
    fn real_ast_candidates_preserve_bodies_without_claiming_registration() {
        let cases: Vec<Value> =
            serde_json::from_slice(&fs::read(std::env::var("TEST_CENSUS_CASES").unwrap()).unwrap())
                .unwrap();
        for case in cases {
            let report: Value =
                serde_json::from_slice(&fs::read(case["report"].as_str().unwrap()).unwrap())
                    .unwrap();
            let source = Path::new(report["source"].as_str().unwrap());
            assert_eq!(
                report["source_sha256"],
                hash(&fs::read(source).unwrap()).unwrap()
            );
            assert_eq!(report["parse_complete"], case["expected_parse_complete"]);
            assert_eq!(report["verification_executed"], false);
            assert_eq!(report["runtime_executed"], false);
            if !case["expected_tests"].is_null() {
                let names: Vec<_> = report["test_candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t["path"].clone())
                    .collect();
                assert_eq!(json!(names), case["expected_tests"]);
            }
            if case["name"] == "visible" {
                assert_eq!(
                    report["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|i| i["kind"] == "macro_opaque")
                        .count(),
                    2
                );
            }
            for item in report["items"].as_array().unwrap() {
                if let Some(body) = item.get("body") {
                    let source = fs::read(source).unwrap();
                    let start = body["source_bytes"][0].as_u64().unwrap() as usize;
                    let end = body["source_bytes"][1].as_u64().unwrap() as usize;
                    assert_eq!(
                        &source[start..end],
                        body["source_text"].as_str().unwrap().as_bytes()
                    );
                    assert_eq!(
                        body["source_text"].as_str().unwrap().replace("\r\n", "\n"),
                        body["text"].as_str().unwrap()
                    );
                }
            }
        }
    }
}
