//! Find guard-returning functions whose guard type stores a reference to its owner type.
use crate::{Result, hash, vir, write_json};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn source_span(node: &vir::Node) -> Option<&str> {
    let values = node.list()?;
    matches!(values.first()?.atom(), Some("@" | "@@"))
        .then(|| values.get(1)?.atom())
        .flatten()
}

fn datatype_path(name: &vir::Node) -> Option<&str> {
    let values = name.list()?;
    (values.first()?.atom() == Some("Dt") && values.get(1)?.atom() == Some("Path"))
        .then(|| values.get(2)?.atom())
        .flatten()
}

fn type_path(typ: &vir::Node) -> Option<&str> {
    let values = typ.unwrap().list()?;
    (values.first()?.atom() == Some("Typ") && values.get(1)?.atom() == Some("Datatype"))
        .then(|| values.get(2).and_then(datatype_path))
        .flatten()
}

fn type_args(typ: &vir::Node) -> Vec<&vir::Node> {
    typ.unwrap()
        .list()
        .and_then(|values| values.get(3))
        .and_then(vir::Node::list)
        .unwrap_or_default()
        .iter()
        .collect()
}

fn type_label(typ: &vir::Node) -> String {
    let Some(parts) = typ.unwrap().list() else {
        return typ.atom().unwrap_or("?").to_owned();
    };
    match parts.get(1).and_then(vir::Node::atom) {
        Some("TypParam") => parts
            .get(2)
            .and_then(vir::Node::atom)
            .unwrap_or("?")
            .to_owned(),
        Some("Datatype") => {
            let path = parts.get(2).and_then(datatype_path).unwrap_or("?");
            let args = type_args(typ);
            if args.is_empty() {
                path.to_owned()
            } else {
                format!(
                    "{path}<{}>",
                    args.iter()
                        .map(|arg| type_label(arg))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        _ => crate::contracts::render(typ),
    }
}

fn parameter_names(function: &vir::Node) -> Vec<String> {
    function
        .field("params")
        .and_then(vir::Node::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|param| {
            param
                .field("name")
                .and_then(vir::Node::list)
                .and_then(|parts| parts.get(1))
                .and_then(vir::Node::atom)
                .map(str::to_owned)
        })
        .collect()
}

fn return_shape(function: &vir::Node, guard_path: &str) -> &'static str {
    let Some(typ) = function.field("outer_ret").and_then(|ret| ret.field("typ")) else {
        return "unsupported";
    };
    if type_path(typ) == Some(guard_path) {
        return "direct";
    }
    let option_inner = (type_path(typ) == Some("core::option::Option"))
        .then(|| type_args(typ).first().copied())
        .flatten();
    if option_inner.and_then(type_path) == Some(guard_path) {
        "option"
    } else {
        "unsupported"
    }
}

fn datatype_params(datatype: &vir::Node) -> Vec<String> {
    datatype
        .field("typ_params")
        .and_then(vir::Node::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|param| param.list()?.get(1)?.atom().map(str::to_owned))
        .collect()
}

fn owner_target(field_type: &vir::Node) -> Option<&vir::Node> {
    let values = field_type.unwrap().list()?;
    if values.first()?.atom() == Some("Typ")
        && values.get(1)?.atom() == Some("Decorate")
        && values.get(2)?.list()?.get(1)?.atom() == Some("Ref")
    {
        values.last()
    } else {
        None
    }
}

fn declared_parameter(typ: &vir::Node) -> Option<&str> {
    let values = typ.unwrap().list()?;
    (values.first()?.atom() == Some("Typ") && values.get(1)?.atom() == Some("TypParam"))
        .then(|| values.get(2)?.atom())
        .flatten()
}

fn owner_reference_fields(datatype: &vir::Node) -> Vec<Value> {
    let params = datatype_params(datatype);
    let mut result = Vec::new();
    for variant in datatype
        .field("variants")
        .and_then(vir::Node::list)
        .unwrap_or_default()
    {
        for field in variant
            .field("fields")
            .and_then(vir::Node::list)
            .unwrap_or_default()
        {
            let Some(parts) = field.list() else { continue };
            if parts.first().and_then(vir::Node::atom) != Some("->") {
                continue;
            }
            let Some(field_name) = parts.get(1).and_then(vir::Node::atom) else {
                continue;
            };
            let Some(field_type) = parts
                .get(2)
                .and_then(vir::Node::list)
                .and_then(|v| v.get(1))
            else {
                continue;
            };
            let Some(target) = owner_target(field_type) else {
                continue;
            };
            let Some(owner_path) = type_path(target) else {
                continue;
            };
            let args = type_args(target);
            let owner_params = args
                .iter()
                .map(|arg| declared_parameter(arg))
                .collect::<Option<Vec<_>>>();
            let exact_generic_owner = owner_params.as_ref().is_some_and(|names| {
                names.len() == params.len()
                    && names
                        .iter()
                        .zip(&params)
                        .all(|(name, param)| *name == param)
            });
            result.push(json!({
                "field": field_name,
                "owner_type": owner_path,
                "owner_type_arguments": args.iter().map(|arg| type_label(arg)).collect::<Vec<_>>(),
                "returned_type_parameters": params,
                "all_return_type_parameters_forwarded_in_order": exact_generic_owner,
                "evidence": "VIR datatype field is a shared reference to a datatype whose generic arguments forward every guard type parameter in order"
            }));
        }
    }
    result
}

fn datatype_fields(datatype: &vir::Node) -> Vec<(String, &vir::Node)> {
    datatype
        .field("variants")
        .and_then(vir::Node::list)
        .unwrap_or_default()
        .iter()
        .flat_map(|variant| {
            variant
                .field("fields")
                .and_then(vir::Node::list)
                .unwrap_or_default()
        })
        .filter_map(|field| {
            let parts = field.list()?;
            (parts.first()?.atom() == Some("->")).then(|| {
                Some((
                    parts.get(1)?.atom()?.to_owned(),
                    parts.get(2)?.list()?.get(1)?,
                ))
            })?
        })
        .collect()
}

fn type_args_are_declared_parameters(typ: &vir::Node, datatype: &vir::Node) -> bool {
    let params = datatype_params(datatype);
    let args = type_args(typ);
    args.len() == params.len()
        && args
            .iter()
            .zip(params)
            .all(|(arg, param)| declared_parameter(arg) == Some(param.as_str()))
}

fn owner_reference_paths(
    datatype: &vir::Node,
    type_index: &BTreeMap<String, (&vir::Node, String)>,
) -> Vec<Value> {
    fn walk(
        datatype: &vir::Node,
        type_index: &BTreeMap<String, (&vir::Node, String)>,
        prefix: &[String],
        visiting: &mut BTreeSet<String>,
        output: &mut Vec<Value>,
    ) {
        for mut field in owner_reference_fields(datatype) {
            let Some(name) = field["field"].as_str() else {
                continue;
            };
            let mut path = prefix.to_vec();
            path.push(name.to_owned());
            field["field_path"] = json!(path);
            output.push(field);
        }
        let Some(current_path) = datatype.field("name").and_then(datatype_path) else {
            return;
        };
        for (field_name, field_type) in datatype_fields(datatype) {
            let Some(child_path) = type_path(field_type) else {
                continue;
            };
            let Some((child, _)) = type_index.get(child_path) else {
                continue;
            };
            if child_path == current_path
                || visiting.contains(child_path)
                || !type_args_are_declared_parameters(field_type, child)
            {
                continue;
            }
            let mut path = prefix.to_vec();
            path.push(field_name);
            visiting.insert(child_path.to_owned());
            walk(child, type_index, &path, visiting, output);
            visiting.remove(child_path);
        }
    }

    let mut output = Vec::new();
    let mut visiting = BTreeSet::new();
    if let Some(path) = datatype.field("name").and_then(datatype_path) {
        visiting.insert(path.to_owned());
    }
    walk(datatype, type_index, &[], &mut visiting, &mut output);
    output
}

fn return_types(function: &vir::Node, output: &mut Vec<(String, Vec<String>)>) {
    let Some(ret_type) = function.field("inner_ret").and_then(|ret| ret.field("typ")) else {
        return;
    };
    fn walk(typ: &vir::Node, output: &mut Vec<(String, Vec<String>)>) {
        if let Some(path) = type_path(typ) {
            output.push((
                path.to_owned(),
                type_args(typ).iter().map(|arg| type_label(arg)).collect(),
            ));
        }
        if let Some(values) = typ.unwrap().list() {
            for child in values {
                walk(child, output);
            }
        }
    }
    walk(ret_type, output);
}

fn owner_receiver_matches(
    function: &vir::Node,
    owner_path: &str,
    owner_args: &[String],
    type_index: &BTreeMap<String, (&vir::Node, String)>,
) -> Option<Value> {
    for (index, param) in function
        .field("params")
        .and_then(vir::Node::list)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let Some(typ) = param.field("typ") else {
            continue;
        };
        let receiver_type = typ
            .unwrap()
            .list()
            .and_then(|parts| {
                (parts.first()?.atom() == Some("Typ")
                    && parts.get(1)?.atom() == Some("Decorate")
                    && parts.get(2)?.list()?.get(1)?.atom() == Some("Ref"))
                .then(|| parts.last())
                .flatten()
            })
            .unwrap_or(typ);
        let args = type_args(receiver_type)
            .iter()
            .map(|arg| type_label(arg))
            .collect::<Vec<_>>();
        let mut projection = if type_path(receiver_type) == Some(owner_path) && args == owner_args {
            Some(Vec::new())
        } else {
            None
        };
        if projection.is_none() {
            fn find_projection(
                datatype: &vir::Node,
                type_index: &BTreeMap<String, (&vir::Node, String)>,
                owner_path: &str,
                owner_args: &[String],
                prefix: &[String],
                visiting: &mut BTreeSet<String>,
            ) -> Option<Vec<String>> {
                for (field_name, field_type) in datatype_fields(datatype) {
                    let target = owner_target(field_type).unwrap_or(field_type);
                    let args = type_args(target)
                        .iter()
                        .map(|arg| type_label(arg))
                        .collect::<Vec<_>>();
                    let mut path = prefix.to_vec();
                    path.push(field_name);
                    if type_path(target) == Some(owner_path) && args == owner_args {
                        return Some(path);
                    }
                    let Some(child_path) = type_path(field_type) else {
                        continue;
                    };
                    let Some((child, _)) = type_index.get(child_path) else {
                        continue;
                    };
                    if child_path == owner_path
                        || visiting.contains(child_path)
                        || !type_args_are_declared_parameters(field_type, child)
                    {
                        continue;
                    }
                    visiting.insert(child_path.to_owned());
                    let found =
                        find_projection(child, type_index, owner_path, owner_args, &path, visiting);
                    visiting.remove(child_path);
                    if found.is_some() {
                        return found;
                    }
                }
                None
            }
            if let Some(path) = type_path(receiver_type)
                && let Some((datatype, _)) = type_index.get(path)
            {
                let mut visiting = BTreeSet::from([path.to_owned()]);
                projection = find_projection(
                    datatype,
                    type_index,
                    owner_path,
                    owner_args,
                    &[],
                    &mut visiting,
                );
            }
        }
        let Some(projection) = projection else {
            continue;
        };
        let name = param
            .field("name")
            .and_then(vir::Node::list)
            .and_then(|parts| parts.get(1))
            .and_then(vir::Node::atom)
            .unwrap_or("unknown");
        return Some(json!({
            "parameter_index": index,
            "parameter_name": name,
            "type_arguments_match_guard_owner_field": true,
            "parameter_type_arguments": args,
            "receiver_type_path": type_path(receiver_type),
            "owner_projection_path": projection
        }));
    }
    None
}

fn body_owner_constructors(
    node: &vir::Node,
    owner_return_type: &str,
    owner_field: &str,
    self_binder: Option<&vir::Node>,
    output: &mut Vec<Value>,
) {
    let Some(parts) = node.unwrap().list() else {
        return;
    };
    if parts.first().and_then(vir::Node::atom) == Some(">")
        && parts.get(1).and_then(vir::Node::atom) == Some("Ctor")
        && parts.get(2).and_then(datatype_path) == Some(owner_return_type)
    {
        for field in parts.get(4).and_then(vir::Node::list).unwrap_or_default() {
            let Some(field_parts) = field.list() else {
                continue;
            };
            if field_parts.first().and_then(vir::Node::atom) != Some("->")
                || field_parts.get(1).and_then(vir::Node::atom) != Some(owner_field)
            {
                continue;
            }
            let expression = field_parts.get(2);
            let direct_self_read = expression
                .and_then(|expr| expr.unwrap().list())
                .is_some_and(|expr| {
                    expr.first().and_then(vir::Node::atom) == Some(">")
                        && expr.get(1).and_then(vir::Node::atom) == Some("ReadPlace")
                        && expr
                            .get(2)
                            .and_then(|place| place.unwrap().list())
                            .is_some_and(|place| {
                                place.first().and_then(vir::Node::atom) == Some("Place")
                                    && place.get(1).and_then(vir::Node::atom) == Some("Local")
                                    && place.get(2).is_some_and(|binding| {
                                        self_binder.is_some_and(|self_binder| {
                                            crate::contracts::render(binding)
                                                == crate::contracts::render(self_binder)
                                        })
                                    })
                            })
                });
            output.push(json!({
                "constructor_span": source_span(node),
                "field": owner_field,
                "initializer_span": expression.and_then(source_span),
                "direct_read_of_self_binder": direct_self_read
            }));
        }
    }
    for child in parts {
        body_owner_constructors(child, owner_return_type, owner_field, self_binder, output);
    }
}

fn block_tail_expression(function: &vir::Node) -> Option<&vir::Node> {
    let block = function.field("body")?.unwrap().list()?;
    if let Some(expression) = block.get(3) {
        let parts = expression.unwrap().list();
        if parts.is_some_and(|parts| {
            parts.first().and_then(vir::Node::atom) == Some(">")
                && matches!(
                    parts.get(1).and_then(vir::Node::atom),
                    Some("Ctor" | "Call")
                )
        }) {
            return Some(expression);
        }
    }
    let statements = block.get(2)?.list()?;
    statements.iter().rev().find_map(|statement| {
        let parts = statement.unwrap().list()?;
        if parts.first()?.atom() != Some("Stmt") || parts.get(1)?.atom() != Some("Expr") {
            return None;
        }
        let expression = parts.get(2)?;
        let expression_parts = expression.unwrap().list()?;
        (expression_parts.first()?.atom() == Some(">")
            && matches!(expression_parts.get(1)?.atom(), Some("Ctor" | "Call")))
        .then_some(expression)
    })
}

fn constructor_field<'a>(expression: &'a vir::Node, field_name: &str) -> Option<&'a vir::Node> {
    let parts = expression.unwrap().list()?;
    if parts.first()?.atom() != Some(">") || parts.get(1)?.atom() != Some("Ctor") {
        return None;
    }
    parts.get(4)?.list()?.iter().find_map(|field| {
        let values = field.list()?;
        (values.first()?.atom() == Some("->") && values.get(1)?.atom() == Some(field_name))
            .then(|| values.get(2))
            .flatten()
    })
}

