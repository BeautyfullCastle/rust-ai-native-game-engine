// Post chain on scene-linear RGBA16F: bloom pyramid, light shafts, then the one
// place where exposure, tone mapping, grading and display encoding happen.

@group(0) @binding(0) var tex_a: texture_2d<f32>;
@group(0) @binding(1) var tex_b: texture_2d<f32>;
@group(0) @binding(2) var tex_c: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

@group(0) @binding(5) var tex_d: texture_2d<f32>;

struct Post {
    // x, y: sun position in texture coordinates, z: shaft strength, w: width / height.
    sun: vec4<f32>,
    // x: exposure, y: bloom strength, z: 1 when the shader must encode sRGB, w: time.
    grade: vec4<f32>,
    // rgb: sun color, w: how far the eye adapts toward the new brightness this frame.
    sun_color: vec4<f32>,
    // x: saturation, y: contrast, z: vignette, w: grain.
    tone: vec4<f32>,
    // x: 1 when exposure follows the metered brightness, y: brightness where bloom starts.
    meter: vec4<f32>,
};
@group(0) @binding(4) var<uniform> post: Post;

struct Screen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_screen(@builtin(vertex_index) index: u32) -> Screen {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let p = corners[index];
    var o: Screen;
    o.position = vec4<f32>(p, 0.0, 1.0);
    o.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}

fn box4(uv: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    let o = texel * 0.5;
    return (textureSampleLevel(tex_a, linear_sampler, uv + vec2<f32>(-o.x, -o.y) * 2.0, 0.0).rgb
        + textureSampleLevel(tex_a, linear_sampler, uv + vec2<f32>(o.x, -o.y) * 2.0, 0.0).rgb
        + textureSampleLevel(tex_a, linear_sampler, uv + vec2<f32>(-o.x, o.y) * 2.0, 0.0).rgb
        + textureSampleLevel(tex_a, linear_sampler, uv + vec2<f32>(o.x, o.y) * 2.0, 0.0).rgb
        + textureSampleLevel(tex_a, linear_sampler, uv, 0.0).rgb * 4.0) / 8.0;
}

// Scene to half resolution. Only light above the knee blooms; very bright texels are bounded.
@fragment
fn fs_prefilter(in: Screen) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(tex_a));
    let c = min(box4(in.uv, texel), vec3<f32>(16.0));
    let peak = max(c.r, max(c.g, c.b));
    let knee = smoothstep(post.meter.y, post.meter.y + 1.5, peak);
    return vec4<f32>(c * knee, 1.0);
}

@fragment
fn fs_down(in: Screen) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(tex_a));
    return vec4<f32>(box4(in.uv, texel), 1.0);
}

// Tent filter of the smaller level plus this level's own detail.
@fragment
fn fs_up(in: Screen) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(tex_a));
    var sum = textureSampleLevel(tex_a, linear_sampler, in.uv, 0.0).rgb * 4.0;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv + vec2<f32>(t.x, 0.0), 0.0).rgb * 2.0;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv - vec2<f32>(t.x, 0.0), 0.0).rgb * 2.0;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv + vec2<f32>(0.0, t.y), 0.0).rgb * 2.0;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv - vec2<f32>(0.0, t.y), 0.0).rgb * 2.0;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv + t, 0.0).rgb;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv - t, 0.0).rgb;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv + vec2<f32>(t.x, -t.y), 0.0).rgb;
    sum = sum + textureSampleLevel(tex_a, linear_sampler, in.uv + vec2<f32>(-t.x, t.y), 0.0).rgb;
    let own = textureSampleLevel(tex_b, linear_sampler, in.uv, 0.0).rgb;
    return vec4<f32>(sum / 16.0 + own, 1.0);
}

