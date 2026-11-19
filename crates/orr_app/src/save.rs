//! Save slots: the verified state of an offline run in a file, and back.
//!
//! A save is the deterministic frame snapshot of the simulation (`Frame::to_bytes`: the tick,
//! every component, singleton and list, the random generator state) plus a small header: the game
//! id and version, the player count and tick rate, the checksum of the frame, a label and the time.
//! Loading starts a new offline session from that frame. The checksum of the new session is the
//! checksum that was saved. Nothing of the file touches the running session until it was read,
//! checked and decoded completely.
//!
//! The files live in one folder per game (see [`SaveStore::default_dir`]), one file per slot:
//! `slot-<name>.orrsave`. A write goes to a temporary file in the same folder, is synced and then
//! renamed over the slot, so a slot holds either the old save or the new one.
//!
//! File format, version 1 (little endian, lengths are `u32` unless noted):
//!
//! ```text
//! "ORSV"  version u32 (= 1)
//! game id (len, utf-8)   game version u32   saved_at u64 (unix seconds)   label (len, utf-8)
//! players u8   tick_rate u32   tick u64   frame checksum u64   frame bytes (len u64, bytes)
//! xxh3 u64 of every byte before it
//! ```
//!
//! There is one current version. A file of another version is refused with a message that names
//! both versions; the engine is in development and does not migrate saves.

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use orr_ecs::Frame;
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Game, Simulation};
use xxhash_rust::xxh3::xxh3_64;

/// The save format this build writes and reads.
pub const SAVE_VERSION: u32 = 1;
const MAGIC: [u8; 4] = *b"ORSV";
const EXTENSION: &str = "orrsave";
const PREFIX: &str = "slot-";
/// The most bytes of one save file that is read.
pub const MAX_SAVE_BYTES: u64 = 64 << 20;
const MAX_NAME: usize = 64;
const MAX_LABEL_BYTES: usize = 120;
const MAX_SLOT: usize = 32;

/// Why a save could not be written, read or used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// This run has no save slots: the game did not call `App::saves`, or the run is networked.
    Unavailable(String),
    /// A slot name must be 1 to 32 letters, digits, `-` or `_`.
    BadSlot(String),
    /// There is no save in the slot.
    NotFound(String),
    /// The file is damaged: wrong magic, cut off, a bad length or checksum, or a frame that does not decode.
    Corrupt(String),
    /// The file was written by another save format version.
    UnsupportedVersion { found: u32, supported: u32 },
    /// The save belongs to another game.
    ForeignGame { found: String, expected: String },
    /// The save was written by another version of this game.
    GameVersion { found: u32, expected: u32 },
    /// The save does not fit this run (player count, tick rate).
    Mismatch(String),
    /// A file system error.
    Io(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Unavailable(why) => write!(f, "saving is not available: {why}"),
            SaveError::BadSlot(name) => write!(f, "bad slot name {name:?}: use 1 to {MAX_SLOT} letters, digits, '-' or '_'"),
            SaveError::NotFound(slot) => write!(f, "no save in slot {slot:?}"),
            SaveError::Corrupt(why) => write!(f, "the save is damaged: {why}"),
            SaveError::UnsupportedVersion { found, supported } => {
                write!(f, "the save has format version {found}, this build reads only version {supported}; saves are not migrated")
            }
            SaveError::ForeignGame { found, expected } => write!(f, "the save belongs to the game {found:?}, not to {expected:?}"),
            SaveError::GameVersion { found, expected } => {
                write!(f, "the save was made by game version {found}, this is version {expected}; saves are not migrated")
            }
            SaveError::Mismatch(why) => write!(f, "the save does not fit this run: {why}"),
            SaveError::Io(why) => write!(f, "file error: {why}"),
        }
    }
}

impl std::error::Error for SaveError {}

fn io(context: &str, error: std::io::Error) -> SaveError {
    SaveError::Io(format!("{context}: {error}"))
}

/// What the header of a save says, without the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveInfo {
    pub slot: String,
    /// The text the game gave when it saved (a level name, "Checkpoint 2"). Shown in a slot list.
    pub label: String,
    /// When the save was written: seconds since 1970-01-01 UTC.
    pub saved_at: u64,
    pub tick: u64,
    /// The checksum of the saved frame (`Frame::checksum`).
    pub checksum: u64,
    pub game_version: u32,
    pub players: u8,
    pub tick_rate: u32,
}

