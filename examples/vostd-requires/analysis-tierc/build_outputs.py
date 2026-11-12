import json, collections, re, datetime, csv, os
BASE="/tmp/tierc-analysis"
rows=json.load(open(f"{BASE}/tierc_classified.json"))
A=set(json.load(open(f"{BASE}/A_paths.json")))
SCREEN=json.load(open("/home/jinser/vie/work/AES/spec-check/examples/vostd-requires/screening.json"))

def simple(r):
    f=r.get("features") or {}
    return r["class"]=="mechanical" or (not f.get("call") and not f.get("quantifier") and not f.get("ctor"))
D={x["path"] for x in rows if not any(simple(r) for r in x["reqs"]) and x["path"] not in A}

A_META={
# path: (group, cost, benefit, confidence, expected_tier, evidence_note, registry)
"ostd::mm::page_size":("A-projection-typeconst",1,1,"high","A","requires 1<=level<=C::NR_LEVELS()+1; C=UserPtConfig/KernelPtConfig makes it scalar",""),
"ostd::mm::page_table::PageTableEntryTrait::new_pt":("A-projection-typeconst",1,1,"high","A","requires paddr<MAX_PADDR; Self=arch::x86::mm::PageTableEntry",""),
"ostd::mm::page_table::cursor::locking::dfs_get_idx_range":("A-projection-scalar",2,1,"medium","A","6 scalar range/page-size clauses over va_range fields",""),
"ostd::mm::page_table::is_valid_range":("A-projection-scalar",1,1,"high","A","requires r.end>0 (source line 838); C unused",""),
"ostd::mm::page_table::pte_index":("A-projection-typeconst",1,1,"high","A","1<=level<=C::NR_LEVELS(); C=UserPtConfig",""),
"ostd::mm::page_table::pte_index_bit_offset":("A-projection-typeconst",1,1,"high","A","1<=level<=C::NR_LEVELS(); C=UserPtConfig",""),
"ostd::specs::mm::frame::unique::impl&%5::tracked_borrow_repr_perm":("A-projection-tag",1,1,"medium","A","self.repr_perm is Some -> bool projection",""),
"ostd::specs::mm::page_table::cursor::owners::impl&%0::do_inc_index":("A-projection-scalar",1,1,"high","A","old(self).idx+1<NR_ENTRIES; idx scalar projection",""),
"ostd::specs::mm::page_table::cursor::owners::impl&%2::lemma_view_mappings_contains":("A-projection-scalar",1,1,"high","A","1<=self.level<=NR_LEVELS; level scalar field",""),
"ostd::specs::mm::page_table::node::entry_owners::impl&%1::tracked_borrow_mut_node":("A-projection-tag",1,1,"medium","A","old(self).kind is Node -> enum tag projection",""),
"ostd::specs::mm::page_table::node::entry_owners::impl&%1::tracked_borrow_node":("A-projection-tag",1,1,"medium","A","self.kind is Node -> enum tag projection",""),
"ostd::specs::mm::page_table::node::entry_owners::impl&%1::tracked_set_frame_prop":("A-projection-tag",1,1,"medium","A","old(self).kind is Frame -> enum tag projection",""),
"ostd::specs::mm::page_table::node::entry_owners::impl&%1::tracked_take_node":("A-projection-tag",1,1,"medium","A","old(self).kind is Node -> enum tag projection",""),
"ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_bound":("A-mechanical",1,1,"high","A","0<=n; mechanical, E=usize",""),
"ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_monotone":("A-mechanical",1,1,"high","A","0<=n1<=n2; mechanical",""),
"ostd::specs::mm::virt_mem::impl&%5::add":("A-projection-scalar",1,1,"high","A","0<=old(self).vaddr+n<usize::MAX; vaddr scalar field",""),
"ostd::util::range_alloc::impl&%4::alloc_specific":("A-projection-scalar",1,2,"high","A","self@.start/end and allocate_range.start/end scalar projections (range_alloc.rs:214)","F-009"),
"ostd::util::range_alloc::impl&%4::free":("A-projection-scalar",1,2,"high","A","self@/range projections + allocated.id()==self.id() (range_alloc.rs:480-482)","F-009"),
"ostd::util::ops::range_difference":("A-type-axiom",2,3,"medium","A","type axioms obeys_cmp/finite_range_matches_ord + forall cloned; T=usize (ops.rs:89-91)","F-025"),
"ostd::util::ops::lemma_range_difference_set":("A-type-axiom",2,2,"medium","A","type axioms obeys_cmp/finite_range_matches_ord; T=usize (ops.rs:52-53)","F-025"),
"ostd::sync::mutex::impl&%7::new":("A-shape-construction",1,3,"medium","A","perm.id()==mutex.cell_id(); shape Live / construction Vacuous (mutex.rs:225)","V-001"),
"ostd::sync::mutex::impl&%3::release_lock":("A-projection-id",1,2,"medium","A","perm.id()==self.cell_id(); id projection (mutex.rs:153)","F-002(adjacent)"),
"ostd::sync::mutex::impl&%3::unlock":("A-projection-id",1,2,"medium","A","perm.id()==self.cell_id(); id projection (mutex.rs:113)","F-002(adjacent)"),
"ostd::specs::mm::embedding::cursor::vm_space_cursor_embedded":("A-construction-model",2,3,"high","A","vm_space.inv() && old(regions).inv(); construction probe ConfirmedVacuous (M6)","V-002"),
"ostd::specs::mm::embedding::cursor::vm_space_cursor_mut_embedded":("A-construction-model",2,3,"high","A","vm_space.inv() && old(regions).inv(); construction probe ConfirmedVacuous (M6)","V-002"),
"ostd::mm::page_table::cursor::path_slot_as_mut":("A-shape-array",2,2,"medium","A","idx<NR_LEVELS && old(path)[idx] is Some; model path as Seq<Option<id>> (cursor/mod.rs:134)","M1-row04"),
"ostd::specs::mm::page_table::cursor::owners::impl&%0::tracked_restore":("A-projection-scalar",1,2,"high","A","old(self).idx < old(self).children.len() (owners.rs:193)","M1-row17"),
"ostd::specs::task::cpu_local::impl&%5::lemma_agree":("A-projection-id",1,1,"high","A","self.id()==auth.id() (cpu_local.rs:195)",""),
"ostd::mm::frame::meta::impl&%1::addr_of":("A-shape-id",1,1,"low","A","self == perm.value(); id equality, PointsTo construction blocked (meta.rs:361)",""),
"ostd::mm::io::VmIo::read_bytes":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_read_requires(); all crate impls return true -> candidate dead/vacuous (io.rs:1312)","io-family"),
"ostd::mm::io::VmIo::read_val":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_read_requires() (io.rs:1326)","io-family"),
"ostd::mm::io::VmIo::read_slice":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_read_requires() (io.rs:1343)","io-family"),
"ostd::mm::io::VmIo::write_bytes":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_write_requires() (io.rs:1363)","io-family"),
"ostd::mm::io::VmIo::write_val":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_write_requires() (io.rs:1378)","io-family"),
"ostd::mm::io::VmIo::write_slice":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_write_requires() (io.rs:1391)","io-family"),
"ostd::mm::io::VmIo::write_vals":("A-type-level",1,3,"medium","A","!Self::obeys_vmio_write_requires() + offset+align<=usize::MAX (io.rs:1429)","io-family"),
"ostd::sync::rcu::impl&%6::update":("A-shape-property",1,2,"medium","A","self.is_nullable() || new_ptr is Some; bool/Option projection (rcu/mod.rs:301)","rcu-family"),
"ostd::sync::rcu::impl&%7::compare_exchange":("A-shape-property",1,2,"medium","A","self.rcu.is_nullable() || new_ptr is Some (rcu/mod.rs:539)","rcu-family"),
}
# page_size lemma family and page_table lemma family auto
for x in rows:
    p=x["path"]
    if p.startswith("ostd::specs::mm::page_table::cursor::page_size_lemmas::"):
        A_META.setdefault(p,("A-projection-typeconst",1,1,"high","A","level/va scalar constraints + C constants; C=UserPtConfig",""))
    if p.startswith("ostd::specs::mm::page_table::lemma_") and p not in A_META:
        A_META.setdefault(p,("A-projection-typeconst",1,1,"high","A","level/va scalar constraints + page_size/NR_LEVELS constants; C=UserPtConfig",""))
    if p.startswith("ostd::cpu::set::impl&%6::"):
        A_META.setdefault(p,("A-inv-model",2,2,"medium","A","requires self.inv() over bitset; concrete bitvector model / in-crate source inspection (cpu/set.rs:113)","F-026(adjacent)"))