fn function_path(node: &vir::Node) -> Option<&str> {
    let parts = node.unwrap().list()?;
    if parts.first()?.atom() == Some("Fun") {
        return node.field("path").and_then(vir::Node::atom);
    }
    parts.iter().find_map(function_path)
}

fn place_projection(place: &vir::Node) -> Option<(String, Vec<String>, bool)> {
    let parts = place.unwrap().list()?;
    match parts.get(1)?.atom()? {
        "Local" => Some((
            parts.get(2)?.list()?.get(1)?.atom()?.to_owned(),
            Vec::new(),
            false,
        )),
        "Field" => {
            let (root, mut path, through_temporary_read) = place_projection(parts.get(3)?)?;
            path.push(parts.get(2)?.field("field")?.atom()?.to_owned());
            Some((root, path, through_temporary_read))
        }
        "Temporary" => {
            let expression = parts.get(2)?.unwrap().list()?;
            (expression.get(1)?.atom()? == "ReadPlace")
                .then(|| {
                    place_projection(expression.get(2)?).map(|(root, path, _)| (root, path, true))
                })
                .flatten()
        }
        _ => None,
    }
}

fn returned_call<'a>(
    function: &'a vir::Node,
    field_path: &[String],
) -> Option<(String, Vec<String>, Vec<String>, &'a vir::Node)> {
    let mut expression = block_tail_expression(function)?;
    let mut remaining = field_path;
    let mut constructor_prefix = Vec::new();
    loop {
        let parts = expression.unwrap().list()?;
        if parts.first()?.atom() != Some(">") {
            return None;
        }
        match parts.get(1)?.atom()? {
            "Ctor" if !remaining.is_empty() => {
                expression = constructor_field(expression, &remaining[0])?;
                constructor_prefix.push(remaining[0].clone());
                remaining = &remaining[1..];
            }
            "Call" => {
                let target = expression.field("target")?;
                let callee = function_path(target)?.to_owned();
                return Some((callee, constructor_prefix, remaining.to_vec(), expression));
            }
            _ => return None,
        }
    }
}

