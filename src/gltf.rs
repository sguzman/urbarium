//! Minimal glTF 2.0 binary (.glb) exporter for Urbarium's indexed meshes.
//! All data is meters, with X east, Y up, Z north. Stable material groups
//! allow a WebGPU viewer to avoid parsing verbose Wavefront OBJ text.

use crate::geometry::{Mesh, Vertex};
use crate::roads::{RoadScene, SurfaceKind};
use crate::{City, Point};
use serde_json::{json, Value};
use std::io::{self, Write};

fn append_f32(data: &mut Vec<u8>, value: f32) { data.extend(value.to_le_bytes()); }
fn append_u32(data: &mut Vec<u8>, value: u32) { data.extend(value.to_le_bytes()); }
fn pad(data: &mut Vec<u8>, byte: u8) {
    while data.len()%4!=0 { data.push(byte); }
}
fn length(len: usize) -> io::Result<u32> {
    u32::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidData,"glTF buffer exceeds 4 GiB"))
}

#[derive(Clone, Copy)]
struct Part<'a> {
    name: &'static str,
    mesh: &'a Mesh,
    color: [f32;4],
}

fn ground(region: &crate::Region) -> Mesh {
    let lo=region.project(region.bbox[0],region.bbox[1]);
    let hi=region.project(region.bbox[2],region.bbox[3]);
    let y=-0.12;
    let points=[
        Point{x:lo.x,z:lo.z},Point{x:hi.x,z:lo.z},
        Point{x:lo.x,z:hi.z},Point{x:hi.x,z:hi.z},
    ];
    Mesh {
        vertices:points.iter().map(|p| Vertex {
            position:[p.x as f32,y,p.z as f32],
            normal:[0.0,1.0,0.0],
        }).collect(),
        indices:vec![0,2,1,1,2,3],
    }
}

fn add_view(views: &mut Vec<Value>, offset: usize, size: usize, target: u32) -> io::Result<usize> {
    let index=views.len();
    views.push(json!({
        "buffer":0, "byteOffset":length(offset)?, "byteLength":length(size)?,
        "target":target
    }));
    Ok(index)
}

