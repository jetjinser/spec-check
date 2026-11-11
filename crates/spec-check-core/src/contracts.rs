use crate::{
    Result, ir,
    vir::{self, Node},
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

fn path_of(function: &Node) -> Option<&str> {
    function.field("name")?.field("path")?.atom()
}

fn span_of(node: &Node) -> Option<&str> {
    let list = node.list()?;
    if matches!(list.first()?.atom(), Some("@" | "@@")) {
        list.get(1)?.atom()
    } else {
        None
    }
}

pub(crate) fn render(node: &Node) -> String {
    match node {
        Node::Atom(value) => format!("{:?}", value),
        Node::List(items) => format!(
            "({})",
            items.iter().map(render).collect::<Vec<_>>().join(" ")
        ),
    }
}

fn excerpt(node: &Node, cwd: &Path, cache: &mut HashMap<String, String>) -> Value {
    let Some(span_text) = span_of(node) else {
        return json!({"available": false, "reason": "no enclosing source span"});
    };
    let Ok((path, coordinates)) = ir::span(span_text, cwd) else {
        return json!({"available": false, "span": span_text, "reason": "unsupported source span"});
    };
    let key = path.to_string_lossy().into_owned();
    if !cache.contains_key(&key) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            cache.insert(key.clone(), text);
        } else {
            return json!({"available": false, "span": span_text, "path": key, "reason": "source unavailable"});
        }
    }
    let text = &cache[&key];
    match ir::offsets(text, coordinates) {
        Ok((start, end)) => {
            json!({"available": true, "span": span_text, "path": key, "snippet": &text[start..end]})
        }
        Err(error) => {
            json!({"available": false, "span": span_text, "path": key, "reason": error.to_string()})
        }
    }
}

fn clauses(
    function: &Node,
    field: &str,
    cwd: &Path,
    cache: &mut HashMap<String, String>,
) -> Vec<Value> {
    let values = function
        .field(field)
        .and_then(Node::list)
        .unwrap_or_default();
    let clauses = match (field, values) {
        ("ensure", [tag, regular, defaults]) if tag.atom() == Some("tuple") => {
            match (regular.list(), defaults.list()) {
                (Some(regular), Some(defaults)) => {
                    regular.iter().chain(defaults).collect::<Vec<_>>()
                }
                _ => values.iter().collect(),
            }
        }
        _ => values.iter().collect(),
    };
    clauses
        .into_iter()
        .map(|clause| json!({"vir": render(clause), "source": excerpt(clause, cwd, cache)}))
        .collect()
}

pub(crate) fn contract(function: &Node, cwd: &Path, cache: &mut HashMap<String, String>) -> Value {
    let params = function.field("params").and_then(Node::list).unwrap_or_default().iter().map(|param| {
        json!({"name": param.field("name").map(render), "type": param.field("typ").map(render), "mode": param.field("mode").and_then(Node::atom)})
    }).collect::<Vec<_>>();
    let ret = function
        .field("inner_ret")
        .and_then(|n| n.field("typ"))
        .map(render);
    let external = function
        .field("attrs")
        .and_then(|a| a.field("is_external_body"))
        .and_then(Node::atom);
    json!({"mode": function.field("mode").and_then(Node::atom), "params": params,
        "return_type": ret, "requires": clauses(function, "require", cwd, cache),
        "ensures": clauses(function, "ensure", cwd, cache),
        "external_body": external.map(|v| v == "true"),
        "function_span": span_of(function),"returns":function.field("returns").map(render),
        "type_parameters":function.field("typ_params").map(render),"type_bounds":function.field("typ_bounds").map(render),
        "mask":function.field("mask_spec").map(render),"kind":function.field("kind").map(render)})
}

fn find_path(node: &Node) -> Option<String> {
    let mut paths = Vec::new();
    vir::walk_calls(node, &mut paths);
    paths.into_iter().next()
}