fn node_kind(node: &vir::Node) -> Option<&str> {
    let parts = node.unwrap().list()?;
    (parts.first()?.atom() == Some(">"))
        .then(|| parts.get(1)?.atom())
        .flatten()
}

fn read_place_projection(node: &vir::Node) -> Option<(String, Vec<String>, bool)> {
    let parts = node.unwrap().list()?;
    (parts.first()?.atom() == Some(">") && parts.get(1)?.atom() == Some("ReadPlace"))
        .then(|| parts.get(2).and_then(place_projection))
        .flatten()
}

fn closure_tail_calls<'a>(
    node: &'a vir::Node,
    output: &mut Vec<(&'a vir::Node, &'a vir::Node)>,
) {
    if node_kind(node) == Some("NonSpecClosure") {
        if let Some(expression) = block_tail_expression(node)
            && node_kind(expression) == Some("Call")
        {
            output.push((node, expression));
        }
        return;
    }
    if let Some(parts) = node.list() {
        for child in parts {
            closure_tail_calls(child, output);
        }
    }
}

fn calls_through_closures<'a>(
    node: &'a vir::Node,
    output: &mut Vec<(&'a vir::Node, &'a vir::Node, &'a vir::Node)>,
) {
    if node_kind(node) == Some("Call")
        && let Some(target) = node.field("target")
        && let Some(helper) = function_path(target)
        && helper.ends_with("::wait_until")
    {
        for argument in node
            .field("args")
            .and_then(vir::Node::list)
            .unwrap_or_default()
            .iter()
        {
            let mut tails = Vec::new();
            closure_tail_calls(argument, &mut tails);
            for (closure, tail) in tails {
                output.push((node, closure, tail));
            }
        }
    }
    if let Some(parts) = node.list() {
        for child in parts {
            calls_through_closures(child, output);
        }
    }
}

