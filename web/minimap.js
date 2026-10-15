/* Urbarium 2D locator: real city footprints and centerlines, not a second
 * 3D geometry pipeline. North-up, centered on the player's Rust camera.
 * This map covers the current bounded Downtown LA dataset, not LA County.
 */
(() => {
  "use strict";
  const canvas = document.getElementById("minimap-canvas");
  const ctx = canvas?.getContext("2d");
  const scaleLabel = document.getElementById("map-scale");
  const coordsLabel = document.getElementById("map-coordinates");
  if (!ctx || !scaleLabel || !coordsLabel) return;
  const size = 260;
  const radius = Math.PI / 180 * 6371008.8;
  const states = { city: null, scale: 650, last: 0, shapes: [] };
  // scale is the visible width in local meters.
  function zoom(factor) {
    states.scale = Math.max(100, Math.min(2500, states.scale * factor));
    paint();
  }
  document.getElementById("map-zoom-in")?.addEventListener("click", () => zoom(0.65));
  document.getElementById("map-zoom-out")?.addEventListener("click", () => zoom(1 / 0.65));
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    zoom(event.deltaY > 0 ? 1.18 : 1 / 1.18);
  }, { passive: false });
  for (const type of ["click", "mousedown", "mouseup"]) {
    canvas.addEventListener(type, (event) => event.stopPropagation());
  }
  const status = () => document.getElementById("status");
  const readNumber = (name, fallback = 0) => {
    const n = Number(status()?.getAttribute(name));
    return Number.isFinite(n) ? n : fallback;
  };
  const rgb = {
    road: "#5e7789", footway: "#bfbea9", steps: "#d4b58c",
    rail: "#79a7ab", ground: "#233a38", building: "#405b63",
    boundary: "#788a8a"
  };
  const roadKinds = new Set([
    "motorway", "motorway_link", "trunk", "trunk_link", "primary", "primary_link",
    "secondary", "secondary_link", "tertiary", "tertiary_link",
    "residential", "living_street", "service", "unclassified", "track"
  ]);
  const shapeColor = kind => roadKinds.has(kind) ? rgb.road :
      kind === "steps" ? rgb.steps :
      ["rail", "tram", "light_rail"].includes(kind) ? rgb.rail : rgb.footway;
  const project = (lon, lat, region) => ({
    x: (lon - region.origin[0]) * radius * Math.cos(region.origin[1] * Math.PI / 180),
    z: (lat - region.origin[1]) * radius,
  });
  function paint() {
    const city = states.city;
    if (!city) return;
    const x = readNumber("data-camera-x");
    const z = readNumber("data-camera-z");
    const yaw = readNumber("data-camera-yaw");
    const ppm = size / states.scale;
    const xy = (p) => [size / 2 + (p.x - x) * ppm,
                       size / 2 - (p.z - z) * ppm];
    ctx.clearRect(0, 0, size, size);
    ctx.fillStyle = "#111f2a";
    ctx.fillRect(0, 0, size, size);
    const sw = xy(project(city.region.bbox[0], city.region.bbox[1], city.region));
    const ne = xy(project(city.region.bbox[2], city.region.bbox[3], city.region));
    ctx.fillStyle = rgb.ground;
    ctx.fillRect(sw[0], ne[1], ne[0] - sw[0], sw[1] - ne[1]);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, size, size);
    ctx.clip();
    ctx.fillStyle = rgb.building;
    // Each source building may contain outer rings and courtyard holes.
    for (const building of city.buildings) {
      for (const polygon of building.polygons ?? []) {
        const rings = polygon.rings ?? [];
        if (!rings.length) continue;
        ctx.beginPath();
        for (const ring of rings) {
          ring.forEach((p, i) => {
            const [sx, sy] = xy(p);
            if (i === 0) ctx.moveTo(sx, sy);
            else ctx.lineTo(sx, sy);
          });
          ctx.closePath();
        }
        ctx.fill("evenodd");
      }
    }
    for (const street of city.streets) {
      if (!street.path || street.path.length < 2) continue;
      ctx.strokeStyle = shapeColor(street.kind);
      ctx.lineWidth = roadKinds.has(street.kind) ? 1.7 : 0.9;
      ctx.lineCap = "round";
      ctx.beginPath();
      for (const [i, p] of street.path.entries()) {
        const [sx, sy] = xy(p);
        if (i === 0) ctx.moveTo(sx, sy);
        else ctx.lineTo(sx, sy);
      }
      ctx.stroke();
    }
    ctx.strokeStyle = rgb.boundary;
    ctx.lineWidth = 1;
    ctx.strokeRect(sw[0], ne[1], ne[0] - sw[0], sw[1] - ne[1]);
    ctx.restore();
    // North stays at the top; arrow rotates with the actual FPS yaw.
    ctx.save();
    ctx.translate(size / 2, size / 2);
    ctx.rotate(yaw);
    ctx.fillStyle = "#ffdf86";
    ctx.strokeStyle = "#1c2830";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(0, -12);
    ctx.lineTo(-7, 9);
    ctx.lineTo(0, 5);
    ctx.lineTo(7, 9);
    ctx.closePath();
    ctx.stroke();
    ctx.fill();
    ctx.restore();
    scaleLabel.textContent = `${Math.round(states.scale)} m across`;
    const lon = city.region.origin[0] + x /
      (radius * Math.cos(city.region.origin[1] * Math.PI / 180));
    const lat = city.region.origin[1] + z / radius;
    coordsLabel.textContent = `${lat.toFixed(5)}° N, ${Math.abs(lon).toFixed(5)}° W`;
    canvas.setAttribute("data-map-ready", "true");
    canvas.setAttribute("data-map-width-m", String(states.scale));
    canvas.setAttribute("data-player-x", String(x));
    canvas.setAttribute("data-player-z", String(z));
  }
  async function init() {
    try {
      const response = await fetch("./assets/downtown-la.json");
      if (!response.ok) throw new Error("HTTP " + response.status);
      const city = await response.json();
      if (!city.region || !Array.isArray(city.streets)) throw Error("Invalid city file");
      states.city = city;
      paint();
      // Do not redraw polygons at 60fps. Camera update is independent.
      setInterval(paint, 150);
    } catch (error) {
      scaleLabel.textContent = "Map unavailable";
      coordsLabel.textContent = String(error);
    }
  }
  init();
})();
