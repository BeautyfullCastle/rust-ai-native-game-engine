import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { CombatPresentation, CombatAudio, sampleClip } from '../combat-presentation.mjs';
import { LegacySnapshotAdapter } from './fixtures/legacy-snapshot-adapter.mjs';

const freeze = object => { if (object && typeof object === 'object') { Object.freeze(object); for (const child of Object.values(object)) freeze(child); } return object; };
const actor = (id = 'arbitrary-monster', x = 40) => ({ id, x, y: 90, visualKey: 'not-a-fixed-roster', color: '#ffd888', renderData: [1, 2, 3] });
const advance = (p, ms) => { while (ms > 0) { const step = Math.min(ms, 100); p.advance(step); ms -= step; } };
const observation = (tick, events = [], actors = [actor()], projectiles = []) => ({ tick, actors, projectiles, events });
class FakeContext {
  constructor() { this.state = 'suspended'; this.currentTime = 0; this.destination = {}; this.nodes = []; this.resumeCalls = 0; this.suspendCalls = 0; }
  createGain() { const n = { gain: { value: 0, setValueAtTime() {}, linearRampToValueAtTime() {}, exponentialRampToValueAtTime() {} }, connect() {}, disconnect() { n.disconnected = true; } }; this.nodes.push(n); return n; }
  createOscillator() { const n = { frequency: { setValueAtTime() {}, exponentialRampToValueAtTime() {} }, connect() {}, disconnect() { n.disconnected = true; }, start() {}, stop(time) { if (time === undefined) n.onended?.(); } }; this.nodes.push(n); return n; }
  async resume() { this.resumeCalls++; this.state = 'running'; }
  async suspend() { this.suspendCalls++; this.state = 'suspended'; }
  async close() { this.state = 'closed'; }
}
const gesture = { type: 'click', isTrusted: true };

test('arbitrary actors animate without mutating frozen observations', () => {
  const p = new CombatPresentation();
  const input = freeze(observation(1, [], [actor('unknown-one'), actor('unknown-two', 100)]));
  const before = JSON.stringify(input);
  p.observe(input);
  const poses = input.actors.map(a => p.pose(a.id));
  advance(p, 250);
  for (const [i, a] of input.actors.entries()) assert.notDeepEqual(p.pose(a.id), poses[i]);
  assert.equal(JSON.stringify(input), before);
  assert.equal(p.pose('unknown-one').mode, 'procedural-single-sprite');
  assert.equal(p.stats().actors, 2);
});

test('actual atlas rectangles loop, clamp, and switch by action phase', () => {
  const frames = [[0, 0, 16, 16], [16, 0, 16, 16], [32, 0, 16, 16]];
  const manifest = { 'not-a-fixed-roster': { image: 'test-atlas.png', clips: { idle: { frames, fps: 10 }, attack: { frames, fps: 10, windupMs: 110, releaseMs: 70, recoveryMs: 160 }, hit: { frames, fps: 20 }, death: { frames, fps: 10 } } } };
  assert.equal(sampleClip(manifest['not-a-fixed-roster'], 'idle', 100).index, 1);
  assert.equal(sampleClip(manifest['not-a-fixed-roster'], 'idle', 300).index, 0);
  assert.equal(sampleClip(manifest['not-a-fixed-roster'], 'attack', 900).index, 2);
  assert.equal(sampleClip({ clips: { idle: { frames: [[0, 0, -1, 16]] } } }, 'idle', 0), null);
  const p = new CombatPresentation({ manifest });
  p.observe(observation(1, [{ id: 'a', type: 'attack', actorId: actor().id }]));
  assert.equal(p.pose(actor().id).phase, 'windup');
  assert.equal(p.stats().particles, 0, 'no release cue during windup');
  advance(p, 120);
  assert.equal(p.pose(actor().id).phase, 'release');
  assert.equal(p.frame(actor().id).index, 1);
  assert.equal(p.frame(actor().id).mode, 'sprite-frame');
  assert.ok(p.stats().particles > 0);
  advance(p, 70); assert.equal(p.pose(actor().id).phase, 'recovery');
  advance(p, 160); assert.equal(p.pose(actor().id).phase, 'idle');
  p.observe(observation(2, [{ id: 'h', type: 'hit', actorId: actor().id }]));
  assert.equal(p.pose(actor().id).clip, 'hit');
  p.observe(observation(3, [{ id: 'd', type: 'death', actorId: actor().id }], []));
  p.drawGhosts((a, pose, frame) => { assert.equal(pose.clip, 'death'); assert.equal(frame.clip, 'death'); });
  advance(p, 301); assert.equal(p.stats().ghosts, 0);
});

