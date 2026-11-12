# Generator controls (M7e)

Purpose: show that `screen-requires` generation/runner detects the three
expected outcome classes on hand-written contracts, independently of the
vostd census. These are **tool controls**, not vostd findings.

## Manifest

`manifest.json` (quantifier entries only):

| entry | contract | expected |
|---|---|---|
| `constant_false` | `requires always_false()` (zero-arg open spec fn returning `false`), dummy `_unused` parameter | **ConfirmedVacuous** |
| `bool_sat` | `requires identity_bool(b)`, bool parameter, witness `true` | **ConfirmedLive** |
| `bool_unsat` | `requires contra_bool(b)` = `b && !b`, bool parameter, no witness | **ConfirmedVacuous** |

## Run

```
cargo-spec-check analyses-vacuity --manifest examples/vostd-requires/controls/manifest.json \
  --cargo-verus <SDK>/target-verus/release/cargo-verus --z3 <SDK>/z3 --vstd <SDK>/vstd \
  --out /tmp/m7e-synth/out
```

vstd `2060 verified / 0 errors`; probe crate `6 verified / 3 errors`.
Result: **3/3 as expected** (`report.json`):

```
constant_false: ConfirmedVacuous
bool_sat: ConfirmedLive
bool_unsat: ConfirmedVacuous
```

## Relevance to the vostd run

The vostd census contains 162 function records whose **declared-path wrapper**
has `requires false`, but census canonicalization selects the
`_VERUS_VERIFIED_` body, whose real contract is not false; those wrappers are
therefore not reported. This control demonstrates that if a canonical
`requires` really were `false` (or mechanically contradictory over bool /
zero-arg clauses), the generated probe would report `ConfirmedVacuous` —
i.e. the 0-vacuity vostd result is not a generator blind spot for this shape.
