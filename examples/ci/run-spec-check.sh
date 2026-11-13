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
#     census/           census output (census.json); only when SPEC_CHECK_CENSUS=1
#     census-vir/       VIR export input (crate.vir, ~100 MB); same condition
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
#   SPEC_CHECK_CENSUS=1    optional census pass (default off). Exports VIR with
#                          `cargo dv focus --targets ... -- --no-verify --log
#                          vir` (minutes, ~100 MB) and runs
#                          `cargo-spec-check census` into CHECK_OUT/census/.
#                          Census output is facts, never a failure verdict; a
#                          missing export capability or VIR is a pipeline error.
#   CENSUS_CRATE           crate name passed to census --crate; default: ostd.
#   CENSUS_TARGET          dv target to export; default: $CENSUS_CRATE.
#   CENSUS_VIR             existing VIR file; when set, export is skipped.
#   CENSUS_VIR_DIR         export destination (cleared first); default:
#                          $CHECK_OUT/census-vir. Point outside CHECK_OUT to
#                          keep the artifact small.
#   CENSUS_SOURCE_ROOT     optional census --source-root (relative file paths).
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
SPEC_CHECK_CENSUS="${SPEC_CHECK_CENSUS:-0}"
CENSUS_CRATE="${CENSUS_CRATE:-ostd}"
CENSUS_TARGET="${CENSUS_TARGET:-$CENSUS_CRATE}"
CENSUS_VIR_DIR="${CENSUS_VIR_DIR:-$CHECK_OUT/census-vir}"

if [[ "$SPEC_CHECK_CENSUS" == "1" && -n "${CENSUS_VIR:-}" ]]; then
  [[ -f "$CENSUS_VIR" ]] || fail "CENSUS_VIR does not exist: $CENSUS_VIR"
  CENSUS_VIR="$(readlink -f "$CENSUS_VIR")"
fi

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
INNER=$(cat <<'SPEC_CHECK_INNER'
set -euo pipefail
cargo build --manifest-path "$SPEC_CHECK_DIR/Cargo.toml" ${SPEC_CHECK_CARGO_FLAGS:-}
cargo fetch --manifest-path "$CAPTURE_MANIFEST"

# Optional census: pure VIR analysis, not a check analysis. It needs a VIR
# export (minutes, ~100 MB) and therefore stays opt-in; findings are facts.
if [[ "${SPEC_CHECK_CENSUS:-0}" == "1" ]]; then
  export VERUS_USE_RUSTUP="${VERUS_USE_RUSTUP:-0}"
  export VERUS_Z3_PATH="${VERUS_Z3_PATH:-$VERUS_SDK/z3}"
  export RUSTC_BOOTSTRAP="${RUSTC_BOOTSTRAP:-1}"
  if [[ -n "${CENSUS_VIR:-}" ]]; then
    census_vir="$CENSUS_VIR"
  else
    if [[ ! -f "$VOSTD_DIR/dv/Cargo.toml" ]] \
      || ! grep -Eq '^[[:space:]]*dv[[:space:]]*=' "$VOSTD_DIR/.cargo/config.toml" 2>/dev/null; then
      printf '%s\n' \
        "run-spec-check: SPEC_CHECK_CENSUS=1 needs VIR export, but '$VOSTD_DIR' has no 'cargo dv' alias (dv submodule missing)." \
        "  Export once, then rerun with CENSUS_VIR=<dir>/crate.vir:" \
        "    (cd $VOSTD_DIR && cargo dv focus --targets $CENSUS_TARGET -- --no-verify --log vir --log-dir <dir>)" >&2
      exit 2
    fi
    if [[ -z "$CENSUS_VIR_DIR" || "$CENSUS_VIR_DIR" == "/" ]]; then
      echo "run-spec-check: refusing to clear CENSUS_VIR_DIR='$CENSUS_VIR_DIR'" >&2
      exit 2
    fi
    rm -rf "$CENSUS_VIR_DIR"
    mkdir -p "$CENSUS_VIR_DIR"
    CENSUS_VIR_DIR="$(readlink -f "$CENSUS_VIR_DIR")"
    ( cd "$VOSTD_DIR" && cargo dv focus --targets "$CENSUS_TARGET" -- \
        --no-verify --log vir --log-dir "$CENSUS_VIR_DIR" )
    census_vir="$CENSUS_VIR_DIR/crate.vir"
    if [[ ! -f "$census_vir" ]]; then
      printf '%s\n' \
        "run-spec-check: VIR export produced no $census_vir." \
        "  Use a fresh CENSUS_VIR_DIR (cargo may skip writing an unchanged crate), or pass CENSUS_VIR=<existing crate.vir>." >&2
      exit 2
    fi
  fi
  census_tmp="$CHECK_OUT/.census.tmp"
  rm -rf "$census_tmp"
  census_args=(--vir "$census_vir" --crate "$CENSUS_CRATE" --out "$census_tmp")
  if [[ -n "${CENSUS_SOURCE_ROOT:-}" ]]; then
    census_args+=(--source-root "$CENSUS_SOURCE_ROOT")
  fi
  if ! "$SPEC_CHECK_DIR/target/debug/cargo-spec-check" census "${census_args[@]}"; then
    rm -rf "$census_tmp"
    exit 2
  fi
  rm -rf "$CHECK_OUT/census"
  mv "$census_tmp" "$CHECK_OUT/census"
fi

exec "$SPEC_CHECK_DIR/target/debug/cargo-spec-check" check \
  --project "$CHECK_OUT/project.json" \
  --cargo-verus "$VERUS_SDK/target-verus/release/cargo-verus" \
  --z3 "$VERUS_SDK/z3" \
  --vstd "$VERUS_SDK/vstd" \
  --out "$CHECK_OUT/check" ${SPEC_CHECK_JSON:+--json}
SPEC_CHECK_INNER
)

export SPEC_CHECK_DIR CHECK_OUT VERUS_SDK CAPTURE_MANIFEST \
  SPEC_CHECK_CARGO_FLAGS SPEC_CHECK_JSON \
  SPEC_CHECK_CENSUS CENSUS_CRATE CENSUS_TARGET CENSUS_VIR CENSUS_VIR_DIR \
  CENSUS_SOURCE_ROOT VOSTD_DIR

if [[ "${SPEC_CHECK_NO_NIX:-0}" == "1" ]] || ! command -v nix >/dev/null 2>&1; then
  bash -c "$INNER"
else
  nix develop "$VOSTD_DIR/nix" -c bash -c "$INNER"
fi

printf 'run-spec-check: done. report: %s/check/report.json (artifact dir %s)\n' \
  "$CHECK_OUT" "$CHECK_OUT"
