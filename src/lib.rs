//! Geographic ingestion and compact, renderer-independent city geometry.
//! Positions are meters relative to each region's WGS84 origin:
//! X increases east; Z increases north.

pub mod geometry;
pub mod roads;
pub mod road_grade;
pub mod gltf;
pub mod collision;
pub mod terrain;
pub mod world;
pub mod streaming;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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
    /// OSM way node IDs, retained when every coordinate has a corresponding ID.
    /// These allow exact connectivity recovery at shared street junction nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub osm_node_ids: Vec<i64>,
    /// Selected source tags relevant to accurate road and pedestrian geometry.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
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
    let mut member_way_ids = BTreeSet::new();
    // Assemble relations first so tagged member ways cannot create duplicate buildings.
    for element in elements {
        if let Some((building, refs)) = osm_multipolygon_building(element, &region) {
            buildings.push(building);
            member_way_ids.extend(refs);
        }
    }
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
            let osm_node_ids = element.get("nodes").and_then(Value::as_array)
                .filter(|nodes| nodes.len() == geometry.len() && coordinates.len() == geometry.len())
                .map(|nodes| nodes.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
                .filter(|nodes| nodes.len() == geometry.len())
                .unwrap_or_default();
            const STREET_TAGS: [&str; 15] = [
                "lanes", "width", "maxspeed", "sidewalk", "foot",
                "bicycle", "surface", "bridge", "tunnel", "layer",
                "oneway", "access", "junction", "lit", "service",
            ];
            let street_tags = STREET_TAGS.iter()
                .filter_map(|key| tags.get(*key).and_then(Value::as_str)
                    .map(|value| ((*key).to_owned(), value.to_owned())))
                .collect::<BTreeMap<String, String>>();
            let original_path: Vec<Point> = coordinates.iter()
                .map(|(lon, lat)| region.project(*lon, *lat)).collect();
            let clipped = clip_street_path(&original_path, &region);
            for (part, path) in clipped.iter().enumerate() {
                let is_unchanged = clipped.len() == 1 && *path == original_path;
                streets.push(Street {
                    source_id: if clipped.len() == 1 {
                        format!("osm:way:{id}")
                    } else {
                        format!("osm:way:{id}:part:{part}")
                    },
                    name: name.clone(),
                    kind: kind.to_owned(),
                    path: path.clone(),
                    osm_node_ids: if is_unchanged { osm_node_ids.clone() } else { Vec::new() },
                    tags: street_tags.clone(),
                });
            }
        }
        if !member_way_ids.contains(&id)
            && tags.get("building").and_then(Value::as_str).is_some_and(|v| v != "no")
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

/// Assemble a closed relation from ordered/unordered, potentially reversed OSM way segments.
/// Reject incomplete rings rather than silently inventing geometry or sealing gaps.
fn stitch_rings(segments: Vec<Vec<(f64, f64)>>) -> Vec<Vec<(f64, f64)>> {
    let mut pending = segments;
    let mut rings = Vec::new();
    while !pending.is_empty() {
        let mut path = pending.remove(0);
        if path.len() < 2 { continue; }
        loop {
            if path.len() >= 4 && path.first() == path.last() {
                path.pop();
                if path.len() >= 3 { rings.push(path); }
                break;
            }
            let start = *path.first().unwrap();
            let end = *path.last().unwrap();
            let mut match_index = None;
            let mut append = true;
            let mut reverse = false;
            for (index, segment) in pending.iter().enumerate() {
                if segment.len() < 2 { continue; }
                if segment.first() == Some(&end) {
                    match_index = Some(index);
                } else if segment.last() == Some(&end) {
                    match_index = Some(index);
                    reverse = true;
                } else if segment.last() == Some(&start) {
                    match_index = Some(index);
                    append = false;
                } else if segment.first() == Some(&start) {
                    match_index = Some(index);
                    append = false;
                    reverse = true;
                }
                if match_index.is_some() { break; }
            }
            let Some(index) = match_index else { break; };
            let mut next = pending.remove(index);
            if reverse { next.reverse(); }
            if append {
                path.extend(next.into_iter().skip(1));
            } else {
                next.pop();
                next.extend(path);
                path = next;
            }
        }
    }
    rings
}

