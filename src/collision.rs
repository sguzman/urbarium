//! Planar building collision from geographic footprint polygons.
//! Intentional first walking milestone: flat ground at y=0, circle-shaped
//! pedestrian clearance, approximate roof heights, courtyard holes preserved.
//! This is *not* yet full 3D physics or elevation-aware pavement collision.

use crate::{City, Point};

#[derive(Debug)]
struct Obstacle {
    rings: Vec<Vec<Point>>,
    bounds: [f64; 4],
    height_m: f64,
}

/// Precomputed footprint obstacles, built once after loading a city slice.
#[derive(Debug, Default)]
pub struct CollisionWorld {
    obstacles: Vec<Obstacle>,
    bounds: [f64; 4],
}

impl CollisionWorld {
    pub fn from_city(city: &City) -> Self {
        let lower = city.region.project(city.region.bbox[0], city.region.bbox[1]);
        let upper = city.region.project(city.region.bbox[2], city.region.bbox[3]);
        let mut world = Self {
            obstacles: Vec::new(),
            bounds: [lower.x,lower.z,upper.x,upper.z],
        };
        for building in &city.buildings {
            let height = building.height_m
                .or_else(|| building.levels.map(|levels| levels*3.1))
                .unwrap_or(9.0);
            if !height.is_finite() || height <= 0.0 { continue; }
            for polygon in &building.polygons {
                if polygon.rings.is_empty() || polygon.rings[0].len()<3 { continue; }
                let mut bbox = [f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
                if polygon.rings.iter().flatten().any(|p| !p.x.is_finite() || !p.z.is_finite()) {
                    continue;
                }
                for p in &polygon.rings[0] {
                    bbox[0]=bbox[0].min(p.x);
                    bbox[1]=bbox[1].min(p.z);
                    bbox[2]=bbox[2].max(p.x);
                    bbox[3]=bbox[3].max(p.z);
                }
                world.obstacles.push(Obstacle {
                    rings:polygon.rings.clone(), bounds:bbox, height_m:height
                });
            }
        }
        world
    }

    pub fn obstacle_count(&self) -> usize {self.obstacles.len()}

    /// Tests the player's horizontal circle at eye elevation. Heights above
    /// the building roof are not blocked, retaining aerial free-flight.
    pub fn blocked(&self, x: f64, z: f64, eye_y: f64, radius: f64) -> bool {
        if !x.is_finite() || !z.is_finite() || !eye_y.is_finite()
            || !radius.is_finite() || radius < 0.0 { return true; }
        let [xmin,zmin,xmax,zmax]=self.bounds;
        if x-radius < xmin || z-radius < zmin || x+radius > xmax || z+radius > zmax {
            return true;
        }
        self.obstacles.iter().any(|building| {
            if eye_y > building.height_m + 0.3 {return false;}
            let [a,b,c,d]=building.bounds;
            if x+radius < a || x-radius > c || z+radius < b || z-radius > d {
                return false;
            }
            let p=Point{x,z};
            let inside_outer=inside_ring(p,&building.rings[0]);
            let inside_hole=building.rings.iter().skip(1).any(|ring| inside_ring(p,ring));
            if inside_outer && !inside_hole {return true;}
            // Circles must also be kept clear of exterior and courtyard walls.
            building.rings.iter().any(|ring| ring_distance_sq(p,ring) < radius*radius)
        })
    }

    /// Substep long movement to avoid tunneling through narrow buildings;
    /// on contact try two cardinal components separately to slide along walls.
    pub fn move_sliding(&self, start: [f64;2], end: [f64;2], eye_y: f64, radius: f64) -> [f64;2] {
        let dx=end[0]-start[0];
        let dz=end[1]-start[1];
        if !dx.is_finite() || !dz.is_finite() {return start;}
        let length=dx.hypot(dz);
        if length < 1e-10 {return start;}
        let steps=(length/0.25).ceil().clamp(1.0,128.0) as usize;
        let mut pos=start;
        for _ in 0..steps {
            let sx=dx/steps as f64;
            let sz=dz/steps as f64;
            let candidate=[pos[0]+sx,pos[1]+sz];
            if !self.blocked(candidate[0],candidate[1],eye_y,radius) {
                pos=candidate;
            } else if sx.abs()>sz.abs() {
                if !self.blocked(pos[0]+sx,pos[1],eye_y,radius) {pos[0]+=sx;}
                if !self.blocked(pos[0],pos[1]+sz,eye_y,radius) {pos[1]+=sz;}
            } else {
                if !self.blocked(pos[0],pos[1]+sz,eye_y,radius) {pos[1]+=sz;}
                if !self.blocked(pos[0]+sx,pos[1],eye_y,radius) {pos[0]+=sx;}
            }
        }
        pos
    }
}

fn inside_ring(p: Point, ring: &[Point]) -> bool {
    if ring.len()<3 {return false;}
    let mut inside=false;
    let mut previous=*ring.last().unwrap();
    for &current in ring {
        if (current.z>p.z)!=(previous.z>p.z) {
            let crossing=(previous.x-current.x)*(p.z-current.z)/
                (previous.z-current.z)+current.x;
            if p.x<crossing {inside=!inside;}
        }
        previous=current;
    }
    inside
}

fn ring_distance_sq(p: Point, ring: &[Point]) -> f64 {
    if ring.len()<2 {return f64::INFINITY;}
    let mut minimum=f64::INFINITY;
    for i in 0..ring.len() {
        let a=ring[i];
        let b=ring[(i+1)%ring.len()];
        let dx=b.x-a.x;
        let dz=b.z-a.z;
        let length=dx*dx+dz*dz;
        if length<1e-12 {continue;}
        let t=(((p.x-a.x)*dx+(p.z-a.z)*dz)/length).clamp(0.0,1.0);
        let ex=p.x-(a.x+t*dx);
        let ez=p.z-(a.z+t*dz);
        minimum=minimum.min(ex*ex+ez*ez);
    }
    minimum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Building, Polygon, Region};
    fn world() -> CollisionWorld {
        let outer=vec![
            Point{x:0.0,z:0.0}, Point{x:10.0,z:0.0},
            Point{x:10.0,z:10.0}, Point{x:0.0,z:10.0},
        ];
        let hole=vec![
            Point{x:3.0,z:3.0}, Point{x:7.0,z:3.0},
            Point{x:7.0,z:7.0}, Point{x:3.0,z:7.0},
        ];
        let region=Region{id:"test".into(),name:"test".into(),
            origin:[0.0,0.0],bbox:[-0.01,-0.01,0.01,0.01]};
        CollisionWorld::from_city(&City {
            schema_version:1,region,provenance:vec![],streets:vec![],
            buildings:vec![Building {
                source_id:"b".into(),name:None,height_m:Some(15.0),levels:None,
                polygons:vec![Polygon {rings:vec![outer,hole]}]
            }],
        })
    }
    #[test]
    fn solid_walls_and_courtyard_hole() {
        let w=world();
        assert_eq!(w.obstacle_count(),1);
        assert!(w.blocked(2.0,2.0,1.7,0.35));
        assert!(!w.blocked(5.0,5.0,1.7,0.35));
        assert!(w.blocked(3.2,5.0,1.7,0.35)); // courtyard wall
        assert!(!w.blocked(-2.0,5.0,1.7,0.35));
        assert!(!w.blocked(5.0,1.0,20.0,0.35)); // fly above roof
    }
    #[test]
    fn swept_movement_does_not_tunnel_through_walls() {
        let w=world();
        let end=w.move_sliding([-5.0,5.0],[15.0,5.0],1.7,0.35);
        assert!(end[0] < -0.1,"must not jump through the building: {end:?}");
        assert!(!w.blocked(end[0],end[1],1.7,0.35));
    }
    #[test]
    fn movement_slides_along_footprint_edges() {
        let w=world();
        let end=w.move_sliding([-2.0,1.0],[2.0,-3.0],1.7,0.35);
        assert!(end[1] < -1.0,"sliding should permit tangential travel: {end:?}");
        assert!(!w.blocked(end[0],end[1],1.7,0.35));
    }
    #[test]
    fn prevents_exiting_tile_and_invalid_positions() {
        let w=world();
        assert!(w.blocked(f64::NAN,1.0,1.7,0.35));
        assert!(w.blocked(1e8,0.0,1.7,0.35));
    }
}
