# Architecture

## Crates

- `spec-check-core` — the engine. Currently a verbatim port of the research
  lineage (flat module list). Target organisation, applied incrementally:
  - `capture/` — Rustc/Verus invocation, probe build, replay, inventory
  - `scan/` — contract scan, demand census, guard-owner joins
  - `client/` — obligation/client generation
  - `findings/` — typed finding schema and report rendering
- `spec-check-analyses` — pluggable analyses. Each analysis is a generic
  pass over captured artifacts; see the crate docs for the extension recipe.
- `cargo-spec-check` — CLI shell; parsing and dispatch only.

## Rules

1. No target-project knowledge in tool code: Vostd family configs, snapshot
   adaptation patches and ktest gating are project-side inputs.
2. Outputs are deterministic and hash-validated (`changed_inputs` discipline).
3. Pure-logic unit tests must not require a Verus toolchain; toolchain-dependent
   integration tests use small synthetic fixtures under `tests/fixtures`.
4. Engine modules ported verbatim keep their names; renaming/refactoring them
   is done incrementally with replay validation against archived evidence.

## Adding an analysis

See `crates/spec-check-analyses/src/lib.rs` (crate docs) for the recipe.
