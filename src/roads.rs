//! Low-poly ribbon meshes for streets, walking paths, steps, and surface rail.
//! Source linework is a centerline. Widths and materials are *visual approximations*;
//! a traversable, intersection-aware road network is a separate later milestone.

use crate::geometry::{Mesh, Vertex};
use crate::{City, Point, Street};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Write};

#[derive(Clone, Copy, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Road,
    Walk,
    Steps,
    Rail,
}
impl SurfaceKind {
    fn material(self) -> &'static str {
        match self {
            Self::Road => "road_asphalt",
            Self::Walk => "walk_pavement",
            Self::Steps => "steps_stone",
            Self::Rail => "rail_surface",
        }
    }
}

#[derive(Debug, Default)]
pub struct RoadScene {
    pub layers: BTreeMap<SurfaceKind, Mesh>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoadReport {
    pub source_segments: usize,
    pub segments_meshed: usize,
    pub segments_skipped: usize,
    pub segments_degenerate: usize,
    /// Filled at-grade T/cross intersections inferred from shared road endpoints.
    pub junction_patches: usize,
    pub total_vertices: usize,
    pub total_triangles: usize,
    pub by_surface: BTreeMap<String, usize>,
}

/// Deliberately ignore subway and unknown classes: these must not be rendered
/// as ordinary at-grade asphalt strips. There is no terrain/bridge geometry yet.
fn surface_class(street: &Street) -> Option<(SurfaceKind, f64)> {
    let classification = street.kind.as_str();
    let (kind, default_width): (SurfaceKind, f64) = match classification {
        "motorway" | "motorway_link" | "trunk" | "trunk_link" => (SurfaceKind::Road, 12.0),
        "primary" | "primary_link" | "secondary" | "secondary_link" => (SurfaceKind::Road, 9.0),
        "tertiary" | "tertiary_link" | "unclassified" => (SurfaceKind::Road, 7.0),
        "residential" | "living_street" => (SurfaceKind::Road, 6.5),
        "service" | "track" => (SurfaceKind::Road, 5.5),
        "footway" | "pedestrian" | "path" | "bridleway" => (SurfaceKind::Walk, 2.5),
        "cycleway" => (SurfaceKind::Walk, 2.0),
        "steps" => (SurfaceKind::Steps, 2.0),
        "light_rail" | "rail" | "tram" | "funicular" => (SurfaceKind::Rail, 3.0),
        _ => return None,
    };
    // Width is a source observation only if it is positive and plausible.
    // For OSM roadways, a tagged lane count is a better approximation than
    // a class default. Overture classes generally lack a width measurement.
    let width = street.tags.get("width")
        .and_then(|v| parse_width(v))
        .or_else(|| {
            if kind != SurfaceKind::Road { return None; }
            street.tags.get("lanes")
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|n| (1..=12).contains(n))
                .map(|n| n as f64 * 3.2)
        })
        .unwrap_or(default_width);
    Some((kind, width))
}

fn parse_width(value: &str) -> Option<f64> {
    let normalized = value.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(v) = normalized.strip_suffix("ft") {
        (v.trim(), 0.3048)
    } else if let Some(v) = normalized.strip_suffix('m') {
        (v.trim(), 1.0)
    } else { (normalized.as_str(), 1.0) };
    number.parse::<f64>().ok()
        .map(|v| v * multiplier)
        .filter(|v| v.is_finite() && (0.6..=60.0).contains(v))
}

fn direction(a: Point, b: Point) -> Option<[f64;2]> {
    let dx = b.x-a.x;
    let dz = b.z-a.z;
    let length = dx.hypot(dz);
    (length > 0.01 && length.is_finite()).then_some([dx/length,dz/length])
}

