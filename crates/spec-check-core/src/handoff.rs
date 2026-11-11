//! Affine permission transfer through straight-line typed VIR bodies.
use crate::{Result, permission_guidance::Carrier, vir::Node};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn part(n: &Node, i: usize) -> Option<&Node> {
    n.unwrap().list()?.get(i)
}
fn tag(n: &Node) -> Option<&str> {
    (part(n, 0)?.atom() == Some(">"))
        .then(|| part(n, 1)?.atom())
        .flatten()
}
fn key(n: &Node) -> Result<String> {
    let xs = n.unwrap().list().ok_or("missing permission place")?;
    match (
        xs.first().and_then(Node::atom),
        xs.get(1).and_then(Node::atom),
    ) {
        (Some("Place"), Some("Local")) => Ok(crate::contracts::render(&xs[2])),
        (Some("Place"), Some("Field")) => Ok(format!(
            "{}.{}.{}",
            key(&xs[3])?,
            crate::contracts::render(xs[2].field("datatype").ok_or("missing field type")?),
            xs[2]
                .field("field")
                .and_then(Node::atom)
                .ok_or("missing field identity")?
        )),
        _ => Err("unsupported owned place".into()),
    }
}
fn live(n: &Node, env: &BTreeMap<String, bool>) -> bool {
    key(n).ok().is_some_and(|k| env.get(&k) == Some(&true))
        || n.list().is_some_and(|xs| xs.iter().any(|n| live(n, env)))
}
fn identity(f: &Node) -> Result<&str> {
    f.field("name")
        .and_then(|n| n.field("path"))
        .and_then(Node::atom)
        .ok_or_else(|| "missing permission function identity".into())
}
pub(crate) fn trace(f: &Node, carrier: &Carrier, functions: &[Node]) -> Result<Value> {
    let receiver = f
        .field("params")
        .and_then(Node::list)
        .and_then(|ps| ps.first())
        .ok_or("missing owned receiver")?;
    if receiver
        .field("typ")
        .and_then(|t| crate::permission_guidance::direct_carrier(t, std::slice::from_ref(carrier)))
        != Some(carrier.path.as_str())
    {
        return Err("permission handoff requires an owned carrier receiver".into());
    }
    let seed = format!(
        "{}.{}.{}",
        crate::contracts::render(receiver.field("name").ok_or("missing receiver name")?),
        crate::contracts::render(&Node::List(vec![
            Node::Atom("Dt".into()),
            Node::Atom("Path".into()),
            Node::Atom(carrier.path.clone())
        ])),
        carrier.field
    );
    run(f, seed, functions, &[])
}
pub(crate) fn functions(trace: &Value, names: &mut std::collections::BTreeSet<String>) {
    if let Some(name) = trace["function"].as_str() {
        names.insert(name.into());
    }
    if let Some(events) = trace["events"].as_array() {
        for event in events {
            if let Some(nested) = event.get("transfer") {
                functions(nested, names);
            }
        }
    }
}
fn run(f: &Node, seed: String, functions: &[Node], stack: &[String]) -> Result<Value> {
    let name = identity(f)?;
    if stack.len() >= 8 || stack.iter().any(|p| p == name) {
        return Err("recursive permission handoff unsupported".into());
    }
    if f.field("mode").and_then(Node::atom) != Some("Exec")
        || f.field("attrs")
            .and_then(|a| a.field("is_external_body"))
            .and_then(Node::atom)
            != Some("false")
    {
        return Err("permission handoff requires a native executable body".into());
    }
    let mut stack = stack.to_vec();
    stack.push(name.into());
    let mut state = State {
        env: BTreeMap::from([(seed, true)]),
        events: Vec::new(),
        functions,
        stack: &stack,
    };
    let returned = state.eval(f.field("body").ok_or("missing permission body")?)?;
    if state.env.values().any(|v| *v) || (!returned && state.events.is_empty()) {
        return Err("permission has no accounted return or invariant sink".into());
    }
    Ok(json!({"function":name,"returned_permission":returned,"events":state.events}))
}
struct State<'a> {
    env: BTreeMap<String, bool>,
    events: Vec<Value>,
    functions: &'a [Node],
    stack: &'a [String],
}
impl State<'_> {
    fn eval(&mut self, n: &Node) -> Result<bool> {
        match tag(n) {
            Some("Block") => {
                for stmt in part(n, 2)
                    .and_then(Node::list)
                    .ok_or("missing block statements")?
                {
                    match part(stmt, 1).and_then(Node::atom) {
                        Some("Expr") => {
                            if self.eval(part(stmt, 2).ok_or("missing statement expression")?)? {
                                return Err("discarded permission expression".into());
                            }
                        }
                        Some("Decl") => {
                            if let Some(init) = stmt.field("init")
                                && self.eval(init)?
                            {
                                return Err("permission declaration pattern unsupported".into());
                            }
                        }
                        _ => return Err("unsupported permission statement".into()),
                    }
                }
                self.eval(part(n, 3).ok_or("missing block tail")?)
            }
            Some("ReadPlace") => {
                let place = part(n, 2).ok_or("missing read place")?;
                if let Ok(k) = key(place)
                    && self.env.get(&k) == Some(&true)
                {
                    let kind = part(n, 3)
                        .and_then(|n| n.field("preliminary_kind"))
                        .ok_or("missing permission read kind")?;
                    if part(kind, 1).and_then(Node::atom) != Some("Move") {
                        return Err("permission borrow requires a separate transfer rule".into());
                    }
                    self.env.insert(k, false);
                    return Ok(true);
                }
                if live(n, &self.env) {
                    return Err("unsupported permission projection".into());
                }
                Ok(false)
            }
            Some("Ghost") => self.eval(n.field("expr").ok_or("missing ghost expression")?),
            Some("Ctor") => {
                let dt = part(n, 2).ok_or("missing permission constructor type")?;
                if part(dt, 1).and_then(Node::atom) != Some("Tuple")
                    && part(dt, 2).and_then(Node::atom) != Some("core::option::Option")
                {
                    return Err("permission container constructor unsupported".into());
                }
                let mut count = 0;
                for field in part(n, 4)
                    .and_then(Node::list)
                    .ok_or("missing constructor fields")?
                {
                    count +=
                        usize::from(self.eval(part(field, 2).ok_or("missing constructor value")?)?);
                }
                if count > 1 {
                    return Err("duplicated permission container".into());
                }
                Ok(count == 1)
            }
            Some("Assign") => {
                let value = self.eval(n.field("rhs").ok_or("missing assignment value")?)?;
                let k = match key(n.field("place").ok_or("missing assignment place")?) {
                    Ok(k) => k,
                    Err(e) => {
                        if value {
                            return Err(e);
                        }
                        return Ok(false);
                    }
                };
                if self.env.get(&k) == Some(&true) {
                    return Err("overwritten permission".into());
                }
                self.env.insert(k, value);
                Ok(false)
            }
            Some("OpenInvariant") => {
                if part(n, 5).and_then(|n| part(n, 1)).and_then(Node::atom) != Some("Atomic") {
                    return Err("non-atomic permission sink unsupported".into());
                }
                let binder = part(n, 3)
                    .and_then(|n| part(n, 1))
                    .ok_or("missing invariant binding")?;
                let k = crate::contracts::render(binder);
                if self.env.get(&k) == Some(&true) {
                    return Err("invariant binding shadows a live permission".into());
                }
                self.env.insert(k.clone(), false);
                if self.eval(part(n, 4).ok_or("missing invariant body")?)? {
                    return Err("permission escapes invariant expression".into());
                }
                if self.env.remove(&k) == Some(true) {
                    self.events
                        .push(json!({"invariant_sink":crate::contracts::render(binder)}));
                }
                Ok(false)
            }
            Some("Call") => {
                let mut argument = None;
                for (i, arg) in n
                    .field("args")
                    .and_then(Node::list)
                    .ok_or("missing handoff arguments")?
                    .iter()
                    .enumerate()
                {
                    if self.eval(arg)? && argument.replace(i).is_some() {
                        return Err("duplicate permission arguments".into());
                    }
                }
                let Some(i) = argument else {
                    return Ok(false);
                };
                let target = n.field("target").ok_or("missing handoff call")?;
                if part(target, 2)
                    .and_then(|n| part(n, 1))
                    .and_then(Node::atom)
                    != Some("Static")
                {
                    return Err("dynamic permission handoff unsupported".into());
                }
                let name = part(target, 3)
                    .and_then(|n| n.field("path"))
                    .and_then(Node::atom)
                    .ok_or("missing handoff target")?;
                let f = self
                    .functions
                    .iter()
                    .find(|f| identity(f).ok() == Some(name))
                    .ok_or("missing handoff callee body")?;
                let param = f
                    .field("params")
                    .and_then(Node::list)
                    .and_then(|ps| ps.get(i))
                    .ok_or("missing handoff parameter")?;
                let seed = crate::contracts::render(
                    param
                        .field("name")
                        .ok_or("missing handoff parameter name")?,
                );
                let result = run(f, seed, self.functions, self.stack)?;
                let returned = result["returned_permission"] == true;
                self.events
                    .push(json!({"call":name,"argument":i,"transfer":result}));
                Ok(returned)
            }
            Some("If" | "Match" | "Loop" | "Return") => {
                Err("conditional permission handoff unsupported".into())
            }
            _ => {
                if live(n, &self.env) {
                    return Err("unsupported permission expression".into());
                }
                Ok(false)
            }
        }
    }
}
