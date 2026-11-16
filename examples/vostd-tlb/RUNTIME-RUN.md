# Runtime / executable verification status

## QEMU hardware/VM experiment — ACHIEVED

`qemu-hw-test/` builds a minimal 32-bit PAE test kernel and boots it in QEMU
11.1. It caches a 2MB translation (frame A, 0xAA), splits the page table to 4KB,
remaps the VA to frame B (0xBB) **without invlpg**, and reads again:

```
warm read: 0xAA
after 2M split+remap (no invlpg): 0xAA
after invlpg: 0xBB
verdict: CONFIRMED stale 2M-page translation without flush
```

A 4MB PSE variant shows the same. This is a real VM execution of the hazard
pattern; it does not execute ostd.

## ostd kernel run

Still unavailable: OSDK builds plain-Rust kernels, the frozen ostd is
Verus-annotated and has no OSDK integration; Asterinas' runtime ostd is a
different 0.18 codebase.

## Model test

`model-test/` remains a separate executable model (not the kernel).

# Runtime / executable verification status

## QEMU

`nix shell nixpkgs#qemu -c qemu-system-x86_64 --version` works (QEMU 11.1.2).
QEMU is available; a bootable image and runner are not.

## Runner found (but not for verified ostd)

The Asterinas devshell provides `cargo-osdk 0.16.0` and QEMU, and the Asterinas
root has an `OSDK.toml` plus an OSDK test-kernel. However:

- Asterinas' `ostd` is the **runtime, unverified** codebase at version 0.18;
  the frozen vostd `ostd` is 0.16 and is Verus-annotated (`verus!`, `vstd`).
- OSDK's test-kernel builds with plain cargo/rustc, not the Verus toolchain.
  `cargo check -p ostd` in the vostd tree fails with `E0554` (nightly features)
  under the pinned stable devshell; more fundamentally, the Verus `vstd`
  dependency has no plain-rustc build.
- There is no OSDK↔Verus integration that would build the verified ostd into a
  bootable image.

A runtime test on Asterinas' unverified ostd would exercise a different
codebase/version and would not validate the frozen verified artifact; it is
therefore not used as runtime evidence here.

## Earlier blocker notes

- `cargo-osdk` is not a nixpkgs attribute (it is in the Asterinas devshell).
- The rewritten vostd tree has no `osdk/` directory or `OSDK.toml`.
- The ktest framework runs in a kernel image under the Asterinas runner, which
  is not present in this environment.

Exact blocker: **QEMU present, no OSDK/runner or kernel image to boot**.

## Executable model test (not the kernel)

`model-test/` keeps page table and TLB caches separate and asserts:

- split-then-remap without a flush leaves the stale huge TLB entry;
- split-gated flush removes it;
- plain new mappings take no flush;
- unmap per-item ops + dispatch_all revoke entries;
- no dispatch leaves them (W2 shape);
- a Range flush removes all intersecting mappings.

Run:

```
nix develop ~/vie/work/AES/vostd/nix -c cargo run --offline
# tlb model tests passed
```

This is a model-level executable check, not a kernel run; do not cite it as
runtime evidence for ostd.
