#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-navigation",
    feature = "mesh-animated-models",
    feature = "mesh-export",
    feature = "mesh-mission"
))]
#![recursion_limit = "256"]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, HostSpec, Mode,
};
use orr_navigation::{NavigationStatus, SearchBudget};
use orr_remote::mesh_navigation::{
    mesh_from_view,
    mission::{self, MissionView},
};
use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const STATIC_MODEL: &str = "foreground.glb";
const ANIMATED_MODEL: &str = "animated.glb";
const LICENSE: &str = "LICENSE.txt";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    workspace_source: PathBuf,
    hidden_workspace: PathBuf,
    workspace_hidden: bool,
    scene: PathBuf,
    duplicate_directory: PathBuf,
    duplicate_scene: PathBuf,
    static_source: PathBuf,
    hidden_static: PathBuf,
    animation_source: PathBuf,
    hidden_animation: PathBuf,
    captures: PathBuf,
    export_directory: PathBuf,
    runtime: PathBuf,
    static_model_sha256: String,
    static_license_sha256: String,
    animation_model_sha256: String,
    animation_license_sha256: String,
    sources_hidden: bool,
    static_hidden: bool,
    animation_hidden: bool,
    sources_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_WORKSPACE_COPY_ACCEPTANCE").as_deref(),
            Ok("1"),
            "set ORR_MESH_WORKSPACE_COPY_ACCEPTANCE=1 for GPU acceptance"
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
        let temporary = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_WORKSPACE_COPY_CAPTURE_DIR")
                .expect("set a fresh absolute capture directory"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture directory must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).expect("capture inventory directory must be fresh");

        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let workspace_source = root.join("source workspace");
        let hidden_workspace = root.join("source workspace.hidden");
        fs::create_dir(&workspace_source).unwrap();
        let workspace_source = workspace_source.canonicalize().unwrap();
        let requested_scene = PathBuf::from(
            std::env::var_os("ORR_MESH_WORKSPACE_COPY_SCENE").unwrap_or_else(|| {
                workspace_source
                    .join("authored-mesh.scene.yaml")
                    .into_os_string()
            }),
        );
        assert!(requested_scene.is_absolute() && !requested_scene.exists());
        let scene_parent = requested_scene
            .parent()
            .filter(|parent| parent.is_dir())
            .expect("scene parent must exist")
            .canonicalize()
            .unwrap();
        assert_eq!(
            scene_parent, workspace_source,
            "scene must stay in its owned source workspace"
        );
        let scene = scene_parent.join(requested_scene.file_name().unwrap());
        assert!(!scene.exists());

        let requested_destination = PathBuf::from(
            std::env::var_os("ORR_MESH_WORKSPACE_COPY_DESTINATION")
                .unwrap_or_else(|| root.join("duplicated workspace").into_os_string()),
        );
        assert!(requested_destination.is_absolute());
        let destination_parent = requested_destination
            .parent()
            .filter(|parent| parent.is_dir())
            .expect("duplicate workspace parent must exist")
            .canonicalize()
            .unwrap();
        let duplicate_directory =
            destination_parent.join(requested_destination.file_name().unwrap());
        assert!(
            !duplicate_directory.exists(),
            "duplicate destination must be absent"
        );
        assert!(
            !duplicate_directory.starts_with(&workspace_source),
            "duplicate destination must remain outside the source workspace"
        );
        let duplicate_scene = duplicate_directory.join(scene.file_name().unwrap());

        let static_source = root.join("local static package");
        let hidden_static = root.join("local static package.hidden");
        let animation_source = root.join("local animation package");
        let hidden_animation = root.join("local animation package.hidden");
        fs::create_dir(&static_source).unwrap();
        fs::create_dir(&animation_source).unwrap();
        let static_assets =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for filename in [
            "orr.package.json",
            STATIC_MODEL,
            "background.glb",
            LICENSE,
            "generate.py",
        ] {
            fs::copy(static_assets.join(filename), static_source.join(filename)).unwrap();
        }
        let animation_assets =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
        for filename in ["orr.package.json", ANIMATED_MODEL, LICENSE] {
            fs::copy(
                animation_assets.join(filename),
                animation_source.join(filename),
            )
            .unwrap();
        }

        let export_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_WORKSPACE_COPY_EXPORT_OUTPUT")
                .expect("set an absent mesh export output directory"),
        );
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("set the archived trusted mesh player path"),
        );
        assert!(export_directory.is_absolute() && !export_directory.exists());
        assert!(export_directory.parent().is_some_and(Path::is_dir));
        assert!(runtime.is_absolute() && runtime.is_file());

        let static_model_sha256 = sha256_hex(&fs::read(static_source.join(STATIC_MODEL)).unwrap());
        let static_license_sha256 = sha256_hex(&fs::read(static_source.join(LICENSE)).unwrap());
        let animation_model_sha256 =
            sha256_hex(&fs::read(animation_source.join(ANIMATED_MODEL)).unwrap());
        let animation_license_sha256 =
            sha256_hex(&fs::read(animation_source.join(LICENSE)).unwrap());

        Self {
            _temporary: temporary,
            root,
            physics_scene,
            physics_bytes,
            workspace_source,
            hidden_workspace,
            workspace_hidden: false,
            scene,
            duplicate_directory,
            duplicate_scene,
            static_source,
            hidden_static,
            animation_source,
            hidden_animation,
            captures,
            export_directory,
            runtime,
            static_model_sha256,
            static_license_sha256,
            animation_model_sha256,
            animation_license_sha256,
            sources_hidden: false,
            static_hidden: false,
            animation_hidden: false,
            sources_restored: false,
        }
    }

    fn harness(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::start(&HostSpec::local_game(
            self.physics_scene.clone(),
            EditorGame::PhysGame,
        ))
        .unwrap();
        editor.sync();
        Harness::builder()
            .with_size([1600.0, 1100.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |context| EditorApp::new(editor, context.wgpu_render_state.clone()))
    }

    fn hide_sources(&mut self) -> Result<(), String> {
        fs::rename(&self.workspace_source, &self.hidden_workspace)
            .map_err(|error| format!("hide source workspace: {error}"))?;
        self.workspace_hidden = true;
        fs::rename(&self.static_source, &self.hidden_static)
            .map_err(|error| format!("hide static package source: {error}"))?;
        self.static_hidden = true;
        fs::rename(&self.animation_source, &self.hidden_animation)
            .map_err(|error| format!("hide animation package source: {error}"))?;
        self.animation_hidden = true;
        self.sources_hidden = true;
        Ok(())
    }

    fn restore_sources(&mut self) -> Result<(), String> {
        if !self.sources_hidden {
            return Err("source workspace and both packages were not hidden".into());
        }
        if self.workspace_hidden {
            fs::rename(&self.hidden_workspace, &self.workspace_source)
                .map_err(|error| format!("restore source workspace: {error}"))?;
            self.workspace_hidden = false;
        }
        if self.static_hidden {
            fs::rename(&self.hidden_static, &self.static_source)
                .map_err(|error| format!("restore static source: {error}"))?;
            self.static_hidden = false;
        }
        if self.animation_hidden {
            fs::rename(&self.hidden_animation, &self.animation_source)
                .map_err(|error| format!("restore animation source: {error}"))?;
            self.animation_hidden = false;
        }
        let hashes = [
            sha256_hex(
                &fs::read(self.static_source.join(STATIC_MODEL))
                    .map_err(|error| error.to_string())?,
            ),
            sha256_hex(
                &fs::read(self.static_source.join(LICENSE)).map_err(|error| error.to_string())?,
            ),
            sha256_hex(
                &fs::read(self.animation_source.join(ANIMATED_MODEL))
                    .map_err(|error| error.to_string())?,
            ),
            sha256_hex(
                &fs::read(self.animation_source.join(LICENSE))
                    .map_err(|error| error.to_string())?,
            ),
        ];
        if hashes
            != [
                self.static_model_sha256.clone(),
                self.static_license_sha256.clone(),
                self.animation_model_sha256.clone(),
                self.animation_license_sha256.clone(),
            ]
        {
            return Err("restored package hashes differ from their originals".into());
        }
        self.sources_hidden = false;
        self.sources_restored = true;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.workspace_hidden
            && !self.workspace_source.exists()
            && self.hidden_workspace.exists()
        {
            if let Err(error) = fs::rename(&self.hidden_workspace, &self.workspace_source) {
                eprintln!("Could not restore hidden source workspace: {error}");
            }
        }
        if self.static_hidden && !self.static_source.exists() && self.hidden_static.exists() {
            if let Err(error) = fs::rename(&self.hidden_static, &self.static_source) {
                eprintln!("Could not restore hidden static package: {error}");
            }
        }
        if self.animation_hidden
            && !self.animation_source.exists()
            && self.hidden_animation.exists()
        {
            if let Err(error) = fs::rename(&self.hidden_animation, &self.animation_source) {
                eprintln!("Could not restore hidden animation package: {error}");
            }
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

fn assert_hardware_gpu() -> String {
    let gpu = orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default())
        .expect("workspace-copy acceptance requires an available GPU");
    assert!(
        !gpu.is_software(),
        "software rendering is not GPU acceptance evidence"
    );
    gpu.adapter_name().to_owned()
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

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, menu);
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    click(harness, item);
}

fn type_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
    settle(harness);
}

