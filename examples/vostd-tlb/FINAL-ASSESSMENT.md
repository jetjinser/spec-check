# Final internal assessment: did we reach the core outcome?

Date: 2026-11-15 (updated 2026-11-16). Frozen rev 43da079. Upstream actions are deferred by the
user; this assessment covers the internal work only.

## 1. Discovery / proof — ACHIEVED

- W1/W2: wrong `map`/`unmap` bodies without TLB flushes verify at 1495/0
  (F-029/F-030).
- A5/A6: the unmodified `map` fails the new postcondition at the early return
  (1494/1), even with an entry-backing precondition, so the missing flush is on
  the huge-page split path, not an unbacked stale entry.
- Reachability: a concrete valid view with a present 2M mapping at the target VA
  verifies (1496/0, `reach-witness.patch`).
- Locks: no safety witness; safety is enforced by linear permissions (negative
  result), with `try_lock` always-None as a liveness witness (F-031).

## 2. Patch quality — MOSTLY ACHIEVED

- `map`: split-gated flush; plain new mappings take no flush. Combined pristine
  crate 1500/0; W1 + postcondition 1499/1.
- `unmap`: `dispatch_all` model + `Range` flush semantics + revocation
  postcondition. Combined pristine 1500/0; W2 + postcondition 1499/1.
- Combined series `combined-tlb-contracts.patch` applies cleanly to pristine
  43da079 (`patch -p1 --dry-run`), unlike the earlier ktest-based patches; a
  macro-config difference between the ktest workspace and pristine required the
  `apply_all` proof on both map flush exits.
- **Residual (final):** one trailing `Range` op in `unmap` is retained. The A2
  removal attempts reached 1499/1 (removed-coverage invariant verified) and
  1498/2 with the boundary-parent bridge; the remaining case needs a
  processed-interval history invariant. Engineering decision: keep the one
  extra range flush rather than grow the proof. `A2-ATTEMPT.md` records the
  exact obligations.

## 3. Runtime evidence — HARDWARE-LEVEL ACHIEVED, OSTD RUN STILL ABSENT

- **New:** a minimal 32-bit PAE kernel in QEMU 11.1 confirms that a 2MB
  translation survives a split+4K remap without invalidation (warm 0xAA; no
  invlpg 0xAA; invlpg 0xBB). See `qemu-hw-test/` and `qemu_pae.log`. This is a
  real VM execution of the hazard, not an ostd run.
- ostd kernel run remains absent: OSDK builds plain-Rust kernels, the frozen
  ostd is Verus-built, and no bootable image for the frozen verification tree
  exists. Asterinas' runtime ostd is a different 0.18 codebase.
- An executable **model** test (`model-test/`) passes: split-then-remap without
  a flush leaves the stale TLB entry; split-gated flush removes it; unmap ops +
  dispatch_all revoke; no dispatch leaves entries; Range removes all
  intersecting mappings. This is not a kernel run.

## 4. Upstream status — DEFERRED

- Upstream identified (`asterinas/vostd`) and an issue package prepared, but the
  user deferred all upstream/external actions. No submission, no feedback, no
  claim of either.

## 5. Impact — NONE YET

- No merged change, no deployment, no adopted CI check.
- Durable output: verified candidate patches, executable model test, issue
  package, and this assessment.

## Verdict

The core **scientific** outcome is reached: wrong implementations pass the
current TLB contracts, the real `map` split path misses a flush, and two
verified patches reject the witnesses on the pristine revision.

The fuller outcome is **not fully reached**: A2 keeps a redundant trailing
Range op for the proof; a hardware/VM-level hazard reproduction now exists in
QEMU, but an **ostd kernel run** is still absent; upstream is deferred. Next
internal steps: finish the coverage invariant to drop the trailing op, and add
a bootable image for the frozen verified ostd when available.

## Update (2026-11-15, second internal pass)

- A2 removal was retried on the pristine combined tree: 1498/2, with the same
  loop-coverage invariant failure plus the revocation postcondition. The
  trailing Range op remains in the delivered patch; exact obligations are in
  `A2-ATTEMPT.md`.
- Runtime: the QEMU hardware/VM experiment (`qemu-hw-test/`) confirms the
  hazard but does not execute ostd. OSDK 0.16 and QEMU are available, but OSDK
  builds plain-Rust kernels; the frozen verified ostd is Verus-built and has no
  OSDK integration. The blocker is architectural (Verus vs OSDK), not a missing
  tool. Asterinas' runtime ostd is a different 0.18 codebase, so a test there
  would not validate the frozen artifact.
