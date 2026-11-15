# P2 patch run record (unmap revocation)

Patch: `unmap-tlb-flush.patch` (two files, 17 added lines).

## Results

| run | source | result |
|---|---|---|
| P2 | model `flush_range` + one trailing `Range` flush + postcondition | **1495 / 0** |
| W2 + clause | wrong body (no flush) + postcondition | **1494 / 1**, postcondition failed |

The postcondition:

```
forall|m: Mapping|
    old(cursor_owner)@.mappings.contains(m)
    && old(cursor_owner)@.cur_va <= m.va_range.start
    && m.va_range.start < old(cursor_owner)@.cur_va + len
    ==> !final(tlb_model).mappings.contains(m)
```

## What the patch actually fixes

The model has two independent weaknesses:

1. `dispatch_tlb_flush_spec` processed only `pending.last()`, while the real
   dispatcher processes the whole stack.
2. `TlbFlushOp::Range(r)` flushed only mappings covering `r.start`, not the
   whole range.

The patch fixes (2) with `flush_range`, and works around (1) by issuing one
`Range(start..end)` after the loop so that the last-pending operation covers
every mapping the loop removed. It is conservative: it adds a range flush that
the real implementation does not need, and it leaves earlier pending ops in the
model (harmless for `inv`).

## Upstream-preferred shape

Make `dispatch_tlb_flush_spec` fold over the entire pending sequence (clear
`pending`, apply each op in order). Then the existing per-mapping flushes should
support the postcondition directly, with no trailing range flush. That proof
needs an induction lemma ("an op covering `m` in `pending` removes `m` from
`apply_all`") threaded through the unmap loop; it is the follow-up, not part of
this minimal candidate.

Keeping the per-item `issue_tlb_flush_with` calls is required for frame
lifetime: `_with` holds the frame until the flush is dispatched.

## Residual

- Candidate patch, not an upstream PR; changes runtime work (one extra range
  flush) and model semantics.
- No runtime counterexample; W2 is verifier-accepted wrong code.
- Verified against the frozen rev only.
