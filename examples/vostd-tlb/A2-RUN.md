# A2: dispatch_all TLB model + unmap revocation

Patch: `unmap-tlb-flush-dispatch-all.patch` (2 files, 146 added lines).

## What changed

1. `TlbModel::apply_op` / `apply_all`: a faithful fold over the pending
   sequence. `flush_range` removes every mapping intersecting the range.
2. `dispatch_tlb_flush_spec` now clears `pending` and applies **all** ops in
   order (previously it processed only `pending.last()`).
3. `unmap` keeps the per-item `issue_tlb_flush_with` calls (frame lifetime),
   adds one trailing `Range(start..end)` op, and carries a revocation
   postcondition: no old mapping starting in the range remains in the final
   TLB.
4. Proof: `apply_op` removes covered mappings, `apply_all` is monotone
   (subset), and a per-mapping induction shows any mapping covered by a pending
   op is absent after `apply_all`. The trailing Range is the witness for the
   postcondition.

## Verification

| run | result |
|---|---|
| patched crate | **1500 / 0** (1495 + 5 new proof/spec functions) |
| W2 (all flushes deleted) + postcondition | **1499 / 1**, postcondition failed |
| baseline | 1495 / 0 |

Static flush behavior:

| path | ops |
|---|---|
| each removed frame / stray subtree | one `issue_tlb_flush_with` (frame held until dispatch) |
| end of `unmap` | one trailing `Range(start..end)` |
| dispatch | applies every pending op (previously only the last) |

## Residual: trailing Range not yet removed

The goal for A2 included removing the trailing Range op. The current proof uses
that op as the witness that every old mapping in the range is covered. Removing
it requires a loop invariant relating the processed interval to the issued ops
(e.g., a ghost `covered` set and "every old mapping with start in
`[start, cursor_va)` is covered"), plus the corresponding arm assertions. That
proof was not completed here; the trailing range flush is redundant at runtime
because the per-item ops already cover the mappings, but it is what the current
proof consumes.

This is a proof-engineering residual, not a correctness one: the model is now
faithful (all pending ops applied), and the patch rejects W2.

## API / boundary

- No public API change; `dispatch_tlb_flush` remains `external_body` with a
  stronger model spec.
- Model-level evidence only; no hardware reproduction.
- Frozen rev 43da079.

## Later note (2026-11-16)

This run's boundary statements describe the verification-level run above. A
separate QEMU hardware/VM experiment now reproduces the stale-translation hazard
(`qemu-hw-test/`); it does not execute ostd and does not change this run's
result.
