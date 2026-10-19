//! Diagnostic intersections of at-grade source-derived asphalt and building roofs.
//!
//! A footprint can conflict with mapped carriageways because of source
//! registration, elevated arcades or genuinely bad geometry. We must identify
//! the source IDs and intersecting AREA before deciding what to alter.
//! Do not erase whole road triangles or remove complete building meshes.

use crate::{Building,BuildingPart,City,Point,Street};
use crate::road_clearance::ClearanceIndex;
use crate::geometry::{mesh_building,HeightPolicy,Mesh};
use crate::roads::{at_grade_motor_road,ribbon,surface_class,SurfaceKind};
use serde::Serialize;
use std::collections::BTreeMap;

/// A footprint overlap alone is not grounds to delete road geometry.
#[derive(Debug,Clone,Copy,Serialize,PartialEq,Eq,PartialOrd,Ord)]
#[serde(rename_all="snake_case")]
pub enum ConflictCause {
    /// The imported source centerline itself passes through a solid roof.
    /// Requires source/grade investigation; reducing the road width cannot fix it.
    CenterlineInsideFootprint,
    /// The source provides physical width/lanes, so automated narrowing
    /// would override measured or explicitly tagged information.
    TaggedWidthEnvelope,
    /// Centerline is outside the building; an estimated class width or bend
    /// miter intrudes into the building wall.
    InferredWidthEnvelope,
}
/// A source building part intersecting the road footprint. Part volumes are
/// not yet substituted for the parent geometry; evidence must be reviewed
/// before treating apparent underpasses as open spaces.
#[derive(Debug,Clone,Serialize)]
pub struct PartEvidence {
    pub part_id:String,
    pub overlap_m2:f64,
    pub centerline_inside_m:f64,
    pub measured_min_height_m:Option<f64>,
    pub min_floor:Option<u32>,
    /// True only with explicit metric clearance, never inferred floor counts.
    pub verified_above_ground:bool,
}
#[derive(Debug,Clone,Serialize)]
pub struct Conflict {
    pub building_id:String,
    pub street_id:String,
    pub road_kind:String,
    pub overlap_m2:f64,
    pub cause:ConflictCause,
    pub centerline_inside_m:f64,
    pub centerline_length_m:f64,
    /// WGS84 longitude, latitude of the affected source-line position;
    /// never a made-up building address or manually inferred grade.
    pub focus_lonlat:[f64;2],
    #[serde(skip_serializing_if="Vec::is_empty")]
    pub source_part_evidence:Vec<PartEvidence>,
}
#[derive(Debug,Serialize)]
pub struct ConflictReport {
    pub region_id:String,
    pub buildings_checked:usize,
    /// Source-qualified raised volumes cannot collide with surface asphalt.
    pub raised_buildings_excluded:usize,
    pub at_grade_streets_checked:usize,
    pub conflict_pairs:usize,
    pub sum_triangle_overlap_m2:f64,
    pub by_cause:BTreeMap<ConflictCause,usize>,
    pub area_m2_by_cause:BTreeMap<ConflictCause,f64>,
    pub total_centerline_inside_m:f64,
    pub parent_buildings_with_parts:usize,
    pub total_parts_available:usize,
    pub conflict_pairs_with_part_evidence:usize,
    pub conflict_pairs_with_measured_raised_part_evidence:usize,
    pub largest:Vec<Conflict>,
}
#[derive(Clone)]
struct Footprint {
    source_id:String,
    bounds:[f64;4],
    roof:Vec<[Point;3]>,
    /// Original solid outer boundaries and courtyard holes are needed to
    /// exclude a centerline that merely runs along a building wall.
    polygons:Vec<Vec<Vec<Point>>>,
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
    Ok(Footprint{source_id:building.source_id.clone(),bounds:extent,roof,
        polygons:building.polygons.iter().map(|p|p.rings.clone()).collect()})
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
/// Parametric interval of one source segment inside a single upward-facing
/// building roof triangle. Projected triangles are the exact same mesh that
/// the viewer draws, including courtyard triangulation.
fn segment_roof_interval(a:Point,b:Point,triangle:[Point;3])->Option<(f64,f64)> {
    let cross=signed(triangle[0],triangle[1],triangle[2]);
    if cross.abs()<1e-10{return None;}
    let direction=if cross>=0.0 {1.0}else{-1.0};
    let mut enter:f64=0.0;
    let mut leave:f64=1.0;
    for i in 0..3 {
        let left=triangle[i];
        let right=triangle[(i+1)%3];
        let at_start=direction*signed(left,right,a);
        let at_end=direction*signed(left,right,b);
        let delta=at_end-at_start;
        if delta.abs()<1e-11 {
            if at_start<0.0{return None;}
            continue;
        }
        let edge_t=-at_start/delta;
        if delta>0.0 {enter=enter.max(edge_t);}
        else {leave=leave.min(edge_t);}
        if leave<=enter{return None;}
    }
    (leave>enter && enter<1.0 && leave>0.0)
        .then_some((enter.max(0.0),leave.min(1.0)))
}
/// Deduplicate overlapping intervals, including a source line aligned with
/// the diagonal shared by TWO roof triangles. Otherwise that alignment can
/// double-count both source penetration and conflict severity.
fn union_intervals(mut intervals:Vec<(f64,f64)>)->Vec<(f64,f64)> {
    intervals.sort_by(|a,b|a.0.total_cmp(&b.0));
    let mut merged:Vec<(f64,f64)>=Vec::new();
    for (lo,hi) in intervals {
        if let Some(last)=merged.last_mut(){
            if lo<=last.1+1e-10 {last.1=last.1.max(hi);continue;}
        }
        merged.push((lo,hi));
    }
    merged
}
/// Source polygons provide a strict interior test: tracing along a
/// building's OUTER perimeter is not the same as crossing its solid interior.
/// Triangulation diagonals are not outer perimeters and remain valid interior.
fn on_ring_edge(p:Point,a:Point,b:Point)->bool{
    let dx=b.x-a.x;let dz=b.z-a.z;
    let length=dx.hypot(dz);
    if length<=1e-12{return (p.x-a.x).hypot(p.z-a.z)<1e-7;}
    let distance=signed(a,b,p).abs()/length;
    let projection=(p.x-a.x)*dx+(p.z-a.z)*dz;
    distance<1e-7 && projection>=-1e-7 && projection<=length*length+1e-7
}
fn in_ring(p:Point,ring:&[Point])->Option<bool>{
    if ring.len()<3{return Some(false);}
    let mut inside=false;
    let mut previous=*ring.last().unwrap();
    for &current in ring {
        if on_ring_edge(p,previous,current){return None;}
        if (current.z>p.z)!=(previous.z>p.z){
            let crossing=current.x+(p.z-current.z)*
                (previous.x-current.x)/(previous.z-current.z);
            if p.x<crossing{inside=!inside;}
        }
        previous=current;
    }
    Some(inside)
}
fn in_solid_interior(p:Point,roof:&Footprint)->bool{
    roof.polygons.iter().any(|rings|{
        let Some(outer)=rings.first() else{return false;};
        if in_ring(p,outer)!=Some(true){return false;}
        rings.iter().skip(1).all(|ring|in_ring(p,ring)==Some(false))
    })
}
/// Return total length of the *source centerline* passing through a solid
/// roof and the midpoint of its longest contained section for map QA.
fn centerline_roof(street:&Street,roof:&Footprint)->(f64,Option<Point>){
    let mut total=0.0;
    let mut longest=0.0;
    let mut focus=None;
    for segment in street.path.windows(2) {
        let a=segment[0];let b=segment[1];
        let length=(a.x-b.x).hypot(a.z-b.z);
        if !length.is_finite()||length<=1e-9 ||
            !intersects(bounds(&[a,b]),roof.bounds){continue;}
        let intervals=roof.roof.iter()
            .filter_map(|&triangle|segment_roof_interval(a,b,triangle))
            .collect::<Vec<_>>();
        for (lo,hi) in union_intervals(intervals){
            let t=(lo+hi)*0.5;
            let midpoint=Point{x:a.x+(b.x-a.x)*t,z:a.z+(b.z-a.z)*t};
            // A source centerline running ON an exterior wall is a
            // width-envelope issue, not solid building penetration.
            if !in_solid_interior(midpoint,roof){continue;}
            let covered=(hi-lo)*length;
            total+=covered;
            if covered>longest {
                longest=covered;
                focus=Some(midpoint);
            }
        }
    }
    (total,focus)
}
fn point_near_footprint(street:&Street,roof:&Footprint)->Point {
    let target=Point{x:(roof.bounds[0]+roof.bounds[2])*0.5,
                     z:(roof.bounds[1]+roof.bounds[3])*0.5};
    let mut best=(f64::INFINITY,street.path[0]);
    for segment in street.path.windows(2){
        let a=segment[0];let b=segment[1];
        let dx=b.x-a.x;let dz=b.z-a.z;
        let len2=dx*dx+dz*dz;
        let t=if len2>1e-12 {((target.x-a.x)*dx+(target.z-a.z)*dz)/len2}
            else {0.0};
        let t=t.clamp(0.0,1.0);
        let p=Point{x:a.x+t*dx,z:a.z+t*dz};
        let dist2=(p.x-target.x).powi(2)+(p.z-target.z).powi(2);
        if dist2<best.0 {best=(dist2,p);}
    }
    best.1
}
fn source_line_length(street:&Street)->f64 {
    street.path.windows(2)
        .map(|s|(s[1].x-s[0].x).hypot(s[1].z-s[0].z)).sum()
}
fn classify_overlap(street:&Street,centerline_m:f64)->ConflictCause {
    if centerline_m>0.05 {ConflictCause::CenterlineInsideFootprint}
    else if street.tags.contains_key("width")||street.tags.contains_key("lanes"){
        ConflictCause::TaggedWidthEnvelope
    }else{ConflictCause::InferredWidthEnvelope}
}
/// Assess a road against the PART footprints separately from the parent
/// roofprint. A building_part may float above a driveway, but the presence of
/// some elevated parts does not prove the rest of the parent is empty.
fn inspect_parts(parent:&Building,street:&Street,road:&Mesh)
    ->Result<Vec<PartEvidence>,String>{
    let mut evidence=Vec::new();
    for part in &parent.parts {
        let child=part_as_building(part);
        let roof=building_roof(&child)?;
        if !street.path.windows(2).any(|segment|
            intersects(bounds(segment),roof.bounds)){
            continue;
        }
        let area=conflict_area(road,&roof);
        let (inside,_)=centerline_roof(street,&roof);
        if area<0.05&&inside<0.05 {continue;}
        evidence.push(PartEvidence{
            part_id:part.source_id.clone(),
            overlap_m2:(area*1000.0).round()/1000.0,
            centerline_inside_m:(inside*1000.0).round()/1000.0,
            measured_min_height_m:part.min_height_m,
            min_floor:part.min_floor,
            verified_above_ground:part.min_height_m.is_some_and(|m|m>0.6),
        });
    }
    evidence.sort_by(|a,b|b.overlap_m2.total_cmp(&a.overlap_m2)
        .then(a.part_id.cmp(&b.part_id)));
    Ok(evidence)
}
fn part_as_building(part:&BuildingPart)->Building{
    Building{
        source_id:part.source_id.clone(),name:None,
        height_m:part.height_m,levels:part.levels,
        min_height_m:part.min_height_m,min_floor:part.min_floor,
        polygons:part.polygons.clone(),parts:Vec::new(),
    }
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
    let grounded=|b:&&Building|HeightPolicy::default().base_for(b)<=0.6;
    let roofs:Vec<Footprint>=city.buildings.iter()
        .filter(grounded).map(building_roof).collect::<Result<_,_>>()?;
    let raised_buildings_excluded=city.buildings.len()-roofs.len();
    let clearance=ClearanceIndex::new(&city.buildings);
    let mut checked=0usize;
    let mut conflicts=Vec::new();
    let mut total=0.0;
    let mut by_cause:BTreeMap<ConflictCause,usize>=BTreeMap::new();
    let mut area_m2_by_cause:BTreeMap<ConflictCause,f64>=BTreeMap::new();
    let mut total_centerline_inside_m=0.0;
    let mut conflict_pairs_with_part_evidence=0usize;
    let mut conflict_pairs_with_measured_raised_part_evidence=0usize;
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
            let (inside_m,where_inside)=centerline_roof(street,&roofs[i]);
            let cause=classify_overlap(street,inside_m);
            *by_cause.entry(cause).or_default()+=1;
            *area_m2_by_cause.entry(cause).or_default()+=area;
            total_centerline_inside_m+=inside_m;
            let focus=where_inside.unwrap_or_else(||point_near_footprint(street,&roofs[i]));
            let parent=city.buildings.iter()
                .find(|b|b.source_id==roofs[i].source_id)
                .ok_or("roof owner missing from city")?;
            let part_evidence=inspect_parts(parent,street,&mesh)?;
            if !part_evidence.is_empty(){conflict_pairs_with_part_evidence+=1;}
            if part_evidence.iter().any(|part|part.verified_above_ground){
                conflict_pairs_with_measured_raised_part_evidence+=1;
            }
            conflicts.push(Conflict{
                building_id:roofs[i].source_id.clone(),
                street_id:street.source_id.clone(),
                road_kind:street.kind.clone(),
                overlap_m2:(area*1000.0).round()/1000.0,
                cause,centerline_inside_m:(inside_m*1000.0).round()/1000.0,
                centerline_length_m:(source_line_length(street)*1000.0).round()/1000.0,
                focus_lonlat:city.region.unproject(focus),
                source_part_evidence:part_evidence,
            });
        }
    }
    conflicts.sort_by(|a,b|b.overlap_m2.total_cmp(&a.overlap_m2)
        .then(a.building_id.cmp(&b.building_id))
        .then(a.street_id.cmp(&b.street_id)));
    for area in area_m2_by_cause.values_mut(){*area=(*area*1000.0).round()/1000.0;}
    Ok(ConflictReport{
        region_id:city.region.id.clone(),
        buildings_checked:roofs.len(),raised_buildings_excluded,
        at_grade_streets_checked:checked,
        conflict_pairs:conflicts.len(),
        sum_triangle_overlap_m2:(total*1000.0).round()/1000.0,
        by_cause,area_m2_by_cause,
        total_centerline_inside_m:(total_centerline_inside_m*1000.0).round()/1000.0,
        parent_buildings_with_parts:city.buildings.iter()
            .filter(|b|!b.parts.is_empty()).count(),
        total_parts_available:city.buildings.iter().map(|b|b.parts.len()).sum(),
        conflict_pairs_with_part_evidence,
        conflict_pairs_with_measured_raised_part_evidence,
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
            height_m:Some(10.0),levels:None,parts:Vec::new(),
            min_height_m:None,min_floor:None,polygons:vec![Polygon{rings}]}
    }
    #[test]
    fn crossing_carriageway_reports_real_area_not_point_contact(){
        let c=city(street(p(-5.,5.),p(15.,5.),2.0),building(false));
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.conflict_pairs,1);
        assert!((report.sum_triangle_overlap_m2-20.0).abs()<0.02);
    }
    #[test]
    fn conflicting_roofprint_reports_source_part_evidence_without_erasing_parent(){
        let mut parent=building(false);
        parent.parts.push(BuildingPart{
            source_id:"part:raised".into(),height_m:Some(6.0),levels:None,
            min_height_m:Some(4.0),min_floor:None,
            polygons:parent.polygons.clone(),
        });
        let c=city(street(p(-5.,5.),p(15.,5.),2.0),parent);
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.conflict_pairs,1);
        assert_eq!(report.parent_buildings_with_parts,1);
        assert_eq!(report.total_parts_available,1);
        assert_eq!(report.conflict_pairs_with_part_evidence,1);
        assert_eq!(report.conflict_pairs_with_measured_raised_part_evidence,1);
        assert_eq!(report.largest[0].source_part_evidence.len(),1);
        assert!((report.largest[0].source_part_evidence[0].centerline_inside_m-10.0).abs()<0.002);
        assert!(report.largest[0].source_part_evidence[0].verified_above_ground);
        assert_eq!(report.largest[0].cause,ConflictCause::CenterlineInsideFootprint);
    }
    #[test]
    fn unrelated_part_does_not_prove_road_passes_under_building(){
        let mut parent=building(false);
        parent.parts.push(BuildingPart{
            source_id:"part:elsewhere".into(),height_m:Some(4.0),levels:None,
            min_height_m:Some(4.0),min_floor:None,
            polygons:vec![Polygon{rings:vec![vec![
                p(20.,20.),p(30.,20.),p(30.,30.),p(20.,30.)
            ]]}],
        });
        let report=audit_road_buildings(
            &city(street(p(-5.,5.),p(15.,5.),2.0),parent)).unwrap();
        assert_eq!(report.conflict_pairs,1);
        assert_eq!(report.conflict_pairs_with_part_evidence,0);
        assert_eq!(report.conflict_pairs_with_measured_raised_part_evidence,0);
        assert!(report.largest[0].source_part_evidence.is_empty());
    }
    #[test]
    fn measured_overpass_building_no_longer_counts_as_asphalt_collision(){
        let mut floating=building(false);
        floating.min_height_m=Some(4.0);
        let c=city(street(p(-5.,5.),p(15.,5.),2.0),floating);
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.raised_buildings_excluded,1);
        assert_eq!(report.conflict_pairs,0);
        assert_eq!(report.sum_triangle_overlap_m2,0.0);
    }
    #[test]
    fn min_floor_only_is_flagged_as_an_approximate_suspended_base(){
        let mut floating=building(false);
        floating.min_floor=Some(2);
        let c=city(street(p(-5.,5.),p(15.,5.),2.0),floating);
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.raised_buildings_excluded,1);
        assert_eq!(report.conflict_pairs,0);
        assert!(c.buildings[0].min_height_m.is_none());
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
    fn classifies_real_centerline_penetration_not_merely_a_wide_road(){
        let c=city(street(p(-5.,5.),p(15.,5.),2.0),building(false));
        let report=audit_road_buildings(&c).unwrap();
        let conflict=&report.largest[0];
        assert_eq!(conflict.cause,ConflictCause::CenterlineInsideFootprint);
        assert!((conflict.centerline_inside_m-10.0).abs()<0.002);
        assert!((conflict.centerline_length_m-20.0).abs()<0.002);
        assert_eq!(report.by_cause[&ConflictCause::CenterlineInsideFootprint],1);
        assert!((report.total_centerline_inside_m-10.0).abs()<0.002);
        let [lon,lat]=conflict.focus_lonlat;
        assert!((lon-0.0).abs()<0.001 && (lat-0.0).abs()<0.001);
    }
    #[test]
    fn width_collision_without_centerline_intrusion_is_separate(){
        let c=city(street(p(-5.,-1.),p(15.,-1.),4.0),building(false));
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.conflict_pairs,1);
        assert_eq!(report.largest[0].cause,ConflictCause::TaggedWidthEnvelope);
        assert_eq!(report.largest[0].centerline_inside_m,0.0);
        assert_eq!(report.by_cause[&ConflictCause::TaggedWidthEnvelope],1);
    }
    #[test]
    fn triangle_diagonal_does_not_double_count_centerline_length(){
        let roof=building_roof(&building(false)).unwrap();
        // For an outer square the diagonal is an internal tessellation edge.
        let line=street(p(0.,0.),p(10.,10.),2.0);
        let (length,_) = centerline_roof(&line,&roof);
        assert!((length-200.0_f64.sqrt()).abs()<0.001,
            "adjacent roof triangles cannot count the same source interval twice");
    }
    #[test]
    fn centerline_along_outer_wall_is_not_source_penetration(){
        let roof=building_roof(&building(false)).unwrap();
        let line=street(p(-5.,0.),p(15.,0.),2.0);
        let (length,_) = centerline_roof(&line,&roof);
        assert!(length<0.00001,"boundary-tracing road is an edge problem");
    }
    #[test]
    fn centerline_through_courtyard_hole_is_not_source_penetration(){
        let roof=building_roof(&building(true)).unwrap();
        let line=street(p(3.5,5.),p(6.5,5.),1.0);
        let (length,_) = centerline_roof(&line,&roof);
        assert!(length<0.00001);
    }
    #[test]
    fn inferred_service_shoulder_collision_not_mislabeled_as_centerline(){
        let mut service=street(p(-5.,-1.),p(15.,-1.),5.5);
        service.kind="service".into();
        service.tags.clear();
        let c=city(service,building(false));
        let report=audit_road_buildings(&c).unwrap();
        assert_eq!(report.by_cause.get(&ConflictCause::CenterlineInsideFootprint),None);
        // The clearance pass may remove this overlap entirely, which is good.
        for conflict in report.largest {
            assert_eq!(conflict.cause,ConflictCause::InferredWidthEnvelope);
            assert_eq!(conflict.centerline_inside_m,0.0);
        }
    }
    #[test]
    fn tunnel_is_not_treated_as_surface_road(){
        let mut c=city(street(p(-5.,5.),p(15.,5.),2.0),building(false));
        c.streets[0].tags.insert("tunnel".into(),"yes".into());
        assert_eq!(audit_road_buildings(&c).unwrap().at_grade_streets_checked,0);
    }
}
