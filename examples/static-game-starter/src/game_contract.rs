//! Application-owned typed admission shared by editor, headless and standalone players.
//!
//! Semantic identity belongs to the compiled game and this admitted scene. A raw
//! ORRP recording is not an authenticated identity envelope and does not embed
//! the full scene or input descriptors. This starter accepts canonical current
//! engine-writer v3 ORRP only; other encodings may remain valid to the engine.
use crate::{
    delivery::{Courier, Delivery, DeliveryGame, DeliveryInput},
    switchboard::{Board, Cursor, SwitchGame, SwitchInput},
};
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_reflect::{Reflect, Scene, SceneIndex, TypeRegistry};
use orr_session::{PlaySession, ReplayReader, ReplayWriter};
use orr_sim::{Game, PlayerSlot, Simulation};
use serde_json::{json, Value};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::Read,
    marker::PhantomData,
    path::Path,
    sync::Arc,
};

const MAX_SCENE_BYTES: usize = 256 << 10;
const MAX_REPLAY_BYTES: usize = 4 << 20;
const MAX_REPLAY_BODY_BYTES: usize = 16 << 20;
const MAX_REPLAY_TICKS: u64 = 36_000;

/// Finite, presentation-independent held keys. Public fields preserve the small
/// adapter API; use `new` when constructing values from untrusted input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldKeys {
    pub x: i8,
    pub y: i8,
    pub action: bool,
}
impl HeldKeys {
    pub fn new(x: i8, y: i8, action: bool) -> Result<Self, String> {
        if !(-1..=1).contains(&x) || !(-1..=1).contains(&y) {
            return Err("held-key axes must be in -1..=1".into());
        }
        Ok(Self { x, y, action })
    }
}

pub trait Starter:
    Game<Config = (), Event = u32, Command = crate::NoCommand, Input: Reflect>
{
    const ID: &'static str;
    const ALIAS: &'static str;
    const INPUT: &'static str;
    const BUILD: u64;
    const SCENE: &'static str;
    fn types() -> TypeRegistry;
    fn validate(frame: &orr_ecs::Frame) -> bool;
    fn valid_input(input: &Self::Input) -> bool;
    fn won(frame: &orr_ecs::Frame) -> bool;
}
impl Starter for DeliveryGame {
    const ID: &'static str = "StarterDelivery";
    const ALIAS: &'static str = "delivery";
    const INPUT: &'static str = "DeliveryInput";
    const BUILD: u64 = crate::delivery::BUILD;
    const SCENE: &'static str = crate::delivery::SCENE;
    fn types() -> TypeRegistry {
        crate::delivery::types()
    }
    fn valid_input(i: &DeliveryInput) -> bool {
        (-1..=1).contains(&i.dx) && (-1..=1).contains(&i.dy) && i.deliver <= 1
    }
    fn won(f: &orr_ecs::Frame) -> bool {
        f.singleton::<Delivery>().won != 0
    }
    fn validate(f: &orr_ecs::Frame) -> bool {
        let d = f.singleton::<Delivery>();
        f.dense::<Courier>().0.len() == 1 && d.won == 0 && d.action_held == 0
    }
}
impl Starter for SwitchGame {
    const ID: &'static str = "StarterSwitchboard";
    const ALIAS: &'static str = "switchboard";
    const INPUT: &'static str = "SwitchInput";
    const BUILD: u64 = crate::switchboard::BUILD;
    const SCENE: &'static str = crate::switchboard::SCENE;
    fn types() -> TypeRegistry {
        crate::switchboard::types()
    }
    fn valid_input(i: &SwitchInput) -> bool {
        [orr_fp::FP::ZERO, orr_fp::FP::ONE, -orr_fp::FP::ONE].contains(&i.selection)
            && i.toggle <= 1
            && i.pad == 0
    }
    fn won(f: &orr_ecs::Frame) -> bool {
        f.singleton::<Board>().solved != 0
    }
    fn validate(f: &orr_ecs::Frame) -> bool {
        let b = f.singleton::<Board>();
        let cursors = f.dense::<Cursor>().1;
        cursors.len() == 1
            && cursors[0].selected < 4
            && cursors[0].held == 0
            && b.bits < 16
            && (1..16).contains(&b.target)
            && b.solved == 0
            && b.moves == 0
    }
}
struct Admission<G>(PhantomData<fn() -> G>);
impl<G: Starter> BakeAdmission for Admission<G> {
    fn admit(
        &self,
        _: &Scene,
        frame: &mut orr_ecs::Frame,
        _: &SceneIndex,
    ) -> Result<(), EditError> {
        if G::validate(frame) {
            Ok(())
        } else {
            Err(EditError::Invalid(
                "starter requires one valid actor and initial-only state".into(),
            ))
        }
    }
    fn allow_play_edits(&self) -> bool {
        false
    }
}
/// Both editor host and source player call this exact loader/admission.
pub fn document<G: Starter>(text: &str) -> Result<EditorDoc, String> {
    if text.len() > MAX_SCENE_BYTES {
        return Err("scene exceeds 256 KiB".into());
    }
    EditorDoc::from_yaml_with_admission(
        text,
        G::types(),
        Simulation::<G>::build_registry(),
        crate::SEED,
        Some(Arc::new(Admission::<G>(PhantomData))),
    )
    .map_err(|e| e.to_string())
}
pub fn player<G: Starter>(text: &str) -> Result<PlaySession<G>, String> {
    session_from_document::<G>(&document::<G>(text)?)
}
pub fn inputs<G: Starter>() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    t.register_component::<G::Input>(G::INPUT);
    t
}
fn session_from_document<G: Starter>(doc: &EditorDoc) -> Result<PlaySession<G>, String> {
    let mut config = doc.play_config(1, crate::TICK_RATE);
    config.game_id = G::ID.into();
    config.build_id = G::BUILD;
    config.seed = crate::SEED;
    config.tick_rate = crate::TICK_RATE;
    config.player_count = 1;
    config.start_paused = true;
    PlaySession::from_frame(config, doc.frame()).map_err(|e| e.to_string())
}

