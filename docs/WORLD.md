# World structure, terrain and geometry

How source geography is transformed into sections, coordinate frames, DEM-aware terrain, roads, building geometry and collision. Source centerlines and footprint massing are **not** surveyed architecture or fully reconstructed transportation infrastructure.

[Documentation index](INDEX.md) · [Project README](../README.md)

---

## Geometry contract

- WGS84 bounding boxes are `[west, south, east, north]`; a region has a WGS84 origin.
- Output is meters relative to the origin: `x` increases east, `z` increases north.
- Roads are tagged centerlines, not surveyed curb edges or navigable pavement meshes. Source node IDs preserve future junction connectivity, and selected tags retain sidewalk, lane, bridge/tunnel and access information.
- Buildings are polygon/multipolygon footprints with holes supported in GeoJSON.
- Height is optional and represents a known reported measurement; **missing height is not silently guessed**.
- Overture `building_part` features are downloaded alongside each region's
  buildings and joined **only** to an already-owned parent by source
  `building_id`. Parts retain independent polygon footprints, measured height,
  `min_height`, `min_floor` and floor counts, and are serialized nested under
  the parent. Missing parent IDs (including those owned by an adjacent region)
  are reported and do not create duplicate independent building masses.
  Duplicate part IDs are suppressed. Older city assets without `parts` work.
- The road/building collision audit now reports source part polygons touching
  each conflict, measured vertical clearances and centerline penetration, so
  a raised over-driveway part can be distinguished from an unsupported guess.
  A raised part is **evidence**, not automatic permission to delete a parent
  building or its road. The renderer still uses original parent massing until
  safe polygon-completeness and parent/part occlusion rules exist.

- Import measured `min_height` as bottom clearance above ground, and optional
  `min_floor` as an integer source floor index, not metric height. A missing
  `min_height` stays `None`; only `HeightPolicy.meters_per_level` may use
  `min_floor` as an explicitly **render-only** approximate clearance.
  Source `height` is the vertical extent **from bottom to roof**, so raised
  roofs render at `min_height + height` rather than merely `height`.
- Raised building volume meshes have side walls starting at the rendered
  base and a downward-facing underside. Ground-based buildings still extend
  foundations below terrain to prevent slope clipping. **Render-only**
  `min_floor` estimates may position visual geometry higher, but cannot open
  a verified walking passage or excuse road/building conflicts: road-clearance
  and walking obstacles keep the footprint conservatively obstructed down
  to ground level until metric `min_height` is available. Aerial collision
  retains at least the displayed upper roof bound.
- If bottom clearance is **measured in meters** and more than 0.6 m, the
  planar overlap audit excludes the suspended volume from the at-grade
  conflict count. Floor-index estimates alone never qualify for this
  exclusion. Unknown-base buildings remain conservative, and 2D overlap
  without vertical data is not resolved by deleting roads or inventing
  elevations. Building parts are not yet independently reconstructed.
- The Downtown Overture extractor must use the actual `num_floors` column:
  `level` means visual ordering, **not** number of floors.
- Mesh generation, terrain, collision, traffic and rendering belong in later stages, using this data as input.
- The data layer retains source IDs and attribution so future updates can reconcile features.

## First adjacent section (October 2026)

The pilot contains independently generated **Downtown LA** and
**Little Tokyo / northern Arts District** sections. Both now render together
by default as a joined scene; `?world=0` selects standalone mode. Each source
section retains its own Overture city JSON, DEM grid, origin, and content-hash
cache key. The renderer preserves the quality and backend choices when
switching modes. The joined mode is a bounded two-region preview rather than
unlimited city streaming.

Source region definitions are enumerated in `regions/catalog.json` and
validated by `scripts/validate-region-catalog.py`. A new region can use
`bash scripts/build-region-overture.sh regions/NAME.json NAME` followed by
`python3 scripts/fetch-terrain.py --region regions/NAME.json --out
data/processed/NAME-terrain.json`. CI acquires and verifies both sections
from the same source pipeline. The catalog records a common geographic world
reference; section-local origins must be transformed to that reference before
true seamless stitching is introduced. No invented continuity or measured
bridge elevations are claimed.

## Joined-world GPU preview (October 17)

