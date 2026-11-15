# Lock guards — threat model, contract matrix, negative coverage

Evidence note: the matrix and P3/P4 were written after the B1 run; no per-candidate prediction was pre-registered. B2–B4 are argued from the type/contracts, not run.

Scope: `Mutex` (primary), with the same guard/linear-permission pattern in
`RwLock` and `SpinLock`.

## Properties

- **P3 (failure semantics / progress)**: `try_lock` on a free mutex returns
  `Some`; `None` means the mutex was held. A `try_lock` that always returns
  `None` is a deadlock/liveness defect.
- **P4 (exclusivity and identity)**: a returned guard is for this mutex, and
  two guards for the same mutex cannot exist at once; writing through a guard
  changes the mutex's value.

P4 is the memory-safety property.

## Contract matrix

| item | contract |
|---|---|
| `Mutex::try_lock` | `ensures ret is Some ==> ret->0.rel_mutex(*self)` only |
| `rel_mutex` | `*self.mutex == mutex` (guard identity, no value/lock-state relation) |
| `Mutex::lock` | no `#[verus_spec]` at all |
| `Mutex::acquire_lock` | `ret ==> locked_state is Some`; `!ret ==> locked_state is None` (internal, exact) |
| `MutexGuard::new` | requires `perm.id() == mutex.cell_id()`, ensures `ret.rel_mutex(*mutex)` |
| `MutexGuard::deref` | returns `self.view()` |
| `MutexGuard::deref_mut` | `final(self).view() == *final(ret)`, `old(self).view() == *ret` |
| `MutexGuard::drop` | manual `drop(self)`, no ensures |

The guard is built from a tracked `PointsTo<T>` consumed by
`acquire_lock`. That linear permission, not the public contract, is what makes
P4 hold.

## B1: always-`None` try_lock — machine witness (liveness)

Body replaced by `None` (`apply_b1.py`). Result on the frozen rev:
**1495 verified / 0 errors**. The public contract is vacuous for `None`, so the
wrong body is accepted. This violates P3: a free mutex can never be acquired
through `try_lock`.

Patch status: **blocked in place**. The contract needs `ret is None ==>
mutex-locked-at-entry`, but a `Mutex` exposes no ghost predicate for the lock
bit (`AtomicBool`'s ghost value is private to the `wf` invariant). Adding a
mirror ghost field to a `const fn`-constructed `Mutex` whose methods take
`&self` is an API/proof redesign, not a minimal spec patch. Options:

1. expose a closed `Mutex::is_locked` spec derived from the atomic ghost
   (requires changing `struct_with_invariants!`/atomic ghost API);
2. add a caller-held ghost token updated by acquire/release;
3. leave as a documented liveness limitation (current state; see F-013 for the
   converse direction, held ⇒ None).

## B2: snapshot `deref_mut` — not constructible

A wrong `deref_mut` returning a reference to a copy would satisfy the two-state
view clauses at return, but `T: ?Sized` and the absence of a second owned value
mean the body cannot materialize such a reference. Constructing one needs a
leaked allocation plus `T: Clone`/`Sized`; the generic signature excludes it.
No witness.

## B3: forged `Some` guard while held — blocked by linearity

Returning `Some` requires a `MutexGuard::new`, whose precondition is the
`PointsTo<T>` permission for this mutex. `acquire_lock` consumes that
permission when it succeeds and returns `None` when it fails, so a held mutex
has no permission available to build a guard. The type system, not the
contract, rejects the forged guard. No witness.

## B4: `lock` with no contract — capability gap, not a safety hole

`lock` has no spec, so callers cannot derive `guard@ == mutex@` or the lock
state. A wrong body still cannot violate P4: it must obtain the same linear
permission to return a guard. The missing contract limits what callers can
prove; it does not admit an unsafe implementation. No witness.

## Conclusion (negative for the safety question)

Within B1–B4, **no wrong implementation that violates exclusivity/identity (P4)
passes verification**. Lock safety is enforced by linear permissions and guard
construction preconditions; the public contracts are weaker than the mechanism
but not the safety boundary.

The one machine witness (B1) violates failure/progress semantics (P3), and its
contract fix is blocked by missing ghost-state exposure, not by proof effort.
This is a liveness residual, not a memory-safety hole.

Registry: F-031 (`wrong_implementation_witness`, liveness scope).