/// One file of the save folder, as [`SaveStore::list`] sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveEntry {
    pub slot: String,
    /// The header, or why the file cannot be used.
    pub info: Result<SaveInfo, SaveError>,
}

/// A save that was read and checked: the frame is decoded and its checksum is the saved one.
pub struct LoadedSave {
    pub info: SaveInfo,
    frame: Frame,
}

impl LoadedSave {
    /// The saved frame.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// A new play session that starts at the saved tick with the saved state. `seed` only labels
    /// the recording of the session (the random generator state is part of the frame). Fails
    /// when the checksum of the new session is not the saved one.
    pub fn into_session<G: Game>(&self, seed: u64) -> Result<PlaySession<G>, SaveError> {
        let cfg = PlayConfig::new(self.info.players, seed, self.info.tick_rate);
        let session = PlaySession::<G>::from_frame(cfg, &self.frame).map_err(|e| SaveError::Corrupt(e.to_string()))?;
        let checksum = session.simulation().checksum();
        if checksum != self.info.checksum || session.simulation().tick() != self.info.tick {
            return Err(SaveError::Corrupt(format!("the new session has checksum {checksum:#x}, the save has {:#x}", self.info.checksum)));
        }
        Ok(session)
    }
}

/// The save folder of one game.
#[derive(Clone, Debug)]
pub struct SaveStore {
    dir: PathBuf,
    game: String,
    game_version: u32,
}

