use super::*;
use sha2::{Digest, Sha256};

fn fixture() -> (tempfile::TempDir, Editor, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("source.scene.yaml");
    let editor = crate::mesh_hazards_creator::prepare(&scene, &Default::default(), true).unwrap();
    (temp, editor, scene)
}
fn request(editor: &mut Editor, scene: &Path, runtime: &Path) -> Request {
    let (_, _, checksum) = editor.mesh_creation_source_stamp().unwrap();
    Request {
        scene: scene.into(),
        epoch: editor.scene_load_epoch(),
        checksum,
        scene_identity: Identity::read(scene).unwrap(),
        runtime: Identity::read(runtime).unwrap(),
        destination: scene.with_file_name("bundle"),
    }
}
#[test]
fn untrusted_and_blocked_exports_are_read_only() {
    let (_temp, mut editor, scene) = fixture();
    let before = fs::read(&scene).unwrap();
    let mut panel = Panel::default();
    assert!(!panel.export(&mut editor, None));
    assert!(!panel.busy());
    panel.trusted_runtime = true;
    assert!(!panel.export(&mut editor, Some("Apply hazard and mission drafts first")));
    assert!(panel.status.contains("drafts"));
    assert_eq!(fs::read(scene).unwrap(), before);
    assert!(!editor.is_dirty());
}
#[test]
fn active_job_rejects_second_dispatch_and_disconnect_releases_busy() {
    let (_temp, mut editor, _) = fixture();
    let (sender, receiver) = mpsc::sync_channel(1);
    let mut panel = Panel {
        pending: Some(receiver),
        ..Default::default()
    };
    assert!(panel.busy());
    assert!(!panel.export(&mut editor, None));
    drop(sender);
    panel.poll(&mut editor, None);
    assert!(!panel.busy());
    assert!(panel.status.contains("disconnected"));
}
#[test]
fn completion_stamp_rejects_epoch_checksum_path_and_runtime_changes() {
    let (temp, mut editor, scene) = fixture();
    let runtime = temp.path().join("player");
    fs::write(&runtime, b"fixture runtime").unwrap();
    let stamp = request(&mut editor, &scene, &runtime);
    assert!(source_matches(&mut editor, None, &stamp));
    assert!(!source_matches(&mut editor, Some("dirty draft"), &stamp));
    let mut changed = stamp.clone();
    changed.epoch += 1;
    assert!(!source_matches(&mut editor, None, &changed));
    changed = stamp.clone();
    changed.checksum ^= 1;
    assert!(!source_matches(&mut editor, None, &changed));
    changed = stamp.clone();
    changed.scene = scene.with_file_name("another.scene.yaml");
    assert!(!source_matches(&mut editor, None, &changed));
    fs::write(&runtime, b"replacement runtime bytes").unwrap();
    assert!(!source_matches(&mut editor, None, &stamp));
    assert!(run(&stamp).unwrap_err().contains("identity changed"));
    assert!(!stamp.destination.exists());
}
#[test]
fn play_mode_and_missing_trust_are_not_eligible() {
    let (_temp, mut editor, _) = fixture();
    assert!(eligible(&editor, None, true));
    assert!(!eligible(&editor, None, false));
    editor.play();
    editor.sync();
    assert!(!eligible(&editor, None, true));
}
#[test]
fn identity_and_digest_are_file_only_and_bounded() {
    let temp = tempfile::tempdir().unwrap();
    assert!(Identity::read(temp.path()).is_err());
    let runtime = temp.path().join("runtime");
    fs::write(&runtime, b"hello").unwrap();
    assert_eq!(
        digest_runtime(&runtime).unwrap(),
        Sha256::digest(b"hello")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
}

/// Real production EditorApp UI path. Exports but never executes the player.
/// Software Vulkan is acceptable here and is reported explicitly, not GPU evidence.
#[test]
#[ignore = "requires explicit self-built player and fresh UI evidence directory"]
fn actual_editor_ui_exports_real_hazard_player_without_launching_it() {
    use crate::app::EditorApp;
    use egui::{Key, Modifiers, accesskit::Role};
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use egui_wgpu::{WgpuSetupCreateNew, wgpu};
    use std::time::{Duration, Instant};

    fn settle(h: &mut Harness<'_, EditorApp>) {
        for _ in 0..3 {
            h.state_mut().editor.sync();
            h.run_steps(2);
        }
    }
    fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
        h.get_by_label(label).click_accesskit();
        settle(h);
    }
    fn fill(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
        h.get_by_role_and_label(Role::TextInput, label).focus();
        h.run_steps(2);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, label)
            .type_text(text);
        settle(h);
    }
    fn wait_status(h: &mut Harness<'_, EditorApp>, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            settle(h);
            if h.query_by_label_contains(expected).is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "export UI timed out waiting for {expected}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn capture(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
        settle(h);
        let frame = h.render().expect("render actual production editor");
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width() as usize, frame.height() as usize],
            frame.as_raw(),
        );
        crate::app::write_png(&root.join(name), &image).unwrap();
    }
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    let root = PathBuf::from(
        std::env::var_os("ORR_HAZARD_EXPORT_UI_DIR")
            .expect("set fresh absolute ORR_HAZARD_EXPORT_UI_DIR"),
    );
    let runtime = PathBuf::from(
        std::env::var_os("ORR_HAZARD_EXPORT_RUNTIME")
            .expect("set self-built QA player ORR_HAZARD_EXPORT_RUNTIME"),
    );
    assert!(root.is_absolute() && runtime.is_absolute());
    assert!(runtime.is_file());
    fs::create_dir(&root).expect("UI evidence directory must be fresh");
    let scene = root.join("source.scene.yaml");
    let output = root.join("hazard-game");
    let editor = crate::mesh_hazards_creator::prepare(&scene, &Default::default(), true).unwrap();
    let source_before = fs::read(&scene).unwrap();
    let runtime_before = digest_runtime(&runtime).unwrap();
    fs::write(root.join("before.json"), serde_json::to_vec_pretty(&serde_json::json!({"scene":scene,"scene_sha256":hash(&source_before),"runtime":runtime,"runtime_sha256":runtime_before})).unwrap()).unwrap();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    let mut h = Harness::builder()
        .with_size([1500.0, 1050.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    click(&mut h, "File");
    click(&mut h, "Export hazard game…");
    fill(
        &mut h,
        "Hazard player executable",
        &runtime.display().to_string(),
    );
    fill(
        &mut h,
        "New output directory (must not exist)",
        &output.display().to_string(),
    );
    assert!(
        h.get_by_role_and_label(Role::Button, "Export hazard game")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "I trust this self-built, QA-tested hazard player");
    capture(&mut h, &root, "01-export-ready.png");
    h.get_by_role_and_label(Role::Button, "Export hazard game")
        .click_accesskit();
    wait_status(&mut h, "Hazard export complete at");
    assert!(
        h.query_by_label_contains("Player was not launched.")
            .is_some()
    );
    capture(&mut h, &root, "02-export-complete.png");
    let prepared = bundle::open_bundle(&output).unwrap();
    assert_eq!(fs::read(&prepared.scene).unwrap(), source_before);
    assert_eq!(prepared.runtime_sha256, runtime_before);
    assert_eq!(fs::read_dir(&output).unwrap().count(), 3);
    let manifest_before = fs::read(output.join(bundle::MANIFEST_FILENAME)).unwrap();
    // The second real UI dispatch must refuse overwrite and preserve all files.
    h.get_by_role_and_label(Role::Button, "Export hazard game")
        .click_accesskit();
    wait_status(&mut h, "Hazard export failed:");
    assert_eq!(
        fs::read(output.join(bundle::MANIFEST_FILENAME)).unwrap(),
        manifest_before
    );
    assert_eq!(bundle::open_bundle(&output).unwrap(), prepared);
    capture(&mut h, &root, "03-overwrite-refused.png");
    // Mutate the authoritative scene only in memory. Dirty source cannot dispatch.
    let patch = {
        let frame = h.state().editor.snapshot().unwrap().predicted();
        let mesh = orr_remote::mesh_navigation::mesh_from_view(frame).unwrap();
        orr_remote::mesh_navigation::hazards::patch_request(&Default::default(), &mesh).unwrap()
    };
    h.state_mut()
        .editor
        .host_call("world.singleton.patch", patch)
        .unwrap();
    settle(&mut h);
    assert!(h.state().editor.is_dirty());
    assert!(
        h.get_by_role_and_label(Role::Button, "Export hazard game")
            .accesskit_node()
            .is_disabled()
    );
    capture(&mut h, &root, "04-dirty-scene-blocked.png");
    assert_eq!(fs::read(&scene).unwrap(), source_before);
    assert_eq!(digest_runtime(&runtime).unwrap(), runtime_before);
    fs::write(root.join("receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"ui":"production EditorApp / egui accesskit","renderer":"Vulkan; software allowed, not hardware acceptance",
        "bundle":output,"source_scene":scene,"source_scene_sha256_before":hash(&source_before),"source_scene_sha256_after":hash(&fs::read(&scene).unwrap()),
        "source_runtime":runtime,"runtime_sha256_before":runtime_before,"runtime_sha256_after":digest_runtime(&runtime).unwrap(),
        "manifest_sha256":hash(&manifest_before),"inventory_files":3,"overwrite_refused":true,"dirty_scene_blocked":true,"runtime_executed":false
    })).unwrap()).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn runtime_digest_refuses_fifo_replacement_after_panel_identity_without_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("player");
    fs::write(&path, b"regular runtime").unwrap();
    let identity = Identity::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let _ = sender.send(digest_runtime(&identity.path));
    });
    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("runtime FIFO replacement blocked panel hashing");
    assert!(result.is_err());
    worker.join().unwrap();
}

