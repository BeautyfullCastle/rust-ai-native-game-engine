use super::*;
use orr_games::coop_defense_game as game;

fn event(kind: Kind) -> DefenseEvent {
    DefenseEvent {
        kind: kind as u32,
        board: 0,
        value: 1,
        unit_id: 1,
        enemy_id: 2,
        projectile_id: 3,
        x: 100,
        y: 100,
        ..Default::default()
    }
}
fn emit(vfx: &mut DefenseVfx, tick: u64, status: EventStatus<DefenseEvent>) {
    vfx.consume_items([(EventKey::new(tick, 0, 0), status)])
        .unwrap();
}
#[test]
fn genuine_embedded_pngs_decode_and_nearest_textures_preserve_alpha() {
    let ctx = egui::Context::default();
    let mut vfx = DefenseVfx::default();
    vfx.prepare(&ctx).unwrap();
    assert_eq!(vfx.assets.len(), 2);
    for (index, (_, _, png, pivot)) in SPECS.iter().enumerate() {
        let asset = &vfx.assets[index];
        let atlas = asset.document.atlas();
        let pixels = orr_sprite::png_import::decode_atlas(png, atlas.width, atlas.height).unwrap();
        assert!(pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
        assert_eq!(asset.pivot, *pivot);
        assert_eq!(asset.document.clips()[0].frames().len(), [16, 28][index]);
        assert_eq!(asset.document.clips()[0].duration_ms(), [528, 924][index]);
        assert_eq!(
            asset.texture.size(),
            [atlas.width as usize, atlas.height as usize]
        );
    }
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    let uploads: Vec<_> = output
        .textures_delta
        .set
        .iter()
        .filter(|(id, _)| vfx.assets.iter().any(|a| a.texture.id() == **id))
        .collect();
    assert_eq!(uploads.len(), 2);
    assert!(uploads.iter().all(|(_, deltas)| deltas
        .iter()
        .all(|delta| delta.options == TextureOptions::NEAREST)));
    for (index, asset) in vfx.assets.iter().enumerate() {
        let (_, deltas) = uploads
            .iter()
            .copied()
            .find(|(id, _)| **id == asset.texture.id())
            .unwrap();
        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].pos.is_none());
        let egui::ImageData::Color(upload) = &deltas[0].image;
        let atlas = asset.document.atlas();
        let rgba = orr_sprite::png_import::decode_atlas(SPECS[index].2, atlas.width, atlas.height)
            .unwrap();
        let expected: Vec<_> = rgba
            .chunks_exact(4)
            .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        assert_eq!(
            upload.pixels, expected,
            "straight RGBA must be converted exactly once"
        );
    }
    let ids: Vec<_> = vfx.assets.iter().map(|a| a.texture.id()).collect();
    vfx.prepare(&ctx).unwrap();
    assert_eq!(
        ids,
        vfx.assets
            .iter()
            .map(|a| a.texture.id())
            .collect::<Vec<_>>()
    );
    output.textures_delta.clear();
}
#[test]
fn predicted_verified_duplicate_never_restarts_and_cancel_removes() {
    let mut v = DefenseVfx::default();
    let hit = event(Kind::Hit);
    emit(&mut v, 1, EventStatus::Predicted(hit));
    v.advance_to(5);
    emit(&mut v, 1, EventStatus::Verified(hit));
    emit(&mut v, 1, EventStatus::Verified(hit));
    assert_eq!((v.state.started, v.state.active.len()), (1, 1));
    assert_eq!(v.state.active.values().next().unwrap().born, 1);
    emit(&mut v, 1, EventStatus::Canceled);
    assert!(v.state.active.is_empty());
    assert_eq!(v.state.canceled, 1);
    // A rollback can replace a canceled key with a corrected actual payload.
    emit(
        &mut v,
        1,
        EventStatus::Predicted(DefenseEvent { x: 200, ..hit }),
    );
    assert_eq!(v.state.active.values().next().unwrap().event.x, 200);
}
#[test]
fn only_real_hit_release_map_and_all_other_events_are_inert() {
    let mut v = DefenseVfx::default();
    for (index, kind) in [
        Kind::Summon,
        Kind::Merge,
        Kind::Death,
        Kind::Leak,
        Kind::Wave,
        Kind::Finish,
        Kind::Move,
        Kind::EnemySpawn,
        Kind::ProjectileExpired,
    ]
    .into_iter()
    .enumerate()
    {
        emit(&mut v, index as u64 + 1, EventStatus::Verified(event(kind)));
    }
    assert_eq!(v.state.started, 0);
    emit(&mut v, 20, EventStatus::Verified(event(Kind::Release)));
    emit(
        &mut v,
        21,
        EventStatus::Verified(DefenseEvent {
            role: 2,
            ..event(Kind::Hit)
        }),
    );
    assert_eq!(
        v.state.active.values().map(|e| e.asset).collect::<Vec<_>>(),
        [0, 1]
    );
}
#[test]
fn one_shot_expiry_is_session_clock_driven_and_never_replays() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 1, EventStatus::Verified(event(Kind::Hit)));
    v.advance_to(32); // 516ms, still active.
    assert_eq!(v.state.active.len(), 1);
    v.advance_to(32); // Paused redraw or repeated snapshot.
    assert_eq!(v.state.active.len(), 1);
    v.advance_to(33); // 533ms, beyond the 528ms one-shot.
    assert!(v.state.active.is_empty());
    emit(&mut v, 1, EventStatus::Verified(event(Kind::Hit)));
    assert!(v.state.active.is_empty());
    assert_eq!(v.state.started, 1);
}
#[test]
fn restart_generation_and_loaded_session_clear_history_without_backlog() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 1, EventStatus::Verified(event(Kind::Hit)));
    let restart = DefenseEvent {
        generation: 1,
        ..event(Kind::Restart)
    };
    emit(&mut v, 2, EventStatus::Verified(restart));
    assert!(v.state.active.is_empty());
    emit(&mut v, 2, EventStatus::Verified(restart));
    emit(
        &mut v,
        3,
        EventStatus::Verified(DefenseEvent {
            generation: 1,
            ..event(Kind::Hit)
        }),
    );
    assert_eq!(v.state.active.len(), 1);
    assert!(v
        .consume_items([(
            EventKey::new(4, 0, 0),
            EventStatus::Verified(event(Kind::Hit))
        )])
        .is_err());
    assert!(v
        .consume_items([(EventKey::new(2, 0, 0), EventStatus::Canceled)])
        .is_err());
    v.replace(100, 4);
    emit(&mut v, 99, EventStatus::Verified(event(Kind::Hit)));
    assert!(v.state.active.is_empty());
    emit(
        &mut v,
        101,
        EventStatus::Verified(DefenseEvent {
            generation: 4,
            ..event(Kind::Hit)
        }),
    );
    assert_eq!(v.state.active.len(), 1);
}
#[test]
fn invalid_late_record_leaves_entire_batch_unpublished() {
    let mut v = DefenseVfx::default();
    let before = v.state.clone();
    let result = v.consume_items([
        (
            EventKey::new(1, 0, 0),
            EventStatus::Verified(event(Kind::Hit)),
        ),
        (
            EventKey::new(1, 0, 1),
            EventStatus::Verified(DefenseEvent {
                projectile_id: 0,
                ..event(Kind::Hit)
            }),
        ),
    ]);
    assert!(result.is_err());
    assert_eq!(before, v.state);
    assert!(v
        .consume_items([(EventKey::new(2, 1, 0), EventStatus::Canceled)])
        .is_err());
    assert_eq!(before, v.state);
}
#[test]
fn active_history_and_input_batches_are_bounded() {
    let mut v = DefenseVfx::default();
    for tick in 1..=MAX_SEEN as u64 + 10 {
        emit(&mut v, tick, EventStatus::Verified(event(Kind::Hit)));
    }
    assert!(v.state.active.len() <= MAX_ACTIVE);
    assert_eq!(v.state.seen.len(), MAX_SEEN);
    assert!(v.state.dropped > 0);
    let started = v.state.started;
    emit(&mut v, 1, EventStatus::Verified(event(Kind::Hit)));
    assert_eq!(v.state.started, started);
    let oversized = EventBatch::verified(
        (0..=MAX_BATCH)
            .map(|n| orr_sim::SimEvent {
                key: EventKey::new(5000, 0, n as u32),
                payload: event(Kind::Hit),
            })
            .collect(),
    );
    assert!(v.consume(&oversized).is_err());
}
#[test]
fn full_public_producer_budget_is_valid_and_drops_only_visual_overflow() {
    let public_budget = 2 * game::MAX_EVENTS_PER_TICK;
    assert_eq!(MAX_BATCH, public_budget);
    let mut v = DefenseVfx::default();
    let batch = EventBatch::verified(
        (0..public_budget)
            .map(|n| orr_sim::SimEvent {
                key: EventKey::new(1, 0, n as u32),
                payload: event(Kind::Hit),
            })
            .collect(),
    );
    v.consume(&batch).unwrap();
    assert!(v.state.active.len() <= MAX_ACTIVE);
    assert_eq!(v.state.seen.len(), MAX_SEEN);
    assert!(v.state.dropped > 0);
}
#[test]
fn real_session_events_repeat_without_changes_to_any_gameplay_bytes() {
    let mut session = crate::defense_app::session(Default::default());
    let mut peer = crate::defense_app::session(Default::default());
    let mut v = DefenseVfx::default();
    let mut sequence = 0;
    let mut hits = 0;
    for tick in 0..1200 {
        let command = if tick % 45 == 0 {
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
        let result = session.advance(command, vec![]);
        peer.advance(command, vec![]);
        let (orr_session::AdvanceResult::Advanced { events, .. }
        | orr_session::AdvanceResult::Stalled { events }) = &result;
        hits += events.iter().filter(|(_, s)| matches!(s, EventStatus::Verified(e) | EventStatus::Predicted(e) if e.event_kind() == Some(Kind::Hit))).count();
        v.consume(events).unwrap();
        let once = v.state.clone();
        v.consume(events).unwrap();
        assert_eq!(v.state, once);
        v.advance_to(session.predicted_frame().tick());
        assert_eq!(
            session.predicted_frame().to_bytes(),
            peer.predicted_frame().to_bytes()
        );
    }
    assert!(hits > 0 && v.state.started > 0);
}
#[test]
fn painter_uses_exact_uv_pivot_and_clipped_nearest_texture_without_frame_writes() {
    let ctx = egui::Context::default();
    let mut v = DefenseVfx::default();
    v.prepare(&ctx).unwrap();
    emit(&mut v, 1, EventStatus::Verified(event(Kind::Hit)));
    v.advance_to(1);
    let frame = crate::defense_app::session(Default::default());
    let before = frame.predicted_frame().to_bytes();
    let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 940.0));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            v.paint(&ui.painter().with_clip_rect(screen), Layout::new(screen));
        },
    );
    let meshes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| {
            if let egui::Shape::Mesh(m) = &s.shape {
                Some((s.clip_rect, m))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(meshes.len(), 1);
    let (clip, mesh) = meshes[0];
    assert_eq!(clip, screen);
    assert_eq!(mesh.texture_id, v.assets[0].texture.id());
    assert_eq!(mesh.vertices[0].uv, egui::Pos2::ZERO);
    assert_eq!(
        mesh.vertices[0].pos,
        egui::pos2(200.0 - 44.0 * 0.75, 205.0 - 64.0 * 0.75)
    );
    assert_eq!(frame.predicted_frame().to_bytes(), before);
    output.textures_delta.clear();
}
