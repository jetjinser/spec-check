#![no_std]
use vstd::cell::{pcell::*, CellId};
use vstd::prelude::*;

verus! {

// ============================================================================
// M2 spike: tracked resource modeling for MutexGuard::new (vostd
// sync/mutex.rs:217-228 stub, requires perm.id() == mutex.cell_id()).
//
// Route (b) probes: flat quantifier probes over the REAL vstd PointsTo type.
// Route (c) probes: construction probes over a faithful two-module mint model.
// Predictions are pre-registered in docs/M2-tracked-modeling.md.
// ============================================================================

// ---------------------------------------------------------------------------
// Route (b): flat forall/exists probes at real tracked semantics.
// ---------------------------------------------------------------------------

pub open spec fn pre_guard_new_b(cell_id: CellId, perm: PointsTo<u64>) -> bool {
    perm.id() == cell_id
}

// b1: vacuity probe. PREDICTED FAIL: PCell::new's ensures gives a matched
// (cell, perm) pair, refuting the forall.
proof fn b1_guard_new_flat_vacuous_should_fail()
    ensures
        forall|cell_id: CellId, perm: PointsTo<u64>|
            !pre_guard_new_b(cell_id, perm),
{
}

// b2: liveness probe, constructive witness by minting.
// PREDICTED VERIFIED -> ConfirmedLive (intrinsically satisfiable).
// Runs as an exec fn: verus rejects exec-mode calls (PCell::new) inside
// proof fn bodies ("cannot call function ... with mode exec"), so the mint
// must happen in exec code; the exists ensures is proved from the minted
// terms via the proof block.
pub fn b2_guard_new_flat_live()
    ensures
        exists|cell_id: CellId, perm: PointsTo<u64>|
            pre_guard_new_b(cell_id, perm),
{
    let (cell, Tracked(perm)) = PCell::new(5u64);
    proof {
        assert(pre_guard_new_b(cell.id(), perm));
    }
}

// ---------------------------------------------------------------------------
// Route (c): faithful minimal model of vostd Mutex mint/entombment.
//
// Fields are PRIVATE to this module (mirrors vostd field privacy): code
// outside `model` cannot touch the entombed perm. `mutex_new` mints the
// PointsTo via PCell::new INSIDE and has no ghost ensures (faithful to the
// frozen source, where Mutex::new carries no #[verus_spec] contract).
// ---------------------------------------------------------------------------

mod model {
    use super::*;

    pub struct MutexModel {
        cell: PCell<u64>,
        perm: Tracked<PointsTo<u64>>,
    }

    pub struct GuardModel {
        pub perm: Tracked<PointsTo<u64>>,
    }

    // pub closed (not open): the body touches private fields; callers outside
    // the module may call it but cannot unfold it (same choice as vostd's
    // Mutex::cell_id, sync/mutex.rs:43-45).
    pub closed spec fn cell_id(m: &MutexModel) -> CellId {
        m.cell.id()
    }

    pub fn mutex_new(val: u64) -> (m: MutexModel) {
        let (cell, Tracked(perm)) = PCell::new(val);
        MutexModel { cell, perm: Tracked(perm) }
    }

    // The stub under analysis: vostd MutexGuard::new external_body face,
    // with-clause tracked argument, requires perm.id() == mutex.cell_id().
    #[verifier::external_body]
    pub fn guard_new(m: &MutexModel, Tracked(perm): Tracked<PointsTo<u64>>) -> (g: GuardModel)
        requires
            perm.id() == cell_id(m),
        ensures
            g.perm@.id() == cell_id(m),
    {
        loop {}
    }
}

// ---------------------------------------------------------------------------
// c1: vacuity probe (construction criterion). An external caller attempts the
// ONLY fabrication channel available outside `model`: minting a fresh pair.
// PREDICTED FAIL: pt2.id() == model::cell_id(m) is unprovable (no ghost facts
// escape mutex_new; the entombed perm is structurally inaccessible).
// Expected diagnostic: "failed this precondition" on guard_new's requires.
// ---------------------------------------------------------------------------
pub fn c1_external_caller_guard_attempt(m: &model::MutexModel) -> (g: model::GuardModel) {
    let (_cell2, Tracked(pt2)) = PCell::new(7u64);
    model::guard_new(m, Tracked(pt2))
}

// ---------------------------------------------------------------------------
// c2: live control (holder face). A caller that HOLDS a matching perm
// satisfies the stub contract (mirrors vostd's internal try_lock caller,
// which extracts the perm from the lock invariant). PREDICTED VERIFIED.
// ---------------------------------------------------------------------------
pub fn c2_holder_face_guard_call(
    m: &model::MutexModel,
    Tracked(perm): Tracked<PointsTo<u64>>,
) -> (g: model::GuardModel)
    requires
        perm.id() == model::cell_id(m),
    ensures
        g.perm@.id() == model::cell_id(m),
{
    model::guard_new(m, Tracked(perm))
}

// ---------------------------------------------------------------------------
// c3: negative control for the vacuity classifier. PCell::read has the SAME
// tracked-requires shape (self.id() == perm.id()) but IS externally
// constructible: the caller mints its own pair. PREDICTED VERIFIED
// -> ConfirmedLive (no false positive on constructible tracked contracts).
// ---------------------------------------------------------------------------
pub fn c3_pcell_read_external_caller(val: u64) -> (r: u64)
    ensures
        r == val,
{
    let (cell, Tracked(perm)) = PCell::new(val);
    cell.read(Tracked(&perm))
}

// ---------------------------------------------------------------------------
// s1 (soundness probe): moved out of this crate — see s1-assume-new/ .
// A compile-level verus rejection cannot coexist with per-probe verification
// results in one crate. Its archived run records the actual rejection form.
// ---------------------------------------------------------------------------

} // verus!
