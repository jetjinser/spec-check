#!/usr/bin/env bash
set -euo pipefail
SDK="$HOME/vie/work/AES/vostd/tools/verus/source"
REL="$SDK/target-verus/release"
export PATH="$REL:$PATH"
export VERUS_USE_RUSTUP=0
export VERUS_Z3_PATH="$SDK/z3"
export RUSTC_BOOTSTRAP=1
exec "$REL/cargo-verus" check -p ostd --offline --target-dir /tmp/vostd-witness-target --target x86_64-unknown-none --lib --
