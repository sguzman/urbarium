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
    # Reproduce the user's real regression: browser storage contains
    # valid-looking but pre-bridge geography from an older deployment.
    # The new build MUST ignore it and fetch the fingerprinted assets once.
    page.goto("http://127.0.0.1:8010/assets/landmarks.json",
              wait_until="domcontentloaded")
    page.evaluate("""async () => {
        const store=await caches.open("urbarium-geography-downtown-la-v1");
        for(const name of ["downtown-la.json","downtown-la-terrain.json"]){
            await store.put("/assets/"+name,new Response('{"stale":true}',{
                headers:{"content-type":"application/json"}}));
        }
    }""")
    # WebGPU validates the retired default; WebGL2 covers opt-in prototype.
    bridge_preview=force_webgl
    # CI software-Vulkan rendering is an explicit exception, not production default.
    suffix="&bridges=1" if bridge_preview else "&allow-software=1"
    # Exercise the small probe independently of the costly city rendering app.
    probe_page=browser.new_page(accept_downloads=True)
    probe_page.goto("http://127.0.0.1:8010/gpu-test.html",wait_until="domcontentloaded")
    with probe_page.expect_download(timeout=30000) as download_event:
        probe_page.locator("#run-probe").click()
    probe_download=download_event.value
    assert probe_download.suggested_filename.startswith("urbarium-gpu-report-"),probe_download.suggested_filename
    assert probe_download.suggested_filename.endswith(".json")
    import json
    with open(probe_download.path(),encoding="utf-8") as f:
        probe_report=json.load(f)
    assert probe_report["schemaVersion"]==1,probe_report
    assert probe_report["webgl2"]["available"] is True,probe_report
    assert probe_report["webgpu"]["lowPower"]["status"] in (
        "available","null-adapter","error","unsupported"),probe_report
    assert "userAgent" in probe_report and "timestamp" in probe_report
    probe_page.close()
    print("Standalone GPU diagnostic JSON downloaded and parsed")
    page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}", wait_until="domcontentloaded")
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
        # Progressive geometry is now the default; retain the full-scene
        # compatibility path for explicit progressive=0 URLs.
        assert page.evaluate("() => document.documentElement.dataset.progressivePreview")=="true"
        preview_count=int(page.locator("#status").get_attribute("data-first-frame-streets"))
        source_count=int(page.locator("#status").get_attribute("data-source-streets"))
        assert 0<preview_count<source_count,(preview_count,source_count)
        assert float(page.locator("#status").get_attribute(
            "data-first-frame-street-radius-m"))==450.0
        print("Camera-local first frame: source roads",source_count,
              "preview roads",preview_count)
        page.wait_for_function("""() => document.querySelector("#status")?.dataset.firstFrameDetail==="base" """,timeout=60000)
        page.wait_for_function("""() => document.querySelector("#status")?.dataset.sceneDetail==="complete" """,timeout=60000)
        assert page.locator("#chunk-stream-status").count()==1
        if not bridge_preview:
            assert page.locator("#status").get_attribute("data-chunk-streaming")=="true"
            resident=int(page.locator("#status").get_attribute("data-chunks-resident"))
            wanted=int(page.locator("#status").get_attribute("data-chunks-wanted"))
            region_total=int(page.locator("#status").get_attribute("data-chunks-region-total"))
            assert 9<region_total<=96,region_total
            # No distance-based skyline cutoff: the bounded pilot's entire
            # geographic extent must have independently resident meshes.
            assert 0<resident==wanted==region_total<=96,(resident,wanted,region_total)
            assert page.locator("#status").get_attribute("data-chunk-initial-complete")=="true"
            assert int(page.locator("#status").get_attribute("data-chunks-built"))==resident
            print("Complete pilot skyline: all geographic chunks resident",resident)
        else:
            assert page.locator("#status").get_attribute("data-chunk-streaming") is None, (
                "Experimental bridge mode must retain its full-scene fallback")
        assert page.locator("#renderer-select").count() == 1
        assert page.locator("#renderer-health").count() == 1
        # Small, keyboard-accessible tabbed HUD must not restart the app,
        # navigate, or alter the source geography cache.
        tab_names=["explore","graphics","details","advanced"]
        assert page.locator(".hud").bounding_box()["width"]<=355
        assert page.locator(".hud").get_attribute("data-active-tab")=="explore"
        before_hud_url=page.url
        before_hud_frames=int(page.locator("#status").get_attribute("data-frame-count") or 0)
        before_geo_requests=page.evaluate(
            "() => window.__urbariumGeoCache?.networkRequests||0")
        for tab_name in tab_names:
            page.locator("#hud-tab-"+tab_name).click()
            assert page.locator(".hud").get_attribute("data-active-tab")==tab_name
            assert page.locator("#hud-tab-"+tab_name).get_attribute("aria-selected")=="true"
            assert page.locator("#hud-pane-"+tab_name).is_visible()
            assert page.locator(".hud-pane:visible").count()==1
        assert page.url==before_hud_url
        assert page.evaluate("() => window.__urbariumGeoCache?.networkRequests||0")==before_geo_requests
        page.wait_for_function("""(before) => Number(
            document.querySelector("#status")?.dataset.frameCount)>before""",
            arg=before_hud_frames,timeout=15000)
        page.locator("#hud-tab-graphics").click()
        print("Tabbed HUD: 340px cap, exclusive accessible panels, no renderer reload or geography fetch")
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
        # Capture actual adapter and swapchain transfer. Software-only
        # CI adapters may be anonymized; empty diagnostics are a regression.
        gpu_details=page.locator("#status").evaluate("""s => ({
            adapter:s.dataset.gpuAdapter,device:s.dataset.gpuDeviceType,
            format:s.dataset.gpuSurfaceFormat,transfer:s.dataset.gpuColorTransfer
        })""")
        assert all(gpu_details.values()),gpu_details
        assert "Unorm" in gpu_details["format"],gpu_details
        assert ("sRGB attachment" in gpu_details["transfer"]) == (
            "Srgb" in gpu_details["format"]),gpu_details
        print("Graphics device, format and transfer:",gpu_details)
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
        # Live resolution changes must resize color/depth backing without
        # recreating the WASM scene or causing another city download.
        page.locator("#quality-select").select_option("balanced")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status").dataset;
            const c=document.querySelector("canvas:not(#minimap-canvas)");
            const w=Number(s.surfaceWidth),h=Number(s.surfaceHeight);
            return document.documentElement.dataset.renderPixelBudget==="1100000"
                && w*h>0 && w*h<=1110000 && c.width===w && c.height===h;
        }""",timeout=15000)
        page.locator("#quality-select").select_option("high")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status").dataset;
            const c=document.querySelector("canvas:not(#minimap-canvas)");
            const w=Number(s.surfaceWidth),h=Number(s.surfaceHeight);
            return document.documentElement.dataset.renderPixelBudget==="1800000"
                && w*h>1100000 && c.width===w && c.height===h;
        }""",timeout=15000)
        assert page.locator("#quality-select").input_value()=="high"
        print("Live rendering resolution settings verified")
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
        elevated_attr=page.locator("#status").get_attribute("data-elevated-road-spans")
        assert elevated_attr is not None,"Missing source-grade bridge diagnostics"
        elevated=int(elevated_attr)
        assert 0<elevated<=1600,(
            "Bridge source metadata must remain available in either viewing mode",elevated)
        assert page.locator("#status").get_attribute(
            "data-bridge-preview-active")==str(bridge_preview).lower()
        bridge_triangles=int(page.locator("#status").get_attribute(
            "data-bridge-mesh-triangles") or "-1")
        assert (bridge_triangles>0)==bridge_preview,(
            "Retired default must allocate zero experimental bridge triangles",
            bridge_preview,bridge_triangles)
        freeway_triangles=int(page.locator("#status").get_attribute(
            "data-freeway-mesh-triangles") or "-1")
        assert (freeway_triangles>0)==bridge_preview,(
            "Normal view must allocate zero unfinished freeway triangles",
            bridge_preview,freeway_triangles)
        page.locator("#hud-tab-advanced").click()
        toggle=page.locator("#bridge-preview-toggle")
        assert toggle.get_attribute("aria-pressed")==str(bridge_preview).lower()
        assert page.locator("#inspect-overpass").is_visible()==bridge_preview
        if not bridge_preview:
            assert "hidden" in page.locator("#bridge-preview-note").inner_text()
            assert page.locator("#inspect-overpass").is_disabled()
        print("Bridge preview flag/mesh triangle allocation:",bridge_preview,bridge_triangles)
        page.locator("#hud-tab-explore").click()
        eye_height=float(page.locator("#status").get_attribute("data-eye-clearance"))
        assert 1.65 <= eye_height <= 1.75, f"Expected ~1.7 m eye height, got {eye_height}"
        canvas.click(position={"x": 700, "y": 500})
        page.wait_for_function(
            "() => document.pointerLockElement === document.querySelector('canvas:not(#minimap-canvas)')",
            timeout=12000,
        )
        page.wait_for_function(
            "() => document.querySelector('#input-state')?.dataset.pointerLocked === 'true'",
            timeout=10000,
        )
        # The legacy data-camera-yaw diagnostic is published only every
        # eighth frame. With a freshly upgraded progressive GPU mesh, a
        # fixed 300 ms sleep can read the same stale heading twice. Use
        # the camera's real per-frame pose instead, and still require an
        # actual positive heading change from pointer-lock motion.
        def camera_heading():
            return float(page.locator("#status").get_attribute("data-camera-pose").split(",")[3])
        before_yaw=camera_heading()
        page.mouse.move(790,500,steps=5)
        page.wait_for_function("""(before) => {
            const pose=document.querySelector("#status")?.dataset.cameraPose;
            const heading=Number(pose?.split(",")[3]);
            return Number.isFinite(heading) && heading>before+0.03;
        }""",arg=before_yaw,timeout=12000)
        after_yaw=camera_heading()
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
        # Progressive enrichment replaces the initial "ready" text with
        # final mesh statistics. Assert durable rendering state, not a
        # transient informational string.
        assert page.locator("#status").get_attribute("data-render-stage")=="first-present"
        assert page.locator("#status").get_attribute("data-scene-detail")=="complete"
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
        assert page.locator("#named-buildings-panel").count()==1
        named_count=int(page.locator("#named-building-count").inner_text())
        assert 0<=named_count<=obstacles,named_count
        if named_count:
            assert page.locator("#named-buildings-panel").is_visible()
            page.locator("#named-buildings-panel summary").click()
            assert page.locator("#named-building-search").is_visible()
            assert page.locator(".named-building-item").count()>=1
            sample=page.locator(".named-building-item strong").first.inner_text()
            page.locator("#named-building-search").fill(sample)
            assert page.locator(".named-building-item").count()>=1
            assert sample in page.locator(".named-building-item").first.inner_text()
            page.locator("#named-building-search").fill("")
            page.locator("#named-buildings-panel summary").click()
        else:
            assert page.locator("#named-buildings-panel").is_hidden()
        print("Named building directory is conditional on real source names: ",named_count)
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.getAttribute('data-landmark-count') === '9'",
            timeout=15000,
        )
        assert page.locator("#world-landmarks .world-landmark").count() == 9
        assert page.locator("#minimap-canvas").get_attribute("data-legal-count") == "1"
        categorized=page.locator("#world-landmarks .world-landmark").evaluate_all(
            "(els) => Object.fromEntries(els.map(el => [el.textContent.split(' · ')[0],el.dataset.kind]))")
        assert categorized.get("Mosk Courthouse")=="legal",categorized
        assert categorized.get("City Hall")=="civic",categorized
        assert categorized.get("Central Library")=="library",categorized
        assert categorized.get("Grand Park")=="park",categorized
        assert categorized.get("Disney Hall")=="culture",categorized
        assert sum(1 for kind in categorized.values() if kind=="legal")==1,categorized
        assert page.locator("#world-landmarks").is_visible()
        assert page.locator("#status").get_attribute("data-camera-pose") is not None
        print("Nine source-backed landmarks are mapped and anchored to the camera")
        # A repeat fetch must be fulfilled from persistent CacheStorage,
        # without any second HTTP transfer to this flaky-network-sensitive app.
        saved=page.evaluate("""async () => {
            const store=await caches.open("urbarium-geography-downtown-la-v1");
            const versions=window.__urbariumGeoCache.versions;
            const lookup=async filename=>{
                const url=new URL("./assets/"+filename,location.href);
                url.searchParams.set("urbarium_sha256",versions[filename]);
                return Boolean(await store.match(url.href));
            };
            const stale=await store.match("./assets/downtown-la.json");
            const staleTerrain=await store.match("./assets/downtown-la-terrain.json");
            return {city:await lookup("downtown-la.json"),
                    terrain:await lookup("downtown-la-terrain.json"),
                    stale:!!stale,staleTerrain:!!staleTerrain,
                    versions};
        }""")
        assert saved["city"] and saved["terrain"],(
            f"Fingerprint-keyed geography missing on first visit: {saved}")
        assert not saved["stale"] and not saved["staleTerrain"],(
            f"Old URL-only cached geography not evicted: {saved}")
        assert all(len(v)==64 for v in saved["versions"].values()),saved
        assert int(page.locator("#status").get_attribute("data-elevated-road-spans"))>0,(
            "Cached pre-bridge geography was erroneously reused")
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
        print("Versioned CacheStorage ignored stale pre-bridge data and reused SHA-matched assets without HTTP")
        cache_state=page.evaluate("() => window.__urbariumGeoCache")
        assert cache_state["networkRequests"]==2,cache_state
        assert cache_state["cacheHits"]>=2,cache_state
        assert set(cache_state["assets"])=={"City","Terrain"},cache_state
        page.locator("#hud-tab-details").click()
        assert page.locator("#geodata-cache-state").inner_text().count("CACHE HIT")==2
        assert page.locator("#boot-stage").count()==1
        page.wait_for_function(
            "() => document.querySelector('#boot-stage')?.dataset.phase === 'first-present'",
            timeout=8000,
        )
        assert "3D rendered" in page.locator("#boot-stage").inner_text()
        assert page.locator("#geography-request-count").is_visible()
        assert page.locator("#geography-request-count").inner_text().endswith(": 2")
        assert "WASM" in page.locator("#startup-code-state").inner_text()
        assert "first frame at about" in page.locator("#boot-stage").inner_text()
        print("Cache provenance, WASM transfer telemetry, and explicit startup phase visible")
        page.locator("#hud-tab-explore").click()

        palette=page.evaluate("() => window.urbariumLandmarkPalette")
        assert palette.get("park") and palette.get("library") and palette.get("legal")
        assert page.locator("#layer-landmarks").count()==0, (
            "Do not gate category colors behind a separate hidden checkbox")
        legend=page.locator("#landmark-legend")
        assert legend.is_visible(), "Interactive legend must be visible inside minimap"
        assert page.locator(".minimap #landmark-legend").count()==1
        assert legend.get_attribute("data-visible-mask")=="255"
        assert "Legal (1)" in legend.inner_text()
        assert "Culture (2)" in legend.inner_text()
        assert page.locator("#minimap-canvas").get_attribute("data-category-colors")=="true"
        labels=page.locator("#world-landmarks .world-landmark").evaluate_all(
            "(els) => els.map(el => el.textContent)")
        assert "Mosk Courthouse · Legal" in labels,labels
        assert "City Hall · Civic" in labels,labels
        assert page.locator("#world-landmarks .world-landmark").count()==9

        # CSS outline must actually use the landmark-specific color.
        outlines=page.evaluate("""() => {
            const els=[...document.querySelectorAll('.world-landmark')];
            const palette=window.urbariumLandmarkPalette;
            const rgb=h=>[1,3,5].map(i=>parseInt(h.slice(i,i+2),16)).join(', ');
            return els.map(e=>({
                kind:e.dataset.kind,
                expected:'rgb('+rgb(palette[e.dataset.kind])+')',
                actual:getComputedStyle(e).borderTopColor
            }));
        }""")
        assert all(x["expected"]==x["actual"] for x in outlines),outlines
        assert len(set(x["actual"] for x in outlines)) >= 7,outlines

        # Test actual minimap PIXELS, not just canvas metadata. Zoom out
        # until all nine pins are inside the viewport, then inspect their
        # center pixels; the old regression rendered every pin yellow.
        minimap.evaluate("""e=>{
            for(let i=0;i<5;i++)document.getElementById('map-zoom-out').click();
        }""")
        page.wait_for_function(
            "() => Number(document.querySelector('#minimap-canvas')?.dataset.mapWidthM)>=2400",
            timeout=8000,
        )
        page.wait_for_function(
            "() => Number(document.querySelector('#minimap-canvas')?.dataset.paintedLandmarkCount)>=9",
            timeout=8000,
        )
        pin_pixels=page.evaluate("""async () => {
            const canvas=document.querySelector('#minimap-canvas');
            const data=await (await fetch('./assets/landmarks.json')).json();
            const city=JSON.parse(await (await fetch('./assets/downtown-la.json')).text());
            const radius=Math.PI/180*6371008.8;
            const centerX=Number(canvas.dataset.playerX),centerZ=Number(canvas.dataset.playerZ);
            const zoom=Number(canvas.dataset.mapWidthM),ppm=260/zoom;
            const dpr=canvas.width/260;
            const ctx=canvas.getContext('2d'),pixels={};
            for(const lm of data.landmarks){
                const x=(lm.coordinates[0]-city.region.origin[0])*radius*
                    Math.cos(city.region.origin[1]*Math.PI/180);
                const z=(lm.coordinates[1]-city.region.origin[1])*radius;
                const sx=Math.round((130+(x-centerX)*ppm)*dpr);
                const sy=Math.round((130-(z-centerZ)*ppm)*dpr);
                if(sx<5||sy<5||sx>=canvas.width-5||sy>=canvas.height-5)continue;
                const px=ctx.getImageData(sx,sy,1,1).data;
                pixels[lm.id]={
                    kind:lm.kind, actual:Array.from(px).slice(0,3),
                    expected:[1,3,5].map(i=>parseInt(
                        window.urbariumLandmarkPalette[lm.kind].slice(i,i+2),16))
                };
            }
            return pixels;
        }""")
        assert len(pin_pixels)>=7,pin_pixels
        assert all(p["actual"]==p["expected"] for p in pin_pixels.values()),pin_pixels
        print("World billboard outline and actual minimap pin pixels match category palette")

        legal=page.locator('.legend-entry[data-kind="legal"]')
        assert legal.get_attribute("aria-pressed")=="true"
        legal.click()
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.dataset.filterVisibleCount === '8'",
            timeout=8000,
        )
        assert legal.get_attribute("aria-pressed")=="false"
        assert legend.get_attribute("data-visible-mask")=="253"
        assert page.locator('.world-landmark[data-kind="legal"]').is_hidden()
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.dataset.paintedLandmarkCount === '8'",
            timeout=8000,
        )
        assert page.evaluate(
            "() => localStorage.getItem('urbarium-landmark-filter-v3')"
        ) == "253"
        assert page.locator('.world-landmark[data-kind="civic"]').count()==1
        page.locator("#landmarks-hide-all").click()
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.dataset.paintedLandmarkCount === '0'",
            timeout=8000,
        )
        assert legend.get_attribute("data-visible-mask")=="0"
        assert page.locator('.world-landmark:not([hidden])').count()==0
        page.locator("#landmarks-show-all").click()
        page.wait_for_function(
            "() => document.querySelector('#minimap-canvas')?.dataset.paintedLandmarkCount === '9'",
            timeout=8000,
        )
        assert legend.get_attribute("data-visible-mask")=="255"
        assert legal.get_attribute("aria-pressed")=="true"
        print("Interactive per-category filtering, All/None and pin/billboard visibility passed")

        sky=page.locator("#time-of-day")
        assert sky.get_attribute("data-clock-mode")=="live"
        assert page.locator("#time-la-now").get_attribute("aria-pressed")=="true"
        page.wait_for_function(
            "() => document.querySelector('#time-label')?.textContent.endsWith(' LA')",
            timeout=8000)
        def la_clock_error():
            return page.evaluate("""() => {
                const parts=new Intl.DateTimeFormat("en-US",{
                    timeZone:"America/Los_Angeles",hour:"2-digit",
                    minute:"2-digit",second:"2-digit",hourCycle:"h23"
                }).formatToParts(new Date());
                const v=k=>Number(parts.find(p=>p.type===k)?.value||0);
                const target=(v("hour")*3600+v("minute")*60+v("second"));
                const actual=Number(document.querySelector("#time-of-day").dataset.hour)*3600;
                return Math.min(Math.abs(actual-target),
                    86400-Math.abs(actual-target));
            }""")
        assert la_clock_error()<3, "Sky did not default to live Los Angeles civil time"
        before_live=float(sky.get_attribute("data-hour"))
        page.wait_for_function("""(before) => {
            const now=Number(document.querySelector('#time-of-day')?.dataset.hour);
            const progressed=(now-before+24)%24;
            return progressed>0.00020 && progressed<0.004;
        }""",arg=before_live,timeout=8000)
        sky.evaluate("""e => {
            e.value="18";e.dispatchEvent(new Event("input",{bubbles:true}));
        }""")
        assert sky.get_attribute("data-clock-mode")=="manual"
        assert page.locator("#time-la-now").get_attribute("aria-pressed")=="false"
        assert "18:00" in page.locator("#time-label").inner_text()
        assert float(sky.get_attribute("data-hour"))==18
        sky.evaluate("""e => {
            e.value="6";e.dispatchEvent(new Event("input",{bubbles:true}));
        }""")
        assert "06:00" in page.locator("#time-label").inner_text()
        assert float(sky.get_attribute("data-hour"))==6
        page.locator("#time-la-now").click()
        page.wait_for_function(
            "() => document.querySelector('#time-of-day')?.dataset.clockMode === 'live'",
            timeout=8000)
        assert la_clock_error()<3, "Resume LA did not return to live clock"
        print("Live Los Angeles clock, continuously advancing sun, manual override and resume passed")

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
        page.locator("#hud-tab-details").click()
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
        if bridge_preview:
            page.locator("#hud-tab-advanced").click()
            # Inspect a real bridge AFTER camera-motion and pointer-lock
            # baseline checks; teleportation must not change their assumptions.
            assert page.locator("#inspect-overpass").is_enabled()
            assert f"({elevated})" in page.locator("#inspect-overpass").inner_text()
            before_inspection=page.locator("#status").get_attribute("data-camera-pose")
            page.locator("#inspect-overpass").click()
            page.wait_for_function(
                """(old) => document.querySelector('#status')?.dataset.cameraPose !== old""",
                arg=before_inspection, timeout=10000)
            after_inspection=page.locator("#status").get_attribute("data-camera-pose")
            assert after_inspection != before_inspection
            assert page.locator("#status").get_attribute("data-inspect-overpass-request")=="1"
            # A frame counter is not proof that the target is visible. Older
            # builds produced a flat gray screenshot after this jump. Wait for
            # two presentations and inspect pixels in the unobstructed canvas.
            rendered_before=int(page.locator("#status").get_attribute("data-frame-count"))
            page.wait_for_function(
                "(n) => Number(document.querySelector('#status')?.dataset.frameCount) >= n+2",
                arg=rendered_before,timeout=12000)
            page.wait_for_timeout(200)
            with Image.open(BytesIO(canvas.screenshot())) as im:
                im=im.convert("RGB")
                width,height=im.size
                sample_colors=set()
                # Sample the full unobstructed 3D canvas, not a thin
                # vertical strip that can accidentally contain mostly sky
                # or one building facade after UI/viewport changes.
                for iy in range(15,90,5):
                    for ix in range(15,90,5):
                        sample_colors.add(im.getpixel((
                            min(width-1,int(ix*width/100)),
                            min(height-1,int(iy*height/100))
                        )))
            print("Overpass overview distinct unobstructed canvas colors:",len(sample_colors))
            # The actual low-poly city has a small palette. The old broken
            # inspector rendered one uniform color; the corrected screenshot
            # has visibly distinct terrain, streets and building surfaces.
            assert len(sample_colors)>=5,(
                "Overpass inspector produced flat/no-scene rendering",len(sample_colors))
            print("Overpass inspector relocated the 3D camera to visible terrain and road deck")
        # Exercise the diagnostic path without actually killing the graphics
        # process. Verify crash info is persisted locally and conservative
        # GPU mode changes both rendering resolution and URL configuration.
        if force_webgl:
            page.locator("#hud-tab-graphics").click()
            assert page.locator("#safe-gpu-toggle").get_attribute("aria-pressed")=="false"
            page.evaluate("""() => document.dispatchEvent(
                new Event('webglcontextlost',{bubbles:true,cancelable:true})
            )""")
            report=page.evaluate("""() => JSON.parse(
                localStorage.getItem('urbarium-renderer-last-session-v1'))""")
            assert report["gpuDeviceLost"]=="true",report
            assert "WebGL context lost" in report["gpuFault"],report
            page.locator("#safe-gpu-toggle").click()
            page.wait_for_function(
                "() => document.documentElement.dataset.safeGpu === 'true'",
                timeout=10000)
            page.wait_for_function(
                "() => Number(document.querySelector('#status')?.dataset.frameCount)>3",
                timeout=30000)
            dims=page.evaluate("""() => {
                const d=document.querySelector('#status').dataset;
                return [Number(d.surfaceWidth),Number(d.surfaceHeight)];
            }""")
            page.wait_for_function("""() => {
                const d=document.querySelector('#status').dataset;
                const canvas=document.querySelector('canvas:not(#minimap-canvas):not(#webgpu-control)');
                const w=Number(d.surfaceWidth),h=Number(d.surfaceHeight);
                return w*h>0 && w*h<=660000 &&
                    canvas?.width===w && canvas?.height===h;
            }""",timeout=15000)
            dims=page.evaluate("""() => {
                const d=document.querySelector('#status').dataset;
                const c=document.querySelector('canvas:not(#minimap-canvas):not(#webgpu-control)');
                return [Number(d.surfaceWidth),Number(d.surfaceHeight),c.width,c.height];
            }""")
            assert dims[0]*dims[1]<=660000 and dims[:2]==dims[2:],(
                "Conservative mode must cap actual canvas AND depth backing pixels",dims)
            assert "safe=1" in page.url
            print("GPU fault persisted locally; conservative 30fps/640k-pixel restart succeeded",dims)
        # Switch to a high, genuinely navigable city view without replacing
        # source meshes or restarting the browser; restore street-level afterward.
        page.locator("#hud-tab-explore").click()
        page.locator("#camera-overview").click()
        page.wait_for_function("""() => {
            const s=document.querySelector('#status');
            return s?.dataset.cameraMode==='overview' &&
                Number(s.dataset.eyeClearance)>100;
        }""",timeout=15000)
        print("City overview camera switched to elevated view")
        if not bridge_preview:
            # The elevated view must retain the entire bounded city,
            # INCLUDING the blocks beyond the old nine-cell radius.
            # Merely seeing local buildings is an unacceptable regression.
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                const resident=Number(s?.dataset.chunksResident);
                return resident>9 &&
                    resident===Number(s?.dataset.chunksRegionTotal) &&
                    resident===Number(s?.dataset.chunksWanted) &&
                    s?.dataset.sceneDetail==="complete";
            }""",timeout=90000)
            after_stream_move=page.locator("#status")
            assert int(after_stream_move.get_attribute("data-chunks-evicted"))==0, (
                "Relocating the camera within Downtown must not discard visible skyline")
            print("Elevated overview: all distant building chunks retained",
                  after_stream_move.get_attribute("data-chunks-resident"))
        page.locator("#camera-street").click()
        page.wait_for_function("""() => {
            const s=document.querySelector('#status');
            return s?.dataset.cameraMode==='street' &&
                Number(s.dataset.eyeClearance)<2;
        }""",timeout=15000)
        # Hiding UI must not pause drawing, and the restore button must work.
        const_before=page.locator("#status").get_attribute("data-frame-count")
        page.locator("#scene-ui-toggle").click()
        assert page.locator("#scene-ui-toggle").inner_text()=="Show panels"
        assert not page.locator(".hud").is_visible()
        page.wait_for_function("""(before) => Number(
            document.querySelector('#status')?.dataset.frameCount)>Number(before)
        """,arg=const_before,timeout=15000)
        page.locator("#scene-ui-toggle").click()
        assert page.locator(".hud").is_visible()
        print("City overview and HUD-free view keep renderer running")
        # The real 3D renderer must download JSON diagnostics, not just copy
        # them to the clipboard. This includes the backend, frame counters,
        # crash telemetry and sampled presented frame rate.
        page.wait_for_function(
            "() => Number(document.querySelector('#status')?.dataset.frameCount||0)>2",
            timeout=15000)
        page.locator("#hud-tab-details").click()
        with page.expect_download(timeout=15000) as render_event:
            page.locator("#download-render-report").click()
        render_download=render_event.value
        assert render_download.suggested_filename.startswith("urbarium-render-report-")
        with open(render_download.path(),encoding="utf-8") as f:
            renderer_report=json.load(f)
        assert renderer_report["schemaVersion"]==1,renderer_report
        assert renderer_report["current"]["backend"] in ("Gl","BrowserWebGpu"),renderer_report
        assert renderer_report["current"]["frames"]>2,renderer_report
        assert renderer_report["current"]["gpuDeviceType"],renderer_report
        assert isinstance(renderer_report["current"]["fpsSamples"],list),renderer_report
        startup=renderer_report["current"]["startup"]
        assert startup["pageUptimeMs"]>0,startup
        assert startup["wasmResource"]["finished"] is True,startup
        assert any(phase["phase"]=="first-present" for phase in startup["phases"]),startup
        print("Actual 3D rendering report downloaded and parsed")
        if not force_webgl:
            # Software WebGPU must not become the real production renderer.
            # The SwiftShader-only test browser should fall back to WebGL2.
            page.goto("http://127.0.0.1:8010/?renderer=webgpu",wait_until="domcontentloaded")
            page.wait_for_function("""() => location.search.includes("renderer=webgl2") &&
                document.querySelector("#gpu-fallback-message")?.hidden===false &&
                document.querySelector("#status")?.dataset.gpuBackend==="Gl" """,timeout=60000)
            print("WebGPU CPU adapter automatically fell back to hardware WebGL2 mode")
        # A second independently packaged section is a real Overture city,
        # not a fake 2D preview. Exercise its own city/DEM/3D camera.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}&region=little-tokyo-east",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector('#status');
            return s?.dataset.renderStage==='first-present' &&
                s.dataset.regionId==='little-tokyo-east' &&
                Number(s.dataset.frameCount)>2;
        }""",timeout=60000)
        assert page.locator("#region-select").input_value()=="little-tokyo-east"
        assert page.locator("#status").get_attribute("data-gpu-backend")==selected_backend
        assert page.locator("#minimap-canvas").get_attribute("data-map-ready")=="true"
        print("Little Tokyo east section: independent real 3D + terrain + minimap passed")
        # Real neighboring geometry loaded into a separate GPU buffer under a
        # strict residency cap, with camera crossing the geographic seam.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}&world=1&progressive=0",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector('#status');
            return s?.dataset.renderStage==='first-present' &&
                Number(s.dataset.worldSections)===2 &&
                Number(s.dataset.worldSecondaryDrawcalls)>0 &&
                Number(s.dataset.frameCount)>3;
        }""",timeout=90000)
        assert page.locator("#world-preview-toggle").get_attribute("aria-pressed")=="true"
        assert page.locator("#progressive-preview-toggle").get_attribute("aria-pressed")=="false"
        assert page.locator("#status").get_attribute("data-first-frame-detail")=="joined"
        # Joined-world startup previously displayed the misleading generic
        # "Starting application" for the neighbor-fetch stage. Ensure the
        # real phase has a distinct title even after its work is complete.
        assert page.evaluate("""() => document.querySelector('#status')?.dataset.worldSections""")=="2"
        # Regression: the old joined-world mover compared altitude against
        # a fake 0-m ground level and climbed by terrain height EVERY FRAME.
        # Confirm real keyboard flight controls preserve absolute altitude.
        page.locator("#hud-tab-explore").click()
        page.locator("#camera-overview").click()
        page.wait_for_function("""() => document.querySelector('#status')?.dataset.cameraMode==='overview'""")
        # Winit receives keyboard movement only when its canvas has focus.
        # A click on City overview leaves browser focus on the HTML button.
        page.locator("canvas:not(#minimap-canvas):not(#webgpu-control)").click(
            position={"x":700,"y":500})
        page.wait_for_function("""() => document.pointerLockElement ===
            document.querySelector('canvas:not(#minimap-canvas):not(#webgpu-control)')""",
            timeout=12000)
        def joined_pose():
            return [float(v) for v in page.locator("#status").get_attribute("data-camera-pose").split(",")]
        from time import sleep
        start=joined_pose()
        page.keyboard.down("w")
        sleep(1.0)
        page.keyboard.up("w")
        mid=joined_pose()
        assert abs(mid[1]-start[1])<0.2,("Forward flight must hold altitude",start,mid)
        assert abs(mid[0]-start[0])+abs(mid[2]-start[2])>0.05,("Forward input ignored",start,mid)
        page.keyboard.down("s")
        sleep(1.0)
        page.keyboard.up("s")
        back=joined_pose()
        assert abs(back[1]-start[1])<0.2,("Backward flight must hold altitude",start,back)
        page.keyboard.down("e")
        sleep(0.4)
        page.keyboard.up("e")
        up=joined_pose()
        assert up[1]>back[1]+0.2,("E must raise the camera",back,up)
        page.keyboard.down("q")
        sleep(0.4)
        page.keyboard.up("q")
        down=joined_pose()
        assert down[1]<up[1]-0.2,("Q must lower the camera",up,down)
        print("Joined world WASD height stability and explicit E/Q altitude controls passed",start,mid,back,up,down)
        assert not errors,errors
        print("Joined world preview: 2 independent GPU buffers and visible neighbor draw calls")
        # Progressive first frame must be actual terrain + street polygons
        # before buildings and the neighboring city are ready, then upgrade
        # both GPU meshes without navigation or resetting the camera.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}&world=1",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.renderStage==="first-present" &&
                s.dataset.firstFrameDetail==="base" &&
                Number(s.dataset.frameCount)>1;
        }""",timeout=90000)
        assert page.locator("#progressive-preview-toggle").get_attribute("aria-pressed")=="true"
        assert "3D rendered" in page.locator("#boot-stage").inner_text()
        early_url=page.url
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.sceneDetail==="joined" &&
                Number(s.dataset.worldSections)===2 &&
                Number(s.dataset.worldSecondaryDrawcalls)>0 &&
                Number(s.dataset.frameCount)>6;
        }""",timeout=90000)
        assert page.url==early_url
        assert not errors,errors
        print("Progressive scene: first terrain/streets frame before full city and neighboring tile, then joined-world upgrade passed")
        if force_webgl:
            # Separately test the normal WebGL2 renderer, without the
            # experimental freeway override that deliberately bypasses
            # primary mesh streaming.
            page.goto("http://127.0.0.1:8010/?renderer=webgl2",
                      wait_until="domcontentloaded")
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return s?.dataset.sceneDetail==="complete" &&
                    s.dataset.chunkStreaming==="true" &&
                    Number(s.dataset.chunksResident)>9 &&
                    Number(s.dataset.chunksResident)===Number(s.dataset.chunksRegionTotal);
            }""",timeout=90000)
            page.locator("#camera-overview").click()
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return s?.dataset.cameraMode==="overview" &&
                    s?.dataset.sceneDetail==="complete" &&
                    Number(s.dataset.chunksResident)>9 &&
                    Number(s.dataset.chunksResident)===Number(s.dataset.chunksRegionTotal);
            }""",timeout=90000)
            assert int(page.locator("#status").get_attribute("data-chunks-evicted"))==0
            assert not errors,errors
            print("WebGL2 skyline: complete primary-city chunk set persists across overview")

        # SW must cache the exact fingerprinted WASM, then serve the same
        # build from CacheStorage on a subsequent navigation. It may not
        # intercept the first page before clients.claim(), so the first
        # controlled reload primes it when necessary.
        page.wait_for_function("""() => !!navigator.serviceWorker?.controller &&
            window.__urbariumWasmCache?.registration==="registered" """,timeout=25000)
        cache_url=f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}&progressive=0"
        page.goto(cache_url,wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.renderStage==="first-present" &&
                window.__urbariumWasmCache?.lastEvent!=="none";
        }""",timeout=90000)
        page.wait_for_function("""() => window.__urbariumWasmCache?.stores>0 ||
            window.__urbariumWasmCache?.hits>0""",timeout=25000)
        page.reload(wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.renderStage==="first-present" &&
                window.__urbariumWasmCache?.hits>0;
        }""",timeout=90000)
        assert page.locator("#wasm-cache-state").count()==1
        page.locator("#hud-tab-details").click()
        page.wait_for_function("""() =>
            document.querySelector("#wasm-cache-state")?.textContent.includes("CACHE HIT (disk)")""",
            timeout=10000)
        assert not errors,errors
        print("WASM service worker: exact hashed module served from persistent disk cache on reload")
        assert page.locator('link[rel="preload"][type="application/wasm"]').count()==0
        # Critical cold-start regression: a fresh browser profile has no
        # service worker when the *first* WASM request starts. That very
        # first response must populate CacheStorage. One reload must then
        # receive a real disk cache hit, without requiring a third visit.
        isolated=browser.new_context(viewport={"width":1280,"height":800})
        try:
            first=isolated.new_page()
            if force_webgl:
                first.add_init_script("""
                    Object.defineProperty(navigator, 'gpu', {
                        configurable:true,value:undefined
                    });
                """)
            first.goto(cache_url,wait_until="domcontentloaded")
            # Inspect CacheStorage itself. A cache write may come either
            # from the first-page fetch wrapper or from the service worker
            # if it happened to take control early; message delivery is
            # not the authoritative measure of whether the bytes are saved.
            first.wait_for_function("""async () => {
                if(document.querySelector("#status")?.dataset.renderStage!=="first-present")
                    return false;
                const bucket=await caches.open("urbarium-wasm-binaries-v1");
                return (await bucket.keys()).some(req=>
                    /^urbarium-viewer-[0-9a-f]+_bg\\.wasm$/.test(
                        new URL(req.url).pathname.split("/").pop()));
            }""",timeout=90000)
            first.wait_for_function("""() => !!navigator.serviceWorker.controller """,
                                    timeout=30000)
            stored=first.evaluate("""async () => {
                const cache=await caches.open("urbarium-wasm-binaries-v1");
                return (await cache.keys()).map(req=>new URL(req.url).pathname);
            }""")
            assert len(stored)==1 and stored[0].endswith("_bg.wasm"),stored
            first.reload(wait_until="domcontentloaded")
            first.wait_for_function("""() =>
                document.querySelector("#status")?.dataset.renderStage==="first-present" &&
                window.__urbariumWasmCache?.hits>0
            """,timeout=90000)
            assert first.evaluate("() => window.__urbariumWasmCache?.lastEvent")=="hit"
            print("Cold browser: first WASM fetch cached, immediate next reload served from disk")
        finally:
            isolated.close()
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
