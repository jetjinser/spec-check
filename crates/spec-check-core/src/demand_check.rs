//! Replay source demands in their original module and test local inconsistency.
use crate::{
    Result, attribution as a, capture::Invocation, client, hash, ir, runner, vir, write_json,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    context: PathBuf,
    census: PathBuf,
    capture_inputs: PathBuf,
    sources: Vec<PathBuf>,
}

fn single_target_attempt(result: &Value) -> bool {
    result["summary"] == json!([1, 0]) || result["summary"] == json!([0, 1])
}

fn false_assertions(n: &vir::Node) -> usize {
    let own = usize::from(
        a::head(n) == Some("AssertAssume")
            && n.field("is_assume").and_then(vir::Node::atom) == Some("false")
            && n.field("expr").is_some_and(|e| {
                a::head(e) == Some("Const")
                    && a::items(e).get(2).is_some_and(|c| {
                        a::atom(c, 1) == Some("Bool") && a::atom(c, 2) == Some("false")
                    })
            }),
    );
    own + n
        .unwrap()
        .list()
        .unwrap_or_default()
        .iter()
        .map(false_assertions)
        .sum::<usize>()
}

pub(crate) fn false_count(text: &str, name: &str) -> Result<usize> {
    let functions = vir::functions(text)?;
    let selected = functions
        .iter()
        .filter(|f| {
            f.field("name")
                .and_then(|n| n.field("path"))
                .and_then(vir::Node::atom)
                == Some(name)
        })
        .collect::<Vec<_>>();
    let [f] = selected.as_slice() else {
        return Err("probe caller identity ambiguous".into());
    };
    Ok(f.field("body").map_or(0, false_assertions))
}

pub(crate) fn verifier_filter(f: &vir::Node, module: &str, name: &str) -> String {
    let self_type = f
        .field("params")
        .and_then(vir::Node::list)
        .and_then(|ps| {
            ps.iter().find(|p| {
                p.field("name")
                    .is_some_and(|n| a::atom(n, 1) == Some("self"))
            })
        })
        .and_then(|p| p.field("typ"));
    let Some(mut typ) = self_type else {
        return name.to_owned();
    };
    loop {
        let reference = a::atom(typ, 1) == Some("MutRef")
            || (a::atom(typ, 1) == Some("Decorate")
                && a::items(typ)
                    .get(2)
                    .is_some_and(|d| a::atom(d, 1) == Some("Ref")));
        if !reference {
            break;
        }
        let Some(inner) = a::items(typ).last() else {
            break;
        };
        typ = inner;
    }
    let path = if a::atom(typ, 1) == Some("Datatype") {
        a::items(typ)
            .get(2)
            .and_then(|d| a::items(d).get(2))
            .and_then(vir::Node::atom)
    } else {
        None
    };
    let Some(path) = path else {
        return name.to_owned();
    };
    let full = format!("{path}::{name}");
    let krate = path.split("::").next().unwrap_or("");
    let prefix = if module.is_empty() {
        format!("{krate}::")
    } else {
        format!("{krate}::{module}::")
    };
    full.strip_prefix(&prefix).unwrap_or(&full).to_owned()
}

