//! Source-bound linkage from compiler-resolved definitions to native VIR interfaces.
use crate::{
    Result,
    capture::Invocation,
    contracts, hash, ir, project, resource, runner,
    vir::{self, Node},
    write_json,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

// Rustc local indices and expansion hygiene IDs are process-local. The debug
// DefId also carries the complete crate-disambiguated DefPath, which we retain.
pub(crate) fn stable(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();
    while !rest.is_empty() {
        if rest.starts_with("DefId(")
            && let Some(end) = rest.find(" ~ ")
        {
            let numbers = &rest[6..end];
            if numbers.chars().all(|c| c.is_ascii_digit() || c == ':') {
                out.push_str("DefId(");
                rest = &rest[end + 3..];
                continue;
            }
        }
        if rest.starts_with("(#")
            && let Some(end) = rest.find(')')
            && rest[2..end].chars().all(|c| c.is_ascii_digit())
        {
            rest = &rest[end + 1..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn field_path(f: &Node) -> Option<&str> {
    f.field("name")?.field("path")?.atom()
}
fn type_key(t: &Node) -> Option<String> {
    let v = t.unwrap().list()?;
    match v.get(1)?.atom()? {
        "Bool" => Some("bool".into()),
        "Int" => {
            let range = v.get(2)?.list()?;
            Some(match range.get(1)?.atom()? {
                "USize" => "usize".into(),
                "ISize" => "isize".into(),
                "U" => format!("u{}", range.get(2)?.atom()?),
                "I" => format!("i{}", range.get(2)?.atom()?),
                _ => return None,
            })
        }
        "Primitive" => match v.get(2)?.list()?.get(1)?.atom()? {
            "Slice" => Some(format!("slice<{}>", type_key(v.get(3)?.list()?.first()?)?)),
            "Str" => Some("str".into()),
            _ => None,
        },
        "Decorate" => {
            let decoration = v.get(2)?.list()?.get(1)?.atom()?;
            match decoration {
                "Never" => Some("!".into()),
                "Ref" => Some(format!("&{}", type_key(v.get(4)?)?)),
                "MutRef" => Some(format!("&mut {}", type_key(v.get(4)?)?)),
                _ => None,
            }
        }
        "Datatype" => {
            let kind = v.get(2)?.list()?;
            let args = v
                .get(3)?
                .list()?
                .iter()
                .map(type_key)
                .collect::<Option<Vec<_>>>()?
                .join(",");
            if kind.get(1)?.atom() == Some("Tuple") {
                Some(format!("tuple[{args}]"))
            } else if kind.get(1)?.atom() == Some("Path") {
                Some(format!("{}<{args}>", kind.get(2)?.atom()?))
            } else {
                None
            }
        }
        "TypParam" => Some(format!("param:{}", v.get(2)?.atom()?)),
        _ => None,
    }
}
// The keys encode compiler types, not source spelling. Only input occurrences
// infer substitutions; a return type must agree with the same substitution.
fn match_type(
    pattern: &str,
    actual: &str,
    vars: &mut BTreeMap<String, String>,
    bind: bool,
) -> bool {
    if let Some(param) = pattern.strip_prefix("param:") {
        return match vars.get(param) {
            Some(value) => value == actual,
            None if bind => {
                vars.insert(param.into(), actual.into());
                true
            }
            None => pattern == actual,
        };
    }
    for prefix in ["&mut ", "&"] {
        if let Some(inner) = pattern.strip_prefix(prefix) {
            if prefix == "&" && actual.starts_with("&mut ") {
                return false;
            }
            return actual
                .strip_prefix(prefix)
                .is_some_and(|value| match_type(inner, value, vars, bind));
        }
    }
    fn parts(key: &str) -> Option<(&str, Vec<&str>)> {
        let start = key.find(['<', '['])?;
        let mut stack = Vec::new();
        let mut fields = Vec::new();
        let mut field = start + 1;
        for (i, c) in key.char_indices().skip_while(|(i, _)| *i < start) {
            match c {
                '<' | '[' => stack.push(c),
                '>' | ']' => {
                    let open = stack.pop()?;
                    if (open, c) != ('<', '>') && (open, c) != ('[', ']') {
                        return None;
                    }
                    if stack.is_empty() {
                        if i + 1 != key.len() || (field == i && field != start + 1) {
                            return None;
                        }
                        if field != i {
                            fields.push(&key[field..i]);
                        }
                    }
                }
                ',' if stack.len() == 1 => {
                    if field == i {
                        return None;
                    }
                    fields.push(&key[field..i]);
                    field = i + 1;
                }
                _ => {}
            }
        }
        stack.is_empty().then_some((&key[..=start], fields))
    }
    match (parts(pattern), parts(actual)) {
        (Some((p, ps)), Some((a, xs))) => {
            p == a
                && ps.len() == xs.len()
                && ps.iter().zip(xs).all(|(p, a)| match_type(p, a, vars, bind))
        }
        (None, None) => pattern == actual,
        _ => false,
    }
}
fn signature_substitution(f: &Node, types: &[String]) -> Option<BTreeMap<String, String>> {
    if types.len() != 5 {
        return None;
    }
    let inputs = f.field("params").and_then(Node::list).and_then(|p| {
        p.iter()
            .map(|p| type_key(p.field("typ")?))
            .collect::<Option<Vec<_>>>()
    });
    let output = f
        .field("outer_ret")
        .and_then(|r| r.field("typ"))
        .and_then(type_key);
    let (Some(inputs), Some(output)) = (inputs, output) else {
        return None;
    };
    let actual = if types[3].is_empty() {
        vec![]
    } else {
        types[3].split('|').collect::<Vec<_>>()
    };
    let mut vars = BTreeMap::new();
    (inputs.len() == actual.len()
        && inputs
            .iter()
            .zip(actual)
            .all(|(p, a)| match_type(p, a, &mut vars, true))
        && match_type(&output, &types[4], &mut vars, false))
    .then_some(vars)
}
fn compatible(f: &Node, types: &[String]) -> bool {
    signature_substitution(f, types).is_some()
}
fn source_matches(f: &Node, binding: &[String], cwd: &Path) -> bool {
    if f.field("proxy").is_some_and(|p| p.atom() != Some("None")) {
        return true;
    }
    let pos = f
        .list()
        .and_then(|v| v.get(1))
        .and_then(Node::atom)
        .and_then(|s| ir::span(s, cwd).ok());
    let Some((p, a)) = pos else { return false };
    [7, 10].into_iter().any(|i| {
        binding.get(i).is_some_and(|s| {
            s.split('|').any(|s| {
                ir::span(s, cwd).ok().is_some_and(|(q, b)| {
                    p == q
                        && (a[..2] == b[..2]
                            || (i == 10
                                && binding[7] != *s
                                && b[..2] <= a[..2]
                                && a[2..] <= b[2..]))
                })
            })
        })
    })
}
pub(crate) fn projected_interface<'a>(
    runtime: &'a Node,
    functions: &'a [Node],
    cwd: &Path,
) -> Result<&'a Node> {
    let source = |f: &Node| {
        f.list()
            .and_then(|v| v.get(1))
            .and_then(Node::atom)
            .and_then(|s| ir::span(s, cwd).ok())
    };
    let position = source(runtime).ok_or("runtime interface has no source identity")?;
    let same_source = functions
        .iter()
        .filter(|f| source(f).as_ref() == Some(&position))
        .collect::<Vec<_>>();
    let resolved = resource::resolve_signature(&same_source)?;
    if field_path(resolved) == field_path(runtime) {
        return Err("no distinct verification interface".into());
    }
    Ok(resolved)
}

fn verification_interface<'a>(
    runtime: &'a Node,
    call: &Value,
    binding: &[String],
    owners: &[Vec<String>],
    functions: &'a [Node],
    index: &BTreeMap<String, Vec<usize>>,
    cwd: &Path,
) -> Option<&'a Node> {
    let runtime_path = field_path(runtime)?;
    if let Some(i) = source_callsite_interface(call, binding, owners, functions, index) {
        let candidate = &functions[i];
        if field_path(candidate) != Some(runtime_path) {
            return Some(candidate);
        }
    }
    projected_interface(runtime, functions, cwd)
        .ok()
        .filter(|candidate| field_path(candidate) != Some(runtime_path))
}

fn runtime_call_path(path: &str) -> String {
    if let Some((prefix, verified)) = path.rsplit_once("::_VERUS_VERIFIED_") {
        format!("{prefix}::{verified}")
    } else {
        path.to_owned()
    }
}

fn source_callsite_interface(
    call: &Value,
    binding: &[String],
    owners: &[Vec<String>],
    functions: &[Node],
    index: &BTreeMap<String, Vec<usize>>,
) -> Option<usize> {
    let caller = owners.iter().find(|owner| {
        owner.len() == 4 && stable(&owner[1]) == stable(call["owner"].as_str().unwrap_or(""))
    })?;
    let span = call["span"].as_str()?;
    let expected = stable(&binding[6]);
    // Verus may put the checked body on a `_VERUS_VERIFIED_` peer of the
    // Rustc runtime owner. Restrict peer lookup to the same owner source span.
    let callers = owners
        .iter()
        .filter(|owner| {
            owner.len() == 4
                && stable(&owner[3]) == stable(&caller[3])
                && runtime_call_path(&owner[2]) == caller[2]
        })
        .filter_map(|owner| index.get(&owner[2]))
        .flatten()
        .collect::<Vec<_>>();
    let mut targets = Vec::new();
    for caller_index in callers {
        if let Some(body) = functions[*caller_index].field("body") {
            let mut sites = Vec::new();
            vir::call_sites(body, &mut sites);
            targets.extend(sites.into_iter().filter_map(|(site_span, path)| {
                (stable(&site_span) == stable(span) && runtime_call_path(&path) == expected)
                    .then_some(path)
            }));
        }
    }
    let [path] = targets.as_slice() else {
        return None;
    };
    let candidates = index.get(path)?;
    let [candidate] = candidates.as_slice() else {
        return None;
    };
    (functions[*candidate].field("mode").and_then(Node::atom) == Some("Exec")).then_some(*candidate)
}

fn caller_vir_body_status(
    call: &Value,
    owners: &[Vec<String>],
    functions: &[Node],
    index: &BTreeMap<String, Vec<usize>>,
) -> &'static str {
    let Some(caller) = owners.iter().find(|owner| {
        owner.len() == 4 && stable(&owner[1]) == stable(call["owner"].as_str().unwrap_or(""))
    }) else {
        return "owner_not_found";
    };
    let mut candidates = owners
        .iter()
        .filter(|owner| {
            owner.len() == 4
                && stable(&owner[3]) == stable(&caller[3])
                && runtime_call_path(&owner[2]) == caller[2]
        })
        .filter_map(|owner| index.get(&owner[2]))
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.is_empty() {
        return "no_native_vir_owner";
    }
    let external_body = candidates.iter().any(|i| {
        functions[*i]
            .field("attrs")
            .and_then(|attrs| attrs.field("is_external_body"))
            .and_then(Node::atom)
            == Some("true")
    });
    let has_body = candidates.iter().any(|i| {
        functions[*i]
            .field("body")
            .is_some_and(|body| body.atom() != Some("None"))
    });
    match (external_body, has_body) {
        (true, true) => "mixed_external_and_body",
        (true, false) => "external_body",
        (false, true) => "body_present",
        (false, false) => "declaration_only",
    }
}

