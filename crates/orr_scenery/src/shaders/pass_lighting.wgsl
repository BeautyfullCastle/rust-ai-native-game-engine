// Sun shadows from the engine's shared shadow map, for the outdoor pass. The
// map is the one every engine batch casts into, so trees shade the terrain and
// the terrain shades the trees.
//
// Globals use: shadow0 = the map's light matrix, cascades.x = one texel,
// cascades.z = normal offset in meters, cascades.w = depth bias,
// shadow.x = soft edge width in texels, shadow.y = 1 when the frame has shadows.

@group(0) @binding(1) var shadow_map0: texture_depth_2d;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;

fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.shadow.y < 0.5) {
        return 1.0;
    }
    let facing = saturate(dot(n, g.sun_dir.xyz));
    let offset = g.cascades.z * (0.6 + 1.4 * (1.0 - facing));
    let l = g.shadow0 * vec4<f32>(world + n * offset, 1.0);
    let uv = vec2<f32>(l.x * 0.5 + 0.5, 0.5 - l.y * 0.5);
    let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    if (edge <= 0.0 || l.z >= 1.0 || l.z <= 0.0) {
        return 1.0;
    }
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * g.cascades.x * g.shadow.x;
            sum = sum + textureSampleCompareLevel(shadow_map0, shadow_sampler, uv + o, l.z - g.cascades.w);
        }
    }
    return mix(1.0, sum / 9.0, smoothstep(0.0, 0.04, edge));
}
