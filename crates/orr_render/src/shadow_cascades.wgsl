// Sun shadows from the shared shadow map in cascades: up to four light
// matrices, each drawn into its own tile of the map and covering a farther
// stretch of the view. Each shader that reads it declares `shared_shadow_map`,
// `shared_shadow_sampler` and `shadow_cascades` (a ShadowCascades uniform).

struct ShadowCascades {
    // World to each cascade's clip space.
    vp: array<mat4x4<f32>, 4>,
    // xy: the cascade's tile origin in the map, z: the tile's size, both in
    // texture coordinates; w: depth bias.
    tiles: array<vec4<f32>, 4>,
    // x: normal offset in meters, y: far end along the view in meters,
    // z: one texel of the map in texture coordinates.
    ranges: array<vec4<f32>, 4>,
    // xyz: the camera position, w: the number of cascades (0: none).
    eye: vec4<f32>,
    // xyz: the view direction, w: the part of each cascade's far end where it fades into the next.
    forward: vec4<f32>,
};

// Visibility of the sun in cascade `k`, or -1 where the point is outside it.
fn cascade_visibility(k: u32, world: vec3<f32>, n: vec3<f32>) -> f32 {
    let range = shadow_cascades.ranges[k];
    let tile = shadow_cascades.tiles[k];
    let lp = shadow_cascades.vp[k] * vec4<f32>(world + n * range.x, 1.0);
    let local = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    // The 3 x 3 filter stays inside the tile.
    let margin = 1.5 * range.z / tile.z;
    if (any(local < vec2<f32>(margin)) || any(local > vec2<f32>(1.0 - margin)) || lp.z < 0.0 || lp.z > 1.0) {
        return -1.0;
    }
    let uv = tile.xy + local * tile.z;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            sum = sum + textureSampleCompareLevel(shared_shadow_map, shared_shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * range.z, lp.z - tile.w);
        }
    }
    return sum / 9.0;
}

// 1 in sunlight, 0 in full shadow. The nearest cascade that holds the point
// decides; near its far end it fades into the next one, and the last one
// fades into sunlight.
fn cascaded_visibility(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let count = u32(shadow_cascades.eye.w);
    let depth = dot(world - shadow_cascades.eye.xyz, shadow_cascades.forward.xyz);
    for (var k = 0u; k < min(count, 4u); k = k + 1u) {
        let far = shadow_cascades.ranges[k].y;
        if (depth > far) {
            continue;
        }
        let here = cascade_visibility(k, world, n);
        if (here < 0.0) {
            continue;
        }
        let fade = far * (1.0 - shadow_cascades.forward.w);
        if (depth > fade) {
            let t = (depth - fade) / max(far - fade, 1e-6);
            var next = 1.0;
            if (k + 1u < count) {
                next = cascade_visibility(k + 1u, world, n);
            }
            if (next >= 0.0) {
                return mix(here, next, t);
            }
        }
        return here;
    }
    return 1.0;
}