/// Keep a strip continuous through bends, without unbounded miter spikes.
/// It intentionally does not invent connected intersection polygons.
fn ribbon(path: &[Point], width: f64, height: f32) -> Result<Option<Mesh>, String> {
    let mut points: Vec<Point> = Vec::new();
    for &point in path {
        if !point.x.is_finite() || !point.z.is_finite() {
            return Err("non-finite street coordinate".into());
        }
        if points.last().is_none_or(|p| direction(*p, point).is_some()) {
            points.push(point);
        }
    }
    if points.len() < 2 { return Ok(None); }
    let segment_normals: Vec<[f64;2]> = points.windows(2)
        .map(|pair| {
            let d = direction(pair[0], pair[1]).expect("duplicates filtered");
            [-d[1],d[0]]
        }).collect();
    let half = width * 0.5;
    let mut mesh = Mesh::default();
    for (i, &point) in points.iter().enumerate() {
        let (offset_x, offset_z) = if i == 0 {
            let n = segment_normals[0];
            (n[0]*half,n[1]*half)
        } else if i+1 == points.len() {
            let n = segment_normals[segment_normals.len()-1];
            (n[0]*half,n[1]*half)
        } else {
            let a=segment_normals[i-1];
            let b=segment_normals[i];
            let sum=[a[0]+b[0],a[1]+b[1]];
            let magnitude=sum[0].hypot(sum[1]);
            if magnitude < 1e-6 {
                (b[0]*half,b[1]*half)
            } else {
                let m=[sum[0]/magnitude,sum[1]/magnitude];
                let dot=(m[0]*b[0]+m[1]*b[1]).abs().max(0.01);
                let miter_length=(half/dot).min(2.0*half);
                (m[0]*miter_length,m[1]*miter_length)
            }
        };
        let l=Vertex {
            position: [(point.x+offset_x) as f32,height,(point.z+offset_z) as f32],
            normal: [0.0,1.0,0.0],
        };
        let r=Vertex {
            position: [(point.x-offset_x) as f32,height,(point.z-offset_z) as f32],
            normal: [0.0,1.0,0.0],
        };
        mesh.vertices.extend([l,r]);
    }
    for i in 0..points.len()-1 {
        let base=u32::try_from(i.checked_mul(2).ok_or("ribbon too large")?)
            .map_err(|_| "ribbon exceeds 32-bit indices")?;
        // Positive Y facing triangles in an X-east/Z-north world.
        mesh.indices.extend([base,base+2,base+1,base+1,base+2,base+3]);
    }
    Ok(Some(mesh))
}

#[derive(Clone,Copy)]
struct RoadEnd {
    x:f64, z:f64, width:f64,
    direction:[f64;2],
}

/// Patches only real mapped *road endpoints* joining at grade. We do not
/// infer a junction from two centerlines merely crossing in 2D, which might
/// be an overpass, an underpass, or an unconnected footbridge.
fn add_junction_patches(city:&City, scene:&mut RoadScene, report:&mut RoadReport) -> Result<(),String> {
    let mut groups:Vec<Vec<RoadEnd>>=Vec::new();
    let mut cells:BTreeMap<(i64,i64),Vec<usize>>=BTreeMap::new();
    for street in &city.streets {
        let Some((SurfaceKind::Road,width))=surface_class(street) else {continue};
        if street.path.len()<2
            || street.tags.get("bridge").is_some_and(|v|v!="no")
            || street.tags.get("tunnel").is_some_and(|v|v!="no")
            || street.tags.get("layer").is_some_and(|v|v!="0") {
            continue;
        }
        for (p,q) in [
            (street.path[0],street.path[1]),
            (street.path[street.path.len()-1],street.path[street.path.len()-2]),
        ] {
            let Some(heading)=direction(p,q) else {continue};
            let item=RoadEnd{x:p.x,z:p.z,width,direction:heading};
            let cell=((p.x/2.0).floor() as i64,(p.z/2.0).floor() as i64);
            let mut found=None;
            for ox in -1..=1 {
                for oz in -1..=1 {
                    if let Some(indices)=cells.get(&(cell.0+ox,cell.1+oz)) {
                        for &index in indices {
                            let anchor=groups[index][0];
                            if (p.x-anchor.x).hypot(p.z-anchor.z)<1.0 {
                                found=Some(index);
                                break;
                            }
                        }
                    }
                    if found.is_some(){break;}
                }
                if found.is_some(){break;}
            }
            if let Some(index)=found {
                groups[index].push(item);
            } else {
                let index=groups.len();
                groups.push(vec![item]);
                cells.entry(cell).or_default().push(index);
            }
        }
    }
    for group in groups {
        let mut arms:Vec<RoadEnd>=Vec::new();
        for &item in &group {
            // A straight segment split in two is not a 3-way intersection.
            if arms.iter().all(|other| {
                other.direction[0]*item.direction[0]+
                    other.direction[1]*item.direction[1] < 0.95
            }) { arms.push(item); }
        }
        if arms.len()<3 {continue;}
        let x=group.iter().map(|e|e.x).sum::<f64>()/group.len() as f64;
        let z=group.iter().map(|e|e.z).sum::<f64>()/group.len() as f64;
        let mut edge=Vec::with_capacity(arms.len()*2);
        for arm in &arms {
            let nx=-arm.direction[1];
            let nz=arm.direction[0];
            let width=arm.width/2.0;
            edge.push((x+nx*width,z+nz*width));
            edge.push((x-nx*width,z-nz*width));
        }
        let hull=junction_hull(edge);
        if hull.len()<3 {continue;}
        let layer=scene.layers.entry(SurfaceKind::Road).or_default();
        let offset=u32::try_from(layer.vertices.len())
            .map_err(|_|"road layer exceeds 32-bit indices")?;
        if hull.len() >= u32::MAX as usize - offset as usize {
            return Err("junction patch exceeds 32-bit indices".into());
        }
        // A 1mm height bias prevents z-fighting without covering vertical
        // features; the patch is deliberately the same asphalt material.
        let y=0.081;
        layer.vertices.push(Vertex{
            position:[x as f32,y,z as f32],normal:[0.0,1.0,0.0]
        });
        for &(ex,ez) in &hull {
            layer.vertices.push(Vertex{
                position:[ex as f32,y,ez as f32],normal:[0.0,1.0,0.0]
            });
        }
        for i in 0..hull.len() {
            let a=offset+1+i as u32;
            let b=offset+1+((i+1)%hull.len()) as u32;
            layer.indices.extend([offset,b,a]); // +Y in X east / Z north
        }
        report.junction_patches+=1;
    }
    Ok(())
}

