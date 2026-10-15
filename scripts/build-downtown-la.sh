#!/usr/bin/env bash
# Reproducible acquisition and ingestion for the first LA pilot.
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:-osm}"
if [[ "$mode" != "osm" && "$mode" != "overture" ]]; then
  echo "Usage: bash scripts/build-downtown-la.sh [osm|overture]" >&2
  exit 2
fi

bash scripts/fetch-downtown-la.sh
args=(
  ingest
  --region regions/downtown-la.json
  --osm data/raw/downtown-la-overpass.json
  --out data/processed/downtown-la.json
)
if [[ "$mode" == "overture" ]]; then
  bash scripts/fetch-overture-downtown.sh
  args+=(
    --buildings-geojson data/raw/downtown-la-overture-buildings.geojson
    --height-unit meters
    --source-name "Overture Maps buildings"
    --source-url "https://docs.overturemaps.org/getting-data/"
    --source-license "See Overture source metadata and license terms"
  )
fi
cargo run --release -- "${args[@]}"
echo "City ingest finished; output: data/processed/downtown-la.json"
