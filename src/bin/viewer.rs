//! First WASM/WebGPU city viewer: actual procedural Urbarium meshes, no JS geometry.
//! FPS pointer lock, WASD walk, E/Q fly, Shift accelerates.

use std::collections::HashSet;
use std::mem::size_of;
use web_time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use urbarium::geometry::{HeightPolicy, Mesh};
use urbarium::roads::{mesh_streets, SurfaceKind};
use urbarium::City;
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
}
const VERTEX_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform { view_proj: [[f32; 4]; 4] }

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
        let view = Mat4::look_to_rh(self.position, self.forward(), Vec3::Y);
        let projection = Mat4::perspective_rh(68.0_f32.to_radians(), aspect, 0.06, 4500.0);
        projection * view
    }
    fn mouse_look(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.0025;
        self.pitch = (self.pitch - dy * 0.0025).clamp(-1.52, 1.52);
    }
    fn move_by(&mut self, pressed:&HashSet<KeyCode>, elapsed:f32,
        world:Option<&CollisionWorld>, terrain:Option<(&TerrainGrid,&Region)>) {
        let ground_forward = Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos());
        let right = Vec3::new(-ground_forward.z, 0.0, ground_forward.x);
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
    }));
    indices.extend(mesh.indices.iter().map(|idx| idx+base));
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SceneLayer { Ground, Buildings, Road, Walk, Steps, Rail }
#[derive(Clone, Copy)]
struct DrawBatch {
    layer: SceneLayer,
    first_index: u32,
    count: u32,
}
fn combined_city(city:&City, terrain:&TerrainGrid) -> Result<(Vec<GpuVertex>,Vec<u32>,Vec<DrawBatch>),String> {
    let (buildings,building_report)=terrain.mesh_buildings(city,HeightPolicy::default())?;
    let (mut roads,road_report)=mesh_streets(city)?;
    terrain.drape_roads(&city.region,&mut roads);
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
            SurfaceKind::Walk => (SceneLayer::Walk,[0.60,0.58,0.51]),
            SurfaceKind::Steps => (SceneLayer::Steps,[0.74,0.62,0.46]),
            SurfaceKind::Rail => (SceneLayer::Rail,[0.40,0.52,0.61]),
        };
        add(&mesh,color,layer)?;
    }
    info(&format!("Loaded {} buildings, {} transport spans, {} vertices, {} triangles",
        building_report.buildings_meshed,road_report.segments_meshed,
        verts.len(),indices.len()/3));
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
        SceneLayer::Ground | SceneLayer::Buildings | SceneLayer::Road => true,
        // These independent centerlines are not surveyed sidewalk boundaries
        // or grade-correct rail beds; isolate layers for debugging.
        SceneLayer::Walk | SceneLayer::Steps => layer_enabled("layer-walk",true),
        SceneLayer::Rail => layer_enabled("layer-rail",false),
    }
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
    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.depth=Self::depth_view(&self.device,self.config.width,self.config.height);
    }
    fn update(&mut self, pressed: &HashSet<KeyCode>, dt: f32) {
        self.camera.move_by(pressed,dt,Some(&self.collision),Some((&self.terrain,&self.region)));
        let vp=self.camera.view_projection(self.config.width,self.config.height);
        self.queue.write_buffer(&self.camera_buffer,0,
            bytemuck::bytes_of(&CameraUniform{view_proj:vp.to_cols_array_2d()}));
    }
    fn draw(&mut self) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) |
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.resize(self.window.inner_size()); return false;
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
    info("Loading Downtown LA streets and buildings...");
    let city=fetch_city().await?;
    let terrain=fetch_terrain().await?;
    terrain.validate(&city.region)?;
    set_terrain_size(terrain.heights_m.len());
    let (vertices,indices,batches)=combined_city(&city,&terrain)?;
    info("Connecting to WebGPU or WebGL2...");
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
    config.present_mode=wgpu::PresentMode::Fifo;
    surface.configure(&device,&config);
    let collision=CollisionWorld::from_city_with_terrain(&city,Some(&terrain));
    set_collision_count(collision.obstacle_count());
    let camera=Camera::spawn_at_street(&city,&collision,&terrain);
    let camera_buffer=device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:Some("Camera uniform"),
        contents:bytemuck::bytes_of(&CameraUniform {
            view_proj:camera.view_projection(config.width,config.height).to_cols_array_2d()
        }),
        usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,
    });
    let camera_layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{
        label:Some("Camera layout"),
        entries:&[wgpu::BindGroupLayoutEntry{
            binding:0,visibility:wgpu::ShaderStages::VERTEX,
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
        window,surface,device,queue,config,pipeline,depth,camera,collision,terrain,region:city.region,
        camera_buffer,camera_bind_group,vertices:vertex_buffer,indices:index_buffer,index_count,
        rendered_frames:0,batches,gpu_mesh_bytes,
        active_draw_calls:0,active_triangles:0,
    };
    info(&format!("Downtown LA ready ({backend}) — DEM terrain and building walls, WASD walk, Shift run, E/Q fly, Esc release"));
    proxy.send_event(AppEvent::Ready(graphics))
        .map_err(|_|"Viewer window closed before initialization".to_string())
}

enum AppEvent { Ready(Graphics), Error(String), PointerLock(bool) }
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
        let mut attrs=Window::default_attributes()
            .with_title("Urbarium | Downtown Los Angeles")
            .with_inner_size(PhysicalSize::new(1280,800));
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
        if let Stage::Ready(gfx)=&self.state {gfx.window.request_redraw();}
    }
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
    fn fps_look_is_not_inverted_and_camera_is_human_scaled() {
        let mut camera=Camera::default();
        camera.mouse_look(100.0,0.0);
        assert!(camera.forward().x > 0.0, "rightward mouse motion looks right");
        assert!((camera.position.y-1.7).abs()<f32::EPSILON);
        assert!(camera.speed < 5.0);
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
