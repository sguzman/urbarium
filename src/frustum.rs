//! Conservative draw-call frustum rejection for already resident 3D meshes.
//!
//! This NEVER changes chunk residency, geographic source data, collision or
//! geometry generation. Every candidate bound comes from actual mesh
//! vertices (not from a chunk's nominal street-grid extent). Ambiguous
//! or invalid bounds are always drawn.

/// Additional world-space padding beyond each clip plane.
pub const DRAW_MARGIN_M:f32=30.0;

#[derive(Clone,Copy,Debug)]
pub struct Bounds3 {
    pub min:[f32;3],
    pub max:[f32;3],
}
impl Bounds3 {
    pub fn from_positions(positions:impl IntoIterator<Item=[f32;3]>)->Option<Self>{
        let mut bounds:Option<Self>=None;
        for p in positions {
            if !p.iter().all(|n|n.is_finite()){return None;}
            if let Some(b)=&mut bounds{
                for axis in 0..3{
                    b.min[axis]=b.min[axis].min(p[axis]);
                    b.max[axis]=b.max[axis].max(p[axis]);
                }
            }else{bounds=Some(Self{min:p,max:p});}
        }
        bounds
    }
    fn valid(&self)->bool {
        (0..3).all(|i|self.min[i].is_finite()&&self.max[i].is_finite()
                       &&self.min[i]<=self.max[i])
    }
}

#[derive(Clone,Copy)]
pub struct ConservativeFrustum {
    planes:[[f32;4];6],
    valid:bool,
}
impl ConservativeFrustum{
    /// Left-handed WebGPU projection: -w <= x/y <= w, 0 <= z <= w.
    /// The input is a column-major homogeneous clip matrix.
    pub fn from_clip_cols(cols:[[f32;4];4])->Self {
        let row=|r:usize|[cols[0][r],cols[1][r],cols[2][r],cols[3][r]];
        let a=row(0);let b=row(1);let z=row(2);let w=row(3);
        let plus=|a:[f32;4],b:[f32;4]|std::array::from_fn(|i|a[i]+b[i]);
        let minus=|a:[f32;4],b:[f32;4]|std::array::from_fn(|i|a[i]-b[i]);
        let planes=[plus(w,a),minus(w,a),plus(w,b),minus(w,b),z,minus(w,z)];
        let valid=planes.iter().all(|p|{
            p.iter().all(|v|v.is_finite()) &&
            p[0].hypot(p[1]).hypot(p[2])>1e-8
        });
        Self{planes,valid}
    }
    /// Reject only if an *entire real mesh AABB* lies strictly outside a
    /// clip plane, plus 30m safety. Not depth/occlusion culling.
    pub fn may_draw(&self,bounds:Option<Bounds3>)->bool{
        let Some(b)=bounds else{return true};
        if !self.valid || !b.valid(){return true;}
        for p in self.planes{
            let furthest=[
                if p[0]>=0.0{b.max[0]}else{b.min[0]},
                if p[1]>=0.0{b.max[1]}else{b.min[1]},
                if p[2]>=0.0{b.max[2]}else{b.min[2]},
            ];
            let signed=p[0]*furthest[0]+p[1]*furthest[1]+
                p[2]*furthest[2]+p[3];
            let slack=DRAW_MARGIN_M*p[0].hypot(p[1]).hypot(p[2]);
            if signed.is_finite() && signed < -slack{return false;}
        }
        true
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    use glam::{Mat4,Vec3};
    fn frustum(at:Vec3,dir:Vec3)->ConservativeFrustum{
        let view=Mat4::look_to_lh(at,dir,Vec3::Y);
        let proj=Mat4::perspective_lh(68.0_f32.to_radians(),16.0/9.0,0.06,4500.0);
        ConservativeFrustum::from_clip_cols((proj*view).to_cols_array_2d())
    }
    fn bbox(a:[f32;3],b:[f32;3])->Option<Bounds3>{
        Bounds3::from_positions([a,b])
    }
    #[test]
    fn forward_visible_side_and_behind_conservatively_skipped(){
        let f=frustum(Vec3::new(0.0,1.7,0.0),Vec3::Z);
        assert!(f.may_draw(bbox([-30.0,0.0,80.0],[30.0,180.0,180.0])));
        assert!(f.may_draw(bbox([-3.0,0.0,-3.0],[3.0,20.0,3.0])));
        assert!(!f.may_draw(bbox([900.0,0.0,100.0],[930.0,250.0,160.0])));
        assert!(!f.may_draw(bbox([-930.0,0.0,100.0],[-900.0,250.0,160.0])));
        assert!(!f.may_draw(bbox([-15.0,0.0,-600.0],[15.0,50.0,-550.0])));
    }
    #[test]
    fn turning_changes_draw_calls_not_mesh_residency(){
        let east=frustum(Vec3::new(0.0,10.0,0.0),Vec3::X);
        let west=frustum(Vec3::new(0.0,10.0,0.0),-Vec3::X);
        let distant=bbox([500.0,0.0,-80.0],[560.0,200.0,80.0]);
        assert!(east.may_draw(distant));
        assert!(!west.may_draw(distant));
        let crossing=bbox([-50.0,-10.0,-50.0],[50.0,280.0,50.0]);
        assert!(east.may_draw(crossing)&&west.may_draw(crossing));
    }
    #[test]
    fn overview_keeps_tall_and_partial_edge_buildings(){
        let eye=Vec3::new(-400.0,180.0,-400.0);
        let target=Vec3::new(100.0,35.0,150.0);
        let f=frustum(eye,(target-eye).normalize());
        assert!(f.may_draw(bbox([60.0,0.0,90.0],[145.0,260.0,200.0])));
        assert!(f.may_draw(bbox([-75.0,0.0,-40.0],[45.0,550.0,30.0])));
    }
    #[test]
    fn malformed_or_unknown_bounds_are_never_rejected(){
        let f=frustum(Vec3::ZERO,Vec3::Z);
        assert!(f.may_draw(None));
        assert!(f.may_draw(Some(Bounds3{min:[f32::NAN,0.0,0.0],
                                        max:[1.0,1.0,1.0]})));
        let invalid=ConservativeFrustum::from_clip_cols([[0.0;4];4]);
        assert!(invalid.may_draw(bbox([1000.0,0.0,0.0],[1200.0,200.0,200.0])));
    }
}
