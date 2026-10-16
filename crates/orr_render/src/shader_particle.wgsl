struct Globals { view_projection: mat4x4<f32>, right: vec4<f32>, up: vec4<f32> }
@group(0) @binding(0) var<uniform> globals: Globals;
struct VertexOut { @builtin(position) clip: vec4<f32>, @location(0) local: vec2<f32>, @location(1) color: vec4<f32> }
@vertex fn vs_particle(@builtin(vertex_index) vertex: u32, @location(0) position_size: vec4<f32>, @location(1) color: vec4<f32>) -> VertexOut {
    let corners = array<vec2<f32>, 6>(vec2(-1.0,-1.0), vec2(1.0,-1.0), vec2(1.0,1.0), vec2(-1.0,-1.0), vec2(1.0,1.0), vec2(-1.0,1.0));
    let local = corners[vertex];
    let world = position_size.xyz + (globals.right.xyz * local.x + globals.up.xyz * local.y) * position_size.w * 0.5;
    var out: VertexOut; out.clip = globals.view_projection * vec4(world, 1.0); out.local = local; out.color = color; return out;
}
@fragment fn fs_particle(in: VertexOut) -> @location(0) vec4<f32> {
    let radius = length(in.local);
    if radius >= 1.0 { discard; }
    let coverage = 1.0 - smoothstep(0.65, 1.0, radius);
    return vec4(in.color.rgb, in.color.a * coverage);
}