# validate all A in metadata
missing=[p for p in A if p not in A_META]
if missing:
    print("MISSING A_META",len(missing)); print("\n".join(missing[:20]))

# registry map by substring
def registry_for(p):
    out=[]
    if "mutex::impl&%7::new" in p: out.append("V-001")
    if "vm_space_cursor_embedded" in p or "vm_space_cursor_mut_embedded" in p: out.append("V-002")
    if "range_alloc::impl&%4::alloc_specific" in p or "range_alloc::impl&%4::free" in p: out.append("F-009")
    if "util::ops::range_difference" in p or "lemma_range_difference_set" in p: out.append("F-025")
    if "cpu::set::impl&%6::" in p: out.append("F-026(adjacent)")
    if "mutex::impl&%3::release_lock" in p or "mutex::impl&%3::unlock" in p: out.append("F-002(adjacent)")
    if "page_table::cursor::locking::try_traverse" in p: out.append("V-002/M1-row02")
    if "cursor::path_slot_as_mut" in p: out.append("M1-row04")
    if "entry::impl&%2::alloc_if_none" in p: out.append("M1-row05")
    if "entry::impl&%2::split_if_mapped_huge" in p: out.append("M1-row06")
    if "entry::impl&%3::alloc_absent_child" in p: out.append("M1-row09")
    if "node::impl&%2::lock" in p: out.append("M1-row11")
    if "node::impl&%2::make_guard_unchecked" in p: out.append("M1-row12")
    if "cursor::owners::impl&%0::tracked_restore" in p: out.append("M1-row17")
    return out

