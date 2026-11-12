#!/usr/bin/env bash
# M7b first-station replay: extract -> synthesize -> divergence.
# Paths are the frozen 43da079 ktest-channel workspace and the local Verus tools.
set -euo pipefail
cd "$(dirname "$0")/../.."
BIN=./target/debug/cargo-spec-check
SRC=/tmp/vostd-43da079-ktest-chan/ostd/src
VERUS=/home/jinser/vie/work/AES/vostd/tools/verus/source

"$BIN" extract-ktest \
  --source-root "$SRC" \
  --config examples/vostd-ktest/extract-config.json \
  --out examples/vostd-ktest/extracted-candidates.json \
  --stats examples/vostd-ktest/extract-stats.json

"$BIN" synthesize \
  --candidates examples/vostd-ktest/extracted-candidates.json \
  --recipes examples/vostd-ktest/recipes-m7b.json \
  --out examples/vostd-ktest/divergence-manifest-m7b.json \
  --select K007,K131

timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  "cd '$PWD' && '$BIN' analyses-divergence \
     --manifest examples/vostd-ktest/divergence-manifest-m7b.json \
     --cargo-verus '$VERUS/target-verus/release/cargo-verus' \
     --z3 '$VERUS/z3' \
     --vstd '$VERUS/vstd' \
     --out /tmp/vostd-divergence-m7b-run1"
