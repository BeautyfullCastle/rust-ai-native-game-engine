// Real WASM producer contract. Optional old WASM proves gameplay is unchanged.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const binary = process.argv[2];
assert.ok(binary, 'usage: node tools/check_coop_defense_presentation.mjs new.wasm [baseline.wasm]');
async function load(path) {
  const w = (await WebAssembly.instantiate(fs.readFileSync(path), {})).instance.exports;
  return {
    init(seed) { w.defense_init(seed); },
    step(...input) { w.defense_step(...input); },
    read() {
      const p = w.defense_snapshot(), n = w.defense_snapshot_len();
      return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer, p, n)));
    },
  };
}
const game = await load(binary);
const peer = await load(binary);
const baseline = process.argv[3] && await load(process.argv[3]);
const legacy = s => {
  // maxHp and lastWave are additive authoring metadata absent in older ABI snapshots.
  const { presentation, checksum, maxHp, lastWave, ...state } = s;
  return { ...state, units: s.units.map(u => u.slice(0, 7)), projectiles: s.projectiles.map(p => p.slice(0, 6)) };
};
const kinds = new Set(['summon', 'merge', 'death', 'leak', 'wave', 'finish', 'restart', 'release', 'hit', 'projectile_expired', 'move', 'enemy_spawn']);
const coverage = new Set();
const shots = new Map();
let maximumBatch = 0;
function validate(s) {
  assert.equal(s.maxHp, 30, 'default Scenario HUD maximum (authored75 checked in scene-loading.test.mjs)');
  assert.equal(s.lastWave, 10, 'default Campaign keeps the historic ten-wave run');
  const p = s.presentation;
  assert.equal(p.version, 1);
  assert.equal(p.delivery, 'latest_tick');
  assert.equal(p.windup, false, 'no fabricated windup');
  assert.equal(p.status, 'complete');
  assert.equal(p.fullResync, false);
  assert.equal(p.dropped, 0);
  assert.ok(/^\d+$/.test(p.source) && /^\d+$/.test(p.tick));
  assert.ok(p.events.length <= 1024);
  maximumBatch = Math.max(maximumBatch, p.events.length);
  for (const [seq, e] of p.events.entries()) {
    assert.equal(e.tick, p.tick, 'engine EventKey is post-step tick');
    assert.equal(e.system, 0);
    assert.equal(e.seq, seq);
    assert.equal(e.generation, p.generation);
    assert.ok(kinds.has(e.kind));
    coverage.add(e.kind);
    if (e.kind === 'release') {
      assert.ok(e.unitId && e.projectileId && e.enemyId);
      assert.ok(!shots.has(e.projectileId), 'projectile lifetime cannot be reused');
      shots.set(e.projectileId, e);
    }
    if (['hit', 'death', 'projectile_expired'].includes(e.kind)) {
      const release = shots.get(e.projectileId);
      assert.ok(release, 'impact must reference a real release');
      assert.equal(e.unitId, release.unitId);
      assert.equal(e.role, release.role);
      if (e.kind === 'hit') assert.ok(e.value > 0);
      if (e.kind === 'death') assert.equal(e.hpAfter, 0);
    }
    if (e.kind === 'leak') {
      assert.ok(e.enemyId);
      assert.equal(e.projectileId, 0, 'leak is not a projectile kill');
      assert.equal(e.unitId, 0);
    }
  }
  for (const rows of [s.units.map(u => u[7]), s.projectiles.map(p => p[6])]) {
    assert.equal(new Set(rows).size, rows.length);
    assert.ok(rows.every(id => Number.isInteger(id) && id > 0));
  }
}
// Exact movement and reset namespace, including duplicate reads/held input.
game.init(71825);
game.step(1, 0, 0, 1);
let s = game.read();
validate(s);
const firstId = s.units[0][7], source = s.presentation.source;
assert.deepEqual(game.read(), s, 'reads must not drain or rewrite keys');
game.step(1, 0, 0, 1);
assert.ok(game.read().presentation.events.every(e => e.kind !== 'summon'));
game.step(2, 0, 11, 2);
s = game.read();
assert.equal(s.units[0][7], firstId);
assert.equal(s.presentation.events.find(e => e.kind === 'move').unitId, firstId);
game.step(5, 0, 0, 3);
s = game.read();
assert.equal(s.tick, 0);
assert.equal(s.presentation.tick, '4');
assert.equal(s.presentation.generation, 1);
assert.equal(s.presentation.source, source);
game.step(1, 0, 0, 4);
assert.equal(game.read().units[0][7], firstId, 'same numeric ID is safe only in new generation');
game.init(71825);
assert.notEqual(game.read().presentation.source, source);
assert.equal(game.read().presentation.generation, 0);
// Native parity fixture and real producer replay at every tick, not just final state.
for (const g of [game, peer, baseline].filter(Boolean)) g.init(71825);
shots.clear();
let sequence = 0;
for (let tick = 0; tick < 1200; tick++) {
  const input = tick % 30 === 0 ? [1, 0, 0, ++sequence] : [0, 0, 0, 0];
  for (const g of [game, peer, baseline].filter(Boolean)) g.step(...input);
  s = game.read();
  validate(s);
  const other = peer.read();
  const canonical = x => ({ ...x, presentation: { ...x.presentation, source: '<new-source>' } });
  assert.deepEqual(canonical(s), canonical(other));
  if (baseline) assert.deepEqual(legacy(s), legacy(baseline.read()), `gameplay changed at tick ${tick + 1}`);
}
// Historical pre-Scenario identity fixture was da0ca42edf9cf8a2; Scenario-v1 golden
// was 9056df75782da6ab before Campaign became a checksummed singleton.
// Native legacy seed/input trace independently reproduces the current 0x04ee32c4 build.
assert.equal(s.checksum, 'be2bdbfedfabd614', 'independently reproduced native Scenario+Campaign fixture');
assert.ok(coverage.has('release') && coverage.has('hit') && coverage.has('death'));
const parityChecksum = s.checksum;
// Real leaks must not become death events, including later terminal ticks.
for (const g of [game, baseline].filter(Boolean)) g.init(42);
shots.clear();
let leakTicks = 0;
for (let tick = 0; tick < 10000; tick++) {
  for (const g of [game, baseline].filter(Boolean)) g.step(0, 0, 0, 0);
  s = game.read();
  validate(s);
  if (baseline) assert.deepEqual(legacy(s), legacy(baseline.read()));
  if (s.presentation.events.some(e => e.kind === 'leak')) leakTicks++;
  if (s.phase !== 0) break;
}
assert.ok(leakTicks > 0);
assert.equal(s.phase, 2);
console.log(JSON.stringify({ passed: true, nativeParity: parityChecksum, gameplayBaselineCompared: !!baseline, maximumBatch, leakTicks, observedKinds: [...coverage].sort(), checks: ['stable move ID', 'distinct restart/source namespace', 'immutable duplicate read', 'duplicate input', 'exact EventKey forwarding', 'source/projectile/victim attribution', 'replayed event equality', 'no fabricated windup', 'leak is not death'] }, null, 2));
