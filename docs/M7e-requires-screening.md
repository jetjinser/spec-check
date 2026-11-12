# M7e — census-driven `requires` screening (design)

Status: implemented in `spec-check-core::requires` + `cargo-spec-check
screen-requires`; first station archived under `examples/vostd-requires/`.
Pre-registration and run archive were written before the probe run.
Tier C modeling (generics/references/ghost) is a separate opt-in extension:
`docs/M7e-tierc-modeling.md` (`--config`, default off).

## 1. Goal and boundary

M7a captures per-function VIR facts (path/mode/visibility/source location/
external_body/trait/post sources/requires preview) into a census. M7e turns
that census into a **census-wide** screening of the `requires` side of
contracts, replacing the hand-curated 60-interface list of M1:

- enumerate every Exec/Proof function in the crate;
- decide whether its precondition can be probed, and if so, generate a
  quantifier probe;
- run the existing `vacuity` runner unchanged;
- report the funnel and every non-screened function with a reason.

It remains an **offline** step. It is not part of the nightly `check` flow:
generation needs only the census, but execution builds a probe crate and
therefore follows the `analyses-vacuity` budget discipline.

## 2. Census additions (M7e)

`census.json` keeps schema `spec-check.census/1`; M7e adds three additive
per-function fields (existing consumers are unaffected):

- `generics`: `{count, names}` from VIR `typ_params`.
- `params`: ordered `{name, type, kind, mode}` with `kind` one of `int`,
  `bool`, `char`, `ghost`, `reference`, `opaque`.
- `requires`: one record per clause with `span`/`file`/`line`, optional
  source `text` (sliced via `--source-root`+VIR spans), `class`
  (`mechanical`/`unsupported`), a structured `ast` for mechanical clauses,
  `vars`/`consts`/`calls`, and feature flags (`call`, `field_access`,
  `quantifier`, `old_or_final`, `unary_opr`, `ctor`, `trigger`).

`mechanical` is a fact about translatability, not a judgement: the clause is
a boolean expression over arithmetic/comparisons/boolean operators whose free
variables are all scalar (integer/bool) parameters. Everything else is
`unsupported`; the original text/span is preserved for the report.

Census canonicalization still applies: when both `p::f` and
`p::_VERUS_VERIFIED_f` exist, the verified body wins. This matters in
practice: `requires false` frequently appears on the *unverified wrapper*
record while the verified twin carries the real contract. Screening therefore
reads the canonical (verified) record only — wrapper `false` markers are not
reported as vacuity.

## 3. Screening tiers

Referenced parameters are the only ones that can constrain the
precondition; an unreferenced `&mut`/`Tracked` parameter is irrelevant.

| Tier | Condition | Action | Expected verdict |
|---|---|---|---|
| none | no `requires` clauses | not a candidate; empty conjunction is trivially satisfiable | — |
| A | no blocker and **all** clauses mechanical | generate one quantifier probe; bounded witness search | `ConfirmedLive` if a witness exists; otherwise `CandidateInconclusive` (a `ConfirmedVacuous` here is an E6 true positive and must be replicated) |
| B | no blocker but ≥1 `unsupported` clause | generate probe; unsupported clauses become named `uninterp spec fn`s | `CandidateInconclusive` (verifier incompleteness); a `ConfirmedVacuous` here is still a machine result and is reported/replicated |
| C | blocker present | no probe; record reason(s) | **not** "no defect" |

Blockers (Tier C), evaluated over *referenced* parameters only:

- `generic:<names>` — the function has type parameters. (Conservative: a
  purely scalar clause over a generic function is not screened.)
- `ghost_param:<name>` — a referenced `Tracked<...>`/`Ghost<...>` parameter;
  abstracting it to an id destroys the resource/permission meaning.
- `reference_param:<name>` — a referenced `&`/`&mut` parameter; alias/deref
  semantics are not modelled by an id.

Tier C is recorded per function in `screening.json` with reasons, clause
count and source span. Its size is reported explicitly; it is never folded
into "no defect".

### Non-screening reason classes (reported separately)

- `no_requires`: precondition is an empty conjunction.
- `generic`, `ghost_param`, `reference_param` (Tier C blockers).
- `uninterp` (Tier B): generated, but the verdict is expected to be
  inconclusive.
- `not_selected` (batch/filter/limit): generated candidate outside the
  current batch.

## 4. Generation rules

1. **Parameters.** Only parameters referenced by at least one clause are
   bound. Integers/bools keep their kind (`usize`/`bool`); `char` and opaque
   by-value types become `<name>_id: usize` (M1 discipline). Ghost, reference
   and generic parameters never reach this point (Tier C).
