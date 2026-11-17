//! How a run advances: in real time, or to a chosen tick and then held
//! (for `--ticks` and screenshots).

use crate::cli::Args;

/// Most ticks run in one frame while catching up to `--ticks`, so the window stays responsive.
const MAX_STEPS_PER_FRAME: u64 = 240;
/// Longest frame time the view is told about, in seconds (a stall must not make entities jump).
const MAX_VIEW_DT: f32 = 0.1;

/// What the bridge does this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Advance {
    /// Ticks follow the clock.
    Realtime,
    /// Run this many ticks now, whatever the clock says.
    Steps(u32),
    /// The target tick is reached: no more ticks.
    Frozen,
}

/// What to do after a frame is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Continue,
    /// Save the picture and quit.
    Capture,
}

pub(crate) struct Pacer {
    target: Option<u64>,
    shot_frames: Option<u32>,
    settled_frames: u32,
}

impl Pacer {
    pub fn new(args: &Args) -> Self {
        Self { target: args.ticks, shot_frames: args.screenshot.as_ref().map(|_| args.frames), settled_frames: 0 }
    }

    pub fn advance(&self, head_tick: u64) -> Advance {
        match self.target {
            None => Advance::Realtime,
            Some(target) if head_tick < target => Advance::Steps((target - head_tick).min(MAX_STEPS_PER_FRAME) as u32),
            Some(_) => Advance::Frozen,
        }
    }

    /// The time the view moves forward this frame. With a target tick it is exactly one
    /// tick per frame, so a picture does not depend on the speed of the machine.
    pub fn view_dt(&self, real_secs: f32, tick_rate: u32) -> f32 {
        match self.target {
            Some(_) => 1.0 / tick_rate.max(1) as f32,
            None => real_secs.clamp(0.0, MAX_VIEW_DT),
        }
    }

    /// Call after each drawn frame with the tick the sim is at.
    pub fn frame_done(&mut self, head_tick: u64) -> Outcome {
        let settled = self.target.is_none_or(|t| head_tick >= t);
        if settled {
            self.settled_frames += 1;
        }
        match self.shot_frames {
            Some(n) if settled && self.settled_frames >= n => Outcome::Capture,
            _ => Outcome::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pacer(line: &str) -> Pacer {
        Pacer::new(&Args::parse(line.split_whitespace().map(String::from)).unwrap())
    }

    #[test]
    fn without_a_target_ticks_follow_the_clock() {
        let mut p = pacer("");
        assert_eq!(p.advance(0), Advance::Realtime);
        assert_eq!(p.advance(10_000), Advance::Realtime);
        for _ in 0..100 {
            assert_eq!(p.frame_done(5), Outcome::Continue);
        }
    }

    #[test]
    fn a_target_is_reached_in_capped_steps_then_held() {
        let p = pacer("--ticks 500");
        assert_eq!(p.advance(0), Advance::Steps(240));
        assert_eq!(p.advance(240), Advance::Steps(240));
        assert_eq!(p.advance(480), Advance::Steps(20));
        assert_eq!(p.advance(500), Advance::Frozen);
    }

    #[test]
    fn a_picture_waits_for_the_requested_number_of_settled_frames() {
        let mut p = pacer("--screenshot a.png --frames 3");
        assert_eq!(p.frame_done(1), Outcome::Continue);
        assert_eq!(p.frame_done(2), Outcome::Continue);
        assert_eq!(p.frame_done(3), Outcome::Capture);
    }

    #[test]
    fn frames_before_the_target_tick_do_not_count() {
        let mut p = pacer("--screenshot a.png --frames 2 --ticks 100");
        assert_eq!(p.frame_done(60), Outcome::Continue);
        assert_eq!(p.frame_done(99), Outcome::Continue);
        assert_eq!(p.frame_done(100), Outcome::Continue);
        assert_eq!(p.frame_done(100), Outcome::Capture);
    }

    #[test]
    fn zero_frames_capture_at_once() {
        let mut p = pacer("--screenshot a.png --frames 0");
        assert_eq!(p.frame_done(0), Outcome::Capture);
    }

    #[test]
    fn view_time_is_the_frame_time_capped_or_one_tick_with_a_target() {
        assert_eq!(pacer("").view_dt(0.016, 60), 0.016);
        assert_eq!(pacer("").view_dt(2.0, 60), MAX_VIEW_DT);
        assert_eq!(pacer("--ticks 10").view_dt(0.5, 50), 0.02);
    }
}
