//! Common projected scene coordinates for independently sourced geographic sections.
//! No full metro mesh is needed: source-local vertices are translated only
//! when their section is resident in the viewer. Heights share the same datum.
use crate::Region;

pub const MAX_RESIDENT_SECTIONS: usize = 2;
pub const EARTH_RADIUS_METERS:f64=6_371_008.8;

#[derive(Clone, Copy, Debug)]
pub struct TilePlacement {
    pub east_m:f32,
    pub north_m:f32,
    pub up_m:f32,
    pub east_scale:f32,
}
impl TilePlacement {
    pub fn between(reference:&Region,reference_absolute_elevation:f32,
        local:&Region,local_absolute_elevation:f32)->Self {
        let origin=reference.project(local.origin[0],local.origin[1]);
        Self{
            east_m:origin.x as f32,
            north_m:origin.z as f32,
            up_m:local_absolute_elevation-reference_absolute_elevation,
            east_scale:(reference.origin[1].to_radians().cos()/
                local.origin[1].to_radians().cos()) as f32,
        }
    }
    pub fn point(&self,x:f32,y:f32,z:f32)->[f32;3] {
        [self.east_m+x*self.east_scale,self.up_m+y,self.north_m+z]
    }
    pub fn local(&self,x:f64,z:f64)->[f64;2] {
        [(x-self.east_m as f64)/self.east_scale as f64,z-self.north_m as f64]
    }
    pub fn bounds(&self, region:&Region)->[f64;4] {
        let sw=region.project(region.bbox[0],region.bbox[1]);
        let ne=region.project(region.bbox[2],region.bbox[3]);
        let a=self.point(sw.x as f32,0.0,sw.z as f32);
        let b=self.point(ne.x as f32,0.0,ne.z as f32);
        [a[0] as f64,a[2] as f64,b[0] as f64,b[2] as f64]
    }
}

/// Rank detailed geometry across ALL loaded geographic sections using one
/// camera position and one total cell budget. Nonresident cells can retain
/// their independent source-footprint low-detail skyline buffers.
///
/// Coordinates are compared in the same reference frame; keys remain local
/// to their owning source region so no two regions collide in the map.
pub fn prioritized_world_cells(camera:crate::Point,
    sections:&[(&Region,TilePlacement)],max_cells:usize)
    ->Vec<(String,crate::streaming::ChunkKey)> {
    let mut ranked=Vec::new();
    for (region,placement) in sections {
        for key in crate::streaming::all_region_chunks(region) {
            let b=key.bounds();
            let p=placement.point(b[0] as f32,0.0,b[1] as f32);
            let q=placement.point(b[2] as f32,0.0,b[3] as f32);
            let (left,right)=(p[0].min(q[0]) as f64,p[0].max(q[0]) as f64);
            let (bottom,top)=(p[2].min(q[2]) as f64,p[2].max(q[2]) as f64);
            let dx=camera.x.clamp(left,right)-camera.x;
            let dz=camera.z.clamp(bottom,top)-camera.z;
            ranked.push((dx*dx+dz*dz,region.id.as_str(),key));
        }
    }
    ranked.sort_by(|a,b|a.0.total_cmp(&b.0)
        .then_with(||a.1.cmp(b.1))
        .then_with(||a.2.cmp(&b.2)));
    ranked.truncate(max_cells.clamp(1,crate::streaming::MAX_RESIDENT_CHUNKS));
    ranked.into_iter().map(|(_,id,key)|(id.to_owned(),key)).collect()
}

