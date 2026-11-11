//! Finite owned-array helper lowering and its independent coverage gates.
use crate::{
    Result, hash,
    ir::{self, Record},
    vir::{self, Node},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const TEMPLATE: &str = include_str!("../../../experiments/test-spec/range_incremental.rs");
const STEPS: &[u8] = include_bytes!("../../../experiments/test-spec/range_step_correspondence.rs");
const OBSERVATIONS: &str = include_str!("../../../experiments/test-spec/range_observation.rs");

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn binding(records: &[Record], place: &str) -> Result<String> {
    let found = records
        .iter()
        .filter(|r| r["kind"] == "binding" && r["place"] == place)
        .collect::<Vec<_>>();
    let [found] = found.as_slice() else {
        return Err("one source binding required".into());
    };
    if !regex::Regex::new(r"^[A-Za-z_]\w*$")?.is_match(&found["name"]) {
        return Err("unsupported source identifier".into());
    }
    Ok(found["name"].clone())
}

fn read(node: &Node) -> Option<&Node> {
    let v = node.unwrap().list()?;
    if v.first()?.atom() != Some(">") || v.get(1)?.atom() != Some("ReadPlace") {
        return None;
    }
    let place = v.get(2)?.unwrap().list()?;
    (place.first()?.atom() == Some("Place") && place.get(1)?.atom() == Some("Local"))
        .then(|| &place[2])
}

fn call_is(node: &Node, path: &str, args: &[&Node]) -> bool {
    let Some(v) = node.unwrap().list() else {
        return false;
    };
    if v.first().and_then(Node::atom) != Some(">") || v.get(1).and_then(Node::atom) != Some("Call")
    {
        return false;
    }
    let target = node
        .field("target")
        .and_then(Node::list)
        .and_then(|v| v.get(3))
        .and_then(|n| n.field("path"))
        .and_then(Node::atom);
    let actual = node.field("args").and_then(Node::list);
    target == Some(path)
        && actual.is_some_and(|values| {
            values.len() == args.len()
                && values
                    .iter()
                    .zip(args)
                    .all(|(value, expected)| read(value) == Some(*expected))
        })
}

fn logical<'a>(node: &'a Node, op: &str) -> Option<(&'a Node, &'a Node)> {
    let v = node.unwrap().list()?;
    let operator = v.get(2)?.list()?;
    if v.get(1)?.atom() != Some("Logical")
        || operator.first()?.atom() != Some("LogicalOp")
        || operator.get(1)?.atom() != Some(op)
    {
        return None;
    }
    Some((v.get(3)?, v.get(4)?))
}

fn positive_relation(node: &Node, ret: &Node, params: &[&Node], model: &str) -> bool {
    if let Some((left, right)) = logical(node, "And") {
        return positive_relation(left, ret, params, model)
            || positive_relation(right, ret, params, model);
    }
    let Some(v) = node.unwrap().list() else {
        return false;
    };
    if v.get(1).and_then(Node::atom) == Some("Block") && v.get(2).and_then(Node::list) == Some(&[])
    {
        return v
            .get(3)
            .is_some_and(|n| positive_relation(n, ret, params, model));
    }
    let op = v.get(2).and_then(Node::list);
    v.get(1).and_then(Node::atom) == Some("Binary")
        && op.is_some_and(|o| {
            o.iter().filter_map(Node::atom).collect::<Vec<_>>() == ["BinaryOp", "Eq", "Spec"]
        })
        && v.get(3)
            .is_some_and(|n| call_is(n, "vstd::std_specs::iter::IteratorSpec::remaining", &[ret]))
        && v.get(4).is_some_and(|n| call_is(n, model, params))
}

