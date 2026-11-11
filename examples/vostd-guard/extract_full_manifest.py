#!/usr/bin/env python3
"""Extract the full 60-interface probe manifest from the guard-return inventory.

Reads guard-return-inventory.csv, locates each interface's `#[verus_spec(...)]`
contract (or in-signature requires for spec/proof/axiom fns) in the frozen
vostd source, and transcribes every requires clause into manifest entries:

- tracked parameters (Tracked<...>) are abstracted to opaque usize stand-ins
  (discipline carried over from examples/vostd-guard/EXTRACTION.md);
- scalar comparisons/arithmetic over lifted scalars/consts are transcribed
  mechanically as named `open spec fn`s (quantifier triggers);
- clauses that cannot be expressed over plain parameters become named
  `uninterp spec fn` declarations (constraint structure kept by name, body
  abstracted); every clause is recorded in EXTRACTION-NOTES.md with its
  abstraction loss;
- interfaces whose contract cannot be located are recorded as
  extraction_skipped with the reason — no entries are fabricated.

Outputs:
- full-manifest.json   (anchor entries reused verbatim from manifest.json,
                        auto-extracted entries, M1 controls, construction
                        entries reused verbatim from tracked/manifest.json)
- EXTRACTION-NOTES.md  (clause-by-clause mapping + pre-registered expected
                        verdicts, written BEFORE any run)

Judgments come only from analyses-vacuity runs (see FULL-RUN-60.md).
"""

import argparse
import csv
import json
import os
import re
from collections import OrderedDict

DEFAULT_CSV = ("/home/jinser/vie/work/AES/research/experiments/"
               "spec-quality-two-day/guard-return-inventory.csv")
DEFAULT_SRC = "/tmp/vostd-43da079-ktest-chan"
HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_M1_MANIFEST = os.path.join(HERE, "manifest.json")
DEFAULT_TRACKED_MANIFEST = os.path.join(HERE, "tracked", "manifest.json")
DEFAULT_OUT_MANIFEST = os.path.join(HERE, "full-manifest.json")
DEFAULT_OUT_NOTES = os.path.join(HERE, "EXTRACTION-NOTES.md")

SCALAR_TYPES = {"usize", "u64", "u32", "u16", "u8", "u128", "int", "i32",
                "i64", "i16", "i8", "nat", "bool", "char"}

# Known-vacuous stub contracts (CSV entry_precondition_vacuous=true; the 7
# spans cover 14 CSV rows because each stub shares its span with its
# _VERUS_VERIFIED twin) are pinned to the M1 archive entries verbatim.
ANCHOR_MAP = {
    ("ostd/src/mm/page_table/cursor/locking.rs", 384):
        "try_traverse_and_lock_subtree_root_shape",
    ("ostd/src/mm/page_table/node/entry.rs", 616): "alloc_if_none_shape",
    ("ostd/src/mm/page_table/node/entry.rs", 968): "split_if_mapped_huge_shape",
    ("ostd/src/mm/page_table/node/entry.rs", 2141): "alloc_absent_child_shape",
    ("ostd/src/mm/page_table/node/mod.rs", 375): "node_lock_shape",
    ("ostd/src/mm/page_table/node/mod.rs", 401): "make_guard_unchecked_shape",
    ("ostd/src/sync/mutex.rs", 225): "mutex_guard_new_shape",
}

M1_CONTROLS = {"mutex_new_any_value_control": "ConfirmedLive",
               "arith_bounds_live_control": "ConfirmedLive",
               "arith_oob_vacuous_control": "ConfirmedVacuous"}
TRACKED_EXPECTED = {"mutex_guard_new_construction": "ConfirmedVacuous",
                    "mutex_guard_new_holder_face": "ConfirmedLive",
                    "mutex_guard_new_quantifier_shape": "ConfirmedLive"}

ITEM_BOUNDARY_RE = re.compile(
    r"^\s*(?:}\s*$|pub (?:unsafe )?(?:async )?(?:extern )?"
    r"(?:open |closed |spec |proof |axiom )?fn |fn |impl |struct |enum "
    r"|trait |mod |use |const |static |type )")

SECTIONS = ("with", "requires", "ensures", "invariant", "decreases")


# ---------------------------------------------------------------------------
# Source location
# ---------------------------------------------------------------------------

def collect_verus_spec_attrs(lines):
    """All `#[verus_spec ...]` blocks as (start, end, text), 0-based inclusive."""
    out = []
    for j, t in enumerate(lines):
        if "#[verus_spec" not in t:
            continue
        if any(s <= j <= e for s, e, _ in out):
            continue
        buf, depth, seen_open, k = [], 0, False, j
        while k < len(lines):
            tt = lines[k]
            done = False
            for ch in tt:
                if ch in "([{":
                    depth += 1
                    seen_open = True
                elif ch in ")]}":
                    depth -= 1
                    if seen_open and depth == 0:
                        buf.append(tt)
                        out.append((j, k, "\n".join(buf)))
                        done = True
                        break
            if done:
                break
            buf.append(tt)
            k += 1
    return out


