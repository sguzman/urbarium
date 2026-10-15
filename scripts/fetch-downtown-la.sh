#!/usr/bin/env bash
# Obtain a bounded Overpass OSM snapshot (roads and building footprints).
# Community Overpass services are rate-limited. Do not run this in a tight loop.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p data/raw
# Public instances may reject generic cloud clients. Identify this project and fall
# back once only; do not repeatedly hammer shared OSM infrastructure.
primary="${OVERPASS_URL:-https://overpass-api.de/api/interpreter}"
fallback="https://overpass.kumi.systems/api/interpreter"
user_agent="Urbarium/0.1 (+https://github.com/sguzman/urbarium)"
output="data/raw/downtown-la-overpass.json"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
query='[out:json][timeout:120];(way["highway"](34.045,-118.258,34.056,-118.242);way["building"](34.045,-118.258,34.056,-118.242);relation["type"="multipolygon"]["building"](34.045,-118.258,34.056,-118.242););out geom;'
echo "Fetching Downtown LA roads and building footprints from Overpass..."
if ! curl --fail --location --silent --show-error --max-time 180 \
    --user-agent "$user_agent" -H 'Accept: application/json' \
    --data-urlencode "data=$query" "$primary" -o "$tmp"; then
  if [[ -n "${OVERPASS_URL:-}" ]]; then
    echo "Explicit OVERPASS_URL failed, not using a fallback." >&2
    exit 1
  fi
  echo "Primary Overpass unavailable; attempting one alternate endpoint..." >&2
  curl --fail --location --silent --show-error --max-time 180 \
    --user-agent "$user_agent" -H 'Accept: application/json' \
    --data-urlencode "data=$query" "$fallback" -o "$tmp"
fi
if ! grep -q '"elements"' "$tmp"; then
  echo "Overpass did not return an elements array; refusing to save invalid data." >&2
  exit 1
fi
mv "$tmp" "$output"
echo "Saved $output"
