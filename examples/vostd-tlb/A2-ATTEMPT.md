# A2 attempt: removing the trailing Range flush

Status: **not completed**; exact failing obligations recorded. The verified
`dispatch_all` + trailing Range version remains the candidate.

## Change attempted

- Removed the trailing `issue_tlb_flush(Range(start_va_exec..end_va))`.
- Added the loop invariant: every old mapping with start in
  `[start_va, cursor_va)` is covered by some pending op
  (`TlbModel::op_covers`).
- Final proof consumes that invariant via `lemma_apply_all_removed`.

## Result

Full crate: 1499 verified / 1 error. Errors:

1. `invariant not satisfied at end of loop body` (vm_space.rs:1090)
2. `loop invariant not satisfied` at the loop exit (vm_space.rs:1164)

## Missing obligations

- **Mapped arm**: after `take_next`, old mappings in `[va, new_cursor)` are
  covered by `Address(va)` only if they contain `va`. This needs the
  split-refinement fact (the old parent of the returned sub-mapping) plus
  old-view non-overlap to exclude a second old mapping starting inside the
  parent.
- **Stray arm**: the interval upper bound must be tied to `va + len_frag`
  (the subtree range), not just the new cursor; `take_next` does not expose the
  exact final cursor in its ensures.
- **Break/None**: preserving coverage for the interval before the break needs
  `take_next`'s `None` filter fact plus the pending-prefix argument.

These are proof-engineering tasks, not new counterexamples; the trailing Range
op is a sufficient witness for the postcondition. Fallback kept:
`unmap-tlb-flush-dispatch-all.patch` (1500/0; W2 + postcondition 1499/1).

## Pristine combined repeat (2026-11-15)

Repeated on the pristine combined tree (`combined-tlb-contracts.patch` base),
removing the trailing Range and adding the same invariant: **1498 verified /
2 errors**.

- `invariant not satisfied at end of loop body` (vm_space.rs:1170)
- `postcondition not satisfied` at the revocation clause (vm_space.rs:1095)

Same missing obligations as above; the ktest/ktest-macro difference is not the
cause. The verified `dispatch_all` + trailing Range version remains the
delivered patch.

## A2c progress: removed-coverage invariant verified

Route: keep per-item ops + dispatch_all, remove the trailing Range, and track
`forall m in removed, exists pending op covering m` (no `covered_end` interval).

- Added `pending_before = tlb_model.pending` before the match and proved
  `tlb_model.pending == pending_before.push(issued_op)`.
- Added the loop invariant: every `removed` mapping is covered by some pending
  op.
- Maintenance: old removed mappings keep their witness (push prefix); Mapped
  adds `mm` with `Address(va)`; Stray adds `subtree` with `Range(va..va+len)`.
- Result: **1499 / 1** — the invariant and both arm proofs verify. The only
  remaining failure is the revocation postcondition at the end:

  `forall m in old view, start <= m.start < end ==> !final(tlb).mappings.contains(m)`

Remaining gap: bridge an old mapping in the range to either `removed` or a
removed sub-mapping of a boundary split; then use `lemma_apply_all_removed`.
The split-parent case (old mapping starts in range and is split at `end`) is
the missing bridge. No trailing Range is needed for the removed part.

## A2d plan: the missing bridge invariant

To close the final revocation postcondition without the trailing Range, add

```
// E: every old mapping whose start has been passed is either removed, or a
// removed sub-mapping shares its start.
forall|m: Mapping| old(cursor_owner)@.mappings.contains(m)
    && start_va <= m.va_range.start && m.va_range.start < cursor_owner@.cur_va
    ==> removed.contains(m)
        || exists|sm: Mapping| removed.contains(sm)
            && sm.va_range.start == m.va_range.start
            && m.va_range.start <= sm.va_range.start
            && sm.va_range.end <= m.va_range.end
```

Maintenance:
- Mapped: take_next's filter gives `m.start >= va`; take_next's split refinement
  and old-view non-overlap give `mm` (the removed sub-mapping at `va`) as the
  witness, with `mm.start == m.start == va` for every old `m` whose start is in
  the processed slot.
- Stray: the subtree filter puts `m` itself into `removed`.
- Break: `take_next`'s `None` empty filter covers the tail; E is preserved with
  `cursor == end_va`.

Final: if `m ∈ removed`, D gives an op covering `m`; otherwise D gives an op
covering `sm`, and the same op covers `m` because `sm ⊆ m`. Then
`lemma_apply_all_removed` closes the postcondition.
