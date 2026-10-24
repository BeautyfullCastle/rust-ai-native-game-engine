import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import crypto from 'node:crypto';
import { StrictEventAdapter, applyStrictSnapshot, eventKey } from '../strict-event-adapter.mjs';
import { CombatPresentation } from '../combat-presentation.mjs';
const clone = x => JSON.parse(JSON.stringify(x));
const freeze = x => { if (x && typeof x === 'object') { Object.freeze(x); for (const child of Object.values(x)) freeze(child); } return x; };
const unit = [0, 0, 17, 1, 0, 100, 200, 1];
const enemy = [10, 0, 21, 100, 100, 0, 150, 200];
const projectile = [0, 17, 10, 100, 200, 10, 20, 1];
function evt(kind, tick, seq = 0, payload = {}) { return { tick: String(tick), system: 0, seq, kind, generation: 0, unitId: 0, otherUnitId: 0, enemyId: 0, projectileId: 0, board: 0, role: 0, value: 0, hpAfter: 0, x: 0, y: 0, ...payload }; }
function snap(tick, { units = [unit], enemies = [enemy], projectiles = [], events = [], header = {} } = {}) { return clone({ units, enemies, projectiles, presentation: { version: 1, source: '1', generation: 0, tick: String(tick), delivery: 'latest_tick', status: 'complete', dropped: 0, fullResync: false, windup: false, events, ...header } }); }
const release = (tick, seq = 0) => evt('release', tick, seq, { unitId: 1, enemyId: 10, projectileId: 20, role: 17, value: 10, x: 100, y: 200 });
const hit = (tick, seq = 0, hpAfter = 90) => evt('hit', tick, seq, { unitId: 1, enemyId: 10, projectileId: 20, role: 17, value: 10, hpAfter, x: 150, y: 200 });
const advance = (p, ms) => { while (ms > 0) { const dt = Math.min(ms, 100); p.advance(dt); ms -= dt; } };
function pair() { return { p: new CombatPresentation(), a: new StrictEventAdapter() }; }

test('strict consumer uses exact engine release/hit keys and never mutates snapshots', () => {
  const { p, a } = pair(); applyStrictSnapshot(p, a, freeze(snap(0)));
  const input = freeze(snap(1, { projectiles: [projectile], events: [release(1)] })), text = JSON.stringify(input);
  const result = applyStrictSnapshot(p, a, input);
  assert.equal(result.status, 'applied'); assert.equal(result.observation.events[0].id, eventKey('1', 0, release(1)));
  assert.equal(p.pose(a.unitId(unit)).phase, 'release'); assert.equal(p.stats().attacks, 1); assert.equal(JSON.stringify(input), text);
  applyStrictSnapshot(p, a, freeze(snap(2, { enemies: [[...enemy.slice(0, 3), 90, ...enemy.slice(4)]], events: [hit(2)] })));
  assert.equal(p.stats().hits, 1); assert.equal(p.stats().deaths, 0);
});

test('repeated batch and repeated exact EventKey cause only one effect', () => {
  const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0));
  const input = snap(1, { projectiles: [projectile], events: [release(1), release(1)] });
  assert.equal(applyStrictSnapshot(p, a, input).status, 'applied'); assert.equal(p.stats().attacks, 1);
  assert.equal(applyStrictSnapshot(p, a, clone(input)).status, 'duplicate'); assert.equal(p.stats().attacks, 1);
  assert.equal(a.stats().duplicateBatches, 1); assert.ok(a.stats().duplicateEvents >= 1);
  const conflict = clone(input); conflict.presentation.events[1].value++;
  assert.equal(applyStrictSnapshot(p, a, conflict).status, 'rejected'); assert.equal(p.stats().particles, 0);
});

test('snapshot HP/cooldown changes and projectile disappearance never infer events', () => {
  const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0, { projectiles: [projectile] }));
  const changedUnit = [...unit]; changedUnit[4] = 60;
  const changedEnemy = [...enemy]; changedEnemy[3] = 1;
  applyStrictSnapshot(p, a, snap(1, { units: [changedUnit], enemies: [changedEnemy], projectiles: [] }));
  assert.equal(p.stats().attacks, 0); assert.equal(p.stats().hits, 0); assert.equal(p.stats().particles, 0);
  applyStrictSnapshot(p, a, snap(2, { units: [changedUnit], enemies: [] }));
  assert.equal(p.stats().deaths, 0); assert.equal(p.stats().removals, 0);
});

