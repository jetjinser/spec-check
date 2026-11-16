# spec-check

`cargo-spec-check` — a cargo subcommand that checks Verus spec quality:
capture (Rustc HIR/MIR + Verus VIR) a target crate and its tests, register
test-derived demands as obligations, and let Verus decide whether the
contracts imply them — plus generic spec-defect analyses (vacuity, test
divergence, ...) over the captured artifacts.

## Layout

| crate | role |
| --- | --- |
| `spec-check-core` | capture/inventory/check engine (ported from the research lineage) |
| `spec-check-analyses` | pluggable analyses over captured artifacts |
| `cargo-spec-check` | CLI shell |

See `docs/ARCHITECTURE.md`. Vostd-specific knowledge (family configs,
snapshot adaptation patches, ktest gating) lives in project-side inputs,
never in this repository.

## Provenance

The core engine is ported verbatim from
`research/tools/cargo-spec-check` at research revision `5d54c0690208`
so archived campaign evidence keeps replaying bit-for-bit. Internal
re-organisation is incremental and tracked in `docs/ARCHITECTURE.md`.

## Contract mutation

`analyses-mutation` is a controlled contract-adequacy loop. A project-side
manifest supplies exact old/new source snippets; the tool runs focused Verus on
each mutant, full-confirms focus survivors, restores the source, and writes a
report. A `verified_survivor` is a **contract-gap candidate**, never an
automatic bug verdict.

```sh
cargo-spec-check analyses-mutation \
  --manifest MUTATIONS.json \
  --cargo-verus PATH --z3 PATH --vstd PATH --out OUT
```

The generator can produce that manifest from a frozen source file:

```sh
cargo-spec-check mutation-generate   --config GENERATE.json --out MUTATIONS.json --report REPORT.json
```

See `docs/MUTATION.md` for both schemas and evidence boundaries.
