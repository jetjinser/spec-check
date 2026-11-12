//! Spec census: per-function outputs and postcondition mention facts from VIR.
//!
//! Pure analysis over a Verus VIR log. It does not inject probes, run the
//! verifier, or make defect judgements. See `docs/M7a-spec-census.md`.

use crate::vir::{self, Node};
use crate::{Result, hash, write_json};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

const SCHEMA: &str = "spec-check.census/1";

fn typ(node: Option<&Node>) -> Option<&Node> {
    let node = node?;
    let inner = node.unwrap();
    inner.field("typ")
}

fn render_compact(node: &Node) -> String {
    if let Some(atom) = node.atom() {
        return atom.to_owned();
    }
    let Some(items) = node.list() else {
        return String::new();
    };
    if items.first().and_then(Node::atom) == Some("VarIdent") {
        return items.get(1).and_then(Node::atom).unwrap_or("_").to_owned();
    }
    if matches!(items.first().and_then(Node::atom), Some("@" | "@@")) {
        return items.get(2).map(render_compact).unwrap_or_default();
    }
    items
        .iter()
        .map(render_compact)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_relative(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn param_name(node: &Node) -> Option<String> {
    let inner = node.unwrap();
    let name = inner.field("name")?.unwrap();
    if name.list()?.first()?.atom()? != "VarIdent" {
        return None;
    }
    name.list()?.get(1)?.atom().map(str::to_owned)
}

fn type_contains_mutref(node: &Node) -> bool {
    let Some(items) = node.list() else {
        return false;
    };
    if items.first().and_then(Node::atom) == Some("Typ")
        && items.get(1).and_then(Node::atom) == Some("MutRef")
    {
        return true;
    }
    items.iter().any(type_contains_mutref)
}

fn type_is_unit(node: &Node) -> bool {
    let Some(items) = node.list() else {
        return false;
    };
    items.first().and_then(Node::atom) == Some("Typ")
        && items.get(1).and_then(Node::atom) == Some("Datatype")
        && items.get(2).and_then(Node::list).is_some_and(|dt| {
            dt.first().and_then(Node::atom) == Some("Dt")
                && dt.get(1).and_then(Node::atom) == Some("Tuple")
                && dt.get(2).and_then(Node::atom) == Some("0")
        })
}

fn unwrap_type(node: &Node) -> &Node {
    let Some(items) = node.list() else {
        return node;
    };
    if items.first().and_then(Node::atom) == Some("Typ") {
        if items.get(1).and_then(Node::atom) == Some("Decorate") {
            if let Some(inner) = items.get(3) {
                return unwrap_type(inner);
            }
        }
        if items.get(1).and_then(Node::atom) == Some("MutRef") {
            if let Some(inner) = items.get(2) {
                return unwrap_type(inner);
            }
        }
    }
    node
}

fn type_path(node: &Node) -> Option<String> {
    let node = unwrap_type(node);
    let items = node.list()?;
    if items.first().and_then(Node::atom) != Some("Typ")
        || items.get(1).and_then(Node::atom) != Some("Datatype")
    {
        return None;
    }
    let dt = items.get(2)?.list()?;
    if dt.first().and_then(Node::atom) != Some("Dt")
        || dt.get(1).and_then(Node::atom) != Some("Path")
    {
        return None;
    }
    dt.get(2).and_then(Node::atom).map(str::to_owned)
}

fn is_ghost_type(node: &Node) -> bool {
    let Some(items) = node.list() else {
        return false;
    };
    if items.first().and_then(Node::atom) == Some("Typ")
        && items.get(1).and_then(Node::atom) == Some("Decorate")
    {
        let decoration = items
            .get(2)
            .and_then(Node::list)
            .and_then(|d| d.get(1))
            .and_then(Node::atom);
        if matches!(decoration, Some("Tracked" | "Ghost")) {
            return true;
        }
    }
    false
}

#[derive(Debug)]
struct PatternComponent {
    name: Option<String>,
    typ: Option<Node>,
}

fn find_injected_pattern(clauses: &[Node]) -> Option<Vec<PatternComponent>> {
    let first = clauses.first()?.unwrap();
    let items = first.list()?;
    if items.first().and_then(Node::atom) != Some(">")
        || items.get(1).and_then(Node::atom) != Some("Block")
    {
        return None;
    }
    for statement in items.get(2)?.list()? {
        let statement = statement.unwrap();
        let Some(fields) = statement.list() else {
            continue;
        };
        if fields.first().and_then(Node::atom) != Some("Stmt")
            || fields.get(1).and_then(Node::atom) != Some("Decl")
        {
            continue;
        }
        let Some(pattern) = statement.field("pattern") else {
            continue;
        };
        let pattern = pattern.unwrap();
        let Some(parts) = pattern.list() else {
            continue;
        };
        if parts.first().and_then(Node::atom) != Some("Pattern")
            || parts.get(1).and_then(Node::atom) != Some("Constructor")
        {
            continue;
        }
        let Some(dt) = parts.get(2).and_then(Node::list) else {
            continue;
        };
        if dt.first().and_then(Node::atom) != Some("Dt")
            || dt.get(1).and_then(Node::atom) != Some("Tuple")
            || dt.get(2).and_then(Node::atom) == Some("0")
        {
            continue;
        }
        let mut components = Vec::new();
        for component in parts.get(4).and_then(Node::list).unwrap_or_default() {
            let Some(fields) = component.list() else {
                continue;
            };
            if fields.first().and_then(Node::atom) != Some("->") {
                continue;
            }
            let mut name = None;
            let mut typ = None;
            collect_pattern_binding(component, &mut name, &mut typ);
            components.push(PatternComponent { name, typ });
        }
        if !components.is_empty() {
            return Some(components);
        }
    }
    None
}

fn collect_pattern_binding(node: &Node, name: &mut Option<String>, typ: &mut Option<Node>) {
    let Some(items) = node.list() else {
        return;
    };
    if items.first().and_then(Node::atom) == Some("PatternBinding") {
        if let Some(value) = node.field("name").and_then(Node::list) {
            if value.first().and_then(Node::atom) == Some("VarIdent") {
                *name = value.get(1).and_then(Node::atom).map(str::to_owned);
            }
        }
        if let Some(value) = node.field("typ") {
            *typ = Some(value.clone());
        }
    }
    for child in items {
        collect_pattern_binding(child, name, typ);
    }
}

fn ensure_parts(node: &Node) -> (&[Node], &[Node]) {
    let Some(items) = node.list() else {
        return (&[], &[]);
    };
    if items.first().and_then(Node::atom) == Some("tuple") && items.len() == 3 {
        (
            items[1].list().unwrap_or(&[]),
            items[2].list().unwrap_or(&[]),
        )
    } else {
        (items, &[])
    }
}

fn collect_vars(node: &Node, names: &mut BTreeSet<String>) {
    let Some(items) = node.list() else {
        return;
    };
    if items.first().and_then(Node::atom) == Some("VarIdent") {
        if let Some(name) = items.get(1).and_then(Node::atom) {
            names.insert(name.to_owned());
        }
    }
    for child in items {
        collect_vars(child, names);
    }
}

fn collect_final_vars(node: &Node, names: &mut BTreeSet<String>) {
    let Some(items) = node.list() else {
        return;
    };
    if items.first().and_then(Node::atom) == Some(">")
        && items.get(1).and_then(Node::atom) == Some("Unary")
    {
        let operator = items.get(2).and_then(Node::list).unwrap_or_default();
        if operator.first().and_then(Node::atom) == Some("UnaryOp")
            && operator.get(1).and_then(Node::atom) == Some("MutRefFuture")
        {
            let source = operator.get(2).and_then(Node::list).unwrap_or_default();
            if source.first().and_then(Node::atom) == Some("MutRefFutureSourceName")
                && source.get(1).and_then(Node::atom) == Some("Final")
            {
                collect_vars(node, names);
            }
        }
    }
    for child in items {
        collect_final_vars(child, names);
    }
}

fn datatype_invariants(text: &str) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for datatype in vir::datatypes(text)? {
        let datatype = datatype.unwrap();
        let Some(name) = datatype.field("name").and_then(Node::list) else {
            continue;
        };
        if name.first().and_then(Node::atom) != Some("Dt")
            || name.get(1).and_then(Node::atom) != Some("Path")
        {
            continue;
        }
        let Some(path) = name.get(2).and_then(Node::atom) else {
            continue;
        };
        let Some(invariant) = datatype.field("user_defined_invariant_fn") else {
            continue;
        };
        if invariant.atom() == Some("None") {
            continue;
        }
        let Some(fun) = invariant.field("fun") else {
            continue;
        };
        let Some(invariant_path) = fun.field("path").and_then(Node::atom) else {
            continue;
        };
        let short = invariant_path.rsplit("::").next().unwrap_or(invariant_path);
        map.insert(path.to_owned(), short.to_owned());
    }
    Ok(map)
}

fn function_path(function: &Node) -> Option<&str> {
    function.field("name")?.field("path")?.atom()
}

fn kind(function: &Node) -> (&str, Option<String>, Option<String>) {
    let Some(node) = function.field("kind") else {
        return ("static", None, None);
    };
    let Some(items) = node.list() else {
        return ("static", None, None);
    };
    let name = items.get(1).and_then(Node::atom).unwrap_or("static");
    let method = node
        .field("method")
        .and_then(|m| m.field("path"))
        .and_then(Node::atom)
        .map(str::to_owned);
    let trait_path = node
        .field("trait_path")
        .and_then(Node::atom)
        .map(str::to_owned);
    let kind = match name {
        "TraitMethodImpl" => "trait_impl",
        "TraitMethodDecl" => "trait_decl",
        "TraitMethodDefault" => "trait_default",
        _ => "static",
    };
    (kind, method, trait_path)
}

fn visibility(function: &Node) -> Value {
    let Some(node) = function.field("visibility") else {
        return json!({"kind": "unknown", "restricted_to": null});
    };
    let restricted = node.field("restricted_to");
    if restricted.and_then(Node::atom) == Some("None") {
        json!({"kind": "pub", "restricted_to": null})
    } else {
        json!({
            "kind": "non_pub",
            "restricted_to": restricted.map(crate::contracts::render),
        })
    }
}

fn source(span: Option<&str>) -> Value {
    let Some(span) = span else {
        return json!({"span": null, "file": null, "line": null});
    };
    match crate::ir::span(span, Path::new("")) {
        Ok((file, coordinates)) => json!({
            "span": span,
            "file": if file.is_absolute() {
                file
            } else {
                normalize_relative(&file)
            }.to_string_lossy(),
            "line": coordinates[0],
        }),
        Err(_) => json!({"span": span, "file": null, "line": null}),
    }
}

fn type_decoration(node: &Node) -> Option<&str> {
    let items = node.list()?;
    if items.first().and_then(Node::atom) != Some("Typ")
        || items.get(1).and_then(Node::atom) != Some("Decorate")
    {
        return None;
    }
    items
        .get(2)
        .and_then(Node::list)
        .and_then(|d| d.get(1))
        .and_then(Node::atom)
}

fn type_is_reference(node: &Node) -> bool {
    let Some(items) = node.list() else {
        return false;
    };
    if items.first().and_then(Node::atom) != Some("Typ") {
        return false;
    }
    match items.get(1).and_then(Node::atom) {
        Some("MutRef") => true,
        Some("Decorate") => type_decoration(node) == Some("Ref"),
        _ => false,
    }
}

fn type_kind(node: Option<&Node>) -> &'static str {
    let Some(node) = node else {
        return "opaque";
    };
    if is_ghost_type(node) {
        return "ghost";
    }
    if type_is_reference(node) {
        return "reference";
    }
    let Some(items) = node.list() else {
        return "opaque";
    };
    if items.first().and_then(Node::atom) != Some("Typ") {
        return "opaque";
    }
    match items.get(1).and_then(Node::atom) {
        Some("Int") => "int",
        Some("Bool") => "bool",
        Some("Char") => "char",
        _ => "opaque",
    }
}

