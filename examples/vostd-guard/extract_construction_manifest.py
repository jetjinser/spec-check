#!/usr/bin/env python3
"""Extract construction-probe manifests for the 10 CandidateInconclusive
entries of guard-return-inventory.csv (60-interface quantifier run).

Mechanically transcribes the frozen-source `#[verus_spec]` contracts into
manifest v1.1 `probe: "construction"` entries over REAL workspace types, and
emits two manifests plus the pre-registered expectations:

- construction-manifest.json — the verifying run:
  * holder faces for all 10 contracts (face_requires=true; args are manifest
    params; direct calls to the real contract face where it is visible from
    the injection point, contract-face stubs where it is not);
  * fabrication faces for the two contracts whose tracked args have a
    complete no_std minting chain (vm_space_cursor_embedded/_mut);
  * the archived mutex_guard_new fab+holder anchors (tracked/manifest.json)
    reused verbatim as control anchors.
- construction-fab-blocked.json — leaf evidence run: fabrication faces for
  the 8 contracts whose tracked-arg minting chains are structurally blocked
  at the no_std target. This manifest is EXPECTED to fail rustc name
  resolution; the per-module compile errors at the minting call sites are
  the machine evidence of fabrication-channel absence (not a verdict run).
- CONSTRUCTION-EXPECTED.md — pre-registered predictions (written BEFORE any
  run of these manifests; never rewritten afterwards).

Minting strategy table (no_std target `x86_64-unknown-none`, root-level
injection at ostd/src/spec_check_probes):

  mintable   vstd pcell::PointsTo / pcell_maybe_uninit::PointsTo
                 via PCell::new (const fn, no_std OK)
             vstd_extra Count<T,N> via Count::alloc (proof fn; T mintable)
             Guards via pub-field struct literal (Set<usize>)
             OwnerSubtree via TreeNode::tracked_new_val over a mintable
                 EntryOwner (Absent kind; pub fields; TreePath::new)
             MetaRegionOwners via pub-field struct literal (empty maps;
                 constructible but not inv-valid)
             raw_ptr::PointsTo via PointsToRaw::empty axiom chain
  blocked    vstd simple_pptr::PointsTo — minting APIs (PPtr::new/empty) are
                 #[cfg(feature = "std")]-gated; struct fields private
             vstd_extra array_ptr::PointsTo — ArrayPtr::new std-gated;
                 fields private
             FrameRef / PageTableGuard values — pub types but their tracked
                 fields need the blocked permissions; the mm-internal
                 FrameRef::borrow_paddr constructor is invisible at the
                 root injection point (visibility wall)
             MetadataPerm/FracMetadataPerm/MetaSlotOwner/NodeOwner/
                 CursorOwner/CursorContinuation — transitively blocked
  (Tracked::assume_new is compile-blocked in verus — archived separately in
  examples/tracked-modeling-spike/s1-assume-new.)

Probe architecture per contract: `direct` = call the real (pub) contract
face from the injected module; `stub` = minimal external_body transcription
of the contract face over real types (visibility wall). Renames applied
mechanically: receiver `self` -> named param; `C` -> UserPtConfig (the
concrete pub PageTableConfig used by the specs layer); `A` ->
DisabledPreemptGuard (a pub InAtomicMode impl); `Self::` -> concrete type.

All tool-side mechanisms are manifest v1.1 (crates/spec-check-analyses/
vacuity.rs) — this script, the tables and the emitted verus source are
project-side inputs only.
"""

import argparse
import json
import os
import re

from extract_full_manifest import (balanced_signature,
                                   collect_verus_spec_attrs,
                                   gap_is_attrs_or_comments,
                                   parse_signature_contract)

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SRC = "/tmp/vostd-43da079-ktest-chan"
DEFAULT_TRACKED = os.path.join(HERE, "tracked", "manifest.json")
DEFAULT_OUT_DIR = os.path.join(HERE, "construction")

TARGET_WORKSPACE = {
    "manifest_path": "/tmp/vostd-43da079-ktest-chan/Cargo.toml",
    "inject_dir": "ostd/src/spec_check_probes",
    "gate_cfg": "ktest",
    "extra_cfgs": ["ktest_capture_gap"],
    "target": "x86_64-unknown-none",
}

# ---------------------------------------------------------------------------
# Mechanical-transcription vocabulary (concrete instantiation of generics)

