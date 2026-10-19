#!/usr/bin/env python3
"""Publish only source-centerline conflicts as reproducible 3D inspection links.

No handwritten OSM IDs, guessed 3D passages, or executable HTML are stored.
The inputs are the SAME Rust road/building auditor and packaged City JSON
that the production browser validates.
"""
import argparse
import json
import subprocess
from pathlib import Path

MARKER = "ROAD_BUILDING_CONFLICT_QA "

def manifest_from_reports(reports):
    cases = []
    regions = []
    for report in reports:
        region_id = report["region_id"]
        total = report["by_cause"].get("centerline_inside_footprint", 0)
        items = [item for item in report["largest"]
                 if item["cause"] == "centerline_inside_footprint"]
        # largest is capped at 20. Fail rather than publish a silently
        # incomplete locator list when more conflicts appear in new regions.
        if len(items) != total:
            raise ValueError(
                f"{region_id}: only {len(items)} of {total} centerline conflicts "
                "included in truncated audit report")
        regions.append({"region_id": region_id, "centerline_conflicts": total})
        for item in items:
            lon, lat = item["focus_lonlat"]
            if not (-180 <= lon <= 180 and -90 <= lat <= 90):
                raise ValueError("invalid geographic conflict locator")
            cases.append({
                "region_id": region_id,
                "building_id": item["building_id"],
                "street_id": item["street_id"],
                "road_kind": item["road_kind"],
                "focus_lonlat": [lon, lat],
                "overlap_m2": item["overlap_m2"],
                "centerline_inside_m": item["centerline_inside_m"],
                "centerline_length_m": item["centerline_length_m"],
                "source_part_evidence": item.get("source_part_evidence", []),
            })
    cases.sort(key=lambda item: (-item["overlap_m2"], item["region_id"],
                                 item["street_id"]))
    return {
        "schema_version": 1,
        "interpretation": "Source road centerline enters a building roof footprint. "
                          "Not proof of a mapping error, collision, or traversable passage.",
        "regions": regions,
        "cases": cases,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--city", action="append", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--binary", default="target/release/urbarium")
    args = parser.parse_args()
    reports = []
    for path in args.city:
        result = subprocess.run(
            [args.binary, "audit-road-buildings", "--city", str(path)],
            capture_output=True, text=True, check=True,
        )
        record = next((line[len(MARKER):] for line in result.stdout.splitlines()
                       if line.startswith(MARKER)), None)
        if record is None:
            raise RuntimeError(f"missing audit report: {path}")
        reports.append(json.loads(record))
    output = manifest_from_reports(reports)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print("ROAD_CONFLICT_INSPECTION_QA " + json.dumps({
        "cases": len(output["cases"]),
        "regions": output["regions"],
        "output": str(args.out),
    }))

if __name__ == "__main__":
    main()
