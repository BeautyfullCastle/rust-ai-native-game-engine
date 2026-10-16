#![cfg(all(feature = "collect-dodge", feature = "sprites"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../../orr_sample/tests/common/collect_sprites.rs"]
mod common;
use egui_kittest::Harness;
use orr_editor::{Editor, EditorApp, HostSpec};
use orr_sample::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
use std::time::{Duration, Instant};
fn open(root: &std::path::Path) -> PreparedProject {
    PreparedProject::open_with_presentation(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
    )
    .unwrap()
}
fn editor(p: &PreparedProject) -> Editor {
    let mut e = Editor::start(&HostSpec::PreparedCollect {
        scene: p.path().into(),
        text: p.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    e.sync();
    e
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.run_steps(1);
        if h.state().editor.yard_rows_coherent()
            && h.state()
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == h.state().editor.is_playing_mode())
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn collect_sprite_author_save_undo_reopen_and_tick_phase() {
    let dir = common::tempdir();
    common::fixture(dir.path());
    let p = open(dir.path());
    let checksum = p.scene().frame().checksum();
    let e = editor(&p);
    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .build_eframe(|cc| {
            let mut a = EditorApp::new(e, None);
            orr_editor::project::install_collect_presentation(&mut a, p, &cc.egui_ctx);
            a
        });
    settle(&mut h);
    assert_eq!(
        h.state()
            .sprites
            .sampled_region(&h.state().editor, "e_00000001"),
        Some(10)
    );
    let ctx = h.ctx.clone();
    {
        let a = h.state_mut();
        assert!(a.editor.select_named("player"));
        a.editor.sync();
        a.sprites.package = "sample-sprites".into();
        a.sprites.document = "sprites.json".into();
        a.sprites.source = orr_editor::sprite_bindings::Source::Region(21);
        a.sprites.scale = 0.5;
        a.sprites.assign(&ctx, &a.editor).unwrap();
        assert_eq!(a.sprites.sampled_region(&a.editor, "e_00000001"), Some(21));
        a.sprites.undo(&ctx).unwrap();
        assert_eq!(a.sprites.sampled_region(&a.editor, "e_00000001"), Some(10));
        a.sprites.redo(&ctx).unwrap();
        a.sprites.bindings.as_mut().unwrap().save().unwrap();
        assert_eq!(a.editor.checksum(), checksum);
    }
    assert_eq!(
        open(dir.path()).sprites().unwrap().document.bindings["e_00000001"].source,
        orr_sample::project_sprites::Source::Region(21)
    );
    {
        let a = h.state_mut();
        a.sprites.source = orr_editor::sprite_bindings::Source::Clip("idle".into());
        a.sprites.assign(&ctx, &a.editor).unwrap();
        a.sprites.bindings.as_mut().unwrap().save().unwrap();
        a.editor.step(36);
        a.editor.sync();
    }
    settle(&mut h);
    assert_eq!(
        h.state()
            .sprites
            .sampled_region(&h.state().editor, "e_00000001"),
        Some(11)
    );
    h.run_steps(5);
    assert_eq!(
        h.state()
            .sprites
            .sampled_region(&h.state().editor, "e_00000001"),
        Some(11)
    );
    h.state_mut().editor.seek(0);
    h.state_mut().editor.sync();
    settle(&mut h);
    assert_eq!(
        h.state()
            .sprites
            .sampled_region(&h.state().editor, "e_00000001"),
        Some(10)
    );
}
#[test]
#[ignore = "mandatory composed EditorApp installed-atlas framebuffer proof"]
fn collect_sprite_actual_editor_pixels() {
    let dir = common::tempdir();
    common::fixture(dir.path());
    let p = open(dir.path());
    let e = editor(&p);
    let owned = p.sprites().unwrap().assets.clone();
    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .renderer(egui_kittest::wgpu::WgpuTestRenderer::with_render_options(
            egui_wgpu::RendererOptions {
                predictable_texture_filtering: false,
                ..egui_wgpu::RendererOptions::PREDICTABLE
            },
        ))
        .build_eframe(|cc| {
            let mut a = EditorApp::new(e, cc.wgpu_render_state.clone());
            orr_editor::project::install_collect_presentation(&mut a, p, &cc.egui_ctx);
            a
        });
    settle(&mut h);
    h.run_steps(3);
    assert_eq!(
        h.ctx.pixels_per_point(),
        1.0,
        "texel oracle uses one physical pixel per egui point"
    );
    assert!(h.state().has_gpu());
    let image = h.render().unwrap();
    // Inspect the composed egui output, not the primitive viewport-only texture.
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let mut checked = 0;
    for (guid, binding) in &app.sprites.bindings.as_ref().unwrap().document().bindings {
        let row = app
            .editor
            .rows()
            .iter()
            .find(|r| r.guid.as_ref().is_some_and(|g| g.as_str() == guid))
            .unwrap();
        let body = app
            .editor
            .bodies()
            .iter()
            .find(|b| b.entity == row.entity)
            .unwrap();
        let asset = &owned[&(binding.package.clone(), binding.document.clone())];
        let region = asset
            .document
            .region(binding.region(&asset.document, 0).unwrap())
            .unwrap();
        for y in 0..region.height {
            for x in 0..region.width {
                let i =
                    (((region.y + y) * asset.document.atlas().width + region.x + x) * 4) as usize;
                if asset.rgba[i + 3] != 255 {
                    continue;
                }
                let at = app.editor.camera.world_to_screen(
                    [
                        body.pos[0]
                            + (x as f32 + 0.5 - region.width as f32 / 2.0)
                                * binding.units_per_pixel,
                        body.pos[1]
                            + (region.height as f32 / 2.0 - y as f32 - 0.5)
                                * binding.units_per_pixel,
                    ],
                    app.ui.viewport_px,
                );
                let px = image.get_pixel(
                    (rect.min.x + at[0]).floor() as u32,
                    (rect.min.y + at[1]).floor() as u32,
                );
                for c in 0..3 {
                    assert!(
                        px[c].abs_diff(asset.rgba[i + c]) <= 4,
                        "guid{guid} texel{x},{y} channel{c}: {} vs {}",
                        px[c],
                        asset.rgba[i + c]
                    );
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 80);
    if let Some(path) = std::env::var_os("ORR_COLLECT_EDITOR_CAPTURE") {
        image.save(path).unwrap();
    }
}
