#!/usr/bin/env python3
"""Static comparison of ktest demands against function postconditions.

Reads extract-ktest candidates and a spec-check census, then assigns one of:
  run_supported / run_gap  -- existing M7b probe verdicts
  static_covered           -- the function's ensures mention the asserted output
  no_postcondition         -- the function has no effective postcondition
  not_expressible          -- the demand needs models absent from the frozen surface

Classification of unrun candidates is static, not a verification verdict.
"""
import json, sys
cands = json.load(open(sys.argv[1]))
census = json.load(open(sys.argv[2]))
by_name = {}
for f in census["functions"]:
    by_name.setdefault(f["name"], []).append(f)

# Project-side adjudication (frozen rev 43da079). Basis is archival evidence or
# source reading; see README.md.
CLASS = {
 "K007": ("run_supported", "M7b run: ObligationSupported (push value readback)"),
 "K131": ("run_gap", "M7b run: ContractGapCandidate; ensures only constrains ret is Some"),
 "K078": ("static_covered", "vm_space::query ensures cover both Ok(mapped) and Ok(None) branches"),
 "K080": ("static_covered", "vm_space::find_next ensures give res->0 == va for Some"),
 "K068": ("static_covered", "frame start_paddr inherits declaration ensures and mentions the return"),
 "K014": ("static_covered", "AtomicCpuSet::contains ensures/decl mention the return"),
 "K069": ("static_covered", "page-table node entry ensures mention return and &mut output"),
 "K075": ("no_postcondition", "disable_preempt has no effective postcondition"),
 "K070": ("not_expressible", "PageTable::cursor_mut needs tracked owner/guards and RCU model"),
 "K071": ("not_expressible", "PageTable::cursor_mut needs tracked owner/guards and RCU model"),
 "K072": ("not_expressible", "page_walk is not in the census verified surface"),
 "K073": ("not_expressible", "page_walk is not in the census verified surface"),
 "K012": ("not_expressible", "candidate is a test-local iterator property, not an interface contract"),
}
for k in [f"K{n}" for n in range(145, 154)]:
    CLASS.setdefault(k, ("static_covered", "range_difference ensures mention the returned sequence; helper iterator model absent"))

rows = []
for c in cands:
    if c.get("rating") not in ("direct", "needs_model_helper", "needs_model_quantified"):
        continue
    verdict, basis = CLASS[c["id"]]
    hits = by_name.get(c["function"], [])
    post = [(h["path"], h["postcondition"]["present"], h["postcondition"]["own_clauses"],
             h["postcondition"]["decl_clauses"], [o["mentioned"] for o in h["outputs"]]) for h in hits[:3]]
    rows.append({"id": c["id"], "function": c["function"], "rating": c["rating"],
                 "family": c["family"], "assertions": [a["kind"] for a in c.get("assertions", [])],
                 "classification": verdict, "basis": basis,
                 "census_postconditions": post})
json.dump({"schema": "spec-check.runtime-consistency/1", "rows": rows},
          open(sys.argv[3], "w"), indent=1, ensure_ascii=False)
from collections import Counter
print(len(rows), Counter(r["classification"] for r in rows))
