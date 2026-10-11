#![cfg(feature = "animation")]
#![allow(clippy::float_arithmetic)]
use orr_model::animation::*;
use orr_model::{Dependency, Image, Material, Vertex, Wrap, IDENTITY};

fn translation(x: f32, y: f32, z: f32) -> Matrix4 {
    let mut matrix = IDENTITY;
    matrix[3] = [x, y, z, 1.0];
    matrix
}
fn source() -> AnimatedSource {
    let vertex = |position, joint| SkinnedVertex {
        vertex: Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        joints: [joint, 0, 0, 0],
        weights: [1.0, 0.0, 0.0, 0.0],
    };
    AnimatedSource {
        format: "orr_animated_model".into(),
        version: 1,
        asset_id: "character.gltf".into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "a".repeat(64),
        }],
        materials: vec![Material {
            base_color: [1.0; 4],
            image: 0,
            linear_filter: false,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        }],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
        // Intentionally not topologically sorted. A non-joint parent carries
        // x translation; the mesh node differs from the skeleton transform.
        nodes: vec![
            AnimatedNode {
                name: "tip".into(),
                children: vec![],
                rest: Trs {
                    translation: [0.0, 1.0, 0.0],
                    ..Trs::default()
                },
            },
            AnimatedNode {
                name: "mesh".into(),
                children: vec![],
                rest: Trs {
                    translation: [50.0, 0.0, 0.0],
                    ..Trs::default()
                },
            },
            AnimatedNode {
                name: "joint".into(),
                children: vec![0],
                rest: Trs {
                    translation: [0.0, 1.0, 0.0],
                    ..Trs::default()
                },
            },
            AnimatedNode {
                name: "parent".into(),
                children: vec![1, 2],
                rest: Trs {
                    translation: [2.0, 0.0, 0.0],
                    ..Trs::default()
                },
            },
        ],
        skins: vec![Skin {
            name: "skeleton".into(),
            joints: vec![2, 0],
            inverse_bind_matrices: vec![translation(-2.0, -1.0, 0.0), translation(-2.0, -2.0, 0.0)],
        }],
        primitives: vec![AnimatedPrimitive {
            id: "character.gltf#node=1/mesh=0/primitive=0".into(),
            node: 1,
            skin: Some(0),
            vertices: vec![
                vertex([0.0, 0.0, 0.0], 0),
                vertex([1.0, 0.0, 0.0], 1),
                vertex([0.0, 1.0, 0.0], 1),
            ],
            indices: vec![0, 1, 2],
            material: 0,
        }],
        clips: vec![AnimationClip {
            name: "move".into(),
            channels: vec![AnimationChannel {
                node: 2,
                interpolation: Interpolation::Linear,
                times: vec![2.0, 4.0],
                values: ChannelValues::Translation(vec![[0.0, 1.0, 0.0], [2.0, 1.0, 0.0]]),
            }],
        }],
    }
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}
fn close3(a: [f32; 3], b: [f32; 3]) {
    for i in 0..3 {
        close(a[i], b[i]);
    }
}
fn rejected(mut change: impl FnMut(&mut AnimatedSource), message: &str) {
    let mut input = source();
    change(&mut input);
    let error = AnimatedModel::new(input).unwrap_err();
    assert!(
        error.to_string().contains(message),
        "unexpected error: {error}"
    );
}

#[test]
fn rest_hierarchy_inverse_bind_and_mesh_transform_convention() {
    let model = AnimatedModel::new(source()).unwrap();
    let pose = model.rest_pose().unwrap();
    close3(
        pose.global()[0][3][..3].try_into().unwrap(),
        [2.0, 2.0, 0.0],
    );
    close3(
        pose.global()[1][3][..3].try_into().unwrap(),
        [52.0, 0.0, 0.0],
    );
    for matrix in &pose.skin_matrices()[0] {
        assert_eq!(*matrix, IDENTITY);
    }
    let deformed = model.deform(&pose).unwrap();
    assert_eq!(deformed[0].transform, IDENTITY);
    close3(deformed[0].vertices[0].position, [0.0, 0.0, 0.0]);
    close3(deformed[0].vertices[1].position, [1.0, 0.0, 0.0]);
}

