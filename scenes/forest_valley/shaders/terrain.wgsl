// Valley ground: meadow grass, forest floor under the trees and bare rock on
// steep slopes, broken up by noise at three sizes. Small bumps tilt the
// normal so flat ground still catches the light.
// @param grass color = 0.085, 0.15, 0.035
// @param forest_floor color = 0.095, 0.085, 0.04
// @param stone color = 0.21, 0.205, 0.195
// @param rock_slope float 0.05..0.9 = 0.3
// @param forest_share float 0..1 = 0.5
// @param bumps float 0..2 = 0.9
// @param detail float 0..1 = 1
// Repeating detail images, average linear 0.5: grass blades and rock grain.
// @texture grass_detail color = 0.5, 0.5, 0.5, 1
// @texture rock_detail color = 0.5, 0.5, 0.5, 1

fn t_hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn t_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = t_hash(i);
    let b = t_hash(i + vec2<f32>(1.0, 0.0));
    let c = t_hash(i + vec2<f32>(0.0, 1.0));
    let d = t_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn t_fbm(p: vec2<f32>) -> f32 {
    let q = vec2<f32>(p.x * 1.6 - p.y * 1.2, p.x * 1.2 + p.y * 1.6);
    let r = vec2<f32>(q.x * 1.6 - q.y * 1.2, q.x * 1.2 + q.y * 1.6);
    return (t_noise(p) * 0.5 + t_noise(q) * 0.25 + t_noise(r) * 0.125) / 0.875;
}

// The rock image on three planes, blended by the normal, so cliffs are not stretched.
fn rock_triplanar(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let w = pow(abs(n), vec3<f32>(4.0));
    let w3 = w / (w.x + w.y + w.z);
    let x = rock_detail(world.zy * 0.18).rgb;
    let y = rock_detail(world.xz * 0.18).rgb;
    let z = rock_detail(world.xy * 0.18).rgb;
    return x * w3.x + y * w3.y + z * w3.z;
}

fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    let xz = in.world.xz;
    // Read the images before any branch, at two sizes so the repeat does not show.
    let grass_near = grass_detail(xz * 0.45).rgb;
    let grass_far = grass_detail(xz * 0.061 + vec2<f32>(0.37, 0.71)).rgb;
    let rock = rock_triplanar(in.world, in.normal);
    let broad = t_noise(xz * 0.045);
    let medium = t_fbm(xz * 0.35);
    let fine = t_noise(xz * 3.1);

    let e = 0.35;
    let bump = vec2<f32>(
        t_fbm((xz + vec2<f32>(e, 0.0)) * 0.9) - t_fbm((xz - vec2<f32>(e, 0.0)) * 0.9),
        t_fbm((xz + vec2<f32>(0.0, e)) * 0.9) - t_fbm((xz - vec2<f32>(0.0, e)) * 0.9),
    );
    let n = normalize(in.normal + vec3<f32>(-bump.x, 0.0, -bump.y) * params.bumps);
    let slope = 1.0 - n.y;

    let grass_image = mix(vec3<f32>(1.0), grass_near * grass_far * 4.0, params.detail);
    let rock_image = mix(vec3<f32>(1.0), rock * 2.0, params.detail);
    let grass = mix(params.grass * 0.6, params.grass * 1.25, broad) * (0.75 + 0.5 * medium) * grass_image;
    let litter = mix(params.forest_floor, params.forest_floor * vec3<f32>(0.6, 1.1, 0.7), smoothstep(0.35, 0.7, medium)) * (0.75 + 0.5 * fine);
    let strata = sin(in.world.y * 1.7 + medium * 5.0) * 0.5 + 0.5;
    let cracks = smoothstep(0.42, 0.5, abs(t_noise(xz * 0.22 + in.world.y * 0.13) - 0.5) * 2.0);
    let stone = params.stone * (0.7 + 0.6 * (strata * 0.55 + fine * 0.45)) * (0.72 + 0.5 * broad) * (0.55 + 0.45 * cracks) * rock_image;

    let forest = smoothstep(0.3, 0.75, t_fbm(xz * 0.012 + vec2<f32>(40.0, 0.0)) + params.forest_share - 0.5 + (medium - 0.5) * 0.4);
    var albedo = mix(grass, litter, forest);
    albedo = mix(albedo, stone, smoothstep(params.rock_slope, params.rock_slope + 0.18, slope + (medium - 0.5) * 0.16));

    s.albedo = albedo;
    s.normal = n;
    s.roughness = 0.9;
    s.metallic = 0.0;
    return s;
}
