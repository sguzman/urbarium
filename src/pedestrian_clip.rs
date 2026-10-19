//! Exact asphalt-first ownership of ground-level surface triangles.
//!
//! Overture pedestrian lines are centerlines, not surveyed curb polygons.
//! Rendering them just below asphalt is NOT sufficient on DEM slopes: the
//! surfaces can still show through. Remove only the planar overlap between
//! rendered road triangles and pedestrian triangles. Keep the rest of each
//! pedestrian triangle, rather than dropping whole strips at intersections.
//! Source street/path centerlines are never modified.
//!
//! Only actual at-grade Road/Motorway meshes mask Walk/Steps. Hidden freeways,
//! elevated bridge previews and underground linework do not carve fake gaps.

use crate::geometry::{Mesh, Vertex};
use crate::roads::{RoadScene, SurfaceKind};
use std::collections::{BTreeMap, BTreeSet};

const GRID_M: f64 = 24.0;
const MIN_AREA_M2: f64 = 0.00001;
type Bounds = [f64; 4];

#[derive(Clone, Copy)]
struct RoadTriangle {
    vertices: [Vertex; 3],
    bounds: Bounds,
}

fn point(v: Vertex) -> [f64; 2] {
    [v.position[0] as f64, v.position[2] as f64]
}
fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) -
    (b[1] - a[1]) * (p[0] - a[0])
}
fn signed_area(poly: &[Vertex]) -> f64 {
    if poly.len() < 3 { return 0.0; }
    let mut area = 0.0;
    for i in 0..poly.len() {
        let a = point(poly[i]);
        let b = point(poly[(i+1) % poly.len()]);
        area += a[0]*b[1] - b[0]*a[1];
    }
    area * 0.5
}
fn bounds(poly: &[Vertex]) -> Bounds {
    let mut b = [f64::INFINITY, f64::INFINITY,
                 f64::NEG_INFINITY, f64::NEG_INFINITY];
    for &v in poly {
        let p = point(v);
        b[0] = b[0].min(p[0]); b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]); b[3] = b[3].max(p[1]);
    }
    b
}
fn overlaps(a: Bounds, b: Bounds) -> bool {
    a[0] < b[2] && a[2] > b[0] &&
    a[1] < b[3] && a[3] > b[1]
}
fn lerp(a: Vertex, b: Vertex, t: f64) -> Vertex {
    let mut v = a;
    for i in 0..3 {
        v.position[i] = (a.position[i] as f64 +
            (b.position[i] as f64 - a.position[i] as f64)*t) as f32;
        v.normal[i] = (a.normal[i] as f64 +
            (b.normal[i] as f64 - a.normal[i] as f64)*t) as f32;
    }
    v
}
/// Split a convex polygon by a directed boundary. Both returned polygons
/// share the exact interpolated crossing vertices.
fn split(poly: &[Vertex], a: [f64; 2], b: [f64; 2],
         orientation: f64) -> (Vec<Vertex>, Vec<Vertex>) {
    let mut inside = Vec::with_capacity(poly.len()+1);
    let mut outside = Vec::with_capacity(poly.len()+1);
    if poly.is_empty() { return (inside,outside); }
    let mut prev = *poly.last().unwrap();
    let mut last = orientation*cross(a,b,point(prev));
    for &curr in poly {
        let now = orientation*cross(a,b,point(curr));
        // Boundaries belong to asphalt; the overlapping WALK triangle has
        // no area there. Both polygons may share boundary vertices.
        let prev_in = last >= 0.0;
        let curr_in = now >= 0.0;
        if prev_in != curr_in {
            let denominator = last-now;
            if denominator.abs()>1e-18 {
                let v=lerp(prev,curr,(last/denominator).clamp(0.0,1.0));
                inside.push(v);
                outside.push(v);
            }
        }
        if curr_in { inside.push(curr); }
        else { outside.push(curr); }
        prev=curr;
        last=now;
    }
    (inside,outside)
}
/// Subtract one asphalt triangle from one convex pedestrian polygon.
/// On each edge, emit the outside area and carry the inside to the next
/// edge. Output pieces partition the difference and retain source winding.
fn subtract(poly: Vec<Vertex>, road: [Vertex; 3]) -> Vec<Vec<Vertex>> {
    if !overlaps(bounds(&poly),bounds(&road)) {return vec![poly];}
    let road_area=signed_area(&road);
    if road_area.abs()<MIN_AREA_M2 {return vec![poly];}
    let orientation=road_area.signum();
    let mut remaining=poly;
    let mut output=Vec::new();
    for i in 0..3 {
        let a=point(road[i]);
        let b=point(road[(i+1)%3]);
        let (inside,outside)=split(&remaining,a,b,orientation);
        if signed_area(&outside).abs()>MIN_AREA_M2 {
            output.push(outside);
        }
        if signed_area(&inside).abs()<=MIN_AREA_M2 {
            return output;
        }
        remaining=inside;
    }
    // Remaining polygon is inside the street and is deliberately discarded.
    output
}

