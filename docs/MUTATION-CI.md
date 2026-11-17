# Mutation in CI (advisory example)

Use mutation as an **advisory** job first. It reports candidates; it does not
automatically confirm defects.

```yaml
name: mutation-advisory
on: [pull_request]

jobs:
  mutation:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install spec-check
        run: cargo install --path crates/cargo-spec-check
      - name: Run mutation audit
        run: |
          cargo-spec-check mutation run \
            --manifest mutation.manifest.json \
            --cargo-verus "$CARGO_VERUS" \
            --z3 "$VERUS_Z3" \
            --vstd "$VSTD" \
            --out mutation-out \
            --mode advisory
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: mutation-report
          path: mutation-out/
```

`advisory` exits 0 and keeps the report. A project can later choose
`--mode strict` to fail on unhandled survivors/timeouts/unknowns, but that is a
policy decision and still does not promote a candidate to a confirmed bug.

Do not make low-impact candidates blocking. Route confirmed high/critical gaps
through human triage and the project's repair process.