def generic_recipe(x):
    names=x["generics"]
    if not names: return "none"
    if x["kind"]=="trait_decl":
        # concrete impls known for PageTableEntryTrait/VmIo/NonNullPtr
        if any(n=="Self%" for n in names): return "G1-instantiate-concrete-impl"
        return "G2-type-choice"
    if any("impl%" in n for n in names) or any("Projection" in n for n in names): return "G3-associated-projection"
    if any(n in ("C",) for n in names): return "G1-instantiate-C"
    if any(n in ("M",) for n in names): return "G2-type-choice-M"
    if any(n in ("Self%",) for n in names): return "G1-instantiate-concrete-impl"
    if any(n in ("E","Fallibility") for n in names): return "G1-instantiate-simple"
    if any(n in ("A","P","T","V","L","G","I","F","R","FCancel","Dma","M1","impl%1","impl%2","impl%3") for n in names): return "G2-type-choice"
    return "G2-type-choice"

def reference_recipe(x):
    if not any(r.startswith("reference_param:") for r in x["reasons"]): return "none"
    allsimple=all(simple(r) for r in x["reqs"])
    if allsimple: return "R1-scalar-projection"
    if x["has_mut_ref"] and not any(simple(r) for r in x["reqs"]): return "R4-mut-state-invariant"
    if x["classes"].get("mechanical") and x["classes"].get("unsupported"): return "R3-mixed"
    return "R2-invariant-id"

