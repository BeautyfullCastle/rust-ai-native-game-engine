//! Pure, isolated burst playback of an explicitly accepted document snapshot.
//!
//! The preview origin is world zero and the seed is fixed across starts/restarts.
//! Repeatability is for the same snapshot and delta sequence, not simulation state.
//! Callers own wall-clock sampling and must reset it on pause/focus/visibility
//! transitions so time spent in the background never becomes catch-up work.
use orr_bridge::EventKey;
use orr_sample::particle_preset::{ParticlePreset, PresetError};
use orr_view::{
    particles::{ParticleDraw, ParticleError, ParticlePool, MAX_STEP},
    Vec3,
};

pub const PREVIEW_ORIGIN: Vec3 = Vec3::new(0.0, 0.0, 0.0);
pub const PREVIEW_SEED: u64 = 0x4f52_5245_5259;

/// No drafts, document history, runtime bridge, files, or GPU resources are owned.
/// Only explicit `start` replaces the accepted snapshot; restart preserves it.
#[derive(Default)]
pub struct Preview {
    pool: ParticlePool,
    snapshot: Option<ParticlePreset>,
    playing: bool,
}

impl Preview {
    /// Capture the accepted document on an explicit Preview/Refresh action.
    /// Invalid input leaves the previous snapshot and playback untouched.
    pub fn start(&mut self, accepted: &ParticlePreset) -> Result<(), PresetError> {
        let emitter = accepted.to_emitter(PREVIEW_ORIGIN, PREVIEW_SEED)?;
        self.pool.reset();
        self.pool
            .start(EventKey::new(0, 0, 0), Some(emitter))
            .expect("a validated preset produces a valid emitter");
        self.snapshot = Some(accepted.clone());
        self.playing = true;
        Ok(())
    }

