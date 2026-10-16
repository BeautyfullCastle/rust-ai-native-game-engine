//! Snapshot-driven, presentation-only sprite playback and Arena camera follow.
//!
//! Only the previous displayed, coherent observation is retained for each
//! configured GUID. Motion is inferred from its position delta over at most
//! eight forward ticks. This deliberately cannot reconstruct transitions in
//! skipped intermediate frames (including moving away and returning). A seek,
//! rewind, epoch change or longer gap starts at idle frame zero instead. There
//! is no wall-clock animation and nothing here writes to the host or sidecar.
//! An entire unobserved Stop/Start cycle with identical scene, epoch, ticks and
//! recorded checksums is indistinguishable from uninterrupted playback: the
//! existing snapshot API supplies no unique play-session identifier.

use std::{collections::BTreeMap, path::PathBuf};

use orr_ecs::Entity;
use orr_render::{camera::CameraFollow, Camera};

use crate::{
    editor::Editor,
    game::{Drawable, EditorGame},
    model::EntityRow,
    sprite_bindings::Document,
};

pub use orr_sample::project_playback::PlaybackState;
use orr_sample::project_playback::{Observation, Position, SnapshotPlayback};

/// Resolves both halves of the join, including the complete entity generation.
/// Callers must additionally establish coherence of these asynchronously read
/// rows with the displayed snapshot before calling this function.
fn resolve_position(rows: &[EntityRow], bodies: &[Drawable], guid: &str) -> Option<Position> {
    let row = rows
        .iter()
        .find(|row| row.guid.as_ref().is_some_and(|g| g.as_str() == guid))?;
    let body = bodies.iter().find(|body| body.entity == row.entity)?;
    body.pos.iter().all(|v| v.is_finite()).then_some(Position {
        entity: row.entity,
        pos: body.pos,
    })
}

/// Call after the editor pumps its snapshots and before drawing the viewport.
/// Manual navigation should call [`Self::suspend_follow`] before moving the
/// camera. Follow is active only inside Play (paused/stepped Play included).
/// When the displayed snapshot returns to Edit, Stop restores the complete
/// camera pose captured on entering Play if follow was actually applied.
#[derive(Default)]
pub struct SpritePlayback {
    #[cfg(feature="collect-dodge")] collect_elapsed: Option<u32>,
    document: Option<Document>,
    scene: Option<PathBuf>,
    scene_matches: bool,
    sampler: SnapshotPlayback,
    follow: CameraFollow<Entity>,
    suspended: bool,
    session_active: bool,
    edit_camera: Option<Camera>,
    restore_camera_on_stop: bool,
    diagnostic: Option<String>,
}

impl SpritePlayback {
    pub fn state(&self, guid: &str) -> PlaybackState {
        self.sampler.state(guid)
    }

    /// Explicit Open/New/Close must call this, even when the replacement has
    /// identical contents. Keep the edit-camera restore point across sidecar
    /// replacement during Play; resetting the whole controller would lose it.
    pub fn reset_document(&mut self) {
        self.document = None;
        self.reset_observations();
        self.suspended = false;
        self.diagnostic = None;
    }

    pub fn suspend_follow(&mut self) {
        if !self.session_active
            || !self.scene_matches
            || !self
                .document
                .as_ref()
                .is_some_and(|document| document.camera_follow.is_some())
        {
            return;
        }
        self.suspended = true;
        self.follow.stop();
    }

    pub fn resume_follow(&mut self) {
        self.suspended = false;
    }

