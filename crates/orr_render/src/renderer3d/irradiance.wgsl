// Already cosine-convolved linear irradiance. Explicit positive real SH basis (Stanford equation 3):
// Y00, y, z, x, xy, yz, 3z²−1, xz, x²−y². Never reconvolve or gamma decode.
struct IrradianceUniform {
    origin: vec4<f32>,
    inverse_spacing: vec4<f32>,
    dimensions: vec4<u32>,
    maximum: vec4<f32>,
    coefficients: array<vec4<f32>, 576>,
};
fn probe_coefficient(node: vec3<u32>, band: u32) -> vec3<f32> {
    let dims = PROBE_GLOBAL.irradiance.dimensions.xyz;
    let index = node.x + dims.x * (node.y + dims.y * node.z);
    return PROBE_GLOBAL.irradiance.coefficients[index * 9u + band].xyz;
}
fn probe_diffuse(world: vec3<f32>, normal: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    if PROBE_GLOBAL.irradiance.dimensions.w == 0u { return fallback; }
    let origin = PROBE_GLOBAL.irradiance.origin.xyz;
    let maximum = PROBE_GLOBAL.irradiance.maximum.xyz;
    if any(world < origin) || any(world > maximum) { return fallback; }
    let last = PROBE_GLOBAL.irradiance.dimensions.xyz - vec3<u32>(1u);
    let local = (world - origin) * PROBE_GLOBAL.irradiance.inverse_spacing.xyz;
    // Test world bounds before rounding q, and make exact maximum faces exact.
    let q = select(clamp(local, vec3<f32>(0.0), vec3<f32>(last)), vec3<f32>(last), world == maximum);
    let edge = min(q, vec3<f32>(last) - q);
    let weight = clamp(min(edge.x, min(edge.y, edge.z)), 0.0, 1.0);
    // Boundary equality returns the exact legacy value, including maximum faces.
    if weight <= 0.0 { return fallback; }
    let low = min(vec3<u32>(floor(q)), last - vec3<u32>(1u));
    let t = q - vec3<f32>(low);
    // Valid opposing vertex normals can cancel after raster interpolation.
    // Match CPU no-sample semantics rather than evaluating SH at a zero vector.
    let normal_scale = max(abs(normal.x), max(abs(normal.y), abs(normal.z)));
    if !all(abs(normal) <= vec3<f32>(3.402823e38)) || !(normal_scale > 0.0) { return fallback; }
    let scaled_normal = normal / normal_scale;
    let n = scaled_normal * inverseSqrt(dot(scaled_normal, scaled_normal));
    let basis = array<f32, 9>(
        0.2820948, 0.48860252*n.y, 0.48860252*n.z, 0.48860252*n.x,
        1.0925485*n.x*n.y, 1.0925485*n.y*n.z,
        0.31539157*(3.0*n.z*n.z-1.0), 1.0925485*n.x*n.z,
        0.54627424*(n.x*n.x-n.y*n.y)
    );
    var irradiance = vec3<f32>(0.0);
    for (var band = 0u; band < 9u; band += 1u) {
        // Interpolate signed coefficients first; clamp only the reconstructed E.
        let x00 = mix(probe_coefficient(low,band), probe_coefficient(low+vec3<u32>(1u,0u,0u),band),t.x);
        let x10 = mix(probe_coefficient(low+vec3<u32>(0u,1u,0u),band), probe_coefficient(low+vec3<u32>(1u,1u,0u),band),t.x);
        let x01 = mix(probe_coefficient(low+vec3<u32>(0u,0u,1u),band), probe_coefficient(low+vec3<u32>(1u,0u,1u),band),t.x);
        let x11 = mix(probe_coefficient(low+vec3<u32>(0u,1u,1u),band), probe_coefficient(low+vec3<u32>(1u,1u,1u),band),t.x);
        let coefficient = mix(mix(x00,x10,t.y), mix(x01,x11,t.y), t.z);
        irradiance += coefficient * basis[band];
    }
    return mix(fallback, max(irradiance, vec3<f32>(0.0)) / 3.14159265, weight);
}
