use super::*;
use orr_model_bindings::animation_time::sample_clip_time_at_rate;
use orr_model_bindings::model_bindings::PlaybackMode;
use orr_sample::room_character::{Document, PlaybackRate};

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}
fn author(f: &Fixture) -> Document {
    let mut h = f.app(false);
    settle(&mut h);
    wait(&mut h, "pickup source coherent", |a| {
        a.editor.yard_rows_coherent()
    });
    let before = frame_bytes(&mut h.state_mut().editor);
    let old = h.state().editor.room_character().unwrap().clone();
    if h.query_by_label("Play key pickup clip once").is_none() {
        click(&mut h, "Room character");
    }
    click(&mut h, "Play key pickup clip once");
    click(&mut h, "Key pickup clip");
    click(&mut h, "Key pickup clip 2: escaped (1.00s)");
    click(&mut h, "Key pickup speed");
    click(&mut h, "Key pickup speed 1x");
    assert_eq!(h.state().editor.room_character(), Some(&old));
    assert!(
        h.state()
            .room_character
            .as_ref()
            .unwrap()
            .has_unapplied_changes()
    );
    click(&mut h, "Apply character settings");
    let document = h.state().editor.room_character().unwrap().clone();
    assert_eq!(document.schema, 6);
    assert_eq!(document.crossfade_ticks, Some(0));
    assert_eq!(document.motion.unwrap().pickup.unwrap().clip, 2);
    assert_eq!(
        document.motion.unwrap().pickup.unwrap().speed,
        PlaybackRate::Normal
    );
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().editor.room_character(), Some(&old));
    click(&mut h, "Redo character edit");
    assert_eq!(h.state().editor.room_character(), Some(&document));
    // Dispatch to the current widget identity without depending on scroll layout.
    let save = h.get_by_label("Save character and model bindings");
    assert!(
        !save.accesskit_node().is_disabled(),
        "paired Save must be enabled after Redo"
    );
    save.click_accesskit();
    settle(&mut h);
    assert_eq!(
        Document::parse(&fs::read(f.root.join("room.character.json")).unwrap()).unwrap(),
        document,
        "actual Save widget must persist the expected policy before closing"
    );
    assert!(
        !h.state().room_character.as_ref().unwrap().dirty(),
        "actual Save must advance the character baseline"
    );
    assert_eq!(frame_bytes(&mut h.state_mut().editor), before);
    drop(h);
    let mut h = f.app(false);
    settle(&mut h);
    assert_eq!(h.state().editor.room_character(), Some(&document));
    document
}

#[test]
fn pickup_once_widgets_stage_apply_history_save_and_reopen_without_frame_changes() {
    let f = Fixture::new();
    let document = author(&f);
    assert_eq!(
        Document::parse(&fs::read(f.root.join("room.character.json")).unwrap()).unwrap(),
        document
    );
}

