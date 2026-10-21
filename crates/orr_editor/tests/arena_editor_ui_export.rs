//! Explicit acceptance, not part of ordinary CPU unit checks. Requires a separately
//! built matching player, bubblewrap and a working production capture adapter.
#![cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    feature = "arena-project-export",
    feature = "project-create",
    feature = "project-ui"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{editor::Owner, Editor};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "common/arena_export.rs"]
mod evidence;
use evidence::{digest, field, git, inventory, png};
#[path = "common/arena_ui_evidence.rs"]
mod ui_evidence;
use ui_evidence::{consumer_command, recorded, recorded_status, ConsumerPaths};
#[test]
#[ignore = "requires ORR_ARENA_UI_EXPORT_RUNTIME, bubblewrap source hiding and a working capture adapter"]
fn actual_widget_saved_korean_ui_exports_relocates_and_runs() {
    let runtime = PathBuf::from(
        std::env::var_os("ORR_ARENA_UI_EXPORT_RUNTIME")
            .expect("supply separately built matching arena runtime"),
    )
    .canonicalize()
    .unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let requested = PathBuf::from(
        std::env::var_os("ORR_ARENA_UI_EXPORT_EVIDENCE_DIR")
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
        seed: "editor-ui-export-282".into(),
    })
    .unwrap();
    install_ui(&project);
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
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |cc| prepared.into_app(editor, None, &cc.egui_ctx).unwrap());
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
    let before_rejection = inventory(&project);
    // Real unrelated runtime must independently reject UI and fail exporter smoke.
    let plain_runtime = PathBuf::from(
        std::env::var_os("ORR_ARENA_UI_PROJECT_ONLY_RUNTIME")
            .expect("supply separately built project-only runtime"),
    )
    .canonicalize()
    .unwrap();
    assert_ne!(digest(&plain_runtime), runtime_hash);
    let reject_stdout = recorded_status(
        &root,
        "project-only",
        Command::new(&plain_runtime)
            .current_dir(&root)
            .arg("--project")
            .arg(&project)
            .args(["--headless", "--ticks", "0"]),
        1,
    );
    let unsupported_ui = "error: saved project declares UI but this host does not support it";
    assert!(reject_stdout.is_empty());
    assert_eq!(
        fs::read_to_string(root.join("project-only.stderr.txt"))
            .unwrap()
            .trim(),
        unsupported_ui
    );
    let rejected_output = root.join("project-only-rejected");
    let rejection =
        orr_sample::project_export::export(&orr_sample::project_export::ExportOptions {
            project: project.clone(),
            runtime: plain_runtime.clone(),
            runtime_sha256: digest(&plain_runtime),
            output: rejected_output.clone(),
            trusted_runtime: true,
            source_revision: None,
        })
        .unwrap_err();
    fs::write(root.join("project-only-export-rejection.txt"), &rejection).unwrap();
    assert_eq!(
        rejection.trim(),
        format!("trusted runtime smoke failed (exit status: 1): {unsupported_ui}")
    );
    assert!(!rejected_output.exists());
    assert_eq!(
        inventory(&project),
        before_rejection,
        "failed runtime/export must preserve source"
    );
    fs::write(root.join("project-only-runtime.json"), serde_json::to_vec_pretty(&serde_json::json!({"path":plain_runtime,"sha256":digest(&plain_runtime),"status":1,"stdout":"project-only.stdout.txt","stderr":"project-only.stderr.txt","export_error":rejection,"published":false})).unwrap()).unwrap();
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
        if !producer.starts_with(&repo) && !producer.starts_with(runtime.parent().unwrap()) {
            assert!(!root.starts_with(producer.parent().unwrap()));
            cmd.arg("--tmpfs").arg(producer.parent().unwrap());
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
            .as_chunks::<4>().0.iter()
            .filter(|p| p[3] == 255 && (p[0].abs_diff(p[1]) > 20 || p[1].abs_diff(p[2]) > 20))
            .any(|p| a
                .as_chunks::<4>().0.iter()
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
    let mut receipt = serde_json::json!({"schema":1,"source_head":source_head,"source_tree":source_tree,
        "runtime":{"path":runtime,"sha256":runtime_hash},"test_producer":{"path":producer,"sha256":producer_hash},
        "terminal_joined_success":true,"source_hidden":true,"bundle_read_only":true,"source_preserved":true,
        "source_files":source_files,"source_files_before":source_files,"source_files_after":inventory(&project),"bundle_files":inventory(&bundle),
        "captures":[{"path":"initial.png","width":512,"height":512,"sha256":digest(&initial)},{"path":"moved.png","width":512,"height":512,"sha256":digest(&moved)}],
        "commands":["project-only","source-move-fire","isolated-proof","relocated-fire-1","relocated-fire-2","relocated-move","capture-initial","capture-moved"]});
    // Launch the production-module UI consumer from outside the hidden source.
    let consumer = root.join("ui-consumer");
    fs::copy(&producer, &consumer).unwrap();
    assert_eq!(digest(&consumer), producer_hash);
    let consumed = recorded(
        &root,
        "isolated-ui-controls",
        consumer_command(
            &mut isolated(),
            ConsumerPaths {
                repository: &repo,
                project: &project,
                runtime: &runtime,
                producer: &producer,
                bundle: &bundle,
                evidence: &root,
            },
            &consumer,
            "relocated_ui_production_consumer",
        ),
    );
    assert!(consumed.contains("consumer-source-hidden=true consumer-runtime-hidden=true consumer-producer-hidden=true consumer-bundle-read-only=true"));
    assert!(consumed.contains("UI controls and visible negative passed"));
    receipt["commands"]
        .as_array_mut()
        .unwrap()
        .push("isolated-ui-controls".into());
    let app_producer = PathBuf::from(
        std::env::var_os("ORR_ARENA_UI_APP_TEST_PRODUCER")
            .expect("fresh pinned sample libtest producer required"),
    )
    .canonicalize()
    .unwrap();
    let app_consumer = root.join("app-ui-consumer");
    fs::copy(&app_producer, &app_consumer).unwrap();
    let app_producer_hash = digest(&app_producer);
    assert_eq!(digest(&app_consumer), app_producer_hash);
    fs::write(root.join("app-consumer-producer.json"), serde_json::to_vec_pretty(&serde_json::json!({"source_head":source_head,"source_tree":source_tree,"original":app_producer,"copied":app_consumer,"sha256":app_producer_hash,"test":"app::project_ui_tests::relocated_bundle_real_widgets_restart_original_scene"})).unwrap()).unwrap();
    let mut app_command = isolated();
    // Hide the independently located original producer too, without hiding the
    // retained copied executable or the evidence directory containing it.
    if !app_producer.starts_with(&repo) && !app_producer.starts_with(runtime.parent().unwrap()) {
        assert!(!root.starts_with(app_producer.parent().unwrap()));
        app_command
            .arg("--tmpfs")
            .arg(app_producer.parent().unwrap());
    }
    let restarted = recorded(
        &root,
        "isolated-app-restart",
        consumer_command(
            &mut app_command,
            ConsumerPaths {
                repository: &repo,
                project: &project,
                runtime: &runtime,
                producer: &app_producer,
                bundle: &bundle,
                evidence: &root,
            },
            &app_consumer,
            "app::project_ui_tests::relocated_bundle_real_widgets_restart_original_scene",
        ),
    );
    assert!(restarted.contains("consumer-source-hidden=true consumer-runtime-hidden=true consumer-producer-hidden=true consumer-bundle-read-only=true"));
    assert!(
        restarted.contains("Relocated App real-widget Restart restored original scene and replay")
    );
    receipt["commands"]
        .as_array_mut()
        .unwrap()
        .push("isolated-app-restart".into());
    let font_package = &manifest.payload.packages["korean-game-ui"];
    for asset in [
        "orr.package.json",
        "OrreryKoreanUI.otf",
        "OFL.txt",
        "COPYRIGHT.txt",
        "corpus.txt",
        "font-manifest.json",
    ] {
        let relative = format!(".orr/packages/objects/{font_package}/{asset}");
        assert_eq!(
            fs::read(project.join(&relative)).unwrap(),
            fs::read(bundle.join("project").join(&relative)).unwrap(),
            "font/license closure {asset}"
        );
    }
    for name in [
        "ui-negative-no-overlay.png",
        "ui-negative-no-text.png",
        "ui-glyphs-before.png",
        "ui-glyphs-after.png",
        "ui-title.png",
        "ui-playing.png",
        "ui-menu.png",
        "ui-continued.png",
        "ui-restart-action.png",
    ] {
        let path = root.join(name);
        let _ = png(&path); // decoded dimensions, not a file-presence assertion
        receipt["captures"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"path":name,"width":512,"height":512,"sha256":digest(&path)}));
    }
    assert_eq!(
        inventory(&project),
        source_files,
        "source remains unchanged after all consumers"
    );
    assert_eq!(
        inventory(&bundle),
        receipt["bundle_files"].as_array().unwrap().clone(),
        "read-only bundle remains unchanged"
    );
    // This receipt is separate from #279; module execution is not OS-window input.
    fs::write(root.join("ui-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "source_head":source_head,"source_tree":source_tree,"font_closure":true,"actual_app_dispatch":true,"controls_consumer":"production modules, headless software GPU; not OS-window input", "independent_no_ui_runtime_sha256":digest(&plain_runtime),"consumer_sha256":digest(&consumer),"app_consumer_sha256":app_producer_hash,"actual_app_restart":true,"visible_negative":"transparent text only; identical production HUD geometry, text-region differential", "text_oracle":"ui-text-oracle.json", "byte_identity_fixture":"separate parser/corpus-valid same-length metadata mutation unit test"
    })).unwrap()).unwrap();
    fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    stage("complete", true);
}

fn install_ui(project: &Path) {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    let installer = orr_package::Project::open_for_install(
        project,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    installer.install(&[package]).unwrap();
    let path = project.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["ui"] = serde_json::json!({"profile":"arena-korean-v1","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}});
    fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

#[path = "common/arena_ui_consumer.rs"]
mod ui_consumer;
#[test]
#[ignore = "child-only production module consumer; caller must source-hide and mount bundle read-only"]
fn relocated_ui_production_consumer() {
    let bundle =
        PathBuf::from(std::env::var_os("ORR_UI_CONSUMER_BUNDLE").expect("bundle required"));
    let evidence =
        PathBuf::from(std::env::var_os("ORR_UI_CONSUMER_EVIDENCE").expect("evidence required"));
    ui_consumer::consume(&bundle, &evidence);
}
