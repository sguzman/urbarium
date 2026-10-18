# Browser viewer and interaction

Controls, display behavior, minimap, landmarks and other browser-facing features. Dated sections retain implementation context and known fidelity limits. **WebGL2 is the compatibility default**; WebGPU is selectable.

[Documentation index](INDEX.md) · [Project README](../README.md)

Camera-following mesh chunks are the default in normal single-region
progressive mode. The entire Downtown LA skyline now stays resident,
including distant buildings visible from the elevated overview camera.
Details reports resident/built/evicted cell counts; use `?stream=0` for the
historical full-city enrichment path.
Joined Downtown LA + Little Tokyo is the **default**. Use `?world=0` only
for standalone region diagnosis. Joined mode uses the separately verified
full-section renderer; the experimental freeway preview (`?bridges=1`)
remains opt-in. The minimap fits both regions, draws their real source
geometries in the same geographic frame, and correctly locates the
camera marker when zoomed out.

---

## Experimental WebGPU browser viewer

A Rust/winit/wgpu viewer is now implemented in `src/bin/viewer.rs`.
It generates GPU buffers from the same Rust `mesh_city` and `mesh_streets`
functions used by the OBJ/GLB exporters, without a second JS mesh generator.
The browser selects native browser WebGPU when supported and automatically
falls back to WebGL2 if WebGPU is unavailable. Automated Chromium QA must
render visible pixels through both paths before publishing to GitHub Pages.

The browser loads `assets/downtown-la.json` and supports WASD, Shift, E/Q,
pointer-locked FPS mouse look, with a 1.7 m eye-height spawn on a mapped Downtown LA roadway. Click the viewer once to capture the mouse; Esc releases it. Automated Chromium QA has passed canvas startup, WebGPU rendering
initialization and WASD input. DEM relief, footprint collision and first-pass
connected-junction patches now exist; full physics, curb surveys and grade separation remain pending.

For a local browser bundle, install the `wasm32-unknown-unknown` Rust
target and Trunk v0.21.14, generate `data/processed/downtown-la.json`
using the existing Overture workflow, then copy that city JSON to
`web/assets/downtown-la.json` and use `trunk serve --open`. The
`web/assets` folder is intentionally excluded from Git. CI checks
native and wasm compilation separately.

## Compact tabbed viewer HUD (October 17)

The main left overlay is now a narrow, scroll-bounded tabbed panel:
**Explore** (camera, area, joined world, layers, sky), **Graphics**
(renderer, resolution, GPU safety), **Details** (cache status, loading
phase, GPU diagnostic report, performance Stats), and **Advanced**
(experimental freeway geometry). Only one section is visible; tabs use
proper keyboard-accessible tab/tabpanel roles and preserve their selection
across same-session reloads. Switching tabs is entirely in-page and does
not restart WASM, redownload geography or reset the camera.

Existing geography SHA256 cache hit semantics are unchanged. A deployment
that genuinely changes the Rust/WASM binary can require a new fingerprinted
WASM download, and WASM compilation can take time even when bytes were
previously cached. This should not be confused with repeated downloads of
unchanged city and DEM JSON files.

## First-person fast travel and geographic minimap

- Press **Shift once** to toggle **fast travel** (~61.2 m/s, 18× walking);
  press Shift again to return to a 3.4 m/s walk. You do not need to hold Shift.
- The top-right, north-up **Downtown LA minimap** uses the same 563 source
  footprints and 1,587 transportation centerlines as the viewer. Your
  live position and facing arrow track Rust's camera, and **+/−** or the
  map's mouse wheel change zoom. Its coverage is currently the bounded
  Downtown LA pilot, not the entire county.
- Roads are subdivided along their length and width before DEM draping,
  preventing broad asphalt quads from cutting through hills. The height
  sampler is now consistent with the terrain's actual GPU triangle split,
  unlike the prior bilinear-vs-triangulated discrepancy. This improves
  terrain intersections but does **not** turn roads into surveyed surfaces
  or resolve bridge elevations.

## October 17 city viewing improvements

The initial ground-level camera can place most building geometry outside
the visible region, especially with the large control overlays open. The live
viewer now offers **City overview**, which places a camera in the actual pilot
bounds above the source terrain and faces the map center, and **Street view**,
which returns to the original 1.7 m road-center spawn. Neither action rebuilds
meshes, reloads geography, changes the GPU backend, or creates fake buildings.
The fixed bottom-right **Hide panels / Show panels** button hides the HUD,
minimap, and credits without pausing the 3D renderer, making visual inspection
possible even on smaller screens. Browser QA covers these controls.

## October 17 flight altitude and pedestrian asphalt cleanup

Aerial camera motion no longer inherits the terrain's slope. The earlier
implementation updated eye elevation to `ground(x,z) + eye_clearance` on
every horizontal movement, so forward travel over a downslope lowered the
camera and reversing direction raised it. Now walking at 1.7 m continues
tracking ground while free flight (E/Space to ascend, Q/Ctrl to descend)
holds absolute world-space altitude across horizontal movement. To avoid
clipping below the DEM, encountering higher terrain can still raise the
camera to the 1.7 m safety floor; descent to ground returns to walking.

