# A2 final: trailing Range flush removed (verified)

Date: 2026-11-16. Merged to `master`; the `baseline-2026-11-16` tag is
intentionally **not** moved. The trailing
`TlbFlushOp::Range(start_va..end_va)` is **removed** from `unmap`; the result is
verified at **1502 verified / 0 errors** with `dispatch_all` + per-item
`_with` flushes.

## Patch

`a2-no-trailing/combined-tlb-contracts-no-trailing.patch` (728 lines, five
files) applies cleanly to pristine 43da079 (`patch -p1 --dry-run` checked) and
supersedes the old combined patch for the unmap part; the old
`combined-tlb-contracts.patch` is retained for reference.

Files changed vs pristine:

- `ostd/specs/mm/page_table/cursor/split_while_huge_lemmas.rs` — new
  `split_while_huge_preserves_start_fragment`.
- `ostd/specs/mm/tlb.rs` — new `lemma_op_covers_submapping`.
- `ostd/src/mm/kspace/kvirt_area.rs`, `ostd/src/mm/page_table/cursor/mod.rs` —
  unchanged A1 map changes (carried from combined).
- `ostd/src/mm/vm_space.rs` — A1 map contract + A2 no-trailing proof.

## Key invariant (E'')

Cursor-independent, unlike the failed E bridge:

```text
forall old m,
  start_va <= m.va_range.start < end_va
  ==>
  removed.contains(m)
  || exists sm,
       adjusted_base.contains(sm)
       && sm.va_range.start == m.va_range.start
       && sm.va_range.end <= m.va_range.end
```

Why this closes the break: E'' does not mention `cursor_owner@.cur_va`, so the
`take_next == None` cursor jump cannot break it. The break changes only cursor
state; `removed`/`adjusted_base` are unchanged.

Maintenance:

- **Mapped**: old witness `sm0 ∈ old_adjusted`; if `sm0 ∈ old_removed`, it is
  already in the new `adjusted_base`; otherwise `sm0 ∈ prev_mappings` and the new
  split-start-fragment lemma gives a fragment `sm2` of `sm0` at the same start,
  which is in `sm_set ⊆ adjusted_base`.
- **Stray**: `adjusted_base` is unchanged; the old witness survives.
- **Initial**: `adjusted_base` is the old view and `removed` is empty, so `m`
  witnesses itself.

The new split lemma is the missing fact: every mapping in a view has a
descendant in `split_while_huge(size)` whose VA start equals the original start
(the first page fragment), with end inside the original range.

## Final postcondition

After `dispatch_all`, for every old `m` in `[start, end)`:

- if `removed(m)`: removed-coverage gives a pending op covering `m`;
- else E'' gives `sm2 ∈ adjusted_base` with the same start/end inside `m`. At
  loop exit `cursor >= end`; if `sm2` were not removed, the existing
  “not-yet-removed” invariant would either put its start at/after the cursor
  (contradiction with `m.start < end`) or make it a boundary-parent fragment,
  which is impossible because `m.start >= start_va` and old-view VA ranges are
  disjoint. So `sm2 ∈ removed`; removed-coverage gives an op covering `sm2`, and
  `lemma_op_covers_submapping` lifts it to `m`.

Then `TlbModel::lemma_apply_all_removed` yields the revocation postcondition
without any trailing Range op.

## Verification

| run | tree | result |
|---|---|---|
| no trailing | `/tmp/vostd-a2-interval` | **1502 / 0** (`A2-1502-0.log`) |
| W1 (map flush deleted) + postcondition | `/tmp/vostd-combined-w1` | **1499 / 1**, postcondition failed (`W1-1499-1.log`) |
| W2 (unmap flush deleted) + postcondition | `/tmp/vostd-combined-w2` | **1499 / 1**, postcondition failed (`W2-1499-1.log`) |

W1/W2 are run on the shared combined model (contract + dispatch semantics); the
no-trailing patch changes only the unmap proof and keeps that model. If the
per-item flush calls are deleted from the no-trailing tree, the proof also loses
the pending-coverage invariant (additional obligations), which is still
rejection, not acceptance.

## Reproduction

```sh
cp -a /tmp/vostd-43da079 /tmp/vostd-a2-interval
cd /tmp/vostd-a2-interval
patch -p1 < .../combined-tlb-contracts-no-trailing.patch
sed 's#/tmp/vostd-witness-target#/tmp/vostd-a2-target#' /tmp/run-witness.sh > /tmp/run-witness-a2.sh
nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd /tmp/vostd-a2-interval && /tmp/run-witness-a2.sh'
# 1502 verified, 0 errors
```

## Boundaries

Frozen rev 43da079 only; no upstream, no deck, no spec-check tool changes.
The runtime F-029 correction (split returns `Err(Mapped)`; real map flushes) is
independent and remains in `ostd-runtime-2026-11-16/`.
