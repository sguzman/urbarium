#!/usr/bin/env bash
# Build a bounded, independent Overture city section (not a mega-region).
# No raw Overture downloads are committed, and every section has its own assets.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# != 2 ]]; then
  echo "Usage: bash scripts/build-region-overture.sh regions/NAME.json NAME" >&2
  exit 2
fi
region="$1"
stem="$2"
if [[ ! "$stem" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || [[ ! -f "$region" ]]; then
  echo "Invalid region path or safe section identifier" >&2
  exit 2
fi
command -v overturemaps >/dev/null || { echo "overturemaps CLI required" >&2; exit 1; }
bbox=$(python3 - "$region" "$stem" <<'PY'
import json,sys
r=json.load(open(sys.argv[1],encoding="utf-8"))
assert r["id"]==sys.argv[2], "Region ID disagrees with output stem"
assert r["bbox"][0]<r["bbox"][2] and r["bbox"][1]<r["bbox"][3]
print(",".join(map(str,r["bbox"])))
PY
)
mkdir -p data/raw data/processed
streets="data/raw/${stem}-overture-segments.geojson"
buildings="data/raw/${stem}-overture-buildings.geojson"
city="data/processed/${stem}.json"
echo "Acquiring Overture city section ${stem}: ${bbox}"
overturemaps download --bbox="$bbox" --type=segment -f geojson -o "$streets"
overturemaps download --bbox="$bbox" --type=building -f geojson -o "$buildings"
cargo run --release -- ingest \
  --region "$region" --streets-geojson "$streets" \
  --buildings-geojson "$buildings" --height-unit meters \
  --source-name "Overture Maps" \
  --source-url "https://docs.overturemaps.org/getting-data/" \
  --source-license "Data licensing varies by theme and source; consult Overture metadata" \
  --out "$city"
python3 - "$city" <<'PY'
import json,sys
data=json.load(open(sys.argv[1],encoding="utf-8"))
if not data["streets"] or not data["buildings"]:
    raise SystemExit("Region ingest yielded missing transport/building data")
print("SECTION_OK",data["region"]["id"],"streets",len(data["streets"]),"buildings",len(data["buildings"]))
PY
