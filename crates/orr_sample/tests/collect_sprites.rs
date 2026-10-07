#![cfg(all(feature = "collect-sprites", feature = "project-export"))]
#![allow(clippy::disallowed_types)]
#[path = "common/collect_sprites.rs"]
mod common;
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot, SimControl};
use orr_sample::collect_game::{CollectInput, CollectRun, WON};
use orr_sample::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
use std::fs;
fn open(root: &std::path::Path) -> Result<PreparedProject, String> {
    PreparedProject::open_with_presentation(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
    )
}
#[test]
fn collect_sprite_admission_is_explicit_and_atomic() {
    let dir = tempfile::tempdir().unwrap();
    common::fixture(dir.path());
    assert!(PreparedProject::open(dir.path()).is_err());
    let p = open(dir.path()).unwrap();
    assert_eq!(p.sprites().unwrap().assets.len(), 1);
    let checksum = p.scene().frame().checksum();
    let original = fs::read(dir.path().join("view.json")).unwrap();
    for (field, value) in [
        ("source", serde_json::json!({"Clip":"missing"})),
        ("package", serde_json::json!("missing")),
        ("units_per_pixel", serde_json::json!(-1)),
    ] {
        common::change(dir.path().join("view.json"), |v| {
            v["bindings"]["e_00000001"][field] = value
        });
        assert!(open(dir.path()).is_err());
        fs::write(dir.path().join("view.json"), &original).unwrap();
    }
    common::change(dir.path().join("view.json"), |v| {
        let b = v["bindings"]["e_00000001"].take();
        v["bindings"]["e_99999999"] = b;
        v["bindings"].as_object_mut().unwrap().remove("e_00000001");
    });
    assert!(open(dir.path()).is_err());
    fs::write(dir.path().join("view.json"), &original).unwrap();
    common::change(dir.path().join("orr.project.json"), |v| {
        v["entry"].as_object_mut().unwrap().remove("sprites");
    });
    let primitive = open(dir.path()).unwrap();
    assert_eq!(primitive.scene().frame().checksum(), checksum);
    assert!(primitive.sprites().is_none());
}
#[test]
fn actual_snapshot_clip_seek_collect_hide_restart_and_source_loss() {
    let dir = tempfile::tempdir().unwrap();
    common::fixture(dir.path());
    let project = open(dir.path()).unwrap();
    let initial = project.scene().frame().checksum();
    let mut presentation = project.presentation();
    let mut bridge = InProc::new(
        PlayHost::new(project.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    fs::remove_dir_all(dir.path()).unwrap(); // All runtime assets and GUID index are owned.
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(presentation.sprites().len(), 2);
    assert_eq!(presentation.sprites()[0].instance.region, 10);
    bridge.step(36);
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(presentation.sprites()[0].instance.region, 11);
    let before = bridge.snapshot().unwrap().predicted().checksum();
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(bridge.snapshot().unwrap().predicted().checksum(), before);
    bridge.control(orr_session::ControlOp::Seek(0)).unwrap();
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(presentation.sprites()[0].instance.region, 10);
    bridge.control(orr_session::ControlOp::Play).unwrap();
    bridge
        .set_input(
            PlayerSlot(0),
            CollectInput {
                x: orr_fp::FP::ONE,
                ..Default::default()
            },
        )
        .unwrap();
    for _ in 0..120 {
        bridge.step(1);
        presentation.update(bridge.snapshot().as_ref()).unwrap();
    }
    assert_eq!(
        bridge
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .phase,
        WON
    );
    assert_eq!(presentation.sprites().len(), 1); // Hidden collected sprite, not stale GUID cache.
    bridge
        .set_input(
            PlayerSlot(0),
            CollectInput {
                buttons: orr_sample::collect_game::RESTART,
                ..Default::default()
            },
        )
        .unwrap();
    bridge.step(1);
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    assert_eq!(presentation.sprites().len(), 2);
    assert_eq!(presentation.sprites()[0].instance.region, 10);
    assert_eq!(project.scene().frame().checksum(), initial);
}
#[test]
fn collect_follow_hidden_target_holds_camera_and_locomotion_resets() {
    let dir = tempfile::tempdir().unwrap();
    common::fixture(dir.path());
    common::change(dir.path().join("view.json"), |v| {
        v["camera_follow"] = serde_json::json!("e_00000002");
        v["bindings"]["e_00000001"]["source"] =
            serde_json::json!({"Locomotion":{"idle":"idle","walk":"walk"}});
    });
    let project = open(dir.path()).unwrap();
    let mut p = project.presentation();
    let mut b = InProc::new(
        PlayHost::new(project.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    p.update(b.snapshot().as_ref()).unwrap();
    assert_eq!(p.camera.center, [10.0, 0.0]);
    b.set_input(
        PlayerSlot(0),
        CollectInput {
            x: orr_fp::FP::ONE,
            ..Default::default()
        },
    )
    .unwrap();
    for _ in 0..8 {
        b.step(1);
        p.update(b.snapshot().as_ref()).unwrap();
    }
    assert!(p.state("e_00000001").moving);
    assert_eq!(p.camera.center, [10.0, 0.0]);
    b.set_input(
        PlayerSlot(0),
        CollectInput {
            buttons: orr_sample::collect_game::RESTART,
            ..Default::default()
        },
    )
    .unwrap();
    b.step(1);
    p.update(b.snapshot().as_ref()).unwrap();
    assert!(!p.state("e_00000001").moving);
    assert_eq!(p.sprites()[0].instance.region, 10);
}
#[test]
fn rejects_other_scene_root_missing_camera_region_and_ui() {
    let dir = tempfile::tempdir().unwrap();
    common::fixture(dir.path());
    let view = fs::read(dir.path().join("view.json")).unwrap();
    for (field, value) in [
        ("scene", serde_json::json!("../outside.yaml")),
        ("project", serde_json::json!("..")),
        ("camera_follow", serde_json::json!("e_99999999")),
    ] {
        common::change(dir.path().join("view.json"), |v| v[field] = value);
        assert!(open(dir.path()).is_err());
        fs::write(dir.path().join("view.json"), &view).unwrap();
    }
    common::change(dir.path().join("view.json"), |v| {
        v["bindings"]["e_00000001"]["source"] = serde_json::json!({"Region":999})
    });
    assert!(open(dir.path()).is_err());
    fs::write(dir.path().join("view.json"), &view).unwrap();
    common::change(
        dir.path().join("orr.project.json"),
        |v| v["entry"]["ui"] = serde_json::json!({"profile":"arena-korean-v1","font":{"package":"sample-sprites","asset":"x.ttf"}}),
    );
    assert!(open(dir.path()).is_err());
}
#[test]
#[ignore = "mandatory explicit software-GPU installed-atlas proof"]
fn collect_real_atlas_pixels_read_only_runtime() {
    use orr_render::orr_rhi::{TextureFormat, Wgpu, WgpuOptions};
    use orr_sample::project_compositor::ProjectCompositor;
    let dir = tempfile::tempdir().unwrap();
    common::fixture(dir.path());
    let project = open(dir.path()).unwrap();
    let mut p = project.presentation();
    let b = InProc::new(
        PlayHost::new(project.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    p.update(b.snapshot().as_ref()).unwrap();
    assert_eq!(p.sprites().len(), 2);
    let gpu = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("mandatory software GPU");
    assert!(gpu.is_software());
    let size = (1024, 1024);
    let target =
        orr_render::OffscreenTarget::new(&gpu, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
    let mut c =
        ProjectCompositor::new(gpu.clone(), TextureFormat::Rgba8UnormSrgb, p.assets()).unwrap();
    fs::remove_dir_all(dir.path()).unwrap();
    c.draw(
        target.render_view(),
        size,
        p.shapes(),
        p.sprites(),
        &p.camera,
    )
    .unwrap();
    let rgba = target.read_rgba8();
    // Independent texel oracle: actual owned atlas bytes sampled at each opaque texel's world center.
    for draw in p.sprites() {
        let asset = &p.assets()[&draw.asset];
        let region = asset.document.region(draw.instance.region).unwrap();
        let mut checked = 0;
        for y in 0..region.height {
            for x in 0..region.width {
                let i =
                    (((region.y + y) * asset.document.atlas().width + region.x + x) * 4) as usize;
                if asset.rgba[i + 3] != 255 {
                    continue;
                }
                let at = p.camera.world_to_screen(
                    [
                        draw.instance.position[0]
                            + (x as f32 + 0.5 - region.width as f32 / 2.0) * 0.5,
                        draw.instance.position[1]
                            + (region.height as f32 / 2.0 - y as f32 - 0.5) * 0.5,
                    ],
                    size,
                );
                let j = ((at[1].floor() as u32 * size.0 + at[0].floor() as u32) * 4) as usize;
                for channel in 0..3 {
                    assert!(
                        rgba[j + channel].abs_diff(asset.rgba[i + channel]) <= 3,
                        "texel {x},{y} region{} channel{channel}: {} vs {}",
                        region.id,
                        rgba[j + channel],
                        asset.rgba[i + channel]
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 40);
    }
    if let Some(path) = std::env::var_os("ORR_COLLECT_SPRITE_CAPTURE") {
        let mut e = png::Encoder::new(fs::File::create(path).unwrap(), size.0, size.1);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&rgba).unwrap();
    }
}

#[test]
fn unbound_collect_camera_retains_previous_auto_framing() {
    let dir = tempfile::tempdir().unwrap();
    common::fixture(dir.path());
    common::change(dir.path().join("orr.project.json"), |v| {
        v["entry"].as_object_mut().unwrap().remove("sprites");
    });
    let project = open(dir.path()).unwrap();
    let mut p = project.presentation();
    let mut b = InProc::new(
        PlayHost::new(project.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    b.set_input(
        PlayerSlot(0),
        CollectInput {
            x: -orr_fp::FP::ONE,
            ..Default::default()
        },
    )
    .unwrap();
    for _ in 0..180 {
        b.step(1);
        p.update(b.snapshot().as_ref()).unwrap();
    }
    let snapshot = b.snapshot().unwrap();
    let expected = orr_sample::collect_view::scene_camera(snapshot.predicted());
    assert_eq!(p.camera.center, expected.center);
    assert_eq!(p.camera.half_extent, expected.half_extent);
}
