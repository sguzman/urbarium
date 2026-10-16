//! First WASM/WebGPU city viewer: actual procedural Urbarium meshes, no JS geometry.
//! FPS pointer lock, WASD walk, E/Q fly, Shift accelerates.

use std::collections::HashSet;
use std::mem::size_of;
use web_time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use urbarium::geometry::{mesh_building, HeightPolicy, Mesh, Vertex};
use urbarium::roads::{mesh_streets, SurfaceKind};
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
    fn move_by(&mut self, pressed:&HashSet<KeyCode>, elapsed:f32,
        world:Option<&CollisionWorld>, terrain:Option<(&TerrainGrid,&Region)>) {
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
            let movement=delta.normalize() * elapsed.min(0.08) * self.speed * boost;
            self.eye_clearance=(self.eye_clearance+movement.y).max(1.7);
            let start=[self.position.x as f64,self.position.z as f64];
            let end=[start[0]+movement.x as f64,start[1]+movement.z as f64];
            let ground_at=|x:f64,z:f64| -> f32 {
                terrain.map(|(grid,region)| grid.sample_surface(region,x,z)).unwrap_or(0.0)
            };
            let eye=ground_at(end[0],end[1])+self.eye_clearance;
            let resolved=if let Some(world)=world {
                world.move_sliding(start,end,eye as f64,0.35)
            }else{end};
            self.position.x=resolved[0] as f32;
            self.position.z=resolved[1] as f32;
            self.position.y=ground_at(resolved[0],resolved[1])+self.eye_clearance;
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
fn combined_city(city:&City, terrain:&TerrainGrid) -> Result<(Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>),String> {
    let catalog:LandmarkCatalog=serde_json::from_str(include_str!("../../web/landmarks.json"))
        .map_err(|e|format!("bundled landmark catalog: {e}"))?;
    if catalog.region_id!=city.region.id {
        return Err("landmark catalog must match the current region".into());
    }
    let (buildings,building_report)=terrain.mesh_buildings(city,HeightPolicy::default())?;
    let (mut roads,road_report)=mesh_streets(city)?;
    terrain.drape_roads(&city.region,&mut roads)?;
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
    for (kind,mesh) in roads.layers {
        let (layer,color)=match kind {
            SurfaceKind::Road => (SceneLayer::Road,[0.24,0.26,0.29]),
            SurfaceKind::Motorway => (SceneLayer::Road,[0.14,0.19,0.23]),
            SurfaceKind::MotorwayStripe|SurfaceKind::BridgeStripe =>
                (SceneLayer::Road,[0.91,0.89,0.77]),
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
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function publish_city_for_minimap(data) {
    window.__urbariumCityJson = data;
    window.dispatchEvent(new Event('urbarium-city-ready'));
}
export function publish_terrain_for_minimap(data) {
    window.__urbariumTerrainJson = data;
    window.dispatchEvent(new Event('urbarium-terrain-ready'));
}
")]
extern "C" {
    fn publish_city_for_minimap(data:&str);
    fn publish_terrain_for_minimap(data:&str);
}

async fn fetch_city() -> Result<City, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let response = gloo_net::http::Request::get("./assets/downtown-la.json")
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
        publish_city_for_minimap(&data);
        Ok(city)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let data=std::fs::read_to_string("data/processed/downtown-la.json")
            .map_err(|e| format!("native viewer requires data/processed/downtown-la.json: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("city JSON: {e}"))
    }
}

#[cfg(target_arch="wasm32")]
fn set_elevated_span_count(count:usize) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
            let _=el.set_attribute("data-elevated-road-spans",&count.to_string());
        }
        if let Some(button)=doc.get_element_by_id("inspect-overpass") {
            button.set_text_content(Some(&format!("Inspect overpass ({count})")));
            if count>0 {let _=button.remove_attribute("disabled");}
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
        }
    }
}
#[cfg(not(target_arch="wasm32"))]
fn set_browser_backend(_: &str) {}

#[cfg(target_arch="wasm32")]
fn set_browser_stage(stage: &str) {
    if let Some(doc)=web_sys::window().and_then(|w|w.document()) {
        if let Some(el)=doc.get_element_by_id("status") {
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

async fn fetch_terrain() -> Result<TerrainGrid,String> {
    #[cfg(target_arch="wasm32")]
    {
        let response=gloo_net::http::Request::get("./assets/downtown-la-terrain.json")
            .send().await.map_err(|e|format!("DEM fetch: {e}"))?;
        if !response.ok(){return Err(format!("DEM returned HTTP {}",response.status()));}
        set_browser_stage("terrain-decoding");
        info("Downloaded/read elevation data; decoding terrain...");
        let data=response.text().await.map_err(|e|format!("DEM body: {e}"))?;
        let terrain:TerrainGrid=serde_json::from_str(&data)
            .map_err(|e|format!("DEM JSON: {e}"))?;
        // Share the exact DEM grid as well; labels and scene use one datum.
        publish_terrain_for_minimap(&data);
        Ok(terrain)
    }
    #[cfg(not(target_arch="wasm32"))]
    {
        let data=std::fs::read_to_string("data/processed/downtown-la-terrain.json")
            .map_err(|e|format!("native viewer requires a terrain grid: {e}"))?;
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

struct Graphics {
    window: SharedWindow,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    depth: wgpu::TextureView,
    camera: Camera,
    collision: CollisionWorld,
    terrain: TerrainGrid,
    region: Region,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    batches:Vec<DrawBatch>,
    rendered_frames: u64,
    gpu_mesh_bytes: usize,
    active_draw_calls:u32,
    active_triangles:u32,
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
        self.config.width=size.width.max(1);
        self.config.height=size.height.max(1);
        self.surface.configure(&self.device,&self.config);
        self.depth=Self::depth_view(&self.device,self.config.width,self.config.height);
        expose_surface_size(self.config.width,self.config.height);
    }
    fn resize(&mut self,size:PhysicalSize<u32>) {
        if self.config.width==size.width.max(1) &&
            self.config.height==size.height.max(1) {return;}
        self.reconfigure_surface(size);
    }
    fn update(&mut self, pressed: &HashSet<KeyCode>, dt: f32) {
        let request=browser_inspect_overpass_request();
        if request!=self.inspect_request {
            self.inspect_request=request;
            if let Some(camera)=self.bridge_overlook.clone() {
                self.camera=camera;
                self.camera.fast_mode=false;
                set_speed_label(false);
            }
        }
        self.camera.move_by(pressed,dt,Some(&self.collision),Some((&self.terrain,&self.region)));
        let vp=self.camera.view_projection(self.config.width,self.config.height);
        self.queue.write_buffer(&self.camera_buffer,0,
            bytemuck::bytes_of(&CameraUniform{
                view_proj:vp.to_cols_array_2d(),
                options:[
                    1.0,browser_sky_hour(),self.camera.yaw,self.camera.pitch],
                sky:[self.config.width as f32/self.config.height.max(1) as f32,
                     browser_category_mask() as f32,0.0,0.0],
            }));
    }
    fn draw(&mut self) -> bool {
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
    info("Loading Downtown LA streets and buildings...");
    let city=fetch_city().await?;
    set_browser_stage("terrain-fetch");
    info("Loading Downtown LA elevation grid...");
    let terrain=fetch_terrain().await?;
    set_browser_stage("geometry-build");
    info("Generating 3D terrain, buildings and streets...");
    terrain.validate(&city.region)?;
    set_terrain_size(terrain.heights_m.len());
    let (vertices,indices,batches)=combined_city(&city,&terrain)?;
    let bridge_overlook=bridge_overlook(&city,&terrain);
    set_browser_stage("gpu-init");
    info("Geography ready; initializing WebGPU or WebGL2...");
    let instance=wgpu::util::new_instance_with_webgpu_detection(
        wgpu::InstanceDescriptor::new_without_display_handle()).await;
    let surface=instance.create_surface(window.clone())
        .map_err(|e|format!("surface: {e}"))?;
    let adapter=instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference:wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter:false,
        compatible_surface:Some(&surface),
        apply_limit_buckets:false,
    }).await.map_err(|e|format!("Neither WebGPU nor WebGL2 could initialize: {e}. Check browser WebGL2 support and hardware acceleration."))?;
    let backend = format!("{:?}",adapter.get_info().backend);
    set_browser_backend(&backend);
    let (device,queue)=adapter.request_device(&wgpu::DeviceDescriptor {
        label:Some("Urbarium device"),
        required_features:wgpu::Features::empty(),
        required_limits:wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        memory_hints:wgpu::MemoryHints::Performance,
        trace:Default::default(),
        experimental_features:Default::default(),
    }).await.map_err(|e|format!("Graphics device ({backend}): {e}"))?;
    let size=window.inner_size();
    let mut config=surface.get_default_config(&adapter,size.width.max(1),size.height.max(1))
        .ok_or("No compatible GPU surface configuration")?;
    expose_surface_size(config.width,config.height);
    config.present_mode=wgpu::PresentMode::Fifo;
    surface.configure(&device,&config);
    let collision=CollisionWorld::from_city_with_terrain(&city,Some(&terrain));
    set_collision_count(collision.obstacle_count());
    let camera=Camera::spawn_at_street(&city,&collision,&terrain);
    let camera_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:Some("Camera uniform"),
        contents:bytemuck::bytes_of(&CameraUniform {
            view_proj:camera.view_projection(config.width,config.height).to_cols_array_2d(),
            options:[0.0,12.0,camera.yaw,camera.pitch],
            sky:[config.width as f32/config.height.max(1) as f32,0.0,0.0,0.0],
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
    let gpu_mesh_bytes=vertices.len()*size_of::<GpuVertex>()+
        indices.len()*size_of::<u32>();
    let graphics=Graphics {
        window,surface,device,queue,config,pipeline,sky_pipeline,depth,camera,collision,terrain,region:city.region,
        camera_buffer,camera_bind_group,vertices:vertex_buffer,indices:index_buffer,index_count,
        rendered_frames:0,batches,gpu_mesh_bytes,
        active_draw_calls:0,active_triangles:0,
        bridge_overlook,inspect_request:0,
    };
    info(&format!("Downtown LA ready ({backend}) — DEM terrain and building walls, WASD walk, Shift run, E/Q fly, Esc release"));
    proxy.send_event(AppEvent::Ready(graphics))
        .map_err(|_|"Viewer window closed before initialization".to_string())
}

enum AppEvent { Ready(Graphics), Error(String), PointerLock(bool), ViewportResize(PhysicalSize<u32>), VisibilityChanged }
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
                if let Stage::Ready(gfx)=&mut self.state {gfx.resize(size);}
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
                let now=Instant::now();
                let interval=self.last_frame.elapsed().as_secs_f32();
                self.last_frame=now;
                if let Stage::Ready(gfx)=&mut self.state {
                    gfx.update(&self.keys,interval.min(0.08));
                    let rendered=gfx.draw();
                    if rendered && gfx.rendered_frames == 1 {
                        set_browser_stage("first-present");
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
            if let Stage::Ready(gfx)=&self.state {gfx.window.request_redraw();}
        }
    }
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
fn bridge_overlook(city:&City,terrain:&TerrainGrid)->Option<Camera> {
    let mut best:Option<(&urbarium::Street,Point,Point,f64)>=None;
    for street in &city.streets{
        let is_bridge=street.tags.get("bridge").is_some_and(|v|v=="yes") ||
            street.tags.get("layer").and_then(|v|v.parse::<i32>().ok())
                .is_some_and(|n|n>0);
        if !is_bridge || street.tags.get("tunnel").is_some_and(|v|v=="yes"){continue;}
        for pair in street.path.windows(2){
            let len=(pair[1].x-pair[0].x).hypot(pair[1].z-pair[0].z);
            if len>best.as_ref().map_or(0.0,|b|b.3){
                best=Some((street,pair[0],pair[1],len));
            }
        }
    }
    let (street,a,b,len)=best?;
    if len<1.0 {return None;}
    let mid=Point{x:(a.x+b.x)*0.5,z:(a.z+b.z)*0.5};
    let perp=Point{x:-(b.z-a.z)/len,z:(b.x-a.x)/len};
    let side=24.0;
    let x=mid.x+perp.x*side;
    let z=mid.z+perp.z*side;
    let ground=terrain.sample_surface(&city.region,x,z);
    let level=street.tags.get("layer")
        .and_then(|v|v.parse::<f32>().ok()).unwrap_or(1.0).clamp(1.0,3.0);
    let deck_y=terrain.sample_surface(&city.region,mid.x,mid.z)+5.8*level+0.24;
    let position=Vec3::new(x as f32,ground+2.8,z as f32);
    let yaw=((mid.x-x) as f32).atan2((mid.z-z) as f32);
    let pitch=((deck_y-position.y)/side as f32).atan();
    Some(Camera{position,yaw,pitch,eye_clearance:2.8,
        speed:3.4,fast_mode:false})
}
#[cfg(target_arch="wasm32")]
fn browser_viewport_size() -> PhysicalSize<u32> {
    if let Some(win)=web_sys::window() {
        let width=win.inner_width().ok().and_then(|v|v.as_f64()).unwrap_or(1280.0);
        let height=win.inner_height().ok().and_then(|v|v.as_f64()).unwrap_or(800.0);
        // Bound framebuffer/depth texture footprint on HiDPI fullscreen Edge.
        // CSS viewport remains full-width; only drawing resolution scales.
        let pixel_budget=1_800_000.0;
        let max_scale=(pixel_budget/(width.max(1.0)*height.max(1.0))).sqrt();
        let dpr=win.device_pixel_ratio().min(2.0).min(max_scale).max(0.45);
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
