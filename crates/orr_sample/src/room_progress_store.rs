//! Typed Room completion persistence using the shared guarded Linux transport.
//! This is a monotone completion record, not a checkpoint or simulation save.
use super::*;

pub const PROFILE: &str = "room-escape-clear-v1";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomClearKey {
    pub game_id: [u8; 16],
    pub challenge: [u8; 32],
}
impl RoomClearKey {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.game_id[6] >> 4 != 4 || self.game_id[8] >> 6 != 2 {
            return Err("Room completion requires a UUIDv4 game identity".into());
        }
        Ok(())
    }
    pub(super) fn game(&self) -> String {
        let h = hex(&self.game_id);
        format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        )
    }
    pub fn paths_from_environment(&self) -> Result<ProgressPaths, String> {
        self.paths_from_environment_values(
            std::env::var_os("XDG_DATA_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
    pub fn paths_from_environment_values(
        &self,
        xdg: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<ProgressPaths, String> {
        self.validate()?;
        environment_paths(xdg, home, &self.game(), PROFILE, &self.challenge)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompletionV1 {
    pub(super) version: u32,
    pub(super) game_id: String,
    pub(super) profile: String,
    pub(super) challenge: String,
    pub(super) cleared: bool,
}
impl CompletionV1 {
    pub(super) fn new(key: &RoomClearKey, cleared: bool) -> Self {
        Self {
            version: 1,
            game_id: key.game(),
            profile: PROFILE.into(),
            challenge: hex(&key.challenge),
            cleared,
        }
    }
}
#[derive(Debug)]
pub struct RoomClearStore {
    paths: ProgressPaths,
    key: RoomClearKey,
    loaded: Loaded,
}
impl RoomClearStore {
    pub fn open(paths: ProgressPaths, key: RoomClearKey) -> Result<Self, String> {
        key.validate()?;
        #[cfg(target_os = "linux")]
        let loaded = linux::load_codec(&paths, StoreCodec::Room(&key));
        #[cfg(not(target_os = "linux"))]
        let loaded = Loaded::readonly(0, "progress persistence is supported only on Linux");
        if loaded.best > 1 {
            return Err("invalid internal Room completion value".into());
        }
        Ok(Self { paths, key, loaded })
    }
    pub fn cleared(&self) -> bool {
        self.loaded.best == 1
    }
    pub fn notice(&self) -> Option<&str> {
        self.loaded.notice.as_deref()
    }
    pub fn mark_cleared(&mut self) -> Result<CommitOutcome, String> {
        self.mark_with_checkpoint(|_| Ok(()))
    }
    fn mark_with_checkpoint(
        &mut self,
        checkpoint: impl FnMut(&str) -> Result<(), String>,
    ) -> Result<CommitOutcome, String> {
        if matches!(self.loaded.write_state, WriteState::ReadOnly) {
            return Err(self
                .loaded
                .notice
                .clone()
                .unwrap_or_else(|| "progress is read-only".into()));
        }
        #[cfg(target_os = "linux")]
        {
            let (value, outcome) =
                linux::commit_codec(&self.paths, StoreCodec::Room(&self.key), 1, checkpoint)?;
            if value > 1 {
                return Err("invalid internal Room completion value".into());
            }
            self.loaded.best = value;
            self.loaded.write_state = WriteState::Ready;
            self.loaded.notice = match &outcome {
                CommitOutcome::DurablySaved => None,
                CommitOutcome::DurabilityUncertain(reason) => Some(reason.clone()),
            };
            if matches!(outcome, CommitOutcome::DurabilityUncertain(_)) {
                self.loaded.write_state = WriteState::ReadOnly;
            }
            Ok(outcome)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = checkpoint;
            Err("progress persistence is supported only on Linux".into())
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "room_progress_store_tests.rs"]
mod tests;
