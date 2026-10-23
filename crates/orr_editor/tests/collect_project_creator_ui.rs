//! Actual main-editor File dialogs, saved project admission and software-GPU proof.
//! Physical egui events are injected; this is not native OS keyboard evidence.
//! The ignored GPU gate is mandatory acceptance, never a missing-adapter skip.
#![cfg(all(
    feature = "project-create",
    feature = "collect-ui",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_sample::{
    authored_ui::{Document as UiDocument, Kind},
    collect_game::{CollectActor, CollectRun, PLAYER, PLAYING, WON},
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

const NEW: &str = "New Collect project…";
const OPEN: &str = "Open Collect project…";
const GAME_ID: &str = "12345678-1234-4234-8234-123456789abc";
const SEED: &str = "editor-file-collect-creator-v1";
const WINDOW: [f32; 2] = [1800.0, 1200.0];
type Files = BTreeMap<String, Vec<u8>>;
type Image = (u32, u32, Vec<u8>);

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
}
fn snapshot(root: &Path) -> Files {
    fn visit(root: &Path, at: &Path, files: &mut Files) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "project cannot depend on source links");
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(kind.is_file());
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = Files::new();
    visit(root, root, &mut files);
    files
}
fn installed(files: &Files) -> Files {
    files
        .iter()
        .filter(|(name, _)| name.starts_with(".orr/") || *name == "orr.packages.lock.json")
        .map(|(name, bytes)| (name.clone(), bytes.clone()))
        .collect()
}
fn prepared(root: &Path) -> PreparedProject {
    PreparedProject::open_with_ui(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
        true,
    )
    .unwrap()
}
fn software_vulkan() -> egui_wgpu::WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan
                    && info.device_type == wgpu::DeviceType::Cpu
                    && info.name.to_lowercase().contains("llvmpipe")
            })
            .cloned()
            .ok_or_else(|| "mandatory llvmpipe Vulkan CPU adapter unavailable".into())
    }));
    setup.into()
}
fn initial_scene(root: &Path) -> PathBuf {
    let scene = root.join("initial-physics.scene.yaml");
    fs::copy(orr_editor::editor::default_scene_path(), &scene).unwrap();
    scene
}
fn start(scene: &Path, evidence: Option<&Path>) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open(scene).unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::PhysGame);
    let builder = Harness::builder().with_size(WINDOW).with_max_steps(120);
    let mut h = if let Some(evidence) = evidence {
        let evidence = evidence.to_owned();
        // HarnessBuilder::wgpu_setup uses PREDICTABLE regardless of the
        // builder options. Construct the exact production renderer explicitly.
        let render_state = egui_kittest::wgpu::create_render_state(
            software_vulkan(),
            egui_wgpu::RendererOptions::default(),
        );
        builder
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
                render_state,
            ))
            .build_eframe(move |cc| {
                let info = cc.wgpu_render_state.as_ref().unwrap().adapter.get_info();
                assert_eq!(info.device_type, wgpu::DeviceType::Cpu);
                assert_eq!(info.backend, wgpu::Backend::Vulkan);
                fs::write(
                    evidence.join("adapter.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "name": info.name, "backend": format!("{:?}", info.backend),
                        "device_type": format!("{:?}", info.device_type), "driver": info.driver,
                        "driver_info": info.driver_info, "renderer_options": "default",
                        "predictable_texture_filtering": false, "dithering": true,
                    }))
                    .unwrap(),
                )
                .unwrap();
                EditorApp::new(editor, cc.wgpu_render_state.clone())
            })
    } else {
        builder
            .renderer(egui_kittest::LazyRenderer::Uninitialized {
                textures_delta: Default::default(),
                builder: None,
            })
            .build_eframe(move |_| EditorApp::new(editor, None))
    };
    settle(&mut h);
    h
}
fn wait(h: &mut Harness<'_, EditorApp>, reason: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        h.run_steps(1);
        assert!(
            h.output().viewport_output.values().all(|output| output
                .commands
                .iter()
                .all(|command| !matches!(command, egui::ViewportCommand::Close))),
            "editor must remain open"
        );
        if ready(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{reason}: {:?}",
            h.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    h.run_steps(3);
    wait(h, "coherent final painted frame", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == app.editor.is_playing_mode())
    });
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}
fn menu(h: &mut Harness<'_, EditorApp>, item: &str) {
    click(h, "File");
    let button = h.get_by_role_and_label(Role::Button, item);
    assert!(
        !button.accesskit_node().is_disabled(),
        "File menu disabled: {item}"
    );
    button.click();
    settle(h);
}
fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    if value.is_empty() {
        h.key_press(Key::Backspace);
    } else {
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
    }
    h.run_steps(3);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(value),
        "the explicit field value must not be silently changed"
    );
}
fn value(h: &mut Harness<'_, EditorApp>, old: &str, new: &str) {
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some(old))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some(old))
        .type_text(new);
    h.run_steps(3);
}
fn new_form(h: &mut Harness<'_, EditorApp>, root: &Path, seed: &str, game_id: &str) {
    menu(h, NEW);
    field(h, "Project directory", root.to_str().unwrap());
    field(h, "Seed", seed);
    field(h, "Game UUIDv4", game_id);
}
fn create(h: &mut Harness<'_, EditorApp>, root: &Path) {
    new_form(h, root, SEED, GAME_ID);
    click(h, "Create and open");
    admitted(h, root);
    assert!(
        h.query_by_role_and_label(Role::TextInput, "Project directory")
            .is_none(),
        "successful creation closes the modal"
    );
}
fn open(h: &mut Harness<'_, EditorApp>, root: &Path) {
    menu(h, OPEN);
    field(h, "Project directory", root.to_str().unwrap());
    click(h, "Open project");
    admitted(h, root);
}
fn admitted(h: &mut Harness<'_, EditorApp>, root: &Path) {
    wait(h, "admitted Collect host and presentation", |app| {
        app.editor.game().is_collect()
            && app.editor.path() == Some(root.join("level.scene.yaml"))
            && app.collect_ui.is_some()
            && app.sprites.bindings.is_some()
    });
    settle(h);
    let project = prepared(root);
    assert_eq!(
        h.state().editor.checksum(),
        project.scene().frame().checksum()
    );
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document(),
        &project.ui().unwrap().document
    );
    assert_eq!(
        h.state().sprites.bindings.as_ref().unwrap().document(),
        &project.sprites().unwrap().document
    );
    assert!(!h.state().editor.is_dirty());
    assert!(h.state().sprites.scene_matches(&h.state().editor));
}
#[derive(Debug, PartialEq)]
struct HostStamp {
    game: EditorGame,
    path: Option<PathBuf>,
    checksum: u64,
    history: orr_editor::model::History,
    scene: String,
    mode: Mode,
    input: Phase,
}
fn stamp(h: &mut Harness<'_, EditorApp>) -> HostStamp {
    settle(h);
    let editor = &mut h.state_mut().editor;
    let scene = editor
        .host_call("scene.save", serde_json::json!({}))
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    HostStamp {
        game: editor.game(),
        path: editor.path(),
        checksum: editor.checksum(),
        history: editor.history().clone(),
        scene,
        mode: editor.mode(),
        input: editor.input_phase(),
    }
}
fn identity(root: &Path) -> (serde_json::Value, Vec<String>) {
    let project = prepared(root);
    let scene = orr_reflect::Scene::parse(
        project.scene().text(),
        &orr_sample::collect_project::types(),
    )
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("orr.project.json")).unwrap()).unwrap();
    (
        manifest["progress"].clone(),
        scene.entities.keys().map(ToString::to_string).collect(),
    )
}
fn assert_assets(root: &Path) {
    let project = prepared(root);
    let package = orr_editor::sprite_bindings::open_project(root).unwrap();
    let lock = package.verify().unwrap();
    assert_eq!(lock.direct.len(), 2);
    assert_eq!(lock.direct["sample-sprites"], "1.0.0");
    assert_eq!(lock.direct["korean-game-ui"], "1.0.0");
    assert_eq!(
        package
            .read_asset("sample-sprites", "lantern_keeper.png")
            .unwrap(),
        include_bytes!("../../../assets/sprite_demo/lantern_keeper.png")
    );
    assert_eq!(
        package
            .read_asset("korean-game-ui", "OrreryKoreanUI.otf")
            .unwrap(),
        include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf")
    );
    assert_eq!(
        project.ui().unwrap().font,
        include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf")
    );
    assert_eq!(project.sprites().unwrap().document.bindings.len(), 4);
    assert_eq!(
        identity(root).0,
        serde_json::json!({"schema":1,"game_id":GAME_ID,"profile":"collect-dodge-highscore-v1"})
    );
}
fn select_ui_node(h: &mut Harness<'_, EditorApp>, name: &str) {
    h.get_by_role_and_label(Role::ComboBox, "UI node").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, name).click();
    h.run_steps(3);
}
fn edit_and_save(h: &mut Harness<'_, EditorApp>, root: &Path) {
    let before = snapshot(root);
    click(h, "Collect UI document");
    select_ui_node(h, "score");
    value(h, "Score: ", "게임 메뉴 ");
    assert_eq!(
        snapshot(root),
        before,
        "HUD editing must not implicitly save"
    );
    click(h, "Undo UI edit");
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document(),
        &UiDocument::default_collect()
    );
    value(h, "Score: ", "게임 메뉴 ");
    click(h, "Save UI document");
    let ui = h.state().collect_ui.as_ref().unwrap().document().clone();
    assert_eq!(prepared(root).ui().unwrap().document, ui);
    click(h, "Collect UI document");
    click(h, "first_collectible");
    wait(h, "inspector", |app| app.editor.inspect().is_some());
    value(h, "10", "12");
    h.key_press(Key::Enter);
    wait(h, "actual inspector change", |app| {
        app.editor.is_dirty() && app.editor.bodies().iter().any(|b| b.pos == [12.0, 0.0])
    });
    assert_eq!(
        fs::read(root.join("level.scene.yaml")).unwrap(),
        before["level.scene.yaml"]
    );
    click(h, "File");
    h.get_by(|node| {
        node.role() == Role::Button
            && node
                .label()
                .is_some_and(|label| label == "Save" || label.starts_with("Save Ctrl"))
    })
    .click();
    wait(h, "explicit scene Save", |app| !app.editor.is_dirty());
    let after = snapshot(root);
    assert_ne!(before["level.scene.yaml"], after["level.scene.yaml"]);
    assert_ne!(before["level.ui.json"], after["level.ui.json"]);
    let mut expected = before;
    expected.insert("level.scene.yaml".into(), after["level.scene.yaml"].clone());
    expected.insert("level.ui.json".into(), after["level.ui.json"].clone());
    assert_eq!(
        after, expected,
        "only explicitly saved documents may change"
    );
}

