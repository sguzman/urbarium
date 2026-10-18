/* CPU-only Rust mesh worker; no GPU, window or canvas.
 * Reuses downloaded City/DEM JSON and caches the worker's exact WASM version
 * independently of the viewer executable. Cache failures never block startup.
 */
"use strict";
const workerWasmSha256="__URBARIUM_WORKER_SHA256__";
const workerCacheName="urbarium-worker-wasm-v1";
const workerWasmPath="./urbarium-mesh-worker_bg.wasm";
const maxWorkerVersions=2;
let ready=false;
let failed=false;
let engine=null;
function reportCache(kind,detail=""){
  self.postMessage({type:"cache",kind,detail,version:workerWasmSha256.slice(0,12)});
}
async function getVersionedWasm(){
  const url=new URL(workerWasmPath,self.location.href);
  if(!/^[0-9a-f]{64}$/.test(workerWasmSha256))
    throw Error("Worker executable missing its release content hash");
  url.searchParams.set("urbarium_sha256",workerWasmSha256);
  const request=new Request(url);
  let cache=null;
  try{
    cache=await caches.open(workerCacheName);
    const saved=await cache.match(request);
    if(saved){
      reportCache("hit");
      return saved;
    }
  }catch(error){
    reportCache("unavailable",String(error));
  }
  const response=await fetch(request);
  if(!response.ok)throw Error("Worker WASM HTTP "+response.status);
  reportCache("network");
  if(cache){
    // CacheStorage consumes a clone; initialization uses the original body
    // immediately. A slow/full disk must not delay compiling the worker.
    const copy=response.clone();
    void (async()=>{
      await cache.put(request,copy);
      const entries=await cache.keys();
      const keep=new Set([request.url]);
      for(const old of entries.slice().reverse()){
        if(keep.size>=maxWorkerVersions)break;
        keep.add(old.url);
      }
      for(const old of entries){
        if(!keep.has(old.url))await cache.delete(old);
      }
      reportCache("stored");
    })().catch(error=>reportCache("unavailable",String(error)));
  }
  return response;
}
self.onmessage=async event=>{
  const data=event.data;
  if(data?.type==="init"){
    try {
      importScripts("./urbarium-mesh-worker.js");
      engine=wasm_bindgen;
      if(typeof engine!=="function")throw Error("WASM worker loader missing");
      await engine(await getVersionedWasm());
      engine.initialize_worker(data.city,data.terrain);
      ready=true;
      self.postMessage({type:"ready"});
    }catch(error){
      failed=true;
      self.postMessage({type:"error",message:String(error)});
    }
    return;
  }
  if(data?.type==="mesh" && ready && !failed){
    try{
      const payload=engine.mesh_chunk(data.x,data.z);
      self.postMessage({type:"mesh",x:data.x,z:data.z,payload});
    }catch(error){
      failed=true;
      self.postMessage({type:"error",message:String(error)});
    }
  }
};
