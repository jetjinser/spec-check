# Final assessment: did we reach the core outcome?

Date: 2026-11-15. Frozen rev 43da079. Evidence is verification-level unless
stated otherwise.

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

## 2. Patch quality — PARTIAL

- `map` optimized: only split/replacement flushes; plain new mappings take no
  flush. Full crate 1495/0; W1+postcondition 1494/1. (`map-tlb-flush-optimized.patch`)
- `unmap`: `dispatch_all` model + `Range` semantics + revocation postcondition.
  Full crate 1500/0; W2+postcondition 1499/1.
  **Residual:** one trailing `Range` op is retained as the proof witness;
  removing it needs a loop coverage invariant. At runtime it is redundant.
- Both patches are candidates; no upstream review; `Cursor::map` gains an
  internal `split_out` parameter.

## 3. Runtime evidence — NOT ACHIEVED

- No QEMU (`qemu-system-x86_64` absent), no `cargo-osdk`/`osdk`, empty
  `ostd/osdk/`; the ktest workspace cannot execute here.
- Replaced by the Verus reachability witness; no hardware stale-TLB demo, no
  full cursor-owner tree execution.

## 4. Upstream status — NOT SUBMITTED (exact blocker)

- Upstream identified: `asterinas/vostd`.
- No `gh`, no `GH_TOKEN`/`GITHUB_TOKEN`, no write channel; anonymous read only.
- `UPSTREAM-ISSUE.md` and `SEND-PACKAGE.md` are ready to send manually.
- Feedback: none (not submitted).

## 5. Impact — NONE YET

- No accepted upstream change, no deployment, no CI check adopted.
- The repository artifacts are the only durable output.

## Verdict

The core scientific question is answered with machine-checked evidence: **yes,
wrong implementations can pass the current TLB contracts, and one real `map`
path is missing a flush**. Two candidate patches are verified and reject the
witnesses.

The core *outcome* in the fuller sense — optimized patches, runtime
confirmation, and upstream adoption — is **not fully reached**: A2 has a proof
residual, runtime is blocked by the environment, and submission is blocked by
missing credentials. The shortest path to closure is: (1) finish the unmap
coverage invariant and drop the trailing Range, (2) provide a GitHub token or
submit the prepared issue, (3) run a minimal QEMU test if a runner becomes
available.
