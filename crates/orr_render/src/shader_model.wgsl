struct Globals {
    view_proj: mat4x4<f32>, direction: vec4<f32>, sun: vec4<f32>,
    // params: exposure, tone map, shader sRGB encode, scene-linear HDR output.
    sky: vec4<f32>, ground: vec4<f32>, params: vec4<f32>,
    point_position_range: vec4<f32>, point_color_intensity: vec4<f32>,
    light_vp: mat4x4<f32>, shadow: vec4<f32>,
};
struct Object {
    world: mat4x4<f32>, normal: mat4x4<f32>, color: vec4<f32>, sampling: vec4<u32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var<uniform> object: Object;
struct Varying {
    @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) world_position: vec3<f32>,
};
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> Varying {
    var out: Varying;
    let world=model_world(position);
    out.position=globals.view_proj*world;
    out.world_position=world.xyz;
    out.normal=(object.normal*vec4<f32>(normal,0.0)).xyz;
    out.uv=uv;
    return out;
}
// Explicit addressing keeps sampler behavior backend-neutral without expanding
// RHI. Bilinear taps wrap individually, including across repeat/mirror seams.
fn address(p:i32,n:i32,mode:u32) -> i32 {
    if mode==0u { return clamp(p,0,n-1); }
    if mode==1u { return ((p%n)+n)%n; }
    let v=((p%(2*n))+2*n)%(2*n);
    return select(2*n-1-v,v,v<n);
}
fn texel(p:vec2<i32>,size:vec2<i32>) -> vec4<f32> {
    return textureLoad(base_color,vec2<i32>(address(p.x,size.x,object.sampling.x),address(p.y,size.y,object.sampling.y)),0);
}
fn reduce_uv(v:f32,mode:u32) -> f32 {
    if mode==0u { return clamp(v,0.0,1.0); }
    // Tiny negative inputs may round fract(v) up to 1.0. Keep the
    // last-texel side for nearest sampling rather than wrapping to texel zero.
    if mode==1u { return min(fract(v),0.9999999403953552); }
    return v-2.0*floor(v/2.0);
}
fn sample_base(uv:vec2<f32>) -> vec4<f32> {
    let size=vec2<i32>(textureDimensions(base_color));
    // Reduce before multiplying by extent. Otherwise large allowed UVs lose
    // the half-texel offset in f32 and incorrectly collapse bilinear seam taps.
    let reduced=vec2<f32>(reduce_uv(uv.x,object.sampling.x),reduce_uv(uv.y,object.sampling.y));
    let p=reduced*vec2<f32>(size);
    if object.sampling.z==0u { return texel(vec2<i32>(floor(p)),size); }
    let low=vec2<i32>(floor(p-0.5));
    let t=fract(p-0.5);
    let a=mix(texel(low,size),texel(low+vec2<i32>(1,0),size),t.x);
    let b=mix(texel(low+vec2<i32>(0,1),size),texel(low+vec2<i32>(1,1),size),t.x);
    return mix(a,b,t.y);
}
fn aces(x:vec3<f32>) -> vec3<f32> {
    return clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14),vec3<f32>(0.0),vec3<f32>(1.0));
}
fn linear_to_srgb(c:vec3<f32>) -> vec3<f32> {
    return select(1.055*pow(max(c,vec3<f32>(0.0)),vec3<f32>(1.0/2.4))-0.055,12.92*c,c<=vec3<f32>(0.0031308));
}
// Bounded stylized diffuse, intentionally not inverse-square/PBR lighting.
// Disabled and coincident lights return before dividing by range/distance.
fn point_diffuse(world_position:vec3<f32>,normal:vec3<f32>) -> vec3<f32> {
    if globals.point_color_intensity.w<=0.0 { return vec3<f32>(0.0); }
    let delta=globals.point_position_range.xyz-world_position;
    let distance=length(delta);
    if distance<=1e-6 { return vec3<f32>(0.0); }
    let attenuation=max(1.0-distance/globals.point_position_range.w,0.0);
    let diffuse=max(dot(normal,delta/distance),0.0);
    return globals.point_color_intensity.xyz*globals.point_color_intensity.w
        *diffuse*attenuation*attenuation;
}
fn shade(in:Varying, visibility:f32) -> vec4<f32> {
    let n=in.normal*inverseSqrt(max(dot(in.normal,in.normal),1e-20));
    let ambient=mix(globals.ground.xyz,globals.sky.xyz,n.y*0.5+0.5)*globals.sky.w;
    let sun=globals.sun.xyz*globals.sun.w*max(dot(n,-globals.direction.xyz),0.0)*visibility;
    let point=point_diffuse(in.world_position,n);
    var color=sample_base(in.uv).rgb*object.color.rgb*(ambient+sun+point);
    // Finite admitted lighting may exceed half-float range; saturate only at
    // representation limits, never at display white, before the FP16 write.
    if globals.params.w>0.5 { return vec4<f32>(clamp(color,vec3<f32>(0.0),vec3<f32>(65504.0)),1.0); }
    color*=globals.params.x;
    if globals.params.y>0.5 { color=aces(color); } else { color=clamp(color,vec3<f32>(0.0),vec3<f32>(1.0)); }
    if globals.params.z>0.5 { color=linear_to_srgb(color); }
    // glTF OPAQUE ignores texture/factor alpha.
    return vec4<f32>(color,1.0);
}

@group(1) @binding(0) var shared_shadow_map: texture_depth_2d;
@group(1) @binding(1) var shared_shadow_sampler: sampler_comparison;
fn shared_visibility(world:vec3<f32>, n:vec3<f32>) -> f32 {
    let lp=globals.light_vp*vec4<f32>(world+n*globals.shadow.y,1.0);
    let uv=vec2<f32>(lp.x*0.5+0.5,0.5-lp.y*0.5);
    if uv.x<0.0 || uv.x>1.0 || uv.y<0.0 || uv.y>1.0 || lp.z<0.0 || lp.z>1.0 { return 1.0; }
    var sum=0.0;
    for (var y=-1; y<=1; y=y+1) { for (var x=-1; x<=1; x=x+1) {
        sum+=textureSampleCompareLevel(shared_shadow_map,shared_shadow_sampler,uv+vec2<f32>(f32(x),f32(y))*globals.shadow.x,lp.z-globals.shadow.z);
    } }
    return sum/9.0;
}
@fragment fn fs_main(in:Varying) -> @location(0) vec4<f32> { return shade(in,1.0); }
@fragment fn fs_composed(in:Varying) -> @location(0) vec4<f32> {
    let n=in.normal*inverseSqrt(max(dot(in.normal,in.normal),1e-20));
    return shade(in,shared_visibility(in.world_position,n));
}

fn model_world(position:vec3<f32>) -> vec4<f32> { return object.world*vec4<f32>(position,1.0); }
@vertex fn vs_shadow(@location(0) position:vec3<f32>) -> @builtin(position) vec4<f32> { return globals.light_vp*model_world(position); }
