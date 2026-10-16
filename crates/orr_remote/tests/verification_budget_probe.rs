//! Explicitly invoked public-API fixture for the verification-budget probe.
//!
//! This file intentionally contains no private timing hooks. The Python driver
//! selects one cell per fresh test-process invocation with
//! `ORR_VERIFICATION_BUDGET_CELL`; normal Cargo test runs leave the probe
//! ignored.
#![allow(clippy::disallowed_types)]

use std::io::{self, Write};
use std::time::Instant;

use orr_edit::{EditorDoc, PlayController, VerifyInputs, VerifyOptions};
use orr_reflect::TypeRegistry;
use orr_remote::{call_local, Caps, ErpTarget, GameHooks, HostLimits};
use orr_sample::physics_game::{register_reflect, PhysGame, PhysInput, PhysMetrics};
use orr_session::{ControlOp, ReplayReader};
use orr_sim::{PlayerSlot, Simulation};
use serde_json::{json, Value as JsonValue};

const PROTOCOL: u64 = 1;
const PREFIX: &str = "ORR_VERIFICATION_BUDGET_PROBE ";
const SCENE_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scenes/physics_demo.scene.yaml"
));
const FIXTURE_SEED: u64 = 7;
const PLAYERS: u8 = 2;
const REPLAY_TICKS: u64 = 256;
const EXECUTION_TICKS: u32 = 16;
// This generated fixture records the initial boundary and every executed tick.
const EXECUTION_CHECKSUM_BOUNDARIES: u32 = EXECUTION_TICKS + 1;
const MAX_YAML_BYTES: usize = 256 * 1024;
const MAX_ENTITIES: usize = 128;
const MAX_REPLAY_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    CoreIdle { ticks: u32, parallel: bool },
    CoreReplay { parallel: bool },
    LocalIdle { series: bool },
    LocalReplay,
}

impl Cell {
    fn name(self) -> &'static str {
        match self {
            Self::CoreIdle {
                ticks: 15,
                parallel: false,
            } => "core_idle15_serial",
            Self::CoreIdle {
                ticks: 15,
                parallel: true,
            } => "core_idle15_parallel",
            Self::CoreIdle {
                ticks: 16,
                parallel: false,
            } => "core_idle16_serial",
            Self::CoreIdle {
                ticks: 16,
                parallel: true,
            } => "core_idle16_parallel",
            Self::CoreReplay { parallel: false } => "core_replay16_serial",
            Self::CoreReplay { parallel: true } => "core_replay16_parallel",
            Self::LocalIdle { series: false } => "local_idle16_series_off",
            Self::LocalIdle { series: true } => "local_idle16_series_on",
            Self::LocalReplay => "local_replay16_series_on",
            _ => unreachable!("Cell variants are restricted to the approved matrix"),
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "core_idle15_serial" => Self::CoreIdle {
                ticks: 15,
                parallel: false,
            },
            "core_idle15_parallel" => Self::CoreIdle {
                ticks: 15,
                parallel: true,
            },
            "core_idle16_serial" => Self::CoreIdle {
                ticks: 16,
                parallel: false,
            },
            "core_idle16_parallel" => Self::CoreIdle {
                ticks: 16,
                parallel: true,
            },
            "core_replay16_serial" => Self::CoreReplay { parallel: false },
            "core_replay16_parallel" => Self::CoreReplay { parallel: true },
            "local_idle16_series_off" => Self::LocalIdle { series: false },
            "local_idle16_series_on" => Self::LocalIdle { series: true },
            "local_replay16_series_on" => Self::LocalReplay,
            _ => return None,
        })
    }

    fn execution_ticks(self) -> u32 {
        match self {
            Self::CoreIdle { ticks, .. } => ticks,
            Self::CoreReplay { .. } | Self::LocalIdle { .. } | Self::LocalReplay => EXECUTION_TICKS,
        }
    }

    fn requested_parallel(self) -> Option<bool> {
        match self {
            Self::CoreIdle { parallel, .. } | Self::CoreReplay { parallel } => Some(parallel),
            Self::LocalIdle { .. } | Self::LocalReplay => None,
        }
    }

    fn effective_parallel(self) -> bool {
        self.requested_parallel().unwrap_or(true) && self.execution_ticks() >= 16
    }

    fn series(self) -> bool {
        matches!(self, Self::LocalIdle { series: true } | Self::LocalReplay)
    }
}

