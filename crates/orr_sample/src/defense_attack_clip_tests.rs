use super::*;
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unit() -> game::DefenseUnit {
    game::DefenseUnit {
        active: 1,
        id: 1,
        role: game::RIVET,
        tier: 2,
        ..Default::default()
    }
}
fn ready() -> (egui::Context, PixelMotion) {
    let ctx = egui::Context::default();
    let mut skin = PixelMotion::default();
    skin.prepare(&ctx).unwrap();
    (ctx, skin)
}

#[test]
fn attack_clip_admits_unchanged_generated_rgba_and_exact_provenance() {
    let (_, skin) = ready();
    let asset = skin.attack.unwrap();
    assert_eq!(asset.document.regions().len(), 8);
    assert_eq!(asset.document.clips()[0].duration_ms(), 320);
    assert_eq!(asset.document.clips()[0].mode(), PlaybackMode::Once);
    assert_eq!((asset.pivot, asset.scale), ([222.0, 362.0], 0.184));
    assert_eq!(
        hex(&Sha256::digest(ATTACK_PNG)),
        "2248b8ee1c32eabc4ef54bf8975f215f304e5605c0d04f6b73504757ff83782f"
    );
    let provenance: serde_json::Value =
        serde_json::from_str(include_str!("../assets/stormglass-attack-clip/SOURCE.json")).unwrap();
    for (name, bytes) in [
        ("attack.png", ATTACK_PNG),
        ("sprites.json", ATTACK_JSON.as_bytes()),
        (
            "NOTICE.txt",
            include_bytes!("../assets/stormglass-attack-clip/NOTICE.txt").as_slice(),
        ),
        (
            "PROMPTS.txt",
            include_bytes!("../assets/stormglass-attack-clip/PROMPTS.txt").as_slice(),
        ),
    ] {
        assert_eq!(provenance["sha256"][name], hex(&Sha256::digest(bytes)));
    }
    let rgba = orr_sprite::png_import::decode_atlas(ATTACK_PNG, 1774, 887).unwrap();
    assert!(rgba.chunks_exact(4).any(|p| p[3] == 0));
    assert!(rgba.chunks_exact(4).any(|p| p[3] > 0));
    for region in asset.document.regions() {
        let mut visible = 0;
        for y in 0..region.height {
            for x in 0..region.width {
                let alpha = rgba[(((region.y + y) * 1774 + region.x + x) * 4 + 3) as usize];
                if alpha > 32 {
                    visible += 1;
                    assert!(x > 16 && x + 16 < region.width && y > 16 && y + 16 < region.height);
                }
            }
        }
        assert!(visible > 1000);
    }
}

#[test]
fn attack_clip_nearest_upload_is_exact_and_preparation_is_idempotent() {
    let (ctx, mut skin) = ready();
    let id = skin.test_attack_texture_id();
    skin.prepare(&ctx).unwrap();
    assert_eq!(skin.test_attack_texture_id(), id);
    let mut output = ctx.run_ui(Default::default(), |_| {});
    let (_, deltas) = output
        .textures_delta
        .set
        .iter()
        .find(|(texture, _)| **texture == id)
        .unwrap();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].options, TextureOptions::NEAREST);
    let egui::ImageData::Color(image) = &deltas[0].image;
    assert_eq!(image.size, [1774, 887]);
    let rgba = orr_sprite::png_import::decode_atlas(ATTACK_PNG, 1774, 887).unwrap();
    let expected: Vec<_> = rgba
        .chunks_exact(4)
        .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    assert_eq!(image.pixels, expected);
    output.textures_delta.clear();
}

#[test]
fn attack_clip_uses_post_release_session_ticks_once_and_fixed_pivot() {
    let (ctx, skin) = ready();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let xs = [0, 444, 887, 1330];
    for age in 0..=20 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    skin.paint_release(ui.painter(), pos2(250.0, 400.0), 1.0, &unit(), age, false),
                    age < 20
                );
            },
        );
        let meshes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(m) if m.texture_id == skin.test_attack_texture_id() => Some(m),
                _ => None,
            })
            .collect();
        if age < 20 {
            assert_eq!(meshes.len(), 1);
            let region = (age * 1000 / 60 / 40) as usize;
            assert_eq!(
                meshes[0].vertices[0].uv,
                pos2(
                    xs[region % 4] as f32 / 1774.0,
                    if region < 4 { 0.0 } else { 444.0 / 887.0 }
                )
            );
            let origin = meshes[0].vertices[0].pos;
            assert!((origin.x - (250.0 - 222.0 * 0.184)).abs() < 0.001);
            assert!((origin.y - (420.0 - 362.0 * 0.184)).abs() < 0.001);
        } else {
            assert!(meshes.is_empty());
        }
        output.textures_delta.clear();
    }
}

#[test]
fn attack_clip_rejects_missing_asset_inactive_other_roles_and_expired_age() {
    let (ctx, skin) = ready();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        for value in [
            game::DefenseUnit {
                active: 0,
                ..unit()
            },
            game::DefenseUnit {
                role: game::MORTAR,
                ..unit()
            },
        ] {
            assert!(!skin.paint_release(ui.painter(), Pos2::ZERO, 1.0, &value, 0, false));
        }
        assert!(!PixelMotion::default().paint_release(
            ui.painter(),
            Pos2::ZERO,
            1.0,
            &unit(),
            0,
            false
        ));
        assert!(!skin.paint_release(ui.painter(), Pos2::ZERO, 1.0, &unit(), u64::MAX, false));
    });
    assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == skin.test_attack_texture_id())));
    output.textures_delta.clear();
}

#[test]
fn attack_clip_mirrors_uv_about_the_same_foot_anchor_for_every_frame() {
    let (ctx, skin) = ready();
    let asset = skin.attack.as_ref().unwrap();
    for (region_index, age) in [0, 3, 5, 8, 10, 12, 15, 17].into_iter().enumerate() {
        let region = asset.document.region(region_index as u32).unwrap();
        let uv = asset.document.uv_rect(region_index as u32).unwrap();
        for flip in [false, true] {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                assert!(skin.paint_release(
                    ui.painter(),
                    pos2(250.0, 400.0),
                    1.0,
                    &unit(),
                    age,
                    flip
                ));
            });
            let mesh = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Mesh(m) if m.texture_id == skin.test_attack_texture_id() => {
                        Some(m)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                mesh.vertices[0].uv,
                pos2(if flip { uv[2] } else { uv[0] }, uv[1])
            );
            assert_eq!(
                mesh.vertices[1].uv,
                pos2(if flip { uv[0] } else { uv[2] }, uv[1])
            );
            let pivot = if flip {
                region.width as f32 - 222.0
            } else {
                222.0
            };
            assert!((mesh.vertices[0].pos.x + pivot * 0.184 - 250.0).abs() < 0.001);
            assert!((mesh.vertices[0].pos.y + 362.0 * 0.184 - 420.0).abs() < 0.001);
            output.textures_delta.clear();
        }
    }
}
