# P1 patch run record

Patch: `map-tlb-flush.patch` (27 lines, one file).
Baseline: 1495 verified / 0 errors. All runs below use the same command
(`run-check.sh`).

## Results

| run | source | result |
|---|---|---|
| A4 | real code + postcondition + flush on the absent/split return path | **1495 / 0** |
| A5 | real code + postcondition, no code change | **1494 / 1**, postcondition failed at the `return` exit |
| W1 + clause | wrong body (no flush at all) + postcondition | **1494 / 1**, postcondition failed |

A5 is the important one: with the new postcondition, the *real* `map` fails at
the early return. That path is taken when the slot is absent, including when
`map_loop` split a huge page that covered the address. The real body only
flushes on the `Err(frag)` (replaced-frame) path, so an old TLB entry covering
the address can survive. The A4 patch adds an address flush before that return
and the postcondition then verifies.

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
- The failure of A5 is a verification-level witness that the real body lacks
  the flush on that path; it is not a runtime reproduction on hardware.
- P2 (`unmap`) is separate: W2 is confirmed, the `Range` model fix and the
  corresponding postcondition are pending.
