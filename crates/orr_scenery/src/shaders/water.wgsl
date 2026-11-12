// The stream surface: moving ripples, sky reflection by Fresnel, depth tint,
// whitewater at the cascades and a thin foam line at the shore.

struct WaterIn {
    @location(0) position: vec3<f32>,
    // x: depth, y: cascade, z: across (-1 to 1), w: surface slope per meter.
    @location(1) data: vec4<f32>,
    @location(2) flow: vec2<f32>,
};

struct WaterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) data: vec4<f32>,
    @location(2) flow: vec2<f32>,
};

@vertex
fn vs_water(i: WaterIn) -> WaterOut {
    var o: WaterOut;
    o.clip = g.view_proj * vec4<f32>(i.position, 1.0);
    o.world = i.position;
    o.data = i.data;
    o.flow = i.flow;
    return o;
}

fn ripple(p: vec2<f32>, flow: vec2<f32>, speed: f32) -> f32 {
    let t = g.camera.w;
    let a = p * 1.3 - flow * t * speed;
    let b = p * 3.1 - flow * t * speed * 1.7 + vec2<f32>(3.1, 1.7);
    let c = p * 7.3 - flow * t * speed * 2.6 + vec2<f32>(t * 0.2, 0.0);
    return noise2(a) * 0.55 + noise2(b) * 0.3 + noise2(c) * 0.15;
}

@fragment
fn fs_water(in: WaterOut) -> @location(0) vec4<f32> {
    let depth = in.data.x;
    if (depth < -0.05) {
        discard;
    }
    let cascade = in.data.y;
    let flow = normalize(in.flow);
    let xz = in.world.xz;
    let speed = mix(0.45, 3.2, cascade) * g.water.y;
    let view_distance = distance(in.world, g.camera.xyz);
    let near = 1.0 - smoothstep(60.0, 220.0, view_distance);

    let e = 0.12;
    let strength = mix(0.07, 0.4, cascade) * (0.35 + 0.65 * near) * g.water.x;
    let dx = ripple(xz + vec2<f32>(e, 0.0), flow, speed) - ripple(xz - vec2<f32>(e, 0.0), flow, speed);
    let dz = ripple(xz + vec2<f32>(0.0, e), flow, speed) - ripple(xz - vec2<f32>(0.0, e), flow, speed);
    let n = normalize(vec3<f32>(-dx * strength / e, 1.0, -dz * strength / e - in.data.w));

    let v = normalize(g.camera.xyz - in.world);
    let ndv = max(dot(n, v), 0.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
    let r = reflect(-v, n);
    // Low reflection rays hit the forest, not the sky.
    let trees = mix(vec3<f32>(0.012, 0.03, 0.012), vec3<f32>(0.03, 0.065, 0.02), noise2(r.xz * 6.0)) * (g.ambient_sky.rgb * 2.5 + g.sun_color.rgb * g.sun_dir.w * 0.25);
    let up = vec3<f32>(r.x, abs(r.y), r.z);
    let reflection = mix(trees, sky_base(up), smoothstep(0.22, 0.6, up.y));

    let shadow = sun_shadow(in.world, vec3<f32>(0.0, 1.0, 0.0));
    let sun = g.sun_color.rgb * g.sun_dir.w;
    let glint = (pow(max(dot(r, g.sun_dir.xyz), 0.0), 900.0) * 6.0 + pow(max(dot(r, g.sun_dir.xyz), 0.0), 80.0) * 0.03) * g.water.w;

    // Water body: clear in the shallows, green where it is deep.
    let deep = max(depth, 0.0);
    let transmittance = exp(-deep * vec3<f32>(1.6, 0.55, 0.5) / g.water_color.w);
    let opacity = 1.0 - (transmittance.r + transmittance.g + transmittance.b) / 3.0;
    let scatter = g.water_color.rgb * (sun * (0.25 + 0.75 * shadow) * saturate(g.sun_dir.y * 2.0 + 0.2) + g.ambient_sky.rgb);

    // Whitewater streaks stretch along the flow.
    let along = dot(xz, flow);
    let across = dot(xz, vec2<f32>(-flow.y, flow.x));
    let t = g.camera.w;
    let streak = fbm2(vec2<f32>(across * 1.7, along * 0.38 - t * speed * 0.55));
    let churn = noise2(vec2<f32>(across * 4.0, along * 1.4 - t * speed * 1.2));
    var foam = smoothstep(0.18, 0.68, (streak - 0.5) * 2.4 + (churn - 0.5) * 0.9 + cascade * 0.95 - 0.3) * cascade;
    // Still water meets the bank with a thin broken foam line.
    let shore = (1.0 - smoothstep(0.0, 0.09, depth)) * smoothstep(0.35, 0.65, noise2(xz * 2.6 + vec2<f32>(t * 0.1, 0.0)));
    foam = saturate((foam + shore * 0.55) * g.water.z);
    // Foam is lit like snow; fine noise gives it lumps and blue troughs.
    let lumps = noise2(vec2<f32>(across * 9.0, along * 3.5 - t * speed * 2.0));
    let foam_color = mix(vec3<f32>(0.62, 0.74, 0.8), vec3<f32>(0.95, 0.97, 0.98), smoothstep(0.25, 0.7, lumps * 0.6 + streak * 0.4))
        * (sun * (0.2 + 0.8 * shadow) * saturate(g.sun_dir.y + 0.25) * 0.3 + g.ambient_sky.rgb * 1.5);

    let body_alpha = opacity;
    var alpha = fresnel + (1.0 - fresnel) * body_alpha;
    var color = (reflection * fresnel + scatter * (1.0 - fresnel) * body_alpha) / max(alpha, 1e-3);
    color = color + sun * shadow * glint * (1.0 - foam);
    color = mix(color, foam_color, foam);
    alpha = max(alpha, foam);
    // Fade the last few centimeters so the edge does not show the mesh.
    alpha = alpha * smoothstep(-0.05, 0.03, depth);
    alpha = max(alpha, saturate(glint * shadow) * smoothstep(-0.05, 0.03, depth));
    return vec4<f32>(apply_fog(color, in.world), saturate(alpha));
}
