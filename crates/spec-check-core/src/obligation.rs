//! Exact scalar queries for call preconditions and optional final assertions.
use crate::{
    Result,
    attribution::{self as a, Sort, Term},
    contracts, hash,
    vir::{self, Node},
    write_json,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

type Rows = Vec<(String, String, Value)>;
fn field<'a>(n: &'a Node, key: &str) -> Result<&'a Node> {
    n.field(key).ok_or_else(|| format!("missing {key}").into())
}
fn list(n: &Node) -> Result<&[Node]> {
    n.list().ok_or_else(|| "expected VIR list".into())
}
fn path(n: &Node) -> Option<&str> {
    n.field("name")?.field("path")?.atom()
}
fn eligible(f: &Node) -> Result<()> {
    if a::atom(field(f, "kind")?, 1) != Some("Static") || field(f, "mode")?.atom() != Some("Exec") {
        return Err("only static exec functions supported".into());
    }
    for key in ["typ_params", "typ_bounds"] {
        if !list(field(f, key)?)?.is_empty() {
            return Err("generic contracts unsupported".into());
        }
    }
    for key in ["mask_spec", "unwind_spec", "atomic_update"] {
        if field(f, key)?.atom() != Some("None") {
            return Err("additional contract semantics unsupported".into());
        }
    }
    Ok(())
}
fn parameters(
    f: &Node,
    prefix: &str,
    vars: &mut BTreeMap<String, Sort>,
    rows: &mut Rows,
) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    for (i, p) in list(field(f, "params")?)?.iter().enumerate() {
        if field(p, "mode")?.atom() != Some("Exec") || field(p, "user_mut")?.atom() != Some("false")
        {
            return Err("mutable or ghost parameters unsupported".into());
        }
        let alias = format!("{prefix}{i}");
        let (sort, domain) = a::sort(field(p, "typ")?)?;
        vars.insert(alias.clone(), sort);
        if let Some(domain) = domain {
            rows.push((
                format!("domain_{alias}"),
                domain.replace('@', &alias),
                json!({"kind":"type_domain"}),
            ));
        }
        env.insert(a::name(field(p, "name")?)?, alias);
    }
    Ok(env)
}
fn predicates(
    nodes: &[Node],
    env: &BTreeMap<String, String>,
    vars: &BTreeMap<String, Sort>,
    root: &Path,
    label: &str,
) -> Result<Rows> {
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (formula, sort) = a::encode(&a::expression(n, env)?, vars)?;
            if sort != Sort::Bool {
                return Err("non-boolean predicate".into());
            }
            Ok((format!("{label}_{i}"), formula, a::origin(n, root)?))
        })
        .collect()
}
fn ensures(f: &Node) -> Result<&[Node]> {
    let xs = list(field(f, "ensure")?)?;
    if xs.len() != 3 || xs[0].atom() != Some("tuple") || !list(&xs[2])?.is_empty() {
        return Err("unsupported ensures binder".into());
    }
    list(&xs[1])
}

// Only unwrap containers whose complete semantics is the contained expression.
fn assertion(n: &Node) -> Result<&Node> {
    match a::head(n) {
        Some("Stmt") if a::atom(n, 1) == Some("Expr") && a::items(n).len() == 3 => {
            assertion(&a::items(n)[2])
        }
        Some("Ghost")
            if field(n, "tracked")?.atom() == Some("false")
                && field(n, "alloc_wrapper")?.atom() == Some("false") =>
        {
            assertion(field(n, "expr")?)
        }
        Some("Block") => {
            let xs = a::items(n);
            let stmts = a::items(xs.get(2).ok_or("invalid block")?);
            if xs.len() != 4 {
                return Err("invalid block".into());
            }
            match (stmts, xs[3].atom()) {
                ([one], Some("None")) => assertion(one),
                ([], None) => assertion(&xs[3]),
                _ => Err("assertion wrapper contains extra statements".into()),
            }
        }
        Some("AssertAssume") if field(n, "is_assume")?.atom() == Some("false") => {
            Ok(field(n, "expr")?)
        }
        _ => Err(
            "expected a plain assertion; branches, assumptions and proof blocks unsupported".into(),
        ),
    }
}

