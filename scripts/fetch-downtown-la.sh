#!/usr/bin/env bash
# Obtain a bounded Overpass OSM snapshot (roads and building footprints).
# Community Overpass services are rate-limited. Do not run this in a tight loop.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p data/raw
endpoint="${OVERPASS_URL:-https://overpass-api.de/api/interpreter}"
output="data/raw/downtown-la-overpass.json"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
query='[out:json][timeout:120];(way["highway"](34.045,-118.258,34.056,-118.242);way["building"](34.045,-118.258,34.056,-118.242););out geom;'
echo "Fetching Downtown LA roads and building footprints from Overpass..."
curl --fail --location --silent --show-error --retry 2 --max-time 180 \
  --data-urlencode "data=$query" "$endpoint" -o "$tmp"
if ! grep -q '"elements"' "$tmp"; then
  echo "Overpass did not return an elements array; refusing to save invalid data." >&2
  exit 1
fi
mv "$tmp" "$output"
echo "Saved $output"
