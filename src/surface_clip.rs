//! Exact horizontal ownership of *finished* surface triangles by render cell.
//!
//! Centerline clipping before ribbon generation loses neighboring bend normals,
//! fabricates endpoints, and causes road gaps/overlaps at every new chunk seam.
//! Build the continuous source surface first, then cut triangles against the
//! cell's geographic rectangle. Preserve unclipped vertices by index so the
//! common case remains cheap. Buildings are intentionally NOT cut here.

use crate::geometry::{Mesh, Vertex};
use std::collections::HashMap;

fn inside(v: Vertex, axis: usize, limit: f32, keep_greater: bool) -> bool {
    if keep_greater { v.position[axis] >= limit }
    else { v.position[axis] <= limit }
}

fn interpolate(a: Vertex, b: Vertex, axis: usize, limit: f32) -> Vertex {
    let delta = b.position[axis] - a.position[axis];
    let t = if delta.abs() < 1e-12 { 0.0 }
        else { ((limit - a.position[axis]) / delta).clamp(0.0, 1.0) };
    let mut result = a;
    for k in 0..3 {
        result.position[k] = a.position[k] + (b.position[k] - a.position[k]) * t;
        result.normal[k] = a.normal[k] + (b.normal[k] - a.normal[k]) * t;
    }
    // Snap to the identical floating-point seam used by the adjacent tile.
    result.position[axis] = limit;
    result
}

fn clip_polygon(mut polygon: Vec<Vertex>, axis: usize, limit: f32,
    keep_greater: bool) -> Vec<Vertex> {
    if polygon.is_empty() { return polygon; }
    let mut clipped = Vec::with_capacity(polygon.len() + 1);
    let mut previous = *polygon.last().unwrap();
    let mut previous_inside = inside(previous, axis, limit, keep_greater);
    for current in polygon.drain(..) {
        let current_inside = inside(current, axis, limit, keep_greater);
        if previous_inside != current_inside {
            clipped.push(interpolate(previous, current, axis, limit));
        }
        if current_inside { clipped.push(current); }
        previous = current;
        previous_inside = current_inside;
    }
    clipped
}

fn area_xz(a: Vertex, b: Vertex, c: Vertex) -> f64 {
    let ax = a.position[0] as f64;
    let az = a.position[2] as f64;
    ((b.position[0] as f64 - ax) * (c.position[2] as f64 - az)
        - (b.position[2] as f64 - az) * (c.position[0] as f64 - ax)).abs() * 0.5
}

// Share existing and newly cut polygon vertices by their exact f32 bit pattern.
// Avoid geometric quantization here: independently classified overlapping
// pavements and elevation levels must never be accidentally merged.
type VertexKey = [u32;6];
fn vertex_key(v:Vertex)->VertexKey {
    [v.position[0].to_bits(),v.position[1].to_bits(),v.position[2].to_bits(),
     v.normal[0].to_bits(),v.normal[1].to_bits(),v.normal[2].to_bits()]
}
fn intern(out:&mut Mesh,lookup:&mut HashMap<VertexKey,u32>,v:Vertex)->Result<u32,String>{
    let key=vertex_key(v);
    if let Some(&index)=lookup.get(&key){return Ok(index);}
    let index=u32::try_from(out.vertices.len())
        .map_err(|_|"surface vertex count overflow")?;
    out.vertices.push(v);
    lookup.insert(key,index);
    Ok(index)
}

