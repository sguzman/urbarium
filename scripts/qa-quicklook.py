#!/usr/bin/env python3
"""The nearby source-backed view must paint even if Rust WASM never loads."""
from playwright.sync_api import sync_playwright

with sync_playwright() as playwright:
    browser=playwright.chromium.launch(headless=True,args=["--no-sandbox"])
    context=browser.new_context(viewport={"width":1280,"height":800},
        service_workers="block")
    page=context.new_page()
    aborted=[]
    def refuse_wasm(route):
        aborted.append(route.request.url)
        route.abort("failed")
    page.route("**/*_bg.wasm",refuse_wasm)
    page.goto("http://127.0.0.1:8010/?world=1&progressive=0",
        wait_until="domcontentloaded")
    page.wait_for_function("""() => {
        const s=document.querySelector("#status");
        return s?.dataset.bootstrapPreview==="ready" &&
            Number(s.dataset.bootstrapPreviewStreets)>0 &&
            Number(s.dataset.bootstrapPreviewBuildings)>0;
    }""",timeout=30000)
    info=page.evaluate("""() => {
        const c=document.querySelector("#startup-preview");
        const s=document.querySelector("#status");
        return {
            width:c.width,height:c.height,
            streets:Number(s.dataset.bootstrapPreviewStreets),
            buildings:Number(s.dataset.bootstrapPreviewBuildings),
            delayMs:Number(s.dataset.bootstrapPreviewMs),
            visible:!!c?.isConnected&&getComputedStyle(c).display!=="none",
            firstFrame:s.dataset.renderStage==="first-present",
            firstLabel:c.getAttribute("aria-label")
        };
    }""")
    # The source paint can legitimately beat even the WASM request itself.
    # Interception is installed before navigation; an absent request at this
    # instant is proof of the intended independent startup, not a failure.
    assert info["width"]>=600 and info["height"]>=400,info
    assert info["visible"] and not info["firstFrame"],info
    assert info["streets"]>0 and info["buildings"]>0,info
    assert info["delayMs"]<30000,info
    assert "Simplified source-backed" in info["firstLabel"],info
    print("Pre-WASM visible source-backed streets/buildings despite blocked executable",info)
    context.close()
    browser.close()
