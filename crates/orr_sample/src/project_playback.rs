//! Shared snapshot-driven, presentation-only authored sprite cursors.
//! Positions must come from the same coherent snapshot and complete entity generation.
//! Repeated snapshots freeze; rewind, epoch/history change and >8-tick gaps reset.
use crate::project_sprites::{Document, Source};
use orr_ecs::Entity;
use std::collections::BTreeMap;
const MAX_FORWARD_TICKS: u64 = 8;

/// Sampling state for one configured scene GUID. Unknown/missing GUIDs are idle
/// at frame zero, never a cursor inherited from a recyclable entity index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackState {
    pub moving: bool,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub seq: u64,
    pub tick: u64,
    pub epoch: u64,
    pub tick_rate: u32,
    pub checksum: u64,
    pub playing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Advance {
    Reset,
    Still,
    Forward,
}

impl Observation {
    fn matches_recorded_history(self, recent_checksums: &[(u64, u64)]) -> bool {
        recent_checksums
            .iter()
            .find(|(tick, _)| *tick == self.tick)
            .is_none_or(|(_, checksum)| *checksum == self.checksum)
    }

    fn advance_from(self, previous: Option<Self>) -> Advance {
        let Some(previous) = previous else {
            return Advance::Reset;
        };
        if self == previous {
            return Advance::Still;
        }
        if self.epoch != previous.epoch
            || self.seq <= previous.seq
            || self.tick < previous.tick
            || self.tick_rate == 0
            || self.tick_rate != previous.tick_rate
        {
            return Advance::Reset;
        }
        if self.tick == previous.tick {
            // Play/Pause can publish new metadata without changing the frame.
            // A changed pose/checksum at that same tick is a discontinuity.
            return if self.checksum == previous.checksum {
                Advance::Still
            } else {
                Advance::Reset
            };
        }
        if self.tick - previous.tick > MAX_FORWARD_TICKS {
            Advance::Reset
        } else {
            // Advancing paused snapshots are Step, and count exactly like
            // running snapshots. The playing flag never drives a UI clock.
            Advance::Forward
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub entity: Entity,
    pub pos: [f32; 2],
}

#[derive(Default)]
struct Cursor {
    previous: Option<Position>,
    clip_started_tick: u64,
    state: PlaybackState,
}

impl Cursor {
    fn observe(
        &mut self,
        observation: Observation,
        advance: Advance,
        position: Option<Position>,
        locomotion: bool,
    ) {
        let position = position.filter(|p| p.pos.iter().all(|v| v.is_finite()));
        let previous = self.previous;
        self.previous = position;
        let (Some(previous), Some(position)) = (previous, position) else {
            self.reset(observation.tick);
            return;
        };
        if advance == Advance::Reset
            || previous.entity != position.entity
            || (advance == Advance::Still && previous.pos != position.pos)
        {
            self.reset(observation.tick);
            return;
        }
        if advance == Advance::Still {
            return;
        }
        let moving = locomotion && previous.pos != position.pos;
        if moving != self.state.moving {
            self.clip_started_tick = observation.tick;
        }
        self.state = PlaybackState {
            moving,
            elapsed_ms: tick_millis(
                observation.tick.saturating_sub(self.clip_started_tick),
                observation.tick_rate,
            ),
        };
    }

    fn reset(&mut self, tick: u64) {
        self.clip_started_tick = tick;
        self.state = PlaybackState::default();
    }
}

fn tick_millis(ticks: u64, tick_rate: u32) -> u64 {
    (u128::from(ticks) * 1000 / u128::from(tick_rate.max(1))).min(u128::from(u64::MAX)) as u64
}

#[derive(Default)]
pub struct SnapshotPlayback {
    observation: Option<Observation>,
    cursors: BTreeMap<String, Cursor>,
}
impl SnapshotPlayback {
    pub fn state(&self, guid: &str) -> PlaybackState {
        self.cursors
            .get(guid)
            .map_or(PlaybackState::default(), |cursor| cursor.state)
    }
    pub fn reset(&mut self) {
        self.observation = None;
        self.cursors.clear();
    }
    pub fn observe(
        &mut self,
        document: &Document,
        observation: Observation,
        positions: &BTreeMap<String, Option<Position>>,
        recent_checksums: &[(u64, u64)],
    ) {
        // Reuse the snapshot's bounded checksum window without retaining it.
        // This also rejects a replaced recording whose head happens to move
        // forward a few ticks with the same epoch and sequence direction.
        let changed_history = self.observation.is_some_and(|previous| {
            observation.tick > previous.tick && !previous.matches_recorded_history(recent_checksums)
        });
        let advance = if changed_history {
            Advance::Reset
        } else {
            observation.advance_from(self.observation)
        };
        self.observation = Some(observation);
        self.cursors
            .retain(|guid, _| document.bindings.contains_key(guid));
        for (guid, binding) in &document.bindings {
            self.cursors.entry(guid.clone()).or_default().observe(
                observation,
                advance,
                positions.get(guid).copied().flatten(),
                matches!(binding.source, Source::Locomotion { .. }),
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_sprites::Binding;
    fn observation(tick: u64) -> Observation {
        Observation {
            seq: tick + 1,
            tick,
            epoch: 0,
            tick_rate: 60,
            checksum: tick + 100,
            playing: false,
        }
    }

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

    fn observe(playback: &mut SnapshotPlayback, tick: u64, a: Option<f32>, b: Option<f32>) {
        playback.observe(
            &document(),
            observation(tick),
            &BTreeMap::from([
                ("e_00000001".into(), a.map(|x| position(1, x))),
                ("e_00000002".into(), b.map(|x| position(2, x))),
            ]),
            &[],
        );
    }

    #[test]
    fn paused_snapshot_freezes_and_steps_advance_independent_cursors() {
        let mut playback = SnapshotPlayback::default();
        observe(&mut playback, 0, Some(0.0), Some(0.0));
        observe(&mut playback, 1, Some(1.0), Some(0.0));
        assert_eq!(
            playback.state("e_00000001"),
            PlaybackState {
                moving: true,
                elapsed_ms: 0
            }
        );
        assert_eq!(
            playback.state("e_00000002"),
            PlaybackState {
                moving: false,
                elapsed_ms: 16
            }
        );
        observe(&mut playback, 2, Some(2.0), Some(0.0));
        let moving = playback.state("e_00000001");
        let idle = playback.state("e_00000002");
        for _ in 0..20 {
            observe(&mut playback, 2, Some(2.0), Some(0.0));
        }
        assert_eq!(playback.state("e_00000001"), moving);
        assert_eq!(playback.state("e_00000002"), idle);
        assert_eq!(moving.elapsed_ms, 16);
        assert_eq!(idle.elapsed_ms, 33);
        observe(&mut playback, 3, Some(2.0), Some(1.0));
        assert_eq!(playback.state("e_00000001"), PlaybackState::default());
        assert_eq!(
            playback.state("e_00000002"),
            PlaybackState {
                moving: true,
                elapsed_ms: 0
            }
        );
    }
    #[test]
    fn metadata_only_pause_does_not_reset_a_clip() {
        let previous = Observation {
            playing: true,
            ..observation(3)
        };
        let paused = Observation {
            seq: previous.seq + 1,
            playing: false,
            ..previous
        };
        assert_eq!(paused.advance_from(Some(previous)), Advance::Still);
        let stepped = Observation {
            seq: paused.seq + 1,
            ..observation(4)
        };
        assert_eq!(stepped.advance_from(Some(paused)), Advance::Forward);
    }
    #[test]
    fn seek_rewind_jump_checksum_change_and_sequence_restart_reset() {
        let previous = observation(10);
        for changed in [
            Observation {
                seq: 12,
                epoch: 1,
                ..observation(11)
            },
            Observation {
                seq: 12,
                ..observation(9)
            },
            Observation {
                seq: 12,
                ..observation(30)
            },
            Observation {
                seq: 12,
                checksum: 999,
                ..previous
            },
            Observation {
                seq: 1,
                ..observation(11)
            },
            Observation {
                seq: 12,
                tick_rate: 30,
                ..observation(11)
            },
        ] {
            assert_eq!(changed.advance_from(Some(previous)), Advance::Reset);
            let mut cursor = Cursor {
                previous: Some(position(1, 0.0)),
                clip_started_tick: 0,
                state: PlaybackState {
                    moving: true,
                    elapsed_ms: 100,
                },
            };
            cursor.observe(changed, Advance::Reset, Some(position(1, 5.0)), true);
            assert_eq!(cursor.state, PlaybackState::default());
        }
        assert_eq!(
            observation(18).advance_from(Some(previous)),
            Advance::Forward
        );
    }
    #[test]
    fn missing_and_recycled_entities_never_inherit_a_cursor() {
        let mut playback = SnapshotPlayback::default();
        observe(&mut playback, 0, Some(0.0), Some(0.0));
        observe(&mut playback, 1, Some(1.0), Some(0.0));
        observe(&mut playback, 2, None, Some(0.0));
        assert_eq!(playback.state("e_00000001"), PlaybackState::default());
        observe(&mut playback, 3, Some(8.0), Some(0.0));
        assert_eq!(playback.state("e_00000001"), PlaybackState::default());
        observe(&mut playback, 4, Some(9.0), Some(0.0));
        let recycled = Position {
            entity: Entity {
                index: 1,
                version: 1,
            },
            pos: [10.0, 0.0],
        };
        playback.observe(
            &document(),
            observation(5),
            &BTreeMap::from([("e_00000001".into(), Some(recycled))]),
            &[],
        );
        assert_eq!(playback.state("e_00000001"), PlaybackState::default());
        assert_eq!(playback.state("not configured"), PlaybackState::default());
    }
    #[test]
    fn plain_clip_ignores_motion_transitions_and_ticks_do_not_round_per_frame() {
        let mut cursor = Cursor::default();
        for tick in 0..=60 {
            cursor.observe(
                observation(tick),
                if tick == 0 {
                    Advance::Reset
                } else {
                    Advance::Forward
                },
                Some(position(1, (tick % 3) as f32)),
                false,
            );
        }
        assert_eq!(
            cursor.state,
            PlaybackState {
                moving: false,
                elapsed_ms: 1000
            }
        );
        assert_eq!(tick_millis(u64::MAX, 1), u64::MAX);
    }
    #[test]
    fn retained_history_contains_only_current_bindings() {
        let mut playback = SnapshotPlayback::default();
        observe(&mut playback, 0, Some(0.0), Some(0.0));
        let mut smaller = document();
        smaller.bindings.remove("e_00000001");
        playback.observe(&smaller, observation(1), &BTreeMap::new(), &[]);
        assert_eq!(playback.cursors.len(), 1);
        assert_eq!(playback.state("e_00000001"), PlaybackState::default());
        playback.reset();
        assert!(playback.cursors.is_empty());
        assert!(playback.observation.is_none());
    }
    #[test]
    fn changed_recording_checksum_resets_a_small_forward_span() {
        let mut playback = SnapshotPlayback::default();
        observe(&mut playback, 0, Some(0.0), Some(0.0));
        observe(&mut playback, 1, Some(1.0), Some(0.0));
        observe(&mut playback, 2, Some(2.0), Some(0.0));
        assert!(playback.state("e_00000001").moving);
        let positions = BTreeMap::from([
            ("e_00000001".into(), Some(position(1, 3.0))),
            ("e_00000002".into(), Some(position(2, 0.0))),
        ]);
        playback.observe(&document(), observation(3), &positions, &[(2, 999)]);
        assert_eq!(playback.state("e_00000001"), PlaybackState::default());
        assert_eq!(playback.state("e_00000002"), PlaybackState::default());
        assert!(observation(2).matches_recorded_history(&[(2, 102)]));
        assert!(observation(2).matches_recorded_history(&[]));
    }
}
