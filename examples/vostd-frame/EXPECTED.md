# Frame probes — pre-registration (written before any probe run)

Static census (census-r2, same VIR as M7a/M7e) reports 147 explicit frame
functions and 216 mutable-output functions with an absent explicit frame.
This probe set discriminates "no explicit `final(e) == old(e)` clause" from
"caller cannot prove preservation". Ten public interfaces are tested; each
calls the function and asserts one concrete preservation fact about state the
callee should not touch. Classification is the divergence classifier:
`assertion failed` in the obligation function with a clean baseline = `gap`;
verified = `supported`.

## Predictions

| id | interface | asserted fact | expected |
|---|---|---|---|
| f01 | `MemoryRegionArray::push` | earlier elements keep their values | supported |
| f02 | `MemoryRegionArray::push` (full) | failed push leaves the array unchanged | supported |
| f03 | `CpuSet::add` | a different CPU stays absent | supported |
| f04 | `CpuSet::remove` | membership of a different CPU is unchanged | supported |
| f05 | `VmWriter::write_once` (no space) | failed write leaves cursor/end/id unchanged | supported |
| f06 | `VmReader::limit` | returned reader keeps cursor and ghost id | supported |
| f07 | `VmReader::skip` | returned reader keeps `end` and ghost id | supported |
| f08 | `VmWriter::limit` | returned writer keeps cursor and ghost id | supported |
| f09 | `MutexGuard::deref_mut` | guard's viewed value is unchanged when no write occurs | gap |
| f10 | `RwLockWriteGuard::deref_mut` | guard's viewed value is unchanged when no write occurs | gap |

Basis: f01/f02 are implied by `final(self)@ == old(self)@.push(...)` and the
full-state equality on error; f03/f04 by `final(self)@ == old(self)@.insert/remove`;
f05 by `*old(self) == *final(self)` on `Err`; f06–f08 by the `r.*` clauses.
f09/f10: the contract relates `old(self).view() == *ret` and
`final(self).view() == *final(ret)` but never `final(self).view() == old(self).view()`;
with no write through `ret`, a caller cannot derive preservation.

## Model scope

Preservation is asserted on the frozen spec surface with concrete values.
`gap` means the contract does not prove this fact; it is not a runtime
counterexample. `supported` does not claim frame completeness.

## Expected totals

Supported 8, Gap 2, Inconclusive 0. Any other outcome is recorded as a
pre-registration mismatch with the compile/verification reason, and is not
rewritten.
