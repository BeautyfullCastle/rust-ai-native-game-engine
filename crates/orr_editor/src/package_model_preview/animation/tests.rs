use super::super::{load, verify_loaded, Completion, Job, PreviewRequest, State};
use super::*;
use crate::model_bindings::ModelKind;
use orr_model::animation::*;
use orr_model::{Dependency, Image, Material, Vertex, Wrap, IDENTITY};
use orr_package::Project;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const PACKAGE: &str = "preview-animation";
const ASSET: &str = "character.glb";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    request: PreviewRequest,
}

impl Fixture {
    fn new() -> Self {
        Self::with_model(model(), &["animation"])
    }

    fn with_model(model: AnimatedModel, capabilities: &[&str]) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let root = base.join("project");
        let source = base.join("source");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join(ASSET), model.to_bytes().unwrap()).unwrap();
        fs::write(source.join("LICENSE.txt"), b"Original animation fixture\n").unwrap();
        fs::write(
            source.join("orr.package.json"),
            serde_json::to_vec(&orr_package::Manifest {
                schema: 1,
                name: PACKAGE.into(),
                version: "1.0.0".into(),
                engine: "*".into(),
                capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
                dependencies: BTreeMap::new(),
                files: BTreeSet::from([ASSET.into(), "LICENSE.txt".into()]),
            })
            .unwrap(),
        )
        .unwrap();
        let project = Project::open(&root, orr_sample::room_project::compiled_runtime()).unwrap();
        let lock = project.install(std::slice::from_ref(&source)).unwrap();
        let request = PreviewRequest {
            project_root: root.clone(),
            package_id: PACKAGE.into(),
            asset_path: ASSET.into(),
            digest: lock.packages[PACKAGE].digest.clone(),
            generation: 9,
            kind: ModelKind::Animated,
        };
        Self {
            _temporary: temporary,
            root,
            source,
            request,
        }
    }

    fn project(&self) -> Project {
        Project::open(&self.root, orr_sample::room_project::compiled_runtime()).unwrap()
    }

    fn installed(&self, asset: &str) -> PathBuf {
        self.root
            .join(".orr/packages/objects")
            .join(&self.request.digest)
            .join(asset)
    }

    fn state(&self) -> State {
        let epoch = Arc::new(());
        let mut state = State {
            active: Some(Job {
                request: self.request.clone(),
                epoch: Arc::clone(&epoch),
            }),
            ..State::default()
        };
        state.accept(Completion {
            job: Job {
                request: self.request.clone(),
                epoch,
            },
            result: load(&self.request),
        });
        assert!(state.ready().is_some(), "{}", state.status);
        state
    }
}

fn model() -> AnimatedModel {
    let clip = |name: &str, times, values| AnimationClip {
        name: name.into(),
        channels: vec![AnimationChannel {
            node: 0,
            interpolation: Interpolation::Linear,
            times,
            values,
        }],
    };
    AnimatedModel::new(AnimatedSource {
        format: "orr_animated_model".into(),
        version: 1,
        asset_id: ASSET.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
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
        nodes: vec![
            AnimatedNode {
                name: "joint".into(),
                children: vec![],
                rest: Trs::default(),
            },
            AnimatedNode {
                name: "mesh".into(),
                children: vec![],
                rest: Trs {
                    translation: [99.0, 0.0, 0.0],
                    ..Trs::default()
                },
            },
        ],
        skins: vec![Skin {
            name: "skin".into(),
            joints: vec![0],
            inverse_bind_matrices: vec![IDENTITY],
        }],
        primitives: vec![AnimatedPrimitive {
            id: format!("{ASSET}#node=1/mesh=0/primitive=0"),
            node: 1,
            skin: Some(0),
            vertices: [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]]
                .into_iter()
                .map(|position| SkinnedVertex {
                    vertex: Vertex {
                        position,
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.0; 2],
                    },
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                })
                .collect(),
            indices: vec![0, 1, 2],
            material: 0,
        }],
        clips: vec![
            clip(
                "duplicate",
                vec![0.0, 2.0],
                ChannelValues::Translation(vec![[0.0; 3], [2.0, 0.0, 0.0]]),
            ),
            clip(
                "duplicate",
                vec![0.0, 2.0],
                ChannelValues::Scale(vec![[1.0; 3], [2.0, 0.5, 1.0]]),
            ),
            clip(
                "",
                vec![0.0],
                ChannelValues::Translation(vec![[4.0, 0.0, 0.0]]),
            ),
            clip(
                "",
                vec![0.0, 1.0],
                ChannelValues::Translation(vec![[0.0; 3], [-3.0, 0.0, 0.0]]),
            ),
        ],
    })
    .unwrap()
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out)
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
fn animated_load_starts_at_clip_zero_and_sampling_preserves_project_bytes() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let mut state = fixture.state();
    let ready = state.ready().unwrap();
    let initial = ready.animation.as_ref().unwrap();
    assert_eq!((initial.clip_index, initial.seconds), (0, 0.0));
    assert_eq!(ready.loaded.kind(), ModelKind::Animated);
    assert_eq!(ready.bounds.center, [0.0, 1.0, 0.0]);
    let source = ready.loaded.animated_model().unwrap().source().clone();
    state.sample(1, 2.0).unwrap();
    assert_eq!(
        (
            state
                .ready()
                .unwrap()
                .animation
                .as_ref()
                .unwrap()
                .clip_index,
            state.ready().unwrap().animation.as_ref().unwrap().seconds
        ),
        (1, 2.0)
    );
    assert_eq!(
        *state
            .ready()
            .unwrap()
            .loaded
            .animated_model()
            .unwrap()
            .source(),
        source
    );
    assert_eq!(snapshot(&fixture.root), before);
}

