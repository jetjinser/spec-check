# TLB patch — pre-registration (written before patch verification)

## Targeted clauses

**P1 (map, frame isolation).** Add to `VmSpace::map`:

```
old(cursor_owner)@.present() ==>
    forall|m: Mapping|
        old(cursor_owner)@.mappings.contains(m)
        && m.va_range.start <= old(self).pt_cursor.0.va < m.va_range.end
        ==> !final(tlb_model).mappings.contains(m)
```

Reading: if an old mapping covered the mapped VA, the final TLB may not still
hold any such mapping. The antecedent makes the clause vacuous on the
absent-entry path.

**P2 (unmap, revocation).** After the model fix, add to `VmSpace::unmap`:

```
forall|m: Mapping|
    old(cursor_owner)@.mappings.contains(m)
    && start_va <= m.va_range.start < end_va
    ==> !final(tlb_model).mappings.contains(m)
```

**Model fix (P2 prerequisite).** `TlbModel::dispatch_tlb_flush_spec` handles
`TlbFlushOp::Range(r)` by flushing only `r.start`. A range flush must remove
every cached mapping that intersects `r`.

## Predictions (before verification)

- P1: the real `map` still verifies with the clause after one supporting
  postcondition on `Cursor::map` (`Ok` implies no mapping was present at the
  current VA). W1 (no flush) fails the new postcondition.
- P2: without the `Range` model fix the clause is not provable; with it, the
  real `unmap` needs the per-fragment flush facts threaded through the loop
  invariant. Predicted outcome: P2 is harder than P1; if it does not verify
  within a bounded attempt, record `Candidate` with the exact failing
  obligation and keep W2 as the confirmed contract-gap witness.
- Property-client check after patching: W1/W2 must fail at the new
  postcondition; the real implementations must stay at 0 errors. A witness that
  still verifies after the patch falsifies the patch.

## Pass criteria

- Full `cargo-verus check -p ostd --target x86_64-unknown-none --lib`: 0 errors
  on the patched real code.
- W1/W2 with the patch applied: verification fails, and the failure span is the
  new postcondition.
- No change to the crate's public API, no axiom added.

## Boundary

The clauses observe model state, not hardware. A `Confirmed` patch means the
verifier now rejects the two wrong bodies; it does not prove real hardware
coherence beyond `TlbModel`'s assumptions.
