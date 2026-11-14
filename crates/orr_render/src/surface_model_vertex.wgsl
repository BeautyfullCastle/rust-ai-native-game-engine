// The vertex entry of a surface shader on a static model, after surface_model_glue.wgsl.

@vertex fn vs_model_surface(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> ModelSurfaceOut {
    var out: ModelSurfaceOut;
    let world = model_world(position);
    out.position = globals.view_proj * world;
    out.world_position = world.xyz;
    out.normal = (object.normal * vec4<f32>(normal, 0.0)).xyz;
    out.uv = uv;
    out.local = position;
    return out;
}