fn triangle(mesh: &Mesh, indices: &[u32]) -> Result<[Vertex; 3], String> {
    let [a,b,c]=[indices[0] as usize,indices[1] as usize,indices[2] as usize];
    let get=|i:usize|mesh.vertices.get(i).copied()
        .ok_or_else(||"transport mesh triangle references missing vertex".to_string());
    let tri=[get(a)?,get(b)?,get(c)?];
    if !tri.iter().flat_map(|v|v.position.iter().chain(v.normal.iter()))
        .all(|n|n.is_finite()) {
        return Err("transport mesh contains non-finite vertices".into());
    }
    Ok(tri)
}

fn emit(poly: &[Vertex], output: &mut Mesh) -> Result<(), String> {
    if poly.len()<3 {return Ok(());}
    // A retained convex piece fans into triangles preserving input winding.
    for i in 1..poly.len()-1 {
        let vertices=[poly[0],poly[i],poly[i+1]];
        if signed_area(&vertices).abs()<=MIN_AREA_M2 {continue;}
        let base=u32::try_from(output.vertices.len())
            .map_err(|_|"pedestrian clip exceeds u32 vertices")?;
        if base>u32::MAX-3 {return Err("pedestrian clip exceeds u32 indices".into());}
        output.vertices.extend(vertices);
        output.indices.extend([base,base+1,base+2]);
    }
    Ok(())
}

fn cells(b:Bounds) -> impl Iterator<Item=(i32,i32)> {
    let minx=(b[0]/GRID_M).floor() as i32;
    let maxx=(b[2]/GRID_M).floor() as i32;
    let minz=(b[1]/GRID_M).floor() as i32;
    let maxz=(b[3]/GRID_M).floor() as i32;
    (minx..=maxx).flat_map(move |x|(minz..=maxz).map(move |z|(x,z)))
}

fn clipped(walk: &Mesh, roads: &[RoadTriangle],
           grid: &BTreeMap<(i32,i32),Vec<usize>>) -> Result<(Mesh,usize),String> {
    if walk.indices.len()%3!=0 {return Err("invalid pedestrian index count".into());}
    // Reuse the indexed source vertices and triangles whenever they never
    // intersect asphalt. Only intersection fragments need new vertices.
    // This avoids exploding normal footpath GPU memory/draw work.
    let mut output=Mesh{vertices:walk.vertices.clone(),indices:Vec::new()};
    let mut affected=0;
    for indices in walk.indices.chunks_exact(3) {
        let original=triangle(walk,indices)?;
        let b=bounds(&original);
        let mut candidates=BTreeSet::new();
        for cell in cells(b) {
            if let Some(list)=grid.get(&cell) {
                candidates.extend(list.iter().copied());
            }
        }
        if candidates.is_empty(){
            output.indices.extend_from_slice(indices);
            continue;
        }
        let mut pieces=vec![original.to_vec()];
        for id in candidates {
            let road=&roads[id];
            if !overlaps(b,road.bounds){continue;}
            let mut next=Vec::new();
            for piece in pieces {
                next.extend(subtract(piece,road.vertices));
            }
            pieces=next;
            if pieces.is_empty(){break;}
        }
        let retained_area:f64=pieces.iter().map(|p|signed_area(p).abs()).sum();
        if signed_area(&original).abs()-retained_area>MIN_AREA_M2 {
            affected+=1;
            for piece in pieces {emit(&piece,&mut output)?;}
        }else{
            // No positive-area crossing; keep original compact indexed mesh.
            output.indices.extend_from_slice(indices);
        }
    }
    Ok((output,affected))
}