fn empty_contract_part(closure: &vir::Node, name: &str) -> Option<bool> {
    closure
        .field(name)
        .and_then(vir::Node::list)
        .map(|parts| parts.is_empty())
}

fn function_ensures_empty(function: &vir::Node) -> Option<bool> {
    let parts = function.field("ensure")?.unwrap().list()?;
    (parts.first()?.atom() == Some("tuple")).then(|| {
        parts.get(2).and_then(vir::Node::list).is_some_and(|items| items.is_empty())
    })
}

fn ensures_returns_callback_result(function: &vir::Node) -> bool {
    let Some(ensures) = function.field("ensure") else {
        return false;
    };
    let Some(return_name) = function
        .field("outer_ret")
        .and_then(|ret| ret.field("name"))
        .and_then(vir::Node::list)
        .and_then(|parts| parts.get(1))
        .and_then(vir::Node::atom)
    else {
        return false;
    };
    let Some(callback_name) = function
        .field("params")
        .and_then(vir::Node::list)
        .and_then(|params| params.last())
        .and_then(|param| param.field("name"))
        .and_then(vir::Node::list)
        .and_then(|parts| parts.get(1))
        .and_then(vir::Node::atom)
    else {
        return false;
    };
    fn direct_local(node: &vir::Node) -> Option<String> {
        let parts = node.unwrap().list().unwrap_or_default();
        (parts.first().and_then(vir::Node::atom) == Some(">")
            && parts.get(1).and_then(vir::Node::atom) == Some("Ctor")
            && parts.get(2).and_then(datatype_path) == Some("core::option::Option")
            && parts.get(3).and_then(vir::Node::atom) == Some("Some")
        )
            .then(|| parts.get(4))
            .flatten()?
            .list()?
            .iter()
            .find_map(|field| {
                let parts = field.list()?;
                (parts.first()?.atom() == Some("->") && parts.get(1)?.atom() == Some("0"))
                    .then(|| parts.get(2))
                    .flatten()
            })
            .and_then(read_place_projection)
            .and_then(|(root, path, _)| path.is_empty().then_some(root))
    }
    fn links_callback_postcondition(node: &vir::Node, callback: &str, result: &str) -> bool {
        if node_kind(node) == Some("Call")
            && node
                .field("target")
                .and_then(function_path)
                .is_some_and(|path| path.ends_with("::ensures"))
            && let Some(args) = node.field("args").and_then(vir::Node::list)
            && args.len() >= 3
            && read_place_projection(&args[0])
                .is_some_and(|(root, path, _)| root == callback && path.is_empty())
            && direct_local(&args[2]).as_deref() == Some(result)
        {
            return true;
        }
        node.list().is_some_and(|parts| {
            parts
                .iter()
                .any(|child| links_callback_postcondition(child, callback, result))
        })
    }
    links_callback_postcondition(ensures, callback_name, return_name)
}

