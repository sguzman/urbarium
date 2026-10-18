# Startup, caches, stability and performance

Loading stages, progressive geometry, independently versioned geographic data, WASM caching, graphics safety controls and performance diagnostics. Browser storage is best-effort; a successful CI run does **not** guarantee any particular performance on hardware-specific Edge or graphics drivers.

[Documentation index](INDEX.md) · [Project README](../README.md)

---

## Startup delay diagnosis (October 17)

Opening the `Details` tab does not initiate any HTTP request or restart
WASM. The user-reported `Starting application · 27s` message was a
**mislabeling** of an unrecognized `neighbor-fetch` stage in the opt-in
joined-world path. At that stage the viewer may fetch/cache-hit the second
region, deserialize geography and build another CPU mesh before GPU startup;
the stage label alone cannot distinguish how much time each consumed.

The Details panel now displays the recognized neighboring-section stage and
**current-stage seconds versus total startup seconds**, plus independent
metrics for (1) city/terrain CacheStorage network requests during this visit
and (2) browser-reported WebAssembly resource transfer size, duration, and
completion time. A `CACHE HIT` confirms geography was reused; it does not
mean the Rust/WASM module or CPU mesh was cached. A zero browser-reported
transfer size is ambiguous: disk cache and unavailable timing details cannot
be distinguished in every environment. A Resource Timing entry only exists
after that request completes, so a missing entry does **not** prove no
download started. The JSON renderer report now contains `startup` telemetry.
These diagnostics do not currently shorten the first frame time or guarantee
that startup is exclusively CPU-bound; optimization comes after identifying
the expensive stage.

## Progressive first frame (default, October 17)

The Advanced tab now exposes `Progressive first frame`, enabled by default.
Rather than blocking GPU initialization until both complete city sections
have been triangulated, it renders real source-derived **terrain and draped
streets** first. Only after that first GPU presentation does the browser
build the full primary buildings, labeled landmarks and final roads, then
fetch/build the adjoining section when joined mode is enabled. GPU vertex
and index buffers are swapped on the event loop between frames; the camera
position and backend do not restart, and the earlier buffers are released.
The Details panel indicates `base`, `complete`, `neighbor` or `joined`
enrichment. Standalone non-progressive mode remains available using `?progressive=0`
(or disabling the Advanced toggle) as a compatibility fallback.

This is real geometry staging, not a screenshot or billboard. The initial
meshing pass currently repeats the road triangulation in the full pass:
further optimization can partition by camera-visible chunks and avoid work
for off-screen buildings. It also cannot eliminate a slow initial WASM
transfer: no Rust code runs before the WASM module arrives and starts.

Release builds now run `wasm-opt -Oz` with the module's required
bulk-memory and nontrapping float-conversion features enabled. The
uncompressed WASM decreased from 4,672,314 to 3,932,014 bytes (15.8%).
This reduces code bytes transferred on a cold load, but does not prove any
particular time-to-first-frame improvement on the user's connection.

## Conservative view-frustum draw-call culling (October 18)

The normal single-region streaming renderer keeps the complete Downtown pilot
GPU-resident (all 30 mesh cells), including tall buildings and distant skyline
blocks. The frustum test uses **actual chunk mesh vertex AABBs**, never just
nominal 300-meter grid extents, and expands each clipping plane by a
**30-meter world-space safety margin**. Entire chunks are skipped only when
their full bounds lie outside a camera plane plus the margin. Buildings hidden
behind other buildings are not removed; no unsupported occlusion assumptions
are made. Rotating the camera makes resident meshes drawable immediately,
without downloading, generating, or uploading their geometry again.

Use `?frustum=0` to disable only draw-call filtering while retaining exactly
the same GPU-resident city. The Details view and downloadable diagnostics
record draw-visible and rejected chunk counts separately from residency.
Browser regression compares canvas pixels with frustum culling enabled and
disabled at the **same fixed camera pose**, and checks the full skyline from
the elevated overview. Joined-world and experimental freeway modes remain
untouched. View-frustum filtering does not reduce initial mesh construction
or WASM transfer time.

