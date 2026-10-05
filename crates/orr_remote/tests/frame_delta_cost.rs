//! Matched data-plane cost probe; run the ignored test only in a quiet release lane.
#![allow(clippy::disallowed_types)] // Tool-side monotonic wall-clock measurement.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use orr_ecs::{ComponentRegistry, Frame};
use orr_remote::frame_delta::{DecodeError, Decoder, Encoder, FrameRecord, FrameScope};
use orr_remote::wire::{self, FrameCodecLimits, FrameCodecMeta, DEFAULT_FRAME_CODEC_LIMITS};
use orr_sample::physics_game::{dynamic_count, PhysConfig, PhysGame, SceneMode};
use orr_sim::{Simulation, TickInputs};
use serde_json::{json, Value};

const STEPS: usize = 32;
const PASSES: usize = 5;

struct Sample {
    frame: Frame,
    raw_len: usize,
    scope: FrameScope,
    sequence: u64,
}

struct Case {
    name: &'static str,
    samples: Vec<Sample>,
}

fn sample(frame: &Frame, generation: u64, timeline: u64, sequence: u64) -> Sample {
    Sample {
        frame: frame.clone(),
        raw_len: frame.to_bytes().len(),
        scope: FrameScope {
            stream_generation: generation,
            play_epoch: 1,
            timeline_epoch: timeline,
        },
        sequence,
    }
}

fn fixture() -> (Arc<ComponentRegistry>, Vec<Case>) {
    // Same normal PhysGame fixture as measure.rs: 1,000 dynamic bodies, plus
    // the game's walls, obstacles and two paddles. No new bodies are spawned.
    let mut sim = Simulation::<PhysGame>::new(PhysConfig::new(1000, SceneMode::Rain), 60, 7);
    assert_eq!(dynamic_count(sim.frame_mut()), 1000);
    let registry = Arc::clone(sim.registry());
    let mut frames = vec![sim.frame().clone()];
    for _ in 0..STEPS {
        sim.step(&TickInputs::new(sim.tick() + 1, 2));
        frames.push(sim.frame().clone());
    }
    let first = frames[0].to_bytes();
    let second = frames[1].to_bytes();
    assert_ne!(first[16..first.len() - 8], second[16..second.len() - 8]);
    let idle = (0..=STEPS)
        .map(|i| {
            // Controlled idle payload, not a claim that the physics has settled.
            let mut frame = frames[0].clone();
            frame.set_tick(i as u64);
            sample(&frame, 1, 1, i as u64 + 1)
        })
        .collect();
    let changing = frames
        .iter()
        .enumerate()
        .map(|(i, frame)| sample(frame, 1, 1, i as u64 + 1))
        .collect();

    // Restore and re-simulate real snapshots outside any measured region.
    sim.restore(&frames[8]);
    let mut rollback = vec![sample(&frames[16], 1, 1, 1), sample(sim.frame(), 1, 2, 2)];
    for (i, original) in frames.iter().enumerate().take(17).skip(9) {
        sim.step(&TickInputs::new(sim.tick() + 1, 2));
        assert_frame(original, sim.frame());
        rollback.push(sample(sim.frame(), 1, 2, (i - 6) as u64));
    }
    let seek = vec![
        sample(&frames[32], 1, 1, 1),
        sample(&frames[8], 1, 2, 2),
        sample(&frames[9], 1, 2, 3),
    ];
    let reconnect = vec![
        sample(&frames[31], 1, 1, 1),
        sample(&frames[31], 2, 1, 1),
        sample(&frames[32], 2, 1, 2),
    ];
    (
        registry,
        vec![
            Case {
                name: "idle_tick_only",
                samples: idle,
            },
            Case {
                name: "changing_physics",
                samples: changing,
            },
            Case {
                name: "rollback_resimulation",
                samples: rollback,
            },
            Case {
                name: "backward_seek",
                samples: seek,
            },
            Case {
                name: "reconnect",
                samples: reconnect,
            },
        ],
    )
}

