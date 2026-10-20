//! Explicit acceptance, not part of ordinary CPU unit checks. Requires a separately
//! built matching player, bubblewrap and a working production capture adapter.
#![cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    feature = "arena-project-export",
    feature = "project-create"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{editor::Owner, Editor, EditorApp};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
}
fn success(output: std::process::Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn png(path: &Path) -> Vec<u8> {
    let mut reader = png::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!((info.width, info.height), (512, 512));
    bytes.truncate(info.buffer_size());
    bytes
}
#[test]
#[ignore = "requires ORR_ARENA_EXPORT_RUNTIME, bubblewrap source hiding and a working capture adapter"]
fn actual_widget_generated_edited_project_relocates_and_runs_with_sprites() {
    let runtime = PathBuf::from(
        std::env::var_os("ORR_ARENA_EXPORT_RUNTIME")
            .expect("supply separately built matching arena runtime"),
    )
    .canonicalize()
    .unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    assert!(!root.starts_with(&repo));
    let project = root.join("generated");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: project.clone(),
        template: orr_sample::project_create::TEMPLATE.into(),
        seed: "editor-export-279".into(),
    })
    .unwrap();
    let prepared = orr_editor::project::PreparedProject::open(&project).unwrap();
    let mut editor = Editor::start(&prepared.host_spec()).unwrap();
    editor.sync();
    assert!(editor.select_named("hero"));
    assert!(editor.set_field(
        &Owner::Component("Position".into()),
        "pos.x",
        orr_reflect::Value::Fixed(orr_fp::FP::from_int(-54))
    ));
    editor.sync();
    assert!(editor.save());
    editor.sync();
    let checksum = editor.checksum();
    let scene = project.join("arena.scene.yaml");
    assert!(editor.open_path(&scene));
    editor.sync();
    assert_eq!(editor.checksum(), checksum);
    editor.play();
    editor.sync();
    editor.stop();
    editor.sync();
    assert_eq!(editor.checksum(), checksum);
    // Re-admit after explicit save/reopen, then install the exact production cache.
    let prepared = orr_editor::project::PreparedProject::open(&project).unwrap();
    let ctx = egui::Context::default();
    let app = prepared.into_app(editor, None, &ctx).unwrap();
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    h.run_steps(3);
    h.get_by_label("Arena export").click();
    h.run_steps(3);
    let output = root.join("exported");
    let hash = format!("{:x}", Sha256::digest(fs::read(&runtime).unwrap()));
    field(&mut h, "Project path", project.to_str().unwrap());
    field(&mut h, "New output path", output.to_str().unwrap());
    field(&mut h, "Runtime path", runtime.to_str().unwrap());
    field(&mut h, "Expected lowercase SHA256", &hash);
    h.get_by_label("I trust this supplied runtime and allow smoke verification to execute it")
        .click();
    h.run_steps(2);
    let source_scene = fs::read(&scene).unwrap();
    h.get_by_label("Export saved Arena").click();
    h.run_steps(2);
    let deadline = Instant::now() + Duration::from_secs(30);
    while h
        .query_by_label("Arena export completed and joined")
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "actual widget export did not report terminal joined success"
        );
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..10 {
        h.run_steps(1);
    }
    drop(h); // must join any final worker tail before source hiding.
    assert_eq!(fs::read(&scene).unwrap(), source_scene);
    let bundle = root.join("relocated read only game");
    fs::rename(output, &bundle).unwrap();
    let expected = success(
        Command::new(&runtime)
            .arg("--project")
            .arg(&project)
            .args(["--headless", "--ticks", "3", "--hold", "right,fire"])
            .output()
            .unwrap(),
    );
    let run = |ticks: &str, hold: &str, capture: Option<&Path>| {
        let mut cmd = Command::new("bwrap");
        cmd.args(["--ro-bind", "/", "/", "--bind"])
            .arg(&root)
            .arg(&root)
            .arg("--ro-bind")
            .arg(&bundle)
            .arg(&bundle)
            .arg("--tmpfs")
            .arg(&repo)
            .arg("--tmpfs")
            .arg(&project);
        if !runtime.starts_with(&repo) {
            cmd.arg("--tmpfs").arg(runtime.parent().unwrap());
        }
        cmd.args(["--","/bin/sh","-c","test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" || exit 91; shift 3; exec \"$@\"","sh"])
            .arg(repo.join("Cargo.toml")).arg(project.join("orr.project.json")).arg(&runtime)
            .arg(bundle.join("run-arena")).args(["--headless","--ticks",ticks,"--hold",hold]);
        if let Some(path) = capture {
            cmd.arg("--capture").arg(path);
        }
        success(
            cmd.current_dir(&root)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap(),
        )
    };
    assert_eq!(run("3", "right,fire", None), expected);
    assert_eq!(run("3", "right,fire", None), expected);
    assert_ne!(
        run("3", "right", None),
        expected,
        "fire changes production simulation"
    );
    let initial = root.join("initial.png");
    let moved = root.join("moved.png");
    assert!(run("0", "idle", Some(&initial)).contains("project capture adapter:"));
    assert!(run("3", "right", Some(&moved)).contains("project capture adapter:"));
    let a = png(&initial);
    let b = png(&moved);
    assert_ne!(a, b, "production compositor reflects movement");
    let prepared = orr_sample::project::PreparedProject::open(
        &project,
        orr_sample::project_runtime::compiled_runtime(),
    )
    .unwrap();
    let asset = prepared.sprites().unwrap().assets.values().next().unwrap();
    assert!(
        asset.rgba.chunks_exact(4).filter(|p| p[3] == 255).any(|p| a
            .chunks_exact(4)
            .filter(|q| q.iter().zip(p).all(|(x, y)| x.abs_diff(*y) <= 3))
            .count()
            > 20),
        "actual installed sprite texels must occur in production output"
    );
    let manifest: orr_sample::project_export::ExportManifest =
        serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
    assert!(
        manifest
            .payload
            .files
            .iter()
            .any(|f| f.path.to_ascii_lowercase().contains("license")),
        "license closure retained"
    );
    assert_eq!(fs::read(scene).unwrap(), source_scene);
}
