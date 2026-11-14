//! Cross-fades, state machine transitions, clip events, root motion, the 1D
//! blend space, masked and additive layers, and two-bone IK, on a three-joint leg.
#![cfg(feature = "animation")]
#![allow(clippy::float_arithmetic)]
use orr_model::animation::*;
use orr_model::{Dependency, Image, Material, Vertex, Wrap, IDENTITY};

const IDLE: u32 = 0;
const WALK: u32 = 1;
const RUN: u32 = 2;

/// Quaternion for `degrees` about z.
fn about_z(degrees: f32) -> [f32; 4] {
    let half = degrees.to_radians() * 0.5;
    [0.0, 0.0, half.sin(), half.cos()]
}

fn translation_channel(node: u32, end: f32, x: f32) -> AnimationChannel {
    AnimationChannel { node, interpolation: Interpolation::Linear, times: vec![0.0, end], values: ChannelValues::Translation(vec![[0.0; 3], [x, 0.0, 0.0]]) }
}

fn knee_channel(end: f32, degrees: f32) -> AnimationChannel {
    AnimationChannel { node: 1, interpolation: Interpolation::Linear, times: vec![0.0, end], values: ChannelValues::Rotation(vec![about_z(0.0), about_z(degrees)]) }
}

/// Hip at the origin, knee 1 below it, foot 1 below the knee. Idle holds still for
/// 1 s, walk moves the hip 2 along x and bends the knee 90 degrees in 1 s, run
/// moves the hip 4 and bends the knee 45 degrees in 0.5 s.
fn leg() -> AnimatedModel {
    let node = |name: &str, children: Vec<u32>, y: f32| AnimatedNode { name: name.into(), children, rest: Trs { translation: [0.0, y, 0.0], ..Trs::default() } };
    let vertex = |position, joint| SkinnedVertex { vertex: Vertex { position, normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0] }, joints: [joint, 0, 0, 0], weights: [1.0, 0.0, 0.0, 0.0] };
    let down = |y: f32| {
        let mut m = IDENTITY;
        m[3][1] = y;
        m
    };
    AnimatedModel::new(AnimatedSource {
        format: "orr_animated_model".into(),
        version: 1,
        asset_id: "leg.gltf".into(),
        dependencies: vec![Dependency { uri: "$source".into(), sha256: "a".repeat(64) }],
        materials: vec![Material { base_color: [1.0; 4], image: 0, linear_filter: false, wrap_s: Wrap::Clamp, wrap_t: Wrap::Clamp }],
        images: vec![Image { width: 1, height: 1, rgba8: vec![255; 4] }],
        nodes: vec![node("hip", vec![1], 0.0), node("knee", vec![2], -1.0), node("foot", vec![], -1.0)],
        skins: vec![Skin { name: "leg".into(), joints: vec![0, 1, 2], inverse_bind_matrices: vec![IDENTITY, down(1.0), down(2.0)] }],
        primitives: vec![AnimatedPrimitive {
            id: "leg.gltf#node=0/mesh=0/primitive=0".into(),
            node: 0,
            skin: Some(0),
            vertices: vec![vertex([0.0, 0.0, 0.0], 0), vertex([0.1, -1.0, 0.0], 1), vertex([0.0, -2.0, 0.0], 2)],
            indices: vec![0, 1, 2],
            material: 0,
        }],
        clips: vec![
            AnimationClip { name: "idle".into(), channels: vec![translation_channel(0, 1.0, 0.0)] },
            AnimationClip { name: "walk".into(), channels: vec![translation_channel(0, 1.0, 2.0), knee_channel(1.0, 90.0)] },
            AnimationClip { name: "run".into(), channels: vec![translation_channel(0, 0.5, 4.0), knee_channel(0.5, 45.0)] },
        ],
    })
    .unwrap()
}

fn state(name: &str, clip: u32) -> AnimationState {
    AnimationState { name: name.into(), motion: Motion::Clip(clip), looping: true, speed: 1.0 }
}

