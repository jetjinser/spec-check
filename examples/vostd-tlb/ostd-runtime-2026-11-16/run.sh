#!/usr/bin/env bash
set -euo pipefail
# Requires /tmp/asterinas-v016 (Asterinas v0.16.0 worktree), the local OSDK
# built from its osdk/ with OSDK_LOCAL_DEV=1, and nightly-2025-02-01.
cd /tmp/asterinas-v016
export PATH="$HOME/.rustup/toolchains/nightly-2025-02-01-x86_64-unknown-linux-gnu/bin:/tmp/osdk-v016/bin:$PATH"
export SHELL="$(command -v bash)"
export CC="$(command -v cc)"
export CXX="$(command -v c++)"
export RUSTFLAGS="-C linker=$(command -v cc)"
export OVMF=off
cd ostd
cargo osdk test tlb_probe_huge_split --boot-method qemu-direct