#[cfg(unix)]
#[test]
fn runtime_digest_refuses_symlink_replacement_after_panel_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("player");
    let target = path.with_file_name("other-runtime");
    fs::write(&path, b"regular runtime").unwrap();
    fs::write(&target, b"must not hash linked content").unwrap();
    let identity = Identity::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(digest_runtime(&identity.path).is_err());
}

#[test]
fn completion_inputs_preserve_entered_path_form_and_reject_cancellation_or_changes() {
    let (temp, mut editor, scene) = fixture();
    let runtime = temp.path().canonicalize().unwrap().join("player");
    fs::write(&runtime, b"runtime").unwrap();
    let mut stamp = request(&mut editor, &scene, &runtime);
    let root = runtime.parent().unwrap().display();
    #[cfg(windows)]
    let entered = format!("{root}\\player");
    #[cfg(not(windows))]
    let entered = format!("{root}//player");
    stamp.runtime.path = PathBuf::from(&entered);
    let output = stamp.destination.display().to_string();
    assert!(inputs_match(&entered, &output, &stamp, false, true));
    assert!(!inputs_match(&entered, &output, &stamp, true, true));
    assert!(!inputs_match(&entered, &output, &stamp, false, false));
    assert!(!inputs_match(
        &entered,
        "changed-output",
        &stamp,
        false,
        true
    ));
    assert!(!inputs_match(
        "changed-runtime",
        &output,
        &stamp,
        false,
        true
    ));
    #[cfg(windows)]
    {
        // Ordinary C:\ paths remain ordinary, rather than being changed to \\?\ form.
        stamp.runtime.path = PathBuf::from(r"C:\QA\orr_mesh_hazards_player.exe");
        assert!(inputs_match(
            r"C:\QA\orr_mesh_hazards_player.exe",
            &output,
            &stamp,
            false,
            true
        ));
    }
}
