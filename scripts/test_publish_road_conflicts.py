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
        "road_kind": "service",
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
        self.assertEqual(result["regions"][1]["centerline_conflicts"], 0)
        self.assertIn("Not proof", result["interpretation"])

    def test_capped_source_report_is_rejected(self):
        r = report("downtown-la")
        r["by_cause"]["centerline_inside_footprint"] = 2
        with self.assertRaisesRegex(ValueError, "truncated audit"):
            module.manifest_from_reports([r])


if __name__ == "__main__":
    unittest.main()
