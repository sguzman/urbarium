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
