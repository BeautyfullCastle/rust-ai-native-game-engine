//! The editor window. `cargo run -p orr_editor --release`.
//!
//! Flags: see `--help` (`--scene`, `--select`, `--play-ticks`, `--script`,
//! `--screenshot <png> --frames <n>`).
use std::path::PathBuf;

use orr_editor::cli::{Args, USAGE};
use orr_editor::editor::{default_scene_path, Editor};
use orr_editor::{script, EditorApp, ScreenshotJob};

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(2);
}

fn main() {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(m) => {
            eprintln!("{m}");
            std::process::exit(if m == USAGE { 0 } else { 2 });
        }
    };
    let path: PathBuf = args.scene.clone().unwrap_or_else(default_scene_path);
    let mut editor = Editor::open(&path).unwrap_or_else(|e| fail(&e));
    if let Some(name) = &args.select {
        if !editor.select_named(name) {
            fail(&format!("--select: no entity named '{name}'"));
        }
    }
    if let Some(file) = &args.script {
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| fail(&format!("{}: {e}", file.display())));
        if let Err(e) = script::run_script(&mut editor, &text) {
            fail(&e);
        }
    }
    if let Some(n) = args.play_ticks {
        editor.step(n);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([args.size.0, args.size.1]).with_title(editor.title()),
        ..Default::default()
    };
    let shot = args.screenshot.clone().map(|p| ScreenshotJob::new(p, args.frames));
    let result = eframe::run_native(
        "Orrery Editor",
        options,
        Box::new(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            if let Some(job) = shot {
                app = app.with_screenshot(job);
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        fail(&format!("eframe: {e}"));
    }
}
