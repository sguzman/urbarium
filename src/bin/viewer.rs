//! First WASM/WebGPU city viewer: actual procedural Urbarium meshes, no JS geometry.
//! FPS pointer lock, WASD walk, E/Q fly, Shift accelerates.

use std::collections::HashSet;
use std::mem::size_of;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use web_time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use urbarium::geometry::{mesh_building, HeightPolicy, Mesh, Vertex};
use urbarium::roads::{mesh_streets_with_bridges, SurfaceKind};
use urbarium::{Building, City, Point};
use urbarium::collision::CollisionWorld;
use urbarium::terrain::TerrainGrid;
use urbarium::Region;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

#[cfg(target_arch = "wasm32")]
type SharedWindow = std::rc::Rc<Window>;
#[cfg(not(target_arch = "wasm32"))]
type SharedWindow = std::sync::Arc<Window>;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    category_color: [f32; 3],
    category_id: f32,
}
const VERTEX_ATTRS: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3,
                             2 => Float32x3, 3 => Float32x3, 4 => Float32];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    // x: category tint enabled, y: hour, z: yaw, w: pitch
    options: [f32; 4],
    // x: canvas aspect ratio
    sky: [f32; 4],
}

#[derive(Clone, Debug)]
struct Camera {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    speed: f32,
    eye_clearance: f32,
    fast_mode: bool,
}
impl Default for Camera {
    fn default() -> Self {
        Self { position: Vec3::new(0.0, 1.7, 0.0),
            yaw: std::f32::consts::PI, pitch: 0.0, speed: 3.4, eye_clearance:1.7, fast_mode:false }
    }
}
impl Camera {
    /// Select a real roadway centerline nearest Downtown LA's reference point.
    /// Do not start the viewer 95 meters over the city or inside an arbitrary
    /// building mass. This is a flat-ground, non-collision-aware preview.
    fn spawn_at_street(city:&City, world:&CollisionWorld, terrain:&TerrainGrid) -> Self {
        let mut camera=Self::default();
        let mut closest=f64::INFINITY;
        for street in &city.streets {
            if !matches!(street.kind.as_str(),
                "residential"|"living_street"|"service"|"tertiary"|
                "secondary"|"primary"|"unclassified"|"pedestrian"
            ) {continue;}
            for pair in street.path.windows(2) {
                let dx=pair[1].x-pair[0].x;
                let dz=pair[1].z-pair[0].z;
                let distance=dx.hypot(dz);
                if !(6.0..1000.0).contains(&distance) {continue;}
                let x=(pair[0].x+pair[1].x)*0.5;
                let z=(pair[0].z+pair[1].z)*0.5;
                let score=x*x+z*z;
                let eye=terrain.sample_surface(&city.region,x,z) as f64 + 1.7;
                if score<closest && !world.blocked(x,z,eye,0.35) {
                    closest=score;
                    camera.position=Vec3::new(x as f32,eye as f32,z as f32);
                    camera.yaw=(dx as f32).atan2(dz as f32);
                }
            }
        }
        camera
    }

    /// Scenic camera inside the real pilot bounds, looking towards its center.
    /// It changes only the view, not source geometry or road topology.
    fn city_overview(city:&City,world:&CollisionWorld,terrain:&TerrainGrid)->Self {
        let [west,south,east,north]=city.region.bbox;
        let sw=city.region.project(west,south);
        let ne=city.region.project(east,north);
        let center=Point{x:(sw.x+ne.x)*0.5,z:(sw.z+ne.z)*0.5};
        let target_y=terrain.sample_surface(&city.region,center.x,center.z)+35.0;
        let width=(ne.x-sw.x).abs();
        let depth=(ne.z-sw.z).abs();
        for (ox,oz) in [(-0.30,-0.31),(0.30,-0.31),(-0.30,0.31),(0.30,0.31)]{
            let x=center.x+width*ox;
            let z=center.z+depth*oz;
            let clearance=155.0_f32;
            let eye=terrain.sample_surface(&city.region,x,z)+clearance;
            if world.blocked(x,z,eye as f64,0.35){continue;}
            let delta_x=(center.x-x) as f32;
            let delta_z=(center.z-z) as f32;
            let horizontal=delta_x.hypot(delta_z);
            return Self {
                position:Vec3::new(x as f32,eye,z as f32),
                yaw:delta_x.atan2(delta_z),
                pitch:((target_y-eye)/horizontal.max(0.1)).atan(),
                speed:3.4,eye_clearance:clearance,fast_mode:false,
            };
        }
        let mut camera=Self::spawn_at_street(city,world,terrain);
        camera.position.y+=155.0;
        camera.eye_clearance+=155.0;
        camera.pitch=-0.25;
        camera
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos())
    }
    fn view_projection(&self, width: u32, height: u32) -> Mat4 {
        let aspect = width as f32 / height.max(1) as f32;
        // Geographic axes are X=east, Y=up, Z=north: a left-handed frame.
        // Use LH camera matrices so east is screen-right when facing north.
        let view = Mat4::look_to_lh(self.position, self.forward(), Vec3::Y);
        let projection = Mat4::perspective_lh(68.0_f32.to_radians(), aspect, 0.06, 4500.0);
        projection * view
    }
    fn mouse_look(&mut self, dx: f32, dy: f32) {
        // Clockwise from geographic north is positive: right mouse = eastward turn.
        self.yaw += dx * 0.0025;
        self.pitch = (self.pitch - dy * 0.0025).clamp(-1.52, 1.52);
    }
    // Control interpretation must be IDENTICAL in a single section and in
    // the joined world. Only world-space collision / ground sampling differ.
    fn control_displacement(&mut self,pressed:&HashSet<KeyCode>,elapsed:f32)->Vec3 {
        let ground_forward = Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos());
        let right = Vec3::new(ground_forward.z, 0.0, -ground_forward.x);
        let mut delta = Vec3::ZERO;
        if pressed.contains(&KeyCode::KeyW) { delta += ground_forward; }
        if pressed.contains(&KeyCode::KeyS) { delta -= ground_forward; }
        if pressed.contains(&KeyCode::KeyD) { delta += right; }
        if pressed.contains(&KeyCode::KeyA) { delta -= right; }
        if pressed.contains(&KeyCode::KeyE) || pressed.contains(&KeyCode::Space) {
            delta += Vec3::Y;
        }
        if pressed.contains(&KeyCode::KeyQ) || pressed.contains(&KeyCode::ControlLeft) {
            delta -= Vec3::Y;
        }
        if pressed.contains(&KeyCode::ArrowLeft) { self.mouse_look(-130.0*elapsed, 0.0); }
        if pressed.contains(&KeyCode::ArrowRight) { self.mouse_look(130.0*elapsed, 0.0); }
        if pressed.contains(&KeyCode::ArrowUp) { self.mouse_look(0.0, -130.0*elapsed); }
        if pressed.contains(&KeyCode::ArrowDown) { self.mouse_look(0.0, 130.0*elapsed); }
        if delta.length_squared() > 0.0 {
            let boost = if self.fast_mode { 18.0 } else { 1.0 };
            delta.normalize() * elapsed.min(0.08) * self.speed * boost
        }else{Vec3::ZERO}
    }
    fn move_by(&mut self, pressed:&HashSet<KeyCode>, elapsed:f32,
        world:Option<&CollisionWorld>, terrain:Option<(&TerrainGrid,&Region)>) {
        let movement=self.control_displacement(pressed,elapsed);
        if movement.length_squared()>0.0 {
            let start=[self.position.x as f64,self.position.z as f64];
            let end=[start[0]+movement.x as f64,start[1]+movement.z as f64];
            let ground_at=|x:f64,z:f64| -> f32 {
                terrain.map(|(grid,region)| grid.sample_surface(region,x,z)).unwrap_or(0.0)
            };
            // Walking follows the DEM at the standard 1.7 m eye clearance.
            // Once E/Space raises the camera, horizontal motion becomes free
            // flight at a fixed world-space altitude, even over sloping DEM.
            // Q/Ctrl lowers it again; reaching the ground resumes walking.
            // This also fixes city-overview flight turning into a rollercoaster
            // over terrain ridges and valleys.
            let flying=self.eye_clearance>1.701 || movement.y>0.0;
            let intended_y=if flying {
                self.position.y+movement.y
            }else{
                ground_at(end[0],end[1])+1.7
            };
            let candidate_eye=intended_y.max(ground_at(end[0],end[1])+1.7);
            let resolved=if let Some(world)=world {
                world.move_sliding(start,end,candidate_eye as f64,0.35)
            }else{end};
            self.position.x=resolved[0] as f32;
            self.position.z=resolved[1] as f32;
            // Safety clearance is a FLOOR, not a directive to follow every
            // downhill grade while flying. Uphill terrain can lift the camera
            // to avoid underground motion; descending requires Q/Ctrl.
            let ground=ground_at(resolved[0],resolved[1]);
            self.position.y=intended_y.max(ground+1.7);
            self.eye_clearance=self.position.y-ground;
        }
    }
}

