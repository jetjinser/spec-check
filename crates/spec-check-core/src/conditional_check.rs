//! Validate generated body replacements against the captured source contracts.
use crate::{
    Result, capture::Invocation, client, conditional, demand_check, hash, option_alternative,
    runner, vir, write_json,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    mutations: PathBuf,
    vir: PathBuf,
    context: PathBuf,
    capture_inputs: PathBuf,
}
fn path(f: &vir::Node) -> Option<&str> {
    f.field("name")?.field("path")?.atom()
}
fn target<'a>(fs: &'a [vir::Node], name: &str) -> Result<&'a vir::Node> {
    let fs = fs
        .iter()
        .filter(|f| path(f) == Some(name))
        .collect::<Vec<_>>();
    match fs.as_slice() {
        [f] => Ok(f),
        _ => Err("function identity missing or ambiguous".into()),
    }
}
fn proved_queries(proof: &Value) -> bool {
    proof["status"] == "proved"
        && proof["exit_code"] == 0
        && proof["summary"]
            .as_array()
            .is_some_and(|n| n.len() == 2 && n[0].as_u64().is_some_and(|v| v > 0) && n[1] == 0)
}
fn compact(work: &Path, name: &str) -> Result<()> {
    for prefix in ["baseline", "alternative", "false-control"] {
        let file = work.join(format!("{prefix}-vir/crate.vir"));
        if !file.is_file() {
            continue;
        }
        let text = fs::read_to_string(&file)?;
        let slice = vir::function_text(&text, name)?;
        write_json(
            &file.with_extension("full-log.json"),
            &json!({"full_sha256":hash(text.as_bytes())?,"full_bytes":text.len(),"retained_function":name,"retained_sha256":hash(slice.as_bytes())?}),
        )?;
        fs::write(file, slice)?;
    }
    let cache = work.join("output");
    if cache.is_dir() {
        let mut files = BTreeMap::new();
        runner::source_hashes(&cache, &mut files)?;
        write_json(&work.join("compiler-cache.json"), &files)?;
        fs::remove_dir_all(cache)?;
    }
    Ok(())
}
pub fn run(config: &Path, sdk: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("conditional-check output already exists".into());
    }
    let cfg: Config = serde_json::from_slice(&fs::read(config)?)?;
    let inv: Invocation = serde_json::from_slice(&fs::read(&cfg.context)?)?;
    client::full_verification_context(&inv)?;
    let captured: BTreeMap<PathBuf, String> =
        serde_json::from_slice(&fs::read(&cfg.capture_inputs)?)?;
    let package = Path::new(
        inv.env
            .get("CARGO_MANIFEST_DIR")
            .ok_or("missing package root")?,
    )
    .canonicalize()?;
    let validate = || -> Result<()> {
        let mut count = 0;
        for (p, h) in &captured {
            if p.starts_with(&package) {
                count += 1;
                if hash(&fs::read(p)?)? != *h {
                    return Err(format!("captured source changed: {}", p.display()).into());
                }
            }
        }
        if count == 0 {
            return Err("capture contains no package sources".into());
        }
        for tool in ["verus", "rust_verify", "cargo-verus"] {
            let h = hash(&fs::read(sdk.join(tool))?)?;
            if !captured
                .iter()
                .any(|(p, v)| p.file_name().is_some_and(|n| n == tool) && *v == h)
            {
                return Err("SDK differs from capture".into());
            }
        }
        Ok(())
    };
    validate()?;
    let input: Value = serde_json::from_slice(&fs::read(cfg.mutations.join("report.json"))?)?;
    let original = fs::read_to_string(&cfg.vir)?;
    if hash(original.as_bytes())? != input["vir_sha256"] {
        return Err("mutation VIR changed".into());
    }
    let functions = vir::functions(&original)?;
    let datatypes = vir::datatypes(&original)?;
    let source_root =
        Path::new(input["source_root"].as_str().ok_or("missing source root")?).canonicalize()?;
    let crate_root = inv
        .args
        .iter()
        .map(|a| inv.cwd.join(a))
        .find(|p| p.extension().is_some_and(|e| e == "rs") && p.is_file())
        .ok_or("crate root unavailable")?
        .canonicalize()?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    write_json(&out.join("registration.json"), &input)?;
    write_json(
        &out.join("inputs.json"),
        &json!({"config":config,"config_sha256":hash(&fs::read(config)?)?,
        "context_sha256":hash(&fs::read(&cfg.context)?)?,"capture_inputs_sha256":hash(&fs::read(&cfg.capture_inputs)?)?,
        "tool_sha256":hash(&fs::read(std::env::current_exe()?)?)?}),
    )?;
    let mut results = Vec::new();
    for row in input["results"]
        .as_array()
        .ok_or("missing registered functions")?
    {
        let name = row["function"].as_str().ok_or("missing function")?;
        if row["result"]["status"] != "generated_unchecked" {
            results.push(json!({"function":name,"result":row["result"],"runtime":"not_run"}));
            continue;
        }
        for mutation in row["result"]["mutations"]
            .as_array()
            .ok_or("missing mutations")?
        {
            let work = out.join(format!("case-{}", results.len()));
            fs::create_dir(&work)?;
            let attempt = || -> Result<Value> {
                validate()?;
                let f = target(&functions, name)?;
                let source = source_root
                    .join(
                        mutation["source_relative"]
                            .as_str()
                            .ok_or("missing source path")?,
                    )
                    .canonicalize()?;
                let relative = source.strip_prefix(&package)?;
                let code = fs::read_to_string(&source)?;
                if hash(code.as_bytes())? != mutation["source_sha256"] {
                    return Err("mutation source changed".into());
                }
                let start = mutation["body_edit"][0].as_u64().ok_or("missing edit")? as usize;
                let end = mutation["body_edit"][1].as_u64().ok_or("missing edit")? as usize;
                let expr = mutation["expression"]
                    .as_str()
                    .ok_or("missing replacement")?;
                if !conditional::alternatives(f, &datatypes)?
                    .iter()
                    .any(|candidate| candidate == expr)
                {
                    return Err("expression does not match the typed mutation rules".into());
                }
                let proposed =
                    fs::read_to_string(mutation["artifact"].as_str().ok_or("missing artifact")?)?;
                if start > end
                    || !code.is_char_boundary(start)
                    || !code.is_char_boundary(end)
                    || !code
                        .get(start..end)
                        .is_some_and(|body| body.starts_with('{') && body.ends_with('}'))
                {
                    return Err("invalid function-body edit range".into());
                }
                let mut expected = code.clone();
                expected.replace_range(start..end, &format!("{{ {expr} }}"));
                if proposed != expected {
                    return Err("replacement changed source outside selected body".into());
                }
                let snapshot = work.join("package");
                runner::copy_package(&package, &snapshot)?;
                let mut args = runner::clean(
                    &inv.args,
                    &work.join("output"),
                    Some((
                        &crate_root,
                        &snapshot.join(crate_root.strip_prefix(&package)?),
                    )),
                    &inv.cwd,
                )?;
                let module = f
                    .field("owning_module")
                    .and_then(vir::Node::atom)
                    .ok_or("missing owning module")?
                    .trim_end_matches("::");
                let module = module.split("::").skip(1).collect::<Vec<_>>().join("::");
                if module.is_empty() {
                    args.push("--verify-root".into());
                } else {
                    args.extend(["--verify-only-module".into(), module.clone()]);
                }
                let method = name.rsplit("::").next().ok_or("missing method")?;
                args.extend([
                    "--verify-function".into(),
                    demand_check::verifier_filter(f, &module, method),
                ]);
                let baseline = runner::prove(sdk, &inv, &args, &work, "baseline")?;
                let baseline_native = work.join("baseline-vir/crate.vir");
                if !baseline_native.is_file() {
                    return Ok(
                        json!({"status":"not_analyzed","stage":"baseline","baseline":baseline,"runtime":"not_run"}),
                    );
                }
                let baseline_text = fs::read_to_string(baseline_native)?;
                let baseline_fs = vir::functions(vir::function_text(&baseline_text, name)?)?;
                let baseline_f = target(&baseline_fs, name)?;
                if option_alternative::contract(f)? != option_alternative::contract(baseline_f)? {
                    return Err("current source contract differs from input VIR".into());
                }
                let baseline_target =
                    vir::checked_target_in(&baseline_text, name, &snapshot, &inv.cwd)?;
                fs::write(snapshot.join(relative), &proposed)?;
                let proof = runner::prove(sdk, &inv, &args, &work, "alternative")?;
                let native = work.join("alternative-vir/crate.vir");
                if !native.exists() {
                    return Ok(
                        json!({"status":"not_analyzed","baseline":baseline,"verification":proof}),
                    );
                }
                let text = fs::read_to_string(native)?;
                let fs = vir::functions(vir::function_text(&text, name)?)?;
                let after = target(&fs, name)?;
                let unchanged =
                    option_alternative::contract(f)? == option_alternative::contract(after)?;
                let checked = vir::checked_target_in(&text, name, &snapshot, &inv.cwd)?;
                let preliminary = unchanged
                    && checked.is_some()
                    && proof["status"] == "proved"
                    && proof["summary"] == json!([1, 0])
                    // Verus counts queries; a selected function can generate several.
                    && baseline_target.is_some()
                    && proved_queries(&baseline);
                let mut negative = Value::Null;
                let mut false_count = 0;
                if preliminary {
                    let mut control = code.clone();
                    control.replace_range(
                        start..end,
                        &format!("{{ ::vstd::prelude::proof! {{ assert(false); }} {expr} }}"),
                    );
                    fs::write(snapshot.join(relative), control)?;
                    negative = runner::prove(sdk, &inv, &args, &work, "false-control")?;
                    let control_text =
                        fs::read_to_string(work.join("false-control-vir/crate.vir"))?;
                    let control_fs = vir::functions(vir::function_text(&control_text, name)?)?;
                    if option_alternative::contract(f)?
                        != option_alternative::contract(target(&control_fs, name)?)?
                    {
                        return Err("false control altered contract".into());
                    }
                    false_count =
                        demand_check::false_count(vir::function_text(&control_text, name)?, name)?;
                    fs::write(snapshot.join(relative), &proposed)?;
                }
                let accepted = preliminary
                    && negative["status"] == "not_proved"
                    && negative["summary"] == json!([0, 1])
                    && false_count == 1;
                Ok(
                    json!({"status":if accepted {"contract_accepts_alternative"} else {"alternative_not_established"},"baseline":baseline,"baseline_target":baseline_target,"verification":proof,"contract_unchanged":unchanged,"target":checked,"false_control":negative,"injected_false_assertions":false_count,"runtime":"not_run"}),
                )
            };
            let mut result = match attempt() {
                Ok(v) => v,
                Err(e) => {
                    json!({"status":"unsupported","reason":e.to_string(),"runtime":"not_run"})
                }
            };
            if let Err(error) = compact(&work, name) {
                result["retention_error"] = json!(error.to_string());
            }
            write_json(&work.join("report.json"), &result)?;
            results
                .push(json!({"function":name,"mutation":mutation,"result":result,"evidence":work}));
            write_json(
                &out.join("report.json"),
                &json!({"results":results,"complete":false,"confirmed_defects":0}),
            )?;
        }
    }
    validate()?;
    write_json(
        &out.join("report.json"),
        &json!({"results":results,"complete":true,"confirmed_defects":0,"interpretation":"Native contract acceptance with entry false-assertion sensitivity; existing tests must still reject the alternative before a test/spec gap is established."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_queries_are_not_a_function_count() {
        for n in [1, 3] {
            assert!(proved_queries(
                &json!({"status":"proved","exit_code":0,"summary":[n,0]})
            ));
        }
        for proof in [
            json!({"status":"proved","exit_code":0,"summary":[0,0]}),
            json!({"status":"not_proved","exit_code":1,"summary":[3,1]}),
            json!({"status":"tool_failure","exit_code":1,"summary":[3,0]}),
        ] {
            assert!(!proved_queries(&proof));
        }
    }
    #[test]
    #[ignore = "requires CONDITIONAL_NATIVE_CASE from an actual completed three-way native run"]
    fn recorded_acceptance_retains_contract_and_rejects_false() {
        let dir = PathBuf::from(std::env::var("CONDITIONAL_NATIVE_CASE").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
        let name = report["target"]["path"].as_str().unwrap();
        let baseline = runner::recorded_proof(&dir, "baseline").unwrap();
        let alternative = runner::recorded_proof(&dir, "alternative").unwrap();
        let control = runner::recorded_proof(&dir, "false-control").unwrap();
        assert!(proved_queries(&baseline));
        assert_eq!(alternative["status"], "proved");
        assert_eq!(alternative["summary"], json!([1, 0]));
        assert_eq!(control["status"], "not_proved");
        assert_eq!(control["summary"], json!([0, 1]));
        let original =
            vir::functions(&fs::read_to_string(dir.join("baseline-vir/crate.vir")).unwrap())
                .unwrap();
        for prefix in ["alternative", "false-control"] {
            let text = fs::read_to_string(dir.join(format!("{prefix}-vir/crate.vir"))).unwrap();
            let functions = vir::functions(&text).unwrap();
            assert_eq!(
                option_alternative::contract(target(&original, name).unwrap()).unwrap(),
                option_alternative::contract(target(&functions, name).unwrap()).unwrap()
            );
            assert_eq!(
                demand_check::false_count(&text, name).unwrap(),
                usize::from(prefix == "false-control")
            );
        }
    }
}
