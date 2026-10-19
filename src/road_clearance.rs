//! Building-aware clearance for source centerlines with *unknown* width.
//!
//! Overture service roads usually lack a measured physical carriageway width.
//! A fixed 5.5m display ribbon can then intrude sideways into a building,
//! even when the actual centerline is wholly outside that footprint.
//!
//! Keep the centerline and explicitly surveyed/tagged widths untouched.
//! For inferred service/track ribbons only, adjust each side independently to
//! the first intersected building wall. This is a display envelope, not a
//! new road survey or a change to the source data. Whole-city footprints
//! are used by every render cell, so widths are identical at chunk seams.
//! Where the SOURCE centerline itself lies inside a building, do not pretend
//! to repair an ambiguous covered/internal road: preserve it for audit.

use crate::{Building,Point,Street};
use crate::geometry::Mesh;

const WALL_MARGIN_M:f64=0.12;
const MIN_EDGE_M:f64=0.025;
const CROSS_STEPS:usize=crate::roads::RIBBON_CROSS_STEPS;
const STRIDE:usize=CROSS_STEPS+1;

#[derive(Clone)]
struct Wall {
    bounds:[f64;4],
    rings:Vec<Vec<Point>>,
}
pub struct ClearanceIndex {
    walls:Vec<Wall>,
}
fn bounds_of(points:&[Point])->[f64;4]{
    let mut b=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for p in points {
        b[0]=b[0].min(p.x);b[1]=b[1].min(p.z);
        b[2]=b[2].max(p.x);b[3]=b[3].max(p.z);
    }
    b
}
fn overlaps(a:[f64;4],b:[f64;4])->bool{
    a[0]<=b[2]&&a[2]>=b[0]&&a[1]<=b[3]&&a[3]>=b[1]
}
fn inside_ring(p:Point,ring:&[Point])->bool{
    if ring.len()<3 {return false;}
    let mut inside=false;
    let mut last=*ring.last().unwrap();
    for &next in ring {
        if (next.z>p.z)!=(last.z>p.z) {
            let intersection=next.x+(p.z-next.z)*(last.x-next.x)/(last.z-next.z);
            if p.x<intersection {inside=!inside;}
        }
        last=next;
    }
    inside
}
fn inside_footprint(p:Point,wall:&Wall)->bool {
    inside_ring(p,&wall.rings[0]) &&
        !wall.rings.iter().skip(1).any(|ring|inside_ring(p,ring))
}
fn cross(a:Point,b:Point)->f64{a.x*b.z-a.z*b.x}
fn sub(a:Point,b:Point)->Point{Point{x:a.x-b.x,z:a.z-b.z}}
fn lerp(a:Point,b:Point,t:f64)->Point{Point{x:a.x+(b.x-a.x)*t,z:a.z+(b.z-a.z)*t}}

