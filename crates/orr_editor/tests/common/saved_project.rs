//! Saved-project acceptance helpers. Runtime opening never installs a package.
#![allow(dead_code, clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "arena_sprites.rs"]
mod editor_ui;
pub use editor_ui::{
    body_position, click, document, key, moving, no_error, region, settle, wait_for, HERO, TARGET,
};

use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, project::PreparedProject, Editor, EditorApp};
use std::path::{Path, PathBuf};

pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "fixture must contain real files, not source links"
        );
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            assert!(kind.is_file());
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub fn tempdir() -> tempfile::TempDir {
    let root = std::env::temp_dir().canonicalize().unwrap();
    tempfile::tempdir_in(root).unwrap()
}

pub struct SavedFixture {
    pub root: tempfile::TempDir,
}
impl SavedFixture {
    pub fn new() -> Self {
        let root = tempdir();
        copy_tree(
            &repository().join("assets/saved_arena_project"),
            root.path(),
        );
        assert!(root.path().join("orr.packages.lock.json").is_file());
        assert!(root.path().join(".orr/packages/objects").is_dir());
        Self { root }
    }
    pub fn scene(&self) -> PathBuf {
        self.root.path().join("arena.scene.yaml")
    }
    pub fn sidecar(&self) -> PathBuf {
        self.root.path().join("arena.sprites.json")
    }
    pub fn headless(&self) -> Harness<'static, EditorApp> {
        open_headless(self.root.path())
    }
    /// Remove the initial location, rather than retaining a fallback directory.
    pub fn relocate(self) -> Self {
        let moved = tempdir();
        copy_tree(self.root.path(), moved.path());
        let old = self.root.path().to_owned();
        self.root.close().unwrap();
        assert!(!old.exists());
        Self { root: moved }
    }
}

pub fn prepare(root: &Path) -> (PreparedProject, Editor) {
    let prepared = PreparedProject::open(root).unwrap();
    assert_eq!(prepared.root(), root.canonicalize().unwrap());
    assert_eq!(
        prepared.scene_path(),
        root.join("arena.scene.yaml").canonicalize().unwrap()
    );
    let mut editor = Editor::start(&prepared.host_spec()).unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Arena);
    assert_eq!(editor.rows().len(), 2);
    // Same deliberate framing as the existing Arena sprite acceptance fixture.
    editor.camera = orr_render::Camera::new([0.0, 0.0], 170.0);
    (prepared, editor)
}

pub fn open_headless(root: &Path) -> Harness<'static, EditorApp> {
    let (prepared, editor) = prepare(root);
    Harness::builder()
        .with_size([1500.0, 1400.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| prepared.into_app(editor, None, &cc.egui_ctx))
}

pub fn open_gpu(root: &Path) -> Harness<'static, EditorApp> {
    let (prepared, editor) = prepare(root);
    Harness::builder()
        .with_size([1500.0, 1400.0])
        .with_step_dt(1.0 / 60.0)
        // PREDICTABLE's manual bilinear filter ignores NEAREST samplers. Honor
        // the production sprite sampler so opaque atlas texels stay opaque.
        .with_render_options(egui_wgpu::RendererOptions {
            predictable_texture_filtering: false,
            ..egui_wgpu::RendererOptions::PREDICTABLE
        })
        .wgpu()
        .build_eframe(move |cc| {
            prepared.into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx)
        })
}

pub fn assert_loaded(h: &Harness<'_, EditorApp>, follow: &str) {
    no_error(h);
    assert_eq!(h.state().editor.game(), EditorGame::Arena);
    assert!(h.state().sprites.scene_matches(&h.state().editor));
    assert_eq!(document(h).bindings.len(), 2);
    assert_eq!(document(h).camera_follow.as_deref(), Some(follow));
    assert_eq!(region(h.state(), HERO), Some(10));
    assert_eq!(region(h.state(), TARGET), Some(20));
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
}

