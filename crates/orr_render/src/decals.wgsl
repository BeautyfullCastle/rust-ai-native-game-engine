// Decals: one fullscreen pass after the opaque scene. Each pixel rebuilds its
// world position from depth, finds the decal boxes around it and returns the
// product of their colors. The pipeline multiplies the lit scene by it.

struct GpuDecal {
    // World to box space: x, y and z run -0.5..0.5 inside the box.
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    color: vec4<f32>,
    region: vec4<f32>,
    // Square shape, softness, atlas region, unused.
    style: vec4<f32>,
};

struct Decals {
    inv_view_proj: mat4x4<f32>,
    info: vec4<f32>,
    list: array<GpuDecal, 32>,
};

@group(0) @binding(0) var<uniform> decals: Decals;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

@vertex fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(positions[vi], 0.0, 1.0);
}

fn world_at(p: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(depth));
    let q = clamp(p, vec2<i32>(0), size - 1);
    let d = textureLoad(depth, q, 0);
    let uv = (vec2<f32>(q) + 0.5) / vec2<f32>(size);
    let w = decals.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return w.xyz / w.w;
}

@fragment fn fs_decals(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(frag.xy);
    if textureLoad(depth, p, 0) >= 1.0 {
        return vec4<f32>(1.0);
    }
    let world = world_at(p);
    // The nearer neighbour on each axis keeps silhouette edges from bending the normal.
    // A neighbour outside the view is skipped, since it would clamp back onto this pixel.
    let size = vec2<i32>(textureDimensions(depth));
    let has_l = p.x > 0;
    let has_r = p.x + 1 < size.x;
    let has_u = p.y > 0;
    let has_b = p.y + 1 < size.y;
    let l = world - world_at(p - vec2<i32>(1, 0));
    let r = world_at(p + vec2<i32>(1, 0)) - world;
    let u = world - world_at(p - vec2<i32>(0, 1));
    let b = world_at(p + vec2<i32>(0, 1)) - world;
    var dx = vec3<f32>(0.0);
    if has_l && has_r {
        dx = select(l, r, dot(r, r) < dot(l, l));
    } else if has_l {
        dx = l;
    } else if has_r {
        dx = r;
    }
    var dy = vec3<f32>(0.0);
    if has_u && has_b {
        dy = select(u, b, dot(b, b) < dot(u, u));
    } else if has_u {
        dy = u;
    } else if has_b {
        dy = b;
    }
    // Screen x runs right and y down, so this cross points toward the camera.
    // A view one pixel wide or high has no difference to build it from: the decal then faces the projection.
    let c = cross(dy, dx);
    let known = dot(c, c) > 0.0;
    let n = select(vec3<f32>(0.0), normalize(c), known);
    var factor = vec3<f32>(1.0);
    let count = u32(decals.info.x);
    for (var i = 0u; i < count; i++) {
        let k = decals.list[i];
        let local = vec3<f32>(dot(k.right.xyz, world) + k.right.w, dot(k.up.xyz, world) + k.up.w, dot(k.forward.xyz, world) + k.forward.w);
        if any(abs(local) > vec3<f32>(0.5)) {
            continue;
        }
        // Surfaces turned away from the projection fade out instead of showing stretched stripes.
        let angle = select(1.0, smoothstep(0.1, 0.35, dot(n, -normalize(k.forward.xyz))), known);
        let soft = max(k.style.y, 1e-3) * 0.5;
        var tint = k.color.rgb;
        var cover: f32;
        if k.style.z > 0.5 {
            let uv = vec2<f32>(local.x + 0.5, 0.5 - local.y);
            // Half a texel inside the region, so filtering never reads the neighbouring region.
            let half = 0.5 / vec2<f32>(textureDimensions(atlas));
            let lo = min(k.region.xy, k.region.zw) + half;
            let hi = max(k.region.xy, k.region.zw) - half;
            let at = clamp(mix(k.region.xy, k.region.zw, uv), min(lo, hi), max(lo, hi));
            let texel = textureSampleLevel(atlas, atlas_sampler, at, 0.0);
            tint *= texel.rgb;
            cover = texel.a;
        } else if k.style.x > 0.5 {
            cover = smoothstep(0.0, soft, 0.5 - max(abs(local.x), abs(local.y)));
        } else {
            cover = smoothstep(0.0, soft, 0.5 - length(local.xy));
        }
        let ends = smoothstep(0.0, 0.1, 0.5 - abs(local.z));
        let a = clamp(k.color.a * cover * angle * ends, 0.0, 1.0);
        factor *= mix(vec3<f32>(1.0), tint, a);
    }
    return vec4<f32>(factor, 1.0);
}
