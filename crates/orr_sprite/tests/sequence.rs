use orr_sprite::{
    sequence::ping_pong_clip, Atlas, ClipSource, Frame, Playback, PlaybackMode, Region, Sample,
    SpriteDocument, SpriteSource, FORMAT, MAX_CLIP_ID_BYTES, MAX_FRAMES_PER_CLIP, MAX_TOTAL_FRAMES,
    VERSION,
};

fn frame(region: u32, duration_ms: u64) -> Frame {
    Frame {
        region,
        duration_ms,
    }
}

fn source(clips: Vec<ClipSource>) -> SpriteSource {
    SpriteSource {
        format: FORMAT.into(),
        version: VERSION,
        atlas: Atlas {
            image: "inert/missing-sheet.png".into(),
            width: 4,
            height: 1,
        },
        regions: [7, 42, 99, 123]
            .into_iter()
            .enumerate()
            .map(|(x, id)| Region {
                id,
                x: x as u32,
                y: 0,
                width: 1,
                height: 1,
            })
            .collect(),
        clips,
    }
}

fn document(forward: &[Frame]) -> SpriteDocument {
    SpriteDocument::new(source(vec![
        ping_pong_clip("bounce".into(), forward).unwrap()
    ]))
    .unwrap()
}

#[test]
fn unequal_durations_have_exact_half_open_boundaries() {
    let d = document(&[frame(7, 10), frame(42, 20), frame(99, 30)]);
    let clip = d.clip("bounce").unwrap();
    assert_eq!(clip.mode(), PlaybackMode::Loop);
    assert_eq!(
        clip.frames(),
        [frame(7, 10), frame(42, 20), frame(99, 30), frame(42, 20)]
    );
    assert_eq!(clip.duration_ms(), 80);
    for (time, region, frame_index) in [
        (0, 7, 0),
        (9, 7, 0),
        (10, 42, 1),
        (29, 42, 1),
        (30, 99, 2),
        (59, 99, 2),
        (60, 42, 3),
        (79, 42, 3),
        (80, 7, 0),
        (160, 7, 0),
    ] {
        assert_eq!(
            clip.sample(time),
            Sample {
                region,
                frame_index,
                finished: false,
            },
            "time {time}"
        );
    }
}

#[test]
fn four_frames_reverse_only_the_interior() {
    let d = document(&[frame(7, 2), frame(42, 3), frame(99, 5), frame(123, 7)]);
    let clip = d.clip("bounce").unwrap();
    assert_eq!(
        clip.frames(),
        [
            frame(7, 2),
            frame(42, 3),
            frame(99, 5),
            frame(123, 7),
            frame(99, 5),
            frame(42, 3),
        ]
    );
    assert_eq!(clip.duration_ms(), 25);
    assert_eq!(clip.sample(17).frame_index, 4);
    assert_eq!(clip.sample(22).frame_index, 5);
    assert_eq!(clip.sample(25).frame_index, 0);
}

#[test]
fn repeated_region_ids_preserve_positions_and_durations() {
    let forward = [frame(7, 2), frame(42, 3), frame(7, 5), frame(42, 7)];
    let clip = ping_pong_clip("repeat".into(), &forward).unwrap();
    assert_eq!(
        clip.frames,
        [
            frame(7, 2),
            frame(42, 3),
            frame(7, 5),
            frame(42, 7),
            frame(7, 5),
            frame(42, 3),
        ]
    );
    assert_eq!(
        forward,
        [frame(7, 2), frame(42, 3), frame(7, 5), frame(42, 7)]
    );
}

#[test]
fn singleton_accepts_max_duration_and_wraps() {
    let forward = [frame(7, u64::MAX)];
    let d = document(&forward);
    let clip = d.clip("bounce").unwrap();
    assert_eq!(clip.frames(), forward);
    assert_eq!(clip.duration_ms(), u64::MAX);
    for time in [0, 1, u64::MAX - 1, u64::MAX] {
        assert_eq!(
            clip.sample(time),
            Sample {
                region: 7,
                frame_index: 0,
                finished: false,
            }
        );
    }
}

