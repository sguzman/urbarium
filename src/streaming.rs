//! Small, source-derived geographic preview selection for the first frame.
//!
//! A preview is deliberately not a source-data replacement or a completed
//! streaming system. Keep entire source street paths when they touch the
//! camera circle, so a long road crossing the view is not lost just because
//! both endpoints lie outside the radius. Full-city geometry is generated
//! after the first frame as before.

use crate::{City, Point, Street};

pub const FIRST_FRAME_STREET_RADIUS_M: f64 = 450.0;

/// The shortest squared distance from a planar point to a line segment.
fn segment_distance_squared(point:Point,start:Point,end:Point)->f64 {
    let dx=end.x-start.x;
    let dz=end.z-start.z;
    let length_squared=dx*dx+dz*dz;
    if length_squared<=f64::EPSILON {
        return (point.x-start.x).powi(2)+(point.z-start.z).powi(2);
    }
    let t=(((point.x-start.x)*dx+(point.z-start.z)*dz)/length_squared)
        .clamp(0.0,1.0);
    (point.x-(start.x+t*dx)).powi(2)+(point.z-(start.z+t*dz)).powi(2)
}

/// True if a source street has at least one segment meeting the view circle.
pub fn street_intersects_radius(street:&Street,center:Point,radius_m:f64)->bool {
    if !radius_m.is_finite() || radius_m<0.0 ||
        !center.x.is_finite() || !center.z.is_finite() {return false;}
    let limit=radius_m*radius_m;
    street.path.windows(2).any(|pair|
        segment_distance_squared(center,pair[0],pair[1])<=limit)
}

/// Build a street-only preview input; untouched source city stays authoritative.
///
/// Source paths are not clipped at the radius (junctions/width rules need
/// original geometry). Building and provenance collections are omitted here:
/// terrain and the original city are supplied separately by the viewer.
pub fn first_frame_streets(city:&City,center:Point,radius_m:f64)->City {
    City {
        schema_version:city.schema_version,
        region:city.region.clone(),
        provenance:Vec::new(),
        buildings:Vec::new(),
        streets:city.streets.iter()
            .filter(|street|street_intersects_radius(street,center,radius_m))
            .cloned().collect(),
    }
}


/* Camera-following mesh cells. Every cell owns its building footprints once,
 * and every street segment is clipped to exactly one cell interior (shared
 * geometric boundaries may coincide but do not duplicate surface area).
 * The original source city and terrain are kept unchanged for collision,
 * minimap and explicit full-scene fallback.
 */
pub const CHUNK_SIZE_M:f64=300.0;
pub const CHUNK_RADIUS:i32=1;
pub const MAX_RESIDENT_CHUNKS:usize=9;

