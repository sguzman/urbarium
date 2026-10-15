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
    qa_mode = "webgl2" if force_webgl else "webgpu"
    page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}", wait_until="domcontentloaded")
    page.wait_for_function(
        "(mode) => document.querySelector('#renderer-select')?.value === mode",
        arg=qa_mode, timeout=8000
    )
    try:
        page.wait_for_selector("canvas:not(#minimap-canvas)", timeout=30000)
        page.wait_for_function(
            "() => { const e=document.querySelector('#status');"
            " return e && (e.textContent.includes('Downtown LA ready')"
            " || e.textContent.includes('Viewer error')); }",
            timeout=60000,
        )
        assert page.locator("#renderer-select").count() == 1
        assert page.locator("#renderer-health").count() == 1
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
        if not force_webgl:
            assert "BrowserWebGpu" in selected_backend, selected_backend
        else:
            assert selected_backend == "Gl", selected_backend
        assert not errors, errors
        canvas = page.locator("canvas:not(#minimap-canvas)")
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
            "() => document.pointerLockElement === document.querySelector('canvas:not(#minimap-canvas)')",
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
        page.keyboard.press("Shift")
        page.wait_for_function(
            "() => document.querySelector('#status')?.getAttribute('data-speed-mode') === 'fast'",
            timeout=6000,
        )
        assert float(page.locator("#status").get_attribute("data-speed-mps")) > 50
        assert "FAST" in page.locator("#speed-state").inner_text()
        page.keyboard.press("Shift")
        page.wait_for_function(
            "() => document.querySelector('#status')?.getAttribute('data-speed-mode') === 'walk'",
            timeout=6000,
        )
        print("Shift toggles 18x fast movement and back to walk")
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
        minimap=page.locator("#minimap-canvas")
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.getAttribute('data-map-ready') === 'true'",
            timeout=15000,
        )
        old_zoom=float(minimap.get_attribute("data-map-width-m"))
        assert old_zoom > 100
        page.locator("#map-zoom-in").click()
        page.wait_for_function(
            "(before) => Number(document.querySelector('#minimap-canvas')?.dataset.mapWidthM) < before",
            arg=old_zoom, timeout=6000,
        )
        zoomed=float(minimap.get_attribute("data-map-width-m"))
        page.locator("#map-zoom-out").click()
        assert float(minimap.get_attribute("data-map-width-m")) > zoomed
        player_x=float(minimap.get_attribute("data-player-x"))
        camera_x=float(page.locator("#status").get_attribute("data-camera-x"))
        assert abs(player_x-camera_x) < 2.0, (player_x,camera_x)
        print("Minimap scene, camera marker and zoom controls passed")
        page.wait_for_function(
            "() => document.querySelector('#renderer-health')?.textContent.includes('renderer active')",
            timeout=6000,
        )
        assert page.locator("#renderer-health").is_visible()
        print("User-visible 3D frame health counter passed")
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
