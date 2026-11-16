# Dossier: TLB flush contracts (ostd, frozen rev 43da079)

Maintainer-ready summary. Everything below is reproducible from this directory
and is a **candidate**, not an upstream submission.

## Claim

Two public operations that mutate user page tables do not require a TLB flush
in their contracts. Wrong bodies that omit the flush verify at 1495/0 on the
frozen revision:

- `VmSpace::map` (F-029): replacement and split paths.
- `VmSpace::unmap` (F-030): removal path.

This is a contract gap. The frozen-revision A5/A6 verification-level run also
failed at the early return when the new postcondition was added body-unchanged
(1494/1), including with an entry precondition backing the TLB. That reading
was **not** reproduced at runtime: ostd v0.16.0 shows the split returns
`Err(Mapped)`, so `VmSpace::map` takes its flush branch. Do not present A5/A6
as a confirmed real-code defect.

## Evidence

| item | file |
|---|---|
| threat model, P1/P2 | `THREAT-MODEL.md` |
| W1/W2 mutation scripts | `apply_w1.py`, `apply_w2.py` |
| P1 patch + run record | `map-tlb-flush-optimized.patch`, `A1-RUN.md` (conservative: `map-tlb-flush.patch`) |
| P2 patch + run record | `unmap-tlb-flush-dispatch-all.patch`, `A2-RUN.md` (earlier: `unmap-tlb-flush.patch`) |
| pre-registration | `PATCH-EXPECTED.md` |

Baseline and patched runs: `run-check.sh` (cargo-verus check -p ostd,
`x86_64-unknown-none`, lib).

## Proposed changes

1. **map**
   - Add the postcondition: if an old mapping covered the VA, no such mapping
     remains in `final(tlb_model).mappings`.
   - On the frozen revision, the A1 candidate flushes the address on the
     early-return (absent/split) path. Whether a body change is still needed on
     the target runtime revision must be checked there; v0.16.0 already flushes
     on the `Err(Mapped)` split return.
2. **unmap**
   - Fix the model: `TlbFlushOp::Range` must remove every intersecting mapping
     (`flush_range`), not only the mapping covering `range.start`.
   - Add the revocation postcondition for old mappings starting in
     `[start, start + len)`.
   - Make dispatch apply the whole pending stack (`dispatch_all`), keep the
     per-item `issue_tlb_flush_with` calls for frame lifetime, and drop the
     trailing flush. The no-trailing proof is verified at 1502/0
     (`a2-no-trailing/combined-tlb-contracts-no-trailing.patch`).
3. **tests**
   - Existing ktest/unit suites should be re-run (not run in this goal; the
     frozen copy's ktest harness is available).
   - The two negative bodies (`apply_w1.py`, `apply_w2.py`) are useful as
     one-off regression evidence, not as permanent CI tests.

## CI / adoption

Once the postconditions land, normal verification enforces them; no new job is
needed for the contract. A cheap optional guard would be a `compile_fail`
style check that the two bad bodies no longer verify. Runtime cost depends on
the target revision: v0.16.0 already flushes on the `Err(Mapped)` split return,
so do not add a hot-path flush without checking. The split-flag A1 shape is
only a frozen-rev candidate.

## Not claimed

- The ostd evidence is verification-level. A separate hardware/VM-level
  reproduction of the stale-translation hazard exists (`qemu-hw-test/`, QEMU
  2M PAE and 4M PSE), but it does not execute ostd.
- The witnesses show the contract admits wrong bodies. A5/A6 is a
  verification-level observation from the frozen revision, not a confirmed
  real-body defect; runtime v0.16.0 flushes on the split path. F-029 is a
  contract gap.
- Frozen rev 43da079 only; no upstream review yet.
