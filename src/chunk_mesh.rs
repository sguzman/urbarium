//! Reusable CPU-only renderer mesh assembly. Both the on-page viewer and
//! background WASM worker consume these same Rust functions: no geometry fork.

use crate::{City, Point, Region, Building};
use crate::streaming::ChunkKey;
use crate::surface_clip::clip_surface_to_bounds;
use crate::geometry::{mesh_building, HeightPolicy, Mesh, Vertex};
use crate::roads::{mesh_streets_with_bridges, SurfaceKind};
use crate::terrain::TerrainGrid;
use bytemuck::{Pod, Zeroable};
use serde::{Serialize, Deserialize};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Serialize, Deserialize)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub category_color: [f32; 3],
    pub category_id: f32,
}

pub fn append_part(
    mesh: &Mesh, color: [f32;3], verts: &mut Vec<GpuVertex>, indices: &mut Vec<u32>
) -> Result<(), String> {
    let base = u32::try_from(verts.len()).map_err(|_| "too many city vertices")?;
    if mesh.indices.iter().any(|idx| *idx as usize >= mesh.vertices.len() ||
        idx.checked_add(base).is_none()) {
        return Err("invalid source geometry index".into());
    }
    verts.extend(mesh.vertices.iter().map(|v| GpuVertex {
        position: v.position, normal: v.normal, color,
        category_color:[0.0;3], category_id:0.0,
    }));
    indices.extend(mesh.indices.iter().map(|idx| idx+base));
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneLayer { Ground, Buildings, Road, ElevatedRoad, BridgeStructure, Walk, Steps, Rail, Marker(u32) }
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DrawBatch {
    pub layer: SceneLayer,
    pub first_index: u32,
    pub count: u32,
}
#[derive(serde::Deserialize)]
pub struct LandmarkCatalog {
    pub region_id:String,
    pub landmarks:Vec<LandmarkAnchor>,
}
#[derive(serde::Deserialize)]
pub struct LandmarkAnchor {
    pub kind:String,
    pub coordinates:[f64;2],
}
// IDs deliberately match the category palette insertion order in index.html.
pub fn category_index(kind:&str)->Option<u32> {
    match kind {
        "civic"=>Some(0),"legal"=>Some(1),"library"=>Some(2),
        "culture"=>Some(3),"park"=>Some(4),"railway"=>Some(5),
        "market"=>Some(6),"historic"=>Some(7),_=>None,
    }
}
pub fn category_rgb(kind:&str)->Option<[f32;3]> {
    match kind {
        "civic"=>Some([0.933,0.663,0.420]),
        "legal"=>Some([0.467,0.827,0.812]),
        "library"=>Some([0.506,0.749,1.0]),
        "culture"=>Some([0.804,0.604,0.965]),
        "park"=>Some([0.506,0.851,0.612]),
        "railway"=>Some([0.961,0.773,0.408]),
        "market"=>Some([0.949,0.553,0.529]),
        "historic"=>Some([0.761,0.698,0.647]),
        _=>None,
    }
}
fn inside_ring(p:Point,ring:&[Point])->bool {
    if ring.len()<3 {return false;}
    let mut inside=false;
    for i in 0..ring.len() {
        let a=ring[i];
        let b=ring[(i+1)%ring.len()];
        if (a.z>p.z)!=(b.z>p.z) &&
            p.x < (b.x-a.x)*(p.z-a.z)/(b.z-a.z)+a.x {
            inside=!inside;
        }
    }
    inside
}
pub fn building_landmark_color(building:&Building,region:&Region, marks:&[LandmarkAnchor])
    -> Option<([f32;3],u32)>
{
    for mark in marks {
        let anchor=region.project(mark.coordinates[0],mark.coordinates[1]);
        for polygon in &building.polygons {
            if let Some(outer)=polygon.rings.first() {
                if inside_ring(anchor,outer) &&
                    !polygon.rings.iter().skip(1).any(|hole|inside_ring(anchor,hole)) {
                    return category_rgb(&mark.kind)
                        .zip(category_index(&mark.kind));
                }
            }
        }
    }
    None
}
pub fn landmark_disc(terrain:&TerrainGrid,region:&Region,p:Point,radius:f64)->Mesh {
    // Small representative ground marker. NOT a surveyed park boundary.
    let mut mesh=Mesh::default();
    let center_y=terrain.sample_surface(region,p.x,p.z)+0.6;
    mesh.vertices.push(Vertex{
        position:[p.x as f32,center_y,p.z as f32],normal:[0.0,1.0,0.0]});
    for i in 0..16 {
        let angle=i as f64*std::f64::consts::TAU/16.0;
        let x=p.x+radius*angle.cos();
        let z=p.z+radius*angle.sin();
        let y=terrain.sample_surface(region,x,z)+0.6;
        mesh.vertices.push(Vertex{position:[x as f32,y,z as f32],
            normal:[0.0,1.0,0.0]});
    }
    for i in 0..16 {
        mesh.indices.extend([0,1+i as u32,1+(i as u32+1)%16]);
    }
    mesh
}

// A cell contributes buildings, classified street fragments and source-backed
// landmark discs, but NOT another full-region terrain mesh. Each building has
// one canonical owner cell; complete road surfaces are clipped to cells.
pub fn chunk_geometry(city:&City,terrain:&TerrainGrid,
    key:ChunkKey)
    ->Result<(Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>),String>{
    let (buildings,report)=terrain.mesh_buildings(city,HeightPolicy::default())?;
    if report.buildings_skipped>0 {
        return Err(format!("{} chunk buildings could not be meshed",report.buildings_skipped));
    }
    let (mut roads,_)=mesh_streets_with_bridges(city,false)?;
    // First flatten the source-qualified bridge spans exactly once. The
    // ordinary viewer must not resurrect hidden freeway ribbons.
    crate::road_grade::flatten_bridge_spans(&mut roads)?;

    // Preserve full-path bend normals, actual intersections and layer widths
    // through planar meshing. Clip only the FINISHED horizontal polygons,
    // before expensive DEM-face splitting. This removes 80m halo triangles
    // from the terrain pass instead of rebuilding them per render chunk.
    // Region bounds also prevent bleed into the joined neighboring region.
    let sw=city.region.project(city.region.bbox[0],city.region.bbox[1]);
    let ne=city.region.project(city.region.bbox[2],city.region.bbox[3]);
    let tile=key.bounds();
    let bounds=[
        tile[0].max(sw.x),tile[1].max(sw.z),
        tile[2].min(ne.x),tile[3].min(ne.z),
    ];
    if bounds[0]>=bounds[2] || bounds[1]>=bounds[3] {
        return Err("chunk is outside its source region".into());
    }
    for mesh in roads.layers.values_mut() {
        *mesh=clip_surface_to_bounds(mesh,bounds)?;
    }
    terrain.conform_prepared_surface_layers(&city.region,&mut roads)?;
    let mut vertices=Vec::new();
    let mut indices=Vec::new();
    let mut batches=Vec::new();
    let mut add=|mesh:&Mesh,color:[f32;3],layer:SceneLayer|->Result<(),String>{
        let start=u32::try_from(indices.len()).map_err(|_|"chunk index count overflow")?;
        append_part(mesh,color,&mut vertices,&mut indices)?;
        let end=u32::try_from(indices.len()).map_err(|_|"chunk index count overflow")?;
        if end>start {batches.push(DrawBatch{layer,first_index:start,count:end-start});}
        Ok(())
    };
    add(&buildings,[0.71,0.76,0.81],SceneLayer::Buildings)?;
    for (kind,mesh) in roads.layers {
        let (layer,color)=match kind {
            SurfaceKind::Road=>(SceneLayer::Road,[0.24,0.26,0.29]),
            SurfaceKind::Motorway=>(SceneLayer::Road,[0.14,0.19,0.23]),
            SurfaceKind::MotorwayStripe=>(SceneLayer::Road,[0.91,0.89,0.77]),
            SurfaceKind::Walk=>(SceneLayer::Walk,[0.60,0.58,0.51]),
            SurfaceKind::Steps=>(SceneLayer::Steps,[0.74,0.62,0.46]),
            SurfaceKind::Rail=>(SceneLayer::Rail,[0.40,0.52,0.61]),
            SurfaceKind::ElevatedRoad=>(SceneLayer::ElevatedRoad,[0.26,0.29,0.32]),
            SurfaceKind::BridgeFascia=>(SceneLayer::BridgeStructure,[0.48,0.51,0.52]),
            SurfaceKind::BridgeSupports=>(SceneLayer::BridgeStructure,[0.61,0.63,0.62]),
            SurfaceKind::BridgeRails=>(SceneLayer::BridgeStructure,[0.76,0.78,0.78]),
            SurfaceKind::BridgeStripe=>(SceneLayer::ElevatedRoad,[0.91,0.89,0.77]),
        };
        add(&mesh,color,layer)?;
    }
    let mut landmarks:LandmarkCatalog=
        serde_json::from_str(include_str!("../web/landmarks.json"))
            .map_err(|err|format!("bundled landmarks: {err}"))?;
    if landmarks.region_id!=city.region.id{landmarks.landmarks.clear();}
    // Marker ownership is canonical by source position, independently
    // of the building that might contain the landmark.
    for landmark in &landmarks.landmarks {
        let [lon,lat]=landmark.coordinates;
        let p=city.region.project(lon,lat);
        if ChunkKey::at(p)!=key{continue;}
        if let Some(rgb)=category_rgb(&landmark.kind) {
            if let Some(category)=category_index(&landmark.kind){
                let disc=landmark_disc(terrain,&city.region,p,
                    if landmark.kind=="park"{15.0}else{6.0});
                add(&disc,rgb,SceneLayer::Marker(category))?;
            }
        }
    }
    // Category tint is applied to each complete, original building mesh;
    // no polygon is cut at cell edges.
    let mut cursor=0usize;
    for building in &city.buildings {
        let Ok(building_mesh)=mesh_building(building,HeightPolicy::default()) else{continue};
        let end=cursor+building_mesh.vertices.len();
        if end>buildings.vertices.len(){return Err("chunk building vertex ordering mismatch".into());}
        if let Some((rgb,id))=building_landmark_color(
            building,&city.region,&landmarks.landmarks){
            for vertex in &mut vertices[cursor..end]{
                vertex.category_color=rgb;
                vertex.category_id=(id+1) as f32;
            }
        }
        cursor=end;
    }
    if cursor!=buildings.vertices.len(){return Err("chunk building count mismatch".into());}
    Ok((vertices,indices,batches))
}


#[derive(Serialize,Deserialize)]
pub struct ChunkMeshPayload {
    pub vertices:Vec<GpuVertex>,
    pub indices:Vec<u32>,
    pub batches:Vec<DrawBatch>,
}
pub fn build_chunk_payload(source:&City,terrain:&TerrainGrid,key:ChunkKey)
    ->Result<ChunkMeshPayload,String>{
    let selected=crate::streaming::chunk_city(source,key);
    let (vertices,indices,batches)=chunk_geometry(&selected,terrain,key)?;
    Ok(ChunkMeshPayload{vertices,indices,batches})
}
