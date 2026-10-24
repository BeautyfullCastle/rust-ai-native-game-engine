// Explicit legacy-only fixture adapter. Never imported by v10 game.js.
// Temporary adapter for the unchanged v7 Rust ABI. No economy/progression logic.
// The ABI has enemy IDs, but no stable unit/projectile IDs or combat event stream.
export const unitVisualId = u => `unit:${u[0]}:${u[1]}:${u[2]}:${u[3]}`;
export const enemyVisualId = e => `enemy:${e[0]}`;
const tint = key => { let n = 0; for (const c of String(key)) n = (n * 31 + c.charCodeAt(0)) >>> 0; return `hsl(${n % 360} 68% 67%)`; };
export class LegacySnapshotAdapter {
  constructor({ project = (x, y) => [x, y], scale = () => 1, projectilePosition, colorFor = tint } = {}) {
    this.project = project; this.scale = scale; this.projectilePosition = projectilePosition; this.colorFor = colorFor; this.reset();
  }
  reset() { this.previous = null; this.projectiles = []; this.serial = 0; }
  observe(state) {
    if (!state) return { tick: 0, actors: [], projectiles: [], events: [] };
    if (this.previous && state.tick < this.previous.tick) this.reset();
    const old = this.previous;
    const actors = [], events = [];
    const oldUnits = new Map((old?.units || []).map(u => [unitVisualId(u), u]));
    const oldEnemies = new Map((old?.enemies || []).map(e => [e[0], e]));
    for (const u of state.units || []) {
      const id = unitVisualId(u), p = this.project(u[5], u[6], u[0]), key = `unit:${u[2]}`;
      actors.push({ id, x: p[0], y: p[1], scale: this.scale(u[0]), color: this.colorFor(key), visualKey: key, moving: false, renderData: [...u] });
      const before = oldUnits.get(id);
      // Cooldown rises only after a successful spawn of a projectile in this ABI.
      // We have already observed release, so do not invent an earlier windup.
      const projectile = (state.projectiles || []).find(p => p[0] === u[0] && p[1] === u[2] && Math.hypot(p[3] - u[5], p[4] - u[6]) < 2);
      if (old && u[4] > (before?.[4] || 0) && (before || projectile)) {
        const target = projectile && (state.enemies || []).find(e => e[0] === projectile[2]);
        const to = target ? this.project(target[6], target[7], target[1]) : [p[0] + 1, p[1]];
        events.push({ id: `${state.tick}:attack:${id}`, type: 'attack', actorId: id, phase: 'release', dx: to[0] - p[0], dy: to[1] - p[1], evidence: 'cooldown-rise-release-observed' });
      }
    }
    const enemyIds = new Set();
    for (const e of state.enemies || []) {
      enemyIds.add(e[0]);
      const id = enemyVisualId(e), p = this.project(e[6], e[7], e[1]), before = oldEnemies.get(e[0]), key = `enemy:${e[2]}`;
      actors.push({ id, x: p[0], y: p[1], scale: this.scale(e[1]), color: this.colorFor(key), visualKey: key, moving: !!before && (before[6] !== e[6] || before[7] !== e[7]), renderData: [...e] });
      if (before && e[3] < before[3]) events.push({ id: `${state.tick}:hit:${id}`, type: 'hit', actorId: id, evidence: 'snapshot-hp-decreased' });
    }
    const removed = (old?.enemies || []).filter(e => !enemyIds.has(e[0]));
    // Batch attribution is safe only when every retirement is corroborated as a
    // kill, with no leaks in that observation. Mixed/unknown causes stay neutral.
    const allDeaths = removed.length > 0 && state.kills - old.kills === removed.length && state.leaks === old.leaks;
    for (const e of removed) events.push({ id: `${state.tick}:retire:${e[0]}`, type: allDeaths ? 'death' : 'remove', actorId: enemyVisualId(e), evidence: allDeaths ? 'all-removed-correlated-kill-count-no-leaks' : 'unknown-or-leak-retirement' });
    const used = new Set(), projectiles = [], tracks = [];
    for (const row of (state.projectiles || []).slice(0, 128)) {
      // Cosmetic nearest-neighbour continuity, never source/target/damage authority.
      let candidate = null, distance = Infinity;
      for (const previous of this.projectiles) {
        if (used.has(previous.id) || previous.row[0] !== row[0] || previous.row[1] !== row[1] || previous.row[2] !== row[2]) continue;
        const d = Math.hypot(previous.row[3] - row[3], previous.row[4] - row[4]);
        if (d <= 40 && d < distance) { candidate = previous; distance = d; }
      }
      const id = candidate?.id || `projectile:${++this.serial}`;
      used.add(id);
      const p = this.projectilePosition ? this.projectilePosition(row, state) : this.project(row[3], row[4], row[0]);
      projectiles.push({ id, x: p[0], y: p[1], color: this.colorFor(`unit:${row[1]}`), size: 3.2 * Math.max(.65, this.scale(row[0])) });
      tracks.push({ id, row: [...row] });
    }
    this.projectiles = tracks;
    // Retain the minimum history; do not keep a mutable reference to Rust output.
    this.previous = { tick: state.tick, kills: state.kills, leaks: state.leaks, units: (state.units || []).map(u => [...u]), enemies: (state.enemies || []).map(e => [...e]) };
    return { tick: state.tick, actors, projectiles, events };
  }
}