fn caller_vir_callsite_status(
    call: &Value,
    binding: Option<&Vec<String>>,
    owners: &[Vec<String>],
    functions: &[Node],
    index: &BTreeMap<String, Vec<usize>>,
) -> &'static str {
    let Some(binding) = binding else {
        return "compiler_binding_missing";
    };
    let Some(caller) = owners.iter().find(|owner| {
        owner.len() == 4 && stable(&owner[1]) == stable(call["owner"].as_str().unwrap_or(""))
    }) else {
        return "no_native_vir_owner";
    };
    let callers = owners
        .iter()
        .filter(|owner| {
            owner.len() == 4
                && stable(&owner[3]) == stable(&caller[3])
                && runtime_call_path(&owner[2]) == caller[2]
        })
        .filter_map(|owner| index.get(&owner[2]))
        .flatten()
        .collect::<Vec<_>>();
    let expected_span = stable(call["span"].as_str().unwrap_or(""));
    let expected_path = runtime_call_path(&binding[6]);
    let mut has_body = false;
    for caller in callers {
        if let Some(body) = functions[*caller].field("body")
            && body.atom() != Some("None")
        {
            has_body = true;
            let mut sites = Vec::new();
            vir::call_sites(body, &mut sites);
            if sites.into_iter().any(|(span, path)| {
                stable(&span) == expected_span && runtime_call_path(&path) == expected_path
            }) {
                return "exact_callsite_present";
            }
        }
    }
    if has_body {
        "no_exact_callsite"
    } else {
        "caller_body_absent"
    }
}

