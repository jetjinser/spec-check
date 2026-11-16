# P4: can the frozen Verus artifact be booted? (2026-11-16)

Timeboxed probe. Result: **architectural blocker**, no bootable frozen artifact.

## Toolchain inventory

- Verus: `0.2026.10.08.d128dda`, profile release, platform linux_x86_64,
  toolchain `1.98.1-x86_64-unknown-linux-gnu`.
- Rust: `1.98.1` (installed via rustup; frozen `rust-toolchain.toml`).
- Frozen tree: `/tmp/vostd-43da079` (vostd 0.16.0).
- `vstd` rlib exists under the Verus SDK and in the frozen tree's
  `tools/verus/source/vstd/target`.
- `rust_verify --compile` exists; `cargo verus build` verifies and then runs
  rustc.

## What works

Frozen `ostd` is a Verus library and builds for the bare-metal target:

```
cargo verus build -p ostd --offline --target x86_64-unknown-none
# Finished dev profile ... in 2m 20s
# target/x86_64-unknown-none/debug/libostd.rlib
```

The first attempt without `--target x86_64-unknown-none` failed on
`acpi` (target-specific dependency), which is expected; the target build
succeeds.

## Why it is still not a bootable artifact

1. **No kernel entry / boot glue in the verified crate.** `ostd/src/lib.rs`
   exports `boot`, `mm`, `task`, etc., but the runtime-facing pieces a kernel
   needs are commented out: `prelude`, `panic`, `logger`, `console`, `io`,
   `smp`, and the `ostd_macros` re-exports (including `main`/`panic_handler`).
   The `ktest` module is inside a commented block (lines ~189–199).
2. **The OSDK test kernel needs exactly those pieces.**
   Asterinas v0.16.0 `osdk/deps/test-kernel/src/lib.rs` imports
   `ostd::ktest::{...}`, `ostd::panic::begin_panic`, `ostd::prelude::abort`,
   and uses `#[ostd::ktest::main]`. None of these exist in the frozen lib.
3. **Re-enabling them would change the artifact.** Restoring runtime modules
   and macros would be new code, not rev 43da079; the result could not be cited
   as the frozen verified artifact.
4. **Toolchain/ABI split.** Frozen code is built with rustc 1.98.1; OSDK
   0.16.0 builds its test kernel with nightly-2025-02-01 (1.86). A Verus rlib
   cannot be linked into that kernel's metadata/ABI; compiling the whole kernel
   with 1.98.1 would require restoring the missing runtime surface above.
5. **No OSDK integration in the frozen tree.** The rewritten tree has no
   `osdk/`/`OSDK.toml`; the OSDK path is an external runtime codebase.

## Conclusion

The frozen crate can be verified and compiled as a library, but it is a
verification fork, not a bootable kernel. Running it in QEMU requires
restoring code outside the frozen revision (and changing toolchains), which
would invalidate the "frozen verified artifact" claim. This is a genuine
architectural blocker, not a missing command.

Evidence boundaries unchanged: the QEMU harness and the Asterinas v0.16.0
ktest remain hardware/runtime evidence; neither is the frozen Verus artifact.

## Commands

```sh
nix shell nixpkgs#rustup nixpkgs#gcc -c bash -c '
  REL=$HOME/vie/work/AES/vostd/tools/verus/source/target-verus/release
  export PATH="$REL:$HOME/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin:$PATH"
  export VERUS_Z3_PATH=$HOME/vie/work/AES/vostd/tools/verus/source/z3
  cd /tmp/vostd-43da079
  cargo verus build -p ostd --offline --target x86_64-unknown-none
'
```

Log: `cargo-verus-build.log`.