fn append_part(
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneLayer { Ground, Buildings, Road, ElevatedRoad, BridgeStructure, Walk, Steps, Rail, Marker(u32) }
#[derive(Clone, Copy)]
struct DrawBatch {
    layer: SceneLayer,
    first_index: u32,
    count: u32,
}
#[derive(serde::Deserialize)]
struct LandmarkCatalog {
    region_id:String,
    landmarks:Vec<LandmarkAnchor>,
}
#[derive(serde::Deserialize)]
struct LandmarkAnchor {
    kind:String,
    coordinates:[f64;2],
}
// IDs deliberately match the category palette insertion order in index.html.
fn category_index(kind:&str)->Option<u32> {
    match kind {
        "civic"=>Some(0),"legal"=>Some(1),"library"=>Some(2),
        "culture"=>Some(3),"park"=>Some(4),"railway"=>Some(5),
        "market"=>Some(6),"historic"=>Some(7),_=>None,
    }
}
fn category_rgb(kind:&str)->Option<[f32;3]> {
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
fn building_landmark_color(building:&Building,region:&Region, marks:&[LandmarkAnchor])
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
fn landmark_disc(terrain:&TerrainGrid,region:&Region,p:Point,radius:f64)->Mesh {
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
fn combined_city(city:&City,terrain:&TerrainGrid,bridge_preview:bool)
    ->Result<(Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>),String> {
    let catalog:LandmarkCatalog=serde_json::from_str(include_str!("../../web/landmarks.json"))
        .map_err(|e|format!("bundled landmark catalog: {e}"))?;
    // The first section has verified landmark anchors; additional sections
    // may have none yet. Do not misplace Downtown labels in another region.
    let mut catalog=catalog;
    if catalog.region_id!=city.region.id {catalog.landmarks.clear();}
    let (buildings,building_report)=terrain.mesh_buildings(city,HeightPolicy::default())?;
    let (mut roads,road_report)=mesh_streets_with_bridges(city,bridge_preview)?;
    terrain.drape_roads_with_preview(&city.region,&mut roads,bridge_preview)?;
    if building_report.buildings_meshed == 0 || building_report.buildings_skipped > 0 {
        return Err(format!("city has {} buildings meshed, {} skipped",
            building_report.buildings_meshed, building_report.buildings_skipped));
    }
    let mut verts=Vec::new();
    let mut indices=Vec::new();
    let mut batches=Vec::new();
    let mut add = |mesh:&Mesh,color:[f32;3],layer:SceneLayer| -> Result<(),String> {
        let start=u32::try_from(indices.len()).map_err(|_|"too many city indices")?;
        append_part(mesh,color,&mut verts,&mut indices)?;
        let end=u32::try_from(indices.len()).map_err(|_|"too many city indices")?;
        if end>start {
            batches.push(DrawBatch{layer,first_index:start,count:end-start});
        }
        Ok(())
    };
    let ground=terrain.mesh(&city.region)?;
    add(&ground,[0.39,0.44,0.42],SceneLayer::Ground)?;
    add(&buildings,[0.71,0.76,0.81],SceneLayer::Buildings)?;
    let bridge_triangles=roads.layers.iter().filter(|(kind,_)|
        matches!(kind,SurfaceKind::ElevatedRoad|SurfaceKind::BridgeFascia|
            SurfaceKind::BridgeSupports|SurfaceKind::BridgeRails|
            SurfaceKind::BridgeStripe))
        .map(|(_,mesh)|mesh.indices.len()/3).sum();
    set_bridge_mesh_triangles(bridge_triangles);
    let freeway_triangles=roads.layers.iter().filter(|(kind,_)|
        matches!(kind,SurfaceKind::Motorway|SurfaceKind::MotorwayStripe))
        .map(|(_,mesh)|mesh.indices.len()/3).sum();
    set_freeway_mesh_triangles(freeway_triangles);
    for (kind,mesh) in roads.layers {
        let (layer,color)=match kind {
            SurfaceKind::Road => (SceneLayer::Road,[0.24,0.26,0.29]),
            SurfaceKind::Motorway => (SceneLayer::Road,[0.14,0.19,0.23]),
            SurfaceKind::MotorwayStripe =>
                (SceneLayer::Road,[0.91,0.89,0.77]),
            SurfaceKind::BridgeStripe =>
                (SceneLayer::ElevatedRoad,[0.91,0.89,0.77]),
            SurfaceKind::ElevatedRoad => (SceneLayer::ElevatedRoad,[0.26,0.29,0.32]),
            SurfaceKind::BridgeFascia => (SceneLayer::BridgeStructure,[0.48,0.51,0.52]),
            SurfaceKind::BridgeSupports => (SceneLayer::BridgeStructure,[0.61,0.63,0.62]),
            SurfaceKind::BridgeRails => (SceneLayer::BridgeStructure,[0.76,0.78,0.78]),
            SurfaceKind::Walk => (SceneLayer::Walk,[0.60,0.58,0.51]),
            SurfaceKind::Steps => (SceneLayer::Steps,[0.74,0.62,0.46]),
            SurfaceKind::Rail => (SceneLayer::Rail,[0.40,0.52,0.61]),
        };
        add(&mesh,color,layer)?;
    }
    // For parks and sites not enclosed in a building polygon, a small colored
    // ground disc represents the geographic anchor, not an invented footprint.
    for mark in &catalog.landmarks {
        let [lon,lat]=mark.coordinates;
        if lon<city.region.bbox[0]||lon>city.region.bbox[2]||
           lat<city.region.bbox[1]||lat>city.region.bbox[3] {continue;}
        if let Some(rgb)=category_rgb(&mark.kind) {
            let p=city.region.project(lon,lat);
            let disc=landmark_disc(terrain,&city.region,p,
                if mark.kind=="park" {15.0} else {6.0});
            if let Some(category)=category_index(&mark.kind) {
                add(&disc,rgb,SceneLayer::Marker(category))?;
            }
        }
    }
    // Category tints are assigned to real matched building geometry, using
    // precisely the same per-building mesh ordering as TerrainGrid.
    // All other massing retains its original color when the toggle is off.
    let mut cursor=ground.vertices.len();
    let building_end=cursor+buildings.vertices.len();
    for building in &city.buildings {
        let Ok(mesh)=mesh_building(building,HeightPolicy::default()) else {continue;};
        let end=cursor+mesh.vertices.len();
        if end>building_end {return Err("landmark vertex ranges exceed building layer".into());}
        if let Some((rgb,id))=building_landmark_color(building,&city.region,&catalog.landmarks) {
            for vertex in &mut verts[cursor..end] {
                vertex.category_color=rgb;
                vertex.category_id=(id+1) as f32;
            }
        }
        cursor=end;
    }
    if cursor!=building_end {return Err("landmark building ranges do not match terrain mesh".into());}
    set_elevated_span_count(road_report.elevated_spans);
    info(&format!("Loaded {} buildings, {} transport spans ({} source-tagged elevated), {} vertices, {} triangles",
        building_report.buildings_meshed,road_report.segments_meshed,
        road_report.elevated_spans,verts.len(),indices.len()/3));
    Ok((verts,indices,batches))
}

// The first frame is genuine geographic terrain plus all street and
// pedestrian geometry. Buildings are meshed after that first frame, so
// the GPU is usable while the expensive enrichment is still pending.
// Uses the exact same road generator as the final mesh, not placeholder
// raster images or invented street geometry.
fn coarse_city(city:&City,terrain:&TerrainGrid,bridges:bool,spawn:Point)
    ->Result<(Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>),String>{
    let mut vertices=Vec::new();
    let mut indices=Vec::new();
    let mut batches=Vec::new();
    let mut add=|mesh:&Mesh,color:[f32;3],layer:SceneLayer|->Result<(),String>{
        let start=u32::try_from(indices.len()).map_err(|_|"too many indices")?;
        append_part(mesh,color,&mut vertices,&mut indices)?;
        let end=u32::try_from(indices.len()).map_err(|_|"too many indices")?;
        if end>start{batches.push(DrawBatch{layer,first_index:start,count:end-start});}
        Ok(())
    };
    let ground=terrain.mesh(&city.region)?;
    add(&ground,[0.39,0.44,0.42],SceneLayer::Ground)?;
    // Only mesh streets near the actual spawn for first presentation.
    // The deferred complete pass still consumes the authoritative full city.
    let preview=urbarium::streaming::first_frame_streets(
        city,spawn,urbarium::streaming::FIRST_FRAME_STREET_RADIUS_M);
    set_first_frame_street_telemetry(preview.streets.len(),city.streets.len());
    let (mut roads,_)=mesh_streets_with_bridges(&preview,bridges)?;
    terrain.drape_roads_with_preview(&city.region,&mut roads,bridges)?;
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
    Ok((vertices,indices,batches))
}

fn info(message: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(el) = doc.get_element_by_id("status") {
                el.set_text_content(Some(message));
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("Urbarium: {message}");
}

#[cfg(target_arch="wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function publish_city_for_minimap(data) {
    window.__urbariumCityJson = data;
    window.dispatchEvent(new Event('urbarium-city-ready'));
}
export function publish_terrain_for_minimap(data) {
    window.__urbariumTerrainJson = data;
    window.dispatchEvent(new Event('urbarium-terrain-ready'));
}
export function fallback_from_software_webgpu() {
    try {sessionStorage.setItem("urbarium-webgpu-fallback-v1",JSON.stringify({
        time:new Date().toISOString(),reason:"WebGPU selected a CPU / software adapter",
        requested:"webgpu",selected:"webgl2"
    }));} catch (_) {}
    const url=new URL(location.href);
    url.searchParams.set("renderer","webgl2");
    url.searchParams.delete("allow-software");
    // Replace, rather than adding a broken software-renderer history entry.
    location.replace(url.href);
}
"#)]
extern "C" {
    fn publish_city_for_minimap(data:&str);
    fn publish_terrain_for_minimap(data:&str);
    fn fallback_from_software_webgpu();
}

fn browser_progressive()->bool {
    #[cfg(target_arch="wasm32")]
    if let Some(value)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.document_element())
        .and_then(|el|el.get_attribute("data-progressive-preview")) {
        return value=="true";
    }
    false
}
fn set_first_frame_street_telemetry(shown:usize,total:usize) {
    #[cfg(target_arch="wasm32")]
    if let Some(el)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status")) {
        let _=el.set_attribute("data-first-frame-streets",&shown.to_string());
        let _=el.set_attribute("data-source-streets",&total.to_string());
        let _=el.set_attribute("data-first-frame-street-radius-m",
            &urbarium::streaming::FIRST_FRAME_STREET_RADIUS_M.to_string());
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=(shown,total);
}
fn set_scene_detail(stage:&str) {
    #[cfg(target_arch="wasm32")]
    if let Some(el)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status")) {
        let _=el.set_attribute("data-scene-detail",stage);
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=stage;
}

fn mark_first_frame_detail(){
    #[cfg(target_arch="wasm32")]
    if let Some(el)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status")) {
        let stage=el.get_attribute("data-scene-detail").unwrap_or_else(||"unknown".into());
        let _=el.set_attribute("data-first-frame-detail",&stage);
    }
}

fn browser_joined_world()->bool {
    #[cfg(target_arch="wasm32")]
    if let Some(value)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.document_element())
        .and_then(|el|el.get_attribute("data-world-preview")) {
        return value=="true";
    }
    false
}
fn set_world_telemetry(sections:usize,draws:u32) {
    #[cfg(target_arch="wasm32")]
    if let Some(el)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status")) {
        let _=el.set_attribute("data-world-sections",&sections.to_string());
        let _=el.set_attribute("data-world-secondary-drawcalls",&draws.to_string());
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=(sections,draws);
}

fn browser_region_id()->String {
    #[cfg(target_arch="wasm32")]
    if let Some(id)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.document_element())
        .and_then(|e|e.get_attribute("data-active-region")) {
        // Restrict paths to safe ASCII identifiers even if the page is modified.
        if !id.is_empty() && id.len()<80 &&
            id.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'-') {
            return id;
        }
    }
    "downtown-la".into()
}