/// Reads a bounded regular scene file and rejects invalid UTF-8.
pub fn read_scene(path: &Path) -> Result<String, String> {
    String::from_utf8(read_regular(path, MAX_SCENE_BYTES)?).map_err(|e| e.to_string())
}

/// Reads at most `limit` bytes from a regular file, checking its metadata before
/// and after the read and confirming the path still names the opened file.
/// On Unix, a raced leaf symlink is not followed and a raced FIFO cannot block
/// the open. Parent directories must remain trusted: this is not a defense
/// against hostile parent-directory races or an atomic snapshot of file content.
/// On non-Unix targets, only static trusted paths are supported: the portable
/// open cannot enforce no-follow/nonblocking behavior or compare file identity.
pub fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let limit_u64 = u64::try_from(limit).map_err(|_| "input byte limit is too large")?;
    let read_limit = limit_u64
        .checked_add(1)
        .ok_or("input byte limit is too large")?;
    let before = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("input must be a regular file, not a symlink or special file".into());
    }
    if before.len() > limit_u64 {
        return Err(format!("input exceeds {limit} byte limit"));
    }
    let (mut file, opened) = open_regular(path)?;
    if opened.len() > limit_u64 || !same_file_snapshot(&before, &opened) {
        return Err("input changed while opening".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("input exceeds {limit} byte limit"));
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    let path_after = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !after.is_file()
        || !path_after.is_file()
        || bytes.len() as u64 != opened.len()
        || !same_file_snapshot(&opened, &after)
        || !same_file_snapshot(&opened, &path_after)
    {
        return Err("input changed while reading".into());
    }
    Ok(bytes)
}

fn open_regular(path: &Path) -> Result<(File, Metadata), String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
        );
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("opened input must be a regular file".into());
    }
    Ok((file, metadata))
}

#[cfg(unix)]
fn same_file_snapshot(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(not(unix))]
fn same_file_snapshot(left: &Metadata, right: &Metadata) -> bool {
    // Portable metadata cannot establish identity; callers must use static paths.
    left.len() == right.len()
        && matches!((left.modified(), right.modified()), (Ok(a), Ok(b)) if a == b)
}

/// An immutable admitted scene and its complete local semantic identity.
/// Neither source checkout metadata nor ORRP bytes authenticate this identity.
pub struct Admitted<G: Starter> {
    document: EditorDoc,
    identity: Value,
    game: PhantomData<fn() -> G>,
}

