// The Eastern dragon: a long body with scales and belly plates, a red mane,
// golden horns and whiskers. The mesh is rebuilt along the head's path every
// frame, so it carries no instance data.

struct DragonIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // x: part (0 scales, 1 belly, 2 mane, 3 horn, 4 whisker, 5 eye, 6 pupil and nostril, 7 teeth), y: meters along the body, z: angle around it.
    @location(3) params: vec3<f32>,
};

struct DragonOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) params: vec3<f32>,
};

@vertex
fn vs_dragon(i: DragonIn) -> DragonOut {
    var o: DragonOut;
    o.world = i.position;
    o.clip = g.view_proj * vec4<f32>(i.position, 1.0);
    o.normal = i.normal;
    o.color = i.color;
    o.params = i.params;
    return o;
}

@vertex
fn vs_dragon_shadow(i: DragonIn) -> @builtin(position) vec4<f32> {
    var clip = g.view_proj * vec4<f32>(i.position, 1.0);
    clip.z = max(clip.z, 0.0);
    return clip;
}

@fragment
fn fs_dragon(in: DragonOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let part = i32(in.params.x + 0.5);
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    var s: Surface;
    s.albedo = in.color;
    s.normal = n;
    s.roughness = 0.5;
    s.ao = 1.0;
    s.translucency = vec3<f32>(0.0);
    s.specular = 0.6;
    if (part == 0) {
        // Overlapping scales in offset rows: each scale is lighter at its middle and dark at its rim.
        let row = in.params.y * 2.6;
        let column = in.params.z * 3.8 + 0.5 * floor(row);
        let cell = fract(vec2<f32>(column, row)) - vec2<f32>(0.5, 0.35);
        let rim = smoothstep(0.18, 0.48, length(cell * vec2<f32>(1.0, 1.3)));
        let sheen = hash21(floor(vec2<f32>(column, row)));
        s.albedo = in.color * (1.0 - 0.45 * rim) * (0.85 + 0.25 * sheen);
        s.roughness = mix(0.45, 0.7, rim);
        s.specular = mix(0.7, 0.35, rim);
    } else if (part == 1) {
        // Belly plates across the body.
        let plate = fract(in.params.y * 1.7);
        s.albedo = in.color * (0.78 + 0.22 * smoothstep(0.0, 0.25, plate) * (1.0 - smoothstep(0.75, 1.0, plate)));
        s.roughness = 0.55;
    } else if (part == 2) {
        // The mane passes light like thin hair.
        s.translucency = in.color * 0.7;
        s.roughness = 0.7;
        s.specular = 0.3;
    } else if (part == 3 || part == 4) {
        s.roughness = 0.25;
        s.specular = 1.6;
    } else if (part == 5) {
        s.roughness = 0.1;
        s.specular = 3.0;
    } else if (part == 6) {
        // Wet and dark: pupils and nostrils.
        s.roughness = 0.08;
        s.specular = 2.5;
    } else if (part == 7) {
        s.roughness = 0.3;
        s.specular = 1.2;
    }
    let shadow = sun_shadow(in.world, n);
    var color = shade(s, in.world, shadow);
    if (part == 5) {
        // Eyes glow faintly.
        color = color + in.color * 0.6;
    }
    return vec4<f32>(color, 1.0);
}
