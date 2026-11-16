# Runtime / executable verification status

Four distinct things are tracked here and must not be conflated:

1. **QEMU hardware/VM experiment** — executed; confirms the hazard.
2. **ostd v0.16.0 runtime ktest** — executed on the unverified same-version
   tree; confirms the hazard and corrects F-029's real-body claim.
3. **Frozen verified artifact run** — still unavailable.
4. **Executable model test** — executed; not a kernel.

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

## 2. ostd v0.16.0 runtime ktest — ACHIEVED

`ostd-runtime-2026-11-16/` adds `tlb_probe_huge_split` to Asterinas v0.16.0 and
runs it with `cargo osdk test ... --boot-method qemu-direct` (QEMU 11.1,
nightly-2025-02-01, local OSDK 0.16). It splits a huge kernel linear mapping,
remaps the 4K entry to frame B without invlpg, reads stale 0xAA, then invlpg and
reads 0xBB. Result: `1 passed; 0 failed`.

**Correction:** `Cursor::map` returns `Err(Mapped)` on the split/replacement, so
`VmSpace::map` runs its flush branch; user `VmSpace` cannot create a 2M mapping
in v0.16.0 (`Frame::map_level()==1`). The A5/A6 "real map misses a flush" claim
is not reproduced; F-029 is a contract gap, not a confirmed real-code defect.

## 3. Frozen verified artifact run — architectural blocker

- The frozen crate **can** be verified and compiled as a bare-metal library:
  `cargo verus build -p ostd --target x86_64-unknown-none` reports
  `1495 verified, 0 errors` and produces `libostd.rlib` (P4, 2026-11-16).
- It is not a bootable kernel: `ostd/src/lib.rs` comments out `prelude`,
  `panic`, `logger`, `console`, `io`, `smp`, and the `ostd_macros`/`ktest`
  exports; the OSDK test kernel imports exactly those (`ostd::ktest`,
  `ostd::panic`, `ostd::prelude`).
- Restoring them would be new code outside rev 43da079, so it would no longer
  be the frozen verified artifact.
- Toolchain/ABI: frozen code is compiled with rustc 1.98.1; OSDK 0.16.0 uses
  nightly-2025-02-01 (1.86). Linking the Verus rlib into the OSDK kernel is not
  compatible; compiling the whole kernel with 1.98.1 first requires the missing
  runtime surface above.
- No OSDK integration exists in the frozen tree (`osdk/`/`OSDK.toml` absent).

See `verus-artifact-2026-11-16/P4-RUN.md` for commands/logs. This is an
architectural blocker, not a missing command.

## 4. Executable model test — not the kernel

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
