//! Bounded presentation clock; never touches a host tick or game state.
#![allow(clippy::float_arithmetic)]
const SAMPLE_INTERVAL: f64 = 1.0 / 30.0;

#[derive(Default)]
pub(super) struct Transport {
    pub(super) running: bool,
    seconds: f64,
    clock: Option<(f64, u64)>,
    elapsed: f64,
}

impl Transport {
    pub(super) fn pause(&mut self) {
        self.running = false;
        self.clock = None;
        self.elapsed = 0.0;
    }

    pub(super) fn start(&mut self, seconds: f32, duration: f32, now: f64, frame: u64) {
        self.pause();
        self.seconds = f64::from(seconds);
        if now.is_finite() && duration.is_finite() && duration > seconds && seconds >= 0.0 {
            self.running = true;
            self.clock = Some((now, frame));
        }
    }

    // Produces at most one candidate per UI frame and 30 Hz. The owner commits
    // it only after source pins and pose/bounds sampling accept it.
    pub(super) fn advance(
        &mut self,
        now: f64,
        frame: u64,
        focused: bool,
        duration: f32,
    ) -> Option<f32> {
        if !self.running {
            return None;
        }
        let Some((last, last_frame)) = self.clock else {
            self.pause();
            return None;
        };
        let dt = now - last;
        if !focused
            || !now.is_finite()
            || !dt.is_finite()
            || !(0.0..=1.0).contains(&dt)
            || frame < last_frame
            || frame > last_frame.saturating_add(1)
            || !duration.is_finite()
            || duration <= 0.0
        {
            self.pause();
            return None;
        }
        if frame == last_frame {
            return None;
        }
        self.clock = Some((now, frame));
        self.elapsed += dt;
        if self.elapsed < SAMPLE_INTERVAL {
            return None;
        }
        Some((self.seconds + self.elapsed).min(f64::from(duration)) as f32)
    }

    pub(super) fn accept(&mut self, seconds: f32, duration: f32) {
        self.seconds = f64::from(seconds);
        self.elapsed = 0.0;
        if seconds >= duration {
            self.pause();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_cadence_duplicate_pass_and_endpoint() {
        let mut t = Transport::default();
        t.start(0.0, 1.0, 0.0, 0);
        assert_eq!(t.advance(0.01, 1, true, 1.0), None);
        assert_eq!(t.advance(0.02, 1, true, 1.0), None);
        assert_eq!(t.advance(0.02, 2, true, 1.0), None);
        let p = t.advance(0.04, 3, true, 1.0).unwrap();
        assert!((p - 0.04).abs() < 0.0001);
        t.accept(p, 1.0);
        assert_eq!(t.advance(0.04, 3, true, 1.0), None);
        assert_eq!(t.advance(1.0, 4, true, 1.0), Some(1.0));
        t.accept(1.0, 1.0);
        assert!(!t.running);
    }
    #[test]
    fn pause_restart_and_zero_duration() {
        let mut t = Transport::default();
        t.start(0.5, 1.0, 0.0, 0);
        t.pause();
        assert_eq!(t.advance(0.5, 1, true, 1.0), None);
        t.start(0.0, 1.0, 0.5, 1);
        assert_eq!(t.advance(0.75, 2, true, 1.0), Some(0.25));
        t.start(0.0, 0.0, 0.75, 2);
        assert!(!t.running);
    }
    #[test]
    fn clock_faults_and_missing_frames_pause_without_catchup() {
        for (time, frame, focus) in [
            (f64::NAN, 1, true),
            (f64::INFINITY, 1, true),
            (-1.0, 1, true),
            (1.1, 1, true),
            (0.1, 2, true),
            (0.1, 1, false),
        ] {
            let mut t = Transport::default();
            t.start(0.5, 1.0, 0.0, 0);
            assert_eq!(t.advance(time, frame, focus, 1.0), None);
            assert!(!t.running);
            assert_eq!(t.seconds, 0.5);
        }
        let mut t = Transport::default();
        t.start(0.5, 1.0, 0.0, 3);
        assert_eq!(t.advance(0.1, 2, true, 1.0), None);
        assert!(!t.running);
        for duration in [f32::NAN, f32::INFINITY, -1.0] {
            t.start(0.0, duration, 0.0, 0);
            assert!(!t.running);
        }
    }
}
