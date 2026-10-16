// Scene data is finite, nonnegative linear RGBA16F. Bloom remains in this space.
struct Params { display: vec4<f32>, bloom_params: vec4<f32> };
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> p: Params;
struct Screen { @builtin(position) position: vec4<f32> };
@vertex fn vs_fullscreen(@builtin(vertex_index) vi:u32) -> Screen {
    var positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    var out:Screen; out.position=vec4<f32>(positions[vi],0.0,1.0); return out;
}
fn load_scene(pos:vec2<i32>) -> vec3<f32> {
    return textureLoad(scene,clamp(pos,vec2<i32>(0),vec2<i32>(textureDimensions(scene))-1),0).rgb;
}
fn bright(c:vec3<f32>) -> vec3<f32> {
    let peak=max(c.r,max(c.g,c.b));
    return c*(max(peak-p.bloom_params.x,0.0)/max(peak,1e-20));
}
@fragment fn fs_extract(in:Screen) -> @location(0) vec4<f32> {
    let q=vec2<i32>(in.position.xy)*2;
    // Threshold each full-resolution tap, then average. Odd edges clamp safely.
    let c=(bright(load_scene(q))+bright(load_scene(q+vec2<i32>(1,0)))+bright(load_scene(q+vec2<i32>(0,1)))+bright(load_scene(q+vec2<i32>(1,1))))*0.25;
    return vec4<f32>(clamp(c,vec3<f32>(0.0),vec3<f32>(65504.0)),1.0);
}
@fragment fn fs_blur(in:Screen) -> @location(0) vec4<f32> {
    let q=vec2<i32>(in.position.xy); let radius=i32(p.bloom_params.y); let axis=vec2<i32>(p.bloom_params.zw);
    var sum=vec3<f32>(0.0); var total=0.0;
    for(var i=-8;i<=8;i+=1) { if abs(i)<=radius { let weight=f32(radius+1-abs(i)); sum+=load_scene(q+axis*i)*weight; total+=weight; } }
    return vec4<f32>(clamp(sum/total,vec3<f32>(0.0),vec3<f32>(65504.0)),1.0);
}
fn aces(x:vec3<f32>) -> vec3<f32> {
    return clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14),vec3<f32>(0.0),vec3<f32>(1.0));
}
fn linear_to_srgb(c:vec3<f32>) -> vec3<f32> {
    return select(1.055*pow(c,vec3<f32>(1.0/2.4))-0.055,12.92*c,c<=vec3<f32>(0.0031308));
}
@fragment fn fs_final(in:Screen) -> @location(0) vec4<f32> {
    let pos=vec2<i32>(in.position.xy); let source=textureLoad(scene,pos,0);
    var c=source.rgb;
    if p.display.w>0.0 {
        let uv=in.position.xy/vec2<f32>(textureDimensions(scene));
        c+=textureSampleLevel(bloom,linear_sampler,uv,0.0).rgb*p.display.w;
    }
    c*=p.display.x;
    if p.display.y>0.5 { c=aces(c); } else { c=clamp(c,vec3<f32>(0.0),vec3<f32>(1.0)); }
    if p.display.z>0.5 { c=linear_to_srgb(c); }
    return vec4<f32>(c,clamp(source.a,0.0,1.0));
}
