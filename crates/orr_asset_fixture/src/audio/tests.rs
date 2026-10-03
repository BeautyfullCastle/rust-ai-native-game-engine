#![allow(clippy::disallowed_types)]

use super::*;
use crate::game::{self, AssetFixtureGame, Input};
use crate::{BundleBytes, BUILD_ID, PLAYER_COUNT, SEED, TICKS, TICK_RATE};
use orr_asset::{encode_manifest, manifest_encoded_len, ManifestEntry};
use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, Lifecycle, PlayConfig,
    PlayHost, PlaySession, ViewResync,
};
use orr_sim::PlayerSlot;

fn prepared() -> PreparedFixture {
    PreparedFixture::embedded().unwrap()
}
fn audio() -> FixtureAudio {
    FixtureAudio::embedded_offline(&prepared(), AudioMode::Required).unwrap()
}
fn event(tick: u64, status: EventStatus<Impact>) -> BridgeEvent<Impact> {
    BridgeEvent::Sim {
        key: EventKey::new(tick, 0, 0),
        status,
    }
}
fn predicted(tick: u64) -> BridgeEvent<Impact> {
    event(tick, EventStatus::Predicted(Impact { cue: 1 }))
}
fn verified(tick: u64) -> BridgeEvent<Impact> {
    event(tick, EventStatus::Verified(Impact { cue: 1 }))
}
fn batch(events: Vec<BridgeEvent<Impact>>) -> ViewUpdate<Impact> {
    ViewUpdate {
        snapshot: None,
        events,
        resync: None,
    }
}
fn render(audio: &mut FixtureAudio, frames: usize) -> Vec<f32> {
    let mut pcm = vec![0.0; frames * 2];
    for block in pcm.chunks_mut(256) {
        audio.render(block).unwrap();
    }
    assert!(pcm.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    pcm
}
fn audible(pcm: &[f32]) -> bool {
    pcm.iter().any(|v| v.abs() > 0.001)
}
fn silent(pcm: &[f32]) -> bool {
    pcm.iter().all(|v| v.abs() < 0.000001)
}

#[test]
fn required_auto_off_report_missing_and_corrupt_view_without_weakening_admission() {
    let fixture = prepared();
    let missing = ViewBundleBytes {
        manifest: crate::VIEW_BYTES,
        objects: &[],
    };
    assert!(matches!(
        FixtureAudio::open_offline(&fixture, AudioMode::Required, missing),
        Err(Error::MissingArtifact)
    ));
    let mut muted = FixtureAudio::open_offline(&fixture, AudioMode::Auto, missing).unwrap();
    assert!(
        matches!(muted.status(), AudioStatus::Muted { reason } if reason.contains("MissingArtifact"))
    );
    muted.update(1, &batch(vec![predicted(1)]));
    assert!(silent(&render(&mut muted, 1000)));
    assert_eq!(muted.stats().started, 0);
    // The already admitted sim remains usable; the original strict preparation
    // still rejects an incomplete package, even when the view policy is auto.
    assert_eq!(fixture.start().advance().unwrap().state.tick, 1);
    let objects = [ArtifactBytes {
        sha256: sha256(crate::MOTION_BYTES),
        bytes: crate::MOTION_BYTES,
    }];
    assert!(matches!(
        PreparedFixture::prepare(
            BundleBytes {
                sim_manifest: crate::SIM_BYTES,
                view_manifest: crate::VIEW_BYTES,
                objects: &objects,
            },
            fixture.release()
        ),
        Err(Error::MissingArtifact)
    ));

    let mut corrupt = crate::IMPACT_BYTES.to_vec();
    corrupt[8] ^= 1;
    let objects = [ArtifactBytes {
        sha256: sha256(crate::IMPACT_BYTES),
        bytes: &corrupt,
    }];
    let bundle = ViewBundleBytes {
        manifest: crate::VIEW_BYTES,
        objects: &objects,
    };
    assert!(matches!(
        FixtureAudio::open_offline(&fixture, AudioMode::Required, bundle),
        Err(Error::DigestMismatch)
    ));
    assert!(
        matches!(FixtureAudio::open_offline(&fixture, AudioMode::Auto, bundle).unwrap().status(),
        AudioStatus::Muted { reason } if reason.contains("DigestMismatch"))
    );
    let mut manifest = crate::VIEW_BYTES.to_vec();
    manifest[40] ^= 1;
    assert!(matches!(
        FixtureAudio::open_offline(
            &fixture,
            AudioMode::Required,
            ViewBundleBytes {
                manifest: &manifest,
                objects: &objects
            }
        ),
        Err(Error::DigestMismatch)
    ));

    let mut off = FixtureAudio::open_offline(
        &fixture,
        AudioMode::Off,
        ViewBundleBytes {
            manifest: b"not even a manifest",
            objects: &[],
        },
    )
    .unwrap();
    assert_eq!(off.status(), &AudioStatus::Off);
    assert_eq!(off.preload_stats(), PreloadStats::default());
    assert!(silent(&render(&mut off, 1000)));
}

#[test]
fn preload_is_owned_and_only_the_authored_cue_maps_to_a_clip() {
    let fixture = prepared();
    let mut pcm = crate::IMPACT_BYTES.to_vec();
    let objects = [ArtifactBytes {
        sha256: sha256(&pcm),
        bytes: &pcm,
    }];
    let mut owner = FixtureAudio::open_offline(
        &fixture,
        AudioMode::Required,
        ViewBundleBytes {
            manifest: crate::VIEW_BYTES,
            objects: &objects,
        },
    )
    .unwrap();
    pcm.fill(0);
    drop(pcm);
    assert_eq!(
        owner.preload_stats(),
        PreloadStats {
            records: 1,
            manifest_bytes: 72,
            cooked_bytes: 14408,
            decoded_bytes: 57600,
        }
    );
    owner.update(
        1,
        &batch(vec![event(1, EventStatus::Verified(Impact { cue: 999 }))]),
    );
    assert!(silent(&render(&mut owner, 1000)));
    owner.update(1, &batch(vec![verified(2)]));
    let output = render(&mut owner, 10000);
    assert!(audible(&output[..4000]));
    assert!(silent(&output[18000..]));
    assert!(output.chunks_exact(2).all(|frame| frame[0] == frame[1]));
    assert_eq!(owner.stats().started, 1);
    assert_eq!(owner.voice_count(), 0);
}

#[test]
fn prediction_confirmation_cancellation_and_same_key_replacement_keep_order() {
    let mut one = audio();
    let mut owner = audio();
    one.update(1, &batch(vec![predicted(1)]));
    owner.update(
        1,
        &batch(vec![predicted(1), predicted(1), verified(1), verified(1)]),
    );
    assert_eq!(render(&mut one, 10000), render(&mut owner, 10000));
    assert_eq!(owner.stats().started, 1);
    assert_eq!(owner.stats().confirmed, 1);
    assert_eq!(owner.stats().duplicates, 2);
    owner.update(1, &batch(vec![verified(1)]));
    assert!(silent(&render(&mut owner, 1000)));

    owner.update(1, &batch(vec![predicted(2)]));
    assert!(audible(&render(&mut owner, 500)));
    owner.update(1, &batch(vec![event(2, EventStatus::Canceled)]));
    let tail = render(&mut owner, 2000);
    assert!(audible(&tail[..256]));
    assert!(silent(&tail[2048..]));
    owner.update(1, &batch(vec![verified(2)]));
    assert!(silent(&render(&mut owner, 1000)));
    owner.update(
        1,
        &batch(vec![
            predicted(2),
            event(2, EventStatus::Canceled),
            predicted(2),
            verified(2),
        ]),
    );
    assert_eq!(owner.stats().started, 4);
    assert!(audible(&render(&mut owner, 1000)));
}

#[test]
fn ordered_lifecycle_resync_and_source_replacement_reach_the_same_owner() {
    let mut owner = audio();
    owner.update(7, &batch(vec![predicted(1)]));
    render(&mut owner, 500);
    owner.update(
        7,
        &batch(vec![
            BridgeEvent::Lifecycle(Lifecycle::Seeked { from: 20, to: 10 }),
            verified(1),
        ]),
    );
    assert!(silent(&render(&mut owner, 2000)[2048..]));
    owner.update(7, &batch(vec![verified(11)]));
    assert_eq!(owner.stats().started, 2);
    owner.update(
        7,
        &batch(vec![
            BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 11 }),
            BridgeEvent::Lifecycle(Lifecycle::Branched {
                tick: 11,
                dropped: 5,
            }),
            verified(12),
        ]),
    );
    assert!(silent(&render(&mut owner, 2000)[2048..]));
    owner.update(
        7,
        &batch(vec![
            BridgeEvent::Lifecycle(Lifecycle::Resumed { tick: 11 }),
            verified(12),
            verified(13),
        ]),
    );
    assert_eq!(owner.stats().started, 3);
    let mut update = batch(vec![verified(14)]);
    update.resync = Some(ViewResync {
        generation: 1,
        discarded_events: 3,
        head_tick: 20,
        verified_tick: 20,
        disconnected: false,
        last_desync: None,
        lifecycle: Vec::new(),
    });
    owner.update(7, &update);
    assert!(silent(&render(&mut owner, 2000)[2048..]));
    owner.update(
        7,
        &batch(vec![
            BridgeEvent::Lifecycle(Lifecycle::Disconnected),
            verified(21),
        ]),
    );
    assert_eq!(owner.stats().started, 3);
    owner.update(8, &batch(vec![verified(1)]));
    assert!(audible(&render(&mut owner, 500)));
    assert_eq!(owner.stats().started, 4);
    owner.update(
        8,
        &batch(vec![
            BridgeEvent::Lifecycle(Lifecycle::SessionStarted {
                tick_rate: TICK_RATE,
                player_count: PLAYER_COUNT,
                local_slot: PlayerSlot(0),
            }),
            verified(1),
        ]),
    );
    assert_eq!(owner.stats().started, 5);
}

