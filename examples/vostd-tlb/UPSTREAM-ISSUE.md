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

## Related verification-level observation (not a confirmed real-code defect)

Adding the postcondition to the unmodified frozen body fails at the early
return (`Ok` path) at 1494/1; it also fails with an entry precondition that TLB
entries at the VA are backed by the old cursor view (A6). This is a
verification-level observation about rev 43da079. Runtime ostd v0.16.0 shows
that a split returns `Err(Mapped)`, so `VmSpace::map` takes its flush branch;
the “real body misses a flush” reading was not reproduced. F-029 remains a
contract gap, not a confirmed implementation bug.

## Proposed changes

1. `map`: postcondition "no old mapping covering the VA remains in
   `final(tlb_model).mappings`"; flush on the split path. The optimized patch
   has `map_loop` return a `split` flag and flushes only then (no flush on a
   plain new mapping); `Cursor::map` gains a `split_out: bool` and the
   `Ok && !split => no old mapping at VA` fact. Full crate: 1495/0; W1 with the
   postcondition fails at 1494/1.
2. `unmap`: make `TlbModel::dispatch_tlb_flush_spec` apply the whole pending
   sequence (not only the last op) and make `Range` remove every intersecting
   mapping; add the revocation postcondition. The final no-trailing series is
   **1502/0**, and W2 with the postcondition still fails at 1499/1. The
   trailing `Range(start..end)` op is removed by the cursor-independent E''
   invariant; the per-item `issue_tlb_flush_with` ops cover the mappings.

## Reproduction

Patches and scripts (in the spec-check repo, `examples/vostd-tlb/`):

- `a2-no-trailing/combined-tlb-contracts-no-trailing.patch` (canonical)
- historical standalone: `map-tlb-flush-optimized.patch`,
  `unmap-tlb-flush-dispatch-all.patch`
- `apply_w1.py`, `apply_w2.py`, `run-check.sh`
- `THREAT-MODEL.md`, `A1-RUN.md`, `A2-FINAL.md`, `PATCH-RUN.md`, `B-RUN.md`

Commands: `cargo-verus check -p ostd --offline --target x86_64-unknown-none
--lib` with the pinned SDK (`VERUS_USE_RUSTUP=0`, `VERUS_Z3_PATH`),
on the frozen rev; baseline 1495/0.

## Boundary

Verification-level evidence for ostd. A separate QEMU hardware/VM experiment
(`qemu-hw-test/`) reproduces the stale-translation hazard for a 2MB split+remap,
but it does not execute ostd. A model-level reachability witness shows a valid view
with a present 2M mapping at the target VA (`reach-witness.patch`, 1496/0).
The `map` postcondition and any split-flag change are internal API changes;
review welcome on the preferred shape. The `map` body change is a frozen-rev
candidate, not a claim that v0.16.0 misses a flush.
