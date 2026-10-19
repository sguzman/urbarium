"""Behavioral checks for audit-derived 3D conflict review manifest."""
import importlib.util
import unittest
from pathlib import Path

path = Path(__file__).resolve().with_name("publish_road_conflicts.py")
spec = importlib.util.spec_from_file_location("publish_road_conflicts", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def report(region, conflict=True):
    case = {
        "cause": "centerline_inside_footprint",
        "building_id": "geojson:building",
        "street_id": "geojson:street",
        "street_name": "Example Service Lane",
        "road_kind": "service",
        "source_line_pattern": "ends_inside_footprint",
        "overlap_m2": 25.0,
        "centerline_inside_m": 4.5,
        "centerline_length_m": 10.0,
        "focus_lonlat": [-118.25, 34.05],
    }
    return {
        "region_id": region,
        "by_cause": {"centerline_inside_footprint": int(conflict)},
        "largest": [case] if conflict else [],
    }


class ManifestTests(unittest.TestCase):
    def test_generated_cases_preserve_source_ids_and_coordinates(self):
        result = module.manifest_from_reports(
            [report("downtown-la"), report("little-tokyo-east", False)])
        self.assertEqual(len(result["cases"]), 1)
        self.assertEqual(result["cases"][0]["focus_lonlat"], [-118.25, 34.05])
        self.assertEqual(result["cases"][0]["building_id"], "geojson:building")
        self.assertEqual(result["cases"][0]["street_name"], "Example Service Lane")
        self.assertEqual(result["cases"][0]["source_line_pattern"],
                         "ends_inside_footprint")
        self.assertEqual(result["regions"][1]["centerline_conflicts"], 0)
        self.assertIn("Not proof", result["interpretation"])

    def test_complete_source_report_survives_top_twenty_cap(self):
        r = report("downtown-la")
        r["by_cause"]["centerline_inside_footprint"] = 25
        r["source_centerline_conflicts"] = [
            {**r["largest"][0], "street_id": f"street-{n:02}"}
            for n in range(25)
        ]
        r["largest"] = r["source_centerline_conflicts"][:20]
        result = module.manifest_from_reports([r])
        self.assertEqual(len(result["cases"]), 25)
        self.assertEqual(result["regions"][0]["centerline_conflicts"], 25)
        self.assertEqual(len({c["street_id"] for c in result["cases"]}), 25)

    def test_inconsistent_complete_source_report_is_rejected(self):
        r = report("downtown-la")
        r["by_cause"]["centerline_inside_footprint"] = 2
        r["source_centerline_conflicts"] = r["largest"]
        with self.assertRaisesRegex(ValueError, "inconsistent audit"):
            module.manifest_from_reports([r])

    def test_capped_source_report_is_rejected(self):
        r = report("downtown-la")
        r["by_cause"]["centerline_inside_footprint"] = 2
        with self.assertRaisesRegex(ValueError, "truncated audit"):
            module.manifest_from_reports([r])


if __name__ == "__main__":
    unittest.main()
