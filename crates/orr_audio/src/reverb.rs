//! A send/return reverb: a Freeverb-style bus (parallel damped comb filters into
//! series allpass filters, with a stereo spread and an optional pre-delay) and
//! zones that set, from the listener's position, how much of each spatial sound
//! is sent into it. All buffers are allocated when the bus is created. The
//! audio thread only reads smoothed parameters and clear requests from atomics.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;
use kira::Frame;

/// The longest pre-delay the bus supports, in seconds.
pub const MAX_PRE_DELAY: f32 = 0.1;
/// Most zones that [`EventAudio::add_reverb_zone`](crate::EventAudio::add_reverb_zone) accepts.
pub const MAX_REVERB_ZONES: usize = 64;

/// How a room sounds and how much of a sound is sent into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// 0 is a small room with a short tail, 1 a large space with a long tail.
    pub room_size: f32,
    /// 0 keeps the tail bright, 1 dulls it quickly.
    pub damping: f32,
    /// Seconds between the dry sound and the first echoes, 0..=[`MAX_PRE_DELAY`].
    pub pre_delay: f32,
    /// How much of each spatial sound is sent into the reverb, 0 (none) to 1.
    pub wet: f32,
}
impl ReverbParams {
    /// No reverb: the open air.
    pub const OUTDOOR: Self = Self { room_size: 0.5, damping: 0.5, pre_delay: 0.0, wet: 0.0 };
    pub const SMALL_ROOM: Self = Self { room_size: 0.35, damping: 0.5, pre_delay: 0.004, wet: 0.2 };
    pub const HALL: Self = Self { room_size: 0.8, damping: 0.35, pre_delay: 0.025, wet: 0.35 };
    pub const CAVE: Self = Self { room_size: 0.92, damping: 0.2, pre_delay: 0.04, wet: 0.5 };

    pub(crate) fn is_valid(&self) -> bool {
        [self.room_size, self.damping, self.pre_delay, self.wet].iter().all(|v| v.is_finite())
    }
    pub(crate) fn clamped(self) -> Self {
        Self {
            room_size: self.room_size.clamp(0.0, 1.0),
            damping: self.damping.clamp(0.0, 1.0),
            pre_delay: self.pre_delay.clamp(0.0, MAX_PRE_DELAY),
            wet: self.wet.clamp(0.0, 1.0),
        }
    }
}

/// The volume that a zone covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReverbShape {
    Sphere { radius: f32 },
    /// An axis-aligned box, from the center out by `half_extents` on each axis.
    Box { half_extents: [f32; 3] },
}

/// A place where sounds are heard with reverb. Its parameters apply fully while
/// the listener is inside the shape and fade out over `fade` meters beyond it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbZone {
    pub center: [f32; 3],
    pub shape: ReverbShape,
    pub params: ReverbParams,
    /// Meters outside the shape over which the zone's weight falls from 1 to 0.
    pub fade: f32,
}
impl ReverbZone {
    pub fn sphere(center: [f32; 3], radius: f32, params: ReverbParams) -> Self {
        Self { center, shape: ReverbShape::Sphere { radius }, params, fade: 2.0 }
    }
    pub fn cuboid(center: [f32; 3], half_extents: [f32; 3], params: ReverbParams) -> Self {
        Self { center, shape: ReverbShape::Box { half_extents }, params, fade: 2.0 }
    }
    /// This zone with its edge fading over `meters` instead of the default 2.
    pub fn fade(mut self, meters: f32) -> Self {
        self.fade = meters;
        self
    }

    pub(crate) fn is_valid(&self) -> bool {
        let shape = match self.shape {
            ReverbShape::Sphere { radius } => radius.is_finite() && radius > 0.0,
            ReverbShape::Box { half_extents } => half_extents.iter().all(|v| v.is_finite() && *v > 0.0),
        };
        shape && self.center.iter().all(|v| v.is_finite()) && self.fade.is_finite() && self.fade >= 0.0 && self.params.is_valid()
    }

    /// How strongly this zone applies at `position`: 1 inside, 0 beyond `fade`
    /// meters outside, and linear in between.
    pub fn weight(&self, position: [f32; 3]) -> f32 {
        let offset: [f32; 3] = std::array::from_fn(|i| position[i] - self.center[i]);
        let outside = match self.shape {
            ReverbShape::Sphere { radius } => (offset.iter().map(|v| v * v).sum::<f32>().sqrt() - radius).max(0.0),
            ReverbShape::Box { half_extents } => {
                offset.iter().zip(&half_extents).map(|(o, h)| (o.abs() - h).max(0.0).powi(2)).sum::<f32>().sqrt()
            }
        };
        if outside <= 0.0 {
            1.0
        } else if self.fade <= 0.0 {
            0.0
        } else {
            (1.0 - outside / self.fade).clamp(0.0, 1.0)
        }
    }
}