test('release observations skip invented windup and events are idempotent', () => {
  const p = new CombatPresentation();
  const event = { id: 'release', type: 'attack', actorId: actor().id, elapsedMs: 110 };
  p.observe(observation(1, [event, event]));
  assert.equal(p.pose(actor().id).phase, 'release');
  assert.equal(p.stats().attacks, 1);
  p.observe(observation(2, [event]));
  assert.equal(p.stats().attacks, 1);
});

test('observed release respects authored windup timing and fixed-size frame pivots', () => {
  const definition = { image: 'test.png', logicalSize: [229, 229], clips: { attack: { frames: [[0, 0, 229, 229], [229, 0, 229, 229]], pivots: [[113, 209], [105, 207]], fps: 10, windupMs: 250, releaseMs: 90, recoveryMs: 160 } } };
  const p = new CombatPresentation({ manifest: { 'not-a-fixed-roster': definition } });
  p.observe(observation(1, [{ id: 'release', type: 'attack', actorId: actor().id, phase: 'release' }]));
  assert.equal(p.pose(actor().id).phase, 'release'); assert.ok(p.stats().particles > 0);
  assert.deepEqual(p.frame(actor().id).pivot, [105, 207]); assert.deepEqual(p.frame(actor().id).logicalSize, [229, 229]);
});

test('authored hit clip plays its full frame duration without procedural foot wobble', () => {
  const frames = Array.from({ length: 6 }, (_, i) => [i * 32, 0, 32, 32]);
  const p = new CombatPresentation({ manifest: { 'not-a-fixed-roster': { image: 'test.png', clips: { hit: { frames, fps: 16 } } } } });
  p.observe(observation(1, [{ id: 'hit', type: 'hit', actorId: actor().id }]));
  advance(p, 330); assert.equal(p.pose(actor().id).clip, 'hit'); assert.equal(p.frame(actor().id).index, 5);
  assert.equal(p.pose(actor().id).x, 0); assert.equal(p.pose(actor().id).y, 0); assert.equal(p.pose(actor().id).scaleX, 1);
  advance(p, 50); assert.equal(p.pose(actor().id).clip, 'idle');
});

test('terminal gameplay can stop while presentation death tails settle', () => {
  const p = new CombatPresentation();
  p.observe(observation(1));
  p.observe({ ...observation(2, [{ id: 'final-death', type: 'death', actorId: actor().id }], []), phase: 1 });
  assert.equal(p.stats().ghosts, 1);
  advance(p, 700); assert.equal(p.stats().ghosts, 0); assert.equal(p.stats().particles, 0);
  const source = fs.readFileSync(new URL('../game.js', import.meta.url), 'utf8');
  assert.match(source, /presentation\.setPaused\(paused\|\|ready\)/, 'terminal phase must not freeze FX in integration');
});

test('particle, actor, projectile, trail, ghost and event-history budgets hold', () => {
  const p = new CombatPresentation({ budgets: { actors: 5, particles: 7, projectiles: 3, trailPoints: 2, ghosts: 2, eventIds: 16 } });
  const actors = Array.from({ length: 10 }, (_, i) => actor(String(i)));
  for (let tick = 1; tick <= 300; tick++) {
    const events = Array.from({ length: 10 }, (_, i) => ({ id: `${tick}:${i}`, type: i % 2 ? 'death' : 'hit', actorId: String(i % 5) }));
    const projectiles = Array.from({ length: 10 }, (_, i) => ({ id: `p${i}`, x: tick, y: i }));
    p.observe(observation(tick, events, actors, projectiles)); p.advance(1);
    assert.ok(p.stats().particles <= 7); assert.ok(p.stats().actors <= 5); assert.ok(p.stats().projectiles <= 3); assert.ok(p.stats().ghosts <= 2); assert.ok(p.stats().eventIds <= 16);
    for (const projectile of p.projectiles.values()) assert.ok(projectile.trail.length <= 2);
  }
  assert.ok(p.stats().droppedParticles > 0);
  assert.ok(p.stats().droppedProjectiles > 0);
});

