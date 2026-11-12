//! ktest assertion -> caller-obligation candidate extraction.
//!
//! Read-only, build-free extraction over a Rust source tree.  The scanner is
//! deliberately lexical (comments/strings masked with identical byte offsets)
//! because the tool must run before any target build; type information is
//! recovered only from local evidence (annotations, constructors, a
//! project-side method-variant table) and every inference records its source.
//!
//! Project-specific knowledge (private-observation patterns, environment
//! dependencies, gating cfg names, method variant tables, research anchors,
//! function overrides) lives in [`Config`], never in this module's defaults.
//!
//! Design: `docs/M7b-ktest-obligation-extraction.md`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_TEST_ATTR: &str = "ktest";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// One arch-path disconnection rule (e.g. `lib.rs` selects x86 while the file
/// lives under `arch/riscv`).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ArchRule {
    /// Top-level module name (`arch`).
    pub module: String,
    /// Path prefix inside that module that is not compiled (`riscv`).
    pub excluded_prefix: String,
    /// Regex over the crate root source that would select the excluded path.
    pub selected_path_regex: String,
}

/// Project-side extraction configuration. All fields are optional; empty
/// lists disable the corresponding project rule.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Test attributes to discover (default: `ktest`).
    pub test_attributes: Vec<String>,
    /// Cfg names marking a proof-side capture gap.
    pub capture_gap_cfgs: Vec<String>,
    /// Crate root file name (default: `lib.rs`).
    pub crate_root_file: String,
    /// Top-level modules exempt from the declaration check (e.g. `specs`).
    pub top_level_module_exclusions: Vec<String>,
    /// Arch disconnection rules.
    pub arch_rules: Vec<ArchRule>,
    /// File suffixes treated as framework self-tests (e.g. `lib.rs`).
    pub exclude_file_suffixes: Vec<String>,
    /// Assertions matching these regexes are private/test-internal observations.
    pub private_observation_patterns: Vec<String>,
    /// Body matching these regexes depends on scheduler/hardware/global state.
    pub environment_patterns: Vec<String>,
    /// Loop/iterator markers.
    pub loop_patterns: Vec<String>,
    /// Local helper name prefixes to inline (`assert`, `check`).
    pub helper_prefixes: Vec<String>,
    /// Helper names that force `needs_model_helper` even when assertions were
    /// found directly.
    pub helper_hint_patterns: Vec<String>,
    /// Test-name patterns that imply a state transition.
    pub state_name_patterns: Vec<String>,
    /// Readback fields that make a two-object comparison an invariant.
    pub invariant_field_patterns: Vec<String>,
    /// Allocator-reuse / policy assumptions (test assumptions, not contracts).
    pub allocator_reuse_patterns: Vec<String>,
    /// Iterate/consume methods whose argument spans are quantified contexts.
    pub iterator_methods: Vec<String>,
    /// Project-side method -> `option` | `result` table (medium confidence).
    pub method_variants: BTreeMap<String, String>,
    /// Test path -> research anchor label (audit only).
    pub research_anchors: BTreeMap<String, String>,
    /// Test path -> primary function override (project-side disambiguation).
    pub function_overrides: BTreeMap<String, String>,
    /// Enable one-level cross-file `assert*/check*` helper inlining.
    pub cross_file_helpers: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            test_attributes: Vec::new(),
            capture_gap_cfgs: Vec::new(),
            crate_root_file: "lib.rs".into(),
            top_level_module_exclusions: Vec::new(),
            arch_rules: Vec::new(),
            exclude_file_suffixes: Vec::new(),
            private_observation_patterns: Vec::new(),
            environment_patterns: Vec::new(),
            loop_patterns: Vec::new(),
            helper_prefixes: vec!["assert".into(), "check".into()],
            helper_hint_patterns: Vec::new(),
            state_name_patterns: Vec::new(),
            invariant_field_patterns: Vec::new(),
            allocator_reuse_patterns: Vec::new(),
            iterator_methods: vec![
                "map".into(),
                "filter_map".into(),
                "filter".into(),
                "for_each".into(),
                "any".into(),
                "all".into(),
                "find_map".into(),
                "flat_map".into(),
                "inspect".into(),
            ],
            method_variants: BTreeMap::new(),
            research_anchors: BTreeMap::new(),
            function_overrides: BTreeMap::new(),
            cross_file_helpers: false,
        }
    }
}

struct Rules {
    cfg: Config,
    test_attrs: Vec<String>,
    private: Vec<Regex>,
    env: Vec<Regex>,
    loops: Vec<Regex>,
    helper_hints: Vec<Regex>,
    state_names: Vec<Regex>,
    invariant_fields: Vec<Regex>,
    allocator_reuse: Vec<Regex>,
    iterator_methods: BTreeSet<String>,
}

fn compile(patterns: &[String]) -> Result<Vec<Regex>, String> {
    patterns
        .iter()
        .map(|p| Regex::new(p).map_err(|e| format!("invalid config regex {p:?}: {e}")))
        .collect()
}

impl Rules {
    fn new(mut cfg: Config) -> Result<Self, String> {
        if cfg.test_attributes.is_empty() {
            cfg.test_attributes.push(DEFAULT_TEST_ATTR.into());
        }
        Ok(Self {
            test_attrs: cfg.test_attributes.clone(),
            private: compile(&cfg.private_observation_patterns)?,
            env: compile(&cfg.environment_patterns)?,
            loops: compile(&cfg.loop_patterns)?,
            helper_hints: compile(&cfg.helper_hint_patterns)?,
            state_names: compile(&cfg.state_name_patterns)?,
            invariant_fields: compile(&cfg.invariant_field_patterns)?,
            allocator_reuse: compile(&cfg.allocator_reuse_patterns)?,
            iterator_methods: cfg.iterator_methods.iter().cloned().collect(),
            cfg,
        })
    }

    fn any_match(patterns: &[Regex], text: &str) -> bool {
        patterns.iter().any(|r| r.is_match(text))
    }

    fn is_private(&self, text: &str) -> bool {
        Self::any_match(&self.private, text)
    }

    fn is_env(&self, text: &str) -> bool {
        Self::any_match(&self.env, text)
    }

    fn has_loop(&self, text: &str) -> bool {
        Self::any_match(&self.loops, text)
    }

    fn has_helper_hint(&self, text: &str) -> bool {
        Self::any_match(&self.helper_hints, text)
    }

    fn is_state_name(&self, name: &str) -> bool {
        Self::any_match(&self.state_names, name)
    }

    fn has_invariant_field(&self, text: &str) -> bool {
        Self::any_match(&self.invariant_fields, text)
    }

    fn is_allocator_reuse(&self, text: &str) -> bool {
        Self::any_match(&self.allocator_reuse, text)
    }
}

