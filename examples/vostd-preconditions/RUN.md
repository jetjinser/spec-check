# Over-restrictive preconditions — run record

Pre-registration: `EXPECTED.md`. Inputs: census of frozen ostd VIR (774
functions with requires), 10 boundary clients on pristine copy
`/tmp/vostd-43da079-boundary` of the ktest workspace.

## Static screen (774)

| class | functions |
|---|---|
| invariant / wf present, mixed | 445 |
| invariant/wf plus value comparisons | 76 |
| value comparisons only | 68 |
| other / uninterp / quantified | 185 |

521 functions (67%) mention `inv`/`wf`/`invariants`. These preconditions are
structural, not value-domain choices; an over-restriction claim would need a
documented legitimate call rejected by them, and none was found.

## Demand corpus

- 22 non-blocked ktest candidates: every extracted call verifies against the
  target crate, so it satisfies the callee's preconditions by construction.
  Conflicts: **0**.
- Documentation boundary table: 10 value constraints, each with a documented
  invalid/panic rule and a violating edge call.

## Boundary probes (10)

Command `analyses-divergence` on the boundary manifest. Result: baseline clean,
target 1497 verified / 10 errors. Every failure is `precondition not satisfied`
at the client call (10/10), exactly the pre-registered `ExpectedReject`:

| id | call | actual |
|---|---|---|
| p01 | `MemoryRegion::new(MAX_PADDR, 1)` | precondition reject |
| p02 | `page_size(PagingConsts, 0)` | precondition reject |
| p03 | `page_size(PagingConsts, 16)` | precondition reject |
| p04 | `paddr_to_vaddr(usize::MAX)` | precondition reject |
| p05 | `AtomicCpuSet::contains(cpu, Release)` | precondition reject |
| p06 | `AtomicCpuSet::load(AcqRel)` | precondition reject |
| p07 | `AtomicCpuSet::store(v, Acquire)` | precondition reject |
| p08 | `VmReader::skip(9)` on 8 bytes | precondition reject |
| p09 | `VmWriter::skip(9)` on 8 bytes | precondition reject |
| p10 | `MemoryRegion::new(MAX_PADDR - 1, 2)` | precondition reject |

The divergence classifier labels a call-site failure `ContractGapCandidate`;
here the failure is recharacterized as `ExpectedReject` because the demand table
says the call is invalid/panic-documented. No over-restrictive candidate was
confirmed: **0/10 conflicts**.

## Reading and residuals

- Within the sampled value constraints, preconditions align with documented
  validity/panic rules; none excludes a documented successful call.
- Invariant preconditions (521 functions) were not probed: violating them
  requires constructing ill-formed states, which the safe surface does not
  expose. They remain a screening class, not a result.
- 68 value-only functions; 10 were adjudicated by hand. The remaining 58 are
  mostly lemmas, internal frame/meta accessors, and ACPI parsers whose docs
  state panic-on-invalid. No conflict was found in those read during selection.
- "Documented invalid" is a source-reading judgement; quotes are in
  `EXPECTED.md`. If docs are silent, a rejected call would be a candidate, not
  an expected reject; none of the ten was silent.
