//! `mutation-generate`: deterministic source mutation generation.
//!
//! v1 is deliberately conservative and function-scoped. It reads a source file,
//! extracts the named function bodies with brace matching, and applies a fixed
//! set of line-local operators. Generated mutants are exact old/new snippets
//! for [`crate::mutation::Manifest`]; compilation is not assumed. The report
//! keeps compile/yield evidence for triage.
//!
//! This is not a general Rust or Verus mutation engine. Operators are frozen and
//! the report states how many sites were skipped because the snippet was not
//! unique in the source.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, ops::Range, path::PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub target_workspace: PathBuf,
    pub source: PathBuf,
    pub package: String,
    pub module: String,
    #[serde(default)]
    pub target: Option<String>,
    pub target_dir: PathBuf,
    #[serde(default = "default_true")]
    pub full_confirm: bool,
    pub functions: Vec<String>,
    #[serde(default)]
    pub operators: Vec<String>,
    #[serde(default = "default_max")]
    pub max_mutants: usize,
}

fn default_true() -> bool {
    true
}
fn default_max() -> usize {
    80
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub function: String,
    pub operator: String,
    pub line: usize,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerationReport {
    pub source: PathBuf,
    pub functions: BTreeMap<String, usize>,
    pub generated: usize,
    pub skipped_non_unique: usize,
    pub skipped_same: usize,
    pub truncated: usize,
    pub by_operator: BTreeMap<String, usize>,
    pub candidates: Vec<Candidate>,
}

fn all_operators() -> [&'static str; 6] {
    [
        "return_constant",
        "literal_shift",
        "comparison_flip",
        "boolean_flip",
        "state_value",
        "index_zero",
    ]
}

/// Find the body of the first `fn name` in `source`. Returns the byte range of
/// the body (exclusive of braces) and the 1-based line number of `{`.
pub fn function_body(source: &str, name: &str) -> Option<(Range<usize>, usize)> {
    let pattern = format!(r"\bfn\s+{}\b", regex::escape(name));
    let re = Regex::new(&pattern).ok()?;
    let m = re.find(source)?;
    let open = source[m.end()..].find('{')? + m.end();
    let close = matching_brace(source, open)?;
    let line = source[..open].bytes().filter(|b| *b == b'\n').count() + 1;
    Some((open + 1..close, line))
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    let mut mode = 0u8; // 0 code, 1 line comment, 2 block comment, 3 string, 4 char
    while i < bytes.len() {
        let b = bytes[i];
        match mode {
            0 => match b {
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                    mode = 1;
                    i += 2;
                    continue;
                }
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    mode = 2;
                    i += 2;
                    continue;
                }
                b'"' => mode = 3,
                b'\'' => {
                    // A lifetime such as `'rcu` is not a char literal.
                    if i + 2 < bytes.len() && (bytes[i + 1] == b'\\' || bytes[i + 2] == b'\'') {
                        mode = 4;
                    }
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
            1 => {
                if b == b'\n' {
                    mode = 0;
                }
            }
            2 => {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    mode = 0;
                    i += 2;
                    continue;
                }
            }
            3 => {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == b'"' {
                    mode = 0;
                }
            }
            4 => {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == b'\'' {
                    mode = 0;
                }
            }
            _ => unreachable!(),
        }
        i += 1;
    }
    None
}

fn line_offsets(body: &str, base: usize) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, line) in body.split_inclusive('\n').enumerate() {
        out.push((start + base, line));
        let _ = i;
        start += line.len();
    }
    out
}

fn skip_line(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty()
        || t.starts_with("//")
        || t.starts_with("/*")
        || t.starts_with('*')
        || t.starts_with("#[")
        || t.starts_with("proof")
        || t.starts_with("requires")
        || t.starts_with("ensures")
        || t.starts_with("invariant")
        || t.starts_with("decreases")
        || t.starts_with("assume")
        || t.starts_with("broadcast")
        || t.starts_with("reveal")
        || t.contains("&&&")
        || t.contains("|||")
        || t.contains("==>")
        || t.contains("implies")
        || t.contains("forall")
        || t.contains("exists")
        || t.contains("old(")
        || t.contains("final(")
}

fn replace_once(line: &str, old: &str, new: &str) -> Option<String> {
    if old.is_empty() || old == new {
        return None;
    }
    Some(line.replacen(old, new, 1))
}