fn assert_frame(expected: &Frame, actual: &Frame) {
    assert_eq!(actual.to_bytes(), expected.to_bytes());
    assert_eq!(actual.checksum(), expected.checksum());
    assert_eq!(actual.tick(), expected.tick());
}

fn metadata(sample: &Sample) -> Value {
    // Fixed shared metadata keeps both sides comparable and bytes repeatable.
    // This is a codec/data-plane probe, not a capture of a live host session.
    json!({
        "tick": sample.frame.tick(), "epoch": 1, "tick_rate": 60,
        "player_count": 2, "sent_at_us": 1791216000000000_u64,
        "mode": "play", "timeline": null,
        "delivery": {
            "subscription": sample.scope.stream_generation.to_string(),
            "timeline": sample.scope.timeline_epoch.to_string(),
            "through_cursor": "0", "count": "0", "loss_generation": "0", "lifecycle": []
        }
    })
}

fn negotiated_metadata(sample: &Sample) -> Value {
    let mut meta = metadata(sample);
    meta["play_epoch"] = json!(sample.scope.play_epoch.to_string());
    meta
}

fn encode_negotiated(
    encoder: &mut Encoder,
    sample: &Sample,
    meta: &Value,
    limits: FrameCodecLimits,
) -> Vec<u8> {
    let identity = FrameCodecMeta {
        subscription: sample.scope.stream_generation,
        sequence: sample.sequence,
        reset_generation: sample.scope.timeline_epoch,
    };
    let prepared = encoder
        .prepare(&sample.frame, sample.scope)
        .expect("prepare");
    // Match FrameCodecState::send_frame's actual compressed-envelope choice,
    // not Encoder::encode's uncompressed heuristic. Include both candidates' work.
    let full =
        wire::encode_frame_record_message(meta, identity, prepared.full_record(), limits).ok();
    let delta = prepared
        .delta_record()
        .and_then(|record| wire::encode_frame_record_message(meta, identity, record, limits).ok());
    let bytes = match (full, delta) {
        (Some(full), Some(delta)) if delta.len() < full.len() => delta,
        (Some(full), _) => full,
        (None, Some(delta)) => delta,
        (None, None) => panic!("neither candidate fits"),
    };
    encoder.commit(prepared).expect("commit encoder");
    bytes
}

#[derive(Default)]
struct Costs {
    bytes: Vec<usize>,
    encode_ns: Vec<u128>,
    decode_ns: Vec<u128>,
    full: usize,
    delta: usize,
    encoder_baseline_max: usize,
    decoder_baseline_max: usize,
}

fn legacy_pass(registry: &Arc<ComponentRegistry>, case: &Case) -> Costs {
    let mut costs = Costs::default();
    for sample in &case.samples {
        let meta = metadata(sample);
        let start = Instant::now();
        let message =
            wire::encode_frame_message(black_box(&meta), &black_box(&sample.frame).to_bytes());
        let encode_ns = start.elapsed().as_nanos();
        let start = Instant::now();
        let (_, raw) = wire::decode_frame_message(black_box(&message)).expect("legacy envelope");
        let decoded = Frame::from_bytes(Arc::clone(registry), &raw).expect("legacy Frame");
        let decode_ns = start.elapsed().as_nanos();
        assert_frame(&sample.frame, &decoded);
        costs.bytes.push(message.len());
        costs.encode_ns.push(encode_ns);
        costs.decode_ns.push(decode_ns);
        costs.full += 1;
    }
    costs
}

