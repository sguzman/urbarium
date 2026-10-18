//! Terrain-conforming surface tessellation.
//!
//! Sampling just the original ribbon vertices is insufficient: a road triangle
//! can cross a ridge formed by the triangulated DEM and slice through ground
//! between its sampled vertices. Cut only those road triangles that span DEM
//! triangle boundaries, then place each resulting polygon on its own *planar*
//! ground triangle plus the original material clearance. This gives a geometric
//! (rather than empirical height-bias) no-penetration guarantee.

use crate::geometry::{Mesh,Vertex};
use crate::terrain::TerrainGrid;
use crate::Region;
use std::collections::HashMap;

#[derive(Clone,Copy)]
struct GroundVertex { x:f32,z:f32,h:f32 }

fn cross(a:GroundVertex,b:GroundVertex,x:f32,z:f32)->f64 {
    (b.x as f64-a.x as f64)*(z as f64-a.z as f64)
        -(b.z as f64-a.z as f64)*(x as f64-a.x as f64)
}
fn area(a:Vertex,b:Vertex,c:Vertex)->f64 {
    (((b.position[0]-a.position[0]) as f64)
        *((c.position[2]-a.position[2]) as f64)
       -((b.position[2]-a.position[2]) as f64)
        *((c.position[0]-a.position[0]) as f64)).abs()*0.5
}
fn edge_clip(poly:&[Vertex],a:GroundVertex,b:GroundVertex)->Vec<Vertex> {
    if poly.is_empty(){return Vec::new();}
    let mut result=Vec::with_capacity(poly.len()+1);
    // DEM triangles follow the same clockwise XZ winding as the ground mesh.
    let mut prev=*poly.last().unwrap();
    let mut prev_side=cross(a,b,prev.position[0],prev.position[2]);
    for &current in poly {
        let side=cross(a,b,current.position[0],current.position[2]);
        let was_inside=prev_side<=0.0;
        let is_inside=side<=0.0;
        if was_inside!=is_inside {
            let denom=prev_side-side;
            let t=if denom.abs()<1e-16 {0.0}
                else{(prev_side/denom).clamp(0.0,1.0) as f32};
            let mut cut=prev;
            for component in 0..3 {
                cut.position[component]=prev.position[component]+
                    (current.position[component]-prev.position[component])*t;
                cut.normal[component]=prev.normal[component]+
                    (current.normal[component]-prev.normal[component])*t;
            }
            result.push(cut);
        }
        if is_inside {result.push(current);}
        prev=current;
        prev_side=side;
    }
    result
}
fn intersect_face(triangle:[Vertex;3],ground:[GroundVertex;3])->Vec<Vertex> {
    let mut result=triangle.to_vec();
    for i in 0..3 {
        result=edge_clip(&result,ground[i],ground[(i+1)%3]);
        if result.len()<3 {break;}
    }
    result
}
fn height_on_face(p:Vertex,ground:[GroundVertex;3])->f32 {
    let d=cross(ground[0],ground[1],ground[2].x,ground[2].z);
    let w1=cross(ground[0],ground[2],p.position[0],p.position[2])/-d;
    let w2=cross(ground[0],ground[1],p.position[0],p.position[2])/d;
    (ground[0].h as f64 + (ground[1].h-ground[0].h) as f64*w1
        + (ground[2].h-ground[0].h) as f64*w2) as f32
}
fn face_of(grid:&TerrainGrid, region:&Region, x:f32,z:f32)
    ->(usize,usize,bool) {
    let a=region.project(region.bbox[0],region.bbox[1]);
    let b=region.project(region.bbox[2],region.bbox[3]);
    let fx=((x as f64-a.x)/(b.x-a.x)*(grid.width-1) as f64)
        .clamp(0.0,(grid.width-1) as f64);
    let fz=((z as f64-a.z)/(b.z-a.z)*(grid.height-1) as f64)
        .clamp(0.0,(grid.height-1) as f64);
    let col=(fx.floor() as usize).min(grid.width-2);
    let row=(fz.floor() as usize).min(grid.height-2);
    (col,row,(fx-col as f64)+(fz-row as f64)<=1.0)
}
// Tessellating each source triangle against DEM face boundaries creates the
// same edge intersections repeatedly. The transport pipeline sends 52 bytes
// per GPU vertex, so duplicating the three vertices of every small clipped
// triangle caused a large memory/transfer regression. Indexed vertex welding
// restores sharing across original and cut triangles. Millimeter identity is
// below the precision of the f32 world coordinates; the y coordinate and
// normal remain part of the identity so overlapping distinct levels do not
// get accidentally welded together.
type WeldKey=[i64;6];
fn weld_key(vertex:Vertex)->Result<WeldKey,String> {
    if !vertex.position.iter().chain(vertex.normal.iter())
        .all(|v|v.is_finite()) {
        return Err("non-finite terrain surface vertex".into());
    }
    let mut key=[0_i64;6];
    for i in 0..3 {
        key[i]=(vertex.position[i] as f64*1_000.0).round() as i64;
        key[i+3]=(vertex.normal[i] as f64*100_000.0).round() as i64;
    }
    Ok(key)
}
fn intern_vertex(out:&mut Mesh,cache:&mut HashMap<WeldKey,u32>,v:Vertex)
    ->Result<u32,String> {
    let key=weld_key(v)?;
    if let Some(&index)=cache.get(&key){return Ok(index);}
    let index=u32::try_from(out.vertices.len())
        .map_err(|_|"terrain-conforming surface too large")?;
    out.vertices.push(v);
    cache.insert(key,index);
    Ok(index)
}
fn push_triangle(out:&mut Mesh,cache:&mut HashMap<WeldKey,u32>,
    triangle:[Vertex;3])->Result<(),String> {
    if area(triangle[0],triangle[1],triangle[2])<1e-7{return Ok(());}
    let ids=[
        intern_vertex(out,cache,triangle[0])?,
        intern_vertex(out,cache,triangle[1])?,
        intern_vertex(out,cache,triangle[2])?,
    ];
    // Tiny slivers that collapse during sub-mm welding are not triangles.
    if ids[0]!=ids[1] && ids[1]!=ids[2] && ids[0]!=ids[2] {
        out.indices.extend(ids);
    }
    Ok(())
}