fn interleaved(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Light shafts: march from each pixel toward the sun and count open sky on the way.
// Scene alpha is 1 on geometry and lower on sky.
@fragment
fn fs_shafts(in: Screen) -> @location(0) vec4<f32> {
    if (post.sun.z <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let steps = 48;
    let delta = (post.sun.xy - in.uv) / f32(steps);
    var p = in.uv + delta * interleaved(in.position.xy);
    var sum = 0.0;
    var weight = 1.0;
    for (var i = 0; i < steps; i = i + 1) {
        let open = 1.0 - textureSampleLevel(tex_a, linear_sampler, p, 0.0).a;
        let inside = step(0.0, p.x) * step(p.x, 1.0) * step(0.0, p.y) * step(p.y, 1.0);
        sum = sum + open * weight * inside;
        weight = weight * 0.972;
        p = p + delta;
    }
    let offset = (in.uv - post.sun.xy) * vec2<f32>(post.sun.w, 1.0);
    let falloff = exp(-length(offset) * 1.9);
    let shafts = sum / f32(steps) * falloff * post.sun.z;
    return vec4<f32>(post.sun_color.rgb * shafts, 1.0);
}

// Light meter, step 1: log luminance of the scene on a coarse grid.
@fragment
fn fs_meter(in: Screen) -> @location(0) vec4<f32> {
    let cell = 1.0 / vec2<f32>(64.0, 36.0);
    var sum = 0.0;
    // Sixteen taps spread over the cell keep moving leaves and water from making the exposure flicker.
    for (var i = 0; i < 16; i = i + 1) {
        let o = (vec2<f32>(f32(i % 4), f32(i / 4)) - 1.5) * 0.25 * cell;
        let c = textureSampleLevel(tex_a, linear_sampler, in.uv + o, 0.0).rgb;
        sum = sum + log(max(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4));
    }
    return vec4<f32>(sum / 16.0, 0.0, 0.0, 1.0);
}

// Light meter, step 2: one center-weighted average that follows the scene over time,
// as an eye does. tex_a is the grid, tex_b the value of the previous frame.
@fragment
fn fs_adapt(in: Screen) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex_a));
    var sum = 0.0;
    var total = 0.0;
    for (var y = 0; y < size.y; y = y + 1) {
        for (var x = 0; x < size.x; x = x + 1) {
            let uv = (vec2<f32>(f32(x), f32(y)) + 0.5) / vec2<f32>(size);
            let w = 1.0 - 0.7 * smoothstep(0.1, 0.7, length(uv - vec2<f32>(0.5, 0.55)));
            sum = sum + textureLoad(tex_a, vec2<i32>(x, y), 0).r * w;
            total = total + w;
        }
    }
    let before = textureLoad(tex_b, vec2<i32>(0, 0), 0).r;
    return vec4<f32>(mix(before, sum / total, post.sun_color.w), 0.0, 0.0, 1.0);
}

fn tone_curve(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}

// ACES fit by Stephen Hill: keeps hue better than the per-channel curve.
fn aces(color: vec3<f32>) -> vec3<f32> {
    let to_working = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    let to_display = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    return clamp(to_display * tone_curve(to_working * color), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_final(in: Screen) -> @location(0) vec4<f32> {
    var color = textureSampleLevel(tex_a, linear_sampler, in.uv, 0.0).rgb;
    color = color + textureSampleLevel(tex_b, linear_sampler, in.uv, 0.0).rgb * post.grade.y;
    // The shaft buffer is half resolution; four taps hide its dither.
    let t = 1.0 / vec2<f32>(textureDimensions(tex_c));
    let shafts = (textureSampleLevel(tex_c, linear_sampler, in.uv + t * vec2<f32>(0.5, 0.5), 0.0).rgb
        + textureSampleLevel(tex_c, linear_sampler, in.uv + t * vec2<f32>(-0.5, 0.5), 0.0).rgb
        + textureSampleLevel(tex_c, linear_sampler, in.uv + t * vec2<f32>(0.5, -0.5), 0.0).rgb
        + textureSampleLevel(tex_c, linear_sampler, in.uv + t * vec2<f32>(-0.5, -0.5), 0.0).rgb) * 0.25;
    color = color + shafts;
    // Expose for the metered brightness, within the range an eye can adapt.
    let metered = exp(textureLoad(tex_d, vec2<i32>(0, 0), 0).r);
    let exposure = post.grade.x * mix(1.0, clamp(0.165 / metered, 0.55, 4.5), post.meter.x);
    color = aces(color * exposure);

    // Grade: a little more contrast and saturation, warm highlights, cool shadows.
    let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = mix(vec3<f32>(luma), color, post.tone.x);
    color = color * mix(vec3<f32>(0.96, 0.99, 1.06), vec3<f32>(1.04, 1.0, 0.95), smoothstep(0.1, 0.8, luma));
    color = saturate(color);
    color = mix(color, color * color * (3.0 - 2.0 * color), post.tone.y);

    let centered = in.uv - 0.5;
    color = color * (1.0 - post.tone.z * smoothstep(0.25, 0.95, length(centered * vec2<f32>(1.15, 1.0)) * 1.25));

    if (post.grade.z > 0.5) {
        color = linear_to_srgb(color);
    }
    // One step of noise removes banding in the sky.
    color = color + (interleaved(in.position.xy + post.grade.w) - 0.5) * post.tone.w / 255.0;
    return vec4<f32>(color, 1.0);
}
