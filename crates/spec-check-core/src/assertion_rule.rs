use crate::{Result, hash, ir::Record};
use serde_json::{Value, json};

const TEMPLATE: &str = include_str!("../../../experiments/test-spec/assertion-rule/usize.template");
const MODEL: &[u8] = include_bytes!("../../../experiments/test-spec/assertion-rule/semantics.rs");
const APPROVED: &str = include_str!("../../../experiments/test-spec/assertion-rule/rule.json");

pub fn validate(records: &[Record], origin: &str) -> Result<()> {
    for index in ["0", "1"] {
        let domains = records
            .iter()
            .filter(|r| {
                r["kind"] == "assert_domain" && r["assertion_span"] == origin && r["index"] == index
            })
            .collect::<Vec<_>>();
        let [domain] = domains.as_slice() else {
            return Err("usize assertion rule requires one Rustc domain record per operand".into());
        };
        if domain["drop_free"] != "true"
            || domain["local_control"] != "true"
            || domain["direct_macro"] != "true"
            || domain["word_bits"] != "64"
        {
            return Err("usize assertion rule requires direct core macro source, 64-bit usize, Drop-free subexpression types and local safe control".into());
        }
    }
    Ok(())
}

pub(crate) fn range_option(records: &[Record], typ: &str) -> bool {
    ["std", "core"].iter().any(|root| {
        typ == format!("{root}::option::Option<{root}::ops::Range<usize>>")
            && records.iter().any(|instance| {
                instance["kind"] == "instance"
                    && instance["target"]
                        == format!("<{root}::option::Option<T> as {root}::cmp::PartialEq>::eq")
                    && instance["generic_args"] == format!("[{root}::ops::Range<usize>]")
                    && records.iter().any(|call| {
                        call["kind"] == "call"
                            && call["block"] == instance["block"]
                            && call["target"] == format!("{root}::cmp::PartialEq::eq")
                            && call["generic_args"] == format!("[{typ}, {typ}]")
                    })
            })
    })
}

pub fn stamp() -> Result<Value> {
    let approved: Value = serde_json::from_str(APPROVED)?;
    if approved["template_sha256"] != hash(TEMPLATE.as_bytes())?
        || approved["model_sha256"] != hash(MODEL)?
    {
        return Err("assertion rule changed; recheck its model and template binding".into());
    }
    Ok(approved)
}

pub fn compiler_binding(binding: &Value) -> Result<()> {
    let approved = stamp()?;
    if binding["release"] != approved["compiler_release"]
        || binding["macro_sha256"] != approved["macro_sha256"]
    {
        return Err("usize rule requires its pinned Rustc and installed core macro source".into());
    }
    Ok(())
}

pub fn instantiate(left: &str, right: &str) -> Result<String> {
    let (prefix, tail) = TEMPLATE
        .trim_end()
        .split_once("@LEFT@")
        .ok_or("missing left placeholder")?;
    let (middle, suffix) = tail
        .split_once("@RIGHT@")
        .ok_or("missing right placeholder")?;
    Ok(format!("{prefix}{left}{middle}{right}{suffix}"))
}

pub fn report(records: &[Record]) -> Result<Value> {
    let mut instances = Vec::new();
    for operand in records.iter().filter(|r| {
        r["kind"] == "assert_operand"
            && matches!(r["macro"].as_str(), "std::assert_eq" | "core::assert_eq")
            && r["index"] == "0"
    }) {
        let primitive = operand["type"] == "usize";
        if primitive {
            validate(records, &operand["assertion_span"])?;
        }
        instances.push(
            json!({"span":operand["assertion_span"],"type":operand["type"],
            "rule":if primitive {"usize-assert-eq-v1"} else if range_option(records, &operand["type"]) {"original-option-range-equality-outside-theorem"} else {"prototype-vec-outside-theorem"},
            "domains":records.iter().filter(|r| r["kind"] == "assert_domain"
                && r["assertion_span"] == operand["assertion_span"]).collect::<Vec<_>>()}),
        );
    }
    Ok(json!({"binding":stamp()?,"instances":instances,
        "scope":"Local rule in a finite observation model; same operand evaluator and primitive reference semantics are assumed. No whole-Rust, arbitrary-CFG, cleanup, panic-payload or Vec theorem. Whole-client MIR correspondence remains independent."}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires RANGE_EQUALITY_BODIES from actual Rustc extraction"]
    fn actual_range_equality_requires_resolved_standard_instance() {
        let raw = std::fs::read_to_string(std::env::var("RANGE_EQUALITY_BODIES").unwrap()).unwrap();
        let bodies = crate::client::borrow_records(&raw).unwrap();
        let records = bodies
            .values()
            .find(|records| {
                records.iter().any(|r| {
                    r["kind"] == "assert_operand"
                        && r["type"] == "std::option::Option<std::ops::Range<usize>>"
                })
            })
            .unwrap();
        let typ = "std::option::Option<std::ops::Range<usize>>";
        assert!(range_option(records, typ));
        let mut changed = records.clone();
        changed.retain(|r| r["kind"] != "instance");
        assert!(!range_option(&changed, typ));
        let mut changed = records.clone();
        for record in changed.iter_mut().filter(|r| r["kind"] == "instance") {
            record.insert("generic_args".into(), "[custom::Range]".into());
        }
        assert!(!range_option(&changed, typ));
        assert!(!range_option(
            records,
            "custom::Option<custom::Range<usize>>"
        ));
    }

    #[test]
    fn operand_text_cannot_replace_template_placeholders() {
        let generated = instantiate("f(\"@RIGHT@\")", "2usize").unwrap();
        assert!(generated.contains("&(f(\"@RIGHT@\"))"));
        assert!(generated.contains("&(2usize)"));
    }
    #[test]
    fn absent_or_false_domain_records_are_rejected() {
        assert!(validate(&[], "span").is_err());
        let records = ["0", "1"].map(|index| {
            Record::from([
                ("kind".into(), "assert_domain".into()),
                ("index".into(), index.into()),
                ("assertion_span".into(), "span".into()),
                ("drop_free".into(), "true".into()),
                ("local_control".into(), "true".into()),
                ("direct_macro".into(), "true".into()),
                ("word_bits".into(), "64".into()),
            ])
        });
        assert!(validate(&records, "span").is_ok());
        let mut core = records.to_vec();
        core.push(Record::from([
            ("kind".into(), "assert_operand".into()),
            ("macro".into(), "core::assert_eq".into()),
            ("index".into(), "0".into()),
            ("assertion_span".into(), "span".into()),
            ("type".into(), "usize".into()),
        ]));
        let core_report = report(&core).unwrap();
        assert_eq!(core_report["instances"].as_array().unwrap().len(), 1);
        core.last_mut()
            .unwrap()
            .insert("macro".into(), "std::assert_eq".into());
        assert_eq!(report(&core).unwrap(), core_report);
        let mut altered = records.clone();
        altered[1].insert("drop_free".into(), "false".into());
        assert!(validate(&altered, "span").is_err());
        let mut duplicated = records.to_vec();
        duplicated.push(records[0].clone());
        assert!(validate(&duplicated, "span").is_err());
    }
}
