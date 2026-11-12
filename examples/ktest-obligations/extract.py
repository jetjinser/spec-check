#!/usr/bin/env python3
"""Prototype extractor: OSTD `#[ktest]` assertions -> caller-obligation candidates.

Read-only, build-free, regex/brace-matching based.  Input: a frozen source tree
(`ostd/src`).  Output: JSON candidate list plus statistics.

The extractor is deliberately conservative: every classification is recorded
with the evidence that produced it, and heuristics are named so that a later
productised implementation can replace them with typed HIR / VIR information.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Optional

# ---------------------------------------------------------------------------
# Lexical helpers
# ---------------------------------------------------------------------------

def mask_source(text: str) -> str:
    """Blank out comments, string literals and char literals, preserving offsets.

    Newlines are preserved so that a character offset in the masked text still
    maps to the same line in the raw text.
    """
    out = list(text)
    i, n = 0, len(text)

    def blank(a: int, b: int) -> None:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            depth = 1
            j = i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            blank(i, j)
            i = j
        elif c == "'":
            m = re.match(r"'(?:\\.|[^\\'\n])'", text[i:])
            if m:  # char literal
                blank(i, i + len(m.group(0)))
                i += len(m.group(0))
            else:  # lifetime, e.g. 'a
                i += 1
        else:
            i += 1
    return "".join(out)


def match_delim(masked: str, open_idx: int) -> int:
    """Return the index of the delimiter matching masked[open_idx]."""
    pairs = {"(": ")", "[": "]", "{": "}"}
    op = masked[open_idx]
    cl = pairs[op]
    depth = 0
    for i in range(open_idx, len(masked)):
        if masked[i] == op:
            depth += 1
        elif masked[i] == cl:
            depth -= 1
            if depth == 0:
                return i
    return -1


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def norm(s: str, limit: int = 400) -> str:
    s = re.sub(r"\s+", " ", s).strip()
    return s if len(s) <= limit else s[: limit - 3] + "..."


def split_top_commas(text: str) -> list[tuple[int, int]]:
    args, depth, start = [], 0, 0
    for i, c in enumerate(text):
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            args.append((start, i))
            start = i + 1
    args.append((start, len(text)))
    return args


def split_top_args(masked: str, open_idx: int, close_idx: int) -> list[str]:
    """Split the arguments of a macro invocation on top-level commas."""
    args, depth, start = [], 0, open_idx + 1
    for i in range(open_idx + 1, close_idx):
        c = masked[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            args.append((start, i))
            start = i + 1
    args.append((start, close_idx))
    return args


# ---------------------------------------------------------------------------
# Module path / gating
# ---------------------------------------------------------------------------

def file_module_parts(rel: Path) -> list[str]:
    parts = list(rel.with_suffix("").parts)
    if parts and parts[-1] in ("mod", "lib"):
        parts = parts[:-1]
    return parts


def enclosing_mods(masked: str, offset: int) -> list[tuple[str, int, str]]:
    """Return [(name, mod_start, attrs_window)] for mods enclosing offset."""
    found = []
    for m in re.finditer(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{", masked):
        if m.start() >= offset:
            continue
        ob = masked.find("{", m.start(), m.end())
        cb = match_delim(masked, ob)
        if 0 <= cb and m.start() < offset < cb:
            found.append((m.group(1), m.start(), masked[max(0, m.start() - 400): m.start()]))
    found.sort(key=lambda x: x[1])
    return found


def attrs_before(masked: str, pos: int) -> str:
    return masked[max(0, pos - 400): pos]


def parent_module_file(root: Path, rel: Path) -> Optional[Path]:
    if rel.parent == Path("."):
        return root / "lib.rs"
    d = rel.parent
    cand = root / d / "mod.rs"
    if cand.exists():
        return cand
    cand2 = (root / d).with_suffix(".rs")
    if cand2.exists():
        return cand2
    return None


def declaration_gate(root: Path, rel: Path) -> tuple[bool, str]:
    """Detect a `ktest_capture_gap` gate on the file's own `mod` declaration."""
    stem = rel.stem
    parent = parent_module_file(root, rel)
    if parent is None:
        return False, ""
    try:
        ptext = mask_source(parent.read_text(errors="replace"))
    except OSError:
        return False, ""
    for m in re.finditer(rf"\bmod\s+{re.escape(stem)}\s*;", ptext):
        window = ptext[max(0, m.start() - 400): m.start()]
        if "ktest_capture_gap" in window:
            return True, f"{parent.relative_to(root.parent)} declares `mod {stem};` under ktest_capture_gap"
    return False, ""


