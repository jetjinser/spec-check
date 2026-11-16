# P0: runtime v0.16.0 vs frozen 43da079

Compared files:
`ostd/src/mm/vm_space.rs`, `ostd/src/mm/tlb.rs` (Asterinas v0.16.0
`4e78653b5` vs `/tmp/vostd-43da079`, vostd 0.16.0).

The files differ substantially in line count because the frozen tree carries
Verus annotations/proof blocks. The runtime-relevant shape is the same:

- `CursorMut::map` (runtime `vm_space.rs`) returns early on `Ok(())` from the
  page-table cursor and flushes on `Err(PageTableFrag::Mapped)`.
- Frozen `VmSpace::map` has the same `let Err(frag) = ... else { return; }`
  shape, with `verus_spec` annotations on the flush calls.
- `Cursor::map` in both trees splits a huge child and then
  `replace_cur_entry(...)`; on a split/replacement it returns
  `Err(PageTableFrag::Mapped)`.
- `Frame::map_level()` is hard-coded `1` in both trees; user `VmSpace` items are
  base pages, so a 2M user mapping cannot be constructed via the public API.

Hashes (working files, not normalized):

```
c7ffd207bcd3bd1dc7990403f4b561e5176fad81104d510b90433a2af6c0b3ad  Asterinas v0.16.0 vm_space.rs
111e4bcc024dab9b3e1a3fed4877cd9c27e9f8dcb9e539449be9e00939fc62a4  frozen 43da079 vm_space.rs
```

P0 runner check (same environment as the runtime test):

```
cargo osdk test start_boundary_mapping --boot-method qemu-direct
# 1 passed; 0 failed; 152 filtered out
```

This runner check is the gate that made P2 possible; it does not test TLB
behavior.
