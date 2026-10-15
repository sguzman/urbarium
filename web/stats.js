/* Browser diagnostics for Urbarium. Only real/observable metrics are shown.
 * No browser provides arbitrary per-app system CPU/GPU utilization or VRAM.
 */
(() => {
  "use strict";
  const status = document.getElementById("status");
  const panel = document.getElementById("stats-panel");
  const toggle = document.getElementById("stats-toggle");
  if (!status || !panel || !toggle) return;
  const get = (id) => document.getElementById(id);
  const set = (id, value) => {const node=get(id);if(node)node.textContent=value;};
  const fmt = (value, digits=1) =>
    Number.isFinite(value) ? value.toFixed(digits) : "—";
  const num = (attribute) => {
    const str = status.getAttribute(attribute);
    if (str == null || str === "") return NaN;
    const n = Number(str);
    return Number.isFinite(n) ? n : NaN;
  };
  const mebibytes = (bytes) => Number.isFinite(bytes)
    ? fmt(bytes / 1048576, 2) + " MiB" : "N/A";
  const messageKey = "urbarium-last-render-error";
  const diagnostics = get("stats-last-error");
  const recentMessage = () => {
    try {return sessionStorage.getItem(messageKey) ?? "";}catch(_){return "";}
  };
  if (recentMessage() && diagnostics) {
    diagnostics.textContent = "Previous error: " + recentMessage();
  }
  function recordError(message) {
    const safe = String(message).slice(0, 450);
    console.warn("[Urbarium diagnostic]", safe);
    if (diagnostics) diagnostics.textContent = safe;
    try {sessionStorage.setItem(messageKey, safe);}catch(_){}
  }
  window.addEventListener("error", e => recordError(e.message || "Browser error"));
  window.addEventListener("unhandledrejection", e =>
    recordError("Unhandled rejection: " + String(e.reason)));
  // This observes explicit WebGL context loss (e.g. GPU process reset).
  const observed = new WeakSet();
  function observeCanvas() {
    for (const canvas of document.querySelectorAll("canvas:not(#minimap-canvas)")) {
      if(observed.has(canvas)) continue;
      observed.add(canvas);
      canvas.addEventListener("webglcontextlost", e => {
        e.preventDefault();
        recordError("WebGL graphics context lost: renderer may need a reload");
      });
      canvas.addEventListener("webglcontextrestored", () =>
        recordError("WebGL graphics context restored; reload if city stays blank"));
    }
  }
  const chooser = (id) => {
    const input = get(id);
    if(!input) return;
    const setVisible = () => {
      input.setAttribute("data-visible", input.checked ? "true" : "false");
    };
    input.addEventListener("change", setVisible);
    setVisible();
  };
  chooser("layer-walk");
  chooser("layer-rail");
  toggle.addEventListener("click", () => {
    const visible = panel.hidden;
    panel.hidden = !visible;
    toggle.setAttribute("aria-pressed", String(visible));
    toggle.textContent = visible ? "Stats: on" : "Stats: off";
  });
  let previousFrames = 0;
  let previousTime = performance.now();
  let previousAdvance = performance.now();
  function refresh() {
    observeCanvas();
    const now = performance.now();
    const frames = num("data-frame-count");
    const elapsed = (now - previousTime) / 1000;
    const presented = Number.isFinite(frames) && frames >= previousFrames
      ? frames - previousFrames : 0;
    const fps = presented / Math.max(elapsed,0.001);
    if (presented) previousAdvance=now;
    if (now - previousAdvance > 4500) {
      set("stats-fps", "STALLED");
    } else {
      set("stats-fps", fmt(fps,1));
    }
    previousFrames=Number.isFinite(frames)?frames:previousFrames;
    previousTime=now;
    if(panel.hidden) return;
    set("stats-backend",status.getAttribute("data-gpu-backend") || "Loading");
    set("stats-frame",fmt(num("data-frame-ms"),2)+" ms");
    set("stats-cpu-frame",fmt(num("data-cpu-frame-ms"),2)+" ms");
    set("stats-triangles",Number.isFinite(num("data-triangles-drawn"))
      ?Math.round(num("data-triangles-drawn")).toLocaleString():"—");
    set("stats-drawcalls",fmt(num("data-draw-calls"),0));
    set("stats-gpu-buffers",mebibytes(num("data-gpu-mesh-bytes")));
    const heap = performance.memory?.usedJSHeapSize;
    set("stats-js-heap",mebibytes(heap));
  }
  refresh();
  setInterval(refresh,1000);
})();
