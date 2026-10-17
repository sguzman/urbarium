/* Cache by the build's actual geographic asset SHA-256, not by URL alone.
 * Old cache entries are never mistaken for a freshly processed city.
 * Identical geographic bytes survive application/WASM deployments unchanged.
 */
(() => {
  "use strict";
  const name="urbarium-geography-downtown-la-v1"; // Preserve old valid pilot cache entries.
  // Stamped into HTML at release build time using actual city/terrain bytes.
  // A missing stamp deliberately bypasses persistent caching rather than
  // returning unverified stale geography from a previous application release.
  const versions=window.__urbariumGeographyVersions||{};
  const assetNames=new Map(Object.keys(versions)
    .filter(f=>/^[a-z0-9-]+\.json$/.test(f))
    .map(f=>[f,(f.endsWith("-terrain.json")?"Terrain ":"City ")+
        f.replace(/-terrain\.json$|\.json$/,"")]));
  // Only versioned geographic files are intercepted. Unlisted paths do
  // ordinary fetches, preventing unrelated web assets entering CacheStorage.
  const nativeFetch=window.fetch.bind(window);
  const inFlight=new Map();
  const states=Object.create(null);
  const telemetry={assets:states,networkRequests:0,cacheHits:0,versions};
  window.__urbariumGeoCache=telemetry;
  const eligible=req=>{
    const url=new URL(req.url);
    const folder=new URL("./assets/",location.href);
    return req.method==="GET" && url.origin===location.origin &&
      url.pathname.startsWith(folder.pathname) &&
      url.pathname.slice(folder.pathname.length).indexOf("/")===-1 &&
      assetNames.has(url.pathname.slice(folder.pathname.length));
  };
  const versionOf=req=>{
    const file=new URL(req.url).pathname.split("/").pop();
    const sha=versions[file];
    return typeof sha==="string" && /^[0-9a-f]{64}$/.test(sha) ? sha : null;
  };
  const keyedRequest=(req,sha)=>{
    const url=new URL(req.url);
    url.searchParams.set("urbarium_sha256",sha);
    return new Request(url,req);
  };
  function emit(request,state,bytes=null,error=null){
    const filename=new URL(request.url).pathname.split("/").pop();
    const label=assetNames.get(filename);
    states[label]={state,bytes,error,time:performance.now(),
      version:versionOf(request)?.slice(0,12)||"unverified"};
    const snapshot=Object.values(states);
    const text=snapshot.map(item=>{
      const entry=Object.entries(states).find(([,value])=>value===item);
      return entry[0]+": "+item.state+
        (item.bytes>0?" ("+(item.bytes/1048576).toFixed(2)+" MB)":"")+
        " ["+item.version+"]";
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
    const sha=versionOf(req);
    if(!sha) {
      emit(req,"UNVERIFIED: fetching from network (no asset fingerprint)");
      telemetry.networkRequests++;
      return nativeFetch(req,{cache:"no-cache"});
    }
    const key=keyedRequest(req,sha);
    let cache;
    try {cache=await caches.open(name);}
    catch(error){
      emit(req,"browser cache unavailable",null,String(error));
      telemetry.networkRequests++;
      return nativeFetch(key);
    }
    try{
      // Strict match: never use an older URL-only cached city from v1.
      const saved=await cache.match(key);
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
    // Versioned request URL bypasses browser HTTP caches for older geography.
    const response=await nativeFetch(key);
    if(response.ok){
      try{
        await cache.put(key,response.clone());
        // Bound CacheStorage size: keep only current revision per asset.
        const oldKeys=await cache.keys();
        for(const old of oldKeys){
          if(new URL(old.url).pathname===new URL(key.url).pathname &&
             old.url!==key.url)await cache.delete(old);
        }
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
    let req;
    try{req=new Request(input,init);}catch(_){return nativeFetch(input,init);}
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