def gap_is_attrs_or_comments(lines, lo, hi):
    """lines[lo:hi] must contain only blanks, comments, or other attributes."""
    for j in range(lo, hi):
        st = lines[j].strip()
        if st == "" or st.startswith("//") or st.startswith("#["):
            continue
        return False
    return True


def balanced_signature(lines, start):
    """From the fn decl line, collect through the `{`/`;` that closes the
    in-signature contract (all brackets balanced before the final brace)."""
    buf, j = [], start
    while j < min(len(lines), start + 200):
        t = lines[j]
        ends = t.rstrip().endswith("{") or t.rstrip().endswith(";")
        buf.append(t)
        core = re.sub(r"[{;]\s*$", "", "\n".join(buf))
        if (ends and core.count("(") == core.count(")")
                and core.count("{") == core.count("}")
                and core.count("[") == core.count("]")):
            return "\n".join(buf)
        j += 1
    return "\n".join(buf)


def locate_contract(lines, fn_line):
    """Return (kind, text) with kind in {attr, signature, none}."""
    for s, e, text in collect_verus_spec_attrs(lines):
        if e < fn_line and gap_is_attrs_or_comments(lines, e + 1, fn_line):
            return "attr", text
    sig = balanced_signature(lines, fn_line)
    if re.search(r"\brequires\b", sig):
        return "signature", sig
    return "none", None


# ---------------------------------------------------------------------------
# Contract text parsing
# ---------------------------------------------------------------------------

def split_top(s, angles=False):
    """Split on top-level commas (optionally angle-aware for type lists)."""
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif angles and ch == "<":
            depth += 1
        elif angles and ch == ">":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


def parse_verus_spec_attr(text):
    """Parse attr text -> dict(ret, with, requires, ensures)."""
    t = text.strip()
    if t == "#[verus_spec]":
        return {"ret": None, "with": [], "requires": [], "ensures": []}
    inner = t[len("#[verus_spec"):].strip()
    inner = inner[1:inner.rfind(")")]  # strip leading '(' and trailing ')'
    ret = None
    m = re.match(r"^\s*([A-Za-z_]\w*)\s*=>", inner)
    if m and m.group(1) != "with":
        ret = m.group(1)
        inner = inner[m.end():]
    out = {"ret": ret, "with": [], "requires": [], "ensures": []}
    cur, chunks = None, []
    for line in inner.split("\n"):
        st = line.strip()
        m = re.match(r"(with|requires|ensures|invariant|decreases)\b\s*(.*)$",
                     st)
        if m:
            if cur is not None:
                out[cur].append(" ".join(chunks))
            cur, chunks = m.group(1), ([m.group(2)] if m.group(2) else [])
            continue
        chunks.append(st)
    if cur is not None:
        out[cur].append(" ".join(chunks))
    for k in ("with", "requires", "ensures"):
        body = " ".join(x for x in out[k] if x)
        out[k] = split_top(body, angles=(k == "with")) if body else []
    return out


def parse_signature_contract(text):
    """For spec/proof/axiom fns: requires/ensures live in the signature."""
    out = {"with": [], "requires": [], "ensures": []}
    m = re.search(r"\)\s*(requires|ensures)\b", text)
    if not m:
        return out
    region = text[m.start(1):]
    region = re.sub(r"(\{|;)\s*$", "", region).rstrip()
    parts = re.split(r"\b(requires|ensures|invariant|decreases)\b", region)
    cur = None
    for p in parts:
        if p in ("requires", "ensures", "invariant", "decreases"):
            cur = p if p in out else None
        elif cur is not None:
            out[cur].append(p)
    for k in ("requires", "ensures"):
        body = re.sub(r"\{\s*$", "", " ".join(out[k]))
        out[k] = split_top(body) if body.strip() else []
    return out


def parse_fn_params(sig_text):
    """Extract (name, type) list from a fn signature's parameter list."""
    m = re.search(r"\bfn\b[^{;]*?\(", sig_text, re.S)
    if not m:
        return []
    start = m.end() - 1
    depth, j = 0, start
    while j < len(sig_text):
        if sig_text[j] == "(":
            depth += 1
        elif sig_text[j] == ")":
            depth -= 1
            if depth == 0:
                break
        j += 1
    params = []
    for raw in split_top(sig_text[start + 1:j], angles=True):
        p = raw.strip()
        if not p:
            continue
        p = re.sub(r"^(mut|tracked|Ghost)\s+", "", p)
        p = re.sub(r"^@\s*", "", p)
        if p in ("self",) or p.startswith(("&self", "&mut self")):
            params.append(("self", p))
            continue
        pm = re.match(r"(&*\s*mut\s+)?(&*\s*)?([A-Za-z_]\w*)\s*:\s*(.+)",
                      p, re.S)
        if pm:
            params.append((pm.group(3), pm.group(4).strip()))
        else:
            params.append((None, p))
    return params


