# Contract mutation (`analyses-mutation`)

## Purpose

Contract mutation asks a narrow question: **does the verified contract reject
a body that differs from the original in a specified way?** A mutant that still
verifies is a contract-gap candidate. It is not, by itself, an implementation
bug, a runtime counterexample, or a claim about prevalence.

The analysis is deliberately project-neutral. The caller supplies a frozen
target workspace and a manifest of exact snippets; target-specific knowledge
does not live in this repository.

## Manifest

```json
{
  "target_workspace": "/path/to/frozen/checkout",
  "source": "ostd/src/mm/frame/segment.rs",
  "package": "ostd",
  "module": "mm::frame::segment",
  "target": "x86_64-unknown-none",
  "target_dir": "/tmp/spec-check-mutation-target",
  "full_confirm": true,
  "full_target_dir": "/tmp/spec-check-mutation-target/full",
  "timeout_seconds": 900,
  "mutations": [
    {
      "id": "SP-07",
      "description": "Allow a zero split offset",
      "old": "        assert!(0 < offset && offset < self.size());",
      "new": "        assert!(0 <= offset && offset < self.size());"
    }
  ]
}
```

- Every `old` snippet must occur exactly once in the pristine source.
- `module` is used for `cargo-verus focus --verify-module`.
- A focus-verified mutant is full-confirmed with `cargo-verus check` when
  `full_confirm` is true. Full confirmation uses `<target_dir>/full` by default,
  so a cached focused run cannot hide the full verification summary; override
  with `full_target_dir`. Each Verus run has a wall timeout (`timeout_seconds`,
  default 900); timed-out runs are reported as `timeout`, not as survivors.
  On Unix the verifier is put in its own process group and the group is killed,
  so rustc/z3 descendants are stopped with cargo-verus.
- Source bytes are restored after the run, including on error.
- `--vstd` is accepted for CLI parity with the other analyses; the target
  workspace's build configuration supplies the actual Verus/vstd context.

## Command

```sh
cargo-spec-check mutation \
  --manifest MUTATIONS.json \
  --cargo-verus /path/to/cargo-verus \
  --z3 /path/to/z3 \
  --vstd /path/to/verus/source \
  --out OUT [--mode advisory|strict]
```

`analyses-mutation` is kept as a compatibility alias. Advisory is the default;
strict exits non-zero on survivors/timeouts/unknowns without promoting them. Validate a manifest
without running Verus:

```sh
cargo-spec-check mutation --manifest MUTATIONS.json --dry-run
```

Outputs:

- `report.json` — baseline focus, per-mutant focus/full results, summary.
- `results.jsonl` — one machine-readable record per mutant.
- `logs/` — captured Verus output per run.

Each run result also records `timed_out` and `cache_status` (`fresh`, `cached`,
`timeout`, or `unknown`). Use a fresh target directory for an authoritative
full confirmation; a cached cargo run cannot provide a verification summary.

## Interpretation

- `verified_survivor`: both focus and full verification accepted the mutant.
  Candidate gap; requires human intent review against docs, callers, and tests.
- `rejected`: the contract rejected the mutant. This is evidence that the
  contract covers that mutation.
- `focus_verified_full_rejected`: partial verification was insufficient; the
  full check is authoritative.
- `compile_error` / `unknown`: not findings; inspect the log.

Do not promote a survivor to a confirmed defect automatically. Record the
intent source, runtime/test evidence, and any candidate contract repair
separately.


## Generation v1/v2 (`mutation-generate`)

`mutation-generate` is a conservative, function-scoped generator. It extracts
named function bodies with brace matching, applies a frozen set of line-local
operators, and writes an exact-snippet manifest for `analyses-mutation`.

v1.1 filters proof/`assert ... by`/invariant regions and spec markers. v2 also
skips `=>`, `->`, generic, and bit-shift delimiters so syntax is not mutated as
a comparison.

```json
{
  "target_workspace": "/path/to/frozen/checkout",
  "source": "src/file.rs",
  "package": "pkg",
  "module": "mod::path",
  "target": "x86_64-unknown-none",
  "target_dir": "/tmp/mutation-target",
  "full_confirm": true,
  "functions": ["f", "g"],
  "operators": ["return_constant", "literal_shift", "comparison_flip", "boolean_flip", "state_value", "index_zero"],
  "max_mutants": 80
}
```

```sh
cargo-spec-check mutation-generate   --config GENERATE.json --out MUTANTS.json --report GENERATION-REPORT.json
```

The report records generated counts by operator and skipped non-unique snippets.
Generated mutants are not assumed to compile; run `analyses-mutation` to measure
compile/reject/survivor rates. The generator does not claim semantic validity,
reachability, or completeness. The syntax filter improves compileability; the
retrospective benchmark still has exact recall 0/3, so HIR/MIR-guided or
template-directed generation remains future work.


## Generic triage (`mutation-triage`)

`mutation-triage` joins a manifest, its JSONL results, and the pristine source:

```sh
cargo-spec-check mutation-triage \
  --manifest MUTATIONS.json \
  --results OUT/results.jsonl \
  --source PATH/TO/SOURCE \
  --registry OPTIONAL_FINDINGS.json \
  --out OUT/triage.json
```

It classifies proof/spec regions, property templates, registry overlaps,
local test/source intent references, and the confirmation route. It never
promotes a survivor. Project-specific equivalence judgement, runtime evidence,
and contract repair stay project-side.


## Schemas

Versioned contracts live in `schemas/`:

- `mutation.manifest.v1.schema.json`
- `mutation.report.v1.schema.json`
- `mutation.triage.v1.schema.json`

Manifests are validated by the tool. Reports and triage output carry the same
`schema_version` and can be checked with any JSON Schema 2020-12 validator.
Older manifests without `schema_version` default to v1 at runtime.
