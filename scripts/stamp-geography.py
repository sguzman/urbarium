#!/usr/bin/env python3
"""Stamp immutable geography versions into the HTML before Trunk bundles it.

The cache key is the SHA256 of actual generated geographic asset bytes, so
viewer-only deploys never redownload a byte-identical city. A geographic
data update cannot accidentally use an older URL-only CacheStorage entry.
"""
import hashlib
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parent.parent
HTML=ROOT/"index.html"
ASSETS=ROOT/"web"/"assets"
CATALOG=json.loads((ROOT/"regions"/"catalog.json").read_text(encoding="utf-8"))
REGIONS=CATALOG["regions"]
FILES=tuple(filename for region in REGIONS for filename in
    (region["id"]+".json",region["id"]+"-terrain.json"))
ANCHOR="window.__urbariumGeographyVersions = null;"
CATALOG_ANCHOR="window.__urbariumRegionCatalog = null;"

def stamp():
    versions={}
    for filename in FILES:
        path=ASSETS/filename
        if not path.is_file():
            raise SystemExit(f"Missing geography asset: {path}")
        versions[filename]=hashlib.sha256(path.read_bytes()).hexdigest()
    page=HTML.read_text(encoding="utf-8")
    if page.count(ANCHOR)!=1:
        raise SystemExit("Version anchor missing or duplicated; refusing unversioned cache build")
    if page.count(CATALOG_ANCHOR)!=1:
        raise SystemExit("Region catalog anchor missing or duplicated")
    page=page.replace(ANCHOR,
        "window.__urbariumGeographyVersions = "+
        json.dumps(versions,sort_keys=True,separators=(",",":"))+";")
    page=page.replace(CATALOG_ANCHOR,
        "window.__urbariumRegionCatalog = "+
        json.dumps(CATALOG,sort_keys=True,separators=(",",":"))+";")
    HTML.write_text(page,encoding="utf-8")
    print("Versioned geography cache:", {k:v[:16] for k,v in versions.items()})

if __name__=="__main__":
    stamp()