pub fn admit<G: Starter>(text: &str) -> Result<Admitted<G>, String> {
    let document = document::<G>(text)?;
    let schema: Value =
        serde_json::from_str(&document.types().legacy_json_schema()).map_err(|e| e.to_string())?;
    let input_schema: Value = serde_json::from_str(
        &inputs::<G>()
            .type_json_schema(G::INPUT)
            .ok_or("missing typed input schema")?,
    )
    .map_err(|e| e.to_string())?;
    let identity = json!({
        "game": G::ID,
        "alias": G::ALIAS,
        "build_id": format!("0x{:016x}", G::BUILD),
        "schema": schema,
        "input_contract": {
            "schema": input_schema,
            "value_format": orr_remote::methods::VALUE_FORMAT,
            "max_players": 1,
        },
        "seed": crate::SEED,
        "tick_rate": crate::TICK_RATE,
        "player_count": 1,
        "initial_checksum": format!("0x{:016x}", document.frame().checksum()),
    });
    Ok(Admitted {
        document,
        identity,
        game: PhantomData,
    })
}

impl<G: Starter> Admitted<G> {
    pub fn identity(&self) -> &Value {
        &self.identity
    }

    pub fn session(&self) -> Result<PlaySession<G>, String> {
        session_from_document::<G>(&self.document)
    }

    /// Opens only bounded, canonical engine-writer v3 ORRP recordings of this
    /// exact admitted scene. Full schema descriptors are local to this compiled
    /// admission, not embedded in or authenticated by the replay header.
    ///
    /// Every supplied checksum and keyframe is checked against independent
    /// simulation from the admitted frame before a viewer can use it. Rebuilding
    /// with the public writer additionally rejects trailing bytes, duplicate
    /// records, ignored input-mask bits and debug commands at any tick. This
    /// narrow canonical-encoding policy does not change the engine's ORRP format.
    pub fn open_replay(&self, bytes: &[u8]) -> Result<PlaySession<G>, String> {
        preflight_replay::<G>(bytes)?;
        let reader = ReplayReader::<G>::parse(bytes).map_err(|e| e.to_string())?;
        let last = reader.last_tick();
        if last > MAX_REPLAY_TICKS || reader.first_tick() != 1 || reader.tick_count() as u64 != last
        {
            return Err("replay inputs must be contiguous from tick 1, at most 36000 ticks".into());
        }
        if reader.nearest_keyframe(0) != Some(0) {
            return Err(
                "replay requires the admitted tick-0 keyframe; setup fallback is forbidden".into(),
            );
        }
        // Validate the complete finite input trace before running any simulation.
        for tick in 1..=last {
            let (inputs, commands) = reader.tick(tick).ok_or("replay has an input gap")?;
            if inputs.len() != 1 || !G::valid_input(&inputs[0]) {
                return Err(format!(
                    "replay input at tick {tick} is outside the finite input contract"
                ));
            }
            if !commands.is_empty() {
                return Err("starter replays cannot contain gameplay commands".into());
            }
        }
        let mut previous_checksum_tick = None;
        for &(tick, _) in &reader.checksums {
            if tick > last || previous_checksum_tick.is_some_and(|previous| tick <= previous) {
                return Err(
                    "replay checksums must be ordered, unique, and within the input timeline"
                        .into(),
                );
            }
            previous_checksum_tick = Some(tick);
        }
        let mut expected = self.session()?;
        let mut canonical = ReplayWriter::<G>::new(reader.header.clone());
        let mut checksum_index = 0;
        let mut checked_keyframes = 0;
        for tick in 0..=last {
            if !reader.debug_commands(tick).is_empty() {
                return Err("starter replays cannot contain debug edits".into());
            }
            if tick > 0 {
                let (inputs, _) = reader.tick(tick).ok_or("replay has an input gap")?;
                expected.set_input(PlayerSlot(0), inputs[0]);
                expected
                    .step_now()
                    .ok_or("independent replay simulation could not advance")?;
                canonical.record_tick(tick, inputs, &[]);
            }
            if reader.nearest_keyframe(tick) == Some(tick) {
                // Seeking exactly to a supplied keyframe decodes it against the
                // compiled schema, without trusting it as the resimulation base.
                let supplied = reader.seek((), tick).map_err(|e| e.to_string())?;
                if supplied.frame().tick() != tick
                    || supplied.checksum() != expected.frame().checksum()
                    || supplied.frame().to_bytes() != expected.frame().to_bytes()
                {
                    return Err(format!(
                        "replay keyframe at tick {tick} differs from the admitted trajectory"
                    ));
                }
                canonical.record_keyframe(expected.frame());
                checked_keyframes += 1;
            }
            if let Some(&(at, checksum)) = reader.checksums.get(checksum_index) {
                if at == tick {
                    if checksum != expected.frame().checksum() {
                        return Err(format!(
                            "replay checksum at tick {tick} differs from the admitted trajectory"
                        ));
                    }
                    canonical.record_checksum(tick, checksum);
                    checksum_index += 1;
                }
            }
        }
        if checked_keyframes != reader.keyframe_count() {
            return Err("replay has keyframes outside its input timeline".into());
        }
        if canonical.finish() != bytes {
            return Err("starter admission accepts only canonical engine-writer v3 ORRP without commands, debug edits, duplicate records, or trailing bytes; legacy and alternative encodings are not supported here".into());
        }
        PlaySession::open_replay(bytes, (), G::BUILD).map_err(|e| e.to_string())
    }
}