pub fn run(config: &Path, sdk: &Path, out: &Path) -> Result<()> {
    let cfg: Config = serde_json::from_slice(&fs::read(config)?)?;
    let inv: Invocation = serde_json::from_slice(&fs::read(&cfg.context)?)?;
    client::full_verification_context(&inv)?;
    let census: Value = serde_json::from_slice(&fs::read(&cfg.census)?)?;
    let original_vir = Path::new(census["vir"].as_str().ok_or("missing VIR input")?);
    if hash(&fs::read(original_vir)?)? != census["vir_sha256"] {
        return Err("census VIR changed".into());
    }
    let native_functions = vir::functions(&fs::read_to_string(original_vir)?)?;
    let package =
        Path::new(inv.env.get("CARGO_MANIFEST_DIR").ok_or("missing package")?).canonicalize()?;
    let captured: std::collections::BTreeMap<PathBuf, String> =
        serde_json::from_slice(&fs::read(&cfg.capture_inputs)?)?;
    let guarded = captured
        .iter()
        .filter(|(p, _)| p.starts_with(&package))
        .collect::<Vec<_>>();
    if guarded.is_empty() {
        return Err("capture has no package inputs".into());
    }
    let validate = || -> Result<()> {
        for (file, digest) in &guarded {
            if hash(&fs::read(file)?)? != **digest {
                return Err(format!("captured source changed: {}", file.display()).into());
            }
        }
        for tool in ["verus", "rust_verify", "cargo-verus"] {
            let digest = hash(&fs::read(sdk.join(tool))?)?;
            if !captured
                .iter()
                .any(|(p, h)| p.file_name().is_some_and(|n| n == tool) && h == &digest)
            {
                return Err("SDK differs from capture".into());
            }
        }
        Ok(())
    };
    validate()?;
    let sources = cfg
        .sources
        .iter()
        .map(|s| {
            if s.is_absolute()
                || s.components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("sources must be package-relative".into());
            }
            let p = package.join(s).canonicalize()?;
            if !p.starts_with(&package) {
                return Err("source escapes package".into());
            }
            Ok(p)
        })
        .collect::<Result<Vec<_>>>()?;
    let crate_root = inv
        .args
        .iter()
        .map(|a| inv.cwd.join(a))
        .find(|p| p.extension().is_some_and(|s| s == "rs") && p.is_file())
        .ok_or("crate root unavailable")?
        .canonicalize()?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let mut results = Vec::new();
    let mut registrations = Vec::new();
    for caller in census["callers"].as_array().ok_or("missing callers")? {
        let Some(span) = caller["span"].as_str() else {
            continue;
        };
        let (source, _) = ir::span(span, &inv.cwd)?;
        if !sources.contains(&source.canonicalize()?) {
            continue;
        }
        registrations.push(json!({"caller":caller["function"],"span":caller["span"],"body_available":caller["body_available"],"explicit_demands":caller["demands"].as_array().ok_or("missing demands")?.iter().filter(|d|d["status"]=="requires_translation").count(),"classification":"existing_native_caller_replay"}));
        for demand in caller["demands"].as_array().ok_or("missing demands")? {
            if demand["status"] != "requires_translation" {
                continue;
            }
            let case = out.join(format!("case-{}", results.len()));
            fs::create_dir(&case)?;
            let work = || -> Result<Value> {
                let call_span = demand["span"].as_str().ok_or("missing demand span")?;
                if !call_span.ends_with("(#0)") {
                    return Err("macro-expanded target requires source correspondence".into());
                }
                let (file, coords) = ir::span(call_span, &inv.cwd)?;
                let file = file.canonicalize()?;
                if file != source.canonicalize()? {
                    return Err("demand outside caller source".into());
                }
                let code = fs::read_to_string(&file)?;
                let (a, b) = ir::offsets(&code, coords)?;
                if a == b {
                    return Err("empty demand span".into());
                }
                let function = caller["function"]
                    .as_str()
                    .ok_or("missing caller identity")?;
                let (module, name) = function.rsplit_once("::").ok_or("invalid caller path")?;
                let module = module
                    .split("::")
                    .skip(1)
                    .take_while(|s| !s.starts_with("impl&"))
                    .collect::<Vec<_>>()
                    .join("::");
                let snapshot = case.join("package");
                runner::copy_package(&package, &snapshot)?;
                let copied_root = snapshot.join(crate_root.strip_prefix(&package)?);
                let copied_file = snapshot.join(file.strip_prefix(&package)?);
                let mut args = runner::clean(
                    &inv.args,
                    &case.join("output"),
                    Some((&crate_root, &copied_root)),
                    &inv.cwd,
                )?;
                if module.is_empty() {
                    args.push("--verify-root".into());
                } else {
                    args.extend(["--verify-only-module".into(), module.clone()]);
                }
                let definition = native_functions
                    .iter()
                    .find(|f| {
                        f.field("name")
                            .and_then(|n| n.field("path"))
                            .and_then(vir::Node::atom)
                            == Some(function)
                    })
                    .ok_or("caller not in original VIR")?;
                let filter = verifier_filter(definition, &module, name);
                args.extend(["--verify-function".into(), filter]);
                let baseline = runner::prove(sdk, &inv, &args, &case, "baseline")?;
                let base_vir = case.join("baseline-vir/crate.vir");
                if !base_vir.is_file() {
                    return Ok(json!({"status":"not_analyzed","baseline":baseline}));
                }
                let base_false_count = false_count(&fs::read_to_string(&base_vir)?, function)?;
                let target = vir::checked_target_in(
                    &fs::read_to_string(&base_vir)?,
                    function,
                    &snapshot,
                    &inv.cwd,
                )?
                .ok_or("baseline target not checked")?;
                // The wrapper stays exactly at the expression's original evaluation point.
                let ghost = demand["context"]
                    .as_array()
                    .is_some_and(|xs| xs.iter().any(|c| c["kind"] == "Ghost"));
                let assertion = if ghost {
                    "assert(false);"
                } else {
                    "vstd::prelude::proof! { assert(false); }"
                };
                let replacement = format!("{{ {assertion} {} }}", &code[a..b]);
                let mut changed = code.clone();
                changed.replace_range(a..b, &replacement);
                fs::write(&copied_file, &changed)?;
                let probe = runner::prove(sdk, &inv, &args, &case, "consistency")?;
                let probe_vir = case.join("consistency-vir/crate.vir");
                let probe_target = if probe_vir.is_file() {
                    vir::checked_target_in(
                        &fs::read_to_string(&probe_vir)?,
                        function,
                        &snapshot,
                        &inv.cwd,
                    )?
                } else {
                    None
                };
                let probe_present = probe_vir.is_file()
                    && false_count(&fs::read_to_string(&probe_vir)?, function)?
                        == base_false_count + 1
                    && probe_target.is_some();
                let baseline_single_target =
                    baseline["summary"] == json!([1, 0]) || baseline["summary"] == json!([0, 1]);
                if fs::read_to_string(&file)? != code {
                    return Err("original source changed".into());
                }
                Ok(
                    json!({"status":if probe_present && baseline_single_target && single_target_attempt(&probe) {"native_demand_checked"}else{"probe_not_established"},"probe_present":probe_present,"baseline_single_target":baseline_single_target,"baseline":baseline,"baseline_target":target,"consistency":probe,"consistency_target":probe_target,"source":file,"source_sha256":hash(code.as_bytes())?,"mutation":{"start":a,"end":b,"original":&code[a..b],"replacement":replacement,"output_sha256":hash(changed.as_bytes())?},"interpretation":"Baseline checks the original caller, including existing entry conditions. A proved false probe indicates an inconsistent or unreachable target context; an unproved probe establishes neither satisfiability nor reachability. No new engineering property is introduced."}),
                )
            };
            let result = match work() {
                Ok(r) => r,
                Err(e) => json!({"status":"unsupported","reason":e.to_string()}),
            };
            write_json(&case.join("report.json"), &result)?;
            results.push(json!({"caller":caller["function"],"demand":demand,"result":result,"evidence":case}));
        }
    }
    validate()?;
    write_json(
        &out.join("report.json"),
        &json!({"results":results,"registered_callers":registrations,"config":cfg.sources,"census_sha256":hash(&fs::read(&cfg.census)?)?,"context_sha256":hash(&fs::read(&cfg.context)?)?,"capture_inputs_sha256":hash(&fs::read(&cfg.capture_inputs)?)?,"guarded_package_inputs":guarded.len(),"per_client_hints":0,"new_confirmed_defects":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compilation_failure_cannot_count_as_a_checked_probe() {
        assert!(!single_target_attempt(
            &json!({"status":"not_analyzed","summary":null})
        ));
        assert!(!single_target_attempt(&json!({"summary":[2,0]})));
        assert!(single_target_attempt(&json!({"summary":[0,1]})));
    }
    #[test]
    fn method_filter_uses_the_compiler_self_type() {
        let fs=vir::functions("(Function :params ((Param :name (VarIdent self) :typ (Typ Datatype (Dt Path ostd::mm::frame::Frame) () ()))))").unwrap();
        assert_eq!(
            verifier_filter(&fs[0], "mm::frame::frame_ref", "into_raw"),
            "ostd::mm::frame::Frame::into_raw"
        );
        assert_eq!(
            verifier_filter(&fs[0], "mm::frame", "into_raw"),
            "Frame::into_raw"
        );
    }
    #[test]
    fn root_module_method_omits_the_crate_prefix() {
        let fs = vir::functions("(Function :params ((Param :name (VarIdent self 2) :typ (Typ MutRef (Typ Datatype (Dt Path id_alloc::IdAlloc) () ())))))").unwrap();
        assert_eq!(
            verifier_filter(&fs[0], "", "alloc_specific"),
            "IdAlloc::alloc_specific"
        );
    }
    #[test]
    fn reference_receivers_select_the_nominal_method() {
        for typ in [
            "(Typ MutRef (Typ Datatype (Dt Path ostd::cpu::set::AtomicCpuSet) () ()))",
            "(Typ Decorate (TypDecoration Ref) None (Typ Datatype (Dt Path ostd::cpu::set::AtomicCpuSet) () ()))",
        ] {
            let text = format!("(Function :params ((Param :name (VarIdent self 2) :typ {typ})))");
            let fs = vir::functions(&text).unwrap();
            assert_eq!(
                verifier_filter(&fs[0], "cpu::set", "contains"),
                "AtomicCpuSet::contains"
            );
        }
    }
    #[test]
    fn false_probe_requires_assertion_not_assumption_or_literal() {
        let fs=vir::functions(r#"(Function :body (Block (> AssertAssume :is_assume false :expr (> Const (Constant Bool false))) (> AssertAssume :is_assume true :expr (> Const (Constant Bool false))) (> Const (Constant Bool false))))"#).unwrap();
        assert_eq!(false_assertions(fs[0].field("body").unwrap()), 1);
    }
}
