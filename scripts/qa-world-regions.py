#!/usr/bin/env python3
"""General geometry/source seam gate for EVERY region in the catalog.

Run against the packaged data, not hand-picked downtown fixtures. No external
network or display. Only source-derived assertions; tolerance diagnostics for
DEM seams are emitted with region IDs for targeted source repair.
"""
import json,math,sys
from pathlib import Path

root=Path(__file__).resolve().parents[1]
assets=Path(sys.argv[1] if len(sys.argv)>1 else "dist/assets")
catalog=json.loads((root/"regions/catalog.json").read_text())
regions=catalog["regions"]
earth=6_371_008.8
def world(p,reg):
    lon=reg["origin"][0]+p["x"]/(earth*math.cos(math.radians(reg["origin"][1])))
    lat=reg["origin"][1]+p["z"]/earth
    return lon,lat
def finite(*values):return all(math.isfinite(float(v)) for v in values)
def bounds(points):
    return min(p[0] for p in points),min(p[1] for p in points),max(p[0] for p in points),max(p[1] for p in points)
def sample(t,lat,lon):
    west,south,east,north=t["bbox"]
    ix=max(0,min(t["width"]-1,(lon-west)/(east-west)*(t["width"]-1)))
    iz=max(0,min(t["height"]-1,(lat-south)/(north-south)*(t["height"]-1)))
    x=min(t["width"]-2,int(ix));z=min(t["height"]-2,int(iz))
    u=ix-x;v=iz-z
    def h(dx,dz):return t["heights_m"][(z+dz)*t["width"]+x+dx]
    sw,se,nw,ne=h(0,0),h(1,0),h(0,1),h(1,1)
    value=sw+(se-sw)*u+(nw-sw)*v if u+v<=1 else ne+(nw-ne)*(1-u)+(se-ne)*(1-v)
    return value+t["origin_elevation_m"]
data={}
summary={}
for record in regions:
    rid=record["id"]
    city=json.loads((assets/f"{rid}.json").read_text())
    terrain=json.loads((assets/f"{rid}-terrain.json").read_text())
    reg=city["region"]
    assert reg["id"]==rid and reg["bbox"]==record["bbox"],rid
    assert terrain["region_id"]==rid and terrain["bbox"]==reg["bbox"],rid
    assert terrain["width"]>=2 and terrain["height"]>=2
    assert terrain["width"]*terrain["height"]==len(terrain["heights_m"]),rid
    assert finite(terrain["origin_elevation_m"],*terrain["heights_m"]),rid
    assert city["buildings"] and city["streets"],rid
    rw,rs,re,rn=reg["bbox"]
    assigned=set()
    crossing=0
    for b in city["buildings"]:
        assert b["source_id"] not in assigned,(rid,"duplicate building",b["source_id"])
        assigned.add(b["source_id"])
        assert b["polygons"],(rid,b["source_id"])
        outline=[]
        for poly in b["polygons"]:
            assert poly["rings"] and all(len(ring)>=3 for ring in poly["rings"]),rid
            for ring in poly["rings"]:
                assert all(finite(p["x"],p["z"]) for p in ring),rid
            outline.extend(world(p,reg) for p in poly["rings"][0])
        cx=sum(p[0] for p in outline)/len(outline)
        cy=sum(p[1] for p in outline)/len(outline)
        # Half-open single-owner policy; small numerical tolerance only.
        assert rw-1e-8<=cx<re+1e-8 and rs-1e-8<=cy<rn+1e-8,(
            rid,b["source_id"],"nonresident building owner",cx,cy)
        bb=bounds(outline)
        if bb[0]<rw or bb[2]>re or bb[1]<rs or bb[3]>rn:
            crossing+=1  # Whole footprint MUST NOT be chopped to tile boundary.
    badroads=0
    for road in city["streets"]:
        path=road.get("path") or []
        assert len(path)>=2,(rid,"degenerate street",road.get("source_id"))
        assert all(finite(p["x"],p["z"]) for p in path),rid
        for p in path:
            lon,lat=world(p,reg)
            if lon<rw-1e-5 or lon>re+1e-5 or lat<rs-1e-5 or lat>rn+1e-5:
                badroads+=1
    assert badroads==0,(rid,"street geometry escaped bounded source",badroads)
    data[rid]=(city,terrain)
    summary[rid]={"buildings":len(city["buildings"]),
                  "completeBorderBuildings":crossing,
                  "streets":len(city["streets"]),
                  "terrain":f'{terrain["width"]}x{terrain["height"]}'}

# A building that spans the seam must NOT appear in both region GPU meshes.
for i,a in enumerate(regions):
    for b in regions[i+1:]:
        first,ta=data[a["id"]];second,tb=data[b["id"]]
        common={x["source_id"] for x in first["buildings"]}&{
            x["source_id"] for x in second["buildings"]}
        assert not common,(a["id"],b["id"],"duplicated seam buildings",sorted(common)[:10])
        aw,as_,ae,an=a["bbox"];bw,bs,be,bn=b["bbox"]
        orientation=None
        if abs(ae-bw)<=1e-9 or abs(be-aw)<=1e-9:
            lon=ae if abs(ae-bw)<=1e-9 else be
            low,high=max(as_,bs),min(an,bn)
            if high>low:
                orientation="east-west"
                points=[(lat,lon) for lat in (
                    low+(high-low)*i/20 for i in range(21))]
        elif abs(an-bs)<=1e-9 or abs(bn-as_)<=1e-9:
            lat=an if abs(an-bs)<=1e-9 else bn
            low,high=max(aw,bw),min(ae,be)
            if high>low:
                orientation="north-south"
                points=[(lat,lon) for lon in (
                    low+(high-low)*i/20 for i in range(21))]
        if not orientation:continue
        seams=[abs(sample(ta,lat,lon)-sample(tb,lat,lon))
               for lat,lon in points]
        greatest=max(seams)
        # A vertical seam in either direction must not contain a large
        # discontinuity from separate DEM acquisitions.
        assert greatest<20,(a["id"],b["id"],orientation,
                            "terrain crack",greatest)
        summary[f'{a["id"]}+{b["id"]}']={
            "sharedBoundary":orientation,
            "sampledPoints":len(seams),
            "maximumElevationMismatchM":round(greatest,3)}
print("WORLD_GEOMETRY_QA",json.dumps(summary,sort_keys=True))
