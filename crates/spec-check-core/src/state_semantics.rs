//! Source-bound recognizer for a narrow boolean-sequence state transition shape.

use crate::{Result, vir::Node};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub constructor_path: String,
    pub ctor_span: String,
    pub transition_path: String,
    pub transition_span: String,
    pub selector: String,
    pub predicate: String,
    pub witness_lemma: String,
    pub constructor_length: String,
    pub invariant_trait_call: Option<String>,
}

fn head(node: &Node) -> Option<&str> {
    let values = node.unwrap().list()?;
    if values.first().and_then(Node::atom) == Some(">") {
        values.get(1)?.atom()
    } else {
        values.first()?.atom()
    }
}

fn children(node: &Node) -> &[Node] {
    let values = node.unwrap().list().unwrap_or(&[]);
    if values.first().and_then(Node::atom) == Some(">") {
        &values[1..]
    } else {
        values
    }
}

fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    out.push(node);
    for child in children(node) {
        walk(child, out);
    }
}

pub(crate) fn nodes<'a>(node: &'a Node, kind: &str) -> Vec<&'a Node> {
    let mut all = Vec::new();
    walk(node, &mut all);
    all.into_iter().filter(|n| head(n) == Some(kind)).collect()
}

fn atom_at(node: &Node, index: usize) -> Option<&str> {
    children(node).get(index)?.atom()
}

fn path_safe(path: &str) -> bool {
    !path.is_empty()
        && path.split("::").all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '&' | '%'))
        })
}

fn path(node: &Node) -> Option<&str> {
    let name = node.field("name")?;
    name.field("path")?.atom()
}

pub(crate) fn span(node: &Node) -> Option<&str> {
    let values = node.list()?;
    matches!(values.first()?.atom(), Some("@" | "@@")).then(|| values.get(1)?.atom())?
}

pub(crate) fn call_path(node: &Node) -> Option<&str> {
    if head(node) != Some("Call") {
        return None;
    }
    let target = node.field("target")?;
    let kind = children(target).get(2);
    if let Some(path) = kind
        .and_then(|kind| kind.field("resolved"))
        .and_then(|resolved| resolved.field("path"))
        .and_then(Node::atom)
    {
        return Some(path);
    }
    let mut found = Vec::new();
    walk(target, &mut found);
    found.into_iter().find_map(|n| {
        (head(n) == Some("Fun"))
            .then(|| n.field("path").and_then(Node::atom))
            .flatten()
    })
}

pub(crate) fn call_args(node: &Node) -> &[Node] {
    node.field("args").and_then(Node::list).unwrap_or(&[])
}

pub(crate) fn local_read(node: &Node) -> Option<&Node> {
    for read in nodes(node, "ReadPlace") {
        let place = children(read).get(1)?;
        if head(place) == Some("Place") && atom_at(place, 1) == Some("Local") {
            return nodes(place, "VarIdent").into_iter().next();
        }
    }
    None
}

pub(crate) fn has_old_self(node: &Node, receiver: &Node) -> bool {
    nodes(node, "Old")
        .iter()
        .any(|old| local_read(old) == Some(receiver))
}

pub(crate) fn has_final_self(node: &Node, receiver: &Node) -> bool {
    nodes(node, "MutRefFutureSourceName")
        .iter()
        .any(|n| atom_at(n, 1) == Some("Final"))
        && local_read(node) == Some(receiver)
}

pub(crate) fn old_view(node: &Node, receiver: &Node) -> bool {
    call_path(node).is_some_and(|p| p.ends_with("::view")) && has_old_self(node, receiver)
}

pub(crate) fn final_view(node: &Node, receiver: &Node) -> bool {
    call_path(node).is_some_and(|p| p.ends_with("::view")) && has_final_self(node, receiver)
}

fn ret_view(node: &Node, ret: &Node) -> bool {
    call_path(node).is_some_and(|p| p.ends_with("::view"))
        && local_read(node).is_some_and(|v| v == ret)
}

fn bool_const(node: &Node, expected: bool) -> bool {
    if head(node) != Some("Const") {
        return false;
    }
    let Some(constant) = children(node).get(1) else {
        return false;
    };
    head(constant) == Some("Constant")
        && atom_at(constant, 1) == Some("Bool")
        && atom_at(constant, 2) == Some(if expected { "true" } else { "false" })
}

fn bool_seq_call(node: &Node, method: &str) -> bool {
    call_path(node) == Some(method)
        && node.field("target").is_some_and(|target| {
            nodes(target, "Typ")
                .iter()
                .any(|ty| children(ty).get(1).and_then(Node::atom) == Some("Bool"))
        })
}

