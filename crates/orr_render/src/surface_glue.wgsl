// The engine side of a surface shader on the 3D renderer's shapes. It sits
// between the renderer's shader and the contract (surface_contract.wgsl).

@group(1) @binding(0) var surface_texture_0: texture_2d<f32>;
@group(1) @binding(1) var surface_texture_1: texture_2d<f32>;
@group(1) @binding(2) var surface_texture_2: texture_2d<f32>;
@group(1) @binding(3) var surface_texture_3: texture_2d<f32>;
@group(1) @binding(4) var surface_sampler: sampler;

struct SurfaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) material: vec3<f32>,
    @location(4) local: vec3<f32>,
    @location(5) @interpolate(flat) layers: f32,
    @location(6) @interpolate(flat) p0: vec4<f32>,
    @location(7) @interpolate(flat) p1: vec4<f32>,
    @location(8) @interpolate(flat) p2: vec4<f32>,
    @location(9) @interpolate(flat) p3: vec4<f32>,
    @location(10) @interpolate(flat) flags: f32,
};

@vertex
fn vs_surface(
    i: MeshIn,
    @location(8) p0: vec4<f32>,
    @location(9) p1: vec4<f32>,
    @location(10) p2: vec4<f32>,
    @location(11) p3: vec4<f32>,
) -> SurfaceOut {
    var o: SurfaceOut;
    let w = world_position(i);
    o.clip = g.view_proj * vec4<f32>(w, 1.0);
    o.world = w;
    o.normal = quat_rotate(i.rot, normalize(i.normal / max(i.scale_flags.xyz, vec3<f32>(1e-6))));
    o.color = i.color.rgb;
    o.material = i.material.xyz;
    o.local = i.pos * i.scale_flags.xyz + vec3<f32>(0.0, i.cap * i.pos_ext.w, 0.0);
    o.layers = i.material.w;
    o.flags = i.scale_flags.w;
    o.p0 = p0;
    o.p1 = p1;
    o.p2 = p2;
    o.p3 = p3;
    return o;
}

@fragment
fn fs_surface(in: SurfaceOut) -> @location(0) vec4<f32> {
    for (var k = 0; k < 4; k = k + 1) {
        surface_raw[k] = in.p0[k];
        surface_raw[k + 4] = in.p1[k];
        surface_raw[k + 8] = in.p2[k];
        surface_raw[k + 12] = in.p3[k];
    }
    params = surface_params();
    var input: SurfaceInput;
    input.world = in.world;
    input.local = in.local;
    input.uv = vec2<f32>(0.0);
    input.normal = output_normalize(in.normal);
    // The ground checker of the built-in material is part of the instance color.
    var color = in.color;
    if (in.flags > 0.5) {
        let cell = floor(in.world.x * 0.5) + floor(in.world.z * 0.5);
        color = color * select(1.0, 0.72, fract(cell * 0.5) > 0.25);
    }
    input.color = color;
    input.roughness = in.material.x;
    input.metallic = in.material.y;
    input.emissive = in.material.z;
    input.time = g.viewport.w + surface_textures_keep();
    var s = surface(input);
    // A zero or broken normal from the shader falls back to the shape's normal.
    let length2 = dot(s.normal, s.normal);
    s.normal = select(input.normal, s.normal * inverseSqrt(max(length2, 1e-20)), length2 > 1e-12);
    return shade_surface(SurfacePoint(in.world, in.layers), s);
}