def ghost_recipe(x):
    types=[p["type"] for p in x["params"] if p["kind"]=="ghost"]
    if not types: return "none"
    if all(("Ghost<usize>" in t or "Ghost<vstd::cell::CellId>" in t or t.startswith("Ghost<")) for t in types): return "T1-trivial-ghost"
    blocked=any(any(k in t for k in ["simple_pptr::PointsTo","array_ptr::PointsTo","FrameRef","PageTableGuard","MetadataPerm","FracMetadataPerm","MetaSlotOwner","NodeOwner","CursorOwner","CursorContinuation","EntryOwner","PageTableOwner","LinkedListOwner","UniqueFrameOwner","MetaSlotStorage","VmIoOwner","VmSpaceOwner","TlbModel","MemView","KVirtAreaOwner"]) for t in types)
    if blocked: return "T3-construction-blocked"
    if any("MetaRegionOwners" in t for t in types): return "T2p-metaregion-partial"
    if all(any(k in t for k in ["Guards","pcell::PointsTo","pcell_maybe_uninit::PointsTo","count_auth::Count","OwnerSubtree"]) for t in types): return "T2-constructible"
    if any("PermissionU64" in t or "PermissionU32" in t for t in types): return "T2-unknown-atomic-perm"
    return "T_unknown"

outf=[]
for x in rows:
    p=x["path"]
    gr=generic_recipe(x); rr=reference_recipe(x); tr=ghost_recipe(x)
    reg=""
    if p in A:
        group,cost,benefit,conf,tier,note,reg=A_META[p]
        code_map={
          "A-projection-typeconst":"G1-instantiate-C + R1-scalar-projection",
          "A-projection-scalar":"R1-scalar-projection (+G1/G2 instantiation)",
          "A-projection-tag":"G1-instantiate + R1-enum-tag-projection",
          "A-projection-id":"G1-instantiate + R1-id-projection",
          "A-mechanical":"G1-instantiate-simple + mechanical",
          "A-type-axiom":"G2-type-choice + type-axiom-eval",
          "A-shape-construction":"G1-instantiate + R1-id-projection + construction-probe",
          "A-construction-model":"R2-invariant-id + construction-probe",
          "A-shape-array":"G1-instantiate + R1-array-element-model",
          "A-shape-id":"R1-id-projection (shape)",
          "A-type-level":"G1-instantiate-concrete-impl + type-level-eval",
          "A-inv-model":"R2-invariant-id + concrete-inv-model",
          "A-shape-property":"G2-type-choice + R1-bool/option-projection",
        }
        recipe_code=code_map.get(group, f"{gr}+{rr}+{tr}".replace("none+","").replace("+none",""))
        exp="A"
    elif p in D:
        group="D-deferred"; cost=3; benefit=1; conf="low"; tier="C(deferred)"; note="no mechanical/scalar-projection clause; only opaque invariant/resource predicates"; exp="C(deferred)"
        recipe_code=f"{gr}+{rr}+{tr}".replace("none+","").replace("+none","")
    else:
        group="B-uninterp"; cost=2; benefit=1; conf="medium"; tier="B"; note="at least one scalar clause but >=1 opaque clause remains; uninterp abstraction expected Inconclusive"; exp="B"
        recipe_code=f"{gr}+{rr}+{tr}".replace("none+","").replace("+none","")
    regs=registry_for(p)
    if reg and reg not in regs: regs.insert(0,reg)
    risk="low"
    if group=="A-type-level": risk="medium (crate-scope only; external impls may set obeys=false)"
    elif group in ("A-shape-construction","A-construction-model","A-shape-id","A-shape-array","A-shape-property"): risk="medium-high (model-relative; id/shape abstraction may not preserve reachability)"
    elif group=="A-type-axiom": risk="medium (type instance choice; T=usize assumption)"
    elif group=="A-inv-model": risk="medium (private bitset replicated as abstract model)"
    elif group=="A-projection-typeconst": risk="low-medium (C/Self instance choice changes constants)"
    elif p in D: risk="high (abstraction loses resource/permission meaning; B-only at best)"
    outf.append({
        "path":p,"module":x["module"],"name":x["name"],"kind":x["kind"],"mode":x["mode"],
        "source":x["source"],
        "blockers":{"generic":x["generics"],
                    "reference":[{"name":q["name"],"type":q["type"],"mutable":q["type"].startswith("&mut")} for q in x["params"] if q["kind"]=="reference"],
                    "ghost":[{"name":q["name"],"type":q["type"]} for q in x["params"] if q["kind"]=="ghost"]},
        "blocker_combo":x["combo"],
        "requires":{"clauses":len(x["reqs"]),"mechanical":x["classes"].get("mechanical",0),"unsupported":x["classes"].get("unsupported",0),
                    "features":x["features"],"shape_tags":x["shape_tags"],"primary_shape":x["shape"],
                    "texts":x["texts"]},
        "recipe":{"code":recipe_code,"group":group,"generic":gr,"reference":rr,"ghost":tr,
                  "steps":[note],"expected_tier":tier,"confidence":conf},
        "cost":cost,"benefit":benefit,"cost_benefit":round(benefit/cost,2),
        "risk":risk,
        "registry":regs,
    })

