#!/usr/bin/env bash
# Reproduce the M7c recipe-library extension run (11 entries) on an isolated
# copy of the frozen ktest workspace. The frozen source itself is never
# modified; the copy is hashed before and after the run.
set -euo pipefail

SPEC_CHECK="${SPEC_CHECK:-$HOME/vie/work/AES/spec-check}"
FROZEN="${FROZEN:-/tmp/vostd-43da079-ktest-chan}"
COPY="${COPY:-/tmp/vostd-43da079-ktest-chan-b}"
VOSTD="${VOSTD:-$HOME/vie/work/AES/vostd}"
OUT="${OUT:-/tmp/vostd-divergence-m7b-ext-run2}"
SELECT="K145,K146,K147,K148,K149,K150,K151,K152,K153,K012,K014"

if [ ! -d "$COPY" ]; then
  cp -a "$FROZEN" "$COPY"
fi

# Integrity: the copy must be byte-identical to the frozen tree.
(cd /tmp && find "$(basename "$FROZEN")" -type f -print0 | sort -z | xargs -0 sha256sum \
  | sed "s# $(basename "$FROZEN")/# $(basename "$COPY")/#" \
  | sha256sum -c >/tmp/vostd-m7b-ext-copy-check.log)
tail -1 /tmp/vostd-m7b-ext-copy-check.log

# Pre-run snapshot of the copy (restoration check after divergence).
(cd /tmp && find "$(basename "$COPY")" -type f -print0 | sort -z | xargs -0 sha256sum \
  >/tmp/vostd-m7b-ext-before.sha256)

cd "$SPEC_CHECK"
cargo build -p cargo-spec-check

./target/debug/cargo-spec-check synthesize \
  --candidates examples/vostd-ktest/extracted-candidates.json \
  --recipes examples/vostd-ktest/recipes-m7b-ext.json \
  --out examples/vostd-ktest/divergence-manifest-m7b-ext.json \
  --select "$SELECT"

timeout 900 nix develop "$VOSTD/nix" -c bash -c \
  "cd '$SPEC_CHECK' && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-ktest/divergence-manifest-m7b-ext.json \
     --cargo-verus '$VOSTD/tools/verus/source/target-verus/release/cargo-verus' \
     --z3 '$VOSTD/tools/verus/source/z3' \
     --vstd '$VOSTD/tools/verus/source/vstd' \
     --out '$OUT'"

cp "$OUT/report.json" examples/vostd-ktest/divergence-report-m7b-ext.json

# Post-run restoration check: the copy must be byte-identical again.
(cd /tmp && find "$(basename "$COPY")" -type f -print0 | sort -z | xargs -0 sha256sum \
  >/tmp/vostd-m7b-ext-after.sha256)
diff /tmp/vostd-m7b-ext-before.sha256 /tmp/vostd-m7b-ext-after.sha256
echo "workspace restored: byte-identical"
