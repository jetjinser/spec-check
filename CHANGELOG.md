# Changelog

## 0.3.0

- Add `mutation generate|run|triage` as the stable mutation CLI; keep
  `mutation-generate`, `analyses-mutation`, and `mutation-triage` aliases.
- Add mutation manifest validation and `mutation run --dry-run`.
- Add `schema_version` to manifests, reports, and triage output, plus JSON
  schemas under `schemas/`.
- Add per-run timeout, cached-result status, and process-group termination of
  cargo-verus/rustc/z3.
- Isolate full-crate confirmation to `<target_dir>/full`.
- Add `mutation run --mode advisory|strict`; advisory is the default.
- Add generic `mutation-triage` with region/template/registry/intent review.
- Add syntax-aware generator filtering and a Vostd-independent quickstart.
- Add CLI integration, timeout, process-group, and quickstart tests.