struct Fixture {
    doc: EditorDoc,
    initial_frame_bytes: Vec<u8>,
    initial_checksum: u64,
    replay: Option<Vec<u8>>,
    replay_base64: Option<String>,
    replay_ticks: Option<usize>,
    replay_keyframes: Option<usize>,
    entity_count: usize,
}

fn fixture(include_replay: bool) -> Fixture {
    assert!(
        SCENE_YAML.len() <= MAX_YAML_BYTES,
        "physics_demo YAML exceeds the reviewed byte cap"
    );
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    let doc = EditorDoc::from_yaml(
        SCENE_YAML,
        types,
        Simulation::<PhysGame>::build_registry(),
        FIXTURE_SEED,
    )
    .expect("load the ordinary physics_demo fixture");
    let entity_count = doc.view().entities().len();
    assert!(
        entity_count <= MAX_ENTITIES,
        "physics_demo exceeds reviewed entity cap"
    );
    assert_eq!(
        entity_count, 49,
        "physics_demo fixture entity count remains the reviewed baseline"
    );
    let initial_frame_bytes = doc.frame().to_bytes();
    let initial_checksum = doc.checksum();

    // Generate an ordinary, valid replay through the public recording API.
    // The fixture is intentionally longer than the 16-tick executed prefix.
    let (replay, replay_base64, replay_ticks, replay_keyframes) = if include_replay {
        let mut play = PlayController::<PhysGame>::start_play(&doc, doc.play_config(PLAYERS, 60))
            .expect("start fixture play");
        for _ in 0..REPLAY_TICKS {
            for slot in 0..PLAYERS {
                play.session_mut()
                    .set_input(PlayerSlot(slot), PhysInput::default());
            }
            play.control(ControlOp::Step(1));
        }
        let stopped = play.stop_play();
        assert_eq!(
            stopped.tick, REPLAY_TICKS,
            "recorded fixture has the expected source length"
        );
        let replay = stopped.replay;
        assert!(
            replay.len() <= MAX_REPLAY_BYTES,
            "decoded ORRP bytes exceed reviewed cap"
        );
        let parsed =
            ReplayReader::<PhysGame>::parse(&replay).expect("generated fixture replay parses");
        let replay_ticks = parsed.tick_count();
        let replay_keyframes = parsed.keyframe_count();
        assert_eq!(
            replay_ticks, REPLAY_TICKS as usize,
            "replay retains all source ticks"
        );
        assert_eq!(parsed.first_tick(), 1);
        assert_eq!(parsed.last_tick(), REPLAY_TICKS);
        assert_eq!(
            parsed.checksums.first().copied(),
            Some((0, initial_checksum))
        );
        for tick in 0..=u64::from(EXECUTION_TICKS) {
            assert!(
                parsed
                    .checksums
                    .iter()
                    .any(|&(recorded_tick, _)| recorded_tick == tick),
                "generated replay retains every executed checksum boundary"
            );
        }
        assert!(
            parsed.keyframe_count() > 0,
            "generated recording contains a keyframe"
        );
        let replay_base64 = orr_remote::codec::b64_encode(&replay);
        (
            Some(replay),
            Some(replay_base64),
            Some(replay_ticks),
            Some(replay_keyframes),
        )
    } else {
        (None, None, None, None)
    };

    Fixture {
        doc,
        initial_frame_bytes,
        initial_checksum,
        replay,
        replay_base64,
        replay_ticks,
        replay_keyframes,
        entity_count,
    }
}

