#!/usr/bin/env python3
"""Verify independently buildable, non-overlapping, directly adjacent city sections.

This is the groundwork for streaming tiles. It does NOT assert seam-free terrain
or runtime mesh stitching, both of which require separate implementation.
"""
import json,math,re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
catalog=json.loads((ROOT/"regions/catalog.json").read_text(encoding="utf-8"))
assert catalog.get("schema_version")==1
sections=catalog["regions"]
assert len(sections)>=2 and sections[0]["id"]=="downtown-la"
assert len({t["id"] for t in sections})==len(sections)
ref=catalog["world_reference"]
assert len(ref)==2 and all(math.isfinite(v) for v in ref)
for item in sections:
    name=item["id"]
    assert re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*",name), name
    assert item["definition"]==f"regions/{name}.json"
    region=json.loads((ROOT/item["definition"]).read_text(encoding="utf-8"))
    assert region["id"]==name
    assert region["bbox"]==item["bbox"],name
    w,s,e,n=region["bbox"]
    assert w<e and s<n and w<=region["origin"][0]<=e and s<=region["origin"][1]<=n
    assert item["label"].strip()
for i,a in enumerate(sections):
    aw,as_,ae,an=a["bbox"]
    for b in sections[i+1:]:
        bw,bs,be,bn=b["bbox"]
        # Prevent duplicate building polygons in the interiors of sections.
        assert not (min(ae,be)>max(aw,bw)+1e-9 and
                    min(an,bn)>max(as_,bs)+1e-9), (a["id"],b["id"])
adjacent=False
for a in sections:
    if a["id"]!="downtown-la":continue
    for b in sections:
        if b["id"]==a["id"]:continue
        aw,as_,ae,an=a["bbox"];bw,bs,be,bn=b["bbox"]
        if abs(ae-bw)<1e-9 and min(an,bn)>max(as_,bs):
            adjacent=True
assert adjacent,"First expansion must share the Downtown east boundary"
print("REGION_CATALOG_OK",",".join(t["id"] for t in sections))