The default joined world (`?world=0` selects standalone) renders Downtown LA and
Little Tokyo/northern Arts District together in one WebGPU/WebGL2 scene.
The second section retains its own indexed GPU vertex/index buffers and
collision/terrain state. Vertex coordinates are translated to the primary
region's frame; source DEM absolute elevations correct vertical offsets and
the longitudinal latitude scale is corrected across section origins.
The preview has a hard **two-section residency limit**; a distance check
skips the neighbor's draw calls when it is more than 2.4 km away. It never
allocates GPU memory for all future catalog entries. Switching the preview
off restores the previously tested standalone region renderer.

**Not yet** a completed city streaming system: the preview loads both
sections at startup, keeps both GPU buffers resident, and has no distant
building LOD or billboard impostors yet. Those will follow after seam
geometry, camera traversal and bounded eviction are verified. Earlier versions of
the two regions imported the same cross-boundary building into
both GPU buffers. Footprints now have deterministic half-open geographic
ownership by their source-derived centers, and full roofs and walls remain
intact rather than being cut at tile borders. Catalog-wide source geometry
and DEM seam validation now runs before deployment. Surveyed curb and
intersection geometry remains a source-data limitation.

## Road surface ownership and terrain conformity (October 18)

A recurrent defect in chunk streaming was clipping source **centerlines** to
each 300-meter cell before constructing their ribbon meshes. That lost the
preceding and following bend normals, broke roadway corners, and fabricated
artificial road endpoints at every cell border.

The geometry pipeline now enforces these invariants:

- Retain source-derived street centerline vertices and geographic tags;
  select only local runs with an 80-meter context halo and one neighboring
  control vertex at each end. This preserves the same visible miter normals
  while avoiding repeated meshing of entire long streets in every tile.
- Generate full-width asphalt, walking, steps and rail triangles, retaining
  real source bends, junction topology and material layering.
- **Asphalt always owns overlapping ground-level pedestrian surface area.**
  After the visible road ribbons and junction patches are fully formed, subtract
  the actual road triangle footprints from walking and stair triangles. Split
  and keep their non-overlapping fragments on both sides of the carriageway.
  This is a true mesh boolean difference, not a terrain-Z offset, draw-order
  trick, or whole-path deletion. Exclude hidden freeway ribbons and elevated
  bridge decks, which must not make holes in ordinary footways.
- Use a coarse road-triangle spatial index and preserve original vertex
  indices for untouched pedestrian triangles. Build the cut once on continuous
  source-local surfaces **before** geographic chunk ownership and DEM
  conformance, so intersections cannot reappear when terrain bends or a
  source crosses a chunk seam. Source pedestrian centerlines are retained
  for later route/topology work; this rule governs rendered ground surfaces.
- Import full Overture `level_rules` where `between: null` denotes the
  entire feature. Split fractional `is_covered` intervals before clipping
  the region and retain `covered=yes` on their derived spans. Treat source
  qualified bridges, tunnels, nonzero relative levels and covered roads as
  **not at grade**; a surface-only preview must not flatten them onto the
  same pavement plane as opaque buildings. Preserve original source features
  for the dedicated future 3D grade pass.
- Conform transportation surfaces to the **same DEM triangles** used by the
  rendered ground. Per-vertex height sampling alone is insufficient: a road
  triangle can penetrate a nonplanar DEM between its original vertices.
  Clip fully meshed planar surfaces to geographic chunk ownership **before**
  DEM-face subdivision, so source halo geometry is never terrain-tessellated
  just to be discarded. Reuse indexed vertices in both clipping passes.
- Assign **finished** surface triangles to exact intersection of chunk bounds
  and geographic region bounds. Clip polygons, not source centerlines; do not
  invent borders or delete any partial triangle wholesale.
- Keep whole building roofs and walls owned by one deterministic cell; building
  footprint ownership follows a different rule than roadway surface ownership.
- Unit tests cover bent-road surface area conservation across chunk borders,
  bend-safe source context retention, four-cell area conservation, and DEM midpoint/edge clearance even on
  steep nonplanar synthetic terrain.
- Every region in the catalog must pass a **real-data surface ownership gate**:
  compare every 300-meter cell's road, walk, stair and rail footprint area
  against the corresponding slice of the complete region mesh. Fail when the
  difference exceeds 0.2% plus 0.25 square meters per cell/material. This
  catches coverage gaps and duplicate asphalt introduced by new regions.
