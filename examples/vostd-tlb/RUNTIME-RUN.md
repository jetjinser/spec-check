# Runtime / executable verification status

Three distinct things are tracked here and must not be conflated:

1. **QEMU hardware/VM experiment** — executed; confirms the stale-translation
   hazard at the VM level.
2. **ostd kernel run** — attempted/assessed; still unavailable.
3. **Executable model test** — executed; not a kernel.

## 1. QEMU hardware/VM experiment — ACHIEVED

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

## 2. ostd kernel run — unavailable

- OSDK 0.16 and QEMU are available in the Asterinas devshell, but OSDK builds
  plain-Rust kernels while the frozen verified ostd is Verus-built; there is no
  OSDK↔Verus integration.
- `cargo check -p ostd` in the frozen tree fails with `E0554`; the Verus `vstd`
  dependency has no plain-rustc build. The rewritten tree has no `osdk/` or
  `OSDK.toml`.
- Asterinas' runtime ostd is a **different, unverified 0.18 codebase**; testing
  it would not validate the frozen 0.16 verified artifact.
- The ktest framework needs a kernel image/runner that is not present here.

Exact blocker: **no bootable image for the frozen verified ostd**.

## 3. Executable model test — not the kernel

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

Do not cite this model test as runtime evidence for ostd.
