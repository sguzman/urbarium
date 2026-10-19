use std::env;
use std::error::Error;
use std::fs;
use std::io::BufWriter;
use urbarium::geometry::{mesh_city, write_obj, HeightPolicy};
use urbarium::roads::{mesh_streets, write_road_mtl, write_road_obj};
use urbarium::gltf::write_glb;
use urbarium::road_coverage::verify_road_coverage;
use urbarium::road_building_conflicts::audit_road_buildings;
use urbarium::{attach_building_parts_from_geojson, buildings_from_geojson, count_by_road_kind, from_overpass, streets_from_geojson, City, HeightUnit, Provenance, Region};

fn main() {
    if let Err(error) = run() {
        eprintln!("urbarium: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("Usage: urbarium ingest --region FILE --out FILE [--osm FILE] [--streets-geojson FILE] [--buildings-geojson FILE --height-unit meters|feet] [--building-parts-geojson FILE] [--source-name NAME --source-url URL --source-license LICENSE]");
        println!("       urbarium mesh --city FILE --out FILE [--missing-height M] [--meters-per-level M]");
        println!("       urbarium mesh-roads --city FILE --out FILE");
        println!("       urbarium verify-road-cells --city FILE");
        println!("       urbarium audit-road-buildings --city FILE");
        println!("       urbarium glb --city FILE --out FILE");
        println!("All positions in output are local meters (X east, Z north).");
        return Ok(());
    }
    if args[0] == "mesh" {
        return run_mesh(&args[1..]);
    }
    if args[0] == "mesh-roads" {
        return run_roads(&args[1..]);
    }
    if args[0] == "verify-road-cells" {
        return run_verify_road_cells(&args[1..]);
    }
    if args[0] == "audit-road-buildings" {
        return run_audit_road_buildings(&args[1..]);
    }
    if args[0] == "glb" {
        return run_glb(&args[1..]);
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
            "--region", "--osm", "--out", "--streets-geojson", "--buildings-geojson", "--building-parts-geojson", "--height-unit",
            "--source-name", "--source-url", "--source-license",
        ].contains(key) {
            return Err(format!("unknown argument: {key}").into());
        }
    }
    let required = |key: &str| -> Result<&str, Box<dyn Error>> {
        options.get(key).copied().ok_or_else(|| format!("missing {key}").into())
    };
    let region: Region = serde_json::from_str(&fs::read_to_string(required("--region")?)?)?;
    let any_geojson = options.contains_key("--streets-geojson") || options.contains_key("--buildings-geojson") || options.contains_key("--building-parts-geojson");
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
    if let Some(parts_path)=options.get("--building-parts-geojson"){
        if !options.contains_key("--buildings-geojson"){
            return Err("--building-parts-geojson requires --buildings-geojson".into());
        }
        let unit=match required("--height-unit")?{
            "meters"=>HeightUnit::Meters,
            "feet"=>HeightUnit::Feet,
            other=>return Err(format!("invalid --height-unit: {other}").into()),
        };
        let report=attach_building_parts_from_geojson(
            &fs::read_to_string(parts_path)?,&city.region,unit,&mut city.buildings)
                .map_err(|e|format!("building part import: {e}"))?;
        println!("BUILDING_PART_IMPORT_QA {}",serde_json::to_string(&report)?);
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

fn run_mesh(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut options = std::collections::BTreeMap::new();
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        if !pair[0].starts_with("--") || options.insert(pair[0].as_str(), pair[1].as_str()).is_some() {
            return Err(format!("invalid or duplicate mesh argument: {}", pair[0]).into());
        }
    }
    if !pairs.remainder().is_empty() { return Err("all mesh flags require values".into()); }
    for key in options.keys() {
        if !["--city", "--out", "--missing-height", "--meters-per-level"].contains(key) {
            return Err(format!("unknown mesh argument: {key}").into());
        }
    }
    let required = |key: &str| -> Result<&str, Box<dyn Error>> {
        options.get(key).copied().ok_or_else(|| format!("missing {key}").into())
    };
    let policy = HeightPolicy {
        fallback_m: options.get("--missing-height")
            .map(|s| s.parse::<f64>()).transpose()?.unwrap_or(9.0),
        meters_per_level: options.get("--meters-per-level")
            .map(|s| s.parse::<f64>()).transpose()?.unwrap_or(3.1),
    };
    let city: City = serde_json::from_str(&fs::read_to_string(required("--city")?)?)?;
    let (mesh, report) = mesh_city(&city, policy).map_err(|e| format!("mesh: {e}"))?;
    let output = required("--out")?;
    if let Some(parent) = std::path::Path::new(output).parent() {
        fs::create_dir_all(parent)?;
    }
    let writer = BufWriter::new(fs::File::create(output)?);
    write_obj(writer, &mesh)?;
    println!("Mesh: {} building(s), {} skipped, {} vertices, {} triangles, {} approximate heights; output: {}",
        report.buildings_meshed, report.buildings_skipped, report.vertices,
        report.triangles, report.buildings_with_approximate_heights, output);
    Ok(())
}

fn run_audit_road_buildings(args:&[String])->Result<(),Box<dyn Error>>{
    if args.len()!=2 || args[0]!="--city"{
        return Err("audit-road-buildings requires --city FILE".into());
    }
    let city:City=serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let report=audit_road_buildings(&city)?;
    println!("ROAD_BUILDING_CONFLICT_QA {}",serde_json::to_string(&report)?);
    Ok(())
}

fn run_verify_road_cells(args:&[String])->Result<(),Box<dyn Error>>{
    if args.len()!=2 || args[0]!="--city"{
        return Err("verify-road-cells requires --city FILE".into());
    }
    let city:City=serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let report=verify_road_coverage(&city)?;
    println!("ROAD_CELL_COVERAGE_QA {}",serde_json::to_string(&report)?);
    Ok(())
}

fn run_roads(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut opts=std::collections::BTreeMap::new();
    let mut chunks=args.chunks_exact(2);
    for chunk in &mut chunks {
        if !["--city","--out"].contains(&chunk[0].as_str())
            || opts.insert(chunk[0].as_str(),chunk[1].as_str()).is_some()
        { return Err(format!("unknown or duplicate argument: {}",chunk[0]).into()); }
    }
    if !chunks.remainder().is_empty() || !opts.contains_key("--city") || !opts.contains_key("--out") {
        return Err("mesh-roads requires --city FILE and --out FILE".into());
    }
    let city: City=serde_json::from_str(&fs::read_to_string(opts["--city"])?)?;
    let (scene,report)=mesh_streets(&city).map_err(|err|format!("road meshes: {err}"))?;
    let output=std::path::Path::new(opts["--out"]);
    let dir=output.parent().filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(||std::path::Path::new("."));
    fs::create_dir_all(dir)?;
    write_road_mtl(BufWriter::new(fs::File::create(dir.join("urbarium-transport.mtl"))?))?;
    write_road_obj(BufWriter::new(fs::File::create(output)?),&scene)?;
    println!("Road meshes: {} surface spans ({} unknown/underground, {} degenerate), {} filled junctions, {} vertices, {} triangles",
        report.segments_meshed,report.segments_skipped,report.segments_degenerate,
        report.junction_patches,report.total_vertices,report.total_triangles);
    for (kind,count) in report.by_surface {
        println!("  {kind}: {count}");
    }
    println!("OBJ: {}; materials: {}",output.display(),dir.join("urbarium-transport.mtl").display());
    Ok(())
}

fn run_glb(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut opts=std::collections::BTreeMap::new();
    let mut chunks=args.chunks_exact(2);
    for chunk in &mut chunks {
        if !["--city","--out"].contains(&chunk[0].as_str())
            || opts.insert(chunk[0].as_str(),chunk[1].as_str()).is_some()
        { return Err(format!("unknown or duplicate argument: {}",chunk[0]).into()); }
    }
    if !chunks.remainder().is_empty() || !opts.contains_key("--city") || !opts.contains_key("--out") {
        return Err("glb requires --city FILE and --out FILE".into());
    }
    let city: City=serde_json::from_str(&fs::read_to_string(opts["--city"])?)?;
    let (buildings, report)=mesh_city(&city,HeightPolicy::default())?;
    let (roads, road_report)=mesh_streets(&city)?;
    let output=std::path::Path::new(opts["--out"]);
    if let Some(parent)=output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    write_glb(BufWriter::new(fs::File::create(output)?),&city,&buildings,&roads)?;
    println!("GLB: {} buildings ({} skipped), {} road/path spans ({} skipped), {} triangles; output: {}",
        report.buildings_meshed,report.buildings_skipped,road_report.segments_meshed,
        road_report.segments_skipped,report.triangles+road_report.total_triangles+2,
        output.display());
    Ok(())
}
