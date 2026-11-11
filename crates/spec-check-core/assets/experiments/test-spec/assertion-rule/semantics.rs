use vstd::prelude::*;

verus! {

pub ghost struct Eval<S> {
    pub state: S,
    pub value: Outcome,
    pub effects: Seq<int>,
}

pub ghost enum Outcome {
    Value(usize),
    Stop(int),
    Pending,
}

pub ghost enum Event {
    Evaluate(nat, bool, nat, Outcome, Seq<int>),
    Compare(nat, usize, usize),
}

pub ghost enum Halt {
    Next,
    OperandStop(nat, bool, int),
    Pending(nat, bool),
    // The observation stops at entry to the assertion-failure routine.
    AssertionFailure(nat),
}

pub ghost struct Run<S> {
    pub state: S,
    pub trace: Seq<Event>,
    pub halt: Halt,
    // false denotes core::assert_failed, true denotes the generated helper.
    pub failure_callee: Option<bool>,
}

pub open spec fn observe<S>(r: Run<S>) -> (S, Seq<Event>, Halt) {
    (r.state, r.trace, r.halt)
}

pub open spec fn stopped(i: nat, left: bool, value: Outcome) -> Halt {
    match value {
        Outcome::Stop(reason) => Halt::OperandStop(i, left, reason),
        Outcome::Pending => Halt::Pending(i, left),
        Outcome::Value(_) => Halt::Next,
    }
}

pub open spec fn event<S>(i: nat, left: bool, id: nat, r: Eval<S>) -> Event {
    Event::Evaluate(i, left, id, r.value, r.effects)
}

// The source macro first evaluates the borrowed tuple, then tests primitive equality.
pub open spec fn pair<S>(eval: spec_fn(nat, S) -> Eval<S>, i: nat, ids: (nat, nat), s: S)
    -> (Run<S>, Option<(usize, usize)>)
{
    let l = eval(ids.0, s);
    let t = seq![event(i, true, ids.0, l)];
    match l.value {
        Outcome::Stop(_) | Outcome::Pending => (Run { state: l.state, trace: t, halt: stopped(i, true, l.value), failure_callee: None }, None),
        Outcome::Value(a) => {
            let r = eval(ids.1, l.state);
            let t = t.push(event(i, false, ids.1, r));
            match r.value {
                Outcome::Stop(_) | Outcome::Pending => (Run { state: r.state, trace: t, halt: stopped(i, false, r.value), failure_callee: None }, None),
                Outcome::Value(b) => (Run { state: r.state, trace: t, halt: Halt::Next, failure_callee: None }, Some((a, b))),
            }
        },
    }
}

pub open spec fn source<S>(eval: spec_fn(nat, S) -> Eval<S>, i: nat, ids: (nat, nat), s: S) -> Run<S> {
    let (p, values) = pair(eval, i, ids, s);
    match values {
        None => p,
        Some((a, b)) => Run { state: p.state, trace: p.trace.push(Event::Compare(i, a, b)),
            halt: if a == b { Halt::Next } else { Halt::AssertionFailure(i) },
            failure_callee: if a == b { None } else { Some(false) } },
    }
}

// The generated match binds both operands and calls its diverging helper on inequality.
pub open spec fn lowered<S>(eval: spec_fn(nat, S) -> Eval<S>, i: nat, ids: (nat, nat), s: S) -> Run<S> {
    let l = eval(ids.0, s);
    let t = seq![event(i, true, ids.0, l)];
    match l.value {
        Outcome::Stop(_) | Outcome::Pending => Run { state: l.state, trace: t, halt: stopped(i, true, l.value), failure_callee: None },
        Outcome::Value(a) => {
            let r = eval(ids.1, l.state);
            let t = t.push(event(i, false, ids.1, r));
            match r.value {
                Outcome::Stop(_) | Outcome::Pending => Run { state: r.state, trace: t, halt: stopped(i, false, r.value), failure_callee: None },
                Outcome::Value(b) => {
                    let t = t.push(Event::Compare(i, a, b));
                    if !(a == b) {
                        Run { state: r.state, trace: t, halt: Halt::AssertionFailure(i), failure_callee: Some(true) }
                    } else {
                        Run { state: r.state, trace: t, halt: Halt::Next, failure_callee: None }
                    }
                },
            }
        },
    }
}

pub proof fn one<S>(eval: spec_fn(nat, S) -> Eval<S>, i: nat, ids: (nat, nat), s: S)
    ensures
        observe(source(eval, i, ids, s)) == observe(lowered(eval, i, ids, s)),
        lowered(eval, i, ids, s).halt is Next ==> lowered(eval, i, ids, s).failure_callee is None,
{}

pub open spec fn execute<S>(target: bool, eval: spec_fn(nat, S) -> Eval<S>,
    program: Seq<(nat, nat)>, i: nat, s: S) -> Run<S>
    decreases program.len(),
{
    if program.len() == 0 {
        Run { state: s, trace: Seq::empty(), halt: Halt::Next, failure_callee: None }
    } else {
        let head = if target { lowered(eval, i, program[0], s) } else { source(eval, i, program[0], s) };
        if head.halt is Next {
            let tail = execute(target, eval, program.drop_first(), i + 1, head.state);
            Run { state: tail.state, trace: head.trace + tail.trace,
                halt: tail.halt, failure_callee: tail.failure_callee }
        } else { head }
    }
}

pub proof fn sequence<S>(eval: spec_fn(nat, S) -> Eval<S>, program: Seq<(nat, nat)>, i: nat, s: S)
    ensures observe(execute(false, eval, program, i, s)) == observe(execute(true, eval, program, i, s)),
    decreases program.len(),
{
    if program.len() > 0 {
        one(eval, i, program[0], s);
        let head = source(eval, i, program[0], s);
        if head.halt is Next {
            sequence(eval, program.drop_first(), i + 1, head.state);
        }
    }
}

pub open spec fn statement(e: Event) -> nat {
    match e { Event::Evaluate(i, _, _, _, _) => i, Event::Compare(i, _, _) => i }
}

pub proof fn evaluation_order<S>(eval: spec_fn(nat, S) -> Eval<S>, i: nat, ids: (nat, nat), s: S)
    ensures
        ({ let r = source(eval, i, ids, s);
            &&& 1 <= r.trace.len() <= 3
            &&& r.trace[0] == event(i, true, ids.0, eval(ids.0, s))
            &&& ({ let l = eval(ids.0, s);
                match l.value {
                    Outcome::Value(a) => {
                        let right = eval(ids.1, l.state);
                        let prefix = seq![event(i, true, ids.0, l), event(i, false, ids.1, right)];
                        match right.value {
                            Outcome::Value(b) => r.trace == prefix.push(Event::Compare(i, a, b))
                                && r.halt == if a == b { Halt::Next } else { Halt::AssertionFailure(i) },
                            _ => r.trace == prefix && r.halt == stopped(i, false, right.value),
                        }
                    },
                    _ => r.trace == seq![event(i, true, ids.0, l)] && r.halt == stopped(i, true, l.value),
                }
            })
            &&& match r.halt {
                Halt::Next => true,
                Halt::OperandStop(k, _, _) | Halt::Pending(k, _) | Halt::AssertionFailure(k) => k == i,
            }
            &&& forall|j: int| 0 <= j < r.trace.len() ==> statement(#[trigger] r.trace[j]) == i
        }),
{}

pub proof fn first_stop<S>(target: bool, eval: spec_fn(nat, S) -> Eval<S>,
    program: Seq<(nat, nat)>, i: nat, s: S)
    ensures
        ({ let r = execute(target, eval, program, i, s);
            &&& match r.halt {
                Halt::Next => true,
                Halt::OperandStop(k, _, _) | Halt::Pending(k, _) | Halt::AssertionFailure(k) => i <= k < i + program.len(),
            }
            &&& forall|j: int| 0 <= j < r.trace.len() ==> match r.halt {
                Halt::Next => i <= statement(#[trigger] r.trace[j]) < i + program.len(),
                Halt::OperandStop(k, _, _) | Halt::Pending(k, _) | Halt::AssertionFailure(k) =>
                    i <= statement(#[trigger] r.trace[j]) <= k < i + program.len(),
            }
        }),
    decreases program.len(),
{
    if program.len() > 0 {
        evaluation_order(eval, i, program[0], s);
        one(eval, i, program[0], s);
        let head = if target { lowered(eval, i, program[0], s) } else { source(eval, i, program[0], s) };
        if head.halt is Next {
            first_stop(target, eval, program.drop_first(), i + 1, head.state);
            let tail = execute(target, eval, program.drop_first(), i + 1, head.state);
            assert forall|j: int| 0 <= j < (head.trace + tail.trace).len() implies
                match tail.halt {
                    Halt::Next => i <= statement(#[trigger] (head.trace + tail.trace)[j]) < i + program.len(),
                    Halt::OperandStop(k, _, _) | Halt::Pending(k, _) | Halt::AssertionFailure(k) =>
                        i <= statement(#[trigger] (head.trace + tail.trace)[j]) <= k < i + program.len(),
                } by {
                if j < head.trace.len() {
                    assert((head.trace + tail.trace)[j] == head.trace[j]);
                } else {
                    assert((head.trace + tail.trace)[j] == tail.trace[j - head.trace.len()]);
                }
            }
        }
    }
}

}
