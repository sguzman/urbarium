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
        page.wait_for_function(
            "() => Number(document.querySelector('#status')?.getAttribute('data-frame-count') || '0') >= 3",
            timeout=15000,
        )
        print("Render stage:", page.locator("#status").get_attribute("data-render-stage"))
        print("Presented frames:", page.locator("#status").get_attribute("data-frame-count"))
        # Test what the browser compositor actually displays, not merely that
        # the WASM application reported successful initialization.
        page.wait_for_timeout(350)
        pixels=canvas.screenshot()
        from io import BytesIO
        from PIL import Image
        with Image.open(BytesIO(pixels)) as image:
            image=image.convert("RGB")
            w,h=image.size
            values=[image.getpixel((int(w*x),int(h*y))) for x,y in (
                (0.2,0.25),(0.5,0.5),(0.8,0.75),(0.1,0.9),(0.9,0.1)
            )]
        print("Rendered canvas sample:",values)
        assert any(p != (255,255,255) for p in values), "Canvas remains blank white"
        print("Browser viewer WASM + drawn frames + visual canvas + camera keys: passed")
    except Exception:
        print("PAGE ERRORS:", errors)
        print("CURRENT STATUS:", page.locator("#status").inner_text())
        print("RENDER STAGE:", page.locator("#status").get_attribute("data-render-stage"))
        print("RENDER FRAMES:", page.locator("#status").get_attribute("data-frame-count"))
        print("DOCUMENT SCRIPTS:", page.locator("script").count())
        raise
    finally:
        page.screenshot(path="/tmp/urbarium-browser-debug.png", full_page=True)
        browser.close()