fn relation(api: &Node, model: &str) -> Result<()> {
    let ret = api
        .field("inner_ret")
        .and_then(|n| n.field("name"))
        .ok_or("return identity missing")?;
    let params = api
        .field("params")
        .and_then(Node::list)
        .ok_or("producer parameters missing")?
        .iter()
        .map(|p| p.field("name").ok_or("parameter identity missing"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if params.len() != 2 {
        return Err("producer needs two range parameters".into());
    }
    let clauses = api
        .field("ensure")
        .and_then(Node::list)
        .and_then(|n| n.get(1))
        .and_then(Node::list)
        .ok_or("postconditions missing")?;
    let none = |n: &Node| {
        call_is(
            n,
            "vstd::std_specs::iter::IteratorSpec::will_return_none",
            &[ret],
        )
    };
    let laws = |n: &Node| {
        call_is(
            n,
            "vstd::std_specs::iter::IteratorSpec::obeys_prophetic_iter_laws",
            &[ret],
        )
    };
    if !clauses.iter().any(|clause| {
        logical(clause, "Implies").is_some_and(|(guard, body)| {
            let expected_guard =
                none(guard) || logical(guard, "And").is_some_and(|(l, r)| laws(l) && none(r));
            expected_guard && positive_relation(body, ret, &params, model)
        })
    }) {
        return Err("sequence model must be bound to returned remaining() and ordered API parameters under the completion guard".into());
    }
    Ok(())
}

fn model(parent: &[Record], text: &str, crate_name: &str, cwd: &Path) -> Result<(String, Value)> {
    let functions = vir::functions(text)?;
    let producer = calls(parent)[0];
    let path = format!("{crate_name}::{}", producer["target"]);
    let api = functions
        .iter()
        .find(|f| {
            f.field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
                == Some(path.as_str())
        })
        .ok_or("producer VIR definition missing")?;
    let concrete = |f: &Node| {
        f.field("body").and_then(Node::list).is_some()
            && f.field("attrs")
                .and_then(|n| n.field("is_external_body"))
                .and_then(Node::atom)
                == Some("false")
    };
    if api.field("mode").and_then(Node::atom) != Some("Exec") || !concrete(api) {
        return Err("producer requires a concrete checked executable body".into());
    }
    let source_span = api
        .list()
        .and_then(|n| n.get(1))
        .and_then(Node::atom)
        .ok_or("producer source binding missing")?;
    if ir::span(source_span, cwd)? != ir::span(&producer["definition_span"], cwd)? {
        return Err("producer source and VIR declaration differ".into());
    }
    let mut used = Vec::new();
    vir::walk_calls(
        api.field("ensure").ok_or("producer ensures missing")?,
        &mut used,
    );
    used.sort();
    used.dedup();
    let models = functions
        .iter()
        .filter(|f| {
            let name = f
                .field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom);
            f.field("mode").and_then(Node::atom) == Some("Spec")
                && name.is_some_and(|p| used.iter().any(|u| u == p))
                && f.field("params")
                    .and_then(Node::list)
                    .is_some_and(|p| p.len() == 2)
                && f.field("inner_ret")
                    .and_then(|n| n.field("typ"))
                    .and_then(Node::list)
                    .and_then(|n| n.get(2))
                    .and_then(Node::list)
                    .is_some_and(|n| n.last().and_then(Node::atom) == Some("vstd::seq::Seq"))
        })
        .collect::<Vec<_>>();
    let [model] = models.as_slice() else {
        return Err("one two-argument sequence model required".into());
    };
    if !concrete(model) {
        return Err("sequence model requires a concrete local spec body".into());
    }
    let name = model
        .field("name")
        .and_then(|n| n.field("path"))
        .and_then(Node::atom)
        .unwrap();
    relation(api, name)?;
    let local = name
        .strip_prefix(&format!("{crate_name}::"))
        .ok_or("model must be local")?;
    if !regex::Regex::new(r"^[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*$")?.is_match(local) {
        return Err("model path cannot be emitted".into());
    }
    Ok((
        format!("crate::{local}"),
        json!({"producer":path,"model":name,
        "source_span":source_span,"origin":"Sequence-valued spec call in the producer ensures; input relation is checked at each caller"}),
    ))
}

pub struct Lowering {
    pub file: PathBuf,
    pub source: String,
    pub edits: Vec<ir::Edit>,
    pub report: Value,
    pub helper: String,
    pub replacement: String,
}

pub fn lower(
    caller: &[Record],
    parent: &[Record],
    callback: &[Record],
    cwd: &Path,
    vir: &str,
    crate_name: &str,
) -> Result<Lowering> {
    let mut report = inspect(caller, parent, callback, cwd)?;
    let approved: Value = serde_json::from_str(include_str!(
        "../../../experiments/test-spec/range-lowering-profile.json"
    ))?;
    if approved["helper_sha256"] != hash(TEMPLATE.as_bytes())?
        || approved["step_model_sha256"] != hash(STEPS)?
        || approved["observation_model_sha256"] != hash(OBSERVATIONS.as_bytes())?
    {
        return Err("helper template or event model changed; recheck their correspondence".into());
    }
    let each = calls(parent)[2];
    let (standard_file, _) = ir::span(&each["definition_span"], cwd)?;
    let standard_root = standard_file
        .ancestors()
        .find(|p| p.join("iter/traits/iterator.rs").is_file())
        .ok_or("installed core source missing")?;
    for (file, expected) in approved["stdlib_sha256"]
        .as_object()
        .ok_or("standard-library profile missing")?
    {
        if *expected != hash(&fs::read(standard_root.join(file))?)? {
            return Err("standard-library source changed; recheck ordered helper semantics".into());
        }
    }
    let (model, provenance) = model(parent, vir, crate_name, cwd)?;
    let declaration = parent
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("missing helper declaration")?;
    let body = parent
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("missing helper body")?;
    let (file, start, _, header) = crate::guidance::source(&declaration["span"], cwd)?;
    let (body_file, _, end, original_body) = crate::guidance::source(&body["span"], cwd)?;
    if file != body_file {
        return Err("helper declaration and body differ in source file".into());
    }
    let params = [
        binding(parent, "_1")?,
        binding(parent, "_2")?,
        binding(parent, "_3")?,
    ];
    let res = binding(parent, &calls(parent)[0]["destination"])?;
    let val = binding(callback, "_2")?;
    let n = regex::Regex::new(r"const\s+([A-Za-z_]\w*)\s*:\s*usize")?
        .captures(&header)
        .ok_or("one usize const parameter required")?[1]
        .to_owned();
    let producer = &calls(parent)[0]["target"];
    let short = producer.rsplit("::").next().unwrap();
    let expected_body = format!(
        "{{letmut{res}={short}(&{},&{});{}.into_iter().for_each(|{val}|assert_eq!({res}.next(),Some({val})));assert!({res}.next().is_none());}}",
        params[0], params[1], params[2]
    );
    if compact(&original_body) != expected_body {
        return Err("helper source differs from the supported ordered read/assert pattern".into());
    }
    let text = fs::read_to_string(&file)?;
    if text.contains("__client_check_") {
        return Err("generated helper local names collide".into());
    }
    let attr = regex::Regex::new(r"#\[track_caller\]\s*$")?;
    let attribute = attr
        .find(&text[..start])
        .ok_or("reviewed helper needs track_caller attribute")?;
    let (_, tail) = TEMPLATE
        .split_once("{\n    let mut i:")
        .ok_or("loop template marker missing")?;
    let body = format!("    let mut i:{tail}");
    let replacements = BTreeMap::from([
        ("res", res.as_str()),
        ("expected", params[2].as_str()),
        ("N", n.as_str()),
        ("i", "__client_check_i"),
        ("value", "__client_check_value"),
        ("matches", "__client_check_matches"),
        ("actual", "__client_check_actual"),
        ("last", "__client_check_last"),
    ]);
    let body = regex::Regex::new(r"[A-Za-z_]\w*")?.replace_all(&body, |c: &regex::Captures<'_>| {
        replacements.get(&c[0]).copied().unwrap_or(&c[0]).to_owned()
    });
    let replacement = format!(
        "vstd::prelude::verus! {{\n#[track_caller]\n{header}\nrequires {}@ =~= {model}({},{}),\n{{\nlet mut {res} = {short}(&{},&{});\n{body}\n}}\n",
        params[2], params[0], params[1], params[0], params[1]
    );
    let replacement = format!(
        "{replacement}\nmod __client_check_rule {{\n{}\nvstd::prelude::verus! {{\n{}\n{OBSERVATIONS}\n}}\n}}\n",
        include_str!("../../../experiments/test-spec/range_equality.rs"),
        std::str::from_utf8(STEPS)?
    );
    let (caller_file, _, mut edits) = ir::lower(caller, cwd)?;
    if fs::canonicalize(caller_file)? != file {
        return Err("cross-file helper lowering is unsupported".into());
    }
    edits.push(ir::Edit {
        start: attribute.start(),
        end,
        replacement: replacement.clone(),
        origin: body_record_span(parent)?,
    });
    edits.sort_by_key(|e| e.start);
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return Err("overlapping helper edits".into());
    }
    let mut changed = text;
    for edit in edits.iter().rev() {
        changed.replace_range(edit.start..edit.end, &edit.replacement);
    }
    report["lowering"] = json!({"template_sha256":approved["helper_sha256"],"event_model_sha256":approved["step_model_sha256"],"input_relation":provenance,
        "manual_input":"Shared reviewed iterator proof template and standard-library/model binding; no per-test hints or new client/API preconditions",
        "helper_preconditions_added":1,"helper_precondition_check":"Expected-array/model equality is proved at the original caller; the generic helper body is checked separately",
        "semantic_models":"Event correspondence and executable equality models are included and verified in this generated module"});
    Ok(Lowering {
        file,
        source: changed,
        edits,
        report,
        helper: function(parent)?.into(),
        replacement,
    })
}

