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
