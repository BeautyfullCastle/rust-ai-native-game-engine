//! Integer/fixed-point sample-local controller. X then Z are separate physical
//! point paths, not a diagonal sweep. Each is shorter than the smallest cell.
use crate::Course;
use orr_fp::FP;
use std::time::Duration;

pub const CARDINAL_STEP_RAW: i64 = 4096;
pub const DIAGONAL_STEP_RAW: i64 = 2896;
pub const TICKS_PER_SECOND: u32 = 60;
pub const MAX_CATCH_UP_TICKS: u32 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub x: i8,
    pub z: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub anchor: [FP; 3],
    pub collected: u8,
    pub finished: bool,
    pub tick: u64,
}

pub struct Game {
    course: Course,
    state: State,
}
impl Game {
    pub fn new(course: Course) -> Self {
        let state = State {
            anchor: course.spawn(),
            collected: 0,
            finished: false,
            tick: 0,
        };
        Self { course, state }
    }
    pub fn course(&self) -> &Course {
        &self.course
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn restart(&mut self) {
        self.state = State {
            anchor: self.course.spawn(),
            collected: 0,
            finished: false,
            tick: 0,
        };
    }
    pub fn tick(&mut self, input: Input) {
        if self.state.finished {
            return;
        }
        self.state.tick = self.state.tick.saturating_add(1);
        let x = i64::from(input.x.signum());
        let z = i64::from(input.z.signum());
        let step = if x != 0 && z != 0 {
            DIAGONAL_STEP_RAW
        } else {
            CARDINAL_STEP_RAW
        };
        // Bounds in Course make these additions safe, including rejected moves.
        for (axis, direction) in [(0, x), (2, z)] {
            if direction == 0 {
                continue;
            }
            let mut next = self.state.anchor;
            next[axis] = FP::from_raw(next[axis].raw() + direction * step);
            if let Some(y) = self.course.terrain().sample(next[0], next[2]) {
                next[1] = y;
                self.state.anchor = next;
            }
        }
        // XZ tick-sampled distance; at most one ordered beacon per logical tick.
        // The finish is only considered on a subsequent tick after beacon four.
        if self.state.collected < 4 {
            if self.near(self.course.beacons()[usize::from(self.state.collected)]) {
                self.state.collected += 1;
            }
        } else if self.near(self.course.finish()) {
            self.state.finished = true;
        }
    }
    fn near(&self, point: [FP; 3]) -> bool {
        let dx = i128::from(self.state.anchor[0].raw()) - i128::from(point[0].raw());
        let dz = i128::from(self.state.anchor[2].raw()) - i128::from(point[2].raw());
        let radius = i128::from(self.course.radius().raw());
        dx * dx + dz * dz <= radius * radius
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ControlKey {
    W,
    A,
    S,
    D,
    Up,
    Left,
    Down,
    Right,
}

#[derive(Clone, Debug, Default)]
pub struct Controls {
    keys: [bool; 8],
}
impl Controls {
    pub fn set(&mut self, key: ControlKey, down: bool) {
        self.keys[key as usize] = down;
    }
    pub fn clear(&mut self) {
        self.keys = [false; 8];
    }
    pub fn input(&self) -> Input {
        let held =
            |a: ControlKey, b: ControlKey| i8::from(self.keys[a as usize] || self.keys[b as usize]);
        Input {
            x: held(ControlKey::D, ControlKey::Right) - held(ControlKey::A, ControlKey::Left),
            z: held(ControlKey::S, ControlKey::Down) - held(ControlKey::W, ControlKey::Up),
        }
    }
}

/// Rational 60 Hz scheduling without rounding a tick Duration. Admit at most
/// 250 ms per frame, execute at most eight ticks, and discard excess whole ticks
/// while retaining the fractional remainder. This slows under stalls, not jumps.
#[derive(Default)]
pub struct FixedClock {
    phase: u128,
}
impl FixedClock {
    pub fn reset(&mut self) {
        self.phase = 0;
    }
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.phase +=
            elapsed.min(Duration::from_millis(250)).as_nanos() * u128::from(TICKS_PER_SECOND);
        let ticks = (self.phase / 1_000_000_000).min(u128::from(MAX_CATCH_UP_TICKS)) as u32;
        self.phase %= 1_000_000_000;
        ticks
    }
}
