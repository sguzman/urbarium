//! Geographic bare-earth elevations from a bounded gridded DEM.
//! Heights are local meters relative to the WGS84 region origin sample,
//! not absolute sea-level altitude. Sampling uses the same equirectangular
//! local X/Z plane as city geometry. Row zero is the south edge.

use crate::geometry::{mesh_building, HeightPolicy, Mesh, MeshReport, Vertex};
use crate::roads::RoadScene;
use crate::{City, Region};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerrainGrid {
    pub schema_version: u32,
    pub region_id: String,
    pub bbox: [f64;4],
    pub width: usize,
    pub height: usize,
    pub heights_m: Vec<f32>,
    pub origin_elevation_m: f32,
    pub source: String,
}
impl TerrainGrid {
    pub fn validate(&self, region: &Region) -> Result<(), String> {
        if self.schema_version != 1 || self.region_id != region.id
            || self.width < 2 || self.height < 2
            || self.width > 1024 || self.height > 1024
            || self.width.checked_mul(self.height) != Some(self.heights_m.len())
            || !self.origin_elevation_m.is_finite()
            || self.heights_m.iter().any(|v| !v.is_finite())
            || self.bbox.iter().zip(region.bbox).any(|(a,b)| (a-b).abs()>1e-9)
        {
            return Err("terrain grid does not match a finite, bounded city region".into());
        }
        Ok(())
    }

    /// Bilinearly interpolated local height at region-local X/Z meters.
    pub fn sample(&self, region:&Region, x:f64, z:f64) -> f32 {
        let a=region.project(region.bbox[0],region.bbox[1]);
        let b=region.project(region.bbox[2],region.bbox[3]);
        let fx=((x-a.x)/(b.x-a.x)*(self.width-1) as f64)
            .clamp(0.0,(self.width-1) as f64);
        let fz=((z-a.z)/(b.z-a.z)*(self.height-1) as f64)
            .clamp(0.0,(self.height-1) as f64);
        let x0=fx.floor() as usize;
        let z0=fz.floor() as usize;
        let x1=(x0+1).min(self.width-1);
        let z1=(z0+1).min(self.height-1);
        let tx=(fx-x0 as f64) as f32;
        let tz=(fz-z0 as f64) as f32;
        let at=|col,row| self.heights_m[row*self.width+col];
        let low=at(x0,z0)*(1.0-tx)+at(x1,z0)*tx;
        let high=at(x0,z1)*(1.0-tx)+at(x1,z1)*tx;
        low*(1.0-tz)+high*tz
    }

    /// Full rectangular DEM mesh. Unlike the former flat quad, each grid
    /// vertex has a distinct elevation and derived slope normal.
    pub fn mesh(&self, region:&Region) -> Result<Mesh,String> {
        self.validate(region)?;
        let a=region.project(region.bbox[0],region.bbox[1]);
        let b=region.project(region.bbox[2],region.bbox[3]);
        let dx=((b.x-a.x)/(self.width-1) as f64) as f32;
        let dz=((b.z-a.z)/(self.height-1) as f64) as f32;
        let mut mesh=Mesh::default();
        for row in 0..self.height {
            for col in 0..self.width {
                let y=self.heights_m[row*self.width+col];
                let x=(a.x+(b.x-a.x)*col as f64/(self.width-1) as f64) as f32;
                let z=(a.z+(b.z-a.z)*row as f64/(self.height-1) as f64) as f32;
                let l=self.heights_m[row*self.width+col.saturating_sub(1)];
                let r=self.heights_m[row*self.width+(col+1).min(self.width-1)];
                let d=self.heights_m[row.saturating_sub(1)*self.width+col];
                let u=self.heights_m[(row+1).min(self.height-1)*self.width+col];
                let slope_x=(r-l)/if col==0||col+1==self.width {dx}else{2.0*dx};
                let slope_z=(u-d)/if row==0||row+1==self.height {dz}else{2.0*dz};
                let length=(slope_x*slope_x+slope_z*slope_z+1.0).sqrt();
                mesh.vertices.push(Vertex {
                    position:[x,y-0.12,z],
                    normal:[-slope_x/length,1.0/length,-slope_z/length]
                });
            }
        }
        for row in 0..self.height-1 {
            for col in 0..self.width-1 {
                let i=u32::try_from(row*self.width+col).map_err(|_|"terrain mesh too large")?;
                let stride=self.width as u32;
                // Clockwise XZ indices produce upward-facing triangles.
                mesh.indices.extend([i,i+stride,i+1,i+1,i+stride,i+stride+1]);
            }
        }
        Ok(mesh)
    }

