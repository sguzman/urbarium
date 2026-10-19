//! Deterministic road ownership gate for every catalog region.
//!
//! This checks REAL imported streets, not just synthetic grid fixtures.
//! The output of independent camera chunks must partition the horizontal
//! surfaces of a complete source-region mesh. Report deviations by cell and
//! material before new geographic areas can ship. No terrain assumptions
//! are needed: this checks footprint ownership before DEM conformity.

use crate::geometry::Mesh;
use crate::road_grade::flatten_bridge_spans;
use crate::roads::{mesh_streets_with_bridges,mesh_streets_with_clearance,SurfaceKind,RoadScene};
use crate::streaming::{all_region_chunks,chunk_city,ChunkKey};
use crate::surface_clip::clip_surface_to_bounds;
use crate::{City,Building};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug,Serialize)]
pub struct CoverageReport {
    pub region_id:String,
    pub checked_cells:usize,
    pub checked_surfaces:usize,
    pub largest_absolute_error_m2:f64,
    pub largest_relative_error:f64,
    pub reference_area_m2:f64,
    pub chunk_area_m2:f64,
}

fn surface_area(mesh:&Mesh)->Result<f64,String> {
    if mesh.indices.len()%3!=0{return Err("nontriangular road surface index buffer".into());}
    let mut total=0.0;
    for indices in mesh.indices.chunks_exact(3){
        let index_position=|i:u32|mesh.vertices.get(i as usize)
            .map(|v|v.position)
            .ok_or_else(||"road surface index outside vertex buffer".to_owned());
        let a=index_position(indices[0])?;
        let b=index_position(indices[1])?;
        let c=index_position(indices[2])?;
        if a.iter().chain(b.iter()).chain(c.iter()).any(|v|!v.is_finite()){
            return Err("non-finite road geometry".into());
        }
        let cross=(b[0] as f64-a[0] as f64)*(c[2] as f64-a[2] as f64)
            -(b[2] as f64-a[2] as f64)*(c[0] as f64-a[0] as f64);
        total+=cross.abs()*0.5;
    }
    Ok(total)
}
fn scene(city:&City,footprints:&[Building])->Result<RoadScene,String> {
    let (mut roads,_)=mesh_streets_with_clearance(city,footprints,false)?;
    flatten_bridge_spans(&mut roads)?;
    Ok(roads)
}
fn cell_bounds(city:&City,key:ChunkKey)->Option<[f64;4]>{
    let sw=city.region.project(city.region.bbox[0],city.region.bbox[1]);
    let ne=city.region.project(city.region.bbox[2],city.region.bbox[3]);
    let tile=key.bounds();
    let b=[tile[0].max(sw.x),tile[1].max(sw.z),
           tile[2].min(ne.x),tile[3].min(ne.z)];
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn clipped_area(scene:&RoadScene,kind:SurfaceKind,bounds:[f64;4])->Result<f64,String>{
    let Some(mesh)=scene.layers.get(&kind) else{return Ok(0.0);};
    surface_area(&clip_surface_to_bounds(mesh,bounds)?)
}

/// Rebuild each independent chunk using the exact runtime candidate selector,
/// and compare its clipped road/walk area against the SAME cell clipped from
/// the full region scene. Every class is checked, including junction patches.
/// Fail on more than 0.2% area drift or 0.25 square meters per cell/material.
/// The absolute floor admits small f32 rounding differences, not missing
/// street blocks or disconnected sidewalk strips.
pub fn verify_road_coverage(city:&City)->Result<CoverageReport,String>{
    city.region.validate()?;
    let keys=all_region_chunks(&city.region);
    if keys.is_empty() || keys.len()>128{
        return Err(format!("region {} has unsupported road QA cell count {}",city.region.id,keys.len()));
    }
    let full=scene(city,&city.buildings)?;
    let mut checked=0usize;
    let mut total_full=0.0;
    let mut total_chunk=0.0;
    let mut max_abs:f64=0.0;
    let mut max_rel:f64=0.0;
    for key in keys.iter().copied(){
        let Some(bounds)=cell_bounds(city,key) else{continue;};
        let selected=chunk_city(city,key);
        let candidate=scene(&selected,&city.buildings)?;
        let kinds:BTreeSet<SurfaceKind>=full.layers.keys().chain(candidate.layers.keys())
            .copied().collect();
        for kind in kinds{
            let expected=clipped_area(&full,kind,bounds)?;
            let observed=clipped_area(&candidate,kind,bounds)?;
            let difference=(expected-observed).abs();
            if !difference.is_finite(){return Err("non-finite coverage comparison".into());}
            total_full+=expected;
            total_chunk+=observed;
            checked+=1;
            max_abs=max_abs.max(difference);
            max_rel=max_rel.max(difference/expected.max(0.25));
            let permitted=0.25+expected*0.002;
            if difference>permitted {
                return Err(format!(
                    "road ownership mismatch in {} cell ({},{}) {:?}: full {:.4} m2 vs chunk {:.4} m2 (difference {:.4} m2, limit {:.4} m2)",
                    city.region.id,key.x,key.z,kind,expected,observed,difference,permitted
                ));
            }
        }
    }
    Ok(CoverageReport{
        region_id:city.region.id.clone(),checked_cells:keys.len(),
        checked_surfaces:checked,largest_absolute_error_m2:max_abs,
        largest_relative_error:max_rel,
        reference_area_m2:total_full,chunk_area_m2:total_chunk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point,Region,Street};
    use std::collections::BTreeMap;
    fn street(id:&str,kind:&str,coords:&[(f64,f64)])->Street {
        Street {source_id:id.into(),name:None,kind:kind.into(),
            path:coords.iter().map(|&(x,z)|Point{x,z}).collect(),
            osm_node_ids:Vec::new(),tags:BTreeMap::new()}
    }
    #[test]
    fn inferred_road_clearance_uses_buildings_owned_by_neighbor_cell() {
        use crate::{Building,Polygon};
        let mut city=City{schema_version:1,
            region:Region{id:"shared".into(),name:"shared".into(),
                origin:[0.,0.],bbox:[-0.003,-0.003,0.003,0.003]},
            provenance:Vec::new(),buildings:vec![],
            streets:vec![street("service","service",&[(265.,0.),(325.,0.)])]};
        // Building center is in cell x=1, while part of the wall constrains
        // the service road in cell x=0. Partial chunk ownership may not make
        // the road jump from narrowed to unnarrowed at x=300.
        city.buildings.push(Building{
            source_id:"border-wall".into(),name:None,height_m:Some(12.),
            levels:None,polygons:vec![Polygon{rings:vec![vec![
                Point{x:296.,z:2.},Point{x:312.,z:2.},
                Point{x:312.,z:15.},Point{x:296.,z:15.}]]}],
        });
        let owner0=chunk_city(&city,ChunkKey{x:0,z:0});
        assert!(owner0.buildings.is_empty(),
            "the road's own render cell should not own the blocking building");
        let coverage=verify_road_coverage(&city).unwrap();
        assert!(coverage.largest_absolute_error_m2<0.01);
        let raw=mesh_streets_with_bridges(&city,false).unwrap().0;
        let narrowed=scene(&city,&city.buildings).unwrap();
        let area_raw=surface_area(&raw.layers[&SurfaceKind::Road]).unwrap();
        let area_narrow=surface_area(&narrowed.layers[&SurfaceKind::Road]).unwrap();
        assert!(area_narrow+0.1<area_raw,
            "full reference and every chunk must constrain road shoulders");
    }
    #[test]
    fn multipart_crossing_junctions_and_pedestrians_partition_across_cells(){
        let city=City{
            schema_version:1,
            region:Region{id:"fixture".into(),name:"fixture".into(),
                origin:[0.0,0.0],bbox:[-0.003,-0.003,0.003,0.003]},
            provenance:Vec::new(),buildings:Vec::new(),
            streets:vec![
                street("bend","primary",&[(-280.,50.),(-4.,50.),(20.,75.),(320.,75.)]),
                street("branch","residential",&[(-4.,50.),(-4.,-200.)]),
                street("join","residential",&[(-180.,50.),(-4.,50.)]),
                street("foot","footway",&[(-330.,-15.),(330.,80.)]),
                street("unconnected","secondary",&[(150.,-150.),(150.,150.)]),
                street("hidden-freeway","motorway",&[(-330.,250.),(330.,250.)]),
            ],
        };
        let report=verify_road_coverage(&city).unwrap();
        assert!(report.checked_cells>=9);
        assert!(report.checked_surfaces>20);
        assert!(report.reference_area_m2>1000.0);
        assert!((report.reference_area_m2-report.chunk_area_m2).abs()<0.5);
    }
}