#[test]
fn channel_clamps_before_first_and_after_last_key_independently() {
    let mut input = source();
    input.clips[0].channels.push(AnimationChannel {
        node: 0,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 10.0],
        values: ChannelValues::Translation(vec![[0.0, 1.0, 0.0], [0.0, 11.0, 0.0]]),
    });
    let model = AnimatedModel::new(input).unwrap();
    close3(
        model.sample_clip(0, -10.0).unwrap().local()[2].translation,
        [0.0, 1.0, 0.0],
    );
    close3(
        model.sample_clip(0, 1.0).unwrap().local()[2].translation,
        [0.0, 1.0, 0.0],
    );
    close3(
        model.sample_clip(0, 3.0).unwrap().local()[2].translation,
        [1.0, 1.0, 0.0],
    );
    let pose = model.sample_clip(0, 7.0).unwrap();
    close3(pose.local()[2].translation, [2.0, 1.0, 0.0]);
    close3(pose.local()[0].translation, [0.0, 8.0, 0.0]);
}

#[test]
fn step_exact_boundary_and_single_key() {
    let mut input = source();
    input.clips[0].channels[0].interpolation = Interpolation::Step;
    input.clips[0].channels.push(AnimationChannel {
        node: 0,
        interpolation: Interpolation::Linear,
        times: vec![3.0],
        values: ChannelValues::Scale(vec![[2.0, 3.0, 4.0]]),
    });
    let model = AnimatedModel::new(input).unwrap();
    close3(
        model.sample_clip(0, 3.999).unwrap().local()[2].translation,
        [0.0, 1.0, 0.0],
    );
    close3(
        model.sample_clip(0, 4.0).unwrap().local()[2].translation,
        [2.0, 1.0, 0.0],
    );
    for time in [-10.0, 0.0, 3.0, 100.0] {
        close3(
            model.sample_clip(0, time).unwrap().local()[0].scale,
            [2.0, 3.0, 4.0],
        );
    }
}

#[test]
fn rotation_slerp_is_shortest_path_and_normalized() {
    let mut input = source();
    input.clips[0].channels = vec![AnimationChannel {
        node: 2,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 2.0],
        values: ChannelValues::Rotation(vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, -0.70710677, -0.70710677],
        ]),
    }];
    let model = AnimatedModel::new(input).unwrap();
    let q = model.sample_clip(0, 1.0).unwrap().local()[2].rotation;
    close(q[2], (std::f32::consts::PI / 8.0).sin());
    close(q[3], (std::f32::consts::PI / 8.0).cos());
    close(q.iter().map(|x| x * x).sum(), 1.0);
}

#[test]
fn antipodal_rotations_are_identical_and_do_not_generate_nan() {
    let mut input = source();
    input.clips[0].channels = vec![AnimationChannel {
        node: 2,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 2.0],
        values: ChannelValues::Rotation(vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0]]),
    }];
    let model = AnimatedModel::new(input).unwrap();
    for t in [0.0, 0.5, 1.0, 2.0] {
        let pose = model.sample_clip(0, t).unwrap();
        close(pose.local()[2].rotation[3].abs(), 1.0);
        model.deform(&pose).unwrap();
    }
}

