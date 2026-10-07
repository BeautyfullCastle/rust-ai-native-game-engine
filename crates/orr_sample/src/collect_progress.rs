//! Local window progress: semantic identity and view-side persistence only.
//! No save state enters a Frame, replay, network identity or simulation checksum.
use crate::collect_game::{CollectActor, CollectRun};
use crate::collect_project::PreparedProject;
use sha2::{Digest, Sha256};

/// Versioned fixed-width semantic encoding. Each field has a documented width;
/// the actor sequence has a u32 count and is sorted by (kind, ordinal).
/// Paths, GUID spelling, names, YAML layout and build versions are excluded.
pub(crate) fn challenge_digest(project: &PreparedProject) -> [u8; 32] {
    digest_for_rules(project, 1)
}
fn digest_for_rules(project: &PreparedProject, rules_revision: u32) -> [u8; 32] {
    let frame = project.scene().frame();
    let mut hash = Sha256::new();
    let domain = b"orrery.collect-dodge.challenge.v1";
    hash.update((domain.len() as u32).to_le_bytes());
    hash.update(domain);
    hash.update(rules_revision.to_le_bytes()); // Compiled gameplay rules revision.
    hash.update(crate::collect_project::SEED.to_le_bytes());
    hash.update(crate::collect_project::TICK_RATE.to_le_bytes());
    hash.update(
        frame
            .singleton::<CollectRun>()
            .time_limit_ticks
            .to_le_bytes(),
    );
    let mut actors: Vec<_> = frame
        .entities()
        .filter_map(|entity| frame.get::<CollectActor>(entity))
        .collect();
    actors.sort_by_key(|a| (a.kind, a.ordinal));
    hash.update((actors.len() as u32).to_le_bytes());
    for actor in actors {
        hash.update(actor.kind.to_le_bytes());
        hash.update(actor.ordinal.to_le_bytes());
        for value in [
            actor.initial_position.x,
            actor.initial_position.y,
            actor.initial_velocity.x,
            actor.initial_velocity.y,
        ] {
            hash.update(value.0.to_le_bytes());
        }
    }
    hash.finalize().into()
}

#[cfg(target_os = "linux")]
mod storage {
    use super::*;
    use crate::game_progress::{CommitOutcome, ProgressKey, ProgressPaths, ProgressStore};
    use std::path::Path;

