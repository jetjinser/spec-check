#!/usr/bin/env python3
"""Trait-declaration conformance facts from a spec-check census.json.

Reads the census fact layer, groups trait impls by declared-postcondition
presence and by external_body, and emits report.json. No defect judgement.
"""
import json, sys
census = json.load(open(sys.argv[1]))
fns = census["functions"]
impls = [f for f in fns if f["kind"] == "trait_impl"]
with_decl = [f for f in impls if f["postcondition"]["decl_clauses"] > 0 or f["postcondition"]["decl_returns"]]
ext = [f for f in with_decl if f["external_body"]]
unavailable = [f for f in impls if f["postcondition"]["decl_unavailable"]]
out = {
    "schema": "spec-check.conformance-report/1",
    "source": "census.json (spec-check.census/1)",
    "trait_impl_functions": len(impls),
    "impls_with_declared_postcondition": len(with_decl),
    "impls_with_own_and_declared": len([f for f in with_decl if f["postcondition"]["own_clauses"] > 0]),
    "external_body_with_declared_postcondition": [
        {"path": f["path"], "file": f["source"]["file"], "line": f["source"]["line"],
         "decl_clauses": f["postcondition"]["decl_clauses"],
         "own_clauses": f["postcondition"]["own_clauses"],
         "decl_returns": f["postcondition"]["decl_returns"], "trait": f.get("trait")}
        for f in ext],
    "declaration_unavailable": [
        {"path": f["path"], "file": f["source"]["file"], "line": f["source"]["line"], "trait": f.get("trait")}
        for f in unavailable],
}
json.dump(out, open(sys.argv[2], "w"), indent=1, ensure_ascii=False)
print(len(impls), len(with_decl), len(ext), len(unavailable))
