# Dossier: TLB flush contracts (ostd, frozen rev 43da079)

Maintainer-ready summary. Everything below is reproducible from this directory
and is a **candidate**, not an upstream submission.

## Claim

Two public operations that mutate user page tables do not require a TLB flush
in their contracts. Wrong bodies that omit the flush verify at 1495/0 on the
frozen revision:

- `VmSpace::map` (F-029): replacement and split paths.
- `VmSpace::unmap` (F-030): removal path.

This is a contract gap and, for `map`, also a missing flush in the real body on
the absent/split return path: adding the postcondition without changing the
body fails at that return (1494/1). A discriminating run with an entry
precondition ("TLB entries at the VA are backed by the old cursor view") still
fails at the same return, so the counterexample is the split case, not an
unbacked stale entry.

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
   - Flush the address on the early-return (absent/split) path. The
     conservative patch flushes on every absent-slot map.
   - Preferred: have `map_loop` report whether it split a huge page, and flush
     only then (avoids a hot-path flush per new mapping).
2. **unmap**
   - Fix the model: `TlbFlushOp::Range` must remove every intersecting mapping
     (`flush_range`), not only the mapping covering `range.start`.
   - Add the revocation postcondition for old mappings starting in
     `[start, start + len)`.
   - The patch works around the model's single-`pending` dispatch with one
     trailing range flush. Preferred: make dispatch apply the whole pending
     stack (`dispatch_all`), keep the per-item `issue_tlb_flush_with` calls for
     frame lifetime, and drop the trailing flush.
3. **tests**
   - Existing ktest/unit suites should be re-run (not run in this goal; the
     frozen copy's ktest harness is available).
   - The two negative bodies (`apply_w1.py`, `apply_w2.py`) are useful as
     one-off regression evidence, not as permanent CI tests.

## CI / adoption

Once the postconditions land, normal verification enforces them; no new job is
needed for the contract. A cheap optional guard would be a `compile_fail`
style check that the two bad bodies no longer verify. The real cost question is
runtime: the conservative map flush is on a hot path, so the split-flag version
is the recommended upstream shape.

## Not claimed

- No hardware-level reproduction; the evidence is verification-level.
- The witnesses show the contract admits wrong bodies; only the `map` A5 run
  shows the real body missing a flush on one path.
- Frozen rev 43da079 only; no upstream review yet.