#[test]
fn creator_file_widgets_create_save_reopen_preserve_identity() {
    let work = tempdir();
    let scene = initial_scene(work.path());
    let initial = fs::read(&scene).unwrap();
    let root = work.path().join("created Collect project");
    let mut h = start(&scene, None);
    create(&mut h, &root);
    assert_assets(&root);
    let original_identity = identity(&root);
    let original_installed = installed(&snapshot(&root));
    edit_and_save(&mut h, &root);
    let saved = snapshot(&root);
    let saved_checksum = h.state().editor.checksum();
    open(&mut h, &root);
    assert_eq!(h.state().editor.checksum(), saved_checksum);
    assert_eq!(identity(&root), original_identity);
    assert_eq!(installed(&snapshot(&root)), original_installed);
    assert_eq!(snapshot(&root), saved);
    drop(h);
    let mut fresh = start(&scene, None);
    open(&mut fresh, &root);
    assert_eq!(fresh.state().editor.checksum(), saved_checksum);
    gameplay(&mut fresh, None);
    assert_eq!(snapshot(&root), saved);
    assert_eq!(identity(&root), original_identity);
    assert_eq!(fs::read(scene).unwrap(), initial);
}

fn rejected_submit(h: &mut Harness<'_, EditorApp>, button: &str) {
    if !h
        .get_by_role_and_label(Role::Button, button)
        .accesskit_node()
        .is_disabled()
    {
        click(h, button);
    }
    h.get_by_role_and_label(Role::TextInput, "Project directory");
}
#[test]
fn creator_cancel_invalid_inputs_and_existing_dest_preserve_host() {
    use std::os::unix::fs::symlink;
    let work = tempdir();
    let scene = initial_scene(work.path());
    let mut h = start(&scene, None);
    let before = stamp(&mut h);
    let destination = work.path().join("cancelled");
    new_form(&mut h, &destination, SEED, GAME_ID);
    click(&mut h, "Cancel");
    assert!(!destination.exists());
    assert_eq!(stamp(&mut h), before);
    for (path, seed, uuid) in [
        (
            PathBuf::from("relative-project"),
            SEED.to_string(),
            GAME_ID.to_string(),
        ),
        (
            work.path().join("missing/child"),
            SEED.into(),
            GAME_ID.into(),
        ),
        (
            work.path().join("empty-seed"),
            String::new(),
            GAME_ID.into(),
        ),
        (
            work.path().join("bad-seed"),
            "invalid seed".into(),
            GAME_ID.into(),
        ),
        (
            work.path().join("long-seed"),
            "a".repeat(129),
            GAME_ID.into(),
        ),
        (
            work.path().join("bad-uuid"),
            SEED.into(),
            "not-a-uuid".into(),
        ),
        (
            work.path().join("wrong-version"),
            SEED.into(),
            GAME_ID.replacen("4234", "1234", 1),
        ),
        (
            work.path().join("uppercase-uuid"),
            SEED.into(),
            GAME_ID.to_uppercase(),
        ),
    ] {
        new_form(&mut h, &path, &seed, &uuid);
        rejected_submit(&mut h, "Create and open");
        assert!(!path.exists(), "invalid input published {}", path.display());
        assert_eq!(stamp(&mut h), before);
        click(&mut h, "Cancel");
    }
    let existing = work.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("sentinel"), b"must not replace").unwrap();
    let existing_bytes = snapshot(&existing);
    let regular = work.path().join("regular-file");
    fs::write(&regular, b"not a project").unwrap();
    let link = work.path().join("linked-destination");
    symlink(&existing, &link).unwrap();
    let linked_parent = work.path().join("linked-parent");
    symlink(&existing, &linked_parent).unwrap();
    let special = work.path().join("special-fifo");
    assert!(Command::new("mkfifo")
        .arg(&special)
        .status()
        .unwrap()
        .success());
    for destination in [
        &existing,
        &regular,
        &link,
        &linked_parent.join("new"),
        &special,
    ] {
        new_form(&mut h, destination, SEED, GAME_ID);
        rejected_submit(&mut h, "Create and open");
        assert_eq!(stamp(&mut h), before);
        click(&mut h, "Cancel");
    }
    assert_eq!(snapshot(&existing), existing_bytes);
    assert_eq!(fs::read(&regular).unwrap(), b"not a project");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    let root = work.path().join("valid");
    create(&mut h, &root);
    let created = snapshot(&root);
    let collect = stamp(&mut h);
    new_form(
        &mut h,
        &root,
        "second-seed",
        "87654321-4321-4321-8321-cba987654321",
    );
    rejected_submit(&mut h, "Create and open");
    assert_eq!(
        snapshot(&root),
        created,
        "repeat submit never regenerates identity or files"
    );
    assert_eq!(stamp(&mut h), collect);
    click(&mut h, "Cancel");
    let invalid = work.path().join("invalid-project");
    fs::create_dir(&invalid).unwrap();
    fs::write(invalid.join("orr.project.json"), b"invalid json").unwrap();
    let valid_link = work.path().join("linked-valid-project");
    symlink(&root, &valid_link).unwrap();
    let bad_ui = work.path().join("malformed-authored-ui");
    let bad_font = work.path().join("tampered-installed-font");
    for copy in [&bad_ui, &bad_font] {
        for (name, bytes) in &created {
            let destination = copy.join(name);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
    }
    fs::write(bad_ui.join("level.ui.json"), b"malformed UI").unwrap();
    let font = created
        .keys()
        .find(|name| name.ends_with("/OrreryKoreanUI.otf"))
        .unwrap();
    fs::write(bad_font.join(font), b"tampered installed package").unwrap();
    let rejected_ui_bytes = snapshot(&bad_ui);
    let rejected_font_bytes = snapshot(&bad_font);
    let ui_before = h.state().collect_ui.as_ref().unwrap().document().clone();
    let sprites_before = h
        .state()
        .sprites
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    for candidate in [
        &invalid,
        &regular,
        &link,
        &special,
        &valid_link,
        &bad_ui,
        &bad_font,
    ] {
        menu(&mut h, OPEN);
        field(&mut h, "Project directory", candidate.to_str().unwrap());
        rejected_submit(&mut h, "Open project");
        assert_eq!(stamp(&mut h), collect);
        assert_eq!(snapshot(&root), created);
        assert_eq!(
            h.state().collect_ui.as_ref().unwrap().document(),
            &ui_before
        );
        assert_eq!(
            h.state().sprites.bindings.as_ref().unwrap().document(),
            &sprites_before
        );
        click(&mut h, "Cancel");
    }
    assert_eq!(snapshot(&bad_ui), rejected_ui_bytes);
    assert_eq!(snapshot(&bad_font), rejected_font_bytes);
    assert!(
        fs::read_dir(work.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-create-")),
        "no owned transaction leaks"
    );
}

fn assert_switch_blocked(h: &mut Harness<'_, EditorApp>) {
    let before = stamp(h);
    let ui = h.state().collect_ui.as_ref().unwrap().document().clone();
    let sprites = h
        .state()
        .sprites
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    for label in [NEW, OPEN] {
        click(h, "File");
        let item = h.get_by_role_and_label(Role::Button, label);
        assert!(item.accesskit_node().is_disabled(), "dirty or stale presentation must disable {label} independently of dialog field validity");
        h.key_press(Key::Escape);
        settle(h);
        assert_eq!(stamp(h), before);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &ui);
        assert_eq!(
            h.state().sprites.bindings.as_ref().unwrap().document(),
            &sprites
        );
    }
}
#[test]
fn creator_dirty_or_stale_presentation_blocks_switch() {
    let work = tempdir();
    let scene = initial_scene(work.path());
    let root = work.path().join("guarded project");
    let mut h = start(&scene, None);
    create(&mut h, &root);
    let files = snapshot(&root);
    click(&mut h, "Collect UI document");
    select_ui_node(&mut h, "title");
    value(&mut h, "Collect and Dodge · 플레이", "게임 메뉴");
    let dirty_ui = h.state().collect_ui.as_ref().unwrap().document().clone();
    assert_switch_blocked(&mut h);
    assert_eq!(snapshot(&root), files);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &dirty_ui);
    click(&mut h, "Undo UI edit");
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document(),
        &UiDocument::default_collect()
    );
    click(&mut h, "Collect UI document");
    click(&mut h, "Sprite bindings (view only)");
    click(&mut h, "Clear camera follow");
    assert!(h.state().sprites.bindings.as_ref().unwrap().dirty());
    assert_switch_blocked(&mut h);
    assert_eq!(snapshot(&root), files);
    click(&mut h, "Undo binding");
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    // An externally changed accepted UI must not be silently discarded either.
    fs::write(root.join("level.ui.json"), b"stale external UI").unwrap();
    assert_switch_blocked(&mut h);
    assert_eq!(
        fs::read(root.join("level.ui.json")).unwrap(),
        b"stale external UI"
    );
    fs::write(root.join("level.ui.json"), &files["level.ui.json"]).unwrap();
    fs::write(root.join("level.sprites.json"), b"stale external sprites").unwrap();
    assert_switch_blocked(&mut h);
    assert_eq!(
        fs::read(root.join("level.sprites.json")).unwrap(),
        b"stale external sprites"
    );
    fs::write(
        root.join("level.sprites.json"),
        &files["level.sprites.json"],
    )
    .unwrap();
    assert_eq!(snapshot(&root), files);
    open(&mut h, &root);
}