CFG = "UserPtConfig"                  # pub struct, crate::mm::vm_space
ATOMIC = "DisabledPreemptGuard"       # pub InAtomicMode impl, crate::task

IMPORTS = (
    "use core::ops::Range;\n"
    "use crate::arch::mm::{NR_ENTRIES, NR_LEVELS, PageTableEntry, PagingConsts};\n"
    "use crate::mm::{Paddr, PagingLevel, Vaddr, page_size};\n"
    "use crate::mm::frame::meta::{REF_COUNT_MAX, REF_COUNT_UNUSED};\n"
    "use crate::mm::page_table::PageTableConfig;\n"
    "use crate::mm::page_table::{\n"
    "    CursorMut, Entry, PageTable, PageTableGuard, PageTableNodeRef,\n"
    "};\n"
    "use crate::mm::vm_space::vm_space_specs::VmSpaceOwner;\n"
    "use crate::mm::vm_space::UserPtConfig;\n"
    "use crate::specs::mm::embedding::cursor::{\n"
    "    vm_space_cursor_embedded, vm_space_cursor_mut_embedded,\n"
    "};\n"
    "use crate::specs::mm::frame::mapping::{frame_to_index, meta_to_index};\n"
    "use crate::specs::mm::frame::meta_region_owners::MetaRegionOwners;\n"
    "use crate::specs::mm::frame::meta_owners::{\n"
    "    FracMetadataPerm, MetaSlotStorage, MetadataPerm,\n"
    "};\n"
    "use crate::specs::mm::page_table::cursor::CursorView;\n"
    "use crate::specs::mm::page_table::node::{\n"
    "    in_count_window, Guards, NodeOwner, PageMetaOwner,\n"
    "};\n"
    "use crate::specs::mm::page_table::{\n"
    "    CursorContinuation, CursorOwner, EntryOwner, EntryOwnerKind, INC_LEVELS,\n"
    "    OwnerSubtree, PageTableOwner,\n"
    "};\n"
    "use crate::task::DisabledPreemptGuard;\n"
    "use vstd::cell::pcell_maybe_uninit::PCell as PCellMU;\n"
    "use vstd::prelude::*;\n"
    "use vstd::simple_pptr::*;\n"
    "use vstd_extra::array_ptr::*;\n"
    "use vstd_extra::ghost_tree::TreePath;\n"
    "use vstd_extra::ownership::{Inv, OwnerOf};\n"
)

# ---------------------------------------------------------------------------
# Per-entry architecture table (project-side input)
#
# kind: direct (real contract face visible from the injection point)
#     | stub (visibility wall -> external_body transcription)
# fab: mint | blocked
# ---------------------------------------------------------------------------

NODE_LOCK_EFR = ("node.inner@.invariants(*owner),\n"
                 "        node.inner@.external_meta_wf("
                 "owner.frame_permission.resource(), ()),\n"
                 "        old(guards).unlocked(owner.slot_vaddr())")

