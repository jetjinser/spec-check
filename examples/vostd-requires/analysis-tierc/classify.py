import json,re,collections
rows=json.load(open("/tmp/tierc-analysis/tierc_rows.json"))

def clause_shape(r):
    f=r.get("features") or {}
    tags=[]
    if f.get("quantifier"): tags.append("quantifier")
    if f.get("ctor"): tags.append("ctor")
    if f.get("call"): tags.append("call")
    if f.get("field_access"): tags.append("field_access")
    if r.get("consts"): tags.append("const")
    if r.get("class")=="mechanical": tags.insert(0,"mechanical")
    return tags

# whitelisted projection calls (return scalar/id or simple option status)
PROJ=re.compile(r"\.(id|value|cell_id|addr|pptr|vaddr|base|len|is_some|is_none|is_Some|is_None|level|idx|path|parent_level|active|popped_too_high)\s*\(")
ALLOWED_CALLS={"core::mem::size_of","ostd::mm::page_size","ostd::mm::PagingConstsTrait::NR_LEVELS"}
BAD_CALL_HINTS=("inv","wf","spec_index","contains","all_some","dom","value_rec","query_mapping","relate","metaregion","invariants","raw_item","item_into_raw","from_bytes","length_spec","valid_")

def projection_liftable(r):
    if r["class"]=="mechanical": return True
    f=r.get("features") or {}
    if f.get("quantifier") or f.get("ctor"): return False
    calls=r.get("calls",[])
    for c in calls:
        if c in ALLOWED_CALLS: continue
        if re.search(r"::impl&%\d+::(id|value|cell_id|addr|pptr|level|idx|path|parent_level|len)$",c): continue
        if c.endswith("::len") or c.endswith("::is_some") or c.endswith("::is_none"): continue
        return False
    txt=(r.get("text") or "")
    # any remaining call syntax not whitelisted is suspicious
    for m in re.finditer(r"([A-Za-z_][\w:]*)::<[^>]*>\s*\(|([A-Za-z_][\w:]*)\s*\(",txt):
        name=m.group(1) or m.group(2)
        if name in ("forall","exists","size_of"): continue
        if name and name.split("::")[-1] in ("id","value","cell_id","addr","pptr","vaddr","is_some","is_none","len","level","idx","path","parent_level","active","popped_too_high"):
            continue
        if name in ("Some","None","Tracked","Ghost"): continue
        return False
    if re.search(r"\b(inv|wf|spec_index|contains|all_some|dom|query_mapping|relate\w*|metaregion\w*|invariants)\s*\(",txt): return False
    return True

def primary_shape(x):
    tags=set()
    for r in x["reqs"]:
        f=r.get("features") or {}
        for k in ("call","field_access","quantifier","ctor"):
            if f.get(k): tags.add(k)
        if r.get("consts"): tags.add("const")
    for k in ("quantifier","ctor","call","field_access","const"):
        if k in tags: return k
    return "mechanical"

def combo(x):
    g=any(r.startswith("generic:") for r in x["reasons"]); rr=any(r.startswith("reference_param:") for r in x["reasons"]); t=any(r.startswith("ghost_param:") for r in x["reasons"])
    return "".join(k for k,b in [("G",g),("R",rr),("T",t)] if b)

for x in rows:
    x["combo"]=combo(x)
    x["shape"]=primary_shape(x)
    x["shape_tags"]=sorted({t for r in x["reqs"] for t in clause_shape(r)})
    x["liftable"]=all(projection_liftable(r) for r in x["reqs"])
    x["n_calls"]=sum(len(r.get("calls",[])) for r in x["reqs"])
    x["n_clauses"]=len(x["reqs"])
    x["has_mut_ref"]=any(p["kind"]=="reference" and p["type"].startswith("&mut") for p in x["params"])
    x["has_shared_ref"]=any(p["kind"]=="reference" and not p["type"].startswith("&mut") for p in x["params"])
    x["has_ghost"]=any(p["kind"]=="ghost" for p in x["params"])
    x["generic_names"]=x["generics"]
json.dump(rows,open("/tmp/tierc-analysis/tierc_classified.json","w"),indent=1)
print("shape",collections.Counter(x["shape"] for x in rows))
print("combo x shape")
for k,v in collections.Counter((x["combo"],x["shape"]) for x in rows).most_common(): print(k,v)
print("liftable funcs",sum(1 for x in rows if x["liftable"]))
print("liftable by combo",collections.Counter(x["combo"] for x in rows if x["liftable"]))
print("\nLIFTABLE candidates:")
for x in rows:
    if x["liftable"]:
        print("-",x["path"],"|",x["combo"],"|",x["shape"],"|",x["reasons"],"|",[t.replace('\n',' ')[:110] for t in x["texts"]])
