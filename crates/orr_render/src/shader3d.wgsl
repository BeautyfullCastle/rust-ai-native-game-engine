// 3D renderer: instanced meshes with a metal/roughness Blinn-Phong model,
// one directional light with PCF shadow mapping, hemisphere ambient, ACES
// tone mapping and sRGB encoding when the target is not an sRGB format.
// Debug lines are screen space quads of constant pixel width.

struct Globals {
    view_proj: mat4x4<f32>,
    light_vp: mat4x4<f32>,
    cam_pos: vec4<f32>,
    // xyz: unit vector toward the light, w: intensity.
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    // rgb: sky color, w: ambient strength.
    sky: vec4<f32>,
    ground: vec4<f32>,
    // x: shadows on, y: tone map on, z: encode sRGB in the shader, w: exposure.
    params: vec4<f32>,
    // x: shadow map texel size (uv), y: normal offset (world units), z: depth bias.
    shadow: vec4<f32>,
    // x, y: viewport size in pixels.
    viewport: vec4<f32>,
    point_position_range: vec4<f32>,
    point_color_intensity: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_samp: sampler_comparison;

const PI: f32 = 3.14159265;

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) cap: f32,
    @location(3) pos_ext: vec4<f32>,
    @location(4) rot: vec4<f32>,
    @location(5) scale_flags: vec4<f32>,
    @location(6) color: vec4<f32>,
    @location(7) material: vec4<f32>,
};

fn world_position(i: MeshIn) -> vec3<f32> {
    let local = i.pos * i.scale_flags.xyz + vec3<f32>(0.0, i.cap * i.pos_ext.w, 0.0);
    return i.pos_ext.xyz + quat_rotate(i.rot, local);
}

struct MainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) material: vec3<f32>,
    @location(4) flags: f32,
};

@vertex
fn vs_main(i: MeshIn) -> MainOut {
    var o: MainOut;
    let w = world_position(i);
    o.clip = g.view_proj * vec4<f32>(w, 1.0);
    o.world = w;
    // Dividing by the scale keeps normals right under non uniform scale.
    o.normal = quat_rotate(i.rot, normalize(i.normal / max(i.scale_flags.xyz, vec3<f32>(1e-6))));
    o.color = i.color.rgb;
    o.material = i.material.xyz;
    o.flags = i.scale_flags.w;
    return o;
}

// Main and shadow read the same immutable per-frame transforms.
@vertex
fn vs_shadow(i: MeshIn) -> @builtin(position) vec4<f32> {
    return g.light_vp * vec4<f32>(world_position(i), 1.0);
}

fn shadow_visibility(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let p = world + n * g.shadow.y;
    let lp = g.light_vp * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    let reference = lp.z - g.shadow.z;
    // 3x3 taps, each a hardware 2x2 bilinear compare: a soft 4 texel wide edge.
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * g.shadow.x;
            sum = sum + textureSampleCompareLevel(shadow_map, shadow_samp, uv + o, reference);
        }
    }
    let inside = uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0 && lp.z >= 0.0 && lp.z <= 1.0;
    return select(1.0, sum / 9.0, inside);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Same bounded diffuse point term as the imported model shader. Standalone
// Renderer3D leaves intensity zero; the coordinator supplies this optional light.
fn point_diffuse(world: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    if (g.point_color_intensity.w <= 0.0) { return vec3<f32>(0.0); }
    let delta = g.point_position_range.xyz - world;
    let distance = length(delta);
    if (distance <= 1e-6) { return vec3<f32>(0.0); }
    let attenuation = max(1.0 - distance / g.point_position_range.w, 0.0);
    return g.point_color_intensity.xyz * g.point_color_intensity.w
        * max(dot(normal, delta / distance), 0.0) * attenuation * attenuation;
}

@fragment
fn fs_main(in: MainOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = normalize(g.cam_pos.xyz - in.world);
    let l = g.light_dir.xyz;

    var base = in.color;
    if (in.flags > 0.5) {
        let cell = floor(in.world.x * 0.5) + floor(in.world.z * 0.5);
        let odd = fract(cell * 0.5) > 0.25;
        base = base * select(1.0, 0.72, odd);
    }
    let rough = clamp(in.material.x, 0.04, 1.0);
    let metal = clamp(in.material.y, 0.0, 1.0);
    let diffuse_color = base * (1.0 - metal);
    let f0 = mix(vec3<f32>(0.04), base, metal);

    let ndl = max(dot(n, l), 0.0);
    var vis = 1.0;
    if (g.params.x > 0.5) {
        vis = shadow_visibility(in.world, n);
    }
    let radiance = g.light_color.rgb * g.light_dir.w * ndl * vis;

    // Blinn-Phong, normalized, shininess from roughness.
    let h = normalize(l + v);
    let r2 = rough * rough;
    let shininess = clamp(2.0 / (r2 * r2 + 1e-4) - 2.0, 2.0, 400.0);
    let spec_shape = pow(max(dot(n, h), 0.0), shininess) * (shininess + 8.0) / (8.0 * PI);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);

    let hemi = mix(g.ground.rgb, g.sky.rgb, n.y * 0.5 + 0.5) * g.sky.w;
    var color = diffuse_color * (radiance + hemi) + fresnel * spec_shape * radiance + hemi * f0 * 0.35 * (1.0 - rough);
    color = color + diffuse_color * point_diffuse(in.world, n) + base * in.material.z;

    color = color * g.params.w;
    if (g.params.y > 0.5) {
        color = aces(color);
    } else {
        color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (g.params.z > 0.5) {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, 1.0);
}

// ---- debug lines ----

struct LineIn {
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(i: LineIn) -> LineOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[i.vi];
    var ca = g.view_proj * vec4<f32>(i.a.xyz, 1.0);
    var cb = g.view_proj * vec4<f32>(i.b.xyz, 1.0);
    var o: LineOut;
    o.color = i.color;
    let eps = 0.01;
    if (ca.w < eps && cb.w < eps) {
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    // Clip the segment at the near plane (perspective only: w is the distance).
    if (ca.w < eps) {
        ca = mix(ca, cb, (eps - ca.w) / (cb.w - ca.w));
    } else if (cb.w < eps) {
        cb = mix(cb, ca, (eps - cb.w) / (ca.w - cb.w));
    }
    let pa = ca.xy / ca.w;
    let pb = cb.xy / cb.w;
    let za = ca.z / ca.w;
    let zb = cb.z / cb.w;
    let d_px = (pb - pa) * g.viewport.xy * 0.5;
    let len = length(d_px);
    var dir = vec2<f32>(1.0, 0.0);
    if (len > 1e-5) {
        dir = d_px / len;
    }
    let normal_px = vec2<f32>(-dir.y, dir.x);
    let p = mix(pa, pb, corner.x);
    let off = normal_px * corner.y * i.a.w * 0.5 * 2.0 / g.viewport.xy;
    o.clip = vec4<f32>(p + off, mix(za, zb, corner.x), 1.0);
    return o;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    var c = in.color.rgb;
    if (g.params.z > 0.5) {
        c = linear_to_srgb(c);
    }
    return vec4<f32>(c, in.color.a);
}
