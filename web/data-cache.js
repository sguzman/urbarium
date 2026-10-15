/* Preserve the existing cache namespace across viewer redeploys. Never
 * invalidate geographic bytes just because the WASM/HTML changed.
 * This layer also exposes true per-asset cache hits versus network requests.
 */
(() => {
  "use strict";
  const name="urbarium-geography-downtown-la-v1";
  const assetNames=new Map([
    ["downtown-la.json","City"],["downtown-la-terrain.json","Terrain"]
  ]);
  const nativeFetch=window.fetch.bind(window);
  const inFlight=new Map();
  const states=Object.create(null);
  const telemetry={assets:states,networkRequests:0,cacheHits:0};
  window.__urbariumGeoCache=telemetry;
  const eligible=req=>{
    const url=new URL(req.url);
    const folder=new URL("./assets/",location.href);
    return req.method==="GET" && url.origin===location.origin &&
      url.pathname.startsWith(folder.pathname) &&
      url.pathname.slice(folder.pathname.length).indexOf("/")===-1 &&
      assetNames.has(url.pathname.slice(folder.pathname.length));
  };
  function emit(request,state,bytes=null,error=null){
    const filename=new URL(request.url).pathname.split("/").pop();
    const label=assetNames.get(filename);
    states[label]={state,bytes,error,time:performance.now()};
    const snapshot=Object.values(states);
    const text=snapshot.map(item=>{
      const entry=Object.entries(states).find(([,value])=>value===item);
      return entry[0]+": "+item.state+
        (item.bytes>0?" ("+(item.bytes/1048576).toFixed(2)+" MB)":"");
    }).join(" · ");
    window.__urbariumCacheState=text||"checking";
    document.getElementById("geodata-cache-state")?.replaceChildren(
      document.createTextNode("Geography: "+window.__urbariumCacheState));
    window.dispatchEvent(new CustomEvent("urbarium-cache-status",{
      detail:{asset:label,state,bytes,error,
        networkRequests:telemetry.networkRequests,cacheHits:telemetry.cacheHits}
    }));
  }
  async function load(req){
    let cache;
    try {cache=await caches.open(name);}
    catch(error){
      emit(req,"browser cache unavailable",null,String(error));
      telemetry.networkRequests++;
      return nativeFetch(req);
    }
    try{
      const saved=await cache.match(req,{ignoreSearch:true});
      if(saved){
        telemetry.cacheHits++;
        emit(req,"CACHE HIT",Number(saved.headers.get("content-length"))||null);
        return saved;
      }
    }catch(error){
      console.warn("Urbarium geography cache read failed",error);
    }
    emit(req,"NETWORK DOWNLOAD");
    telemetry.networkRequests++;
    const response=await nativeFetch(req);
    if(response.ok){
      try{
        await cache.put(req,response.clone());
        emit(req,"downloaded + saved",
          Number(response.headers.get("content-length"))||null);
      }catch(error){
        console.warn("Urbarium geography cache save failed",error);
        emit(req,"downloaded (not cached)",null,String(error));
      }
    }else{
      emit(req,"HTTP "+response.status);
    }
    return response;
  }
  window.fetch=(input,init)=>{
    const req=new Request(input,init);
    if(!eligible(req)||req.cache==="no-store")return nativeFetch(input,init);
    if(inFlight.has(req.url))return inFlight.get(req.url).then(r=>r.clone());
    const pending=load(req);
    inFlight.set(req.url,pending);
    pending.finally(()=>inFlight.delete(req.url)).catch(()=>{});
    return pending.then(r=>r.clone());
  };
  window.urbariumClearGeography=async()=>{
    if(!("caches" in window))return false;
    const removed=await caches.delete(name);
    for(const key of Object.keys(states))delete states[key];
    window.__urbariumCacheState="cleared; awaiting redownload";
    document.getElementById("geodata-cache-state")?.replaceChildren(
      document.createTextNode("Geography: "+window.__urbariumCacheState));
    return removed;
  };
})();