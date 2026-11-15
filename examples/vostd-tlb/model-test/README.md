# Executable TLB model test (internal)

This is a **model**, not the kernel and not a proof. It keeps the page table
and the cached TLB mappings separate and reproduces the two contract shapes:

1. split-then-remap without a flush leaves the old huge mapping cached while
   the page table is split;
2. the split-gated flush removes it;
3. a plain new mapping takes no flush;
4. unmap with per-item ops + `dispatch_all` revokes entries in both layers;
5. the W2 shape (no dispatch) leaves the TLB entries;
6. a Range flush removes every intersecting mapping, not only `range.start`.

Run (host, no deps, no kernel runner):

```
nix develop ~/vie/work/AES/vostd/nix -c cargo run --offline
```

The binary asserts each case and prints `tlb model tests passed`.
