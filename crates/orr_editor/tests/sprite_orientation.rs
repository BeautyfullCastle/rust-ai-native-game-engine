//! Real editor authoring and composed-frame acceptance, separate from schema and
//! runtime tests. GPU tests are mandatory explicit lanes, never silent skips.
#![cfg(all(feature = "sprites", feature = "collect-dodge"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../../orr_sample/tests/common/sprite_orientation.rs"]
mod fixture;
use egui_kittest::{kittest::Queryable, Harness};
use fixture::*;
use orr_editor::{
    sprite_bindings::{Bindings, Orientation},
    Editor, EditorApp, HostSpec,
};
use orr_sample::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
use std::{
    fs,
    time::{Duration, Instant},
};

fn open_collect(root: &std::path::Path) -> PreparedProject {
    PreparedProject::open_with_presentation(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
    )
    .unwrap()
}
fn harness(fixture: &Fixture, collect: bool, gpu: bool) -> Harness<'static, EditorApp> {
    enum Project {
        Arena(orr_editor::project::PreparedProject),
        Collect(PreparedProject),
    }
    let prepared = if collect {
        Project::Collect(open_collect(&fixture.project))
    } else {
        Project::Arena(orr_editor::project::PreparedProject::open(&fixture.project).unwrap())
    };
    let spec = match &prepared {
        Project::Arena(project) => project.host_spec(),
        Project::Collect(project) => HostSpec::PreparedCollect {
            scene: project.path().into(),
            text: project.scene().text().into(),
            listen: None,
            debug_hooks: false,
        },
    };
    let mut editor = Editor::start(&spec).unwrap();
    editor.sync();
    editor.camera = orr_render::Camera::new(
        if collect { [5.0, 0.0] } else { [0.0, 0.0] },
        if collect { 40.0 } else { 170.0 },
    );
    assert!(editor.select_named(if collect { "player" } else { "hero" }));
    editor.sync();
    let build = move |cc: &mut eframe::CreationContext<'_>| {
        let mut app = match prepared {
            Project::Arena(project) => {
                project.into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx)
            }
            Project::Collect(project) => {
                let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
                orr_editor::project::install_collect_presentation(&mut app, project, &cc.egui_ctx);
                app
            }
        };
        app.sprites.package = PACKAGE.into();
        app.sprites.document = "sprites.json".into();
        app.sprites.source = orr_editor::sprite_bindings::Source::Region(REGION);
        app
    };
    if gpu {
        Harness::builder()
            .with_size([1500.0, 1600.0])
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::with_render_options(
                egui_wgpu::RendererOptions {
                    predictable_texture_filtering: false,
                    ..egui_wgpu::RendererOptions::PREDICTABLE
                },
            ))
            .build_eframe(build)
    } else {
        Harness::builder()
            .with_size([1500.0, 1600.0])
            .build_eframe(build)
    }
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.run_steps(1);
        let e = &h.state().editor;
        if e.yard_rows_coherent()
            && e.snapshot()
                .is_some_and(|s| s.timeline().is_some() == e.is_playing_mode())
        {
            break;
        }
        assert!(Instant::now() < deadline, "editor snapshot did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn document<'a>(h: &'a Harness<'_, EditorApp>) -> &'a orr_editor::sprite_bindings::Document {
    h.state().sprites.bindings.as_ref().unwrap().document()
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
    settle(h);
    assert!(
        h.state().sprites.error().is_none(),
        "{:?}",
        h.state().sprites.error()
    );
}
#[test]
fn orientation_history_restores_exact_legacy_bytes_and_reset_keeps_v3() {
    for version in [1, 2] {
        let fixture = Fixture::new(true);
        change(fixture.project.join("view.json"), |v| {
            v["version"] = serde_json::json!(version)
        });
        let path = fixture.project.join("view.json");
        let mut bindings = Bindings::open(path.clone()).unwrap();
        bindings.save().unwrap();
        let before = fs::read(&path).unwrap();
        let initial = bindings.document().clone();
        let guid = orr_reflect::Guid::parse("e_00000001").unwrap();
        let orientation = Orientation {
            quarter_turns: 1,
            flip_x: true,
            flip_y: false,
        };
        bindings.set_orientation(&guid, Some(orientation)).unwrap();
        assert_eq!(bindings.document().version, 3);
        bindings.undo();
        bindings.save().unwrap();
        assert_eq!(bindings.document(), &initial);
        assert_eq!(fs::read(&path).unwrap(), before);
        bindings.redo();
        let changed = bindings.document().clone();
        bindings.set_orientation(&guid, Some(orientation)).unwrap();
        bindings.undo();
        assert_eq!(
            bindings.document(),
            &initial,
            "repeating the same edit must not add history"
        );
        bindings.redo();
        let mut invalid = orientation;
        invalid.quarter_turns = 4;
        assert!(bindings.set_orientation(&guid, Some(invalid)).is_err());
        assert_eq!(bindings.document(), &changed);
        bindings.set_orientation(&guid, None).unwrap();
        assert_eq!(bindings.document().version, 3);
        assert!(bindings.document().bindings[guid.as_str()]
            .orientation
            .is_none());
        bindings.undo();
        assert_eq!(bindings.document(), &changed);
        bindings.redo();
        bindings.save().unwrap();
        let reopened = Bindings::open(path).unwrap();
        assert_eq!(reopened.document(), bindings.document());
    }
}
#[test]
fn real_widgets_reassignment_save_undo_reset_and_play_source_fences() {
    let fixture = Fixture::new(true);
    let scene = fs::read(fixture.project.join("scene.yaml")).unwrap();
    let mut h = harness(&fixture, true, false);
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let scene_history = h.state().editor.history().entries.len();
    let original = document(&h).clone();
    click(&mut h, "Sprite bindings (view only)");
    click(&mut h, "Selected sprite orientation");
    click(&mut h, "Mirror sprite X");
    click(&mut h, "Sprite quarter turns");
    click(&mut h, "90 degrees");
    let orientation = Orientation {
        quarter_turns: 1,
        flip_x: true,
        flip_y: false,
    };
    assert_eq!(
        document(&h).bindings["e_00000001"].orientation,
        Some(orientation)
    );
    assert_eq!(
        document(&h).bindings["e_00000002"],
        original.bindings["e_00000002"]
    );
    let changed = document(&h).clone();
    click(&mut h, "Save bindings");
    assert_eq!(
        open_collect(&fixture.project).sprites().unwrap().document,
        changed
    );
    // Reassigning the selected actor's art changes only the existing source/scale controls.
    click(&mut h, "Assign sprite to selection");
    assert_eq!(
        document(&h).bindings["e_00000001"].orientation,
        Some(orientation)
    );
    click(&mut h, "Reset sprite orientation");
    assert_eq!(document(&h).version, 3);
    assert!(document(&h).bindings["e_00000001"].orientation.is_none());
    click(&mut h, "Undo binding");
    assert_eq!(
        document(&h).bindings["e_00000001"].orientation,
        Some(orientation)
    );
    click(&mut h, "Redo binding");
    assert!(document(&h).bindings["e_00000001"].orientation.is_none());
    click(&mut h, "Undo binding");
    click(&mut h, "Save bindings");
    let authored = document(&h).clone();
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), scene_history);
    assert_eq!(fs::read(fixture.project.join("scene.yaml")).unwrap(), scene);
    // Every Play/pause/seek/Stop transition retains the authored orientation.
    for tick in [4, 7] {
        h.state_mut().editor.step(tick);
        h.state_mut().editor.sync();
        settle(&mut h);
        let a = h.state_mut();
        assert!(a.sprites.set_selected_orientation(&a.editor, None).is_err());
        assert_eq!(document(&h), &authored);
    }
    h.state_mut().editor.seek(0);
    h.state_mut().editor.sync();
    settle(&mut h);
    assert_eq!(document(&h), &authored);
    h.state_mut().editor.stop();
    h.state_mut().editor.sync();
    settle(&mut h);
    assert_eq!(document(&h), &authored);
    // A tampered installed image must not commit Reset or consume the current
    // history. An explicit reload reports failure but keeps the last-good view.
    let manager = orr_editor::sprite_bindings::open_project(&fixture.project).unwrap();
    let lock = manager.verify().unwrap();
    let image = fixture
        .project
        .join(".orr/packages/objects")
        .join(&lock.packages[PACKAGE].digest)
        .join("atlas.png");
    let before = fs::read(&image).unwrap();
    fs::write(&image, b"invalid package source").unwrap();
    let ctx = h.ctx.clone();
    {
        let a = h.state_mut();
        assert!(a.sprites.set_selected_orientation(&a.editor, None).is_err());
        assert!(a.sprites.reload(&ctx).is_err());
    }
    assert_eq!(document(&h), &authored);
    fs::write(&image, before).unwrap();
    {
        let a = h.state_mut();
        a.sprites.set_selected_orientation(&a.editor, None).unwrap();
        a.sprites.undo(&ctx).unwrap();
    }
    assert_eq!(document(&h), &authored);
    // Changing selection never applies the previous actor's orientation draft.
    h.state_mut().editor.select_named("first_collectible");
    h.state_mut().editor.sync();
    settle(&mut h);
    let other = Orientation {
        quarter_turns: 3,
        flip_x: false,
        flip_y: true,
    };
    {
        let a = h.state_mut();
        a.sprites
            .set_selected_orientation(&a.editor, Some(other))
            .unwrap();
    }
    assert_eq!(
        document(&h).bindings["e_00000001"].orientation,
        Some(orientation)
    );
    assert_eq!(document(&h).bindings["e_00000002"].orientation, Some(other));
    let selected_document = document(&h).clone();
    h.state_mut().editor.select_named("hazard");
    h.state_mut().editor.sync();
    settle(&mut h);
    {
        let app = h.state_mut();
        assert!(app
            .sprites
            .set_selected_orientation(&app.editor, None)
            .is_err());
        app.editor.select(None);
        assert!(app
            .sprites
            .set_selected_orientation(&app.editor, None)
            .is_err());
    }
    let another_scene = fixture.project.join("another.scene.yaml");
    fs::write(&another_scene, &scene).unwrap();
    assert!(h.state_mut().editor.open_path(&another_scene));
    h.state_mut().editor.sync();
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("player"));
    h.state_mut().editor.sync();
    settle(&mut h);
    {
        let app = h.state_mut();
        assert!(app
            .sprites
            .set_selected_orientation(&app.editor, None)
            .is_err());
    }
    assert_eq!(document(&h), &selected_document);
}
#[test]
#[ignore = "mandatory actual composed-editor non-square atlas software-GPU proof"]
fn all_orientations_render_asymmetric_non_square_actual_editor_texels() {
    for collect in [false, true] {
        let fixture = Fixture::new(collect);
        let mut h = harness(&fixture, collect, true);
        settle(&mut h);
        assert!(h.state().has_gpu());
        assert_eq!(h.ctx.pixels_per_point(), 1.0);
        let center = if collect { [0.0, 0.0] } else { [-60.0, 0.0] };
        let source_bytes = snapshot(&fixture.project);
        for orientation in combinations() {
            {
                let a = h.state_mut();
                a.sprites
                    .set_selected_orientation(&a.editor, Some(orientation))
                    .unwrap();
            }
            h.run_steps(3);
            settle(&mut h);
            let image = h.render().unwrap();
            let app = h.state();
            let rect = app.ui.viewport_rect.unwrap();
            assert_texels(orientation, fixture.scale, center, false, |world| {
                let screen = app.editor.camera.world_to_screen(world, app.ui.viewport_px);
                image
                    .get_pixel(
                        (rect.min.x + screen[0]).floor() as u32,
                        (rect.min.y + screen[1]).floor() as u32,
                    )
                    .0
            });
            if let Some(path) = std::env::var_os("ORR_ORIENTATION_CAPTURE_DIR") {
                let path = std::path::PathBuf::from(path);
                fs::create_dir_all(&path).unwrap();
                image
                    .save(path.join(format!(
                        "editor-{}-{}-{}-{}.png",
                        if collect { "collect" } else { "arena" },
                        orientation.quarter_turns,
                        orientation.flip_x,
                        orientation.flip_y
                    )))
                    .unwrap();
            }
        }
        assert_eq!(
            snapshot(&fixture.project),
            source_bytes,
            "orientation authoring is not an implicit Save"
        );
    }
}