    /// Clear all previous particles and replay the same captured snapshot/seed.
    /// Restarting a closed preview does nothing.
    pub fn restart(&mut self) -> Result<(), PresetError> {
        if let Some(snapshot) = self.snapshot.clone() {
            self.start(&snapshot)?;
        }
        Ok(())
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// An expired burst stays stopped; use Restart to emit it again.
    pub fn resume(&mut self) {
        self.playing = self.active() && self.pool.particle_count() > 0;
    }

    /// Closing discards the snapshot, particles, and all event bookkeeping.
    pub fn close(&mut self) {
        self.pool.reset();
        self.snapshot = None;
        self.playing = false;
    }

    /// Reject invalid deltas before any mutation, even while inactive/paused.
    /// Long finite frames advance at most MAX_STEP (0.25 seconds), with no debt.
    pub fn advance(&mut self, dt: f32, focused: bool, visible: bool) -> Result<(), ParticleError> {
        if !dt.is_finite() || dt < 0.0 {
            return Err(ParticleError::InvalidDelta);
        }
        if !self.playing || !focused || !visible {
            return Ok(());
        }
        self.pool.advance(dt.min(MAX_STEP))?;
        if self.pool.particle_count() == 0 {
            self.playing = false;
        }
        Ok(())
    }

    pub fn draws(&self) -> impl ExactSizeIterator<Item = ParticleDraw> + '_ {
        self.pool.draws()
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// A captured session exists, including a paused or naturally expired burst.
    pub fn active(&self) -> bool {
        self.snapshot.is_some()
    }

    pub fn snapshot(&self) -> Option<&ParticlePreset> {
        self.snapshot.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> Preview {
        let mut preview = Preview::default();
        preview.start(&ParticlePreset::default()).unwrap();
        preview
    }

    fn draws(preview: &Preview) -> Vec<ParticleDraw> {
        preview.draws().collect()
    }

    #[test]
    fn same_snapshot_and_sequence_and_restart_are_repeatable() {
        let mut first = started();
        let mut second = started();
        let initial = draws(&first);
        assert!(initial.iter().all(|draw| draw.position == PREVIEW_ORIGIN));
        let sequence = [0.016, 0.031, 0.0, 0.1];
        for dt in sequence {
            first.advance(dt, true, true).unwrap();
            second.advance(dt, true, true).unwrap();
            assert_eq!(draws(&first), draws(&second));
        }
        let final_draws = draws(&first);
        first.restart().unwrap();
        assert_eq!(draws(&first), initial);
        for dt in sequence {
            first.advance(dt, true, true).unwrap();
        }
        assert_eq!(draws(&first), final_draws);
    }

    #[test]
    fn pause_focus_and_visibility_freeze_without_accumulation() {
        let mut preview = started();
        preview.advance(0.1, true, true).unwrap();
        let before = draws(&preview);
        preview.pause();
        preview.advance(10.0, true, true).unwrap();
        assert!(!preview.playing());
        assert_eq!(draws(&preview), before);
        preview.resume();
        assert!(preview.playing());
        for (focused, visible) in [(false, true), (true, false), (false, false)] {
            preview.advance(100.0, focused, visible).unwrap();
            assert_eq!(draws(&preview), before);
        }
        let mut reference = started();
        reference.advance(0.1, true, true).unwrap();
        reference.advance(0.01, true, true).unwrap();
        preview.advance(0.01, true, true).unwrap();
        assert_eq!(draws(&preview), draws(&reference));
    }

    #[test]
    fn invalid_deltas_are_atomic_even_when_paused_hidden_or_closed() {
        for paused in [false, true] {
            for (focused, visible) in [(true, true), (false, true), (true, false)] {
                let mut preview = started();
                preview.advance(0.1, true, true).unwrap();
                if paused {
                    preview.pause();
                }
                let before = draws(&preview);
                let snapshot = preview.snapshot().cloned();
                for dt in [-0.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    assert_eq!(
                        preview.advance(dt, focused, visible),
                        Err(ParticleError::InvalidDelta)
                    );
                    assert_eq!(draws(&preview), before);
                    assert_eq!(preview.snapshot(), snapshot.as_ref());
                    assert_eq!(preview.playing(), !paused);
                }
            }
        }
        let mut closed = Preview::default();
        assert_eq!(
            closed.advance(f32::NAN, false, false),
            Err(ParticleError::InvalidDelta)
        );
        assert!(!closed.active());
    }

    #[test]
    fn long_frame_clamps_to_max_step_without_catchup_debt() {
        for dt in [MAX_STEP + 0.01, 100.0, f32::MAX] {
            let mut preview = started();
            let mut reference = started();
            preview.advance(dt, true, true).unwrap();
            reference.advance(MAX_STEP, true, true).unwrap();
            assert_eq!(draws(&preview), draws(&reference));
            preview.advance(0.01, true, true).unwrap();
            reference.advance(0.01, true, true).unwrap();
            assert_eq!(draws(&preview), draws(&reference));
        }
    }

    #[test]
    fn natural_expiry_stops_and_restart_recreates_exactly_one_burst() {
        let mut preview = started();
        let initial = draws(&preview);
        preview.advance(MAX_STEP, true, true).unwrap();
        preview.advance(MAX_STEP, true, true).unwrap();
        assert_eq!(preview.draws().len(), 0);
        assert!(!preview.playing());
        assert!(preview.active());
        preview.resume();
        assert!(!preview.playing());
        for _ in 0..64 {
            preview.restart().unwrap();
            assert_eq!(draws(&preview), initial);
            assert_eq!(preview.pool.emitter_count(), 0);
            preview.advance(0.01, true, true).unwrap();
        }
    }

    #[test]
    fn snapshots_change_only_on_explicit_start_and_invalid_start_is_atomic() {
        let mut accepted = ParticlePreset::default();
        let mut preview = Preview::default();
        preview.start(&accepted).unwrap();
        let original = accepted.clone();
        accepted.burst = 32;
        accepted.velocity = [2.0, 3.0, 4.0];
        preview.restart().unwrap();
        assert_eq!(preview.snapshot(), Some(&original));
        assert_eq!(preview.draws().len(), original.burst as usize);
        preview.start(&accepted).unwrap();
        assert_eq!(preview.snapshot(), Some(&accepted));
        assert_eq!(preview.draws().len(), 32);
        preview.advance(0.1, true, true).unwrap();
        let before = draws(&preview);
        let mut invalid = accepted.clone();
        invalid.burst = 0;
        assert!(preview.start(&invalid).is_err());
        assert_eq!(draws(&preview), before);
        assert_eq!(preview.snapshot(), Some(&accepted));
        assert!(preview.playing());
    }

    #[test]
    fn close_clears_all_state_and_requires_explicit_new_snapshot() {
        let mut preview = started();
        preview.close();
        assert!(!preview.active());
        assert!(!preview.playing());
        assert!(preview.snapshot().is_none());
        assert_eq!(preview.draws().len(), 0);
        preview.resume();
        preview.restart().unwrap();
        preview.advance(0.1, true, true).unwrap();
        assert!(!preview.active());
        assert!(!preview.playing());
        assert_eq!(preview.draws().len(), 0);
        preview.start(&ParticlePreset::default()).unwrap();
        assert_eq!(draws(&preview), draws(&started()));
    }
}
