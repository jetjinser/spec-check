# A2 final: trailing Range flush retained

Date: 2026-11-16. Branch `work/a2-interval`. This closes the attempt to remove
the trailing `TlbFlushOp::Range(start_va..end_va)` from `unmap`.

## Baseline

`/tmp/vostd-a2-interval` = pristine 43da079 + `combined-tlb-contracts.patch`.
Verification (clean target, `/tmp/run-witness-a2.sh`):
`1908 verified, 0 errors` (the count differs from the earlier 1500 run because
the current target/config verifies more modules; errors are the metric here).

## Attempts

1. **A2c removed-coverage** (`/tmp/vostd-a2c`): per-item ops + `dispatch_all`,
   no trailing Range; invariant "every `removed` mapping is covered by a pending
   op", plus bridge E (old mapping with passed start is removed or has a removed
   sub-mapping at the same start). Result **1498/2**:
   - `loop invariant not satisfied` at the `break` (E has not been extended
     across the cursor jump from `prev_va` to the `take_next` `None` cursor);
   - `postcondition not satisfied` (the final revocation clause).
   The body maintenance of E for Mapped/Stray verifies; the break is the gap.
2. **A2e boundary-parent bridge** (earlier): E reaches 1498/2 with the same
   break failure plus an empty-filter assertion. A huge entry starting before
   the processed VA can be split in an earlier iteration; its later sub-mapping
   is in `prev_mappings`, while the entry itself is not. A per-mapping E over
   the entry view cannot express this history.

## Why the break needs a processed-interval history

At `take_next == None`, the cursor may jump to `end_va`. E is known at the loop
head only for starts `< prev_va`. For an old mapping with start in
`[prev_va, cursor)`, the proof must show it was removed or has a removed
same-start sub-mapping. `take_next`'s `None` ensures only that the *current*
view has no mapping starting in `[prev_va, end_va)`; it does not expose the
split ancestry of old mappings. Closing the gap needs either:

- a new `take_next` ensure/filter fact about the split descendants of old
  mappings, or
- a processed-interval ghost history that records, per processed interval, the
  removed fragments and their split parents, maintained through Mapped, Stray
  and Break.

Both are larger proof surfaces than the value of deleting one redundant range
flush. The `E''` sketch (old mapping not removed ⇒ a descendant in
`adjusted_base` at the same start) still needs a lemma that every page of a
split huge mapping has a fragment at its start, plus the break jump; this was
not closed within the timebox.

## Final decision

Keep the trailing `Range(start_va..end_va)` op. It is one extra range flush per
`unmap`; per-item `_with` ops remain (frame lifetime), and `dispatch_all` is
faithful. The delivered `combined-tlb-contracts.patch` is unchanged
(1500/0 in the archived pristine run; W1/W2 + postcondition 1499/1).

This is a proof-engineering cost decision, not a new counterexample. The runtime
correction of F-029 (split returns `Err(Mapped)`, so the real `VmSpace::map`
flushes) is independent and recorded in `ostd-runtime-2026-11-16/`.

## Reproduction

```sh
cp -a /tmp/vostd-combined /tmp/vostd-a2-interval
sed 's#/tmp/vostd-witness-target#/tmp/vostd-a2-target#' /tmp/run-witness.sh > /tmp/run-witness-a2.sh
nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd /tmp/vostd-a2-interval && /tmp/run-witness-a2.sh'
# 1908 verified, 0 errors
```

A2c log: `/tmp/a2c-rerun.log` (`1498 verified, 2 errors`).