/// Find blocking guard APIs whose `wait_until` callback returns a same-owner
/// `try_*` result. The report records VIR flow evidence and leaves entailment to
/// Verus; it does not infer a missing contract from structure alone.
pub fn closure_forwarder_scan(
    input: &Path,
    owner_report_path: &Path,
    out: &Path,
) -> Result<()> {
    if out.exists() {
        return Err("owner-closure-scan output directory already exists".into());
    }
    let input = input.canonicalize()?;
    let text = fs::read_to_string(&input)?;
    let input_hash = hash(text.as_bytes())?;
    let owner_report: Value = serde_json::from_slice(&fs::read(owner_report_path)?)?;
    if owner_report["input"]["sha256"].as_str() != Some(&input_hash) {
        return Err("owner report was produced from a different VIR input".into());
    }
    let functions = vir::functions(&text)?;
    let function_index = functions
        .iter()
        .filter_map(|function| {
            Some((
                function.field("name")?.field("path")?.atom()?.to_owned(),
                function,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let owner_entries = owner_report["entries"]
        .as_array()
        .ok_or("owner report entries must be an array")?;
    let mut findings = Vec::new();

    for entry in owner_entries {
        let Some(function_name) = entry["function"].as_str() else {
            continue;
        };
        let Some(function) = function_index.get(function_name) else {
            continue;
        };
        let mut closure_calls = Vec::new();
        if let Some(body) = function.field("body") {
            calls_through_closures(body, &mut closure_calls);
        }
        for field in entry["owner_reference_fields"].as_array().into_iter().flatten() {
            let Some(field_path) = field["field_path"].as_array() else {
                continue;
            };
            let field_path = field_path
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let Some(caller_owner_path) = field["matching_input_parameter"]["owner_projection_path"]
                .as_array()
                .map(|parts| parts.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>())
            else {
                continue;
            };
            let mut seen_flows = BTreeSet::new();
            for (helper_call, closure, returned_call) in &closure_calls {
                let flow_key = (
                    source_span(closure).unwrap_or("").to_owned(),
                    source_span(returned_call).unwrap_or("").to_owned(),
                );
                if !seen_flows.insert(flow_key) {
                    continue;
                }
                let Some(target) = returned_call.field("target") else {
                    continue;
                };
                let Some(callee_name) = function_path(target) else {
                    continue;
                };
                let Some(callee_entry) = owner_entries.iter().find(|candidate| {
                    candidate["function"].as_str() == Some(callee_name)
                        && candidate["guard_type"] == entry["guard_type"]
                        && candidate["return_type_arguments"] == entry["return_type_arguments"]
                }) else {
                    continue;
                };
                let Some(callee_field) = callee_entry["owner_reference_fields"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|candidate| {
                        candidate["field_path"].as_array().is_some_and(|parts| {
                            parts.iter().filter_map(Value::as_str).collect::<Vec<_>>() == field_path
                        }) && candidate["matching_input_parameter"]["parameter_name"] == "self"
                            && candidate["all_return_type_parameters_forwarded_in_order"] == true
                    })
                else {
                    continue;
                };
                let Some(receiver) = returned_call
                    .field("args")
                    .and_then(vir::Node::list)
                    .and_then(|args| args.first())
                    .and_then(read_place_projection)
                else {
                    continue;
                };
                let (root, receiver_path, through_temporary_read) = receiver;
                let callee_owner_path = callee_field["matching_input_parameter"]["owner_projection_path"]
                    .as_array()
                    .map(|parts| parts.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>())
                    .unwrap_or_default();
                let mut composed_owner_path = receiver_path.clone();
                composed_owner_path.extend(callee_owner_path.iter().cloned());
                if root != "self" || composed_owner_path != caller_owner_path {
                    continue;
                }
                let helper = helper_call
                    .field("target")
                    .and_then(function_path)
                    .unwrap_or("<unresolved>");
                let helper_span = source_span(helper_call).unwrap_or("<unavailable>");
                let closure_span = source_span(closure).unwrap_or("<unavailable>");
                let returned_call_span = source_span(returned_call).unwrap_or("<unavailable>");
                let callee_function = function_index.get(callee_name);
                findings.push(json!({
                    "caller": function_name,
                    "caller_span": entry["function_span"],
                    "caller_return_shape": entry["return_shape"],
                    "caller_ensures_empty": function_ensures_empty(function),
                    "guard_type": entry["guard_type"],
                    "owner_field_path": field_path,
                    "caller_owner_projection_path": caller_owner_path,
                    "helper": helper,
                    "helper_call_span": helper_span,
                    "closure_span": closure_span,
                    "closure_requires_empty": empty_contract_part(closure, "requires"),
                    "closure_ensures_empty": empty_contract_part(closure, "ensures"),
                    "closure_tail_call": callee_name,
                    "closure_tail_call_span": returned_call_span,
                    "callee_return_shape": callee_entry["return_shape"],
                    "callee_ensures_empty": callee_function.and_then(|f| function_ensures_empty(f)),
                    "callee_constructor_evidence": callee_field["constructor_evidence"],
                    "receiver_argument_path": receiver_path,
                    "receiver_argument_path_contains_temporary_read": through_temporary_read,
                    "helper_contract_links_callback_ensures_to_some_result": helper_contains_closure_ensures(&function_index, helper),
                    "flow_status": if callee_field["constructor_evidence"]["all_matching_initializers_directly_read_self"] == true { "typed closure flow and owner construction linked" } else { "typed closure flow linked; callee implementation does not expose constructor evidence" },
                }));
            }
        }
    }
    findings.sort_by(|a, b| {
        a["caller"].as_str().cmp(&b["caller"].as_str())
            .then(a["closure_tail_call"].as_str().cmp(&b["closure_tail_call"].as_str()))
    });
    let with_self_constructor = findings.iter().filter(|finding| {
        finding["callee_constructor_evidence"]["all_matching_initializers_directly_read_self"]
            == true
    }).count();
    let callback_contracts_empty = findings.iter().filter(|finding| {
        finding["closure_ensures_empty"] == true
    }).count();
    let helpers_link_callback_ensures = findings.iter().filter(|finding| {
        finding["helper_contract_links_callback_ensures_to_some_result"] == true
    }).count();
    let report = json!({
        "schema": "vir-owner-closure-flow-v1",
        "input": {"vir_sha256": input_hash},
        "owner_report_schema": owner_report["schema"],
        "owner_report_vir_sha256": input_hash,
        "summary": {
            "closure_forwarders": findings.len(),
            "callee_self_constructor_evidence": with_self_constructor,
            "callback_ensures_empty": callback_contracts_empty,
            "helper_contract_links_callback_ensures_to_some_result": helpers_link_callback_ensures
        },
        "findings": findings,
        "interpretation": "Structural VIR evidence links a blocking method's wait_until closure to a same-owner try_* result. It does not prove all control-flow paths, helper implementation equivalence, runtime behavior, or API intent."
    });
    fs::create_dir_all(out)?;
    write_json(&out.join("report.json"), &report)?;
    Ok(())
}

fn helper_contains_closure_ensures(
    function_index: &BTreeMap<String, &vir::Node>,
    helper: &str,
) -> bool {
    let Some(function) = function_index.get(helper) else {
        return false;
    };
    ensures_returns_callback_result(function)
}

pub fn run(input: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("guard-owner-scan output directory already exists".into());
    }
    let input = input.canonicalize()?;
    let text = fs::read_to_string(&input)?;
    let functions = vir::functions(&text)?;
    let datatypes = vir::datatypes(&text)?;
    let mut type_index = BTreeMap::new();
    for datatype in &datatypes {
        if let (Some(path), Some(span)) = (
            datatype.field("name").and_then(datatype_path),
            source_span(datatype),
        ) {
            type_index.insert(path.to_owned(), (datatype, span.to_owned()));
        }
    }
    let mut results = Vec::new();
    for function in &functions {
        let Some(name) = function
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(vir::Node::atom)
        else {
            continue;
        };
        let mut returned = Vec::new();
        return_types(function, &mut returned);
        let guards = returned.into_iter().filter(|(path, _)| {
            type_index.get(path).is_some_and(|(datatype, _)| {
                owner_reference_paths(datatype, &type_index)
                    .iter()
                    .any(|field| field["all_return_type_parameters_forwarded_in_order"] == true)
            })
        });
        for (guard_path, concrete_args) in guards {
            if let Some((datatype, datatype_span)) = type_index.get(&guard_path) {
                let self_binder = function
                    .field("params")
                    .and_then(vir::Node::list)
                    .unwrap_or_default()
                    .iter()
                    .find(|param| {
                        param
                            .field("name")
                            .and_then(vir::Node::list)
                            .and_then(|parts| parts.get(1))
                            .and_then(vir::Node::atom)
                            == Some("self")
                    })
                    .and_then(|param| param.field("name"));
                let owner_fields = owner_reference_paths(datatype, &type_index)
                    .into_iter()
                    .map(|mut field| {
                        let mut constructors = Vec::new();
                        if let (Some(body), Some(field_name)) =
                            (function.field("body"), field["field"].as_str())
                        {
                            body_owner_constructors(
                                body,
                                &guard_path,
                                field_name,
                                self_binder,
                                &mut constructors,
                            );
                        }
                        let all_direct_self = !constructors.is_empty()
                            && constructors
                                .iter()
                                .all(|item| item["direct_read_of_self_binder"] == true);
                        field["constructor_evidence"] = json!({
                            "matching_constructor_count": constructors.len(),
                            "all_matching_initializers_directly_read_self": all_direct_self,
                            "instances": constructors
                        });
                        if let (Some(owner_path), Some(owner_args)) = (
                            field["owner_type"].as_str(),
                            field["owner_type_arguments"].as_array(),
                        ) {
                            let args = owner_args
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect::<Vec<_>>();
                            field["matching_input_parameter"] =
                                owner_receiver_matches(function, owner_path, &args, &type_index)
                                    .unwrap_or(Value::Null);
                        }
                        field
                    })
                    .collect::<Vec<_>>();
                results.push(json!({
                    "function": name,
                    "function_span": source_span(function),
                    "parameter_names": parameter_names(function),
                    "return_shape": return_shape(function, &guard_path),
                    "guard_type": guard_path,
                    "return_type_arguments": concrete_args,
                    "guard_type_span": datatype_span,
                    "owner_reference_fields": owner_fields
                }));
            }
        }
    }
    let function_index = functions
        .iter()
        .filter_map(|function| {
            Some((
                function.field("name")?.field("path")?.atom()?.to_owned(),
                function,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let owner_summaries = results
        .iter()
        .flat_map(|entry| {
            entry["owner_reference_fields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|field| {
                    if field["all_return_type_parameters_forwarded_in_order"] != true {
                        return None;
                    }
                    let matched = field.get("matching_input_parameter")?;
                    if matched["type_arguments_match_guard_owner_field"] != true {
                        return None;
                    }
                    Some((
                        entry["function"].as_str()?.to_owned(),
                        field["field_path"]
                            .as_array()?
                            .iter()
                            .map(|part| part.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()?,
                        matched["parameter_name"].as_str()?.to_owned(),
                        matched["owner_projection_path"]
                            .as_array()?
                            .iter()
                            .map(|part| part.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()?,
                    ))
                })
        })
        .fold(
            BTreeMap::<String, Vec<(Vec<String>, String, Vec<String>)>>::new(),
            |mut summaries, (function, field_path, parameter, owner_path)| {
                summaries
                    .entry(function)
                    .or_default()
                    .push((field_path, parameter, owner_path));
                summaries
            },
        );
    let mut linked_return_flows = 0;
    for entry in &mut results {
        let Some(function_name) = entry["function"].as_str() else {
            continue;
        };
        let Some(function) = function_index.get(function_name) else {
            continue;
        };
        for field in entry["owner_reference_fields"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            let Some(field_path) = field["field_path"].as_array().and_then(|parts| {
                parts
                    .iter()
                    .map(|part| part.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            }) else {
                continue;
            };
            let Some((callee_name, constructor_prefix, callee_field_path, call)) =
                returned_call(function, &field_path)
            else {
                continue;
            };
            let Some(callee) = function_index.get(&callee_name) else {
                continue;
            };
            let Some(receiver_index) =
                callee
                    .field("params")
                    .and_then(vir::Node::list)
                    .and_then(|params| {
                        params.iter().position(|param| {
                            param
                                .field("name")
                                .and_then(vir::Node::list)
                                .and_then(|parts| parts.get(1))
                                .and_then(vir::Node::atom)
                                == Some("self")
                        })
                    })
            else {
                continue;
            };
            let Some(actual) = call
                .field("args")
                .and_then(vir::Node::list)
                .and_then(|args| args.get(receiver_index))
            else {
                continue;
            };
            let Some(actual_parts) = actual.unwrap().list() else {
                continue;
            };
            if actual_parts.first().and_then(vir::Node::atom) != Some(">")
                || actual_parts.get(1).and_then(vir::Node::atom) != Some("ReadPlace")
            {
                continue;
            }
            let Some((root, argument_path, through_temporary_read)) =
                actual_parts.get(2).and_then(place_projection)
            else {
                continue;
            };
            if root != "self" {
                continue;
            }
            let callee_relation = owner_summaries
                .get(&callee_name)
                .into_iter()
                .flatten()
                .find(|(path, parameter, _)| *path == callee_field_path && parameter == "self");
            let caller_relation = field
                .get("matching_input_parameter")
                .and_then(serde_json::Value::as_object);
            let Some((_, _, callee_owner_path)) = callee_relation else {
                continue;
            };
            let matched_caller_owner_path = caller_relation
                .and_then(|relation| relation.get("owner_projection_path"))
                .and_then(serde_json::Value::as_array)
                .and_then(|parts| {
                    parts
                        .iter()
                        .map(|part| part.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                });
            let mut composed_guard_path = constructor_prefix.clone();
            composed_guard_path.extend(callee_field_path.clone());
            let mut composed_owner_path = argument_path.clone();
            composed_owner_path.extend(callee_owner_path.iter().cloned());
            let caller_type_match = caller_relation.is_some();
            let relation_linked = composed_guard_path == field_path
                && matched_caller_owner_path
                    .as_ref()
                    .is_none_or(|path| path == &composed_owner_path);
            let caller_owner_path = matched_caller_owner_path.unwrap_or(composed_owner_path);
            field["return_flow"] = json!({
                "callee": callee_name,
                "callee_owner_field_path": callee_field_path,
                "constructor_field_prefix": constructor_prefix,
                "receiver_argument_path": argument_path,
                "receiver_argument_path_contains_temporary_read": through_temporary_read,
                "caller_owner_projection_path": caller_owner_path,
                "caller_owner_path_type_matched": caller_type_match,
                "call_span": source_span(call),
                "linked_to_caller_owner": relation_linked
            });
            if relation_linked {
                linked_return_flows += 1;
            }
        }
    }
    let report = json!({
        "schema": 2,
        "input": {"path": input, "sha256": hash(text.as_bytes())?},
        "functions_in_vir": functions.len(),
        "datatypes_in_vir": datatypes.len(),
        "owner_reference_return_entries": results.len(),
        "owner_reference_field_candidates": results
            .iter()
            .map(|entry| entry["owner_reference_fields"].as_array().map_or(0, Vec::len))
            .sum::<usize>(),
        "functions_with_owner_field_initialized_from_self": results
            .iter()
            .filter_map(|entry| {
                entry["owner_reference_fields"]
                    .as_array()?
                    .iter()
                    .any(|field| {
                        field["constructor_evidence"]["all_matching_initializers_directly_read_self"]
                            == true
                    })
                    .then(|| entry["function"].as_str().map(str::to_owned))
                    .flatten()
            })
            .collect::<BTreeSet<_>>()
            .len(),
        "exact_self_owner_type_matches": results
            .iter()
            .flat_map(|entry| entry["owner_reference_fields"].as_array().into_iter().flatten())
            .filter(|field| {
                field["matching_input_parameter"]["parameter_name"] == "self"
                    && field["matching_input_parameter"]["type_arguments_match_guard_owner_field"] == true
                    && field["matching_input_parameter"]["owner_projection_path"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
            })
            .count(),
        "self_owner_projection_matches": results
            .iter()
            .flat_map(|entry| entry["owner_reference_fields"].as_array().into_iter().flatten())
            .filter(|field| {
                field["matching_input_parameter"]["parameter_name"] == "self"
                    && field["matching_input_parameter"]["type_arguments_match_guard_owner_field"] == true
                    && field["matching_input_parameter"]["owner_projection_path"]
                        .as_array()
                        .is_some_and(|path| !path.is_empty())
            })
            .count(),
        "linked_return_owner_flows": linked_return_flows,
        "entries": results,
        "interpretation": "This scan reports typed owner-field candidates, constructor-field occurrences, and one-hop returned-call owner flows. These facts do not prove all return paths or API intent."
    });
    fs::create_dir_all(out)?;
    write_json(&out.join("report.json"), &report)?;
    println!(
        "{} owner-reference return candidates; VIR fields and receivers scanned",
        results.len()
    );
    Ok(())
}