def crate_disconnection(root: Path, rel: Path) -> tuple[bool, str]:
    """Detect modules that are absent from the crate compilation.

    Two patterns occur in this snapshot:
      * a top-level module (`io`) is commented out in `lib.rs`;
      * the included `arch` path is x86 while the file lives under `arch/riscv`.
    """
    top = rel.parts[0] if rel.parts else ""
    try:
        lib = mask_source((root / "lib.rs").read_text(errors="replace"))
    except OSError:
        return False, ""
    if top and top not in ("lib.rs", "specs"):
        active = re.search(rf"\bpub\s+mod\s+{re.escape(top)}\s*;", lib)
        # `pub mod io;` is inside a block comment, so mask_source erased it.
        if not active:
            return True, f"top-level module `{top}` is not declared (or commented out) in lib.rs"
    raw_lib = (root / "lib.rs").read_text(errors="replace")
    if rel.parts[:2] == ("arch", "riscv"):
        if re.search(r'#\[path\s*=\s*"arch/riscv/mod\.rs"\]\s*\n\s*pub\s+mod\s+arch', raw_lib) is None:
            return True, "lib.rs selects a non-riscv `arch` path; arch/riscv tests are not compiled"
    return False, ""


# ---------------------------------------------------------------------------
# Assertion / demand extraction
# ---------------------------------------------------------------------------

PRIVATE_RE = re.compile(
    r"\.lock\.load\b|INNERS\s*\[|\.callbacks\b|\.root\b|\.inner\b|"
    r"\.tracked_perm\b|\.0\.data\b|BAR\.load\s*\(|FOO\.get_with\b|"
    r"\.atomic\b|\.data\b"
)

ENV_RE = re.compile(
    r"\bTaskOptions\b|\bspawn\s*\(|\.wait\s*\(|wake_up\s*\(|\ball_cpus\b|"
    r"\bnum_cpus\b|KERNEL_PAGE_TABLE|\.activate\s*\(|get_activated_vm_space|"
    r"cpu_local|IrqLine|\.reader\s*\(\s*\)|\.writer\s*\(\s*\)"
)

LOOP_RE = re.compile(r"\bfor\b|\bwhile\b|\.iter\s*\(|\.into_iter\s*\(|\.collect\s*\(|\.zip\s*\(")

HELPER_HINT_RE = re.compile(r"assert_range_difference|check_map_with_property")

LITERAL_RE = re.compile(
    r"^(?:-?\d[\d_]*(?:\.\d+)?|true|false|None|null|"
    r"[A-Z][A-Za-z0-9_:]*::[A-Za-z0-9_]+|"
    r"&?\[.*\]|&?vec!\[.*\])$"
)

