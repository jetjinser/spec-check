# M7e-ext — Tier C modeling: generics, references, ghost/tracked (design + pre-registration)

Status: implemented in `spec-check-core` (`Config`, `--config`,
census `param_uses`); first station archived under
`examples/vostd-requires/` (`config-tierc.json`, `TIERC-EXPECTED.md`,
`TIERC-RUN.md`). Rules below were written **before** the implementation and
before any Tier C probe run; they are the pre-registered contract. The run
archive reports what actually happened and may not rewrite this file.

Parent: `docs/M7e-requires-screening.md` (Tier A/B/C funnel, generation
rules). This note only adds the three Tier C modeling channels and the
project-side configuration that turns them on.

## 1. Goal

M7e left 676/774 requires-bearing vostd functions in Tier C for three
reasons (overlapping): generic 505, reference_param 366, ghost_param 168.
Tier C is not "no defect", it is "not screened". This extension adds three
modeling rules that move a **provably safe subset** into runnable probes:

- generics: project-side type instantiation, then re-classify;
- references: opaque `usize` id plus a recorded access map;
- ghost/tracked: opaque `usize` id.

Two invariants are non-negotiable:

1. **No false vacancies.** Every abstraction is a *relaxation*: every real
   valuation of the precondition induces a valuation of the abstraction, so
   `unsat(abstraction) => unsat(real)`. A `ConfirmedVacuous` produced over an
   abstracted entry is therefore a real true positive; a liveness witness is
   a witness *of the abstraction only* and is never counted as real liveness.
2. **No invented coverage.** A parameter that does not satisfy the safe-use
   rule below stays a Tier C blocker, and the reason says why. Upgrades are
   recorded per function, with the mechanism and the `C -> A/B` evidence.

## 2. Configuration (project side, never in the tool)

`screen-requires --config FILE` reads a JSON document. Without `--config`
the tool behaves exactly as M7e (all three blockers remain Tier C).

```json
{
  "schema": "spec-check.requires-config/1",
  "generic_instances": [
    {"match": "ostd::specs::mm::page_table::node::owners::*", "types": {"E": "u64"}},
    {"match": "*", "types": {"*": "u64"}}
  ],
  "reference": {"mode": "opaque"},
  "ghost": {"mode": "opaque"}
}
```

- `generic_instances` is an ordered list; the **first** matching `match`
  pattern wins. Patterns are exact paths or globs with `*`. `types` maps a
  type-parameter name to a concrete rendered type; `"*"` is the default for
  every generic name the entry does not mention.
- `reference.mode` / `ghost.mode`: `"off"` (default) or `"opaque"`.
- Unknown schema versions are rejected (the loader is strict about
  `schema`), so a stale config cannot silently change meaning.

## 3. Generic instantiation (G rules)

**G1.** A function is *instantiated* iff every name in its `generics.names`
has a mapping (explicit or `*` default) in the first matching entry.
Substitution replaces the whole-token type parameter in each parameter's
rendered type (`Typ TypParam T` and a bare identifier `T`; identifiers keep
`%`/`_` characters).

**G2.** The parameter `kind` is re-derived from the substituted type:
`&...` → reference, `Tracked<...>`/`Ghost<...>` → ghost, `bool` → bool,
`char` and integer types (`uN`/`iN`/`usize`/`isize`/`int`/`nat`) → int,
everything else → opaque.

**G3.** A generic function with an unmapped name keeps Tier C with
`generic_uninstantiated:[names]` (or `generic:[names]` when no rule matched
at all). The name list is the missing set, not the full set.

**G4 (honest limit).** Instantiation does not re-parse clause text. A clause
whose only blocker was the *syntactic* classification of an opaque generic
parameter stays `unsupported` and therefore Tier B, not Tier A. What
instantiation buys is: (a) generic functions are no longer blocked as
functions, and (b) a clause that was already mechanical over scalar
parameters of a generic function becomes runnable.

**G5.** Assigning a type parameter that occurs only in opaque/ghost positions
is semantically inert for the probe: those parameters are abstracted to
`usize` ids anyway. The `*` default exists for that reason. If a future rule
starts rendering the substituted type, the config must become explicit.

## 4. Reference and ghost/tracked abstraction (R/H rules)

The census now records, per clause, every occurrence of a parameter as a
*use* with a kind, plus the occurrence count. Kinds:

| kind | meaning | safe |
|---|---|---|
| `receiver_root` | root of the receiver of a dynamically resolved (method/trait) call | yes |
| `field_root` | root of a field place (`p.f`, `p.f.g`) | yes |
| `deref_root` | root of a deref place (`*p`, `p@`) | yes |
| `plain_root` | root of any other place (`p` compared, passed as a non-receiver argument, returned, ...) | no |

**R1 (reference).** With `reference.mode = "opaque"`, a referenced `&`/`&mut`
parameter is *mappable* iff every use of it in every clause is safe
(`receiver_root`/`field_root`/`deref_root`) **and** the number of recorded
uses equals the number of `Place Local` occurrences of the parameter (an
unaccounted occurrence is treated as unsafe). Wrappers that do not change the
value are stripped before the root is taken: `ReadPlace`, `Place
Temporary`, `Unary MutRefFuture`, `Old`.

**R2 (mapping).** A mappable reference parameter becomes `{sanitized_name}_id:
usize` in the probe. The access map is recorded in `screening.json` as
`{"param", "id", "uses": [{"kind", "callee"?, "field"?, "argc"?}]}`. Because
the clause body may involve arbitrary spec calls, the probe keeps the clause
as a named `uninterp spec fn` over the ids (shape preserved by arity, body
abstracted). The access map is the audit trail; it is not claimed to be a
fully interpreted model.

