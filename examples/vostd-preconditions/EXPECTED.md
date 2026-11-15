# Over-restrictive preconditions — pre-registration (written before the run)

## Question

A precondition is a *candidate over-restriction* only if a call that the
documentation, tests, or runtime treats as legitimate is rejected by the
contract. Violating a documented validity/panic rule is not over-restriction.

## Demand corpus (fixed before the run)

1. 22 non-blocked ktest candidates (M7b): every extracted call is a
   verified runtime demand. They were checked against the callee's `requires`
   by construction; no extracted test call violates a precondition (the
   verification of the target crate would fail). Conflict count: 0.
2. Documentation boundary table: ten value-constraint preconditions from the
   `value-only` / `inv+value` screen, each with the documented rule and an
   out-of-contract edge call.

## Static screen (census, 774 functions with requires)

Clause classification by VIR expression:

| class | functions |
|---|---|
| invariant / wf present, mixed clauses | 445 |
| invariant/wf plus value comparisons | 76 |
| value comparisons only | 68 |
| other / uninterp / quantified | 185 |

521 functions (67%) mention an `inv`/`wf`/`invariants` predicate, so most
preconditions are structural rather than value-domain choices.

## Boundary probes (10)

Each probe calls the function with an argument that violates one clause and is
documented as invalid or panic-inducing. Expected: precondition rejection,
labeled `ExpectedReject`, not a gap.

| id | call | documented rule |
|---|---|---|
| p01 | `MemoryRegion::new(MAX_PADDR, 1, Usable)` | "Constructs a valid memory region" (base+len must fit) |
| p02 | `page_size::<PagingConsts>(0)` | levels are 1-based; level 0 has no page size |
| p03 | `page_size::<PagingConsts>(16)` | level above `NR_LEVELS + 1` |
| p04 | `paddr_to_vaddr(usize::MAX)` | offset addition must not overflow |
| p05 | `AtomicCpuSet::contains(cpu, Ordering::Release)` | load orderings exclude Release/AcqRel |
| p06 | `AtomicCpuSet::load(Ordering::AcqRel)` | load orderings exclude AcqRel |
| p07 | `AtomicCpuSet::store(&v, Ordering::Acquire)` | store orderings exclude Acquire/AcqRel |
| p08 | `VmReader::skip(9)` (8-byte buffer) | "Panics if nbytes > remain" |
| p09 | `VmWriter::skip(9)` (8-byte buffer) | "Panics if nbytes > avail" |
| p10 | `Rhsa::from_bytes(&[])` | documented panic on invalid bytes; minimum length required |

## Prediction

10/10 `ExpectedReject` (precondition failure at the client call); 0
over-restrictive candidates. Any probe that verifies means the contract does
not exclude the edge call (rule not enforced), recorded as a mismatch. If a
probe fails on an assertion or postcondition instead of a precondition, it is
recorded as `Inconclusive` with the exact span.

## Boundary

The probe set tests ten chosen value constraints, not all 774 preconditions.
"Documented invalid" is a source-reading judgement; quotes are archived with
each row. No defect is claimed without a documented legitimate call.
