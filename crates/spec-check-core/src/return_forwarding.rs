//! Bounded correspondence for a call result moved immediately into a fresh local.
use crate::{
    Result,
    ir::{self, Record},
};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn compare(before: &[Record], after: &[Record]) -> Result<Value> {
    if ir::opaque_signature(before)? != ir::opaque_signature(after)? {
        return Ok(json!({"matches":false,"reason":"literal or nominal operand identity changed"}));
    }
    // Parameters need a compiler-provided positional mapping before extending this rule.
    let decl = |xs: &[Record]| {
        xs.iter()
            .find(|r| r["kind"] == "decl")
            .map(|r| r["snippet"].clone())
    };
    if decl(before) != decl(after) || !supports_declaration(before)? {
        return Err("return forwarding requires an unchanged zero-argument declaration".into());
    }
    let (left, a) = normalize(before)?;
    let (right, b) = normalize(after)?;
    Ok(
        json!({"matches":(!a.is_empty() || !b.is_empty()) && left==right,"original":a,"generated":b,
        "scope":"Immediate unborrowed call-result forwarding; all calls, payloads, branches, moves and borrow events retained"}),
    )
}
pub(crate) fn supports_declaration(records: &[Record]) -> Result<bool> {
    let plain = Regex::new(r#"^Some\("fn [A-Za-z_][A-Za-z_0-9]*\(\)"\)$"#)?;
    Ok(records
        .iter()
        .find(|r| r["kind"] == "decl")
        .is_some_and(|r| plain.is_match(&r["snippet"])))
}

fn normalize(records: &[Record]) -> Result<(Vec<Record>, Vec<Value>)> {
    let local = Regex::new(r"\b_\d+\b")?;
    let quoted = Regex::new(r#""(?:[^"\\]|\\.)*""#)?;
    let field = Regex::new(r"(?:\.|::)_\d+\b")?;
    for r in records.iter().filter(|r| ir::runtime(r)) {
        for (key, s) in r {
            if (matches!(key.as_str(), "type" | "target" | "generic_args") && local.is_match(s))
                || field.is_match(s)
                || quoted.find_iter(s).any(|m| local.is_match(m.as_str()))
            {
                return Err("opaque local-like names require structured MIR operands".into());
            }
        }
    }
    let mut data = ir::normalize(records, None)?;
    let types = data
        .iter()
        .filter(|r| r["kind"] == "local")
        .map(|r| (r["place"].clone(), r["type"].clone()))
        .collect::<BTreeMap<_, _>>();
    let names = |s: &str| {
        local
            .find_iter(s)
            .map(|m| m.as_str().to_owned())
            .collect::<BTreeSet<_>>()
    };
    let mut refs = types
        .iter()
        .filter(|(_, t)| t.starts_with('&') && !t.starts_with("&mut ") && !t.contains(" mut "))
        .map(|(p, _)| p.clone())
        .collect::<BTreeSet<_>>();
    for r in &data {
        if r["kind"] == "loan" {
            refs.retain(|p| !names(&r["borrowed_place"]).contains(p));
        }
        if r["kind"] == "stmt" && r["operation"].contains('&') {
            let rhs = r["operation"]
                .split_once('=')
                .map_or(r["operation"].as_str(), |(_, v)| v);
            refs.retain(|p| !names(rhs).contains(p));
        }
    }
    let movement = Regex::new(r"^(_\d+) = move (_\d+)$")?;
    let storage = Regex::new(r"^Storage(?:Live|Dead)\((_[0-9]+)\)$")?;
    let region = Regex::new(r"'\?\d+")?;
    let block = Regex::new(r"bb\d+")?;
    let mut removed = BTreeMap::<String, BTreeSet<usize>>::new();
    let mut certificates = Vec::new();
    let moves = data
        .iter()
        .filter(|r| r["kind"] == "stmt")
        .cloned()
        .collect::<Vec<_>>();
    for transfer in moves {
        let Some(m) = movement.captures(&transfer["operation"]) else {
            continue;
        };
        let (target, source) = (&m[1], &m[2]);
        if target == source || target == "_0" || source == "_0" {
            continue;
        }
        let creators = data
            .iter()
            .filter(|r| r["kind"] == "call" && r["destination"] == source)
            .collect::<Vec<_>>();
        let [creator] = creators.as_slice() else {
            continue;
        };
        if names(&creator["arguments"]).contains(source) {
            continue;
        }
        let successors = data
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == creator["block"]);
        let ret = Regex::new(&format!(
            r" -> \[return: {}(?:, unwind: bb\d+)?\]$",
            regex::escape(&transfer["block"])
        ))?;
        if !successors.is_some_and(|r| ret.is_match(&r["operation"])) {
            continue;
        }
        let predecessors = data
            .iter()
            .filter(|r| {
                r["kind"] == "edge"
                    && block
                        .find_iter(&r["successors"])
                        .any(|m| m.as_str() == transfer["block"])
            })
            .map(|r| r["block"].as_str())
            .collect::<BTreeSet<_>>();
        if predecessors != BTreeSet::from([creator["block"].as_str()]) {
            continue;
        }
        let (Some(a), Some(b)) = (types.get(source), types.get(target)) else {
            continue;
        };
        if region.replace_all(a, "R") != region.replace_all(b, "R") {
            continue;
        }
        let book = |op: &str| {
            [
                format!("StorageLive({source})"),
                format!("StorageDead({source})"),
                format!("FakeRead(ForLet(None), {source})"),
            ]
            .contains(&op.to_owned())
        };
        let prefix_ok = data
            .iter()
            .filter(|r| {
                r["kind"] == "stmt"
                    && r["block"] == transfer["block"]
                    && r["statement_index"].parse::<usize>().unwrap()
                        < transfer["statement_index"].parse::<usize>().unwrap()
            })
            .all(|r| {
                book(&r["operation"])
                    || storage
                        .captures(&r["operation"])
                        .is_some_and(|m| refs.contains(&m[1]))
            });
        if !prefix_ok {
            continue;
        }
        let mut erase = Vec::new();
        let mut valid = true;
        for (i, r) in data.iter().enumerate() {
            if r["kind"] == "local" || !r.values().any(|s| names(s).contains(source)) {
                continue;
            }
            if r == &transfer || (r["kind"] == "stmt" && book(&r["operation"])) {
                erase.push(i);
                continue;
            }
            if r["kind"] == "call" && r["block"] == creator["block"] {
                continue;
            }
            if r["kind"] == "term"
                && r["block"] == creator["block"]
                && r["operation"].starts_with(&format!("{source} = "))
            {
                continue;
            }
            valid = false;
            break;
        }
        if !valid {
            continue;
        }
        certificates
            .push(json!({"source":source,"destination":target,"call":creator,"transfer":transfer}));
        for &i in &erase {
            let r = &data[i];
            removed
                .entry(r["block"].clone())
                .or_default()
                .insert(r["statement_index"].parse()?);
        }
        data = data
            .into_iter()
            .enumerate()
            .filter_map(|(i, r)| {
                (!erase.contains(&i) && !(r["kind"] == "local" && r["place"] == source))
                    .then_some(r)
            })
            .collect();
        for r in &mut data {
            for s in r.values_mut() {
                *s = local
                    .replace_all(s, |c: &regex::Captures| {
                        if &c[0] == source {
                            target.to_owned()
                        } else {
                            c[0].to_owned()
                        }
                    })
                    .into_owned();
            }
        }
    }
    data.retain(|r| {
        if r["kind"] == "stmt"
            && storage
                .captures(&r["operation"])
                .is_some_and(|m| refs.contains(&m[1]))
        {
            removed
                .entry(r["block"].clone())
                .or_default()
                .insert(r["statement_index"].parse().unwrap());
            false
        } else {
            true
        }
    });
    let used = data
        .iter()
        .filter(|r| r["kind"] != "local")
        .flat_map(|r| r.values().flat_map(|s| names(s)))
        .collect::<BTreeSet<_>>();
    data.retain(|r| !(r["kind"] == "local" && r["type"] == "!" && !used.contains(&r["place"])));
    let point = Regex::new(r"(bb\d+)\[(\d+)\]")?;
    for r in &mut data {
        if r["kind"] == "stmt" {
            let i = r["statement_index"].parse::<usize>()?;
            let n = removed
                .get(&r["block"])
                .map_or(0, |xs| xs.range(..i).count());
            r.insert("statement_index".into(), (i - n).to_string());
        }
        let boundary = r["kind"] == "loan_out_of_scope";
        for (key, s) in r.iter_mut() {
            if key == "statement_index" {
                continue;
            }
            let mut invalid = false;
            *s = point
                .replace_all(s, |c: &regex::Captures| {
                    let i = c[2].parse::<usize>().unwrap();
                    let xs = removed.get(&c[1]);
                    if !boundary && xs.is_some_and(|xs| xs.contains(&i)) {
                        invalid = true
                    }
                    format!(
                        "{}[{}]",
                        &c[1],
                        i - xs.map_or(0, |xs| xs.range(..i).count())
                    )
                })
                .into_owned();
            if invalid {
                return Err("forwarding would erase an issued borrow event".into());
            }
        }
    }
    let mut rename = BTreeMap::from([("_0".to_owned(), "_0".to_owned())]);
    for r in &data {
        if !matches!(r["kind"].as_str(), "stmt" | "term" | "call") {
            continue;
        }
        for key in ["operation", "arguments", "destination"] {
            for m in local.find_iter(r.get(key).map_or("", String::as_str)) {
                let next = format!("_{}", rename.len());
                rename.entry(m.as_str().to_owned()).or_insert(next);
            }
        }
    }
    let mut regions = BTreeMap::new();
    for r in &mut data {
        for s in r.values_mut() {
            *s = local
                .replace_all(s, |c: &regex::Captures| {
                    rename
                        .get(&c[0])
                        .cloned()
                        .unwrap_or_else(|| c[0].to_owned())
                })
                .into_owned();
            *s = region
                .replace_all(s, |c: &regex::Captures| {
                    let next = format!("'?{}", regions.len());
                    regions.entry(c[0].to_owned()).or_insert(next).clone()
                })
                .into_owned();
        }
    }
    let mut locals = data
        .iter()
        .filter(|r| r["kind"] == "local")
        .cloned()
        .collect::<Vec<_>>();
    locals.sort_by_key(|r| r["place"][1..].parse::<usize>().unwrap());
    locals.extend(data.into_iter().filter(|r| r["kind"] != "local"));
    Ok((locals, certificates))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(forward: bool) -> Vec<Record> {
        let mut text = "CLIENT_DECL\tunused\tSome(\"fn client()\")\nCLIENT_LOCAL\t_0\t()\nCLIENT_LOCAL\t_1\tGuard\nCLIENT_LOCAL\t_2\t&'?1 Guard\n".to_owned();
        if forward {
            text.push_str("CLIENT_LOCAL\t_3\tGuard\n");
        }
        let destination = if forward { "_3" } else { "_1" };
        text.push_str(&format!("CLIENT_EDGE\tbb0\t[bb1]\nCLIENT_TERM\tbb0\tunused\t{destination} = make(const 7) -> [return: bb1]\nCLIENT_CALL\tbb0\tmake\tunused\t[const 7]\t{destination}\t[]\n"));
        if forward {
            text.push_str("CLIENT_STMT\tbb1\t0\tunused\tFakeRead(ForLet(None), _3)\nCLIENT_STMT\tbb1\t1\tunused\t_1 = move _3\n");
        }
        let index = if forward { 2 } else { 0 };
        text.push_str(&format!("CLIENT_STMT\tbb1\t{index}\tunused\t_2 = &_1\nCLIENT_LOAN\tbb1[{index}]\tShared\t_1\t_2\t'?1\tNotTwoPhase\nCLIENT_LOAN_ISSUED\tbw0\t'?1\tMid(bb1[{index}])\nCLIENT_LOAN_OUT_OF_SCOPE\tbb2[0]\t[bw0]\nCLIENT_EDGE\tbb1\t[bb2]\nCLIENT_TERM\tbb1\tunused\t_0 = consume(move _2) -> [return: bb2]\nCLIENT_CALL\tbb1\tconsume\tunused\t[move _2]\t_0\t[]\nCLIENT_EDGE\tbb2\t[]\nCLIENT_TERM\tbb2\tunused\treturn\n"));
        ir::parse(&text).unwrap()
    }
    #[test]
    fn immediate_transfer_retains_borrow_events() {
        let report = compare(&fixture(false), &fixture(true)).unwrap();
        assert_eq!(report["matches"], true);
        assert_eq!(report["generated"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn extra_uses_and_execution_changes_are_rejected() {
        let before = fixture(false);
        for mutation in [
            "callee",
            "argument",
            "borrow",
            "branch",
            "type",
            "copy",
            "read",
            "alias",
            "duplicate",
        ] {
            let mut after = fixture(true);
            match mutation {
                "callee" => {
                    for r in &mut after {
                        if r["kind"] == "call" && r["target"] == "make" {
                            r.insert("target".into(), "other".into());
                        }
                    }
                }
                "argument" => {
                    for r in &mut after {
                        if r["kind"] == "call" {
                            r.insert("arguments".into(), "[const 9]".into());
                            break;
                        }
                    }
                }
                "borrow" => {
                    for r in &mut after {
                        if r["kind"] == "loan" {
                            r.insert("borrowed_place".into(), "_0".into());
                        }
                    }
                }
                "branch" => {
                    for r in &mut after {
                        if r["kind"] == "edge" && r["block"] == "bb0" {
                            r.insert("successors".into(), "[bb1, bb2]".into());
                        }
                    }
                }
                "type" => {
                    for r in &mut after {
                        if r["kind"] == "local" && r["place"] == "_3" {
                            r.insert("type".into(), "Other".into());
                        }
                    }
                }
                "copy" => {
                    for r in &mut after {
                        if r["kind"] == "stmt" && r["operation"] == "_1 = move _3" {
                            r.insert("operation".into(), "_1 = copy _3".into());
                        }
                    }
                }
                name => {
                    let operation = match name {
                        "read" => "_0 = discriminant(_3)",
                        "alias" => "_2 = &_3",
                        _ => "_0 = move _3",
                    };
                    after.extend(
                        ir::parse(&format!("CLIENT_STMT\tbb1\t3\tunused\t{operation}\n")).unwrap(),
                    );
                }
            }
            assert_ne!(
                compare(&before, &after).unwrap()["matches"],
                true,
                "{mutation}"
            );
        }
    }
    #[test]
    fn parameter_and_opaque_operand_renaming_are_not_supported() {
        let before = fixture(false);
        let mut after = fixture(true);
        after[0].insert("snippet".into(), "Some(\"fn client(x: Guard)\")".into());
        assert!(compare(&before, &after).is_err());
        after = fixture(true);
        for r in &mut after {
            if r["kind"] == "call" {
                r.insert("arguments".into(), "[const \"_3\"]".into());
                break;
            }
        }
        assert_ne!(compare(&before, &after).unwrap()["matches"], true);
    }
}
