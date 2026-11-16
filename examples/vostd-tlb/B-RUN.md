# B: runtime/reachability evidence

## Runtime attempt (blocked)

Checked in this environment:

- `command -v qemu-system-x86_64` -> empty
- `command -v cargo-osdk` / `command -v osdk` -> empty
- `ostd/osdk/` is empty; the vostd Makefile has no QEMU target
- the frozen ktest workspace carries the `ktest` cfg, but running those tests
  needs a built kernel image and QEMU, neither available here

No runtime reproduction was attempted beyond these checks. This is the exact
blocker: **no QEMU and no osdk runner in the environment**.

## Verus reachability witness (verified)

`reach-witness.patch` adds a proof to `ostd/src/mm/vm_space.rs`:

```
proof fn reach_split_path_witness() {
    let m = Mapping { va_range: 0..2097152, pa_range: 0..2097152,
                      page_size: 2097152, property: arbitrary() };
    let view = CursorView::<UserPtConfig> { cur_va: 0, mappings: set![m], ... };
    assert(m.inv());
    assert(view.inv()) by { ... vaddr range ... };
    assert(view.mappings.contains(m));
    assert(view.present());
    assert(view.query_mapping() == m);
    assert(view.query_mapping().page_size == 2097152);
}
```

Result: **1496 verified / 0 errors** (one new proof function). This is a
concrete, valid cursor view with a present 2M mapping covering the target VA,
i.e. the exact condition under which `map_loop` takes the `ChildRef::Frame`
split arm (`split_if_mapped_huge`). It is a model-level witness, not a full
cursor-owner/tree construction.

## Path difference (B2)

- Wrong body W1: no flush on the split/replacement path; with the postcondition
  it fails at 1494/1.
- A1 optimized patch: flushes only when `split` is true (or on replacement);
  full crate verifies at 1495/0.
- A5/A6: adding the postcondition to the unmodified body fails at the early
  return, with and without an entry-backing precondition; the split arm is the
  contract-relevant path.

## Boundary

- No hardware execution; no QEMU/ktest run.
- The witness constructs a view, not a full sequence of page-table operations.
  The operational link (Frame child at a higher level -> split arm) is argued
  from `map_loop`'s match and its proof obligations, not executed.

## Later note (2026-11-16)

This run's boundary statements describe the verification-level run above. A
separate QEMU hardware/VM experiment now reproduces the stale-translation hazard
(`qemu-hw-test/`); it does not execute ostd and does not change this run's
result.