fn candidates_for_line(
    function: &str,
    line_no: usize,
    line: &str,
    enabled: &[String],
) -> Vec<Candidate> {
    if skip_line(line) {
        return Vec::new();
    }
    let mut out = Vec::new();
    fn push(
        out: &mut Vec<Candidate>,
        function: &str,
        line_no: usize,
        line: &str,
        operator: &str,
        old: &str,
        new: &str,
    ) {
        if let Some(s) = replace_once(line, old, new) {
            out.push(Candidate {
                function: function.to_string(),
                operator: operator.to_string(),
                line: line_no,
                old: line.to_string(),
                new: s,
            });
        }
    }

    if enabled.iter().any(|o| o == "return_constant") {
        let re = Regex::new(r"^(\s*)return\s+(0|1|true|false)\s*;\s*$").unwrap();
        if let Some(c) = re.captures(line) {
            let value = &c[2];
            let replacement = match value {
                "0" => "1",
                "1" => "0",
                "true" => "false",
                _ => "true",
            };
            let old = format!("{}{}", &c[1], value);
            let new = format!("{}{}", &c[1], replacement);
            push(
                &mut out,
                function,
                line_no,
                line,
                "return_constant",
                &old,
                &new,
            );
        }
    }

    if enabled.iter().any(|o| o == "literal_shift") {
        let re = Regex::new(r"\b(0|1|[2-9][0-9]*)\b").unwrap();
        for m in re.find_iter(line).take(2) {
            let value = m.as_str();
            let replacement = match value {
                "0" => "1".to_string(),
                "1" => "0".to_string(),
                _ => value
                    .parse::<u64>()
                    .unwrap_or(0)
                    .saturating_add(1)
                    .to_string(),
            };
            push(
                &mut out,
                function,
                line_no,
                line,
                "literal_shift",
                value,
                &replacement,
            );
        }
    }

    if enabled.iter().any(|o| o == "comparison_flip") {
        let re = Regex::new(r"(<=|>=|==|!=|<|>)").unwrap();
        for m in re.find_iter(line).take(2) {
            let replacement = match m.as_str() {
                "<" => "<=",
                "<=" => "<",
                ">" => ">=",
                ">=" => ">",
                "==" => "!=",
                _ => "==",
            };
            let mut replacement_line = line.to_string();
            replacement_line.replace_range(m.start()..m.end(), replacement);
            out.push(Candidate {
                function: function.to_string(),
                operator: "comparison_flip".to_string(),
                line: line_no,
                old: line.to_string(),
                new: replacement_line,
            });
        }
    }

    if enabled.iter().any(|o| o == "boolean_flip") {
        let re = Regex::new(r"\b(true|false)\b").unwrap();
        if let Some(m) = re.find(line) {
            let replacement = if m.as_str() == "true" {
                "false"
            } else {
                "true"
            };
            push(
                &mut out,
                function,
                line_no,
                line,
                "boolean_flip",
                m.as_str(),
                replacement,
            );
        }
    }

    if enabled.iter().any(|o| o == "state_value") {
        let re = Regex::new(r"(\.store\([^;]*,\s*)([0-9]+)(\s*\);)").unwrap();
        if let Some(c) = re.captures(line) {
            let old = format!("{}{}", &c[1], &c[2]);
            let new = format!("{}{}", &c[1], if &c[2] == "0" { "1" } else { "0" });
            push(&mut out, function, line_no, line, "state_value", &old, &new);
        }
    }

    if enabled.iter().any(|o| o == "index_zero") {
        let re = Regex::new(r"(frame_to_index|meta_to_index|from_addr)\(([^()]*)\)").unwrap();
        if let Some(c) = re.captures(line) {
            let old = format!("{}({})", &c[1], &c[2]);
            let new = format!("{}(0)", &c[1]);
            push(&mut out, function, line_no, line, "index_zero", &old, &new);
        }
    }

    out
}

