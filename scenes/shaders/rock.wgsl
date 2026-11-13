// Rough stone: value noise in the shape's own space darkens and tilts the surface.
// @param dark color = 0.18, 0.17, 0.16
// @param light color = 0.48, 0.46, 0.42
// @param scale float 0.5..20 = 4
// @param bumps float 0..1 = 0.35

fn rock_hash(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn rock_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(rock_hash(i), rock_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(rock_hash(i + vec3<f32>(0.0, 1.0, 0.0)), rock_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(rock_hash(i + vec3<f32>(0.0, 0.0, 1.0)), rock_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(rock_hash(i + vec3<f32>(0.0, 1.0, 1.0)), rock_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    let p = in.local * params.scale;
    let n = rock_noise(p) * 0.6 + rock_noise(p * 2.7) * 0.4;
    s.albedo = mix(params.dark, params.light, n);
    let e = 0.15;
    let slope = vec3<f32>(rock_noise(p + vec3<f32>(e, 0.0, 0.0)) - n, rock_noise(p + vec3<f32>(0.0, e, 0.0)) - n, rock_noise(p + vec3<f32>(0.0, 0.0, e)) - n);
    s.normal = normalize(in.normal - slope * params.bumps * 4.0);
    s.roughness = 0.95;
    return s;
}
