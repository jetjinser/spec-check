# M7e-projection — explicit projection semantics for the remaining Tier C requires

Status: design + pre-registration. This file is written **before** the
implementation, the census re-run, and any probe run. The run archive
(`examples/vostd-requires/PROJECTION-RUN.md`) reports what actually happened
and does not rewrite this contract.

Parent documents:

- `docs/M7e-requires-screening.md` — Tier A/B/C funnel and the probe format.
- `docs/M7e-tierc-modeling.md` — generic instantiation and the conservative
  reference/ghost `usize`-id abstraction (R/H, G rules).

This extension addresses the root cause left by M7e-ext: a reference/ghost
parameter was abstracted to an id only when **every** use was a receiver,
field, or deref root (R1/H1). The remaining 129 Tier C functions use those
parameters as ordinary arguments or compare them directly, e.g.

```
perm.id() == mutex.cell_id()
reader.wf(*old(owner_r))
old(self).idx < old(self).children.len()
```

The field/method access was recorded as an audit ledger but never interpreted.
M7e-projection lets the **project side** declare an explicit scalar model for
such a method or field. The tool only performs generic translation, type
checking of the declared model body, tier recomputation, and preamble
injection; it contains no project paths or project-specific semantics.

## 1. Goal and non-goals

Goal: move a function whose remaining blocker is an **explicitly modeled**
projection from Tier C into a runnable A/B probe, and record exactly which
expressions were modeled, by which model, and under which relation.

Non-goals:

- No automatic inference of projection semantics from source text or VIR.
- No claim that an abstract `ConfirmedLive` is real liveness. Every projected
  verdict is **model-relative**.
- No bit-vector, layout, trait-bound, lifetime, aliasing, or deep-structure
  reasoning. Unmodeled constructors, `is`-discriminants, raw bit operations,
  and arbitrary set/map/seq operations stay Tier C.
- No change to the default behavior: without a `projection` section the tool
  behaves exactly like M7e-ext.

## 2. Soundness contract (read this before trusting a verdict)

A projection is a **model-relative** replacement. For a method/field `p` the
project side supplies a scalar spec function `m` and declares its relation to
the real value:

| relation | meaning | `ConfirmedVacuous` evidence | `ConfirmedLive` evidence |
|---|---|---|---|
| `faithful` | `m(args)` equals the real projection for every real valuation | candidate evidence | abstraction only |
| `over-approx` | every real projection value is admitted by `m` (the model is weaker) | candidate evidence | abstraction only |
| `under-approx` | `m` admits fewer values than the real projection | **not** evidence | abstraction only |

The tool trusts the declaration. A wrong `faithful`/`over-approx` declaration
invalidates the corresponding vacuity evidence; this is the same trust boundary
as a hand-written spec function. The run archive must list every relation used.

Formally, let `F` be a real requires clause and `F'` its projection. For a
faithful or over-approximating projection, every real valuation of `F` induces
a valuation of `F'`, so `models(F') ⊇ models(F)`. Therefore

```
unsat(F')  =>  unsat(F)
```

and a Verus `ConfirmedVacuous` on `F'` is a **candidate** true positive that
must be replicated. The converse is not true: a `ConfirmedLive` on `F'` only
says the abstraction is satisfiable. It is never reported as a real witness.

An `uninterp` model (`body: null`) is an arbitrary function of its arguments.
For vacuity it is an over-approximation (quantifying over all functions includes
the real one), but it is deliberately kept at Tier B: Verus cannot prove a
liveness witness through an uninterpreted function, and a `ConfirmedLive` would
not be meaningful.

`under-approx` models are accepted only so the project can record a known
weaker model; entries that use one are never eligible for vacuity evidence.

## 3. Configuration schema (project side, never in the tool)

`screen-requires --config FILE` keeps the M7e-ext schema and adds an optional
`projection` object. The schema string is unchanged
(`spec-check.requires-config/1`), so an old config is still valid; unknown
schema strings are still rejected.

```json
{
  "schema": "spec-check.requires-config/1",
  "generic_instances": [{"match": "*", "types": {"*": "u64"}}],
  "reference": {"mode": "opaque"},
  "ghost": {"mode": "opaque"},
  "projection": {
    "models": [
      {
        "name": "proj_pcell_id",
        "params": ["recv: usize"],
        "returns": "usize",
        "body": "recv"
      },
      {
        "name": "proj_inv",
        "params": ["recv: usize"],
        "returns": "bool",
        "body": null
      }
    ],
    "rules": [
      {
        "match": "ostd::sync::mutex::impl&%7::new",
        "kind": "method",
        "callee": "vstd::cell::pcell::impl&%2::id",
        "model": "proj_pcell_id",
        "relation": "faithful",
        "note": "PointsTo::id is the permission's CellId; the probe id is chosen to be that CellId."
      },
      {
        "match": "*",
        "kind": "method",
        "callee": "ostd::specs::mm::frame::meta_region_owners::impl&%0::inv",
        "model": "proj_inv",
        "relation": "over-approx",
        "note": "uninterpreted invariant predicate: no constraints, vacuity only"
      },
      {
        "match": "*",
        "kind": "field",
        "datatype": "ostd::mm::frame::linked_list::linked_list_owners::LinkedListOwner",
        "field": "idx",
        "model": "proj_owner_idx",
        "relation": "over-approx",
        "note": "uninterpreted field projection"
      }
    ]
  }
}
```