#[test]
fn once_loop_pause_resume_seek_stop_and_reset_are_explicit() {
    let model = AnimatedModel::new(source()).unwrap();
    let mut player = AnimationPlayer::new();
    assert_eq!(player.state(), PlaybackState::Stopped);
    assert_eq!(player.pose(&model).unwrap(), model.rest_pose().unwrap());
    assert!(player.seek(&model, 1.0).is_err());
    player.play(&model, 0, PlaybackMode::Once).unwrap();
    player.advance(&model, 3.0).unwrap();
    player.pause();
    player.advance(&model, 100.0).unwrap();
    close(player.time(), 3.0);
    player.seek(&model, 2.5).unwrap();
    assert_eq!(player.state(), PlaybackState::Paused);
    player.resume();
    player.advance(&model, 1.5).unwrap();
    assert_eq!(player.state(), PlaybackState::Finished);
    close(player.time(), 4.0);
    player.advance(&model, 100.0).unwrap();
    close(player.time(), 4.0);
    player.reset_clip();
    assert_eq!(player.state(), PlaybackState::Paused);
    close(player.time(), 0.0);
    player.resume();
    assert_eq!(player.state(), PlaybackState::Playing);
    player.play(&model, 0, PlaybackMode::Loop).unwrap();
    player.advance(&model, 4.0).unwrap();
    close(player.time(), 0.0);
    player.advance(&model, 11.0).unwrap();
    close(player.time(), 3.0);
    player.seek(&model, 9.0).unwrap();
    close(player.time(), 1.0);
    let before = player.clone();
    for bad in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(player.seek(&model, bad).is_err());
        assert_eq!(before, player);
        assert!(player.advance(&model, bad).is_err());
        assert_eq!(before, player);
    }
    assert!(player.play(&model, 99, PlaybackMode::Loop).is_err());
    assert_eq!(before, player);
    player.stop();
    assert_eq!(player.clip(), None);
    assert_eq!(player.pose(&model).unwrap(), model.rest_pose().unwrap());
}

#[test]
fn zero_duration_and_large_finite_delta_are_safe() {
    let mut input = source();
    input.clips[0].channels[0].times = vec![0.0];
    input.clips[0].channels[0].values = ChannelValues::Translation(vec![[1.0, 1.0, 0.0]]);
    let model = AnimatedModel::new(input).unwrap();
    let mut player = AnimationPlayer::new();
    player.play(&model, 0, PlaybackMode::Loop).unwrap();
    for _ in 0..2 {
        player.advance(&model, f32::MAX).unwrap();
        close(player.time(), 0.0);
    }
    assert_eq!(player.state(), PlaybackState::Playing);
    player.play(&model, 0, PlaybackMode::Once).unwrap();
    player.advance(&model, 0.0).unwrap();
    assert_eq!(player.state(), PlaybackState::Finished);
}

#[test]
fn clip_switch_and_repeated_sampling_always_start_from_rest() {
    let mut input = source();
    input.clips.push(AnimationClip {
        name: "scale-only".into(),
        channels: vec![AnimationChannel {
            node: 0,
            interpolation: Interpolation::Step,
            times: vec![0.0],
            values: ChannelValues::Scale(vec![[2.0; 3]]),
        }],
    });
    let model = AnimatedModel::new(input).unwrap();
    let mut player = AnimationPlayer::new();
    player.play(&model, 0, PlaybackMode::Once).unwrap();
    player.seek(&model, 4.0).unwrap();
    close3(
        player.pose(&model).unwrap().local()[2].translation,
        [2.0, 1.0, 0.0],
    );
    player.play(&model, 1, PlaybackMode::Once).unwrap();
    close(player.time(), 0.0);
    let pose = player.pose(&model).unwrap();
    close3(pose.local()[2].translation, [0.0, 1.0, 0.0]);
    close3(pose.local()[0].scale, [2.0; 3]);
    let late = model.sample_clip(0, 3.0).unwrap();
    model.sample_clip(1, 0.0).unwrap();
    assert_eq!(late, model.sample_clip(0, 3.0).unwrap());
}

#[test]
fn restored_cooked_asset_has_identical_pose_vertices_and_current_bounds() {
    let model = AnimatedModel::new(source()).unwrap();
    let restored = AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap();
    assert_eq!(model.source(), restored.source());
    let pose = model.sample_clip(0, 3.0).unwrap();
    let restored_pose = restored.sample_clip(0, 3.0).unwrap();
    assert_eq!(
        model.deform(&pose).unwrap(),
        restored.deform(&restored_pose).unwrap()
    );
    assert_eq!(
        model.bounds(&pose).unwrap(),
        Bounds {
            min: [1.0, 0.0, 0.0],
            max: [2.0, 1.0, 0.0]
        }
    );
    assert!(restored
        .bounds(&pose)
        .unwrap_err()
        .to_string()
        .contains("different"));
    model.clone().validate_pose(&pose).unwrap();
}