fn physical(h: &mut Harness<'_, EditorApp>, key: Key, pressed: bool) {
    h.event(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}
fn run(h: &Harness<'_, EditorApp>) -> CollectRun {
    *h.state()
        .editor
        .snapshot()
        .unwrap()
        .verified()
        .expect("authoritative displayed snapshot")
        .singleton::<CollectRun>()
}
fn gameplay(h: &mut Harness<'_, EditorApp>, evidence: Option<&Path>) {
    let before = h.state().editor.checksum();
    click(h, LBL_PLAY);
    wait(h, "Play ready for input", |app| {
        app.editor.can_take_control()
    });
    click(h, "Take control");
    wait(h, "actual input lease", |app| {
        app.editor.input_phase() == Phase::Active
    });
    if let Some(evidence) = evidence {
        capture(h, evidence, "playing", 4);
    }
    physical(h, Key::D, true);
    wait(h, "right movement collects both objects", |app| {
        app.editor.snapshot().is_some_and(|s| {
            s.verified()
                .is_some_and(|f| f.singleton::<CollectRun>().phase == WON)
        })
    });
    physical(h, Key::D, false);
    assert_eq!(run(h).score, 2);
    if let Some(evidence) = evidence {
        capture(h, evidence, "won", 2);
    }
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key {
            key: Key::Space,
            physical_key: Some(Key::Space),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    wait(h, "restart edge acknowledged", |app| {
        app.editor.snapshot().is_some_and(|s| {
            s.verified().is_some_and(|f| {
                let run = f.singleton::<CollectRun>();
                run.phase == PLAYING && run.score == 0
            })
        })
    });
    let player = h
        .state()
        .editor
        .snapshot()
        .unwrap()
        .verified()
        .unwrap()
        .dense::<CollectActor>()
        .1
        .iter()
        .find(|actor| actor.kind == PLAYER)
        .unwrap();
    assert_eq!(player.position, orr_fp::FPVec2::ZERO);
    if let Some(evidence) = evidence {
        capture(h, evidence, "restarted", 4);
    }
    click(h, LBL_STOP);
    wait(h, "Stop restores Edit", |app| {
        app.editor.mode() == Mode::Edit
            && app.editor.input_phase() == Phase::Off
            && app
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_none())
    });
    settle(h);
    assert_eq!(h.state().editor.checksum(), before);
    if let Some(evidence) = evidence {
        capture(h, evidence, "stopped", 4);
    }
}

fn image(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Image {
    // Kittest paints its synthetic cursor. Remove that overlay before testing
    // scene texels, without changing the production renderer or scene geometry.
    h.event(egui::Event::PointerGone);
    settle(h);
    assert!(h.state().has_gpu());
    assert_eq!(h.ctx.pixels_per_point(), 1.0);
    let rendered = h.render().expect("mandatory real EditorApp GPU readback");
    assert_eq!(rendered.dimensions(), (WINDOW[0] as u32, WINDOW[1] as u32));
    rendered.save(evidence.join(format!("{name}.png"))).unwrap();
    (rendered.width(), rendered.height(), rendered.into_raw())
}
/// Production nearest sampling of opaque reviewed source pixels. This checks
/// every opaque texel, with exact bytes and the actual viewport geometry.
fn capture(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str, expected: usize) {
    let image = image(h, evidence, name);
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let size = app.ui.viewport_px;
    let bindings = app.sprites.bindings.as_ref().unwrap();
    let mut actors = Vec::new();
    for (guid, binding) in &bindings.document().bindings {
        let row = app
            .editor
            .rows()
            .iter()
            .find(|r| r.guid.as_ref().is_some_and(|g| g.as_str() == guid))
            .unwrap();
        let Some(body) = app
            .editor
            .bodies()
            .iter()
            .find(|body| body.entity == row.entity)
        else {
            continue;
        };
        let asset = orr_editor::sprite_bindings::load_asset(
            bindings.base(),
            &binding.package,
            &binding.document,
        )
        .unwrap();
        let region_id = app
            .sprites
            .sampled_region(&app.editor, guid)
            .expect("visible admitted sprite");
        let region = asset.document.region(region_id).unwrap();
        assert_eq!(
            body.angle, 0.0,
            "starter geometry uses unrotated sprite quads"
        );
        let mut checked = 0;
        for y in 0..region.height {
            for x in 0..region.width {
                let i =
                    (((region.y + y) * asset.document.atlas().width + region.x + x) * 4) as usize;
                if asset.rgba[i + 3] != 255 {
                    continue;
                }
                let at = app.editor.camera.world_to_screen(
                    [
                        body.pos[0]
                            + (x as f32 + 0.5 - region.width as f32 / 2.0)
                                * binding.units_per_pixel,
                        body.pos[1]
                            + (region.height as f32 / 2.0 - y as f32 - 0.5)
                                * binding.units_per_pixel,
                    ],
                    size,
                );
                let px = (rect.min.x + at[0] * rect.width() / size.0 as f32).floor();
                let py = (rect.min.y + at[1] * rect.height() / size.1 as f32).floor();
                assert!(
                    px >= rect.min.x && py >= rect.min.y && px < rect.max.x && py < rect.max.y,
                    "sprite texel must remain inside actual viewport"
                );
                let j = ((py as u32 * image.0 + px as u32) * 4) as usize;
                assert_eq!(
                    &image.2[j..j + 4],
                    &asset.rgba[i..i + 4],
                    "{name}: {guid}, region {region_id}, texel {x},{y} at {px},{py}"
                );
                checked += 1;
            }
        }
        assert!(checked > 40);
        actors.push(serde_json::json!({"guid":guid,"region":region_id,"position":body.pos,"opaque_texels":checked}));
    }
    assert_eq!(actors.len(), expected);
    let snapshot = app.editor.snapshot().unwrap();
    let verified = snapshot.verified().unwrap();
    assert_eq!(verified.tick(), snapshot.tick());
    assert_eq!(verified.checksum(), snapshot.predicted().checksum());
    fs::write(evidence.join(format!("{name}.json")), serde_json::to_vec_pretty(&serde_json::json!({
        "phase":run(h).phase,"score":run(h).score,"tick":snapshot.tick(),
        "checksum":format!("{:016x}",verified.checksum()),"verified_tick":snapshot.verified_tick(),
        "viewport":[rect.min.x,rect.min.y,rect.width(),rect.height()],"viewport_pixels":[size.0,size.1],
        "window_pixels":[image.0,image.1],"actors":actors,"exact_opaque_source_texels":true,
    })).unwrap()).unwrap();
}

fn preview_canvas(h: &Harness<'_, EditorApp>) -> egui::Rect {
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) if matches!(mesh.texture_id, egui::TextureId::User(_)) => {
                let rect = mesh.calc_bounds();
                ((rect.width() - 640.0).abs() < f32::EPSILON
                    && (rect.height() - 360.0).abs() < f32::EPSILON)
                    .then_some(rect)
            }
            _ => None,
        })
        .next()
        .expect("actual 640×360 authored-screen texture quad")
}
fn preview(h: &mut Harness<'_, EditorApp>, evidence: &Path, root: &Path) {
    let files = snapshot(root);
    let host = stamp(h);
    click(h, "Collect UI document");
    click(h, "Preview authored screens");
    let korean = image(h, evidence, "korean-title");
    let canvas = preview_canvas(h);
    select_ui_node(h, "title");
    value(h, "Collect and Dodge · 플레이", "Collect and Dodge · ");
    let blank = image(h, evidence, "title-without-korean");
    assert_eq!(preview_canvas(h), canvas);
    let node = UiDocument::default_collect()
        .nodes
        .into_iter()
        .find(|node| node.id == "title")
        .unwrap();
    let mut changed = 0;
    for y in canvas.min.y.ceil() as u32..canvas.max.y.floor() as u32 {
        for x in canvas.min.x.ceil() as u32..canvas.max.x.floor() as u32 {
            let offset = ((y * korean.0 + x) * 4) as usize;
            if korean.2[offset..offset + 4] != blank.2[offset..offset + 4] {
                let local = egui::pos2(x as f32 - canvas.min.x, y as f32 - canvas.min.y);
                assert!(
                    egui::Rect::from_min_size(
                        egui::pos2(
                            640.0 * f32::from(node.anchor[0]) / 1000.0 + f32::from(node.offset[0]),
                            360.0 * f32::from(node.anchor[1]) / 1000.0 + f32::from(node.offset[1])
                        ),
                        egui::vec2(f32::from(node.size[0]), f32::from(node.size[1]))
                    )
                    .contains(local),
                    "only the title node may change"
                );
                changed += 1;
            }
        }
    }
    assert!(
        changed > 0,
        "installed Korean glyphs must contribute actual preview pixels"
    );
    click(h, "Undo UI edit");
    let restored = image(h, evidence, "korean-title-restored");
    for y in canvas.min.y.ceil() as u32..canvas.max.y.floor() as u32 {
        let start = ((y * korean.0 + canvas.min.x.ceil() as u32) * 4) as usize;
        let end = ((y * korean.0 + canvas.max.x.floor() as u32) * 4) as usize;
        assert_eq!(
            &korean.2[start..end],
            &restored.2[start..end],
            "undo exactly restores preview pixels"
        );
    }
    click(h, "Playing");
    image(h, evidence, "hud-preview");
    click(h, "Close screen preview");
    click(h, "Collect UI document");
    assert_eq!(stamp(h), host);
    assert_eq!(snapshot(root), files);
    fs::write(
        evidence.join("korean-preview.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "canvas":[canvas.min.x,canvas.min.y,canvas.width(),canvas.height()],
            "changed_korean_pixels":changed,"changes_confined_to_title_geometry":true,
            "undo_restores_exact_canvas_pixels":true,"host_and_files_unchanged":true,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "mandatory llvmpipe GPU acceptance with retained evidence and source-hidden child"]
fn creator_file_widgets_llvmpipe_korean_sprites_gameplay() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_CREATOR_EVIDENCE").expect("retained evidence directory"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let work = tempdir();
    let scene = initial_scene(work.path());
    let root = work.path().join("GUI created Collect project");
    let mut h = start(&scene, Some(&evidence));
    create(&mut h, &root);
    assert_assets(&root);
    let original_identity = identity(&root);
    let package_bytes = installed(&snapshot(&root));
    capture(&mut h, &evidence, "created", 4);
    preview(&mut h, &evidence, &root);
    edit_and_save(&mut h, &root);
    let saved = snapshot(&root);
    open(&mut h, &root);
    capture(&mut h, &evidence, "same-app-reopened", 4);
    gameplay(&mut h, Some(&evidence));
    assert_eq!(snapshot(&root), saved);
    assert_eq!(identity(&root), original_identity);
    assert_eq!(installed(&snapshot(&root)), package_bytes);
    drop(h);
    source_hidden_child(work.path(), &root, &scene, &evidence);
    assert_eq!(snapshot(&root), saved);
    let child: serde_json::Value =
        serde_json::from_slice(&fs::read(evidence.join("source-hidden/receipt.json")).unwrap())
            .unwrap();
    assert_eq!(child["source_hidden"], true);
    fs::write(evidence.join("collect-creator-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"template":orr_sample::project_create::COLLECT_UI_TEMPLATE,"seed":SEED,"game_id":GAME_ID,
        "main_editor_app":true,"starts_in_phys_game":true,"actual_file_new_dialog":true,"actual_file_open_dialog":true,
        "same_app_save_reopen":true,"fresh_app_source_hidden_reopen":true,"installed_package_bytes_unchanged":true,
        "identity_and_guids_unchanged":true,"authoritative_win_restart_stop":true,"exact_opaque_source_texels":true,
        "korean_preview_pixels":true,"renderer_options":"default","physical_egui_events":true,"native_os_keyboard":false,
        "window_pixels":[WINDOW[0] as u32,WINDOW[1] as u32],"source_hidden":child,
        "package_files":package_bytes.keys().collect::<Vec<_>>(),"current_exe":std::env::current_exe().unwrap(),
    })).unwrap()).unwrap();
}
fn source_hidden_child(work: &Path, root: &Path, scene: &Path, evidence: &Path) {
    let executable = work.join("collect-project-creator-test");
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    assert!(!work.starts_with(&repository));
    assert!(
        !evidence.starts_with(&repository),
        "source-hidden evidence must be outside repository"
    );
    let child_evidence = evidence.join("source-hidden");
    fs::create_dir(&child_evidence).unwrap();
    let output = Command::new("bwrap")
        .args(["--die-with-parent", "--ro-bind", "/", "/", "--bind"])
        .arg(work)
        .arg(work)
        .arg("--bind")
        .arg(evidence)
        .arg(evidence)
        .arg("--tmpfs")
        .arg(&repository)
        .arg("--")
        .arg(&executable)
        .args([
            "--exact",
            "creator_source_hidden_reopen",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(work)
        .env("ORR_COLLECT_CREATOR_CHILD", root)
        .env("ORR_COLLECT_CREATOR_INITIAL_SCENE", scene)
        .env("ORR_COLLECT_CREATOR_HIDDEN", &repository)
        .env("ORR_COLLECT_CREATOR_EVIDENCE", &child_evidence)
        .output()
        .expect("mandatory bubblewrap source-hidden child");
    fs::write(
        evidence.join("source-hidden-child.log"),
        [&output.stdout[..], &output.stderr[..]].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "source-hidden child failed: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "invoked only by source-hidden GPU parent with exact copied executable"]
fn creator_source_hidden_reopen() {
    let root = PathBuf::from(
        std::env::var_os("ORR_COLLECT_CREATOR_CHILD").expect("source-hidden parent required"),
    );
    let hidden = PathBuf::from(std::env::var_os("ORR_COLLECT_CREATOR_HIDDEN").unwrap());
    assert_eq!(
        fs::read_dir(&hidden).unwrap().count(),
        0,
        "whole source repository is hidden"
    );
    assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).exists());
    let scene = PathBuf::from(std::env::var_os("ORR_COLLECT_CREATOR_INITIAL_SCENE").unwrap());
    let evidence = PathBuf::from(std::env::var_os("ORR_COLLECT_CREATOR_EVIDENCE").unwrap());
    let files = snapshot(&root);
    let before_identity = identity(&root);
    let mut h = start(&scene, Some(&evidence));
    open(&mut h, &root);
    assert_assets(&root);
    capture(&mut h, &evidence, "fresh-reopened", 4);
    // The saved authored Korean HUD is retained, rather than regenerated defaults.
    assert!(h
        .state()
        .collect_ui
        .as_ref()
        .unwrap()
        .document()
        .nodes
        .iter()
        .any(|node| node.id == "score"
            && matches!(&node.kind, Kind::Label { text, .. } if text == "게임 메뉴 ")));
    click(&mut h, "Collect UI document");
    click(&mut h, "Preview authored screens");
    click(&mut h, "Playing");
    image(&mut h, &evidence, "saved-korean-hud");
    click(&mut h, "Close screen preview");
    click(&mut h, "Collect UI document");
    gameplay(&mut h, Some(&evidence));
    assert_eq!(snapshot(&root), files);
    assert_eq!(identity(&root), before_identity);
    fs::write(evidence.join("receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"source_hidden":true,"actual_file_open_dialog":true,"fresh_app":true,
        "installed_bytes_only":true,"files_and_identity_unchanged":true,"gameplay_win_restart_stop":true,
        "renderer_options":"default","native_os_keyboard":false,"current_exe":std::env::current_exe().unwrap(),
    })).unwrap()).unwrap();
}