pub(crate) fn binary_eq(node: &Node) -> Option<(&Node, &Node)> {
    let values = children(node);
    if head(node) != Some("Binary")
        || head(values.get(1)?) != Some("BinaryOp")
        || atom_at(values.get(1)?, 1) != Some("Eq")
    {
        return None;
    }
    Some((values.get(2)?, values.get(3)?))
}

fn constructor_match(function: &Node) -> Option<(String, String)> {
    if function_mode(function) != Some("Exec") {
        return None;
    }
    if function
        .field("typ_params")
        .and_then(Node::list)
        .is_some_and(|p| !p.is_empty())
    {
        return None;
    }
    let params = function.field("params")?.list()?;
    if params.len() != 1 {
        return None;
    }
    let length_ident = params[0].field("name")?;
    if params[0]
        .field("typ")
        .and_then(Node::list)
        .and_then(|t| t.get(1))
        .and_then(Node::atom)
        != Some("Int")
        || !params[0].field("typ").is_some_and(|t| {
            nodes(t, "IntRange")
                .iter()
                .any(|n| atom_at(n, 1) == Some("USize"))
        })
    {
        return None;
    }
    let length_param = atom_at(length_ident, 1)?.to_owned();
    let ret = function.field("outer_ret")?.field("name")?;
    let ensures = function.field("ensure")?;
    for eq in nodes(ensures, "Binary") {
        let Some((lhs, rhs)) = binary_eq(eq) else {
            continue;
        };
        if !ret_view(lhs, ret) || !bool_seq_call(rhs, "vstd::seq::Seq::new") {
            continue;
        }
        let args = call_args(rhs);
        if args.len() != 2
            || local_read(&args[0]) != Some(length_ident)
            || head(&args[1]) != Some("Closure")
            || children(&args[1])
                .get(2)
                .is_none_or(|body| !bool_const(body, false))
        {
            continue;
        }
        return Some((length_param, span(function)?.to_owned()));
    }
    None
}

fn some_arm(node: &Node) -> bool {
    head(node) == Some("Arm")
        && nodes(node.field("pattern").unwrap_or(node), "Pattern")
            .iter()
            .any(|p| atom_at(p, 3) == Some("Some"))
}

fn transition_match(function: &Node) -> Option<(String, String, String)> {
    if function_mode(function) != Some("Exec") {
        return None;
    }
    if function
        .field("typ_params")
        .and_then(Node::list)
        .is_some_and(|p| !p.is_empty())
    {
        return None;
    }
    let receiver = function.field("params")?.list()?.first()?.field("name")?;
    for arm in nodes(function.field("ensure")?, "Arm") {
        if !some_arm(arm) {
            continue;
        }
        for eq in nodes(arm, "Binary") {
            let Some((lhs, rhs)) = binary_eq(eq) else {
                continue;
            };
            if !final_view(lhs, receiver) || !bool_seq_call(rhs, "vstd::seq::Seq::update") {
                continue;
            }
            let args = call_args(rhs);
            if args.len() != 3 || !old_view(&args[0], receiver) || !bool_const(&args[2], true) {
                continue;
            }
            let selector_call = &args[1];
            if head(selector_call) != Some("Call") {
                continue;
            }
            let Some(selector) = call_path(selector_call) else {
                continue;
            };
            let selector_args = call_args(selector_call);
            if selector_args.len() != 1
                || !old_view(&selector_args[0], receiver)
                || !path_safe(selector)
            {
                continue;
            }
            let invariant = final_invariant_call(function)?;
            return Some((selector.to_owned(), span(function)?.to_owned(), invariant));
        }
    }
    None
}

fn function_mode(function: &Node) -> Option<&str> {
    function.field("mode")?.atom()
}

fn predicate_call(node: &Node, predicate: &str, selector: &str) -> Option<Node> {
    if head(node) != Some("Call") || call_path(node) != Some(predicate) {
        return None;
    }
    let args = call_args(node);
    if args.len() != 2 || head(&args[1]) != Some("Call") || call_path(&args[1]) != Some(selector) {
        return None;
    }
    let seq = local_read(&args[0])?;
    (local_read(call_args(&args[1]).first()?)? == seq).then(|| seq.clone())
}

