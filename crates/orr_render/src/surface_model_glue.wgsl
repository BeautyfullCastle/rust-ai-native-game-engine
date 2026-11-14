// The engine side of a surface shader on a static model. It sits between the
// model shader (shader_model.wgsl) and the contract (surface_contract.wgsl).

// Parameter values of the drawn instance and the clock, one uniform per draw.
struct ModelSurfaceValues {
    values: array<vec4<f32>, 4>,
    // x: seconds of the clock the host gives the renderer.
    time: vec4<f32>,
};
@group(0) @binding(3) var<uniform> surface_values: ModelSurfaceValues;
@group(0) @binding(4) var surface_texture_0: texture_2d<f32>;
@group(0) @binding(5) var surface_texture_1: texture_2d<f32>;
@group(0) @binding(6) var surface_texture_2: texture_2d<f32>;
@group(0) @binding(7) var surface_texture_3: texture_2d<f32>;
@group(0) @binding(8) var surface_sampler: sampler;

struct ModelSurfaceOut {
    @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) world_position: vec3<f32>, @location(3) local: vec3<f32>,
};

fn lit_model_surface(in: ModelSurfaceOut, visibility: f32) -> vec4<f32> {
    for (var k = 0; k < 4; k = k + 1) {
        for (var j = 0; j < 4; j = j + 1) {
            surface_raw[k * 4 + j] = surface_values.values[k][j];
        }
    }
    params = surface_params();
    var drawn: Varying;
    drawn.position = in.position;
    drawn.normal = in.normal;
    drawn.uv = in.uv;
    drawn.world_position = in.world_position;
    let base = model_surface(drawn);
    var input: SurfaceInput;
    input.world = in.world_position;
    input.local = in.local;
    input.normal = base.normal;
    input.uv = in.uv;
    input.color = base.albedo;
    input.roughness = base.roughness;
    input.metallic = base.metallic;
    input.emissive = 0.0;
    input.time = surface_values.time.x + surface_textures_keep();
    var s = surface(input);
    // A zero or broken normal from the shader falls back to the mesh normal.
    let length2 = dot(s.normal, s.normal);
    s.normal = select(input.normal, s.normal * inverseSqrt(max(length2, 1e-20)), length2 > 1e-12);
    return shade_surface(ModelPoint(in.world_position), s, visibility);
}

@fragment fn fs_model_surface(in: ModelSurfaceOut) -> @location(0) vec4<f32> {
    return lit_model_surface(in, 1.0);
}

// Receives the shared sun shadow, like fs_composed of the built-in look.
@fragment fn fs_model_surface_shadowed(in: ModelSurfaceOut) -> @location(0) vec4<f32> {
    let n = in.normal * inverseSqrt(max(dot(in.normal, in.normal), 1e-20));
    return lit_model_surface(in, shared_visibility(in.world_position, n));
}
