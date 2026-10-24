// Read-only presentation. This module has no simulation, command, or WASM access.
const clamp = (n, a, b) => Math.max(a, Math.min(b, n));
const finite = (n, fallback = 0) => Number.isFinite(n) ? n : fallback;
const hash = value => { let h = 2166136261; for (const c of String(value)) h = Math.imul(h ^ c.charCodeAt(0), 16777619); return h >>> 0; };
const defaultColor = '#88dbed';
export const DEFAULT_BUDGETS = Object.freeze({ actors: 384, projectiles: 128, particles: 256, ghosts: 48, trailPoints: 8, eventIds: 1024 });
export const ATTACK_TIMING = Object.freeze({ windupMs: 110, releaseMs: 70, recoveryMs: 160 });

// The public frame contract is deliberately independent of species, role, board,
// economy and progression. A clip is actual rectangles, never generated duplicates.
export function sampleClip(definition, name, elapsedMs) {
  const clip = definition?.clips?.[name];
  if (!clip || !Array.isArray(clip.frames) || clip.frames.length === 0) return null;
  const fps = clamp(finite(clip.fps, 10), 1, 120);
  const raw = Math.floor(Math.max(0, elapsedMs) * fps / 1000);
  const loop = clip.loop ?? (name === 'idle' || name === 'move');
  const index = loop ? raw % clip.frames.length : Math.min(raw, clip.frames.length - 1);
  const rect = clip.frames[index];
  if (!Array.isArray(rect) || rect.length !== 4 || !rect.every(Number.isFinite) || rect[0] < 0 || rect[1] < 0 || rect[2] <= 0 || rect[3] <= 0) return null;
  const pivot = clip.pivots?.[index] || definition.pivot;
  const logicalSize = definition.logicalSize;
  return { image: definition.image, rect: [...rect], anchor: [...(definition.anchor || [.5, .86])], pivot: Array.isArray(pivot) && pivot.length === 2 && pivot.every(Number.isFinite) ? [...pivot] : null, logicalSize: Array.isArray(logicalSize) && logicalSize.length === 2 && logicalSize.every(n => Number.isFinite(n) && n > 0) ? [...logicalSize] : null, clip: name, index, frameCount: clip.frames.length, mode: clip.frames.length > 1 ? 'sprite-frame' : 'single-frame' };
}