# ---------------------------------------------------------------------------
# Clause classification (mechanical vs ghost) and transcription
# ---------------------------------------------------------------------------

MECH_TOKEN_RE = re.compile(
    r"\s*([A-Za-z_][A-Za-z0-9_]*|\d+|==>|==|!=|<=|>=|&&|\|\||[-+*/%<>()!])")


def tokenize_mech(clause):
    """Token list if the clause is purely in the mechanical scalar grammar."""
    toks, pos = [], 0
    while pos < len(clause):
        m = MECH_TOKEN_RE.match(clause, pos)
        if not m:
            return None
        toks.append(m.group(1))
        pos = m.end()
    return toks


class MechExpr:
    """Tiny AST for a mechanical clause (verus spec arithmetic = math ints)."""

    def __init__(self, kind, *kids):
        self.kind = kind
        self.kids = kids

    def eval(self, env):
        k = self.kind
        if k == "num":
            return int(self.kids[0])
        if k == "bool":
            return self.kids[0] == "true"
        if k == "var":
            return env[self.kids[0]]
        if k == "not":
            return not self.kids[0].eval(env)
        if k == "neg":
            return -self.kids[0].eval(env)
        if k in ("and", "or", "implies"):
            a = self.kids[0].eval(env)
            b = self.kids[1].eval(env)
            return (a and b) if k == "and" else (a or b) if k == "or" \
                else (not a) or b
        a, b = self.kids[0].eval(env), self.kids[1].eval(env)
        if k == "+":
            return a + b
        if k == "-":
            return a - b
        if k == "*":
            return a * b
        if k == "/":
            return a // b
        if k == "%":
            return a % b
        return {"==": a == b, "!=": a != b, "<": a < b, "<=": a <= b,
                ">": a > b, ">=": a >= b}[k]

    def render(self):
        k = self.kind
        if k in ("num", "var", "bool"):
            return self.kids[0]
        if k == "not":
            return "!" + self._wrap(self.kids[0])
        if k == "neg":
            return "-" + self._wrap(self.kids[0])
        sym = {"and": "&&", "or": "||", "implies": "==>"}.get(k, k)
        return f"{self._wrap(self.kids[0])} {sym} {self._wrap(self.kids[1])}"

    def _wrap(self, kid):
        r = kid.render()
        return f"({r})" if kid.kind in ("and", "or", "implies") else r


class MechParser:
    """Recursive descent over the mechanical token grammar."""

    CMP = {"==": "==", "!=": "!=", "<": "<", "<=": "<=", ">": ">", ">=": ">="}

    def __init__(self, toks):
        self.toks, self.pos = toks, 0

    def peek(self):
        return self.toks[self.pos] if self.pos < len(self.toks) else None

    def take(self, tok=None):
        t = self.peek()
        if tok is not None and t != tok:
            raise ValueError(f"expected {tok}, got {t}")
        self.pos += 1
        return t

    def parse(self):
        e = self.parse_implies()
        if self.pos != len(self.toks):
            raise ValueError("trailing tokens")
        return e

    def parse_implies(self):
        left = self.parse_or()
        if self.peek() == "==>":
            self.take()
            return MechExpr("implies", left, self.parse_implies())
        return left

    def parse_or(self):
        left = self.parse_and()
        while self.peek() == "||":
            self.take()
            left = MechExpr("or", left, self.parse_and())
        return left

    def parse_and(self):
        left = self.parse_cmp()
        while self.peek() == "&&":
            self.take()
            left = MechExpr("and", left, self.parse_cmp())
        return left

    def parse_cmp(self):
        left = self.parse_add()
        t = self.peek()
        if t in self.CMP:
            self.take()
            return MechExpr(self.CMP[t], left, self.parse_add())
        return left

    def parse_add(self):
        left = self.parse_mul()
        while self.peek() in ("+", "-"):
            left = MechExpr(self.take(), left, self.parse_mul())
        return left

    def parse_mul(self):
        left = self.parse_unary()
        while self.peek() in ("*", "/", "%"):
            left = MechExpr(self.take(), left, self.parse_unary())
        return left

    def parse_unary(self):
        t = self.peek()
        if t == "!":
            self.take()
            return MechExpr("not", self.parse_unary())
        if t == "-":
            self.take()
            return MechExpr("neg", self.parse_unary())
        return self.parse_atom()

    def parse_atom(self):
        t = self.take()
        if re.fullmatch(r"\d+", t):
            return MechExpr("num", t)
        if t in ("true", "false"):
            return MechExpr("bool", t)
        if re.fullmatch(r"[A-Za-z_]\w*", t):
            return MechExpr("var", t)
        if t == "(":
            e = self.parse_implies()
            self.take(")")
            return e
        raise ValueError(f"bad atom {t}")


