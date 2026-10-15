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

The sample covers approximately 1.5 × 1.2 km around Bunker Hill / Civic
Center, not the whole city. The first output is real indexed 3D geometry,
**not yet a navigable Rust/WASM/WebGPU demo**. Terrain remains flat, roads
remain centerlines, and fidelity of individual facades is not established.

[Real-data build and downloadable 3D OBJ artifact](https://github.com/sguzman/urbarium/actions/runs/38678603792)
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
Ground is currently a flat plane; the roads are not yet meshed. Known building
heights are used without changes. Missing heights are *visualization-only*
approximations (3.1 m per mapped level, otherwise 9 m); this does not alter
the source city JSON or claim the estimate is real.

## Generate street/path surfaces

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
Crossings, junction patches, level changes, navigability and terrain are
**not** yet solved. Keep road and building OBJ files together when inspecting
the combined pilot model.

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