#[cfg(all(
    feature = "image-reimport",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[test]
fn externally_reimported_source_requires_reload_before_orientation_transaction() {
    use orr_sample::image_reimport::{prepare, Consumer, Options};
    let fixture = Fixture::new(true);
    let orientation = Orientation {
        quarter_turns: 2,
        flip_x: false,
        flip_y: true,
    };
    fixture.orientation(Some(orientation));
    let mut h = harness(&fixture, true, false);
    settle(&mut h);
    let descriptor = document(&h).clone();
    let sidecar = fs::read(fixture.project.join("view.json")).unwrap();
    let replacement = fixture.temp.path().join("replacement.png");
    write_png(&replacement, &pixels(true));
    prepare(&Options {
        project: fixture.project.clone(),
        package: PACKAGE.into(),
        document: "sprites.json".into(),
        image: replacement,
        version: "1.0.1".into(),
        consumer: Consumer {
            collect: true,
            ui: false,
        },
    })
    .unwrap()
    .commit()
    .unwrap();
    {
        let a = h.state_mut();
        assert!(a
            .sprites
            .set_selected_orientation(&a.editor, None)
            .unwrap_err()
            .contains("source changed"));
    }
    assert_eq!(document(&h), &descriptor);
    assert_eq!(
        fs::read(fixture.project.join("view.json")).unwrap(),
        sidecar
    );
    let ctx = h.ctx.clone();
    {
        let a = h.state_mut();
        a.sprites.reload(&ctx).unwrap();
        a.sprites.set_selected_orientation(&a.editor, None).unwrap();
        a.sprites.undo(&ctx).unwrap();
    }
    assert_eq!(document(&h), &descriptor);
    assert_eq!(
        open_collect(&fixture.project).sprites().unwrap().document,
        descriptor
    );
}