fn body_record_span(parent: &[Record]) -> Result<String> {
    Ok(parent
        .iter()
        .find(|r| r["kind"] == "body")
        .ok_or("missing body")?["span"]
        .clone())
}

pub fn generated(plan: &Lowering, text: &str, records: &[Record]) -> Result<Value> {
    if text.matches(&plan.replacement).count() != 1 {
        return Err("generated helper changed or omitted a required assertion".into());
    }
    mir_profile(records, "helper-checked", records)?;
    let calls = calls(records);
    let count = |target: &str| calls.iter().filter(|r| r["target"] == target).count();
    if function(records)? != plan.helper
        || count("std::iter::Iterator::next") != 2
        || count("core::panicking::panic") != 2
        || count("std::option::Option::<T>::is_none") != 1
        || count("std::iter::Iterator::for_each") != 0
    {
        return Err("generated helper read/assertion coverage mismatch".into());
    }
    Ok(
        json!({"status":"matched","assertion_sites":2,"scope":"Pinned source pattern, approved template, read/assert event theorem and generated runtime MIR gates; standard-library/model binding and Rustc remain trusted"}),
    )
}

pub fn checked_models(vir: &str, helper: &str, crate_name: &str, snapshot: &Path) -> Result<Value> {
    let module = helper.rsplit_once("::").ok_or("helper module missing")?.0;
    let prefix = format!("{crate_name}::{module}::__client_check_rule::");
    let names = [
        "usize_eq",
        "range_eq",
        "option_eq",
        "comparison_bridge",
        "ordered_checks_correspond",
        "range_comparison_corresponds",
        "visible_concat",
        "expected_reads_are_unobservable",
        "effectful_evaluations_correspond",
    ];
    let functions = vir::functions(vir)?;
    let mut checked = Vec::new();
    for name in names {
        let path = format!("{prefix}{name}");
        let f = functions
            .iter()
            .find(|f| {
                f.field("name")
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    == Some(&path)
            })
            .ok_or("required semantic model not checked")?;
        let span = f
            .list()
            .and_then(|n| n.get(1))
            .and_then(Node::atom)
            .ok_or("model source missing")?;
        let (file, _) = ir::span(span, Path::new("/"))?;
        if !fs::canonicalize(file)?.starts_with(snapshot)
            || !matches!(f.field("mode").and_then(Node::atom), Some("Exec" | "Proof"))
            || f.field("body").and_then(Node::list).is_none()
            || f.field("attrs")
                .and_then(|n| n.field("is_external_body"))
                .and_then(Node::atom)
                != Some("false")
        {
            return Err("semantic model lacks a checked local body".into());
        }
        checked.push(path);
    }
    Ok(
        json!({"functions":checked,"source":"Generated snapshot","scope":"Native verification includes the executable comparison bridge and finite read/assert correspondence theorem"}),
    )
}