export class CombatAudio {
  constructor({ contextFactory, maxVoices = 12 } = {}) {
    this.factory = contextFactory || (() => { const C = globalThis.AudioContext || globalThis.webkitAudioContext; return C ? new C() : null; });
    this.maxVoices = clamp(Math.floor(finite(maxVoices, 12)), 0, 32);
    this.context = null;
    this.master = null;
    this.voices = new Set();
    this.muted = true;
    this.paused = false;
    this.hidden = false;
    this.unlocked = false;
    this.disposed = false;
    this.played = 0;
    this.dropped = 0;
    this.lastCue = new Map();
    this.error = null;
    this.unlockPromise = null;
    this.syncPromise = null;
    this.syncVersion = 0;
  }
  get enabled() { return this.unlocked && !this.muted && !this.paused && !this.hidden && !this.disposed; }
  async unlock(event) {
    // Called synchronously inside the actual input handler. Programmatic clicks
    // and unsolicited timers cannot create/resume a browser audio context.
    if (this.disposed || !event?.isTrusted || !['click', 'pointerdown', 'keydown', 'touchend'].includes(event.type)) return false;
    if (this.unlockPromise) return this.unlockPromise;
    try {
      if (!this.context) {
        this.context = this.factory();
        if (!this.context) { this.error = 'Web Audio unavailable'; return false; }
        this.master = this.context.createGain();
        this.master.gain.value = 0;
        this.master.connect(this.context.destination);
      }
      this.unlocked = true;
      this.unlockPromise = this.sync().then(() => this.enabled && this.context.state === 'running').finally(() => { this.unlockPromise = null; });
      return await this.unlockPromise;
    } catch (error) { this.error = String(error?.message || error); this.unlocked = false; return false; }
  }
  sync() {
    if (!this.context || this.disposed) return Promise.resolve();
    this.syncVersion++;
    this.master.gain.value = this.enabled ? .16 : 0;
    if (!this.enabled) this.stopVoices();
    if (this.syncPromise) return this.syncPromise;
    // Serialize async context transitions. Rapid mute/unmute or pause/resume
    // must converge to the latest state, even if a browser operation is pending.
    let settledVersion = this.syncVersion;
    this.syncPromise = (async () => {
      let observed;
      do {
        observed = this.syncVersion;
        settledVersion = observed;
        if (this.disposed) break;
        try {
          if (this.enabled && this.context.state !== 'running') await this.context.resume();
          else if (!this.enabled && this.context.state === 'running') await this.context.suspend();
        } catch (error) { this.error = String(error?.message || error); break; }
        if (!this.enabled) { this.master.gain.value = 0; this.stopVoices(); }
      } while (observed !== this.syncVersion);
    })().finally(() => {
      this.syncPromise = null;
      // A no-op reconciliation can finish synchronously, then receive another
      // state change before this microtask. Reconcile that last change as well.
      if (!this.disposed && settledVersion !== this.syncVersion) return this.sync();
    });
    return this.syncPromise;
  }
  setMuted(value) { this.muted = !!value; return this.sync(); }
  setPaused(value) { this.paused = !!value; return this.sync(); }
  setHidden(value) { this.hidden = !!value; return this.sync(); }
  stopVoices() {
    for (const voice of [...this.voices]) { try { voice.osc.stop(); } catch {} voice.cleanup(); }
  }
  cue(kind, variant = 0) {
    if (!this.enabled || this.context?.state !== 'running') return false;
    const now = this.context.currentTime;
    const key = ['attack', 'hit', 'death', 'ui'].includes(kind) ? kind : 'hit';
    if (this.voices.size >= this.maxVoices || now - (this.lastCue.get(key) ?? -Infinity) < (key === 'attack' ? .055 : .04)) { this.dropped++; return false; }
    this.lastCue.set(key, now);
    let osc, gain, voice;
    try {
      const note = hash(variant) % 5;
      const sounds = { attack: [480 + note * 65, 180 + note * 28, .085, 'triangle'], hit: [190, 55, .09, 'triangle'], death: [140, 30, .22, 'sine'], ui: [540, 780, .13, 'sine'] };
      const [from, to, length, type] = sounds[key];
      osc = this.context.createOscillator(); gain = this.context.createGain();
      osc.type = type;
      osc.frequency.setValueAtTime(from, now);
      osc.frequency.exponentialRampToValueAtTime(to, now + length);
      gain.gain.setValueAtTime(.001, now);
      gain.gain.linearRampToValueAtTime(key === 'hit' ? .28 : .19, now + .008);
      gain.gain.exponentialRampToValueAtTime(.001, now + length);
      osc.connect(gain); gain.connect(this.master);
      let cleaned = false;
      voice = { osc, gain, cleanup: () => { if (cleaned) return; cleaned = true; try { osc.disconnect(); gain.disconnect(); } catch {} this.voices.delete(voice); } };
      osc.onended = voice.cleanup;
      this.voices.add(voice);
      osc.start(now); osc.stop(now + length + .01);
      this.played++;
      return true;
    } catch (error) { voice?.cleanup(); try { osc?.disconnect(); gain?.disconnect(); } catch {} this.error = String(error?.message || error); return false; }
  }
  reset() { this.stopVoices(); this.lastCue.clear(); }
  dispose() {
    if (this.disposed) return;
    this.disposed = true; this.reset();
    try { this.master?.disconnect(); const promise = this.context?.close(); promise?.catch?.(() => {}); } catch {}
  }
  stats() { return { unlocked: this.unlocked, muted: this.muted, paused: this.paused, hidden: this.hidden, contextState: this.context?.state || 'not-created', voices: this.voices.size, played: this.played, dropped: this.dropped, error: this.error }; }
}

