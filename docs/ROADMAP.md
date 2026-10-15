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
- [ ] Integrate a documented elevation DEM (3DEP preferred); drape roads while keeping building roofs/foundations coherent and bridge/underpass levels separate.
- [ ] Hand-authored corrections for standout landmarks only when primary data cannot represent them.

## P2 — First playable walk

- [x] Construct indexed building massing meshes from polygons, with concave/courtyard roofs, wall normals, and OBJ export.
- [x] Validate actual Downtown LA OBJ generation: 563 meshed, zero skipped, 38,210 vertices, 21,888 triangles.
- [x] Generate classified, indexed roadway / footpath / stairway / rail surface meshes with bounded joins and material groups.
- [x] Export self-contained combined glTF 2.0 binary with materials, indexed buildings and transportation meshes.
- [x] Validate actual Downtown LA GLB: 34,660 triangles, 6 scene meshes, 1,719,448 bytes, independently loaded via trimesh.
- [ ] Add physically correct intersections, crossings, grade separation and additional roof primitives.
- [x] Add first-person capsule-like planar building-footprint collision with swept horizontal movement, courtyards and wall sliding.
- [ ] Add broad-phase collision spatial indexing, terrain-aware feet, tile streaming, culling and LOD.
- [x] WebGPU renderer through `wgpu`, compiled to WASM; keyboard/pointer flythrough controls validated in headless Chromium.
- [ ] Start at a recognizable Downtown LA site; navigation, FPS and error overlay.
- [x] Reproducible browser QA in Chromium with real Downtown LA scene.
- [ ] Public GitHub Pages URL serving the verified playable browser bundle (deployment workflow configured, live deployment pending).

## P3 — Regional expansion

- [ ] Expand from Downtown LA to neighborhood-sized chunks across Los Angeles County.
- [ ] Add Orange, Ventura, Riverside and San Bernardino counties with consistent tile format.
- [ ] Quality dashboards for completeness, stale features, LOD budget and gap analysis.
- [ ] Deploy streamed, offline-cacheable regional datasets; keep original-source attribution visible.
