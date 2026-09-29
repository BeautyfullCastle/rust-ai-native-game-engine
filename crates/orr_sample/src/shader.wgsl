// Minimal 2D pass: one instanced quad per entity. Circles are cut out of the
// quad with a signed distance in the fragment shader; boxes are the quad
// itself, rotated by the instance angle. World space to clip space:
// (world - center) * scale.

struct Globals {
    center: vec2<f32>,
    scale: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) rot: f32,
    @location(3) shape: u32,
    @location(4) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    // Corner in the body's own frame: -1..1 on each side of the quad.
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: u32,
};

var<private> corners: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let corner = corners[in.vertex];
    let offset = corner * in.half_size;
    let c = cos(in.rot);
    let s = sin(in.rot);
    let rotated = vec2<f32>(c * offset.x - s * offset.y, s * offset.x + c * offset.y);
    let world = in.center + rotated;
    var out: VsOut;
    out.position = vec4<f32>((world - globals.center) * globals.scale, 0.0, 1.0);
    out.local = corner;
    out.color = in.color;
    out.shape = in.shape;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.shape == 0u) {
        let d = length(in.local);
        let edge = max(fwidth(d), 0.0001);
        let coverage = 1.0 - smoothstep(1.0 - edge, 1.0, d);
        if (coverage <= 0.0) {
            discard;
        }
        // A dark dot on the +x side of the body shows its rotation.
        let mark = 1.0 - smoothstep(0.16, 0.22, length(in.local - vec2<f32>(0.55, 0.0)));
        let rgb = mix(in.color.rgb, in.color.rgb * 0.45, mark);
        return vec4<f32>(rgb, in.color.a * coverage);
    }
    return in.color;
}
