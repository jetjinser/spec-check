# ostd v0.16.0 runtime TLB experiment (2026-11-16)

Scope: **Asterinas v0.16.0 runtime ostd, unverified tree**, executed as a ktest
in QEMU. This is not the frozen Verus artifact and not an upstream claim.

## Environment

- worktree: `/tmp/asterinas-v016` at Asterinas `v0.16.0`
  (`4e78653b5`), created with `git worktree add --detach`.
- toolchain: `nightly-2025-02-01` (rustc 1.86.0-nightly), installed with all
  components for the worktree; `cargo-osdk 0.16.0` built from the worktree's
  `osdk/` with `OSDK_LOCAL_DEV=1` (the preinstalled binary was path-baked to a
  different tree).
- build/run: `RUSTFLAGS="-C linker=$(command -v cc)"`, `OVMF=off`,
  `--boot-method qemu-direct`, initramfs from `make initramfs`.
- local workarounds (patches included): qemu_args shebang
  (`#!/usr/bin/env bash`), local OSDK symlink at `~/.cargo/bin/cargo-osdk`
  (backup kept as `cargo-osdk.bak-oom`).

## P0 runner

`cargo osdk test start_boundary_mapping --boot-method qemu-direct`:
`1 passed; 0 failed; 152 filtered out` — ktest + QEMU serial link works.

## P1: VmSpace 2M is not constructible in v0.16.0

`Frame::map_level()` is hard-coded to `1` in both v0.16.0 runtime and frozen
`43da079`; `UserPtConfig::item_into_raw` therefore always maps base pages.
`VmSpace` cannot create a 2M mapping through its public API, so the A5
"split in VmSpace::map" scenario is not reachable from a public
`VmSpace`/`UFrame` state in this version. The experiment was moved to the kernel
page table, whose linear mapping really contains huge pages.

## P2: kernel huge-page split + 4K remap, no flush

Test: `ostd/src/mm/kspace/test.rs::tlb_probe_huge_split`
(`ostd-runtime-test.patch`). Steps:

1. allocate a 2M-aligned physical run A and a 4K frame B; write `0xAA`/`0xBB`;
2. `va = paddr_to_vaddr(pa_a)`, assert `va` is 2M-aligned;
3. assert the linear mapping at `va` has `level > 1` (huge);
4. warm-read `va` -> 0xAA;
5. `cursor.map(MappedItem::Untracked(pa_b, 1, prop))` on a 4K range: splits the
   huge mapping and replaces the 4K entry, **no invlpg**; assert the map returned
   `Err` (an existing mapping was replaced);
6. read `va` -> 0xAA (**stale**);
7. `invlpg va`, read -> 0xBB (**fresh**);
8. restore the 4K entry to A and invlpg.

Result (`ktest-run.log`):

```
test ostd::mm::kspace::test::tlb_probe_huge_split ... ok
test result: ok. 1 passed; 0 failed; 153 filtered out.
```

The assertions require `stale == 0xAA` and `fresh == 0xBB`; a silent early
return is not possible (all preconditions panic).

## Correction to F-029

This experiment **does not reproduce a missing flush in the real
`VmSpace::map`**:

- `Cursor::map` returns `Err(PageTableFrag::Mapped)` when it replaces an
  existing mapping, including the leaf entry created by a huge-page split
  (`old.is_err()` asserted).
- `VmSpace::map` takes its TLB flush on exactly that `Err(frag)` branch.
- In addition, a user `VmSpace` cannot contain a 2M mapping in v0.16.0
  (`map_level()==1`).

Therefore the earlier A5/A6 verification claim "the real map split path misses
a flush" is **not supported at runtime** and is best read as a model/precondition
artifact. The **contract gap remains**: W1 (a body with no flush at all) still
verifies, and the contract has no postcondition requiring the flush. F-029
should be stated as a contract-quality gap, not a confirmed real-code defect.
F-030 (unmap) is likewise a contract gap; the real unmap body does issue flushes.

The QEMU hardware experiment (`qemu-hw-test/`) independently confirms that a
missing flush after split+remap leaves a stale translation; this runtime test
shows the real ostd v0.16.0 **does** invalidate on the replacement path.

## P3 regression

`/tmp/vostd-combined` was re-run after clearing the target cache:
`verification results:: 1908 verified, 0 errors`. The earlier archived count
(1500) came from a different target/config; no errors were produced in this
run. W1/W2 archives are unchanged (1499/1 with the added postconditions).

## Limits

- Same-version **unverified** runtime tree, not the frozen Verus code.
- Single vCPU, TCG, test temporarily splits the kernel linear alias of an
  allocated frame; mapping is restored and invalidated.
- No upstream change; only a local ktest.
