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

- [ ] Ingest versioned Overture building and building-part datasets with tiled streaming.
- [ ] Evaluate 2020/2023 LA County GIS outlines, units, coverage, terms, and permitted redistribution.
- [ ] Merge/conflict resolution keyed by source feature IDs, with geometry overlays and QA reports.
- [x] Add first-pass at-grade T/cross junction patches using shared centerline endpoints and local widths.
- [ ] Acquire surveyed sidewalk geometries, improve road width/curb edges, and handle footpath crossings.
- [x] Acquire and validate a bounded real Mapzen elevation DEM (50.25 m Downtown LA pilot relief) with georeferenced provenance.
- [x] Integrate DEM-based Rust terrain mesh, road draping, building base offsets and first-person camera ground height.
- [ ] Upgrade elevation source to high-resolution 3DEP where available; fix bridge/underpass decks and retaining walls.
- [x] Preserve Overture transportation bridge/tunnel flags, visual level rules and road width rules during GeoJSON ingestion without inventing metric elevations.
- [ ] Split roads at scoped bridge/level transitions and derive grade separation from Overture Z-order and topology; validate crossings against source features.
- [x] Generate initial source-qualified elevated freeway/bridge decks with genuine vertical mesh geometry, slab soffits/fascia, side barriers and approximate support columns; default render layers remain separate from at-grade junctions.
- [ ] Join elevated segment profiles across real Overture connector topology and model true source-scoped bridge transitions, ramps, underpasses, engineering clearances and collisions (never infer surveyed altitude solely from Z-order).
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
- [ ] Add broad-phase collision spatial indexing, tile streaming, culling and LOD, and terrain slope traversal constraints.
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
- [ ] Add surveyed-grade street surfaces, map coverage beyond the pilot, and optional location jumping.
- [x] Reproducible browser QA in Chromium with real Downtown LA scene.
- [ ] Public GitHub Pages URL serving the verified playable browser bundle (deployment workflow configured, live deployment pending).

## P3 — Regional expansion

- [ ] Expand from Downtown LA to neighborhood-sized chunks across Los Angeles County.
- [ ] Add Orange, Ventura, Riverside and San Bernardino counties with consistent tile format.
- [ ] Quality dashboards for completeness, stale features, LOD budget and gap analysis.
- [ ] Deploy streamed, offline-cacheable regional datasets; keep original-source attribution visible.
