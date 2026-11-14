// Scene lights: directional, point and spot lights with a layer mask each.
// A light affects a surface only when the two share a render layer.

struct SceneLight {
    // xyz: position, w: range.
    position_range: vec4<f32>,
    // rgb: color, w: intensity.
    color_intensity: vec4<f32>,
    // xyz: unit direction the light travels, w: 0 directional, 1 point, 2 spot.
    direction_kind: vec4<f32>,
    // x: cosine of the inner cone angle, y: cosine of the outer one, z: layers the light affects,
    // w: the spot shadow tile, -1 without one.
    cone_layers: vec4<f32>,
};

// How much of each scene light reaches the point being shaded, from its spot
// shadow. Entry points that receive shadows fill it with scene_spot_fill.
var<private> scene_spot_visibility: array<f32, 8> = array<f32, 8>(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0);

// Fills scene_spot_visibility for the lights with a spot shadow tile, from
// `spot_shadow_map` and the spot matrices of `shadow_cascades`.
fn scene_spot_fill(world: vec3<f32>, n: vec3<f32>) {
    let count = i32(SCENE_GLOBAL.scene_light_info.x);
    let tiles = i32(shadow_cascades.spot_info.x);
    let texel = shadow_cascades.spot_info.y;
    for (var i = 0; i < count; i = i + 1) {
        let light = SCENE_GLOBAL.scene_lights[i];
        let slot = i32(light.cone_layers.w);
        if (slot < 0 || slot >= tiles) {
            continue;
        }
        // The normal offset grows with distance, as a texel of the cone does.
        let offset = n * length(light.position_range.xyz - world) * shadow_cascades.spot_info.z;
        let lp = shadow_cascades.spot_vp[slot] * vec4<f32>(world + offset, 1.0);
        if (lp.w <= 1e-6) {
            continue;
        }
        let ndc = lp.xyz / lp.w;
        let local = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (any(local < vec2<f32>(0.0)) || any(local > vec2<f32>(1.0)) || ndc.z < 0.0 || ndc.z > 1.0) {
            continue;
        }
        let tile = shadow_cascades.spot_tiles[slot];
        // The 3 x 3 filter stays inside the tile.
        let uv = tile.xy + clamp(local, vec2<f32>(1.5 * texel / tile.z), vec2<f32>(1.0 - 1.5 * texel / tile.z)) * tile.z;
        var sum = 0.0;
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                sum = sum + textureSampleCompareLevel(spot_shadow_map, SCENE_SHADOW_SAMPLER, uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z - tile.w);
            }
        }
        scene_spot_visibility[i] = sum / 9.0;
    }
}

// 1 when a light for `light_layers` reaches a surface on `layers`, else 0.
// A surface that names no layer is on the default layer.
fn scene_layers_meet(light_layers: f32, layers: f32) -> f32 {
    return select(0.0, 1.0, (u32(light_layers) & u32(max(layers, 1.0))) != 0u);
}

// 1 when the sun affects a surface on `layers`, else 0.
fn scene_sun_reaches(layers: f32) -> f32 {
    return scene_layers_meet(SCENE_GLOBAL.scene_light_info.y, layers);
}

// Sun visibility with the shadow strength applied: strength 0 gives no shadow.
fn scene_sun_shadow(visibility: f32) -> f32 {
    return mix(1.0, visibility, SCENE_GLOBAL.scene_light_info.z);
}

// Diffuse light of all scene lights on a surface. A point or spot light reaches
// zero at its range; a spot light also fades from its inner to its outer cone.
fn scene_lights_diffuse(world: vec3<f32>, normal: vec3<f32>, layers: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = i32(SCENE_GLOBAL.scene_light_info.x);
    for (var i = 0; i < count; i = i + 1) {
        let light = SCENE_GLOBAL.scene_lights[i];
        if (scene_layers_meet(light.cone_layers.z, layers) < 0.5) {
            continue;
        }
        var toward = -light.direction_kind.xyz;
        var reach = 1.0;
        if (light.direction_kind.w > 0.5) {
            let delta = light.position_range.xyz - world;
            let dist = length(delta);
            if (dist <= 1e-6) {
                continue;
            }
            toward = delta / dist;
            let falloff = max(1.0 - dist / light.position_range.w, 0.0);
            reach = falloff * falloff;
            if (light.direction_kind.w > 1.5) {
                reach = reach * smoothstep(light.cone_layers.y, light.cone_layers.x, dot(-toward, light.direction_kind.xyz)) * scene_spot_visibility[i];
            }
        }
        sum = sum + light.color_intensity.xyz * light.color_intensity.w * max(dot(normal, toward), 0.0) * reach;
    }
    return sum;
}