def classify_clause(clause, scalar_names, const_names):
    """-> ('mechanical', ast, arg_names) | ('ghost', idents)."""
    toks = tokenize_mech(clause)
    if toks:
        idents = [t for t in toks if re.fullmatch(r"[A-Za-z_]\w*", t)
                  and t not in ("true", "false")]
        if all(i in scalar_names or i in const_names for i in idents):
            try:
                ast = MechParser(toks).parse()
            except ValueError:
                pass
            else:
                args, seen = [], set()
                for i in idents:
                    if i not in seen:
                        seen.add(i)
                        args.append(i)
                return "mechanical", ast, args
    return "ghost", None, re.findall(r"[A-Za-z_]\w*", clause)


def uninterp_slug(idents, max_ids=3):
    seen, out = set(), []
    for i in idents:
        i = i.lower()
        if i not in seen and i not in ("old", "final", "int", "usize", "get"):
            seen.add(i)
            out.append(i)
        if len(out) >= max_ids:
            break
    return "_".join(out) if out else "pred"


# ---------------------------------------------------------------------------
# Witness search (mechanical clauses only; spec arithmetic = math ints)
# ---------------------------------------------------------------------------

def search_witness(constraints, var_types, node_cap=300000):
    """constraints: list of (arg_vars, ast). Returns values in
    first-appearance var order, or None."""
    var_order, seen = [], set()
    for cargs, _ in constraints:
        for v in cargs:
            if v not in seen:
                seen.add(v)
                var_order.append(v)
    if not var_order:
        return []
    nodes = [0]

    def feasible(env):
        for cargs, ast in constraints:
            if all(v in env for v in cargs) and \
                    not ast.eval({v: env[v] for v in cargs}):
                return False
        return True

    def dfs(i, env):
        if nodes[0] > node_cap:
            return None
        nodes[0] += 1
        if i == len(var_order):
            return [env[v] for v in var_order]
        v = var_order[i]
        dom = [False, True] if var_types.get(v) == "bool" else range(8)
        for val in dom:
            env[v] = val
            if feasible(env):
                r = dfs(i + 1, env)
                if r is not None:
                    return r
            del env[v]
        return None

    return dfs(0, {})


def render_witness(values, var_order, var_types):
    parts = []
    for v, val in zip(var_order, values):
        if var_types.get(v) == "bool":
            parts.append("true" if val else "false")
        else:
            parts.append(f"{val}usize")
    return parts[0] if len(parts) == 1 else "(" + ", ".join(parts) + ")"


# ---------------------------------------------------------------------------
# Extraction driver
# ---------------------------------------------------------------------------

def short_fn_name(function):
    return function.split("::")[-1].replace("&%", "impl").replace("%", "")


