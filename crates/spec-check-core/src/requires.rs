//! M7e: census-driven screening of `requires` clauses for unsatisfiable
//! (vacuous) preconditions.
//!
//! Input is a census report produced by [`crate::census`]. The screening
//! makes three decisions per function and is deliberately conservative:
//!
//! - **Tier A**: every `requires` clause is mechanically transcribable over
//!   the referenced scalar parameters (and lifted constants). One quantifier
//!   probe per function is generated; a bounded witness search decides
//!   whether the precondition is satisfiable in the probe model.
//! - **Tier B**: the function has no tracked/ghost/reference/generic blocker,
//!   but at least one clause is only expressible as a named `uninterp spec
//!   fn`. Probes are still generated, but they are expected to be
//!   `CandidateInconclusive` (Verus is incomplete over uninterpreted
//!   predicates); they are never counted as defects.
//! - **Tier C**: the requires references a tracked/ghost parameter, a
//!   reference parameter, or the function has type parameters. No probe is
//!   generated and the reason is recorded. Tier C is **not** evidence of
//!   absence of defects.
//!
//! Functions without any `requires` clauses are outside the target set: the
//! precondition is the empty conjunction and is trivially satisfiable
//! (`no_requires`, not a screening gap).
//!
//! Project-specific knowledge (which functions exist, their types, their
//! clause text) is entirely in the census input. The rules here are generic.

use crate::{Result, write_json};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const SCHEMA: &str = "spec-check.requires-screen/1";
pub const CONFIG_SCHEMA: &str = "spec-check.requires-config/1";

/// Project-side Tier C modeling configuration. The default is exactly the
/// M7e behavior: no instantiation, no reference/ghost abstraction.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub generic_instances: Vec<GenericRule>,
    pub reference_opaque: bool,
    pub ghost_opaque: bool,
    pub projection: ProjectionConfig,
}

#[derive(Debug, Clone)]
pub struct GenericRule {
    pub pattern: String,
    pub types: BTreeMap<String, String>,
}

/// Explicit, project-side scalar projection models (M7e-projection).
#[derive(Debug, Clone, Default)]
pub struct ProjectionConfig {
    pub models: Vec<ModelDef>,
    pub rules: Vec<ProjectionRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    Int,
    Bool,
}

impl ScalarType {
    fn as_verus(self) -> &'static str {
        match self {
            // Model bodies use Verus mathematical integers. Probe entries that
            // contain a projected clause also use `int` parameters so that
            // arithmetic results and model calls type-check; entries with no
            // projected clause keep the M7e `usize` rendering.
            Self::Int => "int",
            Self::Bool => "bool",
        }
    }

    fn is_int(self) -> bool {
        matches!(self, Self::Int)
    }
}

#[derive(Debug, Clone)]
pub struct ModelParam {
    pub name: String,
    pub ty: ScalarType,
}

#[derive(Debug, Clone)]
pub struct ModelDef {
    pub name: String,
    pub params: Vec<ModelParam>,
    pub returns: ScalarType,
    /// `None` is an `uninterp` model. Otherwise a restricted scalar
    /// expression over the formal names.
    pub body: Option<ScalarExpr>,
}

#[derive(Debug, Clone)]
pub enum ProjectionKind {
    Method { callee: String },
    Field { datatype: String, field: String },
    Variant { datatype: String, variant: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relation {
    Faithful,
    OverApprox,
    UnderApprox,
}

impl Relation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Faithful => "faithful",
            Self::OverApprox => "over-approx",
            Self::UnderApprox => "under-approx",
        }
    }

    fn vacuum_evidence(self) -> bool {
        !matches!(self, Self::UnderApprox)
    }
}

#[derive(Debug, Clone)]
pub struct ProjectionRule {
    pub function_match: String,
    pub kind: ProjectionKind,
    /// Index into [`ProjectionConfig::models`].
    pub model: usize,
    pub relation: Relation,
    pub note: String,
}

/// Restricted scalar expression language for model bodies. Only literals,
/// formal variables, `!`, unary `-`, `+ - *`, comparisons, `&& || ==>`, and
/// parentheses are accepted. Anything else is a config load error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarExpr {
    Var(String),
    Int(i64),
    Bool(bool),
    Not(Box<Self>),
    Neg(Box<Self>),
    Bin(String, Box<Self>, Box<Self>),
}

impl Config {
    pub fn from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&text)?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        if value["schema"].as_str() != Some(CONFIG_SCHEMA) {
            return Err(format!("config schema must be {CONFIG_SCHEMA}").into());
        }
        let mut config = Config::default();
        for rule in value["generic_instances"].as_array().into_iter().flatten() {
            let pattern = rule["match"]
                .as_str()
                .ok_or("generic_instances entry needs a match pattern")?;
            let mut types = BTreeMap::new();
            for (name, typ) in rule["types"]
                .as_object()
                .ok_or("generic_instances entry needs a types object")?
            {
                types.insert(
                    name.clone(),
                    typ.as_str()
                        .ok_or("generic instance type must be a string")?
                        .to_owned(),
                );
            }
            config.generic_instances.push(GenericRule {
                pattern: pattern.to_owned(),
                types,
            });
        }
        config.reference_opaque = match value["reference"]["mode"].as_str().unwrap_or("off") {
            "off" => false,
            "opaque" => true,
            other => return Err(format!("unsupported reference mode: {other}").into()),
        };
        config.ghost_opaque = match value["ghost"]["mode"].as_str().unwrap_or("off") {
            "off" => false,
            "opaque" => true,
            other => return Err(format!("unsupported ghost mode: {other}").into()),
        };
        config.projection = ProjectionConfig::from_value(&value["projection"])?;
        Ok(config)
    }

    fn generic_rule(&self, path: &str) -> Option<&GenericRule> {
        self.generic_instances
            .iter()
            .find(|rule| glob_match(&rule.pattern, path))
    }

    fn is_default(&self) -> bool {
        self.generic_instances.is_empty()
            && !self.reference_opaque
            && !self.ghost_opaque
            && self.projection.models.is_empty()
            && self.projection.rules.is_empty()
    }
}

impl ProjectionConfig {
    fn from_value(value: &Value) -> Result<Self> {
        let mut config = Self::default();
        let Some(object) = value.as_object() else {
            if value.is_null() {
                return Ok(config);
            }
            return Err("projection must be an object".into());
        };
        for model in object
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = model["name"]
                .as_str()
                .ok_or("projection model needs a name")?
                .to_owned();
            if name.is_empty()
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || name.as_bytes()[0].is_ascii_digit()
            {
                return Err(format!("invalid projection model name: {name}").into());
            }
            if config.models.iter().any(|existing| existing.name == name) {
                return Err(format!("duplicate projection model name: {name}").into());
            }
            let mut params = Vec::new();
            for param in model["params"]
                .as_array()
                .ok_or("projection model needs a params array")?
            {
                let source = param
                    .as_str()
                    .ok_or("projection model param must be a string")?;
                let (param_name, ty) = source
                    .split_once(':')
                    .ok_or("projection model param must be `name: type`")?;
                let param_name = param_name.trim().to_owned();
                if param_name.is_empty()
                    || params
                        .iter()
                        .any(|existing: &ModelParam| existing.name == param_name)
                {
                    return Err(format!("invalid or duplicate model param: {param_name}").into());
                }
                params.push(ModelParam {
                    name: param_name,
                    ty: parse_scalar_type(ty.trim())?,
                });
            }
            let returns = parse_scalar_type(
                model["returns"]
                    .as_str()
                    .ok_or("projection model needs a returns type")?,
            )?;
            let body = match model.get("body") {
                None | Some(Value::Null) => None,
                Some(Value::String(source)) => {
                    let body = parse_scalar_expr(source)
                        .map_err(|error| format!("model {name} body: {error}"))?;
                    let mut formals = BTreeMap::new();
                    for param in &params {
                        formals.insert(param.name.clone(), param.ty);
                    }
                    let inferred = infer_scalar_expr(&body, &formals)
                        .map_err(|error| format!("model {name} body: {error}"))?;
                    if inferred != returns {
                        return Err(format!(
                            "model {name} body type {} does not match returns {}",
                            inferred.as_verus(),
                            returns.as_verus()
                        )
                        .into());
                    }
                    Some(body)
                }
                Some(_) => return Err(format!("model {name} body must be a string or null").into()),
            };
            config.models.push(ModelDef {
                name,
                params,
                returns,
                body,
            });
        }
        for rule in object
            .get("rules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let function_match = rule["match"].as_str().unwrap_or("*").to_owned();
            let kind = match rule["kind"].as_str() {
                Some("method") => ProjectionKind::Method {
                    callee: rule["callee"]
                        .as_str()
                        .ok_or("method projection rule needs a callee")?
                        .to_owned(),
                },
                Some("field") => ProjectionKind::Field {
                    datatype: rule["datatype"]
                        .as_str()
                        .ok_or("field projection rule needs a datatype")?
                        .to_owned(),
                    field: rule["field"]
                        .as_str()
                        .ok_or("field projection rule needs a field")?
                        .to_owned(),
                },
                Some("variant") => ProjectionKind::Variant {
                    datatype: rule["datatype"]
                        .as_str()
                        .ok_or("variant projection rule needs a datatype")?
                        .to_owned(),
                    variant: rule["variant"]
                        .as_str()
                        .ok_or("variant projection rule needs a variant")?
                        .to_owned(),
                },
                Some(other) => return Err(format!("unsupported projection kind: {other}").into()),
                None => return Err("projection rule needs a kind".into()),
            };
            if matches!(
                kind,
                ProjectionKind::Field { .. } | ProjectionKind::Variant { .. }
            ) {
                // Field models take exactly one root argument. Enforce this
                // here so a stale rule cannot silently change arity.
                let model_name = rule["model"]
                    .as_str()
                    .ok_or("projection rule needs a model")?;
                let model = config
                    .models
                    .iter()
                    .find(|model| model.name == model_name)
                    .ok_or_else(|| format!("unknown projection model: {model_name}"))?;
                if model.params.len() != 1 {
                    return Err(format!(
                        "field projection model {} must take exactly one argument",
                        model.name
                    )
                    .into());
                }
            }
            let model = {
                let name = rule["model"]
                    .as_str()
                    .ok_or("projection rule needs a model")?;
                config
                    .models
                    .iter()
                    .position(|model| model.name == name)
                    .ok_or_else(|| format!("unknown projection model: {name}"))?
            };
            let relation = match rule["relation"].as_str() {
                Some("faithful") => Relation::Faithful,
                Some("over-approx") => Relation::OverApprox,
                Some("under-approx") => Relation::UnderApprox,
                Some(other) => return Err(format!("unsupported relation: {other}").into()),
                None => return Err("projection rule needs a relation".into()),
            };
            let note = rule["note"].as_str().unwrap_or("").to_owned();
            config.rules.push(ProjectionRule {
                function_match,
                kind,
                model,
                relation,
                note,
            });
        }
        Ok(config)
    }

    fn is_empty(&self) -> bool {
        self.models.is_empty() && self.rules.is_empty()
    }

    fn matching_rule(
        &self,
        function: &str,
        callee: Option<&str>,
        datatype: Option<&str>,
        field: Option<&str>,
        variant: Option<&str>,
    ) -> Option<usize> {
        self.rules.iter().position(|rule| {
            if !glob_match(&rule.function_match, function) {
                return false;
            }
            match &rule.kind {
                ProjectionKind::Method { callee: pattern } => {
                    callee.is_some_and(|callee| glob_match(pattern, callee))
                }
                ProjectionKind::Field {
                    datatype: datatype_pattern,
                    field: field_pattern,
                } => {
                    datatype.is_some_and(|datatype| glob_match(datatype_pattern, datatype))
                        && field.is_some_and(|field| glob_match(field_pattern, field))
                }
                ProjectionKind::Variant {
                    datatype: datatype_pattern,
                    variant: variant_pattern,
                } => {
                    datatype.is_some_and(|datatype| glob_match(datatype_pattern, datatype))
                        && variant.is_some_and(|variant| glob_match(variant_pattern, variant))
                }
            }
        })
    }
}

fn parse_scalar_type(text: &str) -> Result<ScalarType> {
    match text {
        "usize" | "int" | "nat" | "u8" | "u16" | "u32" | "u64" | "u128" => Ok(ScalarType::Int),
        "bool" => Ok(ScalarType::Bool),
        other => Err(format!("unsupported scalar type: {other}").into()),
    }
}

