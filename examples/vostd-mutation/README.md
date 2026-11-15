# Contract mutation pilot (2026-11-15)

Question: does the verification stack notice a removed precondition, and does a
caller obligation notice a removed postcondition? The point is a sensitivity
metric for the checks, not new defect claims.

## Setup

Evidence grade: exploratory method calibration. No per-site outcome
predictions were registered before this run, so the outcomes below are not
defect evidence; they only calibrate what the two check layers can observe.

Frozen copy `/tmp/vostd-mut` of the ktest workspace. Baseline
`cargo-verus check -p ostd --target x86_64-unknown-none --lib` (no probes):
**1495 verified / 0 errors**. Then seven `requires` clauses in seven functions
were replaced by `true` (keeping `ensures`/`returns`/`with`), and the crate was
re-verified once in a batch. Error spans attribute failures to the mutated
function.

## Requires relaxation (7 sites)

| id | function | outcome | evidence |
|---|---|---|---|
| m01 | `MemoryRegion::new` | load-bearing | constructed value violates the `MemoryRegion` type invariant |
| m02 | `mm::kspace::paddr_to_vaddr` | load-bearing | possible arithmetic overflow at `pa + BASE` |
| m03 | `mm::page_size` | load-bearing | 3 lemma preconditions fail without the level bound |
| m04 | `AtomicCpuSet::load` | proof-redundant | crate still verifies with the ordering restriction removed |
| m05 | `AtomicCpuSet::store` | proof-redundant | same; loop invariant does not need the exclusions |
| m06 | `VmReader::skip` | load-bearing | `r.inv()` postcondition and `assert!(nbytes <= remain)` fail |
| m07 | `VmWriter::skip` | load-bearing | `r.inv()` postcondition and `assert!(nbytes <= avail)` fail |

Mutant totals: 1489 verified / 6 errors; 5 of 7 mutated functions fail, 2 pass
unchanged. Requires sensitivity = **5/7 (71%)**.

`proof_redundant` is not "safe to remove": `AtomicCpuSet::load` with
`Ordering::AcqRel` (and `store` with `Acquire`/`AcqRel`) is an invalid atomic
usage that panics at runtime. Those preconditions are runtime-validity guards,
not proof obligations. The verification stack does not catch their removal, so
they need tests or a rule-based check, not a proof.

## Ensures deletion (1 site)

Delete the append equality
`final(self)@ == old(self)@.push(region)` from `MemoryRegionArray::push` and
rerun the frame probe `f01_push_preserves_earlier` (which verified on the
pristine contract). Result: **ContractGapCandidate** — the caller obligation
flips from `ObligationSupported` to gap. Ensures sensitivity at the caller
level = **1/1**. Implementation verification alone cannot capture this: fewer
ensures only makes the implementation proof easier.

## Reading

- Preconditions that encode safety/validity are mostly load-bearing; the two
  misses are runtime-ordering guards, which are exactly the class a proof will
  not protect.
- Postcondition deletion is invisible to implementation verification and
  visible to caller obligations; the existing divergence/frame clients are the
  right sensor.
- This is a pilot over one crate and one clause per function. It says nothing
  about coverage of the other 767 requires functions.

## Residual

The pilot does not mutate the 30 confirmed findings. It uses seven
representative preconditions and one supported caller obligation, because
each confirmed finding would need its own contract patch plus a working
obligation client; several (F-018/F-021) already have infeasible candidate
patches. Extending the operator to the confirmed set is left as follow-up.

## Reproduce

- `mutate.py <copy>` applies the seven content-anchored mutations (exact-count
  assertions; no line numbers).
- `run-check.sh` runs the baseline/mutant check with the Verus SDK on PATH and
  `VERUS_USE_RUSTUP=0`, `VERUS_Z3_PATH`, `RUSTC_BOOTSTRAP=1`.
- `manifest-f01-minus-append.json` + `report-f01-minus-append.json` reproduce
  the ensures-deletion case (target workspace points at a copy with the clause
  removed).
