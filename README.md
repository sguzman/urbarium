# Urbarium

**Real cities, reconstructed from primitives.**

Urbarium is a Rust-first pipeline for geographically faithful, low-poly, walkable cities. The first region is Downtown Los Angeles; the long-term scope includes Los Angeles County and neighboring Southern California counties. The intended interactive runtime is **Rust → WebAssembly → WebGPU**.

## Status

**Data ingestion and first 3D massing build validated on real Downtown LA data.**
Rust CI passes, and a successful live Overture build on October 15, 2026
produced a geographic city JSON and a low-poly 3D building OBJ:

- **1,587** clipped transportation segments (including pedestrian/rail/path features).
- **563** building footprints, of which **537** have recorded height values.
- **563 / 563** imported buildings meshed successfully (zero skipped).
- **38,210** mesh vertices and **21,888** triangles.
- **26** render-only estimated heights for buildings without recorded heights.

- **1,540** transportation spans meshed (472 road, 943 pedestrian, 114 steps, 11 rail); **47** unknown/underground spans intentionally excluded.
- A single combined glTF 2.0 binary (.glb) contains **34,660 triangles** in six layers (reference ground, buildings, roads, pedestrian paths, steps, rail).
- **1,719,448 bytes** total for the uncompressed GLB, independently loaded through a separate glTF-capable 3D library.

