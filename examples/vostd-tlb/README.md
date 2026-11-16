# TLB contract case study (frozen ostd 43da079)

Status: **internal case study closed, 2026-11-16**. No upstream submission.
The `baseline-2026-11-16` tag is intentionally not moved.

This directory is the canonical entry for the TLB case. It answers one
main-line question: **do the verified `VmSpace::{map,unmap}` contracts constrain
the TLB behavior they claim to protect?**

## Result at a glance

| Claim | Status | Canonical evidence |
|---|---|---|
| Wrong `map`/`unmap` bodies that omit the TLB flush still verify | **Confirmed** (frozen model, 1495/0) | `THREAT-MODEL.md`, `apply_w1.py`, `apply_w2.py` |
| The contract gap is rejected by strengthened contracts | **Confirmed candidate** (1502/0; W1/W2 1499/1) | `A2-FINAL.md`, `a2-no-trailing/` |
| Stale translation is a real hazard, not just a model artifact | **Confirmed** on QEMU and runtime ostd v0.16.0 | `qemu-hw-test/`, `ostd-runtime-2026-11-16/` |
| The real `map` body misses a flush | **Not confirmed; corrected** | `FINAL-ASSESSMENT.md` §1, `ostd-runtime-2026-11-16/` |
| The frozen verified artifact can be booted and run | **Blocked** (architectural) | `verus-artifact-2026-11-16/P4-RUN.md` |
| Upstream accepted or merged the fix | **Deferred; no submission** | `UPSTREAM-ISSUE.md`, `SEND-PACKAGE.md` |

The F-029 correction matters: the frozen model admits W1/W2, but runtime
v0.16.0 shows `Cursor::map` returns `Err(Mapped)` on a split, so
`VmSpace::map` takes its flush branch. F-029 is a **contract gap**, not a
confirmed implementation defect. Do not merge the two claims.

## Read in this order

1. `FINAL-ASSESSMENT.md` — claim classification, corrections, and limits.
2. `THREAT-MODEL.md` — properties P1/P2 and the W1/W2 witnesses.
3. `A2-FINAL.md` — final unmap proof: trailing `Range` removed, 1502/0.
4. `a2-no-trailing/combined-tlb-contracts-no-trailing.patch` — canonical
   verified series.
5. `qemu-hw-test/` and `ostd-runtime-2026-11-16/` — hardware and runtime
   triangulation.
6. `verus-artifact-2026-11-16/P4-RUN.md` — why the frozen artifact is not run.
7. `UPSTREAM-ISSUE.md`, `SEND-PACKAGE.md` — local handoff package only.

Historical records are retained but are not current conclusions:

- `A2-RUN.md` records the earlier 1500/0 `dispatch_all` + trailing `Range`
  version. `A2-FINAL.md` supersedes it.
- `PATCH-RUN.md` A5/A6 records the verification-level early-return result. The
  2026-11-16 runtime correction supersedes the “real body misses a flush”
  reading.
- `A2-ATTEMPT.md` preserves failed attempts and their obligations.

## Main-line framing

For the revocation property P2, the checked shape is `P ∧ R ⇒ Q`: the caller
and normal-return premises `P`, the existing contract `R`, and “no old mapping
in `[start,end)` remains in the final TLB” as `Q`. W1/W2 satisfy `R` while
violating `Q`, so the original contract does not support the documented
requirement. The strengthened contract does, at 1502/0.

This is a case study in **spec quality**, not a claim that the TLB implementation
is broken. It supplies:

- a mechanically checked negative witness (contract admits a wrong body);
- a verified candidate repair that still rejects both negative bodies;
- runtime/hardware evidence that the admitted behavior is hazardous;
- an explicit boundary where the frozen artifact could not be executed.

## Reproduction

The canonical patch applies to pristine `43da079`:

```sh
cp -a /tmp/vostd-43da079 /tmp/vostd-a2-interval
cd /tmp/vostd-a2-interval
patch -p1 < .../a2-no-trailing/combined-tlb-contracts-no-trailing.patch
/tmp/run-witness-a2.sh    # 1502 verified, 0 errors
```

Exact environment and log commands are in `A2-FINAL.md`,
`qemu-hw-test/README.md`, and `ostd-runtime-2026-11-16/README.md`.
W1/W2 mutation scripts and the baseline command are in `THREAT-MODEL.md` and
`run-check.sh`.

## Boundaries

- Frozen revision only: `43da079`; no upstream, deck, or spec-check tool changes.
- Runtime evidence is from ostd v0.16.0, which is the same version family but
  **not** the frozen Verus artifact.
- The QEMU experiments execute small standalone kernels, not ostd.
- No claim of deployment, merged fix, maintainer acceptance, or external
  validation.
