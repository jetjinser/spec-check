//! Three independent satisfiability queries for complete scalar VIR contracts.
use crate::{
    Result, execute, hash, ir,
    vir::{self, Node},
    write_json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Term {
    Var(String),
    Bool(bool),
    Int(String),
    Not(Box<Term>),
    And(Vec<Term>),
    Or(Vec<Term>),
    Eq([Box<Term>; 2]),
    Lt([Box<Term>; 2]),
    Le([Box<Term>; 2]),
    Implies([Box<Term>; 2]),
    Add([Box<Term>; 2]),
    Sub([Box<Term>; 2]),
    Ite([Box<Term>; 3]),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    vir: PathBuf,
    function: String,
    source_root: PathBuf,
    target: Term,
    target_origin: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sort {
    Bool,
    Int,
}
impl Sort {
    pub(crate) fn smt(self) -> &'static str {
        match self {
            Self::Bool => "Bool",
            Self::Int => "Int",
        }
    }
}
pub(crate) fn items(n: &Node) -> &[Node] {
    n.unwrap().list().unwrap_or(&[])
}
pub(crate) fn atom(n: &Node, i: usize) -> Option<&str> {
    items(n).get(i)?.atom()
}
pub(crate) fn head(n: &Node) -> Option<&str> {
    let i = usize::from(atom(n, 0) == Some(">"));
    atom(n, i)
}
pub(crate) fn name(n: &Node) -> Result<String> {
    Ok(serde_json::to_string(&format!("{n:?}"))?)
}
fn variable(n: &Node, env: &BTreeMap<String, String>) -> Result<Term> {
    Ok(Term::Var(
        env.get(&name(n)?).ok_or("unbound VIR local")?.clone(),
    ))
}
pub(crate) fn expression(n: &Node, env: &BTreeMap<String, String>) -> Result<Term> {
    let v = items(n);
    let expected = match head(n) {
        Some("Const") => 3,
        Some("ReadPlace" | "Unary" | "Block") => 4,
        Some("Logical" | "Binary" | "If") => 5,
        _ => return Err(format!("unsupported VIR expression {:?}", head(n)).into()),
    };
    if v.len() != expected {
        return Err("unsupported VIR expression shape".into());
    }
    match head(n) {
        Some("Const") => match (atom(&v[2], 1), atom(&v[2], 2)) {
            (Some("Bool"), Some(b @ ("true" | "false"))) => Ok(Term::Bool(b == "true")),
            (Some("Int"), Some(i)) => Ok(Term::Int(i.into())),
            _ => Err("unsupported constant".into()),
        },
        Some("ReadPlace")
            if head(&v[2]) == Some("Place")
                && atom(&v[2], 1) == Some("Local")
                && items(&v[2]).len() == 3 =>
        {
            variable(&items(&v[2])[2], env)
        }
        Some("Unary") if atom(&v[2], 1) == Some("Not") => {
            Ok(Term::Not(Box::new(expression(&v[3], env)?)))
        }
        Some("Block") if items(&v[2]).is_empty() && v.len() == 4 => expression(&v[3], env),
        Some("If") => Ok(Term::Ite([
            Box::new(expression(&v[2], env)?),
            Box::new(expression(&v[3], env)?),
            Box::new(expression(&v[4], env)?),
        ])),
        Some("Logical") | Some("Binary") => {
            let pair = [
                Box::new(expression(&v[3], env)?),
                Box::new(expression(&v[4], env)?),
            ];
            match (atom(&v[2], 1), items(&v[2]).get(2)) {
                (Some("Eq"), _) => Ok(Term::Eq(pair)),
                (Some("Ne"), _) => Ok(Term::Not(Box::new(Term::Eq(pair)))),
                (Some("And"), _) => Ok(Term::And(pair.into_iter().map(|x| *x).collect())),
                (Some("Or"), _) => Ok(Term::Or(pair.into_iter().map(|x| *x).collect())),
                (Some("Implies"), _) => Ok(Term::Implies(pair)),
                (Some("Inequality"), Some(op)) => match atom(op, 1) {
                    Some("Lt") => Ok(Term::Lt(pair)),
                    Some("Le") => Ok(Term::Le(pair)),
                    Some("Gt") => Ok(Term::Lt([pair[1].clone(), pair[0].clone()])),
                    Some("Ge") => Ok(Term::Le([pair[1].clone(), pair[0].clone()])),
                    _ => Err("unsupported inequality".into()),
                },
                (Some("Arith"), Some(op))
                    if atom(op, 1) == Some("Add") || atom(op, 1) == Some("Sub") =>
                {
                    // Only mathematical integer arithmetic, never machine truncation.
                    let overflow = items(op).get(2).ok_or("missing arithmetic semantics")?;
                    if atom(overflow, 0) != Some("OverflowBehavior")
                        || items(overflow)
                            .last()
                            .is_none_or(|r| atom(r, 1) != Some("Int"))
                    {
                        return Err("machine arithmetic is unsupported".into());
                    }
                    if atom(op, 1) == Some("Add") {
                        Ok(Term::Add(pair))
                    } else {
                        Ok(Term::Sub(pair))
                    }
                }
                _ => Err("unsupported binary expression".into()),
            }
        }
        _ => Err(format!("unsupported VIR expression {:?}", head(n)).into()),
    }
}
pub(crate) fn encode(t: &Term, vars: &BTreeMap<String, Sort>) -> Result<(String, Sort)> {
    let boolean = Sort::Bool;
    let integer = Sort::Int;
    match t {
        Term::Var(v) => Ok((v.clone(), *vars.get(v).ok_or("unbound target variable")?)),
        Term::Bool(b) => Ok((b.to_string(), boolean)),
        Term::Int(n) => {
            let digits = n.strip_prefix('-').unwrap_or(n);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err("invalid decimal integer".into());
            }
            let normalized = digits.trim_start_matches('0');
            let digits = if normalized.is_empty() {
                "0"
            } else {
                normalized
            };
            Ok((
                if n.starts_with('-') {
                    format!("(- {digits})")
                } else {
                    digits.into()
                },
                integer,
            ))
        }
        Term::Ite(parts) => {
            let (condition, cs) = encode(&parts[0], vars)?;
            let (yes, ys) = encode(&parts[1], vars)?;
            let (no, ns) = encode(&parts[2], vars)?;
            if cs != boolean || ys != ns {
                return Err("conditional requires Bool guard and equal branch sorts".into());
            }
            Ok((format!("(ite {condition} {yes} {no})"), ys))
        }
        Term::Not(a) => {
            let (s, k) = encode(a, vars)?;
            if k != boolean {
                return Err("not requires Bool".into());
            }
            Ok((format!("(not {s})"), boolean))
        }
        Term::And(xs) | Term::Or(xs) => {
            let mut parts = Vec::new();
            for x in xs {
                let (s, k) = encode(x, vars)?;
                if k != boolean {
                    return Err("logical sort mismatch".into());
                }
                parts.push(s)
            }
            let op = if matches!(t, Term::And(_)) {
                "and"
            } else {
                "or"
            };
            Ok((
                if parts.is_empty() {
                    if op == "and" { "true" } else { "false" }.into()
                } else {
                    format!("({op} {})", parts.join(" "))
                },
                boolean,
            ))
        }
        Term::Eq(p)
        | Term::Lt(p)
        | Term::Le(p)
        | Term::Implies(p)
        | Term::Add(p)
        | Term::Sub(p) => {
            let (a, ka) = encode(&p[0], vars)?;
            let (b, kb) = encode(&p[1], vars)?;
            let (op, required, result) = match t {
                Term::Eq(_) => ("=", None, boolean),
                Term::Lt(_) => ("<", Some(integer), boolean),
                Term::Le(_) => ("<=", Some(integer), boolean),
                Term::Implies(_) => ("=>", Some(boolean), boolean),
                Term::Add(_) => ("+", Some(integer), integer),
                _ => ("-", Some(integer), integer),
            };
            if ka != kb || required.is_some_and(|k| k != ka) {
                return Err("binary sort mismatch".into());
            }
            Ok((format!("({op} {a} {b})"), result))
        }
    }
}
fn target_arithmetic(t: &Term, mathematical: &BTreeSet<String>, arithmetic: bool) -> Result<()> {
    match t {
        Term::Var(v) if arithmetic && !mathematical.contains(v) => return Err("target arithmetic requires mathematical int variables; machine/nat arithmetic is unsupported".into()),
        Term::Not(t) => target_arithmetic(t, mathematical, arithmetic)?,
        Term::Ite(parts) => {
            target_arithmetic(&parts[0], mathematical, false)?;
            target_arithmetic(&parts[1], mathematical, arithmetic)?;
            target_arithmetic(&parts[2], mathematical, arithmetic)?;
        }
        Term::And(xs) | Term::Or(xs) => for x in xs { target_arithmetic(x, mathematical, arithmetic)?; },
        Term::Eq(xs) | Term::Lt(xs) | Term::Le(xs) | Term::Implies(xs) | Term::Add(xs) | Term::Sub(xs) => {
            let arithmetic = arithmetic || matches!(t, Term::Add(_) | Term::Sub(_));
            for x in xs { target_arithmetic(x, mathematical, arithmetic)?; }
        },
        _ => (),
    }
    Ok(())
}
pub(crate) fn sort(n: &Node) -> Result<(Sort, Option<String>)> {
    match atom(n, 1) {
        Some("Bool") => Ok((Sort::Bool, None)),
        Some("Int") => match items(n).get(2).and_then(|r| atom(r, 1)) {
            Some("Int") => Ok((Sort::Int, None)),
            Some("Nat") => Ok((Sort::Int, Some("(<= 0 @)".into()))),
            range @ Some("U" | "I") => {
                let bits = items(n)
                    .get(2)
                    .and_then(|r| atom(r, 2))
                    .ok_or("missing width")?
                    .parse::<u32>()?;
                if bits == 0 || bits > 64 {
                    return Err("unsupported integer width".into());
                }
                let domain = if range == Some("I") {
                    let half = 1u128 << (bits - 1);
                    format!("(and (<= (- {half}) @) (<= @ {}))", half - 1)
                } else {
                    format!("(and (<= 0 @) (<= @ {}))", (1u128 << bits) - 1)
                };
                Ok((Sort::Int, Some(domain)))
            }
            _ => Err("unsupported integer type".into()),
        },
        _ => Err("only scalar Bool/int/nat/u8..u64/i8..i64 signatures are supported".into()),
    }
}
pub(crate) fn origin(n: &Node, root: &Path) -> Result<Value> {
    let span = n
        .list()
        .and_then(|x| x.get(1))
        .and_then(Node::atom)
        .ok_or("missing source span")?;
    let (path, coords) = ir::span(span, root)?;
    let path = fs::canonicalize(path)?;
    if !path.starts_with(fs::canonicalize(root)?) {
        return Err("source span is outside declared source_root".into());
    }
    let bytes = fs::read(&path)?;
    let text = std::str::from_utf8(&bytes)?;
    let (a, b) = ir::offsets(text, coords)?;
    Ok(json!({"span":span,"path":path,"file_sha256":hash(&bytes)?,"snippet":&text[a..b]}))
}
pub(crate) fn classify(p: &str, r: &str, q: &str) -> &'static str {
    if p == "unsat" {
        "precondition_unsat"
    } else if p != "sat" {
        "unknown"
    } else if r == "unsat" {
        "relation_unsat"
    } else if r != "sat" {
        "unknown"
    } else if q == "unsat" {
        "contract_sufficient"
    } else if q == "sat" {
        "contract_allows_violation"
    } else {
        "unknown"
    }
}
pub(crate) fn solve(
    z3: &Path,
    out: &Path,
    label: &str,
    decl: &str,
    rows: &[(String, String, Value)],
    vars: &BTreeMap<String, Sort>,
) -> Result<Value> {
    let mut smt = format!(
        "(set-option :timeout 10000)\n(set-option :produce-unsat-cores true)\n(set-option :produce-models true)\n(set-logic QF_LIA)\n{decl}"
    );
    for (id, body, _) in rows {
        smt.push_str(&format!("(assert (! {body} :named {id}))\n"))
    }
    smt.push_str("(check-sat)\n");
    let query = out.join(format!("{label}.smt2"));
    fs::write(&query, &smt)?;
    let result = execute(Command::new(z3).arg("-smt2").arg(&query), out, label)?;
    let status = std::str::from_utf8(&result.stdout)?.trim();
    if !result.status.success() || !["sat", "unsat", "unknown"].contains(&status) {
        return Err(format!("solver failed for {label}; see retained diagnostics").into());
    }
    if status == "sat" {
        smt.push_str(&format!(
            "(get-value ({}))\n",
            vars.keys().cloned().collect::<Vec<_>>().join(" ")
        ))
    } else if status == "unsat" {
        smt.push_str("(get-unsat-core)\n")
    } else {
        smt.push_str("(get-info :reason-unknown)\n")
    }
    fs::write(&query, &smt)?;
    let replay = execute(
        Command::new(z3).arg("-smt2").arg(&query),
        out,
        &format!("{label}-evidence"),
    )?;
    let raw = std::str::from_utf8(&replay.stdout)?;
    if !replay.status.success() || raw.lines().next() != Some(status) {
        return Err("solver replay changed status".into());
    }
    Ok(
        json!({"status":status,"raw_evidence":raw,"query":query,"clauses":rows.iter().map(|(id,body,source)|json!({"id":id,"formula":body,"source":source})).collect::<Vec<_>>(),"interpretation":if status=="sat"{"Logical scalar assignment; resource construction and program reachability unestablished"}else if status=="unsat"{"Core names identify a sufficient subset of this query, not necessarily minimal or Verus dependencies"}else{"No logical conclusion; preserve reason-unknown"}}),
    )
}
type Rows = Vec<(String, String, Value)>;
struct Plan {
    vars: BTreeMap<String, Sort>,
    p: Rows,
    pr: Rows,
    violation: Rows,
    decl: String,
    signature: Vec<Value>,
}
pub(crate) fn source_hashes(v: &Value, hashes: &mut BTreeMap<String, String>) -> Result<()> {
    if let (Some(path), Some(digest)) = (v["path"].as_str(), v["file_sha256"].as_str())
        && hashes
            .insert(path.into(), digest.into())
            .is_some_and(|old| old != digest)
    {
        return Err("source changed between clauses".into());
    }
    match v {
        Value::Object(o) => {
            for child in o.values() {
                source_hashes(child, hashes)?;
            }
        }
        Value::Array(a) => {
            for child in a {
                source_hashes(child, hashes)?;
            }
        }
        _ => (),
    }
    Ok(())
}
pub fn run(request: &Path, z3: &Path, out: &Path) -> Result<()> {
    let request = fs::canonicalize(request)?;
    let z3 = fs::canonicalize(z3)?;
    let bytes = fs::read(&request)?;
    let req: Request = serde_json::from_slice(&bytes)?;
    if req.target_origin.trim().is_empty() {
        return Err("target_origin must explain who supplied Q and why".into());
    }
    let vir_path = fs::canonicalize(&req.vir)?;
    let vir_bytes = fs::read(&vir_path)?;
    let functions = vir::functions(std::str::from_utf8(&vir_bytes)?)?;
    let selected = functions
        .iter()
        .filter(|f| {
            f.field("name")
                .and_then(|n| n.field("path"))
                .and_then(Node::atom)
                == Some(&req.function)
        })
        .collect::<Vec<_>>();
    let [function] = selected.as_slice() else {
        return Err("function path is missing or ambiguous".into());
    };
    fs::create_dir(out)?;
    let out = fs::canonicalize(out)?;
    let inputs = json!({"request":request,"request_sha256":hash(&bytes)?,"vir":vir_path,"vir_sha256":hash(&vir_bytes)?,"z3":z3,"z3_sha256":hash(&fs::read(&z3)?)?,"checker_sha256":hash(&fs::read(std::env::current_exe()?)?)?});
    let work = || -> Result<Plan> {
        let kind = function.field("kind").ok_or("missing function kind")?;
        if atom(kind, 0) != Some("FunctionKind")
            || atom(kind, 1) != Some("Static")
            || items(kind).len() != 2
        {
            return Err("only Static functions are supported; inherited trait contracts require explicit merging".into());
        }
        if !matches!(
            function.field("mode").and_then(Node::atom),
            Some("Exec" | "Proof")
        ) {
            return Err("only Exec/Proof contracts are supported; spec function bodies are not return contracts".into());
        }
        for field in ["typ_params", "typ_bounds"] {
            if !function
                .field(field)
                .and_then(Node::list)
                .is_some_and(|v| v.is_empty())
            {
                return Err("generic contract is unsupported".into());
            }
        }
        if function.field("returns").and_then(Node::atom) != Some("None") {
            return Err("returns clause is unsupported".into());
        }
        let mut env = BTreeMap::new();
        let mut vars = BTreeMap::new();
        let mut p = Vec::new();
        let mut domains = Vec::new();
        let mut signature = Vec::new();
        let mut mathematical = BTreeSet::new();
        let params = function
            .field("params")
            .and_then(Node::list)
            .ok_or("missing parameters")?;
        let ret = function.field("outer_ret").ok_or("missing return")?;
        for (i, param) in params.iter().chain([ret]).enumerate() {
            if param.field("mode").and_then(Node::atom) == Some("Proof") {
                return Err("tracked resource parameters unsupported".into());
            }
            let alias = if i == params.len() {
                "result".into()
            } else {
                format!("p{i}")
            };
            let (k, bound) = sort(param.field("typ").ok_or("missing type")?)?;
            env.insert(
                name(param.field("name").ok_or("missing local name")?)?,
                alias.clone(),
            );
            vars.insert(alias.clone(), k);
            let typ = param.field("typ").ok_or("missing type")?;
            if atom(typ, 1) == Some("Int")
                && items(typ).get(2).is_some_and(|r| atom(r, 1) == Some("Int"))
            {
                mathematical.insert(alias.clone());
            }
            signature.push(json!({"alias":alias,"mode":param.field("mode").and_then(Node::atom),"source":origin(param,&req.source_root)?}));
            if let Some(b) = bound {
                domains.push((
                    format!("type_{alias}"),
                    b.replace('@', &alias),
                    json!({"kind":"scalar_type","source":origin(param,&req.source_root)?}),
                ));
            }
        }
        for (i, clause) in function
            .field("require")
            .and_then(Node::list)
            .ok_or("missing requires")?
            .iter()
            .enumerate()
        {
            let (t, k) = encode(&expression(clause, &env)?, &vars)?;
            if k != Sort::Bool {
                return Err("nonboolean requires".into());
            }
            p.push((
                format!("requires_{i}"),
                t,
                origin(clause, &req.source_root)?,
            ));
        }
        // Return domains belong to R; P only constrains input types.
        let mut r = domains
            .iter()
            .filter(|(id, _, _)| id == "type_result")
            .cloned()
            .collect::<Vec<_>>();
        p.extend(domains.into_iter().filter(|(id, _, _)| id != "type_result"));
        let ensures = function
            .field("ensure")
            .and_then(Node::list)
            .ok_or("missing ensures")?;
        if ensures.len() != 3
            || ensures[0].atom() != Some("tuple")
            || !items(&ensures[2]).is_empty()
        {
            return Err("unsupported ensures binder".into());
        }
        for (i, clause) in items(&ensures[1]).iter().enumerate() {
            let (t, k) = encode(&expression(clause, &env)?, &vars)?;
            if k != Sort::Bool {
                return Err("nonboolean ensures".into());
            }
            r.push((format!("ensures_{i}"), t, origin(clause, &req.source_root)?));
        }
        target_arithmetic(&req.target, &mathematical, false)?;
        let (q, k) = encode(&req.target, &vars)?;
        if k != Sort::Bool {
            return Err("target must be Boolean".into());
        }
        let decl = vars
            .iter()
            .map(|(v, k)| format!("(declare-const {v} {})\n", k.smt()))
            .collect::<String>();
        let mut pr = p.clone();
        pr.extend(r);
        let mut violation = pr.clone();
        violation.push((
            "negated_target".into(),
            format!("(not {q})"),
            json!({"origin":req.target_origin,"term":req.target}),
        ));
        Ok(Plan {
            vars,
            p,
            pr,
            violation,
            decl,
            signature,
        })
    };
    let mut report = match work() {
        Err(e) => json!({"status":"unsupported","reason":e.to_string()}),
        Ok(plan) => {
            let queries = || -> Result<Value> {
                let a = solve(&z3, &out, "precondition", &plan.decl, &plan.p, &plan.vars)?;
                let b = solve(&z3, &out, "relation", &plan.decl, &plan.pr, &plan.vars)?;
                let c = solve(
                    &z3,
                    &out,
                    "violation",
                    &plan.decl,
                    &plan.violation,
                    &plan.vars,
                )?;
                let status = classify(
                    a["status"].as_str().unwrap(),
                    b["status"].as_str().unwrap(),
                    c["status"].as_str().unwrap(),
                );
                Ok(
                    json!({"status":status,"approximation":"exact_within_supported_scalar_contract_fragment",
                    "precondition":a,"relation":b,"violation":c,
                    "signature":plan.signature,"variables":plan.vars.iter().map(|(n,k)|(n,k.smt())).collect::<BTreeMap<_,_>>() }),
                )
            };
            match queries() {
                Ok(v) => v,
                Err(e) => json!({"status":"execution_error","reason":e.to_string()}),
            }
        }
    };
    report["target"] = json!({"term":req.target,"origin":req.target_origin,"binding":"manually_supplied_goal_not_extracted_assertion"});
    report["trusted_dependencies"] = json!({"compiler_VIR_and_translation":"trusted, not formally proved",
        "VIR_producer_context":"Caller must bind this VIR to its native compilation; this low-level command does not establish that correspondence",
        "external_body":function.field("attrs").and_then(|a|a.field("is_external_body")).and_then(Node::atom),
        "function_kind":format!("{:?}",function.field("kind")),"function_mode":function.field("mode").and_then(Node::atom),
        "parameter_modes":function.field("params").and_then(Node::list).unwrap_or_default().iter().map(|p|p.field("mode").and_then(Node::atom)).collect::<Vec<_>>(),
        "implementation_verification":"not established by contract queries",
        "program_reachability":"not established"});
    report["inputs"] = inputs.clone();
    report["function"] = json!(req.function);
    report["scope"] = json!(
        "Complete explicit scalar requires/ensures plus scalar type domains. Q is supplied by the request. No implementation, termination, cfg correspondence, reachability or tracked semantics theorem; omitted contract expressions reject the query."
    );
    if hash(&fs::read(&request)?)? != report["inputs"]["request_sha256"]
        || hash(&fs::read(&vir_path)?)? != report["inputs"]["vir_sha256"]
    {
        return Err("query inputs changed during execution".into());
    }
    let mut sources = BTreeMap::new();
    source_hashes(&report, &mut sources)?;
    for (path, digest) in &sources {
        if hash(&fs::read(path)?)? != *digest {
            return Err("source changed while solving".into());
        }
    }
    if hash(&fs::read(&z3)?)? != inputs["z3_sha256"]
        || hash(&fs::read(std::env::current_exe()?)?)? != inputs["checker_sha256"]
    {
        return Err("solver or checker changed while solving".into());
    }
    report["source_inputs"] = json!(sources);
    write_json(&out.join("report.json"), &report)?;
    let mut lines = vec![
        "# 契约查询".to_owned(),
        String::new(),
        format!(
            "函数 `{}`，结果 `{}`。",
            req.function,
            report["status"].as_str().unwrap_or("unknown")
        ),
        format!("目标由请求者提供：{}。", req.target_origin),
        String::new(),
        "| 查询 | 结果 |".into(),
        "|---|---|".into(),
    ];
    for (key, label) in [
        ("precondition", "P"),
        ("relation", "P ∧ R"),
        ("violation", "P ∧ R ∧ ¬Q"),
    ] {
        lines.push(format!(
            "| {label} | {} |",
            report[key]["status"].as_str().unwrap_or("未运行")
        ));
    }
    if let Some(reason) = report["reason"].as_str() {
        lines.push(format!("\n原因：{reason}"));
    }
    lines.push("\nSAT 表示编码中的逻辑赋值可满足。程序可达性与实现行为需要另行核验；目标由人提供，错误目标也会得到契约允许违反的结果。逐项公式、源位置、模型和冲突子集见 report.json 及查询日志。".into());
    fs::write(out.join("README.md"), lines.join("\n") + "\n")?;
    println!("{}: {}", req.function, report["status"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_relation_is_not_a_guarantee() {
        assert_eq!(classify("sat", "unsat", "unsat"), "relation_unsat");
        assert_eq!(classify("unsat", "unsat", "unsat"), "precondition_unsat");
        assert_eq!(classify("unknown", "sat", "unsat"), "unknown");
        assert_eq!(classify("sat", "sat", "unknown"), "unknown");
    }
    #[test]
    fn conditional_domains_and_sorts_remain_checked() {
        let vars = BTreeMap::from([("p0".into(), Sort::Int)]);
        let bad_guard = Term::Ite([
            Box::new(Term::Var("p0".into())),
            Box::new(Term::Bool(true)),
            Box::new(Term::Bool(false)),
        ]);
        assert!(encode(&bad_guard, &vars).is_err());
        let bad_arms = Term::Ite([
            Box::new(Term::Bool(true)),
            Box::new(Term::Int("1".into())),
            Box::new(Term::Bool(false)),
        ]);
        assert!(encode(&bad_arms, &vars).is_err());
        let hidden_arithmetic = Term::Ite([
            Box::new(Term::Bool(true)),
            Box::new(Term::Add([
                Box::new(Term::Var("p0".into())),
                Box::new(Term::Int("1".into())),
            ])),
            Box::new(Term::Int("0".into())),
        ]);
        assert!(target_arithmetic(&hidden_arithmetic, &BTreeSet::new(), false).is_err());
        assert_eq!(classify("sat", "unknown", "unsat"), "unknown");
    }
    #[test]
    fn target_cannot_inject_smt_or_mix_sorts() {
        let vars = BTreeMap::from([("p0".into(), Sort::Bool)]);
        assert!(encode(&Term::Int("0) (assert false)".into()), &vars).is_err());
        assert!(
            encode(
                &Term::Lt([
                    Box::new(Term::Var("p0".into())),
                    Box::new(Term::Int("0".into()))
                ]),
                &vars
            )
            .is_err()
        );
    }
}