#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord,Hash)]
pub struct ChunkKey{pub x:i32,pub z:i32}
impl ChunkKey {
    pub fn at(p:Point)->Self{
        Self{x:(p.x/CHUNK_SIZE_M).floor() as i32,
            z:(p.z/CHUNK_SIZE_M).floor() as i32}
    }
    pub fn bounds(self)->[f64;4]{
        [self.x as f64*CHUNK_SIZE_M,self.z as f64*CHUNK_SIZE_M,
         (self.x+1) as f64*CHUNK_SIZE_M,(self.z+1) as f64*CHUNK_SIZE_M]
    }
    pub fn center(self)->Point{
        let b=self.bounds();
        Point{x:(b[0]+b[2])*0.5,z:(b[1]+b[3])*0.5}
    }
}
pub fn desired_chunks(camera:Point,region:&crate::Region)->Vec<ChunkKey>{
    let origin=ChunkKey::at(camera);
    let sw=region.project(region.bbox[0],region.bbox[1]);
    let ne=region.project(region.bbox[2],region.bbox[3]);
    let mut chunks=Vec::new();
    for dz in -CHUNK_RADIUS..=CHUNK_RADIUS {
        for dx in -CHUNK_RADIUS..=CHUNK_RADIUS {
            let k=ChunkKey{x:origin.x+dx,z:origin.z+dz};
            let b=k.bounds();
            if b[0]<ne.x && b[2]>sw.x && b[1]<ne.z && b[3]>sw.z {
                chunks.push(k);
            }
        }
    }
    chunks.sort_by(|a,b|{
        let a=a.center();let b=b.center();
        let da=(a.x-camera.x).powi(2)+(a.z-camera.z).powi(2);
        let db=(b.x-camera.x).powi(2)+(b.z-camera.z).powi(2);
        da.total_cmp(&db).then_with(||a.x.total_cmp(&b.x))
    });
    chunks.truncate(MAX_RESIDENT_CHUNKS);
    chunks
}
fn owner(building:&crate::Building)->Option<ChunkKey>{
    let ring=building.polygons.first()?.rings.first()?;
    if ring.is_empty(){return None;}
    let centroid=Point{
        x:ring.iter().map(|p|p.x).sum::<f64>()/ring.len() as f64,
        z:ring.iter().map(|p|p.z).sum::<f64>()/ring.len() as f64,
    };
    Some(ChunkKey::at(centroid))
}
/// Liang-Barsky clipping, using exact shared cell bounds. Endpoint-only and
/// zero-length touches are discarded; true crossings are retained.
fn clip_segment(a:Point,b:Point,bounds:[f64;4])->Option<(Point,Point)>{
    // Half-open cell ownership: a segment lying exactly on a shared
    // north/east border belongs to that neighboring cell only.
    if ((a.x-bounds[2]).abs()<1e-9 && (b.x-bounds[2]).abs()<1e-9)
       || ((a.z-bounds[3]).abs()<1e-9 && (b.z-bounds[3]).abs()<1e-9) {
        return None;
    }
    let dx=b.x-a.x;let dz=b.z-a.z;
    let mut lo:f64=0.0;let mut hi:f64=1.0;
    for (p,q) in [(-dx,a.x-bounds[0]),(dx,bounds[2]-a.x),
                  (-dz,a.z-bounds[1]),(dz,bounds[3]-a.z)] {
        if p.abs()<1e-12 {
            if q<0.0 {return None;}
        }else{
            let t=q/p;
            if p<0.0 {lo=lo.max(t);}else{hi=hi.min(t);}
            if lo>=hi{return None;}
        }
    }
    let start=Point{x:a.x+lo*dx,z:a.z+lo*dz};
    let end=Point{x:a.x+hi*dx,z:a.z+hi*dz};
    if (end.x-start.x).hypot(end.z-start.z)<1e-5{return None;}
    Some((start,end))
}
/// Select geometry owned by the requested cell, without whole-city meshing.
/// Source street tags/class are retained for roads and bridge eligibility.
pub fn chunk_city(source:&City,key:ChunkKey)->City {
    let bbox=key.bounds();
    let mut streets=Vec::new();
    for street in &source.streets {
        for (i,segment) in street.path.windows(2).enumerate() {
            if let Some((a,b))=clip_segment(segment[0],segment[1],bbox){
                let mut fragment=street.clone();
                fragment.source_id=format!("{}:chunk:{}:{}:segment:{}",street.source_id,
                    key.x,key.z,i);
                fragment.path=vec![a,b];
                fragment.osm_node_ids.clear(); // Clipped endpoints are new coordinates
                streets.push(fragment);
            }
        }
    }
    City {
        schema_version:source.schema_version,
        region:source.region.clone(),provenance:Vec::new(),
        streets,buildings:source.buildings.iter()
            .filter(|b|owner(b)==Some(key)).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn street(id:&str, points:&[(f64,f64)])->Street{
        Street {
            source_id:id.to_owned(),name:None,kind:"residential".into(),
            path:points.iter().map(|&(x,z)|Point{x,z}).collect(),
            osm_node_ids:Vec::new(),tags:BTreeMap::new(),
        }
    }
    fn sample_city()->City{
        City {
            schema_version:1,
            region:crate::Region{
                id:"test".into(),name:"Test".into(),
                origin:[0.0,0.0],bbox:[-0.01,-0.01,0.01,0.01],
            },
            provenance:Vec::new(),buildings:Vec::new(),
            streets:vec![
                street("crossing",&[(-800.0,0.0),(800.0,0.0)]),
                street("near",&[(20.0,20.0),(50.0,40.0)]),
                street("far",&[(550.0,550.0),(700.0,700.0)]),
            ],
        }
    }

    #[test]
    fn chunks_follow_camera_and_remain_strictly_bounded() {
        let city=sample_city();
        let at_origin=desired_chunks(Point{x:0.0,z:0.0},&city.region);
        let elsewhere=desired_chunks(Point{x:600.0,z:0.0},&city.region);
        assert!(at_origin.len()<=MAX_RESIDENT_CHUNKS);
        assert!(elsewhere.len()<=MAX_RESIDENT_CHUNKS);
        assert!(at_origin.contains(&ChunkKey{x:0,z:0}));
        assert!(elsewhere.contains(&ChunkKey{x:2,z:0}));
        assert_ne!(at_origin,elsewhere);
        assert!(at_origin.iter().any(|k| !elsewhere.contains(k)));
    }
    #[test]
    fn clipping_preserves_cross_cell_road_without_repeated_area() {
        let city=sample_city();
        let a=chunk_city(&city,ChunkKey{x:0,z:0});
        let b=chunk_city(&city,ChunkKey{x:1,z:0});
        let a_road=a.streets.iter().find(|s|s.source_id.starts_with("crossing")).unwrap();
        let b_road=b.streets.iter().find(|s|s.source_id.starts_with("crossing")).unwrap();
        assert_eq!(a_road.path[0].x,0.0);
        assert_eq!(a_road.path[1].x,CHUNK_SIZE_M);
        assert_eq!(b_road.path[0].x,CHUNK_SIZE_M);
        assert_eq!(b_road.path[1].x,CHUNK_SIZE_M*2.0);
        assert_eq!(city.streets[0].path[0].x,-800.0);
        assert!(a_road.osm_node_ids.is_empty());
    }
    #[test]
    fn a_building_is_owned_by_one_cell_even_when_it_crosses_an_edge() {
        let mut city=sample_city();
        city.buildings.push(crate::Building{
            source_id:"b1".into(),name:None,height_m:Some(10.0),levels:None,
            polygons:vec![crate::Polygon{rings:vec![vec![
                Point{x:290.0,z:12.0},Point{x:320.0,z:12.0},
                Point{x:320.0,z:40.0},Point{x:290.0,z:40.0},
            ]]}],
        });
        let a=chunk_city(&city,ChunkKey{x:0,z:0});
        let b=chunk_city(&city,ChunkKey{x:1,z:0});
        assert_eq!(a.buildings.len()+b.buildings.len(),1);
        assert_eq!(b.buildings[0].source_id,"b1");
        assert_eq!(city.buildings.len(),1);
    }
    #[test]
    fn clip_rejects_zero_length_touch_and_missed_cell() {
        let bbox=[0.0,0.0,300.0,300.0];
        assert!(clip_segment(Point{x:-20.0,z:0.0},Point{x:0.0,z:0.0},bbox).is_none());
        assert!(clip_segment(Point{x:-20.0,z:-10.0},Point{x:-1.0,z:-10.0},bbox).is_none());
    }

    #[test]
    fn long_crossing_street_is_kept_even_when_endpoints_are_distant() {
        let city=sample_city();
        let preview=first_frame_streets(&city,Point{x:0.0,z:0.0},200.0);
        assert_eq!(preview.streets.len(),2);
        assert_eq!(preview.streets[0].source_id,"crossing");
        assert_eq!(preview.streets[0].path,city.streets[0].path);
        assert_eq!(city.streets.len(),3,"source city must remain unchanged");
    }
    #[test]
    fn radius_is_geometric_not_just_vertex_distance() {
        let center=Point{x:0.0,z:0.0};
        assert!(street_intersects_radius(
            &street("touch",&[(-10.0,5.0),(10.0,5.0)]),center,5.0));
        assert!(!street_intersects_radius(
            &street("miss",&[(-10.0,5.01),(10.0,5.01)]),center,5.0));
        assert!(street_intersects_radius(
            &street("bend",&[(-100.0,100.0),(0.0,0.0),(100.0,100.0)]),center,1.0));
        assert!(!street_intersects_radius(
            &street("isolated",&[(0.0,0.0)]),center,5.0));
        assert!(!street_intersects_radius(
            &street("invalid",&[(0.0,0.0),(10.0,10.0)]),center,f64::NAN));
    }
    #[test]
    fn far_roads_are_excluded_without_dropping_full_city_data() {
        let city=sample_city();
        let preview=first_frame_streets(&city,Point{x:0.0,z:0.0},200.0);
        assert!(preview.buildings.is_empty());
        assert!(preview.provenance.is_empty());
        assert_eq!(preview.region.id,city.region.id);
        assert!(preview.streets.iter().all(|s|s.source_id!="far"));
        assert_eq!(city.streets.len(),3);
    }
}
