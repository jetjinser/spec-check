# Frame conditions — run record

Date: 2026-11-15. Inputs: VIR `/tmp/m7a-vir-export2/crate.vir` (frozen rev
43da079), probes on an isolated copy `/tmp/vostd-43da079-frame` of the ktest
workspace. Pre-registration: `EXPECTED.md` (written before the probe runs).
Static rules: `docs/M8-frame-conditions.md`.

## Static census

`cargo-spec-check census` (schema `spec-check.census/1`, additive `frame`
object). Totals over 1,468 Exec/Proof functions:

| bucket | count |
|---|---|
| explicit frame equality (`final(e) == old(e)`, 456 clauses) | 147 |
| absent (postcondition present, no frame equality) | 1,121 |
| no postcondition | 200 |
| mutable-output functions with absent frame | 216 |
| absent but clauses mention `old`/`final` in other relations | 211 |
| absent with neither `old` nor `final` | 910 |
| mutable-output, absent, neither marker | 5 |

Reading: explicit frame equalities are concentrated in `mm` (74) and `specs`
(71); only 2 explicit functions are in `sync`. Most "absent" functions do
mention `old`/`final`, but in relations that update state
(`final@ == old@.insert(x)`), which implies element preservation without
matching the syntactic frame form. Only 5 public mutable-output functions have
no `old`/`final` syntax at all, so the syntactic absence is a weak gap signal.
The probe set tests that distinction.

## Probes

Command:

```
cargo-spec-check analyses-divergence \
  --manifest examples/vostd-frame/divergence-manifest-frame.json \
  --cargo-verus <verus>/cargo-verus --z3 <verus>/z3 --vstd <verus>/vstd \
  --out /tmp/vostd-frame-run6
```

Result: dependency 1908/0, target 1506 verified / 1 error, baseline clean,
workspace restored byte-identical after the run.

| id | interface | asserted fact | expected | actual |
|---|---|---|---|---|
| f01 | `MemoryRegionArray::push` | earlier elements keep values | supported | ObligationSupported |
| f02 | `MemoryRegionArray::push` (full) | failed push leaves array unchanged | supported | ObligationSupported |
| f03 | `CpuSet::add` | other CPU stays absent | supported | ObligationSupported |
| f04 | `CpuSet::remove` | other membership unchanged | supported | ObligationSupported |
| f05 | `VmWriter::write_once` (no space) | cursor/end/id unchanged on error | supported | **ContractGapCandidate** |
| f06 | `VmReader::limit` | returned reader keeps cursor/ghost id | supported | ObligationSupported |
| f07 | `VmReader::skip` | returned reader keeps `end`/ghost id | supported | ObligationSupported |
| f08 | `VmWriter::limit` | returned writer keeps cursor/ghost id | supported | ObligationSupported |
| f09 | `MutexGuard::deref_mut` | viewed value unchanged without write | gap | **ObligationSupported** |
| f10 | `RwLockWriteGuard::deref_mut` | viewed value unchanged without write | gap | **ObligationSupported** |

Supported 9, gap 1, Inconclusive 0. Two pre-registration mismatches are kept
as recorded.

### f09/f10 (predicted gap, actually supported)

The pair `old(self).view() == *ret` / `final(self).view() == *final(ret)`,
together with the reborrow semantics (no write through `ret`), is sufficient
for the verifier to derive `final(self).view() == old(self).view()`. The
prediction that lock guards lack caller-visible frame was wrong; the two-state
reborrow contract does provide it. This corrects an audit intuition and is
evidence for the map cell rather than a defect.

### f05 (recharacterized)

The probe failed its call-site precondition at `io.rs:586`,
`old(self).wf(*old(owner))`, not the intended preservation assertion. After
`VmWriter::limit`, the contract gives `r.inv()`, `r.avail_spec()`,
`r.cursor`, `r.ghost_id` but says nothing about the writer–owner well-formedness
`wf(owner)` (no owner is passed to `limit`). A caller that limits a writer and
then uses it with its tracked owner cannot discharge `wf`.

This is a proof-theoretic contract-gap candidate on an API frame relation. It
does not show a runtime counterexample, and it may be intentional: changing
`end` changes the writer's range, so the owner's view would need a matching
update that `limit` cannot express without the owner. Recorded as a candidate
observation, not a confirmed defect.

## Residual boundaries

- The static definition only recognizes equality-to-old/final; update-style
  equalities and `r.*` reborrow clauses are not counted as frame.
- Probe facts are concrete and frozen-surface only; a gap means "not derivable
  from the current contract", not "implementation violates it".
- `limit`/`skip` probes assert on the returned reborrow `r`; the discarded-
  return form additionally loses `final(self)` facts (observed while iterating
  f05), which is a known Verus reborrow framing quirk, not a contract claim.
