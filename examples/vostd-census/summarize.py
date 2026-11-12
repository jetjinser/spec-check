#!/usr/bin/env python3
"""Reduce a full census.json to the archived summary (counts + findings)."""

import argparse
import json
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("--census", required=True, type=Path)
ap.add_argument("--out", required=True, type=Path)
args = ap.parse_args()

full = json.loads(args.census.read_text())
findings = []
functions = []
for fn in full["functions"]:
    function_findings = []
    for output in fn["outputs"]:
        if not output["finding"]:
            continue
        item = {
            "path": fn["path"],
            "mode": fn["mode"],
            "kind": fn["kind"],
            "external_body": fn["external_body"],
            "post_source": fn["postcondition"]["source"],
            "post_present": fn["postcondition"]["present"],
            "output_kind": output["kind"],
            "name": output["name"],
            "type": output["type"],
            "ghost": output["ghost"],
            "finding": output["finding"],
            "type_invariant": output["type_invariant"],
            "type_invariant_only": output["type_invariant_only"],
            "default_ensures_mention": output["default_ensures_mention"],
        }
        findings.append(item)
        function_findings.append(output["finding"])
    functions.append(
        {
            "path": fn["path"],
            "mode": fn["mode"],
            "kind": fn["kind"],
            "post_source": fn["postcondition"]["source"],
            "post_present": fn["postcondition"]["present"],
            "outputs": len(fn["outputs"]),
            "findings": function_findings,
        }
    )

summary = {
    "schema": full["schema"],
    "vir": full["vir"],
    "crate": full["crate"],
    "counts": full["counts"],
    "functions": functions,
    "findings": findings,
    "limits": full["limits"],
}
args.out.write_text(json.dumps(summary, indent=2) + "\n")
print("functions", len(functions), "findings", len(findings), "bytes", args.out.stat().st_size)
