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

var<private> surface_raw: array<f32, 16>;
