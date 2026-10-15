#!/usr/bin/env python3
"""Build a bounded Downtown LA bare-earth grid from Mapzen Terrarium tiles.

This is a geographic DEM source, *not* synthetic height noise. It uses
actual georeferenced map tiles and samples a small, deterministic grid.
No subscription or credentials are required for the public AWS tiles.
"""
import argparse
import datetime
from io import BytesIO
import json
import math
from pathlib import Path
import urllib.request

from PIL import Image

SOURCE = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium"

def mercator_pixel(lon, lat, zoom):
    latitude = math.radians(lat)
    scale = (1 << zoom) * 256.0
    x = (lon + 180.0) / 360.0 * scale
    y = (1.0 - math.asinh(math.tan(latitude)) / math.pi) / 2.0 * scale
    return x, y

class Terrarium:
    def __init__(self, zoom=14):
        self.zoom = zoom
        self.tiles = {}
    def pixel(self, gx, gy):
        nx, ny = math.floor(gx / 256), math.floor(gy / 256)
        key = (nx, ny)
        if key not in self.tiles:
            url = f"{SOURCE}/{self.zoom}/{nx}/{ny}.png"
            request = urllib.request.Request(url, headers={
                "User-Agent": "Urbarium/0.1 (+https://github.com/sguzman/urbarium)"
            })
            with urllib.request.urlopen(request, timeout=60) as response:
                data = response.read()
            with Image.open(BytesIO(data)) as img:
                if img.size != (256, 256):
                    raise ValueError(f"unexpected terrain tile dimensions: {url}")
                self.tiles[key] = img.convert("RGBA")
        rgba = self.tiles[key].getpixel((gx % 256, gy % 256))
        r, g, b, a = rgba
        if a == 0:
            raise ValueError(f"empty terrain pixel in tile {key}")
        value = r * 256.0 + g + b / 256.0 - 32768.0
        if not math.isfinite(value) or value < -11000 or value > 9000:
            raise ValueError(f"invalid terrain value {value} at {key}")
        return value
    def sample(self, lon, lat):
        gx, gy = mercator_pixel(lon, lat, self.zoom)
        x, y = math.floor(gx), math.floor(gy)
        tx, ty = gx - x, gy - y
        sw = self.pixel(x, y)
        se = self.pixel(x + 1, y)
        nw = self.pixel(x, y + 1)
        ne = self.pixel(x + 1, y + 1)
        return (sw * (1 - tx) * (1 - ty)
                + se * tx * (1 - ty)
                + nw * (1 - tx) * ty
                + ne * tx * ty)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--region", default="regions/downtown-la.json")
    parser.add_argument("--out", default="data/processed/downtown-la-terrain.json")
    parser.add_argument("--resolution", type=int, default=65)
    parser.add_argument("--zoom", type=int, default=14)
    args = parser.parse_args()
    if not 2 <= args.resolution <= 1024 or not 0 <= args.zoom <= 14:
        parser.error("invalid resolution or zoom")
    region = json.loads(Path(args.region).read_text(encoding="utf-8"))
    w, s, e, n = region["bbox"]
    if not (w < e and s < n and w <= region["origin"][0] <= e
            and s <= region["origin"][1] <= n):
        parser.error("invalid region extent")
    terrain = Terrarium(args.zoom)
    origin = terrain.sample(*region["origin"])
    heights = []
    for iy in range(args.resolution):
        lat = s + (n-s) * iy / (args.resolution-1)
        for ix in range(args.resolution):
            lon = w + (e-w) * ix / (args.resolution-1)
            relative = terrain.sample(lon,lat) - origin
            heights.append(round(relative,4))
    doc = {
        "schema_version": 1,
        "region_id": region["id"],
        "bbox": region["bbox"],
        "width": args.resolution,
        "height": args.resolution,
        "heights_m": heights,
        "origin_elevation_m": round(origin,4),
        "source": f"Mapzen Terrain Tiles (Terrarium), zoom {args.zoom}; {SOURCE}"
    }
    path = Path(args.out)
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(doc,separators=(",",":"))+"\n",encoding="utf-8")
    summary = {
        "retrieved_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "source": SOURCE,
        "zoom": args.zoom,
        "tiles": sorted([f"{args.zoom}/{x}/{y}" for x,y in terrain.tiles]),
        "origin_elevation_m": origin,
        "min_local_m": min(heights),
        "max_local_m": max(heights),
        "grid_points": len(heights)
    }
    print("URBARIUM_TERRAIN_REPORT_START")
    print(json.dumps(summary,indent=2))
    print("URBARIUM_TERRAIN_REPORT_END")

if __name__ == "__main__":
    main()