/// Clip asphalt, walk, step and rail surfaces AFTER offsets, bends and junctions
/// have been computed. Bounds are [west_x, south_z, east_x, north_z] in meters.
/// The inside and neighboring tile use the same X/Z boundaries, without
/// duplicating surface area or dropping triangles that straddle a seam.
pub fn clip_surface_to_bounds(mesh: &Mesh, bounds: [f64; 4]) -> Result<Mesh, String> {
    if bounds.iter().any(|v| !v.is_finite()) ||
        bounds[0] >= bounds[2] || bounds[1] >= bounds[3] ||
        mesh.indices.len() % 3 != 0 {
        return Err("invalid surface clipping input".into());
    }
    let edges = [(0usize, bounds[0] as f32, true),
                 (0usize, bounds[2] as f32, false),
                 (2usize, bounds[1] as f32, true),
                 (2usize, bounds[3] as f32, false)];
    let mut out = Mesh::default();
    let mut lookup=HashMap::<VertexKey,u32>::new();
    let mut reused = vec![None; mesh.vertices.len()];
    for indices in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [indices[0] as usize, indices[1] as usize, indices[2] as usize];
        if a >= mesh.vertices.len() || b >= mesh.vertices.len() || c >= mesh.vertices.len() {
            return Err("surface triangle references missing vertex".into());
        }
        let triangle = [mesh.vertices[a], mesh.vertices[b], mesh.vertices[c]];
        if triangle.iter().all(|v| edges.iter().all(|&(axis, limit, greater)|
            inside(*v, axis, limit, greater))) {
            // No triangle expansion or new vertices for the interior majority.
            for &index in indices {
                let slot = &mut reused[index as usize];
                let mapped = match *slot {
                    Some(mapped) => mapped,
                    None => {
                        let mapped=intern(&mut out,&mut lookup,mesh.vertices[index as usize])?;
                        *slot=Some(mapped);
                        mapped
                    }
                };
                out.indices.push(mapped);
            }
            continue;
        }
        // Avoid allocating polygons for triangles wholly outside a plane.
        if edges.iter().any(|&(axis, limit, greater)| triangle.iter()
            .all(|v| !inside(*v, axis, limit, greater))) { continue; }
        let mut poly = triangle.to_vec();
        for &(axis, limit, greater) in &edges {
            poly = clip_polygon(poly, axis, limit, greater);
            if poly.len() < 3 { break; }
        }
        if poly.len() < 3 { continue; }
        for i in 1..poly.len()-1 {
            let [a,b,c] = [poly[0], poly[i], poly[i+1]];
            // Boundary-only touches have no geographic area or road surface.
            if area_xz(a,b,c) < 1e-7 { continue; }
            let ids=[
                intern(&mut out,&mut lookup,a)?,
                intern(&mut out,&mut lookup,b)?,
                intern(&mut out,&mut lookup,c)?,
            ];
            if ids[0]!=ids[1] && ids[0]!=ids[2] && ids[1]!=ids[2] {
                out.indices.extend(ids);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roads::{mesh_streets_with_bridges,SurfaceKind};
    use crate::{City,Point,Region,Street};
    use std::collections::BTreeMap;

    fn area(mesh:&Mesh)->f64 {
        mesh.indices.chunks_exact(3).map(|tri|
            area_xz(mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],mesh.vertices[tri[2] as usize])
        ).sum()
    }
    fn street(points:&[(f64,f64)])->Street {
        Street {source_id:"bend".into(),name:None,kind:"residential".into(),
            path:points.iter().map(|&(x,z)|Point{x,z}).collect(),
            osm_node_ids:vec![],tags:BTreeMap::new()}
    }
    fn road(points:&[(f64,f64)])->Mesh {
        let city=City{schema_version:1,region:Region{id:"test".into(),
            name:"Test".into(),origin:[0.0,0.0],
            bbox:[-0.01,-0.01,0.01,0.01]},provenance:vec![],
            streets:vec![street(points)],buildings:vec![]};
        mesh_streets_with_bridges(&city,false).unwrap().0
            .layers.remove(&SurfaceKind::Road).unwrap()
    }
    #[test]
    fn bent_street_area_is_partitioned_at_tile_boundary_without_loss() {
        let full=road(&[(260.,100.),(299.,100.),(320.,135.),(370.,135.)]);
        let west=clip_surface_to_bounds(&full,[0.,0.,300.,300.]).unwrap();
        let east=clip_surface_to_bounds(&full,[300.,0.,600.,300.]).unwrap();
        let full_area=area(&full);
        assert!((area(&west)+area(&east)-full_area).abs()<0.03,
            "finished road ribbon must have neither a gap nor duplicated area");
        for v in &west.vertices { assert!(v.position[0]<=300.0); }
        for v in &east.vertices { assert!(v.position[0]>=300.0); }
        assert!(!west.indices.is_empty() && !east.indices.is_empty());
    }
    #[test]
    fn surface_ownership_does_not_turn_bends_into_square_endcaps() {
        let full=road(&[(285.,150.),(305.,150.),(315.,170.)]);
        let west=clip_surface_to_bounds(&full,[0.,0.,300.,300.]).unwrap();
        let east=clip_surface_to_bounds(&full,[300.,0.,600.,300.]).unwrap();
        assert!((area(&west)+area(&east)-area(&full)).abs()<0.03);
        let touches=|mesh:&Mesh| mesh.vertices.iter()
            .filter(|v|(v.position[0]-300.0).abs()<1e-4).count();
        assert!(touches(&west)>0 && touches(&east)>0);
    }
    #[test]
    fn clipped_intersection_tessellation_shares_corners_between_triangles() {
        let mut quad=Mesh::default();
        for &(x,z) in &[(-10.,-10.),(10.,-10.),(10.,10.),(-10.,10.)] {
            quad.vertices.push(Vertex {position:[x,0.24,z],normal:[0.,1.,0.]});
        }
        quad.indices.extend([0,3,1,1,3,2]);
        let west=clip_surface_to_bounds(&quad,[-10.,-10.,0.,10.]).unwrap();
        let east=clip_surface_to_bounds(&quad,[0.,-10.,10.,10.]).unwrap();
        assert!((area(&west)-200.).abs()<0.001);
        assert!((area(&east)-200.).abs()<0.001);
        assert!(west.indices.len()>west.vertices.len(),
            "cut triangles must reuse shared GPU vertices");
        assert!(east.indices.len()>east.vertices.len(),
            "cut triangles must reuse shared GPU vertices");
        assert!(west.indices.iter().all(|&i|(i as usize)<west.vertices.len()));
        assert!(east.indices.iter().all(|&i|(i as usize)<east.vertices.len()));
    }
    #[test]
    fn all_four_tile_owners_reconstruct_crossing_surface_area() {
        let mesh=road(&[(-25.,-25.),(25.,25.)]);
        let tiles=[[-50.,-50.,0.,0.],[0.,-50.,50.,0.],
                   [-50.,0.,0.,50.],[0.,0.,50.,50.]];
        let sum:f64=tiles.into_iter()
            .map(|bounds|area(&clip_surface_to_bounds(&mesh,bounds).unwrap()))
            .sum();
        assert!((sum-area(&mesh)).abs()<0.005,
            "four cells must neither overlap nor erase road coverage");
    }
    #[test]
    fn rejects_invalid_triangles_instead_of_panicking() {
        let mut mesh=Mesh::default();
        mesh.indices.extend([0,1,2]);
        assert!(clip_surface_to_bounds(&mesh,[0.,0.,300.,300.]).is_err());
        assert!(clip_surface_to_bounds(&Mesh::default(),[0.,0.,0.,300.]).is_err());
    }
}
