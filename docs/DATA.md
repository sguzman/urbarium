# Geographic data and fidelity

## Primary sources

1. **OpenStreetMap (OSM)** via Overpass: roads, path classifications, names, basic building ways. https://www.openstreetmap.org/copyright — ODbL 1.0, with attribution and database share-alike obligations. The Overpass public servers are rate limited; start with a small downtown bbox.
2. **Overture Maps**: broader building footprints and optional height/floor data, including building parts. https://docs.overturemaps.org/getting-data/ — available as spatially filterable GeoParquet; the CLI can export a bounded GeoJSON. Check the license and upstream provenance for each theme/source.
3. **Los Angeles County GIS building outlines**: published polygon layers include HEIGHT and ELEV, but newer LARIAC sources may be restricted. Example layer: https://dpw.gis.lacounty.gov/dpw/rest/services/buildingfootprints/MapServer/0 . Never bulk-download/redistribute a restricted layer without verifying permission. The layer's coordinate system is State Plane feet; when exporting GeoJSON, explicitly request EPSG:4326 for coordinates, and independently confirm attribute height units.
4. **City of Los Angeles street centerlines**: potential precision cross-check and supplementary attributes; geometry must be reprojected to WGS84. City data may stop at the city boundary.
5. **USGS 3DEP**: later terrain/elevation layer, https://www.usgs.gov/3d-elevation-program .

## Known MVP limitations

- The Overpass importer supports OSM building multipolygon relations with stitched outer/inner way rings, dropping unclosed geometry, but does not yet support every ambiguous/invalid OSM relation topology. Road intersection graphs and turn restrictions need a dedicated topology pass. GeoJSON supports Polygon, MultiPolygon and interior rings.
- Roads are polylines, not clipped road surfaces, lane widths or guaranteed pedestrian connectivity.
- Building footprints and heights are not the same as accurate architectural models. Height tags can be absent or stale.
- GeoJSON building import replaces OSM building footprints to avoid duplicate overlapping buildings. There is not yet a conflation/deduplication system for multiple sources.
- Region-local equirectangular approximation is suitable for small tiles; subdivide counties into small tiles instead of projecting a county-scale region into one flat plane.
- No source dataset is committed to Git. Capture downloaded dataset version, retrieval date and attribution in later manifest work before publishing generated assets.
- OSM's ODbL and potential county source terms apply independently of Urbarium's MIT **software** license.

## Geographic expansion

Use the same coordinate and schema conventions for LA, Orange, Ventura, Riverside and San Bernardino. Each county should be processed in many independently versioned smaller tiles, not one huge model. Future validation should compare coverage, missing heights, gaps, overlap and road topology against authoritative sources before labeling an area faithful.
