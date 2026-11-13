// A surface that glows and fades over time, like a warning light.
// @param glow color = 0.2, 0.9, 1.0
// @param speed float 0..10 = 2
// @param strength float 0..5 = 1.5

fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    let pulse = 0.5 + 0.5 * sin(in.time * params.speed * 6.2831853);
    s.albedo = in.color * 0.3;
    s.emissive = params.glow * params.strength * pulse;
    s.roughness = 0.3;
    return s;
}