2. **Mechanical clauses.** Each clause becomes a named
   `pub open spec fn p<idx>_k<k>(...) -> bool { ... }` so quantifier triggers
   are inferable. Rendering is from the VIR AST (no source parsing):
   comparisons, `==`/`!=`, `+ - *`, `&& || ==>`, `!`, chained comparisons.
   Division/modulo and bitwise operators are deliberately excluded
   (unsupported) to avoid semantics drift.
3. **Unsupported clauses.** The whole clause becomes
   `pub uninterp spec fn p<idx>_k<k>(args) -> bool;` with the referenced
   parameter ids/constants kept as arguments (shape preserved, body
   abstracted). Zero-argument clauses use a zero-argument spec fn.
4. **Constants.** VIR `ConstVar` paths are lifted to fresh `usize` probe
   parameters and are deduplicated across clauses. This is an abstraction:
   the probe asks satisfiability *for some* value of the constant, which is
   conservative for vacuity (it can miss a vacuous obligation but cannot
   create a false vacuity).
5. **Witness search.** Tier A only: bounded DFS over `0,1,...,8,15,16,31,32,
   63,64,255,256,4096` for integers and `{false,true}` for bools, with
   partial-constraint pruning and a node cap. A found witness is emitted as
   the tuple matching the parameter order; it must still pass the probe's
   `assert(pre(witness))` in Verus. No witness means no witness is
   *claimed*, not that none exists.
6. **Naming and determinism.** Entry names are `r<global census index>_<short
   path>`; spec fn names are `p<index>_k<clause>`. Census order is sorted by
   path, so names and batches are deterministic across runs.
7. **Visibility / trait / external_body.** These are carried as `origin`
   metadata. They do not block screening: a `pub` or private item, a trait
   impl, or an `external_body` item still has a precondition. Trait
   *declaration* merging is irrelevant on the requires side (VIR `:require`
   is the function's own).
8. **Manifest compatibility.** Entries use `probe: quantifier`; the extra
   `origin` field is ignored by the runner's serde contract. The vacuity
   runner and classifier are unchanged.

## 5. Cost control

- Generation is pure and takes seconds on a 98 MB VIR census.
- `screen-requires` always writes `manifest.json` + `screening.json`;
  execution happens only when `--cargo-verus --z3 --vstd` are all provided.
- Selection: `--tier A,B` (default), `--filter SUBSTRING`, `--limit N`,
  `--batch K --batch-size N` (0-based contiguous batch over the
  deterministic candidate list). Global index names stay stable across
  batches.
- Recommended budget: calibrate with Tier A (small), then batches of
  `--batch-size 45`–`50` under a 900 s wall budget. Each batch is an
  independent probe crate.
- Incremental reruns: census is the cache (seconds); probe runs are keyed by
  batch manifest. No result cache is shared across batches yet (future work).

## 6. CLI and integration

```
cargo-spec-check screen-requires \
  --census DIR/census.json --out DIR \
  [--tier A,B] [--filter SUBSTRING] [--limit N] \
  [--batch K] [--batch-size N] \
  [--cargo-verus PATH --z3 PATH --vstd PATH]
```

Outputs:

- `manifest.json` — selected quantifier entries (vacuity manifest v1.1).
- `screening.json` — funnel counts, Tier C reasons, per-function rows,
  selected entries with pre-registered expectations.
- `run/report.json`, `run/verdicts.json`, `run/probe-build.log` — only when
  tools are given. `verdicts.json` joins actual verdicts with the
  pre-registered expectation.

Relation to `check`: `screen-requires` is an offline analysis. A future
nightly integration would run census (`--source-root` for clause text) then
one or more `screen-requires` batches; it must not run inside the default
`check` flow until probe-budget and artifact-layout decisions are made.

## 7. Honest limits

- Tier C is large on vostd: most spec lemmas are generic (`C`/`M`) and/or
  take `&self`. 676/774 requires-bearing functions are unscreened. This is a
  coverage gap, not evidence of quality.
- Tier B verdicts are mostly `CandidateInconclusive` by construction;
  uninterpreted predicates admit neither a liveness witness nor a vacuity
  proof. They are not defects.
- Tier A uses a bounded witness domain; "no witness" is not a proof of
  unsatisfiability.
- Constants are lifted. References/ghosts/generics are not modelled by
  default; the opt-in M7e-ext config abstracts a safe subset to Tier B
  (`docs/M7e-tierc-modeling.md`).
- The probe model is not the real implementation; a `ConfirmedVacuous`
  means the *transcribed precondition shape* is unsatisfiable.
- Generator controls (`examples/vostd-requires/controls/`): zero-argument
  `false` → `ConfirmedVacuous`, satisfiable bool → `ConfirmedLive`,
  contradictory bool → `ConfirmedVacuous`; 3/3 as expected.
