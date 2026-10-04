//! The editor window. `cargo run -p orr_editor --release`.
//!
//! The editor is a view: the simulation runs in a host, by default a thread
//! of this process (connected in-process), with `--connect` in another
//! process. Flags: see `--help` (`--game`, `--scene`, `--select`, `--play-ticks`,
//! `--script`, `--screenshot <png> --frames <n>`, `--erp <addr>` with
//! `--erp-token` or `--erp-dev`, `--connect <ws://host:port> [--token t]`).
use std::path::PathBuf;

use orr_editor::cli::{Args, USAGE};
use orr_editor::editor::Editor;
use orr_editor::game::EditorGame;
use orr_editor::{script, EditorApp, HostSpec, ScreenshotJob};
use orr_remote::{Auth, ServerConfig, TokenEntry};

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
    let spec = match &args.connect {
        Some(url) => HostSpec::remote(url, args.token.as_deref()),
        None => {
            let game = args.game.unwrap_or(EditorGame::PhysGame);
            let scene: PathBuf = args.scene.clone().unwrap_or_else(|| game.default_scene_path());
            // With --erp the same host thread also listens for agents: they share the window's document.
            let listen = args.erp.map(|bind| {
                let auth = if args.erp_dev {
                    Auth::DevNoAuth
                } else if args.erp_tokens.is_empty() {
                    fail("--erp needs --erp-token name:token:caps or --erp-dev");
                } else {
                    let tokens = args.erp_tokens.iter().map(|t| TokenEntry::parse(t).unwrap_or_else(|e| fail(&format!("--erp-token: {e}"))));
                    Auth::Tokens(tokens.collect())
                };
                let mut cfg = ServerConfig::new(auth);
                cfg.bind = bind;
                cfg
            });
            let spec = HostSpec::local_game(scene, game);
            match listen {
                Some(cfg) => spec.with_listener(cfg),
                None => spec,
            }
        }
    };
    let mut editor = Editor::start(&spec).unwrap_or_else(|e| fail(&e));
    if let (Some(_), Some((url, _))) = (args.erp, editor.erp_status()) {
        println!("ERP: {url}");
    }
    if let Some(name) = &args.select {
        editor.sync();
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
        editor.sync();
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([args.size.0, args.size.1]).with_title(editor.title()),
        ..Default::default()
    };
    let shot = args.screenshot.clone().map(|p| {
        let job = ScreenshotJob::new(p, args.frames);
        if args.screenshot_settle { job.with_settle() } else { job }
    });
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
