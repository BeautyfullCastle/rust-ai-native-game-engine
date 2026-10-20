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
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{editor::Owner, Editor, EditorApp};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
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
fn digest(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn recorded(root: &Path, stage: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .expect("launch recorded acceptance command");
    let stdout_file = format!("{stage}.stdout.txt");
    let stderr_file = format!("{stage}.stderr.txt");
    fs::write(root.join(&stdout_file), &output.stdout).unwrap();
    fs::write(root.join(&stderr_file), &output.stderr).unwrap();
    let record = serde_json::json!({"stage":stage,"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|s|s.to_string_lossy().into_owned()).collect::<Vec<_>>(),"cwd":command.get_current_dir().map(|p|p.display().to_string()),"status":output.status.code(),"stdout_file":stdout_file,"stderr_file":stderr_file});
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("commands.jsonl"))
            .unwrap(),
        "{record}"
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{stage}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn inventory(root: &Path) -> Vec<serde_json::Value> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<serde_json::Value>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(!meta.file_type().is_symlink());
            if meta.is_dir() {
                visit(root, &path, out);
            } else {
                assert!(meta.is_file());
                out.push(serde_json::json!({"path":path.strip_prefix(root).unwrap().to_str().unwrap(),"bytes":meta.len(),"sha256":digest(&path)}));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out
}
fn git(repo: &Path, arg: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", arg])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().into()
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
    let requested = PathBuf::from(
        std::env::var_os("ORR_ARENA_EXPORT_EVIDENCE_DIR")
            .expect("require fresh retained acceptance evidence directory"),
    );
    assert!(requested.is_absolute());
    assert!(!requested.exists());
    fs::create_dir(&requested).unwrap();
    let root = requested.canonicalize().unwrap();
    assert!(!root.starts_with(&repo));
    assert!(!root.starts_with(runtime.parent().unwrap()));
    let source_head = git(&repo, "HEAD");
    let source_tree = git(&repo, "HEAD^{tree}");
    let producer = std::env::current_exe().unwrap().canonicalize().unwrap();
    let producer_hash = digest(&producer);
    let runtime_hash = digest(&runtime);
    let stage = |name: &str, joined: bool| {
        fs::write(root.join("stage-outcomes.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"source_head":source_head,"source_tree":source_tree,"stage":name,"terminal_joined_success":joined})).unwrap()).unwrap()
    };
    fs::write(root.join("producer-receipt.json"),serde_json::to_vec_pretty(&serde_json::json!({"source_head":source_head,"source_tree":source_tree,"runtime":{"path":runtime,"sha256":runtime_hash},"test_producer":{"path":producer,"sha256":producer_hash}})).unwrap()).unwrap();
    stage("create-and-edit", false);
    let project = root.join("project");
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
    let output = root.join("exported with spaces");
    let hash = runtime_hash.clone();
    field(&mut h, "Project path", project.to_str().unwrap());
    field(&mut h, "New output path", output.to_str().unwrap());
    field(&mut h, "Runtime path", runtime.to_str().unwrap());
    field(&mut h, "Expected lowercase SHA256", &hash);
    h.get_by_label("I trust this supplied runtime and allow smoke verification to execute it")
        .click();
    h.run_steps(2);
    let source_scene = fs::read(&scene).unwrap();
    let source_files = inventory(&project);
    fs::write(
        root.join("source-files-before.json"),
        serde_json::to_vec_pretty(&source_files).unwrap(),
    )
    .unwrap();
    stage("actual-widget-dispatch", false);
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
    stage("terminal-joined-success", true);
    fs::write(root.join("terminal-joined-success.txt"),format!("Actual export widget reached terminal joined success for captured scene {} at checksum 0x{checksum:016x}\n",scene.display())).unwrap();
    drop(h); // success marker is emitted only after terminal and joined worker.
    assert_eq!(fs::read(&scene).unwrap(), source_scene);
    let bundle = root.join("bundle");
    fs::rename(output, &bundle).unwrap();
    stage("relocated-source-hidden-read-only", true);
    let expected = recorded(
        &root,
        "source-move-fire",
        Command::new(&runtime)
            .current_dir(&root)
            .arg("--project")
            .arg(&project)
            .args(["--headless", "--ticks", "3", "--hold", "right,fire"]),
    );
    let isolated = || {
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
        cmd.current_dir(&root)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        cmd
    };
    // Underlying owned directory is writable; the mount, not Unix file mode,
    // must deny this fresh write. Never leave the probe in the delivered bundle.
    fs::write(bundle.join(".write-probe"), b"before read-only mount").unwrap();
    fs::remove_file(bundle.join(".write-probe")).unwrap();
    let proof=recorded(&root,"isolated-proof",isolated().args(["--","/bin/sh","-c", "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" || exit 91; if touch \"$4/.write-probe\" 2>/dev/null; then rm -f \"$4/.write-probe\"; exit 92; fi; printf 'source-hidden=true bundle-read-only=true write-denied=true\\n'", "sh"]).arg(repo.join("Cargo.toml")).arg(project.join("orr.project.json")).arg(&runtime).arg(&bundle));
    assert!(proof.contains("source-hidden=true bundle-read-only=true write-denied=true"));
    let run = |name: &str, ticks: &str, hold: &str, capture: Option<&Path>| {
        let mut cmd = isolated();
        cmd.args(["--","/bin/sh","-c","test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" || exit 91; shift 3; exec \"$@\"","sh"])
            .arg(repo.join("Cargo.toml")).arg(project.join("orr.project.json")).arg(&runtime)
            .arg(bundle.join("run-arena")).args(["--headless","--ticks",ticks,"--hold",hold]);
        if let Some(path) = capture {
            cmd.arg("--capture").arg(path);
        }
        recorded(&root, name, &mut cmd)
    };
    assert_eq!(run("relocated-fire-1", "3", "right,fire", None), expected);
    assert_eq!(run("relocated-fire-2", "3", "right,fire", None), expected);
    assert_ne!(
        run("relocated-move", "3", "right", None),
        expected,
        "fire changes production simulation"
    );
    stage("production-compositor-capture", true);
    let initial = root.join("initial.png");
    let moved = root.join("moved.png");
    assert!(
        run("capture-initial", "0", "idle", Some(&initial)).contains("project capture adapter:")
    );
    assert!(run("capture-moved", "3", "right", Some(&moved)).contains("project capture adapter:"));
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
        asset
            .rgba
            .chunks_exact(4)
            .filter(|p| p[3] == 255 && (p[0].abs_diff(p[1]) > 20 || p[1].abs_diff(p[2]) > 20))
            .any(|p| a
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
    assert_eq!(
        inventory(&project),
        source_files,
        "all captured source bytes preserved"
    );
    assert_eq!(digest(&runtime), runtime_hash);
    let receipt = serde_json::json!({"schema":1,"source_head":source_head,"source_tree":source_tree,
        "runtime":{"path":runtime,"sha256":runtime_hash},"test_producer":{"path":producer,"sha256":producer_hash},
        "terminal_joined_success":true,"source_hidden":true,"bundle_read_only":true,"source_preserved":true,
        "source_files":source_files,"source_files_before":source_files,"source_files_after":inventory(&project),"bundle_files":inventory(&bundle),
        "captures":[{"path":"initial.png","width":512,"height":512,"sha256":digest(&initial)},{"path":"moved.png","width":512,"height":512,"sha256":digest(&moved)}],
        "commands":["source-move-fire","isolated-proof","relocated-fire-1","relocated-fire-2","relocated-move","capture-initial","capture-moved"]});
    fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    stage("complete", true);
}
