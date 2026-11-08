//! Headless egui tests for explicit sprite package installation controls.
#![allow(clippy::disallowed_types)]
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

type WidgetState = (State, Option<Context>, bool);

fn context(root: &Path) -> Context {
    Context {
        bindings_path: root.join("arena.sprites.json"),
        bindings_document: Vec::new(),
        project: root.to_path_buf(),
        project_input: "project".into(),
        scene: root.join("arena.yaml").display().to_string(),
        load_epoch: 4,
        host_epoch: 7,
    }
}

fn harness(
    state: State,
    context: Option<Context>,
    authoring_busy: bool,
) -> Harness<'static, WidgetState> {
    let mut h = Harness::builder()
        .with_size([1100.0, 900.0])
        .build_ui_state(
            |ui, state: &mut WidgetState| {
                let (install, context, authoring_busy) = state;
                install.poll(context.clone());
                install.show(ui, context.clone(), *authoring_busy);
            },
            (state, context, authoring_busy),
        );
    h.run_steps(2);
    h
}

fn click(h: &mut Harness<'_, WidgetState>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click_accesskit();
    h.run_steps(2);
}

fn wait_preview(h: &mut Harness<'_, WidgetState>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().0.busy() {
        h.run_steps(3);
        assert!(Instant::now() < deadline, "{}", h.state().0.status);
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn visit(root: &Path, current: &Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.push((path.strip_prefix(root).unwrap().into(), Vec::new()));
                visit(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

fn package_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let source = temp.path().join("sprite-package");
    fs::create_dir(&source).unwrap();
    let image = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap()
        .join("lantern_keeper.png");
    fs::copy(image, source.join("atlas.png")).unwrap();
    fs::write(source.join("LICENSE.txt"), b"Test sprite package license\n").unwrap();
    let document = orr_sprite::SpriteDocument::new(orr_sprite::SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: orr_sprite::Atlas {
            image: "atlas.png".into(),
            width: 64,
            height: 16,
        },
        regions: vec![orr_sprite::Region {
            id: 0,
            x: 0,
            y: 0,
            width: 64,
            height: 16,
        }],
        clips: Vec::new(),
    })
    .unwrap();
    fs::write(source.join("sprites.json"), document.to_json().unwrap()).unwrap();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "new-ui-package".into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: BTreeSet::from(["sprite".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from([
            "sprites.json".into(),
            "LICENSE.txt".into(),
            "atlas.png".into(),
        ]),
    };
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    (temp, root, source)
}

#[test]
fn inline_install_controls_are_visible_without_context_and_disabled() {
    let mut h = harness(State::default(), None, false);
    assert!(h.query_by_label("Local sprite package directory").is_none());
    click(&mut h, "Install local sprite package");
    assert!(h.query_by_label("Local sprite package directory").is_some());
    assert!(h
        .get_by_label("Preview sprite package install")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Install previewed sprite package")
        .accesskit_node()
        .is_disabled());
    let temp = tempfile::tempdir().unwrap();
    let mut authoring = harness(State::default(), Some(context(temp.path())), true);
    click(&mut authoring, "Install local sprite package");
    assert!(authoring
        .get_by_label("Preview sprite package install")
        .accesskit_node()
        .is_disabled());
}

#[test]
fn asynchronous_valid_preview_shows_summary_without_writing_source_or_project() {
    let (_temp, root, source) = package_fixture();
    let before_project = snapshot(&root);
    let before_source = snapshot(&source);
    let mut state = State::default();
    state.draft.source_directory = source.display().to_string();
    let mut h = harness(state, Some(context(&root)), false);
    click(&mut h, "Install local sprite package");
    click(&mut h, "Preview sprite package install");
    wait_preview(&mut h);
    let prepared = h
        .state()
        .0
        .prepared
        .as_ref()
        .expect("valid read-only preview");
    assert_eq!(prepared.plan.summary().name, "new-ui-package");
    assert_eq!(prepared.plan.summary().version, "1.0.0");
    assert_eq!(
        (
            prepared.plan.summary().width,
            prepared.plan.summary().height
        ),
        (64, 16)
    );
    assert_eq!(
        prepared.plan.summary().files,
        ["LICENSE.txt", "atlas.png", "sprites.json"]
    );
    assert!(h.state().0.status.contains("Review the files"));
    assert!(h
        .query_by_label("new-ui-package 1.0.0 - 64 x 16 - 1 regions - 0 clips")
        .is_some());
    assert!(h
        .query_by_label("Files: LICENSE.txt, atlas.png, sprites.json")
        .is_some());
    assert!(!h
        .get_by_label("Install previewed sprite package")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.draft.source_directory = root.join("changed-source").display().to_string();
    h.run_steps(2);
    assert!(h.state().0.prepared.is_none());
    assert!(h.state().0.status.contains("stale"));
    assert!(h
        .get_by_label("Install previewed sprite package")
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        snapshot(&root),
        before_project,
        "preview must not write the active project"
    );
    assert_eq!(
        snapshot(&source),
        before_source,
        "preview must not change its source package"
    );
}

#[test]
fn busy_disconnect_and_source_context_aba_are_fenced() {
    let (_temp, root, source) = package_fixture();
    let original_context = context(&root);
    let preview_plan = backend::prepare(&original_context.project, &source).unwrap();
    let mut state = State::default();
    state.draft.source_directory = source.display().to_string();
    state.sync(Some(original_context.clone()));
    let stale_pin = state.pin().unwrap();
    let (stale_sender, stale_receiver) = mpsc::channel();
    state.pending = Some(Pending::Preview {
        pin: stale_pin,
        receiver: stale_receiver,
    });
    let mut h = harness(state, Some(original_context.clone()), false);
    click(&mut h, "Install local sprite package");
    assert!(h.state().0.busy());
    assert!(h
        .get_by_label("Preview sprite package install")
        .accesskit_node()
        .is_disabled());
    let original_source = h.state().0.draft.source_directory.clone();
    {
        let (install, _, _) = h.state_mut();
        install.draft.source_directory = root.join("other-source").display().to_string();
        install.sync(Some(original_context.clone()));
        install.draft.source_directory = original_source;
        install.sync(Some(original_context.clone()));
        let changed = Context {
            bindings_document: vec![1],
            scene: root.join("another-scene.yaml").display().to_string(),
            load_epoch: original_context.load_epoch + 1,
            host_epoch: original_context.host_epoch + 1,
            ..original_context.clone()
        };
        install.sync(Some(changed));
        install.sync(Some(original_context.clone()));
    }
    stale_sender.send(Ok(preview_plan)).unwrap();
    h.state_mut().0.poll(Some(original_context.clone()));
    assert!(h.state().0.prepared.is_none());
    assert!(
        h.state().0.status.contains("prior source"),
        "{}",
        h.state().0.status
    );

    let (disconnect_sender, receiver) = mpsc::channel();
    let pin = h.state().0.pin().unwrap();
    h.state_mut().0.pending = Some(Pending::Preview { pin, receiver });
    h.run_steps(2);
    assert!(h.state().0.busy());
    assert!(h
        .get_by_label("Preview sprite package install")
        .accesskit_node()
        .is_disabled());
    drop(disconnect_sender);
    h.run_steps(2);
    h.state_mut().0.poll(Some(original_context));
    assert!(!h.state().0.busy());
    assert!(h.state().0.status.contains("worker disconnected"));
}

#[test]
fn exhausted_tickets_never_wrap_or_revive_install_controls() {
    let temp = tempfile::tempdir().unwrap();
    let original_context = context(temp.path());
    let mut h = harness(State::default(), Some(original_context.clone()), false);
    {
        let (install, current_context, _) = h.state_mut();
        install.context_ticket = u64::MAX - 1;
        install.source_ticket = u64::MAX - 1;
        install.draft_ticket = u64::MAX - 1;
        install.draft.source_directory = temp.path().join("first").display().to_string();
        *current_context = Some(Context {
            load_epoch: original_context.load_epoch + 1,
            ..original_context.clone()
        });
    }
    h.run_steps(2);
    assert_eq!(h.state().0.context_ticket, u64::MAX);
    assert_eq!(h.state().0.source_ticket, u64::MAX);
    assert_eq!(h.state().0.draft_ticket, u64::MAX);
    {
        let (install, current_context, _) = h.state_mut();
        install.draft.source_directory = temp.path().join("second").display().to_string();
        *current_context = Some(original_context);
    }
    h.run_steps(2);
    h.state_mut().0.start_preview();
    assert!(!h.state().0.busy());
    assert!(h.state().0.prepared.is_none());
    assert!(h.state().0.status.contains("tickets exhausted"));
    click(&mut h, "Install local sprite package");
    assert!(h
        .get_by_label("Preview sprite package install")
        .accesskit_node()
        .is_disabled());
}