fn type_scene_path(harness: &mut Harness<'_, EditorApp>, path: &Path, action: &str) {
    type_field(harness, "Scene path", &path.display().to_string());
    click_button(harness, action);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {reason}");
}

fn wait_for_playing(harness: &mut Harness<'_, EditorApp>) {
    wait_for(harness, "MeshNavigation Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual.is_some_and(|actual| {
        actual == expected || actual.canonicalize().ok() == expected.canonicalize().ok()
    })
}

fn sidecar_path(scene: &Path, suffix: &str) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn viewport_pixels(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    label: &str,
) -> Vec<u8> {
    let target = (900, 850);
    for _ in 0..6 {
        settle(harness);
        let actual = harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .target()
            .size();
        if actual == target {
            break;
        }
        let window = harness.ctx.viewport_rect().size();
        let pixels_per_point = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / pixels_per_point,
            window.y + (target.1 as f32 - actual.1 as f32) / pixels_per_point,
        ));
    }
    settle(harness);
    harness
        .render()
        .expect("render production viewport before GPU readback");
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production viewport GPU target");
    let size = viewport.gpu().target().size();
    assert_eq!(size, target);
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&fixture.captures.join(format!("{label}.png")), &image).unwrap();
    pixels
}

fn preview_viewport_pixels(harness: &mut Harness<'_, EditorApp>) -> Vec<u8> {
    settle(harness);
    harness
        .render()
        .expect("render the current animation preview");
    let viewport = harness.state().viewport3d_gpu().unwrap();
    assert_eq!(viewport.gpu().target().size(), (900, 850));
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), 900 * 850 * 4);
    pixels
}

fn changed_pixel_count(first: &[u8], second: &[u8]) -> usize {
    assert_eq!(first.len(), second.len());
    first
        .as_chunks::<4>()
        .0
        .iter()
        .zip(second.as_chunks::<4>().0.iter())
        .filter(|(first_pixel, second_pixel)| {
            first_pixel[..3]
                .iter()
                .zip(second_pixel[..3].iter())
                .any(|(first_channel, second_channel)| first_channel.abs_diff(*second_channel) > 12)
        })
        .count()
}

fn within_raw_radius(
    position: [orr_fp::FP; 3],
    center: [orr_fp::FP; 3],
    radius: orr_fp::FP,
) -> bool {
    let distance_squared = position
        .iter()
        .zip(center)
        .map(|(position_coordinate, center_coordinate)| {
            let delta = i128::from(position_coordinate.raw()) - i128::from(center_coordinate.raw());
            delta * delta
        })
        .sum::<i128>();
    let radius_raw = i128::from(radius.raw());
    distance_squared <= radius_raw * radius_raw
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = harness.state().editor.snapshot().expect("mesh host Frame");
    mesh_from_view(snapshot.predicted()).expect("admitted mesh pin")
}

fn mission_view(harness: &Harness<'_, EditorApp>) -> MissionView {
    mission::from_view(
        harness
            .state()
            .editor
            .snapshot()
            .expect("mesh host Frame")
            .predicted(),
    )
    .expect("admitted mesh mission")
    .expect("enabled mesh mission")
}

fn composite_pixels(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    label: &str,
) -> Vec<u8> {
    let frame = harness
        .render()
        .expect("render complete production App frame");
    let size = [frame.width() as usize, frame.height() as usize];
    let pixels = frame.as_raw().clone();
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    orr_editor::app::write_png(&fixture.captures.join(format!("{label}.png")), &image).unwrap();
    pixels
}

fn rect_within(inner: egui::Rect, outer: egui::Rect) -> bool {
    inner.min.x >= outer.min.x
        && inner.min.y >= outer.min.y
        && inner.max.x <= outer.max.x
        && inner.max.y <= outer.max.y
}