test('tick gaps, sequence gaps, overflow, and fullResync rebuild without catch-up effects', () => {
  for (const kind of ['tick-gap', 'sequence-gap', 'overflow', 'explicit']) {
    const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0)); applyStrictSnapshot(p, a, snap(1, { projectiles: [projectile], events: [release(1)] })); assert.ok(p.stats().particles > 0);
    let input;
    if (kind === 'tick-gap') input = snap(4, { projectiles: [projectile], events: [release(4)] });
    if (kind === 'sequence-gap') input = snap(2, { projectiles: [projectile], events: [release(2, 1)] });
    if (kind === 'overflow') input = snap(2, { header: { status: 'overflow', dropped: 1025, fullResync: true } });
    if (kind === 'explicit') input = snap(2, { projectiles: [projectile], events: [release(2)], header: { fullResync: true } });
    const result = applyStrictSnapshot(p, a, input); assert.equal(result.status, 'resync', kind);
    assert.equal(result.observation.events.length, 0); assert.equal(p.stats().particles, 0); assert.equal(p.stats().ghosts, 0); assert.equal(p.stats().attacks, 0); assert.equal(p.stats().actors, 2);
  }
});

test('malformed/unsupported v1 rejects atomically and never chooses legacy fallback', () => {
  const mutations = [
    s => delete s.presentation, s => s.presentation.version = 2, s => s.presentation.windup = true,
    s => s.presentation.source = '01', s => s.presentation.tick = '9007199254740993x', s => s.presentation.generation = -1,
    s => s.presentation.events[0].kind = 'unknown', s => s.presentation.events[0].tick = '0',
    s => s.presentation.events[0].generation = 2, s => s.presentation.events[0].unitId = 0,
    s => s.presentation.events[0].system = 65536, s => s.units.push([...unit]),
    s => s.presentation.events.push({ ...release(1, 1), value: 50 }),
    s => s.presentation.status = 'overflow', s => s.presentation.dropped = 1,
  ];
  for (const mutate of mutations) {
    const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0));
    const input = snap(1, { projectiles: [projectile], events: [release(1)] }); mutate(input);
    const result = applyStrictSnapshot(p, a, input); assert.equal(result.status, 'rejected'); assert.equal(result.observation, null); assert.equal(p.stats().attacks, 0); assert.equal(p.stats().particles, 0);
    const recovery = applyStrictSnapshot(p, a, snap(2, { projectiles: [projectile], events: [release(2)] })); assert.equal(recovery.status, 'resync'); assert.equal(recovery.observation.events.length, 0);
  }
  const source = fs.readFileSync(new URL('../game.js', import.meta.url), 'utf8'); assert.doesNotMatch(source, /LegacySnapshotAdapter|legacy-snapshot-adapter|function projectilePosition/);
});

test('u64 ticks remain exact; backward tick/source changes clear and rebuild snapshot', () => {
  const { p, a } = pair(), tick = 9007199254740993n;
  applyStrictSnapshot(p, a, snap(tick)); assert.equal(applyStrictSnapshot(p, a, snap(tick + 1n)).status, 'applied');
  assert.equal(a.stats().engineTick, String(tick + 1n)); assert.equal(applyStrictSnapshot(p, a, snap(tick)).reason, 'backward-engine-tick');
  assert.equal(applyStrictSnapshot(p, a, snap(0, { header: { source: '2' } })).status, 'resync');
  assert.equal(applyStrictSnapshot(p, a, snap(tick + 2n)).reason, 'new-source'); assert.equal(a.stats().source, '1'); assert.equal(p.stats().particles, 0);
});

test('restart/new source isolate reused actor IDs and discard old transients', () => {
  const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0)); const original = a.unitId(unit);
  applyStrictSnapshot(p, a, snap(1, { projectiles: [projectile], events: [release(1)] }));
  const restart = snap(2, { units: [], enemies: [], header: { generation: 1 }, events: [evt('restart', 2, 0, { generation: 1 })] });
  assert.equal(applyStrictSnapshot(p, a, restart).reason, 'restart-generation'); assert.equal(p.stats().particles, 0); assert.equal(p.stats().actors, 0);
  applyStrictSnapshot(p, a, snap(3, { header: { generation: 1 } })); assert.notEqual(a.unitId(unit), original);
  assert.equal(applyStrictSnapshot(p, a, snap(4, { header: { generation: 1 }, events: [evt('restart', 4, 0, { generation: 1 })] })).status, 'rejected');
});

