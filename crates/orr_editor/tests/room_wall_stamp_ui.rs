#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "windows-projects",
    feature = "room-authoring"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    editor::Mode,
    game::EditorGame,
    Editor, EditorApp, HostSpec, Target,
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::room_project::PreparedProject;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    other: PathBuf,
    stamp: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("room wall groups project");
        let other = temp.path().join("other room project");
        orr_sample::windows_project::create_room_project(&root, "room-wall-stamps-main")
            .expect("create an admitted Room project");
        orr_sample::windows_project::create_room_project(&other, "room-wall-stamps-other")
            .expect("create an independent Room project");
        let stamp = temp.path().join("two walls 東京.room-stamp.json");
        Self {
            _temp: temp,
            root,
            other,
            stamp,
        }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open(&self.root).unwrap();
        let camera = prepared.take_camera().expect("Room camera is admitted");
        let (_, path, scene, models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        assert_eq!(editor.game(), EditorGame::RoomEscape);
        editor.install_room_camera(camera.document.clone()).unwrap();
        Harness::builder()
            .with_size([1600.0, 1150.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                let mut app = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                app
            })
    }
}

struct HiddenStamp {
    path: PathBuf,
    hidden: PathBuf,
    bytes: Vec<u8>,
    is_hidden: bool,
}

impl HiddenStamp {
    fn new(path: PathBuf) -> Self {
        let bytes = fs::read(&path).expect("saved wall stamp exists");
        let hidden = path.with_file_name("hidden wall stamp.room-stamp.json");
        assert!(!hidden.exists());
        Self {
            path,
            hidden,
            bytes,
            is_hidden: false,
        }
    }

    fn hide(&mut self) {
        assert!(!self.is_hidden);
        fs::rename(&self.path, &self.hidden).expect("hide the source wall stamp");
        self.is_hidden = true;
    }

    fn restore(&mut self) {
        if self.is_hidden {
            assert!(!self.path.exists());
            fs::rename(&self.hidden, &self.path).expect("restore the source wall stamp");
            self.is_hidden = false;
        }
        assert_eq!(fs::read(&self.path).unwrap(), self.bytes);
    }
}

impl Drop for HiddenStamp {
    fn drop(&mut self) {
        if self.is_hidden && !self.path.exists() {
            let _ = fs::rename(&self.hidden, &self.path);
        }
    }
}

fn hardware_vulkan_setup() -> egui_wgpu::WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan
                    && matches!(
                        info.device_type,
                        wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
                    )
            })
            .cloned()
            .ok_or_else(|| "No hardware Vulkan adapter is available".to_owned())
    }));
    setup.into()
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_role(harness: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_role_and_label(role, label).click_accesskit();
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        harness.get_by_label(menu).click_accesskit();
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    harness.get_by_label(item).click_accesskit();
    settle(harness);
}

fn set_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {reason}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn named_wall_guids(app: &EditorApp, name: &str) -> BTreeSet<String> {
    app.editor
        .rows()
        .iter()
        .filter(|row| row.name.as_deref() == Some(name))
        .filter_map(|row| row.guid.as_ref().map(ToString::to_string))
        .collect()
}

fn read_wall_component(app: &mut EditorApp, guid: &str, component: &str) -> orr_reflect::Value {
    app.editor
        .field_of(&Target::parse(guid).unwrap(), component, "")
        .unwrap()
}

fn without_body_position(value: &orr_reflect::Value) -> orr_reflect::Value {
    let orr_reflect::Value::Struct(fields) = value else {
        panic!("expected reflected Body struct, got {value:?}");
    };
    orr_reflect::Value::Struct(
        fields
            .iter()
            .filter(|(name, _)| name != "pos")
            .cloned()
            .collect(),
    )
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn viewport_pixels(app: &EditorApp) -> (u32, u32, Vec<u8>) {
    let viewport = app
        .viewport3d_gpu()
        .expect("actual Room EditorApp 3D viewport")
        .gpu();
    let size = viewport.target().size();
    (size.0, size.1, viewport.read_rgba8())
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>().0.iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(left_pixel, right_pixel)| left_pixel != right_pixel)
        .count()
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(pixels).unwrap();
}

