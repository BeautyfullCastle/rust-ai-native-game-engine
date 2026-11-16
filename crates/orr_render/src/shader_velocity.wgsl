// Motion vectors: each vertex goes to the screen as the scene drew it and
// carries its clip position of this frame (without the jitter) and of the
// last frame. The fragment stores the move between them in texture
// coordinates, with 1 in alpha. An instance with no earlier place has the
// same world position in both, so only the camera moves it.

struct Camera {
    // World to clip space as the scene was drawn, with the jitter.
    raster: mat4x4<f32>,
    // World to clip space without the jitter.
    clean: mat4x4<f32>,
    // The last frame's clean matrix.
    previous: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct Moved {
    @builtin(position) position: vec4<f32>,
    @location(0) now: vec4<f32>,
    @location(1) before: vec4<f32>,
};

fn moved(world: vec3<f32>, before: vec3<f32>) -> Moved {
    var out: Moved;
    out.position = camera.raster * vec4<f32>(world, 1.0);
    out.now = camera.clean * vec4<f32>(world, 1.0);
    out.before = camera.previous * vec4<f32>(before, 1.0);
    return out;
}

@fragment fn fs_velocity(in: Moved) -> @location(0) vec4<f32> {
    // A point behind either camera has no move on the screen.
    if (in.now.w <= 1e-6 || in.before.w <= 1e-6) {
        return vec4<f32>(0.0);
    }
    let now = in.now.xy / in.now.w;
    let before = in.before.xy / in.before.w;
    return vec4<f32>((now - before) * vec2<f32>(0.5, -0.5), 0.0, 1.0);
}

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// Procedural meshes: the instance of this frame and the same instance of the last frame.
fn place(pos: vec3<f32>, cap: f32, pos_ext: vec4<f32>, rot: vec4<f32>, scale: vec3<f32>) -> vec3<f32> {
    return pos_ext.xyz + quat_rotate(rot, pos * scale + vec3<f32>(0.0, cap * pos_ext.w, 0.0));
}

@vertex fn vs_mesh(
    @location(0) pos: vec3<f32>,
    @location(2) cap: f32,
    @location(3) pos_ext: vec4<f32>,
    @location(4) rot: vec4<f32>,
    @location(5) scale_flags: vec4<f32>,
    @location(8) before_pos_ext: vec4<f32>,
    @location(9) before_rot: vec4<f32>,
    @location(10) before_scale_flags: vec4<f32>,
) -> Moved {
    return moved(
        place(pos, cap, pos_ext, rot, scale_flags.xyz),
        place(pos, cap, before_pos_ext, before_rot, before_scale_flags.xyz),
    );
}

// Static models: one placement matrix per draw, now and before.
struct Placement {
    world: mat4x4<f32>,
    before: mat4x4<f32>,
};

@group(0) @binding(1) var<uniform> placement: Placement;

@vertex fn vs_static(@location(0) position: vec3<f32>) -> Moved {
    return moved(
        (placement.world * vec4<f32>(position, 1.0)).xyz,
        (placement.before * vec4<f32>(position, 1.0)).xyz,
    );
}

// Skinned models: the model placement and joint palette of the draw, now and before.
struct Pose {
    world: mat4x4<f32>,
    joints: array<mat4x4<f32>, 64>,
};

struct Poses {
    now: Pose,
    before: Pose,
};

@group(0) @binding(2) var<uniform> poses: Poses;

fn skinned_now(position: vec3<f32>, j: vec4<u32>, w: vec4<f32>) -> vec3<f32> {
    let weight = w / dot(w, vec4<f32>(1.0));
    var skin = poses.now.joints[j.x] * weight.x + poses.now.joints[j.y] * weight.y
        + poses.now.joints[j.z] * weight.z + poses.now.joints[j.w] * weight.w;
    skin[0][3] = 0.0; skin[1][3] = 0.0; skin[2][3] = 0.0; skin[3][3] = 1.0;
    return (poses.now.world * skin * vec4<f32>(position, 1.0)).xyz;
}

fn skinned_before(position: vec3<f32>, j: vec4<u32>, w: vec4<f32>) -> vec3<f32> {
    let weight = w / dot(w, vec4<f32>(1.0));
    var skin = poses.before.joints[j.x] * weight.x + poses.before.joints[j.y] * weight.y
        + poses.before.joints[j.z] * weight.z + poses.before.joints[j.w] * weight.w;
    skin[0][3] = 0.0; skin[1][3] = 0.0; skin[2][3] = 0.0; skin[3][3] = 1.0;
    return (poses.before.world * skin * vec4<f32>(position, 1.0)).xyz;
}

@vertex fn vs_skinned(
    @location(0) position: vec3<f32>,
    @location(3) j0: u32, @location(4) j1: u32, @location(5) j2: u32, @location(6) j3: u32,
    @location(7) weights: vec4<f32>,
) -> Moved {
    let joints = vec4<u32>(j0, j1, j2, j3);
    return moved(skinned_now(position, joints, weights), skinned_before(position, joints, weights));
}
