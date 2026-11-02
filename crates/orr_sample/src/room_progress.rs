//! Optional local Room clear records. Never changes simulation or writes a project.
use crate::{
    game_progress::{
        room::{RoomClearKey, RoomClearStore},
        CommitOutcome,
    },
    room_project::PreparedProject,
};

pub(crate) struct ProgressSession {
    store: Option<RoomClearStore>,
    attempted: bool,
    status: String,
}
impl ProgressSession {
    /// Read-only discovery at native App startup; absent identity does no I/O.
    pub(crate) fn open(project: &PreparedProject) -> Self {
        let Some(identity) = project.progress() else {
            return Self {
                store: None,
                attempted: false,
                status: "Clear record disabled: no authored identity".into(),
            };
        };
        let result = (|| {
            let key = RoomClearKey {
                game_id: identity.game_id_bytes()?,
                challenge: crate::room_progress_digest::challenge_digest(project),
            };
            let paths = key.paths_from_environment()?;
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let folder = exe.parent().ok_or("executable has no parent")?;
            let export = if folder.file_name().is_some_and(|n| n == "bin") {
                folder.parent().unwrap_or(folder)
            } else {
                folder
            };
            crate::game_progress::require_external(paths.directory(), &[project.root(), export])?;
            RoomClearStore::open(paths, key)
        })();
        Self::from_store(result)
    }
    pub(crate) fn from_store(result: Result<RoomClearStore, String>) -> Self {
        match result {
            Ok(store) => {
                let attempted = store.cleared();
                let status = store.notice().map(str::to_owned).unwrap_or_else(|| {
                    if attempted {
                        "Room cleared (loaded)".into()
                    } else {
                        "Room not cleared yet".into()
                    }
                });
                Self {
                    store: Some(store),
                    attempted,
                    status,
                }
            }
            Err(error) => Self {
                store: None,
                attempted: false,
                status: format!("Clear record not saved: {error}"),
            },
        }
    }
    /// Called immediately after a standalone authoritative step, before fallible
    /// presentation. Terminal draws/ticks and restart cannot submit twice.
    pub(crate) fn observe(
        &mut self,
        before_tick: u64,
        before_won: u32,
        after_tick: u64,
        after_won: u32,
    ) {
        if self.attempted
            || before_won != 0
            || after_won != 1
            || before_tick.checked_add(1) != Some(after_tick)
        {
            return;
        }
        self.attempted = true;
        let Some(store) = self.store.as_mut() else {
            return;
        };
        self.status = match store.mark_cleared() {
            Ok(CommitOutcome::DurablySaved) => "Room cleared (saved)".into(),
            Ok(CommitOutcome::DurabilityUncertain(error)) => {
                format!("Clear record durability uncertain: {error}")
            }
            Err(error) => format!("Clear record not saved: {error}"),
        };
        eprintln!("{}", self.status);
    }
    pub(crate) fn status(&self) -> &str {
        &self.status
    }
}
