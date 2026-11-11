#![no_std]
use vstd::cell::{pcell::*, CellId};
use vstd::prelude::*;

verus! {

// s1: soundness probe (route c boundary). Attempts to fabricate a
// PointsTo permission via Tracked::assume_new and call a stub whose
// requires is perm.id() == cell_id(m).
//
// EXPECTED FAIL. Pre-registered possibilities:
//   (A) compile/translation rejection of the assume_new call itself;
//   (B) verification failure on the id-match requires (assume_new has no
//       ensures, so the fabricated term's id is unconstrained).
// Either outcome preserves unforgeability. The first main-crate run
// already recorded outcome (A) verbatim:
//   error: `verus_builtin::impl&%6::assume_new` is not supported
// This crate exists to archive that boundary as a replayable run.

mod model {
    use super::*;

    pub struct MutexModel {
        cell: PCell<u64>,
        perm: Tracked<PointsTo<u64>>,
    }

    pub struct GuardModel {
        pub perm: Tracked<PointsTo<u64>>,
    }

    pub closed spec fn cell_id(m: &MutexModel) -> CellId {
        m.cell.id()
    }

    pub fn mutex_new(val: u64) -> (m: MutexModel) {
        let (cell, Tracked(perm)) = PCell::new(val);
        MutexModel { cell, perm: Tracked(perm) }
    }

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

pub fn s1_assume_new_fabrication_attempt(m: &model::MutexModel) -> (g: model::GuardModel) {
    let fabricated = Tracked::<PointsTo<u64>>::assume_new();
    model::guard_new(m, fabricated)
}

} // verus!
