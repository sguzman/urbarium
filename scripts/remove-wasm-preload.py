#!/usr/bin/env python3
"""Remove Trunk's eager WASM preload so the tracked first fetch is cacheable.

Trunk's module bootstrap still imports the identical content-hashed WASM.
This deliberately leaves CSS and JS modulepreload entries unchanged.
"""
from pathlib import Path
import re
import sys

path = Path(sys.argv[1] if len(sys.argv) > 1 else "dist/index.html")
html = path.read_text(encoding="utf-8")
pattern = re.compile(
    r'<link\b(?=[^>]*\brel="preload")(?=[^>]*\bas="fetch")'
    r'(?=[^>]*\btype="application/wasm")'
    r'(?=[^>]*\bhref="\./urbarium-viewer-[0-9a-f]+_bg\.wasm")[^>]*>'
)
links = pattern.findall(html)
if len(links) != 1:
    raise SystemExit(f"Expected exactly one Trunk WASM preload, found {len(links)}")
if "const wasm = await init(" not in html:
    raise SystemExit("WASM module initializer missing; refusing to change startup")
html = html.replace(links[0], "", 1)
path.write_text(html, encoding="utf-8")
print("Removed eager WASM preload; module bootstrap remains unchanged")
