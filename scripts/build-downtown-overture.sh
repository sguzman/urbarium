#!/usr/bin/env bash
# Download Overture transportation and building GeoJSON without Overpass.
# Requires the official overturemaps CLI (pip install overturemaps).
set -euo pipefail
cd "$(dirname "$0")/.."
command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }
command -v overturemaps >/dev/null || { echo "Install the Overture Maps CLI: pip install overturemaps" >&2; exit 1; }
mkdir -p data/raw data/processed
bbox="$(python3 -c 'import json; print(",".join(str(v) for v in json.load(open("regions/downtown-la.json"))["bbox"]))')"
segments="data/raw/downtown-la-overture-segments.geojson"
buildings="data/raw/downtown-la-overture-buildings.geojson"
parts="data/raw/downtown-la-overture-building-parts.geojson"
echo "Downloading Overture transportation segments within $bbox"
overturemaps download --bbox="$bbox" --type=segment -f geojson -o "$segments"
echo "Downloading Overture building footprints within $bbox"
overturemaps download --bbox="$bbox" --type=building -f geojson -o "$buildings"
echo "Downloading Overture building parts within $bbox"
overturemaps download --bbox="$bbox" --type=building_part -f geojson -o "$parts"
python3 - "$segments" "$buildings" "$parts" <<'PY'
import json, sys
for path in sys.argv[1:]:
    with open(path, encoding="utf-8") as handle:
        document = json.load(handle)
    if document.get("type") != "FeatureCollection" or not isinstance(document.get("features"), list):
        raise SystemExit(f"{path} is not a FeatureCollection")
    print(f"{path}: {len(document['features'])} features")
PY
cargo run --release -- ingest \
  --region regions/downtown-la.json \
  --streets-geojson "$segments" \
  --buildings-geojson "$buildings" \
  --building-parts-geojson "$parts" \
  --height-unit meters \
  --source-name "Overture Maps" \
  --source-url "https://docs.overturemaps.org/getting-data/" \
  --source-license "Data licensing varies by theme and upstream source; consult Overture metadata" \
  --out data/processed/downtown-la.json
python3 - <<'PY'
import datetime, json
from pathlib import Path
data = json.loads(Path("data/processed/downtown-la.json").read_text(encoding="utf-8"))
manifest = {
    "source": "Overture Maps buildings and transportation segments",
    "retrieved_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "release_note": "Overture Python client resolves its current release via STAC",
    "bbox_wgs84": data["region"]["bbox"],
    "streets": len(data["streets"]),
    "buildings": len(data["buildings"]),
    "building_parts": sum(len(b.get("parts",[])) for b in data["buildings"]),
    "buildings_with_parts": sum(bool(b.get("parts")) for b in data["buildings"]),
    "measured_raised_parts": sum(
        p.get("min_height_m",0) is not None and (p.get("min_height_m") or 0)>0
        for b in data["buildings"] for p in b.get("parts",[]")
    ),
    "recorded_building_heights": sum(b["height_m"] is not None for b in data["buildings"]),
    "street_classes": dict(__import__("collections").Counter(s["kind"] for s in data["streets"]).most_common()),
    "source_bridge_spans": sum(
        s["tags"].get("bridge") == "yes" or
        (s["tags"].get("layer","0").lstrip("-").isdigit()
         and int(s["tags"].get("layer","0")) > 0)
        for s in data["streets"]
    ),
    "scoped_road_flag_spans": sum("overture:road_flags" in s["tags"]
        for s in data["streets"]),
    "bridge_flag_spans_any_scope": sum(
        any("is_bridge" in (rule.get("values") or []) for rule in
            json.loads(s["tags"].get("overture:road_flags","[]")))
        for s in data["streets"]
    ),
    "tunnel_flag_spans_any_scope": sum(
        any("is_tunnel" in (rule.get("values") or []) for rule in
            json.loads(s["tags"].get("overture:road_flags","[]")))
        for s in data["streets"]
    ),
    "positive_stack_spans_any_scope": sum(
        any(isinstance(rule.get("value"),int) and rule["value"]>0
            for rule in json.loads(s["tags"].get("overture:level_rules","[]")))
        for s in data["streets"]
    ),
    "provenance": data["provenance"],
}
Path("data/processed/downtown-la-overture-report.json").write_text(
    json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print("URBARIUM_OVERTURE_REPORT_START")
print(json.dumps(manifest, indent=2))
print("URBARIUM_OVERTURE_REPORT_END")
PY