At-grade pedestrian path cleanup now tests the entire walking triangle
against buffered driving-road centerline segments, including triangle
edge crossings and contained short road segments. The old vertex-only
test missed triangles whose corners were outside asphalt even though their
faces crossed into it. Elevated roads, tunnels, and default-hidden
freeways remain excluded from the cutout, avoiding fake holes at flyovers.
These changes are covered by geometry and camera regression tests.

## Landmark categories and time-of-day sky

Landmarks are **color coded by category by default**, with large high-contrast
colored minimap pins and matching category-colored outlines on world-space
name billboards. The name labels also show the category explicitly. There is
no separate global colors checkbox to enable this. An **interactive legend
directly below the minimap** shows each category with its distinctive
color and actual location count. Click a category to toggle visibility of
all its map pins and world-space labels. **All / None** buttons restore or
hide all landmark categories at once. Selection persists in localStorage
under `urbarium-landmark-filter-v3`. The player arrow is always visible.

Rust 3D landmark markers are filtered using the same category bitmask, and
any source building containing a matching geographic landmark anchor is
tinted only while that category is enabled. Untagged buildings are unaffected.
The minimap follows the same footprint containment rule. Parks and other
sites without a matching building have small, explicitly **representative**
ground discs; these are not surveyed park outlines or custom architecture.
The filtered legend shows the actual count per landmark category.
There are nine locations total: Stanley Mosk Courthouse is the **only Legal**
landmark; Los Angeles City Hall is Civic, Central Library is Library,
Walt Disney Concert Hall and The Broad are Culture, Grand Park is Park,
Angels Flight is Transit, Grand Central Market is Market, and the Bradbury
Building is Historic. A category is a semantic site type, not a legal
permission claim.

The **Sky** displays a procedural camera-oriented gradient and sun position.
On every page load it defaults to **Live · LA**: clock time comes from the
`America/Los_Angeles` IANA time zone, including daylight-saving transitions.
A fractional local hour is updated every animation frame so the sun moves
continuously in real time rather than jumping every 15 minutes. The rendered
clock label refreshes every second; the wall clock is resynchronized every
10 seconds and whenever the tab comes back into focus. Dragging the slider
switches to a frozen manual-preview time. **Resume LA** returns to automatic
live time. Switching renderer or refreshing the page defaults to live again.

Neither the sun position nor ambient lighting is a seasonally/astronomically
accurate solar model; building shading still uses a fixed directional light.
The category filters persist locally, while manual time does not.

The nine pins/labels and these simple color-coded markers do **not** mean
the actual landmarks are finished as distinctive high-fidelity structures.

## Live landmark catalog and smooth locator

Nine **georeferenced** Downtown LA landmarks are now included as a
source-attributed catalog with WGS84 coordinates, source URLs and stable IDs.
Examples include City Hall, Stanley Mosk Courthouse, Walt Disney Concert Hall, The Broad, Grand Park,
Grand Central Market, the Bradbury Building, Angels Flight, and Central Library.
Pins appear on the minimap and world-space labels project over the 3D scene
from their real coordinates. The landmarks are **labels anchored in the
geographic world, not custom landmark meshes or surveyed architectural
reconstructions**. Actual high-fidelity building models are a subsequent pass.

The minimap now separates two workloads: a cached 3× viewport basemap made
from the real road/building data, regenerated only when zoom or camera travel
requires it, and a lightweight player marker updated every animation frame.
The Rust viewer exports a compact live pose attribute every frame rather than
a set of camera attributes once per eight frames. This removes the intentional
150 ms stepping and scales better with source geometry complexity.

Coordinate records: `web/landmarks.json`. Refer to each entry's `source_url`
for upstream OpenStreetMap/Wikidata-derived geographic coordinates. These
small annotation records **are stored in Git**; the much larger raw/processed
GIS geometry and DEM bundles remain generated assets excluded from Git.

## Reliable minimap loading

The Rust WASM loader now publishes its already-decoded source city and
terrain payloads through small browser events. The minimap consumes these
directly **instead of issuing duplicate geographic downloads**. This prevents
the previous "TypeError: Failed to fetch" error when the viewer had
successfully loaded the scene but the minimap's redundant request failed.
The map listens for future city/terrain-ready events, so an extended load
does not permanently strand the map in an error state. The in-map ↻ control
provides an explicit independent retry if the shared payload is unavailable.
Optional landmark metadata retries separately and never blocks base-map
drawing. The source JSON's content is not committed; the handoff happens
within the browser. Browser CI requires exactly one city and one terrain
request across the 3D viewer and minimap.

## Fullscreen viewport and stable landmark labels

The browser's 3D canvas now starts at the actual viewport dimensions instead
of the previous hard-coded 1280×800 buffer. Window-resize events update the
winit window, GPU surface, and depth texture together. CI verifies both a
large 1664×926 browser viewport and another 1120×740 live resize.

World landmark names are upright HTML overlays projected using the same
camera handedness, position and 68-degree field of view as the Rust renderer.
They update while moving or turning and are hidden **only** when genuinely
outside the camera's view, behind it, or over 1.8 km away. There is no
turn-time suppression or nearest-three arbitrary disappearance rule.
Minimap pins remain north-up and available regardless of camera rotation.

Geographic coordinates use **X east, Y up, Z north** (left-handed). Both
the 3D camera and the minimap now follow compass bearings clockwise from
north: moving the mouse right increases yaw, moving left decreases it, and
D strafes east while facing north. CI checks projected east-to-screen-right,
geographic strafing, and minimap-arrow synchronization with Rust's yaw.