CONTRACTS = [
    dict(
        key="node_lock",
        csv_rows=[11, 13],
        source="ostd/src/mm/page_table/node/mod.rs:373-383",
        fn_line=375,
        kind="direct",
        holder_construct=(
            "#[verus_spec(with Tracked(owner), Tracked(guards))]\n"
            "let _g = node.lock(guard);"
        ),
        holder_params=(
            f"node: PageTableNodeRef<{CFG}>, guard: &{ATOMIC}, "
            f"Tracked(owner): Tracked<&NodeOwner<{CFG}>>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        with_params=["Tracked(owner)", "Tracked(guards)"],
        efr=NODE_LOCK_EFR,
        fab="blocked",
        fab_attempt="node_owner",
        blocked_leaf="NodeOwner mint: array_ptr::PointsTo (ArrayPtr::new is "
                     "#[cfg(std)]) + simple_pptr::PointsTo (PPtr::new/empty "
                     "are #[cfg(std)]; fields private) have no no_std "
                     "constructor; FracMetadataPerm transitively blocked",
    ),
    dict(
        key="make_guard_unchecked",
        csv_rows=[12, 14],
        source="ostd/src/mm/page_table/node/mod.rs:389-407",
        fn_line=401,
        kind="direct",
        holder_construct=(
            "let _g = unsafe {\n"
            "        #[verus_spec(with Tracked(owner), Tracked(guards))]\n"
            "        node.make_guard_unchecked(guard)\n"
            "    };"
        ),
        holder_params=(
            f"node: PageTableNodeRef<{CFG}>, guard: &{ATOMIC}, "
            f"Tracked(owner): Tracked<&NodeOwner<{CFG}>>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        with_params=["Tracked(owner)", "Tracked(guards)"],
        efr=NODE_LOCK_EFR,
        fab="blocked",
        fab_attempt="node_owner",
        blocked_leaf="same as node_lock (identical tracked-arg set)",
    ),
    dict(
        key="try_traverse_and_lock_subtree_root",
        csv_rows=[2, 3],
        source="ostd/src/mm/page_table/cursor/locking.rs:287-336",
        fn_line=384,
        kind="stub",
        stub_params=(
            f"pt: &PageTable<{CFG}>, _guard: &'rcu {ATOMIC}, va: &Range<Vaddr>, "
            f"Tracked(cursor_own): Tracked<&mut CursorOwner<{CFG}>>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        stub_generics="<'a, 'rcu>",
        stub_ret=f"Option<PageTableGuard<'rcu, {CFG}>>",
        holder_params=(
            f"pt: &PageTable<{CFG}>, guard: &{ATOMIC}, va: &Range<Vaddr>, "
            f"Tracked(cursor_own): Tracked<&mut CursorOwner<{CFG}>>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        holder_call_args=[
            "pt", "guard", "va",
            "Tracked(cursor_own)", "Tracked(regions)", "Tracked(guards)",
        ],
        rename=[(r"CursorMut::<C, A>::", f"CursorMut::<{CFG}, {ATOMIC}>::"),
                (r"\bC::Item", f"<{CFG} as PageTableConfig>::Item")],
        fab="blocked",
        fab_attempt="guard_value",
        blocked_leaf="CursorOwner/ CursorContinuation mint: continuation "
                     "guard field is a PageTableGuard value (blocked tracked "
                     "fields; mm-internal borrow_paddr invisible at root "
                     "injection); regions/ guards otherwise literal-mintable",
    ),
    dict(
        key="alloc_if_none",
        csv_rows=[5, 7],
        source="ostd/src/mm/page_table/node/entry.rs:515-620",
        fn_line=616,
        kind="stub",
        method_form=dict(
            impl_generics="<'rcu>",
            recv=f"Entry<'_, '_, {CFG}>",
            fn_params=f"guard: &'rcu {ATOMIC}",
            ret=f"Option<PageTableGuard<'rcu, {CFG}>>",
        ),
        stub_generics="<'rcu>",
        stub_ret=f"Option<PageTableGuard<'rcu, {CFG}>>",
        holder_params=(
            f"entry: &mut Entry<{CFG}>, guard: &{ATOMIC}, "
            f"Tracked(owner): Tracked<&mut OwnerSubtree<{CFG}>>, "
            f"Tracked(parent_owner): Tracked<&mut NodeOwner<{CFG}>>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        holder_call=(
            "#[verus_spec(with Tracked(owner), Tracked(parent_owner), "
            "Tracked(regions), Tracked(guards))]\n"
            "let _r = entry.stub_alloc_if_none(guard);"
        ),
        rename=[
            (r"\bold\(self\)", "old(entry)"),
            (r"\bfinal\(self\)", "final(entry)"),
            (r"\bC\b", CFG),
        ],
        fab="blocked",
        fab_attempt="node_owner",
        blocked_leaf="parent_owner: NodeOwner mint blocked (array_ptr + "
                     "simple_pptr leaves); owner subtree / regions / guards "
                     "otherwise mintable",
    ),
    dict(
        key="split_if_mapped_huge",
        csv_rows=[6, 8],
        source="ostd/src/mm/page_table/node/entry.rs:851-966",
        fn_line=968,
        kind="stub",
        method_form=dict(
            impl_generics="<'rcu>",
            recv=f"Entry<'_, '_, {CFG}>",
            fn_params=f"guard: &'rcu {ATOMIC}",
            ret=f"Option<PageTableGuard<'rcu, {CFG}>>",
        ),
        stub_generics="<'rcu>",
        stub_ret=f"Option<PageTableGuard<'rcu, {CFG}>>",
        holder_params=(
            f"entry: &mut Entry<{CFG}>, guard: &{ATOMIC}, "
            f"Tracked(owner): Tracked<&mut OwnerSubtree<{CFG}>>, "
            f"Tracked(parent_owner): Tracked<&mut NodeOwner<{CFG}>>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        holder_call=(
            "#[verus_spec(with Tracked(owner), Tracked(parent_owner), "
            "Tracked(regions), Tracked(guards))]\n"
            "let _r = entry.stub_split_if_mapped_huge(guard);"
        ),
        rename=[
            (r"\bold\(self\)", "old(entry)"),
            (r"\bfinal\(self\)", "final(entry)"),
            (r"\bC\b", CFG),
        ],
        fab="blocked",
        fab_attempt="node_owner",
        blocked_leaf="same as alloc_if_none (NodeOwner mint blocked)",
    ),
    dict(
        key="alloc_absent_child",
        csv_rows=[9, 10],
        source="ostd/src/mm/page_table/node/entry.rs:2029-2160",
        fn_line=2141,
        kind="stub",
        method_form=dict(
            impl_generics="<'rcu>",
            recv=f"Entry<'_, '_, {CFG}>",
            fn_params=f"idx: usize, guard: &'rcu {ATOMIC}",
            ret=f"PageTableGuard<'rcu, {CFG}>",
        ),
        stub_generics="<'rcu>",
        stub_ret=f"PageTableGuard<'rcu, {CFG}>",
        holder_params=(
            f"entry: &mut Entry<{CFG}>, idx: usize, guard: &{ATOMIC}, "
            f"Tracked(owner): Tracked<&mut OwnerSubtree<{CFG}>>, "
            f"Tracked(parent_owner): Tracked<&mut NodeOwner<{CFG}>>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>, "
            "Tracked(guards): Tracked<&mut Guards>"
        ),
        holder_call=(
            "#[verus_spec(with Tracked(owner), Tracked(parent_owner), "
            "Tracked(regions), Tracked(guards))]\n"
            "let _r = entry.stub_alloc_absent_child(idx, guard);"
        ),
        rename=[
            (r"\bold\(self\)", "old(entry)"),
            (r"\bfinal\(self\)", "final(entry)"),
            (r"\bC\b", CFG),
            (r"relate_guard\(\*old\(entry\)\)",
             "relate_guard(*old(entry).node)"),
            (r"relate_guard\(\*final\(entry\)\)",
             "relate_guard(*final(entry).node)"),
        ],
        fab="blocked",
        fab_attempt="node_owner",
        blocked_leaf="same as alloc_if_none (NodeOwner mint blocked)",
    ),
    dict(
        key="path_slot_as_mut",
        csv_rows=[4],
        source="ostd/src/mm/page_table/cursor/mod.rs:133-141",
        fn_line=134,
        kind="stub",
        stub_params=(
            f"path: &'a mut [Option<PageTableGuard<'rcu, {CFG}>>; NR_LEVELS], "
            "idx: usize"
        ),
        stub_generics="<'a, 'rcu>",
        stub_ret=f"&'a mut PageTableGuard<'rcu, {CFG}>",
        holder_params=(
            f"path: &mut [Option<PageTableGuard<{CFG}>>; NR_LEVELS], idx: usize"
        ),
        holder_call_args=["path", "idx"],
        rename=[],
        fab="blocked",
        fab_attempt="guard_value",
        blocked_leaf="satisfying path value mint: PageTableGuard value "
                     "blocked (FrameRef tracked fields; borrow_paddr is "
                     "pub(in crate::mm) — visibility wall at root injection)",
    ),
    dict(
        key="vm_space_cursor_embedded",
        csv_rows=[15],
        sig_contract=True,
        source="ostd/specs/mm/embedding/cursor.rs:90-135",
        fn_line=90,
        kind="direct",
        real_path="vm_space_cursor_embedded",
        holder_construct=(
            "proof {\n"
            "    let _r = vm_space_cursor_embedded(vm_space, regions, va);\n"
            "}"
        ),
        holder_params=(
            "va: Range<Vaddr>, "
            "Tracked(vm_space): Tracked<&VmSpaceOwner>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>"
        ),
        holder_call_args=["vm_space", "regions", "va"],
        efr="vm_space.inv(),\n        old(regions).inv()",
        fab="mint",
        fab_mint="vm_space_regions",
    ),
    dict(
        key="vm_space_cursor_mut_embedded",
        csv_rows=[16],
        sig_contract=True,
        source="ostd/specs/mm/embedding/cursor.rs:131-176",
        fn_line=131,
        kind="direct",
        real_path="vm_space_cursor_mut_embedded",
        holder_construct=(
            "proof {\n"
            "    let _r = vm_space_cursor_mut_embedded(vm_space, regions, va);\n"
            "}"
        ),
        holder_params=(
            "va: Range<Vaddr>, "
            "Tracked(vm_space): Tracked<&VmSpaceOwner>, "
            "Tracked(regions): Tracked<&mut MetaRegionOwners>"
        ),
        holder_call_args=["vm_space", "regions", "va"],
        efr="vm_space.inv(),\n        old(regions).inv()",
        fab="mint",
        fab_mint="vm_space_regions",
    ),
    dict(
        key="tracked_restore",
        csv_rows=[17],
        sig_contract=True,
        rename=[(r"\bold\(self\)", "old(cont)")],
        source="ostd/specs/mm/page_table/cursor/owners.rs:192-200",
        fn_line=193,
        kind="direct",
        holder_construct=(
            "proof {\n"
            "    let tracked mut c = cont;\n"
            "    let _g = c.tracked_restore(child);\n"
            "}"
        ),
        holder_params=(
            "Tracked(cont): Tracked<CursorContinuation<UserPtConfig>>, "
            "Tracked(child): Tracked<CursorContinuation<UserPtConfig>>"
        ),
        holder_call_args=["cont", "child"],
        efr="cont.idx < cont.children.len()",
        fab="blocked",
        fab_attempt="guard_value",
        blocked_leaf="CursorContinuation mint blocked (guard field is a "
                     "PageTableGuard value)",
    ),
]

# ---------------------------------------------------------------------------
# Contract extraction (frozen source verbatim)
# ---------------------------------------------------------------------------

def get_lines(src, path):
    return open(os.path.join(src, path)).read().split("\n")


def locate_attr(lines, fn_line):
    """The #[verus_spec ...] block directly above the fn at fn_line."""
    for s, e, text in collect_verus_spec_attrs(lines):
        if e < fn_line and gap_is_attrs_or_comments(lines, e + 1, fn_line):
            return text
    raise SystemExit(f"no verus_spec attr above line {fn_line}")


def strip_comments(text):
    return "\n".join(re.sub(r"//.*$", "", l) for l in text.split("\n"))


def parse_attr_clauses(attr_text):
    """parse_verus_spec_attr with comments stripped and angle-aware clause
    splitting (generic commas like CursorMut::<C, A> must not split)."""
    t = strip_comments(attr_text.strip())
    inner = t[len("#[verus_spec"):].strip()
    inner = inner[1:inner.rfind(")")]
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
        out[k] = split_clauses(body) if body else []
    return out


def split_clauses(body):
    """Split a contract section into top-level comma-separated clauses.

    Only ()/[]/{} brackets and REAL generic angles delimit; comparison `<`/`>`
    (surrounded by spaces) and masked arrow operators do not affect depth, so
    `a < b`, `x >= y` and `Foo::<C, A>::method(..)` all survive intact."""
    masked = body.replace("==>", "\x01").replace("->", "\x02")
    parts, cur = [], []
    depth = angle = 0
    prev = ""
    for ch in masked:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "<":
            if prev and (prev.isalnum() or prev in "_>:"):
                angle += 1
        elif ch == ">":
            if angle > 0 and prev != " ":
                angle -= 1
        if ch == "," and depth == 0 and angle == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
        prev = ch
    parts.append("".join(cur))
    return [p.strip().replace("\x01", "==>").replace("\x02", "->")
            for p in parts if p.strip()]


def apply_rename(clause, rename):
    for pat, rep in rename:
        clause = re.sub(pat, rep, clause)
    return clause


def fmt_clauses(clauses, rename=()):
    body = [apply_rename(c, rename) for c in clauses]
    return ",\n        ".join(body)


# ---------------------------------------------------------------------------
# Stub / preamble emission
# ---------------------------------------------------------------------------

def emit_stub(c, requires, ensures):
    stub_name = f"stub_{c['key']}"
    bind = [(r"\br\b", "res")]
    requires = apply_rename(requires, bind)
    ensures = apply_rename(ensures, bind)
    L = []
    L.append("#[verifier::external_body]")
    L.append(f"pub fn {stub_name}{c['stub_generics']}(")
    L.append(f"    {c['stub_params']},")
    L.append(f") -> (res: {c['stub_ret']})")
    L.append("    requires")
    L.append(f"        {requires},")
    L.append("    ensures")
    L.append(f"        {ensures},")
    L.append("{")
    L.append("    loop {}")
    L.append("}")
    return "\n".join(L), stub_name


def preamble(c, stub_text=None, probe_comment=None):
    parts = [IMPORTS.rstrip("\n")]
    if probe_comment:
        parts.append(probe_comment)
    if stub_text:
        parts.append(stub_text)
    return "\n\n".join(parts) + "\n"


VM_SPACE_MINT_PROOF = """proof {
    let tracked eo = EntryOwner {
        kind: EntryOwnerKind::Absent,
        path: TreePath::new(Seq::empty()),
        parent_level: (INC_LEVELS - 1) as PagingLevel,
    };
    let tracked sub = OwnerSubtree::<UserPtConfig>::tracked_new_val(eo, 0nat);
    let tracked vs_fab = VmSpaceOwner {
        page_table_owner: sub,
        active: false,
        readers: Seq::tracked_empty(),
        writers: Seq::tracked_empty(),
        mem_view: None,
        mv_range: None,
        shared_reader: false,
    };
    let tracked mut regions_fab = MetaRegionOwners {
        slots: Map::tracked_empty(),
        slot_owners: Map::tracked_empty(),
    };
    let _r = %(real_path)s(&vs_fab, &mut regions_fab, va);
}"""

# Fabrication attempt for NodeOwner-typed tracked args. Every minting step
# that exists in the no_std surface is exercised; the expected failure is at
# PPtr::new / ArrayPtr::new (std-gated) — the machine anchor for channel
# absence.
NODE_OWNER_MINT_ATTEMPT = """let (_cell_nc, Tracked(nc_perm)) = PCellMU::new(0u16);
let (_cell_st, Tracked(st_perm)) = PCellMU::new(false);
let (_cell_ss, Tracked(ss_perm)) = PCellMU::new(MetaSlotStorage::Empty([0u8; 39]));
let (_pptr, Tracked(vt_perm)) = PPtr::new(0usize);
let (_arr, Tracked(cp_perm)) = ArrayPtr::new(PageTableEntry::default());
proof {
    let tracked mp = MetadataPerm {
        storage_perm: ss_perm,
        vtable_ptr_perm: vt_perm,
    };
    let tracked frac = FracMetadataPerm::alloc(mp);
    let tracked owner_fab = NodeOwner {
        meta_own: PageMetaOwner { nr_children: nc_perm, stray: st_perm },
        frame_permission: frac,
        children_perm: cp_perm,
        tree_level: 1int,
        slot_index: 0int,
    };
}"""

# Fabrication attempt for PageTableGuard values (CursorContinuation.guard,
# path arrays). Expected failure: borrow_paddr visibility (E0603 family) —
# the machine anchor for the handle-minting wall.
GUARD_VALUE_MINT_ATTEMPT = """let g = unsafe {
    PageTableGuard {
        inner: PageTableNodeRef::borrow_paddr(0usize),
    }
};"""

# ---------------------------------------------------------------------------
# Manifest assembly
# ---------------------------------------------------------------------------

def holder_entry(c, stub_name=None):
    if c["kind"] == "stub" and "holder_call" in c:
        call = c["holder_call"]
        efr = c["_efr"]
    elif c["kind"] == "stub":
        call = f"let _r = {stub_name}({', '.join(c['holder_call_args'])});"
        efr = c["_efr"]
    else:
        call = c["holder_construct"]
        efr = c["efr"]
    return {
        "name": f"{c['key']}_holder_face",
        "probe": "construction",
        "face_requires": True,
        "params": c["holder_params"],
        "expect_fail_requires": efr,
        "model_preamble": c["_preamble"],
        "construct": call,
    }


def fab_entry(c):
    if c["fab"] == "mint":
        return {
            "name": f"{c['key']}_construction",
            "probe": "construction",
            "params": "va: Range<Vaddr>",
            "expect_fail_requires": c["efr"],
            "model_preamble": c["_preamble"],
            "construct": VM_SPACE_MINT_PROOF % {"real_path": c["real_path"]},
        }
    construct = (NODE_OWNER_MINT_ATTEMPT
                 if c["fab_attempt"] == "node_owner"
                 else GUARD_VALUE_MINT_ATTEMPT)
    return {
        "name": f"{c['key']}_construction",
        "probe": "construction",
        "params": "_unused: usize",
        "expect_fail_requires": c["_efr"],
        "model_preamble": c["_preamble"],
        "construct": construct,
    }


def build(src):
    for c in CONTRACTS:
        lines = get_lines(src, c["source"].split(":")[0])
        if c.get("sig_contract"):
            # axiom / proof fns carry their contract in the signature
            text = balanced_signature(lines, c["fn_line"] - 1)
            parsed = parse_signature_contract(text)
        else:
            attr = locate_attr(lines, c["fn_line"] - 1)
            parsed = parse_attr_clauses(attr)
        c["_efr"] = fmt_clauses(parsed["requires"], c.get("rename", []))
        if c["kind"] == "stub":
            requires = fmt_clauses(parsed["requires"], c.get("rename", []))
            ensures = fmt_clauses(parsed["ensures"], c.get("rename", []))
            mf = c.get("method_form")
            if mf is not None:
                # method-form stubs keep the receiver `self` verbatim; the
                # generic config is instantiated with a blanket C rename
                extra = [(r"\bC\b", CFG),
                         (r"Self::", f"Entry::<{CFG}>::")]
                if c["key"] == "alloc_absent_child":
                    # bare `*old(self)`/`*final(self)` is accepted by the
                    # verus checker only inside verified-fn contracts
                    # (rustc never sees erased spec text); the rustc-visible
                    # stub uses the explicit `.node` deref, the same clause
                    # form split_if_mapped_huge's own contract uses for the
                    # identical predicate
                    extra += [
                        (r"relate_guard\(\*old\(self\)\)",
                         "relate_guard(*old(self).node)"),
                        (r"relate_guard\(\*final\(self\)\)",
                         "relate_guard(*final(self).node)"),
                    ]
                requires = fmt_clauses(parsed["requires"], extra)
                ensures = fmt_clauses(parsed["ensures"], extra)
                # receiver-preserving transcription: the stub is a method on
                # the real receiver type carrying the contract as a
                # verus_spec attr (same face shape as the frozen source)
                with_c = fmt_clauses(parsed["with"], [(r"\bC\b", CFG)])
                stub_name = f"stub_{c['key']}"
                stub_text = (
                    "#[verus_verify]\n"
                    f"impl{mf['impl_generics']} {mf['recv']} {{\n"
                    "    #[verifier::external_body]\n"
                    "    #[verus_spec(res =>\n"
                    f"        with {with_c},\n"
                    "        requires\n"
                    f"            {requires},\n"
                    "        ensures\n"
                    f"            {ensures},\n"
                    "    )]\n"
                    f"    pub fn {stub_name}(&mut self, {mf['fn_params']}) "
                    f"-> {mf['ret']} {{\n"
                    "        unimplemented!()\n"
                    "    }\n"
                    "}")
                c["_stub_name"] = stub_name
            else:
                stub_text, stub_name = emit_stub(c, requires, ensures)
                c["_stub_name"] = stub_name
            comment = (
                "// Contract-face stub: mechanical transcription of the\n"
                f"// contract under analysis — {c['source']} — over the\n"
                "// REAL workspace types at the concrete caller face\n"
                f"// C = {CFG}, A = {ATOMIC}. external_body + diverging\n"
                "// body: the contract face is the subject under probe.")
            c["_preamble"] = preamble(c, stub_text, comment)
        else:
            comment = (
                "// Direct probe: the real contract face is public and is\n"
                f"// called as-is — {c['source']}. No transcription gap.")
            c["_preamble"] = preamble(c, None, comment)
    return CONTRACTS


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default=DEFAULT_SRC)
    ap.add_argument("--tracked-manifest", default=DEFAULT_TRACKED)
    ap.add_argument("--out-dir", default=DEFAULT_OUT_DIR)
    args = ap.parse_args()

    contracts = build(args.src)
    out = args.out_dir
    os.makedirs(out, exist_ok=True)

    tracked = json.load(open(args.tracked_manifest))
    anchors = [e for e in tracked["contracts"]
               if e["name"] in ("mutex_guard_new_construction",
                                "mutex_guard_new_holder_face")]

    main_entries, blocked_entries = [], []
    for c in contracts:
        if c["fab"] == "mint":
            main_entries.append(fab_entry(c))
        else:
            blocked_entries.append(fab_entry(c))
        main_entries.append(holder_entry(c, c.get("_stub_name")))

    with open(os.path.join(out, "construction-manifest.json"), "w") as f:
        json.dump({"target_workspace": TARGET_WORKSPACE,
                   "contracts": main_entries + anchors}, f, indent=2)
        f.write("\n")
    with open(os.path.join(out, "construction-fab-blocked.json"), "w") as f:
        json.dump({"target_workspace": TARGET_WORKSPACE,
                   "contracts": blocked_entries}, f, indent=2)
        f.write("\n")

    write_expected(out, contracts, main_entries, blocked_entries, anchors)
    print(f"main entries: {len(main_entries)} (+{len(anchors)} anchors); "
          f"blocked fab faces: {len(blocked_entries)}")


def write_expected(out, contracts, main_entries, blocked_entries, anchors):
    L = ["# CONSTRUCTION 预注册（运行前写定，运行后不改写；2026-11-11）", ""]
    L.append("对象：60 接口 quantifier 全量运行中 10 个 CandidateInconclusive "
             "条目（6 个恒假 stub 锚点 + row04/15/16/17）。生成器："
             "`extract_construction_manifest.py`。预测只基于冻结源静态判读"
             "与 M2/wt-b 已归档判据；运行判定以 manifest 实测为准。")
    L.append("")
    L.append("## 探针架构与铸造策略表（静态判读，先于运行）")
    L.append("")
    L.append("| 契约 | 面类型 | 伪造通道 | 阻断叶（如有） |")
    L.append("|------|--------|----------|----------------|")
    for c in contracts:
        L.append(f"| {c['key']} | {c['kind']} | {c['fab']} | "
                 f"{c.get('blocked_leaf', '—')} |")
    L.append("")
    L.append("## 预注册预测（verifying run，construction-manifest.json）")
    L.append("")
    L.append("| entry | 预测 | 理由 |")
    L.append("|-------|------|------|")
    for e in main_entries:
        if e["name"].endswith("_holder_face"):
            L.append(f"| {e['name']} | ConfirmedLive | holder 面：face "
                     "requires 即契约 requires，调用点义务由面参数平凡 "
                     "discharge（direct 条目直接调用真实契约面，stub 条目"
                     "调用逐字转写面）|")
        elif "vm_space" in e["name"]:
            L.append(f"| {e['name']} | ConfirmedVacuous | fab 面：新铸 "
                     "VmSpaceOwner（tracked_new_val 空 OwnerSubtree）+ 空映射 "
                     "MetaRegionOwners 调真实 axiom；`old(regions).inv()` 的 "
                     "meta-slot 覆盖域子句对空映射不可满足，预期 verifier 在 "
                     "axiom requires 处拒绝，span 归因于 entry 调用点 |")
        else:
            L.append(f"| {e['name']} | ConfirmedVacuous | wt-b 已归档锚点"
                     "（tracked/manifest.json 原样复用）：唯一可写铸造通道"
                     "（新铸 PCell 配对）被拒 |")
    L.append("")
    L.append("## 预注册预测（leaf evidence run，construction-fab-blocked.json）")
    L.append("")
    L.append("8 个 fab 面（每契约一个注入模块）预期在预编译回路"
             "（`cargo-verus check --no-verify`）触发 rustc 命名解析失败：")
    L.append("")
    for c in contracts:
        if c["fab"] == "blocked":
            L.append(f"- `{c['key']}_construction`：铸造调用处报 E0599/E0603 "
                     f"族错误（{c['blocked_leaf']}）")
    L.append("")
    L.append("该 run 不产出判定（构建失败即证据形态本身）；8 条目据此"
             "**不**升级为机器 ConfirmedVacuous——其恒假结论保持为文档化"
             "通道穷尽论证（铸造通道缺席，编译级锚点 + s1 assume_new 归档），"
             "quantifier 判定 CandidateInconclusive 维持\"验证器不完备\"口径，"
             "registry 记证据升级但不虚构机器判定。")
    L.append("")
    L.append("## 预测汇总")
    L.append("")
    live = sum(1 for e in main_entries if e["name"].endswith("_holder_face"))
    vac = len(main_entries) - live
    L.append(f"verifying run 预测：ConfirmedVacuous={vac}，"
             f"ConfirmedLive={live}（合计 {len(main_entries)}，含 "
             f"{len(anchors)} 个 mutex 锚点）；leaf run 预测："
             "8 模块命名解析失败（每模块 ≥1 处铸造调用错误）。")
    with open(os.path.join(out, "CONSTRUCTION-EXPECTED.md"), "w") as f:
        f.write("\n".join(L) + "\n")


if __name__ == "__main__":
    main()
