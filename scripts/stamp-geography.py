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
FILES=("downtown-la.json","downtown-la-terrain.json")
ANCHOR="window.__urbariumGeographyVersions = null;"

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
    page=page.replace(ANCHOR,
        "window.__urbariumGeographyVersions = "+
        json.dumps(versions,sort_keys=True,separators=(",",":"))+";")
    HTML.write_text(page,encoding="utf-8")
    print("Versioned geography cache:", {k:v[:16] for k,v in versions.items()})

if __name__=="__main__":
    stamp()
