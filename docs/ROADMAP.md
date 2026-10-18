# Roadmap

## P0 — Data slice (this bootstrap)

- [x] Rust library schema for buildings, roads, provenance and region-local metric coordinates.
- [x] Bounded Overpass acquisition script for Downtown Los Angeles.
- [x] Import OSM road and building ways, preserving missing survey heights.
- [x] Import optional GeoJSON footprints with multipolygons/holes and explicit height units.
- [x] Import Overture/GeoJSON street LineString and MultiLineString segments independently of OSM.
- [x] Unit tests and CI definition.
- [x] Optional Overture GeoParquet-to-GeoJSON data acquisition script with release provenance.
- [x] Synthetic end-to-end smoke tests for OSM and GeoJSON ingestion (CI configured).
- [ ] Execute and validate a real Downtown LA Overpass download (public instance currently rejects cloud client).
- [x] Add Overture-only bounded street and building download pipeline.
- [x] Run and validate Overture-only acquisition against real Downtown LA (2026-10-15: 1,587 clipped transportation segments; 563 buildings; 537 height values).
- [ ] Capture provenance, source snapshots, and coverage metrics as a reproducible manifest.
- [x] Assemble OSM building multipolygon relations with multi-way rings and interior courtyards.
- [x] Clip OSM and Overture transportation centerlines at region bounds, including long external rail segments.
- [ ] Add robust road topology and relation QA diagnostics; canonical tile ownership for whole buildings.

## P1 — Geographic fidelity

**Current release gate:** experimental 3D freeway geometry is intentionally
**retired from the default browser viewer**. Default retains source road
centerlines as terrain-level reference, without bridge slabs or GPU bridge
batches. The prior model remains accessible only through explicit
`?bridges=1` preview for diagnosis. It must not return by default before
the connected-corridor topology, continuity, visual QA and performance
criteria in [FREEWAY_RECONSTRUCTION.md](FREEWAY_RECONSTRUCTION.md) pass.

- [x] Retire disconnected raised-road fragments from the normal scene and
      replace bridge-tagged gaps with regular draped road strips and markings.
- [x] Gate expensive bridge geometry generation/upload behind an explicit,
      non-persistent, user-visible experimental preview switch.
- [ ] Acquire corridor-sized source transportation data beyond the pilot and
      report connectivity/missing-source endpoints.
- [ ] Build verified connector topology + elevation profiles for freeways;
      review actual interchange screenshots before restoring raised road
      geometry to the default view.


- [ ] Ingest versioned Overture building and building-part datasets with tiled streaming.
- [ ] Evaluate 2020/2023 LA County GIS outlines, units, coverage, terms, and permitted redistribution.
- [ ] Merge/conflict resolution keyed by source feature IDs, with geometry overlays and QA reports.
- [x] Add first-pass at-grade T/cross junction patches using shared centerline endpoints and local widths.
- [ ] Acquire surveyed sidewalk geometries, improve road width/curb edges, and handle footpath crossings.
- [x] Acquire and validate a bounded real Mapzen elevation DEM (50.25 m Downtown LA pilot relief) with georeferenced provenance.
- [x] Integrate DEM-based Rust terrain mesh, road draping, building base offsets and first-person camera ground height.
- [ ] Upgrade elevation source to high-resolution 3DEP where available; fix bridge/underpass decks and retaining walls.
- [x] Preserve Overture transportation bridge/tunnel flags, visual level rules and road width rules during GeoJSON ingestion without inventing metric elevations.
- [x] Split centerlines at source-scoped bridge/level transitions **before geographic clipping**, assigning separate ground/bridge tags per section.
- [ ] Recover Overture connector-aware vertical continuity and crossing rules; validate bridge spans against source geometry and ramp gradients.
- [x] Generate initial source-qualified elevated freeway/bridge decks with genuine vertical mesh geometry, slab soffits/fascia, side barriers and approximate support columns; default render layers remain separate from at-grade junctions.
- [x] Improve road continuity within the pilot: only source-matched at-grade end connections receive smooth visual approaches; keep same-level bridge joins elevated and boundary clips open. Distinguish all motorway-class carriageways and paint edge markings.
- [ ] Join freeway ramps across verified Overture connector graph, add physically realistic longitudinal bridge elevation profiles, pier placement, median/guardrails, lane counts, multi-level interchange geometry, tunnels, surveyed clearances, and deck collision.
- [ ] Extend geography beyond the 1.5 × 1.2 km Downtown pilot when ready to show highways entering/leaving the slice rather than terminating at dataset boundary.
- [ ] Add optional named building discovery from Overture's source names/heights, with careful label density and independently matched provenance for notable buildings.
- [ ] Hand-authored corrections for standout landmarks only when primary data cannot represent them.

