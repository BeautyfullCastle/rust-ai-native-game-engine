// 2D pass (design doc 4.3, WGSL). Two pipelines share this module:
//
// - shapes: one instanced quad per item. Circles and capsules are cut out of
//   the quad with a signed distance in the fragment shader; boxes are the quad
//   itself. All are rotated by the instance angle.
// - lines: one instanced quad per line segment, a fixed width in pixels
//   (debug overlays: AABBs, contacts, rays).
//
// World space to clip space: (world - center) * scale.

struct Globals {
    center: vec2<f32>,
    scale: vec2<f32>,
    // Render target size in pixels (for the pixel-wide lines).
    viewport: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

var<private> corners: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

// ---- shapes ----

struct ShapeIn {
    @builtin(vertex_index) vertex: u32,
    // Circle: half_size = (r, r). Quad: half extents. Capsule: (half length of
    // the axis segment along local x, radius).
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) rot: f32,
    // 0 circle, 1 quad, 2 capsule.
    @location(3) shape: u32,
    @location(4) color: vec4<f32>,
};

struct ShapeOut {
    @builtin(position) position: vec4<f32>,
    // Corner in the item's own frame: -1..1 on each side of the quad.
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: u32,
    // Half extents of the quad and the instance parameters (for the capsule SDF).
    @location(3) @interpolate(flat) extent: vec2<f32>,
    @location(4) @interpolate(flat) params: vec2<f32>,
};

@vertex
fn vs_shape(in: ShapeIn) -> ShapeOut {
    let corner = corners[in.vertex];
    var extent = in.half_size;
    if (in.shape == 2u) {
        extent = vec2<f32>(in.half_size.x + in.half_size.y, in.half_size.y);
    }
    let offset = corner * extent;
    let c = cos(in.rot);
    let s = sin(in.rot);
    let rotated = vec2<f32>(c * offset.x - s * offset.y, s * offset.x + c * offset.y);
    let world = in.center + rotated;
    var out: ShapeOut;
    out.position = vec4<f32>((world - globals.center) * globals.scale, 0.0, 1.0);
    out.local = corner;
    out.color = in.color;
    out.shape = in.shape;
    out.extent = extent;
    out.params = in.half_size;
    return out;
}

@fragment
fn fs_shape(in: ShapeOut) -> @location(0) vec4<f32> {
    // Screen-space derivatives must be taken in uniform control flow (browser WGSL validators
    // reject them inside a branch on `in.shape`), so both edge widths are computed up front.
    let d_circle = length(in.local);
    let edge_circle = max(fwidth(d_circle), 0.0001);
    // Capsule: signed distance to the axis segment, minus the radius (world units).
    let half_len = in.params.x;
    let radius = in.params.y;
    let p = in.local * in.extent;
    let q = vec2<f32>(p.x - clamp(p.x, -half_len, half_len), p.y);
    let d_capsule = length(q) - radius;
    let edge_capsule = max(fwidth(d_capsule), 0.00001);

    if (in.shape == 0u) {
        let coverage = 1.0 - smoothstep(1.0 - edge_circle, 1.0, d_circle);
        if (coverage <= 0.0) {
            discard;
        }
        // A dark dot on the +x side of the body shows its rotation.
        let mark = 1.0 - smoothstep(0.16, 0.22, length(in.local - vec2<f32>(0.55, 0.0)));
        let rgb = mix(in.color.rgb, in.color.rgb * 0.45, mark);
        return vec4<f32>(rgb, in.color.a * coverage);
    }
    if (in.shape == 2u) {
        let coverage = clamp(0.5 - d_capsule / edge_capsule, 0.0, 1.0);
        if (coverage <= 0.0) {
            discard;
        }
        // Dark dot on the +x cap shows the rotation.
        let mark = 1.0 - smoothstep(0.24 * radius, 0.32 * radius, length(p - vec2<f32>(half_len, 0.0)));
        let rgb = mix(in.color.rgb, in.color.rgb * 0.45, mark);
        return vec4<f32>(rgb, in.color.a * coverage);
    }
    return in.color;
}

// ---- lines ----

struct LineIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    // Width in pixels.
    @location(2) width: f32,
    @location(3) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) position: vec4<f32>,
    // Signed distance from the line's center, in pixels.
    @location(0) across: f32,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) half_width: f32,
};

@vertex
fn vs_line(in: LineIn) -> LineOut {
    let corner = corners[in.vertex];
    let half_px = globals.viewport * 0.5;
    // Both ends in pixels (relative to the target center, y up).
    let pa = (in.a - globals.center) * globals.scale * half_px;
    let pb = (in.b - globals.center) * globals.scale * half_px;
    var dir = pb - pa;
    let len = length(dir);
    if (len > 0.0001) {
        dir = dir / len;
    } else {
        dir = vec2<f32>(1.0, 0.0);
    }
    let normal = vec2<f32>(-dir.y, dir.x);
    // One extra pixel each side for the soft edge; square caps.
    let half_width = in.width * 0.5;
    let reach = half_width + 1.0;
    let t = corner.x * 0.5 + 0.5;
    let along = mix(pa, pb, t) + dir * corner.x * half_width;
    let px = along + normal * corner.y * reach;
    var out: LineOut;
    out.position = vec4<f32>(px / half_px, 0.0, 1.0);
    out.across = corner.y * reach;
    out.color = in.color;
    out.half_width = half_width;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    let coverage = clamp(in.half_width + 0.5 - abs(in.across), 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
