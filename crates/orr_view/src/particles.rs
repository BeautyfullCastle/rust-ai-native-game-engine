//! Optional, bounded presentation effects. Never reads or mutates an authoritative Frame.
//! Renderer integration is separate; these are CPU draw descriptions, not rendered pixels.
use crate::Vec3;
use orr_bridge::{BridgeEvent, EventKey, EventStatus, Lifecycle, ViewResync, ViewUpdate};

pub const MAX_PARTICLES: usize = 1024;
pub const MAX_EMITTERS: usize = 16;
/// Matches the bridge's default notification bound. Older tick identities are suppressed.
pub const MAX_EVENT_HISTORY: usize = 4096;
pub const MAX_SECONDS: f32 = 10.0;
pub const MAX_RATE: f32 = 4096.0;
pub const MAX_STEP: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleBlend { Alpha, Additive }

/// All color channels are straight RGBA in 0..=1; sizes are world-space diameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmitterDesc {
    pub origin: Vec3,
    pub velocity: Vec3,
    /// Independent uniform +/- jitter on each velocity component.
    pub velocity_jitter: Vec3,
    pub burst: u32,
    pub rate: f32,
    pub duration: f32,
    pub lifetime: f32,
    pub size_start: f32,
    pub size_end: f32,
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub seed: u64,
    pub blend: ParticleBlend,
}
impl EmitterDesc {
    pub fn validate(&self) -> Result<(), ParticleError> {
        let bounded = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        if self.origin.to_array().into_iter().any(|v| !bounded(v, -1_000_000.0, 1_000_000.0))
            || self.velocity.to_array().into_iter().any(|v| !bounded(v, -10_000.0, 10_000.0))
            || self.velocity_jitter.to_array().into_iter().any(|v| !bounded(v, 0.0, 10_000.0))
            || self.burst > MAX_PARTICLES as u32
            || !bounded(self.rate, 0.0, MAX_RATE)
            || !bounded(self.duration, 0.0, MAX_SECONDS)
            || !bounded(self.lifetime, 0.001, MAX_SECONDS)
            || !bounded(self.size_start, 0.0, 1000.0)
            || !bounded(self.size_end, 0.0, 1000.0)
            || self.color_start.into_iter().chain(self.color_end).any(|v| !bounded(v, 0.0, 1.0))
        { return Err(ParticleError::InvalidEmitter); }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleError { InvalidEmitter, InvalidDelta }
impl std::fmt::Display for ParticleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self { Self::InvalidEmitter => "invalid bounded particle emitter", Self::InvalidDelta => "particle delta must be finite and in 0..=0.25 seconds" })
    }
}
impl std::error::Error for ParticleError {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    /// Emitter accepted; a full particle pool may accept zero burst particles.
    Started,
    Duplicate,
    Suppressed,
    Unmapped,
    /// Only the emitter limit; consumed event identities are still remembered.
    AtCapacity,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleDraw {
    pub key: EventKey,
    pub position: Vec3,
    pub size: f32,
    pub color: [f32; 4],
    pub blend: ParticleBlend,
}
struct Particle { key: EventKey, desc: EmitterDesc, velocity: Vec3, age: f32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct History { key: EventKey, canceled: bool }
struct Emitter { key: EventKey, desc: EmitterDesc, age: f64, emitted: u32 }

/// Allocates three bounded vectors once. Updates never grow their capacities.
/// Presentation is reproducible for the same seed and update sequence, not authoritative.
pub struct ParticlePool {
    particles: Vec<Particle>,
    emitters: Vec<Emitter>,
    history: Vec<History>,
    floor: Option<u64>,
    paused: bool,
    disconnected: bool,
}
impl Default for ParticlePool { fn default() -> Self { Self::new() } }
impl ParticlePool {
    pub fn new() -> Self {
        Self { particles: Vec::with_capacity(MAX_PARTICLES), emitters: Vec::with_capacity(MAX_EMITTERS),
            history: Vec::with_capacity(MAX_EVENT_HISTORY), floor: None, paused: false, disconnected: false }
    }
    pub fn particle_count(&self) -> usize { self.particles.len() }
    pub fn emitter_count(&self) -> usize { self.emitters.len() }
    pub fn is_paused(&self) -> bool { self.paused }
    /// Explicit new presentation session; may reuse event identities.
    pub fn reset(&mut self) {
        self.particles.clear(); self.emitters.clear(); self.history.clear();
        self.floor = None; self.paused = false; self.disconnected = false;
    }
    fn clear_effects(&mut self) {
        self.particles.clear(); self.emitters.clear(); self.history.clear(); self.floor = None;
    }
    fn remember(&mut self, key: EventKey) -> bool {
        if self.history.len() == MAX_EVENT_HISTORY {
            let oldest = self.history.iter().map(|h| h.key.tick).min().expect("full history");
            self.floor = Some(self.floor.map_or(oldest, |f| f.max(oldest)));
            self.history.retain(|h| h.key.tick > oldest);
        }
        if self.floor.is_some_and(|f| key.tick <= f) { return false; }
        self.history.push(History { key, canceled: false }); true
    }
    /// `None` consumes an unmapped identity, so toggling presets cannot replay a confirmation.
    /// Invalid descriptors fail before any history, particle or emitter mutation.
    pub fn start(&mut self, key: EventKey, desc: Option<EmitterDesc>) -> Result<StartOutcome, ParticleError> {
        self.observe(key, desc, true)
    }
    fn observe(&mut self, key: EventKey, desc: Option<EmitterDesc>, predicted: bool) -> Result<StartOutcome, ParticleError> {
        if let Some(d) = desc { d.validate()?; }
        if self.disconnected || self.floor.is_some_and(|f| key.tick <= f) { return Ok(StartOutcome::Suppressed); }
        if let Some(index) = self.history.iter().position(|h| h.key == key) {
            if !self.history[index].canceled || !predicted { return Ok(StartOutcome::Duplicate); }
            self.history.remove(index);
        }
        if !self.remember(key) { return Ok(StartOutcome::Suppressed); }
        let Some(desc) = desc else { return Ok(StartOutcome::Unmapped); };
        if self.emitters.len() == MAX_EMITTERS { return Ok(StartOutcome::AtCapacity); }
        for ordinal in 0..desc.burst.min((MAX_PARTICLES - self.particles.len()) as u32) {
            self.particles.push(particle(key, desc, ordinal));
        }
        if desc.rate > 0.0 && desc.duration > 0.0 {
            self.emitters.push(Emitter { key, desc, age: 0.0, emitted: 0 });
        }
        Ok(StartOutcome::Started)
    }
    /// Cancellation is honored even below the dedup floor. A bounded tombstone
    /// blocks late verification; only a fresh prediction (or explicit start) may reopen it.
    pub fn cancel(&mut self, key: EventKey) {
        self.particles.retain(|p| p.key != key); self.emitters.retain(|e| e.key != key);
        if let Some(entry) = self.history.iter_mut().find(|h| h.key == key) {
            entry.canceled = true;
        } else if self.remember(key) {
            self.history.last_mut().expect("remembered cancellation").canceled = true;
        }
    }
    /// Recovery discards effects and suppresses all old-tick confirmations. No lost bursts replay.
    pub fn resync(&mut self, resync: &ViewResync) {
        self.clear_effects(); self.floor = Some(resync.head_tick);
        for note in &resync.lifecycle {
            match note.last { Lifecycle::Paused { .. } => self.paused = true,
                Lifecycle::Resumed { .. } | Lifecycle::SessionStarted { .. } => self.paused = false, _ => {} }
        }
        self.disconnected = resync.disconnected;
    }
    pub fn lifecycle(&mut self, note: Lifecycle) {
        match note {
            Lifecycle::SessionStarted { .. } => self.reset(),
            Lifecycle::Seeked { .. } | Lifecycle::Branched { .. } => self.clear_effects(),
            Lifecycle::Paused { .. } => self.paused = true,
            Lifecycle::Resumed { .. } => self.paused = false,
            Lifecycle::Disconnected => { self.clear_effects(); self.disconnected = true; }
            // Rollback itself preserves effects; ordered Canceled notifications remove them.
            _ => {}
        }
    }
    pub fn event<E>(&mut self, event: &BridgeEvent<E>, mut map: impl FnMut(&E) -> Option<EmitterDesc>) -> Result<(), ParticleError> {
        match event {
            BridgeEvent::Sim { key, status: EventStatus::Canceled } => self.cancel(*key),
            BridgeEvent::Sim { key, status: EventStatus::Predicted(e) } => { self.observe(*key, map(e), true)?; }
            BridgeEvent::Sim { key, status: EventStatus::Verified(e) } => { self.observe(*key, map(e), false)?; }
            BridgeEvent::Lifecycle(note) => self.lifecycle(*note),
            BridgeEvent::ViewResynced(note) => self.resync(note),
        }
        Ok(())
    }
    /// Share the bridge's ONE poll with visual extraction. Never read a newer snapshot here.
    /// Returns the first descriptor error after processing the entire batch, so later
    /// cancellation and lifecycle notifications cannot be lost behind an invalid mapping.
    pub fn reconcile<E>(&mut self, update: &ViewUpdate<E>, mut map: impl FnMut(&E) -> Option<EmitterDesc>) -> Result<(), ParticleError> {
        if let Some(note) = &update.resync { self.resync(note); }
        let mut first_error = None;
        for event in &update.events {
            if let Err(error) = self.event(event, &mut map) { first_error.get_or_insert(error); }
        }
        first_error.map_or(Ok(()), Err)
    }
    /// Finite bounded deltas only; invalid input is atomic. Pause freezes age/emission.
    /// Rate drops at full capacity consume their ordinal: there is no catch-up debt.
    pub fn advance(&mut self, dt: f32) -> Result<(), ParticleError> {
        if !dt.is_finite() || !(0.0..=MAX_STEP).contains(&dt) { return Err(ParticleError::InvalidDelta); }
        if self.paused || self.disconnected || dt == 0.0 { return Ok(()); }
        for p in &mut self.particles { p.age += dt; }
        self.particles.retain(|p| p.age < p.desc.lifetime);
        for emitter in &mut self.emitters {
            emitter.age = (emitter.age + f64::from(dt)).min(f64::from(emitter.desc.duration));
            let total = (emitter.age * f64::from(emitter.desc.rate)).floor() as u32;
            let accepted = (total - emitter.emitted).min((MAX_PARTICLES - self.particles.len()) as u32);
            for i in 0..accepted {
                self.particles.push(particle(emitter.key, emitter.desc, emitter.desc.burst + emitter.emitted + i));
            }
            emitter.emitted = total;
        }
        self.emitters.retain(|e| e.age < f64::from(e.desc.duration));
        Ok(())
    }
    /// Allocation-free view of bounded billboard data. Ordering/blending belongs to the renderer.
    pub fn draws(&self) -> impl ExactSizeIterator<Item = ParticleDraw> + '_ {
        self.particles.iter().map(|p| {
            let t = p.age / p.desc.lifetime;
            ParticleDraw { key: p.key, position: p.desc.origin + p.velocity * p.age,
                size: p.desc.size_start + (p.desc.size_end - p.desc.size_start) * t,
                color: std::array::from_fn(|i| p.desc.color_start[i] + (p.desc.color_end[i] - p.desc.color_start[i]) * t), blend: p.desc.blend }
        })
    }
}
fn particle(key: EventKey, desc: EmitterDesc, ordinal: u32) -> Particle {
    let mut state = desc.seed ^ u64::from(ordinal).wrapping_mul(0x9e3779b97f4a7c15);
    let mut signed = || {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = state; z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        (((z ^ (z >> 31)) >> 40) as f32 / 16_777_215.0) * 2.0 - 1.0
    };
    let jitter = Vec3::new(signed() * desc.velocity_jitter.x, signed() * desc.velocity_jitter.y, signed() * desc.velocity_jitter.z);
    Particle { key, desc, velocity: desc.velocity + jitter, age: 0.0 }
}
#[cfg(test)]
#[path = "particles_tests.rs"]
mod tests;