/// Inspect all outer framing and the LZ4 size prefix before the existing engine
/// parser allocates its decompression buffer. Its ratio bound alone can permit
/// allocations much larger than this starter's absolute decompressed-byte cap.
fn preflight_replay<G: Starter>(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_REPLAY_BYTES {
        return Err("replay exceeds 4 MiB".into());
    }
    let mut r = ReplayEnvelope { remaining: bytes };
    if r.take(4)? != b"ORRP" {
        return Err("not an ORRP replay".into());
    }
    if r.u32()? != 3 {
        return Err("starter admission requires canonical engine-writer v3 ORRP; legacy encodings are not supported here".into());
    }
    let game_len = r.u32()? as usize;
    if r.take(game_len)? != G::ID.as_bytes() {
        return Err("replay game identity does not match the admitted game".into());
    }
    let build_hash = r.u64()?;
    if build_hash == 0 || build_hash != orr_sim::build_hash_of(G::BUILD, 0) {
        return Err("replay build hash does not match the admitted build".into());
    }
    if r.u64()? != crate::SEED {
        return Err("replay seed does not match the admitted scene".into());
    }
    if r.take(1)? != [1] {
        return Err("starter replay requires exactly one player".into());
    }
    if r.u32()? != crate::TICK_RATE {
        return Err("replay tick rate does not match the admitted game".into());
    }
    if r.u32()? as usize != core::mem::size_of::<G::Input>() {
        return Err("replay input size does not match the compiled typed input".into());
    }
    let compressed_len = r.u32()? as usize;
    let compressed = r.take(compressed_len)?;
    if !r.remaining.is_empty() {
        return Err("replay has trailing bytes after its compressed body".into());
    }
    let prefix: [u8; 4] = compressed
        .get(..4)
        .ok_or("replay is missing its decompressed-size prefix")?
        .try_into()
        .map_err(|_| "replay has an invalid decompressed-size prefix")?;
    if u32::from_le_bytes(prefix) as usize > MAX_REPLAY_BODY_BYTES {
        return Err("replay decompressed body exceeds 16 MiB".into());
    }
    Ok(())
}

/// Small checked reader for the uncompressed outer envelope only. Frame, input,
/// command and LZ4 decoding remain with the existing engine's safe decoders.
struct ReplayEnvelope<'a> {
    remaining: &'a [u8],
}
impl<'a> ReplayEnvelope<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if count > self.remaining.len() {
            return Err("replay outer envelope is truncated".into());
        }
        let (value, remaining) = self.remaining.split_at(count);
        self.remaining = remaining;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| "invalid replay u32")?;
        Ok(u32::from_le_bytes(bytes))
    }
    fn u64(&mut self) -> Result<u64, String> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| "invalid replay u64")?;
        Ok(u64::from_le_bytes(bytes))
    }
}

