#!/usr/bin/env python3
"""Headless functional smoke check for the actual Rust/wgpu WebAssembly app."""
import os
import time
from playwright.sync_api import sync_playwright

force_webgl = os.environ.get("URBARIUM_QA_FORCE_WEBGL") == "1"
print("Forced WebGL2 fallback QA:", force_webgl)

with sync_playwright() as p:
    browser = p.chromium.launch(
        headless=False,
        args=[
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--disable-dev-shm-usage",
            "--window-size=1280,800",
            "--enable-unsafe-webgpu",
            "--enable-features=Vulkan",
            "--use-angle=vulkan",
            "--use-vulkan=swiftshader",
            "--use-webgpu-adapter=swiftshader",
            "--disable-vulkan-surface",
        ],
    )
    page = browser.new_page(viewport={"width": 1280, "height": 800})
    if force_webgl:
        # Hide browser WebGPU before the WASM loader runs, exercising WebGL2.
        page.add_init_script("""
            Object.defineProperty(navigator, 'gpu', {
                configurable: true, value: undefined
            });
        """)
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
        selected_backend = page.locator("#status").get_attribute("data-gpu-backend")
        print("Selected graphics backend:", selected_backend)
        if force_webgl:
            assert selected_backend is not None and "gl" in selected_backend.lower(), (
                "WebGL2 fallback test selected " + repr(selected_backend)
            )
        if "Viewer error" in state:
            raise AssertionError(state)
        assert not errors, errors
        canvas = page.locator("canvas")
        page.wait_for_function(
            "() => Number(document.querySelector('#status')?.getAttribute('data-eye-clearance')) >= 1.7",
            timeout=15000,
        )
        obstacles=int(page.locator("#status").get_attribute("data-building-obstacles") or "0")
        assert obstacles > 300, f"Geographic building colliders missing: {obstacles}"
        terrain_points=int(page.locator("#status").get_attribute("data-terrain-points") or "0")
        assert terrain_points == 4225, f"Terrain DEM missing or incomplete: {terrain_points}"
        print("Footprint obstacles loaded:", obstacles)
        print("Geographic elevation samples loaded:", terrain_points)
        eye_height=float(page.locator("#status").get_attribute("data-eye-clearance"))
        assert 1.65 <= eye_height <= 1.75, f"Expected ~1.7 m eye height, got {eye_height}"
        canvas.click(position={"x": 500, "y": 420})
        page.wait_for_function(
            "() => document.pointerLockElement === document.querySelector('canvas')",
            timeout=12000,
        )
        assert page.locator("#input-state").get_attribute("data-pointer-locked") == "true"
        before_yaw=float(page.locator("#status").get_attribute("data-camera-yaw"))
        page.mouse.move(590,420,steps=5)
        page.wait_for_timeout(300)
        after_yaw=float(page.locator("#status").get_attribute("data-camera-yaw"))
        assert after_yaw < before_yaw-0.03, (
            f"FPS mouse look did not rotate right: {before_yaw} -> {after_yaw}"
        )
        print("Pointer lock + correct non-drag FPS mouse look passed:",before_yaw,"->",after_yaw)
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
        if all(p == (255,255,255) for p in values):
            # Control experiment in exactly the same Chromium instance:
            # can a tiny stand-alone WebGPU command paint a canvas?
            control = page.evaluate("""async () => {
                try {
                    const adapter = await navigator.gpu?.requestAdapter();
                    if (!adapter) return {error: "No WebGPU adapter"};
                    const device = await adapter.requestDevice();
                    const canvas = document.createElement("canvas");
                    canvas.id = "webgpu-control";
                    canvas.width = canvas.height = 128;
                    canvas.style.cssText =
                        "position:fixed;top:200px;left:650px;width:128px;height:128px;z-index:99999";
                    document.body.appendChild(canvas);
                    const ctx = canvas.getContext("webgpu");
                    const format = navigator.gpu.getPreferredCanvasFormat();
                    ctx.configure({device,format,alphaMode:"opaque"});
                    const command = device.createCommandEncoder();
                    const view = ctx.getCurrentTexture().createView();
                    const pass = command.beginRenderPass({
                        colorAttachments:[{view,
                            clearValue:{r:1,g:0,b:0,a:1},
                            loadOp:"clear",storeOp:"store"}]
                    });
                    pass.end();
                    device.queue.submit([command.finish()]);
                    await device.queue.onSubmittedWorkDone();
                    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
                    return {format,adapter:adapter.info?.device ?? "unknown"};
                } catch (e) { return {error:String(e)}; }
            }""")
            print("WebGPU control result:", control)
            if page.locator("#webgpu-control").count():
                with Image.open(BytesIO(page.locator("#webgpu-control").screenshot())) as image:
                    print("WebGPU red-clear control pixel:", image.convert("RGB").getpixel((64,64)))
            print("Canvas count:", page.locator("canvas").count())
        assert any(p != (255,255,255) for p in values), "Canvas remains blank white"
        page.keyboard.press("Escape")
        page.wait_for_function(
            "() => document.pointerLockElement === null",
            timeout=8000,
        )
        print("Pointer released on Escape")
        print("Browser viewer WASM + drawn frames + visual canvas + camera keys: passed")
    except Exception:
        print("PAGE ERRORS:", errors)
        print("CURRENT STATUS:", page.locator("#status").inner_text())
        print("RENDER STAGE:", page.locator("#status").get_attribute("data-render-stage"))
        print("RENDER FRAMES:", page.locator("#status").get_attribute("data-frame-count"))
        print("GPU BACKEND:", page.locator("#status").get_attribute("data-gpu-backend"))
        print("DOCUMENT SCRIPTS:", page.locator("script").count())
        raise
    finally:
        page.screenshot(path=("/tmp/urbarium-browser-debug-webgl.png"
                              if force_webgl else "/tmp/urbarium-browser-debug.png"), full_page=True)
        browser.close()