// ---------------------------------------------------------------------------
// Output schema
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assertion {
    pub kind: String,
    pub category: String,
    pub text: String,
    pub line: usize,
    pub target_call: Option<String>,
    pub note: String,
    pub scope: String,
    pub control_flow: String,
    pub private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_helper: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_helper_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Demand {
    pub kind: String,
    pub demand: String,
    pub line: usize,
    pub text: String,
    pub target_call: Option<String>,
    pub category: String,
    pub scope: String,
    pub control_flow: String,
    pub variant_source: String,
    pub variant_confidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_helper: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub name: String,
    pub line: usize,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub function: String,
    pub family: String,
    pub constructibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constructibility_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constructibility_note: Option<String>,
    pub dedup_key: String,
    pub obligation_draft: String,

    pub test: String,
    pub file: String,
    pub line: usize,
    pub body_span: Value,
    pub attrs: Vec<String>,
    pub should_panic: bool,
    pub capture_gap: bool,
    pub capture_gap_reasons: Vec<String>,
    pub disconnected: bool,
    pub disconnected_reason: String,
    pub categories: Vec<String>,
    pub category: String,
    pub assertions: Vec<Assertion>,
    pub demands: Vec<Demand>,
    pub call_sequence: Vec<Call>,
    pub assertions_via_helper: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_helper_file: Option<String>,
    pub rating: String,
    pub rating_reasons: Vec<String>,
    pub research_anchor: String,
    pub priority_score: i32,
    pub function_source: String,
    pub functions: Vec<String>,
    pub model_requirements: Vec<String>,
}

// ---------------------------------------------------------------------------
// Lexical helpers
// ---------------------------------------------------------------------------

/// Blank comments, strings and char literals with spaces while preserving byte
/// offsets and newlines. Raw strings (`r#"..."#`, `br#"..."#`) are handled too.
pub fn mask_source(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let n = bytes.len();
    let blank = |out: &mut [u8], a: usize, b: usize| {
        for byte in &mut out[a..b] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < n {
        let c = bytes[i];
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let mut j = i + 2;
            while j < n && bytes[j] != b'\n' {
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < n && depth > 0 {
                if j + 1 < n && bytes[j] == b'/' && bytes[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                } else if j + 1 < n && bytes[j] == b'*' && bytes[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == b'"' {
            let mut j = i + 1;
            while j < n {
                if bytes[j] == b'\\' {
                    j = (j + 2).min(n);
                } else if bytes[j] == b'"' {
                    j += 1;
                    break;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == b'r' && (i == 0 || !is_ident(bytes[i - 1])) {
            // Raw string: r"..." / r#"..."# (also after a `b` prefix).
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < n && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                j += 1;
                while j < n {
                    if bytes[j] == b'"' {
                        let mut k = j + 1;
                        let mut seen = 0usize;
                        while k < n && seen < hashes && bytes[k] == b'#' {
                            seen += 1;
                            k += 1;
                        }
                        if seen == hashes {
                            j = k;
                            break;
                        }
                    }
                    j += 1;
                }
                blank(&mut out, i, j);
                i = j;
            } else {
                i += 1;
            }
        } else if c == b'\'' {
            let mut j = i + 1;
            if j < n && bytes[j] == b'\\' {
                j = (j + 2).min(n);
                while j < n && bytes[j] != b'\'' && bytes[j] != b'\n' {
                    j += 1;
                }
                if j < n && bytes[j] == b'\'' {
                    j += 1;
                }
                blank(&mut out, i, j);
                i = j;
            } else {
                // ASCII or multi-byte char literal, or a lifetime.
                let mut k = j;
                while k < n && k < j + 8 && bytes[k] != b'\'' && bytes[k] != b'\n'
                    && !bytes[k].is_ascii_whitespace()
                {
                    k += 1;
                }
                if k < n && bytes[k] == b'\'' {
                    blank(&mut out, i, k + 1);
                    i = k + 1;
                } else {
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn match_delim(masked: &str, open: usize) -> Option<usize> {
    let bytes = masked.as_bytes();
    let opener = *bytes.get(open)?;
    let closer = match opener {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    let mut depth = 0i32;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        if *b == opener {
            depth += 1;
        } else if *b == closer {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

fn strip_leading_comments(s: &str) -> &str {
    let mut rest = s.trim_start();
    loop {
        if let Some(r) = rest.strip_prefix("//") {
            rest = r.split_once('\n').map(|(_, r)| r.trim_start()).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("/*") {
            if let Some(end) = r.find("*/") {
                rest = r[end + 2..].trim_start();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    rest
}

fn norm(s: &str, limit: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(limit).collect();
    if collapsed.chars().count() > limit {
        out.push('…');
    }
    out
}

fn split_top_commas(text: &str) -> Vec<(usize, usize)> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                args.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push((start, text.len()));
    args
}

fn prev_delim(masked: &str, start: usize, pos: usize) -> usize {
    let mut best = start;
    for (i, b) in masked.as_bytes()[start..pos.min(masked.len())]
        .iter()
        .enumerate()
    {
        if matches!(b, b';' | b'{' | b'}') {
            best = start + i;
        }
    }
    best
}

fn next_semicolon(masked: &str, from: usize, end: usize) -> usize {
    masked.as_bytes()[from.min(masked.len())..end.min(masked.len())]
        .iter()
        .position(|b| *b == b';')
        .map(|i| from + i)
        .unwrap_or(end)
}

fn slice<'a>(raw: &'a str, start: usize, end: usize) -> &'a str {
    raw.get(start..end).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Call / classification helpers
// ---------------------------------------------------------------------------

fn method_calls(text: &str) -> Vec<(String, String)> {
    let re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .expect("static regex");
    re.captures_iter(text)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

fn field_receivers(text: &str) -> BTreeSet<String> {
    let re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)")
        .expect("static regex");
    let bytes = text.as_bytes();
    let mut out = BTreeSet::new();
    for c in re.captures_iter(text) {
        let Some(whole) = c.get(0) else { continue };
        let mut j = whole.end();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            out.insert(c[1].to_string());
        }
    }
    out
}

fn target_call(text: &str) -> Option<String> {
    let calls = method_calls(text);
    if let Some((recv, meth)) = calls.last() {
        return Some(format!("{recv}.{meth}"));
    }
    let re = Regex::new(r"\b([A-Z][A-Za-z0-9_]*::[A-Za-z0-9_]+)\s*\(").expect("static regex");
    re.captures_iter(text).last().map(|c| c[1].to_string())
}

fn bare_calls(text: &str) -> Vec<(String, usize)> {
    let re = Regex::new(r"\b([a-z_][A-Za-z0-9_]*)\s*\(").expect("static regex");
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for c in re.captures_iter(text) {
        let Some(whole) = c.get(0) else { continue };
        let start = whole.start();
        if start > 0 {
            let prev = bytes[start - 1];
            if prev == b'.' || prev == b':' || prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }
        out.push((c[1].to_string(), start));
    }
    out
}

fn is_literal(expr: &str) -> bool {
    let re = Regex::new(
        r"^(?:-?\d[\d_]*(?:\.\d+)?|true|false|None|null|[A-Z][A-Za-z0-9_:]*::[A-Za-z0-9_]+|&?\[.*\]|&?vec!\[.*\])$",
    )
    .expect("static regex");
    re.is_match(expr.trim())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpanKind {
    Iterator,
    Loop,
    If,
    IfLet,
    Match,
    LetElse,
}

struct Span {
    start: usize,
    end: usize,
    kind: SpanKind,
}

impl SpanKind {
    fn as_str(self) -> &'static str {
        match self {
            SpanKind::Iterator => "iterator",
            SpanKind::Loop => "loop",
            SpanKind::If => "if",
            SpanKind::IfLet => "if-let",
            SpanKind::Match => "match",
            SpanKind::LetElse => "let-else",
        }
    }
}

fn control_spans(masked: &str, body_start: usize, body_end: usize, rules: &Rules) -> Vec<Span> {
    let body = &masked[body_start..body_end];
    let mut spans = Vec::new();
    let mut add = |start: usize, end: usize, kind: SpanKind| {
        if end > start {
            spans.push(Span {
                start,
                end,
                kind,
            });
        }
    };

    for m in Regex::new(r"\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .expect("static regex")
        .captures_iter(body)
    {
        if !rules.iterator_methods.contains(&m[1]) {
            continue;
        }
        let Some(open) = m.get(0).map(|g| body_start + g.end() - 1) else {
            continue;
        };
        if let Some(close) = match_delim(masked, open) {
            add(open, close + 1, SpanKind::Iterator);
        }
    }

    for (kind, re) in [
        (SpanKind::Loop, r"\b(?:for|while)\b"),
        (SpanKind::Match, r"\bmatch\b"),
        (SpanKind::If, r"\bif\b"),
    ] {
        for m in Regex::new(re).expect("static regex").find_iter(body) {
            let start = body_start + m.start();
            let Some(open_rel) = masked[start..body_end].find('{') else {
                continue;
            };
            let open = start + open_rel;
            if let Some(close) = match_delim(masked, open) {
                let label = if kind == SpanKind::If {
                    let head = &masked[start..open];
                    if head.contains(" let ") {
                        SpanKind::IfLet
                    } else {
                        SpanKind::If
                    }
                } else {
                    kind
                };
                add(start, close + 1, label);
            }
        }
    }

    for m in Regex::new(r"\blet\b").expect("static regex").find_iter(body) {
        let start = body_start + m.start();
        let end = next_semicolon(masked, start, body_end);
        let stmt = &masked[start..end];
        if stmt.contains("else") {
            if let Some(open_rel) = stmt.find('{') {
                let open = start + open_rel;
                if let Some(close) = match_delim(masked, open) {
                    add(start, close + 1, SpanKind::LetElse);
                }
            }
        }
    }

    spans.sort_by_key(|s| (s.end - s.start, s.start));
    spans
}

fn context_at(spans: &[Span], pos: usize) -> (String, String) {
    let mut control = "none".to_string();
    let mut scope = "single".to_string();
    for span in spans {
        if span.start <= pos && pos < span.end {
            if control == "none" {
                control = span.kind.as_str().to_string();
            }
            if matches!(span.kind, SpanKind::Iterator | SpanKind::Loop) {
                scope = "quantified".to_string();
            }
        }
    }
    (scope, control)
}

fn classify_assertion(
    rules: &Rules,
    kind: &str,
    inner: &str,
    body_masked: &str,
) -> (String, String) {
    if rules.is_private(inner) {
        return (
            "excluded_private_observation".into(),
            "reads test-visible private state, not a caller-visible API result".into(),
        );
    }

    if Regex::new(r"\.(is_none|is_some|is_ok|is_err)\s*\(")
        .expect("static regex")
        .is_match(inner)
    {
        return (
            "existence_acquisition".into(),
            "Option/Result shape obligation".into(),
        );
    }
    if inner.contains("matches!")
        && Regex::new(r"\b(Some|None|Ok|Err)\s*[({]")
            .expect("static regex")
            .is_match(inner)
    {
        return (
            "existence_acquisition".into(),
            "pattern-matching obligation (matches!)".into(),
        );
    }
    if matches!(kind, "assert_eq" | "assert_ne")
        && Regex::new(r"\b(Some\s*[({]|None\b)")
            .expect("static regex")
            .is_match(inner)
    {
        return (
            "existence_acquisition".into(),
            "Option shape + payload value obligation".into(),
        );
    }
    if Regex::new(r"\b(Some|None|Ok|Err)\s*\(")
        .expect("static regex")
        .is_match(inner)
        && Regex::new(r"\b(matches|let|if)\b")
            .expect("static regex")
            .is_match(inner)
    {
        return (
            "existence_acquisition".into(),
            "pattern binding obligation".into(),
        );
    }

    if matches!(kind, "assert_eq" | "assert_ne") {
        let args = split_top_commas(inner);
        if args.len() >= 2 {
            let lhs = inner[args[0].0..args[0].1].trim();
            let rhs = inner[args[1].0..args[1].1].trim();
            if matches!(lhs, "true" | "false") || matches!(rhs, "true" | "false") {
                return (
                    "existence_acquisition".into(),
                    "boolean success demand".into(),
                );
            }
            let lrecv: BTreeSet<String> = method_calls(lhs)
                .into_iter()
                .map(|(r, _)| r)
                .chain(field_receivers(lhs))
                .collect();
            let rrecv: BTreeSet<String> = method_calls(rhs)
                .into_iter()
                .map(|(r, _)| r)
                .chain(field_receivers(rhs))
                .collect();
            let both_valuey =
                (!lrecv.is_empty() || !method_calls(lhs).is_empty())
                    && (!rrecv.is_empty() || !method_calls(rhs).is_empty());
            let cross = both_valuey && !lrecv.is_empty() && !rrecv.is_empty() && lrecv != rrecv;
            let relation = rules.has_invariant_field(lhs) || rules.has_invariant_field(rhs);
            if cross || (both_valuey && relation) {
                return (
                    "invariant_consistency".into(),
                    "relation between two object observations".into(),
                );
            }
            return ("value_readback".into(), "concrete value/round-trip equality".into());
        }
        return ("value_readback".into(), "equality assertion".into());
    }

    if Regex::new(r"\.(contains|is_empty|has_remain|all)\s*\(")
        .expect("static regex")
        .is_match(inner)
    {
        if rules.has_loop(body_masked)
            || Regex::new(r"\b(set|list|loaded|atomic_set)\b")
                .expect("static regex")
                .is_match(inner)
        {
            return (
                "invariant_consistency".into(),
                "membership/aggregate consistency".into(),
            );
        }
        return (
            "value_readback".into(),
            "query predicate on parsed value".into(),
        );
    }
    if Regex::new(r"\.(wake_up|wake_one|wake_all|alloc|acquire|lock|try_lock|map|unmap)\s*\(")
        .expect("static regex")
        .is_match(inner)
    {
        return (
            "existence_acquisition".into(),
            "success/acquisition boolean demand".into(),
        );
    }
    ("value_readback".into(), "boolean value predicate".into())
}

// ---------------------------------------------------------------------------
// Assertions / demands / calls
// ---------------------------------------------------------------------------

fn collect_assertions(
    rules: &Rules,
    raw: &str,
    masked: &str,
    body_start: usize,
    body_end: usize,
    spans: &[Span],
) -> Vec<Assertion> {
    let body_masked = &masked[body_start..body_end];
    let mut out = Vec::new();
    let re = Regex::new(r"\b(assert|assert_eq|assert_ne)\s*!\s*\(").expect("static regex");
    for m in re.captures_iter(body_masked) {
        let Some(whole) = m.get(0) else { continue };
        let start = body_start + whole.start();
        let Some(paren) = whole.end().checked_sub(1).map(|i| body_start + i) else {
            continue;
        };
        let Some(close) = match_delim(masked, paren) else {
            continue;
        };
        if close > body_end {
            continue;
        }
        let kind = m[1].to_string();
        let text = norm(slice(raw, start, close + 1), 400);
        let inner = &masked[paren + 1..close];
        let (category, note) = classify_assertion(rules, &kind, inner, body_masked);
        let (scope, control_flow) = context_at(spans, start);
        out.push(Assertion {
            kind,
            category: category.clone(),
            text,
            line: line_of(raw, start),
            target_call: target_call(inner),
            note,
            scope,
            control_flow,
            private: category == "excluded_private_observation",
            via_helper: None,
            via_helper_file: None,
            exclusion_reason: None,
        });
    }
    out
}

fn infer_variant(
    rules: &Rules,
    snippet_masked: &str,
    target: Option<&str>,
) -> (String, String, String) {
    let annotation = Regex::new(r"let\s+[A-Za-z_][A-Za-z0-9_]*\s*:\s*([^=;]+)=")
        .expect("static regex")
        .captures(snippet_masked);
    if let Some(c) = annotation {
        let ty = &c[1];
        if ty.contains("Result") {
            return ("Ok".into(), "type-annotation".into(), "high".into());
        }
        if ty.contains("Option") {
            return ("Some".into(), "type-annotation".into(), "high".into());
        }
    }
    let has_some = Regex::new(r"\b(Some\s*[({]|None\b|Option\s*::)")
        .expect("static regex")
        .is_match(snippet_masked);
    let has_ok = Regex::new(r"\b(Ok\s*[({]|Err\s*[({]|Result\s*::)")
        .expect("static regex")
        .is_match(snippet_masked);
    if has_some && !has_ok {
        return ("Some".into(), "constructor".into(), "high".into());
    }
    if has_ok && !has_some {
        return ("Ok".into(), "constructor".into(), "high".into());
    }
    if let Some(target) = target {
        if let Some((_, method)) = target.rsplit_once('.') {
            if let Some(variant) = rules.cfg.method_variants.get(method) {
                return match variant.as_str() {
                    "option" => ("Some".into(), "method-table".into(), "medium".into()),
                    "result" => ("Ok".into(), "method-table".into(), "medium".into()),
                    _ => (
                        "Some or Ok".into(),
                        "unknown".into(),
                        "low".into(),
                    ),
                };
            }
        }
    }
    ("Some or Ok".into(), "unknown".into(), "low".into())
}

fn collect_demands(
    rules: &Rules,
    raw: &str,
    masked: &str,
    body_start: usize,
    body_end: usize,
    spans: &[Span],
) -> Vec<Demand> {
    let body = &masked[body_start..body_end];
    let mut out = Vec::new();

    let re = Regex::new(r"\.\s*(unwrap|expect)\s*\(").expect("static regex");
    for m in re.captures_iter(body) {
        let Some(whole) = m.get(0) else { continue };
        let pos = body_start + whole.start();
        let prev = prev_delim(masked, body_start, pos);
        let next = next_semicolon(masked, body_start + whole.end(), body_end);
        let snippet_raw = norm(strip_leading_comments(slice(raw, prev + 1, next + 1)), 400);
        let snippet_masked = slice(masked, prev + 1, next + 1);
        let target = target_call(snippet_masked);
        let (demand, source, confidence) =
            infer_variant(rules, snippet_masked, target.as_deref());
        let (scope, control_flow) = context_at(spans, pos);
        out.push(Demand {
            kind: m[1].to_string(),
            demand,
            line: line_of(raw, pos),
            text: snippet_raw,
            target_call: target,
            category: "existence_acquisition".into(),
            scope,
            control_flow,
            variant_source: source,
            variant_confidence: confidence,
            via_helper: None,
        });
    }

    // `let` / `if let` / `while let` LHS patterns, including destructuring and
    // `let ... else`. The LHS is the text before the first top-level `=`.
    for m in Regex::new(r"\b(?:let|if\s+let|while\s+let)\b")
        .expect("static regex")
        .find_iter(body)
    {
        let start = body_start + m.start();
        let end = next_semicolon(masked, start, body_end);
        if end <= start {
            continue;
        }
        let stmt = &masked[start..end];
        let Some(eq) = top_level_assign(stmt) else {
            continue;
        };
        let lhs = &stmt[..eq];
        let mut found = None;
        if lhs.contains("Some(") {
            found = Some("Some");
        } else if lhs.contains("Ok(") {
            found = Some("Ok");
        } else if lhs.contains("Err(") {
            found = Some("Err");
        }
        let Some(demand) = found else { continue };
        let (scope, control_flow) = context_at(spans, start);
        let target = target_call(stmt);
        out.push(Demand {
            kind: "let-pattern".into(),
            demand: demand.into(),
            line: line_of(raw, start),
            text: norm(strip_leading_comments(slice(raw, start, end + 1)), 400),
            target_call: target,
            category: "existence_acquisition".into(),
            scope,
            control_flow,
            variant_source: "pattern".into(),
            variant_confidence: "high".into(),
            via_helper: None,
        });
    }
    out
}

fn top_level_assign(stmt: &str) -> Option<usize> {
    let bytes = stmt.as_bytes();
    let mut depth = 0i32;
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = i.checked_sub(1).and_then(|j| bytes.get(j)).copied();
                let next = bytes.get(i + 1).copied();
                if !matches!(prev, Some(b'=' | b'!' | b'<' | b'>'))
                    && !matches!(next, Some(b'=' | b'>'))
                {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn collect_calls(raw: &str, masked: &str, body_start: usize, body_end: usize) -> Vec<Call> {
    let body = &masked[body_start..body_end];
    let mut calls = Vec::new();
    let skip = BTreeSet::from([
        "unwrap",
        "expect",
        "is_none",
        "is_some",
        "is_ok",
        "is_err",
        "as_ref",
        "to_fallible",
    ]);

    for m in Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .expect("static regex")
        .captures_iter(body)
    {
        if skip.contains(&m[2]) {
            continue;
        }
        calls.push(Call {
            name: m[2].to_string(),
            line: line_of(raw, body_start + m.get(0).unwrap().start()),
            kind: "method".into(),
            receiver: Some(m[1].to_string()),
        });
    }
    for m in Regex::new(r"\b([A-Z][A-Za-z0-9_]*)::([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .expect("static regex")
        .captures_iter(body)
    {
        calls.push(Call {
            name: format!("{}::{}", &m[1], &m[2]),
            line: line_of(raw, body_start + m.get(0).unwrap().start()),
            kind: "assoc".into(),
            receiver: None,
        });
    }
    let bare_skip = BTreeSet::from([
        "if", "while", "for", "match", "Some", "Ok", "Err", "None", "return", "let", "in",
        "move", "async", "unsafe", "loop", "unwrap", "expect", "vec", "format", "panic",
        "assert", "assert_eq", "assert_ne",
    ]);
    for (name, start) in bare_calls(body) {
        if bare_skip.contains(name.as_str()) {
            continue;
        }
        calls.push(Call {
            name,
            line: line_of(raw, body_start + start),
            kind: "bare".into(),
            receiver: None,
        });
    }
    calls.sort_by_key(|c| (c.line, c.name.clone()));
    let mut seen = BTreeSet::new();
    calls.retain(|c| seen.insert((c.line, c.name.clone())));
    calls
}

// ---------------------------------------------------------------------------
// Type / function inference
// ---------------------------------------------------------------------------

fn strip_type(ty: &str) -> String {
    let mut s = ty.trim();
    for prefix in ["&", "mut ", "'_ ", "'static "] {
        while let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
        }
    }
    if let Some(angle) = s.find('<') {
        s = &s[..angle];
    }
    s.rsplit("::").next().unwrap_or(s).trim().to_string()
}

fn infer_locals(masked: &str, body_start: usize, body_end: usize) -> BTreeMap<String, String> {
    let body = &masked[body_start..body_end];
    let mut locals = BTreeMap::new();
    let re = Regex::new(
        r"\blet\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*([^=;\n]+))?\s*=\s*([^;]*)",
    )
    .expect("static regex");
    for c in re.captures_iter(body) {
        let name = c[1].to_string();
        let annotation = c.get(2).map(|m| m.as_str().trim().to_string());
        let rhs = c.get(3).map(|m| m.as_str()).unwrap_or("");
        let ty = annotation
            .as_deref()
            .map(strip_type)
            .filter(|t| !t.is_empty())
            .or_else(|| {
                let assoc =
                    Regex::new(r"^\s*(?:unsafe\s*\{\s*)?([A-Z][A-Za-z0-9_]*)\s*(?:::<[^>]*>)?\s*::")
                        .expect("static regex");
                assoc
                    .captures(rhs)
                    .map(|m| m[1].to_string())
                    .or_else(|| {
                        Regex::new(r"^\s*([A-Z][A-Za-z0-9_]*)\s*\{")
                            .expect("static regex")
                            .captures(rhs)
                            .map(|m| m[1].to_string())
                    })
                    .or_else(|| {
                        // `let f = x.to_fallible();` keeps the receiver type.
                        Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*to_(?:fallible|infallible)\s*\(")
                            .expect("static regex")
                            .captures(rhs)
                            .and_then(|m| locals.get(&m[1]).cloned())
                    })
            });
        if let Some(ty) = ty {
            locals.insert(name, ty);
        }
    }
    locals
}

fn resolve_target(target: &str, locals: &BTreeMap<String, String>) -> (String, String) {
    if target.contains("::") {
        let canonical = Regex::new(r"<[^>]*>")
            .expect("static regex")
            .replace_all(target, "");
        return (canonical.trim_matches(':').to_string(), "associated-call".into());
    }
    if let Some((recv, method)) = target.rsplit_once('.') {
        if let Some(ty) = locals.get(recv) {
            return (format!("{ty}::{method}"), "receiver-type-inference".into());
        }
        return (method.to_string(), "method-name-only".into());
    }
    (target.to_string(), "bare-call".into())
}

fn helper_function(masked: &str, open: usize, close: Option<usize>) -> Option<String> {
    let body = &masked[open + 1..close?];
    let skip = BTreeSet::from([
        "next", "into_iter", "iter", "for_each", "map", "filter", "collect", "push", "clone",
        "len", "is_empty", "contains", "unwrap", "expect", "assert", "assert_eq", "assert_ne",
        "panic", "format", "vec",
    ]);
    for (name, _) in bare_calls(body) {
        if skip.contains(name.as_str()) || name.starts_with("assert") || name.starts_with("check") {
            continue;
        }
        return Some(name);
    }
    let calls = method_calls(body);
    calls
        .into_iter()
        .rev()
        .find(|(_, meth)| !skip.contains(meth.as_str()))
        .map(|(recv, meth)| format!("{recv}.{meth}"))
}

fn derive_function(
    rules: &Rules,
    test: &str,
    assertions: &[Assertion],
    demands: &[Demand],
    locals: &BTreeMap<String, String>,
    helper_fn: Option<String>,
) -> (String, String, Vec<String>) {
    if let Some(function) = rules.cfg.function_overrides.get(test) {
        return (
            function.clone(),
            "override".into(),
            vec![function.clone()],
        );
    }
    let pure = BTreeSet::from([
        "is_none", "is_some", "is_ok", "is_err", "unwrap", "expect", "next", "len",
        "is_empty", "contains", "iter", "collect", "clone", "load", "drop", "read", "write",
        "borrow", "borrow_mut",
    ]);
    let mut scores: BTreeMap<String, (String, i32)> = BTreeMap::new();
    let mut note = |target: &str, weight: i32, source: String| {
        if target.is_empty() {
            return;
        }
        let (function, src) = resolve_target(target, locals);
        if function.is_empty() || pure.contains(function.as_str()) {
            return;
        }
        let entry = scores
            .entry(function)
            .or_insert((src.clone(), 0));
        entry.1 += weight;
        if source == "assert" && entry.0 == "method-name-only" {
            entry.0 = src;
        }
    };
    for a in assertions.iter().filter(|a| !a.private) {
        if let Some(target) = &a.target_call {
            let weight = match a.category.as_str() {
                "existence_acquisition" | "invariant_consistency" => 3,
                "value_readback" => 1,
                _ => 1,
            };
            note(target, weight, "assert".into());
        }
    }
    for d in demands {
        if let Some(target) = &d.target_call {
            note(target, 2, "demand".into());
        }
    }
    if let Some(helper) = helper_fn {
        note(&helper, 4, "helper".into());
    }
    let mut functions: Vec<(String, String, i32)> = scores
        .into_iter()
        .map(|(f, (s, score))| (f, s, score))
        .collect();
    functions.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    if let Some((function, source, _)) = functions.first() {
        return (
            function.clone(),
            source.clone(),
            functions.into_iter().map(|(f, _, _)| f).collect(),
        );
    }
    (test.to_string(), "unknown".into(), Vec::new())
}

// ---------------------------------------------------------------------------
// File loading / discovery
// ---------------------------------------------------------------------------

struct LoadedFile {
    rel: String,
    raw: String,
    masked: String,
    funcs: BTreeMap<String, (usize, usize)>,
}

impl LoadedFile {
    fn load(root: &Path, path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let masked = mask_source(&raw);
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let funcs = local_functions(&masked);
        Ok(Self {
            rel,
            raw,
            masked,
            funcs,
        })
    }
}

fn local_functions(masked: &str) -> BTreeMap<String, (usize, usize)> {
    let mut funcs = BTreeMap::new();
    let re = Regex::new(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>{}]*>)?\s*\(")
        .expect("static regex");
    for c in re.captures_iter(masked) {
        let Some(whole) = c.get(0) else { continue };
        let Some(paren) = whole.end().checked_sub(1) else {
            continue;
        };
        let Some(pclose) = match_delim(masked, paren) else {
            continue;
        };
        let Some(open_rel) = masked[pclose..].find('{') else {
            continue;
        };
        let open = pclose + open_rel;
        if let Some(close) = match_delim(masked, open) {
            funcs.insert(c[1].to_string(), (open, close));
        }
    }
    funcs
}

fn collect_rs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn file_module_parts(rel: &str) -> Vec<String> {
    let mut parts: Vec<String> = rel
        .strip_suffix(".rs")
        .unwrap_or(rel)
        .split('/')
        .map(str::to_string)
        .collect();
    if parts
        .last()
        .is_some_and(|p| p == "mod" || p == "lib")
    {
        parts.pop();
    }
    parts
}

fn enclosing_mods(masked: &str, offset: usize) -> Vec<(String, usize, String)> {
    let mut found = Vec::new();
    let re = Regex::new(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{").expect("static regex");
    for c in re.captures_iter(masked) {
        let Some(whole) = c.get(0) else { continue };
        if whole.start() >= offset {
            continue;
        }
        let Some(open) = masked[whole.start()..whole.end()].find('{') else {
            continue;
        };
        let open = whole.start() + open;
        let Some(close) = match_delim(masked, open) else {
            continue;
        };
        if whole.start() < offset && offset < close {
            let window_start = whole.start().saturating_sub(400);
            found.push((
                c[1].to_string(),
                whole.start(),
                masked[window_start..whole.start()].to_string(),
            ));
        }
    }
    found.sort_by_key(|(_, start, _)| *start);
    found
}

fn parent_module_file(root: &Path, rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel);
    let parent = path.parent().unwrap_or(Path::new(""));
    if parent.as_os_str().is_empty() {
        return Some(root.join("lib.rs"));
    }
    let mod_rs = root.join(parent).join("mod.rs");
    if mod_rs.exists() {
        return Some(mod_rs);
    }
    let sibling = root.join(parent).with_extension("rs");
    sibling.exists().then_some(sibling)
}

fn declaration_gate(root: &Path, rel: &str, rules: &Rules) -> (bool, String) {
    if rules.cfg.capture_gap_cfgs.is_empty() {
        return (false, String::new());
    }
    let stem = Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let Some(parent) = parent_module_file(root, rel) else {
        return (false, String::new());
    };
    let Ok(raw) = fs::read_to_string(&parent) else {
        return (false, String::new());
    };
    let masked = mask_source(&raw);
    let re = Regex::new(&format!(r"\bmod\s+{}\s*;", regex::escape(&stem)))
        .expect("escaped regex");
    for m in re.find_iter(&masked) {
        let window = &masked[m.start().saturating_sub(400)..m.start()];
        if rules
            .cfg
            .capture_gap_cfgs
            .iter()
            .any(|cfg| window.contains(cfg))
        {
            return (
                true,
                format!(
                    "{} declares `mod {stem};` under a capture-gap cfg",
                    parent
                        .strip_prefix(root)
                        .unwrap_or(&parent)
                        .display()
                ),
            );
        }
    }
    (false, String::new())
}

fn crate_disconnection(root: &Path, rel: &str, rules: &Rules) -> (bool, String) {
    let parts: Vec<&str> = rel.split('/').collect();
    let file_stem = Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let top = if parts.len() == 1 {
        file_stem.as_str()
    } else {
        parts.first().copied().unwrap_or("")
    };
    let root_file = root.join(&rules.cfg.crate_root_file);
    let Ok(raw_lib) = fs::read_to_string(&root_file) else {
        return (false, String::new());
    };
    let masked_lib = mask_source(&raw_lib);
    if rel != rules.cfg.crate_root_file
        && !top.is_empty()
        && !rules
            .cfg
            .top_level_module_exclusions
            .iter()
            .any(|e| e == top)
    {
        let re = Regex::new(&format!(
            r"\b(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+{}\s*;",
            regex::escape(top)
        ))
        .expect("escaped regex");
        if !re.is_match(&masked_lib) {
            return (
                true,
                format!("top-level module `{top}` is not declared (or is commented out) in {}", rules.cfg.crate_root_file),
            );
        }
    }
    for rule in &rules.cfg.arch_rules {
        if parts.first() == Some(&rule.module.as_str())
            && rel.starts_with(&format!("{}/{}/", rule.module, rule.excluded_prefix))
        {
            let selected = Regex::new(&rule.selected_path_regex)
                .map(|re| re.is_match(&raw_lib))
                .unwrap_or(false);
            if !selected {
                return (
                    true,
                    format!(
                        "{} selects a non-{} `{}` path; {}/{}/ tests are not compiled",
                        rules.cfg.crate_root_file, rule.excluded_prefix, rule.module,
                        rule.module, rule.excluded_prefix
                    ),
                );
            }
        }
    }
    (false, String::new())
}

fn attrs_before(masked: &str, pos: usize) -> Vec<String> {
    let window_start = pos.saturating_sub(400);
    let window = &masked[window_start..pos];
    let cut = window
        .rfind('}')
        .into_iter()
        .chain(window.rfind(';'))
        .max()
        .map(|i| i + 1)
        .unwrap_or(0);
    let window = &window[cut..];
    Regex::new(r"#\[[^\]]*\]")
        .expect("static regex")
        .find_iter(window)
        .map(|m| norm(m.as_str(), 120))
        .collect()
}

struct TestRecord {
    id: String,
    test: String,
    file_idx: usize,
    file: String,
    line: usize,
    body_start: usize,
    body_end: usize,
    body_start_line: usize,
    body_end_line: usize,
    attrs: Vec<String>,
    should_panic: bool,
    capture_gap: bool,
    capture_gap_reasons: Vec<String>,
    disconnected: bool,
    disconnected_reason: String,
    assertions: Vec<Assertion>,
    demands: Vec<Demand>,
    calls: Vec<Call>,
    via_helper: Vec<String>,
    via_helper_file: Option<String>,
    rating: String,
    rating_reasons: Vec<String>,
    categories: Vec<String>,
    category: String,
    function: String,
    function_source: String,
    functions: Vec<String>,
    priority_score: i32,
    research_anchor: String,
    anchor: Option<String>,
    model_requirements: Vec<String>,
}

fn discover(root: &Path, files: &[LoadedFile], rules: &Rules) -> Vec<TestRecord> {
    let mut tests = Vec::new();
    let mut next_id = 1usize;
    for (file_idx, file) in files.iter().enumerate() {
        let attr_re = Regex::new(&format!(
            r"#\[\s*(?:{})\b[^\]]*\]",
            rules
                .test_attrs
                .iter()
                .map(|a| regex::escape(a))
                .collect::<Vec<_>>()
                .join("|")
        ))
        .expect("escaped regex");
        for m in attr_re.find_iter(&file.masked) {
            let attr_start = m.start();
            let after_end = (m.end() + 600).min(file.masked.len());
            let after = &file.masked[m.end()..after_end];
            let Some(fn_rel) = Regex::new(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
                .expect("static regex")
                .captures(after)
            else {
                continue;
            };
            let fn_pos = m.end() + fn_rel.get(0).unwrap().start();
            let fn_name = fn_rel[1].to_string();
            let attrs = attrs_before(&file.masked, fn_pos);
            let should_panic = attrs.iter().any(|a| a.contains("should_panic"));
            let cfg_gap_attr = attrs.iter().any(|a| {
                rules
                    .cfg
                    .capture_gap_cfgs
                    .iter()
                    .any(|cfg| a.contains(cfg))
            });
            let Some(open_rel) = file.masked[fn_pos..].find('{') else {
                continue;
            };
            let body_start = fn_pos + open_rel;
            let Some(body_end) = match_delim(&file.masked, body_start) else {
                continue;
            };
            if fn_pos < attr_start {
                continue;
            }
            let mods = enclosing_mods(&file.masked, attr_start);
            let inline_gap = mods.iter().any(|(_, _, window)| {
                rules
                    .cfg
                    .capture_gap_cfgs
                    .iter()
                    .any(|cfg| window.contains(cfg))
            });
            let (decl_gap, decl_reason) = declaration_gate(root, &file.rel, rules);
            let (disconnected, disconnected_reason) =
                crate_disconnection(root, &file.rel, rules);

            let mut capture_gap_reasons = Vec::new();
            if inline_gap || cfg_gap_attr {
                capture_gap_reasons
                    .push("inline module/attribute gated by a capture-gap cfg".into());
            }
            if !decl_reason.is_empty() {
                capture_gap_reasons.push(decl_reason);
            }

            let spans = control_spans(&file.masked, body_start + 1, body_end, rules);
            let mut assertions = collect_assertions(
                rules,
                &file.raw,
                &file.masked,
                body_start + 1,
                body_end,
                &spans,
            );
            let demands = collect_demands(
                rules,
                &file.raw,
                &file.masked,
                body_start + 1,
                body_end,
                &spans,
            );
            let calls = collect_calls(
                &file.raw,
                &file.masked,
                body_start + 1,
                body_end,
            );
            let locals = infer_locals(&file.masked, body_start + 1, body_end);

            let mut via_helper = Vec::new();
            let mut via_helper_file = None;
            let mut helper_spans = Vec::new();
            if assertions.is_empty() {
                let called: BTreeSet<String> =
                    calls.iter().map(|c| c.name.clone()).collect();
                for (hname, (hopen, hclose)) in &file.funcs {
                    if called.contains(hname)
                        && rules
                            .cfg
                            .helper_prefixes
                            .iter()
                            .any(|p| hname.starts_with(p))
                    {
                        let hspans =
                            control_spans(&file.masked, *hopen + 1, *hclose, rules);
                        let mut hasserts = collect_assertions(
                            rules,
                            &file.raw,
                            &file.masked,
                            *hopen + 1,
                            *hclose,
                            &hspans,
                        );
                        if !hasserts.is_empty() {
                            for a in &mut hasserts {
                                a.via_helper = Some(hname.clone());
                                a.via_helper_file = Some(file.rel.clone());
                            }
                            assertions.extend(hasserts);
                            via_helper.push(hname.clone());
                            helper_spans.push((file_idx, *hopen, *hclose));
                        }
                    }
                }
                if assertions.is_empty() && rules.cfg.cross_file_helpers {
                    for (other_idx, other) in files.iter().enumerate() {
                        for (hname, (hopen, hclose)) in &other.funcs {
                            if !called.contains(hname)
                                || !rules
                                    .cfg
                                    .helper_prefixes
                                    .iter()
                                    .any(|p| hname.starts_with(p))
                            {
                                continue;
                            }
                            let hspans =
                                control_spans(&other.masked, *hopen + 1, *hclose, rules);
                            let mut hasserts = collect_assertions(
                                rules,
                                &other.raw,
                                &other.masked,
                                *hopen + 1,
                                *hclose,
                                &hspans,
                            );
                            if !hasserts.is_empty() {
                                for a in &mut hasserts {
                                    a.via_helper = Some(hname.clone());
                                    a.via_helper_file = Some(other.rel.clone());
                                }
                                assertions.extend(hasserts);
                                via_helper.push(hname.clone());
                                via_helper_file = Some(other.rel.clone());
                                helper_spans.push((other_idx, *hopen, *hclose));
                                break;
                            }
                        }
                        if !assertions.is_empty() {
                            break;
                        }
                    }
                }
            }

            let mut name_parts = file_module_parts(&file.rel);
            name_parts.extend(mods.iter().map(|(n, _, _)| n.clone()));
            name_parts.push(fn_name);
            let test = name_parts.join("::");

            let helper_fn = helper_spans.first().and_then(|(idx, open, close)| {
                helper_function(&files[*idx].masked, *open, Some(*close))
            });
            let (function, function_source, functions) = derive_function(
                rules,
                &test,
                &assertions,
                &demands,
                &locals,
                helper_fn,
            );

            let mut record = TestRecord {
                id: format!("K{next_id:03}"),
                test,
                file_idx,
                file: file.rel.clone(),
                line: line_of(&file.raw, attr_start),
                body_start,
                body_end,
                body_start_line: line_of(&file.raw, body_start),
                body_end_line: line_of(&file.raw, body_end),
                attrs,
                should_panic,
                capture_gap: inline_gap || cfg_gap_attr || decl_gap,
                capture_gap_reasons,
                disconnected,
                disconnected_reason,
                assertions,
                demands,
                calls,
                via_helper,
                via_helper_file,
                rating: String::new(),
                rating_reasons: Vec::new(),
                categories: Vec::new(),
                category: String::new(),
                function,
                function_source,
                functions,
                priority_score: 0,
                research_anchor: String::new(),
                anchor: None,
                model_requirements: Vec::new(),
            };
            annotate_exclusions(rules, &mut record, &file.raw);
            rate(&rules, &mut record, &file.raw);
            tests.push(record);
            next_id += 1;
        }
    }
    tests
}

fn annotate_exclusions(rules: &Rules, test: &mut TestRecord, raw: &str) {
    let body = slice(raw, test.body_start + 1, test.body_end);
    for a in &mut test.assertions {
        if a.category == "excluded_private_observation" {
            a.exclusion_reason = Some("private_observation".into());
            continue;
        }
        if matches!(a.kind.as_str(), "assert_eq" | "assert_ne") {
            if let Some(open) = a.text.find('(') {
                if let Some(close) = match_delim(&a.text, open) {
                    let args = split_top_commas(&a.text[open + 1..close]);
                    if args.len() >= 2 {
                        let lhs = &a.text[open + 1 + args[0].0..open + 1 + args[0].1];
                        let rhs = &a.text[open + 1 + args[1].0..open + 1 + args[1].1];
                        if is_literal(lhs) && is_literal(rhs) {
                            a.category = "excluded_tautology".into();
                            a.exclusion_reason = Some("tautology".into());
                        }
                    }
                }
            }
        }
        if body.contains("drop(")
            && a.text.contains("start_paddr")
            && Regex::new(r"_before|\bpaddr\b")
                .expect("static regex")
                .is_match(&a.text)
        {
            a.category = "excluded_test_assumption".into();
            a.exclusion_reason = Some("test_assumption_allocator_reuse".into());
        }
        if rules.is_allocator_reuse(&a.text) {
            a.category = "excluded_test_assumption".into();
            a.exclusion_reason = Some("test_assumption".into());
        }
    }
}

fn has_private_only(assertions: &[Assertion]) -> bool {
    !assertions.is_empty()
        && assertions
            .iter()
            .all(|a| a.category == "excluded_private_observation")
}

fn rate(rules: &Rules, test: &mut TestRecord, raw: &str) {
    let body = slice(raw, test.body_start + 1, test.body_end);
    let mut reasons = Vec::new();

    if rules
        .cfg
        .exclude_file_suffixes
        .iter()
        .any(|s| test.file.ends_with(s))
    {
        test.rating = "excluded".into();
        test.rating_reasons = vec!["framework self-test, not an API requirement".into()];
        return;
    }
    if test.should_panic {
        test.rating = "excluded".into();
        test.rating_reasons =
            vec!["#[should_panic]: expected panic is outside the obligation-client model".into()];
        return;
    }

    // Mirrors the prototype rating rule: only private observations are
    // dropped from the candidate's kept set; test-assumption assertions stay
    // as evidence (they are excluded from obligation counting elsewhere).
    let kept: Vec<&Assertion> = test
        .assertions
        .iter()
        .filter(|a| a.category != "excluded_private_observation")
        .collect();
    let categories: BTreeSet<String> =
        kept.iter().map(|a| a.category.clone()).collect();

    if kept.is_empty() {
        if !test.assertions.is_empty() {
            test.rating = "excluded".into();
            test.rating_reasons = vec![
                "all assertions are test-internal observations (private/tautology), not caller obligations"
                    .into(),
            ];
            return;
        }
        if test.demands.is_empty() {
            test.rating = "excluded".into();
            test.rating_reasons = vec!["no assertion and no success demand".into()];
            return;
        }
        reasons.push("only success demands (unwrap/expect) remain; no value/shape assertion".into());
    }

    if test.capture_gap {
        reasons.extend(test.capture_gap_reasons.clone());
        reasons.push("excluded from the proof-side ktest capture".into());
        test.rating = "blocked".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if test.disconnected {
        reasons.push(test.disconnected_reason.clone());
        test.rating = "blocked".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if rules.is_env(body) {
        reasons.push("assertions depend on scheduler/hardware/global runtime state".into());
        test.rating = "blocked".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if has_private_only(&test.assertions) {
        test.rating = "needs_model_private".into();
        test.rating_reasons = vec!["all assertions read private/internal state".into()];
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if rules.has_helper_hint(body) || !test.via_helper.is_empty() {
        let helpers = if test.via_helper.is_empty() {
            "source helper".to_string()
        } else {
            test.via_helper.join(", ")
        };
        reasons.push(format!(
            "assertions live in helper ({helpers}); call trace must be inlined"
        ));
        test.rating = "needs_model_helper".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if rules.has_loop(body) {
        reasons.push("loops/iterators/collections; obligation needs quantified modelling".into());
        test.rating = "needs_model_quantified".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if rules.is_allocator_reuse(body) {
        reasons.push("asserts physical-address reuse across allocations (allocator policy, not API contract)".into());
        test.rating = "test_assumption".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if categories.contains("invariant_consistency") {
        reasons.push("cross-object invariant; requires exported relation predicate / tracked model".into());
        test.rating = "needs_model_invariant".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    if kept
        .iter()
        .all(|a| matches!(a.category.as_str(), "value_readback" | "existence_acquisition"))
    {
        reasons.push("public API call + observable value/shape assertion".into());
        test.rating = "direct".into();
        test.rating_reasons = reasons;
        test.categories = categories.into_iter().collect();
        test.category = primary_category(&test.categories, &test.demands);
        return;
    }
    reasons.push("mixed/unsupported assertion forms".into());
    test.rating = "needs_model".into();
    test.rating_reasons = reasons;
    test.categories = categories.into_iter().collect();
    test.category = primary_category(&test.categories, &test.demands);
}

fn primary_category(categories: &[String], demands: &[Demand]) -> String {
    for p in [
        "state_transition",
        "existence_acquisition",
        "invariant_consistency",
        "value_readback",
    ] {
        if categories.iter().any(|c| c == p) {
            return p.into();
        }
    }
    if !demands.is_empty() {
        return "existence_acquisition".into();
    }
    "excluded".into()
}

fn finalize(rules: &Rules, test: &mut TestRecord, raw: &str) {
    let name = test.test.rsplit("::").next().unwrap_or(&test.test);
    let state_name = rules.is_state_name(name);
    let body = slice(raw, test.body_start + 1, test.body_end);
    let state_body = body.contains("drop(")
        || (body.contains(".clone(") && (body.contains("is_none") || body.contains("None)")))
        || (Regex::new(r"\.(try_lock|wake_up|acquire|alloc_specific|map|unmap|query)\s*\(")
            .expect("static regex")
            .find_iter(body)
            .count()
            >= 2
            && Regex::new(r"is_none|is_some|is_err|is_ok")
                .expect("static regex")
                .is_match(body));
    if test.rating != "excluded" && (state_name || state_body) {
        if matches!(
            test.category.as_str(),
            "existence_acquisition" | "invariant_consistency" | "value_readback"
        ) {
            test.category = "state_transition".into();
        }
        if !test.categories.iter().any(|c| c == "state_transition") {
            test.categories.insert(0, "state_transition".into());
        }
    }

    let kept: Vec<&Assertion> = test
        .assertions
        .iter()
        .filter(|a| !a.category.starts_with("excluded_"))
        .collect();
    let mut score = match test.rating.as_str() {
        "direct" => 30,
        "needs_model" => 12,
        "needs_model_invariant" => 10,
        "needs_model_quantified" => 8,
        "needs_model_helper" => 18,
        "needs_model_private" => 6,
        "test_assumption" => 2,
        "blocked" => 3,
        _ => 0,
    };
    if test.category == "existence_acquisition" {
        score += 6;
    }
    if test.category == "state_transition" {
        score += 5;
    }
    if kept
        .iter()
        .any(|a| matches!(a.kind.as_str(), "assert_eq" | "assert_ne"))
    {
        score += 2;
    }
    score += kept.len().min(6) as i32;
    if let Some(anchor) = test.anchor.clone() {
        score += 20;
        test.research_anchor = anchor;
    }
    test.priority_score = score;

    let mut reqs = BTreeSet::new();
    if !test.via_helper.is_empty() {
        reqs.insert("helper-inline".to_string());
    }
    if test
        .assertions
        .iter()
        .chain(std::iter::empty())
        .any(|a| a.scope == "quantified")
        || test.demands.iter().any(|d| d.scope == "quantified")
    {
        reqs.insert("quantified-model".to_string());
    }
    if test
        .assertions
        .iter()
        .any(|a| matches!(a.control_flow.as_str(), "if-let" | "match" | "let-else"))
        || test
            .demands
            .iter()
            .any(|d| matches!(d.control_flow.as_str(), "if-let" | "match" | "let-else"))
    {
        reqs.insert("control-flow-model".to_string());
    }
    if test.demands.iter().any(|d| d.variant_source == "unknown") {
        reqs.insert("variant-disambiguation".to_string());
    }
    test.model_requirements = reqs.into_iter().collect();
}

// ---------------------------------------------------------------------------
// Candidate rendering / run
// ---------------------------------------------------------------------------

fn family_of(category: &str) -> String {
    match category {
        "value_readback" => "value-readback",
        "existence_acquisition" | "state_transition" => "is_none-demand",
        "invariant_consistency" => "final-relation",
        _ => "other",
    }
    .to_string()
}

fn constructibility_of(rating: &str) -> (&'static str, Option<String>) {
    match rating {
        "direct" => ("constructible", None),
        "needs_model_helper" => (
            "needs-recipe",
            Some("assertions are in a helper; inline the call trace".into()),
        ),
        "needs_model_quantified" => (
            "needs-recipe",
            Some("loop/iterator obligation needs a quantified model".into()),
        ),
        "needs_model_invariant" => (
            "needs-recipe",
            Some("cross-object relation needs an exported predicate/tracked model".into()),
        ),
        "needs_model_private" => (
            "needs-recipe",
            Some("private observation; no external client form".into()),
        ),
        "test_assumption" => (
            "needs-recipe",
            Some("test/allocator policy assumption; not an API contract".into()),
        ),
        _ => ("blocked", None),
    }
}

fn obligation_draft(test: &TestRecord) -> String {
    let mut lines = Vec::new();
    for a in test
        .assertions
        .iter()
        .filter(|a| !a.category.starts_with("excluded_"))
        .take(4)
    {
        lines.push(a.text.clone());
    }
    for d in test.demands.iter().take(4) {
        lines.push(d.text.clone());
    }
    if lines.is_empty() {
        format!("// no assertion/demand text for {}", test.test)
    } else {
        lines.join("\n")
    }
}

fn to_candidate(test: &TestRecord) -> Candidate {
    let (constructibility, reason) = constructibility_of(&test.rating);
    let note = match test.rating.as_str() {
        "needs_model_helper" => Some(format!(
            "via {}",
            if test.via_helper.is_empty() {
                "helper".into()
            } else {
                test.via_helper.join(", ")
            }
        )),
        _ => None,
    };
    Candidate {
        id: test.id.clone(),
        function: test.function.clone(),
        family: family_of(&test.category),
        constructibility: constructibility.into(),
        constructibility_reason: reason,
        constructibility_note: note,
        dedup_key: format!("ktest:{}", test.id),
        obligation_draft: obligation_draft(test),
        test: test.test.clone(),
        file: test.file.clone(),
        line: test.line,
        body_span: json!({
            "start_line": test.body_start_line,
            "end_line": test.body_end_line,
        }),
        attrs: test.attrs.clone(),
        should_panic: test.should_panic,
        capture_gap: test.capture_gap,
        capture_gap_reasons: test.capture_gap_reasons.clone(),
        disconnected: test.disconnected,
        disconnected_reason: test.disconnected_reason.clone(),
        categories: test.categories.clone(),
        category: test.category.clone(),
        assertions: test.assertions.clone(),
        demands: test.demands.clone(),
        call_sequence: test.calls.clone(),
        assertions_via_helper: test.via_helper.clone(),
        via_helper_file: test.via_helper_file.clone(),
        rating: test.rating.clone(),
        rating_reasons: test.rating_reasons.clone(),
        research_anchor: test.research_anchor.clone(),
        priority_score: test.priority_score,
        function_source: test.function_source.clone(),
        functions: test.functions.clone(),
        model_requirements: test.model_requirements.clone(),
    }
}

fn count(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_default() += 1;
}

/// Run extraction, write the candidate array and optional statistics.
pub fn run(
    source_root: &Path,
    config_path: Option<&Path>,
    out: &Path,
    stats_out: Option<&Path>,
) -> Result<Value, String> {
    let cfg = match config_path {
        Some(path) => serde_json::from_slice::<Config>(
            &fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?,
        None => Config::default(),
    };
    let rules = Rules::new(cfg)?;
    let paths = collect_rs(source_root)?;
    let files = paths
        .iter()
        .map(|p| LoadedFile::load(source_root, p))
        .collect::<Result<Vec<_>, _>>()?;
    let mut tests = discover(source_root, &files, &rules);
    for test in &mut tests {
        test.anchor = rules.cfg.research_anchors.get(&test.test).cloned();
        finalize(&rules, test, &files[test.file_idx].raw);
    }

    let mut candidates: Vec<Candidate> = tests.iter().map(to_candidate).collect();
    candidates.sort_by(|a, b| {
        b.priority_score
            .cmp(&a.priority_score)
            .then_with(|| a.test.cmp(&b.test))
    });

    let mut by_rating = BTreeMap::new();
    let mut by_category = BTreeMap::new();
    let mut by_assertion_category = BTreeMap::new();
    let mut by_variant = BTreeMap::new();
    let mut by_variant_source = BTreeMap::new();
    let mut by_scope = BTreeMap::new();
    let mut by_control_flow = BTreeMap::new();
    let mut by_function_source = BTreeMap::new();
    let mut kept_assertions = 0usize;
    let mut all_assertions = 0usize;
    let mut success_demands = 0usize;
    let mut all_demands = 0usize;
    let mut unknown_function = 0usize;

    for c in &candidates {
        count(&mut by_rating, &c.rating);
        count(&mut by_function_source, &c.function_source);
        if c.function_source == "unknown" {
            unknown_function += 1;
        }
        for a in &c.assertions {
            all_assertions += 1;
            if !a.category.starts_with("excluded_") {
                kept_assertions += 1;
            }
            count(&mut by_scope, &format!("assertion:{}", a.scope));
            count(
                &mut by_control_flow,
                &format!("assertion:{}", a.control_flow),
            );
            if c.rating != "excluded" {
                count(&mut by_assertion_category, &a.category);
            }
        }
        if c.rating != "excluded" {
            count(&mut by_category, &c.category);
        }
        for d in &c.demands {
            all_demands += 1;
            if c.rating != "excluded" {
                success_demands += 1;
                count(&mut by_assertion_category, &format!("demand:{}", d.demand));
            }
            count(&mut by_variant, &d.demand);
            count(&mut by_variant_source, &d.variant_source);
            count(&mut by_scope, &format!("demand:{}", d.scope));
            count(
                &mut by_control_flow,
                &format!("demand:{}", d.control_flow),
            );
        }
    }

    let non_excluded = candidates.iter().filter(|c| c.rating != "excluded").count();
    let stats = json!({
        "schema": "spec-check.ktest-stats/1",
        "source_root": source_root,
        "config": config_path,
        "tests_seen": candidates.len(),
        "candidates": non_excluded,
        "excluded": candidates.len() - non_excluded,
        "capture_gap_tests": candidates.iter().filter(|c| c.capture_gap).count(),
        "disconnected_tests": candidates.iter().filter(|c| c.disconnected).count(),
        "should_panic_tests": candidates.iter().filter(|c| c.should_panic).count(),
        "by_rating": by_rating,
        "by_category": by_category,
        "by_assertion_category": by_assertion_category,
        "by_demand_variant": by_variant,
        "by_demand_variant_source": by_variant_source,
        "by_scope": by_scope,
        "by_control_flow": by_control_flow,
        "by_function_source": by_function_source,
        "unknown_function": unknown_function,
        "kept_assertions": kept_assertions,
        "all_assertions_extracted": all_assertions,
        "assertions_via_helper_tests": candidates.iter().filter(|c| !c.assertions_via_helper.is_empty()).count(),
        "success_demands": success_demands,
        "all_demands_extracted": all_demands,
        "model_requirements": candidates.iter().flat_map(|c| c.model_requirements.iter()).fold(BTreeMap::new(), |mut m, r| { *m.entry(r.clone()).or_insert(0usize) += 1; m }),
    });

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let body = serde_json::to_string_pretty(&candidates).map_err(|e| e.to_string())?;
    fs::write(out, format!("{body}\n")).map_err(|e| format!("{}: {e}", out.display()))?;
    if let Some(path) = stats_out {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        fs::write(
            path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&stats).map_err(|e| e.to_string())?
            ),
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(stats)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "spec-check-ktest-{}-{}-{}",
            std::process::id(),
            name,
            id
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn default_rules() -> Rules {
        Rules::new(Config::default()).unwrap()
    }

    #[test]
    fn masking_preserves_offsets_and_blanks_comments_strings() {
        let src = "#[ktest]\nfn f() {\n    let a = \"#[ktest]\"; // #[ktest]\n    /* #[ktest] */\n    let b = r#\"raw #[ktest]\"#;\n    assert!(a == b);\n}\n";
        let masked = mask_source(src);
        assert_eq!(masked.len(), src.len());
        assert_eq!(masked.matches('\n').count(), src.matches('\n').count());
        assert!(masked.contains("#[ktest]"));
        assert!(!masked.contains("\"#[ktest]\""));
        assert!(!masked.contains("/* #[ktest] */"));
        assert!(!masked.contains("raw #[ktest]"));
        assert!(masked.contains("assert!"));
        // line offsets agree
        assert_eq!(line_of(src, src.find("assert!").unwrap()), 6);
        assert_eq!(line_of(&masked, masked.find("assert!").unwrap()), 6);
    }

    #[test]
    fn assertion_classification_covers_value_existence_and_invariant() {
        let mut cfg = Config::default();
        cfg.invariant_field_patterns = vec!["start_paddr".into()];
        let rules = Rules::new(cfg).unwrap();
        let (cat, _) = classify_assertion(&rules, "assert_eq", "g.value(), 5", "");
        assert_eq!(cat, "value_readback");
        let (cat, _) = classify_assertion(&rules, "assert", "lock.try_lock().is_none()", "");
        assert_eq!(cat, "existence_acquisition");
        let (cat, _) = classify_assertion(
            &rules,
            "assert_eq",
            "frame1.start_paddr(), frame2.start_paddr()",
            "",
        );
        assert_eq!(cat, "invariant_consistency");
        let mut cfg = Config::default();
        cfg.private_observation_patterns = vec![r"\.lock\.load\b".into()];
        let rules = Rules::new(cfg).unwrap();
        let (cat, _) = classify_assertion(&rules, "assert", "!lock.lock.load()", "");
        assert_eq!(cat, "excluded_private_observation");
    }

    #[test]
    fn variant_inference_records_its_source() {
        let mut cfg = Config::default();
        cfg.method_variants.insert("push".into(), "result".into());
        cfg.method_variants.insert("try_lock".into(), "option".into());
        let rules = Rules::new(cfg).unwrap();
        assert_eq!(
            infer_variant(&rules, "let x: Result<u64, ()> = f();", None),
            ("Ok".into(), "type-annotation".into(), "high".into())
        );
        assert_eq!(
            infer_variant(&rules, "let x = Some(1u64);", None),
            ("Some".into(), "constructor".into(), "high".into())
        );
        assert_eq!(
            infer_variant(&rules, "let x = regions.push(r);", Some("regions.push")),
            ("Ok".into(), "method-table".into(), "medium".into())
        );
        assert_eq!(
            infer_variant(&rules, "let x = thing.unknown();", Some("thing.unknown")),
            (
                "Some or Ok".into(),
                "unknown".into(),
                "low".into()
            )
        );
    }

    #[test]
    fn quantified_scope_marks_iterator_and_loop_bodies() {
        let rules = default_rules();
        let masked = "fn f() { let v = xs.iter().map(|x| x.unwrap()).collect(); for x in v { assert!(g(x)); } }";
        let start = masked.find('{').unwrap() + 1;
        let end = masked.rfind('}').unwrap();
        let spans = control_spans(masked, start, end, &rules);
        let map_unwrap = masked.find("x.unwrap").unwrap();
        let (scope, control) = context_at(&spans, map_unwrap);
        assert_eq!(scope, "quantified");
        assert_eq!(control, "iterator");
        let loop_assert = masked.find("assert!").unwrap();
        let (scope, control) = context_at(&spans, loop_assert);
        assert_eq!(scope, "quantified");
        assert_eq!(control, "loop");
    }

    #[test]
    fn end_to_end_fixture_is_direct_and_deterministic() {
        let root = temp_root("direct");
        fs::write(root.join("lib.rs"), "pub mod testmod;\n").unwrap();
        fs::write(
            root.join("testmod.rs"),
            r#"
mod test {
    #[ktest]
    fn one() {
        let m = Mutex::new(1u64);
        let g = m.lock();
        assert!(m.try_lock().is_none());
        drop(g);
    }

    #[ktest]
    #[should_panic]
    fn panics() {
        assert_eq!(1, 2);
    }
}
"#,
        )
        .unwrap();
        let out1 = root.join("out1.json");
        let stats1 = root.join("stats1.json");
        let stats = run(&root, None, &out1, Some(&stats1)).unwrap();
        assert_eq!(stats["tests_seen"], 2);
        assert_eq!(stats["candidates"], 1);
        assert_eq!(stats["excluded"], 1);
        let candidates: Vec<Candidate> =
            serde_json::from_slice(&fs::read(&out1).unwrap()).unwrap();
        assert_eq!(candidates.len(), 2);
        let direct = candidates.iter().find(|c| c.rating == "direct").unwrap();
        assert_eq!(direct.function, "Mutex::try_lock");
        assert_eq!(direct.family, "is_none-demand");
        assert_eq!(direct.constructibility, "constructible");
        assert_eq!(direct.category, "state_transition");
        assert!(direct
            .model_requirements
            .iter()
            .all(|r| r != "variant-disambiguation"));

        let out2 = root.join("out2.json");
        run(&root, None, &out2, None).unwrap();
        assert_eq!(fs::read(&out1).unwrap(), fs::read(&out2).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn helper_inlining_attributes_assertions_and_function() {
        let root = temp_root("helper");
        fs::write(root.join("lib.rs"), "pub mod testmod;\n").unwrap();
        fs::write(
            root.join("testmod.rs"),
            r#"
mod test {
    #[track_caller]
    fn assert_range_difference(a: core::ops::Range<usize>, b: core::ops::Range<usize>) {
        let mut res = range_difference(&a, &b);
        assert_eq!(res.next(), Some(a.start..a.end));
        assert!(res.next().is_none());
    }

    #[ktest]
    fn range_difference_contained() {
        assert_range_difference(0..10, 3..7);
    }
}
"#,
        )
        .unwrap();
        let out = root.join("out.json");
        run(&root, None, &out, None).unwrap();
        let candidates: Vec<Candidate> =
            serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        let c = &candidates[0];
        assert_eq!(c.rating, "needs_model_helper");
        assert_eq!(c.assertions_via_helper, vec!["assert_range_difference"]);
        assert_eq!(c.function, "range_difference");
        assert_eq!(c.function_source, "bare-call");
        assert_eq!(c.constructibility, "needs-recipe");
        assert!(c
            .assertions
            .iter()
            .all(|a| a.via_helper.as_deref() == Some("assert_range_difference")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn capture_gap_and_disconnected_are_blocked_not_dropped() {
        let root = temp_root("gates");
        fs::write(root.join("lib.rs"), "/*pub mod io;*/\nmod mm;\n").unwrap();
        fs::create_dir_all(root.join("mm")).unwrap();
        fs::write(
            root.join("mm/mod.rs"),
            "#[cfg(all(ktest, not(ktest_capture_gap)))]\nmod test;\n",
        )
        .unwrap();
        fs::write(
            root.join("mm/test.rs"),
            r#"
#[ktest]
fn gated() {
    assert_eq!(1, 1);
}
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("io")).unwrap();
        fs::write(
            root.join("io/mod.rs"),
            r#"
#[ktest]
fn disconnected() {
    assert_eq!(1, 1);
}
"#,
        )
        .unwrap();
        let mut cfg = Config::default();
        cfg.capture_gap_cfgs = vec!["ktest_capture_gap".into()];
        let cfg_path = root.join("config.json");
        fs::write(&cfg_path, serde_json::to_vec(&cfg).unwrap()).unwrap();
        let out = root.join("out.json");
        run(&root, Some(&cfg_path), &out, None).unwrap();
        let candidates: Vec<Candidate> =
            serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        assert_eq!(candidates.len(), 2);
        let gated = candidates
            .iter()
            .find(|c| c.test.ends_with("gated"))
            .unwrap();
        assert!(gated.capture_gap);
        assert_eq!(gated.rating, "blocked");
        let disconnected = candidates
            .iter()
            .find(|c| c.test.ends_with("disconnected"))
            .unwrap();
        assert!(disconnected.disconnected);
        assert_eq!(disconnected.rating, "blocked");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn destructuring_let_patterns_are_demands() {
        let root = temp_root("destructure");
        fs::write(root.join("lib.rs"), "pub mod testmod;\n").unwrap();
        fs::write(
            root.join("testmod.rs"),
            r#"
mod test {
    #[ktest]
    fn jump() {
        let mut cursor = table.cursor_mut();
        let (va, Some(item)) = cursor.query() else {
            panic!("expected an item");
        };
        assert_eq!(va, 0..4096);
        assert!(item.is_some());
    }
}
"#,
        )
        .unwrap();
        let out = root.join("out.json");
        run(&root, None, &out, None).unwrap();
        let candidates: Vec<Candidate> =
            serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        let demands = &candidates[0].demands;
        assert_eq!(demands.len(), 1); // the let-else destructuring pattern
        assert_eq!(demands[0].kind, "let-pattern");
        assert_eq!(demands[0].demand, "Some");
        assert_eq!(demands[0].control_flow, "let-else");
        let _ = fs::remove_dir_all(&root);
    }
}