def base_type(ty):
    t = ty.replace("&", "").replace("mut", "").strip()
    t = re.sub(r"^(tracked|Ghost)\s*", "", t).strip()
    t = re.sub(r"<.*", "", t).strip()
    return t


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--csv", default=DEFAULT_CSV)
    ap.add_argument("--src", default=DEFAULT_SRC)
    ap.add_argument("--m1-manifest", default=DEFAULT_M1_MANIFEST)
    ap.add_argument("--tracked-manifest", default=DEFAULT_TRACKED_MANIFEST)
    ap.add_argument("--out-manifest", default=DEFAULT_OUT_MANIFEST)
    ap.add_argument("--out-notes", default=DEFAULT_OUT_NOTES)
    args = ap.parse_args()

    rows = list(csv.DictReader(open(args.csv)))
    assert len(rows) == 60, f"expected 60 rows, got {len(rows)}"

    file_cache = {}

    def get_lines(path):
        if path not in file_cache:
            file_cache[path] = open(os.path.join(args.src, path)).read() \
                .split("\n")
        return file_cache[path]

    located = []
    for i, r in enumerate(rows):
        m = re.match(r"(.+?):(\d+):\d+", r["source_span"])
        path, line = m.group(1), int(m.group(2))
        lines = get_lines(path)
        kind, text = locate_contract(lines, line - 1)
        located.append(dict(row=i, fn=r["function"], path=path, line=line,
                            kind=kind, text=text))

    # group twin rows sharing one contract (same path+line)
    groups, by_key = [], {}
    for loc in located:
        key = (loc["path"], loc["line"])
        if loc["kind"] == "none":
            groups.append(dict(rows=[loc["row"]], path=loc["path"],
                               line=loc["line"], kind="none", text=None,
                               fns=[loc["fn"]]))
            continue
        if key in by_key:
            g = by_key[key]
            assert g["text"] == loc["text"], \
                f"span {loc['path']}:{loc['line']} text mismatch"
            g["rows"].append(loc["row"])
            g["fns"].append(loc["fn"])
        else:
            g = dict(rows=[loc["row"]], path=loc["path"], line=loc["line"],
                     kind=loc["kind"], text=loc["text"], fns=[loc["fn"]])
            groups.append(g)
            by_key[key] = g

    m1 = json.load(open(args.m1_manifest))
    tracked = json.load(open(args.tracked_manifest))

    preamble_fns, entries, notes_entries, skipped = [], [], [], []

    for g in groups:
        rowlist = ", ".join(str(x) for x in g["rows"])
        fnlist = "; ".join(f.split("::")[-1] for f in g["fns"])
        span = f'{g["path"]}:{g["line"]}'
        anchor = ANCHOR_MAP.get((g["path"], g["line"]))

        if g["kind"] == "none":
            skipped.append(dict(rows=g["rows"], span=span, fns=g["fns"]))
            continue

        if anchor is not None:
            entry = next(e for e in m1["contracts"] if e["name"] == anchor)
            notes_entries.append(dict(rows=g["rows"], span=span, fns=g["fns"],
                                      kind="anchor", name=anchor, entry=entry,
                                      clauses=[],
                                      expected=M1_ARCHIVED[anchor],
                                      rationale="M1 全量运行（FULL-RUN.md）"
                                                "已归档的机器判定，作为锚点"
                                                "一致性对照"))
            continue

        # ----- auto extraction -----
        if g["kind"] == "attr":
            parsed = parse_verus_spec_attr(g["text"])
            fn_params = parse_fn_params(
                balanced_signature(get_lines(g["path"]), g["line"] - 1))
        else:
            parsed = parse_signature_contract(g["text"])
            fn_params = parse_fn_params(g["text"])
        requires = parsed["requires"]

        # parameter model
        scalar_names, ghost_param_ids, var_types = OrderedDict(), {}, {}
        for name, ty in fn_params:
            if name is None:
                continue
            b = base_type(ty)
            if b in SCALAR_TYPES:
                if name != "self":
                    scalar_names[name] = ty.strip() if ty.strip() in \
                        SCALAR_TYPES else b
                    var_types[name] = "bool" if b == "bool" else "usize"
            else:
                ghost_param_ids[name] = f"{name}_id"
        for wp in parsed["with"]:
            pm = re.match(r"(?:Tracked\(\s*)?([A-Za-z_]\w*)\s*\)?\s*:", wp)
            if pm:
                ghost_param_ids.setdefault(pm.group(1),
                                           f"{pm.group(1)}_id")

        referenced = set()
        for c in requires:
            for i in re.findall(r"[A-Za-z_]\w*", c):
                if i in scalar_names or i in ghost_param_ids:
                    referenced.add(i)

        consts = OrderedDict()
        for c in requires:
            for i in re.findall(r"\b[A-Z][A-Z0-9_]{2,}\b", c):
                consts.setdefault(i, i.lower())
        # normalize lifted const names to lowercase everywhere (params, call
        # args, spec-fn bodies) so the probe crate binds exactly one name
        requires_raw = list(requires)
        for k, v in consts.items():
            requires = [re.sub(r"\b%s\b" % re.escape(k), v, c)
                        for c in requires]
        var_types.update({v: "usize" for v in consts.values()})

        # clause classification and named-fn generation
        entry_tag = f"c{g['rows'][0]}"
        clauses = []
        req_terms = []
        for k, c in enumerate(requires):
            kind2, ast, cargs = classify_clause(c, set(scalar_names),
                                                set(consts.values()))
            cl = dict(text=requires_raw[k], kind=kind2)
            if kind2 == "mechanical":
                arglist = ", ".join(f"{a}: {var_types.get(a, 'usize')}"
                                    for a in cargs)
                ops = [t for t in tokenize_mech(c)
                       if t in ("<", "<=", ">", ">=", "==", "!=")]
                opname = {"<": "lt", "<=": "le", ">": "gt", ">=": "ge",
                          "==": "eq", "!=": "ne"}.get(ops[0] if ops else "",
                                                      "pred")
                fname = f"{entry_tag}_k{k}_{opname}_" + \
                    "_".join(a.lower() for a in cargs)
                preamble_fns.append(
                    f"pub open spec fn {fname}({arglist}) -> bool {{\n"
                    f"    {ast.render()}\n}}")
                req_terms.append(f"{fname}({', '.join(cargs)})")
                cl["fn"] = fname
                cl["args"] = cargs
                cl["body"] = ast.render()
                cl["ast"] = ast
            else:
                idents = cargs
                gargs, seen = [], set()
                for i in idents:
                    tgt, ty = None, "usize"
                    if i in ghost_param_ids and i in referenced:
                        tgt, ty = ghost_param_ids[i], "usize"
                    elif i in consts:
                        tgt = i.lower()
                    elif i in scalar_names and i in referenced:
                        tgt, ty = i, var_types.get(i, "usize")
                    if tgt and tgt not in seen:
                        seen.add(tgt)
                        gargs.append((tgt, ty))
                fname = f"{entry_tag}_k{k}_" + uninterp_slug(idents)
                if gargs:
                    arglist = ", ".join(f"{a}: {t}" for a, t in gargs)
                    call = f"{fname}({', '.join(a for a, _ in gargs)})"
                else:
                    arglist = "_unused: usize"
                    call = f"{fname}(0usize)"
                preamble_fns.append(
                    f"pub uninterp spec fn {fname}({arglist}) -> bool;")
                req_terms.append(call)
                cl["fn"] = fname
                cl["args"] = [a for a, _ in gargs]
                cl["body"] = None
            clauses.append(cl)

        param_order = [f"{n}_id" if n in ghost_param_ids else n
                       for n, ty in fn_params
                       if ((n in scalar_names or n in ghost_param_ids)
                           and n in referenced)]
        param_order.extend(v for v in consts.values() if v not in param_order)
        params_str = ", ".join(f"{p}: {var_types.get(p, 'usize')}"
                               for p in param_order) if param_order \
            else "self_id: usize"

        # witness: mechanical-layer satisfiability record (machine verdict
        # still comes from the probe run; see EXTRACTION.md witness policy).
        # The runner binds the witness tuple positionally over ALL params,
        # so values must cover param_order.
        witness = None
        if not requires:
            witness = "0usize"
        else:
            mech_cons = [(cl["args"], cl["ast"]) for cl in clauses
                         if cl["kind"] == "mechanical"]
            if mech_cons:
                wvars = list(dict.fromkeys(
                    v for cl in clauses if cl["kind"] == "mechanical"
                    for v in cl["args"]))
                wtypes = {v: var_types.get(v, "usize") for v in wvars}
                vals = search_witness(mech_cons, wtypes)
                if vals is not None:
                    wvals = dict(zip(wvars, vals))
                    full = [wvals.get(p, False if var_types.get(p) == "bool"
                                      else 0)
                            for p in param_order]
                    witness = render_witness(full, param_order, var_types)

        name = f"row{g['rows'][0]:02d}_" + short_fn_name(g["fns"][0])
        entry = {
            "name": name,
            "probe": "quantifier",
            "params": params_str,
            "requires": " && ".join(req_terms) if req_terms else "true",
            "covers_csv_rows": g["rows"],
            "source_span": span,
            "abstraction_notes": build_abstraction_notes(parsed, clauses,
                                                         consts, fn_params,
                                                         ghost_param_ids,
                                                         referenced),
        }
        if witness is not None:
            entry["witness"] = witness
        entries.append(entry)
        expected, rationale = expected_verdict_rationale(entry, clauses)
        notes_entries.append(dict(rows=g["rows"], span=span, fns=g["fns"],
                                  kind="auto", name=name, entry=entry,
                                  clauses=clauses, expected=expected,
                                  rationale=rationale))

    # ----- assemble manifest -----
    contracts = [ne["entry"] for ne in notes_entries
                 if ne["kind"] == "anchor"]
    contracts.extend(entries)
    contracts.extend(e for e in m1["contracts"] if e["name"] in M1_CONTROLS)
    contracts.extend(tracked["contracts"])

    manifest = {
        "target_workspace": tracked["target_workspace"],
        "preamble": m1["preamble"] + "\n\n// ===== full-run auto-extracted "
                    "clause fns (rows without M1 anchors; generated by "
                    "extract_full_manifest.py; clause mapping in "
                    "EXTRACTION-NOTES.md) =====\n\n" +
                    "\n\n".join(preamble_fns) + "\n",
        "contracts": contracts,
    }

    with open(args.out_manifest, "w") as f:
        json.dump(manifest, f, indent=2, ensure_ascii=False)
        f.write("\n")

    write_notes(args, rows, notes_entries, skipped, manifest)
    print_summary(notes_entries, skipped, contracts)


