// Scalar single-scattering GGX, correlated Smith visibility and Schlick Fresnel.
// Linear RGB throughout; the caller supplies unit normal/view/light directions.
// Roughness is perceptual. This finite rasterization floor is not a data rewrite.
// No normal map, transmission, clearcoat, specular IBL or energy compensation.
// Reference: Khronos glTF 2.0 Appendix B; Filament standard model visibility.
fn mr_normalize(value: vec3<f32>) -> vec3<f32> {
    return value * inverseSqrt(max(dot(value, value), 1e-20));
}
fn mr_brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, base: vec3<f32>, metallic: f32, roughness: f32) -> vec3<f32> {
    let nv = clamp(dot(n, v), 0.0, 1.0);
    let nl = clamp(dot(n, l), 0.0, 1.0);
    if nv <= 0.0 || nl <= 0.0 { return vec3<f32>(0.0); }
    let sum = v + l;
    if dot(sum, sum) <= 1e-20 { return vec3<f32>(0.0); }
    let h = mr_normalize(sum);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(v, h), 0.0, 1.0);
    let r = max(roughness, 0.045);
    let alpha = r * r;
    let a2 = alpha * alpha;
    // Algebraically equivalent to nh^2*(a2-1)+1, without cancellation at nh=1.
    let denominator = (1.0 - nh * nh) + a2 * nh * nh;
    let distribution = a2 / (3.141592653589793 * denominator * denominator);
    let lambda_v = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let lambda_l = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    let visibility = 0.5 / max(lambda_v + lambda_l, 1e-20);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let one_minus_vh = 1.0 - vh;
    let fifth = one_minus_vh * one_minus_vh * one_minus_vh * one_minus_vh * one_minus_vh;
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * fifth;
    let diffuse = (vec3<f32>(1.0) - fresnel) * base * (1.0 - metallic) / 3.141592653589793;
    return diffuse + fresnel * distribution * visibility;
}

// Equivalent ACES fit without squaring large exposed radiance. The original
// polynomial remains the exact legacy branch. Avoid evaluating overflow even
// in unselected vector lanes: both arguments below are independently bounded.
fn mr_aces(x: vec3<f32>) -> vec3<f32> {
    let low = min(x, vec3<f32>(1.0));
    let polynomial = (low * (2.51 * low + 0.03)) / (low * (2.43 * low + 0.59) + 0.14);
    let reciprocal = vec3<f32>(1.0) / max(x, vec3<f32>(1.0));
    let rational = (2.51 + 0.03 * reciprocal) / (2.43 + 0.59 * reciprocal + 0.14 * reciprocal * reciprocal);
    return clamp(select(polynomial, rational, x > vec3<f32>(1.0)), vec3<f32>(0.0), vec3<f32>(1.0));
}