async fn fetch_city(section:&str,publish:bool) -> Result<City, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let filename=format!("./assets/{section}.json");
        let response = gloo_net::http::Request::get(&filename)
            .send().await.map_err(|e| format!("city fetch: {e}"))?;
        if !response.ok() {
            return Err(format!("city asset returned HTTP {}",response.status()));
        }
        set_browser_stage("city-decoding");
        info("Downloaded/read city data; decoding geography...");
        let data=response.text().await.map_err(|e| format!("city body: {e}"))?;
        let city:City=serde_json::from_str(&data)
            .map_err(|e|format!("city JSON: {e}"))?;
        // The minimap previously repeated this request independently.
        // Share this exact, already validated payload with the browser UI.
        // This prevents a second large fetch from failing on poor networks.
        if publish { publish_city_for_minimap(&data); }
        Ok(city)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _=publish;
        let path=format!("data/processed/{section}.json");
        let data=std::fs::read_to_string(&path)
            .map_err(|e| format!("native viewer requires {path}: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("city JSON: {e}"))
    }
}

#[cfg(target_arch="wasm32")]
fn set_bridge_mesh_triangles(count:usize) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()){
        if let Some(el)=doc.get_element_by_id("status"){
            let _=el.set_attribute("data-bridge-mesh-triangles",&count.to_string());
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_bridge_mesh_triangles(_:usize) {}
#[cfg(target_arch="wasm32")]
fn set_freeway_mesh_triangles(count:usize) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-freeway-mesh-triangles",&count.to_string());
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_freeway_mesh_triangles(_:usize) {}

#[cfg(target_arch="wasm32")]
fn set_elevated_span_count(count:usize) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-elevated-road-spans",&count.to_string());
        }
        if let Some(button)=doc.get_element_by_id("inspect-overpass") {
            let active=doc.get_element_by_id("status")
                .and_then(|el|el.get_attribute("data-bridge-preview-active"))
                .as_deref()==Some("true");
            let label=if active {
                format!("Inspect prototype overpass ({count})")
            }else{"Inspect overpass (preview only)".to_owned()};
            button.set_text_content(Some(&label));
            if active&&count>0 {
                let _=button.remove_attribute("disabled");
            }else{
                let _=button.set_attribute("disabled","");
            }
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_elevated_span_count(_:usize) {}

#[cfg(target_arch="wasm32")]
fn set_collision_count(count: usize) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-building-obstacles",&count.to_string());
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_collision_count(_:usize) {}

#[cfg(target_arch="wasm32")]
fn set_browser_backend(backend: &str) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-gpu-backend",backend);
            let _=el.set_attribute("data-gpu-device-lost","false");
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_browser_backend(_: &str) {}

fn expose_gpu_details(info:&wgpu::AdapterInfo,format:wgpu::TextureFormat,srgb:bool){
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            for (key,value) in [
                ("data-gpu-adapter",if info.name.trim().is_empty() {
                    "Name hidden by browser"
                }else{info.name.as_str()}),
                ("data-gpu-driver",info.driver.as_str()),
                ("data-gpu-device-type",match info.device_type {
                    wgpu::DeviceType::DiscreteGpu=>"Discrete GPU",
                    wgpu::DeviceType::IntegratedGpu=>"Integrated GPU",
                    wgpu::DeviceType::Cpu=>"CPU / software",
                    wgpu::DeviceType::VirtualGpu=>"Virtual GPU",
                    _=>"Other / unknown",
                }),
            ] {let _=el.set_attribute(key,value);}
            let _=el.set_attribute("data-gpu-surface-format",&format!("{format:?}"));
            let _=el.set_attribute("data-gpu-color-transfer",
                if srgb {"sRGB attachment (automatic encode)"}
                else {"Unorm attachment (shader-encoded display color)"});
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=(info,format,srgb);
}

#[cfg(target_arch="wasm32")]
fn set_browser_stage(stage: &str) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            // Once a real frame was presented, background section fetches
            // must not regress startup status to terrain/city decoding.
            // Enrichment belongs in data-scene-detail instead.
            if el.get_attribute("data-render-stage").as_deref()==Some("first-present")
                && stage!="device-lost" {return;}
            let _=el.set_attribute("data-render-stage",stage);
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_browser_stage(_: &str) {}

#[cfg(target_arch="wasm32")]
fn set_browser_frame_count(count: u64) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-frame-count",&count.to_string());
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_browser_frame_count(_: u64) {}

async fn fetch_terrain(section:&str,publish:bool) -> Result<TerrainGrid,String> {
    #[cfg(target_arch="wasm32")]
    {
        let filename=format!("./assets/{section}-terrain.json");
        let response=gloo_net::http::Request::get(&filename)
            .send().await.map_err(|e|format!("DEM fetch: {e}"))?;
        if !response.ok(){return Err(format!("DEM returned HTTP {}",response.status()));}
        set_browser_stage("terrain-decoding");
        info("Downloaded/read elevation data; decoding terrain...");
        let data=response.text().await.map_err(|e|format!("DEM body: {e}"))?;
        let terrain:TerrainGrid=serde_json::from_str(&data)
            .map_err(|e|format!("DEM JSON: {e}"))?;
        // Share the exact DEM grid as well; labels and scene use one datum.
        if publish { publish_terrain_for_minimap(&data); }
        Ok(terrain)
    }
    #[cfg(not(target_arch="wasm32"))]
    {
        let _=publish;
        let path=format!("data/processed/{section}-terrain.json");
        let data=std::fs::read_to_string(&path)
            .map_err(|e|format!("native viewer requires terrain grid {path}: {e}"))?;
        serde_json::from_str(&data).map_err(|e|format!("DEM JSON: {e}"))
    }
}
#[cfg(target_arch="wasm32")]
fn set_terrain_size(points:usize) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-terrain-points",&points.to_string());
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_terrain_size(_:usize) {}

fn scene_layer_visible(layer:SceneLayer)->bool {
    match layer {
        SceneLayer::Ground | SceneLayer::Buildings | SceneLayer::Road |
        SceneLayer::ElevatedRoad | SceneLayer::BridgeStructure => true,
        SceneLayer::Marker(category)=>browser_category_mask() & (1<<category)!=0,
        // These independent centerlines are not surveyed sidewalk boundaries
        // or grade-correct rail beds; isolate layers for debugging.
        SceneLayer::Walk | SceneLayer::Steps => layer_enabled("layer-walk",true),
        SceneLayer::Rail => layer_enabled("layer-rail",false),
    }
}
fn browser_category_mask()->u32 {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("landmark-legend") {
            if let Some(mask)=el.get_attribute("data-visible-mask") {
                if let Ok(mask)=mask.parse::<u32>() {
                    return mask&255;
                }
            }
        }
    }
    255
}
fn browser_sky_hour()->f32 {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("time-of-day") {
            return el.get_attribute("data-hour")
                .and_then(|s|s.parse::<f32>().ok())
                .unwrap_or(12.0).clamp(0.0,24.0);
        }
    }
    12.0
}
fn layer_enabled(id:&str,default:bool)->bool {
    #[cfg(target_arch="wasm32")]
    {
        if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
            if let Some(el)=doc.get_element_by_id(id) {
                return el.get_attribute("data-visible").as_deref()==Some("true");
            }
        }
    }
    let _=id;
    default
}

