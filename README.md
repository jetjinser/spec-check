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