#[test]
#[ignore = "explicit CPU native source-hidden acceptance requires ORR_ROOM_PICKUP_RUNTIME"]
fn pickup_once_source_hidden_export_native_pose_and_restart() {
    use orr_sim::{PlayerSlot, TickInputs};
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let document = author(&f);
    // The native held-input interface sends one interaction edge at tick 1.
    // Keep the generated PLAYER/KEY in range and move EXIT off the held path.
    let mut h = f.app(false);
    settle(&mut h);
    move_actor(
        &mut h,
        &f.guids[&EXIT],
        FPVec3::new(FP::from_raw(262144), FP::HALF, FP::from_raw(262144)),
    );
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    drop(h);
    let saved_character = fs::read(f.root.join("room.character.json")).unwrap();
    let runtime = f._temp.path().join("trusted pickup runtime");
    fs::copy(
        PathBuf::from(
            std::env::var_os("ORR_ROOM_PICKUP_RUNTIME").expect("explicit built runtime required"),
        ),
        &runtime,
    )
    .unwrap();
    let hash: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = f._temp.path().join("deployed pickup game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: f.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: hash,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    assert_eq!(
        fs::read(bundle.join("project/room.character.json")).unwrap(),
        saved_character
    );
    fs::rename(&f.root, f.root.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open_with_capabilities(
        &bundle.join("project"),
        orr_sample::room_project::RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    )
    .unwrap();
    let binding = &prepared.models().document.bindings[&document.player];
    let model = prepared.models().assets[&(binding.package.clone(), binding.asset.clone())]
        .animated_model()
        .unwrap();
    let pickup = document.motion.unwrap().pickup.unwrap();
    let duration = model.source().clips[pickup.clip as usize].duration();
    assert_eq!(duration, 1.0);
    let mut sim = prepared.scene().simulation().unwrap();
    let mut entry = None;
    let mut receipts = BTreeMap::new();
    let mut expected_digests = BTreeMap::new();
    for tick in 0..=180u64 {
        if tick != 0 {
            let mut inputs = TickInputs::new(sim.tick() + 1, 1);
            inputs.set_input(
                PlayerSlot(0),
                orr_sample::room_game::RoomInput {
                    move_x: 1,
                    buttons: orr_sample::room_game::INTERACT,
                    ..Default::default()
                },
            );
            sim.step(&inputs);
        }
        let run = sim.frame().singleton::<RoomRun>();
        if run.key_collected != 0 && entry.is_none() {
            entry = Some(tick);
        }
        assert_eq!(run.won, 0);
        let (clip, seconds) = match entry {
            Some(start) if tick - start < 60 => {
                (pickup.clip, ((tick - start) as f64 / 60.0) as f32)
            }
            Some(_) => {
                let clip = document.carrying;
                (
                    clip,
                    sample_clip_time_at_rate(
                        tick,
                        60,
                        model.source().clips[clip as usize].duration(),
                        PlaybackMode::Loop,
                        document.speed(orr_sample::room_character::State::Carrying),
                    )
                    .unwrap(),
                )
            }
            None => {
                let clip = document.searching;
                (
                    clip,
                    sample_clip_time_at_rate(
                        tick,
                        60,
                        model.source().clips[clip as usize].duration(),
                        PlaybackMode::Loop,
                        document.speed(orr_sample::room_character::State::Searching),
                    )
                    .unwrap(),
                )
            }
        };
        let digest =
            orr_sample::room_app::character_pose_digest(&model.sample_clip(clip, seconds).unwrap());
        receipts.insert(tick, (sim.frame().checksum(), run.key_collected));
        expected_digests.insert(tick, digest);
    }
    let entry = entry.expect("actual authoritative input must collect key");
    assert_eq!(
        entry, 1,
        "held input must collect at its single rising edge"
    );
    let absolute_at_entry = orr_sample::room_app::character_pose_digest(
        &model
            .sample_clip(pickup.clip, (entry as f64 / 60.0) as f32)
            .unwrap(),
    );
    assert_ne!(
        expected_digests[&entry], absolute_at_entry,
        "entry-local phase zero must differ from absolute phase"
    );
    assert_ne!(expected_digests[&entry], expected_digests[&(entry + 30)]);
    assert_ne!(
        expected_digests[&(entry + 30)],
        expected_digests[&(entry + 60)]
    );
    for tick in [
        entry - 1,
        entry,
        entry + 30,
        entry + 59,
        entry + 60,
        entry + 61,
    ] {
        let mut previous = None;
        for _restart in 0..2 {
            let output = std::process::Command::new(bundle.join("run-room-escape"))
                .current_dir(f._temp.path())
                .args([
                    "--headless",
                    "--ticks",
                    &tick.to_string(),
                    "--hold",
                    "right,interact",
                    "--check-model-poses",
                ])
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(
                text.contains(&format!(
                    "room tick: {tick} checksum: 0x{:016x}",
                    receipts[&tick].0
                )),
                "{text}"
            );
            assert!(
                text.contains(&format!("room key: {} won: 0", receipts[&tick].1)),
                "{text}"
            );
            assert!(
                text.contains(&format!("room model poses: {} frames checked;", tick + 1)),
                "{text}"
            );
            assert_eq!(
                text.lines()
                    .find_map(|line| line.strip_prefix("room character observed pose sha256: ")),
                Some(expected_digests[&tick].as_str()),
                "tick {tick}: {text}"
            );
            if let Some(old) = previous {
                assert_eq!(text, old, "fresh process restart must reproduce receipt");
            }
            previous = Some(text);
        }
    }
    assert_eq!(
        fs::read(bundle.join("project/room.character.json")).unwrap(),
        saved_character
    );
}
