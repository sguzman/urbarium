//! Diagnostic intersections of at-grade source-derived asphalt and building roofs.
//!
//! A footprint can conflict with mapped carriageways because of source
//! registration, elevated arcades or genuinely bad geometry. We must identify
//! the source IDs and intersecting AREA before deciding what to alter.
//! Do not erase whole road triangles or remove complete building meshes.

use crate::{Building,City,Point,Street};
use crate::road_clearance::ClearanceIndex;
use crate::geometry::{mesh_building,HeightPolicy,Mesh};
use crate::roads::{at_grade_motor_road,ribbon,surface_class,SurfaceKind};
use serde::Serialize;

#[derive(Debug,Clone,Serialize)]
pub struct Conflict {
    pub building_id:String,
    pub street_id:String,
    pub road_kind:String,
    pub overlap_m2:f64,
}
#[derive(Debug,Serialize)]
pub struct ConflictReport {
    pub region_id:String,
    pub buildings_checked:usize,
    pub at_grade_streets_checked:usize,
    pub conflict_pairs:usize,
    pub sum_triangle_overlap_m2:f64,
    pub largest:Vec<Conflict>,
}
#[derive(Clone)]
struct Footprint {
    source_id:String,
    bounds:[f64;4],
    roof:Vec<[Point;3]>,
}
fn from_vertex(v:[f32;3])->Point {Point{x:v[0] as f64,z:v[2] as f64}}
fn bounds(points:&[Point])->[f64;4]{
    let mut a=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for p in points{
        a[0]=a[0].min(p.x);a[1]=a[1].min(p.z);
        a[2]=a[2].max(p.x);a[3]=a[3].max(p.z);
    }
    a
}
fn intersects(a:[f64;4],b:[f64;4])->bool {
    a[0]<b[2] && a[2]>b[0] && a[1]<b[3] && a[3]>b[1]
}
fn building_roof(building:&Building)->Result<Footprint,String>{
    let mesh=mesh_building(building,HeightPolicy::default())?;
    let mut roof=Vec::new();
    let mut extent=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for tri in mesh.indices.chunks_exact(3){
        let corners=[
            &mesh.vertices[tri[0] as usize],
            &mesh.vertices[tri[1] as usize],
            &mesh.vertices[tri[2] as usize],
        ];
        if corners.iter().any(|v|v.normal[1]<0.9){continue;}
        let points=corners.map(|v|from_vertex(v.position));
        let b=bounds(&points);
        for k in 0..2 {extent[k]=extent[k].min(b[k]);extent[k+2]=extent[k+2].max(b[k+2]);}
        roof.push(points);
    }
    if roof.is_empty(){return Err(format!("building {} has no roof",building.source_id));}
    Ok(Footprint{source_id:building.source_id.clone(),bounds:extent,roof})
}
fn signed(a:Point,b:Point,c:Point)->f64{
    (b.x-a.x)*(c.z-a.z)-(b.z-a.z)*(c.x-a.x)
}
fn polygon_area(poly:&[Point])->f64{
    if poly.len()<3 {return 0.0;}
    let mut sum=0.0;
    for i in 0..poly.len(){
        let a=poly[i];let b=poly[(i+1)%poly.len()];
        sum+=a.x*b.z-b.x*a.z;
    }
    sum.abs()*0.5
}
fn cut(poly:&[Point],a:Point,b:Point,sign:f64)->Vec<Point>{
    if poly.is_empty(){return Vec::new();}
    let mut out=Vec::with_capacity(poly.len()+1);
    let mut prev=*poly.last().unwrap();
    let mut old=sign*signed(a,b,prev);
    for &current in poly{
        let new=sign*signed(a,b,current);
        let was_inside=old>=-1e-10;
        let is_inside=new>=-1e-10;
        if was_inside!=is_inside{
            let denom=old-new;
            if denom.abs()>1e-16 {
                let t=(old/denom).clamp(0.0,1.0);
                out.push(Point{x:prev.x+(current.x-prev.x)*t,
                               z:prev.z+(current.z-prev.z)*t});
            }
        }
        if is_inside {out.push(current);}
        prev=current;old=new;
    }
    out
}
fn intersection_area(a:[Point;3],b:[Point;3])->f64{
    if !intersects(bounds(&a),bounds(&b)){return 0.0;}
    let orientation=if signed(b[0],b[1],b[2])>=0.0 {1.0}else{-1.0};
    let mut poly=a.to_vec();
    for i in 0..3{
        poly=cut(&poly,b[i],b[(i+1)%3],orientation);
        if poly.len()<3{return 0.0;}
    }
    polygon_area(&poly)
}
fn conflict_area(road:&Mesh,roof:&Footprint)->f64{
    let mut area=0.0;
    for indices in road.indices.chunks_exact(3){
        let points=[
            from_vertex(road.vertices[indices[0] as usize].position),
            from_vertex(road.vertices[indices[1] as usize].position),
            from_vertex(road.vertices[indices[2] as usize].position),
        ];
        if !intersects(bounds(&points),roof.bounds){continue;}
        for &roof_tri in &roof.roof{
            area+=intersection_area(points,roof_tri);
        }
    }
    area
}
fn candidates(street:&Street,width:f64,roofs:&[Footprint])->Vec<usize>{
    if street.path.len()<2{return Vec::new();}
    let mut road_bounds=bounds(&street.path);
    // Offset ribbons can reach width/2 * 2 at capped miter corners.
    for i in 0..2{road_bounds[i]-=width;road_bounds[i+2]+=width;}
    roofs.iter().enumerate().filter_map(|(i,r)|
        intersects(road_bounds,r.bounds).then_some(i)).collect()
}

