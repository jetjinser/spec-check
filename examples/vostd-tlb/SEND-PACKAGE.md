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
2. Attach or link the patches:
   - `map-tlb-flush-optimized.patch`
   - `unmap-tlb-flush-dispatch-all.patch`
3. Mention the frozen rev 43da079 and the exact command; offer to open a PR on
   request.
4. Keep the A2 residual (trailing Range) explicit; do not claim it removed.

If a token or `gh` becomes available, the same body can be submitted with:

```
gh issue create -R asterinas/vostd \
  --title "TLB flush is not required by VmSpace::{map,unmap} contracts; wrong implementations verify" \
  --body-file examples/vostd-tlb/UPSTREAM-ISSUE.md
```

After submission, record the issue URL and check for feedback; do not merge or
modify upstream settings.
