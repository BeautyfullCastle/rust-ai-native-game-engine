use super::*;
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap};
use std::collections::{BTreeMap, BTreeSet};

const PACKAGE: &str = "preview-model";
const ASSET: &str = "mesh.orrmodel";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    request: PreviewRequest,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let root = base.join("project");
        let source = base.join("source");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join(ASSET), triangle().to_bytes().unwrap()).unwrap();
        fs::write(source.join("LICENSE.txt"), b"Original test fixture.\n").unwrap();
        let manifest = orr_package::Manifest {
            schema: 1,
            name: PACKAGE.into(),
            version: "1.0.0".into(),
            engine: "*".into(),
            capabilities: BTreeSet::from(["models".into()]),
            dependencies: BTreeMap::new(),
            files: BTreeSet::from([ASSET.into(), "LICENSE.txt".into()]),
        };
        fs::write(
            source.join("orr.package.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let project = Project::open(&root, orr_sample::room_project::compiled_runtime()).unwrap();
        let lock = project.install(std::slice::from_ref(&source)).unwrap();
        let request = PreviewRequest {
            project_root: root.clone(),
            package_id: PACKAGE.into(),
            asset_path: ASSET.into(),
            digest: lock.packages[PACKAGE].digest.clone(),
            generation: 7,
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
}

fn triangle() -> StaticModel {
    let mut transform = orr_model::IDENTITY;
    transform[3] = [2.0, -3.0, 4.0, 1.0];
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: ASSET.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
        }],
        primitives: vec![Primitive {
            id: format!("{ASSET}#node=0/mesh=0/primitive=0"),
            vertices: [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                .into_iter()
                .map(|position| Vertex {
                    position,
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                })
                .collect(),
            indices: vec![0, 1, 2],
            material: 0,
            transform,
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
    })
    .unwrap()
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn active_state(request: &PreviewRequest) -> State {
    State {
        active: Some(Job {
            request: request.clone(),
            epoch: Arc::new(()),
        }),
        ..State::default()
    }
}

fn job(state: &State) -> Job {
    let active = state.active.as_ref().unwrap();
    Job {
        request: active.request.clone(),
        epoch: Arc::clone(&active.epoch),
    }
}

#[test]
fn admitted_static_preview_is_immutable_and_preserves_all_project_bytes() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let ready = load(&fixture.request).unwrap();
    assert_eq!(ready.loaded.package(), PACKAGE);
    assert_eq!(ready.loaded.asset(), ASSET);
    assert_eq!(ready.loaded.package_digest(), fixture.request.digest);
    assert_eq!(ready.loaded.kind(), ModelKind::Static);
    assert_eq!(ready.bounds.center, [3.0, -2.5, 4.0]);
    assert!((ready.bounds.radius - 1.25_f32.sqrt()).abs() < 1.0e-6);
    assert_eq!(snapshot(&fixture.root), before);
    assert_eq!(
        ready.loaded.static_model().unwrap().source(),
        triangle().source()
    );
}

#[test]
fn nondeclared_traversal_wrong_digest_and_nonmodel_requests_are_rejected() {
    let fixture = Fixture::new();
    for asset in [
        "../mesh.orrmodel",
        "missing.glb",
        "LICENSE.txt",
        "animation.oram",
    ] {
        let mut request = fixture.request.clone();
        request.asset_path = asset.into();
        assert!(load(&request).is_err(), "{asset}");
    }
    let mut request = fixture.request.clone();
    request.digest = "f".repeat(64);
    assert!(load(&request).unwrap_err_message().contains("digest"));
    request.digest = "invalid".into();
    assert!(load(&request).is_err());
    request = fixture.request.clone();
    request.project_root = PathBuf::from(".");
    assert!(load(&request).is_err());
}

#[test]
fn corrupt_installed_bytes_are_rejected_before_ready() {
    let fixture = Fixture::new();
    fs::write(fixture.installed(ASSET), b"corrupt model bytes").unwrap();
    assert!(load(&fixture.request).is_err());
}

#[test]
fn final_verification_rejects_corruption_after_decode_including_other_declared_files() {
    for asset in [ASSET, "LICENSE.txt"] {
        let fixture = Fixture::new();
        let ready = load(&fixture.request).unwrap();
        fs::write(fixture.installed(asset), b"changed after decode").unwrap();
        assert!(
            verify_loaded(&fixture.request, &ready.loaded).is_err(),
            "{asset}"
        );
    }
}

#[test]
fn final_verification_rejects_removed_package_and_changed_digest() {
    let fixture = Fixture::new();
    let ready = load(&fixture.request).unwrap();
    fixture.project().remove(PACKAGE).unwrap();
    assert!(verify_loaded(&fixture.request, &ready.loaded)
        .unwrap_err_message()
        .contains("removed"));
    fs::write(
        fixture.source.join("LICENSE.txt"),
        b"New package revision.\n",
    )
    .unwrap();
    let lock = fixture
        .project()
        .install(std::slice::from_ref(&fixture.source))
        .unwrap();
    assert_ne!(lock.packages[PACKAGE].digest, fixture.request.digest);
    assert!(verify_loaded(&fixture.request, &ready.loaded)
        .unwrap_err_message()
        .contains("digest"));
}

#[test]
fn current_completion_is_accepted_and_old_generation_is_discarded() {
    let fixture = Fixture::new();
    let mut state = active_state(&fixture.request);
    let mut old = job(&state);
    old.request.generation -= 1;
    state.accept(Completion {
        job: old,
        result: load(&fixture.request),
    });
    assert!(state.ready().is_none());
    state.accept(Completion {
        job: job(&state),
        result: load(&fixture.request),
    });
    assert!(state.ready().is_some());
    assert!(state.status.starts_with("Verified static model preview."));
}

#[test]
fn old_same_asset_completion_cannot_cross_a_repeated_request_epoch() {
    let fixture = Fixture::new();
    let mut state = active_state(&fixture.request);
    let old = job(&state);
    state.active.as_mut().unwrap().epoch = Arc::new(());
    state.accept(Completion {
        job: old,
        result: load(&fixture.request),
    });
    assert!(state.ready().is_none());
    assert!(state.active_request().is_some());
}

#[test]
fn close_releases_ready_and_discards_late_completion() {
    let fixture = Fixture::new();
    let mut state = active_state(&fixture.request);
    let old = job(&state);
    state.accept(Completion {
        job: job(&state),
        result: load(&fixture.request),
    });
    assert!(state.ready().is_some());
    state.close();
    assert!(state.ready().is_none());
    assert!(state.active_request().is_none());
    assert!(!state.busy());
    state.accept(Completion {
        job: old,
        result: load(&fixture.request),
    });
    assert!(state.ready().is_none());
    assert!(state.status.is_empty());
}

#[test]
fn project_switch_discards_a_queued_completion_before_accepting_it() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let mut state = active_state(&fixture.request);
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(Completion {
            job: job(&state),
            result: load(&fixture.request),
        })
        .unwrap();
    state.pending = Some(receiver);
    state.poll(Some(&other.root));
    assert!(state.ready().is_none());
    assert!(state.active_request().is_none());
    assert!(state.pending.is_none());
}