## Camera-following GPU mesh cells (October 18)

In the normal single-region progressive browser mode, Urbarium retains one
complete terrain mesh but builds **300-meter geometry cells on demand**.
Each visible-neighborhood cell includes source-derived building meshes,
clipped transportation surfaces, and corresponding landmark markers.
The **initial nine-cell radius was visually unacceptable**: elevated views
showed entire sections of buildings disappearing beyond a few hundred meters.
This was corrected by keeping the entire bounded Downtown LA pilot skyline
resident. The desired-cell calculation now includes all overlapping cells
within a conservative 3 km distance, capped at 96 GPU-resident cells for
future larger regions. Cells are constructed one at a time in camera-proximity
order, but the full pilot extent remains available from any camera position.
A camera relocation inside the pilot does not evict still-visible distant
buildings. Large future regions may still use eviction outside the conservative
distance and will require view-frustum-aware LOD and occlusion handling.

The original geographic city and collision data remain resident in CPU memory.
Conservative coverage is intentional: **true occlusion culling has not yet
been implemented**, so no building is hidden merely because its cell is outside
a small local radius.

The initial coarse terrain/road preview is replaced with terrain-only after
the first active neighborhood is complete, avoiding permanent duplicate road
draws. One new chunk is generated per rendered frame; this can still create
main-thread CPU hitches, and true off-thread generation and LOD are future
work. Single-region streaming is enabled by default with progressive startup,
with `?stream=0` restoring the previous full-city enrichment. The
joined-world preview and experimental freeway mode retain the full-scene path
until their topology and memory behavior are separately verified. Browser QA
checks camera relocation, chunk eviction, a maximum of nine residents, and
the previous renderer/fallback behavior.

## Camera-local street preview (October 18)

The progressive first pass now retains only source road/path polylines that
intersect a **450-meter radius around the collision-aware spawn camera**.
Long polylines crossing that radius remain intact even if both endpoints lie
outside it. This reduces the first pass's road triangulation and GPU upload.
The full-detail enrichment still rebuilds and loads all road and building
geometry, and the original geographic city JSON and collision data remain
unfiltered. Full scene startup (`?progressive=0`) remains unchanged.

This is **not** dynamic chunk streaming, camera-tracking mesh paging,
background-thread meshing, or a proven speedup on the user's connection.
The first-frame subset and complete source count are exposed as
`data-first-frame-streets` and `data-source-streets`; browser QA verifies
the first is smaller and nonempty while full scene enrichment still succeeds.

## Persistent WASM executable cache (October 17)

The viewer now registers a scoped service worker (`wasm-sw.js`) without
blocking the first frame. When it controls the page, it intercepts **only**
Trunk's content-fingerprinted `urbarium-viewer-*_bg.wasm` requests. It returns
a CacheStorage hit without a network download, or streams a network response
immediately and saves a clone asynchronously for the next visit. At most the
two latest WASM revisions are retained. All HTML, CSS, generated JavaScript,
geography JSON and other requests continue through their normal paths;
geographic JSON retains its separate SHA256 cache.

The Details tab distinguishes an observed disk cache hit from a fetch whose
Resource Timing transfer count is merely zero. The diagnostic JSON includes
`startup.wasmCache`. On the first visit the service worker may not yet control the document;
therefore the page also intercepts the first WASM response and saves a
clone asynchronously without waiting before 3D startup. Production builds
remove Trunk's eager WASM preload, which otherwise bypasses that first
request interception. The generated module bootstrap remains unchanged. Its next reload
can then use the same binary directly from the service worker. Browser storage eviction, private mode or disabled service workers
may require another download. This helps repeated WASM transfers, **not**
first-ever WASM compilation or full-city geometry generation.

## Persistent geography cache and refresh

