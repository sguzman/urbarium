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

CI exercises small **synthetic** Overpass/GeoJSON features, not a live
Overture or Overpass download. A successful local data build is a remaining
acceptance criterion. No 3D renderer has yet been implemented.