Rules:

- `models` is a flat list with unique names. `params` entries are
  `"name: usize"` (or `int`, both are the same scalar-integer marker) or
  `"name: bool"`; `returns` is `usize`/`int` or `bool`. The generated Verus
  model signatures use `int` for the integer marker. Probe entries that contain
  a projected clause also use `int` parameters, so model calls and arithmetic
  results type-check; entries with no projected clause keep the M7e `usize`
  rendering.
- `body` is a **restricted scalar expression** over the formal names:
  integer/bool literals, formal variables, `!`, unary `-`, `+ - *`,
  `== != < <= > >=`, `&& || ==>`, and parentheses. Anything else is rejected
  at config load. `body: null` means `uninterp`.
- `rules` are ordered; the **first** matching rule wins. `match` is a glob over
  the enclosing function path (`*` default). `kind: method` matches the exact
  VIR callee path (glob allowed); `kind: field` matches the VIR datatype path
  and field name (glob allowed); `kind: variant` matches a VIR datatype path and
  enum variant (the `x is Some` form). Method model arity must equal the number
  of call arguments including the receiver; field and variant model arity must
  be one. A variant model returns `bool`.
- Every rule needs a `relation`; unknown values are rejected. Unused models are
  accepted but recorded in the report so a stale config is visible.
- The tool rejects duplicate model names and model bodies whose inferred type
  does not equal `returns`.

The projection channels require `reference.mode = "opaque"` and/or
`ghost.mode = "opaque"` for the parameters whose ids the models consume; this
is the same policy switch as M7e-ext. The first station uses both `"opaque"`.

## 4. Expression translation rules (generic, no project knowledge)

The census now records a compact expression tree per requires clause
(`requires[].expr`). It is extracted from VIR, so it is available even when the
source span is missing (47 functions / 113 clauses in the archived census).
The tree uses only generic nodes: `int`, `bool`, `var`, `const`, `old`,
`read`, `deref`, `field`, `call`, `ctor`, `quant`, `trigger`, `not`, `neg`,
`bin`, `is_variant`, `opaque`.

Translation to a probe term `T` is:

| expression | translation |
|---|---|
| integer / bool literal | literal |
| scalar parameter | probe parameter (same name) |
| reference/ghost parameter used as a whole value | its `{name}_id` base id; **only** as a model argument |
| `Old(e)` | `translate(e)`; at function entry `old(e) = e` for the projection. This is documented, not silently assumed: it is exact only for the entry-state reading of an id. |
| `ReadPlace(e)`, `Deref(e)` | `translate(e)` (the id abstraction has no pointee) |
| `Trigger(e)`, `CoerceMode(e)`, `Cast(e)`, `WithTriggers(e)` | `translate(e)` (annotation/wrapper only) |
| `x.method(a1, ..., an)` | a model call `m(t_x, t_a1, ..., t_an)` if a method rule matches the resolved callee path and arity; otherwise the clause is unmapped |
| `x.field` | a model call `m(t_x)` if a field rule matches the VIR datatype and field; otherwise unmapped |
| `x is Variant` | a model call `m(t_x)` if a variant rule matches the VIR datatype and variant; otherwise unmapped |
| `e == e'` / `e != e'` | translate operands; a raw reference/ghost id may appear here as its id. This is a deliberate over-approximation of equality of the underlying values (the id is a value-identity token). |
| `Not(e)`, `Neg(e)`, arithmetic/ordering/logical `Bin` | translate operands; both must be scalar (a raw base id may not be ordered or used in arithmetic) |
| `Const` / `ConstVar` / integer type bound (`usize::MAX`, ...) | lifted to a fresh probe parameter (same convention as M7e) |
| `forall` / `exists` | bound variables become fresh scalar quantifier variables; the body is translated. The clause is rendered as an open spec fn but is **Tier B** (the bounded witness search does not decide quantifiers) |
| `Ctor`, `Match`, raw bit operations, blocks, unknown nodes | unmapped; the clause stays Tier C if it mentions a reference/ghost parameter |

A parameter is **projection-covered** iff every clause in which it occurs
translates successfully. A clause with an unmapped node does not cover any
reference/ghost parameter it mentions; the function keeps Tier C with a
`projection_unmapped:<detail>` reason. This is deliberately conservative: an
unmapped subexpression elsewhere in the same clause is not assumed inert.

Model arguments may themselves be projections (`x.inner@.inv()`); translation
is compositional. A model body is type-checked at config load, so a bool field
cannot be declared as `body: "recv"` with an `usize` receiver.

## 5. Tier recomputation and probe generation

