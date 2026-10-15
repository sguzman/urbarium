/* Persistent cache for acquired geography. App code remains network-updated.
 * A first download populates CacheStorage BEFORE Rust's fetch resolves, so
 * even the next immediate reload can work without downloading geography.
 * Only same-origin, read-only geographic assets are cached.
 */
(() => {
  "use strict";
  const name = "urbarium-geography-downtown-la-v1";
  const assets = new Set(["downtown-la.json", "downtown-la-terrain.json"]);
  const original = window.fetch.bind(window);
  const pending = new Map();
  const eligible = req => {
    const url = new URL(req.url);
    const root = new URL("./assets/", location.href);
    return req.method === "GET" && url.origin === location.origin &&
      url.pathname === root.pathname + url.pathname.split("/").pop() &&
      assets.has(url.pathname.split("/").pop());
  };
  const report = (asset, state) => {
    window.__urbariumCacheState = state;
    document.getElementById("geodata-cache-state")?.replaceChildren(
      document.createTextNode("Geography: " + state));
    window.dispatchEvent(new CustomEvent("urbarium-cache-status",
      {detail:{asset,state}}));
  };
  async function fromCache(request) {
    let cache;
    try {cache = await caches.open(name);}
    catch (error) {
      console.warn("Geography cache unavailable; falling back to network", error);
      return original(request);
    }
    try {
      const cached = await cache.match(request, {ignoreSearch:true});
      if (cached) {report(request.url, "cached"); return cached;}
    } catch (error) {console.warn("Cache read unavailable",error);}
    report(request.url, "downloading");
    const response = await original(request);
    if (response.ok) {
      try {await cache.put(request, response.clone());report(request.url,"saved offline");}
      catch (error) {console.warn("Could not persist geography",error);report(request.url,"network only");}
    }
    return response;
  }
  window.fetch = (input, init) => {
    const request = new Request(input, init);
    if (!eligible(request) || request.cache === "no-store")
      return original(input,init);
    const key = request.url;
    if (pending.has(key)) return pending.get(key).then(r=>r.clone());
    const promise = fromCache(request);
    pending.set(key,promise);
    promise.finally(()=>pending.delete(key)).catch(()=>{});
    return promise.then(r=>r.clone());
  };
  window.urbariumClearGeography = async () => {
    if (!("caches" in window)) return false;
    await caches.delete(name);
    report("", "cleared");
    return true;
  };
})();