test('pause/hidden freezes presentation; lifecycle expiry, rewind, reset, dispose clean up', () => {
  const p = new CombatPresentation();
  p.observe(observation(10, [{ id: 'h', type: 'hit', actorId: actor().id }], [actor()], [{ id: 'p', x: 1, y: 1 }]));
  const frozen = JSON.stringify(p.stats());
  p.setPaused(true); advance(p, 400); assert.equal(p.time, 0); assert.ok(p.stats().particles > 0);
  p.setPaused(false); p.setHidden(true); advance(p, 400); assert.equal(p.time, 0);
  p.setHidden(false); advance(p, 700); assert.equal(p.stats().particles, 0);
  p.observe(observation(11, [{ id: 'd', type: 'death', actorId: actor().id }], [])); assert.equal(p.stats().ghosts, 1);
  advance(p, 600); assert.equal(p.stats().ghosts, 0);
  p.observe(observation(1)); assert.equal(p.stats().hits, 0, 'rewind clears history and effects');
  p.reset(); for (const key of ['actors', 'particles', 'ghosts', 'projectiles', 'eventIds']) assert.equal(p.stats()[key], 0);
  p.dispose(); p.observe(observation(2)); p.advance(100); assert.equal(p.stats().actors, 0);
  assert.ok(frozen);
});

test('projectile disappearance does not cause impact, damage, or death', () => {
  const p = new CombatPresentation();
  p.observe(freeze(observation(1, [], [actor()], [{ id: 'p', x: 1, y: 1 }])));
  p.observe(freeze(observation(2)));
  assert.equal(p.stats().projectiles, 0); assert.equal(p.stats().particles, 0); assert.equal(p.stats().hits, 0); assert.equal(p.stats().deaths, 0);
});

test('canvas drawing restores state and consumes no gameplay callbacks', () => {
  const p = new CombatPresentation(); let balance = 0, drawCalls = 0;
  const context = new Proxy({ save() { balance++; }, restore() { balance--; } }, { get: (target, key) => key in target ? target[key] : () => { drawCalls++; } });
  p.observe(observation(1, [{ id: 'h', type: 'hit', actorId: actor().id }], [actor()], [{ id: 'p', x: 1, y: 1 }]));
  p.observe(observation(2, [], [actor()], [{ id: 'p', x: 4, y: 3 }]));
  p.draw(context, 'under'); p.draw(context, 'over'); assert.equal(balance, 0); assert.ok(drawCalls > 0);
});

test('audio requires trusted gesture, enforces polyphony/rate limit, cleans voices', async () => {
  let created = 0; const context = new FakeContext();
  const audio = new CombatAudio({ contextFactory: () => { created++; return context; }, maxVoices: 2 });
  await audio.setMuted(false); assert.equal(created, 0); assert.equal(audio.cue('attack'), false);
  assert.equal(await audio.unlock({ type: 'click', isTrusted: false }), false); assert.equal(created, 0);
  assert.equal(await audio.unlock(gesture), true); assert.equal(created, 1);
  assert.equal(audio.cue('attack'), true); assert.equal(audio.cue('attack'), false);
  context.currentTime += .1; assert.equal(audio.cue('hit'), true);
  context.currentTime += .1; assert.equal(audio.cue('death'), false); assert.equal(audio.stats().voices, 2);
  await audio.setMuted(true); assert.equal(audio.stats().voices, 0); assert.equal(context.state, 'suspended'); assert.equal(audio.cue('hit'), false);
  await audio.setMuted(false); assert.equal(context.state, 'running');
  await audio.setPaused(true); assert.equal(context.state, 'suspended'); assert.equal(audio.cue('hit'), false);
  await audio.setPaused(false); await audio.setHidden(true); assert.equal(context.state, 'suspended');
  await audio.setHidden(false); assert.equal(context.state, 'running');
  audio.reset(); assert.equal(audio.stats().voices, 0);
  audio.dispose(); assert.equal(context.state, 'closed'); assert.equal(await audio.unlock(gesture), false);
});

test('audio async pause/resume races converge to the latest state', async () => {
  const context = new FakeContext();
  const audio = new CombatAudio({ contextFactory: () => context }); await audio.setMuted(false); await audio.unlock(gesture);
  const noop = audio.setMuted(false), pauseAfterNoop = audio.setPaused(true);
  await Promise.all([noop, pauseAfterNoop]); assert.equal(context.state, 'suspended', 'pause after same-turn no-op must settle');
  await audio.setPaused(false);
  let release; context.suspend = () => new Promise(resolve => { release = () => { context.state = 'suspended'; resolve(); }; });
  const pause = audio.setPaused(true); const resume = audio.setPaused(false);
  release(); await Promise.all([pause, resume]); assert.equal(context.state, 'running');
  let releaseResume; context.state = 'suspended'; context.resume = () => new Promise(resolve => { releaseResume = () => { context.state = 'running'; resolve(); }; });
  context.suspend = FakeContext.prototype.suspend.bind(context);
  const unmute = audio.setMuted(false); const mute = audio.setMuted(true); releaseResume(); await Promise.all([unmute, mute]); assert.equal(context.state, 'suspended'); assert.equal(audio.master.gain.value, 0);
});

