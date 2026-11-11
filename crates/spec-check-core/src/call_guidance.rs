//! Failure-linked, bounded spec unfolding. Verus remains the proof checker.
use crate::{
    Result, hash, ir,
    vir::{self, Node},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const FUELS: [u32; 4] = [1, 2, 4, 8];
pub const RLIMIT: u32 = 5;
const MAX_DEFINITIONS: usize = 4;
const MAX_CHAIN: usize = 3;

pub struct Plan {
    pub paths: Vec<(String, bool)>,
    pub report: Value,
}
fn path(n: &Node) -> Option<&str> {
    n.field("name")?.field("path")?.atom()
}
fn span(n: &Node) -> Option<&str> {
    let v = n.list()?;
    matches!(v.first()?.atom(), Some("@" | "@@")).then(|| v.get(1)?.atom())?
}
fn head(n: &Node) -> Option<&str> {
    let v = n.unwrap().list()?;
    v.get(usize::from(v.first()?.atom() == Some(">")))
        .and_then(Node::atom)
}
fn location(n: &Node, cwd: &Path) -> Result<(PathBuf, usize, usize, Value)> {
    let s = span(n).ok_or("missing VIR source span")?;
    let (file, coordinates) = ir::span(s, cwd)?;
    let file = fs::canonicalize(file)?;
    let bytes = fs::read(&file)?;
    let text = std::str::from_utf8(&bytes)?;
    let (a, b) = ir::offsets(text, coordinates)?;
    Ok((
        file.clone(),
        a,
        b,
        json!({"span":s,"path":file,"start":a,"end":b,"source_sha256":hash(&bytes)?,"snippet":&text[a..b]}),
    ))
}
fn calls<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    if head(n) == Some("Call") {
        out.push(n)
    }
    if let Some(v) = n.list() {
        for x in v {
            calls(x, out)
        }
    }
}
fn call_path(n: &Node) -> Option<String> {
    let mut p = Vec::new();
    vir::walk_calls(n.field("target")?, &mut p);
    p.sort();
    p.dedup();
    if p.len() == 1 { p.pop() } else { None }
}
fn accessible(n: &Node, client: &str) -> bool {
    let public = n
        .field("visibility")
        .and_then(|n| n.field("restricted_to"))
        .and_then(Node::atom)
        == Some("None");
    let body = n.field("body_visibility").and_then(Node::list);
    let body_public = body
        .and_then(|v| v.get(2))
        .and_then(|v| v.field("restricted_to"))
        .and_then(Node::atom)
        == Some("None");
    let same_module = n
        .field("owning_module")
        .and_then(Node::atom)
        .is_some_and(|m| client.starts_with(&format!("{}::", m.trim_end_matches("::"))));
    (public && body_public) || same_module
}
fn definition<'a>(p: &str, index: &BTreeMap<&str, Vec<&'a Node>>) -> Result<&'a Node> {
    let list = index
        .get(p)
        .ok_or("spec definition absent from native VIR")?;
    let [n] = list.as_slice() else {
        return Err("ambiguous VIR definition".into());
    };
    Ok(n)
}
struct Graph<'a> {
    index: BTreeMap<&'a str, Vec<&'a Node>>,
    client: &'a str,
    cwd: &'a Path,
    externs: BTreeSet<&'a str>,
    active: Vec<String>,
    definitions: BTreeMap<String, Value>,
    visited: BTreeSet<String>,
}
impl Graph<'_> {
    fn visit(&mut self, p: &str, depth: usize) -> Result<()> {
        if self.active.last().is_some_and(|x| x == p) {
            return Ok(());
        }
        if self.active.iter().any(|x| x == p) {
            return Err("mutual recursion is unsupported".into());
        }
        if self.visited.contains(p) {
            return Ok(());
        }
        if self.visited.len() + self.active.len() >= 16 {
            return Err("spec graph node budget exhausted".into());
        }
        if depth > MAX_CHAIN {
            return Err("spec dependency depth budget exhausted".into());
        }
        let f = definition(p, &self.index)?;
        if f.field("kind")
            .and_then(Node::list)
            .is_none_or(|v| v.len() != 2 || v[1].atom() != Some("Static"))
            || f.field("attrs")
                .and_then(|a| a.field("rlimit"))
                .and_then(Node::atom)
                != Some("None")
        {
            return Err("spec kind or resource override is unsupported".into());
        }
        if f.field("mode").and_then(Node::atom) != Some("Spec")
            || f.field("body").and_then(Node::list).is_none()
            || f.field("typ_params").and_then(Node::list) != Some(&[])
            || f.field("typ_bounds").and_then(Node::list) != Some(&[])
            || f.field("require").and_then(Node::list) != Some(&[])
            || f.field("attrs")
                .and_then(|a| a.field("is_external_body"))
                .and_then(Node::atom)
                != Some("false")
            || !accessible(f, self.client)
        {
            return Err("spec body, signature or visibility is unsupported".into());
        }
        let scalar = |n: &Node| {
            n.list().is_some_and(|v| {
                v.first().and_then(Node::atom) == Some("Typ")
                    && v.get(1)
                        .and_then(Node::atom)
                        .is_some_and(|k| matches!(k, "Int" | "Bool"))
            })
        };
        if !f.field("params").and_then(Node::list).is_some_and(|ps| {
            ps.iter().all(|p| {
                p.field("typ").is_some_and(scalar)
                    && p.field("mode").and_then(Node::atom) == Some("Spec")
            })
        }) || !f
            .field("outer_ret")
            .and_then(|r| r.field("typ"))
            .is_some_and(scalar)
            || f.field("attrs")
                .and_then(|a| a.field("prophecy_dependent"))
                .and_then(Node::atom)
                != Some("false")
        {
            return Err("only non-prophetic scalar spec signatures are supported".into());
        }
        let safe = regex::Regex::new(r"^[A-Za-z_]\w*(?:::[A-Za-z_]\w*)+$")?;
        if !safe.is_match(p) {
            return Err("spec path is not a supported Rust path".into());
        }
        let own = self
            .client
            .split("::")
            .next()
            .ok_or("missing client crate")?;
        let (krate, tail) = p.split_once("::").ok_or("missing crate path")?;
        if krate != own && !self.externs.contains(krate) {
            return Err("canonical spec crate is not an available extern alias".into());
        }
        let emitted = if krate == own {
            format!("crate::{tail}")
        } else {
            p.into()
        };
        let body = f.field("body").unwrap();
        let mut used = Vec::new();
        vir::walk_calls(body, &mut used);
        used.sort();
        used.dedup();
        let recursive = used.iter().any(|c| c == p);
        if recursive
            && f.field("decrease")
                .and_then(Node::list)
                .is_none_or(|x| x.is_empty())
        {
            return Err("recursive spec has no recorded decrease".into());
        }
        let (_, _, _, source) = location(f, self.cwd)?;
        self.active.push(p.into());
        for child in &used {
            self.visit(child, depth + 1)?;
        }
        self.active.pop();
        let opaque = f
            .field("opaqueness")
            .and_then(Node::list)
            .is_some_and(|x| x.len() == 2 && x[1].atom() == Some("Opaque"));
        if opaque {
            if self.definitions.len() >= MAX_DEFINITIONS {
                return Err("spec definition count budget exhausted".into());
            }
            self.definitions.insert(p.into(),json!({"path":p,"rust_path":emitted,"source":source,"self_recursive":recursive,"dependencies":used}));
        }
        self.visited.insert(p.into());
        Ok(())
    }
}
pub fn plan(
    proof: &Value,
    text: &str,
    client: &str,
    file: &Path,
    cwd: &Path,
    args: &[String],
) -> Result<Plan> {
    let functions = vir::functions(text)?;
    let mut index = BTreeMap::<&str, Vec<&Node>>::new();
    for f in &functions {
        if let Some(p) = path(f) {
            index.entry(p).or_default().push(f)
        }
    }
    let target = definition(client, &index)?;
    if target
        .field("attrs")
        .and_then(|a| a.field("rlimit"))
        .and_then(Node::atom)
        != Some("None")
    {
        return Err("target overrides the solver resource budget".into());
    }
    let (target_file, start, end, _) =
        location(target.field("body").ok_or("client body missing")?, cwd)?;
    if target_file != fs::canonicalize(file)? {
        return Err("client body belongs to another source file".into());
    }
    let mut sites = Vec::new();
    calls(target.field("body").unwrap(), &mut sites);
    let externs = args
        .windows(2)
        .filter(|x| x[0] == "--extern")
        .filter_map(|x| x[1].split_once('=').map(|(n, _)| n))
        .collect();
    let mut graph = Graph {
        index,
        client,
        cwd,
        externs,
        active: Vec::new(),
        definitions: BTreeMap::new(),
        visited: BTreeSet::new(),
    };
    let (mut links, mut rejected) = (Vec::new(), Vec::new());
    for d in proof["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["level"] == "error" && d["message"] == "precondition not satisfied")
    {
        let spans = d["spans"]
            .as_array()
            .ok_or("diagnostic has no source spans")?;
        let primaries = spans
            .iter()
            .filter(|s| s["is_primary"] == true)
            .collect::<Vec<_>>();
        let failed = spans
            .iter()
            .filter(|s| s["label"] == "failed precondition")
            .collect::<Vec<_>>();
        let ([primary], [failed]) = (primaries.as_slice(), failed.as_slice()) else {
            rejected.push(json!({"reason":"ambiguous diagnostic spans","diagnostic":d}));
            continue;
        };
        let locate = |s: &Value| -> Result<(PathBuf, usize, usize)> {
            Ok((
                fs::canonicalize(
                    cwd.join(s["file_name"].as_str().ok_or("missing diagnostic file")?),
                )?,
                usize::try_from(s["byte_start"].as_u64().ok_or("missing byte offset")?)?,
                usize::try_from(s["byte_end"].as_u64().ok_or("missing byte offset")?)?,
            ))
        };
        let (primary_file, a, b) = locate(primary)?;
        if primary_file != target_file || a < start || b > end {
            continue;
        }
        let (failed_file, fa, fb) = locate(failed)?;
        let matches = sites
            .iter()
            .filter_map(|call| {
                let (p, ca, cb, source) = location(call, cwd).ok()?;
                if p != primary_file || ca != a || cb != b {
                    return None;
                }
                let callee = call_path(call)?;
                let f = definition(&callee, &graph.index).ok()?;
                let clauses = f.field("require")?.list()?;
                let matching = clauses
                    .iter()
                    .filter_map(|c| {
                        let (p, a, b, s) = location(c, cwd).ok()?;
                        (p == failed_file && a == fa && b == fb).then_some((c, s))
                    })
                    .collect::<Vec<_>>();
                if matching.len() != 1 {
                    return None;
                }
                Some((callee, matching[0].0, source, matching[0].1.clone()))
            })
            .collect::<Vec<_>>();
        let [(callee, clause, call_source, clause_source)] = matches.as_slice() else {
            rejected.push(json!({"reason":"no unique exact call/contract link","diagnostic":d}));
            continue;
        };
        let mut roots = Vec::new();
        vir::walk_calls(clause, &mut roots);
        roots.sort();
        roots.dedup();
        let mut errors = Vec::new();
        for root in &roots {
            let old = graph.definitions.clone();
            let old_visited = graph.visited.clone();
            if let Err(e) = graph.visit(root, 0) {
                graph.active.clear();
                graph.definitions = old;
                graph.visited = old_visited;
                errors.push(json!({"path":root,"reason":e.to_string()}));
            }
        }
        links.push(json!({"callee":callee,"call":call_source,"failed_clause":clause_source,"spec_roots":roots,"unsupported":errors}));
    }
    let paths = graph
        .definitions
        .values()
        .filter_map(|v| {
            v["rust_path"]
                .as_str()
                .map(|p| (p.to_owned(), v["self_recursive"] == true))
        })
        .collect::<Vec<_>>();
    let definitions = graph.definitions.values().cloned().collect::<Vec<_>>();
    Ok(Plan {
        paths,
        report: json!({"status":if definitions.is_empty(){"no_supported_candidate"}else{"planned"},"links":links,"rejected_links":rejected,"definitions":definitions,"budget":{"fuels":FUELS,"max_definitions":MAX_DEFINITIONS,"max_dependency_depth":MAX_CHAIN,"max_graph_nodes":16,"rlimit_per_obligation":RLIMIT,"wall_seconds_per_attempt":30},"scope":"Exact source-linked failed call preconditions; accessible defined spec bodies only. Reveal changes proof guidance, not assumptions. Verus rechecks every attempt."}),
    })
}
pub fn hint(paths: &[(String, bool)], fuel: u32) -> String {
    format!(
        "\nvstd::prelude::proof! {{ {} }}\n",
        paths
            .iter()
            .map(|(p, recursive)| if *recursive {
                format!("reveal_with_fuel({p}, {fuel});")
            } else {
                format!("reveal({p});")
            })
            .collect::<Vec<_>>()
            .join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires SPEC_CALL_GUIDANCE_TARGET from an unguided documentation run"]
    fn native_failure_link_rejects_shifted_call_and_clause() {
        let root = PathBuf::from(std::env::var("SPEC_CALL_GUIDANCE_TARGET").unwrap());
        let case = root.join("sample-0");
        let inv: crate::capture::Invocation =
            serde_json::from_slice(&fs::read(root.join("context.json")).unwrap()).unwrap();
        let raw: Value =
            serde_json::from_slice(&fs::read(case.join("original.json")).unwrap()).unwrap();
        let name = raw["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "function")
            .unwrap()["path"]
            .as_str()
            .unwrap();
        let krate = &inv
            .args
            .windows(2)
            .find(|w| w[0] == "--crate-name")
            .unwrap()[1];
        let client = format!("{krate}::{name}");
        let text = fs::read_to_string(case.join("check-vir/crate.vir")).unwrap();
        let file = case.join("package/src/lib.rs");
        let mut proof: Value =
            serde_json::from_slice(&fs::read(case.join("check.json")).unwrap()).unwrap();
        let original = proof["diagnostics"][0].clone();
        proof["diagnostics"] = json!([original.clone()]);
        let valid = plan(&proof, &text, &client, &file, &inv.cwd, &inv.args).unwrap();
        assert_eq!(valid.paths.len(), 1);
        for label in ["primary", "clause"] {
            proof["diagnostics"] = json!([original.clone()]);
            let span = proof["diagnostics"][0]["spans"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|s| {
                    if label == "primary" {
                        s["is_primary"] == true
                    } else {
                        s["label"] == "failed precondition"
                    }
                })
                .unwrap();
            span["byte_start"] = json!(span["byte_start"].as_u64().unwrap() + 1);
            let rejected = plan(&proof, &text, &client, &file, &inv.cwd, &inv.args).unwrap();
            assert!(rejected.paths.is_empty());
            assert!(
                !rejected.report["rejected_links"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