const RANGE: &str = "std::ops::Range<usize>";
const OPTION: &str = "std::option::Option<std::ops::Range<usize>>";
const EQ: &str = "<std::option::Option<T> as std::cmp::PartialEq>::eq";

fn calls(records: &[Record]) -> Vec<&Record> {
    records.iter().filter(|r| r["kind"] == "call").collect()
}

fn failure_edge(records: &[Record], condition: &Record, failure: &Record) -> Result<()> {
    let call = records
        .iter()
        .find(|r| r["kind"] == "term" && r["block"] == condition["block"])
        .ok_or("comparison terminator missing")?;
    let successor = regex::Regex::new(r" -> \[return: (bb\d+),")?
        .captures(&call["operation"])
        .ok_or("comparison return edge missing")?[1]
        .to_owned();
    let branch = records
        .iter()
        .find(|r| r["kind"] == "term" && r["block"] == successor)
        .ok_or("assertion branch missing")?;
    let expected = format!(
        r"^switchInt\(move {}\) -> \[0: {}, otherwise: bb\d+\]$",
        regex::escape(&condition["destination"]),
        regex::escape(&failure["block"])
    );
    if !regex::Regex::new(&expected)?.is_match(&branch["operation"]) {
        return Err("assertion must fail on the comparison's false branch".into());
    }
    Ok(())
}

fn function(records: &[Record]) -> Result<&str> {
    let names = records
        .iter()
        .filter(|r| r["kind"] == "function")
        .collect::<Vec<_>>();
    match names.as_slice() {
        [record] => Ok(&record["path"]),
        _ => Err("helper coverage needs exactly one selected function".into()),
    }
}

pub fn candidate(records: &[Record]) -> Option<&str> {
    let targets = calls(records);
    let [call] = targets.as_slice() else {
        return None;
    };
    let local_array = records.iter().any(|r| {
        r["kind"] == "source_argument"
            && r["index"] == "2"
            && r["type"].starts_with(&format!("[{RANGE}; "))
    });
    (local_array && call["generic_args"].ends_with("_usize]")).then_some(call["target"].as_str())
}

pub fn closure(parent: &[Record]) -> Result<String> {
    let name = function(parent)?;
    let each = calls(parent)
        .into_iter()
        .filter(|r| r["target"] == "std::iter::Iterator::for_each")
        .collect::<Vec<_>>();
    let [each] = each.as_slice() else {
        return Err("one for_each required".into());
    };
    let suffix = each["generic_args"]
        .split_once(&format!("::{name}::"))
        .ok_or("for_each closure must belong to the selected helper")?
        .1;
    let (closure, _) = suffix.split_once(')').ok_or("missing closure identity")?;
    if !closure.starts_with("{closure#") || !closure.ends_with('}') {
        return Err("unsupported callback identity".into());
    }
    Ok(format!("{name}::{closure}"))
}

pub fn equality(records: &[Record]) -> Result<Value> {
    let profile: Value = serde_json::from_str(include_str!(
        "../../../experiments/test-spec/range-equality-profile.json"
    ))?;
    if profile["model_sha256"]
        != hash(include_bytes!(
            "../../../experiments/test-spec/range_equality.rs"
        ))?
    {
        return Err("equality model changed; recheck model and standard MIR correspondence".into());
    }
    let instances = records
        .iter()
        .filter(|r| {
            r["kind"] == "instance"
                && r["target"] == EQ
                && r["generic_args"] == format!("[{RANGE}]")
        })
        .collect::<Vec<_>>();
    if instances.len() != 1 {
        return Err("comparison must resolve to standard Option<Range<usize>> equality".into());
    }
    let mut seen = BTreeSet::new();
    for record in records.iter().filter(|r| r["kind"] == "eq_body") {
        let approved = &profile["bodies"][&record["target"]];
        if approved["generic_args"] != record["generic_args"]
            || approved["mir_sha256"] != hash(record["body"].as_bytes())?
        {
            return Err("standard equality MIR differs from reviewed profile".into());
        }
        seen.insert(record["target"].clone());
    }
    if seen.len()
        != profile["bodies"]
            .as_object()
            .ok_or("invalid equality profile")?
            .len()
    {
        return Err("incomplete standard equality implementation chain".into());
    }
    Ok(profile)
}

