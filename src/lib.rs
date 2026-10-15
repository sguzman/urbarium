//! Geographic ingestion and compact, renderer-independent city geometry.
//! Positions are meters relative to each region's WGS84 origin:
//! X increases east; Z increases north.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub id: String,
    pub name: String,
    /// [longitude, latitude] in WGS84 degrees.
    pub origin: [f64; 2],
    /// [west, south, east, north] in WGS84 degrees.
    pub bbox: [f64; 4],
}

impl Region {
    pub fn validate(&self) -> Result<(), String> {
        let [w, s, e, n] = self.bbox;
        if !self.origin.iter().chain(self.bbox.iter()).all(|v| v.is_finite())
            || w >= e || s >= n || w < -180.0 || e > 180.0
            || s < -90.0 || n > 90.0
        {
            return Err("region has an invalid WGS84 extent".into());
        }
        if self.origin[0] < w || self.origin[0] > e
            || self.origin[1] < s || self.origin[1] > n
        {
            return Err("region origin must fall inside bbox".into());
        }
        Ok(())
    }

    pub fn project(&self, lon: f64, lat: f64) -> Point {
        let reference_lat = self.origin[1].to_radians();
        Point {
            x: (lon - self.origin[0]).to_radians() * EARTH_RADIUS_M * reference_lat.cos(),
            z: (lat - self.origin[1]).to_radians() * EARTH_RADIUS_M,
        }
    }