export class CombatPresentation {
  constructor({ budgets = {}, audio = new CombatAudio(), manifest = {} } = {}) {
    this.budgets = Object.fromEntries(Object.entries(DEFAULT_BUDGETS).map(([key, value]) => [key, clamp(Math.floor(finite(budgets[key], value)), 0, value)]));
    this.audio = audio;
    this.manifest = manifest;
    this.paused = false; this.hidden = false; this.disposed = false;
    this.reset();
  }
  setManifest(manifest) { this.manifest = manifest || {}; }
  reset() {
    this.time = 0; this.lastTick = null;
    this.actors = new Map(); this.projectiles = new Map(); this.particles = []; this.ghosts = [];
    this.seen = new Set(); this.seenOrder = [];
    this.counts = { attacks: 0, hits: 0, deaths: 0, removals: 0, moves: 0, merges: 0, spawns: 0, retiredProjectiles: 0, droppedParticles: 0, droppedProjectiles: 0 };
    this.audio.reset();
  }
  setPaused(value) { this.paused = !!value; void this.audio.setPaused(value); }
  setHidden(value) { this.hidden = !!value; void this.audio.setHidden(value); }
  observe(observation) {
    if (this.disposed || !observation) return;
    if (this.lastTick !== null && observation.tick < this.lastTick) this.reset();
    if (observation.tick === this.lastTick) return;
    this.lastTick = observation.tick;
    const next = new Map();
    for (const input of (observation.actors || []).slice(0, this.budgets.actors)) {
      if (!input || input.id == null || !Number.isFinite(input.x) || !Number.isFinite(input.y)) continue;
      const id = String(input.id), previous = this.actors.get(id);
      // Own all retained data: frozen snapshots and later host mutation are safe.
      const actor = { id, x: input.x, y: input.y, scale: finite(input.scale, 1), color: input.color || defaultColor, visualKey: String(input.visualKey || ''), moving: !!input.moving, renderData: Array.isArray(input.renderData) ? [...input.renderData] : null };
      next.set(id, { actor, seed: previous?.seed ?? hash(id), attack: previous?.attack || null, hitAt: previous?.hitAt ?? -Infinity, bornAt: previous?.bornAt ?? this.time, move: previous?.move || null, spawnAt: previous?.spawnAt ?? -Infinity });
    }
    // Events are processed while both current and previous actor poses exist.
    for (const event of observation.events || []) this.event(event, next);
    this.actors = next;
    const projectiles = new Map();
    for (const p of (observation.projectiles || []).slice(0, this.budgets.projectiles)) {
      if (!p || p.id == null || !Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
      const id = String(p.id), previous = this.projectiles.get(id);
      const trail = previous ? previous.trail.map(v => ({ ...v })) : [];
      const tail = trail.at(-1);
      if (!tail || Math.hypot(p.x - tail.x, p.y - tail.y) > 1) trail.push({ x: p.x, y: p.y });
      while (trail.length > this.budgets.trailPoints) trail.shift();
      const dx = p.x - (previous?.x ?? p.x - 1), dy = p.y - (previous?.y ?? p.y);
      projectiles.set(id, { id, x: p.x, y: p.y, color: p.color || defaultColor, size: clamp(finite(p.size, 3), 1, 8), angle: dx || dy ? Math.atan2(dy, dx) : previous?.angle || 0, trail });
    }
    this.counts.droppedProjectiles += Math.max(0, (observation.projectiles?.length || 0) - this.budgets.projectiles);
    this.projectiles = projectiles;
    // Missing projectiles are simply retired, never assumed to have hit.
  }
  event(event, current = this.actors) {
    if (this.disposed || !event || event.id == null || this.seen.has(String(event.id))) return;
    const id = String(event.id);
    this.seen.add(id); this.seenOrder.push(id);
    while (this.seenOrder.length > this.budgets.eventIds) this.seen.delete(this.seenOrder.shift());
    const actorId = String(event.actorId);
    const fallback = event.actor && { actor: { ...event.actor, renderData: event.actor.renderData ? [...event.actor.renderData] : null }, seed: hash(actorId), attack: null, hitAt: -Infinity, bornAt: this.time };
    const record = current.get(actorId) || this.actors.get(actorId) || fallback;
    const actor = record?.actor;
    const x = finite(event.x, actor?.x || 0), y = finite(event.y, actor?.y || 0), color = event.color || actor?.color || defaultColor;
    if (event.type === 'attack' && record) {
      const clip = this.manifest[actor.visualKey]?.clips?.attack || {};
      const timing = Object.fromEntries(Object.entries(ATTACK_TIMING).map(([key, value]) => [key, clamp(finite(clip[key], value), 1, 2000)]));
      const elapsed = event.phase === 'release' ? timing.windupMs : Math.max(0, finite(event.elapsedMs));
      record.attack = { since: this.time - elapsed, timing, dx: finite(event.dx, 1), dy: finite(event.dy), released: false };
      this.counts.attacks++;
      this.release(record);
    } else if (event.type === 'hit' && record) {
      record.hitAt = this.time;
      this.burst(x, y, color, 9, 'impact'); this.audio.cue('hit'); this.counts.hits++;
    } else if ((event.type === 'death' || event.type === 'remove') && record) {
      const death = event.type === 'death';
      this.retire(record, death, x, y);
      if (death) { this.burst(x, y, color, 14, 'death'); this.audio.cue('death'); this.counts.deaths++; }
      else this.counts.removals++;
    } else if (event.type === 'projectile_retire') {
      this.projectiles.delete(String(event.projectileId)); this.counts.retiredProjectiles++;
    } else if (event.type === 'move' && record) {
      const before = this.actors.get(actorId)?.actor;
      if (before) record.move = { since: this.time, duration: 150, x: before.x, y: before.y };
      this.counts.moves++;
    } else if (event.type === 'merge' && record) {
      const consumed = this.actors.get(String(event.otherActorId));
      if (consumed) this.retire(consumed, false, consumed.actor.x, consumed.actor.y);
      record.spawnAt = this.time;
      this.burst(x, y, color, 12, 'impact'); this.audio.cue('ui'); this.counts.merges++;
    } else if (event.type === 'spawn' && record) {
      record.spawnAt = this.time; this.burst(x, y, color, 4, 'muzzle'); this.counts.spawns++;
    }
  }
  retire(record, death, x, y) {
    const actor = record.actor, duration = death ? this.clipDuration(actor.visualKey, 'death', 520) : 200;
    if (this.ghosts.length >= this.budgets.ghosts) this.ghosts.shift();
    if (this.budgets.ghosts) this.ghosts.push({ record: { ...record, actor: { ...actor, x, y, renderData: actor.renderData ? [...actor.renderData] : null } }, since: this.time, duration, death });
  }
  clipDuration(key, clip, fallback) {
    const value = this.manifest[key]?.clips?.[clip];
    return value?.frames?.length > 1 ? clamp(value.frames.length * 1000 / clamp(finite(value.fps, 10), 1, 120), 100, 2000) : fallback;
  }
  release(record) {
    const attack = record.attack;
    if (attack && !attack.released && this.time - attack.since >= attack.timing.windupMs) {
      attack.released = true;
      this.burst(record.actor.x, record.actor.y - 14 * record.actor.scale, record.actor.color, 5, 'muzzle');
      this.audio.cue('attack', record.actor.visualKey);
    }
  }
  burst(x, y, color, count, kind) {
    const available = Math.max(0, this.budgets.particles - this.particles.length), take = Math.min(count, available);
    this.counts.droppedParticles += count - take;
    for (let i = 0; i < take; i++) {
      const angle = i * 2.3999632297 + this.time * .003;
      const speed = (kind === 'death' ? 42 : 72) + (i % 4) * 19;
      const duration = kind === 'death' ? 420 + i % 3 * 70 : 220 + i % 4 * 45;
      this.particles.push({ x, y, vx: Math.cos(angle) * speed, vy: Math.sin(angle) * speed - (kind === 'death' ? 30 : 0), age: 0, duration, color, size: kind === 'death' ? 2.8 : 1.6, kind: i === 0 && kind === 'impact' ? 'ring' : kind });
    }
  }
  advance(deltaMs) {
    if (this.disposed || this.paused || this.hidden) return;
    const dt = clamp(finite(deltaMs), 0, 100);
    this.time += dt;
    for (const record of this.actors.values()) this.release(record);
    for (const p of this.particles) { p.age += dt; p.x += p.vx * dt / 1000; p.y += p.vy * dt / 1000; if (p.kind === 'death') p.vy += 80 * dt / 1000; }
    this.particles = this.particles.filter(p => p.age < p.duration);
    this.ghosts = this.ghosts.filter(g => this.time - g.since < g.duration);
  }
  pose(id, ghost = null) {
    const record = ghost?.record || this.actors.get(String(id));
    const neutral = { x: 0, y: 0, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flash: 0, clip: 'idle', phase: 'idle', clipTime: 0, mode: 'procedural-single-sprite' };
    if (!record) return neutral;
    const age = this.time - record.bornAt, phase = this.time * .0036 + record.seed % 61;
    const moving = record.actor.moving;
    const pose = { ...neutral, y: moving ? -Math.abs(Math.sin(phase * 2.5)) * 2.8 : Math.sin(phase) * 1.4, scaleX: 1 + Math.sin(phase) * .022, scaleY: 1 - Math.sin(phase) * .025, rotation: moving ? Math.sin(phase * 2.5) * .065 : Math.sin(phase * .65) * .018, clip: moving ? 'move' : 'idle', phase: moving ? 'move' : 'idle', clipTime: age };
    const attack = record.attack;
    if (attack) {
      const t = this.time - attack.since, { windupMs: w, releaseMs: r, recoveryMs: k } = attack.timing;
      if (t < w + r + k) {
        const length = Math.hypot(attack.dx, attack.dy) || 1, dx = attack.dx / length, dy = attack.dy / length;
        const recoil = t < w ? -3 * Math.sin(t / w * Math.PI / 2) : t < w + r ? 5 * (1 - (t - w) / r) : -1.5 * Math.sin((t - w - r) / k * Math.PI);
        pose.x += dx * recoil; pose.y += dy * recoil;
        pose.rotation += dx * recoil * .022;
        pose.scaleX += t < w ? -.065 * (t / w) : .07 * Math.max(0, 1 - (t - w) / r);
        pose.scaleY += t < w ? .045 * (t / w) : -.06 * Math.max(0, 1 - (t - w) / r);
        pose.clip = 'attack'; pose.clipTime = t; pose.phase = t < w ? 'windup' : t < w + r ? 'release' : 'recovery';
      }
    }
    const hitAge = this.time - record.hitAt, hitDuration = this.clipDuration(record.actor.visualKey, 'hit', 180);
    if (hitAge < hitDuration) {
      const strength = 1 - hitAge / hitDuration;
      pose.x += Math.sin(hitAge * .12) * 2 * strength; pose.scaleX += .12 * strength; pose.scaleY -= .13 * strength; pose.flash = strength;
      pose.clip = 'hit'; pose.clipTime = hitAge; pose.phase = 'hit';
    }
    if (ghost) {
      const t = clamp((this.time - ghost.since) / ghost.duration, 0, 1);
      pose.clip = ghost.death ? 'death' : 'idle'; pose.phase = ghost.death ? 'death' : 'retire'; pose.clipTime = this.time - ghost.since;
      pose.opacity = 1 - t; pose.y += ghost.death ? -14 * Math.sin(t * Math.PI / 2) : 0;
      pose.rotation += ghost.death ? t * .28 : 0; pose.scaleX *= 1 - t * .22; pose.scaleY *= 1 - t * .28;
    }
    const frame = this.frameFor(record.actor, pose);
    pose.mode = frame?.mode === 'sprite-frame' ? 'sprite-frame' : 'procedural-single-sprite';
    // Authored frames own their motion. Do not double-animate feet/pivots with
    // procedural wobble or squash; retain only lifecycle opacity.
    if (frame?.mode === 'sprite-frame' && frame.clip === pose.clip) {
      pose.x = 0; pose.y = 0; pose.scaleX = 1; pose.scaleY = 1; pose.rotation = 0; pose.flash = 0;
    }
    if (record.move && this.time - record.move.since < record.move.duration) {
      const remaining = 1 - clamp((this.time - record.move.since) / record.move.duration, 0, 1), factor = record.actor.scale || 1;
      pose.x += (record.move.x - record.actor.x) / factor * remaining * remaining;
      pose.y += (record.move.y - record.actor.y) / factor * remaining * remaining;
    }
    const spawnAge = this.time - (record.spawnAt ?? -Infinity);
    if (spawnAge < 180) { const progress = clamp(spawnAge / 180, 0, 1); pose.scaleX *= .88 + progress * .12; pose.scaleY *= .88 + progress * .12; }
    return pose;
  }
  frameFor(actor, pose) {
    const definition = this.manifest[actor.visualKey];
    return sampleClip(definition, pose.clip, pose.clipTime) || sampleClip(definition, 'idle', pose.clipTime);
  }
  frame(id, pose = this.pose(id)) { const actor = this.actors.get(String(id))?.actor; return actor ? this.frameFor(actor, pose) : null; }
  drawGhosts(drawActor) { for (const ghost of this.ghosts) { const pose = this.pose(null, ghost); drawActor(ghost.record.actor, pose, this.frameFor(ghost.record.actor, pose)); } }
  draw(ctx, layer = 'over') {
    ctx.save();
    ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    if (layer === 'under') {
      for (const p of this.projectiles.values()) {
        for (let i = 1; i < p.trail.length; i++) {
          const strength = i / p.trail.length;
          ctx.globalAlpha = strength * .42; ctx.strokeStyle = p.color; ctx.lineWidth = p.size * 2 * strength + 1;
          ctx.beginPath(); ctx.moveTo(p.trail[i - 1].x, p.trail[i - 1].y); ctx.lineTo(p.trail[i].x, p.trail[i].y); ctx.stroke();
        }
      }
    } else {
      for (const p of this.projectiles.values()) {
        ctx.save(); ctx.translate(p.x, p.y); ctx.rotate(p.angle);
        ctx.globalAlpha = .2; ctx.fillStyle = p.color; ctx.beginPath(); ctx.ellipse(0, 0, p.size * 3, p.size * 2.3, 0, 0, Math.PI * 2); ctx.fill();
        ctx.globalAlpha = .95; ctx.fillStyle = p.color; ctx.beginPath(); ctx.moveTo(p.size * 2.5, 0); ctx.lineTo(-p.size * 1.7, p.size); ctx.lineTo(-p.size, 0); ctx.lineTo(-p.size * 1.7, -p.size); ctx.closePath(); ctx.fill();
        ctx.fillStyle = '#fffce7'; ctx.beginPath(); ctx.ellipse(0, 0, p.size * 1.1, p.size * .48, 0, 0, Math.PI * 2); ctx.fill(); ctx.restore();
      }
      for (const p of this.particles) {
        const t = p.age / p.duration;
        ctx.globalAlpha = (1 - t) * .9;
        if (p.kind === 'ring') { ctx.strokeStyle = p.color; ctx.lineWidth = 2 * (1 - t); ctx.beginPath(); ctx.arc(p.x, p.y, 4 + t * 22, 0, Math.PI * 2); ctx.stroke(); }
        else { ctx.fillStyle = p.color; ctx.save(); ctx.translate(p.x, p.y); ctx.rotate(t * 3); const size = p.size * (1 - t * .5); ctx.fillRect(-size, -size, size * 2, size * 2); ctx.restore(); }
      }
    }
    ctx.restore();
  }
  stats() { return { time: this.time, actors: this.actors.size, projectiles: this.projectiles.size, particles: this.particles.length, ghosts: this.ghosts.length, eventIds: this.seen.size, ...this.counts, budgets: { ...this.budgets }, audio: this.audio.stats() }; }
  dispose() { if (this.disposed) return; this.reset(); this.disposed = true; this.audio.dispose(); }
}