/// At-grade motor roads only; elevated/underground roads need separate
/// floor-aware topology. A 0.25m2 report threshold avoids point-touch noise.
/// This diagnostic does NOT claim that the road or footprint should be cut.
pub fn audit_road_buildings(city:&City)->Result<ConflictReport,String>{
    let roofs:Vec<Footprint>=city.buildings.iter()
        .map(building_roof).collect::<Result<_,_>>()?;
    let clearance=ClearanceIndex::new(&city.buildings);
    let mut checked=0usize;
    let mut conflicts=Vec::new();
    let mut total=0.0;
    for street in &city.streets{
        if !at_grade_motor_road(street){continue;}
        let Some((kind,width))=surface_class(street) else{continue;};
        if kind!=SurfaceKind::Road {continue;} // freeways hidden in default view
        checked+=1;
        let ids=candidates(street,width,&roofs);
        if ids.is_empty(){continue;}
        let Some(mut mesh)=ribbon(&street.path,width,0.24)? else{continue;};
        clearance.constrain(street,width,&mut mesh);
        for i in ids{
            let area=conflict_area(&mesh,&roofs[i]);
            if area<0.25{continue;}
            if !area.is_finite(){return Err("invalid road/building overlap area".into());}
            total+=area;
            conflicts.push(Conflict{
                building_id:roofs[i].source_id.clone(),
                street_id:street.source_id.clone(),
                road_kind:street.kind.clone(),
                overlap_m2:(area*1000.0).round()/1000.0,
            });
        }
    }
    conflicts.sort_by(|a,b|b.overlap_m2.total_cmp(&a.overlap_m2)
        .then(a.building_id.cmp(&b.building_id))
        .then(a.street_id.cmp(&b.street_id)));
    Ok(ConflictReport{
        region_id:city.region.id.clone(),
        buildings_checked:roofs.len(),at_grade_streets_checked:checked,
        conflict_pairs:conflicts.len(),
        sum_triangle_overlap_m2:(total*1000.0).round()/1000.0,
        largest:conflicts.into_iter().take(20).collect(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Polygon,Region};
    use std::collections::BTreeMap;
    fn city(street:Street,building:Building)->City{
        City{schema_version:1,
            region:Region{id:"test".into(),name:"test".into(),
                origin:[0.,0.],bbox:[-0.01,-0.01,0.01,0.01]},
            provenance:Vec::new(),streets:vec![street],buildings:vec![building]}
    }
    fn street(a:Point,b:Point,width:f64)->Street{
        let mut tags=BTreeMap::new();
        tags.insert("width".into(),format!("{width}m"));
        Street{source_id:"road".into(),name:None,kind:"residential".into(),
            path:vec![a,b],tags,osm_node_ids:vec![]}
    }
    fn p(x:f64,z:f64)->Point{Point{x,z}}
    fn building(hole:bool)->Building {
        let mut rings=vec![vec![p(0.,0.),p(10.,0.),p(10.,10.),p(0.,10.)]];
        if hole{rings.push(vec![p(3.,3.),p(7.,3.),p(7.,7.),p(3.,7.)]);}
        Building{source_id:"building".into(),name:None,
            height_m:Some(10.0),levels:None,polygons:vec![Polygon{rings}]}
    }
    #[test]
    fn crossing_carriageway_reports_real_area_not_point_contact(){
        let c=city(street(p(-5.,5.),p(15.,5.),2.0),building(false));
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.conflict_pairs,1);
        assert!((report.sum_triangle_overlap_m2-20.0).abs()<0.02);
    }
    #[test]
    fn distinct_path_outside_building_reports_zero(){
        let c=city(street(p(-5.,15.),p(15.,15.),2.0),building(false));
        assert_eq!(audit_road_buildings(&c).unwrap().conflict_pairs,0);
    }
    #[test]
    fn courtyard_hole_never_becomes_fake_collision(){
        let c=city(street(p(3.5,5.),p(6.5,5.),1.0),building(true));
        assert_eq!(audit_road_buildings(&c).unwrap().conflict_pairs,0);
    }
    #[test]
    fn tunnel_is_not_treated_as_surface_road(){
        let mut c=city(street(p(-5.,5.),p(15.,5.),2.0),building(false));
        c.streets[0].tags.insert("tunnel".into(),"yes".into());
        assert_eq!(audit_road_buildings(&c).unwrap().at_grade_streets_checked,0);
    }
}
