//! Independent f64 oracle using GGX Smith lambda, not the shader's visibility form.
//! No production material/math helpers are used.
pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
pub fn unit(a: [f64; 3]) -> [f64; 3] {
    let length = dot(a, a).sqrt();
    if length <= 1e-10 {
        [0.0; 3]
    } else {
        a.map(|v| v / length)
    }
}
pub fn brdf(
    n: [f64; 3],
    v: [f64; 3],
    l: [f64; 3],
    base: [f64; 3],
    metal: f64,
    rough: f64,
) -> [f64; 3] {
    let nv = dot(n, v).clamp(0.0, 1.0);
    let nl = dot(n, l).clamp(0.0, 1.0);
    if nv <= 0.0 || nl <= 0.0 {
        return [0.0; 3];
    }
    let half = unit(std::array::from_fn(|i| v[i] + l[i]));
    if half == [0.0; 3] {
        return [0.0; 3];
    }
    let nh = dot(n, half).clamp(0.0, 1.0);
    let vh = dot(v, half).clamp(0.0, 1.0);
    let alpha2 = rough.max(0.045).powi(4);
    let d = alpha2 / (std::f64::consts::PI * (1.0 + (alpha2 - 1.0) * nh * nh).powi(2));
    let lambda = |cosine: f64| {
        ((1.0 + alpha2 * (1.0 - cosine * cosine) / (cosine * cosine)).sqrt() - 1.0) / 2.0
    };
    let g = 1.0 / (1.0 + lambda(nv) + lambda(nl));
    std::array::from_fn(|i| {
        let f0 = 0.04 * (1.0 - metal) + base[i] * metal;
        let f = f0 + (1.0 - f0) * (1.0 - vh).powi(5);
        (1.0 - f) * base[i] * (1.0 - metal) / std::f64::consts::PI + f * d * g / (4.0 * nv * nl)
    })
}
pub fn srgb_decode(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