fn int_range_name(items: &[Node]) -> Option<String> {
    let range = items.get(2)?.list()?;
    match range.get(1)?.atom()? {
        "USize" => Some("usize".into()),
        "ISize" => Some("isize".into()),
        "Int" => Some("int".into()),
        "Nat" => Some("nat".into()),
        "U" => Some(format!("u{}", range.get(2)?.atom()?)),
        "I" => Some(format!("i{}", range.get(2)?.atom()?)),
        other => Some(other.to_owned()),
    }
}

fn render_type(node: &Node) -> String {
    let Some(items) = node.list() else {
        return render_compact(node);
    };
    if items.first().and_then(Node::atom) != Some("Typ") {
        return render_compact(node);
    }
    match items.get(1).and_then(Node::atom) {
        Some("Int") => int_range_name(items).unwrap_or_else(|| render_compact(node)),
        Some("Bool") => "bool".into(),
        Some("Char") => "char".into(),
        Some("MutRef") => items
            .get(2)
            .map(|inner| format!("&mut {}", render_type(inner)))
            .unwrap_or_else(|| render_compact(node)),
        Some("Decorate") => {
            let inner = items.get(4).map(render_type).unwrap_or_default();
            match type_decoration(node) {
                Some("Tracked") => format!("Tracked<{inner}>"),
                Some("Ghost") => format!("Ghost<{inner}>"),
                Some("Ref") => format!("&{inner}"),
                _ => render_compact(node),
            }
        }
        Some("Datatype") => {
            let dt = items.get(2).and_then(Node::list);
            let path = dt
                .and_then(|d| d.get(2))
                .and_then(Node::atom)
                .unwrap_or("?");
            let args = items.get(3).and_then(Node::list).unwrap_or_default();
            if args.is_empty() {
                path.replace("&%", "impl ")
            } else {
                let rendered = args.iter().map(render_type).collect::<Vec<_>>().join(", ");
                format!("{}<{rendered}>", path.replace("&%", "impl "))
            }
        }
        _ => render_compact(node),
    }
}

