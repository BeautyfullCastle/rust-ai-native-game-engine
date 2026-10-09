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
    // All project files, active package bytes and presentation references are
    // admitted before constructing any host or window. Keep this owned candidate
    // until the actual egui context is available for texture restoration.
    #[cfg(feature = "sprites")]
    let project = args.project.as_ref().map(|root| {
        orr_editor::project::PreparedProject::open(root)
            .unwrap_or_else(|error| fail(&format!("--project: {error}")))
    });
    #[cfg(feature = "collect-dodge")]
    let collect_project = args.collect_project.as_ref().map(|root| orr_sample::collect_project::PreparedProject::open_with_audio(root, orr_sample::collect_project::ProgressSupport::MetadataOnly, if cfg!(feature="sprites") { orr_sample::collect_project::SpriteSupport::Supported } else { orr_sample::collect_project::SpriteSupport::Unsupported }, cfg!(feature="collect-ui"), cfg!(feature="collect-audio")).unwrap_or_else(|error| fail(&format!("--collect-project: {error}"))));
    #[cfg(feature="room-project")]
    let room_project=args.room_project.as_ref().map(|root|orr_sample::room_project::PreparedProject::open_with_capabilities(root, cfg!(feature="room-ui"), if cfg!(all(feature="room-checkpoint",target_os="linux")) { orr_sample::room_project::CheckpointSupport::MetadataOnly } else { orr_sample::room_project::CheckpointSupport::Disabled }, cfg!(feature="room-character")).unwrap_or_else(|error|fail(&format!("--room-project: {error}"))));
    #[cfg(feature = "navigation-project")]
    let navigation_project = args.navigation_project.as_ref().map(|root| orr_sample::navigation_project::PreparedProject::open(root).unwrap_or_else(|error| fail(&format!("--navigation-project: {error}"))));
    let spec = match &args.connect {
        Some(url) => HostSpec::remote(url, args.token.as_deref()),
        None => {
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
            let standalone = || {
                let game = args.game.unwrap_or(EditorGame::PhysGame);
                let scene: PathBuf = args.scene.clone().unwrap_or_else(|| game.default_scene_path());
                HostSpec::local_game(scene, game)
            };
            #[cfg(feature = "sprites")]
            let spec = project.as_ref().map_or_else(standalone, |project| project.host_spec());
            #[cfg(not(feature = "sprites"))]
            let spec = standalone();
            #[cfg(feature = "collect-dodge")]
            let spec = collect_project.as_ref().map_or(spec.clone(), |project| HostSpec::PreparedCollect {
                scene: project.path().to_path_buf(), text:project.scene().text().to_owned(), listen:None,debug_hooks:false,
            });
            #[cfg(feature="room-project")]
            let spec=room_project.as_ref().map_or(spec.clone(),|project|HostSpec::PreparedRoom {
                scene:project.path().to_path_buf(),text:project.scene().text().to_owned(),listen:None,debug_hooks:false,
            });
            #[cfg(feature = "navigation-project")]
            let spec = navigation_project.as_ref().map_or(spec.clone(), |project| HostSpec::PreparedNavigation {
                project_root: project.root().to_path_buf(), reload: false, scene: project.path().to_path_buf(), text: project.scene().text().to_owned(), terrain_bytes: project.terrain_bytes().to_vec(), listen: None, debug_hooks: false,
            });
            match listen {
                Some(cfg) => spec.with_listener(cfg),
                None => spec,
            }
        }
    };
    let mut editor = Editor::start(&spec).unwrap_or_else(|e| fail(&e));
    #[cfg(feature = "navigation-project")]
    if let Some(project) = &navigation_project {
        editor.camera3d = orr_sample::navigation_project_view::NavigationCamera::new(orr_bridge::FrameView::of(project.scene().frame())).unwrap_or_else(|error| fail(&error)).orbit;
    }
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
            #[cfg(feature = "sprites")]
            let mut app = match project {
                Some(project) => project.into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx),
                None => EditorApp::new(editor, cc.wgpu_render_state.clone()),
            };
            #[cfg(not(feature = "sprites"))]
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            #[cfg(all(feature="sprites",feature="collect-dodge"))]
            if let Some(project) = collect_project {
                orr_editor::project::install_collect_presentation(&mut app, project, &cc.egui_ctx);
            }
            #[cfg(feature="room-project")]
            if let Some(mut project)=room_project {
                #[cfg(all(feature="room-checkpoint",target_os="linux"))]
                { app.room_checkpoint = Some(orr_editor::room_checkpoint_panel::Panel::new(&project, &app.editor).unwrap_or_else(|error|fail(&error))); }
                #[cfg(feature="room-ui")]
                if let Some(ui) = project.take_ui() {
                    let mut panel = orr_editor::collect_ui_panel::Panel::new_room(ui);
                    panel.bind_room(&app.editor).unwrap_or_else(|error|fail(&error));
                    app.collect_ui = Some(panel);
                }
                if let Some(camera)=project.take_camera() {
                    app.editor.install_room_camera(camera.document.clone()).unwrap_or_else(|error|fail(&error));
                    app.room_camera=Some(orr_editor::room_camera_panel::Panel::new(camera));
                }
                #[cfg(feature="room-character")]
                if let Some(character) = project.take_character() {
                    app.editor.install_room_character(character.document.clone()).unwrap_or_else(|error|fail(&error));
                    app.room_character = Some(orr_editor::room_character_panel::Panel::new(character));
                }
                let (_,_,_,models)=project.into_parts();
                app.models.install_room(models).unwrap_or_else(|error|fail(&format!("room presentation: {error}")));
            }
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