**R3 (unmappable reference).** Any unsafe use, a quantifier/trigger/old
occurrence the walker cannot attribute, or a mismatch between uses and
occurrences leaves the parameter a Tier C blocker with
`reference_param_unmapped:<name>` (policy on) or `reference_param:<name>`
(policy off). This is deliberate: an id is not a dereference, and a
conservative miss is preferred over a wrong mapping.

**H1 (ghost/tracked).** Same rule and same use kinds as R1. A referenced
`Tracked<...>`/`Ghost<...>` parameter becomes `{name}_id: usize`; the mapping
is recorded under `ghost_mapping`. Ghost values are always abstracted at
Tier B (mechanical clauses cannot reference them: the census mechanical
class is scalar-only).

**H2 (verdict expectation).** Any entry that uses R2/H1 is Tier B and
pre-registered `CandidateInconclusive`. A `ConfirmedLive` on such an entry
would be liveness of the abstraction only and must not be reported as real
liveness; a `ConfirmedVacuous` is a true positive (relaxation argument in
§6) and must be replicated.

**H3.** With the policy off, the old reasons (`reference_param`,
`ghost_param`) are kept verbatim, so existing archives remain comparable.

## 5. Tier recomputation and the coverage ledger

`build` classifies each function twice: once with the default config (the
*baseline*, exactly M7e) and once with the supplied config. The screening
report gains:

- `counts.baseline` — baseline A/B/C/no-requires distribution;
- `counts.upgraded` — `{to_a, to_b, still_c}` for functions that had
  requires in the baseline;
- per function `baseline_tier`, `baseline_reasons`, `tier`, `reasons`, and
  `modeling` (`generic_instantiation`, `reference_mapping`, `ghost_mapping`,
  or the blocking reason).

Determinism: all maps are `BTreeMap`/`BTreeSet`, entries keep census order,
and names are `r<index>_<short path>` as before. Batch/limit/filter apply
after tier recomputation, so `--batch`/`--batch-size` and the 900 s budget
carry over unchanged.

Cost: classification is pure and linear in clauses; the generated probe
crate grows with the number of upgraded entries, so the run is split with
the same contiguous batches (`--batch-size 40`–`50`).

## 6. Why the abstraction is sound (for vacuity only)

Let `F` be a requires clause over concrete values. The probe replaces every
reference/ghost parameter by an id and, for a clause that mentions a mapped
parameter, replaces the clause by `u(ids, scalar_params)` for a fresh
uninterpreted predicate `u`. Given any valuation `v` of the real
precondition, choose each `u` to return the truth value of `F(v)`; all
generated conjuncts hold. Hence `models(abstraction) ⊇ models(F)`; if the
abstraction has no model, `F` has none. The bounded witness search and the
Verus `vacuous` proof operate on the abstraction, so:

- `ConfirmedVacuous` ⇒ the *transcribed shape* is unsatisfiable ⇒ real
  precondition unsatisfiable (true positive);
- `ConfirmedLive` ⇒ only "the abstraction is satisfiable"; it is never
  evidence that a real reference/ghost witness exists (hence R3/H2 keep
  these entries at Tier B).

Aliasing of two references is a special case: any real aliasing valuation
corresponds to choosing equal ids, so the abstraction still over-approximates
(ids are unconstrained). This is why no alias analysis is needed for
soundness; it is only needed for precision, which this extension does not
claim.

## 7. Pre-registered expectations (written before the run)

| id | situation | expected |
|---|---|---|
| T1 | baseline (no `--config`) re-run on the archived census | 9 A / 89 B / 676 C and identical per-function tiers to the M7e archive (regression guard) |
| T2 | config on: generic-only function with an all-mechanical scalar clause | baseline C → Tier A, witness search as usual, all entries `ConfirmedLive` (or `CandidateInconclusive` if no witness) |
| T3 | config on: generic-only function with an unsupported clause | baseline C → Tier B, `CandidateInconclusive` |
| T4 | config on: reference/ghost function whose uses are all safe | baseline C → Tier B, `CandidateInconclusive`; mapping recorded |
| T5 | config on: reference/ghost function with an unsafe use | stays Tier C with `*_unmapped` reason; no probe |
| T6 | run: any `ConfirmedVacuous` in an upgraded Tier B entry | E6 true positive: replicate and register; do not treat as model noise |

Numeric expectations (before running): the only predictions we allow are
directional. Generic instantiation is expected to move the ~252 generic-only
functions to Tier B/A, reference policy is expected to move a subset of the
143 reference-only functions, ghost policy a subset of the 12 ghost-only
functions, plus the overlapping combinations. Exact counts are measured, not
predicted; the run archive reports them. `Inconclusive` is not a defect.

## 8. Boundaries

- A mapped reference/ghost entry has no real-value witness; its Tier B
  verdict cannot be upgraded to "live".
- Field/method accesses are audit records, not interpreted functions in the
  probe. Precision (e.g. `p.f == p.f`) is not claimed beyond sharing the id.
- Type substitution is syntactic; no trait-bound, layout, or `size_of`
  reasoning is performed.
- Quantifiers, triggers, `old`, constructors, unary operators, and any
  direct parameter use remain barriers unless the clause only uses the
  blocker parameter through a safe place root.
- No nightly integration; no change to the `check` flow.