/// Captures GPU errors even when they happen asynchronously after a frame.
#[derive(Default)]
struct GpuFault {
    lost: AtomicBool,
    last_error: Mutex<Option<String>>,
}
impl GpuFault {
    fn record(&self,message:String) {
        if let Ok(mut value)=self.last_error.lock() {
            *value=Some(message.chars().take(350).collect());
        }
    }
    fn message(&self)->Option<String> {
        self.last_error.lock().ok().and_then(|value|value.clone())
    }
}
struct ResidentTile {
    vertices:wgpu::Buffer,
    indices:wgpu::Buffer,
    batches:Vec<DrawBatch>,
    terrain:TerrainGrid,
    region:Region,
    collision:CollisionWorld,
    placement:urbarium::world::TilePlacement,
    bounds:[f64;4],
    bytes:usize,
}
impl ResidentTile {
    fn height(&self,x:f64,z:f64)->f32 {
        let local=self.placement.local(x,z);
        self.terrain.sample_surface(&self.region,local[0],local[1])+self.placement.up_m
    }
}
// The first pass deliberately caps residency to the two adjacent city
// sections. Future geographic streaming will evict these buffers, rather
// than expanding a monolithic mesh to county scale.
struct DeferredScene {
    city:City,
    terrain:TerrainGrid,
    bridge_preview:bool,
    joined:bool,
}
type NeighborCpu=(
    Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>,TerrainGrid,Region,
    CollisionWorld,urbarium::world::TilePlacement,[f64;4]
);
struct Graphics {
    window: SharedWindow,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    surface_is_srgb: bool,
    pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    depth: wgpu::TextureView,
    camera: Camera,
    street_camera: Camera,
    overview_camera: Camera,
    camera_request: u32,
    collision: CollisionWorld,
    terrain: TerrainGrid,
    region: Region,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    batches:Vec<DrawBatch>,
    world_preview:bool,
    primary_bounds:[f64;4],
    neighbor:Option<ResidentTile>,
    deferred_scene:Option<DeferredScene>,
    primary_gpu_bytes:usize,
    rendered_frames: u64,
    gpu_mesh_bytes: usize,
    active_draw_calls:u32,
    active_triangles:u32,
    gpu_fault: Arc<GpuFault>,
    bridge_overlook:Option<Camera>,
    inspect_request:u32,
}
impl Graphics {
    fn depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Urbarium depth"),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1),
                depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }).create_view(&wgpu::TextureViewDescriptor::default())
    }
    fn reconfigure_surface(&mut self, size:PhysicalSize<u32>) {
        let size=limit_backing_size(size);
        self.config.width=size.width.max(1);
        self.config.height=size.height.max(1);
        self.surface.configure(&self.device,&self.config);
        self.depth=Self::depth_view(&self.device,self.config.width,self.config.height);
        expose_surface_size(self.config.width,self.config.height);
    }
    fn resize(&mut self,size:PhysicalSize<u32>) {
        let size=limit_backing_size(size);
        if self.config.width==size.width.max(1) &&
            self.config.height==size.height.max(1) {return;}
        self.reconfigure_surface(size);
    }
    fn move_in_world(&mut self,pressed:&HashSet<KeyCode>,dt:f32) {
        // A second terrain-independent Camera::move_by() used to reset the
        // camera's eye_clearance relative to y=0 every frame. Subtracting
        // that clearance from the true one then ADDED the terrain elevation
        // to camera Y on every movement frame (the reported "rocket up" bug).
        // Reuse the original control interpretation, but never recompute
        // clearance in a fictitious zero-height world.
        let movement=self.camera.control_displacement(pressed,dt);
        if movement.length_squared()==0.0 {return;}
        let prior=self.camera.position;
        let mut x=prior.x as f64+movement.x as f64;
        let mut z=prior.z as f64+movement.z as f64;
        let inside=|x:f64,z:f64| {
            urbarium::world::within(self.primary_bounds,x,z) ||
            self.neighbor.as_ref().is_some_and(|tile|
                urbarium::world::within(tile.bounds,x,z))
        };
        let height=|x:f64,z:f64| -> f32 {
            if urbarium::world::within(self.primary_bounds,x,z) {
                self.terrain.sample_surface(&self.region,x,z)
            }else if let Some(tile)=self.neighbor.as_ref().filter(|tile|
                urbarium::world::within(tile.bounds,x,z)) {
                tile.height(x,z)
            }else{
                self.terrain.sample_surface(&self.region,prior.x as f64,prior.z as f64)
            }
        };
        let was_flying=self.camera.eye_clearance>1.701 || movement.y>0.0;
        let mut ground=height(x,z);
        let intended_y=if was_flying {
            prior.y+movement.y
        }else {
            ground+1.7
        };
        // Flight is camera-controlled: crossing an uphill terrain ridge
        // cannot make the avatar rise without E/Space. Stop the horizontal
        // move at that point instead of automatically lifting the view.
        if !inside(x,z) || (was_flying && intended_y<ground+1.7 &&
            (movement.x!=0.0 || movement.z!=0.0)) {
            x=prior.x as f64;
            z=prior.z as f64;
            ground=height(x,z);
        }
        let desired_y=if was_flying {
            intended_y.max(ground+1.7)  // Q can't descend below feet-level
        }else{ground+1.7};
        // Preserve the existing source-local building collision behavior.
        // The thin boundary strip is handled by the joined tile extent
        // instead of treating it as an impassable wall.
        let in_primary=urbarium::world::within(self.primary_bounds,x,z);
        let collision=if in_primary {Some((&self.collision,None))}
            else {self.neighbor.as_ref().map(|tile|(&tile.collision,Some(tile.placement)))};
        if let Some((world,placement))=collision {
            let bounds=if in_primary {self.primary_bounds}
                else {self.neighbor.as_ref().map(|tile|tile.bounds).unwrap_or(self.primary_bounds)};
            let margin=0.5;
            let safely_inside=x>bounds[0]+margin&&x<bounds[2]-margin&&
                z>bounds[1]+margin&&z<bounds[3]-margin;
            let old_in=urbarium::world::within(bounds,prior.x as f64,prior.z as f64);
            if safely_inside&&old_in {
                let (start,end,local_y)=if let Some(t)=placement {
                    (t.local(prior.x as f64,prior.z as f64),t.local(x,z),desired_y-t.up_m)
                }else{([prior.x as f64,prior.z as f64],[x,z],desired_y)};
                let resolved=world.move_sliding(start,end,local_y as f64,0.35);
                if let Some(t)=placement {
                    x=t.east_m as f64+resolved[0]*t.east_scale as f64;
                    z=t.north_m as f64+resolved[1];
                }else{x=resolved[0];z=resolved[1];}
            }
        }
        ground=height(x,z);
        let y=if was_flying {desired_y.max(ground+1.7)}else{ground+1.7};
        self.camera.position=Vec3::new(x as f32,y,z as f32);
        self.camera.eye_clearance=y-ground;
    }
    fn set_primary_mesh(&mut self,vertices:Vec<GpuVertex>,indices:Vec<u32>,batches:Vec<DrawBatch>){
        let bytes=vertices.len()*size_of::<GpuVertex>()+indices.len()*size_of::<u32>();
        self.vertices=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
            label:Some("Enriched primary vertices"),
            contents:bytemuck::cast_slice(&vertices),usage:wgpu::BufferUsages::VERTEX,
        });
        self.indices=self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
            label:Some("Enriched primary indices"),
            contents:bytemuck::cast_slice(&indices),usage:wgpu::BufferUsages::INDEX,
        });
        self.index_count=indices.len() as u32;
        self.batches=batches;
        self.gpu_mesh_bytes=self.gpu_mesh_bytes-self.primary_gpu_bytes+bytes;
        self.primary_gpu_bytes=bytes;
    }
    fn set_neighbor(&mut self,source:NeighborCpu){
        let (vertices,indices,batches,terrain,region,collision,placement,bounds)=source;
        let bytes=vertices.len()*size_of::<GpuVertex>()+indices.len()*size_of::<u32>();
        let resident=ResidentTile{
            vertices:self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
                label:Some("Late neighbor vertices"),
                contents:bytemuck::cast_slice(&vertices),usage:wgpu::BufferUsages::VERTEX,
            }),
            indices:self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
                label:Some("Late neighbor indices"),
                contents:bytemuck::cast_slice(&indices),usage:wgpu::BufferUsages::INDEX,
            }),
            batches,terrain,region,collision,placement,bounds,bytes,
        };
        if let Some(old)=self.neighbor.replace(resident) {
            self.gpu_mesh_bytes=self.gpu_mesh_bytes.saturating_sub(old.bytes);
        }
        self.gpu_mesh_bytes+=bytes;
        set_world_telemetry(2,0);
    }
    fn update(&mut self, pressed: &HashSet<KeyCode>, dt: f32) {
        let (camera_request,overview)=browser_camera_request();
        if camera_request!=self.camera_request {
            self.camera_request=camera_request;
            self.camera=if overview {self.overview_camera.clone()}
                else {self.street_camera.clone()};
            self.camera.fast_mode=false;
            set_speed_label(false);
            set_camera_mode(if overview {"overview"}else{"street"});
        }
        let request=browser_inspect_overpass_request();
        if request!=self.inspect_request {
            self.inspect_request=request;
            if let Some(camera)=self.bridge_overlook.clone() {
                self.camera=camera;
                self.camera.fast_mode=false;
                set_speed_label(false);
            }
        }
        if self.world_preview {
            self.move_in_world(pressed,dt);
        }else{
            self.camera.move_by(pressed,dt,Some(&self.collision),Some((&self.terrain,&self.region)));
        }
        let vp=self.camera.view_projection(self.config.width,self.config.height);
        self.queue.write_buffer(&self.camera_buffer,0,
            bytemuck::bytes_of(&CameraUniform{
                view_proj:vp.to_cols_array_2d(),
                options:[
                    1.0,browser_sky_hour(),self.camera.yaw,self.camera.pitch],
                sky:[self.config.width as f32/self.config.height.max(1) as f32,
                     browser_category_mask() as f32,
                     if self.surface_is_srgb {1.0}else{0.0},0.0],
            }));
    }
    fn draw(&mut self) -> bool {
        if self.gpu_fault.lost.load(Ordering::Acquire) {
            // A lost device cannot recover from surface reconfiguration.
            // Stop submitting work and retain the actual GPU failure.
            return false;
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) |
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                // The surface can be lost WITHOUT a window dimension change.
                // resize() deliberately no-ops for equal dimensions, so it
                // cannot recover a lost swapchain. Force configure instead.
                self.reconfigure_surface(self.window.inner_size());
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout |
            wgpu::CurrentSurfaceTexture::Occluded |
            wgpu::CurrentSurfaceTexture::Validation => return false,
        };
        let view=frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder=self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor{
            label:Some("Urbarium encoder")
        });
        let mut drawn_calls=0u32;
        let mut drawn_triangles=0u32;
        {
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("Urbarium city"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                    view:&view,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations {
                        load:wgpu::LoadOp::Clear(wgpu::Color{r:0.065,g:0.09,b:0.13,a:1.0}),
                        store:wgpu::StoreOp::Store
                    }
                })],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                    view:&self.depth,
                    depth_ops:Some(wgpu::Operations{
                        load:wgpu::LoadOp::Clear(1.0),store:wgpu::StoreOp::Store
                    }),
                    stencil_ops:None,
                }),
                timestamp_writes:None,occlusion_query_set:None,multiview_mask:None,
            });
            // Procedural sky is camera-oriented and depth-independent.
            // It draws first; terrain/buildings/roads then depth-test over it.
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(0,&self.camera_bind_group,&[]);
            pass.draw(0..3,0..1);
            drawn_calls+=1;
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0,&self.camera_bind_group,&[]);
            pass.set_vertex_buffer(0,self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..),wgpu::IndexFormat::Uint32);
            for batch in &self.batches {
                if !scene_layer_visible(batch.layer) {continue;}
                pass.draw_indexed(batch.first_index..batch.first_index+batch.count,0,0..1);
                drawn_calls+=1;
                drawn_triangles+=batch.count/3;
            }
            let mut secondary_draws=0;
            if let Some(neighbor)=&self.neighbor {
                let b=neighbor.bounds;
                let px=(self.camera.position.x as f64).clamp(b[0],b[2]);
                let pz=(self.camera.position.z as f64).clamp(b[1],b[3]);
                let dist=(px-self.camera.position.x as f64).hypot(
                    pz-self.camera.position.z as f64);
                // Draw-distance culling controls GPU work; only two tile
                // buffers are resident, regardless of the catalog size.
                if dist<2400.0 {
                    pass.set_vertex_buffer(0,neighbor.vertices.slice(..));
                    pass.set_index_buffer(neighbor.indices.slice(..),wgpu::IndexFormat::Uint32);
                    for batch in &neighbor.batches {
                        if !scene_layer_visible(batch.layer) {continue;}
                        pass.draw_indexed(batch.first_index..batch.first_index+batch.count,0,0..1);
                        drawn_calls+=1;
                        secondary_draws+=1;
                        drawn_triangles+=batch.count/3;
                    }
                }
            }
            set_world_telemetry(if self.neighbor.is_some(){2}else{1},secondary_draws);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.active_draw_calls=drawn_calls;
        self.active_triangles=drawn_triangles;
        self.rendered_frames += 1;
        true
    }
}

async fn init_graphics(window:SharedWindow,proxy:EventLoopProxy<AppEvent>) {
    let result=try_init(window,proxy.clone()).await;
    if let Err(err)=result {
        info(&format!("Viewer error: {err}"));
        let _=proxy.send_event(AppEvent::Error(err));
    }
}

