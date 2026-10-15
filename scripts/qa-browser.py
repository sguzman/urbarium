#!/usr/bin/env python3
"""Headless functional smoke check for the actual Rust/wgpu WebAssembly app."""
import time
from playwright.sync_api import sync_playwright

with sync_playwright() as p:
    browser = p.chromium.launch(
        headless=True,
        args=[
            "--enable-unsafe-webgpu",
            "--enable-features=Vulkan,WebGPUDeveloperFeatures",
            "--use-angle=swiftshader",
            "--disable-gpu-sandbox",
        ],
    )
    page = browser.new_page(viewport={"width": 1280, "height": 800})
    errors = []
    page.on("pageerror", lambda e: (errors.append(str(e)), print("PAGE ERROR:", str(e))))
    page.on("console", lambda m: print("CONSOLE", m.type, m.text))
    page.on("requestfailed", lambda r: print("REQUEST FAILED:", r.url, r.failure))
    page.on("response", lambda r: print("HTTP ERROR:", r.status, r.url)
            if r.status >= 400 else None)
    page.goto("http://127.0.0.1:8010/", wait_until="domcontentloaded")
    try:
        page.wait_for_selector("canvas", timeout=30000)
        page.wait_for_function(
            "() => { const e=document.querySelector('#status');"
            " return e && (e.textContent.includes('Downtown LA ready')"
            " || e.textContent.includes('Viewer error')); }",
            timeout=60000,
        )
        state = page.locator("#status").inner_text()
        print("Browser status:", state)
        if "Viewer error" in state:
            raise AssertionError(state)
        assert not errors, errors
        canvas = page.locator("canvas")
        canvas.click(position={"x": 500, "y": 420})
        page.keyboard.down("w")
        page.wait_for_timeout(450)
        page.keyboard.up("w")
        page.wait_for_timeout(450)
        assert page.locator("#status").inner_text().startswith("Downtown LA ready")
        print("Browser viewer canvas + WASM load + camera keys: passed")
    except Exception:
        print("PAGE ERRORS:", errors)
        print("CURRENT STATUS:", page.locator("#status").inner_text())
        print("DOCUMENT SCRIPTS:", page.locator("script").count())
        raise
    finally:
        page.screenshot(path="/tmp/urbarium-browser-debug.png", full_page=True)
        browser.close()