fn osm_member_coordinates(element: &Value) -> Option<Vec<(f64, f64)>> {
    let nodes = element.get("geometry")?.as_array()?;
    let coords: Option<Vec<_>> = nodes.iter().map(|node| {
        let lon = node.get("lon")?.as_f64()?;
        let lat = node.get("lat")?.as_f64()?;
        (lon.is_finite() && lat.is_finite()).then_some((lon, lat))
    }).collect();
    coords.filter(|v| v.len() >= 2)
}

fn point_in_ring(point: Point, ring: &[Point]) -> bool {
    if ring.len() < 3 { return false; }
    let mut inside = false;
    let mut previous = *ring.last().unwrap();
    for &current in ring {
        let crosses = (current.z > point.z) != (previous.z > point.z);
        if crosses {
            let x_crossing = (previous.x - current.x) * (point.z - current.z)
                / (previous.z - current.z) + current.x;
            if point.x < x_crossing { inside = !inside; }
        }
        previous = current;
    }
    inside
}

fn osm_multipolygon_building(element: &Value, region: &Region) -> Option<(Building, BTreeSet<i64>)> {
    if element.get("type")?.as_str()? != "relation" { return None; }
    let tags = element.get("tags")?.as_object()?;
    if tags.get("type")?.as_str()? != "multipolygon"
        || !tags.get("building").and_then(Value::as_str).is_some_and(|v| v != "no")
    {
        return None;
    }
    let id = element.get("id")?.as_i64()?;
    let members = element.get("members")?.as_array()?;
    let mut outer = Vec::new();
    let mut inner = Vec::new();
    let mut used = BTreeSet::new();
    for member in members {
        if member.get("type").and_then(Value::as_str) != Some("way") { continue; }
        let Some(coords) = osm_member_coordinates(member) else { continue; };
        match member.get("role").and_then(Value::as_str) {
            Some("outer") | Some("") => {
                if let Some(member_id) = member.get("ref").and_then(Value::as_i64) {
                    used.insert(member_id);
                }
                outer.push(coords);
            },
            Some("inner") => {
                if let Some(member_id) = member.get("ref").and_then(Value::as_i64) {
                    used.insert(member_id);
                }
                inner.push(coords);
            },
            _ => {}
        }
    }
    let exteriors = stitch_rings(outer);
    if exteriors.is_empty() { return None; }
    let mut polygons: Vec<Polygon> = exteriors.into_iter()
        .filter(|points| intersects_coordinates(region, points))
        .map(|points| Polygon { rings: vec![points.into_iter()
            .map(|(lon, lat)| region.project(lon, lat)).collect()] })
        .collect();
    if polygons.is_empty() { return None; }
    // Assign each courtyard to the containing outer footprint. Drop unmatched holes.
    for points in stitch_rings(inner) {
        let ring: Vec<Point> = points.into_iter()
            .map(|(lon, lat)| region.project(lon, lat)).collect();
        let Some(first) = ring.first().copied() else { continue; };
        if let Some(polygon) = polygons.iter_mut()
            .find(|polygon| point_in_ring(first, &polygon.rings[0]))
        {
            polygon.rings.push(ring);
        }
    }
    Some((Building {
        source_id: format!("osm:relation:{id}"),
        name: string_prop(tags.get("name")),
        height_m: tags.get("height").and_then(Value::as_str).and_then(parse_osm_height),
        levels: tags.get("building:levels").and_then(Value::as_str).and_then(parse_positive),
        polygons,
    }, used))
}

/// Clip a transportation centerline to the *actual* region boundary.
/// Long Overture railway or transit segments may run many kilometers outside
/// a queried bbox. This algorithm keeps independent in-region spans separate.
fn clip_segment(a: Point, b: Point, bounds: [f64; 4]) -> Option<(Point, Point)> {
    let dx = b.x - a.x;
    let dz = b.z - a.z;
    let [xmin, zmin, xmax, zmax] = bounds;
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for (p, q) in [
        (-dx, a.x - xmin),
        (dx, xmax - a.x),
        (-dz, a.z - zmin),
        (dz, zmax - a.z),
    ] {
        if p == 0.0 {
            if q < 0.0 { return None; }
        } else {
            let ratio = q / p;
            if p < 0.0 {
                low = low.max(ratio);
            } else {
                high = high.min(ratio);
            }
            if low > high { return None; }
        }
    }
    if high - low <= 1e-12 { return None; } // tangent/corner touch is not a road
    Some((
        Point { x: a.x + low * dx, z: a.z + low * dz },
        Point { x: a.x + high * dx, z: a.z + high * dz },
    ))
}