M1_ARCHIVED = {
    "try_traverse_and_lock_subtree_root_shape": "CandidateInconclusive",
    "alloc_if_none_shape": "CandidateInconclusive",
    "split_if_mapped_huge_shape": "CandidateInconclusive",
    "alloc_absent_child_shape": "CandidateInconclusive",
    "node_lock_shape": "CandidateInconclusive",
    "make_guard_unchecked_shape": "CandidateInconclusive",
    "mutex_guard_new_shape": "ConfirmedLive",
}


def build_abstraction_notes(parsed, clauses, consts, fn_params,
                            ghost_param_ids, referenced):
    notes = []
    if parsed.get("with"):
        names = re.findall(r"[A-Za-z_]\w*", " ".join(parsed["with"]))
        notes.append("with-clause tracked params abstracted to usize ids: "
                     + ", ".join(f"{n} -> {ghost_param_ids[n]}" for n in names
                                 if n in ghost_param_ids))
    for n, ty in fn_params:
        if n in ghost_param_ids and n != "self":
            notes.append(f"param {n}: {ty.strip()} abstracted to "
                         f"{ghost_param_ids[n]} (non-scalar stand-in)")
    if consts:
        notes.append("config consts lifted to parameters: "
                     + ", ".join(f"{k} -> {v}" for k, v in consts.items()))
    for k, cl in enumerate(clauses):
        if cl["kind"] == "mechanical":
            notes.append(f"requires clause {k} mechanical -> {cl['fn']} := "
                         f"{cl['body']}")
        else:
            a = ", ".join(cl["args"]) or "_unused"
            notes.append(f"requires clause {k} untranscribable over plain "
                         f"params -> uninterp {cl['fn']}({a}); body abstracted")
    if not parsed["requires"]:
        notes.append("contract has no requires clauses (empty conjunction); "
                     "probe requires encoded as literal true — entry "
                     "precondition trivially satisfiable by source text")
    return notes


