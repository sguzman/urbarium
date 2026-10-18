#!/usr/bin/env python3
"""Stamp a release worker WASM URL with its actual SHA-256 content version.

This file runs after the independent worker build, without altering its bytes.
Caching an unversioned worker executable across deployments is forbidden.
"""
import hashlib
from pathlib import Path
import re
import sys

root=Path(sys.argv[1]) if len(sys.argv)>1 else Path("dist")
wasm=root/"urbarium-mesh-worker_bg.wasm"
runtime=root/"mesh-worker-runtime.js"
digest=hashlib.sha256(wasm.read_bytes()).hexdigest()
source=runtime.read_text(encoding="utf-8")
marker="__URBARIUM_WORKER_SHA256__"
if source.count(marker)!=1:
    raise SystemExit(f"Expected exactly one worker hash placeholder, found {source.count(marker)}")
output=source.replace(marker,digest)
if not re.search(r'const workerWasmSha256="[a-f0-9]{64}";',output):
    raise SystemExit("Worker hash injection failed")
runtime.write_text(output,encoding="utf-8")
print(f"Stamped worker WASM SHA-256: {digest} ({wasm.stat().st_size} bytes)")
