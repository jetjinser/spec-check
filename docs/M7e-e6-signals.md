# M7e-E6 — first-class signals for pre-registered expectations

Status: design + pre-registration. Written **before** the E6 implementation, the
full projection re-run, and the Tier C expression extension. The run archive
(`examples/vostd-requires/E6-RUN.md`) reports what actually happened and does
not rewrite this contract or `E6-EXPECTED.md`.

Parent documents:

- `docs/M7e-projection-mapping.md` — projection model schema, relations, and
  the soundness argument for model-relative vacuity evidence.
- `examples/vostd-requires/PROJECTION-EXPECTED.md` — P5 predicted the seven
  VmIo `ConfirmedVacuous` verdicts before the projection run.

## 1. Problem

The projection run found seven `ConfirmedVacuous` entries whose tool-internal
`expected` was `CandidateInconclusive`. The discrepancy was recorded in the
run archive by hand. That is the discovery path for V-003, but it is not a
tool output: a second run cannot reproduce the comparison without reading the
archive and re-doing it manually.

E6 makes the comparison a first-class, deterministic product:

1. the project side may pre-register an expected verdict per entry, with a
   basis, a scope, and an optional registry reference;
2. the runner compares the actual verdict against the pre-registered verdict
   **and** against the tool-internal conservative expectation;
3. every mismatch becomes a structured `signal` with a direction, evidence,
   model-relation gate, and a registry suggestion; the tool never edits the
   registry.

The tool-internal expectation is deliberately conservative: Tier A without a
bounded witness and every Tier B entry are `CandidateInconclusive`. A
pre-registered verdict may be stronger. The comparison against both is what
turns "the conservative rule missed it but the prediction was right" into an
explicit E6 signal, while keeping the pre-registration mismatch as a separate
signal source.

## 2. Non-goals

- No automatic registry mutation. `suggested_registry_action` is a string for
  a human/ledger step; `ledger` remains the only writer.
- No claim that a `ConfirmedLive` is real liveness. Live remains
  model-relative; an `expected Vacuous -> actual Live` signal is an expectation
  falsification, never a proof of reachability.
- No replacement of `expected` by pre-registration. The tool-internal
  expectation remains the regression baseline; pre-registration is an
  independent, project-side prediction.
- No per-entry hard-coding in the tool. The expectations file and the
  projection config are project-side inputs.

## 3. Expectations file (project side)

`screen-requires --expectations FILE` reads:

```json
{
  "schema": "spec-check.expectations/1",
  "entries": {
    "r0351_read_bytes": {
      "expected": "ConfirmedVacuous",
      "basis": "P5: VmIo default fallback; obeys_vmio_read_requires is a faithful crate-scope constant model true, so !obeys is unsat",
      "scope": "crate",
      "registry": "V-003"
    }
  }
}
```

Rules:

- `schema` must be exactly `spec-check.expectations/1`; an unknown schema is
  rejected (no silent fallback).
- `entries` is an object keyed by the screening entry name (`rNNNN_short`).
  A key that does not occur in the current screening is reported as
  `unmatched`, not an error (tier recomputation can move or remove entries).
- `expected` must be one of `ConfirmedLive`, `ConfirmedVacuous`,
  `CandidateInconclusive`, `Anomaly`.
- `basis` is required and must be non-empty: a prediction without a stated
  basis is not a pre-registration.
- `scope` is `crate` or `global`; default `crate`. It is attached to every
  signal. `global` is an explicit project claim and is never inferred.
- `registry` is an optional existing registry id. It is copied to the signal
  suggestion; it is not verified or modified.
- An entry without a pre-registration is not a signal by itself. The runner
  still compares the actual verdict against the tool-internal expectation and
  counts the entry under `expectations.missing`.

## 4. Signal definition and direction

For an entry with a pre-registered expectation `P`, tool expectation `T`, and
actual verdict `A`, the runner considers:

- **pre-registration mismatch** iff `P != A`;
- **tool divergence** iff `T != A`.

Both are E6 candidates. If both hold, one signal is emitted with
`expected_source = "pre-registered"` and `tool_expected` / `tool_match` in the
evidence, so the stronger source is not hidden. If only the tool diverges, the
signal has `expected_source = "tool"` and `preregistered_expected` /
`preregistered_match` are attached when available.

Direction is computed from the expected verdict that produced the signal:

| expected | actual | `direction` | meaning |
|---|---|---|---|
| not `ConfirmedVacuous` | `ConfirmedVacuous` | `vacuous-signal` | the tool/pre-registration expected a satisfiable or unknown precondition; the probe found it unsat |
| `ConfirmedVacuous` | not `ConfirmedVacuous` | `expected-falsified` | the predicted vacuity was not reproduced; do not register it |
| any | `Anomaly` | `anomaly` | runner inconsistency; investigate before drawing a conclusion |
| other mismatch | other mismatch | `verdict-divergence` | e.g. Live vs Inconclusive; expectation needs review |

