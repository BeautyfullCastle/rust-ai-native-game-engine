#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
// Filesystem entrypoint is exercised by the real executable, not this module copy.
#[allow(dead_code)]
#[path = "../examples/animation_clip_sequence/authoring.rs"]
mod authoring;
#[allow(dead_code)]
#[path = "common/animation_clip_sequence_oracle.rs"]
mod oracle;
use orr_model::animation::{
    AnimatedModel, AnimatedSource, ChannelValues, Interpolation, MAX_ANIMATION_KEYS, MAX_CLIPS,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
const ID: &str = "sequences/join.sequence.json";
fn recipe() -> Value {
    json!({"format":"orr_animation_clip_sequence","version":1,"source":"source.animated.json","first_clip":0,"second_clip":1,"transition_seconds":1.0,"name":"joined"})
}
fn build(model: &AnimatedModel, r: &Value) -> Result<AnimatedModel, String> {
    let bytes = model.to_bytes().unwrap();
    authoring::sequence_clips(&serde_json::to_vec(r).unwrap(), ID, |path| {
        assert_eq!(path, "source.animated.json");
        Ok(bytes.clone())
    })
}
fn mutated(f: impl FnOnce(&mut AnimatedSource)) -> AnimatedModel {
    let mut source = oracle::fixture().source().clone();
    f(&mut source);
    AnimatedModel::new(source).unwrap()
}
fn close(a: f32, b: f64) {
    assert!((f64::from(a) - b).abs() < 0.00005, "{a} != {b}");
}
fn check_pose(model: &AnimatedModel, t: f64) {
    let pose = model.sample_clip(2, t as f32).unwrap();
    for (a, b) in pose.local().iter().zip(oracle::sequence_local(t)) {
        for i in 0..3 {
            close(a.translation[i], b.translation[i]);
            close(a.scale[i], b.scale[i]);
        }
        let dot: f64 = (0..4)
            .map(|i| f64::from(a.rotation[i]) * b.rotation[i])
            .sum();
        assert!((dot.abs() - 1.).abs() < 0.00001);
    }
    for (a, b) in pose.global().iter().zip(oracle::sequence_globals(t)) {
        for c in 0..4 {
            for r in 0..4 {
                close(a[c][r], b[c][r]);
            }
        }
    }
    for (a, b) in pose.skin_matrices()[0]
        .iter()
        .zip(oracle::sequence_palette(t))
    {
        for c in 0..4 {
            for r in 0..4 {
                close(a[c][r], b[c][r]);
            }
        }
    }
    let mesh = model.deform(&pose).unwrap();
    assert_eq!(mesh.len(), 1);
    for (a, b) in mesh[0].vertices.iter().zip(oracle::sequence_positions(t)) {
        for (a, b) in a.position.into_iter().zip(b) {
            close(a, b);
        }
    }
}
#[test]
fn independent_f64_clamps_quaternions_globals_palettes_and_weighted_vertices() {
    let output = build(&oracle::fixture(), &recipe()).unwrap();
    // A/B interiors, exact endpoints, transition 1/4, 1/2, 3/4, and the
    // staggered channels' late/early/single-key clamp boundaries in one batch.
    for t in [
        -1., 0., 0.125, 0.25, 0.625, 0.75, 1., 1.375, 1.75, 2., 2.25, 2.5, 2.75, 3., 3.125, 3.25,
        3.5, 3.75, 4., 4.375, 4.75, 5., 6.,
    ] {
        check_pose(&output, t);
    }
    let clip = &output.source().clips[2];
    assert_eq!(
        clip.channels.iter().map(|c| c.node).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(clip.duration(), oracle::DURATION as f32);
    for c in &clip.channels {
        assert_eq!(c.interpolation, Interpolation::Linear);
        assert_eq!(c.times.first(), Some(&0.));
        assert_eq!(c.times.last(), Some(&5.));
        assert!(c.times.windows(2).all(|p| p[0] < p[1]));
    }
    // Independent negative controls: incorrect clip order and transition timing
    // must move visible geometry, not merely metadata or an invisible marker.
    for t in [2.25, 2.75] {
        let expected = oracle::sequence_positions(t);
        for wrong in [
            oracle::ordered_positions(1, 0, t, 1.),
            oracle::ordered_positions(0, 1, t, 0.5),
        ] {
            let separation = expected
                .iter()
                .zip(wrong)
                .map(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs())
                .fold(0., f64::max);
            assert!(separation > 0.2);
        }
    }
}
#[test]
fn preserves_original_content_and_deterministic_direct_input_provenance() {
    let input = mutated(|s| {
        s.primitives[0].vertices[0].vertex.uv[0] = -0.0;
        s.nodes[3].rest.translation[1] = -0.0;
    });
    let raw = input.to_bytes().unwrap();
    let r = serde_json::to_vec(&recipe()).unwrap();
    let output = build(&input, &recipe()).unwrap();
    assert_eq!(
        output.to_bytes().unwrap(),
        build(&input, &recipe()).unwrap().to_bytes().unwrap()
    );
    assert_eq!(raw, input.to_bytes().unwrap());
    let a = input.source();
    let b = output.source();
    let mut retained = b.clone();
    retained.asset_id = a.asset_id.clone();
    retained.dependencies = a.dependencies.clone();
    retained.clips.truncate(a.clips.len());
    for (old, new) in a.primitives.iter().zip(&mut retained.primitives) {
        new.id = old.id.clone();
    }
    assert_eq!(
        serde_json::to_vec(&retained).unwrap(),
        serde_json::to_vec(a).unwrap(),
        "exact original serialization, including signed zero"
    );
    assert_eq!(&b.clips[..2], a.clips);
    assert_eq!(b.nodes, a.nodes);
    assert_eq!(b.skins, a.skins);
    assert_eq!(b.images, a.images);
    assert_eq!(b.materials, a.materials);
    assert_eq!(b.asset_id, ID);
    for (a, b) in a.primitives.iter().zip(&b.primitives) {
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.node, b.node);
        assert_eq!(a.skin, b.skin);
        assert_eq!(a.material, b.material);
        assert_eq!(
            a.id.split_once('#').unwrap().1,
            b.id.split_once('#').unwrap().1
        );
    }
    let digest = |v: &[u8]| {
        Sha256::digest(v)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    assert_eq!(b.dependencies.len(), 2);
    assert_eq!(b.dependencies[0].uri, "$source");
    assert_eq!(b.dependencies[0].sha256, digest(&r));
    assert_eq!(b.dependencies[1].uri, "source.animated.json");
    assert_eq!(b.dependencies[1].sha256, digest(&raw));
    let mut next_recipe = recipe();
    next_recipe["name"] = json!("second-join");
    assert!(
        build(&output, &next_recipe).is_err(),
        "new asset identity must differ from cooked input"
    );
    let bytes = output.to_bytes().unwrap();
    assert_eq!(
        AnimatedModel::from_bytes(&bytes)
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
}
#[test]
fn rejects_strict_recipe_invalid_transitions_targets_and_names_as_group() {
    let input = oracle::fixture();
    for (field, value) in [
        ("transition_seconds", json!(0)),
        ("transition_seconds", json!(-1)),
        ("transition_seconds", json!(60.0001)),
        ("transition_seconds", json!(1e-100)),
        ("transition_seconds", json!(1e300)),
        ("first_clip", json!(2)),
        ("second_clip", json!(-1)),
        ("name", json!("")),
        ("name", json!("literal-0")),
        ("name", json!("a\nb")),
        ("name", json!("é".repeat(65))),
        ("source", json!("../escape.animated.json")),
        ("unknown", json!(1)),
        ("version", json!(2)),
    ] {
        let mut r = recipe();
        r[field] = value;
        assert!(build(&input, &r).is_err(), "accepted {r}");
    }
    let mut boundary = recipe();
    boundary["name"] = json!("é".repeat(64));
    assert!(build(&input, &boundary).is_ok());
    let raw = input.to_bytes().unwrap();
    let valid = serde_json::to_string(&recipe()).unwrap();
    for bad in [
        "[]".to_string(),
        "[\"orr_animation_clip_sequence\",1,\"source.animated.json\",0,1,1,\"joined\"]".into(),
        valid.replacen('{', "{\"name\":\"duplicate\",", 1),
        " ".repeat(16385),
        valid.replace("1.0", "1e999"),
    ] {
        assert!(
            authoring::sequence_clips(bad.as_bytes(), ID, |_| Ok(raw.clone())).is_err(),
            "accepted {bad}"
        );
    }
    for mode in 0..3 {
        let m = mutated(|s| match mode {
            0 => s.clips[1].channels[0].interpolation = Interpolation::Step,
            1 => {
                s.clips[1].channels.pop();
            }
            _ => s.clips[1].channels[0].node = 3,
        });
        assert!(build(&m, &recipe()).is_err());
    }
    // Serialized cooked input rejects unsupported interpolation and duplicate
    // targets before sequence construction; no alternate glTF import path.
    for mode in 0..2 {
        let mut value = serde_json::to_value(input.source()).unwrap();
        if mode == 0 {
            value["clips"][0]["channels"][0]["interpolation"] = json!("cubic_spline");
        } else {
            let duplicate = value["clips"][0]["channels"][0].clone();
            value["clips"][0]["channels"]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(authoring::sequence_clips(valid.as_bytes(), ID, |_| Ok(bytes.clone())).is_err());
    }
}
#[test]
fn rejects_shift_collapse_duration_overflow_and_combined_budgets_but_allows_single_key() {
    for mode in 0..3 {
        let m = mutated(|s| {
            if mode == 0 {
                s.clips[0].channels[0].times = vec![0., 3600.];
            } else if mode == 1 {
                s.clips[1].channels[1].times = vec![0., f32::MIN_POSITIVE, 1.];
            } else {
                let c = s.clips[0].clone();
                while s.clips.len() < MAX_CLIPS {
                    let mut c = c.clone();
                    c.name = format!("extra-{}", s.clips.len());
                    s.clips.push(c);
                }
            }
        });
        assert!(build(&m, &recipe()).is_err(), "accepted mode {mode}");
    }
    let mut r = recipe();
    r["transition_seconds"] = json!(1e-9);
    assert!(
        build(&oracle::fixture(), &r).is_err(),
        "transition collapsed at A end"
    );
    let many = mutated(|s| {
        let count = MAX_ANIMATION_KEYS / 2;
        let c = &mut s.clips[0].channels[0];
        c.times = (0..count).map(|i| i as f32 / count as f32 * 2.).collect();
        c.values = ChannelValues::Translation(vec![[0.3, 0.2, 0.]; count]);
    });
    assert!(
        build(&many, &recipe()).is_err(),
        "combined source plus appended key budget"
    );
    let single = mutated(|s| {
        for clip in &mut s.clips {
            for c in &mut clip.channels {
                c.times = vec![0.];
                match &mut c.values {
                    ChannelValues::Translation(v) | ChannelValues::Scale(v) => v.truncate(1),
                    ChannelValues::Rotation(v) => v.truncate(1),
                }
            }
        }
    });
    let out = build(&single, &recipe()).unwrap();
    assert_eq!(out.source().clips[2].duration(), 1.);
    let mut same = recipe();
    same["second_clip"] = json!(0);
    assert!(build(&oracle::fixture(), &same).is_ok());
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
struct Scratch(PathBuf);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl Scratch {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "orr-sequence-cpu-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(fs::canonicalize(p).unwrap())
    }
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn cli() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("ORR_ANIMATION_SEQUENCE_CLI")
            .expect("set verified fresh absolute CLI path from single build owner"),
    );
    assert!(path.is_absolute());
    path
}
#[test]
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[ignore = "requires explicit freshly built ORR_ANIMATION_SEQUENCE_CLI"]
fn actual_cli_writes_deterministically_without_overwrite_or_symlink_following() {
    let dir = Scratch::new();
    let source = dir.0.join("source.animated.json");
    let recipe_path = dir.0.join("join.sequence.json");
    let input = oracle::fixture().to_bytes().unwrap();
    fs::write(&source, &input).unwrap();
    fs::write(&recipe_path, serde_json::to_vec(&recipe()).unwrap()).unwrap();
    let run = |output: &std::path::Path| {
        Command::new(cli()).args(["--recipe",recipe_path.to_str().unwrap(),"--asset-id",ID,"--output",output.to_str().unwrap()]).output().expect("build real animation_clip_sequence example in the shared build batch before this test")
    };
    let output = dir.0.join("new.animated.json");
    let first = run(&output);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let bytes = fs::read(&output).unwrap();
    let model = AnimatedModel::from_bytes(&bytes).unwrap();
    check_pose(&model, 2.5);
    let again = dir.0.join("again.animated.json");
    assert!(run(&again).status.success());
    assert_eq!(bytes, fs::read(&again).unwrap());
    assert!(!run(&output).status.success());
    assert_eq!(bytes, fs::read(&output).unwrap());
    assert_eq!(input, fs::read(&source).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let linked = dir.0.join("linked.animated.json");
        symlink(&output, &linked).unwrap();
        assert!(!run(&linked).status.success());
        assert_eq!(bytes, fs::read(&output).unwrap());
        let linked_parent = dir.0.join("linked-parent");
        symlink(&dir.0, &linked_parent).unwrap();
        let parent_output = linked_parent.join("parent.animated.json");
        assert!(!run(&parent_output).status.success());
        assert!(!dir.0.join("parent.animated.json").exists());
        fs::remove_file(&source).unwrap();
        symlink(&output, &source).unwrap();
        let absent = dir.0.join("absent.animated.json");
        assert!(!run(&absent).status.success());
        assert!(!absent.exists());
    }
}