/// Handle of an added zone, for removing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReverbZoneId(pub(crate) u32);

/// Parameters at `position`: the weighted mean of the zones there. The ambient
/// parameters take the share that the zones leave to 1 (all of it outside every zone).
pub(crate) fn blend(zones: &[(ReverbZoneId, ReverbZone)], ambient: ReverbParams, position: [f32; 3]) -> ReverbParams {
    let mut total = 0.0;
    let mut sum = [0.0f32; 4];
    for (_, zone) in zones {
        let w = zone.weight(position);
        if w > 0.0 {
            total += w;
            let p = zone.params;
            for (s, v) in sum.iter_mut().zip([p.room_size, p.damping, p.pre_delay, p.wet]) {
                *s += w * v;
            }
        }
    }
    if total < 1.0 {
        let rest = 1.0 - total;
        for (s, v) in sum.iter_mut().zip([ambient.room_size, ambient.damping, ambient.pre_delay, ambient.wet]) {
            *s += rest * v;
        }
        total = 1.0;
    }
    ReverbParams { room_size: sum[0] / total, damping: sum[1] / total, pre_delay: sum[2] / total, wet: sum[3] / total }.clamped()
}

/// The bus parameters that the control thread sets and the audio thread reads.
pub(crate) struct Shared {
    room_size: AtomicU32,
    damping: AtomicU32,
    pre_delay: AtomicU32,
    /// Counts the requests to silence the bus. The audio thread acts when it differs from its last seen value.
    clears: AtomicU32,
    /// Seconds over which the bus fades out before it is cleared.
    clear_fade: AtomicU32,
}
impl Shared {
    pub(crate) fn new(params: ReverbParams) -> Arc<Self> {
        let shared = Arc::new(Self {
            room_size: AtomicU32::new(0),
            damping: AtomicU32::new(0),
            pre_delay: AtomicU32::new(0),
            clears: AtomicU32::new(0),
            clear_fade: AtomicU32::new(0),
        });
        shared.store(params);
        shared
    }
    pub(crate) fn store(&self, params: ReverbParams) {
        self.room_size.store(params.room_size.to_bits(), Ordering::Relaxed);
        self.damping.store(params.damping.to_bits(), Ordering::Relaxed);
        self.pre_delay.store(params.pre_delay.to_bits(), Ordering::Relaxed);
    }
    /// Asks the audio thread to fade the bus out over `fade` seconds, then empty every filter
    /// buffer. The tail of the sounds that played before is gone. Sounds that start later are heard.
    pub(crate) fn request_clear(&self, fade: f32) {
        self.clear_fade.store(fade.max(0.0).to_bits(), Ordering::Relaxed);
        self.clears.fetch_add(1, Ordering::Release);
    }
    fn load(&self) -> (f32, f32, f32) {
        let get = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        (get(&self.room_size), get(&self.damping), get(&self.pre_delay))
    }
}

pub(crate) struct ReverbBuilder(pub(crate) Arc<Shared>);
impl EffectBuilder for ReverbBuilder {
    type Handle = ();
    fn build(self) -> (Box<dyn Effect>, ()) {
        (Box::new(Freeverb::new(self.0)), ())
    }
}

// Freeverb tunings (samples at 44.1 kHz) and constants.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const INPUT_GAIN: f32 = 0.015;
const WET_GAIN: f32 = 1.0;
const ALLPASS_FEEDBACK: f32 = 0.5;
/// Parameter changes glide with this time constant, in seconds.
const SMOOTHING: f32 = 0.03;

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    store: f32,
}
impl Comb {
    fn new(len: usize) -> Self {
        Self { buffer: vec![0.0; len.max(1)], index: 0, store: 0.0 }
    }
    fn process(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let out = self.buffer[self.index];
        let store = out * (1.0 - damping) + self.store * damping;
        // Flush values near the denormal range.
        self.store = if store.abs() < 1e-20 { 0.0 } else { store };
        self.buffer[self.index] = input + self.store * feedback;
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
        out
    }
    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.store = 0.0;
    }
}

struct Allpass {
    buffer: Vec<f32>,
    index: usize,
}
impl Allpass {
    fn new(len: usize) -> Self {
        Self { buffer: vec![0.0; len.max(1)], index: 0 }
    }
    fn process(&mut self, input: f32) -> f32 {
        let held = self.buffer[self.index];
        self.buffer[self.index] = input + held * ALLPASS_FEEDBACK;
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
        held - input
    }
    fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}

