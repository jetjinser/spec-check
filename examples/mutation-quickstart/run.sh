#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BIN=${CARGO_SPEC_CHECK:-cargo-spec-check}
OUT=${TMPDIR:-/tmp}/spec-check-quickstart-$$
mkdir -p "$OUT"
cat > "$OUT/config.json" <<JSON
{
  "target_workspace": "$ROOT/fixture",
  "source": "src/lib.rs",
  "package": "quickstart",
  "module": "quickstart",
  "target": null,
  "target_dir": "$OUT/target",
  "full_confirm": true,
  "functions": ["f"],
  "operators": ["return_constant"],
  "max_mutants": 5,
  "timeout_seconds": 30
}
JSON
"$BIN" mutation generate --config "$OUT/config.json" --out "$OUT/manifest.json" --report "$OUT/generation.json"
"$BIN" mutation run --manifest "$OUT/manifest.json" --cargo-verus "$ROOT/fake-verus.sh" --z3 /nonexistent/z3 --vstd "$ROOT" --out "$OUT/run"
"$BIN" mutation triage --manifest "$OUT/manifest.json" --results "$OUT/run/results.jsonl" --source "$ROOT/fixture/src/lib.rs" --out "$OUT/triage.json"
echo "quickstart output: $OUT"
