# Optional Overture buildings pipeline

Overture publishes global building polygons as cloud-hosted GeoParquet, including
`height` and `level` attributes where supplied. This is a secondary data source
for measuring Downtown LA coverage and improving building reconstruction.

**Dependencies:** Rust toolchain, curl, DuckDB CLI with online access for the
spatial/httpfs extensions and Overture cloud parquet, and Python 3 for small
built-in JSON validation.

```sh
bash scripts/build-downtown-la.sh osm
bash scripts/build-downtown-la.sh overture
```

- `osm`: imports the bounded Overpass snapshot for roads and buildings.
- `overture`: imports **OSM roads** and **Overture building polygons**,
  replacing OSM building ways to avoid duplicate overlapping buildings.
- Both produce `data/processed/downtown-la.json`, local meter coordinates
  relative to the region origin; the second overwrites the first.
- Raw downloads, derived datasets and a retrieval manifest are kept out of Git.
- The Overture release is **pinned** by default to `2026-09-23.1`. Override
  with `OVERTURE_RELEASE=YYYY-MM-DD.N` after verifying a newer release.

The script uses a bounding-box intersection filter, meaning geometries on
the boundary can extend beyond the pilot tile. This is deliberate; **accurate
clipping and seamless tiled boundaries are not yet implemented**.

## Why this approach?

The Overture data distribution is GeoParquet and does not require storing
all of LA County locally to extract one neighborhood. DuckDB's `spatial`
extension exports a normal GeoJSON FeatureCollection. The importer expects
EPSG:4326 geographic longitude/latitude and treats Overture heights as meters.

Source documentation:
- https://docs.overturemaps.org/getting-data/
- https://docs.overturemaps.org/getting-data/duckdb/
- https://docs.overturemaps.org/getting-data/cloud-sources/
- https://www.openstreetmap.org/copyright

**Licensing:** the code license (MIT) does not cover downloaded geographic
datasets. Keep source metadata and follow Overture/OSM and upstream
attribution, share-alike, and redistribution requirements. Do not substitute
restricted LA County imagery-derived outlines without checking permission.

## Current validation status

CI exercises synthetic Overpass/GeoJSON features as regression fixtures.
A separate live Overture workflow **successfully** acquired Downtown LA
streets and building footprints on October 15, 2026 and generated the first
real low-poly 3D OBJ. Download and validation:
https://github.com/sguzman/urbarium/actions/runs/38678603792

The live OSM Overpass workflow returned HTTP 406 from the public server;
Overture is the confirmed real-data path. The Overture CLI currently selects
its latest release automatically, so the retrieval timestamp alone does not
pin an immutable source version; exact release capture is pending. The 3D
geometry generator and a WASM browser renderer now exist. See the
[viewer guide](VIEWER.md) for the current browser behavior.
