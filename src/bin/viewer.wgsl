struct Camera {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};
@vertex
fn vs_main(input: VIn) -> VOut {
    var out: VOut;
    out.clip = camera.view_proj * vec4<f32>(input.position, 1.0);
    out.normal = input.normal;
    out.color = input.color;
    return out;
}
@fragment
fn fs_main(input: VOut) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(0.6, 1.0, 0.35));
    let diffuse = max(dot(normalize(input.normal), light), 0.0);
    let shade = 0.38 + 0.62 * diffuse;
    let gamma = pow(input.color * shade, vec3<f32>(0.87));
    return vec4<f32>(gamma, 1.0);
}