    pub fn intersects_lonlat_bbox(&self, bbox: [f64; 4]) -> bool {
        bbox[0] <= self.bbox[2] && bbox[2] >= self.bbox[0]
            && bbox[1] <= self.bbox[3] && bbox[3] >= self.bbox[1]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Street {
    pub source_id: String,
    pub name: Option<String>,
    pub kind: String,
    pub path: Vec<Point>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Polygon {
    /// First ring is exterior; remaining rings are interior holes.
    /// Rings are stored without duplicated closing vertices.
    pub rings: Vec<Vec<Point>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Building {
    pub source_id: String,
    pub name: Option<String>,
    /// Surveyed height only. Missing values stay None, never invented in ingestion.
    pub height_m: Option<f64>,
    pub levels: Option<f64>,
    pub polygons: Vec<Polygon>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub name: String,
    pub url: String,
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct City {
    pub schema_version: u32,
    pub region: Region,
    pub provenance: Vec<Provenance>,
    pub streets: Vec<Street>,
    pub buildings: Vec<Building>,
}

pub fn from_overpass(json: &str, region: Region) -> Result<City, String> {
    region.validate()?;
    let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let elements = root.get("elements").and_then(Value::as_array)
        .ok_or("Overpass JSON is missing elements array")?;
    let mut streets = Vec::new();
    let mut buildings = Vec::new();
    for element in elements {
        if element.get("type").and_then(Value::as_str) != Some("way") {
            continue; // relation multipolygons are handled by a future importer
        }
        let Some(id) = element.get("id").and_then(Value::as_i64) else { continue };
        let Some(geometry) = element.get("geometry").and_then(Value::as_array) else { continue };
        let Some(tags) = element.get("tags").and_then(Value::as_object) else { continue };
        let mut coordinates = Vec::with_capacity(geometry.len());
        for node in geometry {
            let (Some(lon), Some(lat)) = (
                node.get("lon").and_then(Value::as_f64),
                node.get("lat").and_then(Value::as_f64),
            ) else { continue };
            if lon.is_finite() && lat.is_finite() {
                coordinates.push((lon, lat));
            }
        }
        if coordinates.len() < 2 || !intersects_coordinates(&region, &coordinates) {
            continue;
        }
        let name = string_prop(tags.get("name"));
        if let Some(kind) = tags.get("highway").and_then(Value::as_str) {
            streets.push(Street {
                source_id: format!("osm:way:{id}"),
                name: name.clone(),
                kind: kind.to_owned(),
                path: coordinates.iter().map(|(lon, lat)| region.project(*lon, *lat)).collect(),
            });
        }
        if tags.get("building").and_then(Value::as_str).is_some_and(|v| v != "no")
            && coordinates.len() >= 4 && coordinates.first() == coordinates.last()
        {
            let mut ring: Vec<Point> = coordinates.iter()
                .map(|(lon, lat)| region.project(*lon, *lat)).collect();
            ring.pop(); // remove duplicated closing vertex
            if ring.len() >= 3 {
                buildings.push(Building {
                    source_id: format!("osm:way:{id}"),
                    name,
                    height_m: tags.get("height").and_then(Value::as_str).and_then(parse_osm_height),
                    levels: tags.get("building:levels").and_then(Value::as_str)
                        .and_then(parse_positive),
                    polygons: vec![Polygon { rings: vec![ring] }],
                });
            }
        }
    }
    streets.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    buildings.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(City {
        schema_version: 1,
        region,
        provenance: vec![Provenance {
            name: "OpenStreetMap contributors".into(),
            url: "https://www.openstreetmap.org/copyright".into(),
            license: "ODbL-1.0".into(),
        }],
        streets,
        buildings,
    })
}

fn intersects_coordinates(region: &Region, coords: &[(f64, f64)]) -> bool {
    let mut bbox = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for &(lon, lat) in coords {
        bbox[0] = bbox[0].min(lon);
        bbox[1] = bbox[1].min(lat);
        bbox[2] = bbox[2].max(lon);
        bbox[3] = bbox[3].max(lat);
    }
    region.intersects_lonlat_bbox(bbox)
}

fn string_prop(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn parse_positive(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)
}

fn parse_osm_height(raw: &str) -> Option<f64> {
    let trimmed = raw.trim().to_lowercase();
    if let Some(ft) = trimmed.strip_suffix("ft") {
        return parse_positive(ft).map(|h| h * 0.3048);
    }
    if let Some(m) = trimmed.strip_suffix('m') {
        return parse_positive(m);
    }
    parse_positive(&trimmed)
}

#[derive(Clone, Copy, Debug)]
pub enum HeightUnit {
    Meters,
    Feet,
}

/// Imports GeoJSON building Polygon/MultiPolygon footprints (Overture, ArcGIS, etc.).
/// Replaces OSM building footprints in the CLI to avoid duplicate overlapping geometry.
/// The caller must explicitly provide height units and dataset licensing metadata.
pub fn buildings_from_geojson(
    json: &str,
    region: &Region,
    unit: HeightUnit,
) -> Result<Vec<Building>, String> {
    region.validate()?;
    let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let features = root.get("features").and_then(Value::as_array)
        .ok_or("building input must be a GeoJSON FeatureCollection")?;
    let mut buildings = Vec::new();
    for (index, feature) in features.iter().enumerate() {
        let Some(geometry) = feature.get("geometry") else { continue };
        let Some(kind) = geometry.get("type").and_then(Value::as_str) else { continue };
        let Some(coords) = geometry.get("coordinates") else { continue };
        let polygons: Vec<Polygon> = match kind {
            "Polygon" => polygon_from_geojson(coords, region).into_iter().collect(),
            "MultiPolygon" => coords.as_array().into_iter().flatten()
                .filter_map(|polygon| polygon_from_geojson(polygon, region)).collect(),
            _ => continue,
        };
        if polygons.is_empty() {
            continue;
        }
        let props = feature.get("properties").and_then(Value::as_object);
        let prop = |key: &str| props.and_then(|p| p.get(key));
        let id = feature.get("id")
            .or_else(|| prop("id"))
            .or_else(|| prop("BLD_ID"))
            .or_else(|| prop("OBJECTID"))
            .map(value_as_identifier)
            .unwrap_or_else(|| index.to_string());
        let raw_height = prop("height").or_else(|| prop("HEIGHT")).and_then(number_value)
            .filter(|v| *v > 0.0);
        let height_m = raw_height.map(|h| match unit {
            HeightUnit::Meters => h,
            HeightUnit::Feet => h * 0.3048,
        });
        let levels = prop("num_floors").or_else(|| prop("building:levels"))
            .and_then(number_value).filter(|v| *v > 0.0);
        buildings.push(Building {
            source_id: format!("geojson:{id}"),
            name: prop("name").and_then(Value::as_str).map(str::to_owned),
            height_m,
            levels,
            polygons,
        });
    }
    buildings.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(buildings)
}

fn value_as_identifier(value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())
}

fn number_value(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
        .filter(|v| v.is_finite())
}

fn polygon_from_geojson(value: &Value, region: &Region) -> Option<Polygon> {
    let mut rings = Vec::new();
    let mut bbox = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for ring_value in value.as_array()? {
        let mut ring = Vec::new();
        for point in ring_value.as_array()? {
            let xy = point.as_array()?;
            let (lon, lat) = (xy.first()?.as_f64()?, xy.get(1)?.as_f64()?);
            if !lon.is_finite() || !lat.is_finite() { return None; }
            bbox[0] = bbox[0].min(lon);
            bbox[1] = bbox[1].min(lat);
            bbox[2] = bbox[2].max(lon);
            bbox[3] = bbox[3].max(lat);
            ring.push(region.project(lon, lat));
        }
        if ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 {
            return None;
        }
        rings.push(ring);
    }
    if rings.is_empty() || !region.intersects_lonlat_bbox(bbox) {
        return None;
    }
    Some(Polygon { rings })
}

pub fn count_by_road_kind(city: &City) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for street in &city.streets {
        *counts.entry(street.kind.as_str()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downtown() -> Region {
        Region {
            id: "dtla".into(), name: "Downtown LA".into(),
            origin: [-118.25, 34.05],
            bbox: [-118.258, 34.045, -118.242, 34.056],
        }
    }

    #[test]
    fn coordinate_projection_and_region_validation() {
        let region = downtown();
        region.validate().unwrap();
        assert_eq!(region.project(-118.25, 34.05), Point { x: 0.0, z: 0.0 });
        assert!(region.project(-118.249, 34.05).x > 90.0);
        assert!(region.project(-118.25, 34.051).z > 100.0);
    }

    #[test]
    fn parse_osm_ways_without_inventing_heights() {
        let input = r#"{"elements":[
          {"type":"way","id":2,"tags":{"highway":"residential","name":"Example Street"},"geometry":[
            {"lon":-118.25,"lat":34.05},{"lon":-118.249,"lat":34.05}]},
          {"type":"way","id":1,"tags":{"building":"yes","height":"40 ft"},"geometry":[
            {"lon":-118.25,"lat":34.05},{"lon":-118.2499,"lat":34.05},
            {"lon":-118.2499,"lat":34.0501},{"lon":-118.25,"lat":34.05}]},
          {"type":"way","id":3,"tags":{"building":"yes"},"geometry":[
            {"lon":-118.25,"lat":34.05},{"lon":-118.2498,"lat":34.05},
            {"lon":-118.2498,"lat":34.0502},{"lon":-118.25,"lat":34.05}]}
        ]}"#;
        let city = from_overpass(input, downtown()).unwrap();
        assert_eq!(city.streets.len(), 1);
        assert_eq!(city.buildings.len(), 2);
        assert!((city.buildings[0].height_m.unwrap() - 12.192).abs() < 0.001);
        assert_eq!(city.buildings[1].height_m, None);
        assert_eq!(city.buildings[0].polygons[0].rings[0].len(), 3);
    }

    #[test]
    fn geojson_multipolygon_holes_and_units() {
        let input = r#"{"type":"FeatureCollection","features":[{
          "type":"Feature","id":"b1","properties":{"HEIGHT":100},
          "geometry":{"type":"MultiPolygon","coordinates":[[
            [[-118.25,34.05],[-118.249,34.05],[-118.249,34.051],[-118.25,34.05]],
            [[-118.2499,34.0501],[-118.2498,34.0501],[-118.2498,34.0502],[-118.2499,34.0501]]
          ]]}}
        ]}"#;
        let result = buildings_from_geojson(input, &downtown(), HeightUnit::Feet).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].polygons[0].rings.len(), 2);
        assert!((result[0].height_m.unwrap() - 30.48).abs() < 1e-8);
    }

    #[test]
    fn outside_geometry_is_excluded() {
        let input = r#"{"type":"FeatureCollection","features":[{
          "type":"Feature","properties":{},
          "geometry":{"type":"Polygon","coordinates":[[[-120.0,35.0],[-120.01,35.0],[-120.01,35.01],[-120.0,35.0]]]}}
        ]}"#;
        assert!(buildings_from_geojson(input, &downtown(), HeightUnit::Meters).unwrap().is_empty());
    }
}
