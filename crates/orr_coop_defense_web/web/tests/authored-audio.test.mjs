import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { AuthoredAssetAudio, loadAuthoredAssetBank, audioGain } from '../audio/authored-asset-audio.mjs';
import { CoopDefenseAudio, loadAuthoredAudio } from '../audio/coop-defense-audio.mjs';
import { StrictEventAdapter } from '../strict-event-adapter.mjs';

const root = new URL('../assets/audio/', import.meta.url);
const manifest = JSON.parse(await readFile(new URL('bank.json', root)));
const objects = Object.fromEntries(await Promise.all(manifest.cues.map(async r => [r.file, new Uint8Array(await readFile(new URL(r.file, root)))])));
const bank = await loadAuthoredAssetBank(manifest, async file => objects[file]);
const embedded = () => JSON.stringify({ manifest, objects: Object.fromEntries(Object.entries(objects).map(([k, v]) => [k, Buffer.from(v).toString('base64')])) });
function context() {
  const ctx = { state: 'suspended', currentTime: 0, destination: {}, sources: [], buffers: [],
    createGain() { return { gain: { value: 0 }, connect() {}, disconnect() {} }; },
    createBuffer(_channels, length, rate) { const b = { length, rate, copyToChannel(pcm) { this.samples = pcm.slice(); } }; this.buffers.push(b); return b; },
    createBufferSource() { const s = { connect() {}, disconnect() {}, start() { this.started = true; }, stop() { this.stopped = true; } }; this.sources.push(s); return s; },
    async resume() { this.state = 'running'; }, async suspend() { this.state = 'suspended'; }, async close() { this.state = 'closed'; },
  }; return ctx;
}
const gesture = { isTrusted: true, type: 'click' };
const unit = [0, 0, 0, 1, 0, 100, 200, 1];
const enemy = [10, 0, 0, 100, 100, 0, 200, 200];
const projectile = [0, 0, 10, 100, 200, 10, 20, 1];
function event(kind, tick, seq = 0, extra = {}) { return { tick: String(tick), system: 0, seq, kind, generation: 0, unitId: 1, otherUnitId: 0, enemyId: 0, projectileId: 0, board: 0, role: 0, value: 0, hpAfter: 0, x: 100, y: 200, ...extra }; }
function snapshot(tick, events = [], extra = {}) { return { units: [unit], enemies: [enemy], projectiles: [], ...extra, presentation: { version: 1, source: '1', generation: 0, tick: String(tick), delivery: 'latest_tick', status: 'complete', dropped: 0, fullResync: false, windup: false, events } }; }
function pair() { const ctx = context(), audio = new CoopDefenseAudio({ bank, contextFactory: () => ctx }), adapter = new StrictEventAdapter(); const send = s => { const r = adapter.consume(s); audio.observe(r, s.presentation); return r; }; return { ctx, audio, send }; }

