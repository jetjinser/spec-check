# Analysis map: a coordinate system for spec-quality checks

Status: design note. This is an engineering taxonomy, not a completeness theorem.

## Objects

A function contract is modelled as a tuple:

| symbol | meaning |
|---|---|
| P | precondition (`requires`) |
| Q | postcondition (`ensures`) |
| F | frame conditions (what the contract says does not change) |
| R | resource / ghost invariants (tracked state, ownership, permissions) |
| I | implementation behaviour |
| D | what a caller needs to know after the call |

## Axes

**Axis 1 — object under check:** P, Q, F, R, I, trait declarations, the model itself.

**Axis 2 — property:**

- satisfiability: can P hold at all?
- sufficiency: does Q entail D?
- soundness: does I satisfy Q?
- consistency: does Q agree with tests / runtime behaviour?
- mention coverage: is each output mentioned by Q?
- stability: does a small mutation of the contract get caught?

**Axis 3 — evidence method:** static mention analysis, SMT / satisfiability, proof
obligation, constructed witness / counterexample, runtime, differential / mutation.

Every cell (object, property, method) is a candidate analysis. Filling a cell means
writing down four things: the judgment, the certificate, the classifier
(confirmed / gap / candidate / excluded, with model scope), and the cost / coverage.

## Current map

| object × property | existing method | status |
|---|---|---|
| P × satisfiable | quantifier probes; construction probes | implemented |
| P × not too strong | — | **empty** (over-restrictive preconditions) |
| Q × sufficient | caller check programs | implemented |
| Q × mention coverage | static census | implemented |
| Q × sound | Verus verification itself | verified code only; `external_body` is a gap |
| Q × runtime consistency | ktest demand extraction | partial, not systematic |
| F × preservation | — | **empty** (high value: callers cannot know what stays unchanged) |
| R × preservation | construction probes; projection models | partial |
| trait declaration × impl conformance | census declaration inheritance | partial |
| model × faithfulness | manual audit | semi-automatic |
| contract × mutation | — | **empty** (mutation testing of specs) |

## How to read the map

- A filled cell names a method and its evidence grade. An empty cell names work
  that is defined but not implemented.
- Cells that are undecidable or require intent, such as "does Q match what the
  author meant", are labelled human-only rather than left empty.
- The map is useful for scope control: every automated finding should be
  traceable to one cell, and every claim should name its cell and its method.

## Limits of the analogy

- The lambda cube classifies a closed formal system and has proven containment
  relations. This map has design-chosen axes; a different choice of axes yields a
  different map.
- A periodic table predicts new elements. This map only points at classes of
  analyses that do not exist yet. It cannot prove that no other analysis exists.
- Some axes interact: changing the resource model changes what "satisfiable"
  means, so cells carry model scope rather than universal claims.

## Next cells to fill

1. **Frame conditions**: check whether the contract states what is preserved, and
   whether callers can rely on it.
2. **Over-restrictive preconditions**: check whether P excludes legitimate calls,
   using documented demand and real call sites.
3. **Contract/runtime consistency**: turn extracted test demands into systematic
   comparisons between Q and observed behaviour.
