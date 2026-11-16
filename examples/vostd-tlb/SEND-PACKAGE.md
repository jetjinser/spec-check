# Send package and submission blocker

Upstream: https://github.com/asterinas/vostd (the fork's `upstream` remote;
`Cargo.toml` repository field matches).

Channel check (2026-11-15):

- `gh` CLI: not installed.
- `GH_TOKEN` / `GITHUB_TOKEN`: not set.
- `~/.config/gh`: absent.
- Anonymous read access works (`git ls-remote upstream`), but opening an issue
  requires authentication.

Result: **cannot submit from this environment**. No submission is claimed.

## Manual send

1. Create the issue with `UPSTREAM-ISSUE.md` as the body (title from the file).
2. Attach or link the canonical verified series:
   - `a2-no-trailing/combined-tlb-contracts-no-trailing.patch`
   - optional historical standalone patches: `map-tlb-flush-optimized.patch`,
     `unmap-tlb-flush-dispatch-all.patch`
3. Mention the frozen rev 43da079 and the exact command; offer to open a PR on
   request.
4. State the A2 result accurately: the trailing `Range` is removed, 1502/0.
   Do not claim a real `map` implementation defect: runtime v0.16.0 flushes on
   the split path; F-029 is a contract gap.

If a token or `gh` becomes available, the same body can be submitted with:

```
gh issue create -R asterinas/vostd \
  --title "TLB flush is not required by VmSpace::{map,unmap} contracts; wrong implementations verify" \
  --body-file examples/vostd-tlb/UPSTREAM-ISSUE.md
```

After submission, record the issue URL and check for feedback; do not merge or
modify upstream settings.