fn to_walk(fade: f32) -> Transition {
    Transition { from: Some(0), to: 1, conditions: vec![Condition::Above("speed".into(), 0.1)], fade, exit_time: None }
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

fn close_pose(a: &Pose, b: &Pose) {
    for (x, y) in a.local().iter().zip(b.local()) {
        for k in 0..3 {
            close(x.translation[k], y.translation[k]);
        }
        let same = (0..4).all(|k| (x.rotation[k] - y.rotation[k]).abs() < 1e-3) || (0..4).all(|k| (x.rotation[k] + y.rotation[k]).abs() < 1e-3);
        assert!(same, "{:?} != {:?}", x.rotation, y.rotation);
    }
}

fn foot(pose: &Pose) -> [f32; 3] {
    let m = pose.global()[2];
    [m[3][0], m[3][1], m[3][2]]
}

#[test]
fn a_transition_cross_fades_from_the_old_state_to_the_new_one() {
    let model = leg();
    let mut machine = StateMachine::new(&model, vec![state("idle", IDLE), state("walk", WALK)], vec![to_walk(0.5)], vec![]).unwrap();
    machine.update(&model, 0.0).unwrap();
    assert_eq!(machine.current(), 0, "no transition before the parameter changes");
    machine.set_float("speed", 1.0);
    machine.update(&model, 0.0).unwrap();
    assert_eq!(machine.current(), 1);
    assert!(machine.is_fading());
    machine.update(&model, 0.25).unwrap();
    let expected = model.blend_poses(&model.sample_clip(IDLE, 0.25).unwrap(), &model.sample_clip(WALK, 0.25).unwrap(), 0.5).unwrap();
    close_pose(&machine.pose(&model).unwrap(), &expected);
    machine.update(&model, 0.25).unwrap();
    assert!(!machine.is_fading(), "the fade ends after its length");
    close_pose(&machine.pose(&model).unwrap(), &model.sample_clip(WALK, 0.5).unwrap());
}

#[test]
fn a_trigger_is_used_once_and_exit_time_waits_for_the_cycle() {
    let model = leg();
    let back = Transition { from: Some(1), to: 0, conditions: vec![Condition::Trigger("stop".into())], fade: 0.0, exit_time: Some(0.9) };
    let mut machine = StateMachine::new(&model, vec![state("idle", IDLE), state("walk", WALK)], vec![to_walk(0.0), back], vec![]).unwrap();
    machine.set_float("speed", 1.0);
    machine.update(&model, 0.0).unwrap();
    assert_eq!(machine.current(), 1);
    machine.set_float("speed", 0.0);
    machine.trigger("stop");
    machine.update(&model, 0.5).unwrap();
    assert_eq!(machine.current(), 1, "half the cycle is before the exit time");
    machine.update(&model, 0.45).unwrap();
    assert_eq!(machine.current(), 0, "the trigger is taken at the exit time");
    machine.set_float("speed", 1.0);
    machine.update(&model, 0.0).unwrap();
    machine.update(&model, 0.95).unwrap();
    assert_eq!(machine.current(), 1, "the used trigger does not fire again");
}

#[test]
fn clip_events_are_reported_once_including_across_the_loop() {
    let model = leg();
    let events = vec![ClipEvent { clip: WALK, time: 0.1, name: "left".into() }, ClipEvent { clip: WALK, time: 0.6, name: "right".into() }];
    let mut machine = StateMachine::new(&model, vec![state("walk", WALK)], vec![], events).unwrap();
    assert_eq!(machine.update(&model, 0.05).unwrap(), Vec::<String>::new());
    assert_eq!(machine.update(&model, 0.5).unwrap(), vec!["left"]);
    assert_eq!(machine.update(&model, 0.6).unwrap(), vec!["right", "left"], "0.55 to 0.15 passes the end of the loop");
}

#[test]
fn root_motion_moves_out_of_the_pose_and_adds_up_across_loops() {
    let model = leg();
    let mut machine = StateMachine::new(&model, vec![state("walk", WALK)], vec![], vec![]).unwrap();
    machine.extract_root_motion(&model, "hip").unwrap();
    machine.update(&model, 0.5).unwrap();
    close(machine.take_root_motion()[0], 1.0);
    close(machine.pose(&model).unwrap().local()[0].translation[0], 0.0);
    machine.update(&model, 0.75).unwrap();
    close(machine.take_root_motion()[0], 1.5);
    close(machine.take_root_motion()[0], 0.0);
}

#[test]
fn a_blend_space_keeps_both_clips_in_phase() {
    let model = leg();
    let space = BlendSpace1D::new(&model, vec![(4.0, RUN), (2.0, WALK), (0.0, IDLE)]).unwrap();
    close(space.duration(&model, 3.0).unwrap(), 0.75);
    let pose = space.sample(&model, 3.0, 0.5).unwrap();
    let expected = model.blend_poses(&model.sample_clip(WALK, 0.5).unwrap(), &model.sample_clip(RUN, 0.25).unwrap(), 0.5).unwrap();
    close_pose(&pose, &expected);
    close_pose(&space.sample(&model, 9.0, 0.5).unwrap(), &model.sample_clip(RUN, 0.25).unwrap());
    assert!(BlendSpace1D::new(&model, vec![(1.0, WALK), (1.0, RUN)]).is_err());

    let blend = AnimationState { name: "move".into(), motion: Motion::Blend { space, parameter: "speed".into() }, looping: true, speed: 1.0 };
    let mut machine = StateMachine::new(&model, vec![blend], vec![], vec![]).unwrap();
    machine.set_float("speed", 2.0);
    machine.update(&model, 0.5).unwrap();
    close(machine.phase(), 0.5);
}

#[test]
fn a_masked_layer_changes_only_its_bones_and_an_additive_one_adds_its_change() {
    let model = leg();
    let knee_down = BoneMask::subtree(&model, "knee").unwrap();
    assert_eq!((knee_down.weight(0), knee_down.weight(1), knee_down.weight(2)), (0.0, 1.0, 1.0));
    let base = model.sample_clip(RUN, 0.25).unwrap();
    let over = model.sample_clip(WALK, 1.0).unwrap();
    let masked = model.blend_masked(&base, &over, 1.0, &knee_down).unwrap();
    close(masked.local()[0].translation[0], base.local()[0].translation[0]);
    close_pose(&model.pose_from_trs(masked.local().to_vec()).unwrap(), &masked);
    assert_eq!(masked.local()[1].rotation, over.local()[1].rotation);

    // The walk's change from its first frame, added on top of the run pose.
    let reference = model.sample_clip(WALK, 0.0).unwrap();
    let added = model.add_pose(&base, &over, &reference, 1.0, &BoneMask::all(&model)).unwrap();
    close(added.local()[0].translation[0], base.local()[0].translation[0] + 2.0);
    let knee = added.local()[1].rotation;
    let expected = about_z(22.5 + 90.0);
    assert!((0..4).all(|k| (knee[k] - expected[k]).abs() < 1e-3), "{knee:?} != {expected:?}");
}

#[test]
fn an_animator_stacks_an_upper_layer_over_the_base() {
    let model = leg();
    let base = StateMachine::new(&model, vec![state("run", RUN)], vec![], vec![]).unwrap();
    let top = StateMachine::new(&model, vec![state("walk", WALK)], vec![], vec![]).unwrap();
    let mut animator = Animator::new(vec![
        AnimationLayer { machine: base, mask: BoneMask::all(&model), weight: 1.0, blend: LayerBlend::Override },
        AnimationLayer { machine: top, mask: BoneMask::subtree(&model, "knee").unwrap(), weight: 0.5, blend: LayerBlend::Override },
    ])
    .unwrap();
    animator.update(&model, 0.25).unwrap();
    let pose = animator.pose(&model).unwrap();
    let expected = model.blend_masked(&model.sample_clip(RUN, 0.25).unwrap(), &model.sample_clip(WALK, 0.25).unwrap(), 0.5, &BoneMask::subtree(&model, "knee").unwrap()).unwrap();
    close_pose(&pose, &expected);
}

#[test]
fn two_bone_ik_puts_the_foot_on_a_reachable_target_and_reaches_toward_a_far_one() {
    let model = leg();
    let rest = model.rest_pose().unwrap();
    let target = [0.8, -1.4, 0.3];
    let bent = model.two_bone_ik(&rest, "foot", target, [1.0, -1.0, 2.0]).unwrap();
    let reached = foot(&bent);
    for k in 0..3 {
        close(reached[k], target[k]);
    }
    let knee = bent.global()[1][3];
    assert!(knee[2] > 0.05, "the knee turns toward the pole: {knee:?}");
    let far = model.two_bone_ik(&rest, "foot", [10.0, 0.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
    let stretched = foot(&far);
    close((stretched[0] * stretched[0] + stretched[1] * stretched[1] + stretched[2] * stretched[2]).sqrt(), 2.0);
    assert!(stretched[0] > 1.99, "the straight leg points at the far target: {stretched:?}");
    assert!(model.two_bone_ik(&rest, "hip", target, [0.0, 0.0, 1.0]).is_err(), "the hip has no grandparent");
}

#[test]
fn invalid_machines_are_refused() {
    let model = leg();
    let bad_to = Transition { to: 7, ..to_walk(0.0) };
    assert!(StateMachine::new(&model, vec![state("idle", IDLE)], vec![bad_to], vec![]).is_err());
    assert!(StateMachine::new(&model, vec![state("idle", 9)], vec![], vec![]).is_err());
    assert!(StateMachine::new(&model, vec![], vec![], vec![]).is_err());
    let late = ClipEvent { clip: RUN, time: 0.9, name: "late".into() };
    assert!(StateMachine::new(&model, vec![state("run", RUN)], vec![], vec![late]).is_err());
    let mut machine = StateMachine::new(&model, vec![state("idle", IDLE)], vec![], vec![]).unwrap();
    assert!(machine.update(&model, f32::NAN).is_err());
}

#[test]
fn a_2d_blend_space_plays_a_clip_alone_on_its_point_and_mixes_between_points() {
    let model = leg();
    let space = BlendSpace2D::new(&model, vec![([0.0, 0.0], IDLE), ([2.0, 0.0], WALK), ([0.0, 2.0], RUN)]).unwrap();
    let on_walk = space.weights([2.0, 0.0]);
    close(on_walk[1], 1.0);
    let between = space.weights([1.0, 0.0]);
    close(between[0], 0.5);
    close(between[1], 0.5);
    close(between[2], 0.0);
    close(space.weights([9.0, 9.0]).iter().sum::<f32>(), 1.0);
    let expected = model.blend_poses(&model.sample_clip(IDLE, 0.5).unwrap(), &model.sample_clip(WALK, 0.5).unwrap(), 0.5).unwrap();
    close_pose(&space.sample(&model, [1.0, 0.0], 0.5).unwrap(), &expected);
    assert!(BlendSpace2D::new(&model, vec![([1.0, 1.0], IDLE), ([1.0, 1.0], RUN)]).is_err());

    let state = AnimationState { name: "strafe".into(), motion: Motion::Blend2D { space, x: "side".into(), y: "ahead".into() }, looping: true, speed: 1.0 };
    let mut machine = StateMachine::new(&model, vec![state], vec![], vec![]).unwrap();
    machine.set_float("ahead", 2.0);
    machine.update(&model, 0.25).unwrap();
    close(machine.phase(), 0.5);
    close_pose(&machine.pose(&model).unwrap(), &model.sample_clip(RUN, 0.25).unwrap());
}

#[test]
fn look_at_turns_a_node_toward_a_point_within_its_limit() {
    let model = leg();
    let rest = model.rest_pose().unwrap();
    // The knee's own -y axis runs down to the foot; aim it sideways at knee height.
    let turned = model.look_at(&rest, "knee", [0.0, -1.0, 0.0], [5.0, -1.0, 0.0], 3.2, 1.0).unwrap();
    let f = foot(&turned);
    close(f[0], 1.0);
    close(f[1], -1.0);
    let limited = model.look_at(&rest, "knee", [0.0, -1.0, 0.0], [5.0, -1.0, 0.0], 0.5, 1.0).unwrap();
    let g = foot(&limited);
    close(g[0], 0.5f32.sin());
    let half = model.look_at(&rest, "knee", [0.0, -1.0, 0.0], [5.0, -1.0, 0.0], 3.2, 0.5).unwrap();
    close(foot(&half)[0], std::f32::consts::FRAC_PI_4.sin());
    assert!(model.look_at(&rest, "knee", [0.0; 3], [1.0, 0.0, 0.0], 1.0, 1.0).is_err());
}