fn associate(
    call: &Value,
    binding: Option<&Vec<String>>,
    types: Option<&Vec<String>>,
    owners: &[Vec<String>],
    functions: &[Node],
    index: &BTreeMap<String, Vec<usize>>,
    cwd: &Path,
) -> Value {
    let Some(b) = binding else {
        return json!({"status":"unresolved_compiler_target"});
    };
    if b.len() != 11
        || stable(&b[1]) != stable(call["owner"].as_str().unwrap_or(""))
        || b[2] != call["block"]
        || stable(&b[3]) != stable(call["target_id"].as_str().unwrap_or(""))
    {
        return json!({"status":"compiler_identity_mismatch"});
    }
    let candidates = types
        .into_iter()
        .flat_map(|t| index.get(&b[6]).into_iter().flatten().map(move |i| (t, i)))
        .filter_map(|(t, i)| {
            let f = &functions[*i];
            (field_path(f) == Some(b[6].as_str())
                && f.field("mode").and_then(Node::atom) == Some("Exec")
                && compatible(f, t)
                && source_matches(f, b, cwd))
            .then_some(*i)
        })
        .collect::<Vec<_>>();
    let (i, matched_at_callsite) = match candidates.as_slice() {
        [i] => (*i, false),
        [] | [_, ..] => match source_callsite_interface(call, b, owners, functions, index) {
            Some(i) => (i, true),
            None => {
                return json!({"status":if types.is_none(){"unsupported_signature_type"}else if candidates.is_empty(){"no_matching_interface"}else{"ambiguous_interface"},"compiler_binding":b,"candidate_count":candidates.len()});
            }
        },
    };
    let runtime = &functions[i];
    let is_stub = runtime
        .field("require")
        .and_then(Node::list)
        .is_some_and(|r| {
            r.iter().any(|v| {
                v.unwrap()
                    == &Node::List(vec![
                        Node::Atom(">".into()),
                        Node::Atom("Const".into()),
                        Node::List(vec![
                            Node::Atom("Constant".into()),
                            Node::Atom("Bool".into()),
                            Node::Atom("false".into()),
                        ]),
                    ])
            })
        });
    let f = if is_stub {
        match verification_interface(runtime, call, b, owners, functions, index, cwd) {
            Some(f) => f,
            None => {
                return json!({"status":"unusable_false_precondition","interface":field_path(runtime),"compiler_binding":b,
                "reason":"No exact VIR callsite or unique source-bound verification interface"});
            }
        }
    } else {
        runtime
    };
    let trusted = f
        .field("attrs")
        .and_then(|a| a.field("is_external_body"))
        .and_then(Node::atom)
        == Some("true");
    let has_body = f.field("body").is_some_and(|b| b.atom() != Some("None"));
    let fields = f.unwrap().list().unwrap();
    let mut metadata = BTreeMap::new();
    for pair in fields[1..].as_chunks::<2>().0 {
        if pair[0].atom() != Some(":body") {
            metadata.insert(
                pair[0].atom().unwrap_or("invalid"),
                contracts::render(&pair[1]),
            );
        }
    }
    // Keep trait declarations alongside implementations; consumers must preserve
    // their substitutions and requirements when building the verification call.
    let trait_path = f
        .field("kind")
        .and_then(|k| k.field("method"))
        .and_then(|m| m.field("path"))
        .and_then(Node::atom);
    let trait_contracts = trait_path
        .into_iter()
        .flat_map(|p| index.get(p).into_iter().flatten())
        .map(|i| contracts::contract(&functions[*i], cwd, &mut Default::default()))
        .collect::<Vec<_>>();
    let mut result = json!({"status":"linked","interface":field_path(f),"runtime_interface":field_path(runtime),"ghost_projection":is_stub,"compiler_binding":b,"runtime_signature":types,"input_type_substitution":types.and_then(|t|signature_substitution(runtime,t)),
        "trust":if trusted || f.field("proxy").is_some_and(|p|p.atom()!=Some("None")){"external_trusted"}else if !has_body{"imported_contract"}else{"local_body_present"},
        "basis":if matched_at_callsite{"Compiler caller identity, exact MIR/VIR source span and matching runtime path recovered from the native VIR call target; capture fingerprints checked"}else{"Compiler DefPath and Instance resolution, exact native path, runtime signature and definition or macro callsite identity; capture fingerprints checked"},
        "projection_basis":if is_stub{Some("Unique nonexternal Exec interface at the same source position, with equal type parameters/bounds and equal runtime-erased argument/return types; SDK macro semantics remain trusted")}else{None},
        "native_interface_fields":metadata,"trait_declaration":trait_path,"trait_contracts":trait_contracts,
        "contract":contracts::contract(f,cwd,&mut Default::default())});
    if matched_at_callsite {
        result["association_method"] = json!("exact_source_span_and_vir_call_target");
    }
    result
}

