# M8 — Frame conditions: static census and caller probes

Status: design + pre-registration (written before implementation and before runs).

## Question

A contract can describe the return value and the final state of its `&mut`
outputs, yet say nothing about the rest of the state it is allowed to change.
Callers then cannot rely on preservation: after `push(x)`, the contract does not
promise that earlier elements are unchanged. This milestone asks two questions.

1. Static: for each function, does its effective postcondition contain an
   explicit frame clause (`final(e) == old(e)` / `old(e) == final(e)`, including
   quantified forms of the same equality)?
2. Probe: for selected high-impact interfaces with a mutable output and no frame
   clause, can a caller prove a plausible preservation fact after the call?

The static part is a fact layer: absence of a frame clause is a fact, not a
defect. The probe part produces `FrameGapCandidate` (`assertion failed` on the
preservation fact with a clean baseline) or `FrameSupported`.

## Definitions (fixed before implementation)

- **Effective postcondition**: own regular `ensure` clauses plus, for a trait
  impl, the declaration's regular `ensure` clauses. `default_ensures` and
  `returns` are excluded.
- **Old/final marker**: a VIR expression of the form
  `Unary (UnaryOp MutRefFuture (MutRefFutureSourceName Old|Final)) e`.
- **Frame equality**: a `Binary (BinaryOp Eq ...)` clause whose two sides,
  after removing all `Old` and `Final` markers, render identically under the
  census compact renderer, and where at least one side contains an `Old` marker.
  This covers `final(e) == old(e)`, `old(e) == final(e)`, and the same equality
  under a quantifier or implication.
- **Mutable-output function**: a function with at least one `&mut` parameter
  (`Typ MutRef`, including `Tracked<&mut _>` / `Ghost<&mut _>` and `with`
  parameters), as already counted by census.
- **Frame status** per function: `explicit` (>= 1 frame equality), `absent`
  (has a postcondition but no frame equality), `no_post` (no effective
  postcondition). `mut_absent` is the subset of `absent` with a mutable output.

## Static output

`census.json` gains an additive `frame` object per function:
`clauses`, `old_mentions`, `final_mentions`, `mut_params`, `status`; and
`counts.frame` aggregates the four status buckets plus `mut_absent`. The schema
string stays `spec-check.census/1` (additive fields, precedent: M7e).

## Probe protocol

Selection: mutable-output functions with `frame.status = absent`, restricted to
`pub` interfaces on the `ostd` surface, ranked by call-surface size (number of
`&mut` outputs, public visibility, presence of doc comments claiming
preservation). Ten are chosen and listed with the asserted fact in
`examples/vostd-frame/EXPECTED.md` before any probe run.

Each probe is an obligation client in a divergence-style manifest: set up the
receiver, call the function, then assert a preservation fact about state that
the callee is not supposed to touch. Expected labels are registered per probe
(`gap` or `supported`). Classification is the existing divergence classifier:
`assertion failed` in the obligation function with a clean baseline = gap;
verified = supported; compile failure or unrelated errors =
`Inconclusive` with the exact reason.

Model scope: the probe asserts one concrete preservation fact expressible on
the frozen surface. A gap means the current contract does not prove that fact;
it does not claim the implementation violates it, and a supported result does
not claim frame completeness.

## Cost and stopping rules

Two verification runs at most (one batch for all ten clients; one retry only if
a probe fails to compile for a non-judgemental reason, recorded before the
retry). If fewer than ten interfaces admit an expressible preservation fact,
run the expressible subset and record the exact blockers for the rest.

## Reuse

- `spec-check-core::census` helpers (`ensure_parts`, `render_compact`).
- `spec-check-analyses::divergence` runner and classifier.
- Frozen workspace `/tmp/vostd-43da079-ktest-chan` (models/receipts used by
  earlier divergence runs).

## Results (2026-11-15)

Static (1,468 functions): 147 explicit frame equalities (456 clauses), 1,121
absent, 200 no postcondition; 216 mutable-output functions are absent, but only
5 public ones have neither `old` nor `final` anywhere in their postcondition.
The syntactic signal is therefore weak as a gap measure: most contracts express
preservation through update equalities or `r.*` reborrow clauses.

Probes (10, `examples/vostd-frame/`): 9 `ObligationSupported`, 1
`ContractGapCandidate` (`VmWriter::limit` does not preserve `wf(owner)`, a
call-site precondition failure), 0 `Inconclusive`. Two predictions were wrong:
lock-guard `deref_mut` preservation is derivable from the two-state reborrow
contract (predicted gap), and the failed-`write_once` probe hit the owner `wf`
relation instead of the intended state equality. Both are recorded, not
rewritten. See `examples/vostd-frame/RUN.md`.
