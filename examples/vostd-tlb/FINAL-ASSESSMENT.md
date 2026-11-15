# Final internal assessment: did we reach the core outcome?

Date: 2026-11-15. Frozen rev 43da079. Upstream actions are deferred by the
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
- **Residual:** one trailing `Range` op in `unmap` is retained as the proof
  witness. The A2 removal attempt (`A2-ATTEMPT.md`) fails at the loop coverage
  invariant (1499/1; exact obligations listed). The op is redundant at runtime
  but is not removed.

## 3. Runtime evidence — PARTIAL

- QEMU is available (`nix shell nixpkgs#qemu`, 11.1.2).
- Exact blocker: no `cargo-osdk` in nixpkgs, no `osdk/` or `OSDK.toml` in the
  vostd tree, no kernel image/runner. The ktest suite cannot be booted here.
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
Range op for the proof; runtime evidence is model-level because the kernel
runner is absent; upstream is deferred. Next internal steps: finish the coverage
invariant to drop the trailing op, and add a kernel-runner image when available.

## Update (2026-11-15, second internal pass)

- A2 removal was retried on the pristine combined tree: 1498/2, with the same
  loop-coverage invariant failure plus the revocation postcondition. The
  trailing Range op remains in the delivered patch; exact obligations are in
  `A2-ATTEMPT.md`.
- Runtime: OSDK 0.16 and QEMU are available in the Asterinas devshell, but OSDK
  builds plain-Rust kernels; the frozen verified ostd is Verus-built and has no
  OSDK integration. The exact blocker is architectural (Verus vs OSDK), not a
  missing tool. Asterinas' runtime ostd is a different 0.18 codebase, so a test
  there would not validate the frozen artifact. The executable model test is
  the strongest internal runtime evidence available.
