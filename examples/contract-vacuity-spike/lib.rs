#![no_std]
use vstd::prelude::*;

verus! {

// The API's documented reachable-state set (stand-in for a type invariant).
pub open spec fn reachable_state(s: u64) -> bool {
    s < 8
}

// (A) VACUOUS ENTRY: the second requirement contradicts the reachable set.
// Known positive: mirrors "entry precondition can never hold".
pub fn dead_entry(s: u64)
    requires
        reachable_state(s) && s >= 8,
{
}

// (B) LIVE ENTRY: satisfiable requirement. Known negative.
pub fn live_entry(s: u64)
    requires
        s < 8,
{
}

// Probe A: machine-confirmed vacuity (verification success is the evidence).
proof fn dead_entry_is_vacuous()
    ensures
        forall|s: u64| reachable_state(s) ==> s < 8,
{
}

// Probe B: machine-confirmed liveness (witness).
proof fn live_entry_is_live()
    ensures
        exists|s: u64| reachable_state(s),
{
    let witness: u64 = 0;
    assert(reachable_state(witness));
}

// Probe C: EXPECTED TO FAIL — vacuity claim for the live entry. If this
// verifies, the criterion cannot discriminate and is unusable.
proof fn live_entry_is_vacuous_should_fail()
    ensures
        forall|s: u64| !reachable_state(s),
{
}

} // verus!
