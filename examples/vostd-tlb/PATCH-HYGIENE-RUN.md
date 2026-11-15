# Patch hygiene on pristine 43da079

Pre-registration: `PATCH-HYGIENE-EXPECTED.md`.

## Discovery: ktest workspace differs

The ktest copy enables `ostd-macros`/`ostd-test` in `ostd/Cargo.toml` and adds
cfg-gated modules; the pristine rev does not. The A1/A2 patches verified on the
ktest copy did **not** carry their map postcondition proofs over unchanged. The
dispatch_all model changed map's flush proof shape: both the split-flush and
replacement-flush exits needed the `apply_all` coverage lemma
(`TlbModel::lemma_apply_all_removed`) after dispatch.

## Combined patch on pristine

`combined-tlb-contracts.patch` (4 files, 434 diff lines):

| run | result |
|---|---|
| pristine + combined series | **1500 verified / 0 errors** |
| W1: replacement flush removed | **1499 / 1**, postcondition failed |
| W2: all unmap flushes removed | **1499 / 1**, postcondition failed |
| `patch -p1 --dry-run` on pristine | 4 files clean |

Content: split-gated map flush (map_loop flag, `Cursor::map split_out`, TLB
postcondition), `dispatch_all` + `flush_range`, unmap revocation postcondition,
per-item `_with` ops, one trailing `Range` (A2 residual).

The separate `map-tlb-flush-optimized.patch` and
`unmap-tlb-flush-dispatch-all.patch` were produced on the ktest workspace; the
combined pristine patch is the authoritative artifact.
