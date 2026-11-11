use crate::Result;
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
pub type Record = BTreeMap<String, String>;
pub fn parse(text: &str) -> Result<Vec<Record>> {
    let mut result = Vec::new();
    for line in text.lines().filter(|s| s.starts_with("CLIENT_")) {
        let mut parts = line.split('\t');
        let kind = parts
            .next()
            .unwrap()
            .trim_start_matches("CLIENT_")
            .to_lowercase();
        let fields: &[&str] = match kind.as_str() {
            "decl" | "body" => &["span", "snippet"],
            "container" => &["container_kind", "span", "snippet"],
            "closure_kind" => &["closure_kind"],
            "capture" => &[
                "index",
                "binding",
                "binding_id",
                "capture_kind",
                "type",
                "place",
                "projection_count",
                "span",
            ],
            "capture_use" => &["binding_id", "span"],
            "datatype" => &["path", "span", "verified"],
            "datatype_field" => &["path", "field", "type", "environment"],
            "trait_signature" => &["path", "signature", "environment"],
            "function" => &["path"],
            "candidate" => &["path", "span"],
            "parameters" => &["count"],
            "parameter" => &["index", "place", "type"],
            "local" => &["place", "type"],
            "type_fact" => &["place", "type", "needs_drop"],
            "resource_layout" => &["place", "definition", "size", "align", "needs_drop"],
            "test_kind" => &["expected_panic"],
            "source_expr" => &["expression_kind", "span", "snippet"],
            "source_argument" => &["call_span", "index", "span", "type", "snippet"],
            "source_method" | "source_index" => &["span", "target", "target_id"],
            "identity_closure" => &["span", "binding_mode", "type"],
            "call_shape" => &["block", "operation"],
            "assert_operand" => &[
                "macro",
                "assertion_span",
                "index",
                "span",
                "type",
                "snippet",
            ],
            "assert_domain" => &[
                "assertion_span",
                "index",
                "drop_free",
                "local_control",
                "direct_macro",
                "word_bits",
            ],
            "binding" => &["name", "place", "span", "type"],
            "stmt" => &["block", "statement_index", "span", "operation"],
            "edge" => &["block", "successors"],
            "term" => &["block", "span", "operation"],
            "call" => &[
                "block",
                "target",
                "definition_span",
                "arguments",
                "destination",
                "generic_args",
            ],
            "instance" => &["block", "target", "definition_span", "generic_args"],
            "eq_body" => &["target", "definition_span", "generic_args", "body"],
            "loan" => &[
                "location",
                "borrow_kind",
                "borrowed_place",
                "assigned_place",
                "region",
                "activation",
            ],
            "loan_issued" => &["loan", "region", "point"],
            "loan_out_of_scope" => &["location", "loans"],
            _ => return Err(format!("unknown record: {kind}").into()),
        };
        let values = parts.collect::<Vec<_>>();
        if fields.len() != values.len() {
            return Err("invalid compiler record arity".into());
        }
        let mut record = Record::from([("kind".into(), kind)]);
        for (key, value) in fields.iter().zip(values) {
            record.insert((*key).into(), value.into());
        }
        result.push(record);
    }
    Ok(result)
}
pub fn span(span: &str, cwd: &Path) -> Result<(PathBuf, [usize; 4])> {
    static SPAN: OnceLock<Regex> = OnceLock::new();
    let regex =
        SPAN.get_or_init(|| Regex::new(r"^(.*):(\d+):(\d+): (\d+):(\d+) \(#\d+\)$").unwrap());
    let c = regex.captures(span).ok_or("unsupported source span")?;
    let path = Path::new(&c[1]);
    Ok((
        if path.is_absolute() {
            path.to_owned()
        } else {
            cwd.join(path)
        },
        [c[2].parse()?, c[3].parse()?, c[4].parse()?, c[5].parse()?],
    ))
}
pub fn offsets(text: &str, coordinates: [usize; 4]) -> Result<(usize, usize)> {
    let [sl, sc, el, ec] = coordinates;
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if sl == 0 || el < sl || el > lines.len() || sc == 0 || ec == 0 {
        return Err("source span outside file".into());
    }
    // Rustc spans use character columns; byte edits are restricted to ASCII prefixes.
    if !lines[sl - 1].get(..sc - 1).is_some_and(str::is_ascii)
        || !lines[el - 1].get(..ec - 1).is_some_and(str::is_ascii)
    {
        return Err("non-ASCII source columns unsupported".into());
    }
    let start = lines[..sl - 1].iter().map(|s| s.len()).sum::<usize>() + sc - 1;
    let end = lines[..el - 1].iter().map(|s| s.len()).sum::<usize>() + ec - 1;
    if start > end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return Err("invalid source offsets".into());
    }
    Ok((start, end))
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub origin: String,
}
pub fn lower(records: &[Record], cwd: &Path) -> Result<(PathBuf, String, Vec<Edit>)> {
    lower_body(records, cwd, None)
}