## P2 — First playable walk

- [x] Construct indexed building massing meshes from polygons, with concave/courtyard roofs, wall normals, and OBJ export.
- [x] Validate actual Downtown LA OBJ generation: 563 meshed, zero skipped, 38,210 vertices, 21,888 triangles.
- [x] Generate classified, indexed roadway / footpath / stairway / rail surface meshes with bounded joins and material groups.
- [x] Export self-contained combined glTF 2.0 binary with materials, indexed buildings and transportation meshes.
- [x] Validate actual Downtown LA GLB: 34,660 triangles, 6 scene meshes, 1,719,448 bytes, independently loaded via trimesh.
- [ ] Add physically correct intersections, crossings, grade separation and additional roof primitives.
- [x] Add first-person capsule-like planar building-footprint collision with swept horizontal movement, courtyards and wall sliding.
- [x] Limit progressive first-frame road meshing to source geometry within a
      fixed radius of the actual initial camera (full scene still loads later).
- [x] Stream source-derived road/building GPU chunks with bounded residency.
- [x] Preserve distant skyline geometry throughout the small Downtown pilot;
      remove the faulty nine-cell visibility cutoff.
- [x] Introduce conservative frustum **draw-only** rejection using actual
      mesh vertex bounds and pixel-parity regression at fixed camera poses.
- [ ] Consider validated occlusion culling and screen-space LOD only after
      demonstrated savings without visible gaps or false exclusions.
- [x] Extract the shared Rust chunk mesher and create an opt-in CPU-only
      Web Worker with fallback and browser regression coverage.
- [ ] Validate cold-download overhead, mesh parity and frame-time before
      making background meshing the default; then extend streaming to joined
      regions and introduce adaptive LOD, collision indexing, and terrain
      slope traversal constraints.
- [x] WebGPU renderer through `wgpu`, compiled to WASM; keyboard/pointer flythrough controls validated in headless Chromium.
- [x] First-person street-level navigation, mouse capture, error overlay and toggleable 18× fast travel.
- [x] Interactive north-up Downtown LA minimap with zoom, mapped buildings and camera location/heading.
- [x] Move marker tracking to RAF with a cached GIS basemap and compact per-frame Rust camera pose.
- [x] Add eight sourced Downtown LA landmark anchors with projected in-world labels and map pins.
- [ ] Replace label-only landmarks with high-fidelity landmark geometry anchored to existing footprints.
- [x] Subdivide road ribbons for terrain draping; align elevations with the exact rendered DEM triangles.
- [x] Isolate approximate roadway/footpath/rail batches with layer-visibility toggles and deterministic depth priority.
- [x] Add observable FPS/frame-time/CPU-side time/GPU mesh-size statistics and browser-error diagnostics.
- [ ] Investigate hardware-specific WebGPU black presentation and browser/GPU context-loss reports.
- [x] Report actual per-asset CacheStorage hits/network downloads and startup phases rather than claiming the cache is ready before checking it.
- [ ] Diagnose/reproduce user hardware-specific browser crashes and CPU/GPU stalls with retained telemetry, not just successful CI screenshots.
- [x] Replace browser busy Poll loop with Wait-scheduled redraw, pause hidden-tab drawing, bound HiDPI framebuffer size, and resume drawing on visibility changes; verify browser soak.
- [x] Mask pedestrian path mesh triangles inside actual at-grade carriageway footprints; preserve paths beneath bridges and exclude tunnels from road masking.
- [x] Provide source-tagged elevated road inspection camera jump and minimap visual differentiation.
- [ ] Profile a real-user Edge/Hyprland GPU context crash: this CI runner does not reproduce intermittent hardware failures.
- [ ] Add surveyed-grade street surfaces, map coverage beyond the pilot, and optional location jumping.
- [x] Reproducible browser QA in Chromium with real Downtown LA scene.
- [x] Publish the browser viewer to GitHub Pages with an automated WebGPU/WebGL2 verification and deployment workflow.

## P3 — Regional expansion

- [ ] Expand from Downtown LA to neighborhood-sized chunks across Los Angeles County.
- [ ] Add Orange, Ventura, Riverside and San Bernardino counties with consistent tile format.
- [ ] Quality dashboards for completeness, stale features, LOD budget and gap analysis.
- [ ] Deploy streamed, offline-cacheable regional datasets; keep original-source attribution visible.
