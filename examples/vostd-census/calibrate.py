#!/usr/bin/env python3
"""Compare spec-check census facts with the ensures-coverage artifact.

The artifact's `head.results` is a per-function S-expression stream. Its
`(ret ...)` and `(mut ...)` descriptors appear exactly for findings, and the
sum of those descriptors equals the report summary (235). This gives exact
per-function finding category/name without parsing the prose report.

This script is in examples/ because the artifact format is project-specific.
"""

import argparse
import collections
import json
import re
from pathlib import Path

ARTIFACT_SUMMARY = {
    "functions": 1502,
    "exec": 895,
    "proof": 607,
    "pub": 1214,
    "non_pub": 288,
    "with_functions": 1270,
    "with_outputs": 1335,
    "with_findings": 42,
    "with_return": 16,
    "with_mut": 26,
    "without_functions": 232,
    "without_outputs": 193,
    "without_findings": 193,
    "without_return": 158,
    "without_mut": 35,
}


def parse_head(path):
    head = {}
    for line in path.read_text().splitlines():
        m = re.match(r'\(fn "([^"]+)" (exec|proof)\b', line)
        if not m:
            continue
        fn, mode = m.groups()
        outs = re.search(r"\(outs (\d+)\)", line)
        post = re.search(r"\(post (own|decl|own decl)\)", line)
        trait = re.search(r'\(trait "([^"]+)"\)', line)
        name = re.search(r'\(name "([^"]*)"(?: [^)]*)?\)', line)
        at = re.search(r'\(at "([^"]+)" (\d+)\)', line)
        findings = set()
        for ret in re.findall(r'\(ret (#f|"[^"]*")', line):
            findings.add(("return_unmentioned", "" if ret == "#f" else ret.strip('"')))
        for mut in re.findall(r'\(mut "([^"]*)"', line):
            findings.add(("mut_state_unmentioned", mut))
        head[fn] = {
            "mode": mode,
            "outs": int(outs.group(1)) if outs else 0,
            "post": post.group(1) if post else None,
            "trait": trait.group(1) if trait else None,
            "name": name.group(1) if name else None,
            "inferred": " inferred)" in line,
            "external": "external-body" in line,
            "file": at.group(1) if at else None,
            "line": int(at.group(2)) if at else None,
            "findings": findings,
        }
    return head


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--artifact", required=True, type=Path)
    ap.add_argument("--census", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()

    census = json.loads(args.census.read_text())
    ours = {f["path"]: f for f in census["functions"]}
    head = parse_head(args.artifact / "head.results")

    def our_set(fn):
        return {
            (o["finding"], o["name"] or "")
            for o in fn["outputs"]
            if o["finding"]
        }

    def norm_post(value):
        value = value or "none"
        return {"own decl": "own_decl"}.get(value, value)

    inter = sorted(set(ours) & set(head))
    only_ours = sorted(set(ours) - set(head))
    only_art = sorted(set(head) - set(ours))

    out_agree = 0
    out_diff = []
    post_agree = 0
    post_diff = []
    find_agree = 0
    find_diff = []
    set_agree = 0
    set_diff = []
    category_diff = []
    cat_ours = collections.Counter()
    cat_art = collections.Counter()
    for path in inter:
        fn = ours[path]
        art = head[path]
        if len(fn["outputs"]) == art["outs"]:
            out_agree += 1
        else:
            out_diff.append(
                {
                    "path": path,
                    "ours": len(fn["outputs"]),
                    "artifact": art["outs"],
                    "artifact_post": art["post"],
                }
            )
        our_post = fn["postcondition"]["source"]
        if our_post == "decl_unavailable":
            our_post = "decl"
        art_post = norm_post(art["post"])
        if our_post == art_post:
            post_agree += 1
        else:
            post_diff.append(
                {"path": path, "ours": fn["postcondition"]["source"], "artifact": art_post}
            )
        if len(fn["findings"]) == len(art["findings"]):
            find_agree += 1
        else:
            find_diff.append(
                {
                    "path": path,
                    "ours": len(fn["findings"]),
                    "artifact": len(art["findings"]),
                    "artifact_findings": sorted(art["findings"]),
                }
            )
        os_ = our_set(fn)
        as_ = set(art["findings"])
        for kind, _ in os_:
            cat_ours[kind] += 1
        for kind, _ in as_:
            cat_art[kind] += 1
        if os_ == as_:
            set_agree += 1
        elif collections.Counter(k for k, _ in os_) == collections.Counter(k for k, _ in as_):
            category_diff.append({"path": path, "ours": sorted(os_), "artifact": sorted(as_)})
        else:
            set_diff.append({"path": path, "ours": sorted(os_), "artifact": sorted(as_)})

    inferred = {p for p, h in head.items() if h["inferred"]}
    report = {
        "run": {
            "artifact": str(args.artifact),
            "census": str(args.census),
            "census_vir": census.get("vir"),
        },
        "function_set": {
            "artifact": len(head),
            "ours": len(ours),
            "intersection": len(inter),
            "only_ours": len(only_ours),
            "only_artifact": len(only_art),
            "artifact_inferred_parser_gap": len(inferred),
            "artifact_inferred_in_ours": len(inferred & set(ours)),
            "only_ours_examples": only_ours[:10],
            "only_artifact_examples": only_art[:10],
        },
        "metrics": {
            "output_count": {
                "agree": out_agree,
                "total": len(inter),
                "rate": out_agree / len(inter),
            },
            "postcondition_source": {
                "agree": post_agree,
                "total": len(inter),
                "rate": post_agree / len(inter),
            },
            "finding_count": {
                "agree": find_agree,
                "total": len(inter),
                "rate": find_agree / len(inter),
            },
            "finding_set_exact": {
                "agree": set_agree,
                "total": len(inter),
                "rate": set_agree / len(inter),
            },
            "category_totals_on_intersection": {
                "ours": dict(cat_ours),
                "artifact": dict(cat_art),
            },
        },
        "mismatches": {
            "output_count": out_diff,
            "postcondition_source": post_diff,
            "finding_count": find_diff,
            "finding_set_or_category": set_diff,
            "finding_category_with_same_count": category_diff,
        },
        "summary": {"artifact": ARTIFACT_SUMMARY, "ours": census["counts"]},
        "annotations": {
            "decl_unavailable": [
                f["path"]
                for f in census["functions"]
                if f["postcondition"].get("decl_unavailable")
            ],
            "default_ensures_only": [
                f["path"]
                for f in census["functions"]
                if f["postcondition"].get("default_ensures_only")
            ],
        },
    }
    args.out.write_text(json.dumps(report, indent=2) + "\n")

    print("function set:", report["function_set"])
    for key, value in report["metrics"].items():
        print(key, value)
    print("output mismatches", len(out_diff))
    print("post mismatches", len(post_diff))
    print("finding-count mismatches", len(find_diff))
    print("finding-set/category mismatches", len(set_diff))
    print("category-only mismatches", len(category_diff))


if __name__ == "__main__":
    main()
