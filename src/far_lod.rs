//! Lightweight source-derived massing for unloaded detailed mesh cells.
//! A distant building stays visible as a low-vertex box at its real footprint
//! extent and source/estimated height. The original geometry is never changed.
//! This is not a billboard texture or a claim about detailed architecture.
use crate::{Building,City,Point};
use crate::chunk_mesh::{DrawBatch,GpuVertex,SceneLayer};
use crate::geometry::HeightPolicy;
use crate::streaming::ChunkKey;
use crate::terrain::TerrainGrid;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct FarMassing {
    pub vertices:Vec<GpuVertex>,
    pub indices:Vec<u32>,
    /// One draw range per geographic cell; the viewer skips ranges when
    /// that cell's detailed mesh is resident.
    pub cells:Vec<(ChunkKey,DrawBatch)>,
}

fn footprint_extent(building:&Building)->Option<(ChunkKey,[f32;4])>{
    // Match the resident chunk owner's first exterior-ring vertex centroid.
    let first=building.polygons.first()?.rings.first()?;
    if first.is_empty() {return None;}
    let center=Point{
        x:first.iter().map(|p|p.x).sum::<f64>()/first.len() as f64,
        z:first.iter().map(|p|p.z).sum::<f64>()/first.len() as f64,
    };
    let (mut x0,mut z0,mut x1,mut z1)=
        (f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY);
    for point in building.polygons.iter().flat_map(|poly|
        poly.rings.first().into_iter().flat_map(|ring|ring.iter())) {
        x0=x0.min(point.x);z0=z0.min(point.z);
        x1=x1.max(point.x);z1=z1.max(point.z);
    }
    if ![x0,z0,x1,z1].iter().all(|v|v.is_finite())
        || x1-x0<0.2 || z1-z0<0.2
        || !center.x.is_finite() || !center.z.is_finite() {
        return None;
    }
    Some((ChunkKey::at(center),[x0 as f32,z0 as f32,x1 as f32,z1 as f32]))
}

fn add_box(mesh:&mut FarMassing,ext:[f32;4],base:f32,roof:f32)
    ->Result<(),String> {
    let [x0,z0,x1,z1]=ext;
    let corners=[
        [x0,base,z0],[x1,base,z0],[x1,base,z1],[x0,base,z1],
        [x0,roof,z0],[x1,roof,z0],[x1,roof,z1],[x0,roof,z1],
    ];
    let faces=[
        ([0,3,2,1],[0.,-1.,0.]),
        ([4,5,6,7],[0.,1.,0.]),
        ([0,1,5,4],[0.,0.,-1.]),
        ([1,2,6,5],[1.,0.,0.]),
        ([2,3,7,6],[0.,0.,1.]),
        ([3,0,4,7],[-1.,0.,0.]),
    ];
    let base_index=u32::try_from(mesh.vertices.len())
        .map_err(|_|"distant skyline vertex count exceeded u32".to_string())?;
    if base_index>u32::MAX-24 {
        return Err("distant skyline vertex count exceeded u32".into());
    }
    for (face,(indices,normal)) in faces.iter().enumerate() {
        let start=base_index+(face as u32)*4;
        for &corner in indices {
            mesh.vertices.push(GpuVertex{
                position:corners[corner],normal:*normal,
                color:[0.65,0.71,0.76],
                category_color:[0.;3],category_id:0.,
            });
        }
        mesh.indices.extend([start,start+1,start+2,start,start+2,start+3]);
    }
    Ok(())
}

/// Compact stand-ins for all source buildings, grouped by source chunk.
/// GPU memory scales with the number of buildings, NOT detailed polygon edges
/// or full per-cell road meshes. Terrain retains its own full region mesh.
pub fn build_far_massing(city:&City,terrain:&TerrainGrid)
    ->Result<FarMassing,String>{
    let policy=HeightPolicy::default();
    let mut cells:BTreeMap<ChunkKey,Vec<([f32;4],f32,f32)>>=BTreeMap::new();
    for building in &city.buildings {
        let Some((key,ext))=footprint_extent(building) else {continue;};
        let height=building.height_m.or_else(||building.levels
            .map(|v|v*policy.meters_per_level)).unwrap_or(policy.fallback_m);
        let raised=policy.base_for(building);
        if !height.is_finite() || height<=0.0 || height>700.0 ||
           !raised.is_finite() || raised<0.0 || raised>500.0 {continue;}
        let mx=(ext[0]+ext[2])*0.5;
        let mz=(ext[1]+ext[3])*0.5;
        let ground=terrain.sample_surface(&city.region,mx as f64,mz as f64);
        let base=ground+raised as f32-0.2;
        let roof=base+height as f32;
        if base.is_finite() && roof.is_finite() {
            cells.entry(key).or_default().push((ext,base,roof));
        }
    }
    let mut out=FarMassing::default();
    for (key,boxes) in cells {
        let first=u32::try_from(out.indices.len())
            .map_err(|_|"distant skyline index count exceeded u32")?;
        for (ext,base,roof) in boxes {add_box(&mut out,ext,base,roof)?;}
        let end=u32::try_from(out.indices.len())
            .map_err(|_|"distant skyline index count exceeded u32")?;
        if end>first {
            out.cells.push((key,DrawBatch{
                layer:SceneLayer::Buildings,first_index:first,count:end-first
            }));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Polygon,Region};
    #[test]
    fn footprint_owner_keeps_real_extent_and_height(){
        let b=Building{
            source_id:"tower".into(),name:None,height_m:Some(40.),
            levels:None,min_height_m:None,min_floor:None,parts:vec![],
            polygons:vec![Polygon{rings:vec![vec![
                Point{x:-4.,z:-3.},Point{x:6.,z:-3.},
                Point{x:6.,z:8.},Point{x:-4.,z:8.}
            ]]}],
        };
        let (key,ext)=footprint_extent(&b).unwrap();
        assert_eq!(key,ChunkKey::at(Point{x:1.,z:2.5}));
        assert_eq!(ext,[-4.,-3.,6.,8.]);
        let mut far=FarMassing::default();
        add_box(&mut far,ext,12.,52.).unwrap();
        assert_eq!(far.vertices.len(),24);
        assert_eq!(far.indices.len(),36);
        assert!(far.vertices.iter().any(|v|v.position[1]==52.));
        assert!(far.indices.iter().all(|&i|(i as usize)<far.vertices.len()));
    }
    #[test]
    fn outline_holes_do_not_create_extra_fake_skyscrapers(){
        let polygon=Polygon{rings:vec![
            vec![Point{x:0.,z:0.},Point{x:12.,z:0.},
                 Point{x:12.,z:10.},Point{x:0.,z:10.}],
            vec![Point{x:2.,z:2.},Point{x:5.,z:2.},
                 Point{x:5.,z:5.},Point{x:2.,z:5.}]
        ]};
        let building=Building{source_id:"a".into(),name:None,height_m:Some(20.),
            levels:None,min_height_m:None,min_floor:None,
            polygons:vec![polygon],parts:vec![]};
        assert_eq!(footprint_extent(&building).unwrap().1,[0.,0.,12.,10.]);
    }
}
