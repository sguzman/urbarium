#!/usr/bin/env bash
# Download actual Downtown LA building polygons from a pinned Overture release.
# Uses DuckDB's spatial and httpfs extensions to stream only relevant GeoParquet.
# Produces GeoJSON FeatureCollection compatible with Urbarium's GeoJSON importer.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v duckdb >/dev/null || { echo "Install DuckDB CLI (with spatial/httpfs extensions) first." >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 is required to read the region definition." >&2; exit 1; }

release="${OVERTURE_RELEASE:-2026-09-23.1}"
if [[ ! "$release" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}\.[0-9]+$ ]]; then
  echo "Unexpected OVERTURE_RELEASE format: $release" >&2
  exit 1
fi
read -r west south east north < <(python3 -c 'import json; print(*json.load(open("regions/downtown-la.json"))["bbox"])')
mkdir -p data/raw
out="data/raw/downtown-la-overture-buildings.geojson"
tmp="data/raw/.downtown-la-buildings-$$.geojson"
trap 'rm -f "$tmp"' EXIT

duckdb -c "
INSTALL spatial; LOAD spatial;
INSTALL httpfs; LOAD httpfs;
SET s3_region = 'us-west-2';
COPY (
  SELECT id, names.primary AS name, height, level AS num_floors, geometry
  FROM read_parquet(
    's3://overturemaps-us-west-2/release/$release/theme=buildings/type=building/*',
    filename=true, hive_partitioning=1
  )
  WHERE bbox.xmin <= $east AND bbox.xmax >= $west
    AND bbox.ymin <= $north AND bbox.ymax >= $south
) TO '$tmp' WITH (FORMAT GDAL, DRIVER 'GeoJSON', SRS 'EPSG:4326');
"
python3 - "$tmp" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    doc = json.load(handle)
if doc.get("type") != "FeatureCollection" or not isinstance(doc.get("features"), list):
    raise SystemExit("Overture export did not produce a GeoJSON FeatureCollection")
print("Validated Overture building features:", len(doc["features"]))
PY
mv "$tmp" "$out"
# The raw source, extraction metadata, and any derived data are all intentionally gitignored.
python3 - "$release" "$west" "$south" "$east" "$north" <<'PY'
import datetime, json, sys
release, west, south, east, north = sys.argv[1:]
manifest = {
    "source": "Overture Maps buildings",
    "source_url": "https://docs.overturemaps.org/getting-data/",
    "release": release,
    "retrieved_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "bbox_wgs84": [float(west), float(south), float(east), float(north)],
    "local_path": "data/raw/downtown-la-overture-buildings.geojson",
    "license_note": "Check the Overture and component-source licenses before redistribution"
}
with open("data/raw/downtown-la-overture-buildings.manifest.json", "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2)
    handle.write("\n")
PY
echo "Saved $out using Overture release $release"
