# Final internal assessment: did we reach the core outcome?

Date: 2026-11-15 (updated 2026-11-16). Frozen rev 43da079. Upstream actions are deferred by the
user; this assessment covers the internal work only.

## 1. Discovery / proof — ACHIEVED

- W1/W2: wrong `map`/`unmap` bodies without TLB flushes verify at 1495/0
  (F-029/F-030).
- A5/A6: the unmodified `map` fails the new postcondition at the early return
  (1494/1) at the verification level. **Runtime correction (2026-11-16):** in
  Asterinas v0.16.0 the split returns `Err(Mapped)`, so `VmSpace::map` runs its
  flush branch, and a user `VmSpace` cannot hold a 2M mapping
  (`Frame::map_level()==1`). The "real body misses a flush" reading is not
  reproduced; F-029 is a contract gap, not a confirmed real-code defect.
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

## 3. Runtime evidence — HARDWARE + OSTD v0.16.0 RUNTIME ACHIEVED; FROZEN ARTIFACT RUN ABSENT

- **New:** a minimal 32-bit PAE kernel in QEMU 11.1 confirms that a 2MB
  translation survives a split+4K remap without invalidation (warm 0xAA; no
  invlpg 0xAA; invlpg 0xBB). See `qemu-hw-test/` and `qemu_pae.log`.
- **New:** a ktest in Asterinas v0.16.0 (`ostd-runtime-2026-11-16/`) splits a
  huge kernel linear mapping and remaps a 4K entry without invlpg: stale 0xAA,
  invlpg fresh 0xBB; test `ok`. This executes runtime ostd page-table code, not
  the frozen Verus artifact.
- **Correction:** the split returns `Err(Mapped)`, so `VmSpace::map` flushes;
  the A5/A6 real-body claim is not reproduced and F-029 is reclassified as a
  contract gap. See `THREAT-MODEL.md`.
- **Frozen artifact run remains absent:** OSDK builds plain-Rust kernels, the
  frozen ostd is Verus-built, and no bootable image for the frozen verification
  tree exists. The v0.16.0 tag is a same-version *runtime* tree, but it is still
  unverified and not the frozen artifact; the main Asterinas tree is 0.18.
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

The core **scientific** outcome is reached in corrected form: wrong
implementations pass the current TLB contracts (W1/W2), the hazard is confirmed
on QEMU and in runtime ostd v0.16.0, but the real `map` replacement/split path
does flush; F-029 is a contract gap, not a confirmed real-code defect. Two
verified patches reject the witnesses on the pristine revision.

The fuller outcome is **not fully reached**: A2 keeps a redundant trailing
Range op for the proof; hardware/VM and same-version runtime ostd evidence now
exist, but the **frozen verified artifact** still cannot be run; upstream is
deferred. Next internal steps: finish the coverage invariant to drop the
trailing op, and add a bootable image for the frozen verified ostd when
available.

## Update (2026-11-15, second internal pass)

- A2 removal was retried on the pristine combined tree: 1498/2, with the same
  loop-coverage invariant failure plus the revocation postcondition. The
  trailing Range op remains in the delivered patch; exact obligations are in
  `A2-ATTEMPT.md`.
- Runtime: the QEMU hardware/VM experiment (`qemu-hw-test/`) and the v0.16.0
  ostd ktest (`ostd-runtime-2026-11-16/`) confirm the hazard; neither executes
  the frozen Verus artifact. The runtime ktest also corrects F-029's
  real-body claim. OSDK 0.16 and QEMU are available, but OSDK
  builds plain-Rust kernels; the frozen verified ostd is Verus-built and has no
  OSDK integration. The blocker is architectural (Verus vs OSDK), not a missing
  tool. The v0.16.0 runtime tree used in the ktest is same-version but
  unverified; the main Asterinas tree is 0.18. Neither validates the frozen
  artifact.