fn negotiated_pass(registry: &Arc<ComponentRegistry>, case: &Case, baseline_cap: usize) -> Costs {
    let limits = FrameCodecLimits {
        max_baseline_bytes: baseline_cap,
        ..DEFAULT_FRAME_CODEC_LIMITS
    };
    let mut encoder = Encoder::new(limits.max_frame_bytes, limits.max_baseline_bytes);
    let mut decoder = Decoder::new(
        Arc::clone(registry),
        limits.max_frame_bytes,
        limits.max_baseline_bytes,
    );
    let mut costs = Costs::default();
    let mut previous_scope: Option<FrameScope> = None;
    for sample in &case.samples {
        // Reconnect discards both old endpoint states. Timeline resets use
        // an explicit Full with the new scope, as in the production codec.
        if previous_scope.is_some_and(|s| s.stream_generation != sample.scope.stream_generation) {
            encoder.reset();
            decoder.reset();
        }
        let meta = negotiated_metadata(sample);
        let start = Instant::now();
        let message = encode_negotiated(&mut encoder, black_box(sample), black_box(&meta), limits);
        let encode_ns = start.elapsed().as_nanos();
        let start = Instant::now();
        let (_, identity, record) = wire::decode_frame_record_message(black_box(&message), limits)
            .expect("negotiated envelope");
        let decoded = decoder.decode(&record).expect("reconstruct Frame");
        let decode_ns = start.elapsed().as_nanos();
        assert_eq!(identity.sequence, sample.sequence);
        assert_eq!(identity.subscription, sample.scope.stream_generation);
        assert_eq!(identity.reset_generation, sample.scope.timeline_epoch);
        assert_frame(&sample.frame, &decoded);
        let full_required = previous_scope != Some(sample.scope) || baseline_cap == 0;
        if full_required {
            assert!(matches!(record, FrameRecord::Full { .. }));
        }
        match record {
            FrameRecord::Full { .. } => costs.full += 1,
            FrameRecord::Delta { .. } => costs.delta += 1,
        }
        let expected_retention = if sample.raw_len <= baseline_cap {
            sample.raw_len
        } else {
            0
        };
        assert_eq!(encoder.retained_baseline_bytes(), expected_retention);
        assert_eq!(decoder.retained_baseline_bytes(), expected_retention);
        costs.encoder_baseline_max = costs
            .encoder_baseline_max
            .max(encoder.retained_baseline_bytes());
        costs.decoder_baseline_max = costs
            .decoder_baseline_max
            .max(decoder.retained_baseline_bytes());
        assert!(costs.encoder_baseline_max <= baseline_cap);
        assert!(costs.decoder_baseline_max <= baseline_cap);
        costs.bytes.push(message.len());
        costs.encode_ns.push(encode_ns);
        costs.decode_ns.push(decode_ns);
        previous_scope = Some(sample.scope);
    }
    costs
}

#[test]
fn thousand_body_lifecycle_reconstruction_and_bounds() {
    let (registry, cases) = fixture();
    for case in &cases {
        let legacy = legacy_pass(&registry, case);
        let negotiated = negotiated_pass(
            &registry,
            case,
            DEFAULT_FRAME_CODEC_LIMITS.max_baseline_bytes,
        );
        assert_eq!(legacy.bytes.len(), negotiated.bytes.len());
        let no_base = negotiated_pass(&registry, case, 0);
        assert_eq!(no_base.delta, 0);
    }
    let idle = &cases[0];
    let raw_len = idle.samples[0].frame.to_bytes().len();
    assert_eq!(
        negotiated_pass(&registry, idle, raw_len).encoder_baseline_max,
        raw_len
    );
    assert_eq!(
        negotiated_pass(&registry, idle, raw_len - 1).encoder_baseline_max,
        0
    );
}

#[test]
fn thousand_body_missing_and_stale_base_require_explicit_full() {
    let (registry, cases) = fixture();
    let samples = &cases[0].samples;
    let limits = DEFAULT_FRAME_CODEC_LIMITS;
    let mut encoder = Encoder::new(limits.max_frame_bytes, limits.max_baseline_bytes);
    let mut records = Vec::new();
    for sample in &samples[..3] {
        let bytes = encode_negotiated(&mut encoder, sample, &negotiated_metadata(sample), limits);
        records.push(wire::decode_frame_record_message(&bytes, limits).unwrap().2);
    }
    assert!(matches!(records[2], FrameRecord::Delta { .. }));
    let mut decoder = Decoder::new(registry, limits.max_frame_bytes, limits.max_baseline_bytes);
    assert!(matches!(
        decoder.decode(&records[2]),
        Err(DecodeError::NeedFull(_))
    ));
    assert_eq!(decoder.retained_baseline_bytes(), 0);
    decoder.decode(&records[0]).expect("initial Full");
    let before = decoder.baseline_stamp();
    assert!(matches!(
        decoder.decode(&records[2]),
        Err(DecodeError::NeedFull(_))
    ));
    assert_eq!(decoder.baseline_stamp(), before);
    encoder.reset();
    let reset = encode_negotiated(
        &mut encoder,
        &samples[2],
        &negotiated_metadata(&samples[2]),
        limits,
    );
    let (_, _, full) = wire::decode_frame_record_message(&reset, limits).unwrap();
    assert!(matches!(full, FrameRecord::Full { .. }));
    assert_frame(
        &samples[2].frame,
        &decoder.decode(&full).expect("explicit Full recovery"),
    );
}