fn bridge() -> InProc<AssetFixtureGame, PlayHost<AssetFixtureGame>> {
    let mut config = PlayConfig::new(PLAYER_COUNT, SEED, TICK_RATE);
    config.build_id = BUILD_ID;
    InProc::new(
        PlayHost::new(PlaySession::new(config, ()), PlayerSlot(0)),
        BridgeConfig::default(),
    )
}

#[test]
fn one_atomic_poll_per_tick_preserves_every_fixture_checksum_and_event() {
    let fixture = prepared();
    let mut plain = fixture.start();
    let mut enabled = bridge();
    let mut disabled = bridge();
    let mut owner = audio();
    let mut off = FixtureAudio::embedded_offline(&fixture, AudioMode::Off).unwrap();
    let mut was_audible = false;
    let mut event_count = 0;
    for tick in 1..=TICKS {
        let expected = plain.advance().unwrap();
        for bridge in [&mut enabled, &mut disabled] {
            bridge
                .set_input(
                    PlayerSlot(0),
                    Input {
                        axis: game::axis(tick),
                    },
                )
                .unwrap();
            bridge.step(1);
        }
        // Exactly one poll from each bridge. The complete same immutable batch
        // is checked by the visual/test consumer and forwarded to audio.
        let update = enabled.poll_view();
        let baseline = disabled.poll_view();
        assert_eq!(update.events, baseline.events);
        assert_eq!(
            update.snapshot.as_ref().unwrap().predicted().checksum(),
            expected.state.checksum
        );
        assert_eq!(
            baseline.snapshot.as_ref().unwrap().predicted().checksum(),
            expected.state.checksum
        );
        let actual: Vec<_> = update
            .events
            .iter()
            .filter_map(|event| match event {
                BridgeEvent::Sim {
                    key,
                    status: EventStatus::Verified(impact),
                } => Some(crate::CueEvent {
                    key: *key,
                    impact: *impact,
                }),
                _ => None,
            })
            .collect();
        assert_eq!(actual, expected.events);
        event_count += actual.len();
        owner.update(101, &update);
        off.update(202, &baseline);
        was_audible |= audible(&render(&mut owner, 800));
        assert!(silent(&render(&mut off, 800)));
    }
    assert!(was_audible);
    assert_eq!(event_count, 2);
    assert_eq!(owner.stats().started, 2);
    assert_eq!(off.stats().started, 0);
}

