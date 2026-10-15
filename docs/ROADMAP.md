# Roadmap

## P0 — Data slice (this bootstrap)

- [x] Rust library schema for buildings, roads, provenance and region-local metric coordinates.
- [x] Bounded Overpass acquisition script for Downtown Los Angeles.
- [x] Import OSM road and building ways, preserving missing survey heights.
- [x] Import optional GeoJSON footprints with multipolygons/holes and explicit height units.
- [x] Unit tests and CI definition.
- [x] Optional Overture GeoParquet-to-GeoJSON data acquisition script with release provenance.
- [x] Synthetic end-to-end smoke tests for OSM and GeoJSON ingestion (CI configured).
- [ ] Execute and validate a real Downtown LA Overpass download (not yet possible in this environment).
- [ ] Capture provenance, source snapshots, and coverage metrics as a reproducible manifest.
- [ ] Add robust multipolygon/road topology ingestion and bounds clipping.

## P1 — Geographic fidelity

- [ ] Ingest versioned Overture building and building-part datasets with tiled streaming.
- [ ] Evaluate 2020/2023 LA County GIS outlines, units, coverage, terms, and permitted redistribution.
- [ ] Merge/conflict resolution keyed by source feature IDs, with geometry overlays and QA reports.
- [ ] Add street widths, sidewalks, road intersections, footpath crossings.
- [ ] Integrate 3DEP terrain; preserve local elevation and bridge/underpass levels.
- [ ] Hand-authored corrections for standout landmarks only when primary data cannot represent them.

## P2 — First playable walk

- [ ] Construct meshes from polygons, roof primitives and roads, without costly runtime Boolean CSG.
- [ ] Tile spatial indexing, culling, LOD, material batching and simple collision.
- [ ] WebGPU renderer through `wgpu`, compiled to WASM; keyboard/pointer walk controls.
- [ ] Start at a recognizable Downtown LA site; navigation, FPS and error overlay.
- [ ] Reproducible browser QA in Chromium and a publicly accessible demo URL.

## P3 — Regional expansion

- [ ] Expand from Downtown LA to neighborhood-sized chunks across Los Angeles County.
- [ ] Add Orange, Ventura, Riverside and San Bernardino counties with consistent tile format.
- [ ] Quality dashboards for completeness, stale features, LOD budget and gap analysis.
- [ ] Deploy streamed, offline-cacheable regional datasets; keep original-source attribution visible.
