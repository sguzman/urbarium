/* Migration worker: replace the former cache-first WASM service worker.
 * Deliberately NO fetch event listener. All fingerprinted WASM requests go
 * through the browser's ordinary HTTP cache and streaming compilation.
 * Older clients upgrade automatically through the unchanged scope/script.
 */
"use strict";
self.addEventListener("install", event=>{
  event.waitUntil(self.skipWaiting());
});
self.addEventListener("activate", event=>{
  event.waitUntil(self.clients.claim());
});