pub(crate) fn lower_closure_assertions(
    records: &[Record],
    parent: &[Record],
    cwd: &Path,
) -> Result<(PathBuf, String, Vec<Edit>)> {
    let body = parent
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("missing enclosing function body")?;
    let (file, coordinates) = span(&body["span"], cwd)?;
    let (_, anchor) = offsets(&fs::read_to_string(&file)?, coordinates)?;
    lower_body(records, cwd, Some((file, anchor)))
}

fn lower_body(
    records: &[Record],
    cwd: &Path,
    closure: Option<(PathBuf, usize)>,
) -> Result<(PathBuf, String, Vec<Edit>)> {
    let decl = records
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("missing test declaration")?;
    let (file, coords) = span(&decl["span"], cwd)?;
    let source = fs::read_to_string(&file)?;
    let (start, _) = offsets(&source, coords)?;
    let body = records
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("missing test body")?;
    let (body_file, coords) = span(&body["span"], cwd)?;
    if body_file != file {
        return Err("test body and declaration in different files".into());
    }
    let (_, end) = offsets(&source, coords)?;
    if closure
        .as_ref()
        .is_some_and(|(parent, anchor)| parent != &file || *anchor < end || *anchor > source.len())
    {
        return Err(
            "closure assertion helper must follow its enclosing function in the same source file"
                .into(),
        );
    }
    let mut groups: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
    for r in records.iter().filter(|r| {
        r["kind"] == "assert_operand"
            && matches!(r["macro"].as_str(), "std::assert_eq" | "core::assert_eq")
    }) {
        groups
            .entry(r["assertion_span"].clone())
            .or_default()
            .push(r);
    }
    let mut edits = Vec::new();
    for (origin, mut operands) in groups {
        operands.sort_by_key(|r| r["index"].clone());
        if operands.len() != 2
            || operands[0]["index"] != "0"
            || operands[1]["index"] != "1"
            || operands[0]["type"] != operands[1]["type"]
            || !(operands[0]["type"] == "usize"
                || operands[0]["type"].starts_with("std::vec::Vec<")
                || crate::assertion_rule::range_option(records, &operands[0]["type"]))
        {
            return Err(
                "unsupported assert_eq type or structure (Rust entry supports usize, standard Vec, and resolved Option<Range<usize>> equality)"
                    .into(),
            );
        }
        let (path, coords) = span(&origin, cwd)?;
        if path != file {
            return Err("nested/source-generated assertion needs its own translation".into());
        }
        let (a, b) = offsets(&source, coords)?;
        let mut args = Vec::new();
        let mut ranges = Vec::new();
        for operand in &operands {
            let (path, c) = span(&operand["span"], cwd)?;
            if path != file {
                return Err("assertion operand source mismatch".into());
            }
            let (l, r) = offsets(&source, c)?;
            ranges.push((l, r));
            args.push(&source[l..r]);
        }
        let [(ls, le), (rs, re)] = ranges[..] else {
            unreachable!()
        };
        if !(a < ls && ls < le && le < rs && rs < re && re < b) {
            return Err("assertion source ranges overlap".into());
        }
        let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        if !["assert_eq!(", "std::assert_eq!(", "core::assert_eq!("]
            .contains(&compact(&source[a..ls]).as_str())
            || source[le..rs].trim() != ","
            || ![")", ",)"].contains(&compact(&source[re..b]).as_str())
        {
            return Err("custom messages or unsupported assertion syntax".into());
        }
        if operands[0]["type"] == "usize" {
            crate::assertion_rule::validate(records, &origin)?;
        }
        crate::assertion_rule::stamp()?;
        edits.push(Edit {
            start: a,
            end: b,
            origin,
            replacement: crate::assertion_rule::instantiate(args[0], args[1])?,
        });
    }
    edits.sort_by_key(|e| e.start);
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return Err("nested assertion edits unsupported".into());
    }
    if !edits.is_empty() {
        if source.contains("checked_eq_failure") {
            return Err("failure helper name collides".into());
        }
        edits.push(Edit {
            start: closure.as_ref().map_or(end, |(_, anchor)| *anchor),
            end: closure.as_ref().map_or(end, |(_, anchor)| *anchor),
            replacement: format!(
                "\nvstd::prelude::verus! {{\n{}\n}}\n",
                include_str!("../assets/experiments/test-spec/assertion_failure.rs")
            ),
            origin: body["span"].clone(),
        });
    }
    if closure.is_none() {
        edits.push(Edit {
            start,
            end: start,
            replacement: "#[vstd::prelude::verus_verify]\n".into(),
            origin: decl["span"].clone(),
        });
    }
    edits.sort_by_key(|e| e.start);
    let mut changed = source.clone();
    for e in edits.iter().rev() {
        changed.replace_range(e.start..e.end, &e.replacement)
    }
    Ok((file, changed, edits))
}
pub(crate) fn runtime(r: &Record) -> bool {
    matches!(
        r["kind"].as_str(),
        "local" | "stmt" | "edge" | "term" | "call" | "loan" | "loan_issued" | "loan_out_of_scope"
    )
}