// Build a valid package and admission capability, including view-only releases.
fn with_package(frames: &[usize], run: impl FnOnce(&PreparedFixture, ViewBundleBytes<'_>)) {
    let payloads: Vec<Vec<u8>> = frames
        .iter()
        .enumerate()
        .map(|(i, &count)| {
            let mut bytes = Vec::with_capacity(8 + count * 2);
            bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
            bytes.extend_from_slice(&(count as u32).to_le_bytes());
            for _ in 0..count {
                bytes.extend_from_slice(&(i as i16 + 1).to_le_bytes());
            }
            bytes
        })
        .collect();
    let entries: Vec<_> = payloads
        .iter()
        .enumerate()
        .map(|(i, bytes)| ManifestEntry {
            id: AssetRef::from_raw(IMPACT_ID.get() + i as u64),
            type_id: orr_asset::IMPACT_PCM16_TYPE_ID,
            schema_version: 1,
            payload_len: bytes.len() as u64,
            payload_sha256: sha256(bytes),
        })
        .collect();
    let mut manifest = vec![0; manifest_encoded_len(Domain::View, entries.len()).unwrap()];
    encode_manifest(Domain::View, &entries, &mut manifest).unwrap();
    let objects: Vec<_> = payloads
        .iter()
        .map(|bytes| ArtifactBytes {
            sha256: sha256(bytes),
            bytes,
        })
        .collect();
    let mut full = objects.clone();
    full.push(ArtifactBytes {
        sha256: sha256(crate::MOTION_BYTES),
        bytes: crate::MOTION_BYTES,
    });
    let mut binding = prepared().release().clone();
    binding.view_digest = sha256(&manifest);
    let fixture = PreparedFixture::prepare(
        BundleBytes {
            sim_manifest: crate::SIM_BYTES,
            view_manifest: &manifest,
            objects: &full,
        },
        &binding,
    )
    .unwrap();
    run(
        &fixture,
        ViewBundleBytes {
            manifest: &manifest,
            objects: &objects,
        },
    );
}

#[test]
fn decoded_budget_exact_max_and_one_frame_over_are_checked_before_conversion() {
    let mut frames = vec![48000; 10];
    frames.push(MAX_DECODED_BYTES / STEREO_FRAME_BYTES - 480000);
    with_package(&frames, |fixture, bundle| {
        let bank = ClipBank::preload(fixture, bundle).unwrap();
        assert_eq!(bank.stats.decoded_bytes, MAX_DECODED_BYTES);
        assert_eq!(bank.stats.records, 11);
        assert!(matches!(
            FixtureAudio::embedded_offline(fixture, AudioMode::Required),
            Err(Error::DigestMismatch)
        ));
    });
    *frames.last_mut().unwrap() += 1;
    with_package(&frames, |fixture, bundle| {
        assert!(matches!(
            ClipBank::preload(fixture, bundle),
            Err(Error::BudgetExceeded)
        ));
        assert!(
            matches!(FixtureAudio::open_offline(fixture, AudioMode::Auto, bundle).unwrap().status(),
            AudioStatus::Muted { reason } if reason.contains("BudgetExceeded"))
        );
    });
}

#[test]
fn byte_record_and_pcm_limits_cover_maximum_and_plus_one() {
    for limit in [
        orr_asset::MAX_MANIFEST_BYTES,
        orr_asset::MAX_VIEW_PAYLOAD_BYTES as usize,
        MAX_DECODED_BYTES,
    ] {
        assert_eq!(bounded_add(limit - 1, 1, limit).unwrap(), limit);
        assert!(matches!(
            bounded_add(limit, 1, limit),
            Err(Error::BudgetExceeded)
        ));
        assert!(matches!(
            bounded_add(usize::MAX, 1, limit),
            Err(Error::BudgetExceeded)
        ));
    }
    with_package(&[1; orr_asset::MAX_VIEW_RECORDS], |fixture, bundle| {
        assert_eq!(
            ClipBank::preload(fixture, bundle).unwrap().stats.records,
            16
        );
        let mut too_many = bundle.objects.to_vec();
        too_many.push(bundle.objects[0]);
        assert!(matches!(
            ClipBank::preload(
                fixture,
                ViewBundleBytes {
                    objects: &too_many,
                    ..bundle
                }
            ),
            Err(Error::BudgetExceeded)
        ));
    });
    assert!(manifest_encoded_len(Domain::View, orr_asset::MAX_VIEW_RECORDS + 1).is_err());
    with_package(&[orr_asset::MAX_PCM_FRAMES as usize], |fixture, bundle| {
        assert_eq!(
            ClipBank::preload(fixture, bundle)
                .unwrap()
                .stats
                .decoded_bytes,
            384000
        );
    });
    let entry = ManifestEntry {
        id: IMPACT_ID,
        type_id: orr_asset::IMPACT_PCM16_TYPE_ID,
        schema_version: 1,
        payload_len: 8 + (u64::from(orr_asset::MAX_PCM_FRAMES) + 1) * 2,
        payload_sha256: [0; 32],
    };
    assert!(entry.validate(Domain::View).is_err());
    for len in [
        orr_asset::MAX_MANIFEST_BYTES,
        orr_asset::MAX_MANIFEST_BYTES + 1,
    ] {
        assert!(Manifest::decode(&vec![0; len], Domain::View).is_err());
    }
}

#[test]
fn voice_and_history_caps_remain_the_existing_mixer_policy() {
    let mut owner = audio();
    owner.update(1, &batch((1..=33).map(predicted).collect()));
    assert_eq!(owner.stats().started, 32);
    assert_eq!(owner.stats().dropped, 1);
    assert_eq!(owner.voice_count(), 32);
    // Drain before checking PCM amplitude: 32 simultaneous valid clips are
    // allowed by the mixer and clipping/limiting is not an asset guarantee.
    render(&mut owner, 10000);
    owner.update(
        1,
        &batch(
            (34..=4200)
                .map(|tick| event(tick, EventStatus::Verified(Impact { cue: 0 })))
                .collect(),
        ),
    );
    assert!(owner.history_len() <= 4096);
    owner.update(1, &batch(vec![verified(1)]));
    assert_eq!(owner.stats().started, 32);
}

// A local comparator with identical registered state, inputs and events. Only
// the motion source differs: a constant raw 4096 instead of typed table resolve.
struct NoAssetGame;
impl orr_sim::Game for NoAssetGame {
    type Input = Input;
    type Command = game::NoCommand;
    type Event = Impact;
    type Config = ();
    fn register(builder: &mut orr_ecs::ComponentRegistryBuilder) {
        <AssetFixtureGame as orr_sim::Game>::register(builder);
    }
    fn setup(frame: &mut orr_ecs::Frame, config: &()) {
        <AssetFixtureGame as orr_sim::Game>::setup(frame, config);
    }
    fn systems() -> Vec<Box<dyn orr_sim::System<Self>>> {
        vec![Box::new(ConstantMotion)]
    }
}
struct ConstantMotion;
impl orr_sim::System<NoAssetGame> for ConstantMotion {
    fn name(&self) -> &'static str {
        "ConstantMotionBaseline"
    }
    fn run(&mut self, ctx: &mut orr_sim::SimContext<NoAssetGame>) {
        // Match the fixture system's test-only instrumentation.
        game::TICK_CALLS.with(|count| count.set(count.get() + 1));
        let axis = ctx.inputs.input(PlayerSlot(0)).axis;
        for (_, (_, position)) in ctx.frame.query::<(&game::Motion, &mut game::Position)>() {
            position.x += orr_fp::FP::from_raw(4096 * i64::from(axis));
        }
        if ctx.tick == 1 || ctx.tick == 181 {
            ctx.emit(Impact { cue: 1 });
        }
    }
}