`Inconclusive` is never a defect by itself; it only participates in a mismatch.

## 5. Soundness gate

A `vacuous-signal` is candidate evidence only when the abstraction is a
relaxation of the real precondition. The tool derives a relation per entry and
attaches it to the signal:

| derived relation | rule | vacuous signal severity |
|---|---|---|
| `exact` | Tier A, no projected clause, no uninterp clause | candidate |
| `faithful` | all used projection models are `faithful`, no structural relaxation | candidate |
| `over-approx` | every used relation is `faithful`/`over-approx`, or the clause uses a structural relaxation (`Ctor` token, `Clip` cast, `/`/`%` over the extended integer domain, or an uncovered `Match` else token), or a non-projected Tier B clause is an `uninterp` predicate over ids | candidate |
| `under-approx` | any used projection model is `under-approx` | info |
| `undeclared` | the entry uses an abstraction the tool cannot classify | info |

Only `candidate` signals carry `suggested_registry_action =
"replicate-and-register"` (or `"confirm-registry:<id>"` when the
pre-registration names an existing registry entry). `expected-falsified` and
`verdict-divergence` are always `info`; `anomaly` is `candidate` with
`"investigate-anomaly"`.

`scope` is `crate` by default. The VmIo relation is crate-scope: a public trait
may have external impls that return `false`, so the seven signals are crate-scope
candidates, not global claims.

## 6. Structured output

`screen-requires` writes the full object to `run/signals.json` and merges the
same fields into `screening.json` and `run/verdicts.json`:

```json
{
  "schema": "spec-check.e6-signals/1",
  "expectations": {
    "provided": true,
    "entries": 723,
    "matched": 723,
    "missing": 0,
    "unmatched": 0
  },
  "counts": {
    "signals": 7,
    "candidates": 7,
    "info": 0,
    "vacuous_signal": 7,
    "expected_falsified": 0,
    "verdict_divergence": 0,
    "anomaly": 0,
    "by_expected_source": {"tool": 7, "pre-registered": 0}
  },
  "signals": [
    {
      "entry": "r0351_read_bytes",
      "function": "ostd::mm::io::VmIo::read_bytes",
      "tier": "A",
      "expected_source": "tool",
      "expected": "ConfirmedInconclusive",
      "actual": "ConfirmedVacuous",
      "direction": "vacuous-signal",
      "severity": "candidate",
      "preregistered_expected": "ConfirmedVacuous",
      "preregistered_match": true,
      "tool_expected": "CandidateInconclusive",
      "tool_match": false,
      "basis": "P5: ...",
      "model_relation": "faithful",
      "scope": "crate",
      "vacuum_evidence": true,
      "suggested_registry_action": "confirm-registry:V-003",
      "registry": "V-003",
      "evidence": {
        "span": "ostd/src/mm/io.rs:1309:5: 1310:19 (#0)",
        "witness": null,
        "requires": "p351_k0()",
        "detail": "vacuous_probe_failed=false",
        "projection_relations": {"faithful": 1, "over-approx": 0, "under-approx": 0, "structural_relaxed_clauses": 0}
      }
    }
  ]
}
```

Determinism: entries are sorted by entry name; counts use `BTreeMap`; the
`signals` array is stable across runs.

## 7. Interaction with Tier C expression extension

E6 is computed after the Tier C expression extension. Newly upgraded entries
have no pre-registration by default; they are compared against the tool
expectation, counted as `expectations.missing`, and produce a signal only on
divergence. `E6-EXPECTED.md` pre-registers the directional prediction (no new
`ConfirmedVacuous`) and the expected before/after tiers before the extension
probe run. The extension must not silently create a pre-registration after
seeing a verdict.

## 8. Pre-registered expectations (written before implementation/run)

| id | situation | expected |
|---|---|---|
| E6-1 | full projection re-run, 723 entries, `--expectations e6-expectations.json` | exactly 7 signals, all `vacuous-signal`, all entries from the VmIo set, all `expected_source=tool`, `preregistered_match=true`, `model_relation=faithful`, `severity=candidate` |
| E6-2 | the other 716 entries | no signal (`0`), whether compared against tool or pre-registration |
| E6-3 | expectations file with a pre-registered `ConfirmedVacuous` and actual `ConfirmedLive` | `direction=expected-falsified`, `severity=info`; no registry suggestion |
| E6-4 | `under-approx` projection and actual `ConfirmedVacuous` | signal emitted but `severity=info`, `vacuum_evidence=false`, suggestion `review-expectation` |
| E6-5 | entry missing from the expectations file and tool expectation diverges | signal still emitted with `expected_source=tool`, `preregistered_expected=null`, counted in `expectations.missing` |
| E6-6 | unknown schema / invalid verdict / missing basis | expectations load error, no silent fallback |
| E6-7 | same input twice | byte-identical `signals` object |
| E6-8 | Tier C extension | 0 predicted new `ConfirmedVacuous`; if one appears with a sound relation, replicate before registering; never auto-write the registry |
