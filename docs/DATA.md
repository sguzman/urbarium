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