fn profile_records(records: &[Record], parent: &[Record]) -> Result<Vec<Record>> {
    let producer = calls(parent)
        .first()
        .copied()
        .ok_or("helper producer call missing")?;
    let typ = parent
        .iter()
        .find(|r| r["kind"] == "local" && r["place"] == producer["destination"])
        .ok_or("helper producer result type missing")?;
    let opaque = regex::Regex::new(
        r"def_id: DefId\(\d+:\d+ ~ ([A-Za-z_]\w*\[[0-9a-f]+\])::(.+?)::\{opaque#0\}\)",
    )?;
    let identity = opaque
        .captures(&typ["type"])
        .ok_or("helper result needs a source-owned opaque type")?;
    if identity[2] != producer["target"] {
        return Err("opaque result does not belong to the called producer".into());
    }
    let qualified_producer = format!("DefId({}::{}::", &identity[1], &identity[2]);
    let qualified_helper = format!("DefId({}::{}::", &identity[1], function(parent)?);
    let quoted = regex::Regex::new(r#""(?:[^"\\]|\\.)*""#)?;
    let replace = |part: &str| {
        part.replace(&qualified_producer, "DefId($producer::")
            .replace(&qualified_helper, "DefId($helper::")
    };
    let mut normalized = ir::normalize(records, None)?;
    let block = normalized
        .iter()
        .find(|r| r["kind"] == "call" && r["target"] == producer["target"])
        .map(|r| r["block"].clone());
    for record in &mut normalized {
        if record["kind"] == "term" && block.as_ref() == record.get("block") {
            let prefix = format!("= {}::<", producer["target"]);
            if !record["operation"].contains(&prefix) {
                return Err("producer call spelling changed".into());
            }
            record.insert(
                "operation".into(),
                record["operation"].replacen(&prefix, "= $producer::<", 1),
            );
        }
        if record["kind"] == "call" && record["target"] == producer["target"] {
            record.insert("target".into(), "$producer".into());
        }
        for value in record.values_mut() {
            let mut mapped = String::new();
            let mut start = 0;
            for literal in quoted.find_iter(value) {
                mapped.push_str(&replace(&value[start..literal.start()]));
                mapped.push_str(literal.as_str());
                start = literal.end();
            }
            mapped.push_str(&replace(&value[start..]));
            *value = mapped;
        }
    }
    Ok(normalized)
}

fn mir_profile(records: &[Record], name: &str, parent: &[Record]) -> Result<()> {
    let profile: Value = serde_json::from_str(include_str!(
        "../../../experiments/test-spec/range-lowering-profile.json"
    ))?;
    let digest = hash(&serde_json::to_vec(&profile_records(records, parent)?)?)?;
    if profile["role_mir_sha256"][name] != digest {
        return Err(format!("unsupported helper runtime MIR profile: {name}").into());
    }
    Ok(())
}

pub fn configuration(
    parent: &[Record],
    callback: &[Record],
    runtime_parent: &[Record],
    runtime_callback: &[Record],
) -> Result<()> {
    if !ir::compare(runtime_parent, parent, None)?
        || !ir::compare(runtime_callback, callback, None)?
    {
        return Err("helper or closure differs between runtime and proof configurations".into());
    }
    Ok(())
}

pub fn inspect(
    caller: &[Record],
    parent: &[Record],
    callback: &[Record],
    cwd: &Path,
) -> Result<Value> {
    mir_profile(parent, "helper-source", parent)?;
    mir_profile(callback, "helper-callback", parent)?;
    let helper = function(parent)?;
    if candidate(caller) != Some(helper) || function(callback)? != closure(parent)? {
        return Err("caller/helper/callback identities do not connect".into());
    }
    let parent_calls = calls(parent);
    let names = parent_calls
        .iter()
        .skip(1)
        .map(|r| r["target"].as_str())
        .collect::<Vec<_>>();
    if names
        != [
            "std::iter::IntoIterator::into_iter",
            "std::iter::Iterator::for_each",
            "std::iter::Iterator::next",
            "std::option::Option::<T>::is_none",
            "core::panicking::panic",
        ]
    {
        return Err("unsupported helper call structure or missing terminal assertion".into());
    }
    let callback_calls = calls(callback);
    let captures = parent
        .iter()
        .filter(|r| r["kind"] == "loan" && r["borrow_kind"].starts_with("Mut "))
        .collect::<Vec<_>>();
    if captures.len() != 2
        || captures
            .iter()
            .any(|r| r["borrowed_place"] != parent_calls[0]["destination"])
        || !parent_calls[2]["generic_args"]
            .starts_with("[std::array::IntoIter<std::ops::Range<usize>, ")
    {
        return Err("owned array iteration and mutable producer capture required".into());
    }
    if callback_calls
        .iter()
        .map(|r| r["target"].as_str())
        .collect::<Vec<_>>()
        != [
            "std::iter::Iterator::next",
            "std::cmp::PartialEq::eq",
            "core::panicking::assert_failed",
        ]
    {
        return Err("unsupported callback or missing comparison assertion".into());
    }
    let operands = callback
        .iter()
        .filter(|r| r["kind"] == "assert_operand")
        .collect::<Vec<_>>();
    if operands.len() != 2
        || operands
            .iter()
            .any(|r| r["type"] != OPTION || r["macro"] != "std::assert_eq")
        || operands[0]["index"] != "0"
        || operands[1]["index"] != "1"
        || operands[0]["assertion_span"] != operands[1]["assertion_span"]
    {
        return Err("callback comparison assertion coverage incomplete".into());
    }
    let res = binding(parent, &parent_calls[0]["destination"])?;
    let val = binding(callback, "_2")?;
    for (index, expected) in [format!("{res}.next()"), format!("Some({val})")]
        .iter()
        .enumerate()
    {
        let domains = callback
            .iter()
            .filter(|r| {
                r["kind"] == "assert_domain"
                    && r["assertion_span"] == operands[index]["assertion_span"]
                    && r["index"] == index.to_string()
            })
            .collect::<Vec<_>>();
        if domains.len() != 1
            || domains[0]["direct_macro"] != "true"
            || compact(&crate::guidance::source(&operands[index]["span"], cwd)?.3) != *expected
        {
            return Err("comparison requires direct standard macro operands".into());
        }
    }
    failure_edge(parent, parent_calls[4], parent_calls[5])?;
    failure_edge(callback, callback_calls[1], callback_calls[2])?;
    let declaration = parent
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("missing helper declaration")?;
    let call = calls(caller)[0];
    if ir::span(&call["definition_span"], cwd)? != ir::span(&declaration["span"], cwd)? {
        return Err("helper call does not bind to its source declaration".into());
    }
    let arguments = caller
        .iter()
        .filter(|r| r["kind"] == "source_argument")
        .collect::<Vec<_>>();
    if arguments.len() != 3
        || arguments[0]["type"] != RANGE
        || arguments[1]["type"] != RANGE
        || arguments.iter().enumerate().any(|(i, r)| {
            r["index"] != i.to_string() || r["call_span"] != arguments[0]["call_span"]
        })
    {
        return Err("expected two ranges and one owned expected array".into());
    }
    let n = call["generic_args"]
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix("_usize]"))
        .ok_or("expected one concrete usize const argument")?
        .parse::<usize>()?;
    if arguments[2]["type"] != format!("[{RANGE}; {n}_usize]") {
        return Err("const argument and expected array length disagree".into());
    }
    let terminal = parent
        .iter()
        .filter(|r| {
            r["kind"] == "source_expr"
                && r["expression_kind"] == "if"
                && r["snippet"].starts_with("Some(\"assert!(")
        })
        .collect::<Vec<_>>();
    let [terminal] = terminal.as_slice() else {
        return Err("terminal assertion source missing".into());
    };
    let boolean = parent
        .iter()
        .filter(|r| {
            r["kind"] == "assert_operand"
                && r["assertion_span"] == terminal["span"]
                && r["macro"] == "std::assert"
                && r["index"] == "0"
                && r["type"] == "bool"
        })
        .collect::<Vec<_>>();
    if boolean.len() != 1
        || compact(&crate::guidance::source(&boolean[0]["span"], cwd)?.3)
            != format!("{res}.next().is_none()")
    {
        return Err("terminal check requires its standard assert operand".into());
    }
    // Expected ranges are moved by for_each but only their fields are read by
    // the indexed form. Neither value may have destructor effects.
    for (records, local, expected_type) in [
        (parent, "_3", format!("[{RANGE}; {n}_usize]")),
        (callback, "_2", RANGE.to_owned()),
    ] {
        let facts = records
            .iter()
            .filter(|r| r["kind"] == "type_fact" && r["place"] == local)
            .collect::<Vec<_>>();
        let [fact] = facts.as_slice() else {
            return Err("helper requires compiler destructor facts".into());
        };
        // The parent is generic in N; its exact type is checked below by its
        // parameter binding, while the caller supplies the concrete length.
        if fact["needs_drop"] != "false"
            || (local == "_2" && fact["type"] != expected_type)
            || (local == "_3" && !fact["type"].starts_with(&format!("[{RANGE}; ")))
        {
            return Err(
                "expected value must be an owned destructor-free standard range array".into(),
            );
        }
    }
    let equality = equality(callback)?;
    Ok(
        json!({"status":"traced","caller":function(caller)?,"helper":helper,
        "callback":function(callback)?,"const_argument":n,"arguments":arguments,
        "assertions":[{"span":operands[0]["assertion_span"],"context":"callback","instances":n},
            {"span":terminal["span"],"context":"helper","instances":1}],
        "standard_equality":equality,
        "evaluation_conditions":{"expected_values":"owned standard Range<usize> array; compiler confirms no destructor effects", "field_reads":"immutable usize fields, loop bounds proved by Verus", "borrowing":"source mutable capture and generated reborrows have different intervals; exact MIR/loan profiles and caller correspondence required", "observation_scope":"producer/read/check effects, including failure and partial execution; pure expected-value loads projected out"},
        "scope":"Source and compiler coverage; source transformation and proof remain independent gates"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires HELPER_PROFILE_AUDIT with original archive and current extracted records"]
    fn source_bound_profile_audit() {
        let root = PathBuf::from(std::env::var("HELPER_PROFILE_AUDIT").unwrap());
        let read = |name: &str| -> Vec<Record> {
            let value: Value = serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
            serde_json::from_value(value["records"].clone()).unwrap()
        };
        let old_parent = read("old-helper-source.json");
        let parent = read("new-helper-source.json");
        let profile: Value = serde_json::from_str(include_str!(
            "../../../experiments/test-spec/range-lowering-profile.json"
        ))
        .unwrap();
        let mut results = serde_json::Map::new();
        for (name, filename) in [
            ("helper-source", "helper-source"),
            ("helper-callback", "helper-callback"),
            ("helper-checked", "helper-checked"),
        ] {
            let old = read(&format!("old-{filename}.json"));
            assert_eq!(
                hash(&serde_json::to_vec(&ir::normalize(&old, None).unwrap()).unwrap()).unwrap(),
                profile["rust_mir_sha256"][name]
            );
            let a = profile_records(&old, &old_parent).unwrap();
            let current = root.join(format!("new-{filename}.json")).exists();
            if current {
                let b = profile_records(&read(&format!("new-{filename}.json")), &parent).unwrap();
                crate::write_json(&root.join(format!("new-{name}-normalized.json")), &b).unwrap();
                assert_eq!(a, b);
            }
            crate::write_json(&root.join(format!("old-{name}-normalized.json")), &a).unwrap();
            results.insert(name.into(),json!({"matches":if current {json!(true)} else {Value::Null},"baseline_only":!current,"baseline_role_sha256":hash(&serde_json::to_vec(&a).unwrap()).unwrap()}));
        }
        let mut wrong = parent.clone();
        wrong
            .iter_mut()
            .find(|r| r["kind"] == "call")
            .unwrap()
            .insert("target".into(), "unrelated_producer".into());
        assert!(profile_records(&wrong, &wrong).is_err());
        let mut literal = parent.clone();
        literal
            .iter_mut()
            .rev()
            .find(|r| r["kind"] == "call")
            .unwrap()
            .insert(
                "arguments".into(),
                "[const \"DefId(range_use_runtime[da6d]::range_difference::{opaque#0})\"]".into(),
            );
        let normalized = profile_records(&literal, &parent).unwrap();
        assert!(
            normalized.iter().any(|r| r.get("arguments").is_some_and(
                |s| s.contains("DefId(range_use_runtime[da6d]::range_difference::{opaque#0})")
            ))
        );
        assert!(mir_profile(&literal, "helper-source", &parent).is_err());
        let mut branch = parent.clone();
        let term = branch
            .iter_mut()
            .find(|r| r["kind"] == "term" && r["operation"].starts_with("switchInt("))
            .unwrap();
        term.insert(
            "operation".into(),
            term["operation"].replacen("[0:", "[1:", 1),
        );
        assert!(mir_profile(&branch, "helper-source", &parent).is_err());
        crate::write_json(&root.join("audit.json"), &Value::Object(results)).unwrap();
    }

    #[test]
    #[ignore = "requires a captured nine-test CLI run"]
    fn generates_one_shared_helper_without_changing_callers() {
        let root = PathBuf::from(std::env::var("SPEC_HELPER_LOWERING").unwrap()).join("target-0");
        let context: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(root.join("context.json")).unwrap()).unwrap();
        let vir = fs::read_to_string(root.join("baseline-vir/crate.vir")).unwrap();
        let crate_name = &context
            .args
            .windows(2)
            .find(|a| a[0] == "--crate-name")
            .unwrap()[1];
        let package = Path::new(context.env.get("CARGO_MANIFEST_DIR").unwrap());
        let mut provenance = Value::Null;
        for i in 0..9 {
            let case = root.join(format!("sample-{i}"));
            let read = |name: &str| -> Vec<Record> {
                let v: Value =
                    serde_json::from_slice(&fs::read(case.join(format!("{name}.json"))).unwrap())
                        .unwrap();
                serde_json::from_value(v["records"].clone()).unwrap()
            };
            let caller = read("original");
            let plan = lower(
                &caller,
                &read("helper-source"),
                &read("helper-callback"),
                &context.cwd,
                &vir,
                crate_name,
            )
            .unwrap();
            let parent = read("helper-source");
            let callback = read("helper-callback");
            configuration(
                &parent,
                &callback,
                &read("helper-runtime"),
                &read("callback-runtime"),
            )
            .unwrap();
            if let Ok(control) = std::env::var("SPEC_HELPER_CFG_CONTROL") {
                let read_control = |name: &str| -> Vec<Record> {
                    let value: Value = serde_json::from_slice(
                        &fs::read(Path::new(&control).join(format!("{name}.json"))).unwrap(),
                    )
                    .unwrap();
                    serde_json::from_value(value["records"].clone()).unwrap()
                };
                assert!(
                    configuration(
                        &read_control("proof"),
                        &callback,
                        &read_control("runtime"),
                        &callback
                    )
                    .is_err()
                );
            }
            for kind in ["stmt", "term"] {
                let mut changed = parent.clone();
                let record = changed.iter_mut().find(|r| r["kind"] == kind).unwrap();
                record.insert("operation".into(), "changed runtime evaluation".into());
                assert!(configuration(&parent, &callback, &changed, &callback).is_err());
            }
            for drop_fact in ["true", "unknown"] {
                let mut changed = parent.clone();
                changed
                    .iter_mut()
                    .find(|r| r["kind"] == "type_fact" && r["place"] == "_3")
                    .unwrap()
                    .insert("needs_drop".into(), drop_fact.into());
                assert!(inspect(&caller, &changed, &callback, &context.cwd).is_err());
            }
            provenance = plan.report["lowering"]["input_relation"].clone();
            assert_eq!(
                plan.source,
                fs::read_to_string(
                    case.join("package")
                        .join(plan.file.strip_prefix(package).unwrap())
                )
                .unwrap()
            );
            let body = caller.iter().find(|r| r["kind"] == "body").unwrap();
            let (_, _, _, text) = crate::guidance::source(&body["span"], &context.cwd).unwrap();
            assert!(plan.source.contains(&text));
            for statement in [
                "assert!(__client_check_matches);",
                "assert!(__client_check_last.is_none());",
            ] {
                let omitted = plan.source.replacen(statement, "", 1);
                assert_ne!(omitted, plan.source);
                assert!(
                    generated(&plan, &omitted, &[])
                        .unwrap_err()
                        .to_string()
                        .contains("omitted")
                );
            }
            if case.join("helper-checked.json").is_file() {
                assert!(generated(&plan, &plan.source, &read("helper-checked")).is_ok());
                for kind in ["stmt", "term"] {
                    let mut changed = read("helper-checked");
                    changed
                        .iter_mut()
                        .find(|r| r["kind"] == kind)
                        .unwrap()
                        .insert(
                            "operation".into(),
                            "changed generated evaluation order".into(),
                        );
                    assert!(generated(&plan, &plan.source, &changed).is_err());
                }
            }
            if case.join("check-vir/crate.vir").is_file() {
                let checked = fs::read_to_string(case.join("check-vir/crate.vir")).unwrap();
                assert!(
                    checked_models(&checked, &plan.helper, crate_name, &case.join("package"))
                        .is_ok()
                );
                assert!(
                    checked_models(
                        &checked.replace(
                            "::__client_check_rule::ordered_checks_correspond",
                            "::__client_check_rule::omitted"
                        ),
                        &plan.helper,
                        crate_name,
                        &case.join("package")
                    )
                    .is_err()
                );
            }
        }
        let mut functions = vir::functions(&vir).unwrap();
        let api = functions
            .iter_mut()
            .find(|f| {
                f.field("name")
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    == provenance["producer"].as_str()
            })
            .unwrap();
        let model = provenance["model"].as_str().unwrap();
        assert!(relation(api, model).is_ok());
        fn swap_model_arguments(n: &mut Node, model: &str) {
            let selected = n.list().is_some_and(|v| {
                v.first().and_then(Node::atom) == Some(">")
                    && v.get(1).and_then(Node::atom) == Some("Call")
            }) && n
                .field("target")
                .and_then(Node::list)
                .and_then(|v| v.get(3))
                .and_then(|v| v.field("path"))
                .and_then(Node::atom)
                == Some(model);
            if let Node::List(values) = n {
                if selected {
                    let pos = values
                        .iter()
                        .position(|v| v.atom() == Some(":args"))
                        .unwrap();
                    let Node::List(args) = &mut values[pos + 1] else {
                        panic!("call args")
                    };
                    args.swap(0, 1);
                } else {
                    for child in values {
                        swap_model_arguments(child, model);
                    }
                }
            }
        }
        swap_model_arguments(api, model);
        assert!(relation(api, model).is_err());
    }
    #[test]
    #[ignore = "requires live Rustc helper extraction"]
    fn real_helper_coverage_rejects_omissions_and_wrong_equality() {
        let root = std::env::var("SPEC_HELPER_RECORDS").unwrap();
        let read = |name: &str| {
            ir::parse(&std::fs::read_to_string(format!("{root}/{name}.stderr")).unwrap()).unwrap()
        };
        let parent = read("helper");
        let callback = read("callback");
        for i in 0..9 {
            let caller = read(&i.to_string());
            assert!(inspect(&caller, &parent, &callback, Path::new("/tmp")).is_ok());
            let mut omitted = callback.clone();
            omitted.retain(|r| r["kind"] != "assert_operand");
            assert!(inspect(&caller, &parent, &omitted, Path::new("/tmp")).is_err());
            let mut reversed = parent.clone();
            let branch = reversed
                .iter_mut()
                .find(|r| r["kind"] == "term" && r["operation"].starts_with("switchInt("))
                .unwrap();
            branch.insert(
                "operation".into(),
                branch["operation"].replacen("[0:", "[1:", 1),
            );
            assert!(inspect(&caller, &reversed, &callback, Path::new("/tmp")).is_err());
            let mut custom = parent.clone();
            custom
                .iter_mut()
                .find(|r| r["kind"] == "assert_operand")
                .unwrap()
                .insert("macro".into(), "custom::assert".into());
            assert!(inspect(&caller, &custom, &callback, Path::new("/tmp")).is_err());
            let mut missing_terminal = parent.clone();
            missing_terminal
                .retain(|r| r["kind"] != "call" || r["target"] != "core::panicking::panic");
            assert!(inspect(&caller, &missing_terminal, &callback, Path::new("/tmp")).is_err());
        }
        let mut wrong = callback.clone();
        wrong
            .iter_mut()
            .find(|r| r["kind"] == "eq_body")
            .unwrap()
            .insert("body".into(), "return true".into());
        assert!(equality(&wrong).is_err());
    }
}