struct Channel {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}
impl Channel {
    fn new(scale: f64, spread: usize) -> Self {
        let size = |n: usize| ((n + spread) as f64 * scale) as usize;
        Self {
            combs: COMB_TUNING.iter().map(|&n| Comb::new(size(n))).collect(),
            allpasses: ALLPASS_TUNING.iter().map(|&n| Allpass::new(size(n))).collect(),
        }
    }
    fn process(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let mut out = 0.0;
        for comb in &mut self.combs {
            out += comb.process(input, feedback, damping);
        }
        for allpass in &mut self.allpasses {
            out = allpass.process(out);
        }
        out
    }
    fn clear(&mut self) {
        self.combs.iter_mut().for_each(Comb::clear);
        self.allpasses.iter_mut().for_each(Allpass::clear);
    }
}

/// The wet-only reverb effect of the send track.
struct Freeverb {
    shared: Arc<Shared>,
    channels: Option<[Channel; 2]>,
    pre_buffer: Vec<f32>,
    pre_index: usize,
    sample_rate: f32,
    smoothing: f32,
    feedback: f32,
    damping: f32,
    delay: f32,
    /// The value of `Shared::clears` that was already handled.
    seen_clears: u32,
    /// Length of the fade-out in progress and the samples left of it. Zero left: no fade.
    fade_total: usize,
    fade_left: usize,
    /// Level the fade in progress starts from: 1 for a fade that began on a free bus.
    fade_start: f32,
}
impl Freeverb {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            channels: None,
            pre_buffer: Vec::new(),
            pre_index: 0,
            sample_rate: 44_100.0,
            smoothing: 0.0,
            feedback: 0.0,
            damping: 0.0,
            delay: 0.0,
            seen_clears: 0,
            fade_total: 0,
            fade_left: 0,
            fade_start: 1.0,
        }
    }
    /// Feedback, damping and pre-delay in samples that the shared parameters ask for.
    fn targets(&self) -> (f32, f32, f32) {
        let (room, damping, pre_delay) = self.shared.load();
        (0.7 + 0.28 * room.clamp(0.0, 1.0), 0.4 * damping.clamp(0.0, 1.0), pre_delay.clamp(0.0, MAX_PRE_DELAY) * self.sample_rate)
    }
    fn setup(&mut self, sample_rate: u32) {
        let scale = f64::from(sample_rate) / 44_100.0;
        self.channels = Some([Channel::new(scale, 0), Channel::new(scale, STEREO_SPREAD)]);
        self.sample_rate = sample_rate as f32;
        self.pre_buffer = vec![0.0; (MAX_PRE_DELAY * self.sample_rate) as usize + 2];
        self.pre_index = 0;
        self.smoothing = 1.0 - (-1.0 / (SMOOTHING * self.sample_rate)).exp();
        (self.feedback, self.damping, self.delay) = self.targets();
    }
}
impl Effect for Freeverb {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.setup(sample_rate);
    }
    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.setup(sample_rate);
    }
    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        let (target_feedback, target_damping, target_delay) = self.targets();
        let clears = self.shared.clears.load(Ordering::Acquire);
        if clears != self.seen_clears {
            self.seen_clears = clears;
            // A newer request restarts the fade from the level it has now, because the sounds
            // that it stops keep feeding the bus until their own fade ends.
            let fade = f32::from_bits(self.shared.clear_fade.load(Ordering::Relaxed));
            self.fade_start = if self.fade_left > 0 { self.fade_start * self.fade_left as f32 / self.fade_total as f32 } else { 1.0 };
            self.fade_total = ((fade * self.sample_rate) as usize).max(1);
            self.fade_left = self.fade_total;
        }
        let Some([left, right]) = &mut self.channels else { return };
        let len = self.pre_buffer.len();
        for frame in input.iter_mut() {
            self.feedback += (target_feedback - self.feedback) * self.smoothing;
            self.damping += (target_damping - self.damping) * self.smoothing;
            self.delay += (target_delay - self.delay) * self.smoothing;
            self.pre_buffer[self.pre_index] = (frame.left + frame.right) * INPUT_GAIN;
            let read = self.pre_index as f32 - self.delay;
            let read = if read < 0.0 { read + len as f32 } else { read };
            let whole = read as usize;
            let frac = read - whole as f32;
            let a = self.pre_buffer[whole % len];
            let b = self.pre_buffer[(whole + 1) % len];
            let delayed = a + (b - a) * frac;
            self.pre_index = (self.pre_index + 1) % len;
            let mut wet = Frame::new(
                left.process(delayed, self.feedback, self.damping) * WET_GAIN,
                right.process(delayed, self.feedback, self.damping) * WET_GAIN,
            );
            if self.fade_left > 0 {
                let gain = self.fade_start * self.fade_left as f32 / self.fade_total as f32;
                wet = Frame::new(wet.left * gain, wet.right * gain);
                self.fade_left -= 1;
                if self.fade_left == 0 {
                    left.clear();
                    right.clear();
                    self.pre_buffer.fill(0.0);
                }
            }
            *frame = wet;
        }
    }
}
