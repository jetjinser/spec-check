#!/usr/bin/env bash
# spec-check CI/nightly entry point — the same command locally and in CI.
#
#   VOSTD_DIR=~/vie/work/AES/vostd examples/ci/run-spec-check.sh
#
# Enters the vostd nix devshell (unless disabled), builds cargo-spec-check,
# runs `cargo-spec-check check` against the vostd workspace, and collects
# everything an artifact store needs under CHECK_OUT:
#
#   CHECK_OUT/
#     project.json      generated check CONFIG (absolute paths, reproducible)
#     capture.json      capture profile plan (generated default or override)
#     capture/          project-capture output (report.json, changed_inputs)
#     check/            check output (report.json, analyses/<name>/...)
#     run.log           full stdout/stderr of this run
#
# Exit-code contract mirrors `cargo-spec-check check`: only pipeline
# failures (config, capture, tool, unknown analysis) are non-zero;
# verdicts are findings, never failures.
#
# Environment variables:
#   VOSTD_DIR              (required) vostd checkout with the Verus SDK
#                          bootstrapped (`cargo dv bootstrap` in vostd).
#   SPEC_CHECK_DIR         spec-check checkout; default: this script's repo.
#   VERUS_SDK              Verus SDK dir (cargo-verus/, z3, vstd/);
#                          default: $VOSTD_DIR/tools/verus/source.
#   CHECK_OUT              artifact directory; default: target/spec-check-ci.
#   CAPTURE_CONFIG         capture profile plan JSON; default: generated.
#   CAPTURE_MANIFEST       capture target workspace manifest;
#                          default: $VOSTD_DIR/Cargo.toml.
#   ANALYSIS_MANIFEST      vacuity manifest; default: the vostd-guard
#                          shape manifest (examples/vostd-guard/manifest.json).
#   ANALYSIS_NAME          registry name in CONFIG; default: vacuity.
#   DIVERGENCE_MANIFEST    optional second analysis (registry name:
#                          divergence); when set, CONFIG declares both
#                          analyses and they run sequentially in one check.
#   SPEC_CHECK_NO_NIX=1    run in the current environment, skip nix develop.
#   SPEC_CHECK_CARGO_FLAGS extra cargo build flags (e.g. --offline).
#   SPEC_CHECK_JSON=1      pass --json to the check command.
set -euo pipefail

fail() { printf 'run-spec-check: %s\n' "$1" >&2; exit 2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEC_CHECK_DIR="${SPEC_CHECK_DIR:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
VOSTD_DIR="${VOSTD_DIR:?run-spec-check: set VOSTD_DIR to a vostd checkout}"
VOSTD_DIR="$(readlink -f "$VOSTD_DIR")"
SPEC_CHECK_DIR="$(readlink -f "$SPEC_CHECK_DIR")"
VERUS_SDK="${VERUS_SDK:-$VOSTD_DIR/tools/verus/source}"
VERUS_SDK="$(readlink -f "$VERUS_SDK")"
CHECK_OUT="${CHECK_OUT:-$SPEC_CHECK_DIR/target/spec-check-ci}"
CAPTURE_MANIFEST="${CAPTURE_MANIFEST:-$VOSTD_DIR/Cargo.toml}"
ANALYSIS_MANIFEST="${ANALYSIS_MANIFEST:-$SPEC_CHECK_DIR/examples/vostd-guard/manifest.json}"
ANALYSIS_NAME="${ANALYSIS_NAME:-vacuity}"

mkdir -p "$CHECK_OUT"
exec > >(tee "$CHECK_OUT/run.log") 2>&1

for path in \
  "$VERUS_SDK/target-verus/release/cargo-verus" \
  "$VERUS_SDK/z3" \
  "$VERUS_SDK/vstd" \
  "$CAPTURE_MANIFEST" \
  "$ANALYSIS_MANIFEST"; do
  [[ -e "$path" ]] || fail "missing $path (bootstrapped SDK needs 'cargo dv bootstrap' in vostd)"
done
[[ -z "${DIVERGENCE_MANIFEST:-}" || -e "$DIVERGENCE_MANIFEST" ]] \
  || fail "missing $DIVERGENCE_MANIFEST"
[[ -d "$VOSTD_DIR/nix" ]] || fail "$VOSTD_DIR/nix missing (flake devshell expected)"

CAPTURE_CONFIG_FILE="${CAPTURE_CONFIG:-$CHECK_OUT/capture.json}"
if [[ -n "${CAPTURE_CONFIG:-}" ]]; then
  CAPTURE_CONFIG_FILE="$(readlink -f "$CAPTURE_CONFIG")"
else
  cat > "$CAPTURE_CONFIG_FILE" <<'EOF'
{
  "profiles": [
    {
      "name": "ostd-nightly",
      "package": "ostd",
      "target": "x86_64-unknown-none",
      "tests": false,
      "features": [],
      "default_features": true,
      "cfgs": []
    }
  ]
}
EOF
fi

ANALYSES_JSON="{\"name\": \"$ANALYSIS_NAME\", \"manifest\": \"$ANALYSIS_MANIFEST\"}"
if [[ -n "${DIVERGENCE_MANIFEST:-}" ]]; then
  DIVERGENCE_MANIFEST="$(readlink -f "$DIVERGENCE_MANIFEST")"
  ANALYSES_JSON="$ANALYSES_JSON, {\"name\": \"divergence\", \"manifest\": \"$DIVERGENCE_MANIFEST\"}"
fi

cat > "$CHECK_OUT/project.json" <<EOF
{
  "capture": {
    "config": "$CAPTURE_CONFIG_FILE",
    "manifest_path": "$CAPTURE_MANIFEST",
    "verus_root": "$VERUS_SDK/target-verus/release",
    "out": "$CHECK_OUT/capture"
  },
  "analyses": [
    $ANALYSES_JSON
  ]
}
EOF

# Warm the cargo registry before project-capture: it resolves the target
# workspace with `cargo metadata --offline`.
INNER='
set -euo pipefail
cargo build --manifest-path "$SPEC_CHECK_DIR/Cargo.toml" ${SPEC_CHECK_CARGO_FLAGS:-}
cargo fetch --manifest-path "$CAPTURE_MANIFEST"
exec "$SPEC_CHECK_DIR/target/debug/cargo-spec-check" check \
  --project "$CHECK_OUT/project.json" \
  --cargo-verus "$VERUS_SDK/target-verus/release/cargo-verus" \
  --z3 "$VERUS_SDK/z3" \
  --vstd "$VERUS_SDK/vstd" \
  --out "$CHECK_OUT/check" ${SPEC_CHECK_JSON:+--json}
'

export SPEC_CHECK_DIR CHECK_OUT VERUS_SDK CAPTURE_MANIFEST \
  SPEC_CHECK_CARGO_FLAGS SPEC_CHECK_JSON

if [[ "${SPEC_CHECK_NO_NIX:-0}" == "1" ]] || ! command -v nix >/dev/null 2>&1; then
  bash -c "$INNER"
else
  nix develop "$VOSTD_DIR/nix" -c bash -c "$INNER"
fi

printf 'run-spec-check: done. report: %s/check/report.json (artifact dir %s)\n' \
  "$CHECK_OUT" "$CHECK_OUT"