fn call_statement(statement: &Node) -> Result<(Option<&Node>, &Node)> {
    if a::head(statement) != Some("Stmt") {
        return Err("expected a call statement".into());
    }
    let (binding, init) = match a::atom(statement, 1) {
        Some("Decl") if field(statement, "els")?.atom() == Some("None") => {
            let pattern = field(statement, "pattern")?.unwrap();
            if a::atom(pattern, 0) != Some("Pattern") || a::atom(pattern, 1) != Some("Var") {
                return Err("destructuring unsupported".into());
            }
            let binding = a::items(pattern).get(2).ok_or("missing binding")?;
            if field(binding, "user_mut")?.atom() != Some("false")
                || a::atom(field(binding, "by_ref")?, 1) != Some("No")
            {
                return Err("mutable binding unsupported".into());
            }
            (Some(binding), field(statement, "init")?)
        }
        Some("Expr") if a::items(statement).len() == 3 => (None, &a::items(statement)[2]),
        _ => return Err("unsupported call statement".into()),
    };
    let unwrapped = init.unwrap();
    let call =
        if a::atom(unwrapped, 0) == Some("Place") && a::atom(unwrapped, 1) == Some("Temporary") {
            a::items(unwrapped).get(2).ok_or("missing call")?
        } else {
            init
        };
    if a::head(call) != Some("Call") {
        return Err("statement is not a call".into());
    }
    Ok((binding, call))
}

