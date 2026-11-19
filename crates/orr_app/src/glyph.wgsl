// Textured quads of the overlay text: one instance per glyph, in window pixels (y down).
struct Globals {
    viewport: vec2<f32>,
    unused: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

var<private> corners: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
);

struct GlyphIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
};
struct GlyphOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_glyph(in: GlyphIn) -> GlyphOut {
    let corner = corners[in.vertex];
    let pixel = in.rect.xy + corner * in.rect.zw;
    var out: GlyphOut;
    out.position = vec4<f32>(pixel.x / globals.viewport.x * 2.0 - 1.0, 1.0 - pixel.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.uv = mix(in.uv.xy, in.uv.zw, corner);
    out.color = in.color;
    return out;
}

@fragment
fn fs_glyph(in: GlyphOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas, atlas_sampler, in.uv).a;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