def expected_verdict_rationale(entry, clauses):
    mech = [c for c in clauses if c["kind"] == "mechanical"]
    ghost = [c for c in clauses if c["kind"] == "ghost"]
    if not clauses:
        return "ConfirmedLive", "E1: requires 为空合取，exists 见证平凡成立；" \
                                 "forall 否定不可证"
    if mech and not ghost:
        if entry.get("witness"):
            return "ConfirmedLive", "E4: 全机械且见证可满足：空洞探针被见证驳倒，" \
                                     "活性探针带见证 verified"
        return "CandidateInconclusive", "E5: 全机械但搜索无见证：双探针皆败" \
                                         "（验证器不完备）"
    if mech and ghost:
        return "CandidateInconclusive", "E3: 机械+uninterp 混合（M1 根因）： "\
                                         "witness assert 无法穿过 uninterp " \
                                         "合取项，空洞探针亦不可证 → 双败"
    return "CandidateInconclusive", "E2: requires 全为 uninterp 谓词：活性无" \
                                     "法见证、空洞无法证否定 → 双败"


def write_notes(args, rows, notes_entries, skipped, manifest):
    L = []
    L.append("# EXTRACTION-NOTES.md — 60 接口全量提取对照表（预注册，写于运行前）\n")
    L.append(f"生成：`extract_full_manifest.py`（本目录）；输入 `{args.csv}`"
             "（60 行）+ 冻结源 `" + args.src + "`（rev 43da0797bbfa）。"
             "**本文件在运行 analyses-vacuity 之前生成**，含每条目的预期判定"
             "（预注册）；运行结果只归档于 FULL-RUN-60.md，不改写本文件。\n")
    L.append("## 建模边界（沿用 M1/EXTRACTION.md 已验证纪律）\n")
    L.append("- tracked 参数（`Tracked<...>`）与非标量参数抽象为不透明 usize "
             "stand-in（`<name>_id`）；")
    L.append("- requires 中对标量/常数的比较与算术机械转写为具名 `open spec "
             "fn`（触发推断；verus spec 算术为数学语义）；")
    L.append("- ghost 结构子句（字段/方法访问、`old`/`final`、`is`、量词等）"
             "机械不可表达 → 具名 `uninterp spec fn`（保名字与参数形状，"
             "丢定义体），逐条记录抽象损失；被引用的标量参数/提升常数作为 "
             "uninterp fn 参数保留（M1 先例：`in_count_window(parent_level,"
             " idx)`）；")
    L.append("- 无 requires 子句的契约（ensures-only / 裸 `#[verus_spec]`）："
             "探针 requires 编码为字面 `true`——入口前置按源文本平凡可满足；")
    L.append("- 见证策略：含机械子句的条目做 0..=7/bool 回溯搜索求见证"
             "（标量层可满足性的纸面论证；M1 修正后的策略：机器判定以探针"
             "运行为准）；")
    L.append("- 锚点：7 项已知恒假 stub（14 行 CSV，stub 与 "
             "_VERUS_VERIFIED 孪生共享 span）复用 M1 `manifest.json` 条目"
             "原文（锚点一致性对照）；构造探针与 quantifier 形状对照复用 "
             "`tracked/manifest.json` 条目原文；M1 三对照（2 阴性 1 阳性）"
             "原样保留；")
    L.append("- manifest 中 `covers_csv_rows`/`source_span`/"
             "`abstraction_notes` 为溯源元数据（runner serde 忽略未知字段）"
             "。\n")
    L.append("## 预注册判定规则（E 规则）\n")
    L.append("| 规则 | 条目形状 | 预期判定 |")
    L.append("|---|---|---|")
    L.append("| E1 | requires 为空合取 | ConfirmedLive |")
    L.append("| E2 | requires 全为 uninterp | CandidateInconclusive |")
    L.append("| E3 | 机械 + uninterp 混合 | CandidateInconclusive（M1 根因："
             "witness assert 不可证） |")
    L.append("| E4 | 全机械且见证可满足 | ConfirmedLive |")
    L.append("| E5 | 全机械但搜索无见证 | CandidateInconclusive |")
    L.append("| E6 | 机械子句联立矛盾（恒假在标量层） | ConfirmedVacuous"
             "（预期不出现；出现即新证据，须复核） |\n")
    L.append("意外结局预案：任一条目 Anomaly → 分类器/触发推断缺陷排查"
             "（RESULT.md/spike 先例）；E1 条目非 Live → 探针生成器退化；"
             "阳性对照不符 → 全部结果作废。\n")

    n_anchor = sum(1 for ne in notes_entries if ne["kind"] == "anchor")
    n_auto = sum(1 for ne in notes_entries if ne["kind"] == "auto")
    n_rows_anchor = sum(len(ne["rows"]) for ne in notes_entries
                        if ne["kind"] == "anchor")
    n_rows_auto = sum(len(ne["rows"]) for ne in notes_entries
                      if ne["kind"] == "auto")
    n_rows_skip = sum(len(s["rows"]) for s in skipped)
    L.append("## 提取覆盖统计\n")
    L.append(f"- 清单 60 行 = 锚点复用 {n_rows_anchor} 行（{n_anchor} 个去重"
             f"契约）+ 自动提取 {n_rows_auto} 行（{n_auto} 个契约）+ "
             f"extraction_skipped {n_rows_skip} 行；")
    L.append(f"- manifest 共 {len(manifest['contracts'])} 条目 = {n_anchor} "
             f"锚点 + {n_auto} 自动 + {len(M1_CONTROLS)} M1 对照 + 3 构造/"
             "形状对照（tracked 复用）。\n")

    L.append("## 逐契约转写对照表\n")
    for ne in notes_entries:
        rc = ", ".join(str(x) for x in ne["rows"])
        L.append(f"### {ne['name']}（CSV 行 {rc}；{ne['span']}）")
        L.append(f"接口：{'；'.join(ne['fns'])}")
        L.append(f"预期判定：**{ne['expected']}** — {ne['rationale']}")
        if ne["kind"] == "anchor":
            L.append("转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照"
                     "见 M1 EXTRACTION.md 对应章节），不重复转写。\n")
            continue
        e = ne["entry"]
        L.append("```json")
        L.append(json.dumps({k: e[k] for k in e if k in ("params", "requires",
                                                         "witness")},
                            indent=1, ensure_ascii=False))
        L.append("```")
        if ne["clauses"]:
            L.append("| # | 原子句（原文） | 转写 | 类型 |")
            L.append("|---|---|---|---|")
            for k, cl in enumerate(ne["clauses"]):
                orig = cl["text"].replace("\n", " ")
                if len(orig) > 110:
                    orig = orig[:107] + "..."
                if cl["kind"] == "mechanical":
                    tr = f"`{cl['fn']}` := `{cl['body']}`"
                    ty = "机械"
                else:
                    tr = (f"`{cl['fn']}({', '.join(cl['args']) or '_unused'})`"
                          " uninterp")
                    ty = "uninterp"
                L.append(f"| {k} | `{orig}` | {tr} | {ty} |")
        else:
            L.append("（requires 子句：无）")
        for n in e["abstraction_notes"]:
            L.append(f"- {n}")
        L.append("")

    L.append("## extraction_skipped（无契约可提取，如实记录）\n")
    L.append("原因（通用）：冻结源对应 span 处无 `#[verus_spec]` 属性、签名内"
             "亦无 requires 子句——接口无显式契约条款，入口前置为空合取，"
             "不存在可探针的恒假候选；按纪律不硬造条目。\n")
    L.append("| CSV 行 | 接口 | span |")
    L.append("|---|---|---|")
    for s in skipped:
        for r in s["rows"]:
            fn = rows[r]["function"]
            if len(fn) > 64:
                fn = fn[:61] + "..."
            L.append(f"| {r} | `{fn}` | {s['span']} |")
    L.append("")

    dist = {}
    for ne in notes_entries:
        dist[ne["expected"]] = dist.get(ne["expected"], 0) + 1
    for e in manifest["contracts"]:
        if e["name"] in M1_CONTROLS:
            dist[M1_CONTROLS[e["name"]]] = \
                dist.get(M1_CONTROLS[e["name"]], 0) + 1
        elif e.get("probe") == "construction" or \
                e["name"] in TRACKED_EXPECTED:
            dist[TRACKED_EXPECTED[e["name"]]] = \
                dist.get(TRACKED_EXPECTED[e["name"]], 0) + 1
    total = sum(dist.values())
    L.append(f"## 汇总：预期判定分布（预注册，{total} 条目）\n")
    L.append(", ".join(f"{k}={v}" for k, v in sorted(dist.items())) +
             "（运行后以 FULL-RUN-60.md 实测为准）\n")
    with open(args.out_notes, "w") as f:
        f.write("\n".join(L) + "\n")


def print_summary(notes_entries, skipped, contracts):
    dist = {}
    for ne in notes_entries:
        dist[ne["expected"]] = dist.get(ne["expected"], 0) + 1
    print(f"manifest entries: {len(contracts)}")
    print(f"auto/anchor groups: {len(notes_entries)}, skipped rows: "
          f"{sum(len(s['rows']) for s in skipped)}")
    print("expected (pre-registered, auto entries): "
          + ", ".join(f"{k}={v}" for k, v in sorted(dist.items())))


if __name__ == "__main__":
    main()