struct Plan {
    vars: BTreeMap<String, Sort>,
    p: Rows,
    requires: Rows,
    returns: Rows,
    target: Rows,
    call: Value,
    assertion: Value,
    callee: Value,
    steps: Vec<Step>,
    has_assertion: bool,
}
struct Step {
    bindings: Rows,
    requires: Rows,
    returns: Rows,
    call: Value,
    callee: Value,
}
fn plan(f: &Node, functions: &[Node], root: &Path) -> Result<Plan> {
    eligible(f)?;
    if field(field(f, "attrs")?, "is_external_body")?.atom() != Some("false")
        || !ensures(f)?.is_empty()
        || field(f, "returns")?.atom() != Some("None")
    {
        return Err("external caller or additional postconditions unsupported".into());
    }
    let body = field(f, "body")?;
    let xs = a::items(body);
    if a::head(body) != Some("Block") || xs.len() != 4 || xs[3].atom() != Some("None") {
        return Err("expected statement-only caller body".into());
    }
    let statements = a::items(&xs[2]);
    let (last, _) = statements.split_last().ok_or("empty caller")?;
    let target_expr = assertion(last).ok();
    let declarations = if target_expr.is_some() {
        &statements[..statements.len() - 1]
    } else {
        statements
    };
    if declarations.is_empty() {
        return Err("expected a call sequence".into());
    }
    let mut vars = BTreeMap::new();
    let mut p = Vec::new();
    let mut env = parameters(f, "p", &mut vars, &mut p)?;
    p.extend(predicates(
        list(field(f, "require")?)?,
        &env,
        &vars,
        root,
        "entry",
    )?);
    for (_, _, origin) in &mut p {
        origin["premise_source"] = json!(if origin["kind"] == "type_domain" {
            "type_domain"
        } else {
            "caller_entry_contract"
        });
    }
    let mut steps = Vec::new();
    for (step, decl) in declarations.iter().enumerate() {
        let prefix = if step == 0 {
            "a".to_owned()
        } else {
            format!("c{step}_a")
        };
        let result = if step == 0 {
            "result".to_owned()
        } else {
            format!("result_{step}")
        };
        let mut bindings = Vec::new();
        let (binding, call) = call_statement(decl)?;
        if a::head(call) != Some("Call") {
            return Err("initializer is not a call".into());
        }
        let ct = field(call, "target")?;
        let ct = a::items(ct);
        if ct.len() != 7
            || ct[1].atom() != Some("Fun")
            || a::atom(&ct[2], 1) != Some("Static")
            || !list(&ct[4])?.is_empty()
            || !list(&ct[5])?.is_empty()
        {
            return Err("generic, dynamic or non-function call unsupported".into());
        }
        let name = ct[3]
            .field("path")
            .and_then(Node::atom)
            .ok_or("missing callee")?;
        let callees = functions
            .iter()
            .filter(|f| path(f) == Some(name))
            .collect::<Vec<_>>();
        let [callee] = callees.as_slice() else {
            return Err("callee missing or ambiguous".into());
        };
        eligible(callee)?;
        if field(call, "post_args")?.atom() != Some("None")
            || field(call, "body")?.atom() != Some("None")
        {
            return Err("special call behavior unsupported".into());
        }
        let mut callee_env = parameters(callee, &prefix, &mut vars, &mut bindings)?;
        let args = list(field(call, "args")?)?;
        if args.len() != callee_env.len() {
            return Err("argument arity mismatch".into());
        }
        for (i, arg) in args.iter().enumerate() {
            let t = a::expression(arg, &env)?;
            // Argument arithmetic requires evaluation/overflow obligations; keep it unsupported.
            if !matches!(t, Term::Var(_) | Term::Int(_) | Term::Bool(_)) {
                return Err("computed call arguments unsupported".into());
            }
            let (formula, sort) = a::encode(&t, &vars)?;
            if Some(&sort) != vars.get(&format!("{prefix}{i}")) {
                return Err("argument type mismatch".into());
            }
            bindings.push((
                format!("call_{step}_argument_{i}"),
                format!("(= {prefix}{i} {formula})"),
                a::origin(arg, root)?,
            ));
        }
        let requires = predicates(
            list(field(callee, "require")?)?,
            &callee_env,
            &vars,
            root,
            &format!("call_{step}_requires"),
        )?;
        let ret = field(callee, "outer_ret")?;
        if field(ret, "mode")?.atom() != Some("Exec") {
            return Err("tracked return unsupported".into());
        }
        let unit = field(ret, "typ")?.unwrap().list().is_some_and(|typ| {
            (typ.len() == 3
                && typ[0].atom() == Some("Typ")
                && typ[1].atom() == Some("Tuple")
                && typ[2].list().is_some_and(|fields| fields.is_empty()))
                || (typ.len() == 5
                    && typ[0].atom() == Some("Typ")
                    && typ[1].atom() == Some("Datatype")
                    && a::items(&typ[2]).len() == 3
                    && a::atom(&typ[2], 0) == Some("Dt")
                    && a::atom(&typ[2], 1) == Some("Tuple")
                    && a::atom(&typ[2], 2) == Some("0")
                    && typ[3].list().is_some_and(|args| args.is_empty())
                    && typ[4].list().is_some_and(|args| args.is_empty()))
        });
        if unit && binding.is_some() {
            return Err("unit result binding unsupported".into());
        }
        let result_type = if unit {
            None
        } else {
            Some(a::sort(field(ret, "typ")?)?)
        };
        if let Some((sort, _)) = result_type {
            if let Some(binding) = binding {
                if field(binding, "typ")? != field(ret, "typ")? {
                    return Err("result binding type mismatch".into());
                }
                env.insert(a::name(field(binding, "name")?)?, result.clone());
            }
            vars.insert(result.clone(), sort);
            callee_env.insert(a::name(field(ret, "name")?)?, result.clone());
        }
        let mut returns = predicates(
            ensures(callee)?,
            &callee_env,
            &vars,
            root,
            &format!("call_{step}_ensures"),
        )?;
        let value = field(callee, "returns")?;
        if value.atom() != Some("None") {
            let (sort, _) = result_type
                .as_ref()
                .ok_or("unit returns expression unsupported")?;
            let (formula, value_sort) = a::encode(&a::expression(value, &callee_env)?, &vars)?;
            if value_sort != *sort {
                return Err("returns expression type mismatch".into());
            }
            returns.push((
                format!("call_{step}_returns"),
                format!("(= {result} {formula})"),
                a::origin(value, root)?,
            ));
        }
        if let Some((_, Some(domain))) = result_type {
            returns.push((
                format!("domain_{result}"),
                domain.replace('@', &result),
                json!({"kind":"type_domain"}),
            ));
        }
        let trusted = field(field(callee, "attrs")?, "is_external_body")?.atom() == Some("true");
        for (name, _, origin) in &mut bindings {
            origin["premise_source"] = json!(if name.starts_with("domain_") {
                "type_domain"
            } else {
                "argument_binding"
            });
            origin["call_index"] = json!(step);
        }
        let mut requires = requires;
        for (_, _, origin) in &mut requires {
            origin["premise_source"] = json!("callee_precondition");
            origin["call_index"] = json!(step);
        }
        for (name, _, origin) in &mut returns {
            origin["premise_source"] = json!(if name.starts_with("domain_") {
                "type_domain"
            } else if name.ends_with("_returns") {
                "prior_call_returns"
            } else {
                "prior_call_postcondition"
            });
            origin["call_index"] = json!(step);
            origin["trusted_declaration"] = json!(trusted);
        }
        steps.push(Step { bindings, requires, returns, call: a::origin(call, root)?,
        callee: json!({"path":name,"external_body":field(field(callee,"attrs")?,"is_external_body")?.atom(),"contract":contracts::render(callee)}) });
    }
    let target = if let Some(expr) = target_expr {
        predicates(std::slice::from_ref(expr), &env, &vars, root, "assertion")?
    } else {
        Vec::new()
    };
    Ok(Plan {
        vars,
        p,
        requires: steps[0].requires.clone(),
        returns: steps[0].returns.clone(),
        target,
        call: steps[0].call.clone(),
        assertion: target_expr
            .map(|expr| a::origin(expr, root))
            .transpose()?
            .unwrap_or(Value::Null),
        callee: steps[0].callee.clone(),
        steps,
        has_assertion: target_expr.is_some(),
    })
}
fn query(plan: &Plan, p: &Rows, r: &Rows, q: &Rows, z3: &Path, out: &Path) -> Result<Value> {
    fs::create_dir(out)?;
    let decl = plan
        .vars
        .iter()
        .map(|(n, s)| format!("(declare-const {n} {})\n", s.smt()))
        .collect::<String>();
    let mut pr = p.clone();
    pr.extend(r.iter().cloned());
    let mut violation = pr.clone();
    violation.push((
        "negated_target".into(),
        format!(
            "(not (and true {}))",
            q.iter()
                .map(|(_, s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        json!({"source":q}),
    ));
    let pre = a::solve(z3, out, "precondition", &decl, p, &plan.vars)?;
    let rel = a::solve(z3, out, "relation", &decl, &pr, &plan.vars)?;
    let bad = a::solve(z3, out, "violation", &decl, &violation, &plan.vars)?;
    Ok(
        json!({"status":a::classify(pre["status"].as_str().unwrap(),rel["status"].as_str().unwrap(),bad["status"].as_str().unwrap()),"precondition":pre,"relation":rel,"violation":bad}),
    )
}
pub fn run(vir: &Path, function: &str, root: &Path, z3: &Path, out: &Path) -> Result<()> {
    let solver_hash = hash(&fs::read(z3)?)?;
    let bytes = fs::read(vir)?;
    let vir_hash = hash(&bytes)?;
    let functions = vir::functions(std::str::from_utf8(&bytes)?)?;
    if function == "*" {
        fs::create_dir(out)?;
        let mut reports = Vec::new();
        for (i, f) in functions
            .iter()
            .filter(|f| f.field("mode").and_then(Node::atom) == Some("Exec"))
            .enumerate()
        {
            let function = path(f).ok_or("function without identity")?;
            let case = out.join(format!("case-{i}"));
            run_parsed(
                &functions,
                function,
                root,
                z3,
                &case,
                &json!({"vir":vir,"vir_sha256":vir_hash,"function":function,"source_root":root,"z3":z3,"z3_sha256":solver_hash}),
            )?;
            let report: Value = serde_json::from_slice(&fs::read(case.join("report.json"))?)?;
            reports.push(json!({"function":function,"status":report["status"],"evidence":case.join("report.json")}));
        }
        if fs::read(vir)? != bytes || hash(&fs::read(z3)?)? != solver_hash {
            return Err("VIR or solver changed during queries".into());
        }
        return write_json(
            &out.join("report.json"),
            &json!({"results":reports,"scope":"All exec definitions in supplied VIR; unsupported definitions retained"}),
        );
    }
    run_parsed(
        &functions,
        function,
        root,
        z3,
        out,
        &json!({"vir":vir,"vir_sha256":vir_hash,"function":function,"source_root":root,"z3":z3,"z3_sha256":solver_hash}),
    )?;
    if fs::read(vir)? != bytes || hash(&fs::read(z3)?)? != solver_hash {
        return Err("VIR or solver changed during queries".into());
    }
    Ok(())
}

fn run_parsed(
    functions: &[Node],
    function: &str,
    root: &Path,
    z3: &Path,
    out: &Path,
    inputs: &Value,
) -> Result<()> {
    let selected = functions
        .iter()
        .filter(|f| path(f) == Some(function))
        .collect::<Vec<_>>();
    let [f] = selected.as_slice() else {
        return Err("caller missing or ambiguous".into());
    };
    fs::create_dir(out)?;
    let work = || -> Result<Value> {
        let plan = match plan(f, functions, root) {
            Ok(p) => p,
            Err(e) => return Ok(json!({"status":"unsupported","reason":e.to_string()})),
        };
        let mut p = plan.p.clone();
        let mut checks = Vec::new();
        let mut first_call = Value::Null;
        let mut final_assertion = Value::Null;
        let mut failed_obligation = Value::Null;
        let mut status = "unknown".to_owned();
        for (index, step) in plan.steps.iter().enumerate() {
            p.extend(step.bindings.iter().cloned());
            let call = query(
                &plan,
                &p,
                &Vec::new(),
                &step.requires,
                z3,
                &out.join(if index == 0 {
                    "call".into()
                } else {
                    format!("call-{index}")
                }),
            )?;
            if index == 0 {
                first_call = call.clone();
            }
            let call_status = call["status"].as_str().unwrap_or("unknown");
            if call_status != "contract_sufficient" {
                failed_obligation = json!({"kind":"call_precondition","call_index":index,"source":step.call,"conditions":step.requires});
                status = match call_status {
                    "precondition_unsat" => "precondition_unsat",
                    "contract_allows_violation" => "call_precondition_not_entailed",
                    _ => "unknown",
                }
                .into();
                checks.push(json!({"index":index,"call":step.call,"callee":step.callee,"entry_and_bindings":p,"requires":step.requires,"call_check":call,"return_check":"not_attempted_precondition_unestablished"}));
                break;
            }
            p.extend(
                step.requires
                    .iter()
                    .cloned()
                    .map(|(name, formula, mut source)| {
                        source["premise_source"] = json!("established_call_precondition");
                        (name, formula, source)
                    }),
            );
            let final_step = index + 1 == plan.steps.len();
            let target = if final_step {
                plan.target.clone()
            } else {
                Vec::new()
            };
            let relation = query(
                &plan,
                &p,
                &step.returns,
                &target,
                z3,
                &out.join(if final_step {
                    "assertion".into()
                } else {
                    format!("return-{index}")
                }),
            )?;
            status = relation["status"].as_str().unwrap_or("unknown").to_owned();
            checks.push(json!({"index":index,"call":step.call,"callee":step.callee,"entry_and_bindings":p,"requires":step.requires,"returns":step.returns,"call_check":call,"return_check":relation}));
            if final_step && plan.has_assertion {
                final_assertion = relation;
            }
            if status != "contract_sufficient" {
                let assertion_failed =
                    final_step && plan.has_assertion && status != "relation_unsat";
                failed_obligation = json!({"kind":if assertion_failed { "assertion" } else { "return_relation_consistency" },"call_index":index,"source":if assertion_failed { &plan.assertion } else { &step.call },"conditions":if assertion_failed { &plan.target } else { &step.returns }});
                break;
            }
            p.extend(step.returns.iter().cloned());
        }
        Ok(
            json!({"status":status,"call_check":first_call,"assertion_check":final_assertion,"steps":checks,"target":plan.assertion,"call":plan.call,"callee":plan.callee,
            "target_kind":if plan.has_assertion { "assertion_and_call_preconditions" } else { "call_preconditions" },"failed_obligation":failed_obligation,
            "declared_target_count":plan.target.len()+plan.steps.iter().map(|s|s.requires.len()).sum::<usize>(),
            "entry_and_bindings":plan.p,"call_requires":plan.requires,"return_relation":plan.returns,"assertion":plan.target,
            "approximation":"Exact explicit scalar contracts and immutable call sequence within accepted VIR shape",
            "interpretation":"A satisfying assignment demonstrates a contract-allowed state; implementation behavior and caller reachability require separate evidence"}),
        )
    };
    let mut result = match work() {
        Ok(v) => v,
        Err(e) => json!({"status":"execution_error","reason":e.to_string()}),
    };
    let (finding, strength) = match result["status"].as_str().unwrap_or("unknown") {
        "precondition_unsat" => ("inconsistent_entry", "unsat_core"),
        "relation_unsat" => ("inconsistent_return_relation", "unsat_core"),
        "call_precondition_not_entailed" => {
            ("call_precondition_not_entailed", "scalar_countermodel")
        }
        "contract_allows_violation" => {
            ("contract_gap_relative_to_assertion", "scalar_countermodel")
        }
        "contract_sufficient" => ("explicit_contract_sufficient", "unsat_core"),
        "unsupported" => ("unsupported", "no_semantic_conclusion"),
        "execution_error" => ("solver_failure", "no_semantic_conclusion"),
        _ => ("undetermined", "no_semantic_conclusion"),
    };
    result["assessment"] = json!({"finding":finding,"strength":strength,
        "defect_confirmed":false,"qualification":"Contract entailment for extracted call preconditions and any final assertion. Intended behavior, implementation correctness, source-to-VIR correspondence and reachability require separate evidence."});
    result["inputs"] = inputs.clone();
    let mut sources = BTreeMap::new();
    a::source_hashes(&result, &mut sources)?;
    for (path, digest) in &sources {
        if hash(&fs::read(path)?)? != *digest {
            return Err("source changed during queries".into());
        }
    }
    result["source_inputs"] = json!(sources);
    write_json(&out.join("report.json"), &result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement(text: &str) -> Node {
        let functions = vir::functions(&format!("(Function :body {text})")).unwrap();
        functions[0].field("body").unwrap().clone()
    }

    #[test]
    fn discarded_call_preserves_its_source_annotation() {
        let s = statement("(Stmt Expr (@@ \"case.rs:1:1: 1:2 (#0)\" (> Call :args ())))");
        let (binding, call) = call_statement(&s).unwrap();
        assert!(binding.is_none());
        assert_eq!(call.list().unwrap()[0].atom(), Some("@@"));
        assert_eq!(a::head(call), Some("Call"));
    }

    #[test]
    fn assumptions_and_unanalysed_statements_cannot_be_dropped() {
        for text in [
            "(Stmt Expr (> AssertAssume :is_assume true :expr (> Const (Constant Bool false))))",
            "(Stmt Expr (> If (> Const (Constant Bool true)) (> Call) (> Call)))",
            "(Stmt Expr (> Block ((Stmt Expr (> Call))) None))",
        ] {
            assert!(call_statement(&statement(text)).is_err());
        }
    }

    #[test]
    #[ignore = "requires OBLIGATION_CONTROL_VIR/ROOT/Z3 from the real Verus scalar controls"]
    fn real_call_preconditions_and_assertions_keep_separate_sources() {
        let input = std::path::PathBuf::from(std::env::var_os("OBLIGATION_CONTROL_VIR").unwrap());
        let root = std::path::PathBuf::from(std::env::var_os("OBLIGATION_CONTROL_ROOT").unwrap());
        let z3 = std::path::PathBuf::from(std::env::var_os("OBLIGATION_CONTROL_Z3").unwrap());
        let out =
            std::env::temp_dir().join(format!("spec-obligation-controls-{}", std::process::id()));
        fs::create_dir(&out).unwrap();
        for (function, expected, kind) in [
            (
                "good_no_assertion",
                "contract_sufficient",
                "call_preconditions",
            ),
            (
                "weak_no_assertion",
                "call_precondition_not_entailed",
                "call_preconditions",
            ),
            (
                "good_assertion",
                "contract_sufficient",
                "assertion_and_call_preconditions",
            ),
            (
                "bad_assertion",
                "contract_allows_violation",
                "assertion_and_call_preconditions",
            ),
        ] {
            let case = out.join(function);
            run(
                &input,
                &format!("obligation_controls::{function}"),
                &root,
                &z3,
                &case,
            )
            .unwrap();
            let report: Value =
                serde_json::from_slice(&fs::read(case.join("report.json")).unwrap()).unwrap();
            assert_eq!(report["status"], expected, "{report}");
            assert_eq!(report["target_kind"], kind);
            assert_eq!(report["declared_target_count"], 1);
            if function.ends_with("no_assertion") {
                assert!(report["assertion_check"].is_null());
                assert!(report["assertion"].as_array().unwrap().is_empty());
                assert_eq!(
                    report["steps"][1]["requires"][0][2]["premise_source"],
                    "callee_precondition"
                );
            }
            if function == "weak_no_assertion" {
                assert_eq!(report["failed_obligation"]["kind"], "call_precondition");
                assert_eq!(report["failed_obligation"]["call_index"], 1);
            }
            if function == "good_no_assertion" {
                assert!(
                    report["steps"][1]["entry_and_bindings"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| row[2]["premise_source"] == "prior_call_postcondition"
                            && row[2]["call_index"] == 0)
                );
            }
            if function == "bad_assertion" {
                assert_eq!(report["failed_obligation"]["kind"], "assertion");
            }
        }
    }
}
