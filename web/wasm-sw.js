/* Persistent cache for Trunk's content-fingerprinted Rust/WASM executable.
 * HTML, generated JavaScript, geography, and unrelated files are never
 * intercepted. New deployments have new WASM URLs and cannot use stale code.
 */
"use strict";
const CACHE_NAME="urbarium-wasm-binaries-v1";
const MAX_VERSIONS=2;
const scopePath=new URL(self.registration.scope).pathname;
const wasmPattern=/^urbarium-viewer-[0-9a-f]+_bg\.wasm$/;

function isViewerWasm(request){
  if(request.method!=="GET" || request.headers.has("Range"))return false;
  const url=new URL(request.url);
  if(url.origin!==self.location.origin || !url.pathname.startsWith(scopePath))return false;
  const relative=url.pathname.slice(scopePath.length);
  return !relative.includes("/") && wasmPattern.test(relative) && !url.search;
}
async function tell(clientId,kind,url){
  if(!clientId)return;
  const client=await self.clients.get(clientId);
  if(client)client.postMessage({
    type:"urbarium-wasm-cache-v1",
    kind,
    file:new URL(url).pathname.split("/").pop()
  });
}
async function prune(cache,currentRequest){
  const entries=await cache.keys();
  const keep=new Set([currentRequest.url]);
  // CacheStorage enumerates keys in insertion order. Keep the most recent
  // compiled binaries (including the one this event just stored).
  for(const req of entries.slice().reverse()){
    if(keep.size>=MAX_VERSIONS)break;
    keep.add(req.url);
  }
  for(const req of entries){
    if(!keep.has(req.url))await cache.delete(req);
  }
}
self.addEventListener("install",event=>{
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate",event=>{
  event.waitUntil(self.clients.claim());
});
self.addEventListener("fetch",event=>{
  if(!isViewerWasm(event.request))return;
  const req=event.request;
  const clientId=event.clientId;
  event.respondWith((async()=>{
    // CacheStorage can be exceptionally slow or stuck on a stressed
    // multi-tab browser. Do not let its lookup gate the entire WASM loader.
    // A bounded cache lookup preserves fast hits and falls back to normal
    // HTTP-cache-aware network fetching instead of waiting indefinitely.
    let cache=null;
    let slowCache=false;
    try {
      const lookup=(async()=>{
        const opened=await caches.open(CACHE_NAME);
        return {cache:opened,saved:await opened.match(req)};
      })();
      const timeout=new Promise(resolve=>
        setTimeout(()=>resolve({cache:null,saved:null,slow:true}),1500));
      const result=await Promise.race([lookup,timeout]);
      slowCache=!!result.slow;
      cache=result.cache;
      if(result.saved){
        event.waitUntil(tell(clientId,"hit",req.url));
        return result.saved;
      }
    }catch(error){
      console.warn("Urbarium WASM cache unavailable",error);
    }
    // No valid disk hit within 1.5 seconds? Fetch the actual current binary.
    // This bypasses a stalled service-worker lookup, not the usual HTTP cache.
    const response=await fetch(req);
    if(slowCache)event.waitUntil(tell(clientId,"slow-cache-bypass",req.url));
    if(response.ok && response.type==="basic" && cache){
      const copy=response.clone();
      event.waitUntil((async()=>{
        try {
          await cache.put(req,copy);
          await prune(cache,req);
          await tell(clientId,"stored",req.url);
        }catch(error){
          console.warn("Urbarium WASM cache write failed",error);
          await tell(clientId,"unavailable",req.url);
        }
      })());
    }else{
      event.waitUntil(tell(clientId,"network",req.url));
    }
    return response;
  })());
});
