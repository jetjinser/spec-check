use crate::Result;
use serde_json::{Value, json};
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    Atom(String),
    List(Vec<Node>),
}
impl Node {
    pub(crate) fn atom(&self) -> Option<&str> {
        if let Self::Atom(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub(crate) fn list(&self) -> Option<&[Self]> {
        if let Self::List(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub(crate) fn unwrap(&self) -> &Self {
        match self.list() {
            Some([head, _, body, ..]) if matches!(head.atom(), Some("@" | "@@")) => body.unwrap(),
            _ => self,
        }
    }
    pub(crate) fn field(&self, name: &str) -> Option<&Self> {
        let v = self.unwrap().list()?;
        let key = format!(":{name}");
        let i = v.iter().position(|n| n.atom() == Some(&key))?;
        v.get(i + 1)
    }
}
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1
        }
    }
    fn node(&mut self) -> Result<Node> {
        self.ws();
        let start = self.pos;
        let c = *self.bytes.get(start).ok_or("truncated VIR")?;
        self.pos += 1;
        if c == b'(' {
            let mut values = Vec::new();
            loop {
                self.ws();
                if self.bytes.get(self.pos) == Some(&b')') {
                    self.pos += 1;
                    break;
                }
                values.push(self.node()?);
            }
            return Ok(Node::List(values));
        }
        if c == b'"' {
            loop {
                match self.bytes.get(self.pos) {
                    Some(b'\\') => self.pos += 2,
                    Some(b'"') => {
                        self.pos += 1;
                        break;
                    }
                    Some(_) => self.pos += 1,
                    None => return Err("unterminated VIR string".into()),
                }
            }
            return Ok(Node::Atom(serde_json::from_slice(
                &self.bytes[start..self.pos],
            )?));
        }
        if c == b')' {
            return Err("unexpected VIR closing delimiter".into());
        }
        while self.pos < self.bytes.len()
            && !self.bytes[self.pos].is_ascii_whitespace()
            && !b"()".contains(&self.bytes[self.pos])
        {
            self.pos += 1
        }
        Ok(Node::Atom(
            std::str::from_utf8(&self.bytes[start..self.pos])?.to_owned(),
        ))
    }
}
pub(crate) fn functions(text: &str) -> Result<Vec<Node>> {
    named_items(text, "Function")
}
pub(crate) fn datatypes(text: &str) -> Result<Vec<Node>> {
    named_items(text, "Datatype")
}
pub(crate) fn function_text<'a>(text: &'a str, path: &str) -> Result<&'a str> {
    function_slice(text, path)?.ok_or_else(|| "VIR function slice missing".into())
}
fn function_slice<'a>(text: &'a str, path: &str) -> Result<Option<&'a str>> {
    let bytes = text.as_bytes();
    let mut pos = 0;
    let mut found = None;
    while pos < bytes.len() {
        while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
            pos += 1;
        }
        if pos == bytes.len() {
            break;
        }
        let start = pos;
        if bytes[pos] != b'(' {
            return Err("expected a VIR item".into());
        }
        let mut depth = 0usize;
        let mut quoted = false;
        loop {
            let byte = *bytes.get(pos).ok_or("truncated VIR item")?;
            pos += 1;
            if quoted {
                match byte {
                    b'\\' => {
                        pos += 1;
                    }
                    b'"' => quoted = false,
                    _ => {}
                }
            } else {
                match byte {
                    b'"' => quoted = true,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let slice = &text[start..pos];
        let mut header = Parser {
            bytes: slice.as_bytes(),
            pos: 0,
        };
        let kind = loop {
            header.ws();
            if header.bytes.get(header.pos) != Some(&b'(') {
                return Err("invalid VIR item header".into());
            }
            header.pos += 1;
            let head = header.node()?;
            if matches!(head.atom(), Some("@" | "@@")) {
                header.node()?;
            } else {
                break head;
            }
        };
        if kind.atom() != Some("Function") {
            continue;
        }
        loop {
            header.ws();
            if header.bytes.get(header.pos) == Some(&b')') {
                break;
            }
            let key = header.node()?;
            let value = header.node()?;
            if key.atom() == Some(":name") {
                if value.field("path").and_then(Node::atom) == Some(path) {
                    if found.is_some() {
                        return Err("ambiguous VIR function slice".into());
                    }
                    // Parse the retained item as well as its header.
                    Parser {
                        bytes: slice.as_bytes(),
                        pos: 0,
                    }
                    .node()?;
                    found = Some(slice);
                }
                break;
            }
        }
    }
    Ok(found)
}
fn named_items(text: &str, kind: &str) -> Result<Vec<Node>> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let mut result = Vec::new();
    while {
        parser.ws();
        parser.pos < parser.bytes.len()
    } {
        let node = parser.node()?;
        if node
            .unwrap()
            .list()
            .and_then(|v| v.first())
            .and_then(Node::atom)
            == Some(kind)
        {
            result.push(node);
        }
    }
    Ok(result)
}
pub(crate) fn walk_calls(node: &Node, paths: &mut Vec<String>) {
    if let Some(values) = node.list() {
        if values.first().and_then(Node::atom) == Some("CallTarget") {
            let path = values
                .iter()
                .skip(2)
                .find_map(|n| n.field("resolved").and_then(|r| r.field("path")))
                .or_else(|| values.get(3).and_then(|n| n.field("path")));
            if let Some(path) = path.and_then(Node::atom) {
                paths.push(path.to_owned())
            }
        }
        for child in values {
            walk_calls(child, paths)
        }
    }
}
pub(crate) fn call_sites(node: &Node, sites: &mut Vec<(String, String)>) {
    let Some(values) = node.list() else { return };
    let span = values
        .first()
        .and_then(Node::atom)
        .filter(|head| matches!(*head, "@" | "@@"))
        .and_then(|_| values.get(1))
        .and_then(Node::atom);
    let call = node.unwrap();
    if let Some(call_fields) = call.list()
        && call_fields.first().and_then(Node::atom) == Some(">")
        && call_fields.get(1).and_then(Node::atom) == Some("Call")
        && let (Some(span), Some(target)) = (span, call.field("target"))
    {
        let mut paths = Vec::new();
        walk_calls(target, &mut paths);
        if let [path] = paths.as_slice() {
            sites.push((span.to_owned(), path.clone()));
        }
    }
    for child in values {
        call_sites(child, sites);
    }
}
pub fn checked_target(text: &str, path: &str, package: &std::path::Path) -> Result<Option<Value>> {
    checked_target_in(text, path, package, std::path::Path::new("/"))
}

