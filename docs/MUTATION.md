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
  `full_confirm` is true.
- Source bytes are restored after the run, including on error.
- `--vstd` is accepted for CLI parity with the other analyses; the target
  workspace's build configuration supplies the actual Verus/vstd context.

## Command

```sh
cargo-spec-check analyses-mutation \
  --manifest MUTATIONS.json \
  --cargo-verus /path/to/cargo-verus \
  --z3 /path/to/z3 \
  --vstd /path/to/verus/source \
  --out OUT
```

Outputs:

- `report.json` — baseline focus, per-mutant focus/full results, summary.
- `results.jsonl` — one machine-readable record per mutant.
- `logs/` — captured Verus output per run.

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