pub fn within(bounds:[f64;4],x:f64,z:f64)->bool {
    x>=bounds[0]&&x<=bounds[2]&&z>=bounds[1]&&z<=bounds[3]
}
pub fn active_sections<T>(items:&[T], max:usize)->&[T] {
    &items[..items.len().min(max.min(MAX_RESIDENT_SECTIONS))]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn region(id:&str, origin:[f64;2],bbox:[f64;4])->Region{
        Region{id:id.to_owned(),name:id.to_owned(),origin,bbox}
    }
    #[test]
    fn adjacent_boundaries_agree_in_one_world() {
        let west=region("west",[-118.25,34.05],[-118.258,34.045,-118.242,34.056]);
        let east=region("east",[-118.234,34.048],[-118.242,34.04,-118.226,34.056]);
        let a=TilePlacement::between(&west,95.0,&west,95.0);
        let b=TilePlacement::between(&west,95.0,&east,103.0);
        assert_eq!(a.point(0.0,0.0,0.0),[0.0,0.0,0.0]);
        let [_,_,east_edge,_]=a.bounds(&west);
        let [west_edge,_,_,_]=b.bounds(&east);
        assert!((east_edge-west_edge).abs()<0.05,
            "adjacent tile edge must not split due to different source origins");
        assert!((b.up_m-8.0).abs()<0.001);
    }
    #[test]
    fn three_regions_share_one_detail_budget_and_follow_camera(){
        let mid=region("mid",[-118.25,34.05],[-118.258,34.045,-118.242,34.056]);
        let west=region("west",[-118.266,34.05],[-118.274,34.045,-118.258,34.056]);
        let east=region("east",[-118.234,34.05],[-118.242,34.045,-118.226,34.056]);
        let sections=[(&mid,TilePlacement::between(&mid,90.0,&mid,90.0)),
                      (&west,TilePlacement::between(&mid,90.0,&west,90.0)),
                      (&east,TilePlacement::between(&mid,90.0,&east,90.0))];
        let downtown=prioritized_world_cells(crate::Point{x:0.0,z:0.0},&sections,6);
        assert_eq!(downtown.len(),6);
        assert!(downtown.iter().all(|(id,_)|id=="mid"));
        let camera_west=mid.project(-118.269,34.051);
        let in_west=prioritized_world_cells(camera_west,&sections,6);
        assert_eq!(in_west.len(),6);
        assert!(in_west.iter().all(|(id,_)|id=="west"));
        let camera_east=mid.project(-118.231,34.051);
        let in_east=prioritized_world_cells(camera_east,&sections,6);
        assert_eq!(in_east.len(),6);
        assert!(in_east.iter().all(|(id,_)|id=="east"));
        let around_seam=prioritized_world_cells(mid.project(-118.258,34.05),
            &sections,12);
        assert_eq!(around_seam.len(),12);
        assert!(around_seam.iter().any(|(id,_)|id=="mid"));
        assert!(around_seam.iter().any(|(id,_)|id=="west"));
        let repeat=prioritized_world_cells(mid.project(-118.258,34.05),
            &sections,12);
        assert_eq!(around_seam,repeat,"cell ordering must be deterministic");
    }
    #[test]
    fn world_budget_never_multiplies_by_number_of_sections(){
        let a=region("a",[-118.25,34.05],[-118.258,34.045,-118.242,34.056]);
        let origin=TilePlacement::between(&a,0.0,&a,0.0);
        let six=prioritized_world_cells(crate::Point{x:0.0,z:0.0},
            &[(&a,origin),(&a,origin),(&a,origin)],6);
        assert_eq!(six.len(),6);
        let empty=prioritized_world_cells(crate::Point{x:0.0,z:0.0},&[],6);
        assert!(empty.is_empty());
    }
    #[test]
    fn inverse_projection_and_budget_are_bounded(){
        let a=region("one",[-118.25,34.05],[-118.258,34.045,-118.242,34.056]);
        let b=region("two",[-118.234,34.048],[-118.242,34.04,-118.226,34.056]);
        let t=TilePlacement::between(&a,0.0,&b,4.0);
        let pos=b.project(-118.235,34.052);
        let w=t.point(pos.x as f32,2.0,pos.z as f32);
        let local=t.local(w[0] as f64,w[2] as f64);
        assert!((local[0]-pos.x).abs()<0.005);
        assert!((local[1]-pos.z).abs()<0.005);
        assert!((w[1]-6.0).abs()<0.0001);
        assert_eq!(active_sections(&[1,2,3,4],usize::MAX),&[1,2]);
        assert!(!within(t.bounds(&b),100_000.0,0.0));
    }
}