pub fn checked_target_in(
    text: &str,
    path: &str,
    package: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<Option<Value>> {
    let Some(selected) = function_slice(text, path)? else {
        return Ok(None);
    };
    let mut parser = Parser {
        bytes: selected.as_bytes(),
        pos: 0,
    };
    while {
        parser.ws();
        parser.pos < parser.bytes.len()
    } {
        let node = parser.node()?;
        let fn_ = node.unwrap();
        if fn_.list().and_then(|v| v.first()).and_then(Node::atom) != Some("Function") {
            continue;
        }
        if fn_
            .field("name")
            .and_then(|n| n.field("path"))
            .and_then(Node::atom)
            != Some(path)
        {
            continue;
        }
        let span = node
            .list()
            .and_then(|v| v.get(1))
            .and_then(Node::atom)
            .ok_or("target has no source span")?;
        let (file, _) = crate::ir::span(span, cwd)?;
        let file = std::fs::canonicalize(file)?;
        let body = fn_.field("body");
        let external = fn_
            .field("attrs")
            .and_then(|a| a.field("is_external_body"))
            .and_then(Node::atom);
        if !file.starts_with(package)
            || fn_.field("mode").and_then(Node::atom) != Some("Exec")
            || !body.is_some_and(|b| b.list().is_some())
            || external != Some("false")
        {
            return Ok(None);
        }
        let mut callees = Vec::new();
        walk_calls(body.unwrap(), &mut callees);
        callees.sort();
        callees.dedup();
        let requires = fn_
            .field("require")
            .and_then(Node::list)
            .map_or(0, |v| v.len());
        return Ok(Some(
            json!({"path":path,"span":span,"has_body":true,"existing_preconditions":requires,"callees":callees}),
        ));
    }
    Ok(None)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn call_sites_keep_source_spans_and_verified_targets() {
        let text = r#"(@ "client.rs:10:4: 10:15 (#0)" (Function :name (Fun :path p::client) :body (@@ "client.rs:12:8: 12:22 (#0)" (> Call :target (CallTarget Fun (CallTargetKind Static) (Fun :path p::impl&%0::_VERUS_VERIFIED_get) () ()) :args ()))) )"#;
        let function = functions(text).unwrap().pop().unwrap();
        let mut sites = Vec::new();
        call_sites(function.field("body").unwrap(), &mut sites);
        assert_eq!(
            sites,
            vec![(
                "client.rs:12:8: 12:22 (#0)".into(),
                "p::impl&%0::_VERUS_VERIFIED_get".into()
            )]
        );
    }

    #[test]
    fn a_library_only_proof_is_not_a_client_proof() {
        assert!(
            checked_target(
                "(@ \"/p/lib.rs:1:1: 1:4 (#0)\" (Function :name (Fun :path c::api) :mode Exec))",
                "c::test",
                std::path::Path::new("/p")
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn target_requires_a_local_checked_body() {
        let file = std::fs::canonicalize(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vir.rs"),
        )
        .unwrap();
        let text = format!(
            "(@ \"{}:1:1: 1:4 (#0)\" (Function :name (Fun :path c::test) :mode Exec :attrs (Attrs :is_external_body false) :require (false) :body (Block)))",
            file.display()
        );
        assert_eq!(
            checked_target(&text, "c::test", file.parent().unwrap())
                .unwrap()
                .unwrap()["existing_preconditions"],
            1
        );
        assert!(
            checked_target(&text, "c::test", std::path::Path::new("/different-package"))
                .unwrap()
                .is_none()
        );
        assert!(
            checked_target(
                &text.replace(":is_external_body false", ":is_external_body true"),
                "c::test",
                file.parent().unwrap()
            )
            .unwrap()
            .is_none()
        );
    }
}

#[cfg(test)]
mod slice_tests {
    use super::*;
    #[test]
    fn selected_function_preserves_raw_text_and_rejects_ambiguity() {
        let selected =
            "(@ \"file.rs:1:1\" (Function :name (Fun :path p::f) :body (Const \"(quoted)\")))";
        let source = format!("(Datatype :name T)\n{selected}\n(Function :name (Fun :path p::g))");
        assert_eq!(function_text(&source, "p::f").unwrap(), selected);
        assert!(function_text(&source, "p::missing").is_err());
        assert!(function_text(&format!("{selected}\n{selected}"), "p::f").is_err());
    }
}

#[cfg(test)]
mod slice_equivalence_tests {
    use super::*;
    #[test]
    fn selected_tree_matches_full_parser_with_escaped_strings_and_reordered_fields() {
        let text = r#"(@ "file\"(ignored)" (Datatype :name (Dt Path T)))
        (@ "位置.rs:1:1" (Function :mode Exec :body (> Const "escaped\\\"(body)") :name (Fun :path p::f)))
        (Function :name (Fun :path p::other) :body (> Const ")("))"#;
        let full = functions(text).unwrap();
        let part = functions(function_text(text, "p::f").unwrap()).unwrap();
        assert_eq!(part, vec![full[0].clone()]);
        assert!(
            checked_target_in(
                text,
                "p::missing",
                std::path::Path::new("/"),
                std::path::Path::new("/")
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    #[ignore = "requires REAL_VIR_FOR_SLICES from an actual capture"]
    fn actual_large_vir_selection_matches_full_parser() {
        let text = std::fs::read_to_string(std::env::var("REAL_VIR_FOR_SLICES").unwrap()).unwrap();
        let full = functions(&text).unwrap();
        for f in full.iter().step_by((full.len() / 12).max(1)) {
            let path = f
                .field("name")
                .unwrap()
                .field("path")
                .unwrap()
                .atom()
                .unwrap();
            assert_eq!(
                functions(function_text(&text, path).unwrap()).unwrap(),
                vec![f.clone()]
            );
        }
    }
}
