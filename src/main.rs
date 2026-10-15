use std::env;
use std::error::Error;
use std::fs;
use urbarium::{buildings_from_geojson, count_by_road_kind, from_overpass, streets_from_geojson, City, HeightUnit, Provenance, Region};

fn main() {
    if let Err(error) = run() {
        eprintln!("urbarium: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("Usage: urbarium ingest --region FILE --out FILE [--osm FILE] [--streets-geojson FILE] [--buildings-geojson FILE --height-unit meters|feet] [--source-name NAME --source-url URL --source-license LICENSE]");
        println!("All positions in output are local meters (X east, Z north).");
        return Ok(());
    }
    if args[0] != "ingest" {
        return Err(format!("unsupported command: {}", args[0]).into());
    }
    let mut options = std::collections::BTreeMap::new();
    let mut pairs = args[1..].chunks_exact(2);
    for pair in &mut pairs {
        if !pair[0].starts_with("--") || options.insert(pair[0].as_str(), pair[1].as_str()).is_some() {
            return Err(format!("invalid or duplicate argument: {}", pair[0]).into());
        }
    }
    if !pairs.remainder().is_empty() {
        return Err("all flags require a value".into());
    }
    for key in options.keys() {
        if ![
            "--region", "--osm", "--out", "--streets-geojson", "--buildings-geojson", "--height-unit",
            "--source-name", "--source-url", "--source-license",
        ].contains(key) {
            return Err(format!("unknown argument: {key}").into());
        }
    }
    let required = |key: &str| -> Result<&str, Box<dyn Error>> {
        options.get(key).copied().ok_or_else(|| format!("missing {key}").into())
    };
    let region: Region = serde_json::from_str(&fs::read_to_string(required("--region")?)?)?;
    let any_geojson = options.contains_key("--streets-geojson") || options.contains_key("--buildings-geojson");
    if !options.contains_key("--osm") && !any_geojson {
        return Err("provide --osm and/or a GeoJSON street/building source".into());
    }
    let mut city = if let Some(osm_path) = options.get("--osm") {
        from_overpass(&fs::read_to_string(osm_path)?, region)
            .map_err(|error| format!("OSM import: {error}"))?
    } else {
        region.validate().map_err(|e| format!("region: {e}"))?;
        City {
            schema_version: 1, region, provenance: Vec::new(),
            streets: Vec::new(), buildings: Vec::new(),
        }
    };
    if let Some(street_path) = options.get("--streets-geojson") {
        city.streets = streets_from_geojson(&fs::read_to_string(street_path)?, &city.region)
            .map_err(|error| format!("street GeoJSON import: {error}"))?;
    }
    if let Some(building_path) = options.get("--buildings-geojson") {
        let unit = match required("--height-unit")? {
            "meters" => HeightUnit::Meters,
            "feet" => HeightUnit::Feet,
            other => return Err(format!("invalid --height-unit: {other}").into()),
        };
        let buildings = buildings_from_geojson(&fs::read_to_string(building_path)?, &city.region, unit)
            .map_err(|error| format!("GeoJSON import: {error}"))?;
        city.buildings = buildings; // replacement prevents overlap with OSM footprints
    }
    if any_geojson {
        city.provenance.push(Provenance {
            name: required("--source-name")?.to_owned(),
            url: required("--source-url")?.to_owned(),
            license: required("--source-license")?.to_owned(),
        });
    } else if ["--source-name", "--source-url", "--source-license"]
        .iter().any(|key| options.contains_key(key)) {
        return Err("source metadata requires a GeoJSON source".into());
    }
    if options.contains_key("--height-unit") && !options.contains_key("--buildings-geojson") {
        return Err("--height-unit requires --buildings-geojson".into());
    }
    let output = required("--out")?;
    if let Some(parent) = std::path::Path::new(output).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, serde_json::to_vec_pretty(&city)?)?;
    println!("Region: {} | streets: {} | buildings: {} | output: {}",
        city.region.name, city.streets.len(), city.buildings.len(), output);
    for (kind, count) in count_by_road_kind(&city) {
        println!("  {kind}: {count}");
    }
    Ok(())
}
