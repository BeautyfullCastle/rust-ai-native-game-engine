use super::*;
mod fixture {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/room_motion.rs"
    ));
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}
#[test]
fn idle_motion_widgets_save_reopen_and_actual_keyboard_move_stop_seek_use_authored_clips() {
    let f = Fixture::new();
    let mut h = f.app(false);
    settle(&mut h);
    wait(&mut h, "coherent motion source", |app| {
        app.editor.yard_rows_coherent()
    });
    fixture::install(&f.root, &f._temp.path().join("ui idle source"));
    let before = frame_bytes(&mut h.state_mut().editor);
    {
        let app = h.state_mut();
        app.editor
            .select(Some(Target::Guid(f.guids[&PLAYER].clone())));
        app.models.package = fixture::PACKAGE.into();
        app.models.asset = fixture::ASSET.into();
        app.models.kind = orr_model_bindings::model_bindings::ModelKind::Animated;
        app.models.animation = orr_model_bindings::model_bindings::AnimationDescriptor {
            clip_index: 0,
            playback: orr_model_bindings::model_bindings::PlaybackMode::Loop,
        };
        app.models.assign(&app.editor).unwrap();
    }
    settle(&mut h);
    if h.query_by_label("Use idle clips").is_none() {
        click(&mut h, "Room character");
    }
    click(&mut h, "Play escaped clip once");
    click(&mut h, "Use idle clips");
    for label in ["Searching idle clip", "Carrying idle clip"] {
        click(&mut h, label);
        click(&mut h, &format!("{label} 3: idle (1.00s)"));
    }
    click(&mut h, "Carrying key speed");
    click(&mut h, "Carrying key speed 2x");
    click(&mut h, "Play escaped clip once");
    click(&mut h, "Apply character settings");
    let off = h.state().editor.room_character().unwrap().clone();
    assert_eq!(off.schema, 5);
    assert!(!off.escaped_once());
    assert_eq!(off.motion.unwrap().searching_idle, fixture::IDLE);
    click(&mut h, "Play escaped clip once");
    click(&mut h, "Apply character settings");
    let requested = h.state().editor.room_character().unwrap().clone();
    assert_eq!(requested.schema, 5);
    assert!(requested.escaped_once());
    assert_eq!(requested.motion.unwrap().carrying_idle, fixture::IDLE);
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().editor.room_character(), Some(&off));
    click(&mut h, "Redo character edit");
    click(&mut h, "Save character and model bindings");
    assert_eq!(frame_bytes(&mut h.state_mut().editor), before);
    drop(h);
    let mut h = f.app(false);
    settle(&mut h);
    assert_eq!(h.state().editor.room_character(), Some(&requested));
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, "motion control available", |app| {
        app.editor.can_take_control()
    });
    click(&mut h, "Take control");
    wait(&mut h, "motion input active", |app| {
        app.editor.input_phase() == Phase::Active
    });
    wait(&mut h, "initial authored idle", |app| {
        if !app.editor.yard_rows_coherent() {
            return false;
        }
        let placed = app.models.animated_placements(&app.editor).unwrap();
        placed.len() == 1
            && placed[0].pose == placed[0].model.sample_clip(fixture::IDLE, 0.0).unwrap()
    });
    let initial_x = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .unwrap()[0]
        .instance[3][0];
    key(&mut h, egui::Key::D, true);
    wait(&mut h, "real displacement switches to moving clip", |app| {
        if !app.editor.yard_rows_coherent() {
            return false;
        }
        let placed = app.models.animated_placements(&app.editor).unwrap();
        if placed.len() != 1 || placed[0].instance[3][0] <= initial_x {
            return false;
        }
        let snapshot = app.editor.snapshot().unwrap();
        let duration = placed[0].model.source().clips[requested.searching as usize].duration();
        let time = orr_model_bindings::animation_time::sample_clip_time_at_rate(
            snapshot.tick(),
            snapshot.tick_rate(),
            duration,
            orr_model_bindings::model_bindings::PlaybackMode::Loop,
            requested.speed(orr_sample::room_character::State::Searching),
        )
        .unwrap();
        placed[0].pose
            == placed[0]
                .model
                .sample_clip(requested.searching, time)
                .unwrap()
            && placed[0].pose != placed[0].model.sample_clip(fixture::IDLE, 0.0).unwrap()
    });
    key(&mut h, egui::Key::D, false);
    let last = std::cell::Cell::new((
        h.state().editor.snapshot().unwrap().tick(),
        h.state()
            .models
            .animated_placements(&h.state().editor)
            .unwrap()[0]
            .instance[3][0],
    ));
    wait(&mut h, "actual stop returns to idle", |app| {
        if !app.editor.yard_rows_coherent() {
            return false;
        }
        let placed = app.models.animated_placements(&app.editor).unwrap();
        if placed.len() != 1 {
            return false;
        }
        let tick = app.editor.snapshot().unwrap().tick();
        let x = placed[0].instance[3][0];
        let old = last.replace((tick, x));
        tick > old.0
            && x == old.1
            && placed[0].pose == placed[0].model.sample_clip(fixture::IDLE, 0.0).unwrap()
    });
    h.state_mut().editor.pause();
    settle(&mut h);
    h.state_mut().editor.seek(0);
    wait(&mut h, "motion seek resets idle observation", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .timeline()
                .is_some_and(|timeline| timeline.tick == 0)
    });
    let placed = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .unwrap();
    assert_eq!(
        placed[0].pose,
        placed[0].model.sample_clip(fixture::IDLE, 0.0).unwrap()
    );
    click(&mut h, orr_editor::app::LBL_STOP);
    wait(&mut h, "motion stop restores rest", |app| {
        app.editor.mode() == Mode::Edit
            && app.editor.yard_rows_coherent()
            && app
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.timeline().is_none())
    });
    let placed = h
        .state()
        .models
        .animated_placements(&h.state().editor)
        .unwrap();
    assert_eq!(placed[0].pose, placed[0].model.rest_pose().unwrap());
}
