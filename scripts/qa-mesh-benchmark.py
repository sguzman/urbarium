#!/usr/bin/env python3
"""Compare two WARM-CACHE normal Downtown builds on one Chromium renderer.

This intentionally reports measured costs without a flaky CI speedup gate.
Uses the same pre-fetched source geometry, original Rust mesher and camera.
Visual parity and full thirty-cell residency ARE hard release requirements.
"""
from io import BytesIO
import json
from pathlib import Path
from statistics import median
from playwright.sync_api import sync_playwright
from PIL import Image,ImageChops

BASE="http://127.0.0.1:8010/?renderer=webgl2&world=0"
OUTPUT=Path("/tmp/urbarium-mesh-benchmark.json")
BENCH_JS=r"""(() => {
  const record=window.__urbariumBench={
    firstFrameAt:null,completeAt:null,frameIntervals:[],longTasks:[],
    previousFrame:null
  };
  function observe(){
    const s=document.querySelector("#status");
    const now=performance.now();
    if(s?.dataset.renderStage==="first-present" && record.firstFrameAt===null)
      record.firstFrameAt=now;
    if(s?.dataset.sceneDetail==="complete" && record.completeAt===null)
      record.completeAt=now;
  }
  new MutationObserver(observe).observe(document,{
    subtree:true,attributes:true,
    attributeFilter:["data-render-stage","data-scene-detail"]
  });
  try{
    new PerformanceObserver(list=>{
      for(const t of list.getEntries())
        record.longTasks.push({start:t.startTime,duration:t.duration});
    }).observe({type:"longtask",buffered:true});
  }catch(_){}
  function frame(ts){
    if(record.firstFrameAt!==null && record.completeAt===null){
      if(record.previousFrame!==null)
        record.frameIntervals.push(ts-record.previousFrame);
      record.previousFrame=ts;
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
})()"""

def p95(values):
    if not values:return None
    values=sorted(values)
    return round(values[min(len(values)-1,int(0.95*(len(values)-1)))],2)

with sync_playwright() as p:
    browser=p.chromium.launch(headless=False,args=[
        "--no-sandbox","--disable-dev-shm-usage","--window-size=1280,800",
        "--enable-unsafe-webgpu","--enable-features=Vulkan",
        "--use-angle=vulkan","--use-vulkan=swiftshader",
        "--disable-vulkan-surface"
    ])
    page=browser.new_page(viewport={"width":1280,"height":800})
    page.add_init_script(BENCH_JS)
    def run(mode,label):
        page.goto(BASE+("&meshworker=1" if mode=="worker" else "&meshworker=0"),
                  wait_until="domcontentloaded")
        page.wait_for_function("""() => {
            const s=document.querySelector("#status");
            return s?.dataset.sceneDetail==="complete" &&
                s.dataset.chunkStreaming==="true" &&
                Number(s.dataset.chunksResident)>9 &&
                Number(s.dataset.chunksResident)===
                Number(s.dataset.chunksRegionTotal) &&
                Number(s.dataset.chunksEvicted)===0;
        }""",timeout=180000)
        if mode=="worker":
            page.wait_for_function("""() => {
                const s=document.querySelector("#status");
                return s?.dataset.meshWorkerState==="ready" &&
                    Number(s.dataset.meshWorkerChunks)===
                    Number(s.dataset.chunksRegionTotal);
            }""",timeout=180000)
        page.wait_for_timeout(400)  # flush trailing longtask observations
        raw=page.evaluate("""() => {
            const r=window.__urbariumBench;
            const s=document.querySelector("#status");
            return {firstFrameAt:r.firstFrameAt,completeAt:r.completeAt,
                frames:r.frameIntervals,longTasks:r.longTasks,
                resident:Number(s.dataset.chunksResident),
                workerState:s.dataset.meshWorkerState||"disabled",
                workerCache:window.__urbariumWorkerCache?.kind||"disabled",
                workerFormat:s.dataset.meshWorkerFormat||"local",
                workerBytes:Number(s.dataset.meshWorkerBytes||0),
                workerMessages:Number(s.dataset.meshWorkerMessages||0)};
        }""")
        assert raw["firstFrameAt"] is not None and raw["completeAt"] is not None
        interval=raw["completeAt"]-raw["firstFrameAt"]
        tasks=[t["duration"] for t in raw["longTasks"] if
               raw["firstFrameAt"]<=t["start"]<=raw["completeAt"]]
        frames=raw["frames"]
        result={
            "mode":mode,"label":label,
            "meshPhaseMs":round(interval,2),
            "longTaskCount":len(tasks),
            "longTaskTotalMs":round(sum(tasks),2),
            "longTaskWorstMs":round(max(tasks,default=0),2),
            "frameIntervalsP95Ms":p95(frames),
            "frameIntervalWorstMs":round(max(frames,default=0),2),
            "framesSampled":len(frames),
            "resident":raw["resident"],"workerState":raw["workerState"],
            "workerCache":raw["workerCache"],
            "workerFormat":raw["workerFormat"],
            "workerTransferredBytes":raw["workerBytes"],
            "workerMessages":raw["workerMessages"]
        }
        if mode=="worker":
            assert result["workerFormat"]=="UBM1-transfer",result
            assert result["workerMessages"]==result["resident"],result
            assert result["workerTransferredBytes"]>24*result["resident"],result
        canvas=page.locator("canvas:not(#minimap-canvas):not(#webgpu-control)").first
        return result,canvas.screenshot()

    # Warm each WASM path and geography into browser disk caches, then measure
    # identical camera and software WebGL2 with alternating modes.
    run("local","local-warmup")
    run("worker","worker-warmup")
    local,local_img=run("local","local-measured")
    worker,worker_img=run("worker","worker-measured")
    with Image.open(BytesIO(local_img)) as a,Image.open(BytesIO(worker_img)) as b:
        delta=ImageChops.difference(a.convert("RGB"),b.convert("RGB"))
        diff=delta.convert("L").point(lambda v:255 if v>24 else 0)
        fraction=diff.histogram()[255]/(a.width*a.height)
    assert fraction<0.005,("Worker changed visible geometry",fraction)
    assert local["resident"]==worker["resident"]==30
    output={
        "renderer":"Chromium software WebGL2",
        "context":"warm asset and WASM cache; same city and 1280x800 viewport",
        "local":local,"worker":worker,
        "pixelMismatchFraction":round(fraction,6),
        "speedupMeshPhase":round(local["meshPhaseMs"]/max(1.0,worker["meshPhaseMs"]),3)
    }
    OUTPUT.write_text(json.dumps(output,indent=2),encoding="utf-8")
    print("Warm-cache CPU meshing comparison:",json.dumps(output))
    browser.close()
