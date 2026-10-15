struct Camera {
    view_proj: mat4x4<f32>,
    // category tint enabled, local hour, camera yaw, camera pitch
    options: vec4<f32>,
    sky: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) category_color: vec3<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) category_color: vec3<f32>,
};
@vertex
fn vs_main(input: VIn) -> VOut {
    var out: VOut;
    out.clip = camera.view_proj * vec4<f32>(input.position, 1.0);
    out.normal = input.normal;
    out.color = input.color;
    out.category_color = input.category_color;
    return out;
}
@fragment
fn fs_main(input: VOut) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(0.6, 1.0, 0.35));
    let diffuse = max(dot(normalize(input.normal), light), 0.0);
    let shade = 0.38 + 0.62 * diffuse;
    let has_kind = select(0.0, 1.0,
        dot(input.category_color, vec3<f32>(1.0)) > 0.01);
    let color = mix(input.color, input.category_color,
        has_kind * step(0.5, camera.options.x));
    let gamma = pow(color * shade, vec3<f32>(0.87));
    return vec4<f32>(gamma, 1.0);
}

// Actual camera-aware procedural skydome: the ray direction is reconstructed
// from the viewer's geographic +X-east, +Z-north left-handed coordinates.
// No sky image downloads, panorama seam, or fake full-screen screenshot.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};
@vertex
fn sky_vs(@builtin(vertex_index) idx:u32) -> SkyOut {
    let p=array<vec2<f32>,3>(
        vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0)
    );
    var out:SkyOut;
    out.clip=vec4<f32>(p[idx],0.9999,1.0);
    out.ndc=p[idx];
    return out;
}
@fragment
fn sky_fs(input:SkyOut) -> @location(0) vec4<f32> {
    let yaw=camera.options.z;
    let pitch=camera.options.w;
    let cp=cos(pitch);
    let sp=sin(pitch);
    let sy=sin(yaw);
    let cy=cos(yaw);
    let forward=vec3<f32>(sy*cp,sp,cy*cp);
    let right=vec3<f32>(cy,0.0,-sy);
    let up=vec3<f32>(-sy*sp,cp,-cy*sp);
    let tan_half=0.674509; // tan(68 degree vertical FOV / 2)
    let ray=normalize(forward+right*input.ndc.x*tan_half*camera.sky.x+
        up*input.ndc.y*tan_half);
    // Preview sun motion: 06:00 east horizon, noon high in southern sky,
    // 18:00 west horizon. Atmospheric/seasonal accuracy is future work.
    let phase=(camera.options.y-6.0)*3.14159265/12.0;
    let sun=normalize(vec3<f32>(
        cos(phase)*0.86,sin(phase)*0.75,-sin(phase)*0.65));
    let sunlight=smoothstep(-0.11,0.17,sun.y);
    let elevation=smoothstep(-0.13,0.9,ray.y);
    let midday=mix(vec3<f32>(0.69,0.78,0.84),
                   vec3<f32>(0.09,0.38,0.71),elevation);
    let night=mix(vec3<f32>(0.10,0.14,0.23),
                  vec3<f32>(0.007,0.021,0.062),elevation);
    let twilight=1.0-smoothstep(0.0,0.32,abs(sun.y));
    let orange=vec3<f32>(0.98,0.43,0.22);
    let horizon=exp(-abs(ray.y)*12.0)*twilight;
    var color=mix(night,midday,sunlight);
    color=mix(color,orange,horizon*0.62);
    let sun_disc=smoothstep(0.99965,0.99993,dot(ray,sun))*sunlight;
    color=mix(color,vec3<f32>(1.0,0.90,0.68),sun_disc);
    return vec4<f32>(color,1.0);
}