#[test]
fn two_frames_accept_max_total_without_doubling_endpoints() {
    let forward = [frame(7, u64::MAX - 1), frame(42, 1)];
    let d = document(&forward);
    let clip = d.clip("bounce").unwrap();
    assert_eq!(clip.frames(), forward);
    assert_eq!(clip.duration_ms(), u64::MAX);
    assert_eq!(clip.sample(u64::MAX - 2).frame_index, 0);
    assert_eq!(clip.sample(u64::MAX - 1).frame_index, 1);
    assert_eq!(clip.sample(u64::MAX).frame_index, 0);
    assert!(!clip.sample(u64::MAX).finished);
}

#[test]
fn expanded_frame_count_limit_is_inclusive() {
    let forward = vec![frame(7, 1); 2049];
    let clip = ping_pong_clip("limit".into(), &forward).unwrap();
    assert_eq!(clip.frames.len(), MAX_FRAMES_PER_CLIP);
    assert_eq!(clip.frames.len(), 4096);
    let d = SpriteDocument::new(source(vec![clip])).unwrap();
    assert_eq!(d.clip("limit").unwrap().duration_ms(), 4096);
    let over = vec![frame(7, 1); 2050];
    let error = ping_pong_clip("over".into(), &over).unwrap_err();
    assert!(error.to_string().contains("frame count"));
}

#[test]
fn empty_inputs_and_zero_durations_reject() {
    assert!(ping_pong_clip("empty".into(), &[]).is_err());
    assert!(ping_pong_clip("zero".into(), &[frame(7, 0)]).is_err());
    for index in 0..3 {
        let mut forward = [frame(7, 1), frame(42, 1), frame(99, 1)];
        forward[index].duration_ms = 0;
        let original = forward;
        assert!(ping_pong_clip("zero".into(), &forward).is_err());
        assert_eq!(forward, original);
    }
}

#[test]
fn forward_and_return_leg_duration_overflow_reject() {
    for forward in [
        vec![frame(7, u64::MAX), frame(42, 1)],
        // Forward total is exactly MAX; adding the interior must still reject.
        vec![frame(7, u64::MAX - 2), frame(42, 1), frame(99, 1)],
    ] {
        let original = forward.clone();
        let error = ping_pong_clip("overflow".into(), &forward).unwrap_err();
        assert!(error.to_string().contains("duration overflow"));
        assert_eq!(forward, original);
    }
    let d = document(&[frame(7, u64::MAX - 3), frame(42, 1), frame(99, 1)]);
    assert_eq!(d.clip("bounce").unwrap().duration_ms(), u64::MAX);
}

#[test]
fn clip_ids_use_nonblank_utf8_byte_bounds_without_normalization() {
    let forward = [frame(7, 1)];
    for id in ["", " \t\n", "\u{2003}"] {
        assert!(ping_pong_clip(id.into(), &forward).is_err());
    }
    for id in [
        "x".repeat(MAX_CLIP_ID_BYTES),
        "é".repeat(MAX_CLIP_ID_BYTES / 2),
    ] {
        assert_eq!(id.len(), 128);
        let clip = ping_pong_clip(id.clone(), &forward).unwrap();
        assert_eq!(clip.id, id);
        assert!(SpriteDocument::new(source(vec![clip])).is_ok());
        assert!(ping_pong_clip(format!("{id}x"), &forward).is_err());
    }
    let id = "  돌아오기 🚀  ";
    assert_eq!(ping_pong_clip(id.into(), &forward).unwrap().id, id);
}

