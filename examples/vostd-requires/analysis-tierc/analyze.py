import json, collections, re, os
SCREEN="/home/jinser/vie/work/AES/spec-check/examples/vostd-requires/screening.json"
CENSUS="/tmp/m7e-census/census.json"
s=json.load(open(SCREEN)); c=json.load(open(CENSUS))
cfs={f["path"]:f for f in c["functions"]}
rows=[]
for f in s["functions"]:
    if f["tier"]!="C": continue
    cf=cfs.get(f["path"])
    if not cf: raise SystemExit("missing census "+f["path"])
    reqs=cf["requires"]
    feats=collections.Counter()
    classes=collections.Counter()
    texts=[]
    for r in reqs:
        classes[r["class"]]+=1
        for k,v in (r.get("features") or {}).items():
            if v: feats[k]+=1
        texts.append(r.get("text") or "")
    rows.append({"path":f["path"],"module":cf["module"],"name":cf["name"],
        "kind":cf["kind"],"mode":cf["mode"],
        "reasons":f["reasons"],"clauses":f["clauses"],
        "generics":cf["generics"]["names"],
        "params":cf["params"],"reqs":reqs,
        "features":dict(feats),"classes":dict(classes),"texts":texts,
        "source":cf["source"]})
json.dump(rows,open("/tmp/tierc-analysis/tierc_rows.json","w"),indent=1)
print("rows",len(rows))
print("kind",collections.Counter(r["kind"] for r in rows))
print("mode",collections.Counter(r["mode"] for r in rows))
print("class dist",collections.Counter((r["classes"].get("mechanical",0)>0,r["classes"].get("unsupported",0)>0) for r in rows))
print("feature combos",collections.Counter(tuple(sorted(r["features"])) for r in rows).most_common(30))