group_desc={
 "A-projection-typeconst":"实例化泛型后，requires 只剩层级/地址/页大小等标量常量与算术；预期 ConfirmedLive。",
 "A-projection-scalar":"引用/不透明结构体的标量字段（idx/level/start/end/range）投影为标量参数；预期 ConfirmedLive。",
 "A-projection-tag":"枚举 variant 标签/option 状态投影为 bool；预期 ConfirmedLive。",
 "A-projection-id":"ghost/exec 身份等价（id()/value()/cell_id()）投影为 id 等值；shape 层 ConfirmedLive。",
 "A-mechanical":"纯机械子句，只需实例化类型参数；预期 ConfirmedLive。",
 "A-type-axiom":"类型级公理（obeys_cmp/finite_range_matches_ord/cloned）实例化后求值；若失败即真阳性。",
 "A-shape-construction":"Id 等值 shape + 可铸造 tracked 资源构造探针；shape Live / construction Vacuous（V-001 分歧）。",
 "A-construction-model":"requires 为 owner/region 不变式；用真实 axiom/构造资源做 holder/fab 面（V-002 已机器 ConfirmedVacuous）。",
 "A-shape-array":"数组/序列元素存在性（old(path)[idx] is Some）用 Seq<Option<id>> 模型；预期 Live。",
 "A-shape-id":"exec/ghost 对象同一性投影为 id 等值；仅 shape 证据，构造链受 T3 阻断。",
 "A-type-level":"trait 默认方法 requires 以 Self 类型谓词为条件；枚举 crate 内 impl 求值，可能出现 crate-scope 真空洞。",
 "A-inv-model":"requires 为可具体展开的状态不变式（如 CpuSet.inv 的 bitset 约束）；用具体模型/构造探针。",
 "A-shape-property":"闭 spec 谓词（is_nullable/Option is Some）投影为 bool；预期 Live。",
 "B-uninterp":"至少一条标量子句可机械转写，但仍有资源/不变式子句；按 M1 纪律转 uninterp，预期 CandidateInconclusive。",
 "D-deferred":"全部子句为资源/不变式/深结构谓词，无标量或标量投影内容；id 抽象只能得到零信息 B，构造链多被阻断，建议暂缓。",
}
groups=[]
for g,c in collections.Counter(r["recipe"]["group"] for r in outf).most_common():
    ex=[r["path"] for r in outf if r["recipe"]["group"]==g][:4]
    groups.append({"group":g,"count":c,"description":group_desc.get(g,""),"examples":ex,
                   "expected_tier":("A" if g.startswith("A-") else ("B" if g=="B-uninterp" else "C(deferred)"))})

json.dump({"schema":"spec-check.tierc-recipes/1",
 "generated_at":datetime.datetime.now(datetime.timezone.utc).isoformat(),
 "inputs":{"screening":"~/vie/work/AES/spec-check/examples/vostd-requires/screening.json",
           "census":"/tmp/m7e-census/census.json","vir":"/tmp/m7a-vir-export2/crate.vir",
           "frozen_source":"/tmp/vostd-43da079-ktest-chan (43da079)",
           "design":"~/vie/work/AES/spec-check/docs/M7e-requires-screening.md",
           "registry":"~/vie/work/AES/research/experiments/spec-quality-two-day/findings/registry.json"},
 "summary":{"tier_c":len(rows),"after_A":sum(1 for r in outf if r["recipe"]["expected_tier"]=="A"),
            "after_B":sum(1 for r in outf if r["recipe"]["expected_tier"]=="B"),
            "after_C_deferred":sum(1 for r in outf if r["recipe"]["expected_tier"].startswith("C")),
            "combo":dict(collections.Counter(x["combo"] for x in rows)),
            "shape":dict(collections.Counter(x["shape"] for x in rows)),
            "recipe_group":dict(collections.Counter(r["recipe"]["group"] for r in outf))},
 "groups":groups,
 "functions":outf},open(f"{BASE}/recipes.json","w"),indent=1,ensure_ascii=False)

