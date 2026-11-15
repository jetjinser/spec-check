# A1: optimized map patch (split-gated flush)

Patch: `map-tlb-flush-optimized.patch` (3 files, 58 added lines). Supersedes the
conservative `map-tlb-flush.patch` for upstream purposes.

## Shape

1. `Cursor::map_loop` returns `split: bool` and proves
   `split == false ==> final(owner)@ == old(owner)@`; the split flag is set only
   in the `ChildRef::Frame` split arm.
2. `Cursor::map` gains a `split_out: &mut bool` parameter and a new ensure:
   `(res is Ok && !split_out) ==> !old(owner)@.present()`.
   The proof uses `replace_cur_entry`'s absence/Some contract and
   `lemma_cur_entry_absent_not_present`.
3. `VmSpace::map`:
   - replacement (`Err(frag)`) path: address flush as before;
   - `Ok` path: flush **only when `split`**, no flush for a plain new mapping;
   - postcondition: no old mapping covering the VA remains in the final TLB.

`kvirt_area`'s two call sites pass a local `split` and ignore it.

## Verification

| run | result |
|---|---|
| optimized patch, full crate | **1495 / 0** |
| same + delete the replacement-path flush (W1) | **1494 / 1**, postcondition failed |
| map_loop-only step (flag + view-unchanged proof) | 1495 / 0 |
| Cursor::map step (out-param + absence fact) | 1495 / 0 |

Static flush behavior:

| path | flushes |
|---|---|
| plain new mapping (absent slot, no split) | 0 |
| huge-page split then insert | 1 address flush |
| replacement of an existing frame | 1 address flush + frame drop-after-flush |

## API / proof cost

- Internal unsafe `Cursor::map` signature changes; no public `VmSpace` API
  change. The out-param keeps the existing result spec intact, avoiding a
  tuple/struct result rewrite.
- New proof surface: the split flag invariant in `map_loop`, and the
  absence-at-entry fact in `Cursor::map`.
- No runtime flush is added on the hot new-mapping path.

## Residual

- Candidate patch, frozen rev 43da079, no upstream review.
- The split flag is an internal contract; if upstream prefers a result struct,
  the same facts carry over.
- The TLB postcondition is model-level; no hardware reproduction.