fn calls(
    node: &Node,
    inherited_span: Option<&str>,
    out: &mut Vec<Value>,
    index: &HashMap<String, Vec<&Node>>,
    cwd: &Path,
    cache: &mut HashMap<String, String>,
    inline_contracts: bool,
) {
    let Some(items) = node.list() else { return };
    let mut current_span = inherited_span;
    let mut expr = node;
    if matches!(items.first().and_then(Node::atom), Some("@" | "@@")) {
        current_span = items.get(1).and_then(Node::atom).or(current_span);
        if let Some(inner) = items.get(2) {
            expr = inner;
        }
    }
    if let Some(expr_items) = expr.list()
        && expr_items.first().and_then(Node::atom) == Some(">")
        && expr_items.get(1).and_then(Node::atom) == Some("Call")
    {
        let target = expr.field("target");
        let resolved = target.and_then(find_path);
        let candidates = resolved.as_ref().and_then(|p| index.get(p));
        let callee = match candidates.map(Vec::as_slice) {
            Some([function]) if inline_contracts => {
                json!({"status": "indexed", "path": resolved, "contract": contract(function, cwd, cache)})
            }
            Some([_]) => json!({"status":"indexed","path":resolved}),
            Some(_) => json!({"status": "ambiguous_native_identity", "path": resolved}),
            None => {
                json!({"status": "unresolved", "path": resolved, "reason": "no unique native Function record"})
            }
        };
        let site = current_span
            .map(|s| json!({"available": true, "span": s}))
            .unwrap_or_else(
                || json!({"available": false, "reason": "call has no enclosing @/@@ span"}),
            );
        let site = if let Some(span) = current_span {
            let mut source = excerpt(expr, cwd, cache);
            if !source["available"].as_bool().unwrap_or(false) {
                source = excerpt(node, cwd, cache);
            }
            json!({"span": span, "source": source})
        } else {
            site
        };
        out.push(
            json!({"traversal_index": out.len(), "call_site": site, "callee": callee,
            "native_target":target.map(render)}),
        );
    }
    let children = if std::ptr::eq(expr, node) {
        items
    } else {
        expr.list().unwrap_or_default()
    };
    for child in children {
        calls(
            child,
            current_span,
            out,
            index,
            cwd,
            cache,
            inline_contracts,
        );
    }
}

/// Summarize a native VIR function's source-attributed calls and indexed callees' contracts.
/// This inventories contracts and call sites; it does not attribute solver dependencies to
/// individual clauses, assert execution order, or invent resource inputs.
pub fn summarize(text: &str, target: &str, cwd: &Path) -> Result<Value> {
    summarize_functions(&vir::functions(text)?, target, cwd)
}

pub(crate) fn summarize_functions(functions: &[Node], target: &str, cwd: &Path) -> Result<Value> {
    summarize_functions_with(functions, target, cwd, true)
}

pub(crate) fn summarize_compact(functions: &[Node], target: &str, cwd: &Path) -> Result<Value> {
    summarize_functions_with(functions, target, cwd, false)
}

