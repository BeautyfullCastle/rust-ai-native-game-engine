// Horizontal stripes in world space: they stay put while the shape moves through them.
// @param tint color = 0.95, 0.35, 0.1
// @param stripes float 0.5..20 = 3
// @param sharpness float 0..1 = 0.8

fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    let wave = fract(in.world.y * params.stripes);
    let edge = 0.5 * (1.0 - params.sharpness) + 0.001;
    let band = smoothstep(0.5 - edge, 0.5 + edge, wave);
    s.albedo = mix(in.color, params.tint, band);
    return s;
}
