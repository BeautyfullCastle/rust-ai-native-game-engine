// Shared by every scene pass: frame constants, noise, the sky model and fog.
// All colors are scene-linear. The post chain applies exposure and display encoding.

struct Globals {
    // The camera matrix in the main pass, the light matrix in a shadow pass.
    view_proj: mat4x4<f32>,
    shadow0: mat4x4<f32>,
    shadow1: mat4x4<f32>,
    // Camera axes scaled so that forward + x * right + y * up is the ray through clip (x, y).
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_forward: vec4<f32>,
    // xyz: camera position, w: time in seconds.
    camera: vec4<f32>,
    // xyz: unit vector toward the sun, w: intensity.
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    ambient_ground: vec4<f32>,
    // x: density at the base height, y: falloff per meter, z: base height, w: haze share.
    fog: vec4<f32>,
    // x, y: viewport size in pixels, w: ground cover draw distance.
    viewport: vec4<f32>,
    // x, y: shadow map texel size of cascade 0 and 1.
    cascades: vec4<f32>,
    // x: sky brightness, y: sun glow, z: noise level where clouds start, w: cloud speed.
    sky: vec4<f32>,
    // x: soft edge width in texels, y: 1 when sun shadows are on.
    shadow: vec4<f32>,
    // x: wind strength, y: wind speed.
    wind: vec4<f32>,
    // rgb: deep water color, w: clarity.
    water_color: vec4<f32>,
    // x: ripples, y: flow speed, z: foam, w: sun glint.
    water: vec4<f32>,
    // rgb: leaf and grass tint, w: translucency.
    leaf_tint: vec4<f32>,
    // x: leaf pattern contrast, y: share of autumn trees, z: ground bumps, w: caustics.
    detail: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

const PI: f32 = 3.14159265;

fn saturate(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn hash31(p: vec3<f32>) -> f32 {
    var p3 = fract(p * 0.1031);
    p3 = p3 + dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash31(i), hash31(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash31(i + vec3<f32>(0.0, 1.0, 0.0)), hash31(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash31(i + vec3<f32>(0.0, 0.0, 1.0)), hash31(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash31(i + vec3<f32>(0.0, 1.0, 1.0)), hash31(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn fbm2(p: vec2<f32>) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    for (var i = 0; i < 5; i = i + 1) {
        sum = sum + noise2(q) * amplitude;
        amplitude = amplitude * 0.5;
        // Rotate each octave so lattice lines do not stack.
        q = vec2<f32>(q.x * 1.6 - q.y * 1.2, q.x * 1.2 + q.y * 1.6);
    }
    return sum / 0.96875;
}

fn fbm2_low(p: vec2<f32>) -> f32 {
    let q = vec2<f32>(p.x * 1.6 - p.y * 1.2, p.x * 1.2 + p.y * 1.6);
    let r = vec2<f32>(q.x * 1.6 - q.y * 1.2, q.x * 1.2 + q.y * 1.6);
    return (noise2(p) * 0.5 + noise2(q) * 0.25 + noise2(r) * 0.125) / 0.875;
}

// Clear sky radiance without clouds or the sun disc.
fn sky_base(d: vec3<f32>) -> vec3<f32> {
    let sun = g.sun_dir.xyz;
    let day = smoothstep(-0.05, 0.4, sun.y);
    let zenith = mix(vec3<f32>(0.05, 0.075, 0.2), vec3<f32>(0.055, 0.2, 0.66), day);
    let horizon = mix(vec3<f32>(0.78, 0.4, 0.2), vec3<f32>(0.46, 0.66, 0.92), day);
    let up = max(d.y, 0.0);
    var c = mix(horizon, zenith, pow(up, 0.42));
    // Forward scattering brightens and warms the sky around the sun.
    let mu = max(dot(d, sun), 0.0);
    let glow = (0.08 * pow(mu, 6.0) + 0.3 * pow(mu, 48.0)) * g.sky.y;
    c = c + g.sun_color.rgb * glow * (0.3 + 0.7 * (1.0 - up)) * smoothstep(-0.1, 0.05, sun.y);
    // Below the horizon the view ends in haze.
    c = mix(c, horizon * 0.72, 1.0 - smoothstep(-0.3, 0.0, d.y));
    return c * g.sky.x;
}

// Height fog with an exact integral along the view ray, tinted by the sky behind it.
fn apply_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let ray = world - g.camera.xyz;
    let dist = length(ray);
    let dir = ray / max(dist, 1e-4);
    let falloff = g.fog.y;
    let start = g.fog.x * exp(-falloff * (g.camera.y - g.fog.z));
    let slope = falloff * dir.y;
    var optical = start * dist;
    if (abs(slope) > 1e-5) {
        optical = start * (1.0 - exp(-slope * dist)) / slope;
    }
    // A thin uniform haze keeps far ridges soft when the camera is above the fog layer.
    optical = optical + dist * g.fog.x * g.fog.w;
    let transmittance = exp(-max(optical, 0.0));
    let toward = normalize(vec3<f32>(dir.x, abs(dir.y) * 0.3 + 0.04, dir.z));
    let haze = sky_base(toward);
    return color * transmittance + haze * (1.0 - transmittance);
}