fn valid_game(game: &str) -> bool {
    !game.is_empty() && game.len() <= MAX_NAME && game.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_slot(slot: &str) -> bool {
    !slot.is_empty() && slot.len() <= MAX_SLOT && slot.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn absolute_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

impl SaveStore {
    /// The saves of `game` in the per-user data folder: [`SaveStore::default_dir`]. `game` is a
    /// stable id of 1 to 64 letters, digits, `-` or `_`. `game_version` is yours: raise it when an
    /// old save would no longer fit the game; a save of another version is refused.
    pub fn new(game: &str, game_version: u32) -> Result<Self, SaveError> {
        Self::at(Self::default_dir(game)?, game, game_version)
    }

    /// The same with an explicit folder (created at the first write).
    pub fn at(dir: impl Into<PathBuf>, game: &str, game_version: u32) -> Result<Self, SaveError> {
        if !valid_game(game) {
            return Err(SaveError::Unavailable(format!("bad game id {game:?}: use 1 to {MAX_NAME} letters, digits, '-' or '_'")));
        }
        Ok(Self { dir: dir.into(), game: game.to_string(), game_version })
    }

    /// `%LOCALAPPDATA%\orrery\games\<game>\saves` on Windows, `$XDG_DATA_HOME/orrery/games/<game>/saves`
    /// (else `~/.local/share/...`) on Linux, `~/Library/Application Support/orrery/games/<game>/saves` on macOS.
    pub fn default_dir(game: &str) -> Result<PathBuf, SaveError> {
        if !valid_game(game) {
            return Err(SaveError::Unavailable(format!("bad game id {game:?}")));
        }
        #[cfg(windows)]
        let base = absolute_var("LOCALAPPDATA").or_else(|| absolute_var("USERPROFILE").map(|p| p.join("AppData").join("Local")));
        #[cfg(target_os = "macos")]
        let base = absolute_var("HOME").map(|p| p.join("Library").join("Application Support"));
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = absolute_var("XDG_DATA_HOME").or_else(|| absolute_var("HOME").map(|p| p.join(".local").join("share")));
        #[cfg(not(any(windows, unix)))]
        let base: Option<PathBuf> = None;
        let base = base.ok_or_else(|| SaveError::Unavailable("there is no per-user data folder on this system".to_string()))?;
        Ok(base.join("orrery").join("games").join(game).join("saves"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn game(&self) -> &str {
        &self.game
    }

    pub fn game_version(&self) -> u32 {
        self.game_version
    }

    fn path(&self, slot: &str) -> Result<PathBuf, SaveError> {
        if !valid_slot(slot) {
            return Err(SaveError::BadSlot(slot.to_string()));
        }
        Ok(self.dir.join(format!("{PREFIX}{slot}.{EXTENSION}")))
    }

    /// Writes `frame` (the verified frame of a run with `players` players at `tick_rate`) into `slot`.
    pub fn write(&self, slot: &str, label: &str, frame: &Frame, players: u8, tick_rate: u32) -> Result<SaveInfo, SaveError> {
        self.write_bytes(slot, label, &frame.to_bytes(), frame.tick(), frame.checksum(), players, tick_rate)
    }

    /// Like [`SaveStore::write`] with the bytes of `Frame::to_bytes`.
    #[allow(clippy::too_many_arguments)]
    pub fn write_bytes(&self, slot: &str, label: &str, frame: &[u8], tick: u64, checksum: u64, players: u8, tick_rate: u32) -> Result<SaveInfo, SaveError> {
        let path = self.path(slot)?;
        let label = clip_label(label);
        let saved_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let info = SaveInfo { slot: slot.to_string(), label, saved_at, tick, checksum, game_version: self.game_version, players, tick_rate };
        let bytes = encode(&self.game, &info, frame);
        if bytes.len() as u64 > MAX_SAVE_BYTES {
            return Err(SaveError::Io(format!("the save is {} bytes, more than the {MAX_SAVE_BYTES} bytes allowed", bytes.len())));
        }
        self.make_dir()?;
        write_atomic(&path, &bytes)?;
        Ok(info)
    }

    /// Creates the save folder. A link or reparse point in the part that belongs to the game
    /// (`orrery/games/<game>/saves`, or the folder itself for another path) is refused.
    fn make_dir(&self) -> Result<(), SaveError> {
        fs::create_dir_all(&self.dir).map_err(|e| io("create the save folder", e))?;
        let own = self.dir.ends_with(Path::new("orrery").join("games").join(&self.game).join("saves"));
        for folder in self.dir.ancestors().take(if own { 4 } else { 1 }) {
            let metadata = fs::symlink_metadata(folder).map_err(|e| io("check the save folder", e))?;
            if !metadata.is_dir() || orr_fsid::is_link(&metadata) {
                return Err(SaveError::Io(format!("{} is not a real folder", folder.display())));
            }
        }
        Ok(())
    }

    fn read_file(&self, slot: &str) -> Result<Vec<u8>, SaveError> {
        let path = self.path(slot)?;
        let file = match orr_fsid::open_file(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SaveError::NotFound(slot.to_string())),
            Err(e) => return Err(io("open the save", e)),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SAVE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| io("read the save", e))?;
        if bytes.len() as u64 > MAX_SAVE_BYTES {
            return Err(SaveError::Corrupt(format!("the file is larger than {MAX_SAVE_BYTES} bytes")));
        }
        Ok(bytes)
    }

    /// The header of the save in `slot`: checks the file, its version, its game and its game version,
    /// but does not decode the frame.
    pub fn info(&self, slot: &str) -> Result<SaveInfo, SaveError> {
        let bytes = self.read_file(slot)?;
        decode(&bytes, slot, &self.game, self.game_version).map(|(info, _)| info)
    }

    /// Reads, checks and decodes the save in `slot` for game `G`. `players` and `tick_rate` are those of
    /// the run that loads it: a save of another player count or tick rate is refused. The checksum of
    /// the decoded frame is the saved one.
    pub fn load<G: Game>(&self, slot: &str, players: u8, tick_rate: u32) -> Result<LoadedSave, SaveError> {
        let bytes = self.read_file(slot)?;
        let (info, frame_bytes) = decode(&bytes, slot, &self.game, self.game_version)?;
        if info.players != players || info.tick_rate != tick_rate {
            return Err(SaveError::Mismatch(format!(
                "the save has {} players at {} ticks per second, this run has {players} players at {tick_rate}",
                info.players, info.tick_rate
            )));
        }
        let frame = Frame::from_bytes(Simulation::<G>::build_registry(), frame_bytes).map_err(|e| SaveError::Corrupt(e.to_string()))?;
        if frame.checksum() != info.checksum || frame.tick() != info.tick {
            return Err(SaveError::Corrupt("the frame is not the one the header describes".to_string()));
        }
        Ok(LoadedSave { info, frame })
    }

    /// Every slot of the folder in name order. A file that cannot be used is listed with its error.
    /// A missing folder is an empty list.
    pub fn list(&self) -> Result<Vec<SaveEntry>, SaveError> {
        let read = match fs::read_dir(&self.dir) {
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io("list the save folder", e)),
        };
        let mut slots = Vec::new();
        for entry in read {
            let entry = entry.map_err(|e| io("list the save folder", e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(slot) = name.strip_prefix(PREFIX).and_then(|rest| rest.strip_suffix(&format!(".{EXTENSION}"))) else { continue };
            if valid_slot(slot) {
                slots.push(slot.to_string());
            }
        }
        slots.sort();
        Ok(slots.into_iter().map(|slot| SaveEntry { info: self.info(&slot), slot }).collect())
    }

    /// Removes the save in `slot`.
    pub fn delete(&self, slot: &str) -> Result<(), SaveError> {
        let path = self.path(slot)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SaveError::NotFound(slot.to_string())),
            Err(e) => Err(io("delete the save", e)),
        }
    }
}

/// What the last save, load or checkpoint did, for a game to show in its own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveNotice {
    Saved { slot: String, tick: u64 },
    /// A load was accepted. The new session starts when the frame ends.
    Loaded { slot: String, tick: u64 },
    Failed(String),
}

/// How many frames a [`SaveNotice`] stays (2.5 seconds at 60 frames per second).
pub const NOTICE_FRAMES: u32 = 150;

/// The save state of a run, owned by `Views`: the folder, what a load must match, the load that waits
/// for the end of the frame, and the last notice.
#[derive(Default)]
pub(crate) struct SaveDesk {
    store: Option<SaveStore>,
    players: u8,
    tick_rate: u32,
    pending: Option<LoadedSave>,
    notice: Option<(SaveNotice, u32)>,
}

impl SaveDesk {
    pub(crate) fn enable(&mut self, store: SaveStore, players: u8, tick_rate: u32) {
        self.store = Some(store);
        self.players = players;
        self.tick_rate = tick_rate;
    }

    pub(crate) fn enabled(&self) -> bool {
        self.store.is_some()
    }

    fn store(&self) -> Result<&SaveStore, SaveError> {
        self.store.as_ref().ok_or_else(|| SaveError::Unavailable("this run has no save slots: call App::saves, and note that a networked run has none".to_string()))
    }

    fn record<T>(&mut self, result: Result<T, SaveError>, notice: impl FnOnce(&T) -> SaveNotice) -> Result<T, SaveError> {
        let shown = match &result {
            Ok(value) => notice(value),
            Err(error) => SaveNotice::Failed(error.to_string()),
        };
        self.notice = Some((shown, NOTICE_FRAMES));
        result
    }

    /// Saves the verified frame `frame` into `slot`.
    pub(crate) fn save(&mut self, frame: Option<orr_bridge::FrameView<'_>>, slot: &str, label: &str) -> Result<SaveInfo, SaveError> {
        let result = (|| {
            let store = self.store()?;
            let frame = frame.ok_or_else(|| SaveError::Unavailable("there is no verified frame yet".to_string()))?;
            store.write_bytes(slot, label, &frame.to_bytes(), frame.tick(), frame.checksum(), self.players, self.tick_rate)
        })();
        self.record(result, |info| SaveNotice::Saved { slot: info.slot.clone(), tick: info.tick })
    }

    /// Reads and checks the save. When it is good the run replaces its session at the end of the frame.
    pub(crate) fn load<G: Game>(&mut self, slot: &str) -> Result<SaveInfo, SaveError> {
        let result = self.store().and_then(|store| store.load::<G>(slot, self.players, self.tick_rate));
        let result = self.record(result, |loaded| SaveNotice::Loaded { slot: loaded.info.slot.clone(), tick: loaded.info.tick });
        result.map(|loaded| {
            let info = loaded.info.clone();
            self.pending = Some(loaded);
            info
        })
    }

    pub(crate) fn take_pending(&mut self) -> Option<LoadedSave> {
        self.pending.take()
    }

    pub(crate) fn list(&self) -> Result<Vec<SaveEntry>, SaveError> {
        self.store()?.list()
    }

    pub(crate) fn delete(&mut self, slot: &str) -> Result<(), SaveError> {
        let result = self.store().and_then(|store| store.delete(slot));
        if let Err(error) = &result {
            self.notice = Some((SaveNotice::Failed(error.to_string()), NOTICE_FRAMES));
        }
        result
    }

    /// A load that was accepted could not start its session.
    pub(crate) fn fail(&mut self, error: &SaveError) {
        self.notice = Some((SaveNotice::Failed(error.to_string()), NOTICE_FRAMES));
    }

    pub(crate) fn notice(&self) -> Option<&SaveNotice> {
        self.notice.as_ref().map(|(notice, _)| notice)
    }

    /// One frame passes: a notice that was shown for [`NOTICE_FRAMES`] frames goes away.
    pub(crate) fn tick_notice(&mut self) {
        if let Some((_, frames)) = &mut self.notice {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.notice = None;
            }
        }
    }
}

/// At most 120 bytes, cut at a character boundary, without control characters.
fn clip_label(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_LABEL_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn encode(game: &str, info: &SaveInfo, frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 128);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&SAVE_VERSION.to_le_bytes());
    put_str(&mut out, game);
    out.extend_from_slice(&info.game_version.to_le_bytes());
    out.extend_from_slice(&info.saved_at.to_le_bytes());
    put_str(&mut out, &info.label);
    out.push(info.players);
    out.extend_from_slice(&info.tick_rate.to_le_bytes());
    out.extend_from_slice(&info.tick.to_le_bytes());
    out.extend_from_slice(&info.checksum.to_le_bytes());
    out.extend_from_slice(&(frame.len() as u64).to_le_bytes());
    out.extend_from_slice(frame);
    let hash = xxh3_64(&out);
    out.extend_from_slice(&hash.to_le_bytes());
    out
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SaveError> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.bytes.len()).ok_or_else(|| SaveError::Corrupt("the file is cut off".to_string()))?;
        let part = &self.bytes[self.at..end];
        self.at = end;
        Ok(part)
    }

    fn u32(&mut self) -> Result<u32, SaveError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64, SaveError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    fn text(&mut self, max: usize) -> Result<String, SaveError> {
        let len = self.u32()? as usize;
        if len > max {
            return Err(SaveError::Corrupt("a text field is too long".to_string()));
        }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| SaveError::Corrupt("a text field is not utf-8".to_string()))
    }
}

