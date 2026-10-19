//! Deterministic low-poly massing meshes from real geographic footprints.
//! Retains courtyard openings and concave roofs; not photogrammetry.

use crate::{Building, City, Point};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HeightPolicy {
    /// Render-only assumption when no height/levels are known.
    pub fallback_m: f64,
    /// Render-only assumption when building levels exist but height does not.
    pub meters_per_level: f64,
}
impl Default for HeightPolicy {
    fn default() -> Self { Self { fallback_m: 9.0, meters_per_level: 3.1 } }
}
impl HeightPolicy {
    pub fn validate(self) -> Result<(), String> {
        if !self.fallback_m.is_finite() || self.fallback_m <= 0.0
            || !self.meters_per_level.is_finite() || self.meters_per_level <= 0.0
        {
            return Err("height policy values must be positive finite meters".into());
        }
        Ok(())
    }
    fn height_for(self, building: &Building) -> f64 {
        building.height_m
            .or_else(|| building.levels.map(|n| n * self.meters_per_level))
            .unwrap_or(self.fallback_m)
    }
    /// Measured source clearance is authoritative; floor count is an explicitly
    /// approximate fallback only when the metric clearance was not observed.
    pub(crate) fn base_for(self,building:&Building)->f64 {
        building.min_height_m.unwrap_or_else(||
            building.min_floor.map(|floor|floor as f64*self.meters_per_level)
            .unwrap_or(0.0))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MeshReport {
    pub buildings_meshed: usize,
    pub buildings_skipped: usize,
    pub vertices: usize,
    pub triangles: usize,
    /// Display-only heights produced from levels/default, not observed source data.
    pub buildings_with_approximate_heights: usize,
}

pub fn mesh_city(city: &City, policy: HeightPolicy) -> Result<(Mesh, MeshReport), String> {
    policy.validate()?;
    let mut combined = Mesh::default();
    let mut report = MeshReport::default();
    for building in &city.buildings {
        match mesh_building(building, policy) {
            Ok(mesh) => {
                let offset = u32::try_from(combined.vertices.len())
                    .map_err(|_| "city vertex count exceeds 32-bit index space")?;
                if mesh.indices.iter().any(|i| i.checked_add(offset).is_none()) {
                    return Err("city geometry exceeds 32-bit index space".into());
                }
                combined.indices.extend(mesh.indices.into_iter().map(|i| i + offset));
                combined.vertices.extend(mesh.vertices);
                report.buildings_meshed += 1;
                if building.height_m.is_none() {
                    report.buildings_with_approximate_heights += 1;
                }
            }
            Err(_) => report.buildings_skipped += 1,
        }
    }
    report.vertices = combined.vertices.len();
    report.triangles = combined.indices.len() / 3;
    Ok((combined, report))
}

pub fn mesh_building(building: &Building, policy: HeightPolicy) -> Result<Mesh, String> {
    policy.validate()?;
    let height = policy.height_for(building);
    let base=policy.base_for(building);
    let roof=base+height;
    if !height.is_finite()||height<=0.0||!base.is_finite()||
       base<0.0||!roof.is_finite()||roof<=base{
        return Err(format!("{} has an invalid geometry height/base", building.source_id));
    }
    let mut mesh = Mesh::default();
    for polygon in &building.polygons {
        if polygon.rings.is_empty() || polygon.rings.iter().any(|r| r.len() < 3) {
            return Err(format!("{} contains an empty/degenerate ring", building.source_id));
        }
        let mut flat = Vec::new();
        let mut holes = Vec::new();
        let mut points = Vec::new();
        for (ring_id, ring) in polygon.rings.iter().enumerate() {
            if ring_id > 0 { holes.push(points.len()); }
            for p in ring {
                if !p.x.is_finite() || !p.z.is_finite() {
                    return Err(format!("{} contains non-finite points", building.source_id));
                }
                flat.extend([p.x, p.z]);
                points.push(*p);
            }
        }
        let triangles = earcutr::earcut(&flat, &holes, 2)
            .map_err(|e| format!("{} triangulation: {e:?}", building.source_id))?;
        if triangles.is_empty() || triangles.len() % 3 != 0 {
            return Err(format!("{} roof triangulation failed", building.source_id));
        }
        let offset = u32::try_from(mesh.vertices.len()).map_err(|_| "mesh too large")?;
        for p in &points {
            mesh.vertices.push(Vertex {
                position: [p.x as f32, roof as f32, p.z as f32],
                normal: [0.0, 1.0, 0.0],
            });
        }
        // Suspended buildings need a physical underside for upward views.
        // Ground-standing buildings retain their original wall and roof count.
        let under_offset=if base>0.0 {
            let start=u32::try_from(mesh.vertices.len()).map_err(|_|"mesh too large")?;
            for p in &points {
                mesh.vertices.push(Vertex{
                    position:[p.x as f32,base as f32,p.z as f32],
                    normal:[0.0,-1.0,0.0],
                });
            }
            Some(start)
        }else{None};
        for tri in triangles.chunks_exact(3) {
            let (a,b,c) = (points[tri[0]],points[tri[1]],points[tri[2]]);
            let signed_area = (b.x-a.x)*(c.z-a.z) - (b.z-a.z)*(c.x-a.x);
            if signed_area.abs() < 1e-12 { continue; }
            let inds = if signed_area > 0.0 {
                [tri[0], tri[2], tri[1]]
            } else {
                [tri[0], tri[1], tri[2]]
            };
            mesh.indices.extend(inds.into_iter().map(|i| offset + i as u32));
            if let Some(under)=under_offset {
                mesh.indices.extend(inds.into_iter().rev().map(|i|under+i as u32));
            }
        }
        for (ring_id, ring) in polygon.rings.iter().enumerate() {
            let area: f64 = ring.iter().zip(ring.iter().cycle().skip(1))
                .take(ring.len()).map(|(a,b)| a.x*b.z-b.x*a.z).sum();
            if area.abs() < 1e-12 { continue; }
            let exterior_sign = if area > 0.0 { 1.0 } else { -1.0 };
            let wall_sign = if ring_id == 0 { exterior_sign } else { -exterior_sign };
            for i in 0..ring.len() {
                let a: Point = ring[i];
                let b: Point = ring[(i+1) % ring.len()];
                let dx = b.x-a.x;
                let dz = b.z-a.z;
                let length = dx.hypot(dz);
                if length <= 1e-9 { continue; }
                let normal = [
                    (wall_sign*dz/length) as f32,
                    0.0,
                    (-wall_sign*dx/length) as f32,
                ];
                let v = u32::try_from(mesh.vertices.len()).map_err(|_| "mesh too large")?;
                mesh.vertices.extend([
                    Vertex { position: [a.x as f32, base as f32, a.z as f32], normal },
                    Vertex { position: [b.x as f32, base as f32, b.z as f32], normal },
                    Vertex { position: [b.x as f32, roof as f32, b.z as f32], normal },
                    Vertex { position: [a.x as f32, roof as f32, a.z as f32], normal },
                ]);
                if wall_sign > 0.0 {
                    mesh.indices.extend([v, v+2, v+1, v, v+3, v+2]);
                } else {
                    mesh.indices.extend([v, v+1, v+2, v, v+2, v+3]);
                }
            }
        }
    }
    if mesh.indices.is_empty() { return Err("empty building geometry".into()); }
    Ok(mesh)
}

/// Portable plain-text 3D artifact for validating and inspecting geometry.
/// The interactive WebGPU renderer will consume indexed arrays directly.
pub fn write_obj(mut out: impl Write, mesh: &Mesh) -> io::Result<()> {
    writeln!(out, "# Urbarium procedural massing mesh (meters, X east / Z north)")?;
    for vertex in &mesh.vertices {
        let [x,y,z] = vertex.position;
        writeln!(out, "v {x} {y} {z}")?;
    }
    for vertex in &mesh.vertices {
        let [x,y,z] = vertex.normal;
        writeln!(out, "vn {x} {y} {z}")?;
    }
    for triangle in mesh.indices.chunks_exact(3) {
        let mut corners = [String::new(),String::new(),String::new()];
        for (index, source) in triangle.iter().enumerate() {
            let n = source + 1;
            corners[index] = format!("{n}//{n}");
        }
        writeln!(out, "f {} {} {}", corners[0], corners[1], corners[2])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Polygon;

    fn square(name: &str, rings: Vec<Vec<Point>>, height: Option<f64>) -> Building {
        Building {
            source_id:name.into(), name:None, height_m:height, levels:None,
            min_height_m:None,min_floor:None,parts:Vec::new(),polygons:vec![Polygon { rings }]
        }
    }
    fn ring(points: &[[f64;2]]) -> Vec<Point> {
        points.iter().map(|xy| Point{x:xy[0],z:xy[1]}).collect()
    }

    #[test]
    fn square_prism_has_up_roof_and_wall_triangles() {
        let b = square("square", vec![ring(&[
            [0.,0.],[10.,0.],[10.,10.],[0.,10.]
        ])], Some(12.));
        let mesh = mesh_building(&b, HeightPolicy::default()).unwrap();
        assert_eq!(mesh.vertices.len(), 20); // four roof + 16 wall vertices
        assert_eq!(mesh.indices.len() / 3, 10); // two roof + eight wall triangles
        assert!(mesh.vertices.iter().take(4).all(|v| v.normal == [0.,1.,0.]));
        assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()));
    }

    #[test]
    fn measured_building_clearance_lifts_walls_and_roof_without_ground_walls(){
        let mut b=square("over-driveway",vec![ring(&[
            [0.,0.],[10.,0.],[10.,10.],[0.,10.]])],Some(9.0));
        b.min_height_m=Some(4.0);
        b.min_floor=Some(3); // measured meters win over the floor proxy
        let mesh=mesh_building(&b,HeightPolicy::default()).unwrap();
        let upper:Vec<_>=mesh.vertices.iter()
            .filter(|v|v.normal[1]>0.9).collect();
        let walls:Vec<_>=mesh.vertices.iter()
            .filter(|v|v.normal[1].abs()<0.1).collect();
        assert!(upper.iter().all(|v|(v.position[1]-13.0).abs()<0.0001));
        let underside:Vec<_>=mesh.vertices.iter().filter(|v|v.normal[1]< -0.9).collect();
        assert_eq!(underside.len(),4);
        assert!(underside.iter().all(|v|(v.position[1]-4.0).abs()<0.0001));
        assert!(walls.iter().all(|v|
            (v.position[1]-4.0).abs()<0.0001 ||
            (v.position[1]-13.0).abs()<0.0001));
        assert_eq!(mesh.indices.len()/3,12);
    }
    #[test]
    fn min_floor_fallback_is_only_a_render_time_height_assumption(){
        let mut b=square("floor-proxy",vec![ring(&[
            [0.,0.],[10.,0.],[10.,10.],[0.,10.]])],Some(8.0));
        b.min_floor=Some(2);
        let mesh=mesh_building(&b,HeightPolicy::default()).unwrap();
        assert!((mesh.vertices[0].position[1]-14.2).abs()<0.0002);
        assert_eq!(b.min_height_m,None);
        assert_eq!(b.min_floor,Some(2));
    }
    #[test]
    fn no_source_clearance_retains_existing_ground_level_mesh(){
        let b=square("old",vec![ring(&[
            [0.,0.],[10.,0.],[10.,10.],[0.,10.]])],Some(8.0));
        let mesh=mesh_building(&b,HeightPolicy::default()).unwrap();
        assert!(mesh.vertices.iter().any(|v|v.normal[1]==0.0 && v.position[1]==0.0));
        assert!(mesh.vertices.iter().any(|v|v.normal[1]==0.0 && v.position[1]==8.0));
    }

    #[test]
    fn courtyard_roof_respects_hole_area() {
        let b = square("courtyard", vec![
            ring(&[[0.,0.],[10.,0.],[10.,10.],[0.,10.]]),
            ring(&[[3.,3.],[7.,3.],[7.,7.],[3.,7.]])
        ], Some(14.));
        let mesh = mesh_building(&b, HeightPolicy::default()).unwrap();
        let mut roof_area = 0.0;
        for tri in mesh.indices.chunks_exact(3) {
            let verts = [
                mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],
                mesh.vertices[tri[2] as usize],
            ];
            if !verts.iter().all(|v| v.normal[1] > 0.9) { continue; }
            let dx1 = (verts[1].position[0]-verts[0].position[0]) as f64;
            let dz1 = (verts[1].position[2]-verts[0].position[2]) as f64;
            let dx2 = (verts[2].position[0]-verts[0].position[0]) as f64;
            let dz2 = (verts[2].position[2]-verts[0].position[2]) as f64;
            roof_area += (dx1*dz2-dz1*dx2).abs()*0.5;
        }
        assert!((roof_area-84.0).abs() < 0.01);
    }

    #[test]
    fn fallback_height_is_render_only_and_obj_serializes() {
        let mut b = square("levels", vec![ring(&[
            [0.,0.],[1.,0.],[1.,1.],[0.,1.]
        ])], None);
        b.levels=Some(2.0);
        let mesh=mesh_building(&b, HeightPolicy::default()).unwrap();
        assert_eq!(mesh.vertices[0].position[1], 6.2_f32);
        let mut output=Vec::new();
        write_obj(&mut output,&mesh).unwrap();
        let text=String::from_utf8(output).unwrap();
        assert!(text.contains("\nvn "));
        assert_eq!(text.lines().filter(|line| line.starts_with("f ")).count(), 10);
    }
}
