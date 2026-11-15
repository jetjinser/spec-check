# Upstream issue draft (asterinas/vostd)

Title: TLB flush is not required by `VmSpace::{map,unmap}` contracts; wrong
implementations verify

## Summary

On rev 43da079, two wrong bodies that omit the TLB flush still pass full
verification (1495 verified / 0 errors):

- `VmSpace::map`: delete the replacement-path flush (wrong body W1).
- `VmSpace::unmap`: delete every flush (wrong body W2).

The contracts do not observe `TlbModel` at exit (`map` says nothing; `unmap`
only `final(tlb_model).inv()`, and `inv` does not relate cached mappings to the
page table). The `unmap` doc says the flush is required or "memory safety will
be compromised"; the spec is weaker than the documented requirement.

## Additional finding: the real `map` misses a flush

Adding the postcondition to the unmodified body fails at the early return
(`Ok` path) at 1494/1. It also fails with an entry precondition that TLB
entries at the VA are backed by the old cursor view (A6), so the counterexample
is the huge-page split path, not an unbacked stale entry. The body flushes only
on `Err(frag)` (replacement); after `map_loop` splits a huge page covering the
VA, the slot is absent and the function returns without flushing.

## Proposed changes

1. `map`: postcondition "no old mapping covering the VA remains in
   `final(tlb_model).mappings`"; flush on the split path. The optimized patch
   has `map_loop` return a `split` flag and flushes only then (no flush on a
   plain new mapping); `Cursor::map` gains a `split_out: bool` and the
   `Ok && !split => no old mapping at VA` fact. Full crate: 1495/0; W1 with the
   postcondition fails at 1494/1.
2. `unmap`: make `TlbModel::dispatch_tlb_flush_spec` apply the whole pending
   sequence (not only the last op) and make `Range` remove every intersecting
   mapping; add the revocation postcondition. Full crate: 1500/0; W2 with the
   postcondition fails at 1499/1. The current patch keeps one trailing
   `Range(start..end)` op; removing it needs a loop coverage invariant (the
   per-item `issue_tlb_flush_with` ops already cover the mappings at runtime).

## Reproduction

Patches and scripts (in the spec-check repo, `examples/vostd-tlb/`):

- `map-tlb-flush-optimized.patch`, `unmap-tlb-flush-dispatch-all.patch`
- `apply_w1.py`, `apply_w2.py`, `run-check.sh`
- `THREAT-MODEL.md`, `A1-RUN.md`, `A2-RUN.md`, `PATCH-RUN.md`, `B-RUN.md`

Commands: `cargo-verus check -p ostd --offline --target x86_64-unknown-none
--lib` with the pinned SDK (`VERUS_USE_RUSTUP=0`, `VERUS_Z3_PATH`),
on the frozen rev; baseline 1495/0.

## Boundary

Verification-level evidence; no hardware/QEMU reproduction (the environment has
no QEMU or osdk runner). A model-level reachability witness shows a valid view
with a present 2M mapping at the target VA (`reach-witness.patch`, 1496/0).
The `map` postcondition and the split flag are internal API changes; review
welcome on the preferred shape.
