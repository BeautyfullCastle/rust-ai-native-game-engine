// Sun shadows from two cascades and the surface shading used by all lit passes.

@group(0) @binding(1) var shadow_map0: texture_depth_2d;
@group(0) @binding(2) var shadow_map1: texture_depth_2d;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;

fn pcf0(uv: vec2<f32>, reference: f32) -> f32 {
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * g.cascades.x * g.shadow.x;
            sum = sum + textureSampleCompareLevel(shadow_map0, shadow_sampler, uv + o, reference);
        }
    }
    return sum / 9.0;
}

fn pcf1(uv: vec2<f32>, reference: f32) -> f32 {
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * g.cascades.y * g.shadow.x * 0.83;
            sum = sum + textureSampleCompareLevel(shadow_map1, shadow_sampler, uv + o, reference);
        }
    }
    return sum / 9.0;
}

// 1 in sunlight, 0 in shadow. The near cascade fades into the far one at its edge.
fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.shadow.y < 0.5) {
        return 1.0;
    }
    if (g.shadow.y < 0.5) {
        return 1.0;
    }
    let facing = saturate(dot(n, g.sun_dir.xyz));
    let grazing = 1.0 - facing;
    var far = 1.0;
    let l1 = g.shadow1 * vec4<f32>(world + n * (0.25 + 0.6 * grazing), 1.0);
    let uv1 = vec2<f32>(l1.x * 0.5 + 0.5, 0.5 - l1.y * 0.5);
    let edge1 = min(min(uv1.x, 1.0 - uv1.x), min(uv1.y, 1.0 - uv1.y));
    if (edge1 > 0.0 && l1.z < 1.0) {
        far = mix(1.0, pcf1(uv1, l1.z - 0.0006), smoothstep(0.0, 0.05, edge1));
    }
    let l0 = g.shadow0 * vec4<f32>(world + n * (0.04 + 0.1 * grazing), 1.0);
    let uv0 = vec2<f32>(l0.x * 0.5 + 0.5, 0.5 - l0.y * 0.5);
    let edge0 = min(min(uv0.x, 1.0 - uv0.x), min(uv0.y, 1.0 - uv0.y));
    if (edge0 > 0.0 && l0.z < 1.0) {
        return mix(far, pcf0(uv0, l0.z - 0.0004), smoothstep(0.0, 0.08, edge0));
    }
    return far;
}

struct Surface {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    roughness: f32,
    // Ambient visibility.
    ao: f32,
    // Light that passes through thin leaves, as a color.
    translucency: vec3<f32>,
    specular: f32,
};

fn shade(s: Surface, world: vec3<f32>, shadow: f32) -> vec3<f32> {
    let v = normalize(g.camera.xyz - world);
    let l = g.sun_dir.xyz;
    let sun = g.sun_color.rgb * g.sun_dir.w;
    let ndl = max(dot(s.normal, l), 0.0);
    let hemisphere = mix(g.ambient_ground.rgb, g.ambient_sky.rgb, s.normal.y * 0.5 + 0.5);
    var color = s.albedo * (sun * ndl * shadow + hemisphere * s.ao);

    let h = normalize(l + v);
    let ndh = max(dot(s.normal, h), 0.0);
    let a = max(s.roughness * s.roughness, 0.02);
    let a2 = a * a;
    let denominator = ndh * ndh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * denominator * denominator);
    let fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    color = color + sun * shadow * ndl * min(distribution * fresnel * 0.25, 4.0) * s.specular;

    // Leaves and grass glow when the sun is behind them.
    let through = pow(max(dot(v, -l), 0.0), 3.0) * 0.7 + max(dot(-s.normal, l), 0.0) * 0.3;
    // Thin outer leaves pass some light even where the crown shades itself.
    color = color + s.translucency * sun * through * (0.4 + 0.6 * shadow) * s.ao;
    return color;
}
