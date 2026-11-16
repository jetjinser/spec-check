# P1 patch run record

> Historical verification-level record. The 2026-11-16 runtime correction shows
> that ostd v0.16.0 split returns `Err(Mapped)` and `VmSpace::map` flushes;
> A5/A6 is **not** a confirmed real-body defect. F-029 is a contract gap. See
> `FINAL-ASSESSMENT.md` §1 and `ostd-runtime-2026-11-16/`.

Patch: `map-tlb-flush.patch` (27 lines, one file).
Baseline: 1495 verified / 0 errors. All runs below use the same command
(`run-check.sh`).

## Results

| run | source | result |
|---|---|---|
| A4 | real code + postcondition + flush on the absent/split return path | **1495 / 0** |
| A5 | real code + postcondition, no code change | **1494 / 1**, postcondition failed at the `return` exit |
| W1 + clause | wrong body (no flush at all) + postcondition | **1494 / 1**, postcondition failed |

Discriminating run A6: adding an entry precondition that every TLB entry at
the VA is backed by the old cursor view, plus the backing-preserved
postcondition, still fails at the same `return` (1494 / 1). At the
verification level, this excluded unbacked stale entries and was read as the
real body not flushing that path. The 2026-11-16 runtime correction supersedes
that reading.

A5 was read as the *real* `map` failing at the early return. That path is taken
when the slot is absent, including after `map_loop` splits a huge page. The A4
patch adds an address flush before that return and the postcondition then
verifies. Runtime ostd v0.16.0 later showed that a split returns `Err(Mapped)`
and takes the flush branch, so A5 is not a confirmed real-body defect.

W1 + clause shows the same postcondition rejects a body that never flushes.

## Postcondition

```
forall|m: Mapping|
    old(cursor_owner)@.mappings.contains(m)
    && m.va_range.start <= old(self).pt_cursor.0.va
    && old(self).pt_cursor.0.va < m.va_range.end
    ==> !final(tlb_model).mappings.contains(m)
```

No entry precondition is needed: A4 makes the two-state guarantee unconditional
by flushing the address on every early return. In the absent case this is a
redundant flush; in the split case it is required.

## Cost and design alternatives

A4 flushes on every absent-slot map, which is a hot path. The optimized design
is to have `map_loop` / `Cursor::map` report whether it split a huge page (or
return the split fragments) and flush only then. That changes an internal
return type and is deliberately not part of this minimal patch; the
conservative version is the one verified here.

## Residual

- A4 is a candidate patch, not an upstream PR. It changes runtime behavior
  (extra flush), so maintainer review is required.
- A5 is a historical verification-level observation, not a confirmed
  real-body defect; the 2026-11-16 runtime correction found the flush branch on
  the split return. It is not a runtime reproduction on hardware.
- P2 (`unmap`) is separate: W2 is confirmed, the `Range` model fix and the
  corresponding postcondition are pending.
