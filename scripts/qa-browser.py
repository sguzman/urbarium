#!/usr/bin/env python3
"""Headless functional smoke check for the actual Rust/wgpu WebAssembly app."""
import os
import time
import json
from pathlib import Path
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
    page.on("crash", lambda _: errors.append("Chromium tab crashed during viewer QA"))
    page.on("console", lambda m: print("CONSOLE", m.type, m.text))
    page.on("requestfailed", lambda r: print("REQUEST FAILED:", r.url, r.failure))
    page.on("response", lambda r: print("HTTP ERROR:", r.status, r.url)
            if r.status >= 400 else None)
    qa_mode = "webgl2" if force_webgl else "webgpu"
    # Regression scenario: browser storage contains
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
    # world=0 preserves the formerly default single-region regression suite.
    suffix=("&bridges=1" if bridge_preview else "&allow-software=1")+"&world=0"
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
        # The status text is a progress log and can legitimately change
        # after the first frame (for example to building triangle counts).
        # Require real rendered frames, not a transient "ready" phrase.
        page.wait_for_function("""() => {
            const e=document.querySelector('#status');
            return e && ((e.dataset.renderStage==='first-present' &&
                Number(e.dataset.frameCount)>0) ||
                e.textContent.includes('Viewer error'));
        }""",timeout=60000)
        assert page.locator("#status").get_attribute(
            "data-render-stage")=="first-present",page.locator("#status").inner_text()
        # Regression: the temporary JS city image uses a higher z-index than
        # the real GPU canvas. It must be gone, NOT just pointer-transparent,
        # once GPU frames are being presented or the entire scene looks frozen
        # while CSS menus and landmark labels continue to work.
        page.wait_for_function("""() => {
            const status=document.querySelector("#status");
            const cover=document.querySelector("#startup-preview");
            return status?.dataset.renderStage==="first-present" &&
                Number(status.dataset.frameCount)>0 &&
                (!cover || getComputedStyle(cover).display==="none");
        }""",timeout=8000)
        assert page.locator("#startup-preview").count()==0, (
            "Pre-WASM sketch still overlays the live 3D renderer")
        print("GPU first frame has no static street-image overlay")
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
        if not bridge_preview:
            # Pixel-for-pixel A/B at the identical camera pose. No source,
            # camera, buffer residency or meshing changes are permitted;
            # toggle only the draw-call classifier while still rendering.
            from io import BytesIO as _FrustumBytes
            from PIL import Image as _FrustumImage, ImageChops as _FrustumDiff
            def assert_frustum_visual_parity(label):
                target=page.locator("#status")
                canvas=page.locator("canvas:not(#minimap-canvas):not(#webgpu-control)").first
                resident=int(target.get_attribute("data-chunks-resident"))
                assert resident==int(target.get_attribute("data-chunks-region-total"))
                page.evaluate("document.documentElement.dataset.frustumCulling='false'")
                page.wait_for_function("""() => {
                    const e=document.querySelector('#status');
                    return Number(e?.dataset.chunksDrawn)===Number(e?.dataset.chunksResident)
                        && Number(e?.dataset.chunksCulled)===0;
                }""",timeout=12000)
                page.wait_for_timeout(150)
                unfiltered=canvas.screenshot()
                page.evaluate("document.documentElement.dataset.frustumCulling='true'")
                page.wait_for_function("""() => {
                    const e=document.querySelector('#status');
                    return Number(e?.dataset.chunksDrawn)+Number(e?.dataset.chunksCulled)
                           ===Number(e?.dataset.chunksResident);
                }""",timeout=12000)
                page.wait_for_timeout(150)
                filtered=canvas.screenshot()
                with _FrustumImage.open(_FrustumBytes(unfiltered)) as image0, \
                    _FrustumImage.open(_FrustumBytes(filtered)) as image1:
                    delta=_FrustumDiff.difference(image0.convert("RGB"),image1.convert("RGB"))
                    # Ignore slight temporal/driver dithering, but fail
                    # conspicuous missing building/road chunks.
                    changed=delta.convert("L").point(
                        lambda v:255 if v>24 else 0).histogram()[255]
                    fraction=changed/(delta.width*delta.height)
                    assert fraction<0.005,(label,fraction,"missing visible geometry")
                assert int(target.get_attribute("data-chunks-resident"))==resident
                print("Frustum visual parity",label,"changed pixel fraction",round(fraction,6),
                      "GPU chunks kept",resident,"culled",
                      target.get_attribute("data-chunks-culled"))
            assert_frustum_visual_parity("street")
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
        # A speed adjustment is live and must not reinitialize graphics,
        # navigate, or change the ordinary walking velocity.
        old_page_url=page.url
        page.locator("#sprint-speed").evaluate("""el => {
            el.value="180";
            el.dispatchEvent(new Event("input",{bubbles:true}));
        }""")
        page.wait_for_function("""() => {
            const status=document.querySelector("#status");
            return Math.abs(Number(status?.dataset.speedMps)-180)<0.01 &&
                document.querySelector("#sprint-speed-value")?.textContent==="180 m/s" &&
                document.querySelector("#speed-state")?.textContent==="FAST · 180 m/s";
        }""",timeout=8000)
        assert page.url==old_page_url,"Speed slider caused navigation"
        assert page.evaluate("""() =>
            localStorage.getItem("urbarium-fast-travel-mps-v1")""")=="180"
        page.keyboard.press("Shift")
        page.wait_for_function(
            "() => document.querySelector('#status')?.getAttribute('data-speed-mode') === 'walk'",
            timeout=6000,
        )
        assert abs(float(page.locator("#status").get_attribute("data-speed-mps"))-3.4)<0.01
        assert page.locator("#sprint-speed-value").inner_text()=="180 m/s"
        print("Shift toggles fast travel; live slider updates 180 m/s without reload")
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
        # Either persistent disk hits or recent in-memory response sharing
        # avoids the network. A quick browser can reuse both requests before
        # the 10-second in-flight coalescing window expires.
        assert (cache_state["cacheHits"]+
                cache_state["inFlightReuses"])>=2,cache_state
        assert set(cache_state["assets"])=={"City","Terrain"},cache_state
        page.locator("#hud-tab-details").click()
        cache_labels=page.locator("#geodata-cache-state").inner_text()
        assert cache_labels.count("CACHE HIT")+cache_labels.count("SHARED RESPONSE")==2,cache_labels
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
            // Use a comparable 2.3km scale. The joined-world update allows
            // zooming out to 6km, where nearby category pins overlap
            // on the same raster pixels and exact center-color tests
            // are not meaningful.
            for(let i=0;i<3;i++)document.getElementById('map-zoom-out').click();
        }""")
        page.wait_for_function(
            "() => Number(document.querySelector('#minimap-canvas')?.dataset.mapWidthM)>=2300",
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
            assert_frustum_visual_parity("elevated overview")
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
        # Exercise EVERY catalog region automatically on both backends.
        # Geography expansions must not require one-off human screenshots
        # or a manually curated list of region-specific smoke checks.
        catalog=json.loads((Path(__file__).resolve().parents[1]/
            "regions/catalog.json").read_text(encoding="utf-8"))
        region_ids=[r["id"] for r in catalog["regions"]]
        for rid in region_ids:
            normal_suffix=suffix.replace("&bridges=1","")
            page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{normal_suffix}&region={rid}&world=0&detailcells=6",
                      wait_until="domcontentloaded")
            page.wait_for_function("""id => {
                const s=document.querySelector("#status");
                return s?.dataset.regionId===id &&
                    s.dataset.renderStage==="first-present" &&
                    Number(s.dataset.frameCount)>3 &&
                    Number(s.dataset.worldSections)===1;
            }""",arg=rid,timeout=60000)
            assert page.locator("#region-select").input_value()==rid
            assert page.locator("#status").get_attribute("data-gpu-backend")==selected_backend
            page.wait_for_function("""() => Number(
                document.querySelector("#status")?.dataset.chunksBuilt||0)>=3
            """,timeout=60000)
            assert int(page.locator("#status").get_attribute("data-chunks-resident"))<=6
            assert int(page.locator("#status").get_attribute("data-far-massing-bytes"))>0
            assert page.locator("#minimap-canvas").get_attribute("data-map-ready")=="true"
            print("Catalog section QA passed:",rid,"source 3D, terrain, skyline, six-cell budget")
        # Joined world must likewise work from the westernmost pilot, not
        # merely when launching the original Downtown primary source.
        joined_suffix=suffix.replace("&bridges=1","").replace("&world=0","")
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{joined_suffix}&region=i110-westlake&detailcells=6",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            const expected=window.__urbariumRegionCatalog?.regions.length;
            return s?.dataset.regionId==="i110-westlake" &&
                s.dataset.renderStage==="first-present" &&
                Number(s.dataset.frameCount)>3 &&
                Number(s.dataset.worldSections)===expected;
        }""",timeout=90000)
        assert page.locator("#world-preview-toggle").get_attribute("aria-pressed")=="true"
        assert page.locator("#world-preview-toggle").is_enabled()
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            const map=document.querySelector("#minimap-canvas");
            const expected=window.__urbariumRegionCatalog?.regions.length;
            return Number(map?.dataset.mapSections)===expected &&
                Number(s?.dataset.worldDetailedResident)<=6 &&
                Number(s?.dataset.worldSecondaryDrawcalls)>0;
        }""",timeout=30000)
        print("Catalog joined-world QA passed with one shared six-cell budget")
        # A real exploration regression: exercise fast movement for several
        # seconds while every region is resident, and reject device loss,
        # tab crashes, runaway mesh residency and stalled presentations.
        # This runs on both WebGPU and WebGL2 and never changes startup code.
        page.mouse.click(650,450)
        page.wait_for_function(
            "() => document.pointerLockElement !== null",timeout=12000)
        page.locator("#sprint-speed").evaluate("""el=>{
            el.value="180";el.dispatchEvent(new Event("input",{bubbles:true}));
        }""")
        page.keyboard.press("Shift")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.speedMode==="fast" &&
                Math.abs(Number(s.dataset.speedMps)-180)<0.01;
        }""",timeout=10000)
        page.keyboard.down("e")
        page.wait_for_timeout(1000)
        page.keyboard.up("e")
        last_frames=int(page.locator("#status").get_attribute("data-frame-count"))
        for key in ("w","d","s"):
            page.keyboard.down(key)
            try:
                page.wait_for_timeout(4500)
            finally:
                page.keyboard.up(key)
            sample=page.evaluate("""() => {
                const s=document.querySelector("#status");
                return {frames:Number(s.dataset.frameCount),
                    gpuLost:s.dataset.gpuDeviceLost,
                    regions:Number(s.dataset.worldSections),
                    detailed:Number(s.dataset.worldDetailedResident),
                    gpuBytes:Number(s.dataset.gpuMeshBytes)};
            }""")
            assert sample["frames"]>last_frames+2,("Rendering stalled during fast travel",key,sample)
            assert sample["gpuLost"]=="false",("GPU lost during fast travel",key,sample)
            assert sample["regions"]==len(region_ids),("Joined section disappeared",key,sample)
            assert 0<=sample["detailed"]<=6,("Detailed GPU cell budget exceeded",key,sample)
            assert 0<=sample["gpuBytes"]<256*1024*1024,("GPU mesh memory exceeded stress budget",key,sample)
            assert not errors,("Browser error during fast travel",key,errors)
            last_frames=sample["frames"]
        page.keyboard.press("Shift")
        print("Joined-world sustained 180 m/s flight passed on",qa_mode,
              "with resident cells <=6 and no GPU/browser crash")
        # Real neighboring geometry loaded into a separate GPU buffer under a
        # strict residency cap, with camera crossing the geographic seam.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix.replace('&world=0','')}&progressive=0",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector('#status');
            return s?.dataset.renderStage==='first-present' &&
                Number(s.dataset.worldSections)===(document.documentElement.dataset.bridgePreview==="true"?2:window.__urbariumRegionCatalog.regions.length) &&
                Number(s.dataset.worldSecondaryDrawcalls)>0 &&
                Number(s.dataset.frameCount)>3;
        }""",timeout=90000)
        assert page.locator("#world-preview-toggle").get_attribute("aria-pressed")=="true"
        # Normal joined world must refuse the accidental blocking-startup
        # setting. Diagnostic experimental bridges preserve their fallback.
        assert page.locator("#progressive-preview-toggle").get_attribute("aria-pressed")==(
            "false" if bridge_preview else "true")
        page.wait_for_function("""() => {
            const map=document.querySelector("#minimap-canvas");
            return map?.dataset.mapReady==="true" &&
                map.dataset.mapJoined==="true" &&
                Number(map.dataset.mapSections)===(document.documentElement.dataset.bridgePreview==="true"?2:window.__urbariumRegionCatalog.regions.length) &&
                Number(map.dataset.mapFullWidthM)>2700;
        }""",timeout=30000)
        map_info=page.locator("#minimap-canvas")
        fit=float(map_info.get_attribute("data-map-full-width-m"))
        assert float(map_info.get_attribute("data-map-width-m"))>=fit*.97
        bounds=[float(v) for v in map_info.get_attribute(
            "data-map-world-bounds").split(",")]
        assert bounds[2]-bounds[0]>2700 and bounds[3]>bounds[1],bounds
        print("Both joined regions included in geographic minimap",bounds)
        # Pixel-level check in the EASTERN source section, not metadata alone.
        # Missing neighbor geography used to leave background blue here.
        painted=page.evaluate("""() => {
            const map=document.querySelector("#minimap-canvas");
            const b=map.dataset.mapWorldBounds.split(",").map(Number);
            const width=Number(map.dataset.mapWidthM);
            const x=b[0]+.83*(b[2]-b[0]),z=b[1]+.50*(b[3]-b[1]);
            const centerX=(b[0]+b[2])/2,centerZ=(b[1]+b[3])/2;
            const pixelX=130+(x-centerX)*260/width;
            const pixelY=130-(z-centerZ)*260/width;
            const sx=map.width/260,sy=map.height/260;
            return Array.from(map.getContext("2d").getImageData(
                Math.round(pixelX*sx),Math.round(pixelY*sy),1,1).data);
        }""")
        assert painted[:3] != [16,29,39],(
            "Eastern region is still unpainted on the minimap",painted)
        print("Joined minimap eastern geography pixel painted",painted)

        assert page.locator("#status").get_attribute("data-first-frame-detail")==(
            "joined" if bridge_preview else "base")
        # Joined-world startup previously displayed the misleading generic
        # "Starting application" for the neighbor-fetch stage. Ensure the
        # real phase has a distinct title even after its work is complete.
        assert page.evaluate("""() => document.querySelector('#status')?.dataset.worldSections""")==(
            "2" if bridge_preview else str(len(region_ids)))
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
        # Software WebGL2 can go hundreds of milliseconds without a present
        # under CI load. A fixed 400ms keypress can therefore test *zero*
        # simulation frames and misdiagnose the camera as broken. Keep the key
        # held until the required movement is actually observed, with a firm
        # timeout and guaranteed release. The movement requirement is unchanged.
        def assert_vertical_move(key,baseline,direction):
            page.keyboard.down(key)
            try:
                page.wait_for_function("""([base,sign]) => {
                    const pose=document.querySelector("#status")?.dataset.cameraPose;
                    if (!pose) return false;
                    const altitude=Number(pose.split(",")[1]);
                    return Number.isFinite(altitude) && sign*(altitude-base)>0.2;
                }""",arg=[baseline,direction],timeout=20000)
            finally:
                page.keyboard.up(key)
            return joined_pose()
        up=assert_vertical_move("e",back[1],1)
        assert up[1]>back[1]+0.2,("E must raise the camera",back,up)
        down=assert_vertical_move("q",up[1],-1)
        assert down[1]<up[1]-0.2,("Q must lower the camera",up,down)
        print("Joined world WASD height stability and explicit E/Q altitude controls passed",start,mid,back,up,down)
        assert not errors,errors
        print("Joined world preview: 2 independent GPU buffers and visible neighbor draw calls")
        # Progressive first frame must be actual terrain + street polygons
        # before buildings and the neighboring city are ready, then upgrade
        # both GPU meshes without navigation or resetting the camera.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix.replace('&world=0','')}",
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
                Number(s.dataset.worldSections)===(document.documentElement.dataset.bridgePreview==="true"?2:window.__urbariumRegionCatalog.regions.length) &&
                Number(s.dataset.worldSecondaryDrawcalls)>0 &&
                Number(s.dataset.frameCount)>6;
        }""",timeout=90000)
        assert page.url==early_url
        assert page.evaluate("() => document.documentElement.dataset.worldPreview")=="true"
        page.wait_for_function("""() =>
            Number(document.querySelector("#minimap-canvas")?.dataset.mapSections)===(document.documentElement.dataset.bridgePreview==="true"?2:window.__urbariumRegionCatalog.regions.length)
        """,timeout=30000)
        print("Default joined mode: progressive renderer and whole-region map")
        if not bridge_preview:
            assert page.locator("#status").get_attribute("data-chunk-streaming")=="true"
            page.locator("#hud-tab-advanced").click()
            assert page.locator("#progressive-preview-toggle").is_disabled()
            detail=page.locator("#detail-cell-limit")
            assert detail.is_enabled(), "detail limits must be available in the default joined city"
            detail.select_option("6")
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return Number(s?.dataset.detailChunkLimit)===6 &&
                    Number(s?.dataset.chunksResident)===6 &&
                    Number(s?.dataset.farMassingCells)>0 &&
                    Number(s?.dataset.worldSections)===window.__urbariumRegionCatalog.regions.length &&
                    Number(s?.dataset.worldSecondaryDrawcalls)>0 &&
                    s?.dataset.gpuDeviceLost!=="true";
            }""",timeout=30000)
            assert "Shared detailed-cell budget" in page.locator(
                "#detail-cell-note").inner_text()
            total=int(page.locator("#status").get_attribute("data-world-detailed-resident"))
            assert total<=6,("Joined world exceeded global detail budget",total)
            print("Joined-world live detailed-cell cap leaves source-backed distant massing")
        # Exercise sustained joined-region rendering, not only first frame.
        initial_frames=int(page.locator("#status").get_attribute("data-frame-count"))
        for camera_button in ("#camera-overview","#camera-street","#camera-overview"):
            page.locator("#hud-tab-explore").click()
            page.locator(camera_button).click()
            page.wait_for_timeout(1000)
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return s?.dataset.sceneDetail==="joined" &&
                    Number(s.dataset.worldSections)===(document.documentElement.dataset.bridgePreview==="true"?2:window.__urbariumRegionCatalog.regions.length) &&
                    Number(s.dataset.worldSecondaryDrawcalls)>0 &&
                    s.dataset.gpuDeviceLost!=="true" &&
                    s.dataset.renderStage==="first-present";
            }""",timeout=10000)
        page.wait_for_timeout(6000)
        final_frames=int(page.locator("#status").get_attribute("data-frame-count"))
        assert final_frames>initial_frames+10,(initial_frames,final_frames)
        assert page.locator("#status").get_attribute("data-gpu-device-lost")=="false"
        assert not errors,errors
        print("Sustained joined-world GPU context and camera stability",
              initial_frames,final_frames)
        assert not errors,errors
        print("Progressive scene: first terrain/streets frame before full city and neighboring tile, then joined-world upgrade passed")
        if force_webgl:
            # Separately test the normal WebGL2 renderer, without the
            # experimental freeway override that deliberately bypasses
            # primary mesh streaming.
            page.goto("http://127.0.0.1:8010/?renderer=webgl2&world=0",
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
            # Live low-memory experiment: the GPU must evict detailed geometry
            # but keep visible, compact building masses for the missing cells.
            page.locator("#hud-tab-advanced").click()
            detail=page.locator("#detail-cell-limit")
            assert detail.is_enabled()
            detail.select_option("6")
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return Number(s?.dataset.detailChunkLimit)===6 &&
                    Number(s?.dataset.chunksResident)===6 &&
                    Number(s?.dataset.farMassingCells)>0 &&
                    Number(s?.dataset.farMassingBytes)>0 &&
                    s?.dataset.gpuDeviceLost!=="true";
            }""",timeout=25000)
            assert page.locator("#detail-cell-note").is_visible()
            assert "building silhouettes" in page.locator("#detail-cell-note").inner_text()
            page.locator("#hud-tab-explore").click()
            page.wait_for_timeout(600)
            assert int(page.locator("#status").get_attribute("data-frame-count"))>0
            print("Live 6-cell detail budget retains distant source-backed skyline",
                  page.locator("#status").get_attribute("data-far-massing-cells"))

        # Native preload and direct HTTP must not be replaced by a custom
        # fetch wrapper or a cache-first service worker.
        assert page.locator('link[rel="preload"][type="application/wasm"]').count()==1
        assert page.evaluate("() => window.__urbariumWasmCache?.source")=="native-http"
        assert page.locator("#wasm-cache-state").count()==1
        assert not errors,errors
        print("Fingerprint-addressed WASM uses native HTTP preload/cache")

        # Independent Rust CPU mesher worker integration (opt-in until proven
        # safe). Same source City and DEM; all 30 chunks must come from the
        # worker, never silently from the fallback, and the scene must render.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}&world=0&meshworker=1",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() =>
            document.querySelector("#status")?.dataset.renderStage==="first-present"
        """,timeout=90000)
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.sceneDetail==="complete" &&
                s.dataset.meshWorkerState==="ready" &&
                Number(s.dataset.meshWorkerChunks)===Number(s.dataset.chunksRegionTotal) &&
                Number(s.dataset.chunksResident)===Number(s.dataset.chunksRegionTotal) &&
                Number(s.dataset.chunksResident)>9;
        }""",timeout=180000)
        worker_status=page.locator("#status")
        worker_count=int(worker_status.get_attribute("data-mesh-worker-chunks"))
        assert worker_count==int(worker_status.get_attribute("data-chunks-region-total"))
        assert worker_status.get_attribute("data-chunks-evicted")=="0"
        assert worker_status.get_attribute("data-mesh-worker-format")=="UBM1-transfer"
        assert int(worker_status.get_attribute("data-mesh-worker-messages"))==worker_count
        transferred=int(float(worker_status.get_attribute("data-mesh-worker-bytes")))
        assert transferred>worker_count*24,("No binary vertex packets received",transferred)
        print("Binary worker transfer: version UBM1; packets",worker_count,
              "bytes",transferred)
        before_frames=int(worker_status.get_attribute("data-frame-count") or "0")
        # The frame-count DOM diagnostic updates every eighth frame, and
        # software WebGL2 can stall temporarily during screenshot/readback.
        # Wait for the actual counter to advance rather than sampling it
        # after one arbitrary 350 ms sleep.
        page.wait_for_function("""(before) =>
            Number(document.querySelector("#status")?.dataset.frameCount)>before
        """,arg=before_frames,timeout=15000)
        assert not errors,errors
        print("CPU WASM worker: built every primary-region chunk off main render thread",
              worker_count,"GPU chunks resident")

        # First opt-in visit must save the exact content-hashed worker WASM;
        # the next visit must load that binary from persistent disk cache.
        page.wait_for_function("""async () => {
            const store=await caches.open("urbarium-worker-wasm-v1");
            const keys=await store.keys();
            return keys.length===1 && keys[0].url.includes(
                "/urbarium-mesh-worker_bg.wasm?urbarium_sha256=");
        }""",timeout=25000)
        worker_disk=page.evaluate("""async () => {
            const bucket=await caches.open("urbarium-worker-wasm-v1");
            const keys=await bucket.keys();
            return keys.map(key=>key.url);
        }""")
        import re
        assert len(worker_disk)==1 and re.fullmatch(
            r"http://127\.0\.0\.1:8010/urbarium-mesh-worker_bg\.wasm"
            r"\?urbarium_sha256=[0-9a-f]{64}",worker_disk[0]),worker_disk
        # Force a new worker process; an in-memory WASM instance cannot
        # masquerade as a cross-navigation persistent CacheStorage hit.
        page.reload(wait_until="domcontentloaded")
        page.wait_for_function("""() =>
            window.__urbariumWorkerCache?.kind==="hit" &&
            document.querySelector("#status")?.dataset.meshWorkerState==="ready"
        """,timeout=90000)
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.sceneDetail==="complete" &&
                Number(s.dataset.meshWorkerChunks)===
                Number(s.dataset.chunksRegionTotal) &&
                Number(s.dataset.chunksResident)===
                Number(s.dataset.chunksRegionTotal);
        }""",timeout=180000)
        assert not errors,errors
        print("CPU mesh worker: SHA-256 WASM first download saved and next visit reused cached binary")


        # Real browser warm navigation must still render while bypassing
        # custom WASM CacheStorage and service-worker response interception.
        isolated=browser.new_context(viewport={"width":1280,"height":800})
        try:
            first=isolated.new_page()
            wasm_responses=[]
            first.on("response",lambda response:
                wasm_responses.append((response.url,response.from_service_worker))
                if "urbarium-viewer-" in response.url and
                   response.url.split("?")[0].endswith("_bg.wasm") else None)
            if force_webgl:
                first.add_init_script("""Object.defineProperty(navigator,'gpu',{
                    configurable:true,value:undefined});""")
            first.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}&progressive=0",wait_until="domcontentloaded")
            first.wait_for_function("""() =>
                document.querySelector("#status")?.dataset.renderStage==="first-present"
            """,timeout=90000)
            assert wasm_responses and all(not via_worker for _,via_worker in wasm_responses),wasm_responses
            assert first.locator("#startup-preview").count()==0
            print("Native browser WASM response without service worker interception",
                  wasm_responses)
        finally:
            isolated.close()
        # Source-generated forensic review: the five currently unresolved
        # centerline conflicts must be individually reachable in actual 3D.
        # Do not skip tests when future regions change the number of cases.
        from pathlib import Path
        manifest=json.loads(Path("dist/assets/road-conflict-locations.json")
                            .read_text(encoding="utf-8"))
        cases=manifest["cases"]
        assert len(cases)==sum(x["centerline_conflicts"]
                              for x in manifest["regions"]),manifest
        assert len({item["region_id"] for item in cases})>=2,manifest
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}{suffix}",
                  wait_until="domcontentloaded")
        page.wait_for_function("() => document.querySelector('#hud-tab-details')",
                               timeout=12000)
        page.locator("#hud-tab-details").click()
        page.locator("#road-conflicts-load").click()
        page.wait_for_function("""(expected) =>
            !document.querySelector("#road-conflicts-open").disabled &&
            document.querySelector("#road-conflicts-select").options.length===expected
        """,arg=len(cases),timeout=30000)
        # The selected audit case must expose its actual source identities
        # and cautious vertical-evidence interpretation before navigation.
        evidence=page.locator("#road-conflicts-evidence")
        assert evidence.is_visible()
        assert cases[0]["street_id"] in evidence.inner_text()
        assert cases[0]["building_id"] in evidence.inner_text()
        if cases[0].get("street_name"):
            assert cases[0]["street_name"] in evidence.inner_text()
        topology={
            "through_footprint":"crosses this footprint",
            "ends_inside_footprint":"terminates inside",
            "starts_inside_footprint":"begins inside",
            "both_ends_inside_footprint":"Both source endpoints",
        }
        expected_pattern=topology.get(cases[0].get("source_line_pattern"))
        if expected_pattern:
            assert expected_pattern in evidence.inner_text()
        if len(cases)>1:
            page.locator("#road-conflicts-select").select_option("1")
            assert cases[1]["street_id"] in evidence.inner_text()
            page.locator("#road-conflicts-select").select_option("0")
            assert cases[0]["street_id"] in evidence.inner_text()
        # UI selection builds a canonical standalone-region URL instead of
        # relying on the joined world's unrelated secondary coordinate origin.
        selected=cases[0]
        page.locator("#road-conflicts-open").click()
        page.wait_for_function("""([region,lon,lat]) => {
            const s=document.querySelector("#status");
            const u=new URL(location.href);
            return s?.dataset.reviewFocusActive==="true" &&
                s.dataset.regionId===region &&
                u.searchParams.get("world")==="0" &&
                Math.abs(Number(u.searchParams.get("focus").split(",")[0])-lon)<1e-7 &&
                Math.abs(Number(u.searchParams.get("focus").split(",")[1])-lat)<1e-7 &&
                s.dataset.renderStage==="first-present";
        }""",arg=[selected["region_id"],*selected["focus_lonlat"]],timeout=90000)
        import math
        earth=6371008.8
        for region_case in [
            next(x for x in cases if x["region_id"]==region)
            for region in sorted({x["region_id"] for x in cases})
        ]:
            lon,lat=region_case["focus_lonlat"]
            region_id=region_case["region_id"]
            page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}"+
                      ("&allow-software=1" if not force_webgl else "")+
                      f"&world=0&region={region_id}&focus={lon:.9f},{lat:.9f}",
                      wait_until="domcontentloaded")
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return s?.dataset.reviewFocusActive==="true" &&
                    s.dataset.renderStage==="first-present" &&
                    Number(s.dataset.frameCount)>2;
            }""",timeout=90000)
            city=json.loads(Path("dist/assets",region_id+".json").read_text())
            origin=city["region"]["origin"]
            focus_x=math.radians(lon-origin[0])*earth*math.cos(math.radians(origin[1]))
            focus_z=math.radians(lat-origin[1])*earth
            pose=[float(v) for v in page.locator("#status")
                  .get_attribute("data-camera-pose").split(",")]
            assert math.hypot(pose[0]-focus_x,pose[2]-focus_z)<120.0,(
                "Camera did not arrive at audited source location",region_id,pose)
            assert pose[4]<-0.15,("Expected downward-looking elevated camera",pose)
            assert float(page.locator("#status")
                         .get_attribute("data-eye-clearance"))>90.0
            assert page.locator("#status").get_attribute("data-gpu-backend")==selected_backend
            assert not errors,errors
        # Graphics-tab MSAA and lowest-detail skyline are exercised together.
        # This catches multi-sampled color/depth mismatches and low-memory
        # fallback errors on BOTH WebGPU and forced WebGL2.
        page.goto(f"http://127.0.0.1:8010/?renderer={qa_mode}"+
                  ("&allow-software=1" if not force_webgl else "")+
                  "&world=1&detailcells=6&msaa=4",
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.renderStage==="first-present" &&
                Number(s.dataset.frameCount)>2 &&
                Number(s.dataset.chunksResident)===6 &&
                Number(s.dataset.farMassingCells)>0 &&
                ["1","4"].includes(s.dataset.msaaSamples);
        }""",timeout=90000)
        page.locator("#hud-tab-graphics").click()
        assert page.locator("#msaa-select").input_value()=="4"
        actual=page.locator("#status").get_attribute("data-msaa-samples")
        note=page.locator("#msaa-note").inner_text()
        if actual=="4":
            assert "4× MSAA active" in note,note
        else:
            assert "unavailable" in note,note
        assert not errors,errors
        # Changing MSAA must NOT navigate. Preserve camera, terrain/mesh
        # residency, DOM instance, and the already-rendered frame counter.
        baseline=page.evaluate("""() => ({
            timeOrigin:performance.timeOrigin,
            root:document.documentElement,
            pose:document.querySelector("#status").dataset.cameraPose,
            frames:Number(document.querySelector("#status").dataset.frameCount),
            resident:Number(document.querySelector("#status").dataset.chunksResident),
            region:document.querySelector("#status").dataset.regionId
        })""")
        page.locator("#msaa-select").select_option("1")
        page.wait_for_function("""(prior) => {
            const s=document.querySelector("#status");
            return s?.dataset.msaaSamples==="1" &&
                Number(s.dataset.frameCount)>prior;
        }""",arg=baseline["frames"],timeout=20000)
        assert page.locator("#msaa-select").input_value()=="1"
        middle=page.evaluate("""() => ({
            timeOrigin:performance.timeOrigin,
            pose:document.querySelector("#status").dataset.cameraPose,
            frames:Number(document.querySelector("#status").dataset.frameCount),
            resident:Number(document.querySelector("#status").dataset.chunksResident),
            region:document.querySelector("#status").dataset.regionId,
            tab:document.querySelector("#hud-tab-graphics").getAttribute("aria-selected")
        })""")
        assert middle["timeOrigin"]==baseline["timeOrigin"],"MSAA Off navigated the page"
        assert middle["pose"]==baseline["pose"],"MSAA Off moved the camera"
        assert middle["resident"]==baseline["resident"]==6
        assert middle["region"]==baseline["region"]
        assert middle["tab"]=="true","MSAA Off reset the graphics tab"
        page.locator("#msaa-select").select_option("4")
        page.wait_for_function("""([prior,expected]) => {
            const s=document.querySelector("#status");
            return s?.dataset.msaaSamples===expected &&
                Number(s.dataset.frameCount)>prior+2;
        }""",arg=[middle["frames"],actual],timeout=20000)
        final=page.evaluate("""() => ({
            timeOrigin:performance.timeOrigin,
            pose:document.querySelector("#status").dataset.cameraPose,
            frames:Number(document.querySelector("#status").dataset.frameCount),
            resident:Number(document.querySelector("#status").dataset.chunksResident),
            region:document.querySelector("#status").dataset.regionId,
            tab:document.querySelector("#hud-tab-graphics").getAttribute("aria-selected")
        })""")
        assert final["timeOrigin"]==baseline["timeOrigin"],"MSAA 4x navigated the page"
        assert final["pose"]==baseline["pose"],"MSAA 4x moved the camera"
        assert final["resident"]==baseline["resident"]==6
        assert final["region"]==baseline["region"]
        assert final["tab"]=="true","MSAA 4x reset the graphics tab"
        assert final["frames"]>middle["frames"]>baseline["frames"]
        assert not errors,errors
        print("MSAA Off/4x switched LIVE without navigation, camera movement or mesh eviction",
              {"requested":4,"active":actual,"residents":6,
               "farCells":page.locator("#status").get_attribute("data-far-massing-cells")})
        print("Source-driven road conflicts: explicit 3D review links for both regions passed")
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
