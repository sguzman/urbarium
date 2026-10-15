# Geographic data and fidelity

## Primary sources

1. **OpenStreetMap (OSM)** via Overpass: roads, path classifications, names, basic building ways. https://www.openstreetmap.org/copyright — ODbL 1.0, with attribution and database share-alike obligations. The Overpass public servers are rate limited; start with a small downtown bbox.
2. **Overture Maps**: broader building footprints and optional height/floor data, including building parts. https://docs.overturemaps.org/getting-data/ — available as spatially filterable GeoParquet; the CLI can export a bounded GeoJSON. Check the license and upstream provenance for each theme/source.
3. **Los Angeles County GIS building outlines**: published polygon layers include HEIGHT and ELEV, but newer LARIAC sources may be restricted. Example layer: https://dpw.gis.lacounty.gov/dpw/rest/services/buildingfootprints/MapServer/0 . Never bulk-download/redistribute a restricted layer without verifying permission. The layer's coordinate system is State Plane feet; when exporting GeoJSON, explicitly request EPSG:4326 for coordinates, and independently confirm attribute height units.
4. **City of Los Angeles street centerlines**: potential precision cross-check and supplementary attributes; geometry must be reprojected to WGS84. City data may stop at the city boundary.
5. **USGS 3DEP**: later terrain/elevation layer, https://www.usgs.gov/3d-elevation-program .

## Known MVP limitations

- The Overpass importer supports OSM building multipolygon relations with stitched outer/inner way rings, dropping unclosed geometry, but does not yet support every ambiguous/invalid OSM relation topology. Road intersection graphs and turn restrictions need a dedicated topology pass. GeoJSON supports Polygon, MultiPolygon and interior rings.
- Roads are clipped metric polylines within the region bbox, not road surfaces, lane widths or guaranteed pedestrian connectivity. Geometric clipping drops original OSM node IDs for altered paths; segment graph reconstruction remains future work.
- Building footprints and heights are not the same as accurate architectural models. Height tags can be absent or stale.
- GeoJSON building import replaces OSM building footprints to avoid duplicate overlapping buildings. There is not yet a conflation/deduplication system for multiple sources.
- Region-local equirectangular approximation is suitable for small tiles; subdivide counties into small tiles instead of projecting a county-scale region into one flat plane.
- No source dataset is committed to Git. Capture downloaded dataset version, retrieval date and attribution in later manifest work before publishing generated assets.
- OSM's ODbL and potential county source terms apply independently of Urbarium's MIT **software** license.

## Geographic expansion

Use the same coordinate and schema conventions for LA, Orange, Ventura, Riverside and San Bernardino. Each county should be processed in many independently versioned smaller tiles, not one huge model. Future validation should compare coverage, missing heights, gaps, overlap and road topology against authoritative sources before labeling an area faithful.

## Overpass access policy

The public `overpass-api.de` instance introduced restrictions in 2026 that can
return HTTP 406 to automated clients. Our acquisition script identifies the
project with a user-agent and, **at most once**, tries another public instance.
A 406 or 429 stops immediately rather than evading service limits. You can override the endpoint through `OVERPASS_URL`; overrides never trigger
the automatic fallback. Do not schedule continuous polling or scale this
workflow across county-sized extents on public servers. Migrate to versioned
regional PBF/Parquet extracts for the multi-county ingest.

## Overture-only fallback

Urbarium can ingest **both Overture transportation segments and Overture
buildings** without calling Overpass. Use `--streets-geojson` and
`--buildings-geojson` together and omit `--osm`. Road segments are
renderer-neutral metric polylines; the Overture connector graph and access
rules are not yet reconstructed. See `scripts/build-downtown-overture.sh`.

## Building footprints at tile boundaries

Building geometry is **not** cut at the pilot bbox. Real Overture data
currently includes dozens of polygons crossing it. Cutting a courtyard or
irregular roof can damage geometric fidelity and introduce seams; our future
regional chunker should assign each whole building a canonical owner tile,
reference it from overlapping neighbor tiles, and avoid duplicates. Road
centerlines are clipped, because their source segments sometimes traverse
tens of kilometers outside the requested tile.

## Verified Downtown LA pilot (2026-10-15)

The Overture-only acquisition produced 1,587 clipped transportation spans
and 563 building footprints. 537 buildings have source height measurements;
the remaining 26 get a separate *visualization* assumption during mesh
compilation only. A 3D Wavefront OBJ from these footprints contains 38,210
vertices and 21,888 triangles, with zero buildings skipped. Run details:
https://github.com/sguzman/urbarium/actions/runs/38678603792

Overture segments are not all automotive streets: the sample is dominated by
pedestrian footways, steps, and service routes. Source line types must be
honored when constructing navigation and traversal rules. Road lines are
clipped to the pilot bbox. Building polygon vertices can extend beyond the
bbox and need canonical tile ownership when the world is partitioned.

## Transportation mesh conventions

The `mesh-roads` command converts the imported centerlines to narrow
triangulated surface ribbons, grouped into separately addressable materials.
It does not treat centerlines as actual curb/sidewalk edge surveys.
OSM `width` and `lanes` tags are applied when plausible; otherwise the
width comes from explicit class defaults. Overture `footway`, `steps`,
`service`, road classes and surface rail render as distinct groups.
`subway` and unknown categories do not create at-grade surfaces. Bridges,
tunnels, multi-level rail and stair elevation need 3DEP/grade integration
before being topologically or visually faithful. Intersections may overlap
and should not yet be treated as collision-ready paving geometry.

## Street-level prototype quality gates (2026-10-15)

The Overture pilot has **912 footway centerline segments**, but their
`subtype` is merely `road`, not a surveyed sidewalk class. A centerline
cannot establish curb boundaries or walking connectivity. The renderer now
adds width-based asphalt patches only when 3+ street endpoints agree within
1 meter; testing the retained pilot source found 117 candidate junctions.
Crossing 2D geometry alone never grants a connected junction, and explicit
OSM bridge/tunnel/layer tags suppress patch generation. Dataset consumers
should not infer traffic rules or physical grade from these visual patches.

Collision remains a temporary **2D circular footprint constraint** now
referenced to the local DEM building foundation heights; overhead flight
and courtyard holes remain intact. It is not full 3D collision or an
accessible-navigation graph. High-resolution 3DEP and surveyed sidewalk
source integration are still future work.

## Downtown LA terrain source and geometry (2026-10-15)

Mapzen's Terrarium georeferenced tiles, zoom 14, provide a 65×65 bare-earth
sample grid for the 1.5×1.2 km pilot. Four real source tiles were downloaded
by the verified acquisition workflow; the WGS84 region origin sample is
about 85.17 m above sea level, with approximately 50.25 m of local relief.
Grid heights are stored relative to this origin, with row zero at the **south**
edge. Sampling is bilinear in region-local X east/Z north coordinates.
The browser moves the walking camera 1.7 m above that sampled ground and
uses the same datum for draped roads, planar building foundations and the
terrain mesh. Terrain tiles and processed JSON are excluded from Git, but
source metadata are carried in the derived asset. The underlying Mapzen DEM
is not a high-resolution urban curb and bridge survey.

Source and attribution: https://registry.opendata.aws/terrain-tiles/
Format: https://github.com/tilezen/joerd/blob/master/docs/formats.md
