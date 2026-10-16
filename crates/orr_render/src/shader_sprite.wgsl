struct Globals {
    center: vec2<f32>,
    scale: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

var<private> corners: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, -0.5), vec2<f32>(0.5, 0.5),
    vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, 0.5), vec2<f32>(-0.5, 0.5),
);
struct SpriteIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) rotation: f32,
    @location(3) uv: vec4<f32>,
    @location(4) tint: vec4<f32>,
};
struct SpriteOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};
@vertex
fn vs_sprite(in: SpriteIn) -> SpriteOut {
    let corner = corners[in.vertex];
    let offset = corner * in.size;
    let c = cos(in.rotation);
    let s = sin(in.rotation);
    let world = in.center + vec2<f32>(c * offset.x - s * offset.y, s * offset.x + c * offset.y);
    var out: SpriteOut;
    out.position = vec4<f32>((world - globals.center) * globals.scale, 0.0, 1.0);
    // Atlas rows run down; world y runs up. Reversed UV bounds implement flips.
    let unit_uv = vec2<f32>(corner.x + 0.5, 0.5 - corner.y);
    out.uv = mix(in.uv.xy, in.uv.zw, unit_uv);
    out.tint = in.tint;
    return out;
}
@fragment
fn fs_sprite(in: SpriteOut) -> @location(0) vec4<f32> {
    return textureSample(atlas, atlas_sampler, in.uv) * in.tint;
}