test('all ten real PCM objects admit, are mono 48kHz, and keep bounded mix gain', async () => {
  const ctx = context(), owner = new AuthoredAssetAudio({ bank, contextFactory: () => ctx, maxVoices: 32 });
  await owner.setMuted(false); assert.equal(await owner.unlock(gesture), true);
  assert.equal(ctx.buffers.length, 10); assert.ok(ctx.buffers.every(b => b.rate === 48000 && b.length <= 48000));
  assert.ok(32 * audioGain(32) * 8192 / 32768 <= .75);
  owner.beginEpoch({ source: '1', generation: 0, rewindSerial: '0', tick: '0' });
  for (let i = 0; i < 33; ++i) owner.playAsset(bank.assetIds[i % 10], `1/0/0/1/0/${i}`, { unitId: 1, attackId: 0, projectileId: 20 });
  assert.equal(owner.stats().played, 32); assert.equal(owner.stats().dropped, 1);
  owner.dispose(); assert.ok(ctx.sources.every(s => s.stopped));
});
test('tampered PCM, invalid header, oversized peak and duplicate metadata reject', async () => {
  for (const mode of ['tamper', 'header', 'peak', 'duplicate']) {
    const m = structuredClone(manifest), o = Object.fromEntries(Object.entries(objects).map(([k,v]) => [k,v.slice()])), r = m.cues[0];
    if (mode === 'duplicate') m.cues[1] = { ...r };
    else { const bytes = o[r.file]; bytes[mode === 'header' ? 0 : 8] ^= 255; if (mode === 'peak') new DataView(bytes.buffer).setInt16(8, 30000, true); if (mode !== 'tamper') r.sha256 = createHash('sha256').update(bytes).digest('hex'); }
    await assert.rejects(loadAuthoredAssetBank(m, async file => o[file]));
  }
});
test('embedded opt-in only, duplicate ID/malformed content reject with no context', async () => {
  assert.equal(await loadAuthoredAudio({ getElementById: () => null }), null);
  await assert.rejects(loadAuthoredAudio({ querySelectorAll: () => [1,2], getElementById: () => ({ textContent: embedded() }) }), /Duplicate/);
  await assert.rejects(loadAuthoredAudio({ getElementById: () => ({ textContent: '{}' }) }));
  const a = await loadAuthoredAudio({ getElementById: () => ({ textContent: embedded() }) });
  assert.equal(a.stats().contextState, 'not-created');
});
test('gesture lock and suppressed muted events never replay after unlock', async () => {
  const { ctx, audio, send } = pair(); send(snapshot(0, [], { units: [] }));
  assert.equal(await audio.unlock({ ...gesture, isTrusted: false }), false); assert.equal(ctx.buffers.length, 0);
  const s = snapshot(1, [event('summon', 1)]); assert.equal(send(s).status, 'applied');
  assert.equal(audio.stats().suppressed, 1); await audio.setMuted(false); await audio.unlock(gesture);
  send(s); audio.cue('ui'); assert.equal(ctx.sources.length, 0);
});
test('strict actual summon/release/hit/death drive typed PCM once without snapshot mutation', async () => {
  const { ctx, audio, send } = pair(); await audio.setMuted(false); await audio.unlock(gesture);
  send(snapshot(0, [], { units: [] })); send(snapshot(1, [event('summon', 1)]));
  const release = snapshot(2, [event('release', 2, 0, { enemyId: 10, projectileId: 20, value: 10 })], { projectiles: [projectile] });
  const before = JSON.stringify(release); assert.equal(send(release).status, 'applied'); send(release); assert.equal(JSON.stringify(release), before);
  const hit = event('hit', 3, 0, { enemyId: 10, projectileId: 20, value: 100, x: 200 });
  assert.equal(send(snapshot(3, [hit, { ...hit, kind: 'death', seq: 1, value: 0 }], { enemies: [] })).status, 'applied');
  assert.equal(audio.stats().played, 4); assert.equal(audio.stats().adapterError, null);
  assert.deepEqual(ctx.sources.map(s => ctx.buffers.indexOf(s.buffer)), [5,0,3,9]);
});
test('pause, hidden, reset, gap and malformed batch stop voices without catch-up', async () => {
  const { ctx, audio, send } = pair(); await audio.setMuted(false); await audio.unlock(gesture);
  send(snapshot(0, [], { units: [] })); send(snapshot(1, [event('summon', 1)]));
  await audio.setPaused(true); assert.equal(audio.stats().voices, 0); await audio.setPaused(false);
  await audio.setHidden(true); assert.equal(ctx.state, 'suspended'); await audio.setHidden(false);
  send(snapshot(8, [event('summon', 8)])); assert.equal(audio.stats().played, 1);
  const bad = snapshot(9, [event('summon', 9)]); bad.presentation.events[0].unitId = 0;
  assert.equal(send(bad).status, 'rejected'); assert.equal(audio.stats().voices, 0);
  send(snapshot(10)); assert.equal(audio.stats().played, 1);
  send(snapshot(0)); assert.equal(audio.stats().adapterError, null);
  audio.reset(); audio.dispose(); assert.equal(ctx.state, 'closed');
});
test('accepted merge uses merge PCM; generation restart and new source baseline silently', async () => {
  const { audio, ctx, send } = pair(); await audio.setMuted(false); await audio.unlock(gesture);
  const other = [0, 1, 0, 1, 0, 120, 200, 2]; send(snapshot(0, [], { units: [unit, other] }));
  const merged = [...unit]; merged[3] = 2;
  assert.equal(send(snapshot(1, [event('merge', 1, 0, { otherUnitId: 2, value: 2 })], { units: [merged] })).status, 'applied');
  assert.equal(ctx.sources[0].buffer, ctx.buffers[6]);
  const restart = snapshot(2); restart.presentation.generation = 1;
  send(restart); assert.equal(audio.stats().voices, 0); assert.equal(audio.stats().epoch.generation, 1);
  const newSource = snapshot(0); newSource.presentation.source = '2'; send(newSource);
  assert.equal(audio.stats().epoch.source, '2'); assert.equal(audio.stats().played, 1); assert.equal(audio.stats().adapterError, null);
});
test('partial buffer initialization failure closes context and does not unlock', async () => {
  const ctx = context(); let count = 0; const create = ctx.createBuffer.bind(ctx);
  ctx.createBuffer = (...args) => { if (++count === 3) throw Error('decode allocation failure'); return create(...args); };
  const a = new AuthoredAssetAudio({ bank, contextFactory: () => ctx }); await a.setMuted(false);
  assert.equal(await a.unlock(gesture), false); assert.equal(ctx.state, 'closed'); assert.equal(a.stats().bufferCount, 0);
});
test('resume rejection and playback failure fail silent and clean voices', async () => {
  const ctx = context(); ctx.resume = async () => { throw Error('autoplay rejected'); };
  const a = new AuthoredAssetAudio({ bank, contextFactory: () => ctx }); await a.setMuted(false);
  assert.equal(await a.unlock(gesture), false); assert.equal(a.stats().unlocked, false); assert.equal(a.stats().voices, 0);
  const p = pair(); await p.audio.setMuted(false); await p.audio.unlock(gesture); p.send(snapshot(0, [], { units: [] }));
  p.ctx.createBufferSource = () => { throw Error('device gone'); };
  p.send(snapshot(1, [event('summon', 1)])); assert.equal(p.audio.stats().failed, 'runtime-failure'); assert.equal(p.audio.stats().voices, 0);
});
test('bank copies caller bytes before asynchronous hashing and owners isolate buffers', async () => {
  const m = structuredClone(manifest), o = Object.fromEntries(Object.entries(objects).map(([k,v])=>[k,v.slice()]));
  const b = await loadAuthoredAssetBank(m, async f => o[f], async bytes => { const hash = createHash('sha256').update(bytes).digest('hex'); o[m.cues[0].file].fill(0); return hash; });
  const ctx = context(), a = new AuthoredAssetAudio({ bank: b, contextFactory: () => ctx }); await a.unlock(gesture);
  assert.ok(ctx.buffers[0].samples.some(n => n !== 0));
});
test('dispose owns the context after an in-flight resume settles late', async () => {
  const ctx = context(); let settle, closes = 0;
  ctx.resume = () => new Promise(resolve => { settle = () => { ctx.state = 'running'; resolve(); }; });
  ctx.close = async () => { closes++; ctx.state = 'closed'; };
  const a = new AuthoredAssetAudio({ bank, contextFactory: () => ctx }); await a.setMuted(false);
  const unlock = a.unlock(gesture); assert.equal(typeof settle, 'function');
  a.dispose(); a.dispose(); assert.equal(ctx.state, 'closed'); assert.equal(closes, 1);
  settle(); assert.equal(await unlock, false);
  assert.equal(ctx.state, 'closed'); assert.equal(closes, 2);
  assert.equal(a.stats().voices, 0); assert.equal(a.stats().bufferCount, 0); assert.equal(a.stats().disposed, true);
  await a.setMuted(false); await a.setPaused(false); assert.equal(closes, 2);
});
test('pending resume and suspend reconcile the latest repeated mute/pause/hidden intent', async () => {
  for (const method of ['setMuted', 'setPaused', 'setHidden']) {
    const ctx = context(); let resume, suspend;
    ctx.resume = () => new Promise(resolve => { resume = () => { ctx.state = 'running'; resolve(); }; });
    ctx.suspend = () => new Promise(resolve => { suspend = () => { ctx.state = 'suspended'; resolve(); }; });
    const a = new AuthoredAssetAudio({ bank, contextFactory: () => ctx }); await a.setMuted(false);
    const unlocked = a.unlock(gesture);
    const changes = [a[method](true), a[method](false), a[method](true)];
    resume(); await Promise.resolve(); assert.equal(typeof suspend, 'function', method);
    suspend(); assert.equal(await unlocked, false); await Promise.all(changes);
    assert.equal(ctx.state, 'suspended'); assert.equal(a.enabled, false); assert.equal(a.stats().voices, 0);
    const reenabled = a[method](false); resume(); await reenabled;
    assert.equal(ctx.state, 'running'); assert.equal(a.enabled, true);
    const paused = a[method](true), resumed = a[method](false);
    suspend(); await Promise.resolve(); resume(); await Promise.all([paused, resumed]);
    assert.equal(ctx.state, 'running'); assert.equal(a.enabled, true); a.dispose();
  }
});
test('disposed ownership is reasserted for rejected resume and late suspend too', async () => {
  for (const operation of ['resume-reject', 'suspend']) {
    const ctx = context(); let settle;
    const a = new AuthoredAssetAudio({ bank, contextFactory: () => ctx }); await a.setMuted(false);
    let pending;
    if (operation === 'resume-reject') {
      ctx.resume = () => new Promise((_resolve, reject) => { settle = () => { ctx.state = 'running'; reject(Error('late rejection')); }; });
      pending = a.unlock(gesture);
    } else {
      await a.unlock(gesture);
      ctx.suspend = () => new Promise(resolve => { settle = () => { ctx.state = 'suspended'; resolve(); }; });
      pending = a.setPaused(true);
    }
    a.dispose(); settle(); await pending;
    assert.equal(ctx.state, 'closed', operation); assert.equal(a.stats().disposed, true); assert.equal(a.stats().voices, 0);
  }
});
test('prebuilt actual WASM accepted events drive PCM without changing 4000 snapshots', { skip: !process.env.ORR_DEFENSE_WASM }, async () => {
  const bytes = await readFile(process.env.ORR_DEFENSE_WASM);
  const [game, baseline] = await Promise.all([WebAssembly.instantiate(bytes, {}), WebAssembly.instantiate(bytes, {})]);
  const w = game.instance.exports, b = baseline.instance.exports;
  const read = x => JSON.parse(new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.defense_snapshot(), x.defense_snapshot_len())));
  const { audio, ctx, send } = pair(); await audio.setMuted(false); await audio.unlock(gesture);
  w.defense_init(71825); b.defense_init(71825); send(read(w)); let seq = 0, expected = 0;
  for (let tick = 0; tick < 4000; tick++) {
    const input = tick % 90 === 0 ? [1, 0, 0, ++seq] : [0, 0, 0, 0]; w.defense_step(...input); b.defense_step(...input);
    const snapshot = read(w); assert.equal(send(snapshot).status, 'applied');
    expected += snapshot.presentation.events.filter(e => ['summon','merge','release','hit','death'].includes(e.kind)).length;
    send(snapshot); assert.equal(audio.stats().adapterError, null); assert.deepEqual(read(w), read(b));
    for (const source of ctx.sources) source.onended?.();
  }
  assert.ok(expected > 0); assert.equal(audio.stats().played, expected); assert.equal(audio.stats().dropped, 0);
  console.log(JSON.stringify({ wasmSha256: createHash('sha256').update(bytes).digest('hex'), ticks: 4000, played: expected, checksum: read(w).checksum, webAudio: 'mock' }));
});