fn capture_if_requested(
    capture_dir: Option<&Path>,
    label: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
    captures: &mut Vec<serde_json::Value>,
) {
    let Some(capture_dir) = capture_dir else {
        return;
    };
    let filename = format!("{label}.png");
    let path = capture_dir.join(&filename);
    write_png(&path, width, height, pixels);
    let bytes = fs::read(&path).unwrap();
    captures.push(serde_json::json!({
        "name":filename,
        "bytes":bytes.len(),
        "sha256":sha256(&bytes)
    }));
}

#[test]
fn room_wall_groups_capture_instantiate_save_reopen_and_play() {
    let acceptance_requested = std::env::var_os("ORR_REQUIRE_GPU").is_some()
        || std::env::var_os("ORR_ROOM_WALL_STAMPS_ACCEPTANCE").is_some();
    if !acceptance_requested {
        eprintln!("SKIP: set ORR_REQUIRE_GPU=1 or ORR_ROOM_WALL_STAMPS_ACCEPTANCE=1 for Room wall stamp GPU acceptance");
        return;
    }

    let preflight =
        Wgpu::headless(WgpuOptions::default()).expect("Room wall stamps acceptance requires a GPU");
    eprintln!(
        "Room wall stamps GPU preflight: {}",
        preflight.adapter_name()
    );
    drop(preflight);

    let fixture = Fixture::new();
    let capture_dir = std::env::var_os("ORR_ROOM_WALL_STAMPS_CAPTURE_DIR").map(PathBuf::from);
    if let Some(path) = &capture_dir {
        assert!(path.is_absolute());
        assert!(!path.exists());
        fs::create_dir(path).unwrap();
    }
    let mut harness = fixture.app();
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::RoomEscape);
    assert!(harness.state().room_camera.is_some());
    let initial_rows = harness.state().editor.rows().len();

    menu_click(&mut harness, "File", "Room level authoring…");
    set_field(&mut harness, "Room wall name", "stamp_source_a");
    set_field(&mut harness, "Wall center X", "2");
    set_field(&mut harness, "Wall center Z", "2");
    set_field(&mut harness, "Wall half-width X", "0.5");
    set_field(&mut harness, "Wall half-depth Z", "0.5");
    click(&mut harness, "Create Room wall");
    let source_a = named_wall_guids(harness.state(), "stamp_source_a");
    assert_eq!(source_a.len(), 1);

    set_field(&mut harness, "Room wall name", "stamp_source_b");
    set_field(&mut harness, "Wall center X", "3");
    set_field(&mut harness, "Wall center Z", "2");
    click(&mut harness, "Create Room wall");
    let source_b = named_wall_guids(harness.state(), "stamp_source_b");
    assert_eq!(source_b.len(), 1);
    assert_eq!(harness.state().editor.rows().len(), initial_rows + 2);
    let source_guids = source_a.union(&source_b).cloned().collect::<BTreeSet<_>>();
    assert_eq!(source_guids.len(), 2);
    let (source_viewport_width, source_viewport_height, source_wall_pixels) =
        viewport_pixels(harness.state());
    assert!(source_viewport_width > 0 && source_viewport_height > 0);
    assert!(source_wall_pixels.iter().any(|channel| *channel != 0));

    let original_actor = read_wall_component(
        harness.state_mut(),
        source_a.iter().next().unwrap(),
        "RoomEscapeV1::Actor",
    );
    let original_body = read_wall_component(
        harness.state_mut(),
        source_a.iter().next().unwrap(),
        "orr_physics3d::Body",
    );
    let original_body_without_position = without_body_position(&original_body);
    let original_collider = read_wall_component(
        harness.state_mut(),
        source_a.iter().next().unwrap(),
        "orr_physics3d::Collider",
    );

    menu_click(&mut harness, "File", "Room wall groups…");
    click(&mut harness, "Read Room walls");
    set_field(
        &mut harness,
        "Room wall stamp file",
        &fixture.stamp.display().to_string(),
    );
    for guid in &source_guids {
        let name = if source_a.contains(guid) {
            "stamp_source_a"
        } else {
            "stamp_source_b"
        };
        click_role(&mut harness, Role::CheckBox, &format!("{name} ({guid})"));
    }
    click(&mut harness, "Capture selected walls");
    assert!(harness
        .query_by_label("Captured Room wall stamp: 2 wall(s); save explicitly.")
        .is_some());
    click(&mut harness, "Save Room wall stamp");
    let saved_stamp_bytes = fs::read(&fixture.stamp).expect("stamp save must create the file");
    assert!(!saved_stamp_bytes.is_empty());
    assert!(harness
        .query_by_label(&format!(
            "Saved Room wall stamp: {}",
            fixture.stamp.display()
        ))
        .is_some());
    click(&mut harness, "Load Room wall stamp");
    assert!(harness
        .query_by_label("Loaded Room wall stamp: 2 wall(s).")
        .is_some());

    set_field(&mut harness, "Stamp offset X", "0");
    set_field(&mut harness, "Stamp offset Z", "-4");
    let before_first_instance = harness.state().editor.checksum();
    click(&mut harness, "Instantiate Room wall stamp");
    let after_first_instance = harness.state().editor.checksum();
    assert_ne!(after_first_instance, before_first_instance);
    let first_a = named_wall_guids(harness.state(), "stamp_source_a");
    let first_b = named_wall_guids(harness.state(), "stamp_source_b");
    assert_eq!(first_a.len(), 2);
    assert_eq!(first_b.len(), 2);
    let first_new = first_a
        .union(&first_b)
        .filter(|guid| !source_guids.contains(*guid))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(first_new.len(), 2);
    for guid in &first_new {
        let actor = read_wall_component(harness.state_mut(), guid, "RoomEscapeV1::Actor");
        let body = read_wall_component(harness.state_mut(), guid, "orr_physics3d::Body");
        let collider = read_wall_component(harness.state_mut(), guid, "orr_physics3d::Collider");
        assert_eq!(actor, original_actor);
        assert_eq!(without_body_position(&body), original_body_without_position);
        assert_eq!(collider, original_collider);
    }
    let undo_label = "Undo Ctrl+Z";
    menu_click(&mut harness, "Edit", undo_label);
    assert_eq!(harness.state().editor.checksum(), before_first_instance);
    assert_eq!(
        named_wall_guids(harness.state(), "stamp_source_a"),
        source_a
    );
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), after_first_instance);

    let (first_width, first_height, first_pixels) = viewport_pixels(harness.state());
    assert_eq!(
        (first_width, first_height),
        (source_viewport_width, source_viewport_height)
    );
    assert!(changed_pixels(&source_wall_pixels, &first_pixels) > 0);
    let mut captures = Vec::new();
    capture_if_requested(
        capture_dir.as_deref(),
        "instantiated-once",
        first_width,
        first_height,
        &first_pixels,
        &mut captures,
    );

    set_field(&mut harness, "Stamp offset X", "0");
    set_field(&mut harness, "Stamp offset Z", "1");
    click(&mut harness, "Instantiate Room wall stamp");
    let final_checksum = harness.state().editor.checksum();
    assert_ne!(final_checksum, after_first_instance);
    let final_a = named_wall_guids(harness.state(), "stamp_source_a");
    let final_b = named_wall_guids(harness.state(), "stamp_source_b");
    assert_eq!(final_a.len(), 3);
    assert_eq!(final_b.len(), 3);
    let final_guids = final_a.union(&final_b).cloned().collect::<BTreeSet<_>>();
    assert_eq!(final_guids.len(), 6);
    let final_new = final_guids
        .difference(&source_guids)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(final_new.len(), 4);
    assert!(source_guids.is_disjoint(&final_new));
    assert!(final_new.is_superset(&first_new));
    for guid in &final_new {
        let body = read_wall_component(harness.state_mut(), guid, "orr_physics3d::Body");
        let actor = read_wall_component(harness.state_mut(), guid, "RoomEscapeV1::Actor");
        let collider = read_wall_component(harness.state_mut(), guid, "orr_physics3d::Collider");
        assert_eq!(actor, original_actor);
        assert_eq!(without_body_position(&body), original_body_without_position);
        assert_eq!(collider, original_collider);
        assert_ne!(body, original_body);
    }
    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving the expanded Room scene", |app| {
        !app.editor.is_dirty()
    });
    assert_eq!(harness.state().editor.checksum(), final_checksum);
    let saved_scene_bytes = fs::read(fixture.root.join("room.scene.yaml")).unwrap();
    harness.state_mut().editor.select(None);
    settle(&mut harness);
    let (saved_width, saved_height, saved_pixels) = viewport_pixels(harness.state());
    capture_if_requested(
        capture_dir.as_deref(),
        "saved-expanded-room",
        saved_width,
        saved_height,
        &saved_pixels,
        &mut captures,
    );

    let mut hidden_stamp = HiddenStamp::new(fixture.stamp.clone());
    hidden_stamp.hide();
    assert!(!fixture.stamp.exists());
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.other.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "opening a second Room project", |app| {
        !app.windows_projects.busy()
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.other.join("room.scene.yaml"),
            )
    });
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.root.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(
        &mut harness,
        "reopening the Room without its stamp source",
        |app| {
            !app.windows_projects.busy()
                && app
                    .windows_projects
                    .status
                    .starts_with("Opened Room project:")
                && path_matches(
                    app.editor.path().as_deref(),
                    &fixture.root.join("room.scene.yaml"),
                )
                && app.editor.game() == EditorGame::RoomEscape
                && !app.editor.is_dirty()
                && app.editor.rows().len() == initial_rows + 6
                && named_wall_guids(app, "stamp_source_a").len() == 3
                && named_wall_guids(app, "stamp_source_b").len() == 3
                && app.models.placements(&app.editor).len() == 2
                && app.room_camera.is_some()
        },
    );
    assert!(
        !fixture.stamp.exists(),
        "scene reopen must not resolve the stamp source"
    );
    assert_eq!(
        fs::read(fixture.root.join("room.scene.yaml")).unwrap(),
        saved_scene_bytes
    );
    assert_eq!(harness.state().editor.checksum(), final_checksum);
    let reopened_a = named_wall_guids(harness.state(), "stamp_source_a");
    let reopened_b = named_wall_guids(harness.state(), "stamp_source_b");
    let reopened_guids = reopened_a
        .union(&reopened_b)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(reopened_guids, final_guids);
    let (reopened_width, reopened_height, reopened_pixels) = viewport_pixels(harness.state());
    assert_eq!(
        (reopened_width, reopened_height),
        (saved_width, saved_height)
    );
    capture_if_requested(
        capture_dir.as_deref(),
        "reopened-with-stamp-hidden",
        reopened_width,
        reopened_height,
        &reopened_pixels,
        &mut captures,
    );
    assert_eq!(changed_pixels(&saved_pixels, &reopened_pixels), 0);
    hidden_stamp.restore();
    assert_eq!(fs::read(&fixture.stamp).unwrap(), saved_stamp_bytes);

    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    let before_step = harness.state().editor.timeline().unwrap().tick;
    click(&mut harness, LBL_STEP);
    assert!(!harness.state().editor.sim().playing);
    assert_eq!(
        harness.state().editor.timeline().unwrap().tick,
        before_step + 1
    );
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), final_checksum);
    let (play_width, play_height, play_pixels) = viewport_pixels(harness.state());
    capture_if_requested(
        capture_dir.as_deref(),
        "play-stop-restored",
        play_width,
        play_height,
        &play_pixels,
        &mut captures,
    );

    if let Some(capture_dir) = capture_dir {
        let receipt = serde_json::json!({
            "project_root":fixture.root,
            "other_project_root":fixture.other,
            "scene_path":fixture.root.join("room.scene.yaml"),
            "scene_checksum":format!("0x{final_checksum:016x}"),
            "stamp_path":fixture.stamp,
            "stamp_sha256":sha256(&saved_stamp_bytes),
            "stamp_hidden_during_reopen":true,
            "stamp_restored_byte_identical":true,
            "initial_wall_rows":initial_rows,
            "authored_source_wall_guids":source_guids,
            "instantiated_wall_guids":final_new,
            "wall_components_preserved":true,
            "instantiation_undo_redo_verified":true,
            "instances_are_ordinary_saved_actors":true,
            "fresh_project_reopen_without_stamp_source":true,
            "reopen_pixels_match_saved":true,
            "play_pause_step_stop_verified":true,
            "capture_count":captures.len(),
            "captures":captures,
            "physical_os_input":false
        });
        fs::write(
            capture_dir.join("acceptance.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}