fn assert_mesh_inspector_bounds(
    harness: &mut Harness<'_, EditorApp>,
    stage: &str,
) -> serde_json::Value {
    settle(harness);
    harness
        .render()
        .expect("render the settled full App before checking inspector bounds");
    let screen = harness.ctx.input(|input| input.viewport_rect());
    let inspector =
        egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
            .expect("production inspector panel state");
    let panels = ["top", "status", "bottom", "hierarchy", "inspector"].map(|label| {
        (
            label,
            egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new(label))
                .map(|panel| panel.outer_rect),
        )
    });
    let controls = [
        "Capture viewport as game camera",
        "Apply game camera",
        "Undo game camera",
        "Redo game camera",
        "Reset viewport to game camera",
        "Save game camera",
        "Reload game camera",
        "Undo game lighting",
        "Redo game lighting",
        "Save game lighting",
        "Reload game lighting",
        "I confirm removing the saved game lighting",
        "Save game HUD",
        "Reload game HUD",
    ]
    .map(|label| (label, harness.query_by_label(label).map(|node| node.rect())));
    assert!(
        rect_within(inspector.outer_rect, screen),
        "Mesh inspector extends beyond the App viewport at {stage}: inspector={:?}, screen={screen:?}, panels={panels:?}, controls={controls:?}",
        inspector.outer_rect
    );

    let mut fields = serde_json::Map::new();
    for label in ["Lighting direction X", "HUD title"] {
        if harness.query_by_label(label).is_some() {
            let field = harness.get_by_role_and_label(Role::TextInput, label).rect();
            if field.intersects(screen) {
                assert!(
                    field.min.x >= screen.min.x
                        && field.max.x <= screen.max.x
                        && field.min.x >= inspector.outer_rect.min.x
                        && field.max.x <= inspector.outer_rect.max.x,
                    "Visible {label} field escapes the inspector or App viewport at {stage}: field={field:?}, inspector={:?}, screen={screen:?}",
                    inspector.outer_rect
                );
                fields.insert(
                    label.to_owned(),
                    serde_json::json!([field.min.x, field.min.y, field.max.x, field.max.y]),
                );
            }
        }
    }

    serde_json::json!({
        "stage": stage,
        "screen_bounds": [screen.min.x, screen.min.y, screen.max.x, screen.max.y],
        "inspector_bounds": [inspector.outer_rect.min.x, inspector.outer_rect.min.y, inspector.outer_rect.max.x, inspector.outer_rect.max.y],
        "visible_fields": fields,
    })
}

fn skeletal_pose_difference(document: &serde_json::Value, tick: u64) -> usize {
    let source: orr_model::animation::AnimatedSource =
        serde_json::from_value(document["model"].clone()).unwrap();
    let model = orr_model::animation::AnimatedModel::new(source).unwrap();
    let settings = &document["settings"];
    let clip = settings["clip"].as_u64().unwrap() as u32;
    let rate = settings["rate"].as_f64().unwrap() as f32;
    let duration = model.source().clips[clip as usize].duration();
    let sample_time = if duration > 0.0 {
        ((tick as f32 / 60.0) * rate) % duration
    } else {
        0.0
    };
    let initial = model.sample_clip(clip, 0.0).unwrap();
    let sampled = model.sample_clip(clip, sample_time).unwrap();
    initial
        .global()
        .iter()
        .flatten()
        .flatten()
        .zip(sampled.global().iter().flatten().flatten())
        .filter(|(left, right)| (**left - **right).abs() > 0.00001)
        .count()
        + initial
            .skin_matrices()
            .iter()
            .flatten()
            .flatten()
            .flatten()
            .zip(sampled.skin_matrices().iter().flatten().flatten().flatten())
            .filter(|(left, right)| (**left - **right).abs() > 0.00001)
            .count()
}

fn capture_inventory(fixture: &Fixture) -> Vec<serde_json::Value> {
    let mut captures = fs::read_dir(&fixture.captures)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            serde_json::json!({
                "name": path.file_name().unwrap().to_string_lossy(),
                "bytes": bytes.len(),
                "sha256": sha256_hex(&bytes),
            })
        })
        .collect::<Vec<_>>();
    captures.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    captures
}

