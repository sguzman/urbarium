/* CPU-only Rust mesh worker. Never creates a GPU, canvas, or window.
 * Receives the existing first-fetch JSON; doesn't redownload geography.
 * One request is computed at a time to avoid an unbounded queue.
 */
"use strict";
let ready=false;
let failed=false;
let engine=null;
self.onmessage=async event=>{
  const data=event.data;
  if(data?.type==="init"){
    try {
      importScripts("./urbarium-mesh-worker.js");
      engine=self.wasm_bindgen;
      if(typeof engine!=="function")throw Error("WASM worker loader missing");
      await engine("./urbarium-mesh-worker_bg.wasm");
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