/// Checks the file as a whole and returns the header and the bytes of the frame.
fn decode<'a>(bytes: &'a [u8], slot: &str, game: &str, game_version: u32) -> Result<(SaveInfo, &'a [u8]), SaveError> {
    let mut cursor = Cursor { bytes, at: 0 };
    if cursor.take(4)? != MAGIC {
        return Err(SaveError::Corrupt("this is not an Orrery save file".to_string()));
    }
    let version = cursor.u32()?;
    if version != SAVE_VERSION {
        return Err(SaveError::UnsupportedVersion { found: version, supported: SAVE_VERSION });
    }
    if bytes.len() < 8 {
        return Err(SaveError::Corrupt("the file is cut off".to_string()));
    }
    let (body, trailer) = bytes.split_at(bytes.len() - 8);
    if xxh3_64(body) != u64::from_le_bytes(trailer.try_into().expect("8 bytes")) {
        return Err(SaveError::Corrupt("the file checksum does not match".to_string()));
    }
    let mut cursor = Cursor { bytes: body, at: 8 };
    let found_game = cursor.text(MAX_NAME)?;
    if found_game != game {
        return Err(SaveError::ForeignGame { found: found_game, expected: game.to_string() });
    }
    let found_version = cursor.u32()?;
    if found_version != game_version {
        return Err(SaveError::GameVersion { found: found_version, expected: game_version });
    }
    let saved_at = cursor.u64()?;
    let label = cursor.text(MAX_LABEL_BYTES)?;
    let players = cursor.take(1)?[0];
    let tick_rate = cursor.u32()?;
    let tick = cursor.u64()?;
    let checksum = cursor.u64()?;
    let frame_len = usize::try_from(cursor.u64()?).map_err(|_| SaveError::Corrupt("the frame length is too large".to_string()))?;
    let frame = cursor.take(frame_len)?;
    if cursor.at != body.len() {
        return Err(SaveError::Corrupt("bytes follow the frame".to_string()));
    }
    let info = SaveInfo { slot: slot.to_string(), label, saved_at, tick, checksum, game_version: found_version, players, tick_rate };
    Ok((info, frame))
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a temporary file next to `path`, syncs it, and renames it over `path`.
/// On any failure the temporary file is removed and `path` is as it was.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let dir = path.parent().ok_or_else(|| SaveError::Io("the slot file has no folder".to_string()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| io("create the temporary file", e))?;
        file.write_all(bytes).map_err(|e| io("write the save", e))?;
        file.sync_all().map_err(|e| io("sync the save", e))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| io("replace the slot", e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        return result;
    }
    #[cfg(unix)]
    if let Ok(folder) = fs::File::open(dir) {
        let _ = folder.sync_all();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orr_app_save_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn info() -> SaveInfo {
        SaveInfo { slot: "a".to_string(), label: "Level 2".to_string(), saved_at: 99, tick: 7, checksum: 0xABCD, game_version: 3, players: 2, tick_rate: 60 }
    }

    #[test]
    fn the_header_round_trips_and_the_checksum_of_the_file_guards_every_byte() {
        let bytes = encode("demo", &info(), &[1, 2, 3, 4]);
        let (back, frame) = decode(&bytes, "a", "demo", 3).unwrap();
        assert_eq!(back, info());
        assert_eq!(frame, &[1, 2, 3, 4]);
        for at in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[at] ^= 0x01;
            assert!(decode(&damaged, "a", "demo", 3).is_err(), "a flipped bit at {at} was not noticed");
        }
        for len in 0..bytes.len() {
            assert!(decode(&bytes[..len], "a", "demo", 3).is_err(), "a file cut at {len} was accepted");
        }
    }

    #[test]
    fn a_foreign_game_a_game_version_and_a_format_version_are_refused_with_their_names() {
        let bytes = encode("demo", &info(), &[1]);
        assert_eq!(decode(&bytes, "a", "other", 3).unwrap_err(), SaveError::ForeignGame { found: "demo".into(), expected: "other".into() });
        assert_eq!(decode(&bytes, "a", "demo", 4).unwrap_err(), SaveError::GameVersion { found: 3, expected: 4 });
        let mut future = bytes.clone();
        future[4..8].copy_from_slice(&2u32.to_le_bytes());
        let error = decode(&future, "a", "demo", 3).unwrap_err();
        assert_eq!(error, SaveError::UnsupportedVersion { found: 2, supported: 1 });
        assert!(error.to_string().contains("version 2") && error.to_string().contains("not migrated"));
    }

    #[test]
    fn slot_and_game_names_are_checked_before_any_path_is_made() {
        let dir = temp_dir("names");
        let store = SaveStore::at(&dir, "demo", 1).unwrap();
        for bad in ["", "../x", "a/b", "a b", "con.", &"x".repeat(33)] {
            assert_eq!(store.path(bad).unwrap_err(), SaveError::BadSlot(bad.to_string()));
        }
        assert!(SaveStore::at(&dir, "../demo", 1).is_err());
        assert!(SaveStore::at(&dir, "", 1).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_label_is_cut_at_a_character_and_loses_control_characters() {
        assert_eq!(clip_label("a\nb\tc"), "abc");
        let long = "가".repeat(100);
        let cut = clip_label(&long);
        assert!(cut.len() <= MAX_LABEL_BYTES && cut.chars().all(|c| c == '가'));
        assert_eq!(cut.chars().count(), 40);
    }

    #[test]
    fn a_replaced_slot_holds_the_new_bytes_and_leaves_no_temporary_file() {
        let dir = temp_dir("atomic");
        let path = dir.join("slot-a.orrsave");
        write_atomic(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["slot-a.orrsave".to_string()]);
        let _ = fs::remove_dir_all(dir);
    }

    /// A read-only slot file cannot be replaced on Windows: the write fails after the temporary file exists.
    #[cfg(windows)]
    #[test]
    fn a_write_that_fails_keeps_the_old_save_and_removes_the_temporary_file() {
        let dir = temp_dir("atomic_fail");
        let path = dir.join("slot-a.orrsave");
        write_atomic(&path, b"old").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        assert!(write_atomic(&path, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["slot-a.orrsave".to_string()], "no temporary file is left");
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    /// A folder without write permission stops the write before the rename (the test user must not be root).
    #[cfg(unix)]
    #[test]
    fn a_write_that_fails_keeps_the_old_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("atomic_fail");
        let path = dir.join("slot-a.orrsave");
        write_atomic(&path, b"old").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let failed = write_atomic(&path, b"new").is_err();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        if failed {
            assert_eq!(fs::read(&path).unwrap(), b"old");
        }
        let _ = fs::remove_dir_all(dir);
    }
}