fn export_if_requested(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
) -> serde_json::Value {
    menu_click(harness, "File", "Export mesh game…");
    type_field(
        harness,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        harness,
        "Mesh game output directory",
        &fixture.export_directory.display().to_string(),
    );
    click(harness, "I trust this local mesh player executable");
    click_button(harness, "Export mesh game");
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if harness
            .query_by_label_contains("Mesh export complete:")
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        harness
            .query_by_label_contains("Mesh export complete:")
            .is_some(),
        "mesh workspace export did not complete"
    );

    let scene_name = fixture
        .scene
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let expected_files = BTreeSet::from([
        scene_name.clone(),
        format!("{scene_name}.mesh-art.json"),
        format!("{scene_name}.mesh-props.json"),
        format!("{scene_name}.mesh-camera.json"),
        format!("{scene_name}.mesh-animation.json"),
        format!("{scene_name}.mesh-lighting.json"),
        format!("{scene_name}.mesh-hud.json"),
        "orr.mesh-bundle.json".to_owned(),
        "orr_mesh_player.exe".to_owned(),
    ]);
    let export_entries = fs::read_dir(&fixture.export_directory)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(export_entries.len(), 9);
    assert!(export_entries
        .iter()
        .all(|entry| entry.file_type().unwrap().is_file()));
    let actual_files = export_entries
        .into_iter()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_files, expected_files);
    let manifest_bytes = fs::read(fixture.export_directory.join("orr.mesh-bundle.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["scene"]["name"], scene_name);
    let exported_scene_bytes = fs::read(fixture.export_directory.join(&scene_name)).unwrap();
    assert_eq!(
        manifest["scene"]["bytes"],
        exported_scene_bytes.len() as u64
    );
    assert_eq!(
        manifest["scene"]["sha256"],
        sha256_hex(&exported_scene_bytes)
    );
    assert_eq!(manifest["checksum"], harness.state().editor.checksum());
    let graph_revision = mesh_view(harness).graph.revision();
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(graph_revision)
    );
    let graph_revision_hex = graph_revision
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert!(manifest["sidecar"].is_object());
    assert!(manifest["props"].is_object());
    assert!(manifest["camera"].is_object());
    assert!(manifest["animation"].is_object());
    assert!(manifest["lighting"].is_object());
    assert!(manifest["hud"].is_object());
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(
        exported_scene_bytes,
        fs::read(&fixture.duplicate_scene).unwrap()
    );
    for (manifest_key, suffix) in [
        ("sidecar", ".mesh-art.json"),
        ("props", ".mesh-props.json"),
        ("camera", ".mesh-camera.json"),
        ("animation", ".mesh-animation.json"),
        ("lighting", ".mesh-lighting.json"),
        ("hud", ".mesh-hud.json"),
    ] {
        let exported_path = fixture
            .export_directory
            .join(format!("{scene_name}{suffix}"));
        let duplicate_path = sidecar_path(&fixture.duplicate_scene, suffix);
        let exported_bytes = fs::read(&exported_path).unwrap();
        let manifest_record = &manifest[manifest_key];
        assert_eq!(manifest_record["name"], format!("{scene_name}{suffix}"));
        assert_eq!(manifest_record["bytes"], exported_bytes.len() as u64);
        assert_eq!(manifest_record["sha256"], sha256_hex(&exported_bytes));
        assert_eq!(exported_bytes, fs::read(duplicate_path).unwrap());
    }
    let runtime_bytes = fs::read(fixture.export_directory.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(manifest["runtime"]["name"], "orr_mesh_player.exe");
    assert_eq!(manifest["runtime"]["bytes"], runtime_bytes.len() as u64);
    assert_eq!(manifest["runtime"]["sha256"], sha256_hex(&runtime_bytes));
    serde_json::json!({
        "directory": fixture.export_directory,
        "exact_nine_file_bundle": true,
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "runtime_sha256": sha256_hex(&runtime_bytes),
        "scene_checksum": format!("0x{:016x}", harness.state().editor.checksum()),
        "graph_revision": graph_revision_hex.clone(),
        "game_profile": manifest["game_profile"],
        "input_profile": manifest["input_profile"],
        "files": actual_files,
    })
}

#[test]
#[ignore = "requires ORR_MESH_WORKSPACE_COPY_ACCEPTANCE=1 and hardware Vulkan"]
fn duplicate_mesh_workspace_preserves_all_saved_presentations_source_free() {
    let adapter = assert_hardware_gpu();
    let mut fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);

    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "new MeshNavigation scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    click(&mut harness, "Static-mesh navigation");
    if harness.query_by_label("Quad origin X").is_none() {
        click(&mut harness, "Mesh geometry");
    }
    for (label, value) in [
        ("Maximum slope", "4"),
        ("Quad origin X", "0"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "0"),
        ("Quad width", "4"),
        ("Quad depth", "4"),
        ("Quad rise", "0"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Stage walkable quad");
    for (label, value) in [
        ("Quad origin X", "0"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "4"),
        ("Quad width", "4"),
        ("Quad depth", "4"),
        ("Quad rise", "2"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Stage walkable quad");
    for (label, value) in [
        ("Start X", "1"),
        ("Start Y", "0"),
        ("Start Z", "1"),
        ("Goal X", "3"),
        ("Goal Y", "1.5"),
        ("Goal Z", "7"),
        ("Maximum vertical projection", "2"),
        ("Distance per tick", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Place mesh agent");
    let authored_view = mesh_view(&harness);
    assert_eq!(authored_view.graph.triangles().len(), 4);
    assert_eq!(authored_view.agent.status, NavigationStatus::Moving);
    if harness
        .query_by_label("Mission collectible count")
        .is_none()
    {
        click(&mut harness, "Mesh mission");
    }
    click(&mut harness, "Enable mesh mission");
    type_field(&mut harness, "Mission collectible count", "1");
    for (label, value) in [
        ("Mission exit X", "3"),
        ("Mission exit Y", "1.5"),
        ("Mission exit Z", "7"),
        ("Mission exit radius", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Apply mesh mission");
    assert_eq!(mission_view(&harness).run.collected, 0);
    assert_eq!(mission_view(&harness).run.won, 0);
    let initial_mission_config = mission_view(&harness).config;
    let before_checkpoint_checksum = harness.state().editor.checksum();
    let before_checkpoint_history = harness.state().editor.history().entries.len();
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Stage mission route checkpoints")
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Stage mission route checkpoints");
    assert_eq!(
        harness.state().editor.checksum(),
        before_checkpoint_checksum
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_checkpoint_history
    );
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Apply mesh mission")
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Apply mesh mission");
    let authored_mission = mission_view(&harness);
    assert_ne!(
        authored_mission.config.points[0].position,
        initial_mission_config.points[0].position
    );
    let canonical_route = authored_view
        .graph
        .find_path(
            authored_view.pin.spec().start,
            authored_view.pin.spec().goal,
            authored_view.pin.max_vertical,
            SearchBudget {
                max_expansions: authored_view.graph.triangles().len() as u32,
            },
        )
        .unwrap();
    assert!(canonical_route.points[1..canonical_route.points.len() - 1]
        .contains(&authored_mission.config.points[0].position));
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_checkpoint_history + 1
    );
    let applied_checkpoint_checksum = harness.state().editor.checksum();
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    wait_for(&mut harness, "checkpoint mission undo", |app| {
        app.editor.checksum() == before_checkpoint_checksum
    });
    assert_eq!(mission_view(&harness).config, initial_mission_config);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    wait_for(&mut harness, "checkpoint mission redo", |app| {
        app.editor.checksum() == applied_checkpoint_checksum
    });
    assert_eq!(mission_view(&harness).config, authored_mission.config);
    assert_eq!(authored_mission.config.enabled, 1);
    assert_eq!(authored_mission.config.count, 1);
    assert_eq!(authored_mission.run.collected, 0);
    assert_eq!(authored_mission.run.won, 0);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    let graph_revision = mesh_view(&harness).graph.revision();
    let baseline_pixels = viewport_pixels(&mut harness, &fixture, "01-baseline");

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    for (label, value) in [
        (
            "Art package source directory",
            fixture.static_source.display().to_string(),
        ),
        ("Art model asset", STATIC_MODEL.to_owned()),
        ("Art license asset", LICENSE.to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use this agent art");
    click_button(&mut harness, "Load agent art");
    type_field(&mut harness, "Agent art offset X", "0.25");
    type_field(&mut harness, "Agent art offset Y", "0.5");
    type_field(&mut harness, "Agent art offset Z", "0.25");
    type_field(&mut harness, "Agent art scale", "0.5");
    click_button(&mut harness, "Apply agent art");
    click_button(&mut harness, "Save agent art");
    let art_sidecar = sidecar_path(&fixture.scene, ".mesh-art.json");
    let art_bytes = fs::read(&art_sidecar).expect("agent art sidecar was explicitly saved");
    let art_pixels = viewport_pixels(&mut harness, &fixture, "02-agent-art");
    assert!(changed_pixel_count(&baseline_pixels, &art_pixels) > 20);

    if harness
        .query_by_label("Prop package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh props");
    }
    for (label, value) in [
        (
            "Prop package source directory",
            fixture.static_source.display().to_string(),
        ),
        ("Prop model asset", STATIC_MODEL.to_owned()),
        ("Prop license asset", LICENSE.to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use these props");
    click_button(&mut harness, "Load prop candidate");
    for (label, value) in [
        ("Prop name", "Copy-contract prop"),
        ("Prop position X", "3"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "1"),
        ("Prop scale", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Add prop");
    click_button(&mut harness, "Save props");
    let props_sidecar = sidecar_path(&fixture.scene, ".mesh-props.json");
    let props_bytes = fs::read(&props_sidecar).expect("props sidecar was explicitly saved");
    let props_pixels = viewport_pixels(&mut harness, &fixture, "03-agent-art-and-prop");
    assert!(changed_pixel_count(&art_pixels, &props_pixels) > 20);

    if harness.query_by_label("Game camera target X").is_none() {
        click(&mut harness, "Mesh game camera");
    }
    click_button(&mut harness, "Capture viewport as game camera");
    for (label, value) in [
        ("Game camera target X", "2"),
        ("Game camera target Y", "1"),
        ("Game camera target Z", "4"),
        ("Game camera yaw (degrees)", "45"),
        ("Game camera pitch (degrees)", "35"),
        ("Game camera distance", "9"),
        ("Game camera field of view (degrees)", "40"),
        ("Game camera near plane", "0.05"),
        ("Game camera far plane", "200"),
        ("Game camera orthographic half height", "5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click(&mut harness, "Orthographic game camera");
    click_button(&mut harness, "Apply game camera");
    click_button(&mut harness, "Save game camera");
    let camera_sidecar = sidecar_path(&fixture.scene, ".mesh-camera.json");
    let camera_bytes = fs::read(&camera_sidecar).expect("camera sidecar was explicitly saved");
    let camera_document: serde_json::Value = serde_json::from_slice(&camera_bytes).unwrap();
    assert!(camera_document["camera"].is_object());
    let camera_pixels = viewport_pixels(&mut harness, &fixture, "04-camera");
    assert!(changed_pixel_count(&props_pixels, &camera_pixels) > 20);

    if harness
        .query_by_label("Animation package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent animation");
    }
    for (label, value) in [
        (
            "Animation package source directory",
            fixture.animation_source.display().to_string(),
        ),
        ("Animation model asset", ANIMATED_MODEL.to_owned()),
        ("Animation license asset", LICENSE.to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use this animation");
    click_button(&mut harness, "Load mesh animation");
    type_field(&mut harness, "Animation offset X", "0.25");
    type_field(&mut harness, "Animation offset Y", "0.5");
    type_field(&mut harness, "Animation offset Z", "0.25");
    type_field(&mut harness, "Animation scale", "0.5");
    type_field(&mut harness, "Animation rotation X (degrees)", "15");
    type_field(&mut harness, "Animation clip index", "1");
    type_field(&mut harness, "Animation playback rate", "1.5");
    click_button(&mut harness, "Apply mesh animation");
    click_button(&mut harness, "Save mesh animation");
    let animation_sidecar = sidecar_path(&fixture.scene, ".mesh-animation.json");
    let animation_bytes =
        fs::read(&animation_sidecar).expect("animation sidecar was explicitly saved");
    let animation_document: serde_json::Value = serde_json::from_slice(&animation_bytes).unwrap();
    assert_eq!(animation_document["asset"], ANIMATED_MODEL);
    assert_eq!(
        animation_document["asset_sha256"],
        fixture.animation_model_sha256
    );
    assert_eq!(
        animation_document["license_sha256"],
        fixture.animation_license_sha256
    );
    assert!(skeletal_pose_difference(&animation_document, 16) > 0);

    let history_before_animation_preview = harness.state().editor.history().clone();
    let scene_before_animation_preview = fs::read(&fixture.scene).unwrap();
    let authored_tick_before_preview = harness
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .tick();
    assert!(!harness.state().editor.is_dirty());
    type_field(&mut harness, "Animation preview clip", "1");
    type_field(&mut harness, "Animation preview tick", "0");
    click_button(&mut harness, "Preview animation pose");
    let preview_zero_pixels =
        viewport_pixels(&mut harness, &fixture, "17-animation-preview-tick-zero");
    type_field(&mut harness, "Animation preview tick", "16");
    click_button(&mut harness, "Preview animation pose");
    let preview_sixteen_pixels =
        viewport_pixels(&mut harness, &fixture, "18-animation-preview-tick-sixteen");
    let animation_preview_pixel_difference =
        changed_pixel_count(&preview_zero_pixels, &preview_sixteen_pixels);
    assert!(animation_preview_pixel_difference > 20);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(
        harness.state().editor.history(),
        &history_before_animation_preview
    );
    assert_eq!(
        harness
            .state()
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .tick(),
        authored_tick_before_preview
    );
    assert!(!harness.state().editor.is_dirty());
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        scene_before_animation_preview
    );
    assert_eq!(fs::read(&animation_sidecar).unwrap(), animation_bytes);
    assert!(!harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled());

    type_field(&mut harness, "Animation preview tick", "-1");
    click_button(&mut harness, "Preview animation pose");
    assert_eq!(
        preview_viewport_pixels(&mut harness),
        preview_sixteen_pixels
    );
    click_button(&mut harness, "Clear animation preview");
    assert_eq!(preview_viewport_pixels(&mut harness), preview_zero_pixels);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(
        harness.state().editor.history(),
        &history_before_animation_preview
    );
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        scene_before_animation_preview
    );
    assert_eq!(fs::read(&animation_sidecar).unwrap(), animation_bytes);
    let animation_preview_receipt = serde_json::json!({
        "applied_clip": 1,
        "preview_ticks": [0, 16],
        "pose_pixel_difference": animation_preview_pixel_difference,
        "simulation_checksum_history_and_files_unchanged": true,
        "invalid_tick_preserved_accepted_pose": true,
        "clear_restored_authored_pose": true,
        "transient_preview_did_not_block_play": true,
    });

    if harness.query_by_label("Lighting direction X").is_none() {
        click(&mut harness, "Mesh game lighting");
    }
    for (label, value) in [
        ("Lighting direction X", "-0.7"),
        ("Lighting direction Y", "-0.45"),
        ("Lighting direction Z", "-0.2"),
        ("Light color R", "0.35"),
        ("Light color G", "0.75"),
        ("Light color B", "0.95"),
        ("Light intensity", "1.35"),
        ("Ambient", "0.18"),
        ("Shadow center X", "1"),
        ("Shadow center Y", "0.5"),
        ("Shadow center Z", "2"),
        ("Shadow radius", "12"),
        ("Exposure", "1.4"),
    ] {
        type_field(&mut harness, label, value);
    }
    click(&mut harness, "Enable shadows");
    click(&mut harness, "Enable tonemapping");
    click_button(&mut harness, "Apply game lighting");
    click_button(&mut harness, "Save game lighting");
    let lighting_sidecar = sidecar_path(&fixture.scene, ".mesh-lighting.json");
    let lighting_bytes =
        fs::read(&lighting_sidecar).expect("game lighting sidecar was explicitly saved");
    let lighting_document: serde_json::Value = serde_json::from_slice(&lighting_bytes).unwrap();
    assert_eq!(lighting_document["identity"], "ORR_MESH_LIGHTING_V1");
    assert_eq!(lighting_document["lighting"]["tonemap"], false);
    assert_eq!(lighting_document["lighting"]["shadows"], false);

    if harness.query_by_label("HUD title").is_none() {
        click(&mut harness, "Mesh game HUD");
    }
    click_button(&mut harness, LBL_PLAY);
    wait_for_playing(&mut harness);
    click_button(&mut harness, LBL_PAUSE);
    harness.state_mut().editor.seek(3);
    wait_for(&mut harness, "no-HUD comparison tick", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 3 && !timeline.playing)
    });
    let lighting_inspector_layout = assert_mesh_inspector_bounds(&mut harness, "lighting-expanded");
    let no_hud_composite = composite_pixels(&mut harness, &fixture, "10-no-hud-play-tick-three");
    click_button(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    type_field(&mut harness, "HUD title", "THE COPIED MISSION");
    type_field(&mut harness, "HUD idle text", "Choose a destination");
    type_field(
        &mut harness,
        "HUD moving text",
        "Crossing the authored route",
    );
    type_field(&mut harness, "HUD arrived text", "Route endpoint reached");
    type_field(&mut harness, "HUD font size", "27");
    click_button(&mut harness, "Apply game HUD");
    click_button(&mut harness, "Save game HUD");
    let hud_sidecar = sidecar_path(&fixture.scene, ".mesh-hud.json");
    let hud_bytes = fs::read(&hud_sidecar).expect("game HUD sidecar was explicitly saved");
    let hud_document: serde_json::Value = serde_json::from_slice(&hud_bytes).unwrap();
    assert_eq!(hud_document["marker"], "ORR_MESH_HUD_V1");
    assert_eq!(hud_document["hud"]["title"], "THE COPIED MISSION");
    assert_eq!(
        hud_document["hud"]["moving_text"],
        "Crossing the authored route"
    );
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    click_button(&mut harness, LBL_PLAY);
    wait_for_playing(&mut harness);
    click_button(&mut harness, LBL_PAUSE);
    harness.state_mut().editor.seek(3);
    wait_for(&mut harness, "HUD comparison tick", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 3 && !timeline.playing)
    });
    let hud_inspector_layout =
        assert_mesh_inspector_bounds(&mut harness, "lighting-and-hud-expanded");
    let hud_composite = composite_pixels(&mut harness, &fixture, "11-hud-play-tick-three");
    assert!(changed_pixel_count(&no_hud_composite, &hud_composite) > 40);
    assert!(harness.query_by_label("THE COPIED MISSION").is_some());
    assert!(harness
        .query_by_label("Crossing the authored route")
        .is_some());
    click_button(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    type_field(&mut harness, "Light intensity", "1.55");
    let workspace_play_blocked_before_save = harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled();
    assert!(workspace_play_blocked_before_save);
    click(&mut harness, "File");
    assert!(harness
        .get_by_role_and_label(Role::Button, "Save mesh workspace")
        .accesskit_node()
        .is_disabled());
    for path_and_bytes in [
        (&art_sidecar, &art_bytes),
        (&props_sidecar, &props_bytes),
        (&camera_sidecar, &camera_bytes),
        (&animation_sidecar, &animation_bytes),
        (&lighting_sidecar, &lighting_bytes),
        (&hud_sidecar, &hud_bytes),
    ] {
        let observed_bytes = fs::read(path_and_bytes.0).unwrap();
        assert_eq!(observed_bytes.as_slice(), path_and_bytes.1.as_slice());
    }
    harness.key_press(Key::Escape);
    settle(&mut harness);

    let history_before_workspace_save = harness.state().editor.history().clone();
    click_button(&mut harness, "Apply game lighting");
    type_field(
        &mut harness,
        "HUD moving text",
        "SAVED MESH WORKSPACE ROUTE",
    );
    click_button(&mut harness, "Apply game HUD");
    type_field(&mut harness, "Game camera near plane", "0.125");
    click_button(&mut harness, "Apply game camera");
    type_field(&mut harness, "Animation playback rate", "1.25");
    click_button(&mut harness, "Apply mesh animation");
    type_field(&mut harness, "Prop name", "Workspace saved prop");
    click_button(&mut harness, "Update selected prop");
    type_field(&mut harness, "Agent art offset X", "0.375");
    click_button(&mut harness, "Apply agent art");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(
        harness.state().editor.history(),
        &history_before_workspace_save
    );

    click(&mut harness, "File");
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Save mesh workspace")
        .accesskit_node()
        .is_disabled());
    harness.key_press(Key::Escape);
    settle(&mut harness);
    menu_click(&mut harness, "File", "Save mesh workspace");
    let saved_art_bytes = fs::read(&art_sidecar).unwrap();
    let saved_props_bytes = fs::read(&props_sidecar).unwrap();
    let saved_camera_bytes = fs::read(&camera_sidecar).unwrap();
    let saved_animation_bytes = fs::read(&animation_sidecar).unwrap();
    let saved_lighting_bytes = fs::read(&lighting_sidecar).unwrap();
    let saved_hud_bytes = fs::read(&hud_sidecar).unwrap();
    for (old_bytes, new_bytes) in [
        (&art_bytes, &saved_art_bytes),
        (&props_bytes, &saved_props_bytes),
        (&camera_bytes, &saved_camera_bytes),
        (&animation_bytes, &saved_animation_bytes),
        (&lighting_bytes, &saved_lighting_bytes),
        (&hud_bytes, &saved_hud_bytes),
    ] {
        assert_ne!(sha256_hex(old_bytes), sha256_hex(new_bytes));
    }
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(
        harness.state().editor.history(),
        &history_before_workspace_save
    );
    let workspace_play_enabled_after_save = !harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled();
    assert!(workspace_play_enabled_after_save);
    let workspace_save_receipt = serde_json::json!({
        "unapplied_form_disabled_save": true,
        "unapplied_form_left_all_six_sidecars_unchanged": true,
        "six_applied_panels_saved_in_one_file_action": true,
        "all_six_sidecar_sha256_values_changed": true,
        "graph_checksum_and_history_preserved": true,
        "play_blocked_before_save": workspace_play_blocked_before_save,
        "play_enabled_after_save": workspace_play_enabled_after_save,
    });
    let art_bytes = saved_art_bytes;
    let props_bytes = saved_props_bytes;
    let camera_bytes = saved_camera_bytes;
    let camera_sidecar_sha256 = sha256_hex(&camera_bytes);
    let camera_document: serde_json::Value = serde_json::from_slice(&camera_bytes).unwrap();
    let animation_bytes = saved_animation_bytes;
    let animation_document: serde_json::Value = serde_json::from_slice(&animation_bytes).unwrap();
    let lighting_bytes = saved_lighting_bytes;
    let hud_bytes = saved_hud_bytes;

    let all_presentations_pixels = viewport_pixels(&mut harness, &fixture, "05-all-presentations");
    assert!(changed_pixel_count(&camera_pixels, &all_presentations_pixels) > 20);
    let all_sidecars = [
        (art_sidecar, art_bytes),
        (props_sidecar, props_bytes),
        (camera_sidecar, camera_bytes),
        (animation_sidecar, animation_bytes),
        (lighting_sidecar, lighting_bytes),
        (hud_sidecar, hud_bytes),
    ];
    let final_original_pixels = all_presentations_pixels;
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);

    menu_click(&mut harness, "File", "Duplicate mesh workspace…");
    type_field(
        &mut harness,
        "Mesh workspace destination directory",
        &fixture.duplicate_directory.display().to_string(),
    );
    click_button(&mut harness, "Duplicate mesh workspace");
    wait_for(&mut harness, "copied MeshNavigation workspace", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.duplicate_scene)
    });
    let duplicate_scene_bytes =
        fs::read(&fixture.duplicate_scene).expect("duplicate scene published");
    assert_eq!(duplicate_scene_bytes, scene_bytes);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    let mut copied_sidecars = Vec::new();
    for (original_path, original_bytes) in &all_sidecars {
        let source_scene_text = fixture.scene.to_string_lossy();
        let original_path_text = original_path.to_string_lossy();
        let suffix = original_path_text
            .strip_prefix(source_scene_text.as_ref())
            .expect("source sidecar path appends to the original scene path");
        let duplicate_path = sidecar_path(&fixture.duplicate_scene, suffix);
        let duplicate_bytes = fs::read(&duplicate_path).expect("duplicate sidecar published");
        assert_eq!(&duplicate_bytes, original_bytes);
        assert_eq!(fs::read(original_path).unwrap(), *original_bytes);
        copied_sidecars.push((duplicate_path, duplicate_bytes));
    }
    let duplicate_pixels = viewport_pixels(&mut harness, &fixture, "06-duplicate-current-app");
    assert!(changed_pixel_count(&final_original_pixels, &duplicate_pixels) < 20);
    drop(harness);

    fixture
        .hide_sources()
        .expect("hide only the owned local package roots");
    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.duplicate_scene, "OK");
    wait_for(
        &mut reopened,
        "fresh Physics open of copied workspace",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && path_matches(app.editor.path().as_deref(), &fixture.duplicate_scene)
                && app.editor.checksum() == scene_checksum
        },
    );
    assert!(!fixture.static_source.exists() && !fixture.animation_source.exists());
    assert_eq!(fs::read(&fixture.duplicate_scene).unwrap(), scene_bytes);
    for (duplicate_path, duplicate_bytes) in &copied_sidecars {
        assert_eq!(fs::read(duplicate_path).unwrap(), *duplicate_bytes);
    }
    let reopened_mission = mission_view(&reopened);
    assert_eq!(reopened_mission.config, authored_mission.config);
    assert_eq!(reopened_mission.run.collected, 0);
    assert_eq!(reopened_mission.run.won, 0);
    if reopened
        .query_by_label("Mission collectible count")
        .is_none()
    {
        click(&mut reopened, "Mesh mission");
    }
    let reopened_pixels = viewport_pixels(&mut reopened, &fixture, "07-source-hidden-reopen");
    assert!(changed_pixel_count(&final_original_pixels, &reopened_pixels) < 20);
    let reopened_composite =
        composite_pixels(&mut reopened, &fixture, "12-source-hidden-reopen-app");
    assert!(!reopened_composite.is_empty());

    click_button(&mut reopened, LBL_PLAY);
    wait_for_playing(&mut reopened);
    click_button(&mut reopened, LBL_PAUSE);
    click_button(&mut reopened, "⏮");
    wait_for(&mut reopened, "tick-zero paused frame", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
    });
    let play_start = mesh_view(&reopened).agent.position;
    let tick_zero_pixels = viewport_pixels(&mut reopened, &fixture, "08-play-tick-zero");
    for _ in 0..16 {
        click_button(&mut reopened, LBL_STEP);
    }
    let timeline_tick = reopened.state().editor.timeline().unwrap().tick;
    assert_eq!(timeline_tick, 16);
    let played_view = mesh_view(&reopened);
    assert_ne!(played_view.agent.position, play_start);
    let path_length = played_view
        .graph
        .path_length(&played_view.agent.path)
        .unwrap();
    let expected_distance = orr_fp::FP::from_raw(
        path_length
            .raw()
            .min(16 * played_view.agent.spec.distance_per_tick.raw()),
    );
    assert_eq!(played_view.agent.distance, expected_distance);
    assert_eq!(
        played_view.agent.status,
        if expected_distance == path_length {
            NavigationStatus::Arrived
        } else {
            NavigationStatus::Moving
        }
    );
    if played_view.agent.status == NavigationStatus::Arrived {
        assert_eq!(
            played_view.agent.position,
            *played_view.agent.path.points.last().unwrap()
        );
    }
    let play_pixels = viewport_pixels(&mut reopened, &fixture, "09-play-tick-sixteen");
    assert!(changed_pixel_count(&tick_zero_pixels, &play_pixels) > 10);
    let mission_mask = (1u32 << mission_view(&reopened).config.count) - 1;
    let mut mission_won = mission_view(&reopened).run.won == 1;
    while !mission_won && reopened.state().editor.timeline().unwrap().tick < 128 {
        click_button(&mut reopened, LBL_STEP);
        mission_won = mission_view(&reopened).run.won == 1;
    }
    let won_view = mission_view(&reopened);
    assert_eq!(won_view.run.collected, mission_mask);
    assert_eq!(
        won_view.run.won, 1,
        "all route checkpoints and the exit must be reached by simulation"
    );
    let won_agent = mesh_view(&reopened).agent;
    assert!(within_raw_radius(
        won_agent.position,
        won_view.config.exit.position,
        won_view.config.exit.radius
    ));
    assert!(reopened.query_by_label("Mission complete").is_some());
    assert!(reopened.query_by_label("Mission collected: 1/1").is_some());
    assert!(reopened.query_by_label("THE COPIED MISSION").is_some());
    let win_tick = reopened.state().editor.timeline().unwrap().tick;
    let win_frame_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let win_agent_status = format!("{:?}", won_agent.status);
    let expected_hud_status = match won_agent.status {
        NavigationStatus::Stopped => "Choose a destination",
        NavigationStatus::Moving => "SAVED MESH WORKSPACE ROUTE",
        NavigationStatus::Arrived => "Route endpoint reached",
    };
    assert!(reopened.query_by_label(expected_hud_status).is_some());
    let win_pixels = composite_pixels(&mut reopened, &fixture, "13-mission-win-full-app");
    let win_viewport_pixels = viewport_pixels(&mut reopened, &fixture, "15-mission-win-viewport");
    assert!(changed_pixel_count(&tick_zero_pixels, &play_pixels) > 10);

    reopened.state_mut().editor.seek(0);
    wait_for(&mut reopened, "mission seek to tick zero", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
            && app
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.predicted().tick() == 0)
    });
    let reset_mission = mission_view(&reopened);
    assert_eq!(reset_mission.run.collected, 0);
    assert_eq!(reset_mission.run.won, 0);
    reopened.state_mut().editor.seek(win_tick);
    wait_for(&mut reopened, "mission replay to winning tick", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == win_tick && !timeline.playing)
            && app
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.predicted().tick() == win_tick)
    });
    let replay_mission = mission_view(&reopened);
    assert_eq!(replay_mission.run, won_view.run);
    assert_eq!(
        reopened
            .state()
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .checksum(),
        win_frame_checksum
    );
    let replay_pixels = composite_pixels(&mut reopened, &fixture, "14-mission-win-replay-full-app");
    let replay_viewport_pixels =
        viewport_pixels(&mut reopened, &fixture, "16-mission-replay-viewport");
    let replay_frame_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let mission_replay_full_frame_matches = replay_frame_checksum == win_frame_checksum
        && replay_mission.run == won_view.run
        && reopened.state().editor.timeline().unwrap().tick == win_tick;
    assert!(mission_replay_full_frame_matches);
    let mission_replay_viewport_pixel_diff =
        changed_pixel_count(&win_viewport_pixels, &replay_viewport_pixels);
    assert!(mission_replay_viewport_pixel_diff < 20);
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), scene_checksum);
    assert_eq!(mission_view(&reopened).run.collected, 0);
    assert_eq!(mission_view(&reopened).run.won, 0);

    let export_receipt = export_if_requested(&mut reopened, &fixture);
    let duplicate_hash = sha256_hex(&duplicate_scene_bytes);
    let sidecar_receipt = copied_sidecars
        .iter()
        .map(|(path, bytes)| {
            serde_json::json!({
                "path": path,
                "bytes": bytes.len(),
                "sha256": sha256_hex(bytes),
            })
        })
        .collect::<Vec<_>>();
    let graph_revision_hex = graph_revision
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mission_config_receipt = serde_json::json!({
        "enabled": authored_mission.config.enabled,
        "count": authored_mission.config.count,
        "points": authored_mission.config.points[..authored_mission.config.count as usize]
            .iter()
            .map(|point| serde_json::json!({
                "position_raw": point.position.map(|coordinate| coordinate.raw()),
                "radius_raw": point.radius.raw(),
            }))
            .collect::<Vec<_>>(),
        "exit": {
            "position_raw": authored_mission.config.exit.position.map(|coordinate| coordinate.raw()),
            "radius_raw": authored_mission.config.exit.radius.raw(),
        },
        "graph_revision": graph_revision_hex.clone(),
    });
    let sources_hidden_during_fresh_open = fixture.sources_hidden;
    fixture
        .restore_sources()
        .expect("restore and hash-check both source packages");
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    for (original_path, original_bytes) in &all_sidecars {
        assert_eq!(fs::read(original_path).unwrap(), *original_bytes);
    }
    let captures = capture_inventory(&fixture);
    assert_eq!(captures.len(), 18);
    let receipt = serde_json::json!({
        "artifact_root": fixture.root,
        "hardware_vulkan_adapter": adapter,
        "source_scene": fixture.scene,
        "source_scene_sha256": sha256_hex(&scene_bytes),
        "source_scene_checksum": format!("0x{scene_checksum:016x}"),
        "source_workspace": fixture.workspace_source,
        "source_workspace_hidden_during_fresh_open": sources_hidden_during_fresh_open,
        "source_workspace_restored_byte_identical": true,
        "source_roots_hidden_during_fresh_open": [fixture.workspace_source, fixture.static_source, fixture.animation_source],
        "source_roots_restored": fixture.sources_restored,
        "source_sidecars": all_sidecars.iter().map(|(path, bytes)| serde_json::json!({"path": path, "bytes": bytes.len(), "sha256": sha256_hex(bytes)})).collect::<Vec<_>>(),
        "workspace_save": workspace_save_receipt,
        "animation_preview": animation_preview_receipt,
        "duplicate_directory": fixture.duplicate_directory,
        "duplicate_scene": fixture.duplicate_scene,
        "duplicate_scene_sha256": duplicate_hash,
        "duplicate_sidecars": sidecar_receipt,
        "copied_scene_and_sidecars_byte_identical": true,
        "graph_revision": graph_revision_hex.clone(),
        "static_package_source": fixture.static_source,
        "static_asset_sha256": fixture.static_model_sha256,
        "static_license_sha256": fixture.static_license_sha256,
        "animation_package_source": fixture.animation_source,
        "animation_asset_sha256": fixture.animation_model_sha256,
        "animation_license_sha256": fixture.animation_license_sha256,
        "package_sources_hidden_during_fresh_open": sources_hidden_during_fresh_open,
        "package_sources_restored_and_hash_checked": fixture.sources_restored,
        "fresh_physics_open_pixels_match": changed_pixel_count(&final_original_pixels, &reopened_pixels) < 20,
        "play_ticks": timeline_tick,
        "play_distance_raw": played_view.agent.distance.raw(),
        "play_status": format!("{:?}", played_view.agent.status),
        "play_moved_agent_and_changed_pixels": played_view.agent.position != play_start && changed_pixel_count(&tick_zero_pixels, &play_pixels) > 10,
        "skeletal_pose_difference_at_tick_16": skeletal_pose_difference(&animation_document, 16),
        "mission_config": mission_config_receipt,
        "initial_enable_refreshed_route_controls": true,
        "route_stage_preserved_host_and_history": true,
        "route_apply_one_undo_entry": true,
        "route_undo_redo_verified": true,
        "mission_run_after_fresh_open": {"collected": reopened_mission.run.collected, "won": reopened_mission.run.won},
        "mission_run_after_play": {"collected": won_view.run.collected, "won": won_view.run.won},
        "mission_win_tick": win_tick,
        "mission_win_frame_checksum": format!("0x{win_frame_checksum:016x}"),
        "mission_win_agent_status": win_agent_status,
        "win_tick": win_tick,
        "win_frame_checksum": format!("0x{win_frame_checksum:016x}"),
        "mission_replay_full_frame_matches": mission_replay_full_frame_matches,
        "mission_replay_full_frame_checksum": format!("0x{replay_frame_checksum:016x}"),
        "mission_replay_viewport_pixel_diff": mission_replay_viewport_pixel_diff,
        "pixel_comparison": {
            "scope": "raw production 3D viewport GPU RGBA8 readback; full-app PNGs are evidence-only",
            "target_pixels": [900, 850],
            "camera_sidecar_sha256": camera_sidecar_sha256,
            "camera_source": camera_document["camera"],
            "normalization": "same saved camera and viewport dimensions; no crop or rescale",
        },
        "win_full_app_capture_bytes": win_pixels.len(),
        "replay_full_app_capture_bytes": replay_pixels.len(),
        "lighting_inspector_layout": lighting_inspector_layout,
        "lighting_and_hud_inspector_layout": hud_inspector_layout,
        "scene_path": fixture.duplicate_scene,
        "scene_sha256": duplicate_hash,
        "scene_checksum": format!("0x{scene_checksum:016x}"),
        "reprojected_graph_revision": graph_revision_hex,
        "export_directory": export_receipt["directory"],
        "export_manifest_sha256": export_receipt["manifest_sha256"],
        "export_runtime_sha256": export_receipt["runtime_sha256"],
        "export_files9": export_receipt["files"].as_array().unwrap().len(),
        "export_files": export_receipt["files"],
        "export_game_profile": export_receipt["game_profile"],
        "export_input_profile": export_receipt["input_profile"],
        "exact_nine_file_export": export_receipt,
        "physical_os_input": false,
        "capture_count": captures.len(),
        "captures": captures,
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
