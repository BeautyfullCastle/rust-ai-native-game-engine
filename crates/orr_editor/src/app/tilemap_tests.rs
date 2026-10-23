//! Actual headless eframe/AccessKit widgets and egui viewport events.
//! This is not native OS mouse/keyboard or a separately launched desktop window.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use egui_wgpu::wgpu;
use std::{fs, path::Path, process::Command, sync::Arc};

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}
fn open(root: &Path) -> Harness<'static, EditorApp> {
    let project = crate::project::PreparedProject::open(root).unwrap();
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "mandatory software Vulkan adapter unavailable".into())
    }));
    let mut h = Harness::builder()
        .with_size([1800., 2000.])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut editor = Editor::start(&project.host_spec()).unwrap();
            editor.sync();
            project
                .into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx)
                .unwrap()
        });
    h.run_steps(5);
    h
}
fn pointer(h: &mut Harness<'_, EditorApp>, world: [f32; 2]) {
    let app = h.state();
    let rect = app.ui.viewport_rect.expect("actual app viewport");
    let px = app.ui.viewport_px;
    let screen = app.editor.camera.world_to_screen(world, px);
    let point = rect.min
        + egui::vec2(
            screen[0] * rect.width() / px.0 as f32,
            screen[1] * rect.height() / px.1 as f32,
        );
    assert!(rect.contains(point));
    for pressed in [true, false] {
        h.event(egui::Event::PointerMoved(point));
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(2);
    }
}
fn frame(h: &Harness<'_, EditorApp>) -> u64 {
    h.state()
        .editor
        .snapshot()
        .expect("host snapshot")
        .predicted()
        .checksum()
}
fn capture(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) {
    let gpu = h
        .state()
        .viewport_gpu()
        .expect("mandatory app viewport GPU")
        .gpu();
    let adapter = gpu.adapter_name();
    let image = h.render().expect("mandatory whole-egui-frame readback");
    let size = image.dimensions();
    let pixels = image.into_raw();
    let file = format!("{name}.png");
    let mut encoder = png::Encoder::new(
        fs::File::create(evidence.join(&file)).unwrap(),
        size.0,
        size.1,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    // Explicit FNV-1a capture fingerprint (not mislabeled as SHA-256).
    let fingerprint = pixels.iter().fold(0xcbf29ce484222325u64, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    fs::write(evidence.join("partial-editor-receipt.json"),serde_json::to_vec_pretty(&serde_json::json!({"completed":false,"last_capture":file,"rgba_fnv1a64":format!("{fingerprint:016x}"),"size":size,"frame_checksum":h.state().editor.checksum(),"native_os_input":false,"adapter":adapter,"capture_scope":"full egui frame including actor sprites/grid/palette; visual evidence, not pixel verdict"})).unwrap()).unwrap();
}
#[test]
#[ignore = "mandatory explicit actual EditorApp tilemap creation/authoring/Play acceptance"]
fn actual_cli_editor_palette_paint_save_reopen_play() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_EDITOR_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let cli = PathBuf::from(
        std::env::var_os("ORR_NEW_ARENA_BIN").expect("built tilemap creator CLI required"),
    );
    assert!(cli.is_absolute() && cli.is_file());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("created");
    let output = Command::new(&cli)
        .args([
            "--template",
            "arena-tilemap-2d-v1",
            "--seed",
            "editor-acceptance",
            "--output",
        ])
        .arg(&root)
        .output()
        .unwrap();
    fs::write(
        evidence.join("creator.log"),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    assert!(output.status.success(), "actual CLI creation failed");
    let scene = fs::read(root.join("arena.scene.yaml")).unwrap();
    let lock = fs::read(root.join("orr.packages.lock.json")).unwrap();
    let initial_map = fs::read(root.join("arena.tilemap.json")).unwrap();
    let mut h = open(&root);
    let initial_frame = frame(&h);
    let initial_checksum = h.state().editor.checksum();
    let key = ("sample-sprites".into(), "sprites.json".into());
    let texture = h.state().sprites.tilemap_asset(&key).unwrap().1;
    assert_eq!(h.state().sprites.tilemap_assets().len(), 1);
    let ctx = h.ctx.clone();
    assert!(
        h.state_mut()
            .sprites
            .reload(&ctx)
            .unwrap_err()
            .contains("tilemap")
    );
    assert!(
        h.state_mut()
            .sprites
            .open(&ctx, root.join("arena.sprites.json"))
            .unwrap_err()
            .contains("tilemap")
    );
    assert_eq!(h.state().sprites.tilemap_asset(&key).unwrap().1, texture);
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Static region 10");
    let cell = orr_tilemap::CellCoord { column: 4, row: 6 };
    let world = [-56., -8.];
    // A primary press outside the viewport must not become a paint gesture
    // merely because the held pointer later enters it.
    let rect = h.state().ui.viewport_rect.unwrap();
    let px = h.state().ui.viewport_px;
    let at = h.state().editor.camera.world_to_screen(world, px);
    let inside = rect.min
        + egui::vec2(
            at[0] * rect.width() / px.0 as f32,
            at[1] * rect.height() / px.1 as f32,
        );
    let outside = egui::pos2(0., 0.);
    h.event(egui::Event::PointerMoved(outside));
    h.event(egui::Event::PointerButton {
        pos: outside,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    h.event(egui::Event::PointerMoved(inside));
    h.run_steps(2);
    h.event(egui::Event::PointerButton {
        pos: inside,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
    assert!(
        !h.state().tilemap.as_ref().unwrap().dirty(),
        "outside-origin gesture must not paint"
    );
    pointer(&mut h, world);
    pointer(&mut h, world); // The second paint is a no-op.
    capture(&mut h, &evidence, "dirty-paint");
    assert_eq!(
        h.state()
            .tilemap
            .as_ref()
            .unwrap()
            .document()
            .cell(cell)
            .unwrap(),
        Some(10)
    );
    assert!(
        h.state().editor.selected_guids().is_empty(),
        "active tile tool exclusively owns actor click"
    );
    assert!(
        h.state().ui.body_drag.is_none(),
        "paint must not initiate actor drag"
    );
    assert_eq!(frame(&h), initial_frame);
    click(&mut h, "Undo tile");
    assert!(
        !h.state().tilemap.as_ref().unwrap().dirty(),
        "no-op did not create an Undo entry"
    );
    pointer(&mut h, world);
    click(&mut h, "Erase");
    pointer(&mut h, world);
    assert_eq!(
        h.state()
            .tilemap
            .as_ref()
            .unwrap()
            .document()
            .cell(cell)
            .unwrap(),
        None
    );
    click(&mut h, "Undo tile");
    assert!(h.state().tilemap.as_ref().unwrap().dirty());
    let epoch = h.state().editor.scene_load_epoch();
    h.state_mut().restart_from_ui();
    assert_eq!(
        h.state().editor.scene_load_epoch(),
        epoch,
        "installed tilemap refuses UI host restart"
    );
    h.state_mut().play_from_ui();
    h.state_mut().step_from_ui();
    h.run_steps(3);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(frame(&h), initial_frame);
    h.state_mut().save();
    h.run_steps(2);
    assert_eq!(
        fs::read(root.join("arena.tilemap.json")).unwrap(),
        initial_map,
        "Scene Save is not tile Save"
    );
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(1);
    assert!(
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .unwrap()
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::CancelClose)),
        "dirty close must be cancelled"
    );
    for kind in [DialogKind::Open, DialogKind::SaveAs] {
        let destination = temp.path().join("must-not-create.scene.yaml");
        h.state_mut().ui.dialog = Some(Dialog {
            kind,
            text: destination.to_string_lossy().into_owned(),
        });
        h.run_steps(2);
        click(&mut h, "OK");
        assert!(
            h.state().ui.dialog.is_some(),
            "scene identity change rejected"
        );
        assert!(!destination.exists());
        assert_eq!(frame(&h), initial_frame);
        h.state_mut().ui.dialog = None;
        h.run_steps(2);
    }
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    assert_eq!(fs::read(root.join("orr.packages.lock.json")).unwrap(), lock);
    assert_eq!(h.state().editor.checksum(), initial_checksum);
    click(&mut h, "Save tilemap");
    capture(&mut h, &evidence, "saved");
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let saved = h.state().tilemap.as_ref().unwrap().document().clone();
    // Tool-off deliberately releases selection, not silently swallowing actor input.
    click(&mut h, "Tile tool (owns viewport clicks)");
    pointer(&mut h, world);
    assert!(!h.state().editor.selected_guids().is_empty());
    assert_eq!(frame(&h), initial_frame);
    // A bound actor cannot be removed and then published as an unreadable bundle.
    assert!(h.state_mut().editor.select_named("hero"));
    assert!(h.state_mut().editor.delete_selected());
    h.state_mut().editor.sync();
    h.state_mut().save();
    h.run_steps(2);
    assert_eq!(
        fs::read(root.join("arena.scene.yaml")).unwrap(),
        scene,
        "invalid sprite target must prevent Scene Save"
    );
    let forbidden = temp.path().join("forbidden.scene.yaml");
    assert!(!h.state_mut().editor.save());
    assert!(!h.state_mut().editor.save_as(&forbidden));
    assert!(crate::script::run_command(&mut h.state_mut().editor, "save").is_err());
    assert!(
        h.state_mut()
            .editor
            .host_call("scene.save", serde_json::json!({"write":true}))
            .is_err()
    );
    assert!(!forbidden.exists());
    assert_eq!(
        fs::read(root.join("arena.scene.yaml")).unwrap(),
        scene,
        "all public/script writes must refuse invalid targets before disk publication"
    );
    assert!(h.state_mut().editor.undo());
    h.state_mut().editor.sync();
    assert_eq!(
        frame(&h),
        initial_frame,
        "Undo restores valid target and sim frame"
    );
    h.state_mut().editor.select(None);
    assert!(
        h.state_mut().editor.save(),
        "Undo restores a valid document; ordinary Scene Save must remain supported"
    );
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    // A canonical-equivalent symlink is not the admitted publication target:
    // accepting it would atomically replace the alias itself with a new file.
    let alias = temp.path().join("scene-alias.yaml");
    std::os::unix::fs::symlink(root.join("arena.scene.yaml"), &alias).unwrap();
    let alias_target = fs::read_link(&alias).unwrap();
    assert!(!h.state_mut().editor.save_as(&alias));
    assert!(
        h.state_mut()
            .editor
            .host_call("scene.save", serde_json::json!({"write":true,"path":alias}))
            .is_err()
    );
    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&alias).unwrap(), alias_target);
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    // The guard is installed before App creation, so startup scripts cannot
    // publish invalid targets before into_app has a chance to recheck them.
    let prepared = crate::project::PreparedProject::open(&root).unwrap();
    let mut changed = Editor::start(&prepared.host_spec()).unwrap();
    changed.sync();
    assert!(crate::script::run_script(&mut changed, "select hero\ndelete\nsave").is_err());
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    assert!(
        prepared
            .into_app(changed, h.state().render_state.clone(), &h.ctx)
            .is_err()
    );
    // Direct APIs are pinned too, even when no UI guard participates.
    let epoch = h.state().editor.scene_load_epoch();
    assert!(!h.state_mut().editor.restart());
    assert!(
        !h.state_mut()
            .editor
            .open_path(&root.join("arena.scene.yaml"))
    );
    assert!(h.state_mut().editor.host_call("scene.load",serde_json::json!({"text":String::from_utf8(scene.clone()).unwrap(),"path":root.join("arena.scene.yaml")})).is_err());
    assert!(
        h.state_mut()
            .editor
            .host_call(
                "scene.save",
                serde_json::json!({"write":true,"source":"play"})
            )
            .is_err()
    );
    assert_eq!(h.state().editor.scene_load_epoch(), epoch);
    assert!(
        h.state()
            .tilemap
            .as_ref()
            .unwrap()
            .scene_matches(&h.state().editor)
    );
    assert_eq!(frame(&h), initial_frame);
    // Scope control: an ordinary Arena scene without a tilemap admission guard
    // still supports save-as, raw scene open and restart.
    let plain_path = temp.path().join("plain.scene.yaml");
    fs::write(&plain_path, &scene).unwrap();
    let mut plain = Editor::open_game(&plain_path, crate::game::EditorGame::Arena).unwrap();
    plain.sync();
    let plain_copy = temp.path().join("plain-copy.scene.yaml");
    assert!(plain.save_as(&plain_copy));
    assert!(plain_copy.is_file());
    assert!(plain.open_path(&plain_path));
    assert!(plain.restart());
    drop(plain);
    // A fresh host manually substituted by an embedding caller has a new
    // identity even at the same path. Old presentation state stays unusable.
    let mut replacement = Editor::start(h.state().editor.spec()).unwrap();
    replacement.sync();
    h.state_mut().editor = replacement;
    assert!(
        !h.state()
            .tilemap
            .as_ref()
            .unwrap()
            .scene_matches(&h.state().editor)
    );
    let app = h.state_mut();
    assert!(
        app.tilemap
            .as_mut()
            .unwrap()
            .save(&app.editor, &app.sprites)
            .is_err()
    );
    drop(h); // Saved close, then fresh host/cache/GPU admission from disk.
    let mut reopened = open(&root);
    capture(&mut reopened, &evidence, "fresh-reopen");
    assert_eq!(
        reopened.state().tilemap.as_ref().unwrap().document(),
        &saved
    );
    assert_eq!(frame(&reopened), initial_frame);
    reopened.state_mut().play_from_ui();
    reopened.run_steps(4);
    capture(&mut reopened, &evidence, "play");
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    assert_eq!(
        reopened.state().tilemap.as_ref().unwrap().document(),
        &saved
    );
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    assert_eq!(fs::read(root.join("orr.packages.lock.json")).unwrap(), lock);
    fs::write(evidence.join("editor-receipt.json"),serde_json::to_vec_pretty(&serde_json::json!({"completed":true,"captures":["dirty-paint.png","saved.png","fresh-reopen.png","play.png"],"input":"AccessKit widget actions and egui viewport events","native_os_input":false,"fresh_host_reopen":true,"runtime_pixel_oracle":"separate project_tilemap_runtime acceptance"})).unwrap()).unwrap();
}