/// Drape a street/walk/rail mesh to its *triangulated* DEM, not a bilinear
/// approximation. Fast path keeps shared original vertices when an entire
/// triangle already lies on one DEM face. The exceptional crossing triangle
/// is intersected against only the grid cells touched by its bounding box.
pub fn conform_to_ground(grid:&TerrainGrid,region:&Region,mesh:&Mesh)
    ->Result<Mesh,String> {
    grid.validate(region)?;
    if mesh.indices.len()%3!=0 {return Err("bad surface triangle index count".into());}
    let sw=region.project(region.bbox[0],region.bbox[1]);
    let ne=region.project(region.bbox[2],region.bbox[3]);
    let x_at=|col:usize| (sw.x+(ne.x-sw.x)*col as f64/(grid.width-1) as f64) as f32;
    let z_at=|row:usize| (sw.z+(ne.z-sw.z)*row as f64/(grid.height-1) as f64) as f32;
    let height=|col:usize,row:usize|grid.heights_m[row*grid.width+col];
    let ground=|col:usize,row:usize| {
        let (x0,x1,z0,z1)=(x_at(col),x_at(col+1),z_at(row),z_at(row+1));
        let sw=GroundVertex{x:x0,z:z0,h:height(col,row)};
        let se=GroundVertex{x:x1,z:z0,h:height(col+1,row)};
        let nw=GroundVertex{x:x0,z:z1,h:height(col,row+1)};
        let ne=GroundVertex{x:x1,z:z1,h:height(col+1,row+1)};
        [[sw,nw,se],[ne,se,nw]]
    };
    let mut output=Mesh::default();
    let mut welded=HashMap::<WeldKey,u32>::new();
    let mut reused:Vec<Option<u32>>=vec![None;mesh.vertices.len()];
    for indices in mesh.indices.chunks_exact(3) {
        let idx=[indices[0] as usize,indices[1] as usize,indices[2] as usize];
        if idx.iter().any(|&i|i>=mesh.vertices.len()){
            return Err("invalid terrain surface triangle index".into());
        }
        let tri=[mesh.vertices[idx[0]],mesh.vertices[idx[1]],mesh.vertices[idx[2]]];
        let faces=tri.map(|v|face_of(grid,region,v.position[0],v.position[2]));
        if faces[0]==faces[1] && faces[1]==faces[2] {
            // A triangle inside one ground face is already planar with that
            // ground face. Retain its shared original indexing.
            for &index in indices {
                let slot=&mut reused[index as usize];
                let mapped=match *slot {
                    Some(mapped)=>mapped,
                    None=>{
                        let mut vertex=mesh.vertices[index as usize];
                        let face=ground(faces[0].0,faces[0].1)[usize::from(!faces[0].2)];
                        vertex.position[1]+=height_on_face(vertex,face);
                        let mapped=intern_vertex(&mut output,&mut welded,vertex)?;
                        *slot=Some(mapped);
                        mapped
                    }
                };
                output.indices.push(mapped);
            }
            continue;
        }
        let min_x=tri.iter().map(|v|v.position[0]).fold(f32::INFINITY,f32::min);
        let max_x=tri.iter().map(|v|v.position[0]).fold(f32::NEG_INFINITY,f32::max);
        let min_z=tri.iter().map(|v|v.position[2]).fold(f32::INFINITY,f32::min);
        let max_z=tri.iter().map(|v|v.position[2]).fold(f32::NEG_INFINITY,f32::max);
        let (start_x,start_z,_)=face_of(grid,region,min_x,min_z);
        let (end_x,end_z,_)=face_of(grid,region,max_x,max_z);
        for row in start_z..=end_z {
            for col in start_x..=end_x {
                for face in ground(col,row) {
                    let polygon=intersect_face(tri,face);
                    if polygon.len()<3 {continue;}
                    for i in 1..polygon.len()-1 {
                        let mut section=[polygon[0],polygon[i],polygon[i+1]];
                        if area(section[0],section[1],section[2])<1e-7{continue;}
                        for vertex in &mut section {
                            vertex.position[1]+=height_on_face(*vertex,face);
                        }
                        push_triangle(&mut output,&mut welded,section)?;
                    }
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roads::{mesh_streets_with_bridges, SurfaceKind};
    use crate::{City,Street};
    use std::collections::BTreeMap;
    #[test]
    fn overlapping_dem_splits_share_gpu_vertices_without_losing_triangles() {
        let r=Region{id:"flat".into(),name:"flat".into(),
            origin:[0.0005,0.0005],bbox:[0.,0.,0.001,0.001]};
        let grid=TerrainGrid{schema_version:1,region_id:"flat".into(),
            bbox:r.bbox,width:3,height:3,heights_m:vec![0.;9],
            origin_elevation_m:0.,source:"test".into()};
        let corners=[(-45.0,-45.0),(45.0,-45.0),(45.0,45.0),(-45.0,45.0)];
        let mut mesh=Mesh::default();
        for (x,z) in corners {
            mesh.vertices.push(Vertex{position:[x,0.24,z],normal:[0.,1.,0.]});
        }
        mesh.indices.extend([0,3,1,1,3,2]);
        let out=conform_to_ground(&grid,&r,&mesh).unwrap();
        assert!(out.indices.len()>=6);
        assert!(out.vertices.len()*2<out.indices.len(),
            "terrain intersections must share GPU vertices, not copy every corner");
        assert!(out.indices.iter().all(|i|(*i as usize)<out.vertices.len()));
    }
    #[test]
    fn surface_triangles_follow_dem_faces_without_piercing_ground() {
        let region=Region{id:"slope".into(),name:"slope".into(),
            origin:[0.0005,0.0005],bbox:[0.,0.,0.001,0.001]};
        let grid=TerrainGrid{schema_version:1,region_id:"slope".into(),
            bbox:region.bbox,width:2,height:2,
            heights_m:vec![0.,0.,0.,60.],
            origin_elevation_m:0.,source:"synthetic".into()};
        let city=City{schema_version:1,region:region.clone(),
            streets:vec![Street{source_id:"diagonal".into(),name:None,
                kind:"primary".into(),
                path:vec![region.project(0.0002,0.0002),
                          region.project(0.0008,0.0008)],
                tags:BTreeMap::new(),osm_node_ids:vec![]}],
            buildings:vec![],provenance:vec![]};
        let (roads,_)=mesh_streets_with_bridges(&city,false).unwrap();
        let orig=&roads.layers[&SurfaceKind::Road];
        let draped=conform_to_ground(&grid,&region,orig).unwrap();
        assert!(draped.indices.len()>=orig.indices.len());
        for tri in draped.indices.chunks_exact(3) {
            let verts=[draped.vertices[tri[0] as usize],
                       draped.vertices[tri[1] as usize],
                       draped.vertices[tri[2] as usize]];
            for weights in [[1./3.,1./3.,1./3.],
                            [0.5,0.5,0.],[0.,0.5,0.5],[0.5,0.,0.5]] {
                let x=verts.iter().zip(weights).map(|(v,t)|v.position[0] as f64*t).sum();
                let z=verts.iter().zip(weights).map(|(v,t)|v.position[2] as f64*t).sum();
                let y=verts.iter().zip(weights).map(|(v,t)|v.position[1] as f64*t).sum::<f64>() as f32;
                let ground=grid.sample_surface(&region,x,z);
                assert!(y-ground>0.10, "road pierced DEM at ({x},{z}): {y} vs {ground}");
            }
        }
    }
}