/// Generate a manifest from a config. The source file is not modified.
pub fn generate(config: &Config) -> Result<(crate::mutation::Manifest, GenerationReport), String> {
    let source_path = config.target_workspace.join(&config.source);
    let source =
        fs::read_to_string(&source_path).map_err(|e| format!("{}: {e}", source_path.display()))?;
    let enabled: Vec<String> = if config.operators.is_empty() {
        all_operators().iter().map(|s| s.to_string()).collect()
    } else {
        config.operators.clone()
    };
    for op in &enabled {
        if !all_operators().contains(&op.as_str()) {
            return Err(format!("unknown operator: {op}"));
        }
    }

    let mut report = GenerationReport {
        source: config.source.clone(),
        functions: BTreeMap::new(),
        generated: 0,
        skipped_non_unique: 0,
        skipped_same: 0,
        truncated: 0,
        by_operator: BTreeMap::new(),
        candidates: Vec::new(),
    };
    let mut seen = std::collections::BTreeSet::new();
    for function in &config.functions {
        let (body, start_line) = function_body(&source, function)
            .ok_or_else(|| format!("function not found: {function}"))?;
        report.functions.insert(function.clone(), start_line);
        let body_text = &source[body.clone()];
        let mut proof_depth: i32 = 0;
        for (offset, line) in line_offsets(body_text, body.start) {
            let line_no = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
            let t = line.trim_start();
            let starts_proof = t.starts_with("proof")
                || (t.starts_with("assert") && t.contains(" by"))
                || t.starts_with("broadcast")
                || t.starts_with("reveal");
            let opens = line.matches('{').count() as i32;
            let closes = line.matches('}').count() as i32;
            if proof_depth > 0 || starts_proof {
                proof_depth += opens - closes;
                if proof_depth < 0 {
                    proof_depth = 0;
                }
                continue;
            }
            for candidate in candidates_for_line(function, line_no, line, &enabled) {
                if candidate.old == candidate.new {
                    report.skipped_same += 1;
                    continue;
                }
                if source.matches(&candidate.old).count() != 1 {
                    report.skipped_non_unique += 1;
                    continue;
                }
                let key = (
                    candidate.function.clone(),
                    candidate.old.clone(),
                    candidate.new.clone(),
                );
                if !seen.insert(key) {
                    continue;
                }
                *report
                    .by_operator
                    .entry(candidate.operator.clone())
                    .or_default() += 1;
                report.candidates.push(candidate);
            }
        }
    }
    if report.candidates.len() > config.max_mutants {
        report.truncated = report.candidates.len() - config.max_mutants;
        report.candidates.truncate(config.max_mutants);
    }
    report.generated = report.candidates.len();
    let mutations = report
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| crate::mutation::Mutation {
            id: format!("G-{:03}-{}", i + 1, c.operator),
            description: format!("{}: {} at line {}", c.operator, c.function, c.line),
            old: c.old.clone(),
            new: c.new.clone(),
        })
        .collect();
    let manifest = crate::mutation::Manifest {
        target_workspace: config.target_workspace.clone(),
        source: config.source.clone(),
        package: config.package.clone(),
        module: config.module.clone(),
        target: config.target.clone(),
        target_dir: config.target_dir.clone(),
        full_confirm: config.full_confirm,
        mutations,
    };
    Ok((manifest, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_function_body() {
        let src = "fn a() { if x { y(); } }\nfn b() { z(); }\n";
        let (range, line) = function_body(src, "a").unwrap();
        assert_eq!(&src[range], " if x { y(); } ");
        assert_eq!(line, 1);
        assert!(function_body(src, "missing").is_none());
    }

    #[test]
    fn matching_brace_ignores_lifetimes() {
        let src = "fn f<'a>(x: &'a u8) { let y = x; }\n";
        let (range, _) = function_body(src, "f").unwrap();
        assert_eq!(&src[range], " let y = x; ");
    }

    #[test]
    fn proof_regions_are_not_mutated() {
        let dir =
            std::env::temp_dir().join(format!("spec-check-mutgen-proof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let src = "fn f() {\n    proof { assert(x == 0); }\n    assert!(0 < x);\n}\n";
        std::fs::write(dir.join("src/lib.rs"), src).unwrap();
        let config = Config {
            target_workspace: dir.clone(),
            source: "src/lib.rs".into(),
            package: "p".into(),
            module: "m".into(),
            target: None,
            target_dir: dir.join("target"),
            full_confirm: false,
            functions: vec!["f".into()],
            operators: vec!["comparison_flip".into()],
            max_mutants: 10,
        };
        let (manifest, _) = generate(&config).unwrap();
        assert!(
            manifest
                .mutations
                .iter()
                .all(|m| m.old.contains("assert!(0 < x)"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generates_unique_line_mutation() {
        let dir = std::env::temp_dir().join(format!("spec-check-mutgen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn f() { return 0; }\n").unwrap();
        let config = Config {
            target_workspace: dir.clone(),
            source: "src/lib.rs".into(),
            package: "p".into(),
            module: "m".into(),
            target: None,
            target_dir: dir.join("target"),
            full_confirm: false,
            functions: vec!["f".into()],
            operators: vec!["return_constant".into()],
            max_mutants: 10,
        };
        let (manifest, report) = generate(&config).unwrap();
        assert_eq!(report.generated, 1);
        assert_eq!(manifest.mutations[0].old.trim(), "return 0;");
        assert_eq!(manifest.mutations[0].new.trim(), "return 1;");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