fn distribution(values: &[u128]) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    json!({"mean": sorted.iter().sum::<u128>() / sorted.len() as u128,
        "p50": sorted[sorted.len() / 2],
        "p95": sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)]})
}

fn report(pass: usize, case: &Case, mode: &str, costs: Costs) {
    println!(
        "{}",
        json!({
            "pass": pass, "case": case.name, "mode": mode, "frames": costs.bytes.len(),
            "erp_binary_bytes_total": costs.bytes.iter().sum::<usize>(),
            "erp_binary_bytes_first": costs.bytes[0],
            "erp_binary_bytes_min": costs.bytes.iter().min(),
            "erp_binary_bytes_max": costs.bytes.iter().max(),
            "full_records": costs.full, "delta_records": costs.delta,
            "encode_wall_ns": distribution(&costs.encode_ns),
            "decode_wall_ns": distribution(&costs.decode_ns),
            "encoder_retained_baseline_bytes_max": costs.encoder_baseline_max,
            "decoder_retained_baseline_bytes_max": costs.decoder_baseline_max,
            "first_frame_checksum": wire::checksum_text(case.samples[0].frame.checksum()),
            "last_frame_checksum": wire::checksum_text(case.samples.last().unwrap().frame.checksum())
        })
    );
}

#[test]
#[ignore = "cost probe: run in an explicitly reserved, quiet release lane"]
fn measure_full_lz4_vs_negotiated() {
    assert!(
        !black_box(cfg!(debug_assertions)),
        "cost probe requires --release"
    );
    let (registry, cases) = fixture();
    println!(
        "{}",
        json!({"probe": "frame_delta_cost_v1", "dynamic_bodies": 1000,
        "fixture": "PhysGame Rain / layout_seed 0x0DDB_A110 / sim_seed 7 / 60 Hz / 2 players / neutral input",
        "warmup_passes": 1, "measured_passes": PASSES,
        "timing": "monotonic elapsed wall time; CPU cost proxy, not exclusive CPU time",
        "byte_scope": "ERP binary messages only; excludes transport and reset/negotiation control messages",
        "baseline_cap": DEFAULT_FRAME_CODEC_LIMITS.max_baseline_bytes,
        "frame_cap": DEFAULT_FRAME_CODEC_LIMITS.max_frame_bytes})
    );
    for case in &cases {
        black_box(legacy_pass(&registry, case));
        black_box(negotiated_pass(
            &registry,
            case,
            DEFAULT_FRAME_CODEC_LIMITS.max_baseline_bytes,
        ));
    }
    for pass in 0..PASSES {
        for case in &cases {
            let (legacy, negotiated) = if pass % 2 == 0 {
                let legacy = legacy_pass(&registry, case);
                (
                    legacy,
                    negotiated_pass(
                        &registry,
                        case,
                        DEFAULT_FRAME_CODEC_LIMITS.max_baseline_bytes,
                    ),
                )
            } else {
                let negotiated = negotiated_pass(
                    &registry,
                    case,
                    DEFAULT_FRAME_CODEC_LIMITS.max_baseline_bytes,
                );
                (legacy_pass(&registry, case), negotiated)
            };
            report(pass, case, "legacy_full_lz4", legacy);
            report(pass, case, "negotiated", negotiated);
        }
    }
}