/// Finite data, not a script. Validate the entire trace before any tick runs.
/// Each input is decoded using the exact reflected type used by ERP admission.
pub fn trace<G: Starter>(bytes: &[u8]) -> Result<Vec<(u32, G::Input)>, String> {
    if bytes.len() > 65_536 {
        return Err("trace exceeds 64 KiB".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let steps = value.as_array().ok_or("trace must be an array")?;
    if steps.is_empty() || steps.len() > 64 {
        return Err("trace requires 1..=64 steps".into());
    }
    let input_registry = inputs::<G>();
    let info = input_registry.get(G::INPUT).ok_or("missing typed input")?;
    let mut total = 0;
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        let object = step.as_object().ok_or("trace step must be an object")?;
        if object.len() != 2 || !object.contains_key("ticks") || !object.contains_key("input") {
            return Err("trace step requires exactly ticks and input".into());
        }
        let ticks = object["ticks"]
            .as_u64()
            .filter(|n| (1..=600).contains(n))
            .ok_or("ticks must be an integer in 1..=600")? as u32;
        total += ticks;
        if total > 600 {
            return Err("trace exceeds 600 total ticks".into());
        }
        let value = orr_remote::json::json_to_value(info.desc(), &object["input"], false)?;
        let mut bytes = info.default_bytes();
        info.set(&mut bytes, "", value).map_err(|e| e.to_string())?;
        let input =
            bytemuck::try_pod_read_unaligned::<G::Input>(&bytes).map_err(|e| e.to_string())?;
        if !G::valid_input(&input) {
            return Err("trace input is outside the finite held-key domain".into());
        }
        out.push((ticks, input));
    }
    Ok(out)
}

#[cfg(test)]
mod file_read_tests {
    #[cfg(unix)]
    use super::{open_regular, same_file_snapshot};
    use super::{read_regular, read_scene, MAX_SCENE_BYTES};
    use std::fs;

    #[test]
    fn bounded_reader_accepts_regular_content_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"regular content").unwrap();
        assert_eq!(read_regular(&path, 32).unwrap(), b"regular content");
        assert!(read_regular(&path, 0).is_err());
        fs::write(&path, b"").unwrap();
        assert!(read_regular(&path, 0).unwrap().is_empty());
        assert!(read_regular(dir.path(), 32).is_err());
    }

    #[test]
    fn bounded_reader_accepts_exact_cap_and_rejects_over_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        fs::write(&path, b"1234").unwrap();
        assert_eq!(read_regular(&path, 4).unwrap(), b"1234");
        fs::write(&path, b"12345").unwrap();
        assert!(read_regular(&path, 4).is_err());
    }

    #[test]
    fn scene_reader_enforces_utf8_and_256_kib_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scene.yaml");
        fs::write(&path, "scene: café\n").unwrap();
        assert_eq!(read_scene(&path).unwrap(), "scene: café\n");
        fs::write(&path, [0xff]).unwrap();
        assert_eq!(read_regular(&path, 1).unwrap(), [0xff]);
        assert!(read_scene(&path).is_err());
        let exact = "a".repeat(MAX_SCENE_BYTES);
        fs::write(&path, exact.as_bytes()).unwrap();
        assert_eq!(read_scene(&path).unwrap(), exact);
        fs::write(&path, vec![b'a'; MAX_SCENE_BYTES + 1]).unwrap();
        assert!(read_scene(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_reader_rejects_leaf_symlinks_even_at_guarded_open() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, b"data").unwrap();
        symlink(&target, &link).unwrap();
        assert!(read_regular(&link, 4).is_err());
        // Exercise the guarded open without the preliminary path check.
        assert!(open_regular(&link).is_err());
        fs::remove_file(&target).unwrap();
        assert!(read_regular(&link, 4).is_err());
        assert!(open_regular(&link).is_err());
    }

    // rustix does not expose mkfifoat on every Unix target (notably Apple).
    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_reader_rejects_fifo_without_waiting_for_a_writer() {
        use rustix::fs::{mkfifoat, Mode, CWD};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fifo");
        mkfifoat(CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
        assert!(read_regular(&path, 4).is_err());
        // Direct open covers the race-safe path without threads or timing.
        assert!(open_regular(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unix_snapshot_rejects_same_length_file_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        let replacement = dir.path().join("replacement");
        fs::write(&path, b"old").unwrap();
        let (file, before) = open_regular(&path).unwrap();
        fs::write(&replacement, b"new").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(!same_file_snapshot(
            &before,
            &fs::symlink_metadata(&path).unwrap()
        ));
        assert_eq!(file.metadata().unwrap().len(), 3);
    }
}
