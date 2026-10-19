#!/usr/bin/env python3
"""Bounded real cold/warm first-frame measurement; no fake city placeholder."""
import json
import time
from playwright.sync_api import sync_playwright

with sync_playwright() as p:
    browser=p.chromium.launch(headless=False,args=[
        "--no-sandbox","--disable-setuid-sandbox","--disable-dev-shm-usage",
        "--window-size=1280,800","--enable-unsafe-webgpu",
        "--enable-features=Vulkan","--use-angle=vulkan",
        "--use-vulkan=swiftshader","--use-webgpu-adapter=swiftshader",
        "--disable-vulkan-surface",
    ])
    page=browser.new_page(viewport={"width":1280,"height":800})
    errors=[]
    page.on("pageerror",lambda error:errors.append(str(error)))
    url="http://127.0.0.1:8010/?world=1&renderer=webgpu&allow-software=1"
    times=[]
    for visit in ("cold","warm"):
        began=time.monotonic()
        page.goto(url,wait_until="domcontentloaded")
        assert page.locator('link[rel="preload"][type="application/wasm"]').count()==1
        assert page.locator("#startup-preview").count()==0
        page.wait_for_function("""() =>
            document.querySelector("#status")?.dataset.renderStage==="first-present" &&
            Number(document.querySelector("#status").dataset.frameCount)>0
        """,timeout=45000)
        elapsed=(time.monotonic()-began)*1000
        telemetry=page.evaluate("""() => {
            const wasm=performance.getEntriesByType("resource").find(x=>
                /urbarium-viewer-[0-9a-f]+_bg\.wasm/.test(x.name));
            return {
                nativeCache:window.__urbariumWasmCache?.source,
                wasmFetchMs:wasm?Math.round(wasm.responseEnd-wasm.startTime):null,
                wasmResponseAtMs:wasm?Math.round(wasm.responseEnd):null,
                frameCount:Number(document.querySelector("#status").dataset.frameCount),
                previewCanvasCount:document.querySelectorAll("#startup-preview").length
            };
        }""")
        assert telemetry["nativeCache"]=="native-http",telemetry
        assert telemetry["previewCanvasCount"]==0,telemetry
        assert telemetry["wasmFetchMs"] is not None,telemetry
        assert not errors,errors
        times.append({"visit":visit,"firstFrameMs":round(elapsed),"detail":telemetry})
    print("REAL STARTUP TIMING",json.dumps(times,sort_keys=True))
    # Detect a truly wedged startup, but do not assert a hardware-dependent
    # 1-second target against software GPU runners.
    assert max(x["firstFrameMs"] for x in times)<45000,times
    browser.close()