async fn try_init(window:SharedWindow,proxy:EventLoopProxy<AppEvent>) -> Result<(), String> {
    set_browser_stage("city-fetch");
    let primary_id=browser_region_id();
    info(&format!("Loading {primary_id} streets and buildings..."));
    let city=fetch_city(&primary_id,true).await?;
    set_browser_stage("terrain-fetch");
    info(&format!("Loading {primary_id} elevation grid..."));
    let terrain=fetch_terrain(&primary_id,true).await?;
    #[cfg(target_arch="wasm32")]
    if let Some(el)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status")) {
        let _=el.set_attribute("data-region-id",&city.region.id);
    }
    set_browser_stage("geometry-build");
    info("Generating 3D terrain, buildings and streets...");
    terrain.validate(&city.region)?;
    if terrain.region_id!=city.region.id{
        return Err("Selected city section and DEM region IDs do not match".into());
    }
    set_terrain_size(terrain.heights_m.len());
    let bridge_preview=browser_bridge_preview();
    set_bridge_preview_state(bridge_preview);
    let progressive=browser_progressive();
    // Collision-aware spawn must be chosen before the preview mesh, so the
    // first render contains the streets the player is actually looking at.
    let collision=CollisionWorld::from_city_with_terrain(&city,Some(&terrain));
    let spawn_camera=Camera::spawn_at_street(&city,&collision,&terrain);
    let deferred_scene=if progressive {
        Some(DeferredScene{city:city.clone(),terrain:terrain.clone(),
            bridge_preview,joined:browser_joined_world()})
    }else{None};
    let (vertices,indices,batches)=if progressive {
        set_scene_detail("base");
        coarse_city(&city,&terrain,bridge_preview,
            Point{x:spawn_camera.position.x as f64,z:spawn_camera.position.z as f64})?
    }else {
        combined_city(&city,&terrain,bridge_preview)?
    };
    let primary_bounds={
        let sw=city.region.project(city.region.bbox[0],city.region.bbox[1]);
        let ne=city.region.project(city.region.bbox[2],city.region.bbox[3]);
        [sw.x,sw.z,ne.x,ne.z]
    };
    // Only the opted-in preview loads a second independent dataset. It
    // preserves the same GPU backend and keeps old single-section mode intact.
    let secondary_source=if browser_joined_world()&&!progressive {
        let id=if primary_id=="downtown-la" {"little-tokyo-east"}else{"downtown-la"};
        set_browser_stage("neighbor-fetch");
        info(&format!("Loading neighboring section {id}..."));
        let other=fetch_city(id,false).await?;
        let ground=fetch_terrain(id,false).await?;
        ground.validate(&other.region)?;
        let placement=urbarium::world::TilePlacement::between(
            &city.region,terrain.origin_elevation_m,
            &other.region,ground.origin_elevation_m);
        let (mut verts,indices,batches)=combined_city(&other,&ground,bridge_preview)?;
        for vertex in &mut verts {
            vertex.position=placement.point(
                vertex.position[0],vertex.position[1],vertex.position[2]);
        }
        let bounds=placement.bounds(&other.region);
        let obstacles=CollisionWorld::from_city_with_terrain(&other,Some(&ground));
        Some((verts,indices,batches,ground,other.region,obstacles,placement,bounds))
    }else{None};
    let bridge_overlook=if bridge_preview {
        bridge_overlook(&city,&terrain,&collision)
    }else{None};
    set_browser_stage("gpu-init");
    info("Geography ready; initializing WebGPU or WebGL2...");
    let instance=wgpu::util::new_instance_with_webgpu_detection(
        wgpu::InstanceDescriptor::new_without_display_handle()).await;
    let surface=instance.create_surface(window.clone())
        .map_err(|e|format!("surface: {e}"))?;
    // Prefer the compositor iGPU in the browser. The RTX 3090 is reserved for
    // compute/games and must not be woken merely to explore a small map.
    // Native viewers retain the ordinary high-performance preference.
    #[cfg(target_arch="wasm32")]
    let preference=wgpu::PowerPreference::LowPower;
    #[cfg(not(target_arch="wasm32"))]
    let preference=wgpu::PowerPreference::HighPerformance;
    let adapter=instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference:preference,
        force_fallback_adapter:false,
        compatible_surface:Some(&surface),
        apply_limit_buckets:false,
    }).await.map_err(|e|format!("Neither WebGPU nor WebGL2 could initialize: {e}. Check browser WebGL2 support and hardware acceleration."))?;
    let gpu_info=adapter.get_info();
    let backend = format!("{:?}",gpu_info.backend);
    set_browser_backend(&backend);
    #[cfg(target_arch="wasm32")]
    if backend=="BrowserWebGpu" && gpu_info.device_type==wgpu::DeviceType::Cpu &&
        !browser_allows_software_webgpu() {
        set_browser_stage("software-webgpu-fallback");
        info("WebGPU chose a software adapter; returning to hardware WebGL2");
        fallback_from_software_webgpu();
        return Ok(());
    }
    let (device,queue)=adapter.request_device(&wgpu::DeviceDescriptor {
        label:Some("Urbarium device"),
        required_features:wgpu::Features::empty(),
        required_limits:wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        memory_hints:wgpu::MemoryHints::Performance,
        trace:Default::default(),
        experimental_features:Default::default(),
    }).await.map_err(|e|format!("Graphics device ({backend}): {e}"))?;
    let gpu_fault=Arc::new(GpuFault::default());
    device.set_device_lost_callback({
        let faults=Arc::clone(&gpu_fault);
        move |reason,message|{
            if reason!=wgpu::DeviceLostReason::Destroyed {
                faults.lost.store(true,Ordering::Release);
                faults.record(format!("device lost ({reason:?}): {message}"));
            }
        }
    });
    device.on_uncaptured_error(Arc::new({
        let faults=Arc::clone(&gpu_fault);
        move |error|faults.record(format!("uncaptured GPU error: {error}"))
    }));
    let requested=window.inner_size();
    let size=limit_backing_size(requested);
    if size!=requested {
        let _=window.request_inner_size(size);
    }
    let mut config=surface.get_default_config(&adapter,size.width.max(1),size.height.max(1))
        .ok_or("No compatible GPU surface configuration")?;
    expose_surface_size(config.width,config.height);
    config.present_mode=wgpu::PresentMode::Fifo;
    // sRGB render targets encode linear fragment values automatically.
    // Non-sRGB targets require already-encoded display values. The shader
    // explicitly handles both cases, preventing backend-dependent gamma.
    let surface_is_srgb=matches!(config.format,
        wgpu::TextureFormat::Bgra8UnormSrgb|wgpu::TextureFormat::Rgba8UnormSrgb);
    expose_gpu_details(&gpu_info,config.format,surface_is_srgb);
    surface.configure(&device,&config);
    set_collision_count(collision.obstacle_count());
    let camera=spawn_camera;
    let overview_camera=Camera::city_overview(&city,&collision,&terrain);
    set_camera_mode("street");
    let camera_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:Some("Camera uniform"),
        contents:bytemuck::bytes_of(&CameraUniform {
            view_proj:camera.view_projection(config.width,config.height).to_cols_array_2d(),
            options:[0.0,12.0,camera.yaw,camera.pitch],
            sky:[config.width as f32/config.height.max(1) as f32,0.0,
                 if surface_is_srgb {1.0}else{0.0},0.0],
        }),
        usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,
    });
    let camera_layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{
        label:Some("Camera layout"),
        entries:&[wgpu::BindGroupLayoutEntry{
            binding:0,visibility:wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty:wgpu::BindingType::Buffer{
                ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None
            },
            count:None,
        }],
    });
    let camera_bind_group=device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:Some("Camera"),
        layout:&camera_layout,
        entries:&[wgpu::BindGroupEntry{
            binding:0,resource:camera_buffer.as_entire_binding()
        }],
    });
    let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{
        label:Some("Urbarium shader"),
        source:wgpu::ShaderSource::Wgsl(include_str!("viewer.wgsl").into()),
    });
    let layout=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{
        label:Some("City pipeline layout"),
        bind_group_layouts:&[Some(&camera_layout)],
        immediate_size:0,
    });
    let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
        label:Some("Urbarium city pipeline"),
        layout:Some(&layout),
        vertex:wgpu::VertexState{
            module:&shader,entry_point:Some("vs_main"),
            buffers:&[Some(wgpu::VertexBufferLayout{
                array_stride:size_of::<GpuVertex>() as u64,
                step_mode:wgpu::VertexStepMode::Vertex,
                attributes:&VERTEX_ATTRS,
            })],
            compilation_options:Default::default(),
        },
        fragment:Some(wgpu::FragmentState{
            module:&shader,entry_point:Some("fs_main"),
            targets:&[Some(config.format.into())],
            compilation_options:Default::default(),
        }),
        primitive:wgpu::PrimitiveState {
            topology:wgpu::PrimitiveTopology::TriangleList,
            cull_mode:None,..Default::default()
        },
        depth_stencil:Some(wgpu::DepthStencilState {
            format:wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::LessEqual),
            stencil:Default::default(),bias:Default::default(),
        }),
        multisample:Default::default(),multiview_mask:None,cache:None,
    });
    let sky_pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
        label:Some("Urbarium procedural time-of-day sky"),
        layout:Some(&layout),
        vertex:wgpu::VertexState{
            module:&shader,entry_point:Some("sky_vs"),buffers:&[],
            compilation_options:Default::default(),
        },
        fragment:Some(wgpu::FragmentState{
            module:&shader,entry_point:Some("sky_fs"),
            targets:&[Some(config.format.into())],
            compilation_options:Default::default(),
        }),
        primitive:wgpu::PrimitiveState {
            topology:wgpu::PrimitiveTopology::TriangleList,
            cull_mode:None,..Default::default()
        },
        // WebGPU/WebGL2 require the sky pipeline's attachment signature
        // to match the render pass even though sky must not change depth.
        depth_stencil:Some(wgpu::DepthStencilState {
            format:wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled:Some(false),
            depth_compare:Some(wgpu::CompareFunction::Always),
            stencil:Default::default(),bias:Default::default(),
        }),
        multisample:Default::default(),multiview_mask:None,cache:None,
    });
    let depth=Graphics::depth_view(&device,config.width,config.height);
    let vertex_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
        label:Some("Urbarium vertices"),contents:bytemuck::cast_slice(&vertices),
        usage:wgpu::BufferUsages::VERTEX,
    });
    let index_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
        label:Some("Urbarium indices"),contents:bytemuck::cast_slice(&indices),
        usage:wgpu::BufferUsages::INDEX,
    });
    let index_count=u32::try_from(indices.len()).map_err(|_|"scene has too many indices")?;
    let mut gpu_mesh_bytes=vertices.len()*size_of::<GpuVertex>()+
        indices.len()*size_of::<u32>();
    let primary_gpu_bytes=gpu_mesh_bytes;
    let neighbor=secondary_source.map(|(vertices,indices,batches,terrain,region,collision,placement,bounds)|{
        let bytes=vertices.len()*size_of::<GpuVertex>()+indices.len()*size_of::<u32>();
        gpu_mesh_bytes+=bytes;
        ResidentTile{
            vertices:device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
                label:Some("Neighbor section vertices"),
                contents:bytemuck::cast_slice(&vertices),
                usage:wgpu::BufferUsages::VERTEX,
            }),
            indices:device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
                label:Some("Neighbor section indices"),
                contents:bytemuck::cast_slice(&indices),
                usage:wgpu::BufferUsages::INDEX,
            }),
            batches,terrain,region,collision,placement,bounds,bytes
        }
    });
    set_world_telemetry(if neighbor.is_some(){2}else{1},0);
    let label=if city.region.id=="downtown-la" {
        "Downtown LA".to_owned()
    }else{city.region.name.clone()};
    let graphics=Graphics {
        window,surface,device,queue,config,surface_is_srgb,pipeline,sky_pipeline,depth,
        street_camera:camera.clone(),overview_camera,camera_request:0,
        camera,collision,terrain,region:city.region,
        camera_buffer,camera_bind_group,vertices:vertex_buffer,indices:index_buffer,index_count,
        world_preview:browser_joined_world(),primary_bounds,neighbor,
        deferred_scene,primary_gpu_bytes,
        rendered_frames:0,batches,gpu_mesh_bytes,
        active_draw_calls:0,active_triangles:0,
        gpu_fault,
        bridge_overlook,inspect_request:0,
    };
    if !progressive {set_scene_detail(if browser_joined_world(){"joined"}else{"complete"});}
    info(&format!("{} ready ({backend}){}{} — DEM terrain and building walls, WASD walk, Shift run, E/Q fly, Esc release",
        label,if browser_joined_world(){" · Joined 2-section world"}else{""},
        if progressive{" · early terrain and streets"}else{""}));
    proxy.send_event(AppEvent::Ready(graphics))
        .map_err(|_|"Viewer window closed before initialization".to_string())
}