The baseline is still `classify(..., Config::default())` and is exactly M7e.
The supplied config classifies each function once:

1. Generic instantiation (G rules) as before.
2. Every reference/ghost parameter is mappable if either the old safe-use rule
   (R1/H1) holds, or every clause mentioning it is projection-covered.
   Otherwise it is a Tier C blocker.
3. A clause is:
   - **mechanical** (existing scalar AST over scalar parameters),
   - **projected-evaluable** (translated, every model has a body, no
     quantifier),
   - **projected-opaque** (translated, but at least one `uninterp` model or a
     quantifier), or
   - **unsupported** (no translation).
4. Tier:
   - **C** if any reference/ghost blocker is not mappable, or a generic name is
     uninstantiated.
   - **B** if every blocker is covered but any clause is
     projected-opaque/unsupported.
   - **A** if every clause is mechanical or projected-evaluable.
5. Tier A witness search runs over the inlined scalar terms (model bodies are
   substituted into the bounded search). Tier B has no witness, exactly as in
   M7e-ext.

Generated probes:

- Projected clauses are rendered as named `open spec fn`s whose bodies call the
  project-side model functions. This keeps the constraint structure, unlike the
  old whole-clause `uninterp` fallback.
- The model definitions are injected once into the manifest `preamble`
  (`pub open spec fn ... { body }` or `pub uninterp spec fn ...;`), deduplicated
  by name across selected entries.
- Unsupported clauses keep the M7e-ext `uninterp` fallback over their
  parameters/constants.
- `modeling.projection_mapping` records, per parameter, every projection model
  and relation used; `modeling.projection_models` records the injected model
  definitions; `modeling.projection_unmapped` records why a C entry stayed C.
- Batching, `--filter`, `--limit`, and the 900 s per-batch budget are
  unchanged.

Determinism: rules are ordered, model lists and mappings are `BTreeMap`s,
entries keep census order, and generated names stay `r<index>_<short>`.

## 6. Pre-registered expectations (written before implementation/run)

| id | situation | expected |
|---|---|---|
| P1 | no `projection` section | byte-for-byte M7e-ext classification on the same census (regression guard) |
| P2 | identity models on both sides of `perm.id() == mutex.cell_id()` | baseline C → Tier A; both ids are free; witness `(0, 0)`; **ConfirmedLive** (shape/model-relative, reproducing M1 `mutex_guard_new_shape`) |
| P3 | method with no matching rule | stays Tier C; `projection_unmapped` / `*_unmapped` reason; no probe |
| P4 | `body: null` (uninterp) model | projected clause is Tier B; expected `CandidateInconclusive`; `ConfirmedLive` is not real liveness |
| P5 | constant model whose body makes the conjunction unsat | Tier A with no witness; expected `CandidateInconclusive` with the E6 note; if Verus returns `ConfirmedVacuous`, replicate before registering |
| P6 | `under-approx` relation | accepted but never vacuity evidence; expected note says so |
| P7 | rule/model arity or type mismatch, missing relation, duplicate model name, unsupported body token | config load error (no silent fallback) |
| P8 | field/ctor/quantifier/`is` not modeled | clause stays C if it mentions a reference/ghost parameter; quantifier with a fully projected body may be Tier B |
| P9 | run of the projection station | every mapped parameter appears in `projection_mapping`; every verdict matches the per-entry `expected`; `Inconclusive` is not a defect |

Numeric expectations are directional and are **not** predicted here: the exact
number of C→A/B entries, the A/B split, and the verdict distribution are
measured by the run. The implementation is deliberately conservative; a large
remainder of C is an acceptable and expected outcome.

## 7. Cost control

- Config load is pure and linear in models/rules; census expression extraction
  is one extra tree walk per clause.
- Projection classification is linear in clause nodes and model arity.
- The probe crate grows with the number of mapped entries; the first station is
  split into contiguous batches of at most 50 entries, each with an independent
  900 s budget, as in M7e-ext.
- No cross-batch cache, no nightly integration, no change to `check`.

## 8. Boundaries stated up front

- A projection is not a refinement proof. `faithful`/`over-approx` are
  project-side claims with a note; the tool only checks syntax and types.
- Same-parameter identity models are a representation choice. If two distinct
  concrete projections are both declared `body: "recv"` on the same base id,
  the model forces them equal; that is an under-approximation unless the
  project's relation declaration says they are equal. The tool rejects two
  distinct identity models on the same base id in one clause. The first station
  avoids the issue by using identities only for id-valued projections and
  `uninterp` for fields/other values.
- `old` is transparent at entry; this is exact for the id abstraction but not a
  claim about post-state changes.
- Quantifiers are rendered but not decided by the bounded search; they stay
  Tier B.
- Missing source text is not a blocker because the tree comes from VIR, but a
  VIR node the extractor does not understand is an explicit `opaque` node and
  therefore unmapped.
- No claim is made about the 451 C-deferred functions from the feasibility
  analysis; this station only addresses expressions that a project-side scalar
  model can actually express.