#[test]
fn switching_away_then_back_does_not_revive_old_result() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let mut state = active_state(&fixture.request);
    let old = job(&state);
    state.poll(Some(&other.root));
    state.active = Some(Job {
        request: fixture.request.clone(),
        epoch: Arc::new(()),
    });
    state.poll(Some(&fixture.root));
    state.accept(Completion {
        job: old,
        result: load(&fixture.request),
    });
    assert!(state.ready().is_none());
}

#[test]
fn delivery_pin_rejects_removed_lock_and_changed_source_after_worker_verification() {
    for remove in [true, false] {
        let fixture = Fixture::new();
        let mut state = active_state(&fixture.request);
        let ready = load(&fixture.request).unwrap();
        if remove {
            fixture.project().remove(PACKAGE).unwrap();
        } else {
            fs::write(fixture.installed(ASSET), b"corrupt after worker completion").unwrap();
        }
        state.accept(Completion {
            job: job(&state),
            result: Ok(ready),
        });
        assert!(state.ready().is_none());
        assert!(state.active_request().is_some());
        assert!(state.status.starts_with("Static preview unavailable:"));
    }
}

#[test]
fn errors_remain_visible_without_an_admitted_preview() {
    let fixture = Fixture::new();
    let mut state = active_state(&fixture.request);
    state.accept(Completion {
        job: job(&state),
        result: Err("corrupt fixture".into()),
    });
    assert_eq!(state.status, "Static preview unavailable: corrupt fixture");
    assert!(state.active_request().is_some());
    assert!(state.ready().is_none());
}

#[test]
fn repeated_requests_keep_one_running_worker_and_only_latest_waiting_request() {
    let fixture = Fixture::new();
    let mut state = State::default();
    let (sender, receiver) = mpsc::sync_channel::<Completion>(1);
    state.pending = Some(receiver);
    for generation in 0..128 {
        let mut request = fixture.request.clone();
        request.generation = generation;
        state.request(request);
        assert!(state.pending.is_some());
        assert_eq!(
            state.queued.as_ref().unwrap().request.generation,
            generation
        );
    }
    assert!(state.busy());
    state.close();
    assert!(state.pending.is_some());
    assert!(state.queued.is_none());
    assert!(!state.busy());
    drop(sender);
    state.poll(Some(&fixture.root));
    assert!(state.pending.is_none());
    assert!(state.status.is_empty());
}

#[test]
fn asynchronous_worker_completes_without_project_mutation() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let mut state = State::default();
    state.request(fixture.request.clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.busy() && std::time::Instant::now() < deadline {
        state.poll(Some(&fixture.root));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        !state.busy(),
        "preview worker did not complete within test deadline"
    );
    assert!(state.ready().is_some(), "{}", state.status);
    assert_eq!(snapshot(&fixture.root), before);
}

#[cfg(unix)]
#[test]
fn symlinked_project_and_installed_asset_are_rejected() {
    let fixture = Fixture::new();
    let link = fixture.root.parent().unwrap().join("project-link");
    std::os::unix::fs::symlink(&fixture.root, &link).unwrap();
    let mut request = fixture.request.clone();
    request.project_root = link;
    assert!(load(&request).is_err());
    let installed = fixture.installed(ASSET);
    fs::remove_file(&installed).unwrap();
    std::os::unix::fs::symlink(fixture.source.join(ASSET), installed).unwrap();
    assert!(load(&fixture.request).is_err());
}

// Avoid requiring GPU-owned/admitted payloads to implement Debug just for errors.
trait ErrorMessage {
    fn unwrap_err_message(self) -> String;
}
impl<T> ErrorMessage for Result<T, String> {
    fn unwrap_err_message(self) -> String {
        match self {
            Ok(_) => panic!("expected rejection"),
            Err(error) => error,
        }
    }
}
