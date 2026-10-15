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
    page.on("pageerror", lambda e: errors.append(str(e)))
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
    finally:
        page.screenshot(path="/tmp/urbarium-browser-debug.png", full_page=True)
        browser.close()
