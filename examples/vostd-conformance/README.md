# Trait declaration × implementation conformance (static)

Input: census of the frozen ostd VIR (1,468 Exec/Proof functions). The census
already merges a trait declaration's regular `ensures`/`returns` into each impl
for coverage, and records whether the declaration is present in the VIR. This
check reports where declared guarantees and verified implementations are not
connected by verification.

## Counts

| fact | count |
|---|---|
| trait impl functions | 246 |
| impls with a declared postcondition (clauses or `returns`) | 154 |
| impls that also carry their own postcondition | 22 |
| **`external_body` impls with a declared postcondition** | **20** (44 declared clauses) |
| impls whose declaration is unavailable in the VIR | 6 |

Module split of the 20: `mm` 17, `sync` 3. The largest inherited contracts are
`kspace::item_into_raw` (6), `vm_space::item_into_raw` (6),
`page_table::cursor::next` (5), `kspace::clone` (4),
`page_table::node::on_drop` (4).

## Reading

For a trait method, the declaration's `ensures` are the caller-visible
guarantee. A normal impl must discharge them during verification. An
`external_body` impl is trusted: its body is not verified, so the inherited
declaration guarantees are assumed for callers without a proof connecting them
to the implementation. The 20 rows are therefore the trust boundary of trait
conformance in this crate, not automatically defects: several are deliberate
raw-representation transitions (`to_repr`/`from_repr`, `item_into_raw`).

The 6 `declaration_unavailable` rows are impls whose trait declaration is not
in the VIR; their inherited contract cannot be checked from this artifact at
all.

## Boundary

- Static only: `external_body` is read from function attributes; nothing here
  shows a violated guarantee.
- The check does not inspect `assume_specification` or external axioms that may
  discharge a declaration elsewhere.
- Lists and counts regenerate with `report.py <census.json> <out.json>`.