async fn enrich_scene(job:DeferredScene,proxy:EventLoopProxy<AppEvent>) {
    // Only starts once the first real frame was submitted. The short timeout
    // gives the browser a presentation opportunity before synchronous meshing.
    #[cfg(target_arch="wasm32")]
    gloo_timers::future::TimeoutFuture::new(160).await;
    let DeferredScene{city,terrain,bridge_preview,joined}=job;
    info("Adding full buildings and landmark geometry...");
    match combined_city(&city,&terrain,bridge_preview) {
        Ok((vertices,indices,batches))=>{
            let _=proxy.send_event(AppEvent::PrimaryDetail(vertices,indices,batches));
        }
        Err(err)=>{
            set_scene_detail("error");
            info(&format!("Base terrain available; building detail failed: {err}"));
            return;
        }
    }
    if !joined{return;}
    #[cfg(target_arch="wasm32")]
    gloo_timers::future::TimeoutFuture::new(160).await;
    set_scene_detail("neighbor");
    let other_id=if city.region.id=="downtown-la"{"little-tokyo-east"}else{"downtown-la"};
    let result:Result<NeighborCpu,String>=async {
        let other=fetch_city(other_id,false).await?;
        let ground=fetch_terrain(other_id,false).await?;
        ground.validate(&other.region)?;
        let placement=urbarium::world::TilePlacement::between(
            &city.region,terrain.origin_elevation_m,
            &other.region,ground.origin_elevation_m);
        let (mut vertices,indices,batches)=combined_city(&other,&ground,bridge_preview)?;
        for vertex in &mut vertices {
            vertex.position=placement.point(
                vertex.position[0],vertex.position[1],vertex.position[2]);
        }
        let bounds=placement.bounds(&other.region);
        let obstacles=CollisionWorld::from_city_with_terrain(&other,Some(&ground));
        Ok((vertices,indices,batches,ground,other.region,obstacles,placement,bounds))
    }.await;
    match result {
        Ok(data)=>{let _=proxy.send_event(AppEvent::NeighborDetail(Box::new(data)));}
        Err(err)=>{
            set_scene_detail("error");
            info(&format!("Base city available; adjoining region failed: {err}"));
        }
    }
}

enum AppEvent {
    Ready(Graphics),
    PrimaryDetail(Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>),
    NeighborDetail(Box<NeighborCpu>),
    Error(String), PointerLock(bool),
    ViewportResize(PhysicalSize<u32>), VisibilityChanged
}
enum Stage { Boot, Ready(Graphics), Error(String) }
struct App {
    state:Stage,
    window:Option<SharedWindow>,
    keys:HashSet<KeyCode>,
    pointer_locked:bool,
    last_frame:Instant,
}
impl App {
    fn new() -> Self {
        Self{state:Stage::Boot,window:None,keys:HashSet::new(),
            pointer_locked:false,last_frame:Instant::now()}
    }
    #[cfg(not(target_arch="wasm32"))]
    fn release_mouse(&mut self) {
        if let Some(w)=&self.window {
            let _=w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
        }
        self.pointer_locked=false;
        show_mouse_state(false);
    }
}
impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self,event_loop:&ActiveEventLoop) {
        if self.window.is_some() {return;}
        // Never use the old hard-coded 1280x800 on the web: the resulting
        // canvas/WebGL drawing buffer was smaller than the browser viewport.
        #[cfg(target_arch="wasm32")]
        let initial_size=browser_viewport_size();
        #[cfg(not(target_arch="wasm32"))]
        let initial_size=PhysicalSize::new(1280,800);
        let mut attrs=Window::default_attributes()
            .with_title("Urbarium | Downtown Los Angeles")
            .with_inner_size(initial_size);
        #[cfg(target_arch="wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attrs=attrs.with_append(true);
        }
        let window=SharedWindow::new(event_loop.create_window(attrs)
            .expect("could not create Urbarium canvas"));
        #[cfg(target_arch="wasm32")]
        {
            use winit::platform::web::WindowExtWebSys;
            if let Some(canvas)=window.canvas() {canvas.set_tab_index(0);}
            install_pointerlockchange(self_proxy(event_loop));
            install_viewport_resize(self_proxy(event_loop));
            install_visibility_change(self_proxy(event_loop));
        }
        self.window=Some(window.clone());
        // Initialization is asynchronous on WebGPU browsers.
        let proxy=self_proxy(event_loop);
        #[cfg(target_arch="wasm32")]
        wasm_bindgen_futures::spawn_local(init_graphics(window,proxy));
        #[cfg(not(target_arch="wasm32"))]
        pollster::block_on(init_graphics(window,proxy));
    }
    fn user_event(&mut self,_event_loop:&ActiveEventLoop,event:AppEvent) {
        match event {
            AppEvent::Ready(gfx)=>{
                set_browser_stage("graphics-received");
                gfx.window.request_redraw();
                self.state=Stage::Ready(gfx);
            }
            AppEvent::PrimaryDetail(vertices,indices,batches)=>{
                if let Stage::Ready(gfx)=&mut self.state{
                    gfx.set_primary_mesh(vertices,indices,batches);
                    set_scene_detail(if gfx.world_preview{"neighbor"}else{"complete"});
                    gfx.window.request_redraw();
                }
            }
            AppEvent::NeighborDetail(payload)=>{
                if let Stage::Ready(gfx)=&mut self.state{
                    gfx.set_neighbor(*payload);
                    set_scene_detail("joined");
                    gfx.window.request_redraw();
                }
            }
            AppEvent::Error(msg)=>{self.state=Stage::Error(msg);}
            AppEvent::PointerLock(locked)=>{
                self.pointer_locked=locked;
                show_mouse_state(locked);
            }
            AppEvent::VisibilityChanged=>{
                if browser_page_visible(){
                    self.last_frame=Instant::now();
                    if let Some(window)=&self.window {window.request_redraw();}
                }
            }
            AppEvent::ViewportResize(size)=>{
                if let Some(window)=&self.window {
                    let _=window.request_inner_size(size);
                }
                if let Stage::Ready(gfx)=&mut self.state {
                    gfx.resize(size);
                }
            }
        }
    }
    fn window_event(&mut self,event_loop:&ActiveEventLoop,_id:WindowId,event:WindowEvent) {
        match event {
            WindowEvent::CloseRequested=>event_loop.exit(),
            WindowEvent::Resized(size)=>{
                // winit may emit an unbounded CSS pixel size after our
                // viewport resize callback. Limit BOTH the actual canvas
                // backing pixels and the wgpu surface on every path.
                let limited=limit_backing_size(size);
                if limited!=size {
                    if let Some(window)=&self.window {
                        let _=window.request_inner_size(limited);
                    }
                }
                if let Stage::Ready(gfx)=&mut self.state {gfx.resize(limited);}
            },
            WindowEvent::Focused(false)=>{
                self.keys.clear();
                #[cfg(not(target_arch="wasm32"))]
                self.release_mouse();
            },
            WindowEvent::KeyboardInput{event,..}=>{
                if let PhysicalKey::Code(key)=event.physical_key {
                    // Shift toggles fast travel on key-down, not a hold.
                    // Ignore key-repeat so one key press changes mode once.
                    if matches!(key,KeyCode::ShiftLeft|KeyCode::ShiftRight)
                        && event.state==ElementState::Pressed && !event.repeat {
                        if let Stage::Ready(gfx)=&mut self.state {
                            gfx.camera.fast_mode=!gfx.camera.fast_mode;
                            set_speed_label(gfx.camera.fast_mode);
                        }
                    }
                    if event.state==ElementState::Pressed {
                        self.keys.insert(key);
                    } else {self.keys.remove(&key);}
                    if key==KeyCode::Escape {
                        #[cfg(not(target_arch="wasm32"))]
                        self.release_mouse();
                        #[cfg(target_arch="wasm32")]
                        if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
                            // Explicitly release even in automated Chromium sessions.
                            // The pointerlockchange listener then updates UI/input.
                            doc.exit_pointer_lock();
                        }
                    }
                }
            },
            WindowEvent::MouseInput{
                button:MouseButton::Left,state:ElementState::Pressed,..
            }=>{
                if let Some(w)=&self.window {
                    // Browser calls here happen inside the required user gesture.
                    if w.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
                        w.set_cursor_visible(false);
                        #[cfg(not(target_arch="wasm32"))]
                        {
                            self.pointer_locked=true;
                            show_mouse_state(true);
                        }
                    } else {
                        info("Pointer lock unavailable. Click inside the city to retry.");
                    }
                }
            },
            WindowEvent::RedrawRequested=>{
                if !browser_page_visible(){return;}
                // A distinct low-workload diagnostic mode lets us isolate
                // GPU pressure from failures intrinsic to the scene/driver.
                if browser_safe_gpu() &&
                    self.last_frame.elapsed().as_secs_f32()<1.0/30.0 {return;}
                let now=Instant::now();
                let interval=self.last_frame.elapsed().as_secs_f32();
                self.last_frame=now;
                if let Stage::Ready(gfx)=&mut self.state {
                    if let Some(error)=gfx.gpu_fault.message(){
                        expose_gpu_fault(
                            &error,gfx.gpu_fault.lost.load(Ordering::Acquire));
                    }
                    if gfx.gpu_fault.lost.load(Ordering::Acquire) {
                        set_browser_stage("device-lost");
                        return;
                    }
                    gfx.update(&self.keys,interval.min(0.08));
                    let rendered=gfx.draw();
                    if rendered && gfx.rendered_frames == 1 {
                        mark_first_frame_detail();
                        set_browser_stage("first-present");
                        // Delay enrichment until a real terrain + road frame
                        // has been submitted. The old path blocked the first
                        // frame on two complete sections.
                        if let Some(job)=gfx.deferred_scene.take(){
                            let proxy=self_proxy(event_loop);
                            #[cfg(target_arch="wasm32")]
                            wasm_bindgen_futures::spawn_local(enrich_scene(job,proxy));
                            #[cfg(not(target_arch="wasm32"))]
                            pollster::block_on(enrich_scene(job,proxy));
                        }
                    }
                    // One compact pose attribute every frame gives the
                    // 2D minimap and world-space landmark labels 60Hz
                    // camera tracking without redrawing a GIS scene or
                    // setting five separate attributes per frame.
                    expose_camera_pose_fast(&gfx.camera);
                    // Heavier stats/compatibility attributes remain throttled.
                    if gfx.rendered_frames % 8 == 0 {
                        set_browser_frame_count(gfx.rendered_frames);
                        expose_camera_pose(&gfx.camera);
                        expose_gpu_metrics(
                            gfx.gpu_mesh_bytes,gfx.active_draw_calls,
                            gfx.active_triangles,interval*1000.0,
                            now.elapsed().as_secs_f32()*1000.0,
                        );
                    }
                }
            },
            _=>{}
        }
    }
    fn device_event(&mut self,_event_loop:&ActiveEventLoop,_id:DeviceId,event:DeviceEvent) {
        if let DeviceEvent::MouseMotion{delta:(dx,dy)}=event {
            if self.pointer_locked {
                if let Stage::Ready(gfx)=&mut self.state {
                    gfx.camera.mouse_look(dx as f32,dy as f32);
                }
            }
        }
    }
    fn about_to_wait(&mut self,_event_loop:&ActiveEventLoop) {
        // Browser visibility must gate redraw. Running a perpetual Poll event
        // loop in a hidden tab was wasting GPU/CPU and exacerbating instability.
        if browser_page_visible() {
            if let Stage::Ready(gfx)=&self.state {
                if !gfx.gpu_fault.lost.load(Ordering::Acquire) {
                    gfx.window.request_redraw();
                }
            }
        }
    }
}