#[test]
fn generation_is_deterministic_and_preserves_input() {
    let forward = [frame(7, 10), frame(42, 20), frame(99, 30)];
    let original = forward;
    let a = ping_pong_clip("bounce".into(), &forward).unwrap();
    let b = ping_pong_clip("bounce".into(), &forward).unwrap();
    assert_eq!(forward, original);
    assert_eq!(a, b);
    let a = SpriteDocument::new(source(vec![a])).unwrap();
    let b = SpriteDocument::new(source(vec![b])).unwrap();
    assert_eq!(a.to_json().unwrap(), b.to_json().unwrap());
}

#[test]
fn generated_json_round_trips_as_existing_version_one_loop() {
    let d = document(&[frame(7, 10), frame(42, 20), frame(99, 30)]);
    let json = d.to_json().unwrap();
    let authored: SpriteSource = serde_json::from_str(&json).unwrap();
    assert_eq!(authored.format, "orr_sprite");
    assert_eq!(authored.version, 1);
    assert_eq!(authored.clips[0].mode, PlaybackMode::Loop);
    assert!(json.contains("\"mode\": \"loop\""));
    assert!(!json.contains("ping_pong"));
    let restored = SpriteDocument::from_json(&json).unwrap();
    assert_eq!(restored.to_json().unwrap(), json);
    for time in [0, 9, 10, 29, 30, 59, 60, 79, 80, u64::MAX] {
        assert_eq!(
            restored.clip("bounce").unwrap().sample(time),
            d.clip("bounce").unwrap().sample(time)
        );
    }
}

#[test]
fn document_still_rejects_missing_regions_and_duplicate_clip_ids() {
    let missing = ping_pong_clip("missing".into(), &[frame(u32::MAX, 1)]).unwrap();
    assert_eq!(missing.frames, [frame(u32::MAX, 1)]);
    let error = SpriteDocument::new(source(vec![missing])).unwrap_err();
    assert!(error.to_string().contains("missing region"));
    let clip = ping_pong_clip("duplicate".into(), &[frame(7, 1)]).unwrap();
    assert!(SpriteDocument::new(source(vec![clip.clone(), clip])).is_err());
}

#[test]
fn document_total_frame_limit_is_still_enforced() {
    let forward = vec![frame(7, 1); 2049];
    let clips: Vec<_> = (0..4)
        .map(|i| ping_pong_clip(format!("clip{i}"), &forward).unwrap())
        .collect();
    assert_eq!(
        clips.iter().map(|c| c.frames.len()).sum::<usize>(),
        MAX_TOTAL_FRAMES
    );
    assert!(SpriteDocument::new(source(clips.clone())).is_ok());
    let mut over = clips;
    over.push(ping_pong_clip("one-more".into(), &[frame(7, 1)]).unwrap());
    let error = SpriteDocument::new(source(over)).unwrap_err();
    assert!(error.to_string().contains("total frame limit"));
}

#[test]
fn playback_seek_reset_saturation_and_independent_cursors_are_unchanged() {
    let d = document(&[frame(7, 10), frame(42, 20), frame(99, 30)]);
    let clip = d.clip("bounce").unwrap();
    let mut a = Playback::default();
    let mut b = Playback::default();
    a.seek(60);
    b.advance(30);
    assert_eq!(a.sample(clip).frame_index, 3);
    assert_eq!(b.sample(clip).frame_index, 2);
    a.reset();
    assert_eq!(a.elapsed_ms(), 0);
    assert_eq!(a.sample(clip).frame_index, 0);
    assert_eq!(b.elapsed_ms(), 30);
    a.seek(u64::MAX - 1);
    a.advance(10);
    assert_eq!(a.elapsed_ms(), u64::MAX);
    assert_eq!(a.sample(clip), clip.sample(u64::MAX % 80));
    assert_eq!(a.sample(clip).frame_index, 1); // MAX % 80 = 15.
    assert!(!a.sample(clip).finished);
    a.advance(u64::MAX);
    assert_eq!(a.elapsed_ms(), u64::MAX);
    a.reset();
    assert_eq!(a.sample(clip).frame_index, 0);
    assert_eq!(b.sample(clip).frame_index, 2);
}