test('exact mixed death/leak events never attribute a leak as death', () => {
  const { p, a } = pair(), second = [11, ...enemy.slice(1)];
  applyStrictSnapshot(p, a, snap(0, { enemies: [enemy, second], projectiles: [projectile] }));
  const events = [{ ...hit(1, 0, 0), x: 151, y: 202 }, evt('death', 1, 1, { unitId: 1, enemyId: 10, projectileId: 20, role: 17, value: 21, x: 151, y: 202 }), evt('leak', 1, 2, { enemyId: 11, value: 21, hpAfter: 100, x: 180, y: 205 })];
  const result = applyStrictSnapshot(p, a, snap(1, { enemies: [], events })); assert.equal(result.status, 'applied');
  assert.equal(p.stats().hits, 1); assert.equal(p.stats().deaths, 1); assert.equal(p.stats().removals, 1);
  const ghosts = []; p.drawGhosts((actor, pose) => ghosts.push([actor.x, actor.y, pose.phase]));
  assert.deepEqual(ghosts, [[151, 202, 'death'], [180, 205, 'retire']]);
});

test('death must follow its exact lethal hit; reversed, duplicate or mismatched death rejects the whole batch', () => {
  for (const variant of ['reversed', 'mismatched-position', 'duplicate']) {
    const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0, { projectiles: [projectile] }));
    const lethal = hit(1, 0, 0), death = evt('death', 1, 1, { unitId: 1, enemyId: 10, projectileId: 20, role: 17, value: 21, x: 150, y: 200 });
    const events = variant === 'reversed' ? [{ ...death, seq: 0 }, { ...lethal, seq: 1 }] : variant === 'mismatched-position' ? [lethal, { ...death, x: 151 }] : [lethal, death, { ...death, seq: 2 }];
    assert.equal(applyStrictSnapshot(p, a, snap(1, { enemies: [], events })).status, 'rejected'); assert.equal(p.stats().hits, 0); assert.equal(p.stats().deaths, 0);
  }
});

test('same-tick spawn/hit/death preserves event-local pose even without a live snapshot actor', () => {
  const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0, { projectiles: [projectile] }));
  const events = [
    evt('enemy_spawn', 1, 0, { enemyId: 11, value: 21, hpAfter: 100, x: 120, y: 200 }),
    { ...hit(1, 1, 0), enemyId: 11, value: 100, x: 122 },
    evt('death', 1, 2, { unitId: 1, enemyId: 11, projectileId: 20, role: 17, value: 21, x: 122, y: 200 }),
  ];
  const result = applyStrictSnapshot(p, a, snap(1, { events }));
  assert.equal(result.status, 'applied'); assert.equal(result.observation.events[0].actor.x, 120); assert.equal(result.observation.events[0].actor.renderData[3], 100);
  assert.equal(p.stats().spawns, 1); assert.equal(p.stats().hits, 1); assert.equal(p.stats().deaths, 1); assert.equal(p.actors.has('1/0/enemy/11'), false);
  const positions = []; p.drawGhosts(actor => positions.push(actor.x)); assert.deepEqual(positions, [122]);
});

test('logical projectile ID prevents reused pool trails; expiry is neutral retirement', () => {
  const { p, a } = pair(); applyStrictSnapshot(p, a, snap(0, { projectiles: [projectile] }));
  const expired = evt('projectile_expired', 1, 0, { unitId: 1, enemyId: 10, projectileId: 20, role: 17, x: 120, y: 200 });
  const next = [...projectile]; next[6] = 21;
  const released = { ...release(1, 1), projectileId: 21 };
  applyStrictSnapshot(p, a, snap(1, { projectiles: [next], events: [expired, released] }));
  assert.equal(p.stats().retiredProjectiles, 1); assert.equal(p.stats().hits, 0); assert.equal(p.stats().deaths, 0); assert.equal(p.projectiles.size, 1);
  assert.equal(p.projectiles.has('1/0/projectile/20'), false); assert.equal(p.projectiles.get('1/0/projectile/21').trail.length, 1);
});

test('stable move and merge preserve destination identity and animate only exact events', () => {
  const { p, a } = pair(); const second = [...unit]; second[1] = 1; second[5] = 160; second[7] = 2;
  applyStrictSnapshot(p, a, snap(0, { units: [unit, second] }));
  const moved = [...unit]; moved[1] = 2; moved[5] = 220;
  applyStrictSnapshot(p, a, snap(1, { units: [moved, second], events: [evt('move', 1, 0, { unitId: 1, role: 17, value: 2, x: 220, y: 200 })] }));
  assert.equal(a.unitId(moved), '1/0/unit/1'); assert.equal(p.stats().moves, 1); assert.ok(p.pose(a.unitId(moved)).x < -100);
  advance(p, 200); assert.ok(Math.abs(p.pose(a.unitId(moved)).x) < 1);
  const merged = [...second]; merged[3] = 2;
  applyStrictSnapshot(p, a, snap(2, { units: [merged], events: [evt('merge', 2, 0, { unitId: 2, otherUnitId: 1, role: 17, value: 2, x: 160, y: 200 })] }));
  assert.equal(p.stats().merges, 1); assert.equal(p.stats().deaths, 0); assert.ok(p.actors.has('1/0/unit/2')); assert.equal(p.actors.has('1/0/unit/1'), false);
});

