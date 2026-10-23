//! Required, artifact-driven CPU editor verification of the actual native traces.
//! This does not open a GUI, inject keys, or establish native framebuffer proof.
//! Gate separately with `editor-trace-acceptance = ["editor", "dep:sha2"]`.
#[path = "support/acceptance_context.rs"]
mod acceptance_context;
use orr_bridge::FrameView;
use orr_ecs::Component;
use orr_editor::{backend::HostSpec, game::EditorGame, Editor};
use orr_fp::FrameRng;
use orr_session::ReplayReader;
use orr_sim::PlayerSlot;
use orr_static_starter::{
    delivery::{Courier, Delivery, DeliveryGame},
    game_contract::admit,
    integration::registry,
    presentation::StandaloneGame,
    switchboard::{Board, Cursor, SwitchGame},
    SEED, TICK_RATE,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component as PathComponent, Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const NATIVE_SCHEMA: &str = "orr.static-standalone-native/1";
const RECEIPT_SCHEMA: &str = "orr.static-standalone-editor-trace/1";
const MAX_JSON: usize = 1 << 20;
const MAX_SCENE: usize = 256 << 10;
const MAX_REPLAY: usize = 4 << 20;
const MAX_TICKS: u64 = 3_600;

fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

fn hash(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Sha256::digest(bytes)
        .iter()
        .flat_map(|b| {
            [
                char::from(HEX[usize::from(b >> 4)]),
                char::from(HEX[usize::from(b & 15)]),
            ]
        })
        .collect()
}

fn checksum(value: u64) -> String {
    format!("0x{value:016x}")
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn regular_directory(path: &Path) -> Result<()> {
    require(path.is_absolute(), "evidence directory must be absolute")?;
    let mut prefix = PathBuf::new();
    for part in path.components() {
        require(
            !matches!(part, PathComponent::ParentDir),
            "evidence traversal is forbidden",
        )?;
        prefix.push(part);
        let metadata = fs::symlink_metadata(&prefix)?;
        require(
            metadata.file_type().is_dir(),
            format!("not a link-free directory: {}", prefix.display()),
        )?;
    }
    Ok(())
}

/// Cap reads even if the file grows. Never follow an artifact symlink or accept
/// an empty file, FIFO, directory, truncated read, or changing metadata.
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    regular_directory(path.parent().ok_or("artifact has no parent")?)?;
    let before = fs::symlink_metadata(path)?;
    require(
        before.file_type().is_file() && before.len() > 0 && before.len() <= limit as u64,
        format!(
            "missing, linked, empty, special, or oversized artifact: {}",
            path.display()
        ),
    )?;
    let file = File::open(path)?;
    let opened = file.metadata()?;
    require(
        opened.is_file()
            && opened.len() == before.len()
            && opened.modified()? == before.modified()?,
        format!("artifact changed while opening: {}", path.display()),
    )?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(path)?;
    require(
        bytes.len() <= limit
            && bytes.len() as u64 == before.len()
            && after.file_type().is_file()
            && after.len() == before.len()
            && after.modified()? == before.modified()?,
        format!("artifact changed or truncated: {}", path.display()),
    )?;
    Ok(bytes)
}

#[derive(Default)]
struct Evidence {
    reads: Vec<(PathBuf, usize, String)>,
}

impl Evidence {
    fn read(&mut self, path: &Path, limit: usize) -> Result<Vec<u8>> {
        let bytes = read_bounded(path, limit)?;
        self.reads.push((path.to_owned(), limit, hash(&bytes)));
        Ok(bytes)
    }

    fn json(&mut self, path: &Path, limit: usize) -> Result<(Value, String)> {
        let bytes = self.read(path, limit)?;
        Ok((serde_json::from_slice(&bytes)?, hash(&bytes)))
    }

    fn unchanged(&self) -> Result<()> {
        for (path, limit, digest) in &self.reads {
            require(
                hash(&read_bounded(path, *limit)?) == *digest,
                format!(
                    "evidence changed during editor verification: {}",
                    path.display()
                ),
            )?;
        }
        Ok(())
    }
}

fn same_components<T: Component>(actual: FrameView<'_>, expected: FrameView<'_>) -> bool {
    let (actual_ids, actual_values) = actual.dense::<T>();
    let (expected_ids, expected_values) = expected.dense::<T>();
    actual_ids == expected_ids
        && bytemuck::cast_slice::<T, u8>(actual_values)
            == bytemuck::cast_slice::<T, u8>(expected_values)
}

fn same_singleton<T: Component>(actual: FrameView<'_>, expected: FrameView<'_>) -> bool {
    bytemuck::bytes_of(actual.singleton::<T>()) == bytemuck::bytes_of(expected.singleton::<T>())
}

trait TraceGame: StandaloneGame {
    fn typed_state_matches(actual: FrameView<'_>, expected: FrameView<'_>) -> bool;
}

impl TraceGame for DeliveryGame {
    fn typed_state_matches(actual: FrameView<'_>, expected: FrameView<'_>) -> bool {
        same_components::<Courier>(actual, expected) && same_singleton::<Delivery>(actual, expected)
    }
}

impl TraceGame for SwitchGame {
    fn typed_state_matches(actual: FrameView<'_>, expected: FrameView<'_>) -> bool {
        same_components::<Cursor>(actual, expected) && same_singleton::<Board>(actual, expected)
    }
}

fn same_frame<G: TraceGame>(
    actual: FrameView<'_>,
    expected: FrameView<'_>,
    origin: &str,
) -> Result<()> {
    require(
        actual.tick() == expected.tick()
            && actual.checksum() == expected.checksum()
            && actual.alive_count() == expected.alive_count()
            && same_singleton::<FrameRng>(actual, expected)
            && G::typed_state_matches(actual, expected)
            && G::state_value(actual) == G::state_value(expected),
        format!(
            "{} {origin} differs from independently simulated typed state at tick {}",
            G::ALIAS,
            expected.tick()
        ),
    )
}

fn editor_frame<G: TraceGame>(editor: &Editor, expected: FrameView<'_>) -> Result<()> {
    let snapshot = editor
        .snapshot()
        .ok_or("editor did not publish a snapshot")?;
    let timeline = snapshot
        .timeline()
        .ok_or("editor did not start a play timeline")?;
    require(
        !editor.sim().playing
            && !editor.sim().viewer
            && !timeline.playing
            && editor.is_playing_mode()
            && editor.sim().head_tick == expected.tick()
            && snapshot.tick() == expected.tick()
            && timeline.tick == expected.tick()
            && editor.sim().tick_rate == TICK_RATE
            && snapshot.tick_rate() == TICK_RATE
            && editor.sim().player_count == 1
            && editor.checksum() == expected.checksum()
            && editor.sim().checksum == expected.checksum(),
        format!(
            "{} editor is not paused at exact tick {}",
            G::ALIAS,
            expected.tick()
        ),
    )?;
    same_frame::<G>(snapshot.predicted(), expected, "editor snapshot")
}

/// Rebuild every field of admission identity from the registered host contract
/// and actual initial snapshot. Seed is checked against the complete typed RNG
/// state here, then against the actual editor recording's header after stopping.
fn host_identity<G: TraceGame>(editor: &mut Editor, expected: &Value) -> Result<Value> {
    let handle = registry().handle(G::ALIAS)?;
    require(
        editor.game() == EditorGame::Registered(handle),
        "editor used a fallback game",
    )?;
    require(
        matches!(editor.spec(), HostSpec::LocalGame { game: EditorGame::Registered(actual), listen: None, debug_hooks: false, .. } if *actual == handle),
        "editor did not use the checked registered local host route",
    )?;
    let discovery = editor.host_call("rpc.discover", Value::Null)?;
    let schema = editor.host_call("registry.schema", Value::Null)?;
    let input = editor.host_call("registry.input", Value::Null)?;
    let state = editor.host_call("sim.state", Value::Null)?;
    handle.expected_input().validate(&input)?;
    let registered = handle.expected_identity();
    require(
        discovery["erp_version"] == 1
            && discovery["engine"]["game"] == registered.game
            && discovery["engine"]["build_id"] == registered.build_id
            && schema["schema"] == registered.schema,
        "registered host discovery/schema mismatch",
    )?;
    let initial = editor
        .snapshot()
        .ok_or("missing initial editor snapshot")?
        .predicted();
    require(
        initial.tick() == 0
            && *initial.singleton::<FrameRng>() == FrameRng::new(SEED)
            && state["head_tick"] == 0
            && state["playing"] == false
            && state["checksum"] == checksum(initial.checksum()),
        "initial editor scene/seed/checksum mismatch",
    )?;
    let actual = json!({
        "game": discovery["engine"]["game"], "alias": handle.local_alias(),
        "build_id": discovery["engine"]["build_id"], "schema": schema["schema"],
        "input_contract": {"schema": input["schema"], "value_format": input["value_format"], "max_players": input["max_players"]},
        "seed": SEED, "tick_rate": state["tick_rate"], "player_count": state["player_count"],
        "initial_checksum": state["checksum"],
    });
    require(
        actual == *expected,
        "full admitted identity differs from actual registered editor host",
    )?;
    Ok(actual)
}

fn verify_game<G: TraceGame>(
    root: &Path,
    source_sha: &str,
    result_entry: &Value,
    evidence: &mut Evidence,
) -> Result<Value> {
    let directory = root.join(G::ALIAS);
    let scene_path = directory.join("authored.scene.yaml");
    let scene_bytes = evidence.read(&scene_path, MAX_SCENE)?;
    let admitted = admit::<G>(std::str::from_utf8(&scene_bytes)?)?;
    let replay_bytes = evidence.read(&directory.join("execution/record.orrp"), MAX_REPLAY)?;
    // Admission's bounded envelope preflight MUST precede ReplayReader parsing.
    // It validates full scene/input contracts, initial frame, every supplied
    // keyframe/checksum, no debug edits, and canonical current-engine ORRP.
    let mut replay = admitted.open_replay(&replay_bytes)?;
    let reader = ReplayReader::<G>::parse(&replay_bytes)?;
    let last_tick = reader.last_tick();
    require(
        reader.first_tick() == 1
            && (1..=MAX_TICKS).contains(&last_tick)
            && reader.tick_count() as u64 == last_tick
            && reader.checksums.len() as u64 == last_tick + 1,
        "native replay is empty, incomplete, or lacks an every-tick checksum",
    )?;
    for (index, (tick, _)) in reader.checksums.iter().enumerate() {
        require(
            *tick == index as u64,
            "native checksums must cover tick zero and every input tick",
        )?;
    }

    let (receipt, receipt_hash) = evidence.json(&directory.join("receipt.json"), MAX_JSON)?;
    let bundle_root = directory.join("relocated bundle with spaces");
    let (bundle, bundle_hash) = evidence.json(&bundle_root.join("bundle.json"), MAX_JSON)?;
    let (source_trace, source_trace_hash) =
        evidence.json(&directory.join("execution/source-replay.json"), 16 << 20)?;
    let observations = evidence.read(&directory.join("execution/observations.ndjson"), 32 << 20)?;
    let bundled_scene = evidence.read(&bundle_root.join("scene/game.scene.yaml"), MAX_SCENE)?;
    let scene_hash = hash(&scene_bytes);
    let replay_hash = hash(&replay_bytes);
    require(
        result_entry["alias"] == G::ALIAS
            && result_entry["receipt_sha256"] == receipt_hash
            && receipt["schema"] == NATIVE_SCHEMA
            && receipt["status"] == "passed"
            && receipt["alias"] == G::ALIAS
            && receipt["source_sha"] == source_sha
            && receipt["acceptance_context_sha256"] == json!(acceptance_context::compiled_digest())
            && receipt["source_hidden"] == true
            && receipt["input_source"] == "XTEST"
            && receipt["scene_sha256"] == scene_hash
            && receipt["bundle_sha256"] == bundle_hash
            && receipt["observations_sha256"] == hash(&observations)
            && receipt["replay"]["path"] == "record.orrp"
            && receipt["replay"]["sha256"] == replay_hash
            && receipt["replay"]["source_replay_sha256"] == source_trace_hash
            && receipt["replay"]["ticks"] == last_tick,
        format!(
            "{} native receipt/result does not bind the supplied artifacts",
            G::ALIAS
        ),
    )?;
    require(
        bundle["schema"] == 1
            && bundle["source_sha"] == source_sha
            && bundle["runtime_provenance"]["source_sha"] == source_sha
            && &bundle["identity"] == admitted.identity()
            && bundled_scene == scene_bytes
            && bundle["runtime_sha256"] == receipt["runtime_sha256"]
            && bundle["runtime_sha256"]
                .as_str()
                .is_some_and(|s| hex(s, 64))
            && bundle["scene_descriptor_sha256"]
                == hash(&serde_json::to_vec(&admitted.identity()["schema"])?)
            && bundle["input_contract_sha256"]
                == hash(&serde_json::to_vec(&admitted.identity()["input_contract"])?),
        "bundle full identity/source/scene contract differs from admitted native recording",
    )?;
    let rows = source_trace["ticks"]
        .as_array()
        .ok_or("missing independent native source trace")?;
    require(
        rows.len() as u64 == last_tick + 1,
        "independent native source trace has a tick gap",
    )?;

    let mut source = admitted.session()?;
    let initial_checksum = checksum(source.frame().checksum());
    require(
        receipt["replay"]["initial_checksum"] == initial_checksum
            && admitted.identity()["initial_checksum"] == initial_checksum,
        "native receipt initial checksum differs from admitted scene",
    )?;
    let handle = registry().handle(G::ALIAS)?;
    let mut editor = Editor::open_registered(&scene_path, handle)?;
    editor.sync();
    let identity = host_identity::<G>(&mut editor, admitted.identity())?;
    require(
        editor.start_play(),
        "registered editor could not start paused play",
    )?;
    editor.sync();
    for tick in 0..=last_tick {
        let input = if tick == 0 {
            None
        } else {
            let (inputs, commands) = reader.tick(tick).ok_or("raw native replay input gap")?;
            require(
                inputs.len() == 1
                    && commands.is_empty()
                    && reader.debug_commands(tick).is_empty()
                    && G::valid_input(&inputs[0]),
                "invalid native typed input/command",
            )?;
            let input = inputs[0];
            source.set_input(PlayerSlot(0), input);
            source
                .step_now()
                .ok_or("independent admitted source could not step")?;
            replay
                .step_now()
                .ok_or("admitted native replay could not step")?;
            let value = G::input_value(&input);
            let answer =
                editor.host_call("sim.input_value", json!({"player": 0, "value": value}))?;
            require(
                answer["ok"] == true,
                "editor ERP did not admit the actual raw typed input",
            )?;
            editor.step(1);
            editor.sync();
            Some(value)
        };
        let expected = FrameView::of(source.frame());
        same_frame::<G>(
            FrameView::of(replay.frame()),
            expected,
            "admitted native replay",
        )?;
        editor_frame::<G>(&editor, expected)?;
        let index = usize::try_from(tick)?;
        require(
            reader.checksums[index] == (tick, expected.checksum()),
            format!("{} raw ORRP checksum mismatch at tick {tick}", G::ALIAS),
        )?;
        let mut row = json!({"tick": tick, "checksum": checksum(expected.checksum()), "state": G::state_value(expected)});
        if let Some(value) = input {
            row["input"] = value;
        }
        require(
            rows[index] == row,
            format!(
                "{} independent native source trace mismatch at tick {tick}",
                G::ALIAS
            ),
        )?;
    }
    let final_checksum = checksum(source.frame().checksum());
    require(
        receipt["replay"]["final_checksum"] == final_checksum,
        "native final receipt checksum mismatch",
    )?;

    // Inspect the real editor-produced recording in memory, never replace the
    // native artifact or use this output as an alternative input trace.
    let stopped = editor.host_call("sim.stop", json!({"include_replay": true}))?;
    require(
        stopped["tick"] == last_tick && stopped["checksum"] == final_checksum,
        "editor stop boundary changed the verified final state",
    )?;
    let encoded = stopped["replay"]
        .as_str()
        .ok_or("editor did not return its actual recording")?;
    require(
        encoded.len() <= MAX_REPLAY.div_ceil(3) * 4,
        "editor recording exceeds bounded decode size",
    )?;
    let editor_bytes =
        orr_remote::codec::b64_decode(encoded).ok_or("invalid actual editor replay base64")?;
    require(
        stopped["replay_bytes"] == editor_bytes.len(),
        "editor replay byte count mismatch",
    )?;
    admitted.open_replay(&editor_bytes)?;
    let editor_reader = ReplayReader::<G>::parse(&editor_bytes)?;
    require(
        editor_reader.header.seed == SEED
            && editor_reader.header.seed == reader.header.seed
            && editor_reader.header.game_id == reader.header.game_id
            && editor_reader.header.build_hash == reader.header.build_hash
            && editor_reader.header.tick_rate == reader.header.tick_rate
            && editor_reader.header.player_count == reader.header.player_count
            && editor_reader.header.input_size == reader.header.input_size
            && editor_reader.first_tick() == 1
            && editor_reader.last_tick() == last_tick
            && editor_reader.tick_count() == reader.tick_count()
            && editor_reader.checksums == reader.checksums,
        "actual editor recording header or every-tick checksums differ from native ORRP",
    )?;
    for tick in 1..=last_tick {
        let (actual, commands) = editor_reader
            .tick(tick)
            .ok_or("editor recording input gap")?;
        let (native, _) = reader.tick(tick).ok_or("native recording input gap")?;
        require(
            commands.is_empty()
                && actual.len() == 1
                && bytemuck::bytes_of(&actual[0]) == bytemuck::bytes_of(&native[0]),
            "actual editor recording did not retain exact typed native input",
        )?;
    }

    Ok(json!({
        "alias": G::ALIAS, "identity": identity, "identity_sha256": hash(&serde_json::to_vec(admitted.identity())?),
        "source_sha": source_sha, "receipt_sha256": receipt_hash, "bundle_sha256": bundle_hash,
        "scene_sha256": scene_hash, "replay_sha256": replay_hash, "source_replay_sha256": source_trace_hash,
        "editor_recording_sha256": hash(&editor_bytes), "ticks": last_tick,
        "compared_frames": last_tick + 1, "initial_checksum": initial_checksum, "final_checksum": final_checksum,
        "all_ticks_match": true, "typed_state_match": true, "checked_registered_host": true,
    }))
}

#[test]
fn native_recordings_match_editor_route() -> Result<()> {
    require(
        std::env::var("ORR_REQUIRE_STANDALONE_EDITOR_TRACE").as_deref() == Ok("1"),
        "selected editor verifier requires ORR_REQUIRE_STANDALONE_EDITOR_TRACE=1; it never skips",
    )?;
    let source_sha =
        option_env!("ORR_STATIC_STANDALONE_BUILD_SHA").ok_or("missing compile-bound source SHA")?;
    let context = acceptance_context::Context::load(env!("CARGO_MANIFEST_DIR"))?;
    require(
        hex(source_sha, 40) && context.source_sha()? == source_sha,
        "editor compile/run source SHA mismatch",
    )?;
    let run_id = std::env::var("GITHUB_RUN_ID")?;
    let run_attempt = std::env::var("GITHUB_RUN_ATTEMPT")?;
    require(
        [&run_id, &run_attempt]
            .iter()
            .all(|s| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_digit())),
        "missing or invalid hosted run identity",
    )?;
    let root = PathBuf::from(
        std::env::var_os("ORR_STANDALONE_ARTIFACT_DIR")
            .ok_or("missing required native artifact directory")?,
    );
    regular_directory(&root)?;
    let output = root.join("editor-trace.json");
    match fs::symlink_metadata(&output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => {
            return Err(
                "editor verification receipt already exists; never overwrite prior evidence".into(),
            );
        }
    }
    let mut evidence = Evidence::default();
    let (gate, _) = evidence.json(&root.join("gate.json"), MAX_JSON)?;
    require(
        gate["acceptance_context_sha256"] == json!(context.digest())
            && gate["schema"] == NATIVE_SCHEMA
            && gate["provenance"]["source_sha"] == source_sha
            && gate["provenance"]["run_id"] == run_id
            && gate["provenance"]["run_attempt"] == run_attempt,
        "native evidence belongs to a different source or hosted run",
    )?;
    let (native, native_hash) = evidence.json(&root.join("result.json"), MAX_JSON)?;
    require(
        native["acceptance_context_sha256"] == json!(context.digest())
            && native["schema"] == NATIVE_SCHEMA
            && native["status"] == "passed"
            && native["source_sha"] == source_sha,
        "missing successful native result for this compile-bound source",
    )?;
    let games = native["games"]
        .as_array()
        .ok_or("native result has no games")?;
    require(
        games.len() == 2 && games[0]["alias"] == "delivery" && games[1]["alias"] == "switchboard",
        "native result must contain exactly delivery and switchboard, once each",
    )?;
    let delivery = verify_game::<DeliveryGame>(&root, source_sha, &games[0], &mut evidence)?;
    let switchboard = verify_game::<SwitchGame>(&root, source_sha, &games[1], &mut evidence)?;
    evidence.unchanged()?;
    let receipt = json!({
        "schema": RECEIPT_SCHEMA, "status": "passed", "source_sha": source_sha,
        "acceptance_context_sha256": context.digest(),
        "run_id": run_id, "run_attempt": run_attempt, "native_result_sha256": native_hash,
        "native_gui_proof": false, "physical_keyboard_proof": false,
        "scope": "CPU registered editor ERP integration against unchanged admitted native ORRP inputs, initial frame and every tick",
        "games": [delivery, switchboard],
    });
    let bytes = serde_json::to_vec_pretty(&receipt)?;
    require(
        bytes.len() <= MAX_JSON,
        "editor verification receipt exceeds 1 MiB",
    )?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