fn clip_street_path(points: &[Point], region: &Region) -> Vec<Vec<Point>> {
    let southwest = region.project(region.bbox[0], region.bbox[1]);
    let northeast = region.project(region.bbox[2], region.bbox[3]);
    let bounds = [southwest.x, southwest.z, northeast.x, northeast.z];
    let mut paths = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    for vertices in points.windows(2) {
        if let Some((a, b)) = clip_segment(vertices[0], vertices[1], bounds) {
            if let Some(&tail) = current.last() {
                if (tail.x - a.x).abs() > 1e-7 || (tail.z - a.z).abs() > 1e-7 {
                    if current.len() >= 2 { paths.push(std::mem::take(&mut current)); }
                    else { current.clear(); }
                }
            }
            if current.is_empty() { current.push(a); }
            if current.last() != Some(&b) { current.push(b); }
        } else if !current.is_empty() {
            if current.len() >= 2 { paths.push(std::mem::take(&mut current)); }
            else { current.clear(); }
        }
    }
    if current.len() >= 2 { paths.push(current); }
    paths
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

/// Split grade-related source rules by their linearly-referenced fractions
/// **before** regional clipping. Relative levels are Z-order, not metric height.
/// Scoped is_bridge/is_tunnel spans become independently meshable lines, while
/// unqualified source roads remain at grade. All source rules are still retained.
fn split_street_grade_scopes(
    path:&[Point],
    tags:&BTreeMap<String,String>,
)->Vec<(Vec<Point>,BTreeMap<String,String>)> {
    if path.len()<2 {return vec![(path.to_vec(),tags.clone())];}
    let flags:Vec<Value>=tags.get("overture:road_flags")
        .and_then(|text|serde_json::from_str(text).ok()).unwrap_or_default();
    let levels:Vec<Value>=tags.get("overture:level_rules")
        .and_then(|text|serde_json::from_str(text).ok()).unwrap_or_default();
    let relevant=|rule:&Value| {
        rule.get("values").and_then(Value::as_array).is_some_and(|v|
            v.iter().any(|x|matches!(x.as_str(),Some("is_bridge"|"is_tunnel"))))
    };
    if !flags.iter().any(relevant) && levels.is_empty() {
        return vec![(path.to_vec(),tags.clone())];
    }
    let range=|rule:&Value|->Option<(f64,f64)> {
        let Some(value)=rule.get("between") else {return Some((0.0,1.0));};
        if value.is_null(){return Some((0.0,1.0));}
        let a=value.as_array()?;
        if a.len()!=2 {return None;}
        let lo=a[0].as_f64()?;
        let hi=a[1].as_f64()?;
        (lo.is_finite()&&hi.is_finite()&&lo>=0.&&hi<=1.&&hi-lo>0.0000001)
            .then_some((lo,hi))
    };
    let mut cuts=vec![0.0_f64,1.0];
    for rule in flags.iter().filter(|r|relevant(r)).chain(levels.iter()) {
        if let Some((lo,hi))=range(rule) {cuts.extend([lo,hi]);}
    }
    cuts.sort_by(|a,b|a.total_cmp(b));
    cuts.dedup_by(|a,b|(*a-*b).abs()<1e-7);
    if cuts.len()<3 {
        return vec![(path.to_vec(),tags.clone())];
    }
    let mut stations=vec![0.0];
    for w in path.windows(2) {
        let length=(w[1].x-w[0].x).hypot(w[1].z-w[0].z);
        stations.push(stations.last().copied().unwrap_or(0.)+length);
    }
    let total=*stations.last().unwrap_or(&0.);
    if !total.is_finite() || total<0.01 {
        return vec![(path.to_vec(),tags.clone())];
    }
    let sample=|fraction:f64| -> Point {
        let distance=fraction*total;
        let index=stations.partition_point(|&v|v<distance).clamp(1,path.len()-1);
        let span=stations[index]-stations[index-1];
        let t=if span>0.0 {(distance-stations[index-1])/span}
              else {0.0};
        Point {
            x:path[index-1].x+(path[index].x-path[index-1].x)*t,
            z:path[index-1].z+(path[index].z-path[index-1].z)*t,
        }
    };
    let mut result=Vec::new();
    for w in cuts.windows(2) {
        let (lo,hi)=(w[0],w[1]);
        if hi-lo<1e-7 {continue;}
        let center=(lo+hi)*0.5;
        let matches=|rule:&Value|range(rule).is_some_and(|(a,b)|center>=a && center<=b);
        let flag=|name:&str| flags.iter().any(|rule|matches(rule)&&
            rule.get("values").and_then(Value::as_array)
                .is_some_and(|v|v.iter().any(|x|x.as_str()==Some(name))));
        let mut local_tags=tags.clone();
        for key in ["bridge","tunnel","layer"] {local_tags.remove(key);}
        if flag("is_bridge"){local_tags.insert("bridge".into(),"yes".into());}
        if flag("is_tunnel"){local_tags.insert("tunnel".into(),"yes".into());}
        if let Some(level)=levels.iter().filter(|rule|matches(rule))
            .filter_map(|rule|rule.get("value").and_then(Value::as_i64)).last(){
            local_tags.insert("layer".into(),level.to_string());
        }
        let mut section=vec![sample(lo)];
        let begin=lo*total;
        let end=hi*total;
        for (i,&d) in stations.iter().enumerate().skip(1).take(path.len()-2) {
            if d>begin+0.001 && d<end-0.001 {section.push(path[i]);}
        }
        section.push(sample(hi));
        section.dedup_by(|a,b|(a.x-b.x).hypot(a.z-b.z)<0.001);
        if section.len()>=2 {result.push((section,local_tags));}
    }
    if result.is_empty() {vec![(path.to_vec(),tags.clone())]}else{result}
}

/// Import Overture (or other GeoJSON) street/transport segments as city-local
/// metric centerlines. MultiLineString geometries are split into independently
/// addressable segment parts. Nonlinear geometries are ignored.
pub fn streets_from_geojson(json: &str, region: &Region) -> Result<Vec<Street>, String> {
    region.validate()?;
    let root: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let features = root.get("features").and_then(Value::as_array)
        .ok_or("street input must be a GeoJSON FeatureCollection")?;
    let mut streets = Vec::new();
    for (index, feature) in features.iter().enumerate() {
        let Some(geometry) = feature.get("geometry") else { continue };
        let Some(kind) = geometry.get("type").and_then(Value::as_str) else { continue };
        let Some(coords) = geometry.get("coordinates") else { continue };
        let properties = feature.get("properties").and_then(Value::as_object);
        let prop = |key: &str| properties.and_then(|p| p.get(key));
        let id = feature.get("id").or_else(|| prop("id"))
            .map(value_as_identifier).unwrap_or_else(|| index.to_string());
        let name = prop("name").and_then(Value::as_str)
            .or_else(|| prop("names").and_then(|n| n.get("primary")).and_then(Value::as_str))
            .map(str::to_owned);
        let class = prop("class").or_else(|| prop("subtype"))
            .and_then(Value::as_str).unwrap_or("unknown").to_owned();
        let mut tags = BTreeMap::new();
        for key in ["class", "subtype", "surface", "road", "access_restrictions"] {
            if let Some(value) = prop(key).and_then(Value::as_str) {
                tags.insert(key.to_owned(), value.to_owned());
            }
        }
        // Overture transportation is 2D but supplies vertical Z-order and
        // bridge/tunnel flags. Retain full scoped rules rather than silently
        // interpreting visual layer numbers as surveyed metric elevations.
        // Full-segment rules can also be exposed as OSM-style tags for the
        // existing at-grade junction guard.
        let covers_whole_segment = |v:&Value| -> bool {
            let Some(between)=v.get("between") else {return true;};
            let Some(range)=between.as_array() else {return false;};
            range.len()==2 &&
                range[0].as_f64().is_some_and(|x|x<=0.000001) &&
                range[1].as_f64().is_some_and(|x|x>=0.999999)
        };
        if let Some(levels)=prop("level_rules").and_then(Value::as_array) {
            if !levels.is_empty() {
                tags.insert("overture:level_rules".into(),Value::Array(levels.clone()).to_string());
            }
            if levels.len()==1 && covers_whole_segment(&levels[0]) {
                if let Some(level)=levels[0].get("value").and_then(Value::as_i64) {
                    tags.insert("layer".into(),level.to_string());
                }
            }
        }
        if let Some(flags)=prop("road_flags").and_then(Value::as_array) {
            if !flags.is_empty() {
                tags.insert("overture:road_flags".into(),Value::Array(flags.clone()).to_string());
            }
            for rule in flags {
                if !covers_whole_segment(rule) {continue;}
                let names=rule.get("values").and_then(Value::as_array);
                let has=|key:&str|names.is_some_and(|names|
                    names.iter().any(|v|v.as_str()==Some(key)));
                if has("is_bridge") { tags.insert("bridge".into(),"yes".into()); }
                if has("is_tunnel") { tags.insert("tunnel".into(),"yes".into()); }
            }
        }
        if let Some(widths)=prop("width_rules").and_then(Value::as_array) {
            if widths.len()==1 && covers_whole_segment(&widths[0]) {
                if let Some(width)=widths[0].get("value").and_then(number_value)
                    .filter(|v| (1.0..=100.0).contains(v)) {
                    tags.insert("width".into(),format!("{width}m"));
                }
            }
        }
        let paths: Vec<&Value> = match kind {
            "LineString" => vec![coords],
            "MultiLineString" => coords.as_array().into_iter().flatten().collect(),
            _ => continue,
        };
        for (part, path) in paths.into_iter().enumerate() {
            let Some(values) = path.as_array() else { continue };
            let coordinates: Option<Vec<(f64, f64)>> = values.iter().map(|point| {
                let a = point.as_array()?;
                let lon = a.first()?.as_f64()?;
                let lat = a.get(1)?.as_f64()?;
                (lon.is_finite() && lat.is_finite()).then_some((lon, lat))
            }).collect();
            let Some(coordinates) = coordinates else { continue };
            if coordinates.len() < 2 || !intersects_coordinates(region, &coordinates) {
                continue;
            }
            let path: Vec<Point> = coordinates.into_iter()
                .map(|(lon, lat)| region.project(lon, lat)).collect();
            let scopes=split_street_grade_scopes(&path,&tags);
            let multiple=scopes.len()>1;
            for (scope_index,(scoped_path,scoped_tags)) in scopes.into_iter().enumerate() {
                // Cut at source-defined grade transitions before clipping. A
                // clipped segment never silently reinterprets the rule's
                // [0,1] range as a fraction of the shortened geometry.
                let spans=clip_street_path(&scoped_path,region);
                for (piece,clipped) in spans.into_iter().enumerate() {
                    let base=format!("geojson:segment:{id}:{part}");
                    let base=if multiple{format!("{base}:scope:{scope_index}")}else{base};
                    streets.push(Street {
                        source_id:if piece==0{base}
                            else{format!("{base}:clip:{piece}")},
                        name:name.clone(),kind:class.clone(),path:clipped,
                        osm_node_ids:Vec::new(),tags:scoped_tags.clone(),
                    });
                }
            }
        }
    }
    streets.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(streets)
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
    fn preserve_street_topology_and_geometry_tags() {
        let sample = include_str!("../tests/fixtures/synthetic-overpass.json");
        let city = from_overpass(sample, downtown()).unwrap();
        assert_eq!(city.streets.len(), 1);
        assert_eq!(city.streets[0].osm_node_ids, vec![7001, 7002]);
        assert_eq!(city.streets[0].tags.get("sidewalk").map(String::as_str), Some("both"));
        assert_eq!(city.streets[0].tags.get("lanes").map(String::as_str), Some("2"));
        assert_eq!(city.streets[0].tags.get("bridge").map(String::as_str), Some("no"));
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
    fn assembles_multipolygon_building_with_reversed_segments_and_courtyard() {
        let input = r#"{"elements":[
          {"type":"relation","id":500,"tags":{"type":"multipolygon","building":"yes","height":"60 ft"},
            "members":[
              {"type":"way","ref":200,"role":"outer","geometry":[
                {"lon":-118.251,"lat":34.05},{"lon":-118.250,"lat":34.05},
                {"lon":-118.250,"lat":34.051}]},
              {"type":"way","ref":201,"role":"outer","geometry":[
                {"lon":-118.251,"lat":34.05},{"lon":-118.251,"lat":34.051},
                {"lon":-118.250,"lat":34.051}]},
              {"type":"way","ref":202,"role":"inner","geometry":[
                {"lon":-118.2508,"lat":34.0502},{"lon":-118.2504,"lat":34.0502},
                {"lon":-118.2504,"lat":34.0505},{"lon":-118.2508,"lat":34.0505},
                {"lon":-118.2508,"lat":34.0502}]}
            ]},
          {"type":"way","id":200,"tags":{"building":"yes"},"geometry":[
            {"lon":-118.251,"lat":34.05},{"lon":-118.250,"lat":34.05},
            {"lon":-118.250,"lat":34.051},{"lon":-118.251,"lat":34.05}]}
        ]}"#;
        let city = from_overpass(input, downtown()).unwrap();
        assert_eq!(city.buildings.len(), 1, "member way must not be duplicated");
        assert_eq!(city.buildings[0].source_id, "osm:relation:500");
        assert_eq!(city.buildings[0].polygons.len(), 1);
        assert_eq!(city.buildings[0].polygons[0].rings.len(), 2);
        assert_eq!(city.buildings[0].polygons[0].rings[0].len(), 4);
        assert!((city.buildings[0].height_m.unwrap() - 18.288).abs() < 0.001);
    }

    #[test]
    fn scoped_overture_bridge_is_split_before_clipping() {
        let region=downtown();
        let input=r#"{"type":"FeatureCollection","features":[{
          "type":"Feature","id":"partial-bridge",
          "properties":{"class":"primary",
            "road_flags":[{"values":["is_bridge"],"between":[0.2,0.8]}],
            "level_rules":[{"value":1,"between":[0.2,0.8]}]},
          "geometry":{"type":"LineString","coordinates":[
            [-118.252,34.05],[-118.250,34.05]
          ]}
        }]}"#;
        let streets=streets_from_geojson(input,&region).unwrap();
        assert_eq!(streets.len(),3);
        assert_eq!(streets.iter().filter(|r|r.tags.get("bridge")
            .is_some_and(|v|v=="yes")).count(),1);
        assert_eq!(streets[1].tags.get("layer").map(String::as_str),Some("1"));
        assert!(streets[0].tags.get("bridge").is_none());
        assert!(streets[2].tags.get("bridge").is_none());
        let join=|a:Point,b:Point| (a.x-b.x).hypot(a.z-b.z);
        assert!(join(*streets[0].path.last().unwrap(),streets[1].path[0])<0.01);
        assert!(join(*streets[1].path.last().unwrap(),streets[2].path[0])<0.01);
        assert!(streets[1].source_id.contains(":scope:1"));
    }

    #[test]
    fn overture_bridge_level_and_width_rules_survive_geojson_import() {
        let input=r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","id":"bridge-1",
           "properties":{"class":"motorway","subclass":"link",
             "road_flags":[{"values":["is_bridge"],"between":[0,1]}],
             "level_rules":[{"value":2,"between":[0,1]}],
             "width_rules":[{"value":13.5,"between":[0,1]}]},
           "geometry":{"type":"LineString","coordinates":[
             [-118.251,34.05],[-118.250,34.0505]]}},
          {"type":"Feature","id":"scoped",
           "properties":{"class":"primary",
             "road_flags":[{"values":["is_bridge"],"between":[0.2,0.6]}],
             "level_rules":[{"value":1,"between":[0.2,0.6]}]},
           "geometry":{"type":"LineString","coordinates":[
             [-118.252,34.05],[-118.251,34.0505]]}}
        ]}"#;
        let roads=streets_from_geojson(input,&downtown()).unwrap();
        assert_eq!(roads.len(),4);
        let full=roads.iter().find(|r|r.source_id.contains("bridge-1")).unwrap();
        assert_eq!(full.tags.get("bridge").map(String::as_str),Some("yes"));
        assert_eq!(full.tags.get("layer").map(String::as_str),Some("2"));
        assert_eq!(full.tags.get("width").map(String::as_str),Some("13.5m"));
        assert!(full.tags.contains_key("overture:road_flags"));
        let scopes=roads.iter().filter(|r|r.source_id.contains("scoped"))
            .collect::<Vec<_>>();
        assert_eq!(scopes.len(),3);
        assert!(!scopes[0].tags.contains_key("bridge"));
        assert_eq!(scopes[1].tags.get("bridge").map(String::as_str),Some("yes"));
        assert_eq!(scopes[1].tags.get("layer").map(String::as_str),Some("1"));
        assert!(!scopes[2].tags.contains_key("bridge"));
        assert!(scopes[1].tags.contains_key("overture:level_rules"));
    }

    #[test]
    fn imports_geojson_line_and_multiline_streets_without_fake_junctions() {
        let input = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","id":"seg-1",
           "properties":{"class":"residential","names":{"primary":"First Street"}},
           "geometry":{"type":"MultiLineString","coordinates":[
             [[-118.251,34.05],[-118.2505,34.0505]],
             [[-118.2505,34.0505],[-118.250,34.0508]]
           ]}},
          {"type":"Feature","properties":{"class":"service"},
           "geometry":{"type":"LineString","coordinates":[
             [-121,36],[-121.0001,36.0001]
           ]}}
        ]}"#;
        let streets = streets_from_geojson(input, &downtown()).unwrap();
        assert_eq!(streets.len(), 2);
        assert_eq!(streets[0].name.as_deref(), Some("First Street"));
        assert_eq!(streets[0].kind, "residential");
        assert_eq!(streets[0].source_id, "geojson:segment:seg-1:0");
        assert_eq!(streets[1].source_id, "geojson:segment:seg-1:1");
        assert!(streets.iter().all(|s| s.osm_node_ids.is_empty()));
        assert_eq!(streets[0].tags.get("class").map(String::as_str), Some("residential"));
    }

    #[test]
    fn clip_long_transport_segments_to_pilot_extent() {
        let input = r#"{"type":"FeatureCollection","features":[{
          "type":"Feature","id":"long-rail","properties":{"class":"rail"},
          "geometry":{"type":"LineString","coordinates":[
            [-118.8,34.05],[-118.25,34.05],[-117.8,34.05]
          ]}
        }]}"#;
        let region = downtown();
        let streets = streets_from_geojson(input, &region).unwrap();
        assert_eq!(streets.len(), 1);
        assert_eq!(streets[0].path.len(), 3);
        let left = region.project(region.bbox[0], region.bbox[1]).x;
        let right = region.project(region.bbox[2], region.bbox[3]).x;
        let p = &streets[0].path;
        assert!((p[0].x - left).abs() < 1e-6);
        assert!((p[2].x - right).abs() < 1e-6);
        assert!(p.iter().all(|v| v.x >= left-1e-6 && v.x <= right+1e-6));
    }

    #[test]
    fn clipped_osm_ways_do_not_claim_unmodified_node_topology() {
        let sample = r#"{"elements":[{"type":"way","id":901,
          "tags":{"highway":"primary"},"nodes":[10,11],
          "geometry":[{"lon":-118.8,"lat":34.05},{"lon":-118.25,"lat":34.05}]
        }]}"#;
        let city = from_overpass(sample, downtown()).unwrap();
        assert_eq!(city.streets.len(), 1);
        assert!(city.streets[0].osm_node_ids.is_empty());
    }

    #[test]
    fn incomplete_relation_ring_is_rejected() {
        let input = r#"{"elements":[
          {"type":"relation","id":501,"tags":{"type":"multipolygon","building":"yes"},
           "members":[{"type":"way","role":"outer","ref":1,"geometry":[
             {"lon":-118.251,"lat":34.05},{"lon":-118.25,"lat":34.05}]}]}
        ]}"#;
        assert!(from_overpass(input, downtown()).unwrap().buildings.is_empty());
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