#[test]
fn rigid_primitive_uses_full_node_hierarchy() {
    let mut input = source();
    input.primitives[0].skin = None;
    for vertex in &mut input.primitives[0].vertices {
        vertex.joints = [0; 4];
    }
    let model = AnimatedModel::new(input).unwrap();
    close3(
        model.deform(&model.rest_pose().unwrap()).unwrap()[0].vertices[0].position,
        [52.0, 0.0, 0.0],
    );
}

#[test]
fn four_influences_blend_and_nonuniform_scale_normal_match_oracle() {
    let mut input = source();
    for index in 0..2 {
        input.nodes.push(AnimatedNode {
            name: format!("extra{index}"),
            children: vec![],
            rest: Trs::default(),
        });
        input.nodes[3].children.push((4 + index) as u32);
        input.skins[0].joints.push((4 + index) as u32);
        input.skins[0]
            .inverse_bind_matrices
            .push(translation(-2.0, 0.0, 0.0));
    }
    input.clips[0].channels = (0..4)
        .map(|index| AnimationChannel {
            node: input.skins[0].joints[index],
            interpolation: Interpolation::Step,
            times: vec![0.0],
            values: ChannelValues::Scale(vec![[2.0, 1.0, 1.0]]),
        })
        .collect();
    // Scaling the parent joint also scales its child joint. Use extra joints for
    // all-independent influences to make the expected blend directly auditable.
    input.nodes[2].children.clear();
    input.nodes[3].children.push(0);
    input.nodes[0].rest.translation = [0.0, 2.0, 0.0];
    for vertex in &mut input.primitives[0].vertices {
        vertex.joints = [0, 1, 2, 3];
        vertex.weights = [0.1, 0.2, 0.3, 0.4];
        vertex.vertex.normal = [
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ];
    }
    let model = AnimatedModel::new(input).unwrap();
    let pose = model.sample_clip(0, 0.0).unwrap();
    let output = model.deform(&pose).unwrap();
    close3(
        output[0].vertices[1].normal,
        [1.0 / 5.0f32.sqrt(), 2.0 / 5.0f32.sqrt(), 0.0],
    );
    // x = 2*(1 - parent_bind_x) + parent_x = 0 at this posed vertex.
    close(output[0].vertices[1].position[0], 0.0);
}

#[test]
fn singular_mixed_blend_is_rejected_before_rendering() {
    let mut input = source();
    for vertex in &mut input.primitives[0].vertices {
        vertex.joints = [0, 1, 0, 0];
        vertex.weights = [0.5, 0.5, 0.0, 0.0];
    }
    input.clips[0].channels = vec![AnimationChannel {
        node: 0,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 1.0],
        values: ChannelValues::Rotation(vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]]),
    }];
    let model = AnimatedModel::new(input).unwrap();
    let pose = model.sample_clip(0, 1.0).unwrap();
    assert!(model
        .deform(&pose)
        .unwrap_err()
        .to_string()
        .contains("singular"));
    assert!(model.bounds(&pose).is_err());
}

#[test]
fn hierarchy_rejects_invalid_children_multiple_parents_cycles_and_depth() {
    rejected(|s| s.nodes[2].children.push(99), "out of range");
    rejected(|s| s.nodes[3].children.push(0), "multiple parents");
    rejected(|s| s.nodes[3].children.push(2), "duplicate child");
    rejected(|s| s.nodes[0].children.push(3), "cycle");
    rejected(|s| s.nodes[0].children.push(0), "cycle");
    rejected(
        |s| {
            let mut parent = 0;
            for _ in 0..MAX_HIERARCHY_DEPTH {
                let child = s.nodes.len() as u32;
                s.nodes[parent].children.push(child);
                s.nodes.push(AnimatedNode {
                    name: String::new(),
                    children: vec![],
                    rest: Trs::default(),
                });
                parent = child as usize;
            }
        },
        "depth",
    );
}