fn witness_match(function: &Node, selector: &str) -> Option<String> {
    if function_mode(function) != Some("Proof") {
        return None;
    }
    if function
        .field("typ_params")
        .and_then(Node::list)
        .is_some_and(|params| !params.is_empty())
        || function
            .field("params")
            .and_then(Node::list)
            .is_none_or(|params| params.len() != 1)
        || function
            .field("require")
            .and_then(Node::list)
            .is_none_or(|requires| !requires.is_empty())
    {
        return None;
    }
    let ensures = function.field("ensure")?;
    let clauses = children(ensures).get(1)?.list()?;
    let mut candidates = Vec::new();
    for clause in clauses {
        if head(clause) != Some("Call") {
            continue;
        }
        if let Some(predicate) = call_path(clause).filter(|predicate| {
            predicate_call(clause, predicate, selector).is_some() && path_safe(predicate)
        }) {
            candidates.push(predicate.to_owned());
        }
    }
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

/// Discover constructor/transition/observer triples from typed VIR structure.
pub fn discover(functions: &[Node]) -> Result<Vec<Pattern>> {
    let mut constructors = Vec::new();
    let mut transitions = Vec::new();
    for function in functions {
        let Some(path) = path(function) else { continue };
        if !path_safe(path) {
            continue;
        }
        if let Some((length, source_span)) = constructor_match(function) {
            constructors.push((
                path.to_owned(),
                source_span,
                length,
                return_invariant_call(function),
            ));
        }
        if let Some((selector, source_span, invariant)) = transition_match(function) {
            transitions.push((path.to_owned(), source_span, selector, invariant));
        }
    }
    let mut patterns = Vec::new();
    for (constructor_path, constructor_span, constructor_length, invariant_trait_call) in
        &constructors
    {
        for (transition_path, transition_span, selector, transition_invariant) in &transitions {
            if invariant_trait_call
                .as_ref()
                .is_some_and(|i| i != transition_invariant)
            {
                continue;
            }
            let witnesses = functions
                .iter()
                .filter_map(|function| {
                    witness_match(function, selector).and_then(|predicate| {
                        path(function).map(|witness| (witness.to_owned(), predicate))
                    })
                })
                .collect::<Vec<_>>();
            for (witness_lemma, predicate) in witnesses {
                patterns.push(Pattern {
                    constructor_path: constructor_path.clone(),
                    ctor_span: constructor_span.clone(),
                    transition_path: transition_path.clone(),
                    transition_span: transition_span.clone(),
                    selector: selector.clone(),
                    predicate: predicate.clone(),
                    witness_lemma,
                    constructor_length: constructor_length.clone(),
                    invariant_trait_call: invariant_trait_call.clone(),
                });
            }
        }
    }
    patterns.sort_by(|a, b| {
        (&a.constructor_path, &a.transition_path, &a.selector).cmp(&(
            &b.constructor_path,
            &b.transition_path,
            &b.selector,
        ))
    });
    patterns.dedup();
    Ok(patterns)
}

fn return_invariant_call(function: &Node) -> Option<String> {
    let ret = function.field("outer_ret")?.field("name")?;
    let ensures = function.field("ensure")?;
    for call in nodes(ensures, "Call") {
        let args = call_args(call);
        if args.len() != 1 || local_read(&args[0]) != Some(ret) {
            continue;
        }
        let target = call.field("target")?;
        let mut funs = nodes(target, "Fun");
        funs.reverse();
        if let Some(path) = funs
            .into_iter()
            .find_map(|f| f.field("path").and_then(Node::atom))
            .filter(|path| *path == "vstd_extra::ownership::Inv::inv")
        {
            return Some(path.to_owned());
        }
    }
    None
}

fn final_invariant_call(function: &Node) -> Option<String> {
    let receiver = function.field("params")?.list()?.first()?.field("name")?;
    for call in nodes(function.field("ensure")?, "Call") {
        if call_args(call).len() != 1 || !has_final_self(&call_args(call)[0], receiver) {
            continue;
        }
        let target = call.field("target")?;
        let mut funs = nodes(target, "Fun");
        funs.reverse();
        if let Some(path) = funs
            .into_iter()
            .find_map(|f| f.field("path").and_then(Node::atom))
            .filter(|path| *path == "vstd_extra::ownership::Inv::inv")
        {
            return Some(path.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace_atoms(node: &mut Node, from: &str, to: &str) {
        match node {
            Node::Atom(value) if value == from => *value = to.to_owned(),
            Node::List(items) => items
                .iter_mut()
                .for_each(|item| replace_atoms(item, from, to)),
            Node::Atom(_) => {}
        }
    }

    fn field_mut<'a>(node: &'a mut Node, name: &str) -> Option<&'a mut Node> {
        let Node::List(items) = node else { return None };
        let wrapped = matches!(items.first().and_then(Node::atom), Some("@" | "@@"));
        let items = if wrapped {
            let Node::List(inner) = items.get_mut(2)? else {
                return None;
            };
            inner
        } else {
            items
        };
        let key = format!(":{name}");
        let index = items.iter().position(|item| item.atom() == Some(&key))?;
        items.get_mut(index + 1)
    }

    fn break_selector_receiver(node: &mut Node, selector: &str) -> bool {
        let mut changed = false;
        if call_path(node) == Some(selector) {
            changed = field_mut(node, "args")
                .and_then(|args| match args {
                    Node::List(items) => items.first_mut(),
                    Node::Atom(_) => None,
                })
                .is_some_and(|first| {
                    replace_atoms(first, "self", "different_receiver");
                    true
                });
        }
        if let Node::List(items) = node {
            for item in items {
                changed |= break_selector_receiver(item, selector);
            }
        }
        changed
    }

    fn nonconstant_closure(node: &mut Node) -> bool {
        if head(node) == Some("Closure") {
            if let Node::List(items) = node {
                let closure = if matches!(items.first().and_then(Node::atom), Some("@" | "@@")) {
                    match items.get_mut(2) {
                        Some(Node::List(body)) => body,
                        _ => return false,
                    }
                } else {
                    items
                };
                let body_index = if closure.first().and_then(Node::atom) == Some(">") {
                    3
                } else {
                    2
                };
                if let Some(body) = closure.get_mut(body_index) {
                    *body = Node::List(vec![Node::Atom(">".into()), Node::Atom("Unary".into())]);
                    return true;
                }
            }
            return false;
        }
        if let Node::List(items) = node {
            for item in items {
                if nonconstant_closure(item) {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    #[ignore]
    fn real_idalloc_contract_pattern() {
        let root = std::env::var("SPEC_STATE_TARGET")
            .expect("set SPEC_STATE_TARGET to a target directory");
        let input =
            std::env::var("SPEC_STATE_VIR").unwrap_or_else(|_| "baseline-vir/crate.vir".into());
        let text = std::fs::read_to_string(std::path::Path::new(&root).join(input)).unwrap();
        let functions = crate::vir::functions(&text).unwrap();
        let patterns = discover(&functions).unwrap();
        assert_eq!(patterns.len(), 1, "{patterns:#?}");
        let pattern = &patterns[0];
        assert_eq!(pattern.constructor_length, "capacity");
        assert!(pattern.selector.ends_with("::first_zero_index"));
        assert!(pattern.predicate.ends_with("::is_first_zero"));
        assert!(
            pattern
                .witness_lemma
                .ends_with("::lemma_first_zero_index_is_first_zero")
        );

        let flip = |marker: &str, from: &str, to: &str| {
            let anchor = text.find(marker).unwrap();
            let start = anchor + text[anchor..].find(from).unwrap();
            let mut changed = text.clone();
            changed.replace_range(start..start + from.len(), to);
            changed
        };
        for changed in [
            flip("lib.rs:67:56: 67:61", "Bool false", "Bool true"),
            flip("lib.rs:90:85: 90:89", "Bool true", "Bool false"),
        ] {
            let altered = crate::vir::functions(&changed).unwrap();
            assert!(
                discover(&altered).unwrap().is_empty(),
                "changed fill must not match"
            );
        }

        let mut nonconstant = functions.clone();
        let constructor = nonconstant
            .iter_mut()
            .find(|function| path(function) == Some(pattern.constructor_path.as_str()))
            .unwrap();
        assert!(nonconstant_closure(constructor));
        assert!(discover(&nonconstant).unwrap().is_empty());

        let mut renamed = functions.clone();
        for function in &mut renamed {
            replace_atoms(function, &pattern.selector, "renamed_api::select");
            replace_atoms(function, &pattern.predicate, "renamed_api::predicate");
            replace_atoms(function, &pattern.witness_lemma, "renamed_api::witness");
        }
        let renamed = discover(&renamed).unwrap();
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].selector, "renamed_api::select");
        assert_eq!(renamed[0].predicate, "renamed_api::predicate");

        let mut mismatched_receiver = functions.clone();
        let transition = mismatched_receiver
            .iter_mut()
            .find(|function| path(function) == Some(pattern.transition_path.as_str()))
            .unwrap();
        assert!(break_selector_receiver(transition, &pattern.selector));
        assert!(discover(&mismatched_receiver).unwrap().is_empty());

        let mut mismatched_return = functions.clone();
        let constructor = mismatched_return
            .iter_mut()
            .find(|function| path(function) == Some(pattern.constructor_path.as_str()))
            .unwrap();
        replace_atoms(
            field_mut(constructor, "outer_ret").unwrap(),
            "ret",
            "different_return",
        );
        assert!(discover(&mismatched_return).unwrap().is_empty());
    }
}