/// Stop's ERP acknowledgement may precede its displayed Edit frame. Wait for
/// the actual camera lifecycle boundary, not an arbitrary count of UI passes.
pub fn wait_stopped(h: &mut Harness<'_, EditorApp>, camera: orr_render::Camera) {
    wait_for(
        h,
        "coherent stopped Edit frame and restored camera",
        |app| {
            app.editor.mode() == orr_editor::Mode::Edit
                && app
                    .editor
                    .snapshot()
                    .is_some_and(|snapshot| snapshot.timeline().is_none())
                && app.editor.yard_rows_coherent()
                && app.editor.camera == camera
        },
    );
}

pub fn open_inspector(h: &mut Harness<'_, EditorApp>, entity: &str) {
    click(h, entity);
    click(h, "Sprite bindings (view only)");
}

pub fn edit_hero_x(h: &mut Harness<'_, EditorApp>) {
    use egui::accesskit::Role;
    click(h, "hero");
    let field =
        h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some("-60"));
    field.focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some("-60"))
        .type_text("-48");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    settle(h);
    assert_eq!(body_position(h.state(), HERO), [-48.0, 0.0]);
    assert!(h.state().editor.is_dirty());
}

/// Isolated subprocess: no global cwd changes in parallel libtests. With the
/// evidence flag, Linux bubblewrap hides the *entire* repository, including the
/// source art and checked-in fixture. No production fallback can read them.
pub fn relocated_child(root: &Path, test: &str, checksum: u64) {
    let work = tempdir();
    let executable = work.path().join("saved-project-test");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let empty_cwd = work.path().join("empty-cwd");
    std::fs::create_dir(&empty_cwd).unwrap();
    let isolate = std::env::var_os("ORR_REQUIRE_PROJECT_ISOLATION").is_some();
    let mut command = if isolate {
        match std::env::consts::OS {
            "linux" => {}
            unsupported => {
                panic!("repository-hidden evidence needs Linux bubblewrap; got {unsupported}")
            }
        }
        let mut command = std::process::Command::new("bwrap");
        command
            .args(["--ro-bind", "/", "/", "--bind"])
            .arg(root)
            .arg(root)
            .arg("--bind")
            .arg(work.path())
            .arg(work.path())
            .arg("--tmpfs")
            .arg(repository());
        if let Some(captures) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
            let captures = PathBuf::from(captures);
            std::fs::create_dir_all(&captures).unwrap();
            let captures = captures.canonicalize().unwrap();
            command.args(["--bind"]).arg(&captures).arg(&captures);
        }
        // The executable follows all namespace options.
        command.arg("--").arg(&executable);
        command
    } else {
        std::process::Command::new(&executable)
    };
    let output = command
        .args(["--exact", test, "--nocapture"])
        .current_dir(&empty_cwd)
        .env("ORR_SAVED_PROJECT_CHILD", root)
        .env("ORR_SAVED_PROJECT_CHECKSUM", checksum.to_string())
        .env("ORR_SAVED_PROJECT_REPOSITORY", repository())
        .output()
        .expect("start isolated relocated-project test process");
    eprintln!(
        "relocated child stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    eprintln!(
        "relocated child stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "relocated child failed: {}",
        output.status
    );
}

pub fn child_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("ORR_SAVED_PROJECT_CHILD")?);
    assert!(
        !Path::new("assets").exists(),
        "child cwd cannot expose repository assets"
    );
    if std::env::var_os("ORR_REQUIRE_PROJECT_ISOLATION").is_some() {
        let repo = PathBuf::from(std::env::var_os("ORR_SAVED_PROJECT_REPOSITORY").unwrap());
        assert!(!repo.join("assets/sprite_demo").exists());
        assert!(!repo.join("assets/saved_arena_project").exists());
        assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).exists());
    }
    Some(root)
}

pub fn child_checksum() -> u64 {
    std::env::var("ORR_SAVED_PROJECT_CHECKSUM")
        .unwrap()
        .parse()
        .unwrap()
}
