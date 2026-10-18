//! Presentation-only clock. Never advances or seeks the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Speed {
    Quarter,
    Half,
    #[default]
    Normal,
    Double,
}
impl Speed {
    pub(super) const ALL: [Self; 4] = [Self::Quarter, Self::Half, Self::Normal, Self::Double];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Quarter => "0.25×",
            Self::Half => "0.5×",
            Self::Normal => "1×",
            Self::Double => "2×",
        }
    }
    fn factor(self) -> f64 {
        match self {
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::Normal => 1.0,
            Self::Double => 2.0,
        }
    }
}
#[derive(Default)]
pub(super) struct Transport {
    pub(super) tick: u64,
    pub(super) running: bool,
    pub(super) speed: Speed,
    fraction: f64,
    clock: Option<(f64, u64)>,
}
impl Transport {
    pub(super) fn seek(&mut self, tick: u64) {
        self.tick = tick;
        self.fraction = 0.0;
        self.pause();
    }
    pub(super) fn pause(&mut self) {
        self.running = false;
        self.clock = None;
    }
    pub(super) fn play(&mut self, now: f64, frame: u64, end: u64, looped: bool) {
        if !now.is_finite() || end == 0 {
            if end == 0 {
                self.seek(0);
            }
            self.pause();
            return;
        }
        if !looped && self.tick >= end {
            self.seek(0);
        }
        self.running = true;
        self.clock = Some((now, frame));
    }
    pub(super) fn restart(&mut self, now: f64, frame: u64, end: u64, looped: bool) {
        let running = self.running;
        self.seek(0);
        if running {
            self.play(now, frame, end, looped);
        }
    }
    pub(super) fn advance(
        &mut self,
        now: f64,
        frame: u64,
        focused: bool,
        tick_rate: u32,
        end: u64,
        looped: bool,
    ) {
        if !self.running {
            return;
        }
        let Some((last, last_frame)) = self.clock else {
            self.pause();
            return;
        };
        let dt = now - last;
        // Missing inspector frames, lost focus, backward/invalid time and long
        // suspension pause at the last accepted pose; no hidden catch-up.
        if !focused
            || !now.is_finite()
            || !dt.is_finite()
            || !(0.0..=1.0).contains(&dt)
            || frame < last_frame
            || frame > last_frame.saturating_add(1)
            || tick_rate == 0
        {
            self.pause();
            return;
        }
        if frame == last_frame {
            return;
        } // egui may run more than one pass.
        let amount = self.fraction + dt * f64::from(tick_rate) * self.speed.factor();
        if !amount.is_finite() || amount < 0.0 || amount >= u64::MAX as f64 {
            self.pause();
            return;
        }
        let whole = amount.floor() as u64;
        let Some(next) = self.tick.checked_add(whole) else {
            self.pause();
            return;
        };
        if end == 0 {
            self.seek(0);
            return;
        }
        if !looped && next >= end {
            self.seek(end);
            return;
        }
        self.tick = next;
        self.fraction = amount - whole as f64;
        self.clock = Some((now, frame));
    }
}
#[cfg(test)]
#[path = "transport/tests.rs"]
mod tests;