METHOD_CALL_RE = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
FIELD_RE = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\b(?!\s*\()")

INVARIANT_FIELD_RE = re.compile(
    r"reference_count|start_paddr|end_paddr|as_ptr|offset|nframes|direction|"
    r"daddr|paddr|level|size|count|typ|flags|cache|meta|virt_addr|remain|avail|len\s*\("
)

# Tests whose requirements already have a research archive / divergence client.
RESEARCH_ANCHORS = {
    "sync::mutex::test::test_mutex_try_lock_does_not_unlock": "mutex-try-lock-demand",
    "boot::memory_region::test::test_sort_full_non_overlapping": "memory-region-push-test",
    "io::io_mem::allocator::test::conflict_region": "range-allocator-alloc-specific",
    "io::io_mem::allocator::test::illegal_region": "range-allocator-alloc-specific (negative controls)",
    "sync::rwarc::test::lockless_get": "divergence-manifest rwarc family (get_cloned adjacent)",
    "mm::test::io::read_write_val_fallible": "vmio-fallible-write full trace",
    "mm::test::io::read_write_u32_infallible": "vmio-fallible-write infallible control",
    "mm::frame::test::frame::frame_clone": "frame/refcount identity family",
}


def target_call(text: str) -> Optional[str]:
    calls = METHOD_CALL_RE.findall(text)
    assoc = re.findall(r"\b([A-Z][A-Za-z0-9_]*::[A-Za-z0-9_]+)\s*\(", text)
    if calls:
        recv, meth = calls[-1]
        return f"{recv}.{meth}"
    if assoc:
        return assoc[-1]
    return None


def classify_assertion(kind: str, text: str, body: str) -> tuple[str, str]:
    """Return (category, note) for one assertion snippet."""
    if PRIVATE_RE.search(text):
        return "excluded_private_observation", "reads test-visible private state, not a caller-visible API result"

    if re.search(r"\.is_none\s*\(|\.is_some\s*\(|\.is_ok\s*\(|\.is_err\s*\(", text):
        return "existence_acquisition", "Option/Result shape obligation"
    if "matches!" in text and re.search(r"\b(Some|None|Ok|Err)\s*[({]", text):
        return "existence_acquisition", "pattern-matching obligation (matches!)"
    if kind in ("assert_eq", "assert_ne") and re.search(r"(?<![A-Za-z0-9_])(Some\s*[({]|None\b)", text):
        return "existence_acquisition", "Option shape + payload value obligation"
    if re.search(r"\b(Some|None|Ok|Err)\s*\(", text) and re.search(r"\b(matches|let|if)\b", text):
        return "existence_acquisition", "pattern binding obligation"

    if kind in ("assert_eq", "assert_ne"):
        args = split_top_commas(text)
        if len(args) >= 2:
            lhs, rhs = text[args[0][0]: args[0][1]], text[args[1][0]: args[1][1]]
            if re.fullmatch(r"\s*(true|false)\s*", lhs) or re.fullmatch(r"\s*(true|false)\s*", rhs):
                return "existence_acquisition", "boolean success demand"
            lrecv = set(x[0] for x in METHOD_CALL_RE.findall(lhs)) | set(x[0] for x in FIELD_RE.findall(lhs))
            rrecv = set(x[0] for x in METHOD_CALL_RE.findall(rhs)) | set(x[0] for x in FIELD_RE.findall(rhs))
            both_valuey = bool(METHOD_CALL_RE.search(lhs) or FIELD_RE.search(lhs)) and \
                          bool(METHOD_CALL_RE.search(rhs) or FIELD_RE.search(rhs))
            cross = both_valuey and lrecv and rrecv and lrecv != rrecv
            relation = bool(INVARIANT_FIELD_RE.search(lhs) or INVARIANT_FIELD_RE.search(rhs))
            if cross or (both_valuey and relation):
                return "invariant_consistency", "relation between two object observations"
            return "value_readback", "concrete value/round-trip equality"
        return "value_readback", "equality assertion"

    # assert!(...)
    if re.search(r"\.(contains|is_empty|has_remain|all)\s*\(", text):
        if LOOP_RE.search(body) or re.search(r"\b(set|list|loaded|atomic_set)\b", text):
            return "invariant_consistency", "membership/aggregate consistency"
        return "value_readback", "query predicate on parsed value"
    if re.search(r"\.(wake_up|wake_one|wake_all|alloc|acquire|lock|try_lock|map|unmap)\s*\(", text):
        return "existence_acquisition", "success/acquisition boolean demand"
    return "value_readback", "boolean value predicate"


def collect_assertions(raw: str, masked: str, body_start: int, body_end: int, body: str):
    assertions = []
    for m in re.finditer(r"\b(assert|assert_eq|assert_ne)\s*!\s*\(", masked[body_start:body_end]):
        start = body_start + m.start()
        ob = body_start + m.end() - 1
        cb = match_delim(masked, ob)
        if cb < 0:
            continue
        kind = m.group(1)
        text = raw[start: cb + 1]
        inner = masked[ob + 1: cb]
        category, note = classify_assertion(kind, inner, body)
        assertions.append({
            "kind": kind,
            "category": category,
            "text": norm(text),
            "line": line_of(raw, start),
            "target_call": target_call(inner),
            "note": note,
        })
    return assertions


def collect_demands(raw: str, masked: str, body_start: int, body_end: int):
    """unwrap()/expect() success demands and let-pattern existence demands."""
    demands = []
    body = masked[body_start:body_end]

    for m in re.finditer(r"\.\s*(unwrap|expect)\s*\(", body):
        pos = body_start + m.start()
        prev = max(masked.rfind(";", body_start, pos), masked.rfind("{", body_start, pos),
                   masked.rfind("}", body_start, pos))
        nxt = masked.find(";", body_start + m.end())
        nxt = body_end if nxt < 0 or nxt > body_end else nxt
        snippet = raw[prev + 1: nxt + 1]
        snippet = re.sub(r"^\s*(?://[^\n]*\n\s*)*", "", snippet)
        variant = "Some or Ok"
        tm = re.search(r"let\s+\w+\s*:\s*([^=]+)=", snippet)
        if tm:
            ty = tm.group(1)
            if "Result" in ty:
                variant = "Ok"
            elif "Option" in ty:
                variant = "Some"
        demands.append({
            "kind": m.group(1),
            "demand": variant,
            "line": line_of(raw, pos),
            "text": norm(snippet),
            "target_call": target_call(snippet),
            "category": "existence_acquisition",
        })

    for m in re.finditer(r"\b(?:let|if\s+let)\s+(Some|Ok|Err)\s*\(", body):
        pos = body_start + m.start()
        nxt = masked.find(";", pos)
        nxt = body_end if nxt < 0 or nxt > body_end else nxt
        snippet = raw[pos: nxt + 1]
        demands.append({
            "kind": "let-pattern",
            "demand": m.group(1),
            "line": line_of(raw, pos),
            "text": norm(snippet),
            "target_call": target_call(snippet),
            "category": "existence_acquisition",
        })
    return demands


def collect_calls(raw: str, masked: str, body_start: int, body_end: int):
    """Ordered, de-duplicated-by-position method / associated calls."""
    body = masked[body_start:body_end]
    calls = []
    for m in re.finditer(r"\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(", body):
        name = m.group(1)
        if name in {"unwrap", "expect", "is_none", "is_some", "is_ok", "is_err",
                    "map", "clone", "into", "borrow", "borrow_mut", "len", "iter",
                    "collect", "next", "contains", "is_empty", "read", "write",
                    "load", "store", "as_ref", "as_mut_slice", "as_slice", "to_fallible"}:
            # keep map/read/write/contains/etc. but drop pure assertion helpers
            if name in {"unwrap", "expect", "is_none", "is_some", "is_ok", "is_err",
                        "as_ref", "to_fallible"}:
                continue
        calls.append({"name": name, "line": line_of(raw, body_start + m.start())})
    for m in re.finditer(r"\b([A-Z][A-Za-z0-9_]*)::([A-Za-z_][A-Za-z0-9_]*)\s*\(", body):
        calls.append({"name": f"{m.group(1)}::{m.group(2)}",
                      "line": line_of(raw, body_start + m.start())})
    bare_skip = {"if", "while", "for", "match", "Some", "Ok", "Err", "None",
                 "return", "let", "in", "move", "async", "unsafe", "loop"}
    for m in re.finditer(r"(?<![.\w:])([a-z_][A-Za-z0-9_]*)\s*\(", body):
        name = m.group(1)
        if name in bare_skip or name in {"unwrap", "expect", "vec", "format", "panic", "assert"}:
            continue
        calls.append({"name": name, "line": line_of(raw, body_start + m.start())})
    calls.sort(key=lambda c: c["line"])
    # Keep order, but collapse immediate duplicates that share line+name.
    out, seen = [], set()
    for c in calls:
        key = (c["line"], c["name"])
        if key not in seen:
            seen.add(key)
            out.append(c)
    return out


# ---------------------------------------------------------------------------
# Test discovery
# ---------------------------------------------------------------------------

def is_literal(expr: str) -> bool:
    return bool(LITERAL_RE.match(expr.strip()))


def local_functions(raw: str, masked: str):
    """Map local fn name -> (body_open, body_close) for assertion-helper lookup."""
    funcs = {}
    for m in re.finditer(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>{}]*>)?\s*\(", masked):
        po = masked.find("(", m.end() - 1)
        pc = match_delim(masked, po) if po >= 0 else -1
        if pc < 0:
            continue
        ob = masked.find("{", pc)
        if ob < 0:
            continue
        cb = match_delim(masked, ob)
        if cb < 0:
            continue
        funcs[m.group(1)] = (ob, cb)
    return funcs


def discover_tests(root: Path):
    tests = []
    for path in sorted(root.rglob("*.rs")):
        raw = path.read_text(errors="replace")
        masked = mask_source(raw)
        rel = path.relative_to(root)
        funcs = local_functions(raw, masked)
        for km in re.finditer(r"#\[ktest\b[^\]]*\]", masked):
            # collect attribute block around #[ktest]
            after = masked[km.end(): km.end() + 600]
            fn_m = re.search(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(", after)
            if not fn_m:
                continue
            fn_pos = km.end() + fn_m.start()
            fn_name = fn_m.group(1)
            attr_window = masked[max(0, km.start() - 400): fn_pos]
            # Only keep the trailing attribute run (drop previous items).
            cut = max(attr_window.rfind("}"), attr_window.rfind(";"))
            attr_window = attr_window[cut + 1:]
            attrs = re.findall(r"#\[[^\]]*\]", attr_window, re.S)
            should_panic = any("should_panic" in a for a in attrs)
            cfg_gap_attr = any("ktest_capture_gap" in a for a in attrs)

            ob = masked.find("{", fn_pos)
            cb = match_delim(masked, ob) if ob >= 0 else -1
            if cb < 0:
                continue
            body_start, body_end = ob, cb
            body_raw = raw[body_start + 1: body_end]
            body_masked = masked[body_start + 1: body_end]

            mods = enclosing_mods(masked, km.start())
            inline_gap = any("ktest_capture_gap" in a for _, _, a in mods)
            decl_gap, decl_reason = declaration_gate(root, rel)
            disconnected, disc_reason = crate_disconnection(root, rel)

            name_parts = file_module_parts(rel) + [n for n, _, _ in mods] + [fn_name]
            test_path = "::".join(name_parts)

            assertions = collect_assertions(raw, masked, body_start, body_end, body_masked)
            demands = collect_demands(raw, masked, body_start, body_end)
            calls = collect_calls(raw, masked, body_start, body_end)

            via_helper = []
            if not assertions:
                called = {c["name"] for c in calls}
                for hname, (hob, hcb) in funcs.items():
                    if hname in called and re.match(r"(assert|check)", hname):
                        hbody = masked[hob + 1: hcb]
                        hasserts = collect_assertions(raw, masked, hob, hcb, hbody)
                        for a in hasserts:
                            a["via_helper"] = hname
                        if hasserts:
                            assertions.extend(hasserts)
                            via_helper.append(hname)

            tests.append({
                "test": test_path,
                "file": str(rel),
                "line": line_of(raw, km.start()),
                "body_span": {"start_line": line_of(raw, body_start), "end_line": line_of(raw, cb)},
                "attrs": [norm(a, 120) for a in attrs],
                "should_panic": should_panic,
                "capture_gap": inline_gap or cfg_gap_attr or decl_gap,
                "capture_gap_reasons": list(filter(None, [
                    "inline module/attribute gated by ktest_capture_gap" if (inline_gap or cfg_gap_attr) else "",
                    decl_reason,
                ])),
                "disconnected": disconnected,
                "disconnected_reason": disc_reason,
                "assertions": assertions,
                "demands": demands,
                "call_sequence": calls,
                "assertions_via_helper": via_helper,
                "body": body_raw,
            })
    return tests


# ---------------------------------------------------------------------------
# Rating / categorisation
# ---------------------------------------------------------------------------

def has_private_only(assertions) -> bool:
    cats = [a["category"] for a in assertions]
    return bool(cats) and all(c == "excluded_private_observation" for c in cats)


def rate_test(t) -> tuple[str, list[str], list[str]]:
    """Return (rating, reasons, kept_assertion_categories)."""
    reasons = []
    assertions = t["assertions"]
    demands = t["demands"]

    if t["file"].endswith("lib.rs"):
        return "excluded", ["crate harness self-test, not an API requirement"], []

    if t["should_panic"]:
        return "excluded", ["#[should_panic]: expected panic is outside the obligation-client model"], []

    kept = [a for a in assertions if a["category"] != "excluded_private_observation"]
    cats = sorted({a["category"] for a in kept})

    if not kept:
        if assertions:
            return "excluded", ["all assertions are test-internal observations (private/tautology), not caller obligations"], []
        if not demands:
            return "excluded", ["no assertion and no success demand"], []
        reasons.append("only success demands (unwrap/expect) remain; no value/shape assertion")

    if t["capture_gap"]:
        reasons.extend(t["capture_gap_reasons"])
        return "blocked", reasons + ["excluded from the proof-side ktest capture"], cats
    if t["disconnected"]:
        return "blocked", [t["disconnected_reason"]], cats
    if ENV_RE.search(t["body"]):
        reasons.append("assertions depend on scheduler/hardware/global runtime state")
        return "blocked", reasons, cats

    if has_private_only(assertions):
        return "needs_model_private", ["all assertions read private/internal state"], cats

    if HELPER_HINT_RE.search(t["body"]) or t.get("assertions_via_helper"):
        helpers = ", ".join(t.get("assertions_via_helper", [])) or "source helper"
        reasons.append(f"assertions live in helper ({helpers}); call trace must be inlined")
        return "needs_model_helper", reasons, cats

    if LOOP_RE.search(t["body"]):
        reasons.append("loops/iterators/collections; obligation needs quantified modelling")
        return "needs_model_quantified", reasons, cats

    if re.search(r"alloc_(frame|segment)|alloc_segment_with|FrameAllocOptions", t["body"]) and \
       re.search(r"start_paddr\s*\(\s*\)\s*,?\s*(paddr_before|paddr)|paddr_before", t["body"]):
        reasons.append("asserts physical-address reuse across allocations (allocator policy, not API contract)")
        return "test_assumption", reasons, cats

    if any("test_assumption_allocator_reuse" == a.get("exclusion_reason", "") for a in assertions):
        return "test_assumption", reasons, cats

    if "invariant_consistency" in cats:
        reasons.append("cross-object invariant; requires exported relation predicate / tracked model")
        return "needs_model_invariant", reasons, cats

    if all(a["category"] in ("value_readback", "existence_acquisition") for a in kept):
        # Direct if the expected values are literals or derive from call arguments,
        # and no assertion compares addresses of independent allocations.
        if re.search(r"start_paddr\s*\(.*\)\s*,\s*[a-z_]+_before|paddr\s*\(\s*\)\s*,\s*[a-z_]+_before", t["body"]):
            return "test_assumption", ["address reuse across allocations"], cats
        reasons.append("public API call + observable value/shape assertion")
        return "direct", reasons, cats

    reasons.append("mixed/unsupported assertion forms")
    return "needs_model", reasons, cats


def annotate_exclusions(t) -> list[str]:
    """Attach per-assertion exclusion reasons and return test-level ones."""
    notes = []
    for a in t["assertions"]:
        if a["category"] == "excluded_private_observation":
            a["exclusion_reason"] = "test_assumption_private_observation"
            continue
        # literal-vs-literal tautology
        ob = a["text"].find("(")
        if ob >= 0 and a["kind"] in ("assert_eq", "assert_ne"):
            cb = match_delim(a["text"], ob)
            args = split_top_args(a["text"], ob, cb) if cb > 0 else []
            if len(args) >= 2 and is_literal(a["text"][args[0][0]:args[0][1]]) and \
               is_literal(a["text"][args[1][0]:args[1][1]]):
                a["category"] = "excluded_tautology"
                a["exclusion_reason"] = "tautology"
        if t["body"].count("drop(") and "start_paddr" in a["text"] and re.search(r"_before|\bpaddr\b", a["text"]):
            a["category"] = "excluded_test_assumption"
            a["exclusion_reason"] = "test_assumption_allocator_reuse"
    return notes


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("source_root", type=Path, help="path to the frozen ostd/src directory")
    ap.add_argument("--out", type=Path, default=Path("candidates.json"))
    ap.add_argument("--json-indent", type=int, default=1)
    args = ap.parse_args()

    root = args.source_root
    if not root.is_dir():
        print(f"error: {root} is not a directory", file=sys.stderr)
        return 2

    tests = discover_tests(root)
    candidates = []
    for i, t in enumerate(tests, 1):
        annotate_exclusions(t)
        rating, reasons, cats = rate_test(t)
        primary = "excluded"
        if rating != "excluded":
            priority = ["state_transition", "existence_acquisition",
                        "invariant_consistency", "value_readback"]
            for p in priority:
                if p in cats:
                    primary = p
                    break
            if primary == "excluded" and t["demands"]:
                primary = "existence_acquisition"
        # state-transition heuristic is test-body local (drop/free/re-acquire/repeated op)
        name = t["test"].split("::")[-1]
        state_name = re.search(r"try_lock|wake|twice|again|reorder|unmap|"
                               r"alloc_and_free|dealloc|_drop|activ|flusher|"
                               r"map_twice|lockless", name)
        state_body = bool(re.search(r"\bdrop\s*\(", t["body"])) or \
                     (bool(re.search(r"\.(clone|clone_ro)\s*\(", t["body"])) and
                      bool(re.search(r"is_none|None\)", t["body"]))) or \
                     (len(re.findall(r"\.(try_lock|wake_up|acquire|alloc_specific|map|unmap|query)\s*\(", t["body"])) >= 2
                      and bool(re.search(r"is_none|is_some|is_err|is_ok", t["body"])))
        if rating != "excluded" and (state_name or state_body):
            if primary in ("existence_acquisition", "invariant_consistency", "value_readback"):
                primary = "state_transition"
            if "state_transition" not in cats:
                cats = ["state_transition"] + cats

        kept_assertions = [a for a in t["assertions"] if not a["category"].startswith("excluded_")]
        priority_score = {"direct": 30, "needs_model": 12, "needs_model_invariant": 10,
                          "needs_model_quantified": 8, "needs_model_helper": 18,
                          "needs_model_private": 6, "test_assumption": 2,
                          "blocked": 3, "excluded": 0}.get(rating, 0)
        if primary == "existence_acquisition":
            priority_score += 6
        if primary == "state_transition":
            priority_score += 5
        if any(a["kind"] in ("assert_eq", "assert_ne") for a in kept_assertions):
            priority_score += 2
        priority_score += min(len(kept_assertions), 6)
        anchor = RESEARCH_ANCHORS.get(t["test"], "")
        if anchor:
            priority_score += 20

        candidates.append({
            "id": f"K{i:03d}",
            "test": t["test"],
            "file": t["file"],
            "line": t["line"],
            "body_span": t["body_span"],
            "attrs": t["attrs"],
            "should_panic": t["should_panic"],
            "capture_gap": t["capture_gap"],
            "capture_gap_reasons": t["capture_gap_reasons"],
            "disconnected": t["disconnected"],
            "disconnected_reason": t["disconnected_reason"],
            "categories": cats,
            "category": primary,
            "assertions": t["assertions"],
            "demands": t["demands"],
            "call_sequence": t["call_sequence"],
            "assertions_via_helper": t["assertions_via_helper"],
            "rating": rating,
            "rating_reasons": reasons,
            "research_anchor": anchor,
            "priority_score": priority_score,
        })

    # ---------------- statistics ----------------
    rated = [c for c in candidates if c["rating"] != "excluded"]
    by_rating = {}
    for c in candidates:
        by_rating[c["rating"]] = by_rating.get(c["rating"], 0) + 1
    by_category = {}
    for c in rated:
        by_category[c["category"]] = by_category.get(c["category"], 0) + 1
    by_assert_cat = {}
    n_assertions = n_demands = 0
    all_assertions = 0
    all_demands = 0
    for c in candidates:
        all_assertions += len(c["assertions"])
        all_demands += len(c["demands"])
    for c in rated:
        n_assertions += len([a for a in c["assertions"] if not a["category"].startswith("excluded_")])
        n_demands += len(c["demands"])
        for a in c["assertions"]:
            by_assert_cat[a["category"]] = by_assert_cat.get(a["category"], 0) + 1
        for d in c["demands"]:
            by_assert_cat["demand:" + d["demand"]] = by_assert_cat.get("demand:" + d["demand"], 0) + 1

    stats = {
        "tests_seen": len(candidates),
        "candidates": len(rated),
        "excluded": len(candidates) - len(rated),
        "capture_gap_tests": sum(1 for c in candidates if c["capture_gap"]),
        "disconnected_tests": sum(1 for c in candidates if c["disconnected"]),
        "should_panic_tests": sum(1 for c in candidates if c["should_panic"]),
        "by_rating": dict(sorted(by_rating.items())),
        "by_category": dict(sorted(by_category.items())),
        "by_assertion_category": dict(sorted(by_assert_cat.items())),
        "kept_assertions": n_assertions,
        "all_assertions_extracted": all_assertions,
        "assertions_via_helper_tests": sum(1 for c in candidates if c["assertions_via_helper"]),
        "success_demands": n_demands,
        "all_demands_extracted": all_demands,
    }

    payload = {
        "schema": "m7b-ktest-candidates-v1",
        "source_root": str(root),
        "extractor": "extract.py (prototype, regex/brace-matching)",
        "stats": stats,
        "candidates": sorted(candidates, key=lambda c: (-c["priority_score"], c["test"])),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=args.json_indent, ensure_ascii=False) + "\n")
    print(json.dumps(stats, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
