// Instanced trees, rocks and ground cover. One vertex layout serves all of them;
// `params.z` selects bark, leaf, grass or rock shading.

struct PropIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // x: ambient visibility, y: wind sway in meters, z: surface kind.
    @location(3) params: vec3<f32>,
    // xyz: instance position, w: uniform scale.
    @location(4) place: vec4<f32>,
    // x: yaw, y: hue variation, z: brightness, w: family flag.
    @location(5) look: vec4<f32>,
};

struct PropOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) params: vec3<f32>,
    @location(4) look: vec3<f32>,
};

fn turn(v: vec3<f32>, yaw: f32) -> vec3<f32> {
    let c = cos(yaw);
    let s = sin(yaw);
    return vec3<f32>(c * v.x + s * v.z, v.y, c * v.z - s * v.x);
}

// The same motion runs in the main and shadow passes, so shadows follow the branches.
fn prop_world(i: PropIn) -> vec3<f32> {
    var size = i.place.w;
    if (i.params.z > 1.5 && i.params.z < 2.5) {
        // Ground cover shrinks into the ground before its draw distance ends.
        let away = distance(i.place.xyz, g.camera.xyz);
        size = size * (1.0 - smoothstep(g.viewport.w * 0.72, g.viewport.w, away));
    }
    var world = i.place.xyz + turn(i.position * size, i.look.x);
    let t = g.camera.w * g.wind.y;
    let phase = i.place.x * 0.071 + i.place.z * 0.053;
    let gust = sin(t * 0.8 + phase) * 0.6 + sin(t * 1.9 + phase * 2.3) * 0.25 + sin(t * 0.33 + phase * 0.4) * 0.5;
    let flutter = sin(t * 4.7 + world.x * 2.3 + world.y * 1.7 + world.z * 1.9) * 0.3 * step(0.5, i.params.z);
    let sway = (gust + flutter) * i.params.y * size * g.wind.x;
    world = world + vec3<f32>(0.8, 0.0, 0.6) * sway;
    return world;
}

@vertex
fn vs_prop(i: PropIn) -> PropOut {
    var o: PropOut;
    o.world = prop_world(i);
    o.clip = g.view_proj * vec4<f32>(o.world, 1.0);
    o.normal = turn(i.normal, i.look.x);
    o.color = i.color;
    o.params = i.params;
    o.look = i.look.yzw;
    return o;
}

@vertex
fn vs_prop_shadow(i: PropIn) -> @builtin(position) vec4<f32> {
    var clip = g.view_proj * vec4<f32>(prop_world(i), 1.0);
    clip.z = max(clip.z, 0.0);
    return clip;
}

@fragment
fn fs_prop(in: PropOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let kind = in.params.z;
    let hue = in.look.x;
    let bright = in.look.y;
    let flag = in.look.z;
    var n = normalize(in.normal);
    var surface: Surface;
    surface.ao = in.params.x;
    surface.translucency = vec3<f32>(0.0);
    surface.roughness = 0.85;
    surface.specular = 0.25;
    var albedo = in.color;

    if (kind < 0.5) {
        // Bark: long vertical grain.
        let grain = noise3(in.world * vec3<f32>(9.0, 1.3, 9.0));
        albedo = albedo * (0.55 + 0.7 * grain);
        // Moss grows on the lower trunk.
        let moss = smoothstep(0.55, 0.8, noise3(in.world * 1.7)) * 0.6;
        albedo = mix(albedo, vec3<f32>(0.05, 0.1, 0.025), moss);
    } else if (kind < 1.5) {
        // Leaves: each tree has its own hue; noise breaks the mass into clumps.
        if (!front) {
            n = -n;
        }
        let clump = noise3(in.world * 2.6);
        // Dark gaps between bright leaf clusters give the smooth masses a leafy surface.
        var gaps = smoothstep(0.32, 0.66, noise3(in.world * 9.0) * 0.5 + noise3(in.world * 23.0) * 0.5);
        if (flag < 0.5) {
            // Needles: fine streaks, not broad leaf clusters.
            gaps = 0.55 + 0.45 * smoothstep(0.25, 0.7, noise3(in.world * vec3<f32>(14.0, 5.0, 14.0)));
        }
        let tilt = vec3<f32>(noise3(in.world * 5.0), noise3(in.world * 5.0 + 17.0), noise3(in.world * 5.0 + 31.0)) - 0.5;
        n = normalize(n + tilt * 0.8);
        var tint = mix(vec3<f32>(1.25, 1.05, 0.55), vec3<f32>(0.75, 1.0, 1.05), hue);
        if (flag > 0.5) {
            // A few broadleaf trees have turned early.
            // Trees with the highest hue values turn first: yellow, then red.
            let yellow = 1.0 - g.detail.y;
            let red = 1.0 - g.detail.y * 0.36;
            tint = mix(tint, vec3<f32>(1.9, 1.25, 0.4), smoothstep(yellow, yellow + 0.04, hue));
            tint = mix(tint, vec3<f32>(3.0, 0.85, 0.3), smoothstep(red, red + 0.02, hue));
        }
        let contrast = g.detail.x;
        albedo = albedo * tint * g.leaf_tint.rgb * bright * (0.7 + 0.5 * clump) * mix(max(1.0 - 0.32 * contrast, 0.0), 1.0 + 0.12 * contrast, gaps);
        surface.ao = surface.ao * (0.7 + 0.3 * clump) * mix(max(1.0 - 0.3 * contrast, 0.0), 1.0, gaps);
        surface.translucency = albedo * vec3<f32>(0.9, 1.0, 0.35) * 0.65 * g.leaf_tint.w;
        surface.roughness = 0.6;
        surface.specular = 0.5;
    } else if (kind < 2.5) {
        // Grass and ferns keep the ground normal so a meadow shades as one surface.
        albedo = albedo * mix(vec3<f32>(1.2, 1.05, 0.7), vec3<f32>(0.85, 1.0, 0.9), hue) * bright * g.leaf_tint.rgb;
        surface.translucency = albedo * vec3<f32>(0.9, 1.0, 0.4) * 0.3 * g.leaf_tint.w;
        surface.roughness = 0.6;
        surface.specular = 0.4;
    } else {
        // Rock: mottled stone with moss on the upward faces; darker and glossy in the stream.
        let mottle = noise3(in.world * 3.2) * 0.6 + noise3(in.world * 13.0) * 0.4;
        albedo = albedo * bright * (0.6 + 0.8 * mottle) * mix(vec3<f32>(1.08, 1.0, 0.9), vec3<f32>(0.92, 1.0, 1.08), hue);
        let moss = smoothstep(0.5, 0.82, n.y + (mottle - 0.5) * 0.5) * (1.0 - 0.75 * flag);
        albedo = mix(albedo, vec3<f32>(0.055, 0.115, 0.025) * (0.7 + 0.6 * mottle), moss * 0.85);
        albedo = albedo * mix(1.0, 0.72, flag);
        surface.roughness = mix(0.9, 0.4, flag);
        surface.specular = mix(0.3, 1.0, flag);
    }
    surface.albedo = albedo;
    surface.normal = n;
    let shadow = sun_shadow(in.world, n);
    let color = shade(surface, in.world, shadow);
    return vec4<f32>(apply_fog(color, in.world), 1.0);
}
