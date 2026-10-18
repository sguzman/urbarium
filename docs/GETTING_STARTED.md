# Getting started and build commands

Use these commands from the **repository root**. The real-time viewer is available on GitHub Pages; local commands below acquire geography and generate portable assets. The optional data providers have independent access requirements and licenses.

[Documentation index](INDEX.md) · [Project README](../README.md)

---

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

For LA County data, use `--height-unit feet` **only after verifying the HEIGHT field units** on the chosen source. Source permissions must be checked separately. See [data sources and limitations](DATA.md).

## Repeatable Downtown LA data build

To acquire source datasets and generate the local city JSON in one step:

```sh
bash scripts/build-downtown-la.sh osm       # OSM streets + OSM building ways
bash scripts/build-downtown-la.sh overture  # OSM streets + Overture building polygons
```

The second path requires DuckDB and Python 3 and uses a pinned, updatable
Overture release. See [Overture pipeline](OVERTURE.md). Neither download
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

## GitHub Pages deployment

The [browser demo workflow](../.github/workflows/browser-demo.yml)
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