fn emit(cell: &str, event: &str, fields: JsonValue) {
    let mut record = json!({"protocol": PROTOCOL, "event": event, "cell": cell});
    if let (Some(target), Some(extra)) = (record.as_object_mut(), fields.as_object()) {
        target.extend(
            extra
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    println!("{PREFIX}{record}");
    io::stdout().flush().expect("flush probe JSONL record");
}

fn timed<T>(cell: &str, phase: &'static str, call: impl FnOnce() -> T) -> (T, u64) {
    emit(cell, "phase_start", json!({"phase": phase}));
    let started = Instant::now();
    let output = call();
    let elapsed_ns =
        u64::try_from(started.elapsed().as_nanos()).expect("phase duration fits u64 nanoseconds");
    emit(
        cell,
        "phase_end",
        json!({"phase": phase, "elapsed_ns": elapsed_ns}),
    );
    (output, elapsed_ns)
}

fn counts_from_report(report: &orr_edit::VerifyReport) -> JsonValue {
    json!({
        "checksum_samples": report.samples.len(),
        "metric_comparisons": report.metrics.len(),
        "metric_series_values": report.metrics.iter().map(|m| m.base.series.len() + m.candidate.series.len()).sum::<usize>(),
        "returned_json_bytes": null,
    })
}

fn counts_from_json(report: &JsonValue) -> JsonValue {
    let metrics = report.get("metrics").and_then(JsonValue::as_array);
    let metric_series_values = metrics.map_or(0, |items| {
        items
            .iter()
            .map(|item| {
                ["base", "candidate"]
                    .into_iter()
                    .map(|side| {
                        item.get(side)
                            .and_then(|stats| stats.get("series"))
                            .and_then(JsonValue::as_array)
                            .map_or(0, Vec::len)
                    })
                    .sum::<usize>()
            })
            .sum::<usize>()
    });
    json!({
        "checksum_samples": report.get("samples").and_then(JsonValue::as_array).map_or(0, Vec::len),
        "metric_comparisons": metrics.map_or(0, Vec::len),
        "metric_series_values": metric_series_values,
        "returned_json_bytes": null,
    })
}

fn verify_unchanged(fixture: &Fixture) {
    assert_eq!(
        fixture.doc.checksum(),
        fixture.initial_checksum,
        "verification must not change scene checksum"
    );
    assert_eq!(
        fixture.doc.frame().to_bytes(),
        fixture.initial_frame_bytes,
        "verification must not change the full frame bytes"
    );
}

fn run_cell(cell: Cell) {
    let cell_name = cell.name();
    let uses_replay = matches!(cell, Cell::CoreReplay { .. } | Cell::LocalReplay);
    let mut fixture = fixture(uses_replay);
    let frame_bytes = fixture.initial_frame_bytes.len();
    let frame = fixture.doc.frame();
    let mut fixture_json = json!({
        "game": "PhysGame",
        "yaml_bytes": SCENE_YAML.len(),
        "entities": fixture.entity_count,
        "players": PLAYERS,
        "frame_bytes": frame_bytes,
        "replay_base64_bytes": fixture.replay_base64.as_ref().map(String::len),
        "replay_file_bytes": fixture.replay.as_ref().map(Vec::len),
        "replay_decompressed_body_bytes": null,
        "replay_ticks": fixture.replay_ticks,
        "keyframes": fixture.replay_keyframes,
        "initial_checksum": format!("0x{:016x}", fixture.initial_checksum),
    });
    if let Some(encoded) = &fixture.replay_base64 {
        fixture_json["replay_base64"] = json!(encoded);
    }
    emit(cell_name, "start", json!({"fixture": fixture_json}));
    let (base, base_ns) = timed(cell_name, "snapshot_clone_base", || frame.clone());
    let (candidate, candidate_ns) = timed(cell_name, "snapshot_clone_candidate", || frame.clone());
    // Keep both proxy outputs observable even in synchronous-local cells,
    // where verification obtains its own snapshots instead of using these.
    // The optimizer barrier is outside both reported clone intervals.
    std::hint::black_box((&base, &candidate));

    let mut replay_parse_ns = None;
    let mut core_verify_ns = None;
    let mut call_local_ns = None;
    let mut returned_json_encode_ns = None;
    let mut report_counts: JsonValue;
    let report_identical: Option<bool>;
    let mut recording_matches = None;
    let mut recording_checksums_checked = None;
    let checksums: JsonValue;
    let mut returned_json_bytes = None;
    let effective_parallel = cell.effective_parallel();

    match cell {
        Cell::CoreIdle { ticks, parallel } => {
            let inputs =
                VerifyInputs::<PhysGame>::scripted(ticks, PLAYERS, |_, _| PhysInput::default());
            let options = VerifyOptions {
                max_ticks: Some(ticks),
                parallel,
                ..VerifyOptions::default()
            };
            let (report, elapsed) = timed(cell_name, "core_verify", || {
                orr_edit::verify_frames::<PhysGame>(
                    &base,
                    &candidate,
                    &inputs,
                    &PhysMetrics,
                    &options,
                )
            });
            let report = report.expect("idle core verification succeeds");
            assert_eq!(report.ticks, u64::from(ticks));
            assert!(report.identical(), "same frame and inputs remain identical");
            report_counts = counts_from_report(&report);
            report_identical = Some(report.identical());
            checksums = json!({
                "initial": format!("0x{:016x}", report.base_start_checksum),
                "base_final": format!("0x{:016x}", report.base_final_checksum),
                "candidate_final": format!("0x{:016x}", report.candidate_final_checksum),
            });
            core_verify_ns = Some(elapsed);
        }
        Cell::CoreReplay { parallel } => {
            let replay = fixture
                .replay
                .as_ref()
                .expect("replay cell has generated replay");
            let (reader, elapsed) = timed(cell_name, "replay_parse", || {
                ReplayReader::<PhysGame>::parse(replay)
            });
            let reader = reader.expect("valid generated replay parses");
            assert_eq!(reader.tick_count(), REPLAY_TICKS as usize);
            replay_parse_ns = Some(elapsed);
            let inputs = VerifyInputs::Recorded(reader);
            let options = VerifyOptions {
                max_ticks: Some(EXECUTION_TICKS),
                parallel,
                ..VerifyOptions::default()
            };
            let (report, elapsed) = timed(cell_name, "core_verify", || {
                orr_edit::verify_frames::<PhysGame>(
                    &base,
                    &candidate,
                    &inputs,
                    &PhysMetrics,
                    &options,
                )
            });
            let report = report.expect("recorded core verification succeeds");
            assert_eq!(report.ticks, u64::from(EXECUTION_TICKS));
            assert!(report.identical(), "same frame and replay remain identical");
            let recording = report
                .recording
                .expect("recorded verification reports its checksum match");
            assert_eq!(
                recording.checked, EXECUTION_CHECKSUM_BOUNDARIES,
                "initial boundary and all executed ticks are checked against the recording"
            );
            recording_checksums_checked = Some(u64::from(recording.checked));
            assert_eq!(
                recording.mismatches, 0,
                "base frame reproduces generated replay checksums"
            );
            report_counts = counts_from_report(&report);
            report_identical = Some(report.identical());
            recording_matches = Some(recording.mismatches == 0);
            checksums = json!({
                "initial": format!("0x{:016x}", report.base_start_checksum),
                "base_final": format!("0x{:016x}", report.base_final_checksum),
                "candidate_final": format!("0x{:016x}", report.candidate_final_checksum),
            });
            core_verify_ns = Some(elapsed);
        }
        Cell::LocalIdle { series } => {
            let limits = HostLimits {
                player_count: PLAYERS,
                tick_rate: 60,
                max_verify_ticks: 6000,
                game: GameHooks::new("PhysGame").with_metrics(PhysMetrics),
                ..HostLimits::default()
            };
            let mut play = None;
            let mut target = ErpTarget::<PhysGame> {
                doc: &mut fixture.doc,
                play: &mut play,
            };
            let params = json!({
                "inputs": {"kind": "idle", "ticks": EXECUTION_TICKS},
                "series": series,
            });
            let (report, elapsed) = timed(cell_name, "call_local", || {
                call_local(
                    &mut target,
                    &limits,
                    "verification-budget-probe",
                    Caps::ALL,
                    "verify.self",
                    &params,
                )
            });
            let report = report.expect("call_local idle verification succeeds");
            assert_eq!(report["ticks"].as_u64(), Some(u64::from(EXECUTION_TICKS)));
            assert_eq!(report["identical"].as_bool(), Some(true));
            report_identical = Some(true);
            report_counts = counts_from_json(&report);
            checksums = report
                .get("checksums")
                .cloned()
                .expect("verify.self returns checksums");
            let (encoded, encode_elapsed) = timed(cell_name, "returned_json_encode", || {
                serde_json::to_vec(&report)
            });
            let encoded = encoded.expect("returned JSON encoding succeeds");
            assert!(!encoded.is_empty());
            returned_json_bytes = Some(encoded.len());
            call_local_ns = Some(elapsed);
            returned_json_encode_ns = Some(encode_elapsed);
        }
        Cell::LocalReplay => {
            let replay = fixture
                .replay
                .as_ref()
                .expect("replay cell has generated replay");
            let (reader, elapsed) = timed(cell_name, "replay_parse", || {
                ReplayReader::<PhysGame>::parse(replay)
            });
            let reader = reader.expect("valid generated replay parses");
            assert_eq!(reader.tick_count(), REPLAY_TICKS as usize);
            replay_parse_ns = Some(elapsed);
            let limits = HostLimits {
                player_count: PLAYERS,
                tick_rate: 60,
                max_verify_ticks: 6000,
                game: GameHooks::new("PhysGame").with_metrics(PhysMetrics),
                ..HostLimits::default()
            };
            let mut play = None;
            let mut target = ErpTarget::<PhysGame> {
                doc: &mut fixture.doc,
                play: &mut play,
            };
            let params = json!({
                "inputs": {"kind": "replay", "base64": fixture.replay_base64.as_deref().expect("replay cell has base64 input")},
                "ticks": EXECUTION_TICKS,
                "series": true,
            });
            let (report, elapsed) = timed(cell_name, "call_local", || {
                call_local(
                    &mut target,
                    &limits,
                    "verification-budget-probe",
                    Caps::ALL,
                    "verify.self",
                    &params,
                )
            });
            let report = report.expect("call_local valid replay verification succeeds");
            assert_eq!(report["ticks"].as_u64(), Some(u64::from(EXECUTION_TICKS)));
            assert_eq!(report["identical"].as_bool(), Some(true));
            let mismatches = report["recording"]["mismatches"]
                .as_u64()
                .expect("recording mismatch count");
            let checked = report["recording"]["checked"]
                .as_u64()
                .expect("recording checked count");
            assert_eq!(
                checked,
                u64::from(EXECUTION_CHECKSUM_BOUNDARIES),
                "initial boundary and all executed ticks are checked against the recording"
            );
            recording_checksums_checked = Some(checked);
            assert_eq!(
                mismatches, 0,
                "base frame reproduces generated replay checksums"
            );
            report_identical = Some(true);
            recording_matches = Some(true);
            report_counts = counts_from_json(&report);
            checksums = report
                .get("checksums")
                .cloned()
                .expect("verify.self returns checksums");
            let (encoded, encode_elapsed) = timed(cell_name, "returned_json_encode", || {
                serde_json::to_vec(&report)
            });
            let encoded = encoded.expect("returned JSON encoding succeeds");
            assert!(!encoded.is_empty());
            returned_json_bytes = Some(encoded.len());
            call_local_ns = Some(elapsed);
            returned_json_encode_ns = Some(encode_elapsed);
        }
    }

    verify_unchanged(&fixture);
    if let Some(counts) = report_counts.as_object_mut() {
        counts.insert("returned_json_bytes".into(), json!(returned_json_bytes));
    }
    emit(
        cell_name,
        "result",
        json!({
            "status": "passed",
            "execution_ticks": cell.execution_ticks(),
            "requested_parallel": cell.requested_parallel(),
            "effective_parallel": effective_parallel,
            "series": cell.series(),
            "checks": {"identical": report_identical, "recording_matches": recording_matches},
            "recording_checksums_checked": recording_checksums_checked,
            "checksums": checksums,
            "timings_ns": {
                "snapshot_clone_base": base_ns,
                "snapshot_clone_candidate": candidate_ns,
                "replay_parse": replay_parse_ns,
                "core_verify": core_verify_ns,
                "call_local": call_local_ns,
                "returned_json_encode": returned_json_encode_ns,
            },
            "counts": report_counts,
            "unknown": [
                "call_local is one synchronous public aggregate; internal base64 decode, build_inputs, snapshot clones, core verify, and report assembly cannot be timed separately through public APIs.",
                "The clone phases are explicit public Frame::clone proxies, not instrumentation of ErpServer admission copies.",
                "No allocation attribution, worker scheduling profile, RSS bound, or whole-machine memory ceiling is measured.",
            ],
        }),
    );
}

#[test]
#[ignore = "explicitly selected by tools/verification_budget_probe.py; not a normal test or budget guard"]
fn bounded_normal_fixture_probe() {
    let selected = std::env::var("ORR_VERIFICATION_BUDGET_CELL")
        .expect("driver sets ORR_VERIFICATION_BUDGET_CELL");
    let cell = Cell::parse(&selected).unwrap_or_else(|| panic!("unknown probe cell {selected:?}"));
    run_cell(cell);
}

#[test]
fn approved_probe_cells_round_trip_names() {
    let names = [
        "core_idle15_serial",
        "core_idle15_parallel",
        "core_idle16_serial",
        "core_idle16_parallel",
        "core_replay16_serial",
        "core_replay16_parallel",
        "local_idle16_series_off",
        "local_idle16_series_on",
        "local_replay16_series_on",
    ];
    assert_eq!(
        names.len(),
        9,
        "the probe matrix has exactly nine approved cells"
    );
    for name in names {
        let cell = Cell::parse(name).expect("approved cell name parses");
        assert_eq!(cell.name(), name, "cell selector round-trips canonically");
    }
    assert!(
        Cell::parse("core_idle17_parallel").is_none(),
        "unapproved cells are rejected"
    );
}

#[test]
fn effective_parallel_semantics_match_public_defaults_and_tick_threshold() {
    assert!(!Cell::CoreIdle {
        ticks: 15,
        parallel: true
    }
    .effective_parallel());
    assert!(!Cell::CoreIdle {
        ticks: 15,
        parallel: false
    }
    .effective_parallel());
    assert!(Cell::CoreIdle {
        ticks: 16,
        parallel: true
    }
    .effective_parallel());
    assert!(!Cell::CoreIdle {
        ticks: 16,
        parallel: false
    }
    .effective_parallel());
    assert!(Cell::CoreReplay { parallel: true }.effective_parallel());
    assert!(!Cell::CoreReplay { parallel: false }.effective_parallel());
    assert!(Cell::LocalIdle { series: false }.effective_parallel());
    assert!(Cell::LocalIdle { series: true }.effective_parallel());
    assert!(Cell::LocalReplay.effective_parallel());
}