#[test]
fn animated_request_requires_animation_capability_and_rejects_static_route() {
    let fixture = Fixture::with_model(model(), &["models"]);
    assert!(load(&fixture.request)
        .err()
        .unwrap()
        .contains("animation package"));
    let fixture = Fixture::new();
    let mut request = fixture.request.clone();
    request.kind = ModelKind::Static;
    assert!(load(&request).is_err());
}

#[test]
fn explicit_clip_indices_disambiguate_duplicate_and_unnamed_clips() {
    let model = model();
    assert_eq!(model.source().clips[0].name, model.source().clips[1].name);
    assert_eq!(model.source().clips[2].name, model.source().clips[3].name);
    let first = sample(&model, 0, 1.0).unwrap();
    let second = sample(&model, 1, 1.0).unwrap();
    assert_ne!(first.pose, second.pose);
    assert_ne!(first.bounds, second.bounds);
    let unnamed_first = sample(&model, 2, 0.0).unwrap();
    let unnamed_second = sample(&model, 3, 0.0).unwrap();
    assert_ne!(unnamed_first.pose, unnamed_second.pose);
    assert_eq!(unnamed_first.clip_index, 2);
    assert_eq!(unnamed_second.clip_index, 3);
}

#[test]
fn invalid_clip_or_time_keeps_last_accepted_pose_bounds_and_status() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.sample(1, 1.0).unwrap();
    let accepted = state.ready().unwrap().animation.clone();
    let bounds = state.ready().unwrap().bounds;
    let status = state.status.clone();
    for (clip, time) in [
        (u32::MAX, 0.0),
        (0, -0.001),
        (0, 2.001),
        (0, f32::NAN),
        (0, f32::INFINITY),
        (0, f32::NEG_INFINITY),
    ] {
        assert!(state.sample(clip, time).is_err());
        assert_eq!(state.ready().unwrap().animation, accepted);
        assert_eq!(state.ready().unwrap().bounds, bounds);
        assert_eq!(state.status, status);
    }
}

#[test]
fn zero_duration_clip_accepts_only_zero_and_other_endpoints_are_inclusive() {
    let model = model();
    assert_eq!(model.source().clips[2].duration(), 0.0);
    assert!(sample(&model, 2, 0.0).is_ok());
    assert!(sample(&model, 2, -0.0).is_ok());
    assert!(sample(&model, 2, f32::EPSILON).is_err());
    assert!(sample(&model, 2, -f32::EPSILON).is_err());
    assert!(sample(&model, 0, 0.0).is_ok());
    assert!(sample(&model, 0, 2.0).is_ok());
}

#[test]
fn animated_asset_without_clips_fails_explicitly_instead_of_inventing_a_sample() {
    let mut source = model().source().clone();
    source.clips.clear();
    let fixture = Fixture::with_model(AnimatedModel::new(source).unwrap(), &["animation"]);
    assert!(load(&fixture.request).err().unwrap().contains("no clips"));
}

#[test]
fn sampled_bounds_fit_deformed_skin_without_double_mesh_transform() {
    let model = model();
    let accepted = sample(&model, 1, 2.0).unwrap();
    assert_eq!(accepted.bounds.center, [0.0, 0.5, 0.0]);
    assert!((accepted.bounds.radius - 4.25_f32.sqrt()).abs() < 1.0e-6);
    let mut controls = super::super::PreviewCamera::default();
    controls.orbit([25.0, 12.0]);
    controls.zoom(1.0);
    controls.reset();
    let camera = controls.camera(accepted.bounds, (448, 336));
    for primitive in model.deform(&accepted.pose).unwrap() {
        for vertex in primitive.vertices {
            let screen = camera.world_to_screen(vertex.position, (448, 336)).unwrap();
            assert!(screen[0] >= 0.0 && screen[0] <= 448.0);
            assert!(screen[1] >= 0.0 && screen[1] <= 336.0);
        }
    }
    assert_eq!(accepted, sample(&model, 1, 2.0).unwrap());
}