/// Generates portable GLB for the city: a flat reference ground, buildings,
/// and each classified transportation layer in a separate glTF mesh/material.
pub fn write_glb(mut out: impl Write, city: &City, buildings: &Mesh, roads: &RoadScene) -> io::Result<()> {
    let terrain=ground(&city.region);
    let mut parts=vec![
        Part{name:"Reference ground (flat, not terrain)",mesh:&terrain,color:[0.40,0.44,0.41,1.0]},
        Part{name:"Building massing",mesh:buildings,color:[0.69,0.72,0.77,1.0]},
    ];
    for (surface, mesh) in &roads.layers {
        let (name,color)=match surface {
            SurfaceKind::Road=>("Roads (approximate width)",[0.22,0.24,0.27,1.0]),
            SurfaceKind::Walk=>("Pedestrian paths",[0.68,0.64,0.53,1.0]),
            SurfaceKind::Steps=>("Steps (flat approximation)",[0.78,0.63,0.43,1.0]),
            SurfaceKind::Rail=>("Surface rail",[0.36,0.46,0.52,1.0]),
        };
        parts.push(Part{name,mesh,color});
    }
    let mut buffer=Vec::<u8>::new();
    let mut views=Vec::<Value>::new();
    let mut accessors=Vec::<Value>::new();
    let mut nodes=Vec::<Value>::new();
    let mut meshes=Vec::<Value>::new();
    let mut materials=Vec::<Value>::new();
    for part in parts {
        if part.mesh.indices.is_empty() { continue; }
        let vertex_count=part.mesh.vertices.len();
        if vertex_count==0 { continue; }
        if part.mesh.indices.iter().any(|&i| i as usize>=vertex_count) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,"out of range mesh index"));
        }
        let mut min=[f32::INFINITY;3];
        let mut max=[f32::NEG_INFINITY;3];
        let offset=buffer.len();
        for vertex in &part.mesh.vertices {
            if !vertex.position.iter().chain(vertex.normal.iter()).all(|v| v.is_finite()) {
                return Err(io::Error::new(io::ErrorKind::InvalidData,"non-finite geometry"));
            }
            for (i,v) in vertex.position.iter().enumerate() {
                min[i]=min[i].min(*v);
                max[i]=max[i].max(*v);
                append_f32(&mut buffer,*v);
            }
        }
        let position_view=add_view(&mut views,offset,buffer.len()-offset,34962)?;
        let position_accessor=accessors.len();
        accessors.push(json!({
            "bufferView":position_view,"componentType":5126,"count":vertex_count,
            "type":"VEC3","min":min,"max":max
        }));
        let offset=buffer.len();
        for vertex in &part.mesh.vertices {
            for v in vertex.normal { append_f32(&mut buffer,v); }
        }
        let normal_view=add_view(&mut views,offset,buffer.len()-offset,34962)?;
        let normal_accessor=accessors.len();
        accessors.push(json!({
            "bufferView":normal_view,"componentType":5126,"count":vertex_count,"type":"VEC3"
        }));
        let offset=buffer.len();
        for &i in &part.mesh.indices { append_u32(&mut buffer,i); }
        let index_view=add_view(&mut views,offset,buffer.len()-offset,34963)?;
        let index_accessor=accessors.len();
        accessors.push(json!({
            "bufferView":index_view,"componentType":5125,
            "count":part.mesh.indices.len(),"type":"SCALAR"
        }));
        let material=materials.len();
        materials.push(json!({
            "name":part.name,
            "pbrMetallicRoughness":{
                "baseColorFactor":part.color,
                "metallicFactor":0.0,
                "roughnessFactor":1.0
            },
            "doubleSided":true
        }));
        let mesh_index=meshes.len();
        meshes.push(json!({
            "name":part.name,
            "primitives":[{
                "attributes":{"POSITION":position_accessor,"NORMAL":normal_accessor},
                "indices":index_accessor,"material":material,"mode":4
            }]
        }));
        nodes.push(json!({"name":part.name,"mesh":mesh_index}));
    }
    pad(&mut buffer,0);
    let buffer_length=length(buffer.len())?;
    let root=json!({
        "asset":{"version":"2.0","generator":"Urbarium Rust geometry exporter"},
        "scene":0,
        "scenes":[{"name":city.region.name,"nodes":(0..nodes.len()).collect::<Vec<usize>>()}],
        "nodes":nodes,
        "meshes":meshes,
        "materials":materials,
        "buffers":[{"byteLength":buffer_length}],
        "bufferViews":views,
        "accessors":accessors,
        "extras":{
            "region_id":city.region.id,
            "source_provenance":city.provenance,
            "coordinate_system":"x east, y up, z north; meters",
            "limitations":"flat reference ground; no street intersection or terrain heights"
        }
    });
    let mut json_bytes=serde_json::to_vec(&root).map_err(io::Error::other)?;
    pad(&mut json_bytes,b' ');
    let json_len=length(json_bytes.len())?;
    let total=12u64+8+u64::from(json_len)+8+u64::from(buffer_length);
    let total=u32::try_from(total).map_err(|_|io::Error::new(io::ErrorKind::InvalidData,"GLB exceeds 4 GiB"))?;
    out.write_all(&0x46546C67u32.to_le_bytes())?;
    out.write_all(&2u32.to_le_bytes())?;
    out.write_all(&total.to_le_bytes())?;
    out.write_all(&json_len.to_le_bytes())?;
    out.write_all(&0x4E4F534Au32.to_le_bytes())?;
    out.write_all(&json_bytes)?;
    out.write_all(&buffer_length.to_le_bytes())?;
    out.write_all(&0x004E4942u32.to_le_bytes())?;
    out.write_all(&buffer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Region,Provenance};
    #[test]
    fn glb_has_valid_offsets_lengths_and_materials() {
        let city=City {
            schema_version:1,
            region:Region{id:"test".into(),name:"Test".into(),origin:[-118.25,34.05],
                bbox:[-118.251,34.049,-118.249,34.051]},
            provenance:vec![Provenance{name:"Synthetic".into(),url:"https://example.invalid".into(),license:"synthetic".into()}],
            streets:vec![],buildings:vec![],
        };
        let b=Mesh {
            vertices:vec![
                Vertex{position:[0.,0.,0.],normal:[0.,1.,0.]},
                Vertex{position:[1.,0.,0.],normal:[0.,1.,0.]},
                Vertex{position:[0.,0.,1.],normal:[0.,1.,0.]},
            ],
            indices:vec![0,2,1],
        };
        let mut output=Vec::new();
        write_glb(&mut output,&city,&b,&RoadScene::default()).unwrap();
        let le=|a:usize|u32::from_le_bytes(output[a..a+4].try_into().unwrap()) as usize;
        assert_eq!(le(0),0x46546C67);
        assert_eq!(le(4),2);
        assert_eq!(le(8),output.len());
        assert_eq!(le(16),0x4E4F534A);
        let json_len=le(12);
        let meta:Value=serde_json::from_slice(&output[20..20+json_len]).unwrap();
        assert_eq!(meta["asset"]["version"],"2.0");
        assert_eq!(meta["meshes"].as_array().unwrap().len(),2);
        assert_eq!(meta["materials"].as_array().unwrap().len(),2);
        let start=20+json_len;
        assert_eq!(le(start+4),0x004E4942);
        let bin_len=le(start);
        assert_eq!(output.len(),start+8+bin_len);
        for view in meta["bufferViews"].as_array().unwrap() {
            let offset=view["byteOffset"].as_u64().unwrap() as usize;
            let size=view["byteLength"].as_u64().unwrap() as usize;
            assert_eq!(offset%4,0);
            assert!(offset+size<=bin_len);
        }
    }
    #[test]
    fn rejects_bad_indices() {
        let city=City{schema_version:1,
            region:Region{id:"test".into(),name:"Test".into(),origin:[0.,0.],bbox:[-1.,-1.,1.,1.]},
            provenance:vec![],streets:vec![],buildings:vec![]};
        let mesh=Mesh{vertices:vec![Vertex{position:[0.,0.,0.],normal:[0.,1.,0.]}],indices:vec![0,1,0]};
        assert!(write_glb(Vec::new(),&city,&mesh,&RoadScene::default()).is_err());
    }
}
