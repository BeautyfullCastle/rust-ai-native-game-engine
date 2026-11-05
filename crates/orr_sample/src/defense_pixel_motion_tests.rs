use super::*;
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn ready() -> (egui::Context, PixelMotion) {
    let ctx = egui::Context::default();
    let mut skin = PixelMotion::default();
    skin.prepare(&ctx).unwrap();
    (ctx, skin)
}
fn run(tick: u32) -> game::DefenseRun {
    game::DefenseRun {
        tick,
        phase: game::PLAYING,
        ..Default::default()
    }
}
fn unit() -> game::DefenseUnit {
    game::DefenseUnit {
        active: 1,
        role: game::RIVET,
        tier: 3,
        id: 1,
        ..Default::default()
    }
}
#[test]
fn admitted_asset_bytes_geometry_timing_provenance_and_endpoints_are_unchanged() {
    assert_eq!(
        hex(&Sha256::digest(PNG)),
        "efcfb8bc0acbffb76c230846c111ba561d74cd6970ac4e2a721d4e981b6c5b56"
    );
    let (_, skin) = ready();
    let a = skin.asset.unwrap();
    assert_eq!(a.document.clips()[0].mode(), PlaybackMode::Once);
    assert_eq!(a.document.clips()[0].duration_ms(), 1400);
    assert_eq!(a.document.regions().len(), 14);
    assert_eq!(PIVOT, [48.0, 85.0]);
    let source: serde_json::Value = serde_json::from_str(include_str!(
        "../assets/stormglass-pixel-motion/SOURCE.json"
    ))
    .unwrap();
    for (name, bytes) in [
        ("atlas.png", PNG),
        ("sprites.json", JSON.as_bytes()),
        (
            "LICENSE.txt",
            crate::defense_app::PIXEL_MOTION_LICENSES.as_bytes(),
        ),
        (
            "orr.package.json",
            include_bytes!("../assets/stormglass-pixel-motion/orr.package.json").as_slice(),
        ),
    ] {
        assert_eq!(source["sha256"][name], hex(&Sha256::digest(bytes)));
    }
    assert_eq!(source["geometry"]["pivot"], serde_json::json!([48, 85]));
    let rgba = orr_sprite::png_import::decode_atlas(PNG, 672, 224).unwrap();
    for y in 0..112 {
        let first = y * 672 * 4;
        let last = ((112 + y) * 672 + 576) * 4;
        assert_eq!(&rgba[first..first + 96 * 4], &rgba[last..last + 96 * 4]);
    }
}
#[test]
fn nearest_straight_rgba_upload_is_exact_and_prepare_is_idempotent() {
    let (ctx, mut skin) = ready();
    let id = skin.asset.as_ref().unwrap().texture.id();
    skin.prepare(&ctx).unwrap();
    assert_eq!(skin.asset.as_ref().unwrap().texture.id(), id);
    let mut output = ctx.run_ui(Default::default(), |_| {});
    let uploads: Vec<_> = output
        .textures_delta
        .set
        .iter()
        .filter(|(key, _)| **key == id)
        .collect();
    assert_eq!(uploads.len(), 1);
    let deltas = &uploads[0].1;
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].options, TextureOptions::NEAREST);
    assert!(deltas[0].pos.is_none());
    let rgba = orr_sprite::png_import::decode_atlas(PNG, 672, 224).unwrap();
    assert!(rgba.chunks_exact(4).any(|p| p[3] == 0));
    assert!(rgba.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
    let egui::ImageData::Color(upload) = &deltas[0].image;
    assert_eq!(upload.size, [672, 224]);
    let expected: Vec<_> = rgba
        .chunks_exact(4)
        .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    assert_eq!(upload.pixels, expected);
    output.textures_delta.clear();
}
#[test]
fn complete_visual_cycle_preserves_neutral_hold_and_snapshot_clock() {
    let (_, skin) = ready();
    let clip = &skin.asset.as_ref().unwrap().document.clips()[0];
    for tick in 0..168 {
        let elapsed = PixelMotion::elapsed(&run(tick), true);
        assert_eq!(clip.sample(elapsed).frame_index, (tick % 84 / 6) as usize);
        assert!(!clip.sample(elapsed).finished);
        assert_eq!(PixelMotion::elapsed(&run(tick), false), 0);
    }
    assert_eq!(clip.sample(PixelMotion::elapsed(&run(78), true)).region, 13);
    assert_eq!(clip.sample(PixelMotion::elapsed(&run(83), true)).region, 13);
    assert_eq!(clip.sample(PixelMotion::elapsed(&run(84), true)).region, 0);
    assert_eq!(clip.sample(PixelMotion::elapsed(&run(89), true)).region, 0);
    assert_eq!(PixelMotion::elapsed(&run(0), true), 0); // Real restart run tick.
    assert_eq!(PixelMotion::elapsed(&run(41), true), 683); // Restored snapshot.
    assert_eq!(
        PixelMotion::elapsed(
            &game::DefenseRun {
                phase: game::LOST,
                ..run(41)
            },
            true
        ),
        0
    );
    assert_eq!(
        PixelMotion::elapsed(&run(u32::MAX), true),
        (u64::from(u32::MAX) * 1000 / 60) % 1400
    );
}
#[test]
fn rivet_skin_uses_fixed_pivot_uv_and_readable_role_tier_badge_only() {
    let (ctx, skin) = ready();
    let id = skin.asset.as_ref().unwrap().texture.id();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let center = pos2(250.0, 400.0);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            let p = ui.painter().with_clip_rect(screen);
            assert!(skin.paint_unit(&p, center, 1.0, &unit(), &run(48), true));
            for role in 1..6 {
                assert!(!skin.paint_unit(
                    &p,
                    center,
                    1.0,
                    &game::DefenseUnit { role, ..unit() },
                    &run(48),
                    true
                ));
            }
            assert!(!skin.paint_unit(
                &p,
                center,
                1.0,
                &game::DefenseUnit {
                    active: 0,
                    ..unit()
                },
                &run(48),
                true
            ));
            assert!(!PixelMotion::default().paint_unit(&p, center, 1.0, &unit(), &run(48), true));
        },
    );
    let meshes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id == id => Some((s.clip_rect, m)),
            _ => None,
        })
        .collect();
    assert_eq!(meshes.len(), 1);
    let (clip, mesh) = meshes[0];
    assert_eq!(clip, screen);
    assert_eq!(mesh.vertices.len(), 4);
    let bounds = mesh.calc_bounds();
    assert_eq!(
        bounds.min,
        center + vec2(0.0, 20.0) - vec2(48.0, 85.0) * SCALE
    );
    assert!((bounds.width() - 96.0 * SCALE).abs() < 0.001);
    assert!((bounds.height() - 112.0 * SCALE).abs() < 0.001);
    assert_eq!(mesh.vertices[0].uv, pos2(96.0 / 672.0, 0.5));
    assert!(mesh.vertices.iter().all(|v| v.color == Color32::WHITE));
    assert!(output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "Rivet T3")));
    let cell = Rect::from_center_size(center, vec2(78.0, 68.0));
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Ellipse(e) if e.radius == vec2(21.0, 5.0) && cell.contains_rect(e.visual_bounding_rect()))));
    let badges: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(badges.len(), 2);
    assert!(badges
        .iter()
        .all(|r| cell.contains_rect(r.visual_bounding_rect())));
    assert_eq!(badges[0].fill, INK);
    assert_eq!(badges[1].stroke.color, BRONZE);
    output.textures_delta.clear();
}
#[test]
fn real_session_rendering_does_not_change_any_simulation_bytes() {
    let mut session = crate::defense_app::session(Default::default());
    let mut peer = crate::defense_app::session(Default::default());
    let mut ui = crate::defense_ui::DefenseUi::new();
    ui.pixel_motion.prepare(&ui.context).unwrap();
    ui.started = true;
    let mut sequence = 0;
    let mut rivets = 0;
    for tick in 0..360 {
        let input = if tick % 45 == 0 {
            sequence += 1;
            game::strategy_input(
                &game::run(session.predicted_frame()),
                &game::units(session.predicted_frame()),
                0,
                sequence,
            )
        } else {
            Default::default()
        };
        session.advance(input, vec![]);
        peer.advance(input, vec![]);
        let before = session.predicted_frame().to_bytes();
        let (mut output, command) = ui.show(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0))),
                ..Default::default()
            },
            session.predicted_frame(),
        );
        assert!(command.is_none());
        let id = ui.pixel_motion.asset.as_ref().unwrap().texture.id();
        rivets += output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == id))
            .count();
        assert_eq!(session.predicted_frame().to_bytes(), before);
        assert_eq!(before, peer.predicted_frame().to_bytes());
        output.textures_delta.clear();
    }
    assert!(
        rivets > 0,
        "real summoned Rivets must reach the optional native painter"
    );
}
#[test]
fn unprepared_optional_skin_preserves_default_vector_scene_exactly() {
    use crate::defense_view::{paint_view, paint_view_with_unit_skin, Layout};
    let mut session = crate::defense_app::session(Default::default());
    for tick in 0..180 {
        let input = if tick % 30 == 0 {
            game::DefenseInput {
                action: game::SUMMON,
                sequence: tick / 30 + 1,
                ..Default::default()
            }
        } else {
            Default::default()
        };
        session.advance(input, vec![]);
    }
    assert!(game::units(session.predicted_frame())
        .iter()
        .any(|u| u.active != 0));
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0));
    let ctx = egui::Context::default();
    let skin = PixelMotion::default();
    let capture = |optional| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                let p = ui.painter().with_clip_rect(screen);
                let frame = orr_bridge::FrameView::of(session.predicted_frame());
                if optional {
                    paint_view_with_unit_skin(
                        &p,
                        Layout::new(screen),
                        frame,
                        Some(0),
                        |p, c, s, u| skin.paint_unit(p, c, s, u, &run(0), true),
                    );
                } else {
                    paint_view(&p, Layout::new(screen), frame, Some(0));
                }
            },
        )
    };
    let mut default = capture(false);
    let mut optional = capture(true);
    assert_eq!(default.shapes, optional.shapes);
    default.textures_delta.clear();
    optional.textures_delta.clear();
}

#[test]
fn all_frame_alpha_bounds_fit_cells_without_per_frame_normalization() {
    let rgba = orr_sprite::png_import::decode_atlas(PNG, 672, 224).unwrap();
    for frame in 0..14 {
        let mut opaque = 0;
        for y in 0..112 {
            for x in 0..96 {
                let i = ((frame / 7 * 112 + y) * 672 + frame % 7 * 96 + x) * 4;
                if rgba[i + 3] == 0 {
                    continue;
                }
                opaque += 1;
                // Check the full pixel area, not merely its top-left sample.
                let min = vec2(
                    (x as f32 - PIVOT[0]) * SCALE,
                    20.0 + (y as f32 - PIVOT[1]) * SCALE,
                );
                let max = min + vec2(SCALE, SCALE);
                assert!(
                    min.x >= -39.0 && max.x <= 39.0 && min.y >= -34.0 && max.y <= 22.5,
                    "frame {frame}, pixel {x},{y} overlaps cell bounds or tier badge"
                );
            }
        }
        assert!(opaque > 0);
    }
}