#[test]
fn invalid_sampled_bounds_are_rejected_and_degenerate_bounds_remain_finite() {
    for bounds in [
        Bounds {
            min: [1.0; 3],
            max: [-1.0; 3],
        },
        Bounds {
            min: [f32::NAN; 3],
            max: [0.0; 3],
        },
        Bounds {
            min: [0.0; 3],
            max: [f32::INFINITY; 3],
        },
        Bounds {
            min: [0.0; 3],
            max: [1.1e9; 3],
        },
    ] {
        assert!(PreviewBounds::from_animation(bounds).is_err());
    }
    let bounds = PreviewBounds::from_animation(Bounds {
        min: [7.0; 3],
        max: [7.0; 3],
    })
    .unwrap();
    assert_eq!(bounds.center, [7.0; 3]);
    assert_eq!(bounds.radius, 0.125);
}

#[test]
fn changed_kind_or_epoch_cannot_accept_an_old_animated_completion() {
    let fixture = Fixture::new();
    for change_kind in [true, false] {
        let mut state = fixture.state();
        let active = state.active.as_ref().unwrap();
        let job = Job {
            request: active.request.clone(),
            epoch: Arc::clone(&active.epoch),
        };
        state.ready = None;
        if change_kind {
            state.active.as_mut().unwrap().request.kind = ModelKind::Static;
        } else {
            state.active.as_mut().unwrap().epoch = Arc::new(());
        }
        state.accept(Completion {
            job,
            result: load(&fixture.request),
        });
        assert!(state.ready().is_none());
    }
}

#[test]
fn animated_final_verification_rejects_corrupt_removed_and_replaced_packages() {
    let fixture = Fixture::new();
    let ready = load(&fixture.request).unwrap();
    let original = fs::read(fixture.installed(ASSET)).unwrap();
    fs::write(fixture.installed(ASSET), b"corrupt animation").unwrap();
    assert!(verify_loaded(&fixture.request, &ready.loaded).is_err());
    fs::write(fixture.installed(ASSET), original).unwrap();
    fixture.project().remove(PACKAGE).unwrap();
    assert!(verify_loaded(&fixture.request, &ready.loaded).is_err());
    fs::write(fixture.source.join("LICENSE.txt"), b"replacement revision").unwrap();
    fixture
        .project()
        .install(std::slice::from_ref(&fixture.source))
        .unwrap();
    assert!(verify_loaded(&fixture.request, &ready.loaded).is_err());
}

#[test]
fn changed_files_refuse_sampling_without_replacing_last_accepted_pose() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let accepted = state.ready().unwrap().animation.clone();
    let bounds = state.ready().unwrap().bounds;
    fixture.project().remove(PACKAGE).unwrap();
    assert!(state.sample(1, 1.0).is_err());
    assert_eq!(state.ready().unwrap().animation, accepted);
    assert_eq!(state.ready().unwrap().bounds, bounds);
}

#[test]
fn close_and_project_switch_release_animated_state_and_refuse_late_completion() {
    let fixture = Fixture::new();
    for close in [true, false] {
        let mut state = fixture.state();
        let active = state.active.as_ref().unwrap();
        let job = Job {
            request: active.request.clone(),
            epoch: Arc::clone(&active.epoch),
        };
        if close {
            state.close()
        } else {
            state.poll(None)
        }
        assert!(state.ready().is_none());
        assert!(state.sample(0, 0.0).is_err());
        state.accept(Completion {
            job,
            result: load(&fixture.request),
        });
        assert!(state.ready().is_none());
    }
}

#[test]
fn nonzero_first_key_clamps_without_shifting_the_seconds_timeline() {
    let mut source = model().source().clone();
    source.clips[0].channels[0].times = vec![2.0, 4.0];
    let model = AnimatedModel::new(source).unwrap();
    let zero = sample(&model, 0, 0.0).unwrap();
    let before = sample(&model, 0, 1.0).unwrap();
    let first = sample(&model, 0, 2.0).unwrap();
    let middle = sample(&model, 0, 3.0).unwrap();
    let end = sample(&model, 0, 4.0).unwrap();
    assert_eq!(zero.pose, before.pose);
    assert_eq!(before.pose, first.pose);
    assert_eq!(middle.pose.local()[0].translation, [1.0, 0.0, 0.0]);
    assert_eq!(end.pose.local()[0].translation, [2.0, 0.0, 0.0]);
    assert!(sample(&model, 0, 4.01).is_err());
}
