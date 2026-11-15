# TLB contract gaps: wrong implementations that verify

## Threat model

A user process owns virtual addresses in its `VmSpace`; the kernel maps frames
into them and unmaps them when the process exits or memory is re-used. The TLB
is a hardware cache of translations. If a mapping is replaced or removed in the
page table but the corresponding TLB entry survives, the process keeps reading
or writing the old physical frame. If that frame is later re-used as a page
table, another process's data, or kernel memory, the stale entry becomes a
use-after-free / cross-owner write primitive.

Properties:

- **P1 (remap frame isolation)**: after `VmSpace::map` replaces an existing
  mapping at VA `v`, no TLB entry covering `v` may remain for the old frame.
- **P2 (unmap revocation)**: after `VmSpace::unmap` removes mappings in
  `[start, end)`, no TLB entry covering that range may survive.

Both properties are about the *model state* (`TlbModel`), not the untrusted
hardware; the verification model claims to over-approximate what may be in the
TLB.

## Witnesses

Method: delete the TLB operations from the body on a pristine copy of the
frozen ktest workspace, then run the normal verification. If the crate still
verifies with 0 errors, the contract admits the wrong implementation.

Command for both runs:

```
run-check.sh   # cargo-verus check -p ostd --target x86_64-unknown-none --lib
```
with `VERUS_USE_RUSTUP=0`, `VERUS_Z3_PATH`, `RUSTC_BOOTSTRAP=1`.

| witness | mutation | result |
|---|---|---|
| W1 | `VmSpace::map`: delete the replacement-path `issue_tlb_flush_with(Address)` + `dispatch_tlb_flush` | **1495 verified / 0 errors** |
| W2 | `VmSpace::unmap`: delete both `issue_tlb_flush_with` calls and the final `dispatch_tlb_flush` | **1495 verified / 0 errors** |

Baseline on the unmodified copy: 1495 verified / 0 errors. The mutated bodies
therefore satisfy every existing `requires`/`ensures`; the contracts say nothing
about `tlb_model` on exit (map) or only `final(tlb_model).inv()` (unmap), and
`TlbModel::inv` does not relate cached mappings to the page table.

Documentation makes the requirement explicit: `unmap` says "It must issue and
dispatch a TLB flush after the operation. Otherwise, the memory safety will be
compromised." The code comments in `tlb.rs` describe the same hazard for
recycled frames. The documentation is stronger than the contract.

## Why this is not a surface-level modeling artifact

- The real implementation does flush; the witnesses are wrong bodies, not
  proofs about the real code.
- Both witnesses preserve every existing postcondition, including the cursor
  owner `map_spec`/`unmap_spec` and the TLB invariant. The only difference is
  the missing flush, which the contract does not observe.
- A caller cannot compensate: `VmSpace::map`/`unmap` are the only operations
  allowed to mutate the mappings, and their contracts give the caller no
  obligation or postcondition about `TlbModel`. There is no verification-level
  sensor that distinguishes the two bodies.

## Evidence grade and preregistration

`Confirmed` for "the current contract admits a wrong implementation" (the
witnesses are machine-verified). The threat-model text and P1/P2 were written
down after the exploratory body mutations, not before them; no per-witness
prediction was registered. The patch work that follows is pre-registered in
`PATCH-EXPECTED.md`.

## Residual boundaries

- The witnesses show a contract gap, not that the real kernel has stale TLB
  entries.
- Hardware/TLB behavior is modeled by `TlbModel`; the witnesses do not test the
  hardware model itself.
- A fix must add a postcondition that observes the flush, and may need a model
  fix for `TlbFlushOp::Range` (the model currently removes only mappings
  covering `range.start`).
