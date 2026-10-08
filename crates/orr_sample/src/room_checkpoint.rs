//! Optional Room host checkpoint policy. Only the key milestone is persistent;
//! the simulation never reads files and an arbitrary Frame is never restored.
use crate::room_project::PreparedProject;
use orr_games::room_escape_game::RoomActor;
#[cfg(target_os = "linux")]
use orr_games::room_escape_game::{RoomEscapeV1, RoomRun};
use orr_physics3d::{Body, Collider};
#[cfg(target_os = "linux")]
use orr_sim::Simulation;
use sha2::{Digest, Sha256};

/// Fixed-width little-endian semantic encoding, sorted by gameplay role/ordinal.
/// Presentation paths, names, GUIDs, model assets, camera and UI are excluded.
// Bump for semantic Room step/query changes not already represented by the
// encoded constants, including movement speed, edge ordering or collision rules.
const RULES_REVISION: u32 = 1;
pub fn challenge_digest(project: &PreparedProject) -> [u8; 32] {
    digest_for_rules(project, RULES_REVISION)
}
fn digest_for_rules(project: &PreparedProject, rules_revision: u32) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"orrery.room.key-checkpoint.challenge.v1\0");
    hash.update(rules_revision.to_le_bytes());
    hash.update(crate::room_project::SEED.to_le_bytes());
    hash.update(orr_games::room_escape_game::TICK_RATE.to_le_bytes());
    // Revision covers interaction-edge ordering, axis order and speed rules.
    // These public numerical gameplay limits are also encoded explicitly.
    for value in [
        orr_games::room_escape_game::PLAYER_RADIUS,
        orr_games::room_escape_game::PLAYER_Y,
        orr_games::room_escape_game::SKIN,
        orr_games::room_escape_game::CENTER_BOUND,
        orr_games::room_escape_game::INTERACT_DISTANCE,
    ] {
        hash.update(value.0.to_le_bytes());
    }
    let frame = project.scene().frame();
    let actors: Vec<_> = frame
        .entities()
        .map(|entity| {
            (
                frame.get::<RoomActor>(entity).expect("admitted actor"),
                frame.get::<Body>(entity).expect("admitted body"),
                frame.get::<Collider>(entity).expect("admitted collider"),
            )
        })
        .collect();
    hash.update((actors.len() as u32).to_le_bytes());
    let mut records = Vec::with_capacity(actors.len());
    for (actor, body, collider) in actors {
        let mut record = Vec::new();
        for value in [
            actor.kind,
            actor.ordinal,
            body.kind,
            body.sleep,
            body.island,
            collider.shape.kind,
            collider.layer,
            collider.mask,
            collider.flags,
        ] {
            record.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            body.pos.x,
            body.pos.y,
            body.pos.z,
            body.rot.x,
            body.rot.y,
            body.rot.z,
            body.rot.w,
            body.vel.x,
            body.vel.y,
            body.vel.z,
            body.omega.x,
            body.omega.y,
            body.omega.z,
            body.inv_mass,
            body.inv_inertia.x,
            body.inv_inertia.y,
            body.inv_inertia.z,
            body.linear_damping,
            body.angular_damping,
            collider.shape.radius,
            collider.shape.half.x,
            collider.shape.half.y,
            collider.shape.half.z,
            collider.restitution,
            collider.friction,
        ] {
            record.extend_from_slice(&value.0.to_le_bytes());
        }
        records.push(record);
    }
    records.sort();
    for record in records {
        hash.update(record);
    }
    hash.finalize().into()
}

/// Recreate the previously validated initial scene, then install only the key.
/// No positions, physics caches, won state or prior input edges are loaded.
#[cfg(target_os = "linux")]
pub(crate) fn restore(
    project: &PreparedProject,
    key_collected: bool,
) -> Result<Simulation<RoomEscapeV1>, String> {
    let mut simulation = project.scene().simulation()?;
    simulation.frame_mut().set_singleton(RoomRun {
        key_collected: u32::from(key_collected),
        ..RoomRun::default()
    });
    Ok(simulation)
}

