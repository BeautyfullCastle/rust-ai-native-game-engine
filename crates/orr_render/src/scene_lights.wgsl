// Scene lights: directional, point and spot lights with a layer mask each.
// A light affects a surface only when the two share a render layer.

struct SceneLight {
    // xyz: position, w: range.
    position_range: vec4<f32>,
    // rgb: color, w: intensity.
    color_intensity: vec4<f32>,
    // xyz: unit direction the light travels, w: 0 directional, 1 point, 2 spot.
    direction_kind: vec4<f32>,
    // x: cosine of the inner cone angle, y: cosine of the outer one, z: layers the light affects.
    cone_layers: vec4<f32>,
};

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
                reach = reach * smoothstep(light.cone_layers.y, light.cone_layers.x, dot(-toward, light.direction_kind.xyz));
            }
        }
        sum = sum + light.color_intensity.xyz * light.color_intensity.w * max(dot(normal, toward), 0.0) * reach;
    }
    return sum;
}