pub(crate) fn lower_client(records: &[Record], cwd: &Path) -> Result<(PathBuf, String, Vec<Edit>)> {
    let (file, _, mut edits) = lower(records, cwd)?;
    let source = fs::read_to_string(&file)?;
    let containers = records
        .iter()
        .filter(|r| r["kind"] == "container")
        .collect::<Vec<_>>();
    if !containers.is_empty() {
        let [container] = containers.as_slice() else {
            return Err("ambiguous client container".into());
        };
        if !matches!(
            container["container_kind"].as_str(),
            "impl" | "inherent_impl" | "trait"
        ) || !container["span"].ends_with("(#0)")
        {
            return Err("source-generated impl container unsupported".into());
        }
        let (path, coords) = span(&container["span"], cwd)?;
        if path != file {
            return Err("client container is in another source file".into());
        }
        let (start, end) = offsets(&source, coords)?;
        let decl = records
            .iter()
            .find(|r| r["kind"] == "decl")
            .ok_or("missing declaration")?;
        let (_, pos) = span(&decl["span"], cwd)?;
        let (a, b) = offsets(&source, pos)?;
        if start >= a
            || a > b
            || if container["container_kind"] == "trait" {
                b > end
            } else {
                end > a
            }
        {
            return Err("container does not enclose or precede its client".into());
        }
        let marker = "#[vstd::prelude::verus_verify]\n";
        if edits.iter().filter(|e| e.replacement == marker).count() != 1 {
            return Err("client verification attribute is ambiguous".into());
        }
        if container["container_kind"] == "trait" {
            edits.push(Edit {
                start,
                end: start,
                replacement: marker.into(),
                origin: container["span"].clone(),
            });
        }
        if container["container_kind"] == "impl" {
            edits.retain(|e| e.replacement != marker);
            edits.push(Edit {
                start,
                end: start,
                replacement: marker.into(),
                origin: container["span"].clone(),
            });
        }
    }
    edits.push(Edit {
        start: source.len(),
        end: source.len(),
        replacement: "\n#[allow(unused_imports)] use vstd::prelude::verus_spec;\n".into(),
        origin: "Verification macro import".into(),
    });
    edits.sort_by_key(|e| e.start);
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return Err("overlapping client edits".into());
    }
    let mut changed = source;
    for e in edits.iter().rev() {
        changed.replace_range(e.start..e.end, &e.replacement);
    }
    Ok((file, changed, edits))
}