fn browser_bridge_preview()->bool {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        return doc.document_element()
            .and_then(|el|el.get_attribute("data-bridge-preview"))
            .as_deref()==Some("true");
    }
    // Native and offline exports retain explicit detailed-bridge behavior.
    false
}
fn set_bridge_preview_state(preview:bool){
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()){
        if let Some(el)=doc.get_element_by_id("status"){
            let _=el.set_attribute("data-bridge-preview-active",
                if preview{"true"}else{"false"});
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=preview;
}
fn active_pixel_budget()->u64 {
    #[cfg(target_arch="wasm32")]
    {
        let desired=web_sys::window().and_then(|w|w.document())
            .and_then(|d|d.document_element())
            .and_then(|el|el.get_attribute("data-render-pixel-budget"))
            .and_then(|value|value.parse::<u64>().ok())
            .unwrap_or(1_800_000).clamp(400_000,1_800_000);
        return if browser_safe_gpu(){desired.min(640_000)}else{desired};
    }
    #[cfg(not(target_arch="wasm32"))]
    1_800_000
}
fn limit_backing_size(size:PhysicalSize<u32>)->PhysicalSize<u32>{
    #[cfg(target_arch="wasm32")]
    {
        let pixels=size.width.max(1) as u64*size.height.max(1) as u64;
        let budget=active_pixel_budget();
        if pixels>budget {
            let scale=(budget as f64/pixels as f64).sqrt();
            return PhysicalSize::new(
                ((size.width as f64*scale).floor() as u32).max(1),
                ((size.height as f64*scale).floor() as u32).max(1),
            );
        }
    }
    size
}
fn browser_allows_software_webgpu()->bool {
    #[cfg(target_arch="wasm32")]
    return web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.document_element())
        .and_then(|el|el.get_attribute("data-allow-software-webgpu"))
        .as_deref()==Some("true");
    #[cfg(not(target_arch="wasm32"))]
    false
}
fn browser_safe_gpu()->bool {
    #[cfg(target_arch="wasm32")]
    return web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.document_element())
        .and_then(|el|el.get_attribute("data-safe-gpu"))
        .is_some_and(|v|v=="true");
    #[cfg(not(target_arch="wasm32"))]
    false
}
fn expose_gpu_fault(message:&str,lost:bool) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()){
        if let Some(el)=doc.get_element_by_id("status"){
            let _=el.set_attribute("data-gpu-fault",message);
            let _=el.set_attribute("data-gpu-device-lost",
                if lost{"true"}else{"false"});
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=(message,lost);
}
fn browser_page_visible()->bool {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        return !doc.hidden();
    }
    true
}
#[cfg(target_arch="wasm32")]
fn install_visibility_change(proxy:EventLoopProxy<AppEvent>) {
    use wasm_bindgen::{closure::Closure,JsCast};
    let Some(doc)=web_sys::window().and_then(|w|w.document()) else{return;};
    let callback=Closure::<dyn FnMut(web_sys::Event)>::new(move |_|{
        let _=proxy.send_event(AppEvent::VisibilityChanged);
    });
    let _=doc.add_event_listener_with_callback(
        "visibilitychange",callback.as_ref().unchecked_ref());
    callback.forget();
}
#[cfg(not(target_arch="wasm32"))]
fn browser_camera_request()->(u32,bool) {(0,false)}
#[cfg(target_arch="wasm32")]
fn browser_camera_request()->(u32,bool) {
    let el=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status"));
    let Some(el)=el else {return (0,false)};
    let number=el.get_attribute("data-camera-request")
        .and_then(|s|s.parse::<u32>().ok()).unwrap_or(0);
    let overview=el.get_attribute("data-camera-request-mode").as_deref()==Some("overview");
    (number,overview)
}
fn set_camera_mode(mode:&str){
    #[cfg(target_arch="wasm32")]
    if let Some(el)=web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status")) {
        let _=el.set_attribute("data-camera-mode",mode);
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=mode;
}
#[cfg(not(target_arch="wasm32"))]
fn browser_inspect_overpass_request()->u32 {0}
#[cfg(target_arch="wasm32")]
fn browser_inspect_overpass_request()->u32 {
    web_sys::window().and_then(|w|w.document())
        .and_then(|d|d.get_element_by_id("status"))
        .and_then(|e|e.get_attribute("data-inspect-overpass-request"))
        .and_then(|s|s.parse().ok()).unwrap_or(0)
}
// An inspection camera proves the visible deck elevation at a source-tagged
// location. It does not imply walkable bridge collision or surveyed altitude.
fn bridge_overlook(city:&City,terrain:&TerrainGrid,collision:&CollisionWorld)->Option<Camera>{
    let [west,south,east,north]=city.region.bbox;
    let sw=city.region.project(west,south);
    let ne=city.region.project(east,north);
    let mut candidates=Vec::new();
    for street in &city.streets{
        let is_bridge=street.tags.get("bridge").is_some_and(|v|v=="yes") ||
            street.tags.get("layer").and_then(|v|v.parse::<i32>().ok())
                .is_some_and(|n|n>0);
        if !is_bridge||street.tags.get("tunnel").is_some_and(|v|v=="yes"){continue;}
        for pair in street.path.windows(2){
            let length=(pair[1].x-pair[0].x).hypot(pair[1].z-pair[0].z);
            if length<4.0 {continue;}
            let mid=Point{x:(pair[0].x+pair[1].x)*0.5,
                          z:(pair[0].z+pair[1].z)*0.5};
            let margin=(mid.x-sw.x).min(ne.x-mid.x)
                .min(mid.z-sw.z).min(ne.z-mid.z);
            let score=length.min(130.0)+margin.clamp(0.0,180.0)*0.10;
            candidates.push((score,pair[0],pair[1],street,mid,length));
        }
    }
    candidates.sort_by(|a,b|b.0.total_cmp(&a.0));
    for (_,a,b,street,mid,len) in candidates{
        let normal=Point{x:-(b.z-a.z)/len,z:(b.x-a.x)/len};
        let level=street.tags.get("layer")
            .and_then(|v|v.parse::<f32>().ok()).unwrap_or(1.0).clamp(1.0,3.0);
        let deck_y=terrain.sample_surface(&city.region,mid.x,mid.z)+
            5.8*level+0.24;
        // A high three-quarter view shows both the deck and the terrain
        // beneath it. The earlier eye-level perch could show only flat
        // concrete or no city geometry at all in software Chromium.
        for side in [1.0,-1.0]{
            let distance=72.0;
            let x=mid.x+normal.x*distance*side;
            let z=mid.z+normal.z*distance*side;
            if x<sw.x+8.0||x>ne.x-8.0||z<sw.z+8.0||z>ne.z-8.0 {continue;}
            let ground=terrain.sample_surface(&city.region,x,z);
            let eye=ground+66.0;
            if collision.blocked(x,z,eye as f64,0.5){continue;}
            let yaw=((mid.x-x) as f32).atan2((mid.z-z) as f32);
            let horizontal=(mid.x-x).hypot(mid.z-z) as f32;
            let pitch=((deck_y-eye)/horizontal.max(0.1)).atan();
            return Some(Camera{
                position:Vec3::new(x as f32,eye,z as f32),
                yaw,pitch,eye_clearance:66.0,speed:3.4,fast_mode:false,
            });
        }
    }
    None
}
#[cfg(target_arch="wasm32")]
fn browser_viewport_size() -> PhysicalSize<u32> {
    if let Some(win)=web_sys::window() {
        let width=win.inner_width().ok().and_then(|v|v.as_f64()).unwrap_or(1280.0);
        let height=win.inner_height().ok().and_then(|v|v.as_f64()).unwrap_or(800.0);
        // Bound framebuffer/depth texture footprint on HiDPI fullscreen Edge.
        // CSS viewport remains full-width; only drawing resolution scales.
        let pixel_budget=active_pixel_budget() as f64;
        let max_scale=(pixel_budget/(width.max(1.0)*height.max(1.0))).sqrt();
        let dpr=win.device_pixel_ratio().min(2.0).min(max_scale).max(0.05);
        return PhysicalSize::new(
            (width*dpr).round().clamp(1.0,8192.0) as u32,
            (height*dpr).round().clamp(1.0,8192.0) as u32);
    }
    PhysicalSize::new(1280,800)
}

#[cfg(target_arch="wasm32")]
fn install_viewport_resize(proxy:EventLoopProxy<AppEvent>) {
    use wasm_bindgen::{closure::Closure,JsCast};
    let Some(win)=web_sys::window() else {return};
    let callback=Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let _=proxy.send_event(AppEvent::ViewportResize(browser_viewport_size()));
    });
    let _=win.add_event_listener_with_callback("resize",callback.as_ref().unchecked_ref());
    callback.forget();
}