#[test]
fn malformed_skin_references_counts_and_influences_are_rejected() {
    rejected(|s| s.skins[0].joints[0] = 99, "skin joint");
    rejected(|s| s.skins[0].joints[1] = 2, "duplicate skin joint");
    rejected(
        |s| {
            s.skins[0].inverse_bind_matrices.pop();
        },
        "count",
    );
    rejected(|s| s.primitives[0].skin = Some(99), "skin out of range");
    rejected(|s| s.primitives[0].node = 99, "node out of range");
    rejected(
        |s| s.primitives[0].vertices[0].joints[3] = 99,
        "joint out of range",
    );
    rejected(
        |s| s.primitives[0].vertices[0].weights = [0.0; 4],
        "weights",
    );
    rejected(
        |s| s.primitives[0].vertices[0].weights[0] = f32::NAN,
        "weights",
    );
    rejected(
        |s| s.primitives[0].vertices[0].weights = [1.1, -0.1, 0.0, 0.0],
        "weights",
    );
    rejected(
        |s| s.primitives[0].vertices[0].weights = [0.5, 0.5, 0.0, 0.0],
        "duplicate nonzero",
    );
    rejected(
        |s| {
            s.nodes[2].children.clear();
        },
        "hierarchy root",
    );
    rejected(
        |s| s.skins[0].inverse_bind_matrices[0][0][0] = 0.0,
        "singular",
    );
    rejected(
        |s| s.skins[0].inverse_bind_matrices[0][0][0] = -1.0,
        "mirrored",
    );
    rejected(
        |s| s.skins[0].inverse_bind_matrices[0][3][3] = 2.0,
        "non-affine",
    );
}

#[test]
fn channels_reject_duplicate_targets_bad_times_lengths_and_values() {
    rejected(
        |s| {
            let channel = s.clips[0].channels[0].clone();
            s.clips[0].channels.push(channel);
        },
        "duplicate",
    );
    rejected(|s| s.clips[0].channels[0].node = 99, "target");
    for times in [
        vec![1.0, 1.0],
        vec![2.0, 1.0],
        vec![-1.0, 2.0],
        vec![0.0, f32::NAN],
        vec![0.0, f32::INFINITY],
        vec![0.0],
        vec![],
    ] {
        rejected(|s| s.clips[0].channels[0].times = times.clone(), "times");
    }
    rejected(
        |s| s.clips[0].channels[0].values = ChannelValues::Translation(vec![[f32::NAN; 3]; 2]),
        "translation",
    );
    rejected(
        |s| s.clips[0].channels[0].values = ChannelValues::Rotation(vec![[0.0; 4]; 2]),
        "rotation",
    );
    rejected(
        |s| s.clips[0].channels[0].values = ChannelValues::Scale(vec![[-1.0, 1.0, 1.0]; 2]),
        "scale",
    );
    rejected(
        |s| s.clips[0].channels[0].values = ChannelValues::Scale(vec![[0.0, 1.0, 1.0]; 2]),
        "scale",
    );
    rejected(|s| s.nodes[0].rest.scale = [-1.0, -1.0, 1.0], "scale");
    rejected(
        |s| s.nodes[0].rest.rotation = [f32::INFINITY; 4],
        "rotation",
    );
    rejected(|s| s.nodes[0].rest.translation[0] = f32::NAN, "translation");
}

#[test]
fn all_collection_and_aggregate_key_budgets_apply_to_cooked_data() {
    rejected(
        |s| s.skins = vec![s.skins[0].clone(); MAX_SKINS + 1],
        "collection",
    );
    rejected(
        |s| s.clips = vec![s.clips[0].clone(); MAX_CLIPS + 1],
        "collection",
    );
    rejected(
        |s| s.clips[0].channels = vec![s.clips[0].channels[0].clone(); MAX_CHANNELS + 1],
        "channel count",
    );
    rejected(
        |s| {
            s.skins[0].joints = vec![0; MAX_JOINTS + 1];
            s.skins[0].inverse_bind_matrices = vec![IDENTITY; MAX_JOINTS + 1];
        },
        "joint/inverse bind count",
    );
    rejected(
        |s| {
            // Each clip alone is under the cap; the aggregate exceeds it.
            let count = MAX_ANIMATION_KEYS / 2 + 1;
            s.clips[0].channels[0].times = (0..count).map(|n| n as f32).collect();
            s.clips[0].channels[0].values =
                ChannelValues::Translation(vec![[0.0, 1.0, 0.0]; count]);
            s.clips.push(s.clips[0].clone());
        },
        "aggregate animation key",
    );
    rejected(
        |s| {
            // The shared decoded budget includes both images and skin vertex data.
            s.images[0] = Image {
                width: 2048,
                height: 2048,
                rgba8: vec![255; 2048 * 2048 * 4],
            };
            s.images.push(s.images[0].clone());
        },
        "decoded model",
    );
}