- Rust CI and both browser-renderer QA gates must pass before changes to the
  default site. All geometry source modules now trigger browser QA on push.

Unknown-width Overture `service`/`track` roads now use a region-wide building
clearance envelope: generated ribbon edges are allowed to narrow independently
where they would otherwise intersect solid building walls. The original
centerline, all tagged/measured widths, source-road records, and mesh connectivity
are preserved. Courtyard holes are respected; centerlines that actually enter
building interiors stay flagged as ambiguous rather than being silently removed.
Every chunk receives the SAME region-wide footprint context, including buildings
owned by neighboring cells. Full-region road coverage QA uses that same
clearance rule, so chunk seams cannot have different inferred widths.

An additional `audit-road-buildings --city FILE` command measures asphalt
intersection with actual triangulated building footprints (including courtyard
holes). It reports source building/road IDs and geometric overlap areas.
The diagnostic runs on every catalog region in the browser workflow.
It now reports conflict classes: source-centerline penetration into a solid
triangulated building roof, explicitly tagged-width shoulder overlap, and
inferred-width shoulder overlap. The penetration test calculates exact
segment intervals through roof triangles and unions them so shared diagonals
never double-count length, respecting courtyard holes. Each conflict includes
source IDs, affected centerline length and WGS84 focus coordinates. This
separates possible under-building service roads from safe width-envelope
tuning without inventing vertical-grade facts.
This is a **diagnostic, not an automatic repair**: a conflict may indicate
misregistered source data, covered roadway or grade separation. Deleting
either complete surface would introduce new holes. Only at-grade ordinary
roads are assessed; hidden freeways, bridges and tunnels are excluded.

The source geometry is still centerline-derived and has no surveyed curb
boundaries. This eliminates two systemic classes of visual clipping but does
**not** prove that every source-road intersection, building/road conflict,
or joined-region transition is clean. Real-geography browser verification,
bounded frame/GPU memory performance, and recorded visible defects remain
mandatory before calling any road release visually complete.

## Source-backed road conflict inspection (October 19)

The `audit-road-buildings` CLI remains the authority for real road/building
triangle overlap. During packaging, `scripts/publish_road_conflicts.py`
runs that exact Rust auditor on each catalog region and publishes the
**centerline-penetration** cases to
`assets/road-conflict-locations.json`. The Rust audit now exposes the complete
`source_centerline_conflicts` set while retaining `largest` as a capped
top-20 summary. The publisher validates the entire set against the reported
cause count (and rejects truncated older audits), so additional regions or
conflicts cannot silently disappear from the review UI.

**Details → Inspect road conflicts** loads this optional small manifest only
when requested. **Show in 3D** opens the chosen region with
`?region=REGION&world=0&focus=LONGITUDE,LATITUDE`; it uses a reproducible,
elevated Rust camera pointed at the geographic source locus. The loader
validates finite coordinates and region bounds. Selecting a case shows its
original road and building IDs, centerline penetration length and any
building-part evidence, explicitly distinguishing measured raised clearance
from missing evidence. Normal joined-world starts do not request the manifest
or change camera behavior. Source-line reports also preserve observed street
names and distinguish a road terminating inside a footprint from one crossing
through it (strict polygon-interior endpoint tests). That distinction describes
source geometry, **not** whether a surveyed doorway, tunnel or road exists.
No roads, buildings or source tags are changed by this feature.

An overlap is not by itself a mapping error: roads may be internal/covered,
or source geometries may be misregistered. This tool helps inspect the five
currently outstanding centerline cases, not fabricate tunnels or repair source
data without vertical evidence. Browser QA opens the review UI and flies to
one conflict in each catalog region with actual WebGPU and WebGL2 rendering.

### October 19, 2026 source snapshot (commit 8f2b50ef)

