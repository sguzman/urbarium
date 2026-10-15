#!/usr/bin/env bash
# Obtain a bounded Overpass OSM snapshot (roads and building footprints).
# Community Overpass services are rate-limited. Do not run this in a tight loop.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v python3 >/dev/null || { echo "python3 required to read region extent" >&2; exit 1; }
mkdir -p data/raw
# Public instances may reject generic cloud clients. Identify this project and fall
# back once only; do not repeatedly hammer shared OSM infrastructure.
primary="${OVERPASS_URL:-https://overpass-api.de/api/interpreter}"
fallback="https://overpass.kumi.systems/api/interpreter"
user_agent="Urbarium/0.1 (+https://github.com/sguzman/urbarium)"
output="data/raw/downtown-la-overpass.json"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
read -r west south east north < <(python3 -c 'import json; print(*json.load(open("regions/downtown-la.json"))["bbox"])')
bbox="$south,$west,$north,$east"
query="[out:json][timeout:120];(way[\"highway\"]($bbox);way[\"building\"]($bbox);relation[\"type\"=\"multipolygon\"][\"building\"]($bbox););out geom;"
echo "Fetching Downtown LA roads and building footprints from Overpass..."
if status="$(curl --fail --location --silent --show-error --max-time 180 \
    --write-out '%{http_code}' \
    --user-agent "$user_agent" -H 'Accept: application/json' \
    --data-urlencode "data=$query" "$primary" -o "$tmp")"; then
  :
else
  if [[ -n "${OVERPASS_URL:-}" ]]; then
    echo "Explicit OVERPASS_URL failed with status $status, not using a fallback." >&2
    exit 1
  fi
  # Treat 406 as intentional server rejection and 429 as rate limiting:
  # never evade these by switching public servers or retrying.
  case "$status" in
    000|408|500|502|503|504)
      echo "Transient Overpass failure ($status); trying one alternate endpoint..." >&2
      curl --fail --location --silent --show-error --max-time 180 \
        --user-agent "$user_agent" -H 'Accept: application/json' \
        --data-urlencode "data=$query" "$fallback" -o "$tmp"
      ;;
    *)
      echo "Overpass rejected request (HTTP $status); stopping without retry." >&2
      exit 1
      ;;
  esac
fi
if ! grep -q '"elements"' "$tmp"; then
  echo "Overpass did not return an elements array; refusing to save invalid data." >&2
  exit 1
fi
mv "$tmp" "$output"
echo "Saved $output"