#[test]
#[ignore = "informational timing only; run explicitly in the release profile"]
fn measure_asset_tick_and_preload_baselines() {
    use std::hint::black_box;
    use std::time::Instant;
    // First establish that this comparator changes no per-tick state or events.
    let mut asset = game::new_simulation();
    let mut baseline =
        orr_sim::Simulation::<NoAssetGame>::with_build_id((), TICK_RATE, SEED, BUILD_ID);
    for tick in 1..=TICKS {
        assert_eq!(
            asset
                .step(&game::inputs(tick))
                .into_iter()
                .map(|event| (event.key, event.payload))
                .collect::<Vec<_>>(),
            baseline
                .step(&game::inputs(tick))
                .into_iter()
                .map(|event| (event.key, event.payload))
                .collect::<Vec<_>>()
        );
        assert_eq!(asset.checksum(), baseline.checksum());
    }
    let mut measured = [std::time::Duration::ZERO; 2];
    const RUNS: u32 = 1000;
    for round in 0..RUNS {
        // Alternate order to reduce warm-cache/order bias. Setup excluded.
        for which in [round % 2, 1 - round % 2] {
            if which == 0 {
                let mut sim = game::new_simulation();
                let now = Instant::now();
                for tick in 1..=TICKS {
                    black_box(sim.step(black_box(&game::inputs(tick))));
                }
                measured[0] += now.elapsed();
                black_box(sim.checksum());
            } else {
                let mut sim = orr_sim::Simulation::<NoAssetGame>::with_build_id(
                    (),
                    TICK_RATE,
                    SEED,
                    BUILD_ID,
                );
                let now = Instant::now();
                for tick in 1..=TICKS {
                    black_box(sim.step(black_box(&game::inputs(tick))));
                }
                measured[1] += now.elapsed();
                black_box(sim.checksum());
            }
        }
    }
    println!("tick baseline: {} ticks per case, asset={} ns/tick, constant={} ns/tick (setup/checksum excluded; input/event allocations included)",
        u64::from(RUNS) * TICKS, measured[0].as_nanos() / (u128::from(RUNS) * u128::from(TICKS)),
        measured[1].as_nanos() / (u128::from(RUNS) * u128::from(TICKS)));
    let value = orr_asset::MotionProfileV1::decode(&4096_i64.to_le_bytes()).unwrap();
    let records: Vec<_> = (1..=orr_asset::MAX_SIM_RECORDS)
        .map(|id| (AssetRef::from_raw(id as u64), value))
        .collect();
    let table = orr_asset::SimTable::new(&records).unwrap();
    let entries: Vec<_> = records
        .iter()
        .map(|(id, _)| ManifestEntry {
            id: *id,
            type_id: orr_asset::MOTION_PROFILE_TYPE_ID,
            schema_version: 1,
            payload_len: 8,
            payload_sha256: sha256(&value.encode()),
        })
        .collect();
    let mut encoded = vec![0; manifest_encoded_len(Domain::Sim, entries.len()).unwrap()];
    encode_manifest(Domain::Sim, &entries, &mut encoded).unwrap();
    let manifest = Manifest::decode(&encoded, Domain::Sim).unwrap();
    let now = Instant::now();
    for i in 0..1_000_000 {
        let id = AssetRef::from_raw(1 + i % 64);
        let typed = manifest
            .typed_ref::<orr_asset::MotionProfileV1>(black_box(id))
            .unwrap();
        black_box(table.resolve(typed).unwrap());
    }
    println!(
        "64-entry manifest typed-ref + table resolve: {} ns/lookup",
        now.elapsed().as_nanos() / 1_000_000
    );
    let fixture = prepared();
    let now = Instant::now();
    let bank = FixtureAudio::embedded_offline(&fixture, AudioMode::Required).unwrap();
    println!(
        "embedded preload plus offline mixer: {} us; {:?}",
        now.elapsed().as_micros(),
        bank.preload_stats()
    );
    black_box(bank);
}