/// Segment p->q intersects a ring wall at fraction t. This operates on both
/// outer and courtyard wall segments; holes remain open inside their rings.
fn first_boundary(p:Point,q:Point,wall:&Wall)->Option<f64> {
    if !overlaps(bounds_of(&[p,q]),wall.bounds){return None;}
    let d=sub(q,p);
    let mut first:Option<f64>=None;
    for ring in &wall.rings {
        if ring.len()<2{continue;}
        for i in 0..ring.len(){
            let a=ring[i];let b=ring[(i+1)%ring.len()];
            let e=sub(b,a);
            let det=cross(d,e);
            if det.abs()<1e-11 {continue;}
            let rel=sub(a,p);
            let t=cross(rel,e)/det;
            let u=cross(rel,d)/det;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                first=Some(first.map_or(t,|v|v.min(t)));
            }
        }
    }
    first
}
impl ClearanceIndex {
    pub fn new(buildings:&[Building])->Self {
        let mut walls=Vec::new();
        for building in buildings {
            for polygon in &building.polygons {
                let Some(outer)=polygon.rings.first() else{continue;};
                if outer.len()<3 || polygon.rings.iter().flatten()
                    .any(|p|!p.x.is_finite()||!p.z.is_finite()) {continue;}
                walls.push(Wall{
                    bounds:bounds_of(outer),
                    rings:polygon.rings.clone(),
                });
            }
        }
        Self{walls}
    }
    /// Only width-inferred local service roads qualify; measured lanes/widths
    /// must never be silently overwritten by nearby building geometry.
    pub fn eligible(street:&Street)->bool {
        matches!(street.kind.as_str(),"service"|"track") &&
            !street.tags.contains_key("width") &&
            !street.tags.contains_key("lanes")
    }
    fn candidates(&self,street:&Street,width:f64)->Vec<&Wall>{
        if street.path.len()<2{return Vec::new();}
        let mut b=bounds_of(&street.path);
        let reach=width.max(0.0)+1.0; // capped 2x miter, plus rounding
        b[0]-=reach;b[1]-=reach;b[2]+=reach;b[3]+=reach;
        self.walls.iter().filter(|wall|overlaps(b,wall.bounds)).collect()
    }
    /// Edit one already-formed five-across continuous ribbon, preserving its
    /// source center vertices and mesh topology. Only the inferred bilateral
    /// envelope moves; adjacent rows stay smoothly connected.
    pub fn constrain(&self,street:&Street,width:f64,mesh:&mut Mesh){
        if !Self::eligible(street)||self.walls.is_empty()||
            mesh.vertices.len()<2*STRIDE||mesh.vertices.len()%STRIDE!=0||
            !width.is_finite()||width<=0.0 {return;}
        let walls=self.candidates(street,width);
        if walls.is_empty(){return;}
        for row in mesh.vertices.chunks_exact_mut(STRIDE){
            let center=Point{x:row[CROSS_STEPS/2].position[0] as f64,
                             z:row[CROSS_STEPS/2].position[2] as f64};
            // A centerline passing through a building is ambiguous source
            // geometry, not a reason to collapse a road onto one edge.
            if walls.iter().any(|wall|overlaps(wall.bounds,
                [center.x,center.z,center.x,center.z]) &&
                inside_footprint(center,wall)){continue;}
            let original_left=Point{x:row[0].position[0] as f64,z:row[0].position[2] as f64};
            let original_right=Point{x:row[CROSS_STEPS].position[0] as f64,
                                     z:row[CROSS_STEPS].position[2] as f64};
            let adjust=|end:Point|->Point {
                let total=(end.x-center.x).hypot(end.z-center.z);
                if total<=0.000001 {return end;}
                let mut limit=1.0_f64;
                let line_bounds=bounds_of(&[center,end]);
                for wall in &walls {
                    if !overlaps(line_bounds,wall.bounds){continue;}
                    if let Some(t)=first_boundary(center,end,wall) {
                        limit=limit.min(((t*total-WALL_MARGIN_M).max(MIN_EDGE_M)/total).min(1.0));
                    }
                }
                lerp(center,end,limit)
            };
            let left=adjust(original_left);
            let right=adjust(original_right);
            // Keep the midpoint exactly fixed. Changing both edges
            // independently prevents geometric drift into the opposite wall.
            for (k,v) in row.iter_mut().enumerate(){
                if k==CROSS_STEPS/2{continue;}
                let target=if k<CROSS_STEPS/2 {left}else{right};
                let t=if k<CROSS_STEPS/2 {
                    (CROSS_STEPS/2-k) as f64/(CROSS_STEPS/2) as f64
                }else{
                    (k-CROSS_STEPS/2) as f64/(CROSS_STEPS/2) as f64
                };
                let p=lerp(center,target,t);
                v.position[0]=p.x as f32;
                v.position[2]=p.z as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Polygon};
    use crate::roads::ribbon;
    use std::collections::BTreeMap;
    fn p(x:f64,z:f64)->Point{Point{x,z}}
    fn building(outer:Vec<Point>,holes:Vec<Vec<Point>>)->Building{
        let mut rings=vec![outer];rings.extend(holes);
        Building{source_id:"test".into(),name:None,height_m:Some(9.),
            levels:None,polygons:vec![Polygon{rings}]}
    }
    fn service(coords:&[(f64,f64)])->Street{
        Street{source_id:"street".into(),name:None,kind:"service".into(),
            path:coords.iter().map(|&(x,z)|p(x,z)).collect(),
            osm_node_ids:vec![],tags:BTreeMap::new()}
    }
    #[test]
    fn sidewalk_clearance_trims_only_the_building_facing_side(){
        let wall=building(vec![p(-10.,0.),p(10.,0.),p(10.,10.),p(-10.,10.)],vec![]);
        let index=ClearanceIndex::new(&[wall]);
        let street=service(&[(-8.,-2.),(8.,-2.)]);
        let mut mesh=ribbon(&street.path,5.5,0.24).unwrap().unwrap();
        let original=mesh.clone();
        index.constrain(&street,5.5,&mut mesh);
        for i in (0..mesh.vertices.len()).step_by(STRIDE){
            assert_eq!(mesh.vertices[i+2].position,original.vertices[i+2].position,
                "original source centerline must never move");
            assert!(mesh.vertices[i].position[2]<=-0.10,
                "building-facing edge must stay before exterior wall");
            assert!((mesh.vertices[i+4].position[2]-original.vertices[i+4].position[2]).abs()<0.0001,
                "opposite street edge is not affected by a north-side building");
        }
        assert_eq!(mesh.indices,original.indices);
    }
    #[test]
    fn measured_width_is_preserved_even_with_nearby_walls(){
        let wall=building(vec![p(-10.,0.),p(10.,0.),p(10.,10.),p(-10.,10.)],vec![]);
        let index=ClearanceIndex::new(&[wall]);
        let mut street=service(&[(-8.,-2.),(8.,-2.)]);
        street.tags.insert("width".into(),"5.5m".into());
        let mut mesh=ribbon(&street.path,5.5,0.24).unwrap().unwrap();
        let original=mesh.clone();
        index.constrain(&street,5.5,&mut mesh);
        assert_eq!(mesh.vertices.len(),original.vertices.len());
        for (a,b) in mesh.vertices.iter().zip(original.vertices.iter()) {
            assert_eq!(a.position,b.position);
        }
    }
    #[test]
    fn courtyard_holes_remain_open_and_centerline_overlap_stays_auditable(){
        let wall=building(vec![p(-10.,-10.),p(10.,-10.),p(10.,10.),p(-10.,10.)],
            vec![vec![p(-3.,-3.),p(-3.,3.),p(3.,3.),p(3.,-3.)]]);
        let index=ClearanceIndex::new(&[wall]);
        let street=service(&[(-2.,0.),(2.,0.)]);
        let mut mesh=ribbon(&street.path,5.5,0.24).unwrap().unwrap();
        let original=mesh.clone();
        index.constrain(&street,5.5,&mut mesh);
        for (a,b) in mesh.vertices.iter().zip(original.vertices.iter()){
            assert_eq!(a.position,b.position);
        }
        let crossing=service(&[(-12.,5.),(12.,5.)]);
        let mut road=ribbon(&crossing.path,5.5,0.24).unwrap().unwrap();
        let untouched=road.vertices[2].position;
        index.constrain(&crossing,5.5,&mut road);
        assert_eq!(road.vertices[2].position,untouched);
    }
}
