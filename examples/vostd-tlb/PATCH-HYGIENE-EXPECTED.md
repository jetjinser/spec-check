# Patch hygiene pre-registration (pristine 43da079)

Predictions before applying the combined series to a pristine (non-ktest) copy:

1. The A1 edits to `cursor/mod.rs` and `kvirt_area.rs` apply by content anchors;
   the ktest-only differences are elsewhere in those files.
2. The A2 edits to `vm_space.rs`/`tlb.rs` apply cleanly; `vm_space.rs` and
   `tlb.rs` are byte-identical between the ktest copy and pristine.
3. Combined crate verifies with 0 errors (function count may exceed 1495).
4. W1 (map flush removed) and W2 (unmap flushes removed) each produce exactly
   one postcondition failure.
5. If any anchor is missing or a proof fails only on pristine, record the exact
   obligation and do not claim a clean combined patch.
