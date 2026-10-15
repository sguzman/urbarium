# Urbarium

**Real cities, reconstructed from primitives.**

Urbarium is a Rust-first pipeline for geographically faithful, low-poly, walkable cities. The first region is Downtown Los Angeles; the long-term scope includes Los Angeles County and neighboring Southern California counties. The intended interactive runtime is **Rust → WebAssembly → WebGPU**.

## Status

**Phase 1: data foundation.** This repository currently contains a working Rust ingestion *implementation*, not yet a deployed WebGPU demo. The CLI converts OSM/Overture street networks and building footprints into renderer-independent, region-local geometry. It also accepts optional GeoJSON street segments and a GeoJSON building layer (e.g. Overture or a lawfully obtained county source) with explicit height units. Live data acquisition and WASM/WebGPU rendering are not yet validated.

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
