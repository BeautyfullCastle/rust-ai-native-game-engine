// Sun shadows from the engine's shared shadow map, for the outdoor and
// dragon passes. The map is the one every engine batch casts into, in
// cascades, so trees shade the terrain and the terrain shades the trees.
// `cascaded_visibility` comes from the engine (orr_render::SHADOW_CASCADES_WGSL).
//
// Globals use: shadow.y = 1 when the frame has shadows.

@group(0) @binding(1) var shared_shadow_map: texture_depth_2d;
@group(0) @binding(3) var shared_shadow_sampler: sampler_comparison;
@group(0) @binding(4) var<uniform> shadow_cascades: ShadowCascades;

fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.shadow.y < 0.5) {
        return 1.0;
    }
    // Light at a grazing angle needs a larger offset along the normal.
    let facing = saturate(dot(n, g.sun_dir.xyz));
    return cascaded_visibility(world, n * (0.6 + 1.4 * (1.0 - facing)));
}
