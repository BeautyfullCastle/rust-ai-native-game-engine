// The contract of a surface shader, the same on shapes and models. The user's
// file defines
//     fn surface(in: SurfaceInput) -> Surface
// and reads its declared parameters from `params`. The engine glue in front of
// this file (surface_glue.wgsl for shapes, surface_model_glue.wgsl for models)
// fills `surface_raw` and calls `surface`.

// What a surface shader knows about the point it shades.
struct SurfaceInput {
    // World position.
    world: vec3<f32>,
    // Position in the object's own space, before rotation: on a shape, a unit sphere, box or capsule
    // scaled to its size; on a model, the vertex position of its mesh.
    local: vec3<f32>,
    // Unit world-space normal of the shape or mesh.
    normal: vec3<f32>,
    // Texture coordinates of a model's mesh; zero on a shape.
    uv: vec2<f32>,
    // Base color: a shape's instance color (with the ground checker of a checkered shape), or a
    // model's texture times its material color. Then roughness, metallic and emissive strength.
    color: vec3<f32>,
    roughness: f32,
    metallic: f32,
    emissive: f32,
    // Seconds of the clock the host gives the renderer (the editor: since it started).
    time: f32,
};

// The built-in look of the instance: a start value for a surface shader to change.
fn default_surface(in: SurfaceInput) -> Surface {
    var s: Surface;
    s.albedo = in.color;
    s.normal = in.normal;
    s.roughness = in.roughness;
    s.metallic = in.metallic;
    s.emissive = in.color * in.emissive;
    return s;
}

// The surface normal bent by a tangent-space normal map. `texel` is the map's
// color as read (0..1, flat is 0.5, 0.5, 1); `strength` 0 keeps `in.normal` and
// 1 uses the map as drawn. The tangent frame comes from how world position and
// texture coordinates change across the screen, so meshes need no tangents.
// Shapes have no texture coordinates and keep their normal. Call it outside
// branches that differ between neighboring pixels: it reads screen derivatives.
fn normal_map(in: SurfaceInput, texel: vec3<f32>, strength: f32) -> vec3<f32> {
    let dp1 = dpdx(in.world);
    let dp2 = dpdy(in.world);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let n = in.normal;
    // Framebuffer y runs down, the opposite of the usual derivation: both crosses are flipped.
    let across2 = cross(n, dp2);
    let across1 = cross(dp1, n);
    let tangent = across2 * duv1.x + across1 * duv2.x;
    let bitangent = across2 * duv1.y + across1 * duv2.y;
    let scale = inverseSqrt(max(max(dot(tangent, tangent), dot(bitangent, bitangent)), 1e-20));
    let m = texel * 2.0 - 1.0;
    let bent = tangent * scale * m.x * strength + bitangent * scale * m.y * strength + n * max(m.z, 1e-3);
    return normalize(bent);
}

var<private> surface_raw: array<f32, 16>;

// Surface textures. The glue of each renderer binds `surface_texture_0` to
// `surface_texture_3`, each with its whole mip chain, and `surface_sampler`,
// which repeats and filters between mip levels.

// Zero. The renderers bind all four slots and the sampler for every shader,
// and this use keeps each in the pipeline's binding layout when a shader
// reads fewer. Texture values are at most 1, so the step is always zero.
fn surface_textures_keep() -> f32 {
    let n = textureDimensions(surface_texture_0).x + textureDimensions(surface_texture_1).x
        + textureDimensions(surface_texture_2).x + textureDimensions(surface_texture_3).x;
    return f32(min(n, 0u)) + step(2.0, textureSampleLevel(surface_texture_0, surface_sampler, vec2<f32>(0.0), 0.0).x);
}