const bytes = fs.readFileSync(process.env.ORR_DEFENSE_WASM || new URL('../game.wasm', import.meta.url));
async function wasm() { return (await WebAssembly.instantiate(bytes, {})).instance.exports; }
function read(w) { const ptr = w.defense_snapshot(), length = w.defense_snapshot_len(); return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer, ptr, length))); }
test('candidate WASM stream produces exact cue counts and preserves simulation state', async () => {
  assert.ok(WebAssembly.validate(bytes), 'run against the WASM built from this candidate');
  console.log('WASM sha256: ' + crypto.createHash('sha256').update(bytes).digest('hex'));
  const [w, baseline] = await Promise.all([wasm(), wasm()]), { p, a } = pair(); w.defense_init(71825); baseline.defense_init(71825);
  applyStrictSnapshot(p, a, read(w)); let seq = 0; const counts = {}; let maximum = 0;
  for (let tick = 0; tick < 4000; tick++) {
    const input = tick % 90 === 0 ? [1, 0, 0, ++seq] : [0, 0, 0, 0]; w.defense_step(...input); baseline.defense_step(...input);
    const snapshot = freeze(read(w)), original = JSON.stringify(snapshot), result = applyStrictSnapshot(p, a, snapshot);
    assert.equal(result.status, 'applied', `tick ${tick}: ${result.reason}`);
    for (const e of snapshot.presentation.events) counts[e.kind] = (counts[e.kind] || 0) + 1;
    p.advance(1000 / 60); maximum = Math.max(maximum, p.stats().particles);
    assert.equal(JSON.stringify(snapshot), original); assert.deepEqual(read(w), read(baseline));
    assert.equal(applyStrictSnapshot(p, a, read(w)).status, 'duplicate');
  }
  assert.equal(p.stats().attacks, counts.release); assert.equal(p.stats().hits, counts.hit); assert.equal(p.stats().deaths, counts.death); assert.equal(p.stats().merges, counts.merge); assert.equal(p.stats().retiredProjectiles, counts.projectile_expired);
  assert.ok(maximum <= 256); assert.equal(a.stats().rejected, 0);
  console.log(JSON.stringify({ actualEngine: { ticks: 4000, checksum: read(w).checksum, counts, maximumParticles: maximum, presentation: p.stats(), transport: a.stats() } }));
});

test('actual WASM move/restart/source boundaries and terminal leaks match typed events', async () => {
  const w = await wasm(), { p, a } = pair(); w.defense_init(71825); applyStrictSnapshot(p, a, read(w));
  w.defense_step(1, 0, 0, 1); applyStrictSnapshot(p, a, read(w)); const id = read(w).units[0][7];
  w.defense_step(2, 0, 1, 2); const move = read(w); assert.equal(move.units[0][7], id); assert.equal(applyStrictSnapshot(p, a, move).status, 'applied'); assert.equal(p.stats().moves, 1);
  w.defense_step(5, 0, 0, 3); const restarted = read(w); assert.equal(restarted.tick, 0); assert.equal(restarted.presentation.tick, '3'); assert.equal(applyStrictSnapshot(p, a, restarted).reason, 'restart-generation');
  let deaths = 0, leaks = 0, finished = false;
  for (let tick = 0; tick < 5000 && !finished; tick++) {
    w.defense_step(0, 0, 0, 0); const s = read(w); assert.equal(applyStrictSnapshot(p, a, s).status, 'applied'); p.advance(1000 / 60);
    deaths += s.presentation.events.filter(e => e.kind === 'death').length; leaks += s.presentation.events.filter(e => e.kind === 'leak').length; finished = !!s.phase;
  }
  assert.ok(finished); assert.ok(leaks > 0); assert.equal(p.stats().deaths, deaths); assert.equal(p.stats().removals, leaks);
  advance(p, 700); assert.equal(p.stats().ghosts, 0); assert.equal(p.stats().particles, 0);
  w.defense_init(71825); assert.equal(applyStrictSnapshot(p, a, read(w)).reason, 'new-source'); assert.equal(a.stats().source, '2');
});