fn summarize_functions_with(
    functions: &[Node],
    target: &str,
    cwd: &Path,
    inline_contracts: bool,
) -> Result<Value> {
    let mut index: HashMap<String, Vec<&Node>> = HashMap::new();
    let mut entries = Vec::new();
    for function in functions {
        if let Some(path) = path_of(function) {
            index.entry(path.to_owned()).or_default().push(function);
            if path == target {
                entries.push(function);
            }
        }
    }
    let [entry] = entries.as_slice() else {
        if entries.len() > 1 {
            return Ok(
                json!({"status": "target_ambiguous", "target": target, "native_definitions": entries.len()}),
            );
        }
        return Ok(json!({"status": "target_not_found", "target": target}));
    };
    let mut cache = HashMap::new();
    let mut call_sites = Vec::new();
    if let Some(body) = entry.field("body") {
        calls(
            body,
            None,
            &mut call_sites,
            &index,
            cwd,
            &mut cache,
            inline_contracts,
        );
    }
    Ok(
        json!({"status": "summarized", "target": target, "function": contract(entry, cwd, &mut cache),
        "call_sites": call_sites,
        "scope": "Available contracts and source-attributed call sites only; no solver-level clause dependency attribution, no execution-order claim, and no invented resource inputs."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn regular_and_trait_default_ensures_keep_individual_sources() {
        let dir = std::env::temp_dir().join(format!(
            "contract-clause-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("source.rs"), "x > 0\nx < 10\nx != 4\ny == 2\n").unwrap();
        let functions = vir::functions(
            r#"
(Function :name (Fun :path crate::f)
 :require ((@ "source.rs:1:1: 1:6 (#0)" (> Const (Constant Bool true))))
 :ensure (tuple
  ((@ "source.rs:2:1: 2:7 (#0)" (> Const (Constant Bool true)))
   (@ "source.rs:3:1: 3:7 (#0)" (> Const (Constant Bool false))))
  ((@ "source.rs:4:1: 4:7 (#0)" (> Const (Constant Bool true))))))
"#,
        )
        .unwrap();
        let result = contract(&functions[0], &dir, &mut HashMap::new());
        let ensures = result["ensures"].as_array().unwrap();
        assert_eq!(ensures.len(), 3);
        assert!(ensures.iter().all(|c| c["source"]["available"] == true));
        assert_eq!(
            ensures
                .iter()
                .map(|c| c["source"]["snippet"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["x < 10", "x != 4", "y == 2"]
        );
        assert_eq!(result["requires"].as_array().unwrap().len(), 1);
        assert_eq!(result["requires"][0]["source"]["snippet"], "x > 0");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn keeps_repeated_calls_and_marks_missing_native_callee() {
        let dir = std::env::temp_dir().join(format!(
            "contract-trace-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("source.rs"),
            "fn caller() { callee(); callee(); missing(); }\nfn callee(x: usize) -> usize { x }\n",
        )
        .unwrap();
        let vir = r#"
(@ "source.rs:1:1: 1:47 (#0)" (Function :name (Fun :path crate::caller) :mode Exec :params () :inner_ret (@ "source.rs:1:10: 1:10 (#0)" (Param :name (VarIdent "ret") :typ (Typ Unit))) :require () :ensure () :attrs (FunctionAttrs :is_external_body false) :body (@ "source.rs:1:15: 1:47 (#0)" (Block
  (@ "source.rs:1:16: 1:25 (#0)" (> Call :target (CallTarget Fun (CallTargetKind Static) (Fun :path crate::callee)) :args ()))
  (@ "source.rs:1:26: 1:35 (#0)" (> Call :target (CallTarget Fun (CallTargetKind Static) (Fun :path crate::callee)) :args ()))
  (@ "source.rs:1:36: 1:46 (#0)" (> Call :target (CallTarget Fun (CallTargetKind Static)) :args ()))))))
(@ "source.rs:2:1: 2:37 (#0)" (Function :name (Fun :path crate::callee) :mode Exec :params ((@ "source.rs:2:11: 2:19 (#0)" (Param :name (VarIdent "x") :typ (Typ Int)))) :inner_ret (@ "source.rs:2:24: 2:35 (#0)" (Param :name (VarIdent "ret") :typ (Typ Int))) :require () :ensure () :attrs (FunctionAttrs :is_external_body false) :body (Block)))
"#;
        let result = summarize(vir, "crate::caller", &dir).unwrap();
        let calls = result["call_sites"].as_array().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0]["callee"]["status"], "indexed");
        assert_eq!(calls[1]["callee"]["status"], "indexed");
        assert_eq!(calls[2]["callee"]["status"], "unresolved");
        assert_eq!(calls[0]["callee"]["path"], "crate::callee");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dynamic_resolved_call_uses_impl_contract_not_declared_trait_contract() {
        let vir = r#"
(@ "source.rs:1:1: 1:10 (#0)" (Function :name (Fun :path crate::caller) :mode Exec :params () :inner_ret (@ "source.rs:1:1: 1:1 (#0)" (Param :name (VarIdent "ret") :typ (Typ Unit))) :require () :ensure () :attrs (FunctionAttrs :is_external_body false) :body (Block
  (> Call :target (CallTarget Fun (CallTargetKind DynamicResolved :resolved (Fun :path crate::Impl::method) :typs () :impl_paths () :is_trait_default false) (Fun :path crate::Trait::method) () () (CallTargetAttrs :autospec (AutospecUsage IfMarked) :const_var false :assume_external_allowed false)) :args ()))))
(@ "source.rs:2:1: 2:10 (#0)" (Function :name (Fun :path crate::Impl::method) :mode Exec :params () :inner_ret (@ "source.rs:2:1: 2:1 (#0)" (Param :name (VarIdent "ret") :typ (Typ Unit))) :require () :ensure ((Bool true)) :attrs (FunctionAttrs :is_external_body false) :body (Block)))
(@ "source.rs:3:1: 3:10 (#0)" (Function :name (Fun :path crate::Trait::method) :mode Exec :params () :inner_ret (@ "source.rs:3:1: 3:1 (#0)" (Param :name (VarIdent "ret") :typ (Typ Unit))) :require () :ensure ((Bool false)) :attrs (FunctionAttrs :is_external_body false) :body (Block)))
"#;

        let result = summarize(vir, "crate::caller", Path::new("/nonexistent")).unwrap();
        let calls = result["call_sites"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["callee"]["status"], "indexed");
        assert_eq!(calls[0]["callee"]["path"], "crate::Impl::method");
        assert_eq!(
            calls[0]["callee"]["contract"]["ensures"][0]["vir"],
            "(\"Bool\" \"true\")"
        );
    }
}