fn expose_surface_size(width:u32,height:u32) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-surface-width",&width.to_string());
            let _=el.set_attribute("data-surface-height",&height.to_string());
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=(width,height);
}

#[cfg(target_arch="wasm32")]
fn install_pointerlockchange(proxy:EventLoopProxy<AppEvent>) {
    use wasm_bindgen::{closure::Closure,JsCast};
    let Some(doc)=web_sys::window().and_then(|w|w.document()) else {return};
    let observed=doc.clone();
    let callback=Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let _=proxy.send_event(AppEvent::PointerLock(
            observed.pointer_lock_element().is_some()
        ));
    });
    let _=doc.add_event_listener_with_callback(
        "pointerlockchange",callback.as_ref().unchecked_ref()
    );
    callback.forget();
}
fn set_speed_label(fast:bool) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("speed-state") {
            el.set_text_content(Some(if fast {
                "FAST · 61 m/s"
            }else{
                "WALK · 3.4 m/s"
            }));
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=fast;
}
fn expose_gpu_metrics(bytes:usize,draws:u32,triangles:u32,
    frame_ms:f32,cpu_ms:f32) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            for (name,value) in [
                ("data-gpu-mesh-bytes",bytes.to_string()),
                ("data-draw-calls",draws.to_string()),
                ("data-triangles-drawn",triangles.to_string()),
                ("data-frame-ms",format!("{frame_ms:.2}")),
                // This measures CPU-side update, encoding, and submit
                // calls only. GPU work is asynchronous and not included.
                ("data-cpu-frame-ms",format!("{cpu_ms:.2}")),
            ] {
                let _=el.set_attribute(name,&value);
            }
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=(bytes,draws,triangles,frame_ms,cpu_ms);
}
fn expose_camera_pose_fast(camera:&Camera) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let p=&camera.position;
            let _=el.set_attribute("data-camera-pose",
                &format!("{:.3},{:.3},{:.3},{:.5},{:.5}",
                    p.x,p.y,p.z,camera.yaw,camera.pitch));
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=camera;
}
fn expose_camera_pose(camera:&Camera) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-camera-y",&camera.position.y.to_string());
            let _=el.set_attribute("data-eye-clearance",&camera.eye_clearance.to_string());
            let _=el.set_attribute("data-camera-yaw",&camera.yaw.to_string());
            let _=el.set_attribute("data-camera-x",&camera.position.x.to_string());
            let _=el.set_attribute("data-camera-z",&camera.position.z.to_string());
            let _=el.set_attribute("data-speed-mode",if camera.fast_mode {"fast"} else {"walk"});
            let _=el.set_attribute("data-speed-mps",&(camera.speed *
                if camera.fast_mode {18.0}else{1.0}).to_string());
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=camera;
}
fn show_mouse_state(locked:bool) {
    #[cfg(target_arch="wasm32")]
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("input-state") {
            el.set_text_content(Some(if locked {
                "Mouse captured — Esc releases"
            } else {
                "Click anywhere on the city to capture mouse"
            }));
            let _=el.set_attribute("data-pointer-locked",if locked {"true"} else {"false"});
        }
    }
    #[cfg(not(target_arch="wasm32"))]
    let _=locked;
}

// Store the event loop proxy in a thread-local once at startup; ApplicationHandler
// does not itself provide the proxy in resumed().
thread_local! {
    static PROXY: std::cell::RefCell<Option<EventLoopProxy<AppEvent>>> =
        const { std::cell::RefCell::new(None) };
}
fn self_proxy(_: &ActiveEventLoop) -> EventLoopProxy<AppEvent> {
    PROXY.with(|cell|cell.borrow().as_ref().expect("viewer proxy set in main").clone())
}

fn main() {
    #[cfg(target_arch="wasm32")]
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    let event_loop=EventLoop::<AppEvent>::with_user_event().build()
        .expect("window event loop");
    // Poll busy-loops on browsers; Wait lets request_redraw pace frames.
    #[cfg(target_arch="wasm32")]
    event_loop.set_control_flow(ControlFlow::Wait);
    #[cfg(not(target_arch="wasm32"))]
    event_loop.set_control_flow(ControlFlow::Poll);
    PROXY.with(|cell|*cell.borrow_mut()=Some(event_loop.create_proxy()));
    #[cfg(target_arch="wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(App::new());
    }
    #[cfg(not(target_arch="wasm32"))]
    event_loop.run_app(&mut App::new()).expect("event loop failure");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn right_mouse_turns_clockwise_from_geographic_north() {
        let mut camera=Camera::default();
        camera.yaw=0.0; // North: +Z, east: +X.
        camera.mouse_look(100.0,0.0);
        assert!(camera.yaw > 0.0);
        assert!(camera.forward().x > 0.0,
            "right mouse must turn toward geographic east");
        camera.mouse_look(-200.0,0.0);
        assert!(camera.forward().x < 0.0,
            "left mouse must turn toward geographic west");
        assert!((camera.position.y-1.7).abs()<f32::EPSILON);
        assert!(camera.speed < 5.0);
    }
    #[test]
    fn east_is_to_screen_right_when_facing_north() {
        let mut camera=Camera::default();
        camera.yaw=0.0;
        let vp=camera.view_projection(1600,900);
        let east=vp.project_point3(camera.position+Vec3::new(10.0,0.0,30.0));
        let west=vp.project_point3(camera.position+Vec3::new(-10.0,0.0,30.0));
        assert!(east.x>0.0 && west.x<0.0,
            "camera and map must agree that east is right of north");
        assert!((east.x+west.x).abs()<0.001);
    }
    #[test]
    fn wasd_strafing_uses_geographic_right() {
        let mut cam=Camera::default();
        cam.position=Vec3::new(0.0,1.7,0.0);
        cam.yaw=0.0;
        let mut keys=HashSet::new();
        keys.insert(KeyCode::KeyD);
        cam.move_by(&keys,0.04,None,None);
        assert!(cam.position.x>0.0,"D while facing north must move east");
        assert!(cam.position.z.abs()<1e-4);
        keys.clear();keys.insert(KeyCode::KeyA);
        let prior=cam.position.x;
        cam.move_by(&keys,0.04,None,None);
        assert!(cam.position.x<prior,"A while facing north must move west");
    }

    #[test]
    fn fast_travel_is_over_ten_times_walking_speed() {
        let mut walk=Camera::default();
        let mut fast=Camera::default();
        fast.fast_mode=true;
        let mut keys=HashSet::new();
        keys.insert(KeyCode::KeyW);
        walk.move_by(&keys,0.05,None,None);
        fast.move_by(&keys,0.05,None,None);
        let walk_distance=(walk.position.x*walk.position.x+
            walk.position.z*walk.position.z).sqrt();
        let fast_distance=(fast.position.x*fast.position.x+
            fast.position.z*fast.position.z).sqrt();
        assert!(fast_distance>walk_distance*15.0);
        assert_eq!(fast.eye_clearance,1.7);
    }

    #[test]
    fn flight_stays_at_absolute_altitude_across_uphill_and_downhill_dem() {
        let region=Region {
            id:"slope".into(),name:"Slope".into(),
            origin:[0.0,0.0],bbox:[-0.01,-0.01,0.01,0.01],
        };
        let grid=TerrainGrid {
            schema_version:1,region_id:region.id.clone(),
            bbox:region.bbox,width:2,height:2,
            heights_m:vec![0.0,0.0,100.0,100.0],
            origin_elevation_m:0.0,source:"synthetic".into(),
        };
        let mut camera=Camera::default();
        camera.yaw=0.0;
        camera.position.y=grid.sample_surface(&region,0.0,0.0)+100.0;
        camera.eye_clearance=100.0;
        let altitude=camera.position.y;
        let mut keys=HashSet::new();
        keys.insert(KeyCode::KeyW);
        for _ in 0..50 {camera.move_by(&keys,0.08,None,Some((&grid,&region)));}
        assert!(camera.position.z>10.0);
        assert!((camera.position.y-altitude).abs()<0.001,
            "horizontal flight must not descend or climb with the terrain");
        keys.clear();keys.insert(KeyCode::KeyS);
        for _ in 0..50 {camera.move_by(&keys,0.08,None,Some((&grid,&region)));}
        assert!((camera.position.y-altitude).abs()<0.001,
            "return flight must not produce the reverse altitude anomaly");
    }

    #[test]
    fn walking_keeps_ground_clearance_and_flight_resumes_walking_on_landing() {
        let region=Region{
            id:"slope".into(),name:"Slope".into(),
            origin:[0.0,0.0],bbox:[-0.01,-0.01,0.01,0.01],
        };
        let grid=TerrainGrid{
            schema_version:1,region_id:region.id.clone(),
            bbox:region.bbox,width:2,height:2,
            heights_m:vec![0.0,0.0,100.0,100.0],
            origin_elevation_m:0.0,source:"synthetic".into(),
        };
        let mut camera=Camera::default();
        camera.yaw=0.0;
        camera.position.y=grid.sample_surface(&region,0.0,0.0)+1.7;
        let initial=camera.position.y;
        let mut keys=HashSet::new();
        keys.insert(KeyCode::KeyW);
        for _ in 0..50 {camera.move_by(&keys,0.08,None,Some((&grid,&region)));}
        assert!(camera.position.y>initial+0.2,
            "walking must still follow the uphill DEM");
        assert!((camera.eye_clearance-1.7).abs()<0.001);
        keys.clear();keys.insert(KeyCode::KeyE);
        for _ in 0..20 {camera.move_by(&keys,0.08,None,Some((&grid,&region)));}
        assert!(camera.eye_clearance>2.0);
        keys.clear();keys.insert(KeyCode::KeyQ);
        for _ in 0..100 {camera.move_by(&keys,0.08,None,Some((&grid,&region)));}
        assert!((camera.eye_clearance-1.7).abs()<0.001,
            "landing must restore the normal walking camera");
    }

    #[test]
    fn camera_cannot_descend_below_eye_height() {
        let mut camera=Camera::default();
        let mut keys=HashSet::new();
        keys.insert(KeyCode::KeyQ);
        camera.move_by(&keys,1.0,None,None);
        assert!((camera.eye_clearance-1.7).abs()<0.001);
    }
    #[test]
    fn camera_moves_and_pitch_stays_bounded() {
        let mut camera=Camera::default();
        let before=camera.position;
        let mut keys=HashSet::new();
        keys.insert(KeyCode::KeyW);
        camera.move_by(&keys,0.04,None,None);
        assert_ne!(camera.position,before);
        camera.mouse_look(0.0,1e6);
        assert!(camera.pitch >= -1.52);
    }
}
