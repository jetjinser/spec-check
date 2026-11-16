# QEMU hardware/VM test: stale translation after large-page split + remap

Scope: **hardware/VM-level**, not an ostd run and not a proof. It tests the
hazard the TLB contracts are about: a large-page translation cached in the TLB
survives a page-table split and remap when no invalidation is issued.

## Programs

- `boot_pae.S` — 32-bit PAE, **2MB page split to 4KB** (closest to the x86_64
  2M/4K case):
  1. identity-map 1GB with 2MB pages; fill frame A (8MB) with `0xAA`, frame B
     (10MB) with `0xBB`;
  2. read VA `0x00800000` repeatedly (warm TLB), expect `0xAA`;
  3. replace the PDE with a page table, entry 0 -> frame B; **no invlpg**;
     read -> stale `0xAA`;
  4. `invlpg`, read -> `0xBB`.
- `boot.S` — 32-bit PSE, 4MB page split to 4KB, same steps (secondary check).
- `kernel*.elf`, `qemu*.log` — built artifacts and serial logs.
- `build.sh`/`run.sh` (PSE), `build_pae.sh`/`run_pae.sh` (PAE).

## Reproduce

Requires QEMU and binutils (available in the Asterinas nix devshell):

```
cd examples/vostd-tlb/qemu-hw-test
nix develop ~/vie/work/AES/asterinas/nix -c ./build_pae.sh
nix develop ~/vie/work/AES/asterinas/nix -c ./run_pae.sh
```

QEMU command (from `run_pae.sh`):

```
qemu-system-i386 -kernel kernel_pae.elf -m 64M -serial stdio -display none \
  -device isa-debug-exit,iobase=0xf4,iosize=4 -no-reboot
```

## Result (QEMU 11.1.0)

`qemu_pae.log`:

```
warm read: 0xAA
after 2M split+remap (no invlpg): 0xAA
after invlpg: 0xBB
verdict: CONFIRMED stale 2M-page translation without flush
```

`qemu.log` (4MB PSE) shows the same pattern.

## Boundary

- This is a real QEMU execution of the hazard pattern. It does not execute
  ostd; the verified code path is covered by W1/W2/A5/A6 and the executable
  model test separately.
- The ostd `map` code path is 2M -> 4K; the PAE variant matches that shape. The
  PSE variant is a secondary analogue.