test('unsupported and blocked audio degrade without breaking presentation', async () => {
  const missing = new CombatAudio({ contextFactory: () => null }); await missing.setMuted(false); assert.equal(await missing.unlock(gesture), false); assert.equal(missing.cue('attack'), false);
  const context = new FakeContext(); context.resume = async () => { throw Error('test policy block'); };
  const denied = new CombatAudio({ contextFactory: () => context }); await denied.setMuted(false); assert.equal(await denied.unlock(gesture), false); assert.match(denied.stats().error, /test policy block/);
});

test('legacy adapter distinguishes HP change, known kill batch, mixed retirement, and relocated units', () => {
  const adapter = new LegacySnapshotAdapter();
  const make = (tick, enemies, kills = 0, leaks = 0, units = []) => ({ tick, units, enemies, projectiles: [], kills, leaks });
  const e = [1, 0, 8, 100, 100, 0, 100, 100];
  adapter.observe(freeze(make(0, [e])));
  let value = adapter.observe(freeze(make(1, [[1, 0, 8, 90, 100, 0, 101, 100]])));
  assert.equal(value.events[0].type, 'hit');
  value = adapter.observe(freeze(make(2, [], 1))); assert.equal(value.events[0].type, 'death');
  adapter.reset(); adapter.observe(make(0, [e, [2, ...e.slice(1)]]));
  value = adapter.observe(make(1, [], 1, 1)); assert.equal(value.events.length, 2); assert.ok(value.events.every(e => e.type === 'remove'));
  adapter.reset(); adapter.observe(make(0, [], 0, 0, [[0, 0, 9, 1, 20, 100, 100]]));
  value = adapter.observe(make(1, [], 0, 0, [[0, 1, 9, 1, 19, 160, 100]])); assert.equal(value.events.length, 0, 'move does not invent shot');
});

test('real prebuilt WASM remains byte-identical under 3600 presentation ticks', async () => {
  const bytes = fs.readFileSync(process.env.ORR_DEFENSE_WASM || new URL('../game.wasm', import.meta.url));
  const [a, b] = await Promise.all([WebAssembly.instantiate(bytes, {}), WebAssembly.instantiate(bytes, {})]);
  const baseline = a.instance.exports, withPresentation = b.instance.exports;
  const read = wasm => { const ptr = wasm.defense_snapshot(), length = wasm.defense_snapshot_len(); return JSON.parse(new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, ptr, length))); };
  const p = new CombatPresentation(), adapter = new LegacySnapshotAdapter();
  baseline.defense_init(71825); withPresentation.defense_init(71825);
  p.observe(adapter.observe(read(withPresentation)));
  let seq = 0, maximum = 0; const seenKinds = new Set();
  for (let tick = 0; tick < 3600; tick++) {
    const input = tick % 90 === 0 ? [1, 0, 0, ++seq] : [0, 0, 0, 0];
    baseline.defense_step(...input); withPresentation.defense_step(...input);
    const raw = freeze(read(withPresentation)), before = JSON.stringify(raw);
    const normalized = freeze(adapter.observe(raw)); p.observe(normalized); p.advance(1000 / 60);
    for (const a of normalized.actors) seenKinds.add(a.visualKey);
    maximum = Math.max(maximum, p.stats().particles);
    assert.equal(JSON.stringify(raw), before);
    assert.deepEqual(read(withPresentation), read(baseline), `presentation changed snapshot at tick ${tick}`);
  }
  assert.ok(p.stats().attacks > 100); assert.ok(p.stats().hits > 100); assert.ok(p.stats().deaths > 0); assert.ok(seenKinds.size > 6); assert.ok(maximum <= p.budgets.particles);
  console.log(JSON.stringify({ realWasmReplay: { ticks: 3600, checksum: read(baseline).checksum, stats: p.stats(), visualKeys: [...seenKinds], maximumParticles: maximum } }));
});
