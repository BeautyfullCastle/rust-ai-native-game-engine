use super::*;
use orr_model::animation::{
    AnimatedModel, AnimatedNode, AnimationChannel, AnimationClip, ChannelValues, Interpolation, Trs,
};
fn fixture() -> AnimatedModel {
    let original = AnimatedModel::from_bytes(include_bytes!(
        "../../../../../assets/room_character_demo/courier.orrmodel.json"
    ))
    .unwrap();
    let mut s = original.source().clone();
    s.nodes = vec![AnimatedNode {
        name: "root".into(),
        children: vec![],
        rest: Trs {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        },
    }];
    s.skins.clear();
    s.primitives.truncate(1);
    let p = &mut s.primitives[0];
    p.node = 0;
    p.skin = None;
    p.vertices.truncate(3);
    p.indices = vec![0, 1, 2];
    for (v, pos) in p
        .vertices
        .iter_mut()
        .zip([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 4.0, 0.0]])
    {
        v.vertex.position = pos;
        v.joints = [0; 4];
        v.weights = [1.0, 0.0, 0.0, 0.0];
    }
    s.clips = vec![AnimationClip {
        name: "translation".into(),
        channels: vec![AnimationChannel {
            node: 0,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: ChannelValues::Translation(vec![[0.0; 3], [4.0, 0.0, 0.0]]),
        }],
    }];
    AnimatedModel::new(s).unwrap()
}
#[test]
fn independent_literal_pose_bounds_control_fit_including_exact_end() {
    let model = fixture();
    let original = model.to_bytes().unwrap();
    for t in [0.0f32, 0.5, 1.0] {
        let fit = sampled_model(&model, 0, t, Target::Box([1.0; 3])).unwrap();
        assert_eq!(fit.scale, [0.495; 3]);
        assert_eq!(fit.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert!((fit.translation[0] + (1.0 + 4.0 * t) * 0.495).abs() < 1e-6);
        assert!((fit.translation[1] + 0.99).abs() < 1e-6);
    }
    assert_eq!(model.to_bytes().unwrap(), original);
}
#[test]
fn sampled_box_and_sphere_fit_contain_independent_vertices() {
    let model = fixture();
    for target in [Target::Box([1.0, 2.0, 3.0]), Target::Sphere(1.25)] {
        let fit = sampled_model(&model, 0, 1.0, target).unwrap();
        for p in [[4.0f32, 0.0, 0.0], [6.0, 0.0, 0.0], [4.0, 4.0, 0.0]] {
            let q: [f32; 3] = std::array::from_fn(|i| p[i] * fit.scale[i] + fit.translation[i]);
            match target {
                Target::Box(half) => assert!((0..3).all(|i| f64::from(q[i].abs()) <= half[i])),
                Target::Sphere(radius) => {
                    assert!(q.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() <= radius * radius)
                }
            }
        }
    }
}
#[test]
fn invalid_time_clip_and_dimensions_reject_without_model_mutation() {
    let model = fixture();
    let original = model.to_bytes().unwrap();
    for time in [-0.01, 1.00001, f32::NAN, f32::INFINITY] {
        assert!(sampled_model(&model, 0, time, Target::Box([1.0; 3])).is_err());
    }
    assert!(sampled_model(&model, u32::MAX, 0.0, Target::Sphere(1.0)).is_err());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(sampled_model(&model, 0, 0.0, Target::Box([bad, 1.0, 1.0])).is_err());
        assert!(sampled_model(&model, 0, 0.0, Target::Sphere(bad)).is_err());
    }
    assert_eq!(model.to_bytes().unwrap(), original);
}
#[test]
fn degenerate_sampled_geometry_rejects_instead_of_guessing_scale() {
    let mut s = fixture().source().clone();
    for v in &mut s.primitives[0].vertices {
        v.vertex.position = [0.0; 3];
    }
    let model = AnimatedModel::new(s).unwrap();
    assert!(sampled_model(&model, 0, 1.0, Target::Box([1.0; 3])).is_err());
    assert!(sampled_model(&model, 0, 1.0, Target::Sphere(1.0)).is_err());
}