/// Enforce asphalt > pedestrian precedence geometrically, before terrain
/// draping or cell clipping. Called on continuous region-local source meshes,
/// so camera movement and chunk boundaries cannot reintroduce walk patches.
/// Returns original pedestrian triangles actually affected (not output count).
pub fn clip_walks_below_asphalt(scene: &mut RoadScene) -> Result<usize,String> {
    let mut roads=Vec::<RoadTriangle>::new();
    let mut grid=BTreeMap::<(i32,i32),Vec<usize>>::new();
    for kind in [SurfaceKind::Road,SurfaceKind::Motorway] {
        let Some(mesh)=scene.layers.get(&kind) else {continue;};
        if mesh.indices.len()%3!=0 {return Err("invalid asphalt index count".into());}
        for indices in mesh.indices.chunks_exact(3) {
            let vertices=triangle(mesh,indices)?;
            if signed_area(&vertices).abs()<=MIN_AREA_M2 {continue;}
            let b=bounds(&vertices);
            let id=roads.len();
            roads.push(RoadTriangle{vertices,bounds:b});
            for cell in cells(b) {grid.entry(cell).or_default().push(id);}
        }
    }
    if roads.is_empty(){return Ok(0);}
    let mut total=0;
    for kind in [SurfaceKind::Walk,SurfaceKind::Steps] {
        let Some(walk)=scene.layers.get(&kind) else {continue;};
        let (new_mesh,count)=clipped(walk,&roads,&grid)?;
        scene.layers.insert(kind,new_mesh);
        total+=count;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{City, Point, Region, Street};
    use crate::roads::mesh_streets_with_bridges;
    use std::collections::BTreeMap;
    fn street(kind:&str, pts:&[(f64,f64)])->Street {
        Street{source_id:kind.into(),name:None,kind:kind.into(),
            path:pts.iter().map(|&(x,z)|Point{x,z}).collect(),
            tags:BTreeMap::new(),osm_node_ids:vec![]}
    }
    fn scene(streets:Vec<Street>, bridges:bool)->RoadScene {
        mesh_streets_with_bridges(&City {
            schema_version:1,region:Region{id:"test".into(),name:"test".into(),
                origin:[0.,0.],bbox:[-0.01,-0.01,0.01,0.01]},
            provenance:vec![],streets,buildings:vec![],
        },bridges).unwrap().0
    }
    fn area(mesh:&Mesh)->f64 {
        mesh.indices.chunks_exact(3).map(|indices|
            signed_area(&indices.iter()
                .map(|&i|mesh.vertices[i as usize]).collect::<Vec<_>>()).abs()).sum()
    }
    // Positive-area overlap is the invariant, not depth ordering or a
    // misleading screenshot. Every asphalt triangle must have zero remaining
    // pedestrian intersection area, including interior-only crossings.
    fn intersection_area(walk:&Mesh, asphalt:&Mesh)->f64 {
        let mut total=0.0;
        for tri in walk.indices.chunks_exact(3) {
            let wp=[tri[0],tri[1],tri[2]].map(|i|walk.vertices[i as usize]);
            for rt in asphalt.indices.chunks_exact(3) {
                let rp=[rt[0],rt[1],rt[2]].map(|i|asphalt.vertices[i as usize]);
                if !overlaps(bounds(&wp),bounds(&rp)){continue;}
                let mut inside=wp.to_vec();
                let orientation=signed_area(&rp).signum();
                for k in 0..3 {
                    let (s,_)=split(&inside,point(rp[k]),point(rp[(k+1)%3]),orientation);
                    inside=s;
                    if inside.len()<3 {break;}
                }
                total+=signed_area(&inside).abs();
            }
        }
        total
    }
    #[test]
    fn road_priority_cuts_actual_crossing_out_of_walk_mesh() {
        let output=scene(vec![
            street("residential",&[(-15.,0.),(15.,0.)]),
            street("footway",&[(0.,-15.),(0.,15.)])],false);
        assert!(output.layers.contains_key(&SurfaceKind::Road));
        let asphalt=&output.layers[&SurfaceKind::Road];
        let walk=&output.layers[&SurfaceKind::Walk];
        assert!(area(walk)>0.0,"walking surface outside the street survives");
        assert!(intersection_area(walk,asphalt)<0.002,
            "no pedestrian pavement can remain within rendered asphalt");
        assert!(walk.indices.iter().any(|&i|walk.vertices[i as usize].position[2]< -4.0));
        assert!(walk.indices.iter().any(|&i|walk.vertices[i as usize].position[2]>4.0));
    }
    #[test]
    fn diagonal_walk_crossing_and_multiple_streets_leave_no_asphalt_overlap() {
        let output=scene(vec![
            street("primary",&[(-15.,0.),(15.,0.)]),
            street("residential",&[(0.,-15.),(0.,15.)]),
            street("footway",&[(-15.,-15.),(15.,15.)])],false);
        assert!(intersection_area(&output.layers[&SurfaceKind::Walk],
            &output.layers[&SurfaceKind::Road])<0.003);
    }
    #[test]
    fn both_walks_and_steps_are_masked_by_actual_road_mesh() {
        let output=scene(vec![
            street("primary",&[(-15.,0.),(15.,0.)]),
            street("footway",&[(0.,-15.),(0.,15.)]),
            street("steps",&[(5.,-15.),(5.,15.)])],false);
        for kind in [SurfaceKind::Walk,SurfaceKind::Steps] {
            assert!(intersection_area(&output.layers[&kind],
                &output.layers[&SurfaceKind::Road])<0.003);
        }
    }
    #[test]
    fn hidden_freeway_and_tagged_bridge_do_not_cut_at_grade_walks() {
        let walk=street("footway",&[(0.,-15.),(0.,15.)]);
        let empty=scene(vec![walk.clone()],false);
        let freeway=scene(vec![street("motorway",&[(-15.,0.),(15.,0.)]),walk.clone()],false);
        assert_eq!(area(&freeway.layers[&SurfaceKind::Walk]),area(&empty.layers[&SurfaceKind::Walk]));
        let mut bridge=street("primary",&[(-15.,0.),(15.,0.)]);
        bridge.tags.insert("bridge".into(),"yes".into());
        let bridge_scene=scene(vec![bridge,walk],false);
        assert_eq!(area(&bridge_scene.layers[&SurfaceKind::Walk]),area(&empty.layers[&SurfaceKind::Walk]));
    }
    #[test]
    fn polygon_subtraction_preserves_both_sides_of_large_crossing() {
        let v=|x:f32,z:f32|Vertex{position:[x,0.09,z],normal:[0.,1.,0.]};
        let poly=vec![v(-10.,-10.),v(10.,-10.),v(0.,10.)];
        let road=[v(-3.,-2.),v(3.,-2.),v(0.,2.)];
        let parts=subtract(poly.clone(),road);
        let kept:f64=parts.iter().map(|p|signed_area(p).abs()).sum();
        let covered=signed_area(&road).abs();
        assert!(parts.len()>=3,"the inner road triangle must carve a hole");
        assert!((kept+covered-signed_area(&poly).abs()).abs()<0.002);
    }
}