fn local_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b_\d+\b").unwrap())
}
fn locals(text: &str) -> BTreeSet<String> {
    local_regex()
        .find_iter(text)
        .map(|m| m.as_str().to_owned())
        .collect()
}
// The debug-record renamer is not a MIR lexer. Preserve opaque operands separately.
pub(crate) fn opaque_signature(records: &[Record]) -> Result<Vec<(String, String)>> {
    let tokens = Regex::new(r#""(?:[^"\\]|\\.)*"|(?:\.|::)_\d+\b"#)?;
    let mut result = Vec::new();
    for r in records.iter().filter(|r| runtime(r)) {
        for (key, value) in r {
            if key == "target"
                || (matches!(key.as_str(), "type" | "generic_args")
                    && local_regex().is_match(value))
            {
                result.push((key.clone(), value.clone()));
            } else if matches!(key.as_str(), "operation" | "arguments") {
                result.extend(
                    tokens
                        .find_iter(value)
                        .map(|m| (key.clone(), m.as_str().to_owned())),
                );
            }
        }
    }
    Ok(result)
}
pub(crate) fn assertion_opaque_signature(
    records: &[Record],
    failure: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let Some(failure) = failure else {
        return opaque_signature(records);
    };
    // Validate the permitted failure bookkeeping, then inspect untouched tokens.
    // Running the local renamer first could hide a changed string such as "_1".
    normalize(records, Some(failure))?;
    let blocks = records
        .iter()
        .filter(|r| r["kind"] == "call" && r["target"] == failure)
        .map(|r| r["block"].clone())
        .collect::<BTreeSet<_>>();
    let mut visible = records
        .iter()
        .filter(|r| runtime(r))
        .filter(|r| !r.get("block").is_some_and(|b| blocks.contains(b)))
        .cloned()
        .collect::<Vec<_>>();
    let used = visible
        .iter()
        .filter(|r| r["kind"] != "local")
        .flat_map(|r| {
            r.iter()
                .filter(|(k, _)| k.as_str() != "span" && k.as_str() != "definition_span")
                .flat_map(|(_, v)| locals(v))
        })
        .collect::<BTreeSet<_>>();
    visible.retain(|r| r["kind"] != "local" || r["place"] == "_0" || used.contains(&r["place"]));
    opaque_signature(&visible)
}

pub(crate) fn remap(
    mut records: Vec<Record>,
    removed: &BTreeMap<String, Vec<usize>>,
    regions: bool,
) -> Result<Vec<Record>> {
    let names = records
        .iter()
        .filter(|r| r["kind"] == "local")
        .enumerate()
        .map(|(i, r)| (r["place"].clone(), format!("_{i}")))
        .collect::<BTreeMap<_, _>>();
    let local = local_regex();
    let point = Regex::new(r"(bb\d+)\[(\d+)\]")?;
    let region = Regex::new(r"'\?\d+")?;
    let mut renames = BTreeMap::new();
    for r in &mut records {
        if r["kind"] == "stmt" {
            let index = r["statement_index"].parse::<usize>()?;
            let n = removed
                .get(&r["block"])
                .map_or(0, |v| v.iter().filter(|&&x| x < index).count());
            r.insert("statement_index".into(), (index - n).to_string());
        }
        for (key, value) in r
            .iter_mut()
            .filter(|(k, _)| k.as_str() != "span" && k.as_str() != "definition_span")
        {
            let _ = key;
            *value = local
                .replace_all(value, |c: &regex::Captures| {
                    names.get(&c[0]).cloned().unwrap_or_else(|| c[0].to_owned())
                })
                .into_owned();
            if regions {
                *value = region
                    .replace_all(value, |c: &regex::Captures| {
                        let next = format!("'?{}", renames.len());
                        renames.entry(c[0].to_owned()).or_insert(next).clone()
                    })
                    .into_owned();
            }
            let mut invalid = false;
            *value = point
                .replace_all(value, |c: &regex::Captures| {
                    let index = c[2].parse::<usize>().unwrap();
                    let xs = removed.get(&c[1]);
                    if xs.is_some_and(|v| v.contains(&index)) {
                        invalid = true;
                    }
                    format!(
                        "{}[{}]",
                        &c[1],
                        index - xs.map_or(0, |v| v.iter().filter(|&&x| x < index).count())
                    )
                })
                .into_owned();
            if invalid {
                return Err("loan point refers to removed bookkeeping".into());
            }
        }
    }
    Ok(records)
}
pub fn normalize(records: &[Record], failure: Option<&str>) -> Result<Vec<Record>> {
    let mut data = records
        .iter()
        .filter(|r| runtime(r))
        .cloned()
        .collect::<Vec<_>>();
    let candidates = data
        .iter()
        .filter(|r| r["kind"] == "local" && r["type"] == "()" && r["place"] != "_0")
        .map(|r| r["place"].clone())
        .collect::<BTreeSet<_>>();
    let mut removable = candidates.clone();
    for r in data.iter().filter(|r| r["kind"] != "local") {
        let refs = r
            .iter()
            .filter(|(k, _)| k.as_str() != "span" && k.as_str() != "definition_span")
            .flat_map(|(_, s)| locals(s))
            .collect::<BTreeSet<_>>();
        for place in refs.intersection(&candidates) {
            let op = r.get("operation").map(String::as_str).unwrap_or("");
            if r["kind"] != "stmt"
                || ![
                    format!("StorageLive({place})"),
                    format!("StorageDead({place})"),
                    format!("{place} = const ()"),
                ]
                .contains(&op.to_owned())
            {
                removable.remove(place);
            }
        }
    }
    let mut removed: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    data.retain(|r| {
        if r["kind"] == "local" && removable.contains(&r["place"]) {
            return false;
        }
        if r["kind"] == "stmt" && !locals(&r["operation"]).is_disjoint(&removable) {
            removed
                .entry(r["block"].clone())
                .or_default()
                .push(r["statement_index"].parse().unwrap());
            return false;
        }
        true
    });
    data = remap(data, &removed, false)?;
    if let Some(target) = failure {
        let failures = data
            .iter()
            .filter(|r| r["kind"] == "call" && r["target"] == target)
            .map(|r| r["block"].clone())
            .collect::<BTreeSet<_>>();
        if failures.is_empty() {
            return Err("missing selected failure exits".into());
        }
        let types = data
            .iter()
            .filter(|r| r["kind"] == "local")
            .map(|r| (r["place"].clone(), r["type"].clone()))
            .collect::<BTreeMap<_, _>>();
        let storage =
            Regex::new(r"^(Storage(Live|Dead)\(_\d+\)|FakeRead\(ForLet\(None\), _\d+\))$")?;
        let reborrow = Regex::new(r"^&\(\*_\d+\)$")?;
        let move_local = Regex::new(r"^move _\d+$")?;
        for r in data
            .iter()
            .filter(|r| r["kind"] == "stmt" && failures.contains(&r["block"]))
        {
            let op = &r["operation"];
            if storage.is_match(op) {
                continue;
            }
            let (place, value) = op.split_once(" = ").ok_or("effect in failure block")?;
            let ty = types.get(place).ok_or("projected write in failure block")?;
            let allowed = (ty.starts_with("&'?")
                && !ty.contains(" mut ")
                && reborrow.is_match(value))
                || (ty == "core::panicking::AssertKind"
                    && (value == "core::panicking::AssertKind::Eq" || move_local.is_match(value)))
                || ["std", "core"].iter().any(|root| {
                    ty.starts_with(&format!("{root}::option::Option<{root}::fmt::Arguments<"))
                        && value
                            == format!("{root}::option::Option::<{root}::fmt::Arguments<'_>>::None")
                });
            if !allowed {
                return Err("unsupported failure state change or formatting".into());
            }
        }
        data.retain(|r| {
            !r.get("block").is_some_and(|s| failures.contains(s))
                && !r
                    .iter()
                    .filter(|(k, _)| k.as_str() == "point" || k.as_str() == "location")
                    .any(|(_, s)| failures.iter().any(|b| s.contains(&format!("{b}["))))
        });
        let mut failures = failures.into_iter().collect::<Vec<_>>();
        failures.sort_by_key(|s| s[2..].parse::<usize>().unwrap());
        for b in failures {
            data.push(Record::from([
                ("kind".into(), "edge".into()),
                ("block".into(), b.clone()),
                ("successors".into(), "[]".into()),
            ]));
            data.push(Record::from([
                ("kind".into(), "term".into()),
                ("block".into(), b),
                ("operation".into(), "assertion_failure".into()),
            ]));
        }
        let storage = Regex::new(r"^Storage(Live|Dead)\(_\d+\)$")?;
        let mut used = BTreeSet::from(["_0".to_owned()]);
        for r in &data {
            if r["kind"] == "local" || (r["kind"] == "stmt" && storage.is_match(&r["operation"])) {
                continue;
            }
            for (k, s) in r {
                if k != "span" && k != "definition_span" {
                    used.extend(locals(s));
                }
            }
        }
        let mut removed: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        data.retain(|r| {
            if r["kind"] == "local" && !used.contains(&r["place"]) {
                return false;
            }
            if r["kind"] == "stmt"
                && storage.is_match(&r["operation"])
                && locals(&r["operation"]).is_disjoint(&used)
            {
                removed
                    .entry(r["block"].clone())
                    .or_default()
                    .push(r["statement_index"].parse().unwrap());
                return false;
            }
            true
        });
        data = remap(data, &removed, true)?;
    }
    let def_id = Regex::new(r"DefId\(\d+:\d+ ~ ")?;
    let closure = Regex::new(r"\{closure@[^}]+\}")?;
    let argument_span = Regex::new(r", span: [^}]+")?;
    for r in &mut data {
        r.remove("span");
        r.remove("definition_span");
        if r["kind"] == "call" {
            let s = argument_span.replace_all(&r["arguments"], "").into_owned();
            r.insert("arguments".into(), s);
        }
        for s in r.values_mut() {
            *s = def_id.replace_all(s, "DefId(").into_owned();
            *s = closure
                .replace_all(s, "{closure@source-position}")
                .into_owned();
        }
    }
    Ok(data)
}
pub fn compare(before: &[Record], after: &[Record], failure: Option<&str>) -> Result<bool> {
    if let Some(target) = failure {
        let sig = |data: &[Record], name: &str| {
            data.iter()
                .filter(|r| r["kind"] == "call" && r["target"] == name)
                .map(|r| (r["block"].clone(), r["generic_args"].clone()))
                .collect::<Vec<_>>()
        };
        let old = sig(before, "core::panicking::assert_failed");
        if old.is_empty() || old != sig(after, target) {
            return Err("failure signatures changed".into());
        }
    }
    Ok(
        normalize(before, failure.map(|_| "core::panicking::assert_failed"))?
            == normalize(after, failure)?,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    #[test]
    #[ignore = "requires INDEX_HIR_RUN from actual old/new BTreeMap compiler probes"]
    fn index_operands_and_instance_preserve_runtime_records() {
        let run = std::path::PathBuf::from(std::env::var("INDEX_HIR_RUN").unwrap());
        let before =
            parse(&std::fs::read_to_string(run.join("before-metadata.stderr")).unwrap()).unwrap();
        let after =
            parse(&std::fs::read_to_string(run.join("after-metadata.stderr")).unwrap()).unwrap();
        assert!(compare(&before, &after, None).unwrap());
        let indices = after
            .iter()
            .filter(|r| r["kind"] == "source_index")
            .collect::<Vec<_>>();
        assert_eq!(indices.len(), 1);
        let index = indices[0];
        assert_eq!(index["target"], "std::ops::Index::index");
        let arguments = after
            .iter()
            .filter(|r| r["kind"] == "source_argument" && r["call_span"] == index["span"])
            .collect::<Vec<_>>();
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[0]["snippet"], "Some(\"map\")");
        assert_eq!(arguments[1]["snippet"], "Some(\"&37\")");
        let instances = after
            .iter()
            .filter(|r| {
                r["kind"] == "instance"
                    && r["target"]
                        == "<std::collections::BTreeMap<K, V, A> as std::ops::Index<&Q>>::index"
            })
            .collect::<Vec<_>>();
        assert_eq!(instances.len(), 1);
        assert!(instances[0]["definition_span"].contains("collections/btree/map.rs"));
    }
    #[test]
    fn malformed_or_non_ascii_columns_are_rejected() {
        assert!(parse("CLIENT_TEST_KIND\ttrue\textra").is_err());
        assert!(offsets("中x\n", [1, 2, 1, 3]).is_err());
        assert_eq!(offsets("abc\n", [1, 2, 1, 3]).unwrap(), (1, 2));
    }
    fn load(path: &Path) -> Vec<Record> {
        let v: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        serde_json::from_value(v["records"].clone()).unwrap()
    }
    #[test]
    #[ignore = "requires checker-evidence from the research archive"]
    fn golden_multiple_and_mutations() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../checker-evidence/test-spec/multiple-assertions/order/multiple");
        let before = load(&base.join("assert-order-source-extract.json"));
        let after = load(&base.join("assert-order-target-extract.json"));
        let target = Some("cpu::set::test::checked_eq_failure");
        assert!(compare(&before, &after, target).unwrap());
        let mut core_before = before.clone();
        for record in &mut core_before {
            if record
                .get("macro")
                .is_some_and(|name| name == "std::assert_eq")
            {
                record.insert("macro".into(), "core::assert_eq".into());
            }
        }
        assert!(compare(&core_before, &after, target).unwrap());
        for name in [
            "reverse-operands",
            "reverse-branch",
            "failure-effect",
            "drop-assertion",
        ] {
            for i in 0..2 {
                let altered =
                    load(&base.join(format!("assert-order-{name}-{i}-target-extract.json")));
                assert!(
                    !compare(&before, &altered, target).unwrap_or(false),
                    "{name}-{i}"
                );
            }
        }
    }
}