#[test]
fn static_contract_is_not_accepted_as_animation_and_unknown_fields_fail() {
    let model = AnimatedModel::new(source()).unwrap();
    let bytes = model.to_bytes().unwrap();
    assert!(orr_model::StaticModel::from_bytes(&bytes).is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["hidden_extension"] = true.into();
    assert!(AnimatedModel::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    rejected(|s| s.format = "orr_static_model".into(), "format/version");
    rejected(|s| s.version = 2, "format/version");
    rejected(|s| s.primitives[0].indices[2] = 99, "index out of range");
    rejected(|s| s.materials[0].image = 99, "material image");
}

#[test]
fn nonfinite_sampling_and_out_of_range_clips_fail() {
    let model = AnimatedModel::new(source()).unwrap();
    for time in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(model.sample_clip(0, time).is_err());
    }
    assert!(model.sample_clip(99, 0.0).is_err());
}

#[test]
fn player_rejects_foreign_assets_without_partial_state_changes() {
    let model = AnimatedModel::new(source()).unwrap();
    let other = AnimatedModel::new(source()).unwrap();
    let mut player = AnimationPlayer::new();
    player.play(&model, 0, PlaybackMode::Once).unwrap();
    player.advance(&model, 1.0).unwrap();
    let previous = player.clone();
    assert!(player.advance(&other, 1.0).is_err());
    assert_eq!(previous, player);
    assert!(player.seek(&other, 4.0).is_err());
    assert_eq!(previous, player);
    assert!(player.pose(&other).is_err());
    player.pause();
    assert!(player.advance(&other, 1.0).is_err());
    player.pose(&model.clone()).unwrap();
    player.play(&other, 0, PlaybackMode::Loop).unwrap();
    close(player.time(), 0.0);
    player.pose(&other).unwrap();
    assert!(player.pose(&model).is_err());
    player.stop();
    player.pose(&model).unwrap();
    player.pose(&other).unwrap();
}

#[test]
fn loop_remainder_rounding_never_exposes_exact_duration() {
    let model = AnimatedModel::new(source()).unwrap();
    let mut player = AnimationPlayer::new();
    player.play(&model, 0, PlaybackMode::Loop).unwrap();
    let duration = model.source().clips[0].duration();
    player
        .seek(&model, f32::from_bits(duration.to_bits() - 1))
        .unwrap();
    player.advance(&model, 1.5e-7).unwrap();
    assert!(player.time() < duration);
    assert_eq!(player.time(), 0.0);
    assert_eq!(player.state(), PlaybackState::Playing);
}

fn cubic_source() -> AnimatedSource {
    let mut source = source();
    let channel = &mut source.clips[0].channels[0];
    channel.interpolation = Interpolation::CubicSpline;
    // Two-second segment: midpoint x = 1 + (2 * 4 / 8) = 2.
    channel.values = ChannelValues::Translation(vec![
        [0.0; 3],
        [0.0, 1.0, 0.0],
        [4.0, 0.0, 0.0],
        [0.0; 3],
        [2.0, 1.0, 0.0],
        [0.0; 3],
    ]);
    source
}

#[test]
fn cubic_hermite_scales_tangents_and_drives_skin_deformation() {
    let model = AnimatedModel::new(cubic_source()).unwrap();
    for (time, x) in [
        (-1.0, 0.0),
        (2.0, 0.0),
        (2.5, 1.4375),
        (3.0, 2.0),
        (4.0, 2.0),
        (99.0, 2.0),
    ] {
        let pose = model.sample_clip(0, time).unwrap();
        close3(pose.local()[2].translation, [x, 1.0, 0.0]);
        // The first vertex is fully weighted to joint 2; bind cancels its rest.
        close3(
            model.deform(&pose).unwrap()[0].vertices[0].position,
            [x, 0.0, 0.0],
        );
        assert!(model.bounds(&pose).is_ok());
    }
    let bytes = model.to_bytes().unwrap();
    let reopened = AnimatedModel::from_bytes(&bytes).unwrap();
    assert_eq!(model.source(), reopened.source());
    // Sampling another time/clip/rest cannot contaminate a repeated seek.
    let want = model.sample_clip(0, 3.0).unwrap();
    model.sample_clip(0, 4.0).unwrap();
    model.rest_pose().unwrap();
    assert_eq!(want, model.sample_clip(0, 3.0).unwrap());
    assert_eq!(want.local(), reopened.sample_clip(0, 3.0).unwrap().local());
}

#[test]
fn cubic_rotation_normalizes_component_hermite_without_sign_flipping() {
    let mut source = cubic_source();
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![
        [0.0; 4],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0; 4],
        [0.0, 0.0, 0.0, -1.0],
        [0.0; 4],
    ]);
    let model = AnimatedModel::new(source).unwrap();
    let pose = model.sample_clip(0, 3.0).unwrap();
    assert_eq!(pose.local()[2].rotation, [0.0, 0.0, 1.0, 0.0]);
    assert!(model.deform(&pose).is_ok());
}

