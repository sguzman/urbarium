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
    requests = []
    page.on("request", lambda r: requests.append(r.url))
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
        # The former 1280x800 drawing buffer left blank right/bottom borders
        # on larger browser windows. Test both CSS and actual GPU pixel size.
        page.set_viewport_size({"width":1664,"height":926})
        page.wait_for_function("""() => {
            const c=document.querySelector('canvas:not(#minimap-canvas):not(#webgpu-control)');
            const s=document.querySelector('#status');
            if(!c||!s)return false;
            const box=c.getBoundingClientRect();
            const expectedW=Math.round(innerWidth*devicePixelRatio);
            const expectedH=Math.round(innerHeight*devicePixelRatio);
            return Math.abs(box.width-innerWidth)<2 &&
                Math.abs(box.height-innerHeight)<2 &&
                c.width===expectedW && c.height===expectedH &&
                Number(s.getAttribute('data-surface-width'))===expectedW &&
                Number(s.getAttribute('data-surface-height'))===expectedH;
        }""",timeout=15000)
        print("Fullscreen canvas and backing size verified:",
              canvas.evaluate("(c) => [c.width,c.height,c.getBoundingClientRect().width,c.getBoundingClientRect().height]"))
        page.set_viewport_size({"width":1120,"height":740})
        page.wait_for_function("""() => {
            const c=document.querySelector('canvas:not(#minimap-canvas):not(#webgpu-control)');
            const s=document.querySelector('#status');
            return c && s &&
                Math.abs(c.getBoundingClientRect().width-1120)<2 &&
                Math.abs(c.getBoundingClientRect().height-740)<2 &&
                Number(s.getAttribute('data-surface-width'))===c.width &&
                Number(s.getAttribute('data-surface-height'))===c.height;
        }""",timeout=15000)
        print("3D viewport tracks dynamic browser resize")
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
        assert after_yaw > before_yaw+0.03, (
            f"Right mouse must turn compass heading eastward: {before_yaw} -> {after_yaw}"
        )
        print("Pointer lock + correct non-drag FPS mouse look passed:",before_yaw,"->",after_yaw)
        # Cross-check the compass arrow against the precise camera bearing
        # published by Rust. East is sin(yaw), north is cos(yaw).
        page.wait_for_function("""() => {
            const m=document.querySelector('#minimap-canvas');
            const s=document.querySelector('#status');
            if (!m||!s)return false;
            const yaw=Number(s.getAttribute('data-camera-yaw'));
            const east=Number(m.getAttribute('data-heading-east'));
            const north=Number(m.getAttribute('data-heading-north'));
            return Number.isFinite(yaw) &&
                Math.abs(east-Math.sin(yaw))<0.005 &&
                Math.abs(north-Math.cos(yaw))<0.005;
        }""",timeout=8000)
        print("North-up minimap arrow agrees with Rust's east/north bearing")
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
        page.wait_for_function(
            "(before) => Number(document.querySelector('#minimap-canvas')?.dataset.mapWidthM) > before",
            arg=zoomed, timeout=6000,
        )
        player_x=float(minimap.get_attribute("data-player-x"))
        camera_x=float(page.locator("#status").get_attribute("data-camera-x"))
        assert abs(player_x-camera_x) < 2.0, (player_x,camera_x)
        player_z=float(minimap.get_attribute("data-player-z"))
        camera_z=float(page.locator("#status").get_attribute("data-camera-z"))
        assert abs(player_z-camera_z)<2.0, (player_z,camera_z)
        assert minimap.get_attribute("data-map-source") == "rust-shared", (
            "Minimap must consume the payload already fetched by Rust, not refetch"
        )
        city_requests=[u for u in requests if "/assets/downtown-la.json" in u]
        dem_requests=[u for u in requests if "/assets/downtown-la-terrain.json" in u]
        assert len(city_requests) == 1, (
            f"Duplicate city network loads: {city_requests}"
        )
        assert len(dem_requests) == 1, (
            f"Duplicate elevation network loads: {dem_requests}"
        )
        assert page.locator("#map-retry").count() == 1
        assert page.evaluate("() => window.__urbariumCityJson === undefined")
        print("Rust-to-minimap shared data works with exactly one city and DEM request")
        print("Minimap scene, camera marker and zoom controls passed")
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.getAttribute('data-landmark-count') === '9'",
            timeout=15000,
        )
        assert page.locator("#world-landmarks .world-landmark").count() == 9
        assert page.locator("#world-landmarks").is_visible()
        assert page.locator("#status").get_attribute("data-camera-pose") is not None
        print("Nine source-backed landmarks are mapped and anchored to the camera")
        # A repeat fetch must be fulfilled from persistent CacheStorage,
        # without any second HTTP transfer to this flaky-network-sensitive app.
        saved=page.evaluate("""async () => {
            const store=await caches.open("urbarium-geography-downtown-la-v1");
            const city=await store.match("./assets/downtown-la.json");
            const dem=await store.match("./assets/downtown-la-terrain.json");
            return [Boolean(city),Boolean(dem)];
        }""")
        assert saved == [True, True], f"Geography not persisted on first visit: {saved}"
        before=len([u for u in requests if "/assets/downtown-la" in u])
        page.evaluate("""async () => {
            const responses=await Promise.all([
                fetch("./assets/downtown-la.json"),
                fetch("./assets/downtown-la-terrain.json")
            ]);
            if(responses.some(r=>!r.ok))throw Error("Cached geographic fetch failed");
        }""")
        after=len([u for u in requests if "/assets/downtown-la" in u])
        assert after==before, "Cached geography caused redundant network requests"
        print("Geography CacheStorage persisted and reused both assets without HTTP")

        palette=page.evaluate("() => window.urbariumLandmarkPalette")
        assert palette.get("park") and palette.get("library") and palette.get("legal")
        assert page.locator("#landmark-legend").is_hidden()
        page.locator("#layer-landmarks").check()
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.dataset.categoryColors === 'true'",
            timeout=8000,
        )
        assert page.locator("#landmark-legend").is_visible()
        assert page.locator("#world-landmarks .world-landmark").count()==9
        page.locator("#layer-landmarks").uncheck()
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.dataset.categoryColors === 'false'",
            timeout=8000,
        )
        print("Landmark category toggle, minmap category flag and legend passed")
        page.locator("#time-of-day").evaluate("""e => {
            e.value="18";e.dispatchEvent(new Event("input",{bubbles:true}));
        }""")
        assert page.locator("#time-label").inner_text()=="18:00"
        assert page.locator("#time-of-day").get_attribute("data-hour")=="18"
        page.locator("#time-of-day").evaluate("""e => {
            e.value="6";e.dispatchEvent(new Event("input",{bubbles:true}));
        }""")
        assert page.locator("#time-label").inner_text()=="06:00"
        print("Procedural sky time-of-day input and Rust-visible hour passed")

        page.wait_for_function(
            "() => document.querySelector('#world-landmarks')?.getAttribute('data-labels-mode') === 'view-frustum'",
            timeout=8000,
        )
        assert page.locator("#world-landmarks").get_attribute("data-labels-paused") is None
        assert "rotate(" not in page.locator(".world-landmark").first.evaluate(
            "(e) => e.style.transform"
        )
        assert int(page.locator("#world-landmarks").get_attribute("data-labels-visible")) >= 0
        print("World labels track the actual camera frustum without movement-based hiding")
        # Compare map frames to the browser's own requestAnimationFrame
        # budget, not wall-clock FPS. SwiftShader CI is a software GPU and
        # sometimes can present only a handful of total page frames.
        page.evaluate("""() => {
            window.__qaRafCount=0;
            window.__qaTrackRaf=()=>{
                window.__qaRafCount++;
                if(window.__qaRafCount<10000)
                    requestAnimationFrame(window.__qaTrackRaf);
            };
            requestAnimationFrame(window.__qaTrackRaf);
        }""")
        start_map_frames=int(minimap.get_attribute("data-map-frame"))
        start_cache_count=int(minimap.get_attribute("data-map-redraws"))
        page.wait_for_timeout(1200)
        updated_map_frames=int(minimap.get_attribute("data-map-frame"))
        updated_cache_count=int(minimap.get_attribute("data-map-redraws"))
        reference_frames=page.evaluate("() => window.__qaRafCount")
        progressed=updated_map_frames-start_map_frames
        assert reference_frames >= 1, "Chromium did not deliver animation frames"
        assert progressed >= reference_frames - 2, (
            "Minimap skipped requestAnimationFrame callbacks: "+
            str(progressed)+" vs "+str(reference_frames)
        )
        assert updated_cache_count - start_cache_count <= 1, (
            "Static GIS data must not be redrawn every camera frame"
        )
        print("Cached minimap delivered",progressed,"frames out of",
              reference_frames,"available browser animation frames, with",
              updated_cache_count-start_cache_count,"GIS cache refreshes")
        page.wait_for_function(
            "() => document.querySelector('#renderer-health')?.textContent.includes('renderer active')",
            timeout=6000,
        )
        assert page.locator("#renderer-health").is_visible()
        print("User-visible 3D frame health counter passed")
        assert page.locator("#layer-walk").get_attribute("data-visible") == "true"
        assert page.locator("#layer-rail").get_attribute("data-visible") == "false"
        page.locator("#layer-rail").check()
        assert page.locator("#layer-rail").get_attribute("data-visible") == "true"
        page.locator("#layer-rail").uncheck()
        assert page.locator("#layer-rail").get_attribute("data-visible") == "false"
        page.locator("#layer-walk").uncheck()
        assert page.locator("#layer-walk").get_attribute("data-visible") == "false"
        page.locator("#layer-walk").check()
        page.locator("#stats-toggle").click()
        assert page.locator("#stats-toggle").get_attribute("aria-pressed") == "true"
        assert page.locator("#stats-panel").is_visible()
        page.wait_for_function(
            "() => Number(document.querySelector('#status')?.getAttribute('data-gpu-mesh-bytes')) > 0",
            timeout=10000,
        )
        page.wait_for_function(
            "() => document.querySelector('#stats-gpu-buffers')?.textContent.includes('MiB')",
            timeout=6000,
        )
        assert float(page.locator("#status").get_attribute("data-triangles-drawn")) > 1000
        assert "N/A" in page.locator("#stats-panel").inner_text()
        print("Real GPU mesh/frame telemetry and toggleable overlays passed")
        before_frames=int(page.locator("#status").get_attribute("data-frame-count"))
        page.wait_for_timeout(6000)
        after_frames=int(page.locator("#status").get_attribute("data-frame-count"))
        assert after_frames > before_frames+8, (
            f"Renderer stopped during soak: {before_frames} -> {after_frames}"
        )
        assert not errors, errors
        print("6-second continuous rendering stability smoke test passed")
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
