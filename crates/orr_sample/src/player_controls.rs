//! Presentation-only settings transaction. No discovery occurs until explicitly
//! requested by the interactive authored-UI launch route.
use crate::arena_input::{validate_map, ArenaControls};
use crate::player_settings::{
    CommitOutcome, FireBinding, PlayerSettingsV1, SettingsPaths, SettingsStore,
};
use std::path::PathBuf;

/// Resolve external player preferences, never an authored project's content.
/// Protected roots are admitted project and executable/export folders.
pub fn resolve_paths(
    directory: Option<PathBuf>,
    protected_roots: &[PathBuf],
) -> Result<SettingsPaths, String> {
    let paths = match directory {
        Some(directory) => SettingsPaths::from_directory(directory)?,
        None => SettingsPaths::from_environment()?,
    };
    if paths
        .directory()
        .components()
        .any(|part| part.as_os_str() == ".orr")
        || protected_roots
            .iter()
            .any(|root| paths.directory().starts_with(root))
    {
        return Err("settings directory must be external to the project and export".into());
    }
    Ok(paths)
}

pub struct PlayerSettingsSession {
    store: Option<SettingsStore>,
    active: PlayerSettingsV1,
    map: orr_input::ActionMap,
    draft: FireBinding,
    editable: bool,
    external: bool,
    needs_save: bool,
    status: String,
}

impl PlayerSettingsSession {
    /// An explicit input map is session-only and avoids even settings discovery.
    pub fn external(map: orr_input::ActionMap) -> Self {
        Self {
            store: None,
            active: PlayerSettingsV1::default(),
            map,
            draft: FireBinding::Space,
            editable: false,
            external: true,
            needs_save: false,
            status: "외부 입력 파일 사용 중: 이 실행에서는 설정을 저장하지 않습니다.".into(),
        }
    }
    pub fn open(paths: Result<SettingsPaths, String>) -> Self {
        let defaults = PlayerSettingsV1::default();
        let (store, active, editable, needs_save, status) = match paths {
            Ok(paths) => {
                let store = SettingsStore::new(paths);
                let loaded = store.load();
                let editable = loaded.writable();
                let needs_save =
                    editable && loaded.state != crate::player_settings::LoadState::Primary;
                let status = loaded
                    .notice
                    .unwrap_or_else(|| "설정은 이 컴퓨터의 Arena에서 공유됩니다.".into());
                (Some(store), loaded.settings, editable, needs_save, status)
            }
            Err(error) => (
                None,
                defaults,
                false,
                false,
                format!("설정 저장 불가: {error}"),
            ),
        };
        let map = active
            .action_map()
            .expect("loaded or default settings are validated");
        Self {
            store,
            draft: active.fire_binding,
            active,
            map,
            editable,
            external: false,
            needs_save,
            status,
        }
    }
    pub fn action_map(&self) -> orr_input::ActionMap {
        self.map.clone()
    }
    pub fn draft(&self) -> FireBinding {
        self.draft
    }
    pub fn active(&self) -> FireBinding {
        self.active.fire_binding
    }
    pub fn editable(&self) -> bool {
        self.editable
    }
    pub fn is_external(&self) -> bool {
        self.external
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn dirty(&self) -> bool {
        self.needs_save || self.draft != self.active.fire_binding
    }
    pub fn select(&mut self, fire: FireBinding) {
        if self.editable {
            self.draft = fire;
        }
    }
    pub fn cancel(&mut self) {
        self.draft = self.active.fire_binding;
    }
    pub fn reset(&mut self) {
        self.select(FireBinding::Space);
    }
    /// Both adapters are prepared before touching disk. A rejected save leaves
    /// them intact. After publication a warning is honest about durability; the
    /// published map becomes active and further saves require a fresh launch.
    pub fn apply(&mut self, controls: &mut ArenaControls) -> bool {
        if !self.editable || !self.dirty() {
            return false;
        }
        let mut candidate = self.active.clone();
        candidate.fire_binding = self.draft;
        let prepared = candidate.action_map().and_then(|map| {
            validate_map(&map)?;
            controls
                .with_replaced_map(map.clone())
                .map(|controls| (map, controls))
        });
        let (map, prepared) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.status = format!("설정 적용 실패: {error}");
                return false;
            }
        };
        let Some(store) = &self.store else {
            return false;
        };
        match store.commit(&candidate) {
            CommitOutcome::NotPublished(error) => {
                self.status = format!("저장 실패. 이전 조작을 유지합니다: {error}");
                return false;
            }
            CommitOutcome::Published => self.status = "설정을 저장하고 적용했습니다.".into(),
            CommitOutcome::PublishedDurabilityUncertain(error) => {
                self.status = format!("설정은 적용됐지만 저장 안정성을 확인할 수 없습니다. 다시 실행해 확인하세요: {error}");
                self.editable = false;
            }
        }
        self.needs_save = false;
        self.map = map;
        self.active = candidate;
        *controls = prepared;
        true
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn protected_roots_and_internal_package_paths_are_never_profiles() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        assert!(resolve_paths(Some(project.join("settings")), &[project]).is_err());
        assert!(resolve_paths(Some(root.path().join(".orr/settings")), &[]).is_err());
        assert!(resolve_paths(Some(root.path().join("external")), &[]).is_ok());
    }
    #[test]
    fn unavailable_profile_keeps_default_playable_and_read_only() {
        let session = PlayerSettingsSession::open(Err("test unavailable".into()));
        assert!(!session.editable());
        assert!(session.status().contains("test unavailable"));
        assert_eq!(session.action_map(), crate::arena_input::default_map());
    }
    #[test]
    fn reset_is_draft_only_and_cancel_retains_committed_map() {
        let root = tempfile::tempdir().unwrap();
        let paths = SettingsPaths::from_directory(root.path().join("profile")).unwrap();
        let mut session = PlayerSettingsSession::open(Ok(paths.clone()));
        let mut controls = ArenaControls::new(session.action_map()).unwrap();
        session.select(FireBinding::LeftMouse);
        assert!(session.apply(&mut controls));
        let bytes = std::fs::read(paths.primary()).unwrap();
        session.reset();
        assert_eq!(session.draft(), FireBinding::Space);
        assert_eq!(std::fs::read(paths.primary()).unwrap(), bytes);
        session.cancel();
        assert_eq!(session.draft(), FireBinding::LeftMouse);
        assert!(!session.dirty());
        assert!(!session.apply(&mut controls));
        assert_eq!(std::fs::read(paths.primary()).unwrap(), bytes);
    }
}
