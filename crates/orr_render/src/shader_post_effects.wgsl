// Post effects beyond bloom, on scene-linear RGBA16F: light shafts, a light
// meter for auto exposure, and the final pass that exposes, tone maps and
// applies the effects on the finished picture in list order.

struct Effects {
    // x: exposure, y: 1 for ACES tone mapping, z: 1 when the shader must encode sRGB, w: bloom strength.
    display: vec4<f32>,
    // x, y: the sun in texture coordinates, z: shaft strength (0 is off), w: width / height.
    sun: vec4<f32>,
    // rgb: sun color times intensity, w: how much each shaft step keeps (0..1).
    sun_color: vec4<f32>,
    // x: 1 for auto exposure, y: darkest exposure, z: brightest exposure, w: target brightness.
    exposure: vec4<f32>,
    // x: how far the eye adapts toward the metered value this frame, y: a frame number for grain, z: list entries.
    frame: vec4<f32>,
    // Entry i uses list[2i] = (kind, p0, p1, p2) and list[2i + 1] = (p3, 0, 0, 0).
    // Kinds: 4 color grade, 5 vignette, 6 film grain.
    list: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> fx: Effects;
@group(0) @binding(4) var shafts: texture_2d<f32>;
@group(0) @binding(5) var grid: texture_2d<f32>;
@group(0) @binding(6) var depth: texture_depth_2d;
@group(0) @binding(7) var adapted: texture_2d<f32>;

struct Screen { @builtin(position) position: vec4<f32> };

@vertex fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> Screen {
    var positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Screen;
    out.position = vec4<f32>(positions[vi], 0.0, 1.0);
    return out;
}

fn interleaved(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Light shafts at half resolution: march from each pixel toward the sun and
// count open sky (depth at the far plane) on the way.
@fragment fn fs_shafts(in: Screen) -> @location(0) vec4<f32> {
    let full = vec2<f32>(textureDimensions(depth));
    let uv = in.position.xy * 2.0 / full;
    let steps = 48;
    let delta = (fx.sun.xy - uv) / f32(steps);
    var p = uv + delta * interleaved(in.position.xy);
    var sum = 0.0;
    var weight = 1.0;
    let last = vec2<i32>(full) - 1;
    for (var i = 0; i < steps; i = i + 1) {
        let inside = step(0.0, p.x) * step(p.x, 1.0) * step(0.0, p.y) * step(p.y, 1.0);
        let d = textureLoad(depth, clamp(vec2<i32>(p * full), vec2<i32>(0), last), 0);
        sum = sum + step(0.99999, d) * weight * inside;
        weight = weight * fx.sun_color.w;
        p = p + delta;
    }
    let offset = (uv - fx.sun.xy) * vec2<f32>(fx.sun.w, 1.0);
    let falloff = exp(-length(offset) * 1.9);
    return vec4<f32>(fx.sun_color.rgb * (sum / f32(steps) * falloff * fx.sun.z), 1.0);
}

// Light meter, step 1: log luminance on a 64 x 36 grid.
@fragment fn fs_meter(in: Screen) -> @location(0) vec4<f32> {
    let cell = 1.0 / vec2<f32>(64.0, 36.0);
    let center = in.position.xy * cell;
    var sum = 0.0;
    for (var i = 0; i < 16; i = i + 1) {
        let o = (vec2<f32>(f32(i % 4), f32(i / 4)) - 1.5) * 0.25 * cell;
        let c = textureSampleLevel(scene, linear_sampler, center + o, 0.0).rgb;
        sum = sum + log(max(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4));
    }
    return vec4<f32>(sum / 16.0, 0.0, 0.0, 1.0);
}

// Light meter, step 2: one center-weighted average that follows the scene
// over time, as an eye does. `adapted` holds the value of the previous frame.
@fragment fn fs_adapt(in: Screen) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(grid));
    var sum = 0.0;
    var total = 0.0;
    for (var y = 0; y < size.y; y = y + 1) {
        for (var x = 0; x < size.x; x = x + 1) {
            let uv = (vec2<f32>(f32(x), f32(y)) + 0.5) / vec2<f32>(size);
            let w = 1.0 - 0.7 * smoothstep(0.1, 0.7, length(uv - vec2<f32>(0.5, 0.55)));
            sum = sum + textureLoad(grid, vec2<i32>(x, y), 0).r * w;
            total = total + w;
        }
    }
    let before = textureLoad(adapted, vec2<i32>(0, 0), 0).r;
    return vec4<f32>(mix(before, sum / total, fx.frame.x), 0.0, 0.0, 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}

fn grade(c: vec3<f32>, saturation: f32, contrast: f32, warmth: f32, tint: f32) -> vec3<f32> {
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    var out = mix(vec3<f32>(luma), c, saturation);
    out = out * vec3<f32>(1.0 + 0.12 * warmth + 0.05 * tint, 1.0 - 0.1 * tint, 1.0 - 0.12 * warmth + 0.05 * tint);
    out = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(out, out * out * (3.0 - 2.0 * out), contrast);
}

@fragment fn fs_final(in: Screen) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(scene));
    let pos = vec2<i32>(in.position.xy);
    let uv = in.position.xy / size;
    let source = textureLoad(scene, pos, 0);
    var c = source.rgb;
    if (fx.display.w > 0.0) {
        c = c + textureSampleLevel(bloom, linear_sampler, uv, 0.0).rgb * fx.display.w;
    }
    if (fx.sun.z > 0.0) {
        // The shaft buffer is half resolution; four taps hide its dither.
        let t = 1.0 / vec2<f32>(textureDimensions(shafts));
        c = c + (textureSampleLevel(shafts, linear_sampler, uv + t * vec2<f32>(0.5, 0.5), 0.0).rgb
            + textureSampleLevel(shafts, linear_sampler, uv + t * vec2<f32>(-0.5, 0.5), 0.0).rgb
            + textureSampleLevel(shafts, linear_sampler, uv + t * vec2<f32>(0.5, -0.5), 0.0).rgb
            + textureSampleLevel(shafts, linear_sampler, uv + t * vec2<f32>(-0.5, -0.5), 0.0).rgb) * 0.25;
    }
    var exposure = fx.display.x;
    if (fx.exposure.x > 0.5) {
        let metered = exp(textureLoad(adapted, vec2<i32>(0, 0), 0).r);
        exposure = exposure * clamp(fx.exposure.w / max(metered, 1e-4), fx.exposure.y, fx.exposure.z);
    }
    c = c * exposure;
    if (fx.display.y > 0.5) { c = aces(c); } else { c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
    let count = i32(fx.frame.z);
    for (var i = 0; i < 8; i = i + 1) {
        if (i >= count) { break; }
        let a = fx.list[i * 2];
        let b = fx.list[i * 2 + 1];
        let kind = i32(a.x + 0.5);
        if (kind == 4) {
            c = grade(c, a.y, a.z, a.w, b.x);
        } else if (kind == 5) {
            let centered = (uv - 0.5) * vec2<f32>(size.x / size.y, 1.0);
            let soft = max(a.w, 0.01);
            c = c * (1.0 - a.y * smoothstep(a.z - soft, a.z + soft, length(centered)));
        } else if (kind == 6) {
            let n = interleaved(in.position.xy + vec2<f32>(fx.frame.y * 7.0, fx.frame.y * 3.0)) - 0.5;
            c = clamp(c + vec3<f32>(n * a.y * 0.15), vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }
    if (fx.display.z > 0.5) { c = linear_to_srgb(c); }
    return vec4<f32>(c, clamp(source.a, 0.0, 1.0));
}