fn type_param_names(function: &Node) -> Vec<String> {
    function
        .field("typ_params")
        .and_then(Node::list)
        .map(|params| {
            params
                .iter()
                .filter_map(|param| {
                    param
                        .atom()
                        .map(str::to_owned)
                        .or_else(|| param.field("name").and_then(Node::atom).map(str::to_owned))
                        .or_else(|| {
                            param
                                .unwrap()
                                .field("name")
                                .and_then(Node::atom)
                                .map(str::to_owned)
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn param_facts(function: &Node) -> Vec<Value> {
    function
        .field("params")
        .and_then(Node::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|parameter| {
            let name = param_name(parameter)?;
            let typ = typ(Some(parameter));
            Some(json!({
                "name": name,
                "type": typ.map(render_type),
                "kind": type_kind(typ),
                "mode": parameter.field("mode").and_then(Node::atom).unwrap_or("").to_lowercase(),
            }))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Requires expression facts (M7e screening input)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mech {
    Int(i64),
    Bool(bool),
    Var(String),
    Const(String),
    Not(Box<Mech>),
    Neg(Box<Mech>),
    Bin(&'static str, Box<Mech>, Box<Mech>),
}

impl Mech {
    fn vars(&self, names: &mut BTreeSet<String>) {
        match self {
            Self::Var(name) => {
                names.insert(name.clone());
            }
            Self::Not(arg) | Self::Neg(arg) => arg.vars(names),
            Self::Bin(_, lhs, rhs) => {
                lhs.vars(names);
                rhs.vars(names);
            }
            Self::Int(_) | Self::Bool(_) | Self::Const(_) => {}
        }
    }

    fn consts(&self, paths: &mut BTreeSet<String>) {
        match self {
            Self::Const(path) => {
                paths.insert(path.clone());
            }
            Self::Not(arg) | Self::Neg(arg) => arg.consts(paths),
            Self::Bin(_, lhs, rhs) => {
                lhs.consts(paths);
                rhs.consts(paths);
            }
            Self::Int(_) | Self::Bool(_) | Self::Var(_) => {}
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Int(value) => json!({"op": "int", "value": value}),
            Self::Bool(value) => json!({"op": "bool", "value": value}),
            Self::Var(name) => json!({"op": "var", "name": name}),
            Self::Const(path) => json!({"op": "const", "path": path}),
            Self::Not(arg) => json!({"op": "not", "arg": arg.value()}),
            Self::Neg(arg) => json!({"op": "neg", "arg": arg.value()}),
            Self::Bin(sym, lhs, rhs) => {
                json!({"op": "bin", "sym": sym, "lhs": lhs.value(), "rhs": rhs.value()})
            }
        }
    }
}

fn mech(node: &Node) -> Option<Mech> {
    let node = node.unwrap();
    let items = node.list()?;
    if items.first().and_then(Node::atom) != Some(">") {
        return None;
    }
    match items.get(1).and_then(Node::atom)? {
        "Const" => {
            let constant = items.get(2)?.list()?;
            if constant.first().and_then(Node::atom) != Some("Constant") {
                return None;
            }
            match constant.get(1).and_then(Node::atom)? {
                "Int" => Some(Mech::Int(constant.get(2)?.atom()?.parse().ok()?)),
                "Bool" => match constant.get(2)?.atom()? {
                    "true" => Some(Mech::Bool(true)),
                    "false" => Some(Mech::Bool(false)),
                    _ => None,
                },
                _ => None,
            }
        }
        "ConstVar" => items
            .get(2)
            .and_then(|fun| fun.field("path"))
            .and_then(Node::atom)
            .map(|path| Mech::Const(path.to_owned())),
        "ReadPlace" => {
            let place = items.get(2)?.unwrap().list()?;
            if place.first().and_then(Node::atom) != Some("Place")
                || place.get(1).and_then(Node::atom) != Some("Local")
            {
                return None;
            }
            let name = place.get(2)?.list()?;
            if name.first().and_then(Node::atom) != Some("VarIdent") {
                return None;
            }
            Some(Mech::Var(name.get(1)?.atom()?.to_owned()))
        }
        "Var" => {
            let name = items.get(2)?.list()?;
            if name.first().and_then(Node::atom) != Some("VarIdent") {
                return None;
            }
            Some(Mech::Var(name.get(1)?.atom()?.to_owned()))
        }
        "Unary" => {
            let op = items.get(2)?.list()?;
            let arg = items.get(3)?;
            match op.get(1).and_then(Node::atom)? {
                "Not" => Some(Mech::Not(Box::new(mech(arg)?))),
                "Neg" => Some(Mech::Neg(Box::new(mech(arg)?))),
                _ => None,
            }
        }
        "Binary" => {
            let op = items.get(2)?.list()?;
            let lhs = mech(items.get(3)?)?;
            let rhs = mech(items.get(4)?)?;
            let sym = match op.get(1).and_then(Node::atom)? {
                "Inequality" => match op.get(2)?.list()?.get(1)?.atom()? {
                    "Lt" => "<",
                    "Le" => "<=",
                    "Gt" => ">",
                    "Ge" => ">=",
                    "Eq" => "==",
                    "Ne" => "!=",
                    _ => return None,
                },
                "Eq" => "==",
                "Arith" => match op.get(2)?.list()?.get(1)?.atom()? {
                    "Add" => "+",
                    "Sub" => "-",
                    "Mul" => "*",
                    _ => return None,
                },
                _ => return None,
            };
            Some(Mech::Bin(sym, Box::new(lhs), Box::new(rhs)))
        }
        "Logical" => {
            let op = items.get(2)?.list()?;
            let lhs = mech(items.get(3)?)?;
            let rhs = mech(items.get(4)?)?;
            let sym = match op.get(1).and_then(Node::atom)? {
                "And" => "&&",
                "Or" => "||",
                "Implies" => "==>",
                _ => return None,
            };
            Some(Mech::Bin(sym, Box::new(lhs), Box::new(rhs)))
        }
        "Multi" => {
            let op = items.get(2)?.list()?;
            if op.get(1).and_then(Node::atom) != Some("Chained") {
                return None;
            }
            let ops = op.get(2)?.list()?;
            let operands = items.get(3)?.list()?;
            if operands.len() != ops.len() + 1 {
                return None;
            }
            let mut result: Option<Mech> = None;
            for (index, chained) in ops.iter().enumerate() {
                let fields = chained.list()?;
                let inequality = fields.get(2)?.list()?;
                let sym = match inequality.get(1)?.atom()? {
                    "Lt" => "<",
                    "Le" => "<=",
                    "Gt" => ">",
                    "Ge" => ">=",
                    "Eq" => "==",
                    "Ne" => "!=",
                    _ => return None,
                };
                let lhs = mech(operands.get(index)?)?;
                let rhs = mech(operands.get(index + 1)?)?;
                let term = Mech::Bin(sym, Box::new(lhs), Box::new(rhs));
                result = Some(match result {
                    None => term,
                    Some(previous) => Mech::Bin("&&", Box::new(previous), Box::new(term)),
                });
            }
            result
        }
        _ => None,
    }
}

fn mech_vars(node: &Mech, names: &mut BTreeSet<String>) {
    node.vars(names);
}

/// Split a VIR expression node into its tag and the remaining fields,
/// stripping the `@`/`@@` source-span wrapper and the leading `>` marker.
fn expr_parts(node: &Node) -> Option<(&str, &[Node])> {
    let node = node.unwrap();
    let items = node.list()?;
    if items.first().and_then(Node::atom) == Some(">") {
        Some((items.get(1)?.atom()?, &items[2..]))
    } else {
        Some((items.first()?.atom()?, &items[1..]))
    }
}

fn dt_path(node: &Node) -> Option<String> {
    let items = node.unwrap().list()?;
    if items.first().and_then(Node::atom) != Some("Dt")
        || items.get(1).and_then(Node::atom) != Some("Path")
    {
        return None;
    }
    items.get(2).and_then(Node::atom).map(str::to_owned)
}

fn expr_var_ident(node: &Node) -> Option<String> {
    let items = node.unwrap().list()?;
    if items.first().and_then(Node::atom) != Some("VarIdent") {
        return None;
    }
    items.get(1).and_then(Node::atom).map(str::to_owned)
}

fn place_expr(node: &Node) -> Value {
    let Some((tag, rest)) = expr_parts(node) else {
        return json!({"op": "opaque", "tag": "place"});
    };
    match tag {
        "Place" => match rest.first().and_then(Node::atom) {
            Some("Local") => rest.get(1).and_then(expr_var_ident).map_or_else(
                || json!({"op": "opaque", "tag": "place"}),
                |name| json!({"op": "var", "name": name}),
            ),
            Some("Field") => {
                let field = rest
                    .get(1)
                    .and_then(|op| op.field("field"))
                    .and_then(Node::atom)
                    .map(str::to_owned);
                let datatype = rest
                    .get(1)
                    .and_then(|op| op.field("datatype"))
                    .and_then(dt_path);
                let root = rest
                    .get(2)
                    .map_or_else(|| json!({"op": "opaque", "tag": "field-root"}), place_expr);
                json!({
                    "op": "field",
                    "datatype": datatype,
                    "field": field,
                    "root": root,
                })
            }
            Some("Deref" | "DerefMut") => json!({
                "op": "deref",
                "arg": rest.get(1).map_or_else(
                    || json!({"op": "opaque", "tag": "deref"}),
                    place_expr,
                ),
            }),
            Some("Temporary") => rest
                .get(1)
                .map_or_else(|| json!({"op": "opaque", "tag": "temporary"}), clause_expr),
            _ => json!({"op": "opaque", "tag": "place"}),
        },
        "Var" => rest.first().and_then(expr_var_ident).map_or_else(
            || json!({"op": "opaque", "tag": "var"}),
            |name| json!({"op": "var", "name": name}),
        ),
        _ => json!({"op": "opaque", "tag": tag}),
    }
}

fn const_expr(node: &Node, rest: &[Node]) -> Value {
    if let Some(items) = rest.first().and_then(Node::list)
        && items.first().and_then(Node::atom) == Some("Constant")
    {
        let value = items.get(2).and_then(Node::atom).unwrap_or("");
        match items.get(1).and_then(Node::atom) {
            Some("Int") => {
                if let Ok(value) = value.parse::<i64>() {
                    return json!({"op": "int", "value": value});
                }
            }
            Some("Bool") => match value {
                "true" => return json!({"op": "bool", "value": true}),
                "false" => return json!({"op": "bool", "value": false}),
                _ => {}
            },
            _ => {}
        }
    }
    if let Some(value) = rest.first().and_then(Node::atom) {
        if let Ok(value) = value.parse::<i64>() {
            return json!({"op": "int", "value": value});
        }
        match value {
            "true" => return json!({"op": "bool", "value": true}),
            "false" => return json!({"op": "bool", "value": false}),
            _ => {}
        }
    }
    let path = rest
        .first()
        .and_then(|fun| fun.field("path"))
        .and_then(Node::atom)
        .or_else(|| node.field("path").and_then(Node::atom));
    if let Some(path) = path {
        return json!({"op": "const", "path": path});
    }
    json!({"op": "opaque", "tag": "const"})
}

fn inequality_sym(node: &Node) -> Option<&'static str> {
    let items = node.unwrap().list()?;
    match items.get(1).and_then(Node::atom)? {
        "Lt" => Some("<"),
        "Le" => Some("<="),
        "Gt" => Some(">"),
        "Ge" => Some(">="),
        "Eq" => Some("=="),
        "Ne" => Some("!="),
        _ => None,
    }
}

fn binary_sym(op: &Node) -> Option<&'static str> {
    let items = op.unwrap().list()?;
    match items.first().and_then(Node::atom)? {
        "BinaryOp" => match items.get(1).and_then(Node::atom)? {
            "Eq" => Some("=="),
            "Inequality" => inequality_sym(items.get(2)?),
            "Arith" => match items.get(2).and_then(Node::list)?.get(1)?.atom()? {
                "Add" => Some("+"),
                "Sub" => Some("-"),
                "Mul" => Some("*"),
                _ => None,
            },
            _ => None,
        },
        "LogicalOp" => match items.get(1).and_then(Node::atom)? {
            "And" => Some("&&"),
            "Or" => Some("||"),
            "Implies" => Some("==>"),
            _ => None,
        },
        "ChainedOp" => inequality_sym(items.get(2)?),
        "InequalityOp" => inequality_sym(op),
        _ => None,
    }
}

/// Generic, best-effort expression tree for a requires clause. This is the
/// input to the project-side projection translation in `requires`; it is not
/// a judgement and it never interprets a project path.
fn clause_expr(node: &Node) -> Value {
    let Some((tag, rest)) = expr_parts(node) else {
        return json!({"op": "opaque", "tag": "atom"});
    };
    match tag {
        "Const" | "ConstVar" => const_expr(node.unwrap(), rest),
        "Var" => rest.first().and_then(expr_var_ident).map_or_else(
            || json!({"op": "opaque", "tag": "var"}),
            |name| json!({"op": "var", "name": name}),
        ),
        "Place" => place_expr(node),
        "ReadPlace" => json!({
            "op": "read",
            "arg": rest.first().map_or_else(
                || json!({"op": "opaque", "tag": "read"}),
                place_expr,
            ),
        }),
        "Old" => json!({
            "op": "old",
            "arg": rest.first().map_or_else(
                || json!({"op": "opaque", "tag": "old"}),
                clause_expr,
            ),
        }),
        "Unary" => {
            let op = rest.first().and_then(Node::list).unwrap_or_default();
            let arg = rest.get(1).map(clause_expr);
            match op.get(1).and_then(Node::atom) {
                Some("Not") => json!({"op": "not", "arg": arg}),
                Some("Neg") => json!({"op": "neg", "arg": arg}),
                Some("Trigger" | "CoerceMode" | "Cast") => {
                    arg.unwrap_or_else(|| json!({"op": "opaque", "tag": "unary"}))
                }
                Some("MutRefFuture") => json!({"op": "final", "arg": arg}),
                Some("IsVariant") => json!({
                    "op": "is_variant",
                    "datatype": node.unwrap().field("datatype").and_then(dt_path),
                    "variant": node.unwrap().field("variant").and_then(Node::atom),
                    "arg": arg,
                }),
                _ => json!({"op": "opaque", "tag": "unary"}),
            }
        }
        "UnaryOpr" => {
            let op_node = rest.first();
            let op = op_node.and_then(Node::list).unwrap_or_default();
            match op.get(1).and_then(Node::atom) {
                Some("IsVariant") => json!({
                    "op": "is_variant",
                    "datatype": op_node.and_then(|op| op.field("datatype")).and_then(dt_path),
                    "variant": op_node.and_then(|op| op.field("variant")).and_then(Node::atom),
                    "arg": rest.get(1).map(clause_expr),
                }),
                Some("IntegerTypeBound") => json!({
                    "op": "const",
                    "path": format!("$integer_type_bound[{}]", op.get(2).map(render_compact).unwrap_or_default()),
                }),
                other => json!({"op": "opaque", "tag": other.unwrap_or("unary_opr")}),
            }
        }
        "Binary" => {
            let lhs = rest.get(1).map(clause_expr);
            let rhs = rest.get(2).map(clause_expr);
            match (rest.first().and_then(binary_sym), lhs, rhs) {
                (Some(sym), Some(lhs), Some(rhs)) => {
                    json!({"op": "bin", "sym": sym, "lhs": lhs, "rhs": rhs})
                }
                _ => json!({"op": "opaque", "tag": "binary"}),
            }
        }
        "Logical" => {
            let lhs = rest.get(1).map(clause_expr);
            let rhs = rest.get(2).map(clause_expr);
            match (rest.first().and_then(binary_sym), lhs, rhs) {
                (Some(sym), Some(lhs), Some(rhs)) => {
                    json!({"op": "bin", "sym": sym, "lhs": lhs, "rhs": rhs})
                }
                _ => json!({"op": "opaque", "tag": "logical"}),
            }
        }
        "Multi" => {
            let op = rest.first().and_then(Node::list).unwrap_or_default();
            if op.get(1).and_then(Node::atom) != Some("Chained") {
                return json!({"op": "opaque", "tag": "multi"});
            }
            let ops = op.get(2).and_then(Node::list).unwrap_or_default();
            let operands = rest.get(1).and_then(Node::list).unwrap_or_default();
            if operands.len() != ops.len() + 1 {
                return json!({"op": "opaque", "tag": "multi"});
            }
            let mut result: Option<Value> = None;
            for (index, chained) in ops.iter().enumerate() {
                let sym = binary_sym(chained);
                let Some(sym) = sym else {
                    return json!({"op": "opaque", "tag": "multi"});
                };
                let lhs = clause_expr(operands.get(index).unwrap_or(&Node::Atom(String::new())));
                let rhs = clause_expr(
                    operands
                        .get(index + 1)
                        .unwrap_or(&Node::Atom(String::new())),
                );
                let term = json!({"op": "bin", "sym": sym, "lhs": lhs, "rhs": rhs});
                result = Some(match result {
                    None => term,
                    Some(previous) => {
                        json!({"op": "bin", "sym": "&&", "lhs": previous, "rhs": term})
                    }
                });
            }
            result.unwrap_or_else(|| json!({"op": "opaque", "tag": "multi"}))
        }
        "WithTriggers" => rest.last().map_or_else(
            || json!({"op": "opaque", "tag": "with_triggers"}),
            clause_expr,
        ),
        "Call" => {
            let args = node
                .field("args")
                .and_then(Node::list)
                .unwrap_or_default()
                .iter()
                .map(clause_expr)
                .collect::<Vec<_>>();
            json!({
                "op": "call",
                "callee": call_path(node),
                "dynamic": call_is_dynamic(node),
                "args": args,
            })
        }
        "Ctor" => {
            let fields = rest.get(2).and_then(Node::list).unwrap_or_default();
            let mut args = Vec::new();
            for field in fields {
                let items = field.unwrap().list().unwrap_or_default();
                if items.first().and_then(Node::atom) == Some("->")
                    && let Some(value) = items.get(2)
                {
                    args.push(clause_expr(value));
                }
            }
            json!({
                "op": "ctor",
                "datatype": rest.first().map(render_compact),
                "variant": rest.get(1).and_then(Node::atom),
                "args": args,
            })
        }
        "Quant" => {
            let forall = rest.first().and_then(Node::atom) == Some("Forall");
            let mut bindings = Vec::new();
            if let Some(items) = rest.get(1).and_then(Node::list) {
                for binding in items {
                    let fields = binding.unwrap().list().unwrap_or_default();
                    if let Some(name) = fields.get(1).and_then(expr_var_ident) {
                        let typ = fields.get(2).map(render_compact).unwrap_or_default();
                        bindings.push(json!({"name": name, "type": typ}));
                    }
                }
            }
            json!({
                "op": "quant",
                "forall": forall,
                "bindings": bindings,
                "body": rest.get(2).map_or_else(
                    || json!({"op": "opaque", "tag": "quant-body"}),
                    clause_expr,
                ),
            })
        }
        other => json!({"op": "opaque", "tag": other}),
    }
}

#[derive(Debug, Default)]
struct ClauseFeatures {
    call: bool,
    field_access: bool,
    quantifier: bool,
    old_or_final: bool,
    unary_opr: bool,
    ctor: bool,
    trigger: bool,
}

fn scan_features(node: &Node, features: &mut ClauseFeatures) {
    let Some(items) = node.list() else {
        return;
    };
    if items.first().and_then(Node::atom) == Some(">") {
        match items.get(1).and_then(Node::atom) {
            Some("Call") => features.call = true,
            Some("Ctor") => features.ctor = true,
            Some("Quant") => features.quantifier = true,
            Some("UnaryOpr") => features.unary_opr = true,
            Some("Old") => features.old_or_final = true,
            Some("Unary") => {
                let op = items.get(2).and_then(Node::list).unwrap_or_default();
                match op.get(1).and_then(Node::atom) {
                    Some("MutRefFuture") => features.old_or_final = true,
                    Some("Trigger") => features.trigger = true,
                    _ => {}
                }
            }
            Some("ReadPlace") => {
                let place = items
                    .get(2)
                    .map(Node::unwrap)
                    .and_then(Node::list)
                    .unwrap_or_default();
                if place.get(1).and_then(Node::atom) == Some("Field") {
                    features.field_access = true;
                }
            }
            _ => {}
        }
    }
    for child in items {
        scan_features(child, features);
    }
}

fn clause_source(
    span: Option<&str>,
    root: Option<&Path>,
    cache: &mut BTreeMap<PathBuf, String>,
) -> Option<String> {
    let root = root?;
    let span = span?;
    let (file, coordinates) = crate::ir::span(span, root).ok()?;
    let text = cache
        .entry(file.clone())
        .or_insert_with(|| fs::read_to_string(&file).unwrap_or_default());
    if text.is_empty() {
        return None;
    }
    let (start, end) = crate::ir::offsets(text, coordinates).ok()?;
    text.get(start..end).map(str::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlaceSegment {
    Field(String),
    Deref,
}

#[derive(Debug, Clone)]
struct ParamUse {
    name: String,
    kind: &'static str,
    callee: Option<String>,
    field: Option<String>,
    argc: Option<usize>,
}

impl ParamUse {
    fn value(&self) -> Value {
        json!({
            "name": self.name,
            "kind": self.kind,
            "callee": self.callee,
            "field": self.field,
            "argc": self.argc,
        })
    }
}

fn var_ident_name(node: &Node) -> Option<String> {
    let items = node.list()?;
    if items.first().and_then(Node::atom) != Some("VarIdent") {
        return None;
    }
    items.get(1).and_then(Node::atom).map(str::to_owned)
}

fn is_mutref_future_unary(node: &Node) -> bool {
    let node = node.unwrap();
    let Some(items) = node.list() else {
        return false;
    };
    if items.first().and_then(Node::atom) != Some(">")
        || items.get(1).and_then(Node::atom) != Some("Unary")
    {
        return false;
    }
    items
        .get(2)
        .and_then(Node::list)
        .and_then(|op| op.get(1))
        .and_then(Node::atom)
        == Some("MutRefFuture")
}

fn place_root(node: &Node) -> Option<(String, Vec<PlaceSegment>)> {
    let node = node.unwrap();
    let items = node.list()?;
    match items.first().and_then(Node::atom)? {
        "Place" => match items.get(1).and_then(Node::atom)? {
            "Local" => var_ident_name(items.get(2)?).map(|name| (name, Vec::new())),
            "Field" => {
                let field = items
                    .get(2)?
                    .field("field")
                    .and_then(Node::atom)?
                    .to_owned();
                let (name, mut segments) = place_root(items.get(3)?)?;
                segments.push(PlaceSegment::Field(field));
                Some((name, segments))
            }
            "Deref" | "DerefMut" => {
                let (name, mut segments) = place_root(items.get(2)?)?;
                segments.push(PlaceSegment::Deref);
                Some((name, segments))
            }
            "Temporary" => expr_root(items.get(2)?),
            _ => None,
        },
        "ReadPlace" => place_root(items.get(2)?),
        "Old" => expr_root(items.get(2)?),
        "Var" => var_ident_name(items.get(1)?).map(|name| (name, Vec::new())),
        _ => None,
    }
}

fn expr_root(node: &Node) -> Option<(String, Vec<PlaceSegment>)> {
    let node = node.unwrap();
    let items = node.list()?;
    let head = items.first().and_then(Node::atom)?;
    if head == ">" {
        return match items.get(1).and_then(Node::atom)? {
            "ReadPlace" => place_root(items.get(2)?),
            "Old" => expr_root(items.get(2)?),
            "Unary" if is_mutref_future_unary(node) => expr_root(items.get(3)?),
            _ => None,
        };
    }
    match head {
        "Place" | "Var" => place_root(node),
        _ => None,
    }
}

fn call_args(node: &Node) -> Option<Vec<Node>> {
    let node = node.unwrap();
    let items = node.list()?;
    if items.first().and_then(Node::atom) != Some(">")
        || items.get(1).and_then(Node::atom) != Some("Call")
    {
        return None;
    }
    Some(
        node.field("args")
            .and_then(Node::list)
            .unwrap_or_default()
            .to_vec(),
    )
}

fn node_contains_atom(node: &Node, wanted: &str) -> bool {
    if node.atom() == Some(wanted) {
        return true;
    }
    node.list()
        .is_some_and(|items| items.iter().any(|child| node_contains_atom(child, wanted)))
}

fn call_is_dynamic(node: &Node) -> bool {
    node.unwrap()
        .field("target")
        .is_some_and(|target| node_contains_atom(target, "DynamicResolved"))
}

fn call_path(node: &Node) -> Option<String> {
    let mut paths = Vec::new();
    vir::walk_calls(node.unwrap().field("target")?, &mut paths);
    paths.into_iter().next()
}

fn is_read_place(node: &Node) -> bool {
    let node = node.unwrap();
    node.list()
        .and_then(|items| items.get(1))
        .and_then(Node::atom)
        == Some("ReadPlace")
}

fn classify_use(
    root: &(String, Vec<PlaceSegment>),
    params: &BTreeSet<String>,
    uses: &mut Vec<ParamUse>,
) -> bool {
    let (name, segments) = root;
    if !params.contains(name) {
        return false;
    }
    let field = segments.iter().rev().find_map(|segment| match segment {
        PlaceSegment::Field(field) => Some(field.clone()),
        PlaceSegment::Deref => None,
    });
    let kind = if segments
        .iter()
        .any(|segment| matches!(segment, PlaceSegment::Field(_)))
    {
        "field_root"
    } else if segments
        .iter()
        .any(|segment| matches!(segment, PlaceSegment::Deref))
    {
        "deref_root"
    } else {
        "plain_root"
    };
    uses.push(ParamUse {
        name: name.clone(),
        kind,
        callee: None,
        field,
        argc: None,
    });
    true
}

fn walk_param_uses(node: &Node, params: &BTreeSet<String>, uses: &mut Vec<ParamUse>) {
    let node = node.unwrap();
    if let Some(args) = call_args(node) {
        if call_is_dynamic(node) {
            if let Some(root) = args.first().and_then(expr_root) {
                if params.contains(&root.0) {
                    let field = root.1.iter().rev().find_map(|segment| match segment {
                        PlaceSegment::Field(field) => Some(field.clone()),
                        PlaceSegment::Deref => None,
                    });
                    uses.push(ParamUse {
                        name: root.0,
                        kind: "receiver_root",
                        callee: call_path(node),
                        field,
                        argc: Some(args.len()),
                    });
                    for argument in args.iter().skip(1) {
                        walk_param_uses(argument, params, uses);
                    }
                    return;
                }
            }
        }
        for argument in &args {
            walk_param_uses(argument, params, uses);
        }
        return;
    }
    if is_read_place(node) {
        if let Some(root) = expr_root(node)
            && classify_use(&root, params, uses)
        {
            return;
        }
    }
    for child in node.list().unwrap_or_default() {
        walk_param_uses(child, params, uses);
    }
}

fn count_param_occurrences(node: &Node, name: &str, count: &mut usize) {
    let node = node.unwrap();
    if let Some(items) = node.list() {
        if items.first().and_then(Node::atom) == Some("Place")
            && items.get(1).and_then(Node::atom) == Some("Local")
            && items.get(2).and_then(var_ident_name).as_deref() == Some(name)
        {
            *count += 1;
        }
        if items.first().and_then(Node::atom) == Some("Var")
            && items.get(1).and_then(var_ident_name).as_deref() == Some(name)
        {
            *count += 1;
        }
        for child in items {
            count_param_occurrences(child, name, count);
        }
    }
}

fn param_use_facts(
    clause: &Node,
    param_names: &BTreeSet<String>,
) -> (BTreeMap<String, usize>, Vec<Value>) {
    let mut occurrences = BTreeMap::new();
    for name in param_names {
        let mut count = 0usize;
        count_param_occurrences(clause, name, &mut count);
        if count > 0 {
            occurrences.insert(name.clone(), count);
        }
    }
    let mut uses = Vec::new();
    walk_param_uses(clause, param_names, &mut uses);
    (occurrences, uses.iter().map(ParamUse::value).collect())
}

fn requires_facts(
    function: &Node,
    scalar_names: &BTreeSet<String>,
    param_names: &BTreeSet<String>,
    source_root: Option<&Path>,
    cache: &mut BTreeMap<PathBuf, String>,
) -> Vec<Value> {
    function
        .field("require")
        .and_then(Node::list)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, clause)| {
            let span = clause
                .list()
                .and_then(|items| items.get(1))
                .and_then(Node::atom);
            let mut features = ClauseFeatures::default();
            scan_features(clause, &mut features);
            let mut vars = BTreeSet::new();
            collect_vars(clause, &mut vars);
            let mut calls = Vec::new();
            vir::walk_calls(clause, &mut calls);
            calls.sort();
            calls.dedup();
            let mut consts = BTreeSet::new();
            collect_const_vars(clause, &mut consts);
            let (param_occurrences, param_uses) = param_use_facts(clause, param_names);
            let parsed = mech(clause).filter(|expression| {
                let mut names = BTreeSet::new();
                mech_vars(expression, &mut names);
                names.iter().all(|name| scalar_names.contains(name))
            });
            let mut expression_vars = BTreeSet::new();
            if let Some(expression) = &parsed {
                mech_vars(expression, &mut expression_vars);
            }
            let mut expression_consts = BTreeSet::new();
            if let Some(expression) = &parsed {
                expression.consts(&mut expression_consts);
            }
            json!({
                "index": index,
                "span": span,
                "file": span.and_then(|span| saga_span_file(span)),
                "line": span.and_then(|span| saga_span_line(span)),
                "text": clause_source(span, source_root, cache),
                "class": if parsed.is_some() { "mechanical" } else { "unsupported" },
                "ast": parsed.as_ref().map(Mech::value),
                "expr": clause_expr(clause),
                "ast_vars": expression_vars,
                "ast_consts": expression_consts,
                "vars": vars,
                "consts": consts,
                "calls": calls,
                "param_occurrences": param_occurrences,
                "param_uses": param_uses,
                "features": {
                    "call": features.call,
                    "field_access": features.field_access,
                    "quantifier": features.quantifier,
                    "old_or_final": features.old_or_final,
                    "unary_opr": features.unary_opr,
                    "ctor": features.ctor,
                    "trigger": features.trigger,
                },
            })
        })
        .collect()
}

fn saga_span_file(span: &str) -> Option<String> {
    crate::ir::span(span, Path::new(""))
        .ok()
        .map(|(file, _)| file.to_string_lossy().into_owned())
}

fn saga_span_line(span: &str) -> Option<usize> {
    crate::ir::span(span, Path::new(""))
        .ok()
        .map(|(_, coordinates)| coordinates[0])
}

fn collect_const_vars(node: &Node, paths: &mut BTreeSet<String>) {
    let Some(items) = node.list() else {
        return;
    };
    if items.first().and_then(Node::atom) == Some(">")
        && items.get(1).and_then(Node::atom) == Some("ConstVar")
    {
        if let Some(path) = items
            .get(2)
            .and_then(|fun| fun.field("path"))
            .and_then(Node::atom)
        {
            paths.insert(path.to_owned());
        }
    }
    for child in items {
        collect_const_vars(child, paths);
    }
}

fn previews(clauses: &[Node]) -> (usize, Vec<String>) {
    let mut result = Vec::new();
    for clause in clauses.iter().take(3) {
        let mut text = render_compact(clause);
        if text.chars().count() > 300 {
            text = text.chars().take(300).collect::<String>() + "...";
        }
        result.push(text);
    }
    (clauses.len(), result)
}

fn output(
    kind: &str,
    name: Option<String>,
    mention_name: Option<String>,
    named: bool,
    typ: Option<Node>,
    ghost: bool,
    mentions: &MentionSets,
    invariants: &BTreeMap<String, String>,
    returns_mention: bool,
) -> Value {
    let mentioned = match kind {
        "mut_param" => mention_name
            .as_ref()
            .is_some_and(|name| mentions.final_vars.contains(name)),
        _ => {
            (returns_mention && kind == "return")
                || mention_name
                    .as_ref()
                    .is_some_and(|name| mentions.vars.contains(name))
        }
    };
    let default_mention = mention_name.as_ref().is_some_and(|name| {
        if kind == "mut_param" {
            mentions.default_final_vars.contains(name)
        } else {
            mentions.default_vars.contains(name)
        }
    });
    let type_invariant = typ
        .as_ref()
        .and_then(type_path)
        .and_then(|path| invariants.get(&path).cloned());
    let finding = if mentioned {
        None
    } else if kind == "mut_param" {
        Some("mut_state_unmentioned")
    } else {
        Some("return_unmentioned")
    };
    let type_invariant_only = finding.is_some() && type_invariant.is_some();
    json!({
        "kind": kind,
        "name": if named { name } else { None },
        "mention_name": mention_name,
        "named": named,
        "type": typ.as_ref().map(render_compact),
        "ghost": ghost,
        "mentioned": mentioned,
        "mention": if mentioned {
            if kind == "mut_param" { "final" }
            else if returns_mention && kind == "return" { "returns" }
            else { "variable" }
        } else { "none" },
        "default_ensures_mention": default_mention,
        "type_invariant": type_invariant,
        "type_invariant_only": type_invariant_only,
        "finding": finding,
    })
}

struct MentionSets {
    vars: BTreeSet<String>,
    final_vars: BTreeSet<String>,
    default_vars: BTreeSet<String>,
    default_final_vars: BTreeSet<String>,
}

#[allow(clippy::too_many_lines)]
pub fn analyze(text: &str, crate_name: &str, source_root: Option<&Path>) -> Result<Value> {
    let functions = vir::functions(text)?;
    let invariants = datatype_invariants(text)?;
    let mut records = Vec::new();
    let mut by_path: BTreeMap<String, &Node> = BTreeMap::new();
    for outer in &functions {
        let function = outer.unwrap();
        let Some(path) = function_path(function) else {
            continue;
        };
        by_path.entry(path.to_owned()).or_insert(function);
        let mode = function.field("mode").and_then(Node::atom).unwrap_or("");
        if !matches!(mode, "Exec" | "Proof") {
            continue;
        }
        if path != crate_name
            && !path
                .strip_prefix(crate_name)
                .is_some_and(|rest| rest.starts_with("::"))
        {
            continue;
        }
        records.push((outer, function, path.to_owned()));
    }

    let mut canonical: BTreeMap<String, (&Node, &Node)> = BTreeMap::new();
    for (outer, function, path) in &records {
        let declared = path.replace("::_VERUS_VERIFIED_", "::");
        let verified = path.contains("::_VERUS_VERIFIED_");
        canonical
            .entry(declared)
            .and_modify(|slot| {
                let current_verified =
                    function_path(slot.1).is_some_and(|p| p.contains("::_VERUS_VERIFIED_"));
                let current_body = slot
                    .1
                    .field("body")
                    .is_some_and(|body| body.list().is_some());
                let candidate_body = function
                    .field("body")
                    .is_some_and(|body| body.list().is_some());
                if verified && (!current_verified || (!current_body && candidate_body)) {
                    *slot = (outer, function);
                }
            })
            .or_insert((outer, function));
    }

    let mut entries = Vec::new();
    let mut counts = Counts::default();
    let mut source_cache = BTreeMap::new();
    for (path, (outer, function)) in &canonical {
        let entry = function_entry(
            path,
            outer,
            function,
            &by_path,
            &invariants,
            source_root,
            &mut source_cache,
            &mut counts,
        );
        entries.push(entry);
    }

    let source_root = source_root.map(|p| p.to_string_lossy().into_owned());
    Ok(json!({
        "schema": SCHEMA,
        "crate": crate_name,
        "source_root": source_root,
        "scope": "Exec and Proof functions whose VIR path starts with `<crate>::`; _VERUS_VERIFIED_* records canonicalized to declared paths; Spec functions and external crates excluded.",
        "counts": counts.value(),
        "functions": entries,
        "limits": [
            "VIR-only: no source parser and no project-specific paths; visibility/trait/type facts are taken from VIR records.",
            "Outputs are the return value, #[verus_spec] injected return tuple components (unit actual return omitted), and every &mut parameter.",
            "Trait impl postconditions merge the trait declaration's regular ensures/returns; default_ensures are excluded and annotated separately.",
            "A mention is syntactic; conditional postconditions count even when their condition is false.",
            "Unmentioned outputs are facts, not defect judgements.",
        ],
    }))
}

fn function_entry(
    path: &str,
    outer: &Node,
    function: &Node,
    by_path: &BTreeMap<String, &Node>,
    invariants: &BTreeMap<String, String>,
    source_root: Option<&Path>,
    source_cache: &mut BTreeMap<PathBuf, String>,
    counts: &mut Counts,
) -> Value {
    let mode = function.field("mode").and_then(Node::atom).unwrap_or("");
    let (kind, method, trait_path) = kind(function);
    let own_regular = function
        .field("ensure")
        .map(ensure_parts)
        .map(|(regular, _)| regular)
        .unwrap_or_default();
    let own_defaults = function
        .field("ensure")
        .map(ensure_parts)
        .map(|(_, defaults)| defaults)
        .unwrap_or_default();
    let own_returns = function
        .field("returns")
        .filter(|node| node.atom() != Some("None"));
    let declaration = method
        .as_ref()
        .and_then(|method| by_path.get(method).copied());
    let decl_regular = declaration
        .and_then(|d| d.field("ensure"))
        .map(ensure_parts)
        .map(|(regular, _)| regular)
        .unwrap_or_default();
    let decl_defaults = declaration
        .and_then(|d| d.field("ensure"))
        .map(ensure_parts)
        .map(|(_, defaults)| defaults)
        .unwrap_or_default();
    let decl_returns = declaration
        .and_then(|d| d.field("returns"))
        .filter(|node| node.atom() != Some("None"));
    let decl_unavailable = kind == "trait_impl" && declaration.is_none();
    let own_present = !own_regular.is_empty() || own_returns.is_some();
    let decl_present = !decl_regular.is_empty() || decl_returns.is_some();
    let post_present = own_present || decl_present;
    let post_source = match (own_present, decl_present, decl_unavailable) {
        (true, true, _) => "own_decl",
        (true, false, _) => "own",
        (false, true, _) => "decl",
        (false, false, true) => "decl_unavailable",
        (false, false, false) => "none",
    };
    let default_present = !own_defaults.is_empty() || !decl_defaults.is_empty();
    let default_only = !post_present && default_present;

    let mut vars = BTreeSet::new();
    let mut final_vars = BTreeSet::new();
    let mut default_vars = BTreeSet::new();
    let mut default_final_vars = BTreeSet::new();
    let mut returns_mention = own_returns.is_some() || decl_returns.is_some();
    for clause in own_regular.iter().chain(decl_regular.iter()) {
        collect_vars(clause, &mut vars);
        collect_final_vars(clause, &mut final_vars);
    }
    for node in own_returns.iter().chain(decl_returns.iter()) {
        collect_vars(node, &mut vars);
        collect_final_vars(node, &mut final_vars);
    }
    for clause in own_defaults.iter().chain(decl_defaults.iter()) {
        collect_vars(clause, &mut default_vars);
        collect_final_vars(clause, &mut default_final_vars);
    }
    if own_returns.is_none() && decl_returns.is_none() {
        returns_mention = false;
    }
    let mentions = MentionSets {
        vars,
        final_vars,
        default_vars,
        default_final_vars,
    };

    let pattern = find_injected_pattern(own_regular).map(|c| (c, "own"));
    let mut outputs = Vec::new();
    if let Some((components, _)) = &pattern {
        for (index, component) in components.iter().enumerate() {
            let typ = component.typ.as_ref().or_else(|| {
                if index == 0 {
                    function
                        .field("inner_ret")
                        .and_then(|p| p.unwrap().field("typ"))
                } else {
                    None
                }
            });
            let unit = typ.is_some_and(type_is_unit);
            if index == 0 && unit {
                continue;
            }
            let name = component.name.clone();
            let named = name.as_deref().is_some_and(|n| !n.starts_with('%'));
            let ghost = typ.is_some_and(is_ghost_type) || index > 0;
            outputs.push(output(
                if index == 0 { "return" } else { "ghost_return" },
                name.clone(),
                name,
                named,
                typ.cloned(),
                ghost,
                &mentions,
                invariants,
                returns_mention,
            ));
        }
    } else {
        if let Some(return_type) = function
            .field("inner_ret")
            .and_then(|p| p.unwrap().field("typ"))
        {
            if !type_is_unit(return_type) {
                let impl_name = function.field("inner_ret").and_then(param_name);
                let decl_name = declaration
                    .and_then(|d| d.field("inner_ret"))
                    .and_then(param_name);
                let mention_name = if !own_present && decl_present {
                    decl_name.clone().or_else(|| impl_name.clone())
                } else {
                    impl_name.clone()
                };
                let named = mention_name.as_deref().is_some_and(|n| {
                    n != "%return" && n != "_VERUS_ret_ident" && !n.starts_with('%')
                });
                outputs.push(output(
                    "return",
                    mention_name.clone(),
                    mention_name,
                    named,
                    Some(return_type.clone()),
                    is_ghost_type(return_type),
                    &mentions,
                    invariants,
                    returns_mention,
                ));
            }
        }
    }
    for (index, parameter) in function
        .field("params")
        .and_then(Node::list)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let Some(parameter_type) = typ(Some(parameter)) else {
            continue;
        };
        if !type_contains_mutref(parameter_type) {
            continue;
        }
        let name = param_name(parameter);
        let mention_name = if !own_present && decl_present {
            declaration
                .and_then(|d| d.field("params"))
                .and_then(Node::list)
                .and_then(|params| params.get(index))
                .and_then(param_name)
                .or_else(|| name.clone())
        } else {
            name.clone()
        };
        let ghost = is_ghost_type(parameter_type);
        outputs.push(output(
            "mut_param",
            name,
            mention_name,
            true,
            Some(parameter_type.clone()),
            ghost,
            &mentions,
            invariants,
            false,
        ));
    }

    let findings = outputs
        .iter()
        .filter_map(|output| output["finding"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let function_present = post_present;
    let mut output_count = 0usize;
    let mut finding_count = 0usize;
    let mut return_findings = 0usize;
    let mut mut_findings = 0usize;
    let mut type_invariant_only = 0usize;
    for output in &outputs {
        output_count += 1;
        if let Some(finding) = output["finding"].as_str() {
            finding_count += 1;
            match finding {
                "mut_state_unmentioned" => mut_findings += 1,
                _ => return_findings += 1,
            }
            if output["type_invariant_only"] == true {
                type_invariant_only += 1;
            }
        }
    }
    counts.record(
        mode,
        visibility(function)["kind"].as_str().unwrap_or("unknown"),
        function
            .field("attrs")
            .and_then(|attrs| attrs.field("is_external_body"))
            .and_then(Node::atom)
            == Some("true"),
        kind,
        decl_unavailable,
        function_present,
        output_count,
        finding_count,
        return_findings,
        mut_findings,
        type_invariant_only,
    );

    let requires = function
        .field("require")
        .and_then(Node::list)
        .unwrap_or_default();
    let (requires_count, requires_preview) = previews(requires);
    let params = param_facts(function);
    let param_names = params
        .iter()
        .filter_map(|param| param["name"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let scalar_names = params
        .iter()
        .filter(|param| matches!(param["kind"].as_str(), Some("int" | "bool")))
        .filter_map(|param| param["name"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let requires_detail = requires_facts(
        function,
        &scalar_names,
        &param_names,
        source_root,
        source_cache,
    );
    let generics = type_param_names(function);
    let span = outer
        .list()
        .and_then(|items| items.get(1))
        .and_then(Node::atom);
    let mut source = source(span);
    if let (Some(root), Some(file)) = (source_root, source["file"].as_str()) {
        let root = root.to_string_lossy();
        if let Some(relative) = file.strip_prefix(&format!("{}/", root.trim_end_matches('/'))) {
            source["file"] = json!(relative);
        }
    }
    let module = function
        .field("owning_module")
        .and_then(Node::atom)
        .map(|m| m.trim_end_matches("::").to_owned());
    let display_name = path.rsplit("::").next().unwrap_or(path);
    json!({
        "path": path,
        "name": display_name,
        "mode": mode.to_lowercase(),
        "kind": kind,
        "visibility": visibility(function),
        "source": source,
        "external_body": function
            .field("attrs")
            .and_then(|attrs| attrs.field("is_external_body"))
            .and_then(Node::atom)
            == Some("true"),
        "trait": if kind == "trait_impl" {
            json!({"path": trait_path, "method": method, "post_source": post_source})
        } else {
            Value::Null
        },
        "module": module,
        "generics": {
            "count": generics.len(),
            "names": generics,
        },
        "params": params,
        "requires": requires_detail,
        "pattern": pattern.as_ref().map(|(components, source)| json!({"found": true, "source": source, "components": components.len()})),
        "postcondition": {
            "present": post_present,
            "source": post_source,
            "own_clauses": own_regular.len(),
            "decl_clauses": decl_regular.len(),
            "default_clauses": own_defaults.len() + decl_defaults.len(),
            "returns": own_returns.is_some(),
            "decl_returns": decl_returns.is_some(),
            "default_ensures_only": default_only,
            "decl_unavailable": decl_unavailable,
            "requires_count": requires_count,
            "requires_preview": requires_preview,
        },
        "outputs": outputs,
        "findings": findings,
    })
}

#[derive(Default)]
struct Counts {
    functions: usize,
    exec: usize,
    proof: usize,
    pub_functions: usize,
    non_pub: usize,
    external_body: usize,
    trait_impl: usize,
    trait_decl: usize,
    decl_unavailable: usize,
    with_functions: usize,
    with_outputs: usize,
    with_findings: usize,
    with_return: usize,
    with_mut: usize,
    with_type_invariant_only: usize,
    without_functions: usize,
    without_outputs: usize,
    without_findings: usize,
    without_return: usize,
    without_mut: usize,
    without_type_invariant_only: usize,
}

impl Counts {
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        mode: &str,
        visibility: &str,
        external: bool,
        kind: &str,
        decl_unavailable: bool,
        present: bool,
        outputs: usize,
        findings: usize,
        return_findings: usize,
        mut_findings: usize,
        type_invariant_only: usize,
    ) {
        self.functions += 1;
        match mode {
            "Exec" => self.exec += 1,
            "Proof" => self.proof += 1,
            _ => {}
        }
        if visibility == "pub" {
            self.pub_functions += 1;
        } else {
            self.non_pub += 1;
        }
        if external {
            self.external_body += 1;
        }
        match kind {
            "trait_impl" => self.trait_impl += 1,
            "trait_decl" | "trait_default" => self.trait_decl += 1,
            _ => {}
        }
        if decl_unavailable {
            self.decl_unavailable += 1;
        }
        if present {
            self.with_functions += 1;
            self.with_outputs += outputs;
            self.with_findings += findings;
            self.with_return += return_findings;
            self.with_mut += mut_findings;
            self.with_type_invariant_only += type_invariant_only;
        } else {
            self.without_functions += 1;
            self.without_outputs += outputs;
            self.without_findings += findings;
            self.without_return += return_findings;
            self.without_mut += mut_findings;
            self.without_type_invariant_only += type_invariant_only;
        }
    }

    fn value(&self) -> Value {
        json!({
            "functions": self.functions,
            "exec": self.exec,
            "proof": self.proof,
            "pub": self.pub_functions,
            "non_pub": self.non_pub,
            "external_body": self.external_body,
            "trait_impl": self.trait_impl,
            "trait_decl": self.trait_decl,
            "decl_unavailable": self.decl_unavailable,
            "with_postcondition": {
                "functions": self.with_functions,
                "outputs": self.with_outputs,
                "findings": self.with_findings,
                "return_unmentioned": self.with_return,
                "mut_state_unmentioned": self.with_mut,
                "type_invariant_only": self.with_type_invariant_only,
            },
            "without_postcondition": {
                "functions": self.without_functions,
                "outputs": self.without_outputs,
                "findings": self.without_findings,
                "return_unmentioned": self.without_return,
                "mut_state_unmentioned": self.without_mut,
                "type_invariant_only": self.without_type_invariant_only,
            },
        })
    }
}

pub fn run(
    vir_path: &Path,
    crate_name: &str,
    source_root: Option<&Path>,
    out: &Path,
) -> Result<Value> {
    let text = fs::read_to_string(vir_path)?;
    let mut report = analyze(&text, crate_name, source_root)?;
    report["vir"] = json!({"path": vir_path, "sha256": hash(text.as_bytes())?});
    fs::create_dir(out)?;
    write_json(&out.join("census.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::f) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent r (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params ((@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent x (VarIdentDisambiguate RustcId 2)) :typ (Typ MutRef (Typ Int (IntRange USize)))))) :ensure (tuple ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent r (VarIdentDisambiguate RustcId 1)))) (Typ Int (IntRange USize))) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 1))) ()) :require () :attrs (FunctionAttrs :is_external_body false) :body (@@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
    const G: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::g) :mode Proof :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent res (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params ((@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent y (VarIdentDisambiguate RustcId 4)) :typ (Typ MutRef (Typ Int (IntRange USize)))))) :ensure (tuple () ()) :require () :attrs (FunctionAttrs :is_external_body false) :body (@@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
    const H: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::h) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent %return (VarIdentDisambiguate AirLocal)) :typ (Typ Datatype (Dt Tuple 0) () ()))) :params ((@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent z (VarIdentDisambiguate RustcId 5)) :typ (Typ MutRef (Typ Int (IntRange USize)))))) :ensure (tuple ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent z (VarIdentDisambiguate RustcId 5)))) (Typ MutRef (Typ Int (IntRange USize)))) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 1))) ()) :require () :attrs (FunctionAttrs :is_external_body false) :body (@@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
    const FINAL: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::final) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent %return (VarIdentDisambiguate AirLocal)) :typ (Typ Datatype (Dt Tuple 0) () ()))) :params ((@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent x (VarIdentDisambiguate RustcId 2)) :typ (Typ MutRef (Typ Int (IntRange USize)))))) :ensure (tuple ((@@ "t.rs:1:1: 1:2 (#0)" (> Unary (UnaryOp MutRefFuture (MutRefFutureSourceName Final)) (@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent x (VarIdentDisambiguate RustcId 2)))) (Typ MutRef (Typ Int (IntRange USize)))) (UnfinalizedReadKind :preliminary_kind (ReadKind Move) :id 2)) (Typ MutRef (Typ Int (IntRange USize)))))) ()) :require () :attrs (FunctionAttrs :is_external_body false) :body (@@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
    const DECL: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::T::m) :mode Exec :visibility (Visibility :restricted_to None) :kind (FunctionKind TraitMethodDecl :trait_path p::T) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent res (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params () :ensure (tuple ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent res (VarIdentDisambiguate RustcId 1)))) (Typ Int (IntRange USize))) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 1))) ()) :require () :attrs (FunctionAttrs :is_external_body false)))"#;
    const IMPL: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::impl&%0::m) :mode Exec :visibility (Visibility :restricted_to None) :kind (FunctionKind TraitMethodImpl :method (Fun :path p::T::m) :trait_path p::T) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent res (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params () :ensure (tuple () ()) :require () :attrs (FunctionAttrs :is_external_body false)))"#;
    const DEFAULT_ONLY: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::T::d) :mode Exec :visibility (Visibility :restricted_to None) :kind (FunctionKind TraitMethodDecl :trait_path p::T) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent res (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params () :ensure (tuple () ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent res (VarIdentDisambiguate RustcId 1)))) (Typ Int (IntRange USize))) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 1)))) :require () :attrs (FunctionAttrs :is_external_body false)))"#;
    const PATTERN: &str = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::pat) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent _VERUS_ret_ident (VarIdentDisambiguate RustcId 1)) :typ (Typ Datatype (Dt Tuple 2) ((Typ Int (IntRange USize)) (Typ Datatype (Dt Path p::G) () ())) ()))) :params () :ensure (tuple ((@@ "t.rs:1:1: 1:2 (#0)" (> Block ((@ "t.rs:1:1: 1:2 (#0)" (Stmt Decl :pattern (@@ "t.rs:1:1: 1:2 (#0)" (Pattern Constructor (Dt Tuple 2) tuple%2 ((-> 0 (@@ "t.rs:1:1: 1:2 (#0)" (Pattern Var (PatternBinding :name (VarIdent r (VarIdentDisambiguate RustcId 2)) :by_ref (ByRef No) :typ (Typ Int (IntRange USize)) :user_mut false :copy false)))) (-> 1 (@@ "t.rs:1:1: 1:2 (#0)" (Pattern Var (PatternBinding :name (VarIdent g (VarIdentDisambiguate RustcId 3)) :by_ref (ByRef No) :typ (Typ Datatype (Dt Path p::G) () ()) :user_mut false :copy false))))))) :init None :els None :assert_irrefutable false)))))) ()) :require () :attrs (FunctionAttrs :is_external_body false)))"#;

    #[test]
    fn requires_param_uses_distinguish_receiver_field_and_direct_use() {
        let function = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::f) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent r (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params ((@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent self (VarIdentDisambiguate RustcId 2)) :typ (Typ MutRef (Typ Datatype (Dt Path p::T) () ())) :mode Exec)) (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent fld (VarIdentDisambiguate RustcId 3)) :typ (Typ Int (IntRange USize)) :mode Exec))) :ensure (tuple () ()) :require ((@@ "t.rs:1:1: 1:2 (#0)" (> Call :target (CallTarget Fun (CallTargetKind DynamicResolved :resolved (Fun :path p::inv) :typs () :impl_paths () :is_trait_default false) (Fun :path p::inv) () (ImplPath TraitImplPath p::I) (CallTargetAttrs :autospec (AutospecUsage IfMarked) :const_var false :assume_external_allowed false)) :args ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent self (VarIdentDisambiguate RustcId 2))) (Typ Datatype (Dt Path p::T) () ())) (Typ Datatype (Dt Path p::T) () ()) (UnfinalizedReadKind :preliminary_kind (ReadKind Move) :id 1)))) :post_args None :body None) (Typ Bool)) (@@ "t.rs:1:1: 1:2 (#0)" (> Binary (BinaryOp Eq Spec) (@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Field (FieldOpr :datatype (Dt Path p::T) :variant "T" :field "fld" :get_variant false :check (VariantCheck None)) (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent self (VarIdentDisambiguate RustcId 2))) (Typ MutRef (Typ Datatype (Dt Path p::T) () ()))) (Typ Int (IntRange USize))) (Typ Int (IntRange USize))) (Typ Int (IntRange USize)) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 2))) (@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent fld (VarIdentDisambiguate RustcId 3))) (Typ Int (IntRange USize))) (Typ Int (IntRange USize)) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 3)))) (Typ Bool))) :attrs (FunctionAttrs :is_external_body false) :body (@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
        let report = analyze(function, "p", None).unwrap();
        let clauses = report["functions"][0]["requires"].as_array().unwrap();
        assert_eq!(clauses.len(), 2);
        let occurrences = &clauses[0]["param_occurrences"];
        assert_eq!(occurrences["self"], 1);
        let uses = clauses[0]["param_uses"].as_array().unwrap();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0]["name"], "self");
        assert_eq!(uses[0]["kind"], "receiver_root");
        assert_eq!(uses[0]["callee"], "p::inv");
        let uses = clauses[1]["param_uses"].as_array().unwrap();
        assert_eq!(uses.len(), 2);
        assert_eq!(uses[0]["kind"], "field_root");
        assert_eq!(uses[0]["field"], "fld");
        assert_eq!(uses[1]["kind"], "plain_root");
        assert_eq!(uses[1]["name"], "fld");
        assert_eq!(clauses[0]["expr"]["op"], "call");
        assert_eq!(clauses[0]["expr"]["callee"], "p::inv");
        assert_eq!(clauses[0]["expr"]["args"][0]["op"], "read");
        assert_eq!(clauses[1]["expr"]["op"], "bin");
        assert_eq!(clauses[1]["expr"]["sym"], "==");
        assert_eq!(clauses[1]["expr"]["lhs"]["op"], "read");
        assert_eq!(clauses[1]["expr"]["lhs"]["arg"]["op"], "field");
        assert_eq!(clauses[1]["expr"]["lhs"]["arg"]["field"], "fld");
        assert_eq!(clauses[1]["expr"]["lhs"]["arg"]["datatype"], "p::T");
        assert_eq!(clauses[1]["expr"]["rhs"]["op"], "read");
    }

    #[test]
    fn requires_param_uses_flag_old_wrapped_receiver_and_deref() {
        let function = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::g) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent r (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params ((@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent self (VarIdentDisambiguate RustcId 2)) :typ (Typ MutRef (Typ Datatype (Dt Path p::T) () ())) :mode Exec))) :ensure (tuple () ()) :require ((@@ "t.rs:1:1: 1:2 (#0)" (> Call :target (CallTarget Fun (CallTargetKind DynamicResolved :resolved (Fun :path p::inv) :typs () :impl_paths () :is_trait_default false) (Fun :path p::inv) () (ImplPath TraitImplPath p::I) (CallTargetAttrs :autospec (AutospecUsage IfMarked) :const_var false :assume_external_allowed false)) :args ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Temporary (@@ "t.rs:1:1: 1:2 (#0)" (> Old (@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent self (VarIdentDisambiguate RustcId 2))) (Typ MutRef (Typ Datatype (Dt Path p::T) () ()))) (Typ MutRef (Typ Datatype (Dt Path p::T) () ())) (UnfinalizedReadKind :preliminary_kind (ReadKind Move) :id 1))) (Typ MutRef (Typ Datatype (Dt Path p::T) () ())))) (Typ Datatype (Dt Path p::T) () ())) (Typ Datatype (Dt Path p::T) () ())) (UnfinalizedReadKind :preliminary_kind (ReadKind Move) :id 2)))) :post_args None :body None) (Typ Bool)) (@@ "t.rs:1:1: 1:2 (#0)" (> Binary (BinaryOp Eq Spec) (@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Deref (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent self (VarIdentDisambiguate RustcId 2))) (Typ MutRef (Typ Datatype (Dt Path p::T) () ()))) (Typ Int (IntRange USize))) (Typ Int (IntRange USize))) (Typ Int (IntRange USize)) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 3))) (@ "t.rs:1:1: 1:2 (#0)" (Const "0"))) (Typ Bool))) :attrs (FunctionAttrs :is_external_body false) :body (@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
        let report = analyze(function, "p", None).unwrap();
        let clauses = report["functions"][0]["requires"].as_array().unwrap();
        assert_eq!(clauses[0]["features"]["old_or_final"], true);
        let uses = clauses[0]["param_uses"].as_array().unwrap();
        assert_eq!(uses[0]["kind"], "receiver_root");
        let uses = clauses[1]["param_uses"].as_array().unwrap();
        assert_eq!(uses[0]["kind"], "deref_root");
    }

    #[test]
    fn return_and_mut_outputs_follow_mention_rules() {
        let text = [F, G, H].join("\n");
        let report = analyze(&text, "p", None).unwrap();
        let functions = report["functions"].as_array().unwrap();
        assert_eq!(functions.len(), 3);
        assert_eq!(functions[0]["outputs"][0]["mentioned"], true);
        assert_eq!(functions[0]["outputs"][1]["mentioned"], false);
        assert_eq!(
            functions[0]["outputs"][1]["finding"],
            "mut_state_unmentioned"
        );
        assert_eq!(functions[1]["postcondition"]["present"], false);
        assert_eq!(functions[1]["outputs"][0]["finding"], "return_unmentioned");
        assert_eq!(
            functions[1]["outputs"][1]["finding"],
            "mut_state_unmentioned"
        );
        assert_eq!(functions[2]["outputs"].as_array().unwrap().len(), 1);
        assert_eq!(functions[2]["outputs"][0]["kind"], "mut_param");
        assert_eq!(functions[2]["outputs"][0]["mentioned"], false);
    }

    #[test]
    fn final_is_distinct_from_old_and_bare_mentions() {
        let report = analyze(FINAL, "p", None).unwrap();
        let output = &report["functions"][0]["outputs"][0];
        assert_eq!(output["kind"], "mut_param");
        assert_eq!(output["mentioned"], true);
        assert_eq!(output["mention"], "final");
    }

    #[test]
    fn trait_declaration_postcondition_and_default_only_are_distinct() {
        let text = format!("{DECL}\n{IMPL}");
        let report = analyze(&text, "p", None).unwrap();
        let implementation = &report["functions"][1];
        assert_eq!(implementation["postcondition"]["source"], "decl");
        assert_eq!(implementation["outputs"][0]["mentioned"], true);

        let report = analyze(DEFAULT_ONLY, "p", None).unwrap();
        assert_eq!(report["functions"][0]["postcondition"]["present"], false);
        assert_eq!(
            report["functions"][0]["postcondition"]["default_ensures_only"],
            true
        );
        assert_eq!(
            report["functions"][0]["outputs"][0]["finding"],
            "return_unmentioned"
        );
        assert_eq!(
            report["functions"][0]["outputs"][0]["default_ensures_mention"],
            true
        );
    }

    #[test]
    fn injected_pattern_counts_ghost_return_and_omits_unit_actual_return() {
        let report = analyze(PATTERN, "p", None).unwrap();
        let outputs = report["functions"][0]["outputs"].as_array().unwrap();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["kind"], "return");
        assert_eq!(outputs[0]["name"], "r");
        assert_eq!(outputs[1]["kind"], "ghost_return");
        assert_eq!(outputs[1]["name"], "g");

        let unit_actual = PATTERN.replace(
            "(Typ Int (IntRange USize))",
            "(Typ Datatype (Dt Tuple 0) () ())",
        );
        let report = analyze(&unit_actual, "p", None).unwrap();
        let outputs = report["functions"][0]["outputs"].as_array().unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0]["kind"], "ghost_return");
    }

    #[test]
    fn verified_body_wins_and_synthetic_return_name_is_a_mention() {
        let wrapper = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::impl&%0::f) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent %return (VarIdentDisambiguate AirLocal)) :typ (Typ Int (IntRange USize)))) :params () :ensure (tuple () ()) :require () :attrs (FunctionAttrs :is_external_body false)))"#;
        let verified = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::impl&%0::_VERUS_VERIFIED_f) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent %return (VarIdentDisambiguate AirLocal)) :typ (Typ Int (IntRange USize)))) :params () :ensure (tuple ((@@ "t.rs:1:1: 1:2 (#0)" (> ReadPlace (@ "t.rs:1:1: 1:2 (#0)" (Place Local (VarIdent %return (VarIdentDisambiguate AirLocal)))) (Typ Int (IntRange USize))) (UnfinalizedReadKind :preliminary_kind (ReadKind Copy) :id 1))) ()) :require () :attrs (FunctionAttrs :is_external_body false) :body (@@ "t.rs:1:1: 1:2 (#0)" (> Block ()))))"#;
        let report = analyze(&format!("{wrapper}\n{verified}"), "p", None).unwrap();
        let functions = report["functions"].as_array().unwrap();
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0]["path"], "p::impl&%0::f");
        assert_eq!(functions[0]["postcondition"]["present"], true);
        assert_eq!(functions[0]["outputs"][0]["mentioned"], true);
    }

    #[test]
    fn returns_clause_is_a_postcondition_and_mentions_the_return() {
        let returns = r#"(@ "t.rs:1:1: 1:2 (#0)" (Function :name (Fun :path p::f) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:1:1: 1:2 (#0)" (Param :name (VarIdent r (VarIdentDisambiguate RustcId 1)) :typ (Typ Int (IntRange USize)))) :params () :ensure (tuple () ()) :returns (@ "t.rs:1:1: 1:2 (#0)" (> Var (VarIdent r (VarIdentDisambiguate RustcId 1)))) :require () :attrs (FunctionAttrs :is_external_body false)))"#;
        let report = analyze(returns, "p", None).unwrap();
        assert_eq!(report["functions"][0]["postcondition"]["present"], true);
        assert_eq!(report["functions"][0]["outputs"][0]["mentioned"], true);
        assert_eq!(report["functions"][0]["outputs"][0]["mention"], "returns");
    }

    #[test]
    fn type_invariant_and_external_body_are_annotated() {
        let datatype = r#"(@ "t.rs:1:1: 1:2 (#0)" (Datatype :name (Dt Path p::T) :user_defined_invariant_fn (FunWithVis :visibility (Visibility :restricted_to None) :fun (Fun :path p::T::type_inv)) :mode Exec))"#;
        let function = r#"(@ "t.rs:2:1: 2:9 (#0)" (Function :name (Fun :path p::f) :mode Exec :visibility (Visibility :restricted_to None) :inner_ret (@ "t.rs:2:1: 2:2 (#0)" (Param :name (VarIdent r (VarIdentDisambiguate RustcId 1)) :typ (Typ Datatype (Dt Path p::T) () ()))) :params () :ensure (tuple () ()) :require () :attrs (FunctionAttrs :is_external_body true)))"#;
        let report = analyze(&format!("{datatype}\n{function}"), "p", None).unwrap();
        let function = &report["functions"][0];
        assert_eq!(function["external_body"], true);
        assert_eq!(function["outputs"][0]["type_invariant"], "type_inv");
        assert_eq!(function["outputs"][0]["type_invariant_only"], true);
    }
}