Urbarium caches *successfully downloaded* city and elevation JSON with
individual, immutable SHA-256 fingerprints of the **actual generated asset
bytes**. The release workflow stamps those fingerprints into the page before
the WASM build. A cache hit only counts when the exact fingerprint matches;
any older URL-only response from the previous cache implementation is
**never reused**. Matching files survive app updates/reloads without a second
geographic download. When actual geography changes, only the changed asset
is fetched once under a fingerprinted request URL. Older copies are then
evicted from CacheStorage. The minimap consumes the viewer's loaded scene
instead of fetching geographic data a second time.

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
be recovered inside a page. Lost or outdated GPU swapchain surfaces now
force reconfiguration even if the browser viewport dimensions are unchanged;
this fixes a deterministic recovery bug but does not prove all
hardware-specific crashes have been resolved. The old meaningless "cache ready" label has
been removed.
Use **Refresh geography** in the HUD when explicitly wanting new downloaded
city/terrain data. Clearing site data also clears the geographic cache.
Geographic content changes automatically invalidate only their own old
cache keys; updating Rust/WASM without changing geographic bytes does not.
The cache namespace can stay stable even as the importer evolves. Missing
release fingerprints fail safely by **bypassing persistent cache** instead
of returning stale, potentially incompatible scene data. Browser storage
remains best-effort, not a guarantee of permanent offline access.

## Browser stability and pedestrian overlap handling

The WebGL2/WebGPU browser event loop now waits for scheduled redraws instead
of continuously busy-polling, and suspends city rendering when the tab is
hidden (resuming when visible). Fullscreen GPU backing resolution is bounded
to approximately 1.8 million pixels even on HiDPI monitors; the CSS canvas
still fills the viewport. This limits color/depth attachment pressure but
**does not establish that user-specific Edge crashes are solved**. Chromium
automated testing cannot replicate every user hardware/driver combination.
A lightweight local heartbeat now retains the last backend, startup phase,
frame count, framebuffer size, bridge count, asset fingerprints and recent
script error. The renderer now also installs wgpu callbacks for **device
loss and uncaptured GPU errors**, which are reflected in the local diagnostics
when delivered; frame submission stops when the device is lost. A WebGL
context-loss event is likewise recorded immediately in local storage. If
Edge or its GPU process terminates without a normal page-hide event, the
next viewer launch displays a **possible unexpected termination** under
Stats, with a **Copy diagnostics** control. It cannot differentiate every
browser crash from a forced close, and cannot capture errors when the entire
process dies before delivering a callback. No telemetry is sent to a server.

**Rendering resolution** now offers High (default), Balanced, Low and
experimental Adaptive. Resolution selection changes the actual GPU framebuffer
without a page reload or geographic re-download. Adaptive starts at High,
reduces resolution after sustained frame rates below 42 FPS, and cautiously
restores it above 56 FPS. This is a frame-interval heuristic, **not measured GPU
time**, and cannot fix CPU- or driver-bound stalls. The Stats panel reports
GPU adapter identity (where exposed), device type, selected surface format,
color transfer and actual output resolution. The WGSL shader now converts
sRGB display colors to linear light on sRGB attachments, rather than
accidentally applying a second gamma transfer. Browser and driver color
management can still differ beyond the shader's control.

**Conservative GPU** can be enabled from the viewer. It reloads only the
viewer using a 640k-pixel drawing buffer and an approximately 30-FPS frame
cap, retaining the content-hash geographic cache. This is a diagnostic
A/B test and possible GPU-pressure workaround, **not a confirmed Edge crash
fix**. The next requested evidence after a crash is: reopen the page,
expand Stats, select Copy diagnostics, and share the resulting report.

Source footpaths are thin centerline ribbons, not surveyed sidewalk polygons.
The importer now removes pedestrian mesh triangles intersecting a buffered
at-grade road corridor using a 24-meter spatial hash. This prevents sidewalks
and footpaths from visibly punching through asphalt. Source-tagged
elevated roads and tunnels are excluded from that clipping, so their
underlying pedestrian paths remain visible. Real pedestrian crossings
and exact curb geometry require better source data and are not inferred.

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

