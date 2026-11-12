// Terrain: materials come from slope, tree cover and distance to the water.
// Geometry writes alpha 1; only the sky writes 0, which the light shaft pass reads.

struct TerrainIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) data: vec4<f32>,
};

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) data: vec4<f32>,
};

@vertex
fn vs_terrain(i: TerrainIn) -> TerrainOut {
    var o: TerrainOut;
    o.clip = g.view_proj * vec4<f32>(i.position, 1.0);
    o.world = i.position;
    o.normal = i.normal;
    o.data = i.data;
    return o;
}

@vertex
fn vs_terrain_shadow(i: TerrainIn) -> @builtin(position) vec4<f32> {
    var clip = g.view_proj * vec4<f32>(i.position, 1.0);
    // Casters in front of the light volume still cast.
    clip.z = max(clip.z, 0.0);
    return clip;
}

// Distance to the nearest pebble center and a random value for that pebble.
fn pebbles(p: vec2<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var best = 8.0;
    var id = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y));
            let r = o + hash22(i + o) - f;
            let d = dot(r, r);
            if (d < best) {
                best = d;
                id = hash21(i + o + 7.3);
            }
        }
    }
    return vec2<f32>(sqrt(best), id);
}

// Bright moving lines of light on the stream bed.
fn caustics(p: vec2<f32>, t: f32) -> f32 {
    let a = noise2(p * 1.5 + vec2<f32>(t * 0.4, t * 0.23));
    let b = noise2(p * 1.9 - vec2<f32>(t * 0.31, -t * 0.36) + 5.2);
    return pow(saturate(1.0 - abs(a - b) * 5.0), 3.0);
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let xz = in.world.xz;
    let view_distance = distance(in.world, g.camera.xyz);
    let near = 1.0 - smoothstep(40.0, 140.0, view_distance);
    let depth = in.data.y;
    let forest = in.data.z;
    let bank = in.data.w;

    let broad = noise2(xz * 0.045);
    let medium = fbm2_low(xz * 0.35);
    let fine = noise2(xz * 3.1);

    // Small bumps tilt the normal so flat ground still catches the light.
    var n = normalize(in.normal);
    let e = 0.35;
    let bump = vec2<f32>(
        fbm2_low((xz + vec2<f32>(e, 0.0)) * 0.9) - fbm2_low((xz - vec2<f32>(e, 0.0)) * 0.9),
        fbm2_low((xz + vec2<f32>(0.0, e)) * 0.9) - fbm2_low((xz - vec2<f32>(0.0, e)) * 0.9),
    );
    n = normalize(n + vec3<f32>(-bump.x, 0.0, -bump.y) * g.detail.z * near);
    let slope = 1.0 - n.y;

    let grass = mix(vec3<f32>(0.05, 0.11, 0.025), vec3<f32>(0.12, 0.18, 0.045), broad) * (0.75 + 0.5 * medium);
    // Forest floor: needle litter with moss patches and scattered fallen leaves.
    var needles = mix(vec3<f32>(0.13, 0.085, 0.045), vec3<f32>(0.06, 0.1, 0.03), smoothstep(0.35, 0.7, medium));
    let litter = smoothstep(0.72, 0.8, noise2(xz * 7.0)) * near;
    needles = mix(needles, vec3<f32>(0.3, 0.17, 0.05), litter * 0.7) * (0.75 + 0.5 * fine);
    let strata = sin(in.world.y * 1.7 + medium * 5.0) * 0.5 + 0.5;
    let cracks = smoothstep(0.42, 0.5, abs(noise2(xz * 0.22 + in.world.y * 0.13) - 0.5) * 2.0);
    let stone = mix(vec3<f32>(0.15, 0.15, 0.145), vec3<f32>(0.27, 0.265, 0.25), strata * 0.55 + fine * 0.45) * (0.72 + 0.5 * broad) * (0.55 + 0.45 * cracks);
    let pebble = pebbles(xz * 9.0 + vec2<f32>(medium, fine) * 0.8);
    let gravel = mix(vec3<f32>(0.2, 0.19, 0.17), vec3<f32>(0.5, 0.47, 0.42), pebble.y)
        * mix(1.0, 0.62 + 0.5 * (1.0 - smoothstep(0.1, 0.6, pebble.x)), near);

    let rock_weight = smoothstep(0.26, 0.44, slope + (medium - 0.5) * 0.16);
    let gravel_weight = 1.0 - smoothstep(0.4, 4.5, bank + (medium - 0.5) * 3.0);
    var albedo = mix(grass, needles, smoothstep(0.3, 0.75, forest + (medium - 0.5) * 0.4));
    albedo = mix(albedo, gravel, gravel_weight);
    albedo = mix(albedo, stone, rock_weight);

    // Beyond the placed trees the slopes are painted as closed canopy.
    let outside = smoothstep(500.0, 530.0, max(abs(xz.x), abs(xz.y)));
    let crowns = noise2(xz * 0.16) * 0.6 + noise2(xz * 0.41) * 0.4;
    let canopy = mix(vec3<f32>(0.02, 0.05, 0.02), vec3<f32>(0.06, 0.13, 0.04), crowns);
    let canopy_weight = outside * forest * (1.0 - rock_weight);
    albedo = mix(albedo, canopy, canopy_weight);

    var roughness = 0.9;
    var specular = 0.3;
    // Ground just above the waterline is wet: darker and shinier.
    let wet = smoothstep(-0.45, -0.02, depth) * (1.0 - smoothstep(0.0, 0.05, depth));
    albedo = albedo * mix(1.0, 0.55, max(wet, smoothstep(0.0, 0.05, depth)));
    roughness = mix(roughness, 0.35, wet);
    specular = mix(specular, 1.0, wet);

    var surface: Surface;
    surface.albedo = albedo;
    surface.normal = n;
    surface.roughness = roughness;
    surface.ao = in.data.x * mix(1.0, 0.55, canopy_weight * (1.0 - crowns));
    surface.translucency = vec3<f32>(0.0);
    surface.specular = specular;

    let shadow = sun_shadow(in.world, normalize(in.normal));
    var color = shade(surface, in.world, shadow);

    if (depth > 0.0) {
        // Under water: light is absorbed on the way down and focused into caustics.
        let absorb = exp(-depth * vec3<f32>(1.5, 0.5, 0.4) / g.water_color.w);
        let sun = g.sun_color.rgb * g.sun_dir.w;
        let lines = caustics(xz, g.camera.w) * shadow * saturate(g.sun_dir.y * 3.0);
        color = color * absorb + albedo * sun * lines * g.detail.w * absorb * near;
    }
    return vec4<f32>(apply_fog(color, in.world), 1.0);
}