    /// Road/path ribbons drape to local elevation; vertical/bridge semantics
    /// remain pending, so this is not yet a navigable terrain surface.
    pub fn drape_roads(&self, region:&Region, roads:&mut RoadScene) {
        for mesh in roads.layers.values_mut() {
            for vertex in &mut mesh.vertices {
                vertex.position[1]+=self.sample(
                    region,vertex.position[0] as f64,vertex.position[2] as f64);
            }
        }
    }

    /// Keep each building's foundation and roof horizontal by applying one
    /// representative terrain offset per footprint, not a per-vertex warp.
    pub fn mesh_buildings(&self, city:&City, policy:HeightPolicy)
        -> Result<(Mesh,MeshReport),String>
    {
        self.validate(&city.region)?;
        policy.validate()?;
        let mut combined=Mesh::default();
        let mut report=MeshReport::default();
        for building in &city.buildings {
            let Ok(mut mesh)=mesh_building(building,policy) else {
                report.buildings_skipped+=1;
                continue;
            };
            let Some(exterior)=building.polygons.first()
                .and_then(|p|p.rings.first()) else {
                report.buildings_skipped+=1;continue;
            };
            if exterior.is_empty(){report.buildings_skipped+=1;continue;}
            let x=exterior.iter().map(|p|p.x).sum::<f64>()/exterior.len() as f64;
            let z=exterior.iter().map(|p|p.z).sum::<f64>()/exterior.len() as f64;
            let ground=self.sample(&city.region,x,z);
            for vertex in &mut mesh.vertices {vertex.position[1]+=ground;}
            let offset=u32::try_from(combined.vertices.len())
                .map_err(|_|"too many building vertices")?;
            if mesh.indices.iter().any(|i|i.checked_add(offset).is_none()) {
                return Err("building index overflow".into());
            }
            combined.indices.extend(mesh.indices.into_iter().map(|i|i+offset));
            combined.vertices.extend(mesh.vertices);
            report.buildings_meshed+=1;
            if building.height_m.is_none() {
                report.buildings_with_approximate_heights+=1;
            }
        }
        report.vertices=combined.vertices.len();
        report.triangles=combined.indices.len()/3;
        Ok((combined,report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Building,Point,Polygon};
    fn region()->Region {
        Region{id:"test".into(),name:"Test".into(),
            origin:[0.5,0.5],bbox:[0.0,0.0,1.0,1.0]}
    }
    fn grid()->TerrainGrid {
        TerrainGrid {schema_version:1,region_id:"test".into(),
            bbox:[0.0,0.0,1.0,1.0],width:2,height:2,
            heights_m:vec![0.0,10.0,20.0,30.0],
            origin_elevation_m:90.0,source:"synthetic".into()}
    }
    #[test]
    fn bilinear_south_to_north_and_east_to_west_sampling() {
        let g=grid();let r=region();
        assert_eq!(g.sample(&r,0.0,0.0),15.0);
        let west=r.project(0.0,0.0);let east=r.project(1.0,1.0);
        assert_eq!(g.sample(&r,west.x,west.z),0.0);
        assert_eq!(g.sample(&r,east.x,east.z),30.0);
    }
    #[test]
    fn terrain_triangle_counts_and_finite_normals() {
        let mesh=grid().mesh(&region()).unwrap();
        assert_eq!(mesh.vertices.len(),4);
        assert_eq!(mesh.indices.len(),6);
        assert!(mesh.vertices.iter().all(|v|v.normal[1]>0.0));
    }
    #[test]
    fn mismatched_region_or_corrupt_grid_is_rejected() {
        let mut g=grid();g.region_id="other".into();
        assert!(g.validate(&region()).is_err());
        g=grid();g.heights_m.pop();
        assert!(g.validate(&region()).is_err());
    }
    #[test]
    fn building_roof_keeps_one_foundation_elevation() {
        let r=region();
        let city=City {
            schema_version:1,region:r.clone(),provenance:vec![],streets:vec![],
            buildings:vec![Building {
                source_id:"synthetic".into(),name:None,height_m:Some(12.0),levels:None,
                polygons:vec![Polygon { rings:vec![vec![
                    Point{x:-100.0,z:-100.0},Point{x:100.0,z:-100.0},
                    Point{x:100.0,z:100.0},Point{x:-100.0,z:100.0}
                ]]}],
            }]
        };
        let (mesh,report)=grid().mesh_buildings(&city,HeightPolicy::default()).unwrap();
        assert_eq!(report.buildings_meshed,1);
        let roof_y=mesh.vertices[0].position[1];
        assert!(mesh.vertices.iter().take(4).all(|v|(v.position[1]-roof_y).abs()<0.001));
    }
}