    pub(crate) struct ProgressSession {
        store: Option<ProgressStore>,
        attempted: Option<u32>,
        status: String,
    }
    impl ProgressSession {
        /// Called only by the normal standalone window route. Absence of an
        /// explicit identity performs no environment discovery or file access.
        pub(crate) fn open(project: &PreparedProject) -> Self {
            let Some(identity) = project.progress() else {
                return Self {
                    store: None,
                    attempted: None,
                    status: "Progress disabled: no authored identity".into(),
                };
            };
            let result = (|| {
                let key = ProgressKey {
                    game_id: identity.game_id_bytes()?,
                    challenge: challenge_digest(project),
                    goal: project.scene().frame().singleton::<CollectRun>().goal,
                };
                let paths = ProgressPaths::from_environment(&key)?;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                // Export executables live in bundle/bin; protect the whole bundle.
                let folder = exe.parent().ok_or("executable has no parent")?;
                let export = if folder.file_name().is_some_and(|n| n == "bin") {
                    folder.parent().unwrap_or(folder)
                } else {
                    folder
                };
                require_external(paths.directory(), &[project.root(), export])?;
                ProgressStore::open(paths, key)
            })();
            Self::from_store(result)
        }
        fn from_store(result: Result<ProgressStore, String>) -> Self {
            match result {
                Ok(store) => {
                    let status = store
                        .notice()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Best collected: {}", store.best()));
                    Self {
                        store: Some(store),
                        attempted: None,
                        status,
                    }
                }
                Err(error) => Self {
                    store: None,
                    attempted: None,
                    status: format!("Progress not saved: {error}"),
                },
            }
        }
        /// A monotonic reliable host accumulator, never a presentation snapshot.
        /// Each new maximum is attempted once; uncertain publication is not retried.
        pub(crate) fn observe_completed(&mut self, score: Option<u32>) {
            let Some(score) = score else {
                return;
            };
            if self.attempted.is_some_and(|previous| score <= previous) {
                return;
            }
            self.attempted = Some(score);
            let Some(store) = self.store.as_mut() else {
                return;
            };
            self.status = match store.submit_completed(score) {
                Ok(CommitOutcome::DurablySaved) => {
                    format!("Best collected: {} (saved)", store.best())
                }
                Ok(CommitOutcome::DurabilityUncertain(error)) => {
                    format!("Progress durability uncertain: {error}")
                }
                Err(error) => format!("Progress not saved: {error}"),
            };
            eprintln!("{}", self.status);
        }
        pub(crate) fn status(&self) -> &str {
            &self.status
        }
    }
    fn require_external(directory: &Path, protected: &[&Path]) -> Result<(), String> {
        if directory.components().any(|c| c.as_os_str() == ".orr")
            || protected.iter().any(|root| directory.starts_with(root))
        {
            return Err("progress data must be external to project and export roots".into());
        }
        Ok(())
    }
}
#[cfg(target_os = "linux")]
pub(crate) use storage::ProgressSession;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect_project::ProgressSupport;
    fn fixture(scene: &str, id: &str) -> (tempfile::TempDir, PreparedProject) {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        std::fs::write(root.path().join("level.yaml"), scene).unwrap();
        std::fs::write(root.path().join("orr.project.json"), serde_json::json!({"schema":3,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"level.yaml"},"progress":{"schema":1,"game_id":id,"profile":"collect-dodge-highscore-v1"}}).to_string()).unwrap();
        let project =
            PreparedProject::open_with_progress(root.path(), ProgressSupport::MetadataOnly)
                .unwrap();
        (root, project)
    }
    const ID: &str = "12345678-1234-4234-8234-123456789abc";
    const SCENE: &str = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
    #[test]
    fn semantic_identity_ignores_relocation_names_guids_and_formatting() {
        let (_a, a) = fixture(SCENE, ID);
        let edited = format!(
            "# cosmetic\n{}",
            SCENE
                .replace("name: player", "name: renamed hero")
                .replace("e_0000000", "e_1111111")
        );
        let (_b, b) = fixture(&edited, ID);
        assert_eq!(challenge_digest(&a), challenge_digest(&b));
        assert_ne!(a.root(), b.root());
        assert_eq!(a.progress(), b.progress());
    }
    #[test]
    fn semantic_identity_separates_level_changes_and_explicit_forks() {
        let (_a, a) = fixture(SCENE, ID);
        for changed in [
            SCENE.replace("time_limit_ticks: 600", "time_limit_ticks: 601"),
            SCENE.replace("position: [10, 0]", "position: [11, 0]"),
            SCENE.replace(
                "position: [0, 12], velocity: [0, 0]",
                "position: [0, 12], velocity: [1, 0]",
            ),
        ] {
            let (_b, b) = fixture(&changed, ID);
            assert_ne!(challenge_digest(&a), challenge_digest(&b));
        }
        assert_ne!(challenge_digest(&a), digest_for_rules(&a, 2));
        let fork = a
            .progress()
            .unwrap()
            .fork_with_game_id("22345678-1234-4234-8234-123456789abc".into())
            .unwrap();
        let (_b, b) = fixture(SCENE, &fork.game_id);
        assert_eq!(challenge_digest(&a), challenge_digest(&b));
        assert_ne!(a.progress(), b.progress());
    }
    #[test]
    fn metadata_admission_is_explicit_and_never_rewrites_identity() {
        let (root, project) = fixture(SCENE, ID);
        let path = root.path().join("orr.project.json");
        let before = std::fs::read(&path).unwrap();
        assert!(PreparedProject::open(root.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(project.progress().unwrap().game_id, ID);
        let legacy = serde_json::json!({"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"level.yaml"}}).to_string();
        std::fs::write(&path, &legacy).unwrap();
        let legacy_project = PreparedProject::open(root.path()).unwrap();
        assert!(legacy_project.progress().is_none());
        assert_eq!(
            legacy_project.scene().frame().checksum(),
            project.scene().frame().checksum()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy);
    }
    #[test]
    fn isolated_process_relaunch_uses_only_external_progress() {
        let (root, _) = fixture(SCENE, ID);
        let sandbox = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let data = sandbox.path().join("data");
        let before = std::fs::read(root.path().join("orr.project.json")).unwrap();
        for mode in ["win", "relaunch"] {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "collect_progress::tests::isolated_window_progress_child",
                    "--nocapture",
                ])
                .env("ORR_PROGRESS_TEST_PROJECT", root.path())
                .env("ORR_PROGRESS_TEST_MODE", mode)
                .env("XDG_DATA_HOME", &data)
                .env("HOME", sandbox.path().join("home"))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
        }
        assert_eq!(
            std::fs::read(root.path().join("orr.project.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
        assert!(data.join("orrery/games").is_dir());
    }
    /// Explicit child hook in the TEST binary only; production has no hidden
    /// option that allows headless/capture/export smoke to access progress.
    #[test]
    #[ignore = "requires isolated test project and XDG_DATA_HOME"]
    fn isolated_window_progress_child() {
        let project = PreparedProject::open_with_presentation(
            std::env::var_os("ORR_PROGRESS_TEST_PROJECT").expect("explicit project"),
            ProgressSupport::MetadataOnly,
            crate::collect_project::compiled_sprite_support(),
        )
        .unwrap();
        let mode = std::env::var("ORR_PROGRESS_TEST_MODE").expect("explicit mode");
        crate::collect_app::exercise_window_progress(&project, &mode);
    }
}
