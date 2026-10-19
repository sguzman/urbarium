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


/* Camera-following mesh cells. Each cell owns its building footprints once.
 * Streets retain the ORIGINAL source path (including bends and genuine
 * endpoint topology) until after surface tessellation. The finished road,
 * walk, step and rail triangles are clipped to tile bounds in chunk_mesh.
 * Clipping centerlines here previously erased miter context at every seam.
 */
pub const CHUNK_SIZE_M:f64=300.0;
// Keep enough cells to preserve distant skyline in the bounded pilot.
// Larger regions will need screen-space LOD and measured frustum culling.
pub const VIEW_RADIUS_M:f64=3_000.0;
pub const MAX_RESIDENT_CHUNKS:usize=96;

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
/// Every chunk covering the source region is a *potentially visible* chunk.
/// Prioritize proximity without discarding the distant skyline: on the pilot
/// all cells fit within the view radius and the bounded GPU residency budget.
pub fn all_region_chunks(region:&crate::Region)->Vec<ChunkKey>{
    let sw=region.project(region.bbox[0],region.bbox[1]);
    let ne=region.project(region.bbox[2],region.bbox[3]);
    let x0=(sw.x/CHUNK_SIZE_M).floor() as i32;
    let x1=(ne.x/CHUNK_SIZE_M).floor() as i32;
    let z0=(sw.z/CHUNK_SIZE_M).floor() as i32;
    let z1=(ne.z/CHUNK_SIZE_M).floor() as i32;
    let mut chunks=Vec::new();
    for z in z0..=z1{
        for x in x0..=x1{
            let k=ChunkKey{x,z};
            let b=k.bounds();
            if b[0]<ne.x && b[2]>sw.x && b[1]<ne.z && b[3]>sw.z {
                chunks.push(k);
            }
        }
    }
    chunks
}
fn squared_distance_to_cell(p:Point,key:ChunkKey)->f64{
    let b=key.bounds();
    let dx=(p.x.clamp(b[0],b[2])-p.x).abs();
    let dz=(p.z.clamp(b[1],b[3])-p.z).abs();
    dx*dx+dz*dz
}
pub fn desired_chunks(camera:Point,region:&crate::Region)->Vec<ChunkKey>{
    desired_chunks_with_limit(camera,region,MAX_RESIDENT_CHUNKS)
}
/// User-adjustable GPU detail residency. Outside these cells the standalone
/// viewer draws compact distant massing instead of making buildings vanish.
pub fn desired_chunks_with_limit(camera:Point,region:&crate::Region,
    max_detailed_chunks:usize)->Vec<ChunkKey>{
    let mut chunks=all_region_chunks(region);
    // Conservative AABB distance rather than a view-direction cone:
    // an elevated/rotating camera must not make background buildings vanish.
    chunks.retain(|k|squared_distance_to_cell(camera,*k)<=VIEW_RADIUS_M*VIEW_RADIUS_M);
    chunks.sort_by(|a,b|{
        squared_distance_to_cell(camera,*a)
            .total_cmp(&squared_distance_to_cell(camera,*b))
            .then_with(||a.cmp(b))
    });
    chunks.truncate(max_detailed_chunks.clamp(1,MAX_RESIDENT_CHUNKS));
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
/// Upper bound on ribbon reach from its source centerline: 60m source
/// width / 2, times the 2x capped miter at bends = 60m. Leave another 20m
/// for numerical and future intersection margin. Only source candidates near
/// a cell are meshed, while their full paths preserve bend geometry.
pub const ROAD_CONTEXT_HALO_M:f64=80.0;

fn segment_near_cell(a:Point,b:Point,bounds:[f64;4])->bool {
    let halo=ROAD_CONTEXT_HALO_M;
    a.x.min(b.x)<=bounds[2]+halo && a.x.max(b.x)>=bounds[0]-halo &&
    a.z.min(b.z)<=bounds[3]+halo && a.z.max(b.z)>=bounds[1]-halo
}

/// Keep only source spans near a tile, plus one adjacent control vertex on
/// either side of each run. The guard vertices preserve full-path miter
/// normals at every visible bend. Any temporary run end is at least the
/// 80-meter context halo from the rendered cell (ribbon reach <=60m), so
/// it cannot create a visible artificial end cap or junction.
fn local_street_spans(street:&Street,bounds:[f64;4])->Vec<Street>{
    let len=street.path.len();
    if len<2 {return Vec::new();}
    let mut ranges:Vec<(usize,usize)>=Vec::new();
    for (i,pair) in street.path.windows(2).enumerate(){
        if !segment_near_cell(pair[0],pair[1],bounds){continue;}
        let lo=i.saturating_sub(1);
        let hi=(i+2).min(len-1);
        if let Some(last)=ranges.last_mut() {
            if lo<=last.1 {
                last.1=last.1.max(hi);
                continue;
            }
        }
        ranges.push((lo,hi));
    }
    ranges.into_iter().map(|(lo,hi)|{
        let mut part=street.clone();
        part.path=street.path[lo..=hi].to_vec();
        if street.osm_node_ids.len()==len {
            part.osm_node_ids=street.osm_node_ids[lo..=hi].to_vec();
        }else{
            part.osm_node_ids.clear();
        }
        part
    }).collect()
}

/// Choose source-derived road spans with enough neighboring centerline
/// context to render the identical visible bends as the full source city.
/// The finished surface triangles are then clipped to exact geographic
/// tile/region bounds by chunk_mesh; never clip the centerline to that tile.
pub fn chunk_city(source:&City,key:ChunkKey)->City {
    let bounds=key.bounds();
    City {
        schema_version:source.schema_version,
        region:source.region.clone(),provenance:Vec::new(),
        streets:source.streets.iter()
            .flat_map(|street|local_street_spans(street,bounds))
            .collect(),
        buildings:source.buildings.iter()
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
    fn detailed_chunk_budget_is_camera_local_and_bounded(){
        let mut region=crate::Region{
            id:"large".into(),name:"large".into(),
            origin:[-118.25,34.05],bbox:[-118.30,34.00,-118.20,34.10]
        };
        assert!(all_region_chunks(&region).len()>24);
        let selected=desired_chunks_with_limit(Point{x:0.,z:0.},&region,12);
        assert_eq!(selected.len(),12);
        assert!(selected.contains(&ChunkKey::at(Point{x:0.,z:0.})));
        assert_eq!(desired_chunks_with_limit(Point{x:0.,z:0.},&region,100000).len()<=MAX_RESIDENT_CHUNKS,true);
        region.bbox=[-118.255,34.045,-118.245,34.055];
        let small=desired_chunks_with_limit(Point{x:0.,z:0.},&region,12);
        assert!(small.len()<=12);
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
        // A small pilot must not lose buildings when the camera moves:
        // every region cell remains in scope, including distant high-rises.
        assert_eq!(at_origin.len(),all_region_chunks(&city.region).len());
        assert_eq!(at_origin.iter().copied().collect::<std::collections::BTreeSet<_>>(),
            elsewhere.iter().copied().collect());
        assert!(at_origin.len()>9);
    }
    #[test]
    fn all_pilot_cells_survive_elevated_camera_relocation() {
        let region=crate::Region{
            id:"downtown-la".into(),name:"Downtown LA".into(),
            origin:[-118.25,34.05],
            bbox:[-118.258,34.045,-118.242,34.056],
        };
        let all=all_region_chunks(&region);
        assert!(all.len()>9 && all.len()<=MAX_RESIDENT_CHUNKS, "{:?}",all.len());
        for camera in [
            Point{x:0.0,z:0.0}, Point{x:-400.0,z:-400.0},
            Point{x:400.0,z:300.0},Point{x:600.0,z:-300.0},
        ] {
            let requested=desired_chunks(camera,&region);
            let keys:std::collections::BTreeSet<_>=requested.into_iter().collect();
            assert_eq!(keys.len(),all.len(),"camera {camera:?} lost a skyline cell");
            assert!(all.iter().all(|k|keys.contains(k)));
        }
    }
    #[test]
    fn far_away_cells_are_still_bounded_on_large_geography() {
        let huge=crate::Region{
            id:"large".into(),name:"large".into(),
            origin:[0.0,0.0],bbox:[-0.09,-0.09,0.09,0.09],
        };
        let origin=desired_chunks(Point{x:0.0,z:0.0},&huge);
        let remote=desired_chunks(Point{x:5000.0,z:5000.0},&huge);
        assert!(origin.len()<=MAX_RESIDENT_CHUNKS);
        assert!(remote.len()<=MAX_RESIDENT_CHUNKS);
        assert!(!origin.is_empty() && !remote.is_empty());
        assert_ne!(origin.iter().copied().collect::<std::collections::BTreeSet<_>>(),
            remote.iter().copied().collect());
    }

    #[test]
    fn cross_cell_streets_preserve_source_bends_and_real_endpoints() {
        let mut city=sample_city();
        city.streets.push(street("bend",&[
            (275.,100.),(298.,100.),(320.,130.),(350.,130.)
        ]));
        city.streets[0].osm_node_ids=vec![11,12];
        let a=chunk_city(&city,ChunkKey{x:0,z:0});
        let b=chunk_city(&city,ChunkKey{x:1,z:0});
        for selected in [&a,&b] {
            let cross=selected.streets.iter()
                .find(|s|s.source_id=="crossing").unwrap();
            assert_eq!(cross.path,city.streets[0].path);
            assert_eq!(cross.osm_node_ids,[11,12]);
            let bend=selected.streets.iter()
                .find(|s|s.source_id=="bend").unwrap();
            assert_eq!(bend.path,city.streets.last().unwrap().path);
        }
        assert_eq!(city.streets[0].path[0].x,-800.0);
    }
    #[test]
    fn long_winding_source_is_windowed_without_losing_bend_neighbors() {
        let mut city=sample_city();
        let points:(Vec<(f64,f64)>)=(-100..=100)
            .map(|i|(i as f64*10.0,100.0+if i>=30{20.0}else{0.0}))
            .collect();
        let mut winding=street("long",&points);
        winding.osm_node_ids=(0..winding.path.len() as i64).collect();
        city.streets.push(winding);
        let selected=chunk_city(&city,ChunkKey{x:0,z:0});
        let span=selected.streets.iter().find(|s|s.source_id=="long").unwrap();
        assert!(span.path.len()<70,
            "meshing an entire kilometer-long road per tile regresses worker memory");
        assert!(span.path.len()>35);
        assert!(span.path.iter().any(|p|p.x==300.0));
        assert_eq!(span.path.len(),span.osm_node_ids.len());
        for (p,id) in span.path.iter().zip(&span.osm_node_ids){
            assert_eq!(p.x,*id as f64*10.0-1000.0);
        }
        assert_eq!(city.streets.last().unwrap().path.len(),201,
            "source must not be rewritten for other chunks or minimap");
    }
    #[test]
    fn surface_halo_selects_width_without_exploding_to_all_streets() {
        let mut city=sample_city();
        city.streets.push(street("wide",&[(310.,80.),(320.,90.)]));
        city.streets.push(street("remote",&[(500.,80.),(520.,90.)]));
        let chosen=chunk_city(&city,ChunkKey{x:0,z:0});
        assert!(chosen.streets.iter().any(|s|s.source_id=="wide"));
        assert!(!chosen.streets.iter().any(|s|s.source_id=="remote"));
    }
    #[test]
    fn a_building_is_owned_by_one_cell_even_when_it_crosses_an_edge() {
        let mut city=sample_city();
        city.buildings.push(crate::Building{
            source_id:"b1".into(),name:None,height_m:Some(10.0),levels:None,
            min_height_m:None,min_floor:None,parts:Vec::new(),
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