#[test]
fn cubic_tangents_allow_zero_negative_values_but_invalid_outputs_fail_closed() {
    let mut source = cubic_source();
    source.clips[0].channels[0].values = ChannelValues::Scale(vec![
        [0.0; 3], [1.0; 3], [-8.0; 3], [0.0; 3], [1.0; 3], [0.0; 3],
    ]);
    let model = AnimatedModel::new(source).unwrap();
    assert!(model.sample_clip(0, 3.0).is_err()); // Hermite overshoot cannot mirror geometry.
    let mut source = cubic_source();
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![
        [0.0; 4],
        [0.0, 0.0, 0.0, 1.0],
        [0.0; 4],
        [0.0; 4],
        [0.0, 0.0, 0.0, -1.0],
        [0.0; 4],
    ]);
    assert!(AnimatedModel::new(source)
        .unwrap()
        .sample_clip(0, 3.0)
        .is_err());
    let mut huge = cubic_source();
    if let ChannelValues::Translation(values) = &mut huge.clips[0].channels[0].values {
        values[2][0] = f32::MAX;
    }
    assert!(AnimatedModel::new(huge)
        .unwrap()
        .sample_clip(0, 3.0)
        .is_err());
    for index in [0, 2, 3, 5] {
        let mut source = cubic_source();
        if let ChannelValues::Translation(values) = &mut source.clips[0].channels[0].values {
            values[index][0] = f32::NAN;
        }
        assert!(AnimatedModel::new(source).is_err());
    }
    for times in [
        vec![2.0],
        vec![2.0, 2.0],
        vec![4.0, 2.0],
        vec![2.0, f32::INFINITY],
    ] {
        let mut source = cubic_source();
        source.clips[0].channels[0].times = times;
        assert!(AnimatedModel::new(source).is_err());
    }
    let model = AnimatedModel::new(cubic_source()).unwrap();
    for time in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(model.sample_clip(0, time).is_err());
    }
}

#[test]
fn cubic_tangent_entries_count_toward_aggregate_key_budget() {
    let mut source = cubic_source();
    let count = MAX_ANIMATION_KEYS / 3 + 1;
    let channel = &mut source.clips[0].channels[0];
    channel.times = (0..count).map(|index| index as f32).collect();
    channel.values = ChannelValues::Translation(vec![[0.0; 3]; count * 3]);
    assert!(AnimatedModel::new(source)
        .unwrap_err()
        .to_string()
        .contains("key budget"));
}
