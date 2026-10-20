#!/usr/bin/env python3
"""Hoist Trunk's fingerprinted WASM preload into early <head>.

Keep exactly one preload and the original module bootstrap. Browsers can
download the real Rust app before synchronous JavaScript blocks parsing.
"""
from pathlib import Path
import re
import sys

PRELOAD = re.compile(
    r'<link\b(?=[^>]*\brel="preload")(?=[^>]*\bas="fetch")'
    r'(?=[^>]*\btype="application/wasm")'
    r'(?=[^>]*\bhref="\./urbarium-viewer-[0-9a-f]+_bg\.wasm")[^>]*>'
)


def hoist(html: str) -> str:
    found = list(PRELOAD.finditer(html))
    if len(found) != 1:
        raise ValueError(f"Expected one hashed viewer WASM preload, got {len(found)}")
    head = re.search(r"<head(?:\s[^>]*)?>", html, re.I)
    if not head:
        raise ValueError("HTML head missing")
    if html.find("<script", head.end()) < 0:
        raise ValueError("Startup script missing")
    preload = found[0].group()
    without = html[:found[0].start()] + html[found[0].end():]
    output = without[:head.end()] + "\n  " + preload + without[head.end():]
    if output.count(preload) != 1 or output.index(preload) > output.index("<script"):
        raise ValueError("Unsafe WASM preload insertion")
    return output


def self_test() -> None:
    tag = '<link rel="preload" href="./urbarium-viewer-abcdef_bg.wasm" as="fetch" type="application/wasm" crossorigin>'
    html = '<!doctype html><html><head><meta charset="utf-8"><script src="blocking.js"></script></head><body>' + tag + '<script type="module">start()</script></body></html>'
    result = hoist(html)
    assert result.index(tag) < result.index('<script src="blocking.js"')
    assert result.count(tag) == 1
    assert result.count("start()") == 1
    try:
        hoist(html.replace(tag, ""))
    except ValueError:
        pass
    else:
        raise AssertionError("Missing preload must fail")


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        self_test()
        print("WASM preload positioning tests passed")
    else:
        path = Path(sys.argv[1] if len(sys.argv) > 1 else "dist/index.html")
        path.write_text(hoist(path.read_text(encoding="utf-8")), encoding="utf-8")
        print("Moved viewer WASM preload before synchronous JavaScript")
