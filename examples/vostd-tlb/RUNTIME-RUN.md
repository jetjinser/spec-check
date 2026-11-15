# Runtime / executable verification status

## QEMU

`nix shell nixpkgs#qemu -c qemu-system-x86_64 --version` works (QEMU 11.1.2).
QEMU is available; a bootable image and runner are not.

## Runner

- `cargo-osdk` is not a nixpkgs attribute.
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