**October 15 browser milestone:** headed Chromium on software Vulkan
confirmed genuinely rendered Downtown LA rooftops, streets and building
facades (not merely a running WASM process). The automated pixel check saw
separate sky, building, street and terrain colors and passed keyboard camera
interaction. [Validated build and screenshot](https://github.com/sguzman/urbarium/actions/runs/38725915571).
The shader and geometry are procedural Rust/WebGPU, with no screenshot
substitution or static fake viewport. The dataset remains a small downtown
pilot, not LA County coverage.

The sample covers approximately 1.5 × 1.2 km around Bunker Hill / Civic
Center, not the whole city. The outputs include real indexed 3D geometry and a first
**Rust/WASM/WebGPU flythrough viewer**. Chromium automated QA successfully
loaded the Downtown LA scene and exercised camera movement on October 15, 2026.
The viewer now has footprint-aware walking and a sampled terrain surface; roads
remain centerlines, and fidelity of individual facades is not established.

[Validated real-data build, OBJ and combined GLB artifacts](https://github.com/sguzman/urbarium/actions/runs/38680594189)
(artifact expires after seven days). No source data is committed to Git.

## Quick start

Requires a Rust toolchain and `curl`.

```sh
bash scripts/fetch-downtown-la.sh
cargo run --release -- ingest \
  --region regions/downtown-la.json \
  --osm data/raw/downtown-la-overpass.json \
  --out data/processed/downtown-la.json
```

The pilot bounding box covers roughly 1.5 × 1.2 km around Bunker Hill / Civic Center. The generated JSON contains streets, building outlines, optional recorded heights, and provenance.

For higher-quality footprints you can **replace**, not overlay, OSM buildings with a pre-clipped GeoJSON FeatureCollection:

```sh
cargo run --release -- ingest \
  --region regions/downtown-la.json \
  --osm data/raw/downtown-la-overpass.json \
  --buildings-geojson data/raw/my-buildings.geojson \
  --height-unit meters \
  --source-name "Overture Maps" \
  --source-url "https://docs.overturemaps.org/" \
  --source-license "Check source-level licensing" \
  --out data/processed/downtown-la.json
```

For LA County data, use `--height-unit feet` **only after verifying the HEIGHT field units** on the chosen source. Source permissions must be checked separately. See [data sources and limitations](docs/DATA.md).

## Repeatable Downtown LA data build

To acquire source datasets and generate the local city JSON in one step:

```sh
bash scripts/build-downtown-la.sh osm       # OSM streets + OSM building ways
bash scripts/build-downtown-la.sh overture  # OSM streets + Overture building polygons
```

The second path requires DuckDB and Python 3 and uses a pinned, updatable
Overture release. See [Overture pipeline](docs/OVERTURE.md). Neither download
nor build has been confirmed against live services in this environment.

## Overture-only city build

For a bounded real-world LA sample without Overpass, install the official
Overture Maps Python CLI and execute:

```sh
bash scripts/build-downtown-overture.sh
```

This downloads Overture street segments and building polygons and writes the
same `data/processed/downtown-la.json` city schema. It resolves the current
Overture release automatically and records download time and source in a local
report. This route is the best candidate for the first reproducible data demo.

## Generate actual 3D massing geometry

After acquiring a Downtown LA city slice, Urbarium can now build a portable,
indexed 3D triangle mesh using Rust and `earcutr` (with courtyard-aware roof
triangulation and extruded polygon walls):

```sh
cargo run --release -- mesh \
  --city data/processed/downtown-la.json \
  --out data/processed/downtown-la-buildings.obj
```

The OBJ can be inspected in a conventional 3D tool. This is a **real geometric
output**, not a rendered illustration or the eventual WebGPU application.
The legacy OBJ-building-only export remains flat; a separate road-mesh exporter is available. Known building
heights are used without changes. Missing heights are *visualization-only*
approximations (3.1 m per mapped level, otherwise 9 m); this does not alter
the source city JSON or claim the estimate is real.

## Generate street/path surfaces

The flat-world preview places asphalt above crossing pedestrian ribbons.
The independent footway centerlines are preserved for future sidewalks and
navigation, rather than being mistaken for perfectly aligned painted
road markings. Actual curb edges and elevation remain future data tasks.


```sh
cargo run --release -- mesh-roads \
  --city data/processed/downtown-la.json \
  --out data/processed/downtown-la-transport.obj
```

This outputs actual indexed ribbon geometry plus `urbarium-transport.mtl`
with distinct materials for roads, walking paths, stairs, and surface rail.
The ribbons have limited joins to prevent huge spikes at bends. Widths are
only measured when explicitly tagged; all other widths are **visual
defaults**, not surveyed right-of-way or sidewalk edges. Underground subway
and unknown segments are deliberately excluded from the surface mesh.
Connected junction patches and first-pass bare-earth terrain are implemented in the viewer; accurate crossings, retaining walls, curb outlines and complete navigation are **not** yet solved. Keep road and building OBJ files together when inspecting
the combined pilot model.

## Compact combined 3D scene (.glb)

```sh
cargo run --release -- glb \
  --city data/processed/downtown-la.json \
  --out data/processed/downtown-la.glb
```

This creates a **single glTF 2.0 binary** containing a flat reference ground
plane, building massing and categorized road/walk/rail/step meshes, retaining
source attribution in scene metadata. Geometry and normals are stored as 32-bit
binary values rather than verbose OBJ text. Colors are simple neutral PBR
materials; this standalone GLB export still has flat terrain and no textures or
collision, unlike the browser's separate DEM-aware geometry pipeline. Geometry remains generated in Rust; a future
WebGPU/WASM runtime will use the same geometry pipeline.

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

## GitHub Pages deployment

The [browser demo workflow](.github/workflows/browser-demo.yml)
acquires the bounded Downtown LA data, compiles Rust to WASM, verifies the
Trunk-generated static site, runs Chromium/Playwright camera and render
initialization QA, and then uploads/deploys the **tested** `dist/` site using
GitHub's official Pages actions. The deploy job runs only after QA succeeds
on `main`. Nothing under `data/raw`, `data/processed`, or `web/assets`
needs to be committed to Git.

Expected site address: https://sguzman.github.io/urbarium/ .
The verified Pages artifact was accepted by GitHub Actions on October 15,
but the site was still returning HTTP 404 from the public CDN. **Pages must
be configured with Settings → Pages → Build and deployment → Source →
GitHub Actions**, rather than "Deploy from a branch". The latter runs a
separate default repository/Jekyll publish job rather than the validated
Trunk/WASM bundle. After the source is corrected, rerun
"Build and test browser demo" from the Actions tab to publish and verify
the actual site.

## First-person collision and connected street junctions

The current browser demo uses **real building footprint polygons as horizontal
collision obstacles**: the camera starts on an unobstructed source street,
cannot walk through building walls or tile bounds, and slides along walls.
Interior courtyard polygons remain open. Aerial free flight still works when
the camera rises above an estimated or source-recorded roof height.

The transportation generator fills only road intersections at **three or more
connected endpoints**, producing compact asphalt patches from local road widths.
Simple centerline crossings are not automatically promoted to intersections:
bridge, tunnel and grade separation remain unresolved without better data.

**Sidewalk fidelity:** The 2026-10-15 Overture pilot contains 912 pedestrian
`footway` segments, but none identifies a surveyed sidewalk-edge polygon;
these are pedestrian centerlines, not actual curb boundaries. Generating
parallel sidewalks around every street would invent geography, so sidewalks
remain a separate source acquisition and geometry task. The new terrain
layer is sampled from georeferenced bare-earth DEM tiles, without claiming
3DEP lidar precision or surveyed sidewalk/bridge elevations.

## Downtown LA terrain relief from real elevation tiles

The first DEM-enabled browser scene uses a **65 × 65 georeferenced elevation
grid** from [Mapzen Terrain Tiles](https://registry.opendata.aws/terrain-tiles/),
decoded from Terrarium RGB values. The verified Downtown LA source has about
**50.25 meters of relief**, with an origin elevation near 85.17 meters.

```sh
python3 -m pip install Pillow
python3 scripts/fetch-terrain.py --region regions/downtown-la.json --out data/processed/downtown-la-terrain.json
```

The same local-meter reference drives terrain mesh vertices, road/path
ribbons, flat-roofed building massing foundations, and the walking camera's
1.7-meter eye clearance. Aerial movement remains available above rooftops.
The separately exported OBJ/GLB scenes still use their original flat ground
until terrain support is extended to those formats.

The DEM is genuine geographic data, not artificial terrain noise, but it has
heterogeneous upstream resolution and is **not yet high-resolution 3DEP lidar**.
Bridge and tunnel decks, stair elevation transitions, retaining walls, and
actual curb edges need distinct treatment and better source data.
See [Mapzen's Terrarium encoding](https://github.com/tilezen/joerd/blob/master/docs/formats.md)
for the height decoder.

## Persistent geography cache and refresh

Urbarium now stores the *successfully downloaded* Downtown LA city and
elevation JSON in browser CacheStorage on the first visit. A cache hit is
returned before Rust starts parsing and meshing, so refreshing the page does
**not** redownload both geographic assets. Cached data survives normal page
reloads, browser restarts and application redeploys (subject to the browser's
storage eviction policies). The minimap still consumes the Rust viewer's
validated shared scene; it does not download a second copy.

Application scripts, Rust WASM and the page are **not** pinned in this
geography cache, so deploying a viewer update still loads current code.
The launch link opens the same application origin and does not itself erase
any geographic cache. A brand-new browser profile/private window and a
browser eviction can cause geographic data to be downloaded again.

The HUD now says **City: CACHE HIT / NETWORK DOWNLOAD / downloaded + saved**
and the equivalent for **Terrain**, plus measured elapsed startup time and
the current phase (fetch, JSON decoding, 3D geometry generation, GPU setup,
first frame). This distinguishes a genuine network download from slower
local Rust/WASM parsing or GPU initialization. It does **not** mean the
browser can guarantee offline access forever or that a hung GPU can always
be recovered inside a page. The old meaningless "cache ready" label has
been removed.
Use **Refresh geography** in the HUD when explicitly wanting new downloaded
city/terrain data. Clearing site data also clears the geographic cache.
Future geographic region/schema releases must change the cache namespace;
the default remains stable so merely updating the viewer never forces an
expensive redownload. Browser cache availability is best-effort, not a promise
of permanent offline storage.

## Named buildings and real 3D transport grades

The acquired Overture building features already store optional **names**,
recorded heights and floor counts. Only nine places have manually sourced
landmark annotations; named buildings not among them can be discovered from
the building inventory without calling them full architectural recreations.
External heritage descriptions and Wikipedia/Wikidata identifiers require
independent source matching and provenance, not guesses.

The road renderer's meshes are truly 3D, but the current road geometry is
still **terrain-draped**. Even freeway centerlines render at grade.
Overture transportation offers `level_rules` (relative stacking order),
`road_flags` (including `is_bridge`), `width_rules`, and routing connectors.
The GeoJSON importer now preserves these source attributes for future
grade-separated decks, sloped ramps, support structures and correct
over/under crossing handling. The presence of a level is **not** a surveyed
height in meters. Actual elevated highway geometry, pillars, collisions and
connectivity-aware ramp grades are **not yet implemented**. Correctness must
be validated against the source tags and ideally higher-resolution elevation
or city infrastructure surveys before geographic expansion.

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
The category-color toggle persists locally, while manual time does not.

The nine pins/labels and these simple color-coded markers do **not** mean
the actual landmarks are finished as distinctive high-fidelity structures.

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

## Rendering overlays, crashes, and performance diagnostics

Urbarium now treats the transportation categories as independently renderable
GPU batches. Ground-level asphalt is raised above approximate pedestrian
strips to prevent coplanar clipping; the rail preview is **hidden by default**,
because rail/light-rail centerlines are not surveyed bridge, trackbed, or
grade-separation surfaces. Use **Footpaths** and **Rail** switches in the
main HUD to compare the source geometries directly. This does not imply
perfect sidewalk/crossing topology or measured bridge elevations.

Click **Stats: off** in the HUD to open a performance panel. It reports
observed presented FPS, frame interval, **CPU-side frame work** (which does
not include asynchronous GPU work), rendered triangles, draw calls, and
allocated GPU mesh buffer bytes. The browser may additionally expose its
JavaScript heap size. Whole-system CPU utilization, GPU utilization, true
VRAM allocation, and GPU timestamps are not exposed reliably on normal
public browser pages, so Urbarium explicitly reports them as N/A rather
than inventing measurements.

The same panel captures JavaScript exceptions, unhandled rejections, and
WebGL context-loss notifications in the session, including a last-error
message across page reloads. A full tab or browser process crash cannot
always be observed by JavaScript. WebGPU remains experimental for some
drivers; **WebGL2 is the default** until hardware-specific WebGPU
presentation bugs are resolved. Browser CI tests both backends and verifies
that real geometry and performance counters continue advancing over time.

## Geometry contract

- WGS84 bounding boxes are `[west, south, east, north]`; a region has a WGS84 origin.
- Output is meters relative to the origin: `x` increases east, `z` increases north.
- Roads are tagged centerlines, not surveyed curb edges or navigable pavement meshes. Source node IDs preserve future junction connectivity, and selected tags retain sidewalk, lane, bridge/tunnel and access information.
- Buildings are polygon/multipolygon footprints with holes supported in GeoJSON.
- Height is optional and represents a known reported measurement; **missing height is not silently guessed**.
- Mesh generation, terrain, collision, traffic and rendering belong in later stages, using this data as input.
- The data layer retains source IDs and attribution so future updates can reconcile features.

## Non-goals for the first slice

Do not download all of Southern California, do not check giant data assets into Git, and do not pretend an extruded building is a photogrammetrically faithful facade. Build and QA one area first, then expand by geographic tiles and data quality.

## Plans

See [roadmap](docs/ROADMAP.md). Data is stored only on the developer's machine under ignored `data/raw` and `data/processed`; code is MIT, **source datasets retain their separate licenses**.