    pub fn follow_suspended(&self) -> bool {
        self.suspended
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    fn reset_observations(&mut self) {
        #[cfg(feature="collect-dodge")] { self.collect_elapsed = None; }
        self.sampler.reset();
        self.follow.stop();
    }

    fn set_session(&mut self, active: bool, camera: &mut Camera) {
        if self.session_active == active {
            return;
        }
        if active {
            self.edit_camera = Some(*camera);
        } else if let Some(edit_camera) = self.edit_camera.take() {
            if self.restore_camera_on_stop {
                *camera = edit_camera;
            }
        }
        self.restore_camera_on_stop = false;
        self.session_active = active;
        self.reset_observations();
        self.suspended = false;
    }

    pub fn update(
        &mut self,
        editor: &mut Editor,
        document: Option<&Document>,
        scene_matches: bool,
    ) {
        let scene = editor.path();
        let scene_changed = self.scene != scene;
        if scene_changed {
            // A different scene must not receive the old scene's camera pose.
            self.edit_camera = None;
            self.session_active = false;
            self.restore_camera_on_stop = false;
        }
        if scene_changed
            || self.document.as_ref() != document
            || self.scene_matches != scene_matches
        {
            self.reset_document();
            self.scene = scene;
            self.document = document.cloned();
            self.scene_matches = scene_matches;
        }
        // The ERP state can lead or lag the displayed frame. Camera lifecycle
        // follows the frame; a temporary absent snapshot keeps the restore
        // point until the next actual Edit/Play snapshot establishes its mode.
        if let Some(active) = editor
            .snapshot()
            .map(|snapshot| snapshot.timeline().is_some())
        {
            self.set_session(active, &mut editor.camera);
        }
        self.diagnostic = None;
        let Some(document) = document else {
            return;
        };
        if !scene_matches || (editor.game() != EditorGame::Arena && !editor.game().is_collect()) || editor.previewing().is_some() {
            self.reset_observations();
            return;
        }
        let Some(snapshot) = editor.snapshot() else {
            self.reset_observations();
            self.diagnostic = Some("Waiting for a displayed Play snapshot".into());
            return;
        };
        let active = snapshot.timeline().is_some();
        if editor.is_playing_mode() != active {
            // In particular, do not join post-Stop edit rows to an older Play
            // frame: freshly baked edit handles may reuse generation zero.
            self.diagnostic =
                Some("Waiting for the displayed snapshot and host mode to agree".into());
            return;
        }
        let Some(timeline) = snapshot.timeline() else {
            return;
        };
        if timeline.tick != snapshot.tick() || snapshot.tick_rate() == 0 {
            self.reset_observations();
            self.diagnostic = Some("Waiting for a coherent Play timeline".into());
            return;
        }
        if !editor.yard_rows_coherent() {
            // Step/seek and edits refresh ERP rows asynchronously. Holding the
            // previous observation here lets a settled one-tick Step compare
            // against the previous pose, without ever resolving stale rows.
            self.diagnostic =
                Some("Waiting for the scene GUID map to match the displayed snapshot".into());
            return;
        }
        #[cfg(feature="collect-dodge")]
        if editor.game().is_collect() {
            let elapsed = snapshot.predicted().singleton::<orr_sample::collect_game::CollectRun>().elapsed_ticks;
            if self.collect_elapsed.is_some_and(|previous| elapsed < previous) { self.sampler.reset(); }
            self.collect_elapsed = Some(elapsed);
        }
        let observation = Observation {
            seq: snapshot.seq(),
            tick: timeline.tick,
            epoch: timeline.epoch,
            tick_rate: snapshot.tick_rate(),
            checksum: snapshot.predicted().checksum(),
            playing: timeline.playing,
        };
        let positions: BTreeMap<_, _> = document
            .bindings
            .keys()
            .chain(document.camera_follow.iter())
            .map(|guid| {
                (
                    guid.clone(),
                    resolve_position(editor.rows(), editor.bodies(), guid),
                )
            })
            .collect();
        self.observe(
            document,
            observation,
            &positions,
            &timeline.recent_checksums,
        );
        let target = document.camera_follow.as_deref();
        let position = target.and_then(|guid| positions.get(guid).copied().flatten());
        if let Some(guid) = target {
            if position.is_none() {
                self.diagnostic = Some(format!(
                    "Camera follow target is missing or has no drawable position: {guid}"
                ));
            }
        }
        self.update_follow(&mut editor.camera, target.is_some(), position);
    }

    fn observe(
        &mut self,
        document: &Document,
        observation: Observation,
        positions: &BTreeMap<String, Option<Position>>,
        recent_checksums: &[(u64, u64)],
    ) {
        self.sampler
            .observe(document, observation, positions, recent_checksums);
    }

    fn update_follow(&mut self, camera: &mut Camera, configured: bool, position: Option<Position>) {
        // A configured-but-missing target still owns the Play camera lifecycle.
        // Manual navigation in that session must restore the Edit pose on Stop.
        self.restore_camera_on_stop |= configured;
        if !configured || self.suspended {
            self.follow.stop();
            return;
        }
        if let Some(position) = position {
            self.follow.follow(position.entity);
        }
        // Missing targets deliberately hold the last center. In particular,
        // never resolve an old target by entity index alone.
        self.follow.update(camera, |entity| {
            position.filter(|p| p.entity == *entity).map(|p| p.pos)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite_bindings::{Binding, Source};
    use orr_reflect::Guid;
    use orr_sample::editor_view::Outline;
    use orr_view::Style;

    fn position(index: u32, x: f32) -> Position {
        Position {
            entity: Entity { index, version: 0 },
            pos: [x, 0.0],
        }
    }

    fn locomotion() -> Binding {
        Binding {
            package: "demo".into(),
            document: "hero.sprite.json".into(),
            source: Source::Locomotion {
                idle: "idle".into(),
                walk: "walk".into(),
            },
            units_per_pixel: 0.1,
        }
    }

    fn document() -> Document {
        Document {
            version: 2,
            scene: "demo.scene.yaml".into(),
            project: "packages".into(),
            bindings: BTreeMap::from([
                ("e_00000001".into(), locomotion()),
                ("e_00000002".into(), locomotion()),
            ]),
            camera_follow: Some("e_00000001".into()),
        }
    }

    #[test]
    fn guid_resolution_requires_the_full_entity_generation() {
        let old = position(1, 0.0).entity;
        let rows = [EntityRow {
            entity: old,
            guid: Some(Guid::parse("e_00000001").unwrap()),
            name: None,
            components: Vec::new(),
        }];
        let mut bodies = [Drawable {
            entity: Entity { version: 1, ..old },
            pos: [10.0, 5.0],
            angle: 0.0,
            style: Style {
                shape: orr_view::Shape::Circle,
                size: 1.0,
                half_y: 0.0,
                color: [1.0; 4],
            },
            turn: 0.0,
            outline: Outline::Circle { radius: 1.0 },
        }];
        assert!(resolve_position(&rows, &bodies, "e_00000001").is_none());
        bodies[0].entity = old;
        assert_eq!(
            resolve_position(&rows, &bodies, "e_00000001").unwrap().pos,
            [10.0, 5.0]
        );
        bodies[0].pos[0] = f32::NAN;
        assert!(resolve_position(&rows, &bodies, "e_00000001").is_none());
    }
    #[test]
    fn follow_holds_missing_suspends_resumes_and_stop_restores_edit_camera() {
        let edit = Camera::new([3.0, 4.0], 12.0);
        let mut camera = edit;
        let mut playback = SpritePlayback {
            document: Some(document()),
            scene_matches: true,
            ..Default::default()
        };
        playback.set_session(true, &mut camera);
        playback.update_follow(&mut camera, true, Some(position(1, 9.0)));
        assert_eq!(camera, Camera::new([9.0, 0.0], 12.0));
        playback.update_follow(&mut camera, true, None);
        assert_eq!(camera.center, [9.0, 0.0]);
        playback.suspend_follow();
        camera = Camera::new([30.0, 40.0], 6.0);
        playback.update_follow(&mut camera, true, Some(position(1, 15.0)));
        assert!(playback.follow_suspended());
        assert_eq!(camera.center, [30.0, 40.0]);
        playback.resume_follow();
        playback.update_follow(&mut camera, true, Some(position(1, 15.0)));
        assert_eq!(camera, Camera::new([15.0, 0.0], 6.0));
        playback.reset_document();
        playback.set_session(false, &mut camera);
        assert_eq!(camera, edit);
        assert!(!playback.follow_suspended());
        assert!(playback.follow.target().is_none());
    }
    #[test]
    fn an_unconfigured_play_session_does_not_restore_over_manual_navigation() {
        let mut camera = Camera::new([0.0, 0.0], 12.0);
        let mut playback = SpritePlayback::default();
        playback.set_session(true, &mut camera);
        playback.suspend_follow();
        assert!(
            !playback.follow_suspended(),
            "unconfigured navigation never shows a resume action"
        );
        camera = Camera::new([30.0, 40.0], 6.0);
        playback.set_session(false, &mut camera);
        assert_eq!(camera, Camera::new([30.0, 40.0], 6.0));
    }
    #[test]
    fn configured_missing_target_restores_edit_camera_after_manual_navigation() {
        let edit = Camera::new([3.0, 4.0], 12.0);
        let mut camera = edit;
        let mut playback = SpritePlayback {
            document: Some(document()),
            scene_matches: true,
            ..Default::default()
        };
        playback.set_session(true, &mut camera);
        playback.update_follow(&mut camera, true, None);
        assert_eq!(camera, edit);
        playback.suspend_follow();
        assert!(playback.follow_suspended());
        camera.pan_pixels([20.0, 30.0], (800, 600));
        camera.zoom_at(2.0, [200.0, 250.0], (800, 600));
        assert_ne!(camera, edit);
        playback.set_session(false, &mut camera);
        assert_eq!(camera, edit);
    }
}
