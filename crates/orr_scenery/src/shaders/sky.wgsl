// The sky is one triangle at the far plane, drawn after the opaque scene.
// It writes alpha 0 (reduced by cloud cover) so the light shaft pass can find open sky.

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ray: vec3<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyOut {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let p = corners[index];
    var o: SkyOut;
    o.clip = vec4<f32>(p, 1.0, 1.0);
    o.ray = g.cam_forward.xyz + p.x * g.cam_right.xyz + p.y * g.cam_up.xyz;
    return o;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let d = normalize(in.ray);
    let sun = g.sun_dir.xyz;
    var color = sky_base(d);
    var cover = 0.0;
    if (d.y > 0.015) {
        // Clouds live on a flat layer; the divisor bends it toward the horizon.
        let drift = g.camera.w * g.sky.w;
        let p = d.xz / (d.y + 0.14) * 1.25 + vec2<f32>(drift * 0.004, drift * 0.0015);
        let density = fbm2(p);
        cover = smoothstep(g.sky.z, g.sky.z + 0.22, density) * smoothstep(0.015, 0.16, d.y);
        // A second sample toward the sun tells how much cloud the light crossed.
        let toward = fbm2(p + sun.xz * 0.22);
        let lit = saturate(1.0 - (toward - 0.5) * 2.4);
        let mu = max(dot(d, sun), 0.0);
        let bright = g.sun_color.rgb * (1.5 + 1.6 * pow(mu, 12.0)) + g.ambient_sky.rgb * 0.7;
        let dark = g.ambient_sky.rgb * 0.95 + vec3<f32>(0.12);
        color = mix(color, mix(dark, bright, lit), cover * 0.92);
    }
    let mu = dot(d, sun);
    let disc = smoothstep(0.99994, 0.99998, mu) * (1.0 - cover);
    color = color + g.sun_color.rgb * disc * 60.0;
    // A tight halo makes the disc read as the sun, not as a pale dot.
    let halo = pow(max(mu, 0.0), 2500.0) * 6.0 + pow(max(mu, 0.0), 300.0) * 0.7;
    color = color + g.sun_color.rgb * halo * (1.0 - 0.6 * cover);
    return vec4<f32>(min(color, vec3<f32>(60000.0)), cover * 0.7);
}