struct ScalarParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl ScalarParser<'_> {
    fn skip(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self, token: &str) -> bool {
        self.skip();
        self.bytes[self.pos..].starts_with(token.as_bytes())
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.peek(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn expression(&mut self) -> Result<ScalarExpr> {
        self.implies()
    }

    fn implies(&mut self) -> Result<ScalarExpr> {
        let mut lhs = self.or()?;
        while self.eat("==>") {
            let rhs = self.implies()?;
            lhs = ScalarExpr::Bin("==>".into(), Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn or(&mut self) -> Result<ScalarExpr> {
        let mut lhs = self.and()?;
        while self.eat("||") {
            let rhs = self.and()?;
            lhs = ScalarExpr::Bin("||".into(), Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn and(&mut self) -> Result<ScalarExpr> {
        let mut lhs = self.compare()?;
        while self.eat("&&") {
            let rhs = self.compare()?;
            lhs = ScalarExpr::Bin("&&".into(), Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn compare(&mut self) -> Result<ScalarExpr> {
        let lhs = self.sum()?;
        for op in ["==", "!=", "<=", ">=", "<", ">"] {
            if self.eat(op) {
                let rhs = self.sum()?;
                return Ok(ScalarExpr::Bin(op.into(), Box::new(lhs), Box::new(rhs)));
            }
        }
        Ok(lhs)
    }

    fn sum(&mut self) -> Result<ScalarExpr> {
        let mut lhs = self.product()?;
        loop {
            if self.eat("+") {
                let rhs = self.product()?;
                lhs = ScalarExpr::Bin("+".into(), Box::new(lhs), Box::new(rhs));
            } else if self.eat("-") {
                let rhs = self.product()?;
                lhs = ScalarExpr::Bin("-".into(), Box::new(lhs), Box::new(rhs));
            } else {
                return Ok(lhs);
            }
        }
    }

    fn product(&mut self) -> Result<ScalarExpr> {
        let mut lhs = self.unary()?;
        while self.eat("*") {
            let rhs = self.unary()?;
            lhs = ScalarExpr::Bin("*".into(), Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<ScalarExpr> {
        if self.eat("!") {
            return Ok(ScalarExpr::Not(Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(ScalarExpr::Neg(Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<ScalarExpr> {
        self.skip();
        if self.eat("(") {
            let expression = self.expression()?;
            if !self.eat(")") {
                return Err("expected `)` in model body".into());
            }
            return Ok(expression);
        }
        let start = self.pos;
        if self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let text = std::str::from_utf8(&self.bytes[start..self.pos])?;
            return Ok(ScalarExpr::Int(text.parse::<i64>()?));
        }
        if self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphabetic() || self.bytes[self.pos] == b'_')
        {
            while self.pos < self.bytes.len()
                && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
            {
                self.pos += 1;
            }
            let text = std::str::from_utf8(&self.bytes[start..self.pos])?;
            return match text {
                "true" => Ok(ScalarExpr::Bool(true)),
                "false" => Ok(ScalarExpr::Bool(false)),
                _ => Ok(ScalarExpr::Var(text.to_owned())),
            };
        }
        Err("expected an integer, `true`/`false`, a formal name, or `(`".into())
    }
}

fn parse_scalar_expr(text: &str) -> Result<ScalarExpr> {
    let mut parser = ScalarParser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let expression = parser.expression()?;
    parser.skip();
    if parser.pos != parser.bytes.len() {
        return Err(format!(
            "unexpected trailing input at byte {} of model body",
            parser.pos
        )
        .into());
    }
    Ok(expression)
}

fn infer_scalar_expr(
    expression: &ScalarExpr,
    formals: &BTreeMap<String, ScalarType>,
) -> Result<ScalarType> {
    match expression {
        ScalarExpr::Var(name) => formals
            .get(name)
            .copied()
            .ok_or_else(|| format!("unknown formal in model body: {name}").into()),
        ScalarExpr::Int(_) => Ok(ScalarType::Int),
        ScalarExpr::Bool(_) => Ok(ScalarType::Bool),
        ScalarExpr::Not(arg) => {
            let ty = infer_scalar_expr(arg, formals)?;
            if ty != ScalarType::Bool {
                return Err("`!` expects a bool".into());
            }
            Ok(ScalarType::Bool)
        }
        ScalarExpr::Neg(arg) => {
            let ty = infer_scalar_expr(arg, formals)?;
            if ty != ScalarType::Int {
                return Err("unary `-` expects an integer".into());
            }
            Ok(ScalarType::Int)
        }
        ScalarExpr::Bin(sym, lhs, rhs) => {
            let lhs = infer_scalar_expr(lhs, formals)?;
            let rhs = infer_scalar_expr(rhs, formals)?;
            match sym.as_str() {
                "+" | "-" | "*" => {
                    if lhs != ScalarType::Int || rhs != ScalarType::Int {
                        return Err(format!("`{sym}` expects integers").into());
                    }
                    Ok(ScalarType::Int)
                }
                "<" | "<=" | ">" | ">=" => {
                    if lhs != ScalarType::Int || rhs != ScalarType::Int {
                        return Err(format!("`{sym}` expects integers").into());
                    }
                    Ok(ScalarType::Bool)
                }
                "==" | "!=" => {
                    if lhs != rhs {
                        return Err(format!("`{sym}` expects operands of the same type").into());
                    }
                    Ok(ScalarType::Bool)
                }
                "&&" | "||" | "==>" => {
                    if lhs != ScalarType::Bool || rhs != ScalarType::Bool {
                        return Err(format!("`{sym}` expects booleans").into());
                    }
                    Ok(ScalarType::Bool)
                }
                other => Err(format!("unsupported model body operator: {other}").into()),
            }
        }
    }
}

fn render_scalar_expr(expression: &ScalarExpr) -> String {
    match expression {
        ScalarExpr::Var(name) => name.clone(),
        ScalarExpr::Int(value) => value.to_string(),
        ScalarExpr::Bool(value) => value.to_string(),
        ScalarExpr::Not(arg) => format!("!({})", render_scalar_expr(arg)),
        ScalarExpr::Neg(arg) => format!("-({})", render_scalar_expr(arg)),
        ScalarExpr::Bin(sym, lhs, rhs) => format!(
            "({} {} {})",
            render_scalar_expr(lhs),
            sym,
            render_scalar_expr(rhs)
        ),
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.split_once('*') {
        None => pattern == text,
        Some((prefix, suffix)) => {
            text.len() >= prefix.len() + suffix.len()
                && text.starts_with(prefix)
                && text.ends_with(suffix)
        }
    }
}

/// A translated projected clause term. `Base` is an opaque reference/ghost
/// id and may only be consumed by a model call; every other variant is a
/// scalar term.
#[derive(Debug, Clone)]
enum Term {
    Int(i64),
    Bool(bool),
    Var {
        name: String,
        int: bool,
    },
    Base {
        param: String,
        name: String,
    },
    /// A fresh structural token: a constructor value, an uncovered `Match`
    /// else value, or a pattern binding. It may be consumed by a model call
    /// or compared for equality, but it is not an integer for arithmetic or
    /// ordering. It is an existential scalar in the probe, hence an
    /// over-approximation of the real structure.
    Opaque {
        name: String,
        int: bool,
    },
    /// A scalar operation that is exact on real (non-negative) values but
    /// admits more values over the probe's unbounded `int` domain: a `Clip`
    /// cast, `/`, or `%`.
    Relaxed {
        inner: Box<Term>,
        int: bool,
    },
    Ite {
        cond: Box<Term>,
        then: Box<Term>,
        els: Box<Term>,
        int: bool,
    },
    Call {
        model: usize,
        args: Vec<Term>,
        int: bool,
        relation: Relation,
    },
    Quant {
        forall: bool,
        bindings: Vec<(String, bool)>,
        body: Box<Term>,
    },
    Not(Box<Term>),
    Neg(Box<Term>),
    Bin {
        sym: String,
        lhs: Box<Term>,
        rhs: Box<Term>,
        int: bool,
    },
}

#[derive(Debug, Clone)]
enum ClausePlan {
    Mechanical,
    Projected(Term),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProjectionUse {
    param: String,
    kind: &'static str,
    target: String,
    model: String,
    relation: Relation,
    identity: bool,
}

impl ProjectionUse {
    fn value(&self) -> Value {
        json!({
            "param": self.param,
            "kind": self.kind,
            "target": self.target,
            "model": self.model,
            "relation": self.relation.as_str(),
            "identity": self.identity,
        })
    }
}

fn term_type(term: &Term) -> ScalarType {
    match term {
        Term::Int(_)
        | Term::Neg(_)
        | Term::Base { .. }
        | Term::Opaque { int: true, .. }
        | Term::Relaxed { int: true, .. }
        | Term::Ite { int: true, .. }
        | Term::Var { int: true, .. } => ScalarType::Int,
        Term::Bool(_)
        | Term::Not(_)
        | Term::Quant { .. }
        | Term::Opaque { int: false, .. }
        | Term::Relaxed { int: false, .. }
        | Term::Ite { int: false, .. }
        | Term::Var { int: false, .. } => ScalarType::Bool,
        Term::Call { int: true, .. } | Term::Bin { int: true, .. } => ScalarType::Int,
        Term::Call { int: false, .. } | Term::Bin { int: false, .. } => ScalarType::Bool,
    }
}

fn term_is_evaluable(term: &Term, config: &ProjectionConfig) -> bool {
    match term {
        Term::Int(_)
        | Term::Bool(_)
        | Term::Var { .. }
        | Term::Base { .. }
        | Term::Opaque { .. } => true,
        Term::Relaxed { inner, .. } => term_is_evaluable(inner, config),
        Term::Ite {
            cond, then, els, ..
        } => {
            term_is_evaluable(cond, config)
                && term_is_evaluable(then, config)
                && term_is_evaluable(els, config)
        }
        Term::Call { model, args, .. } => {
            config.models[*model].body.is_some()
                && args.iter().all(|arg| term_is_evaluable(arg, config))
        }
        Term::Quant { .. } => false,
        Term::Not(arg) | Term::Neg(arg) => term_is_evaluable(arg, config),
        Term::Bin { lhs, rhs, .. } => {
            term_is_evaluable(lhs, config) && term_is_evaluable(rhs, config)
        }
    }
}

fn term_relations(term: &Term, out: &mut Vec<Relation>) {
    match term {
        Term::Call { args, relation, .. } => {
            out.push(*relation);
            for arg in args {
                term_relations(arg, out);
            }
        }
        Term::Relaxed { inner, .. } => term_relations(inner, out),
        Term::Ite {
            cond, then, els, ..
        } => {
            term_relations(cond, out);
            term_relations(then, out);
            term_relations(els, out);
        }
        Term::Not(arg) | Term::Neg(arg) => term_relations(arg, out),
        Term::Bin { lhs, rhs, .. } => {
            term_relations(lhs, out);
            term_relations(rhs, out);
        }
        Term::Quant { body, .. } => term_relations(body, out),
        Term::Int(_)
        | Term::Bool(_)
        | Term::Var { .. }
        | Term::Base { .. }
        | Term::Opaque { .. } => {}
    }
}

fn term_models(term: &Term, out: &mut BTreeSet<usize>) {
    match term {
        Term::Call { model, args, .. } => {
            out.insert(*model);
            for arg in args {
                term_models(arg, out);
            }
        }
        Term::Relaxed { inner, .. } => term_models(inner, out),
        Term::Ite {
            cond, then, els, ..
        } => {
            term_models(cond, out);
            term_models(then, out);
            term_models(els, out);
        }
        Term::Not(arg) | Term::Neg(arg) => term_models(arg, out),
        Term::Bin { lhs, rhs, .. } => {
            term_models(lhs, out);
            term_models(rhs, out);
        }
        Term::Quant { body, .. } => term_models(body, out),
        Term::Int(_)
        | Term::Bool(_)
        | Term::Var { .. }
        | Term::Base { .. }
        | Term::Opaque { .. } => {}
    }
}

/// A structural relaxation is any abstraction that is not a declared model
/// call: a fresh constructor token, a cast, `/` or `%` over the extended
/// integer domain, or an uncovered `Match` else token. Such a clause is an
/// over-approximation even when every declared model relation is `faithful`.
fn term_structural_relaxation(term: &Term) -> bool {
    match term {
        Term::Opaque { .. } | Term::Relaxed { .. } => true,
        Term::Ite {
            cond, then, els, ..
        } => {
            term_structural_relaxation(cond)
                || term_structural_relaxation(then)
                || term_structural_relaxation(els)
        }
        Term::Call { args, .. } => args.iter().any(term_structural_relaxation),
        Term::Not(arg) | Term::Neg(arg) => term_structural_relaxation(arg),
        Term::Bin { lhs, rhs, .. } => {
            term_structural_relaxation(lhs) || term_structural_relaxation(rhs)
        }
        Term::Quant { body, .. } => term_structural_relaxation(body),
        Term::Int(_) | Term::Bool(_) | Term::Var { .. } | Term::Base { .. } => false,
    }
}

fn term_has_quantifier(term: &Term) -> bool {
    match term {
        Term::Quant { .. } => true,
        Term::Call { args, .. } => args.iter().any(term_has_quantifier),
        Term::Relaxed { inner, .. } => term_has_quantifier(inner),
        Term::Ite {
            cond, then, els, ..
        } => term_has_quantifier(cond) || term_has_quantifier(then) || term_has_quantifier(els),
        Term::Not(arg) | Term::Neg(arg) => term_has_quantifier(arg),
        Term::Bin { lhs, rhs, .. } => term_has_quantifier(lhs) || term_has_quantifier(rhs),
        Term::Int(_)
        | Term::Bool(_)
        | Term::Var { .. }
        | Term::Base { .. }
        | Term::Opaque { .. } => false,
    }
}

fn collect_term_vars(term: &Term, bound: &mut BTreeSet<String>, out: &mut BTreeMap<String, bool>) {
    match term {
        Term::Var { name, int } | Term::Opaque { name, int } => {
            if !bound.contains(name) {
                out.entry(name.clone()).or_insert(*int);
            }
        }
        Term::Base { name, .. } => {
            if !bound.contains(name) {
                out.entry(name.clone()).or_insert(true);
            }
        }
        Term::Relaxed { inner, .. } => collect_term_vars(inner, bound, out),
        Term::Ite {
            cond, then, els, ..
        } => {
            collect_term_vars(cond, bound, out);
            collect_term_vars(then, bound, out);
            collect_term_vars(els, bound, out);
        }
        Term::Call { args, .. } => {
            for arg in args {
                collect_term_vars(arg, bound, out);
            }
        }
        Term::Not(arg) | Term::Neg(arg) => collect_term_vars(arg, bound, out),
        Term::Bin { lhs, rhs, .. } => {
            collect_term_vars(lhs, bound, out);
            collect_term_vars(rhs, bound, out);
        }
        Term::Quant { bindings, body, .. } => {
            let mut added = Vec::new();
            for (name, _) in bindings {
                if bound.insert(name.clone()) {
                    added.push(name.clone());
                }
            }
            collect_term_vars(body, bound, out);
            for name in added {
                bound.remove(&name);
            }
        }
        Term::Int(_) | Term::Bool(_) => {}
    }
}

fn render_term(term: &Term, config: &ProjectionConfig) -> String {
    match term {
        Term::Int(value) => value.to_string(),
        Term::Bool(value) => value.to_string(),
        Term::Var { name, .. } | Term::Base { name, .. } | Term::Opaque { name, .. } => {
            name.clone()
        }
        Term::Relaxed { inner, .. } => render_term(inner, config),
        Term::Ite {
            cond, then, els, ..
        } => format!(
            "if {} {{ {} }} else {{ {} }}",
            render_term(cond, config),
            render_term(then, config),
            render_term(els, config)
        ),
        Term::Call { model, args, .. } => format!(
            "{}({})",
            config.models[*model].name,
            args.iter()
                .map(|arg| render_term(arg, config))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Term::Quant {
            forall,
            bindings,
            body,
        } => format!(
            "{}|{}| {}",
            if *forall { "forall" } else { "exists" },
            bindings
                .iter()
                .map(|(name, int)| format!("{name}: {}", if *int { "int" } else { "bool" }))
                .collect::<Vec<_>>()
                .join(", "),
            render_term(body, config)
        ),
        Term::Not(arg) => format!("!({})", render_term(arg, config)),
        Term::Neg(arg) => format!("-({})", render_term(arg, config)),
        Term::Bin { sym, lhs, rhs, .. } => format!(
            "({} {} {})",
            render_term(lhs, config),
            sym,
            render_term(rhs, config)
        ),
    }
}

fn scalar_expr_value(expression: &ScalarExpr, args: &BTreeMap<String, Value>) -> Option<Value> {
    match expression {
        ScalarExpr::Var(name) => args.get(name).cloned(),
        ScalarExpr::Int(value) => Some(json!({"op": "int", "value": value})),
        ScalarExpr::Bool(value) => Some(json!({"op": "bool", "value": value})),
        ScalarExpr::Not(arg) => Some(json!({
            "op": "not",
            "arg": scalar_expr_value(arg, args)?,
        })),
        ScalarExpr::Neg(arg) => Some(json!({
            "op": "neg",
            "arg": scalar_expr_value(arg, args)?,
        })),
        ScalarExpr::Bin(sym, lhs, rhs) => Some(json!({
            "op": "bin",
            "sym": sym,
            "lhs": scalar_expr_value(lhs, args)?,
            "rhs": scalar_expr_value(rhs, args)?,
        })),
    }
}

fn term_value(term: &Term, config: &ProjectionConfig) -> Option<Value> {
    match term {
        Term::Int(value) => Some(json!({"op": "int", "value": value})),
        Term::Bool(value) => Some(json!({"op": "bool", "value": value})),
        Term::Var { name, .. } | Term::Base { name, .. } | Term::Opaque { name, .. } => {
            Some(json!({"op": "var", "name": name}))
        }
        Term::Relaxed { inner, .. } => term_value(inner, config),
        Term::Ite {
            cond, then, els, ..
        } => match term_value(cond, config)?.as_bool()? {
            true => term_value(then, config),
            false => term_value(els, config),
        },
        Term::Call { model, args, .. } => {
            let body = config.models[*model].body.as_ref()?;
            let mut values = BTreeMap::new();
            for (param, arg) in config.models[*model].params.iter().zip(args) {
                values.insert(param.name.clone(), term_value(arg, config)?);
            }
            scalar_expr_value(body, &values)
        }
        Term::Quant { .. } => None,
        Term::Not(arg) => Some(json!({"op": "not", "arg": term_value(arg, config)?})),
        Term::Neg(arg) => Some(json!({"op": "neg", "arg": term_value(arg, config)?})),
        Term::Bin { sym, lhs, rhs, .. } => Some(json!({
            "op": "bin",
            "sym": sym,
            "lhs": term_value(lhs, config)?,
            "rhs": term_value(rhs, config)?,
        })),
    }
}

fn model_source(model: &ModelDef) -> String {
    let params = model
        .params
        .iter()
        .map(|param| format!("{}: {}", param.name, param.ty.as_verus()))
        .collect::<Vec<_>>()
        .join(", ");
    match &model.body {
        Some(body) => format!(
            "pub open spec fn {}({params}) -> {} {{\n    {}\n}}\n",
            model.name,
            model.returns.as_verus(),
            render_scalar_expr(body)
        ),
        None => format!(
            "pub uninterp spec fn {}({params}) -> {};\n",
            model.name,
            model.returns.as_verus()
        ),
    }
}

struct TranslateCtx<'a> {
    config: &'a ProjectionConfig,
    function: &'a str,
    params: &'a [ParamInfo],
    model: &'a BTreeMap<String, ParamModel>,
    const_names: &'a mut BTreeMap<String, String>,
    used_names: &'a mut BTreeSet<String>,
    scope: Vec<(String, Term)>,
    uses: &'a mut Vec<ProjectionUse>,
    tuple_fields: &'a mut BTreeSet<String>,
    identity_uses: BTreeMap<String, String>,
    fresh: usize,
    extension_allowed: bool,
}

impl TranslateCtx<'_> {
    fn fresh(&mut self, prefix: &str, int: bool) -> Term {
        let name = format!("_{prefix}{}", self.fresh);
        self.fresh += 1;
        Term::Opaque { name, int }
    }
}

fn apply_projection(
    rule_index: usize,
    args: Vec<Term>,
    ctx: &mut TranslateCtx,
) -> std::result::Result<Term, String> {
    let rule = &ctx.config.rules[rule_index];
    let model = &ctx.config.models[rule.model];
    if args.len() != model.params.len() {
        return Err(format!(
            "projection model {} expects {} arguments, got {}",
            model.name,
            model.params.len(),
            args.len()
        ));
    }
    for (index, (arg, param)) in args.iter().zip(&model.params).enumerate() {
        let ty = term_type(arg);
        if ty != param.ty {
            return Err(format!(
                "projection type conflict: model {} argument {index} expects {}, got {}",
                model.name,
                param.ty.as_verus(),
                ty.as_verus()
            ));
        }
    }
    let target = match &rule.kind {
        ProjectionKind::Method { callee } => callee.clone(),
        ProjectionKind::Field { datatype, field } => format!("{datatype}::{field}"),
        ProjectionKind::Variant { datatype, variant } => format!("{datatype}::{variant}"),
    };
    let identity = args.len() == 1
        && matches!(
            model.body.as_ref(),
            Some(ScalarExpr::Var(name)) if name == &model.params[0].name
        );
    for arg in &args {
        if let Term::Base { param, .. } = arg {
            if identity {
                if let Some(previous) = ctx.identity_uses.get(param)
                    && previous != &model.name
                {
                    return Err(format!(
                        "multiple identity projections of `{param}`: {previous} and {}",
                        model.name
                    ));
                }
                ctx.identity_uses.insert(param.clone(), model.name.clone());
            }
            ctx.uses.push(ProjectionUse {
                param: param.clone(),
                kind: match &rule.kind {
                    ProjectionKind::Method { .. } => "method",
                    ProjectionKind::Field { .. } => "field",
                    ProjectionKind::Variant { .. } => "variant",
                },
                target: target.clone(),
                model: model.name.clone(),
                relation: rule.relation,
                identity,
            });
        }
    }
    Ok(Term::Call {
        model: rule.model,
        args,
        int: model.returns.is_int(),
        relation: rule.relation,
    })
}

fn translate_expr(expression: &Value, ctx: &mut TranslateCtx) -> std::result::Result<Term, String> {
    let op = expression["op"]
        .as_str()
        .ok_or_else(|| "expression has no op".to_owned())?;
    match op {
        "int" => expression["value"]
            .as_i64()
            .map(Term::Int)
            .ok_or_else(|| "invalid integer literal".to_owned()),
        "bool" => expression["value"]
            .as_bool()
            .map(Term::Bool)
            .ok_or_else(|| "invalid boolean literal".to_owned()),
        "var" => {
            let name = expression["name"]
                .as_str()
                .ok_or_else(|| "variable expression has no name".to_owned())?;
            if let Some((_, term)) = ctx.scope.iter().rev().find(|(bound, _)| bound == name) {
                return Ok(term.clone());
            }
            let param = ctx
                .params
                .iter()
                .find(|param| param.name == name)
                .ok_or_else(|| format!("unknown variable: {name}"))?;
            let model = ctx.model.get(name);
            match param.kind.as_str() {
                "bool" => Ok(Term::Var {
                    name: model.map_or_else(|| sanitize_ident(name), |entry| entry.probe.clone()),
                    int: false,
                }),
                "int" => Ok(Term::Var {
                    name: model.map_or_else(|| sanitize_ident(name), |entry| entry.probe.clone()),
                    int: true,
                }),
                "reference" | "ghost" | "opaque" => {
                    let probe = model.map_or_else(
                        || format!("{}_id", sanitize_ident(name)),
                        |entry| entry.probe.clone(),
                    );
                    Ok(Term::Base {
                        param: name.to_owned(),
                        name: probe,
                    })
                }
                other => Err(format!("unsupported parameter kind: {other}")),
            }
        }
        "const" => {
            let path = expression["path"]
                .as_str()
                .ok_or_else(|| "constant expression has no path".to_owned())?;
            let name = lift_const(path, ctx.const_names, ctx.used_names);
            Ok(Term::Var { name, int: true })
        }
        "old" | "final" | "read" | "deref" => translate_expr(&expression["arg"], ctx),
        "cast" => {
            if !ctx.extension_allowed {
                return Err("unsupported expression node: cast".into());
            }
            let inner = translate_expr(&expression["arg"], ctx)?;
            if term_type(&inner) != ScalarType::Int {
                return Err("`as` cast expects an integer".into());
            }
            Ok(Term::Relaxed {
                inner: Box::new(inner),
                int: true,
            })
        }
        "not" => {
            let arg = translate_expr(&expression["arg"], ctx)?;
            if term_type(&arg) != ScalarType::Bool {
                return Err("`!` expects a bool".into());
            }
            Ok(Term::Not(Box::new(arg)))
        }
        "neg" => {
            let arg = translate_expr(&expression["arg"], ctx)?;
            if term_type(&arg) != ScalarType::Int {
                return Err("unary `-` expects an integer".into());
            }
            Ok(Term::Neg(Box::new(arg)))
        }
        "bin" => {
            let lhs = translate_expr(&expression["lhs"], ctx)?;
            let rhs = translate_expr(&expression["rhs"], ctx)?;
            let sym = expression["sym"].as_str().unwrap_or("&&");
            let (lhs_ty, rhs_ty) = (term_type(&lhs), term_type(&rhs));
            // Whole-value equality of a reference/ghost id with a scalar
            // projection is an over-approximation of equality of the
            // underlying values; ordering and arithmetic on raw ids remain
            // unmapped.
            if matches!(sym, "==" | "!=") {
                if lhs_ty != rhs_ty {
                    return Err(format!("`{sym}` expects operands of the same type"));
                }
                return Ok(Term::Bin {
                    sym: sym.to_owned(),
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                    int: false,
                });
            }
            if matches!(lhs, Term::Base { .. } | Term::Opaque { .. })
                || matches!(rhs, Term::Base { .. } | Term::Opaque { .. })
            {
                return Err(
                    "a raw reference/ghost id or constructor token may not be used in arithmetic or ordering"
                        .into(),
                );
            }
            if matches!(sym, "/" | "%") && !ctx.extension_allowed {
                return Err(format!("unsupported operator: {sym}"));
            }
            let int = match sym {
                "+" | "-" | "*" | "/" | "%" => {
                    if lhs_ty != ScalarType::Int || rhs_ty != ScalarType::Int {
                        return Err(format!("`{sym}` expects integers"));
                    }
                    true
                }
                "<" | "<=" | ">" | ">=" => {
                    if lhs_ty != ScalarType::Int || rhs_ty != ScalarType::Int {
                        return Err(format!("`{sym}` expects integers"));
                    }
                    false
                }
                "==" | "!=" => {
                    if lhs_ty != rhs_ty {
                        return Err(format!("`{sym}` expects operands of the same type"));
                    }
                    false
                }
                "&&" | "||" | "==>" => {
                    if lhs_ty != ScalarType::Bool || rhs_ty != ScalarType::Bool {
                        return Err(format!("`{sym}` expects booleans"));
                    }
                    false
                }
                other => return Err(format!("unsupported operator: {other}")),
            };
            let term = Term::Bin {
                sym: sym.to_owned(),
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                int,
            };
            if matches!(sym, "/" | "%") {
                Ok(Term::Relaxed {
                    inner: Box::new(term),
                    int,
                })
            } else {
                Ok(term)
            }
        }
        "field" => {
            let root = translate_expr(&expression["root"], ctx)?;
            let datatype = expression["datatype"].as_str();
            let field = expression["field"].as_str();
            let Some(datatype) = datatype else {
                let name = field.unwrap_or("?");
                if ctx.extension_allowed
                    && !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_digit())
                {
                    ctx.tuple_fields.insert(name.to_owned());
                    return Ok(ctx.fresh("t", true));
                }
                return Err(format!(
                    "field without datatype: .{name} (tuple/struct projection cannot be matched to a field rule)"
                ));
            };
            let rule = ctx
                .config
                .matching_rule(ctx.function, None, Some(datatype), field, None)
                .ok_or_else(|| {
                    format!(
                        "unmodeled field: {datatype}::{}",
                        field.unwrap_or("<unknown>")
                    )
                })?;
            apply_projection(rule, vec![root], ctx)
        }
        "call" => {
            let callee = expression["callee"]
                .as_str()
                .ok_or_else(|| "call expression has no callee".to_owned())?;
            let args = expression["args"]
                .as_array()
                .ok_or_else(|| "call expression has no args".to_owned())?
                .iter()
                .map(|arg| translate_expr(arg, ctx))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let rule = ctx
                .config
                .matching_rule(ctx.function, Some(callee), None, None, None)
                .ok_or_else(|| format!("unmodeled call: {callee}"))?;
            apply_projection(rule, args, ctx)
        }
        "quant" => {
            let forall = expression["forall"].as_bool().unwrap_or(true);
            let mut bindings = Vec::new();
            let mut pushed = Vec::new();
            for binding in expression["bindings"].as_array().into_iter().flatten() {
                let name = binding["name"]
                    .as_str()
                    .ok_or_else(|| "quantifier binding has no name".to_owned())?;
                let ty = binding["type"].as_str().unwrap_or("usize");
                let int = !ty.contains("Bool");
                let local = format!("q{}_{}", bindings.len(), sanitize_ident(name));
                ctx.scope.push((
                    name.to_owned(),
                    Term::Var {
                        name: local.clone(),
                        int,
                    },
                ));
                pushed.push(local.clone());
                bindings.push((local, int));
            }
            let body = translate_expr(&expression["body"], ctx)?;
            for _ in &pushed {
                ctx.scope.pop();
            }
            if term_type(&body) != ScalarType::Bool {
                return Err("quantifier body must be a bool".into());
            }
            Ok(Term::Quant {
                forall,
                bindings,
                body: Box::new(body),
            })
        }
        "ctor" => {
            if !ctx.extension_allowed {
                return Err("unmodeled constructor".into());
            }
            Ok(ctx.fresh("c", true))
        }
        "block" => {
            if !ctx.extension_allowed {
                return Err("unsupported expression node: block".into());
            }
            let mut pushed = 0usize;
            for binding in expression["bindings"].as_array().into_iter().flatten() {
                let name = binding["name"]
                    .as_str()
                    .ok_or_else(|| "block binding has no name".to_owned())?;
                let init = translate_expr(&binding["init"], ctx)?;
                ctx.scope.push((name.to_owned(), init));
                pushed += 1;
            }
            let body = translate_expr(&expression["body"], ctx);
            for _ in 0..pushed {
                ctx.scope.pop();
            }
            body
        }
        "match" => {
            if !ctx.extension_allowed {
                return Err("unsupported expression node: match".into());
            }
            translate_match(expression, ctx)
        }
        "is_variant" => {
            let datatype = expression["datatype"].as_str();
            let variant = expression["variant"].as_str();
            let arg = expression
                .get("arg")
                .filter(|value| !value.is_null())
                .ok_or_else(|| {
                    format!(
                        "unmodeled `is` discriminant: {}::{} (no subject)",
                        datatype.unwrap_or("<unknown>"),
                        variant.unwrap_or("<unknown>")
                    )
                })?;
            let arg = translate_expr(arg, ctx)?;
            let rule = ctx
                .config
                .matching_rule(ctx.function, None, datatype, None, variant)
                .ok_or_else(|| {
                    format!(
                        "unmodeled `is` discriminant: {}::{}",
                        datatype.unwrap_or("<unknown>"),
                        variant.unwrap_or("<unknown>")
                    )
                })?;
            apply_projection(rule, vec![arg], ctx)
        }
        "opaque" => Err(format!(
            "opaque VIR node: {}",
            expression["tag"].as_str().unwrap_or("unknown")
        )),
        other => Err(format!("unsupported expression node: {other}")),
    }
}

fn and_terms(lhs: Term, rhs: Term) -> Term {
    match (lhs, rhs) {
        (Term::Bool(true), rhs) => rhs,
        (lhs, Term::Bool(true)) => lhs,
        (lhs, rhs) => Term::Bin {
            sym: "&&".to_owned(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            int: false,
        },
    }
}

fn or_terms(lhs: Term, rhs: Term) -> Term {
    match (lhs, rhs) {
        (Term::Bool(false), rhs) => rhs,
        (lhs, Term::Bool(false)) => lhs,
        (lhs, rhs) => Term::Bin {
            sym: "||".to_owned(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            int: false,
        },
    }
}

fn pattern_condition(
    pattern: &Value,
    subject: &Term,
    ctx: &mut TranslateCtx,
    bindings: &mut Vec<(String, Term)>,
) -> std::result::Result<Term, String> {
    match pattern["kind"].as_str().unwrap_or("unsupported") {
        "wildcard" => Ok(Term::Bool(true)),
        "var" => {
            let name = pattern["name"]
                .as_str()
                .ok_or_else(|| "match variable pattern has no name".to_owned())?;
            bindings.push((name.to_owned(), subject.clone()));
            Ok(Term::Bool(true))
        }
        "ctor" => {
            let datatype = pattern["datatype"].as_str();
            let variant = pattern["variant"].as_str();
            let rule = ctx
                .config
                .matching_rule(ctx.function, None, datatype, None, variant)
                .ok_or_else(|| {
                    format!(
                        "unmodeled match variant: {}::{}",
                        datatype.unwrap_or("<unknown>"),
                        variant.unwrap_or("<unknown>")
                    )
                })?;
            let mut condition = apply_projection(rule, vec![subject.clone()], ctx)?;
            for arg in pattern["args"].as_array().into_iter().flatten() {
                if arg["kind"].as_str() == Some("wildcard") {
                    continue;
                }
                let inner = ctx.fresh("p", true);
                let sub = pattern_condition(arg, &inner, ctx, bindings)?;
                condition = and_terms(condition, sub);
            }
            Ok(condition)
        }
        "or" => {
            let mut condition = Term::Bool(false);
            for sub in pattern["patterns"].as_array().into_iter().flatten() {
                let before = bindings.len();
                let term = pattern_condition(sub, subject, ctx, bindings)?;
                if bindings.len() != before {
                    return Err("match `or` patterns may not bind variables".into());
                }
                condition = or_terms(condition, term);
            }
            Ok(condition)
        }
        other => Err(format!("unsupported match pattern: {other}")),
    }
}

fn translate_match(
    expression: &Value,
    ctx: &mut TranslateCtx,
) -> std::result::Result<Term, String> {
    let scrutinee = translate_expr(&expression["scrutinee"], ctx)?;
    let arms = expression["arms"]
        .as_array()
        .ok_or_else(|| "match has no arms".to_owned())?;
    if arms.is_empty() {
        return Err("match has no arms".into());
    }
    let mut translated = Vec::new();
    for arm in arms {
        let mut bindings = Vec::new();
        let mut condition = pattern_condition(&arm["pattern"], &scrutinee, ctx, &mut bindings)?;
        if let Some(guard) = arm.get("guard").filter(|value| !value.is_null()) {
            let guard = translate_expr(guard, ctx)?;
            if term_type(&guard) != ScalarType::Bool {
                return Err("match guard must be a bool".into());
            }
            condition = and_terms(condition, guard);
        }
        let pushed = bindings.len();
        for (name, term) in bindings {
            ctx.scope.push((name, term));
        }
        let body = translate_expr(&arm["body"], ctx)?;
        for _ in 0..pushed {
            ctx.scope.pop();
        }
        translated.push((condition, body));
    }
    let ty = term_type(&translated[0].1);
    for (_, body) in &translated {
        if term_type(body) != ty {
            return Err("match arms have different types".into());
        }
    }
    let mut result = if matches!(
        translated.last().map(|(condition, _)| condition),
        Some(Term::Bool(true))
    ) {
        translated.pop().expect("non-empty").1
    } else {
        ctx.fresh("m", ty.is_int())
    };
    while let Some((condition, body)) = translated.pop() {
        result = Term::Ite {
            cond: Box::new(condition),
            then: Box::new(body),
            els: Box::new(result),
            int: ty.is_int(),
        };
    }
    Ok(result)
}

/// Selected tiers and deterministic batching options.
#[derive(Debug, Clone)]
pub struct Options {
    pub tiers: BTreeSet<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
    pub batch: Option<usize>,
    pub batch_size: usize,
    pub config: Config,
    /// Optional project-side pre-registration for E6 signals.
    pub expectations: Option<crate::signals::Expectations>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tiers: ["A", "B"].into_iter().map(str::to_owned).collect(),
            filter: None,
            limit: None,
            batch: None,
            batch_size: 50,
            config: Config::default(),
            expectations: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    A,
    B,
    C,
    None,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::None => "none",
        }
    }
}

struct ParamInfo {
    name: String,
    ty: String,
    kind: String,
}

struct ParamModel {
    probe: String,
    int: bool,
}

struct Decision {
    path: String,
    tier: Tier,
    reasons: Vec<String>,
    entry: Option<Value>,
    expected: Option<&'static str>,
    expected_note: Option<String>,
    clauses: usize,
    mechanical: usize,
    projected: usize,
    unsupported: usize,
    modeling: Value,
}

/// Build the manifest and screening report from a census JSON document.
pub fn build(census: &Value, options: &Options) -> Result<Value> {
    let functions = census["functions"]
        .as_array()
        .ok_or("census has no functions array")?;
    let mut decisions = Vec::with_capacity(functions.len());
    let mut baselines = Vec::with_capacity(functions.len());
    let mut name_used = BTreeSet::new();
    let mut baseline_name_used = BTreeSet::new();

    for (index, function) in functions.iter().enumerate() {
        let baseline = classify(
            function,
            index,
            &mut baseline_name_used,
            &Config::default(),
            false,
        );
        baselines.push((baseline.tier, baseline.reasons.clone()));
        let extension_allowed = baseline.tier == Tier::C;
        let decision = classify(
            function,
            index,
            &mut name_used,
            &options.config,
            extension_allowed,
        );
        decisions.push(decision);
    }

    // Stable global entry names are assigned in census order; batching and
    // filtering only select among them.
    let mut candidates = Vec::new();
    for decision in &decisions {
        if decision.entry.is_none() {
            continue;
        }
        if !options.tiers.contains(decision.tier.as_str()) {
            continue;
        }
        if let Some(filter) = &options.filter {
            if !decision.path.contains(filter) {
                continue;
            }
        }
        candidates.push(decision.path.clone());
    }

    let total_candidates = candidates.len();
    let selected = if let Some(batch) = options.batch {
        let start = batch.saturating_mul(options.batch_size);
        candidates
            .iter()
            .skip(start)
            .take(options.batch_size)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        candidates.clone()
    };
    let selected = if let Some(limit) = options.limit {
        selected.into_iter().take(limit).collect::<Vec<_>>()
    } else {
        selected
    };
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();

    let mut contracts = Vec::new();
    let mut preamble = String::from(
        "// Generated by spec-check screen-requires (M7e).\n\
         // Requires clauses are wrapped as named spec fns (quantifier-trigger\n\
         // discipline). Mechanical clauses are open spec bodies; clauses over\n\
         // ghost structure are uninterp spec fns (constraint structure kept by\n\
         // name, body abstracted). Constants referenced by requires are lifted\n\
         // to probe parameters (documented abstraction).\n\n",
    );
    let mut entries = Vec::new();
    let mut emitted_models = BTreeSet::new();
    for decision in &decisions {
        if !selected_set.contains(&decision.path) {
            continue;
        }
        let entry = decision
            .entry
            .as_ref()
            .expect("selected decision has entry");
        let name = entry["name"].as_str().unwrap_or_default();
        if let Some(models) = entry["models"].as_array() {
            for model in models {
                let Some(model_name) = model["name"].as_str() else {
                    continue;
                };
                if !emitted_models.insert(model_name.to_owned()) {
                    continue;
                }
                if let Some(source) = model["source"].as_str() {
                    preamble.push_str(source);
                    preamble.push('\n');
                }
            }
        }
        if let Some(fns) = entry["preamble"].as_str() {
            preamble.push_str(fns);
            preamble.push('\n');
        }
        let mut contract = json!({
            "name": name,
            "probe": "quantifier",
            "params": entry["params"],
            "requires": entry["requires"],
            "origin": entry["origin"],
        });
        if let Some(witness) = entry.get("witness").filter(|value| !value.is_null()) {
            contract["witness"] = witness.clone();
        }
        contracts.push(contract);
        let pre = options
            .expectations
            .as_ref()
            .and_then(|expectations| expectations.entries.get(name));
        entries.push(json!({
            "name": name,
            "path": decision.path,
            "tier": decision.tier.as_str(),
            "expected": decision.expected,
            "expected_note": decision.expected_note,
            "preregistered_expected": pre.map(|entry| entry.expected.clone()),
            "preregistered_basis": pre.map(|entry| entry.basis.clone()),
            "preregistered_scope": pre.map(|entry| entry.scope.clone()),
            "preregistered_registry": pre.and_then(|entry| entry.registry.clone()),
            "params": entry["params"],
            "requires": entry["requires"],
            "witness": entry.get("witness"),
            "clauses": decision.clauses,
            "mechanical_clauses": decision.mechanical,
            "projected_clauses": decision.projected,
            "unsupported_clauses": decision.unsupported,
            "origin": entry["origin"],
            "modeling": decision.modeling,
        }));
    }

    let manifest = json!({
        "preamble": preamble,
        "contracts": contracts,
    });

    let mut tier_counts = BTreeMap::new();
    let mut baseline_counts = BTreeMap::new();
    let mut upgraded = [0usize; 3];
    let mut reason_counts = BTreeMap::new();
    let mut clause_counts = [0usize; 3];
    let mut function_rows = Vec::with_capacity(decisions.len());
    for (decision, (baseline_tier, baseline_reasons)) in decisions.iter().zip(&baselines) {
        *tier_counts.entry(decision.tier.as_str()).or_insert(0usize) += 1;
        *baseline_counts
            .entry(baseline_tier.as_str())
            .or_insert(0usize) += 1;
        if *baseline_tier == Tier::C && decision.tier != Tier::None {
            match decision.tier {
                Tier::A => upgraded[0] += 1,
                Tier::B => upgraded[1] += 1,
                Tier::C => upgraded[2] += 1,
                Tier::None => {}
            }
        }
        let kinds = decision
            .reasons
            .iter()
            .map(|reason| reason.split(':').next().unwrap_or(reason).to_owned())
            .collect::<BTreeSet<_>>();
        for kind in kinds {
            *reason_counts.entry(kind).or_insert(0usize) += 1;
        }
        match decision.tier {
            Tier::A => clause_counts[0] += decision.clauses,
            Tier::B => clause_counts[1] += decision.clauses,
            Tier::C => clause_counts[2] += decision.clauses,
            Tier::None => {}
        }
        function_rows.push(json!({
            "path": decision.path,
            "tier": decision.tier.as_str(),
            "reasons": decision.reasons,
            "baseline_tier": baseline_tier.as_str(),
            "baseline_reasons": baseline_reasons,
            "clauses": decision.clauses,
            "mechanical_clauses": decision.mechanical,
            "projected_clauses": decision.projected,
            "unsupported_clauses": decision.unsupported,
            "entry": decision.entry.as_ref().map(|entry| entry["name"].clone()),
            "expected": decision.expected,
            "preregistered_expected": decision.entry.as_ref().and_then(|entry| {
                options.expectations.as_ref().and_then(|expectations| {
                    expectations.entries.get(entry["name"].as_str().unwrap_or_default())
                        .map(|pre| pre.expected.clone())
                })
            }),
            "selected": selected_set.contains(&decision.path),
            "modeling": decision.modeling,
        }));
    }

    let entry_names = decisions
        .iter()
        .filter_map(|decision| Some(decision.entry.as_ref()?.get("name")?.as_str()?.to_owned()))
        .collect::<BTreeSet<_>>();
    let expectations = options.expectations.as_ref();
    let matched_expectations = expectations
        .map(|expectations| {
            expectations
                .entries
                .keys()
                .filter(|name| entry_names.contains(name.as_str()))
                .count()
        })
        .unwrap_or(0);
    let provided_expectations = expectations
        .map(|expectations| expectations.entries.len())
        .unwrap_or(0);
    let expectations_report = json!({
        "provided": expectations.is_some(),
        "entries": provided_expectations,
        "matched": matched_expectations,
        "missing": entry_names.len().saturating_sub(matched_expectations),
        "unmatched": provided_expectations.saturating_sub(matched_expectations),
    });

    let report = json!({
        "schema": SCHEMA,
        "census": {
            "schema": census["schema"],
            "crate": census["crate"],
            "functions": functions.len(),
        },
        "options": {
            "tiers": options.tiers,
            "filter": options.filter,
            "limit": options.limit,
            "batch": options.batch,
            "batch_size": options.batch_size,
            "config": {
                "is_default": options.config.is_default(),
                "generic_instances": options.config.generic_instances.len(),
                "reference": if options.config.reference_opaque { "opaque" } else { "off" },
                "ghost": if options.config.ghost_opaque { "opaque" } else { "off" },
                "projection": {
                    "models": options.config.projection.models.len(),
                    "rules": options.config.projection.rules.len(),
                    "is_empty": options.config.projection.is_empty(),
                },
            },
        },
        "counts": {
            "functions": functions.len(),
            "with_requires": decisions.iter().filter(|d| d.tier != Tier::None).count(),
            "no_requires": decisions.iter().filter(|d| d.tier == Tier::None).count(),
            "tier_a": tier_counts.get("A").copied().unwrap_or(0),
            "tier_b": tier_counts.get("B").copied().unwrap_or(0),
            "tier_c": tier_counts.get("C").copied().unwrap_or(0),
            "tier_c_reasons": reason_counts,
            "clauses": {
                "tier_a": clause_counts[0],
                "tier_b": clause_counts[1],
                "tier_c": clause_counts[2],
            },
            "baseline": {
                "tier_a": baseline_counts.get("A").copied().unwrap_or(0),
                "tier_b": baseline_counts.get("B").copied().unwrap_or(0),
                "tier_c": baseline_counts.get("C").copied().unwrap_or(0),
            },
            "upgraded": {
                "to_a": upgraded[0],
                "to_b": upgraded[1],
                "still_c": upgraded[2],
            },
            "candidates": total_candidates,
            "selected": entries.len(),
        },
        "entries": entries,
        "expectations": expectations_report,
        "functions": function_rows,
    });

    Ok(json!({"manifest": manifest, "report": report}))
}

/// Write `manifest.json` and `screening.json` under `out`.
pub fn run(census_path: &Path, out: &Path, options: &Options) -> Result<Value> {
    let text = std::fs::read_to_string(census_path)?;
    let census: Value = serde_json::from_str(&text)?;
    let built = build(&census, options)?;
    std::fs::create_dir_all(out)?;
    write_json(&out.join("manifest.json"), &built["manifest"])?;
    write_json(&out.join("screening.json"), &built["report"])?;
    Ok(built)
}

fn classify(
    function: &Value,
    index: usize,
    name_used: &mut BTreeSet<String>,
    config: &Config,
    extension_allowed: bool,
) -> Decision {
    let path = function["path"].as_str().unwrap_or("<unknown>").to_owned();
    let clauses = function["requires"].as_array().cloned().unwrap_or_default();
    let raw_params = function["params"].as_array().cloned().unwrap_or_default();
    let generic_names = function["generics"]["names"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let generic_count = function["generics"]["count"].as_u64().unwrap_or(0);
    let mechanical = clauses
        .iter()
        .filter(|clause| clause["class"].as_str() == Some("mechanical"))
        .count();
    let origin = json!({
        "external_body": function["external_body"],
        "kind": function["kind"],
        "visibility": function["visibility"],
        "trait": function["trait"],
        "source": function["source"],
        "mode": function["mode"],
        "generics": function["generics"],
    });
    let mut modeling = json!({
        "generic_instantiation": Value::Null,
        "reference_mapping": [],
        "ghost_mapping": [],
    });

    if clauses.is_empty() {
        return Decision {
            path,
            tier: Tier::None,
            reasons: Vec::new(),
            entry: None,
            expected: None,
            expected_note: None,
            clauses: 0,
            mechanical: 0,
            projected: 0,
            unsupported: 0,
            modeling,
        };
    }

    let mut reasons = Vec::new();
    let mut params = raw_params
        .iter()
        .filter_map(|param| {
            let name = param["name"].as_str()?.to_owned();
            Some(ParamInfo {
                name,
                ty: param["type"].as_str().unwrap_or("").to_owned(),
                kind: param["kind"].as_str().unwrap_or("opaque").to_owned(),
            })
        })
        .collect::<Vec<_>>();

    // G rules: project-side instantiation, then re-derive parameter kinds.
    if generic_count > 0 && generic_names.is_empty() {
        // Legacy/foreign census without names: keep the M7e blocker.
        reasons.push(format!("generic:{}", function["generics"]["names"]));
    } else if !generic_names.is_empty() {
        match config.generic_rule(&path) {
            Some(rule) => {
                let mut types = BTreeMap::new();
                let mut missing = Vec::new();
                for name in &generic_names {
                    if let Some(ty) = rule.types.get(name).or_else(|| rule.types.get("*")) {
                        types.insert(name.clone(), ty.clone());
                    } else {
                        missing.push(name.clone());
                    }
                }
                if missing.is_empty() {
                    for param in &mut params {
                        param.ty = substitute_type(&param.ty, &types);
                        param.kind = type_kind_of(&param.ty);
                    }
                    modeling["generic_instantiation"] = json!({
                        "matched": rule.pattern,
                        "types": types,
                    });
                } else {
                    reasons.push(format!("generic_uninstantiated:{}", json!(missing)));
                }
            }
            None => reasons.push(format!("generic:{}", function["generics"]["names"])),
        }
    }

    // R/H rules. A referenced reference/ghost parameter is mappable if the
    // old safe-use rule holds, or if every clause mentioning it has an
    // explicit project-side projection translation.
    let referenced = referenced_names(&clauses, &raw_params);
    let model = param_models(&params, &referenced);
    let mut used_names = model
        .values()
        .map(|p| p.probe.clone())
        .collect::<BTreeSet<_>>();
    let mut const_names: BTreeMap<String, String> = BTreeMap::new();
    let mut projection_uses: Vec<ProjectionUse> = Vec::new();
    let mut tuple_fields: BTreeSet<String> = BTreeSet::new();
    let projection_enabled = !config.projection.is_empty();
    let mut plans = Vec::with_capacity(clauses.len());
    for clause in &clauses {
        if clause["class"].as_str() == Some("mechanical") {
            plans.push(ClausePlan::Mechanical);
        } else if projection_enabled && clause.get("expr").is_some_and(|value| !value.is_null()) {
            let mut ctx = TranslateCtx {
                config: &config.projection,
                function: &path,
                params: &params,
                model: &model,
                const_names: &mut const_names,
                used_names: &mut used_names,
                scope: Vec::new(),
                uses: &mut projection_uses,
                tuple_fields: &mut tuple_fields,
                identity_uses: BTreeMap::new(),
                fresh: 0,
                extension_allowed,
            };
            match translate_expr(&clause["expr"], &mut ctx) {
                Ok(term) => plans.push(ClausePlan::Projected(term)),
                Err(detail) => plans.push(ClausePlan::Unsupported(detail)),
            }
        } else if projection_enabled {
            plans.push(ClausePlan::Unsupported(
                "no expression tree (old census); projection not attempted".into(),
            ));
        } else {
            plans.push(ClausePlan::Unsupported(
                "projection models are not configured; M7e behavior retained".into(),
            ));
        }
    }
    projection_uses.sort();
    projection_uses.dedup();
    let projected = plans
        .iter()
        .filter(|plan| matches!(plan, ClausePlan::Projected(_)))
        .count();
    let unsupported = plans
        .iter()
        .filter(|plan| matches!(plan, ClausePlan::Unsupported(_)))
        .count();

    let mut reference_mapping = Vec::new();
    let mut ghost_mapping = Vec::new();
    for param in &params {
        if !referenced.contains(&param.name) {
            continue;
        }
        match param.kind.as_str() {
            "ghost" => {
                if config.ghost_opaque
                    && (param_is_mappable(&param.name, &clauses)
                        || param_projection_covered(&param.name, &clauses, &plans))
                {
                    ghost_mapping.push(mapping_json(param, &clauses, &projection_uses));
                } else {
                    let kind = if config.ghost_opaque {
                        "ghost_param_unmapped"
                    } else {
                        "ghost_param"
                    };
                    reasons.push(format!("{kind}:{}", param.name));
                }
            }
            "reference" => {
                if config.reference_opaque
                    && (param_is_mappable(&param.name, &clauses)
                        || param_projection_covered(&param.name, &clauses, &plans))
                {
                    reference_mapping.push(mapping_json(param, &clauses, &projection_uses));
                } else {
                    let kind = if config.reference_opaque {
                        "reference_param_unmapped"
                    } else {
                        "reference_param"
                    };
                    reasons.push(format!("{kind}:{}", param.name));
                }
            }
            _ => {}
        }
    }

    let blocker_names = params
        .iter()
        .filter(|param| matches!(param.kind.as_str(), "reference" | "ghost"))
        .map(|param| param.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut projection_unmapped = Vec::new();
    for (clause_index, (clause, plan)) in clauses.iter().zip(&plans).enumerate() {
        let ClausePlan::Unsupported(detail) = plan else {
            continue;
        };
        let mentioned = clause["vars"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|name| blocker_names.contains(name))
            .collect::<Vec<_>>();
        if !mentioned.is_empty() {
            projection_unmapped.push(json!({
                "clause": clause_index,
                "detail": detail,
                "params": mentioned,
                "text": clause["text"],
            }));
        }
    }

    let mut clause_modes = Vec::with_capacity(plans.len());
    let mut relations = Vec::new();
    for plan in &plans {
        match plan {
            ClausePlan::Mechanical => clause_modes.push("mechanical"),
            ClausePlan::Projected(term) => {
                term_relations(term, &mut relations);
                if term_is_evaluable(term, &config.projection) {
                    clause_modes.push("projected-evaluable");
                } else {
                    clause_modes.push("projected-opaque");
                }
            }
            ClausePlan::Unsupported(_) => clause_modes.push("unsupported"),
        }
    }
    let vacuum_evidence = relations.iter().all(|relation| relation.vacuum_evidence());
    let has_projection = projected > 0;

    let mut used_models = BTreeSet::new();
    for plan in &plans {
        if let ClausePlan::Projected(term) = plan {
            term_models(term, &mut used_models);
        }
    }
    let projection_models = used_models
        .iter()
        .map(|model_index| {
            let model = &config.projection.models[*model_index];
            json!({
                "name": model.name,
                "params": model.params.iter().map(|param| {
                    json!({"name": param.name, "type": param.ty.as_verus()})
                }).collect::<Vec<_>>(),
                "returns": model.returns.as_verus(),
                "body": model.body.as_ref().map(render_scalar_expr),
                "opaque": model.body.is_none(),
                "source": model_source(model),
            })
        })
        .collect::<Vec<_>>();
    let projection_model_sources = projection_models
        .iter()
        .map(|model| {
            json!({
                "name": model["name"],
                "source": model["source"],
            })
        })
        .collect::<Vec<_>>();
    let projection_mapping = params
        .iter()
        .filter_map(|param| {
            let uses = projection_uses
                .iter()
                .filter(|entry| entry.param == param.name)
                .map(ProjectionUse::value)
                .collect::<Vec<_>>();
            if uses.is_empty() {
                return None;
            }
            Some(json!({
                "param": param.name,
                "base_id": model.get(&param.name).map_or_else(
                    || format!("{}_id", sanitize_ident(&param.name)),
                    |entry| entry.probe.clone(),
                ),
                "uses": uses,
            }))
        })
        .collect::<Vec<_>>();

    modeling["reference_mapping"] = json!(reference_mapping);
    modeling["ghost_mapping"] = json!(ghost_mapping);
    modeling["projection_mapping"] = json!(projection_mapping);
    modeling["projection_models"] = json!(projection_models);
    modeling["projection_unmapped"] = json!(projection_unmapped);
    modeling["tuple_field_projection"] = json!(
        tuple_fields
            .iter()
            .map(|field| json!({
                "field": field,
                "relation": "over-approx",
                "note": "tuple field access without a VIR datatype; fresh scalar token",
            }))
            .collect::<Vec<_>>()
    );
    modeling["clause_modes"] = json!(clause_modes);

    let has_opaque_projection = plans.iter().any(|plan| {
        matches!(plan, ClausePlan::Projected(term) if !term_is_evaluable(term, &config.projection))
    });
    let tier = if !reasons.is_empty() {
        Tier::C
    } else if unsupported > 0 || has_opaque_projection {
        Tier::B
    } else {
        Tier::A
    };

    let structural_relaxed_clauses = plans
        .iter()
        .filter(
            |plan| matches!(plan, ClausePlan::Projected(term) if term_structural_relaxation(term)),
        )
        .count();
    let mut relation_counts = BTreeMap::new();
    for relation in &relations {
        *relation_counts
            .entry(relation.as_str().to_owned())
            .or_insert(0usize) += 1;
    }
    let under_approx = relation_counts.get("under-approx").copied().unwrap_or(0);
    let faithful = relation_counts.get("faithful").copied().unwrap_or(0);
    let over_approx = relation_counts.get("over-approx").copied().unwrap_or(0);
    let vacuum_relation = if projected > 0 {
        if under_approx > 0 {
            "under-approx"
        } else if structural_relaxed_clauses > 0 {
            "over-approx"
        } else if faithful > 0 && over_approx == 0 {
            "faithful"
        } else if faithful + over_approx > 0 {
            "over-approx"
        } else {
            "undeclared"
        }
    } else if tier == Tier::A && unsupported == 0 {
        "exact"
    } else if tier == Tier::B {
        "over-approx"
    } else {
        "undeclared"
    };
    modeling["projection_relations"] = json!({
        "faithful": faithful,
        "over-approx": over_approx,
        "under-approx": under_approx,
        "structural_relaxed_clauses": structural_relaxed_clauses,
    });
    modeling["vacuum_relation"] = json!(vacuum_relation);

    if tier == Tier::C {
        return Decision {
            path,
            tier,
            reasons,
            entry: None,
            expected: None,
            expected_note: None,
            clauses: clauses.len(),
            mechanical,
            projected,
            unsupported,
            modeling,
        };
    }

    let use_int = projected > 0;
    let mut spec_fns = String::new();
    let mut terms = Vec::new();
    let mut constraints = Vec::new();
    let mut param_order: Vec<String> = Vec::new();
    let mut param_types: BTreeMap<String, bool> = model
        .values()
        .map(|entry| (entry.probe.clone(), entry.int))
        .collect();

    for (clause_index, (clause, plan)) in clauses.iter().zip(&plans).enumerate() {
        match plan {
            ClausePlan::Mechanical => {
                let ast = &clause["ast"];
                let rewritten = rewrite_consts(ast, &mut const_names, &mut used_names);
                let mut vars = BTreeSet::new();
                collect_ast_vars(&rewritten, &mut vars);
                let args = ordered_args(&vars, &params, &model, &const_names, &mut param_order);
                render_mech_fn(
                    &mut spec_fns,
                    index,
                    clause_index,
                    &args,
                    &rewritten,
                    &model,
                    use_int,
                );
                terms.push(format!(
                    "p{index}_k{clause_index}({})",
                    args.iter()
                        .map(|a| a.1.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                constraints.push(rewritten);
            }
            ClausePlan::Projected(term) => {
                let mut free = BTreeMap::new();
                let mut bound = BTreeSet::new();
                collect_term_vars(term, &mut bound, &mut free);
                for (name, int) in &free {
                    if !param_order.contains(name) {
                        param_order.push(name.clone());
                    }
                    param_types.insert(name.clone(), *int);
                }
                let arglist = free
                    .iter()
                    .map(|(name, int)| format!("{name}: {}", if *int { "int" } else { "bool" }))
                    .collect::<Vec<_>>()
                    .join(", ");
                let call_args = free.keys().cloned().collect::<Vec<_>>().join(", ");
                if term_has_quantifier(term) {
                    // The bounded witness search cannot decide quantifiers, and
                    // Verus cannot always infer triggers for an arbitrary
                    // projected quantifier. Keep the named constraint but
                    // abstract its body: Tier B / CandidateInconclusive, as for
                    // an opaque projection.
                    spec_fns.push_str(&format!(
                        "pub uninterp spec fn p{index}_k{clause_index}({arglist}) -> bool;\n\n"
                    ));
                } else {
                    spec_fns.push_str(&format!(
                        "pub open spec fn p{index}_k{clause_index}({arglist}) -> bool {{\n    {}\n}}\n\n",
                        render_term(term, &config.projection)
                    ));
                }
                terms.push(format!("p{index}_k{clause_index}({call_args})"));
                if term_is_evaluable(term, &config.projection)
                    && let Some(value) = term_value(term, &config.projection)
                {
                    constraints.push(value);
                }
            }
            ClausePlan::Unsupported(_) => {
                let args = unsupported_args(
                    clause,
                    &params,
                    &model,
                    &mut used_names,
                    &mut const_names,
                    &mut param_order,
                );
                let call_args = args
                    .iter()
                    .map(|a| a.1.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                spec_fns.push_str(&format!(
                    "pub uninterp spec fn p{index}_k{clause_index}({}) -> bool;\n\n",
                    args.iter()
                        .map(|(name, _)| {
                            let int = model.get(name).map_or(true, |entry| entry.int);
                            format!(
                                "{name}: {}",
                                if int {
                                    if use_int { "int" } else { "usize" }
                                } else {
                                    "bool"
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                terms.push(format!("p{index}_k{clause_index}({call_args})"));
            }
        }
    }

    if param_order.is_empty() {
        param_order.push("_unused".into());
        used_names.insert("_unused".into());
    }
    for name in &param_order {
        param_types.entry(name.clone()).or_insert(true);
    }

    // A projected clause with an opaque model or quantifier may not have a
    // witness. Tier A means every clause is evaluable, so the search is
    // complete for the abstraction's scalar core.
    let witness = if tier == Tier::A {
        search_witness(&constraints, &param_order, &param_types)
    } else {
        None
    };
    let witness_source = witness
        .as_ref()
        .map(|values| render_witness(values, &param_order, &param_types, use_int));
    let expected = if tier == Tier::A && witness_source.is_some() {
        "ConfirmedLive"
    } else {
        "CandidateInconclusive"
    };
    let expected_note = if tier == Tier::A {
        if witness_source.is_some() {
            has_projection.then(|| {
                "projection models are model-relative: ConfirmedLive is not real liveness"
                    .to_owned()
            })
        } else if vacuum_evidence {
            Some(
                "no bounded witness over projection models; a ConfirmedVacuous verdict is model-relative candidate evidence and must be replicated (E6)"
                    .to_owned(),
            )
        } else {
            Some("under-approx projection present: ConfirmedVacuous is not evidence".to_owned())
        }
    } else if tier == Tier::B {
        if has_projection {
            Some(if vacuum_evidence {
                "projected clause includes opaque/quantified models: verifier incompleteness expected; ConfirmedLive is model-relative and ConfirmedVacuous is model-relative candidate evidence"
                    .to_owned()
            } else {
                "under-approx projection present: ConfirmedVacuous is not evidence".to_owned()
            })
        } else {
            Some("uninterp clauses: verifier incompleteness expected; not a defect".to_owned())
        }
    } else {
        None
    };

    let params_source = param_order
        .iter()
        .map(|name| {
            let typ = if param_types.get(name).copied().unwrap_or(true) {
                if use_int { "int" } else { "usize" }
            } else {
                "bool"
            };
            format!("{name}: {typ}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let short = path
        .rsplit("::")
        .next()
        .unwrap_or("entry")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let short = short.trim_matches('_');
    let mut name = format!("r{index:04}_{short}");
    while name_used.contains(&name) {
        name.push('_');
    }
    name_used.insert(name.clone());

    let entry = json!({
        "name": name,
        "params": params_source,
        "requires": if terms.is_empty() { "true".to_owned() } else { terms.join(" && ") },
        "witness": witness_source,
        "preamble": spec_fns,
        "models": projection_model_sources,
        "origin": origin,
    });

    Decision {
        path,
        tier,
        reasons,
        entry: Some(entry),
        expected: Some(expected),
        expected_note,
        clauses: clauses.len(),
        mechanical,
        projected,
        unsupported,
        modeling,
    }
}

fn sanitize_ident(name: &str) -> String {
    let mut out = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        out.push('p');
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, 'p');
    }
    out
}

fn type_kind_of(ty: &str) -> String {
    let ty = ty.trim();
    if ty.starts_with('&') {
        "reference".to_owned()
    } else if ty.starts_with("Tracked<") || ty.starts_with("Ghost<") {
        "ghost".to_owned()
    } else if ty == "bool" {
        "bool".to_owned()
    } else if is_int_type(ty) {
        "int".to_owned()
    } else {
        "opaque".to_owned()
    }
}

fn is_int_type(ty: &str) -> bool {
    matches!(ty, "int" | "nat" | "char" | "usize" | "isize")
        || ty
            .strip_prefix(['u', 'i'])
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

fn replace_ident(text: &str, name: &str, replacement: &str) -> String {
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '%';
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if token == name {
            out.push_str(replacement);
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if identifier(c) {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn substitute_type(ty: &str, types: &BTreeMap<String, String>) -> String {
    let mut out = ty.to_owned();
    let mut names = types.keys().collect::<Vec<_>>();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    for name in names {
        let replacement = &types[name];
        out = out.replace(&format!("Typ TypParam {name}"), replacement);
        out = replace_ident(&out, name, replacement);
    }
    out
}

fn param_uses(clause: &Value, name: &str) -> Vec<Value> {
    clause["param_uses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["name"].as_str() == Some(name))
        .cloned()
        .collect()
}

fn param_is_mappable(name: &str, clauses: &[Value]) -> bool {
    let mut seen = false;
    for clause in clauses {
        let occurrences = clause["param_occurrences"][name].as_u64().unwrap_or(0) as usize;
        let uses = param_uses(clause, name);
        if occurrences == 0 && uses.is_empty() {
            continue;
        }
        if occurrences == 0 || uses.len() != occurrences {
            return false;
        }
        for entry in &uses {
            match entry["kind"].as_str() {
                Some("receiver_root" | "field_root" | "deref_root") => {}
                _ => return false,
            }
        }
        seen = true;
    }
    seen
}

fn mapping_json(param: &ParamInfo, clauses: &[Value], projection_uses: &[ProjectionUse]) -> Value {
    let uses = clauses
        .iter()
        .flat_map(|clause| param_uses(clause, &param.name))
        .map(|entry| {
            json!({
                "kind": entry["kind"],
                "callee": entry["callee"],
                "field": entry["field"],
                "argc": entry["argc"],
            })
        })
        .collect::<Vec<_>>();
    let projections = projection_uses
        .iter()
        .filter(|entry| entry.param == param.name)
        .map(ProjectionUse::value)
        .collect::<Vec<_>>();
    json!({
        "param": param.name,
        "id": format!("{}_id", sanitize_ident(&param.name)),
        "uses": uses,
        "projections": projections,
    })
}

fn param_projection_covered(name: &str, clauses: &[Value], plans: &[ClausePlan]) -> bool {
    let mut seen = false;
    for (clause, plan) in clauses.iter().zip(plans) {
        let occurrences = clause["param_occurrences"][name].as_u64().unwrap_or(0) as usize;
        let uses = param_uses(clause, name);
        if occurrences == 0 && uses.is_empty() {
            continue;
        }
        seen = true;
        if !matches!(plan, ClausePlan::Projected(_)) {
            return false;
        }
    }
    seen
}

fn referenced_names(clauses: &[Value], params: &[Value]) -> BTreeSet<String> {
    let param_names = params
        .iter()
        .filter_map(|param| param["name"].as_str())
        .collect::<BTreeSet<_>>();
    let mut referenced = BTreeSet::new();
    for clause in clauses {
        if let Some(vars) = clause["vars"].as_array() {
            for var in vars.iter().filter_map(Value::as_str) {
                if param_names.contains(var) {
                    referenced.insert(var.to_owned());
                }
            }
        }
    }
    referenced
}

fn param_models(
    params: &[ParamInfo],
    referenced: &BTreeSet<String>,
) -> BTreeMap<String, ParamModel> {
    params
        .iter()
        .filter_map(|param| {
            if !referenced.contains(&param.name) {
                return None;
            }
            let int = param.kind != "bool";
            let probe = if param.kind == "int" || param.kind == "bool" {
                sanitize_ident(&param.name)
            } else {
                format!("{}_id", sanitize_ident(&param.name))
            };
            Some((param.name.clone(), ParamModel { probe, int }))
        })
        .collect()
}

fn rewrite_consts(
    ast: &Value,
    const_names: &mut BTreeMap<String, String>,
    used_names: &mut BTreeSet<String>,
) -> Value {
    match ast["op"].as_str() {
        Some("const") => {
            let path = ast["path"].as_str().unwrap_or("const");
            let name = lift_const(path, const_names, used_names);
            json!({"op": "var", "name": name})
        }
        Some("not") => {
            json!({"op": "not", "arg": rewrite_consts(&ast["arg"], const_names, used_names)})
        }
        Some("neg") => {
            json!({"op": "neg", "arg": rewrite_consts(&ast["arg"], const_names, used_names)})
        }
        Some("bin") => json!({
            "op": "bin",
            "sym": ast["sym"],
            "lhs": rewrite_consts(&ast["lhs"], const_names, used_names),
            "rhs": rewrite_consts(&ast["rhs"], const_names, used_names),
        }),
        _ => ast.clone(),
    }
}

fn lift_const(
    path: &str,
    const_names: &mut BTreeMap<String, String>,
    used_names: &mut BTreeSet<String>,
) -> String {
    if let Some(name) = const_names.get(path) {
        return name.clone();
    }
    let base = path
        .rsplit("::")
        .next()
        .unwrap_or("c")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let base = if base.is_empty() {
        "c".to_owned()
    } else {
        base
    };
    let mut name = base.clone();
    let mut suffix = 2;
    while used_names.contains(&name) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    used_names.insert(name.clone());
    const_names.insert(path.to_owned(), name.clone());
    name
}

fn collect_ast_vars(ast: &Value, out: &mut BTreeSet<String>) {
    match ast["op"].as_str() {
        Some("var") => {
            if let Some(name) = ast["name"].as_str() {
                out.insert(name.to_owned());
            }
        }
        Some("not") | Some("neg") => collect_ast_vars(&ast["arg"], out),
        Some("bin") => {
            collect_ast_vars(&ast["lhs"], out);
            collect_ast_vars(&ast["rhs"], out);
        }
        _ => {}
    }
}

fn ordered_args(
    vars: &BTreeSet<String>,
    params: &[ParamInfo],
    model: &BTreeMap<String, ParamModel>,
    const_names: &BTreeMap<String, String>,
    param_order: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut args = Vec::new();
    for param in params {
        let original = param.name.as_str();
        if let Some(entry) = model.get(original) {
            if vars.contains(&entry.probe) || vars.contains(original) {
                args.push((entry.probe.clone(), entry.probe.clone()));
                if !param_order.contains(&entry.probe) {
                    param_order.push(entry.probe.clone());
                }
            }
        }
    }
    for (path, name) in const_names {
        if vars.contains(name) {
            args.push((name.clone(), name.clone()));
            if !param_order.contains(name) {
                param_order.push(name.clone());
            }
            let _ = path;
        }
    }
    args
}

fn unsupported_args(
    clause: &Value,
    params: &[ParamInfo],
    model: &BTreeMap<String, ParamModel>,
    used_names: &mut BTreeSet<String>,
    const_names: &mut BTreeMap<String, String>,
    param_order: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut vars = BTreeSet::new();
    for var in clause["vars"].as_array().into_iter().flatten() {
        if let Some(var) = var.as_str() {
            vars.insert(var.to_owned());
        }
    }
    let mut args = Vec::new();
    for param in params {
        let original = param.name.as_str();
        if vars.contains(original) {
            if let Some(entry) = model.get(original) {
                args.push((entry.probe.clone(), entry.probe.clone()));
                if !param_order.contains(&entry.probe) {
                    param_order.push(entry.probe.clone());
                }
            } else {
                // Free variable that is not a modelable parameter: keep an
                // opaque id with a deterministic name.
                let base = format!("{}_id", sanitize_ident(original));
                let mut name = base.clone();
                let mut suffix = 2;
                while used_names.contains(&name) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                used_names.insert(name.clone());
                args.push((name.clone(), name.clone()));
                if !param_order.contains(&name) {
                    param_order.push(name);
                }
            }
        }
    }
    for path in clause["consts"].as_array().into_iter().flatten() {
        if let Some(path) = path.as_str() {
            let name = lift_const(path, const_names, used_names);
            args.push((name.clone(), name.clone()));
            if !param_order.contains(&name) {
                param_order.push(name);
            }
        }
    }
    args
}

fn render_mech_fn(
    out: &mut String,
    index: usize,
    clause_index: usize,
    args: &[(String, String)],
    ast: &Value,
    model: &BTreeMap<String, ParamModel>,
    use_int: bool,
) {
    let arglist = args
        .iter()
        .map(|(name, _)| {
            let int = model.get(name).map_or(true, |entry| entry.int);
            format!(
                "{name}: {}",
                if int {
                    if use_int { "int" } else { "usize" }
                } else {
                    "bool"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!(
        "pub open spec fn p{index}_k{clause_index}({arglist}) -> bool {{\n    {}\n}}\n\n",
        render_ast(ast)
    ));
}

fn render_ast(ast: &Value) -> String {
    match ast["op"].as_str() {
        Some("int") => ast["value"].to_string(),
        Some("bool") => ast["value"].to_string(),
        Some("var") => ast["name"].as_str().unwrap_or("_").to_owned(),
        Some("const") => ast["path"].as_str().unwrap_or("_").to_owned(),
        Some("not") => format!("!({})", render_ast(&ast["arg"])),
        Some("neg") => format!("-({})", render_ast(&ast["arg"])),
        Some("bin") => format!(
            "({} {} {})",
            render_ast(&ast["lhs"]),
            ast["sym"].as_str().unwrap_or("&&"),
            render_ast(&ast["rhs"])
        ),
        _ => "true".into(),
    }
}

fn search_witness(
    constraints: &[Value],
    param_order: &[String],
    param_types: &BTreeMap<String, bool>,
) -> Option<Vec<Value>> {
    let constraint_vars = constraints
        .iter()
        .map(|ast| {
            let mut vars = BTreeSet::new();
            collect_ast_vars(ast, &mut vars);
            vars
        })
        .collect::<Vec<_>>();
    let domains: Vec<Vec<Value>> = param_order
        .iter()
        .map(|name| {
            if param_types.get(name).copied().unwrap_or(true) {
                [
                    0i64, 1, 2, 3, 4, 5, 6, 7, 8, 15, 16, 31, 32, 63, 64, 255, 256, 4096,
                ]
                .into_iter()
                .map(|value| json!(value))
                .collect()
            } else {
                vec![json!(false), json!(true)]
            }
        })
        .collect();

    let mut env: BTreeMap<String, Value> = BTreeMap::new();
    let mut nodes = 0usize;
    const NODE_CAP: usize = 200_000;

    fn evaluate(ast: &Value, env: &BTreeMap<String, Value>) -> Option<Value> {
        match ast["op"].as_str()? {
            "int" | "bool" => Some(ast["value"].clone()),
            "var" => env.get(ast["name"].as_str()?).cloned(),
            "const" => None,
            "not" => Some(json!(!evaluate(&ast["arg"], env)?.as_bool()?)),
            "neg" => Some(json!(-evaluate(&ast["arg"], env)?.as_i64()?)),
            "bin" => {
                let lhs = evaluate(&ast["lhs"], env)?;
                let rhs = evaluate(&ast["rhs"], env)?;
                let sym = ast["sym"].as_str()?;
                Some(match sym {
                    "+" => json!(lhs.as_i64()? + rhs.as_i64()?),
                    "-" => json!(lhs.as_i64()? - rhs.as_i64()?),
                    "*" => json!(lhs.as_i64()? * rhs.as_i64()?),
                    "/" => {
                        let rhs = rhs.as_i64()?;
                        if rhs == 0 {
                            return None;
                        }
                        json!(lhs.as_i64()? / rhs)
                    }
                    "%" => {
                        let rhs = rhs.as_i64()?;
                        if rhs == 0 {
                            return None;
                        }
                        json!(lhs.as_i64()? % rhs)
                    }
                    "==" => json!(lhs == rhs),
                    "!=" => json!(lhs != rhs),
                    "<" => json!(lhs.as_i64()? < rhs.as_i64()?),
                    "<=" => json!(lhs.as_i64()? <= rhs.as_i64()?),
                    ">" => json!(lhs.as_i64()? > rhs.as_i64()?),
                    ">=" => json!(lhs.as_i64()? >= rhs.as_i64()?),
                    "&&" => json!(lhs.as_bool()? && rhs.as_bool()?),
                    "||" => json!(lhs.as_bool()? || rhs.as_bool()?),
                    "==>" => json!(!lhs.as_bool()? || rhs.as_bool()?),
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    fn feasible(
        constraints: &[Value],
        constraint_vars: &[BTreeSet<String>],
        env: &BTreeMap<String, Value>,
    ) -> bool {
        for (ast, vars) in constraints.iter().zip(constraint_vars) {
            if vars.iter().all(|name| env.contains_key(name)) {
                match evaluate(ast, env) {
                    Some(value) if value.as_bool() == Some(true) => {}
                    _ => return false,
                }
            }
        }
        true
    }

    fn dfs(
        index: usize,
        param_order: &[String],
        domains: &[Vec<Value>],
        constraints: &[Value],
        constraint_vars: &[BTreeSet<String>],
        env: &mut BTreeMap<String, Value>,
        nodes: &mut usize,
    ) -> Option<Vec<Value>> {
        if *nodes > NODE_CAP {
            return None;
        }
        *nodes += 1;
        if index == param_order.len() {
            return Some(param_order.iter().map(|name| env[name].clone()).collect());
        }
        let name = &param_order[index];
        for value in &domains[index] {
            env.insert(name.clone(), value.clone());
            if feasible(constraints, constraint_vars, env) {
                if let Some(values) = dfs(
                    index + 1,
                    param_order,
                    domains,
                    constraints,
                    constraint_vars,
                    env,
                    nodes,
                ) {
                    return Some(values);
                }
            }
            env.remove(name);
        }
        None
    }

    dfs(
        0,
        param_order,
        &domains,
        constraints,
        &constraint_vars,
        &mut env,
        &mut nodes,
    )
}

fn render_witness(
    values: &[Value],
    param_order: &[String],
    param_types: &BTreeMap<String, bool>,
    use_int: bool,
) -> String {
    let parts = values
        .iter()
        .zip(param_order)
        .map(|(value, name)| {
            if param_types.get(name).copied().unwrap_or(true) {
                if use_int {
                    value.as_i64().unwrap_or(0).to_string()
                } else {
                    format!("{}usize", value.as_i64().unwrap_or(0))
                }
            } else {
                value.as_bool().unwrap_or(false).to_string()
            }
        })
        .collect::<Vec<_>>();
    if parts.len() == 1 {
        parts[0].clone()
    } else {
        format!("({})", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, kind: &str) -> Value {
        json!({"name": name, "type": kind, "kind": kind, "mode": "exec"})
    }

    fn clause(index: usize, class: &str, ast: Value, vars: &[&str]) -> Value {
        json!({
            "index": index,
            "span": null,
            "file": null,
            "line": null,
            "text": null,
            "class": class,
            "ast": if class == "mechanical" { ast } else { Value::Null },
            "ast_vars": [],
            "ast_consts": [],
            "vars": vars,
            "consts": [],
            "calls": [],
            "features": {},
        })
    }

    fn function(path: &str, params: Vec<Value>, generics: usize, clauses: Vec<Value>) -> Value {
        json!({
            "path": path,
            "name": path.rsplit("::").next().unwrap_or(path),
            "mode": "exec",
            "kind": "static",
            "visibility": {"kind": "pub", "restricted_to": null},
            "source": {"file": null, "line": null, "span": null},
            "external_body": false,
            "trait": null,
            "generics": {"count": generics, "names": []},
            "params": params,
            "requires": clauses,
        })
    }

    fn census(functions: Vec<Value>) -> Value {
        json!({"schema": "spec-check.census/1", "crate": "p", "functions": functions})
    }

    #[test]
    fn tiers_follow_the_documented_blockers() {
        let lt = json!({
            "op": "bin", "sym": "<",
            "lhs": {"op": "var", "name": "x"},
            "rhs": {"op": "var", "name": "y"}
        });
        let unsupported = clause(0, "unsupported", Value::Null, &["x"]);
        let data = census(vec![
            function(
                "p::a",
                vec![param("x", "int"), param("y", "int")],
                0,
                vec![clause(0, "mechanical", lt.clone(), &["x", "y"])],
            ),
            function(
                "p::b",
                vec![param("x", "int")],
                0,
                vec![unsupported.clone()],
            ),
            function(
                "p::c",
                vec![param("x", "int")],
                1,
                vec![clause(0, "mechanical", lt.clone(), &["x"])],
            ),
            function(
                "p::d",
                vec![param("g", "ghost")],
                0,
                vec![clause(0, "mechanical", lt, &["g"])],
            ),
            function("p::e", vec![param("r", "reference")], 0, vec![]),
            function("p::f", vec![param("x", "int")], 0, vec![]),
        ]);
        let built = build(&data, &Options::default()).unwrap();
        let rows = built["report"]["functions"].as_array().unwrap();
        let tiers = rows
            .iter()
            .map(|row| row["tier"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(tiers, vec!["A", "B", "C", "C", "none", "none"]);
        assert_eq!(built["report"]["counts"]["tier_a"], 1);
        assert_eq!(built["report"]["counts"]["tier_b"], 1);
        assert_eq!(built["report"]["counts"]["tier_c"], 2);
    }

    #[test]
    fn mechanical_clauses_are_wrapped_and_witnessed() {
        let ast = json!({
            "op": "bin", "sym": "<=",
            "lhs": {"op": "var", "name": "x"},
            "rhs": {"op": "int", "value": 3}
        });
        let data = census(vec![function(
            "p::f",
            vec![param("x", "int"), param("unused", "int")],
            0,
            vec![clause(0, "mechanical", ast, &["x"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "x: usize");
        assert_eq!(contract["requires"], "p0_k0(x)");
        assert_eq!(contract["witness"], "0usize");
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("pub open spec fn p0_k0(x: usize) -> bool")
        );
        assert_eq!(built["report"]["entries"][0]["expected"], "ConfirmedLive");
    }

    #[test]
    fn unsatisfiable_scalar_clause_has_no_witness_and_expects_vacuous() {
        let ast = json!({
            "op": "bin", "sym": "&&",
            "lhs": {"op": "bin", "sym": ">", "lhs": {"op": "var", "name": "x"}, "rhs": {"op": "int", "value": 7}},
            "rhs": {"op": "bin", "sym": "<", "lhs": {"op": "var", "name": "x"}, "rhs": {"op": "int", "value": 8}}
        });
        let data = census(vec![function(
            "p::f",
            vec![param("x", "int")],
            0,
            vec![clause(0, "mechanical", ast, &["x"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert!(built["manifest"]["contracts"][0].get("witness").is_none());
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
        assert!(
            built["report"]["entries"][0]["expected_note"]
                .as_str()
                .unwrap()
                .contains("E6")
        );
    }

    #[test]
    fn constant_clauses_are_lifted_to_parameters() {
        let ast = json!({
            "op": "bin", "sym": "<=",
            "lhs": {"op": "var", "name": "x"},
            "rhs": {"op": "const", "path": "p::MAX_LEN"}
        });
        let data = census(vec![function(
            "p::f",
            vec![param("x", "int")],
            0,
            vec![clause(0, "mechanical", ast, &["x"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "x: usize, max_len: usize");
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("p0_k0(x: usize, max_len: usize)")
        );
        assert_eq!(contract["witness"], "(0usize, 0usize)");
    }

    #[test]
    fn unsupported_clause_becomes_zero_arg_uninterp() {
        let data = census(vec![function(
            "p::f",
            vec![param("self", "opaque")],
            0,
            vec![clause(0, "unsupported", Value::Null, &[])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("pub uninterp spec fn p0_k0() -> bool;")
        );
        assert_eq!(built["manifest"]["contracts"][0]["requires"], "p0_k0()");
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
    }

    #[test]
    fn unsupported_clause_binds_opaque_parameters_as_ids() {
        let data = census(vec![function(
            "p::f",
            vec![param("left", "opaque"), param("right", "opaque")],
            0,
            vec![clause(0, "unsupported", Value::Null, &["left", "right"])],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "left_id: usize, right_id: usize");
        assert_eq!(contract["requires"], "p0_k0(left_id, right_id)");
        let preamble = built["manifest"]["preamble"].as_str().unwrap();
        assert!(
            preamble
                .contains("pub uninterp spec fn p0_k0(left_id: usize, right_id: usize) -> bool;"),
            "{preamble}"
        );
    }

    #[test]
    fn boolean_parameters_get_bool_domains_and_witnesses() {
        let data = census(vec![function(
            "p::f",
            vec![param("b", "bool")],
            0,
            vec![clause(
                0,
                "mechanical",
                json!({"op": "var", "name": "b"}),
                &["b"],
            )],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert_eq!(built["manifest"]["contracts"][0]["params"], "b: bool");
        assert_eq!(built["manifest"]["contracts"][0]["witness"], "true");
        assert_eq!(built["report"]["entries"][0]["expected"], "ConfirmedLive");
    }

    #[test]
    fn batching_is_deterministic_and_selects_contiguous_ranges() {
        let ast = json!({"op": "bool", "value": true});
        let functions = (0..5)
            .map(|index| {
                function(
                    &format!("p::f{index}"),
                    vec![param("x", "int")],
                    0,
                    vec![clause(0, "mechanical", ast.clone(), &["x"])],
                )
            })
            .collect();
        let data = census(functions);
        let options = Options {
            batch: Some(1),
            batch_size: 2,
            ..Options::default()
        };
        let built = build(&data, &options).unwrap();
        let entries = built["report"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["path"], "p::f2");
        assert_eq!(entries[1]["path"], "p::f3");
    }

    #[test]
    fn filter_and_limit_apply_after_tier_selection() {
        let ast = json!({"op": "bool", "value": true});
        let functions = (0..3)
            .map(|index| {
                function(
                    &format!("p::keep{index}"),
                    vec![param("x", "int")],
                    0,
                    vec![clause(0, "mechanical", ast.clone(), &["x"])],
                )
            })
            .chain([function(
                "p::drop",
                vec![param("x", "int")],
                0,
                vec![clause(0, "mechanical", ast.clone(), &["x"])],
            )])
            .collect();
        let data = census(functions);
        let options = Options {
            filter: Some("keep".into()),
            limit: Some(2),
            ..Options::default()
        };
        let built = build(&data, &options).unwrap();
        assert_eq!(built["report"]["entries"].as_array().unwrap().len(), 2);
        assert_eq!(built["report"]["counts"]["candidates"], 3);
    }

    fn typed_function(
        path: &str,
        generics: &[&str],
        params: Vec<Value>,
        clauses: Vec<Value>,
    ) -> Value {
        json!({
            "path": path,
            "name": path.rsplit("::").next().unwrap_or(path),
            "mode": "exec",
            "kind": "static",
            "visibility": {"kind": "pub", "restricted_to": null},
            "source": {"file": null, "line": null, "span": null},
            "external_body": false,
            "trait": null,
            "generics": {"count": generics.len(), "names": generics},
            "params": params,
            "requires": clauses,
        })
    }

    fn config(value: Value) -> Config {
        Config::from_value(&value).unwrap()
    }

    fn uses_clause(kind: &str, name: &str, occurrences: usize) -> Value {
        let mut clause = clause(0, "unsupported", Value::Null, &[name]);
        clause["param_occurrences"] = json!({name: occurrences});
        clause["param_uses"] =
            json!([{"name": name, "kind": kind, "callee": "p::inv", "field": null, "argc": 1}]);
        clause
    }

    #[test]
    fn generic_instantiation_reclassifies_and_unmapped_names_stay_c() {
        let lt = json!({
            "op": "bin", "sym": "<=",
            "lhs": {"op": "var", "name": "n"},
            "rhs": {"op": "int", "value": 3}
        });
        let data = census(vec![
            typed_function(
                "p::lemma",
                &["E"],
                vec![param("s", "opaque"), param("n", "int")],
                vec![clause(0, "mechanical", lt.clone(), &["n"])],
            ),
            typed_function(
                "p::other",
                &["E"],
                vec![param("n", "int")],
                vec![clause(0, "mechanical", lt, &["n"])],
            ),
        ]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "generic_instances": [{"match": "p::lemma", "types": {"E": "u64"}}]
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let rows = built["report"]["functions"].as_array().unwrap();
        assert_eq!(rows[0]["tier"], "A");
        assert_eq!(rows[0]["baseline_tier"], "C");
        assert_eq!(
            rows[0]["modeling"]["generic_instantiation"]["matched"],
            "p::lemma"
        );
        assert_eq!(built["report"]["counts"]["baseline"]["tier_c"], 2);
        assert_eq!(built["report"]["counts"]["upgraded"]["to_a"], 1);
        assert_eq!(rows[1]["tier"], "C");
        assert!(
            rows[1]["reasons"][0]
                .as_str()
                .unwrap()
                .starts_with("generic:")
        );
        assert_eq!(built["manifest"]["contracts"][0]["witness"], "0usize");
    }

    #[test]
    fn generic_instantiation_defaults_all_parameters_to_star() {
        let ast = json!({
            "op": "bin", "sym": "==",
            "lhs": {"op": "var", "name": "n"},
            "rhs": {"op": "int", "value": 0}
        });
        let data = census(vec![typed_function(
            "p::f",
            &["T", "U"],
            vec![param("n", "int")],
            vec![clause(0, "mechanical", ast, &["n"])],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "generic_instances": [{"match": "*", "types": {"*": "u64"}}]
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(built["report"]["functions"][0]["tier"], "A");
    }

    #[test]
    fn reference_receiver_use_is_abstracted_to_an_id_at_tier_b() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference"), param("cpu", "opaque")],
            vec![uses_clause("receiver_root", "self", 1)],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"}
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "B");
        assert_eq!(row["baseline_tier"], "C");
        assert_eq!(row["modeling"]["reference_mapping"][0]["id"], "self_id");
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "self_id: usize");
        assert_eq!(contract["requires"], "p0_k0(self_id)");
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
    }

    #[test]
    fn reference_direct_use_stays_tier_c_with_an_unmapped_reason() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference")],
            vec![uses_clause("plain_root", "self", 1)],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"}
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "C");
        assert_eq!(row["reasons"][0], "reference_param_unmapped:self");
        assert!(
            built["manifest"]["contracts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn ghost_receiver_is_abstracted_only_when_the_policy_is_on() {
        let data = census(vec![typed_function(
            "p::perm",
            &[],
            vec![param("tracked", "ghost")],
            vec![uses_clause("receiver_root", "tracked", 1)],
        )]);
        let off = build(&data, &Options::default()).unwrap();
        assert_eq!(off["report"]["functions"][0]["tier"], "C");
        assert_eq!(
            off["report"]["functions"][0]["reasons"][0],
            "ghost_param:tracked"
        );
        let on = build(
            &data,
            &Options {
                config: config(json!({
                    "schema": CONFIG_SCHEMA,
                    "ghost": {"mode": "opaque"}
                })),
                ..Options::default()
            },
        )
        .unwrap();
        let row = &on["report"]["functions"][0];
        assert_eq!(row["tier"], "B");
        assert_eq!(row["modeling"]["ghost_mapping"][0]["id"], "tracked_id");
        assert_eq!(
            on["manifest"]["contracts"][0]["params"],
            "tracked_id: usize"
        );
    }

    #[test]
    fn tier_recomputation_is_deterministic() {
        let data = census(vec![
            typed_function(
                "p::g",
                &["T"],
                vec![param("self", "reference")],
                vec![uses_clause("receiver_root", "self", 1)],
            ),
            typed_function(
                "p::h",
                &[],
                vec![param("self", "reference")],
                vec![uses_clause("plain_root", "self", 1)],
            ),
        ]);
        let options = Options {
            config: config(json!({
                "schema": CONFIG_SCHEMA,
                "generic_instances": [{"match": "*", "types": {"*": "u64"}}],
                "reference": {"mode": "opaque"},
                "ghost": {"mode": "opaque"}
            })),
            ..Options::default()
        };
        let first = build(&data, &options).unwrap();
        let second = build(&data, &options).unwrap();
        assert_eq!(first, second);
    }

    fn projected_clause(expr: Value, uses: &[(&str, &str)]) -> Value {
        let vars = uses.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        let mut clause = clause(0, "unsupported", Value::Null, &vars);
        clause["expr"] = expr;
        let mut occurrences = serde_json::Map::new();
        let mut details = Vec::new();
        for (name, kind) in uses {
            occurrences.insert((*name).to_owned(), json!(1));
            details.push(json!({
                "name": name,
                "kind": kind,
                "callee": null,
                "field": null,
                "argc": null,
            }));
        }
        clause["param_occurrences"] = Value::Object(occurrences);
        clause["param_uses"] = json!(details);
        clause
    }

    fn call_expr(callee: &str, args: Vec<Value>) -> Value {
        json!({"op": "call", "callee": callee, "dynamic": false, "args": args})
    }

    fn var_expr(name: &str) -> Value {
        json!({"op": "var", "name": name})
    }

    fn projection_config(rules: Value) -> Config {
        config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"},
            "ghost": {"mode": "opaque"},
            "projection": {
                "models": [
                    {"name": "pcell_id", "params": ["recv: usize"], "returns": "usize", "body": "recv"},
                    {"name": "cell_id", "params": ["recv: usize"], "returns": "usize", "body": "recv"},
                    {"name": "inv", "params": ["recv: usize"], "returns": "bool", "body": null},
                    {"name": "idx", "params": ["recv: usize"], "returns": "usize", "body": null}
                ],
                "rules": rules
            }
        }))
    }

    fn mutex_expr() -> Value {
        json!({
            "op": "bin", "sym": "==",
            "lhs": call_expr("vstd::cell::pcell::impl&%2::id", vec![var_expr("perm")]),
            "rhs": call_expr("ostd::sync::mutex::impl&%2::cell_id", vec![var_expr("mutex")])
        })
    }

    #[test]
    fn identity_projection_makes_id_equality_mechanical_and_live() {
        let data = census(vec![typed_function(
            "p::new",
            &[],
            vec![param("mutex", "reference"), param("perm", "ghost")],
            vec![projected_clause(
                mutex_expr(),
                &[("mutex", "plain_root"), ("perm", "plain_root")],
            )],
        )]);
        let configured = projection_config(json!([
            {"match": "p::new", "kind": "method", "callee": "vstd::cell::pcell::impl&%2::id", "model": "pcell_id", "relation": "faithful", "note": "CellId is the permission identity"},
            {"match": "p::new", "kind": "method", "callee": "ostd::sync::mutex::impl&%2::cell_id", "model": "cell_id", "relation": "faithful", "note": "CellId projection"}
        ]));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "A");
        assert_eq!(row["baseline_tier"], "C");
        assert_eq!(
            row["modeling"]["projection_mapping"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            row["modeling"]["clause_modes"],
            json!(["projected-evaluable"])
        );
        let contract = &built["manifest"]["contracts"][0];
        assert_eq!(contract["params"], "mutex_id: int, perm_id: int");
        assert_eq!(contract["requires"], "p0_k0(mutex_id, perm_id)");
        assert_eq!(contract["witness"], "(0, 0)");
        let preamble = built["manifest"]["preamble"].as_str().unwrap();
        assert!(preamble.contains("pub open spec fn pcell_id(recv: int) -> int"));
        assert!(preamble.contains("pub open spec fn cell_id(recv: int) -> int"));
        assert_eq!(built["report"]["entries"][0]["expected"], "ConfirmedLive");
        assert!(
            built["report"]["entries"][0]["expected_note"]
                .as_str()
                .unwrap()
                .contains("model-relative")
        );
    }

    #[test]
    fn unmodeled_projection_keeps_tier_c() {
        let data = census(vec![typed_function(
            "p::new",
            &[],
            vec![param("mutex", "reference"), param("perm", "ghost")],
            vec![projected_clause(
                mutex_expr(),
                &[("mutex", "plain_root"), ("perm", "plain_root")],
            )],
        )]);
        let configured = projection_config(json!([]));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "C");
        assert_eq!(
            row["reasons"],
            json!([
                "reference_param_unmapped:mutex",
                "ghost_param_unmapped:perm"
            ])
        );
        assert_eq!(
            row["modeling"]["projection_unmapped"][0]["detail"],
            "unmodeled call: vstd::cell::pcell::impl&%2::id"
        );
        assert!(
            built["manifest"]["contracts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn opaque_projection_is_tier_b_and_not_real_liveness() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference")],
            vec![projected_clause(
                call_expr("p::inv", vec![var_expr("self")]),
                &[("self", "plain_root")],
            )],
        )]);
        let configured = projection_config(json!([
            {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "over-approx", "note": "invariant predicate"}
        ]));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "B");
        assert_eq!(row["modeling"]["clause_modes"], json!(["projected-opaque"]));
        let preamble = built["manifest"]["preamble"].as_str().unwrap();
        assert!(preamble.contains("pub uninterp spec fn inv(recv: int) -> bool;"));
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
        assert!(
            built["report"]["entries"][0]["expected_note"]
                .as_str()
                .unwrap()
                .contains("model-relative")
        );
    }

    #[test]
    fn constant_projection_without_witness_expects_inconclusive_vacuous_candidate() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference")],
            vec![projected_clause(
                call_expr("p::obeys", vec![var_expr("self")]),
                &[("self", "receiver_root")],
            )],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"},
            "projection": {
                "models": [{"name": "obeys", "params": ["recv: usize"], "returns": "bool", "body": "false"}],
                "rules": [{"match": "*", "kind": "method", "callee": "p::obeys", "model": "obeys", "relation": "faithful", "note": "crate-scope all implementations return true"}]
            }
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "A");
        assert!(built["manifest"]["contracts"][0].get("witness").is_none());
        assert_eq!(
            built["report"]["entries"][0]["expected"],
            "CandidateInconclusive"
        );
        assert!(
            built["report"]["entries"][0]["expected_note"]
                .as_str()
                .unwrap()
                .contains("E6")
        );
    }

    #[test]
    fn under_approx_projection_is_never_vacuous_evidence() {
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference")],
            vec![projected_clause(
                call_expr("p::obeys", vec![var_expr("self")]),
                &[("self", "receiver_root")],
            )],
        )]);
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"},
            "projection": {
                "models": [{"name": "obeys", "params": ["recv: usize"], "returns": "bool", "body": "false"}],
                "rules": [{"match": "*", "kind": "method", "callee": "p::obeys", "model": "obeys", "relation": "under-approx", "note": "known weaker"}]
            }
        }));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(
            built["report"]["entries"][0]["expected_note"]
                .as_str()
                .unwrap()
                .contains("under-approx")
        );
    }

    #[test]
    fn field_projection_records_datatype_and_field() {
        let expr = json!({
            "op": "bin", "sym": "<",
            "lhs": {"op": "field", "datatype": "p::Owner", "field": "idx", "root": var_expr("self")},
            "rhs": {"op": "int", "value": 4}
        });
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "ghost")],
            vec![projected_clause(expr, &[("self", "plain_root")])],
        )]);
        let configured = projection_config(json!([
            {"match": "*", "kind": "field", "datatype": "p::Owner", "field": "idx", "model": "idx", "relation": "over-approx", "note": "field projection"}
        ]));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        let row = &built["report"]["functions"][0];
        assert_eq!(row["tier"], "B");
        assert_eq!(
            row["modeling"]["projection_mapping"][0]["uses"][0]["kind"],
            "field"
        );
        assert_eq!(
            row["modeling"]["projection_mapping"][0]["uses"][0]["target"],
            "p::Owner::idx"
        );
    }

    #[test]
    fn quantifier_projection_is_tier_b() {
        let expr = json!({
            "op": "quant",
            "forall": true,
            "bindings": [{"name": "i", "type": "usize"}],
            "body": json!({
                "op": "bin", "sym": "&&",
                "lhs": call_expr("p::inv", vec![var_expr("self")]),
                "rhs": {"op": "bin", "sym": "<", "lhs": var_expr("i"), "rhs": {"op": "int", "value": 4}}
            })
        });
        let data = census(vec![typed_function(
            "p::m",
            &[],
            vec![param("self", "reference")],
            vec![projected_clause(expr, &[("self", "plain_root")])],
        )]);
        let configured = projection_config(json!([
            {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "over-approx", "note": "invariant"}
        ]));
        let built = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(built["report"]["functions"][0]["tier"], "B");
        let preamble = built["manifest"]["preamble"].as_str().unwrap();
        assert!(
            preamble.contains("pub uninterp spec fn p0_k0"),
            "{preamble}"
        );
        assert!(!preamble.contains("forall|q0_i"), "{preamble}");
    }

    #[test]
    fn projection_config_rejects_missing_relation_and_bad_body() {
        let error = Config::from_value(&json!({
            "schema": CONFIG_SCHEMA,
            "projection": {
                "models": [{"name": "m", "params": ["recv: usize"], "returns": "usize", "body": "recv"}],
                "rules": [{"match": "*", "kind": "method", "callee": "p::m", "model": "m"}]
            }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("relation"), "{error}");

        let error = Config::from_value(&json!({
            "schema": CONFIG_SCHEMA,
            "projection": {
                "models": [{"name": "m", "params": ["recv: usize"], "returns": "bool", "body": "recv"}],
                "rules": []
            }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("body type"), "{error}");

        let error = Config::from_value(&json!({
            "schema": CONFIG_SCHEMA,
            "projection": {
                "models": [{"name": "m", "params": ["recv: usize"], "returns": "usize", "body": "recv >> 1"}],
                "rules": []
            }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("body"), "{error}");

        let error = Config::from_value(&json!({
            "schema": CONFIG_SCHEMA,
            "projection": {
                "models": [
                    {"name": "m", "params": ["recv: usize"], "returns": "usize", "body": null},
                    {"name": "m", "params": ["recv: usize"], "returns": "usize", "body": null}
                ],
                "rules": []
            }
        }))
        .unwrap_err();
        assert!(error.to_string().contains("duplicate"), "{error}");
    }

    #[test]
    fn field_model_arity_is_validated_at_config_load() {
        let error = Config::from_value(&json!({
            "schema": CONFIG_SCHEMA,
            "projection": {
                "models": [{"name": "m", "params": ["a: usize", "b: usize"], "returns": "usize", "body": null}],
                "rules": [{"match": "*", "kind": "field", "datatype": "p::T", "field": "f", "model": "m", "relation": "over-approx"}]
            }
        }))
        .unwrap_err();
        assert!(
            error.to_string().contains("exactly one argument"),
            "{error}"
        );
    }

    #[test]
    fn projection_recomputation_is_deterministic() {
        let data = census(vec![typed_function(
            "p::new",
            &[],
            vec![param("mutex", "reference"), param("perm", "ghost")],
            vec![projected_clause(
                mutex_expr(),
                &[("mutex", "plain_root"), ("perm", "plain_root")],
            )],
        )]);
        let configured = projection_config(json!([
            {"match": "p::new", "kind": "method", "callee": "vstd::cell::pcell::impl&%2::id", "model": "pcell_id", "relation": "faithful", "note": ""},
            {"match": "p::new", "kind": "method", "callee": "ostd::sync::mutex::impl&%2::cell_id", "model": "cell_id", "relation": "faithful", "note": ""}
        ]));
        let options = Options {
            config: configured,
            ..Options::default()
        };
        assert_eq!(
            build(&data, &options).unwrap(),
            build(&data, &options).unwrap()
        );
    }

    #[test]
    fn projection_disabled_preserves_m7e_unsupported_classification() {
        let data = census(vec![typed_function(
            "p::f",
            &[],
            vec![param("x", "int")],
            vec![projected_clause(
                json!({
                    "op": "bin", "sym": "==",
                    "lhs": {"op": "int", "value": 1},
                    "rhs": {"op": "int", "value": 1}
                }),
                &[],
            )],
        )]);
        let baseline = build(&data, &Options::default()).unwrap();
        assert_eq!(baseline["report"]["functions"][0]["tier"], "B");
        let configured = config(json!({
            "schema": CONFIG_SCHEMA,
            "projection": {
                "models": [{"name": "m", "params": ["a: int"], "returns": "int", "body": "a"}],
                "rules": []
            }
        }));
        let projected = build(
            &data,
            &Options {
                config: configured,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(projected["report"]["functions"][0]["tier"], "A");
    }

    #[test]
    fn empty_param_mechanical_entry_uses_dummy_parameter() {
        let data = census(vec![function(
            "p::f",
            vec![],
            0,
            vec![clause(
                0,
                "mechanical",
                json!({"op": "bool", "value": false}),
                &[],
            )],
        )]);
        let built = build(&data, &Options::default()).unwrap();
        assert_eq!(
            built["manifest"]["contracts"][0]["params"],
            "_unused: usize"
        );
        assert!(built["manifest"]["contracts"][0].get("witness").is_none());
    }

    fn expression_config(models: Value, rules: Value) -> Config {
        config(json!({
            "schema": CONFIG_SCHEMA,
            "reference": {"mode": "opaque"},
            "ghost": {"mode": "opaque"},
            "projection": {"models": models, "rules": rules}
        }))
    }

    #[test]
    fn constructor_argument_is_a_structural_token() {
        let expr = json!({
            "op": "bin", "sym": "&&",
            "lhs": call_expr("p::inv", vec![var_expr("r")]),
            "rhs": call_expr(
                "p::consume",
                vec![json!({
                    "op": "ctor",
                    "datatype": "Dt Tuple 0",
                    "variant": "tuple%0",
                    "args": []
                })],
            )
        });
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "inv", "params": ["a: usize"], "returns": "bool", "body": "true"},
                    {"name": "consume", "params": ["a: usize"], "returns": "bool", "body": null}
                ]),
                json!([
                    {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "faithful"},
                    {"match": "*", "kind": "method", "callee": "p::consume", "model": "consume", "relation": "over-approx"}
                ]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference"), param("x", "int")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let entry = &built["report"]["entries"][0];
        assert_eq!(entry["tier"], "B");
        assert_eq!(entry["projected_clauses"], 1);
        assert_eq!(entry["modeling"]["vacuum_relation"], "over-approx");
        assert_eq!(
            entry["modeling"]["projection_relations"]["structural_relaxed_clauses"],
            1
        );
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("pub uninterp spec fn consume")
        );
    }

    #[test]
    fn block_with_let_is_translated_in_sequence() {
        let expr = json!({
            "op": "bin", "sym": "&&",
            "lhs": call_expr("p::inv", vec![var_expr("r")]),
            "rhs": {
                "op": "block",
                "bindings": [{
                    "name": "y",
                    "init": {"op": "bin", "sym": "+", "lhs": var_expr("x"), "rhs": {"op": "int", "value": 1}}
                }],
                "body": {"op": "bin", "sym": ">", "lhs": var_expr("y"), "rhs": {"op": "int", "value": 0}}
            }
        });
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "inv", "params": ["a: usize"], "returns": "bool", "body": "true"}
                ]),
                json!([
                    {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "faithful"}
                ]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference"), param("x", "int")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let entry = &built["report"]["entries"][0];
        assert_eq!(entry["tier"], "A");
        assert_eq!(entry["expected"], "ConfirmedLive");
        assert_eq!(entry["modeling"]["clause_modes"][0], "projected-evaluable");
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("x + 1")
        );
    }

    #[test]
    fn cast_and_div_mod_are_over_approx_relaxations() {
        let expr = json!({
            "op": "bin", "sym": "&&",
            "lhs": call_expr("p::inv", vec![var_expr("r")]),
            "rhs": {
                "op": "bin", "sym": "==",
                "lhs": {
                    "op": "bin", "sym": "%",
                    "lhs": {
                        "op": "bin", "sym": "/",
                        "lhs": {
                            "op": "cast",
                            "arg": {"op": "bin", "sym": "+", "lhs": var_expr("x"), "rhs": {"op": "int", "value": 1}}
                        },
                        "rhs": {"op": "int", "value": 2}
                    },
                    "rhs": {"op": "int", "value": 3}
                },
                "rhs": {"op": "int", "value": 0}
            }
        });
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "inv", "params": ["a: usize"], "returns": "bool", "body": "true"}
                ]),
                json!([
                    {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "faithful"}
                ]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference"), param("x", "int")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let entry = &built["report"]["entries"][0];
        assert_eq!(entry["tier"], "A");
        assert_eq!(entry["modeling"]["vacuum_relation"], "over-approx");
        assert!(
            entry["modeling"]["projection_relations"]["structural_relaxed_clauses"]
                .as_u64()
                .unwrap()
                >= 1
        );
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("%")
        );
    }

    #[test]
    fn match_scalarizes_to_an_if_else_chain() {
        let expr = json!({
            "op": "bin", "sym": "&&",
            "lhs": call_expr("p::inv", vec![var_expr("r")]),
            "rhs": {
                "op": "match",
                "scrutinee": var_expr("x"),
                "arms": [
                    {
                        "pattern": {"kind": "ctor", "datatype": "p::T", "variant": "A", "args": []},
                        "guard": {"op": "bool", "value": true},
                        "body": {"op": "bool", "value": true}
                    },
                    {
                        "pattern": {"kind": "wildcard"},
                        "guard": {"op": "bool", "value": true},
                        "body": {"op": "bool", "value": false}
                    }
                ]
            }
        });
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "inv", "params": ["a: usize"], "returns": "bool", "body": "true"},
                    {"name": "is_a", "params": ["a: usize"], "returns": "bool", "body": null}
                ]),
                json!([
                    {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "faithful"},
                    {"match": "*", "kind": "variant", "datatype": "p::T", "variant": "A", "model": "is_a", "relation": "over-approx"}
                ]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference"), param("x", "int")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let entry = &built["report"]["entries"][0];
        assert_eq!(entry["tier"], "B");
        assert_eq!(entry["modeling"]["clause_modes"][0], "projected-opaque");
        assert!(
            built["manifest"]["preamble"]
                .as_str()
                .unwrap()
                .contains("if is_a(x) { true } else { false }")
        );
    }

    #[test]
    fn tuple_field_without_datatype_is_an_opaque_over_approx() {
        let expr = json!({
            "op": "bin", "sym": "&&",
            "lhs": call_expr("p::inv", vec![var_expr("r")]),
            "rhs": {
                "op": "bin", "sym": "==",
                "lhs": {"op": "field", "datatype": null, "field": "0", "root": var_expr("x")},
                "rhs": {"op": "int", "value": 0}
            }
        });
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "inv", "params": ["a: usize"], "returns": "bool", "body": "true"}
                ]),
                json!([
                    {"match": "*", "kind": "method", "callee": "p::inv", "model": "inv", "relation": "faithful"}
                ]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference"), param("x", "int")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let entry = &built["report"]["entries"][0];
        assert_eq!(entry["tier"], "A");
        assert_eq!(entry["modeling"]["vacuum_relation"], "over-approx");
        assert_eq!(entry["modeling"]["tuple_field_projection"][0]["field"], "0");
    }

    #[test]
    fn field_without_datatype_has_an_explicit_diagnostic() {
        let expr = json!({
            "op": "field", "datatype": null, "field": "named", "root": var_expr("r")
        });
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "unused", "params": [], "returns": "bool", "body": "true"}
                ]),
                json!([]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let function = &built["report"]["functions"][0];
        assert_eq!(function["tier"], "C");
        assert!(
            function["modeling"]["projection_unmapped"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("field without datatype")
        );
    }

    #[test]
    fn projection_type_conflict_is_explicit() {
        let expr = call_expr("p::f", vec![var_expr("r")]);
        let options = Options {
            config: expression_config(
                json!([
                    {"name": "m", "params": ["a: bool"], "returns": "bool", "body": null}
                ]),
                json!([
                    {"match": "*", "kind": "method", "callee": "p::f", "model": "m", "relation": "over-approx"}
                ]),
            ),
            ..Options::default()
        };
        let data = census(vec![function(
            "p::f",
            vec![param("r", "reference")],
            0,
            vec![projected_clause(expr, &[("r", "plain_root")])],
        )]);
        let built = build(&data, &options).unwrap();
        let function = &built["report"]["functions"][0];
        assert_eq!(function["tier"], "C");
        assert!(
            function["modeling"]["projection_unmapped"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("projection type conflict")
        );
    }
}