# priority csv: top 20 list
TOP=[
("ostd::mm::io::VmIo::read_bytes","A-type-level",1,3,"new true-positive candidate: all crate impls obeys=true -> !obeys=false"),
("ostd::mm::io::VmIo::write_bytes","A-type-level",1,3,"same cluster (7 fallback methods)"),
("ostd::sync::mutex::impl&%7::new","A-shape-construction",1,3,"V-001 disputed: shape Live vs construction Vacuous"),
("ostd::specs::mm::embedding::cursor::vm_space_cursor_embedded","A-construction-model",2,3,"V-002: construction ConfirmedVacuous (M6)"),
("ostd::specs::mm::embedding::cursor::vm_space_cursor_mut_embedded","A-construction-model",2,3,"V-002: construction ConfirmedVacuous (M6)"),
("ostd::util::ops::range_difference","A-type-axiom",2,3,"F-025: type axioms; instantiate T=usize"),
("ostd::util::ops::lemma_range_difference_set","A-type-axiom",2,2,"F-025 companion lemma"),
("ostd::util::range_alloc::impl&%4::alloc_specific","A-projection-scalar",1,2,"F-009: scalar range projections"),
("ostd::util::range_alloc::impl&%4::free","A-projection-scalar",1,2,"F-009: range projections + id equality"),
("ostd::mm::page_table::cursor::path_slot_as_mut","A-shape-array",2,2,"M1 row04: upgrade Inconclusive -> Live with array model"),
("ostd::specs::mm::page_table::cursor::owners::impl&%0::tracked_restore","A-projection-scalar",1,2,"M1 row17: idx/len scalar projection"),
("ostd::sync::mutex::impl&%3::release_lock","A-projection-id",1,2,"F-002 adjacent: perm/cell id equality"),
("ostd::sync::mutex::impl&%3::unlock","A-projection-id",1,2,"same"),
("ostd::cpu::set::impl&%6::add","A-inv-model",2,2,"F-026 adjacent: CpuSet.inv over bitset"),
("ostd::cpu::set::impl&%6::contains","A-inv-model",2,2,"same inv model; cluster of 9"),
("ostd::sync::rcu::impl&%6::update","A-shape-property",1,2,"RCU family: nullable/Option projection"),
("ostd::mm::page_table::pte_index","A-projection-typeconst",1,1,"cheap A; level range"),
("ostd::mm::page_size","A-projection-typeconst",1,1,"cheap A; 8-lemma family"),
("ostd::specs::mm::page_table::cursor::owners::impl&%2::lemma_view_mappings_contains","A-projection-scalar",1,1,"cheap A; level scalar field"),
("ostd::specs::mm::page_table::node::entry_owners::impl&%1::tracked_borrow_node","A-projection-tag",1,1,"4-function tag family"),
]
byp={r["path"]:r for r in outf}
with open(f"{BASE}/priority.csv","w",newline="") as f:
    w=csv.writer(f)
    w.writerow(["rank","function","blockers","requires_shape","recipe_group","recipe_code","expected_tier","cost","benefit","benefit_per_cost","registry","evidence"])
    for i,(p,g,c,b,note) in enumerate(TOP,1):
        r=byp[p]
        w.writerow([i,p,r["blocker_combo"],r["requires"]["primary_shape"],g,r["recipe"]["code"],"A",c,b,round(b/c,2),";".join(r["registry"]),note])
print("A",sum(1 for r in outf if r["recipe"]["expected_tier"]=="A"),"B",sum(1 for r in outf if r["recipe"]["expected_tier"]=="B"),"D",sum(1 for r in outf if r["recipe"]["expected_tier"].startswith("C")))
print("groups",collections.Counter(r["recipe"]["group"] for r in outf))