The [packaged real-region artifact](https://github.com/sguzman/urbarium/actions/runs/39280235220)
contains **five** centerline/roofprint conflicts: three in Downtown LA,
two in Little Tokyo / northern Arts District. Four are service roads; one is
North Main Street (primary). The largest service road contains **56.405 m** of
source centerline within a parent building footprint, and its source endpoint
terminates inside the footprint; the other four cases have both endpoints
outside their implicated footprint. None of the five has matching
building-part evidence of a measured opening. These are planar source
observations, **not** verified erroneous polygons, drivable passages or
authorization to erase roads. Reacquisition may change the counts.

## Joined-world camera regression fix (October 17)

Joined-world preview now shares the same WASD/arrow/E/Q control-vector
interpretation as standalone mode. A terrain-free phantom movement pass was
incorrectly overwriting eye clearance each frame and adding DEM height
to altitude repeatedly, launching the camera high above the scene. The
single-section mover was always correct; the preview now derives altitude
from its real selected section and holds absolute Y constant in aerial
movement. Flight into higher terrain blocks the horizontal step instead
of raising the camera without explicit ascent. Browser QA holds W/S in
overview mode and asserts no Y drift, then tests explicit E/Q ascent/descent.

## First-person collision and connected street junctions

The current browser demo uses **real building footprint polygons as horizontal
collision obstacles**: the camera starts on an unobstructed source street,
cannot walk through building walls or tile bounds, and slides along walls.
Interior courtyard polygons remain open. Aerial free flight still works when
the camera rises above an estimated or source-recorded roof height.

The transportation generator fills only road intersections at **three or more
connected endpoints**, producing compact asphalt patches from local road widths.
For this pass, candidate endpoint connections must have consistent source OSM
node identity when both provide it; endpoints with no shared node metadata
must agree to within 10 cm, and mixed-metadata endpoints to within 2.5 cm.
Independent OSM node identities never merge just because coordinates overlap.
Artificial source-boundary endpoints are excluded from junction creation.
Synthetic regression tests guard near-miss, shared-node and border cases.
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

## Named buildings and real 3D transport grades

The acquired Overture building features already store optional **names**,
recorded heights and floor counts. An optional **More named buildings**
panel beneath the minimap searches source-provided names in memory and shows
the available source height, number of floors, and original feature ID.
It does not download new metadata and does not pretend every footprint has a
name. **In the October 15 live Overture Downtown LA pilot, zero of the
acquired building polygons have a source-provided name.** The panel is
therefore hidden in this snapshot instead of showing an empty directory.
Extending named-building coverage requires verified additional source data,
not made-up labels. Only nine places have manually sourced landmark annotations; named
buildings not among them are not architectural recreations.
External heritage descriptions and Wikipedia/Wikidata identifiers require
independent source matching and provenance, not guesses.

### Freeway reconstruction reset (October 2026)

**The unfinished freeway corridors are hidden from the default 3D browser
scene**, including the flat motorway/trunk ribbons and edge markings that
remained after removing the raised bridge prototypes. Source building and
terrain geometry, pedestrian paths, ordinary streets and non-freeway bridge
centerlines still render. This is not a geographic claim that freeways are
absent; their source linework remains in scene metadata and the minimap.
Both flat and raised freeway experiments are available only in preview mode.

The default Rust/WASM viewer **does not generate or upload** unfinished
freeway ribbons, edge stripes, freeway junction patches, raised bridge decks,
fascia, rails or pier columns. The browser QA now requires **zero freeway mesh
triangles and zero elevated bridge mesh triangles** in normal mode; the opt-in
preview must generate both. Pedestrian paths are not clipped around hidden
freeways. Geographic bridge metadata and the content-fingerprinted city/DEM
cache are retained.

For development and comparison only, choose **Experimental 3D freeways: on**
(or append `?bridges=1`). This reloads the renderer with the old preliminary
bridge model, highlights source bridge segments in cyan on the minimap, and
enables **Inspect prototype overpass**. Turning the preview off removes the
query parameter and returns to the simpler default. This preference is
deliberately **not saved across ordinary launches**. The opt-in preview
is unfinished and may be visually incorrect. Hidden-default and elevated
preview modes have separate end-to-end tests.

Overture's `level_rules` are a **relative Z-order**, not measured road-deck
elevations. The pilot acquisition is just ~1.5 × 1.2 km and cannot represent
complete nearby freeway interchanges. Rebuilding freeways requires a
connected corridor graph, verified crossing order and bridge coverage,
continuous longitudinal grade profiles, bounds-aware geometry and browser
screenshot validation before the feature returns to the default scene.
See [Freeway reconstruction plan](FREEWAY_RECONSTRUCTION.md).

## Non-goals for the first slice

Do not download all of Southern California, do not check giant data assets into Git, and do not pretend an extruded building is a photogrammetrically faithful facade. Build and QA one area first, then expand by geographic tiles and data quality.