#[cfg(target_os = "linux")]
mod host {
    use super::*;
    use crate::room_checkpoint_store::{
        CheckpointKey, CheckpointPaths, CheckpointStore, CommitOutcome,
    };
    pub(crate) struct CheckpointSession {
        store: Option<CheckpointStore>,
        attempted: bool,
        status: String,
    }
    impl CheckpointSession {
        /// Called only for the normal standalone window. Metadata-only routes
        /// must never call this, including editor/headless/capture/export smoke.
        pub(crate) fn open(project: &PreparedProject) -> Option<Self> {
            let identity = project.checkpoint()?;
            let result = (|| {
                let key = CheckpointKey {
                    game_id: identity.game_id_bytes()?,
                    challenge: challenge_digest(project),
                };
                let paths = CheckpointPaths::from_environment(&key)?;
                let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                let folder = executable.parent().ok_or("executable parent missing")?;
                let bundle = if folder.file_name().is_some_and(|n| n == "bin") {
                    folder.parent().unwrap_or(folder)
                } else {
                    folder
                };
                if paths
                    .directory()
                    .components()
                    .any(|c| c.as_os_str() == ".orr")
                    || [project.root(), bundle]
                        .iter()
                        .any(|root| paths.directory().starts_with(root))
                {
                    return Err(
                        "checkpoint data must be external to project and export roots".into(),
                    );
                }
                CheckpointStore::open(paths, key)
            })();
            Some(Self::from_store(result))
        }
        pub(crate) fn from_store(result: Result<CheckpointStore, String>) -> Self {
            match result {
                Ok(store) => {
                    let status = store.notice().map(str::to_owned).unwrap_or_else(|| {
                        if store.key_collected() {
                            "Key checkpoint available".into()
                        } else {
                            "No key checkpoint".into()
                        }
                    });
                    Self {
                        store: Some(store),
                        attempted: false,
                        status,
                    }
                }
                Err(error) => Self {
                    store: None,
                    attempted: false,
                    status: format!("Checkpoint unavailable: {error}"),
                },
            }
        }
        pub(crate) fn available(&self) -> bool {
            self.store.as_ref().is_some_and(|s| s.key_collected())
        }
        pub(crate) fn status(&self) -> &str {
            &self.status
        }
        /// Called synchronously after every authoritative simulation step.
        pub(crate) fn observe(&mut self, run: RoomRun) {
            if run.key_collected == 0 || self.attempted {
                return;
            }
            self.attempted = true;
            if let Some(store) = &mut self.store {
                let outcome = store.save_key();
                self.report(outcome, "Key checkpoint saved");
            }
        }
        fn report(&mut self, outcome: Result<CommitOutcome, String>, success: &str) -> bool {
            self.status = match outcome {
                Ok(CommitOutcome::DurablySaved) => success.into(),
                Ok(CommitOutcome::DurabilityUncertain(error)) => {
                    format!("Checkpoint durability uncertain: {error}")
                }
                Err(error) => format!("Checkpoint not saved: {error}"),
            };
            eprintln!("{}", self.status);
            self.status == success
        }
        /// Only a proven durable reset authorizes replacing the active game.
        pub(crate) fn new_game(&mut self) -> bool {
            let outcome = self
                .store
                .as_mut()
                .ok_or_else(|| "storage unavailable".into())
                .and_then(|s| s.reset());
            let saved = self.report(outcome, "New Game checkpoint reset saved");
            if saved {
                self.attempted = false;
            }
            saved
        }
    }
}
#[cfg(target_os = "linux")]
pub(crate) use host::CheckpointSession;

#[cfg(all(
    test,
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod tests {
    use super::*;
    #[test]
    fn rules_revision_separates_checkpoint_namespace_without_touching_scene() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("room");
        crate::project_create::create_room_checkpoint(
            &crate::project_create::CreateOptions {
                output: root.clone(),
                template: crate::project_create::ROOM_UI_TEMPLATE.into(),
                seed: "rules".into(),
            },
            "12345678-1234-4234-9234-123456789abc",
        )
        .unwrap();
        let project = PreparedProject::open_with_options(
            &root,
            true,
            crate::room_project::CheckpointSupport::MetadataOnly,
        )
        .unwrap();
        let before = project.scene().frame().checksum();
        assert_ne!(digest_for_rules(&project, 1), digest_for_rules(&project, 2));
        assert_eq!(project.scene().frame().checksum(), before);
    }
}