/// 2D convex hull of surveyed-road-width offset points around a real node.
/// This forms squared-off T/cross junctions rather than oversized circular
/// discs on every Overture path fragment.
fn junction_hull(mut points:Vec<(f64,f64)>) -> Vec<(f64,f64)> {
    points.sort_by(|a,b|a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup_by(|a,b|(a.0-b.0).abs()<0.0001 && (a.1-b.1).abs()<0.0001);
    if points.len()<3 {return Vec::new();}
    let cross=|a:(f64,f64),b:(f64,f64),c:(f64,f64)| {
        (b.0-a.0)*(c.1-a.1)-(b.1-a.1)*(c.0-a.0)
    };
    let mut lower:Vec<(f64,f64)>=Vec::new();
    for &p in &points {
        while lower.len()>=2 &&
            cross(lower[lower.len()-2],lower[lower.len()-1],p)<=0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper:Vec<(f64,f64)>=Vec::new();
    for &p in points.iter().rev() {
        while upper.len()>=2 &&
            cross(upper[upper.len()-2],upper[upper.len()-1],p)<=0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

pub fn mesh_streets(city: &City) -> Result<(RoadScene, RoadReport), String> {
    let mut scene=RoadScene::default();
    let mut report=RoadReport {source_segments:city.streets.len(),..Default::default()};
    for street in &city.streets {
        let Some((kind,width))=surface_class(street) else {
            report.segments_skipped+=1;
            continue;
        };
        // Render-only raised surface offsets avoid coplanar Z-fighting.
        // Roads should visually cover crossing footpath ribbons on the flat
        // preview. Distinct mapping of footways is not a route overlay and
        // must not be draped over asphalt without surveyed grade information.
        let height=match kind {
            SurfaceKind::Road=>0.08,
            SurfaceKind::Walk=>0.035,
            SurfaceKind::Steps=>0.040,
            SurfaceKind::Rail=>0.085,
        };
        let Some(strip)=ribbon(&street.path,width,height)? else {
            report.segments_degenerate+=1;
            continue;
        };
        let layer=scene.layers.entry(kind).or_default();
        let offset=u32::try_from(layer.vertices.len())
            .map_err(|_| "road layer exceeds 32-bit indices")?;
        if strip.indices.iter().any(|i| i.checked_add(offset).is_none()) {
            return Err("road scene exceeds 32-bit indices".into());
        }
        layer.indices.extend(strip.indices.into_iter().map(|i| i+offset));
        layer.vertices.extend(strip.vertices);
        report.segments_meshed+=1;
        *report.by_surface.entry(kind.material().into()).or_insert(0)+=1;
    }
    add_junction_patches(city,&mut scene,&mut report)?;
    report.total_vertices=scene.layers.values().map(|l| l.vertices.len()).sum();
    report.total_triangles=scene.layers.values().map(|l| l.indices.len()/3).sum();
    Ok((scene,report))
}

/// OBJ references a same-directory 'urbarium-transport.mtl' material library.
/// Groups are kept separate so later WebGPU rendering can batch by surface type.
pub fn write_road_obj(mut out: impl Write, scene: &RoadScene) -> io::Result<()> {
    writeln!(out,"# Urbarium transportation massing, approximate widths in meters")?;
    writeln!(out,"mtllib urbarium-transport.mtl")?;
    let mut base: u64 = 0;
    for (kind,mesh) in &scene.layers {
        for v in &mesh.vertices {
            let [x,y,z]=v.position;
            writeln!(out,"v {x} {y} {z}")?;
        }
        for v in &mesh.vertices {
            let [nx,ny,nz]=v.normal;
            writeln!(out,"vn {nx} {ny} {nz}")?;
        }
        writeln!(out,"g {}",kind.material())?;
        writeln!(out,"usemtl {}",kind.material())?;
        for tri in mesh.indices.chunks_exact(3) {
            write!(out,"f")?;
            for idx in tri {
                let global=base+u64::from(*idx)+1;
                write!(out," {global}//{global}")?;
            }
            writeln!(out)?;
        }
        base+=mesh.vertices.len() as u64;
    }
    Ok(())
}

pub fn write_road_mtl(mut out: impl Write) -> io::Result<()> {
    writeln!(out,"# Neutral debug materials, not a measured surface color")?;
    for (name,color) in [
        ("road_asphalt",[0.24,0.26,0.29]),
        ("walk_pavement",[0.58,0.60,0.59]),
        ("steps_stone",[0.65,0.62,0.56]),
        ("rail_surface",[0.34,0.43,0.48]),
    ] {
        writeln!(out,"newmtl {name}")?;
        writeln!(out,"Kd {} {} {}",color[0],color[1],color[2])?;
        writeln!(out,"Ka 0.08 0.08 0.08")?;
        writeln!(out,"Ks 0.0 0.0 0.0")?;
        writeln!(out,"illum 1")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Provenance,Region};
    fn street(kind:&str,coords:&[[f64;2]])->Street {
        Street {
            source_id:"test".into(),name:None,kind:kind.into(),
            path:coords.iter().map(|c|Point{x:c[0],z:c[1]}).collect(),
            osm_node_ids:Vec::new(),tags:BTreeMap::new(),
        }
    }
    fn city(streets:Vec<Street>)->City {
        City {schema_version:1,
            region:Region {id:"test".into(),name:"Test".into(),
                origin:[-118.25,34.05],bbox:[-118.26,34.04,-118.24,34.06]},
            provenance:Vec::<Provenance>::new(),streets,buildings:Vec::new()}
    }
    #[test]
    fn road_is_correct_width_and_faces_up() {
        let (scene, report)=mesh_streets(&city(vec![street("residential",&[[0.,0.],[10.,0.]])])).unwrap();
        assert_eq!(report.segments_meshed,1);
        let mesh=&scene.layers[&SurfaceKind::Road];
        assert_eq!(mesh.vertices.len(),4);
        assert_eq!(mesh.indices.len(),6);
        assert!((mesh.vertices[0].position[2]-3.25).abs()<1e-4);
        assert!((mesh.vertices[1].position[2]+3.25).abs()<1e-4);
        let p=[
            mesh.vertices[mesh.indices[0] as usize].position,
            mesh.vertices[mesh.indices[1] as usize].position,
            mesh.vertices[mesh.indices[2] as usize].position,
        ];
        let signed=(p[1][0]-p[0][0])*(p[2][2]-p[0][2])
            -(p[1][2]-p[0][2])*(p[2][0]-p[0][0]);
        assert!(signed < 0.0,"winding must face +Y");
    }
    #[test]
    fn bends_are_miter_limited_and_duplicates_skipped() {
        let s=street("footway",&[[0.,0.],[0.,0.],[10.,0.],[10.,10.]]);
        let (scene,report)=mesh_streets(&city(vec![s])).unwrap();
        assert_eq!(report.segments_meshed,1);
        let mesh=&scene.layers[&SurfaceKind::Walk];
        assert_eq!(mesh.vertices.len(),6);
        let a=mesh.vertices[2].position;
        let join=(a[0]-10.0).hypot(a[2]);
        assert!(join <= 2.5+1e-4);
    }
    #[test]
    fn roads_occlude_pedestrian_centerlines_at_same_ground_height() {
        let (scene,_) = mesh_streets(&city(vec![
            street("residential",&[[0.,0.],[10.,0.]]),
            street("footway",&[[5.,-10.],[5.,10.]]),
        ])).unwrap();
        assert!(scene.layers[&SurfaceKind::Road].vertices[0].position[1]
            > scene.layers[&SurfaceKind::Walk].vertices[0].position[1]);
        assert_eq!(scene.layers[&SurfaceKind::Walk].vertices.len(),4);
        assert_eq!(scene.layers[&SurfaceKind::Road].vertices.len(),4);
    }

    #[test]
    fn missing_unknown_and_subway_are_not_surface_roads() {
        let mut invalid=street("primary",&[[0.,0.],[10.,0.]]);
        invalid.tags.insert("width".into(),"999m".into());
        let (scene,report)=mesh_streets(&city(vec![
            invalid,street("subway",&[[0.,0.],[10.,0.]]),
            street("unknown",&[[0.,0.],[10.,0.]]),
            street("steps",&[[0.,0.],[0.,0.]]),
        ])).unwrap();
        assert_eq!(report.segments_meshed,1);
        assert_eq!(report.segments_skipped,2);
        assert_eq!(report.segments_degenerate,1);
        assert_eq!(scene.layers[&SurfaceKind::Road].indices.len(),6);
    }
    #[test]
    fn shared_t_junction_gets_a_square_asphalt_patch() {
        let (scene,report)=mesh_streets(&city(vec![
            street("residential",&[[-20.,0.],[0.,0.]]),
            street("residential",&[[0.,0.],[20.,0.]]),
            street("service",&[[0.,0.],[0.,20.]]),
        ])).unwrap();
        assert_eq!(report.junction_patches,1);
        let road=&scene.layers[&SurfaceKind::Road];
        assert!(road.vertices.len()>12);
        assert!(road.indices.len()/3>6);
        assert!(road.indices.iter().all(|&i|(i as usize)<road.vertices.len()));
    }
    #[test]
    fn unconnected_crossings_and_bridges_get_no_fake_junctions() {
        let mut bridge=street("primary",&[[0.,0.],[0.,10.]]);
        bridge.tags.insert("bridge".into(),"yes".into());
        let (scene,report)=mesh_streets(&city(vec![
            street("primary",&[[-10.,0.],[10.,0.]]),
            bridge,
        ])).unwrap();
        assert_eq!(report.junction_patches,0);
        assert_eq!(scene.layers[&SurfaceKind::Road].vertices.len(),8);
    }

    #[test]
    fn obj_groups_and_materials_are_present() {
        let (scene,_)=mesh_streets(&city(vec![
            street("footway",&[[0.,0.],[5.,0.]]),
            street("primary",&[[0.,10.],[5.,10.]]),
        ])).unwrap();
        let mut obj=Vec::new();
        write_road_obj(&mut obj,&scene).unwrap();
        let obj=String::from_utf8(obj).unwrap();
        assert!(obj.contains("mtllib urbarium-transport.mtl"));
        assert!(obj.contains("g road_asphalt"));
        assert!(obj.contains("g walk_pavement"));
        assert_eq!(obj.lines().filter(|l|l.starts_with("f ")).count(),4);
        let mut mtl=Vec::new();
        write_road_mtl(&mut mtl).unwrap();
        assert!(String::from_utf8(mtl).unwrap().contains("newmtl walk_pavement"));
    }
}
