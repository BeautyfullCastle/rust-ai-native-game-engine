// The vertex entry of a surface shader on a skinned model, after
// surface_model_glue.wgsl. It skins like vs_main of shader_skinned.wgsl; the
// local position is the vertex in the bind pose.

@vertex fn vs_skinned_surface(
    @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>,
    @location(3) joint0: u32, @location(4) joint1: u32, @location(5) joint2: u32, @location(6) joint3: u32,
    @location(7) weights: vec4<f32>
) -> ModelSurfaceOut {
    let skin = blend_skin(vec4<u32>(joint0, joint1, joint2, joint3), weights);
    let a = skin[0].xyz;
    let b = skin[1].xyz;
    let c = skin[2].xyz;
    let cofactor = mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));
    let skinned_normal = (cofactor * normal) / dot(a, cross(b, c));
    var out: ModelSurfaceOut;
    let world = skinned_world(position, skin);
    out.position = globals.view_proj * world;
    out.world_position = world.xyz;
    out.normal = normalize((object.normal * vec4<f32>(skinned_normal, 0.0)).xyz);
    out.uv = uv;
    out.local = position;
    return out;
}
