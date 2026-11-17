# Mutation quickstart

This example is intentionally independent of Vostd. It exercises
`mutation generate`, `mutation run`, and `mutation triage` on a tiny source
file. The verifier is a fake script, so no Verus installation is needed.

## Run

From the spec-check repository root:

```sh
cargo install --path crates/cargo-spec-check
./examples/mutation-quickstart/run.sh
```

Or, without installing:

```sh
cargo build -p cargo-spec-check
CARGO_SPEC_CHECK=target/debug/cargo-spec-check \
  ./examples/mutation-quickstart/run.sh
```

The fake verifier always returns `verification results:: 1 verified, 0 errors`;
the point is to show the data flow, not to verify a contract.

## Real use

Replace `fake-verus.sh` with a real `cargo-verus` binary and provide the actual
`--z3` and `--vstd` paths. A real run needs a Verus-compatible workspace and
the project-specific manifest. A survivor is a candidate, not a confirmed
defect; see `docs/MUTATION.md`.