pub fn run(capture: &Path, config: &Path, out: &Path) -> Result<()> {
    project::validate(capture, config)?;
    fs::create_dir(out)?;
    let out = fs::canonicalize(out)?;
    let report: Value = serde_json::from_slice(&fs::read(capture.join("report.json"))?)?;
    let mut profiles = Vec::new();
    let mut evidence = BTreeMap::<std::path::PathBuf, String>::new();
    let mut fingerprint = |p: &Path| -> Result<()> {
        evidence.insert(fs::canonicalize(p)?, hash(&fs::read(p)?)?);
        Ok(())
    };
    for name in ["report.json", "configuration.json", "inputs.json"] {
        fingerprint(&capture.join(name))?;
    }
    fs::copy(std::env::current_exe()?, out.join("link-tool"))?;
    fs::write(out.join("link-rule.rs"), include_str!("link.rs"))?;
    fingerprint(&out.join("link-tool"))?;
    fingerprint(&out.join("link-rule.rs"))?;
    for profile in report["profiles"]
        .as_array()
        .ok_or("capture profiles missing")?
    {
        let name = profile["profile"].as_str().ok_or("profile name missing")?;
        let mut linked_units = Vec::new();
        for unit in profile["units"].as_array().ok_or("capture units missing")? {
            let original = capture
                .join(name)
                .join(unit["evidence"].as_str().ok_or("unit path missing")?);
            let base = original.parent().ok_or("unit directory missing")?;
            let inv: Invocation = serde_json::from_slice(&fs::read(base.join("context.json"))?)?;
            let dest = out
                .join(name)
                .join(base.file_name().ok_or("unit name missing")?);
            fs::create_dir_all(&dest)?;
            let probe = runner::build_probe(&inv, &dest)?.ok_or("link probe build failed")?;
            let original_compiler: Value =
                serde_json::from_slice(&fs::read(base.join("compiler-rule.json"))?)?;
            let current_compiler: Value =
                serde_json::from_slice(&fs::read(dest.join("compiler-rule.json"))?)?;
            if original_compiler["version"] != current_compiler["version"] {
                return Err("compiler version differs from capture".into());
            }
            let mut context = inv.clone();
            context
                .env
                .insert("CLIENT_PROJECT_INVENTORY".into(), "1".into());
            context.env.insert("CLIENT_PROJECT_LINK".into(), "1".into());
            let args = runner::clean(&inv.args, &dest.join("compiler-output"), None, &inv.cwd)?;
            runner::extract_context(&probe, &context, &args, "*", &dest, "binding", true)?;
            let stderr = fs::read_to_string(dest.join("binding.stderr"))?;
            let mut bindings = BTreeMap::new();
            let mut owners = Vec::new();
            let mut types = BTreeMap::new();
            for line in stderr.lines() {
                let row = line.split('\t').map(str::to_owned).collect::<Vec<_>>();
                if row.first().map(String::as_str) == Some("LINK_OWNER") {
                    if row.len() != 4 {
                        return Err("malformed compiler owner identity".into());
                    }
                    owners.push(row);
                } else if row.first().map(String::as_str) == Some("LINK_CALL") {
                    if row.len() != 11 {
                        return Err("malformed compiler binding".into());
                    }
                    if bindings
                        .insert((stable(&row[1]), row[2].clone()), row)
                        .is_some()
                    {
                        return Err("duplicate compiler call binding".into());
                    }
                } else if row.first().map(String::as_str) == Some("LINK_TYPES") {
                    if row.len() != 5 {
                        return Err("malformed compiler signature".into());
                    }
                    types.insert((stable(&row[1]), row[2].clone()), row);
                }
            }
            let native: Value =
                serde_json::from_slice(&fs::read(base.join("verification-input.json"))?)?;
            let vir_path = Path::new(native["vir"].as_str().ok_or("native VIR path missing")?);
            if native["sha256"] != hash(&fs::read(vir_path)?)? {
                return Err("native VIR identity differs from capture".into());
            }
            for p in [
                base.join("context.json"),
                base.join("compiler-rule.json"),
                base.join("verification-input.json"),
                original.join("inventory-records.json"),
                vir_path.to_path_buf(),
                dest.join("binding.stderr"),
                dest.join("probe.rs"),
                dest.join("compiler-rule.json"),
            ] {
                fingerprint(&p)?;
            }
            let driver = Path::new(
                native["driver_source"]
                    .as_str()
                    .ok_or("missing driver source")?,
            );
            let compiler_src = driver.parent().ok_or("missing compiler source directory")?;
            fingerprint(&compiler_src.join("rust_to_vir_base.rs"))?;
            fingerprint(
                &compiler_src
                    .parent()
                    .and_then(Path::parent)
                    .ok_or("missing SDK source directory")?
                    .join("vir/src/def.rs"),
            )?;
            let text = fs::read_to_string(vir_path)?;
            let functions = vir::functions(&text)?;
            let mut index = BTreeMap::<String, Vec<usize>>::new();
            for (i, f) in functions.iter().enumerate() {
                if let Some(path) = field_path(f) {
                    index.entry(path.into()).or_default().push(i);
                }
            }
            write_json(&dest.join("owners.json"), &owners)?;
            fingerprint(&dest.join("owners.json"))?;
            let records: Value =
                serde_json::from_slice(&fs::read(original.join("inventory-records.json"))?)?;
            let replay = project::inventory(&stderr)?;
            let calls = |v: &Value| {
                let mut rows = v["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["kind"] == "mir_call")
                    .map(|r| stable(&r.to_string()))
                    .collect::<Vec<_>>();
                rows.sort();
                rows
            };
            if calls(&records) != calls(&replay) {
                return Err("compiler replay call inventory differs from capture".into());
            }
            let mut rows = Vec::new();
            for call in records["records"]
                .as_array()
                .ok_or("compiler inventory missing")?
                .iter()
                .filter(|r| r["kind"] == "mir_call")
            {
                let key = (
                    stable(call["owner"].as_str().unwrap()),
                    call["block"].as_str().unwrap().to_owned(),
                );
                let binding = bindings.get(&key);
                rows.push(json!({"call":call,"caller_vir_body_status":caller_vir_body_status(call,&owners,&functions,&index),"caller_vir_callsite_status":caller_vir_callsite_status(call,binding,&owners,&functions,&index),"association":associate(call,binding,types.get(&key),&owners,&functions,&index,&inv.cwd)}));
            }
            write_json(&dest.join("relations.json"), &rows)?;
            fingerprint(&dest.join("relations.json"))?;
            let counts = rows
                .iter()
                .fold(BTreeMap::<String, usize>::new(), |mut c, r| {
                    *c.entry(r["association"]["status"].as_str().unwrap().into())
                        .or_default() += 1;
                    c
                });
            linked_units.push(json!({"context":base.join("context.json"),"native_vir":vir_path,"input_calls":rows.len(),"statuses":counts,"evidence":dest.join("relations.json"),"native_vir_sha256":hash(text.as_bytes())?}));
        }
        profiles.push(json!({"profile":name,"units":linked_units}));
    }
    project::validate(capture, config)?;
    write_json(
        &out.join("report.json"),
        &json!({"status":"completed","profiles":profiles,"per_client_mappings":0,"scope":"Interface association only; no contract adequacy or client proof conclusion"}),
    )?;
    fingerprint(&out.join("report.json"))?;
    write_json(
        &out.join("origin.json"),
        &json!({"capture":fs::canonicalize(capture)?,"config":fs::canonicalize(config)?}),
    )?;
    fingerprint(&out.join("origin.json"))?;
    write_json(&out.join("evidence.json"), &evidence)
}

pub fn validate(out: &Path) -> Result<()> {
    let origin: Value = serde_json::from_slice(&fs::read(out.join("origin.json"))?)?;
    project::validate(
        Path::new(origin["capture"].as_str().ok_or("missing capture")?),
        Path::new(origin["config"].as_str().ok_or("missing config")?),
    )?;
    let evidence: BTreeMap<std::path::PathBuf, String> =
        serde_json::from_slice(&fs::read(out.join("evidence.json"))?)?;
    if evidence.is_empty() {
        return Err("empty linkage evidence".into());
    }
    for (p, digest) in evidence {
        if hash(&fs::read(&p)?)? != digest {
            return Err(format!("stale or modified linkage evidence: {}", p.display()).into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_and_str_types_match_between_rustc_and_vir() {
        let u8_type = Node::List(vec![
            Node::Atom("Typ".into()),
            Node::Atom("Int".into()),
            Node::List(vec![
                Node::Atom("IntRange".into()),
                Node::Atom("U".into()),
                Node::Atom("8".into()),
            ]),
        ]);
        let slice_type = Node::List(vec![
            Node::Atom("Typ".into()),
            Node::Atom("Primitive".into()),
            Node::List(vec![
                Node::Atom("Primitive".into()),
                Node::Atom("Slice".into()),
            ]),
            Node::List(vec![u8_type]),
        ]);
        let str_type = Node::List(vec![
            Node::Atom("Typ".into()),
            Node::Atom("Primitive".into()),
            Node::List(vec![
                Node::Atom("Primitive".into()),
                Node::Atom("Str".into()),
            ]),
        ]);
        assert_eq!(type_key(&slice_type).as_deref(), Some("slice<u8>"));
        assert_eq!(type_key(&str_type).as_deref(), Some("str"));
    }

    #[test]
    fn source_callsite_recovery_requires_exact_caller_span_and_runtime_path() {
        let text = r#"
            (@ "client.rs:1:1: 1:4 (#0)" (Function :name (Fun :path p::client) :mode Exec :body None))
            (@ "client.rs:1:1: 1:4 (#0)" (Function :name (Fun :path p::_VERUS_VERIFIED_client) :mode Exec
                :body (@@ "client.rs:9:8: 9:19 (#0)" (> Call :target
                    (CallTarget Fun (CallTargetKind Static) (Fun :path p::api::_VERUS_VERIFIED_get) () ()) :args ()))) )
            (@ "api.rs:2:1: 2:4 (#0)" (Function :name (Fun :path p::api::_VERUS_VERIFIED_get) :mode Exec :body (Block)))
        "#;
        let functions = vir::functions(text).unwrap();
        let mut index = BTreeMap::<String, Vec<usize>>::new();
        for (i, function) in functions.iter().enumerate() {
            if let Some(path) = field_path(function) {
                index.entry(path.into()).or_default().push(i);
            }
        }
        let owners = vec![
            vec![
                "LINK_OWNER".into(),
                "DefId(0:1 ~ p::client)".into(),
                "p::client".into(),
                "client.rs:1:1: 1:4 (#0)".into(),
            ],
            vec![
                "LINK_OWNER".into(),
                "DefId(0:2 ~ p::_VERUS_VERIFIED_client)".into(),
                "p::_VERUS_VERIFIED_client".into(),
                "client.rs:1:1: 1:4 (#0)".into(),
            ],
        ];
        let call = json!({"owner":"DefId(1:8 ~ p::client)","span":"client.rs:9:8: 9:19 (#4)"});
        let binding = vec![
            "LINK_CALL".into(),
            "DefId(0:1 ~ p::client)".into(),
            "bb2".into(),
            "DefId(0:2 ~ p::api::get)".into(),
            "p::api::get".into(),
            "DefId(0:2 ~ p::api::get)".into(),
            "p::api::get".into(),
            "api.rs:2:1: 2:4 (#0)".into(),
            "Item".into(),
            "fn() -> ()".into(),
            "client.rs:9:8: 9:19 (#0)".into(),
        ];
        assert_eq!(
            source_callsite_interface(&call, &binding, &owners, &functions, &index),
            Some(2)
        );
        assert_eq!(
            caller_vir_callsite_status(&call, Some(&binding), &owners, &functions, &index),
            "exact_callsite_present"
        );
        assert_eq!(
            source_callsite_interface(
                &json!({"owner":"DefId(1:8 ~ p::client)","span":"client.rs:9:9: 9:19 (#0)"}),
                &binding,
                &owners,
                &functions,
                &index,
            ),
            None
        );
        assert_eq!(
            caller_vir_callsite_status(
                &json!({"owner":"DefId(1:8 ~ p::client)","span":"client.rs:9:9: 9:19 (#0)"}),
                Some(&binding),
                &owners,
                &functions,
                &index,
            ),
            "no_exact_callsite"
        );
    }

    #[test]
    fn verified_callsite_resolves_peer_when_stub_source_projection_fails() {
        let text = r#"
            (@ "client.rs:1:1: 1:4 (#0)" (Function :name (Fun :path p::client) :mode Exec :body None))
            (@ "client.rs:1:1: 1:4 (#0)" (Function :name (Fun :path p::_VERUS_VERIFIED_client) :mode Exec
                :body (@@ "client.rs:9:8: 9:19 (#0)" (> Call :target
                    (CallTarget Fun (CallTargetKind Static) (Fun :path p::api::_VERUS_VERIFIED_get) () ()) :args ()))) )
            (@ "api.rs:2:1: 2:4 (#0)" (Function :name (Fun :path p::api::get) :mode Exec
                :require ((@@ "api.rs:2:1: 2:4 (#0)" (> Const (Constant Bool false)) (Typ Bool)))
                :attrs (FunctionAttrs :is_external_body true) :body None))
            (@ "api.rs:8:1: 8:10 (#0)" (Function :name (Fun :path p::api::_VERUS_VERIFIED_get) :mode Exec :body (Block)))
        "#;
        let functions = vir::functions(text).unwrap();
        let mut index = BTreeMap::<String, Vec<usize>>::new();
        for (i, function) in functions.iter().enumerate() {
            if let Some(path) = field_path(function) {
                index.entry(path.into()).or_default().push(i);
            }
        }
        let owners = vec![
            vec![
                "LINK_OWNER".into(),
                "DefId(0:1 ~ p::client)".into(),
                "p::client".into(),
                "client.rs:1:1: 1:4 (#0)".into(),
            ],
            vec![
                "LINK_OWNER".into(),
                "DefId(0:2 ~ p::_VERUS_VERIFIED_client)".into(),
                "p::_VERUS_VERIFIED_client".into(),
                "client.rs:1:1: 1:4 (#0)".into(),
            ],
        ];
        let call = json!({"owner":"DefId(1:8 ~ p::client)","span":"client.rs:9:8: 9:19 (#4)"});
        let binding = vec![
            "LINK_CALL".into(),
            "DefId(0:1 ~ p::client)".into(),
            "bb2".into(),
            "DefId(0:3 ~ p::api::get)".into(),
            "p::api::get".into(),
            "DefId(0:3 ~ p::api::get)".into(),
            "p::api::get".into(),
            "api.rs:2:1: 2:4 (#0)".into(),
            "Item".into(),
            "fn() -> ()".into(),
            "client.rs:9:8: 9:19 (#0)".into(),
        ];
        let resolved = verification_interface(
            &functions[2],
            &call,
            &binding,
            &owners,
            &functions,
            &index,
            Path::new("."),
        )
        .unwrap();
        assert_eq!(field_path(resolved), Some("p::api::_VERUS_VERIFIED_get"));
    }

    #[test]
    fn caller_body_status_distinguishes_external_from_native_vir_body() {
        let text = r#"
            (@ "client.rs:1:1: 1:4 (#0)" (Function :name (Fun :path p::external_client) :mode Exec
                :attrs (FunctionAttrs :is_external_body true) :body None))
            (@ "client.rs:2:1: 2:4 (#0)" (Function :name (Fun :path p::checked_client) :mode Exec
                :attrs (FunctionAttrs :is_external_body false) :body (Block)))
        "#;
        let functions = vir::functions(text).unwrap();
        let mut index = BTreeMap::<String, Vec<usize>>::new();
        for (i, function) in functions.iter().enumerate() {
            if let Some(path) = field_path(function) {
                index.entry(path.into()).or_default().push(i);
            }
        }
        let owners = vec![
            vec![
                "LINK_OWNER".into(),
                "DefId(0:1 ~ p::external_client)".into(),
                "p::external_client".into(),
                "client.rs:1:1: 1:4 (#0)".into(),
            ],
            vec![
                "LINK_OWNER".into(),
                "DefId(0:2 ~ p::checked_client)".into(),
                "p::checked_client".into(),
                "client.rs:2:1: 2:4 (#0)".into(),
            ],
        ];
        assert_eq!(
            caller_vir_body_status(
                &json!({"owner":"DefId(1:8 ~ p::external_client)"}),
                &owners,
                &functions,
                &index,
            ),
            "external_body"
        );
        assert_eq!(
            caller_vir_body_status(
                &json!({"owner":"DefId(1:9 ~ p::checked_client)"}),
                &owners,
                &functions,
                &index,
            ),
            "body_present"
        );
    }

    #[test]
    fn process_local_numbers_do_not_replace_definition_identity() {
        assert_eq!(
            stable("DefId(0:3 ~ api[aaaa]::{impl#2}::f) (#12)"),
            stable("DefId(1:9 ~ api[aaaa]::{impl#2}::f) (#999)")
        );
        for other in [
            "api[bbbb]::{impl#2}::f",
            "api[aaaa]::{impl#3}::f",
            "api[aaaa]::{impl#2}::g",
        ] {
            assert_ne!(
                stable("DefId(0:3 ~ api[aaaa]::{impl#2}::f)"),
                stable(&format!("DefId(0:3 ~ {other})"))
            );
        }
        assert_ne!(stable("copy _1"), stable("copy _2"));
        assert_ne!(stable("x.rs:1:2: 3:4 (#1)"), stable("x.rs:1:3: 3:4 (#1)"));
    }

    #[test]
    fn generic_signatures_preserve_substitution_and_return_constraints() {
        let mut vars = BTreeMap::new();
        assert!(match_type(
            "core::option::Option<param:T>",
            "core::option::Option<core::ptr::NonNull<param:U>>",
            &mut vars,
            true
        ));
        assert!(match_type(
            "param:T",
            "core::ptr::NonNull<param:U>",
            &mut vars,
            false
        ));
        assert!(!match_type("param:T", "usize", &mut vars, false));
        assert!(!match_type(
            "param:T",
            "core::ptr::NonNull<bool>",
            &mut vars,
            true
        ));
        assert!(!match_type("param:R", "usize", &mut vars, false));
        assert!(match_type("param:R", "param:R", &mut vars, false));
        assert!(!match_type(
            "&param:X",
            "&mut usize",
            &mut BTreeMap::new(),
            true
        ));
        assert!(!match_type(
            "&mut param:T",
            "&core::ptr::NonNull<param:U>",
            &mut vars,
            true
        ));
        assert!(!match_type(
            "tuple[param:T,bool]",
            "tuple[core::ptr::NonNull<param:U>]",
            &mut vars,
            true
        ));
        assert!(!match_type(
            "core::option::Option<param:T>",
            "other::Option<core::ptr::NonNull<param:U>>",
            &mut vars,
            true
        ));
    }

    #[test]
    #[ignore = "requires LINK_CAPTURE, LINK_OUTPUT and LINK_GENERIC_AUDIT from real project runs"]
    fn real_generic_links_reject_inconsistent_signatures() {
        let capture = std::path::PathBuf::from(std::env::var_os("LINK_CAPTURE").unwrap());
        let output = std::path::PathBuf::from(std::env::var_os("LINK_OUTPUT").unwrap());
        let mut results = Vec::new();
        for profile in ["host-tests", "kernel-library"] {
            let unit = output.join(profile).join("unit-0");
            let input = capture.join(profile).join("unit-0");
            let inv: Invocation =
                serde_json::from_slice(&fs::read(input.join("context.json")).unwrap()).unwrap();
            let native: Value =
                serde_json::from_slice(&fs::read(input.join("verification-input.json")).unwrap())
                    .unwrap();
            let functions =
                vir::functions(&fs::read_to_string(native["vir"].as_str().unwrap()).unwrap())
                    .unwrap();
            let mut index = BTreeMap::<String, Vec<usize>>::new();
            for (i, f) in functions.iter().enumerate() {
                if let Some(path) = field_path(f) {
                    index.entry(path.into()).or_default().push(i);
                }
            }
            let rows: Vec<Value> =
                serde_json::from_slice(&fs::read(unit.join("relations.json")).unwrap()).unwrap();
            for row in rows
                .iter()
                .filter(|r| r["association"]["status"] == "linked")
            {
                let a = &row["association"];
                if a["input_type_substitution"].as_object().unwrap().is_empty() {
                    continue;
                }
                let strings = |v: &Value| {
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>()
                };
                let b = strings(&a["compiler_binding"]);
                let t = strings(&a["runtime_signature"]);
                assert_eq!(
                    associate(
                        &row["call"],
                        Some(&b),
                        Some(&t),
                        &[],
                        &functions,
                        &index,
                        &inv.cwd
                    ),
                    *a
                );
                let mut wrong_return = t.clone();
                wrong_return[4] = "__wrong_return_type__".into();
                let mut wrong_arity = t.clone();
                wrong_arity[3].push_str("|usize");
                for wrong in [wrong_return, wrong_arity] {
                    assert_ne!(
                        associate(
                            &row["call"],
                            Some(&b),
                            Some(&wrong),
                            &[],
                            &functions,
                            &index,
                            &inv.cwd
                        )["status"],
                        "linked"
                    );
                }
                results.push(json!({"profile":profile,"call":row["call"],"substitution":a["input_type_substitution"],"replay_matches":true,"wrong_return_rejected":true,"wrong_arity_rejected":true}));
            }
        }
        assert!(!results.is_empty());
        write_json(
            Path::new(&std::env::var("LINK_GENERIC_AUDIT").unwrap()),
            &results,
        )
        .unwrap();
    }

    // Uses native compiler/VIR evidence, including the frozen evaluation cohort.
    #[test]
    #[ignore = "requires Goal 17 capture and completed project-link run"]
    fn frozen_native_interfaces_and_wrong_targets() {
        let capture = std::env::var("LINK_CAPTURE").unwrap();
        let output = std::env::var("LINK_OUTPUT").unwrap();
        let plan: Value = serde_json::from_str(include_str!(
            "../assets/experiments/test-spec/interface-link-plan.json"
        ))
        .unwrap();
        assert_eq!(
            hash(&fs::read(Path::new(&capture).join("configuration.json")).unwrap()).unwrap(),
            plan["capture_configuration_sha256"]
        );
        let mut results = Vec::new();
        for profile in ["host-tests", "kernel-library"] {
            let base = Path::new(&capture).join(profile).join("unit-0");
            assert_eq!(
                hash(&fs::read(base.join("compile/inventory-records.json")).unwrap()).unwrap(),
                plan["capture_records_sha256"][profile]
            );
            let inv: Invocation =
                serde_json::from_slice(&fs::read(base.join("context.json")).unwrap()).unwrap();
            let native: Value =
                serde_json::from_slice(&fs::read(base.join("verification-input.json")).unwrap())
                    .unwrap();
            let text = fs::read_to_string(native["vir"].as_str().unwrap()).unwrap();
            let functions = vir::functions(&text).unwrap();
            let mut index = BTreeMap::<String, Vec<usize>>::new();
            for (i, f) in functions.iter().enumerate() {
                if let Some(p) = field_path(f) {
                    index.entry(p.into()).or_default().push(i);
                }
            }
            let rows: Value = serde_json::from_slice(
                &fs::read(
                    Path::new(&output)
                        .join(profile)
                        .join("unit-0/relations.json"),
                )
                .unwrap(),
            )
            .unwrap();
            for frozen in plan["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["profile"] == profile)
            {
                let row = rows
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["call"] == frozen["call"])
                    .unwrap();
                let a = &row["association"];
                assert_eq!(a["status"], "linked", "{}", frozen["id"]);
                let b = a["compiler_binding"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                let t = a["runtime_signature"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                let correct = associate(
                    &row["call"],
                    Some(&b),
                    Some(&t),
                    &[],
                    &functions,
                    &index,
                    &inv.cwd,
                );
                assert_eq!(correct["status"], "linked");
                assert_eq!(
                    &correct, a,
                    "reported contract differs from native interface"
                );
                let mut mismatched = b.clone();
                mismatched[3].push_str("wrong-definition");
                assert_eq!(
                    associate(
                        &row["call"],
                        Some(&mismatched),
                        Some(&t),
                        &[],
                        &functions,
                        &index,
                        &inv.cwd
                    )["status"],
                    "compiler_identity_mismatch"
                );
                // Try every other native definition under this compiler identity.
                // This includes overloads, macro siblings and other trait impls.
                let key = b[6].clone();
                let good = index[&key][0];
                let mut wrong_index = BTreeMap::new();
                let alternatives = (0..functions.len())
                    .filter(|i| *i != good)
                    .collect::<Vec<_>>();
                wrong_index.insert(key, alternatives);
                assert_ne!(
                    associate(
                        &row["call"],
                        Some(&b),
                        Some(&t),
                        &[],
                        &functions,
                        &wrong_index,
                        &inv.cwd
                    )["status"],
                    "linked"
                );
                let mut wrong_types = t.clone();
                wrong_types[4] = "tuple[bool,bool,bool]".into();
                assert_ne!(
                    associate(
                        &row["call"],
                        Some(&b),
                        Some(&wrong_types),
                        &[],
                        &functions,
                        &index,
                        &inv.cwd
                    )["status"],
                    "linked"
                );
                results.push(json!({"id":frozen["id"],"interface":a["interface"],"trust":a["trust"],"wrong_definition_rejected":true,"wrong_signature_rejected":true,"other_native_definitions_rejected":functions.len()-1}));
            }
        }
        assert_eq!(results.len(), 12);
        write_json(&Path::new(&output).join("frozen-controls.json"), &results).unwrap();
    }
}
