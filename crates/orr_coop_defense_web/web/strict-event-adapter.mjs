// Consumer of Orrery's verified local ABI v1, not an event bus or simulation.
// No cooldown/HP-difference/projectile-matching inference exists in this module.
const U32 = 0xffffffff, U64 = 18446744073709551615n;
const KINDS = new Set(['summon', 'merge', 'death', 'leak', 'wave', 'finish', 'restart', 'release', 'hit', 'projectile_expired', 'move', 'enemy_spawn']);
const unsigned = (n, max = U32) => Number.isInteger(n) && n >= 0 && n <= max;
const signed = n => Number.isInteger(n) && n >= -2147483648 && n <= 2147483647;
const decimal = value => typeof value === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(value) && BigInt(value) <= U64;
const fingerprintEvent = e => [e.tick, e.system, e.seq, e.kind, e.generation, e.unitId, e.otherUnitId, e.enemyId, e.projectileId, e.board, e.role, e.value, e.hpAfter, e.x, e.y];
const color = key => { let n = 0; for (const c of String(key)) n = (n * 31 + c.charCodeAt(0)) >>> 0; return `hsl(${n % 360} 68% 67%)`; };
export const eventKey = (source, generation, event) => `${source}/${generation}/${event.tick}/${event.system}/${event.seq}`;
export const actorKey = (source, generation, kind, id) => `${source}/${generation}/${kind}/${id}`;

export class StrictEventAdapter {
  constructor({ project = (x, y) => [x, y], scale = () => 1, colorFor = color } = {}) {
    this.project = project; this.scale = scale; this.colorFor = color; this.reset();
  }
  reset() {
    this.anchor = null; this.signature = null; this.needsResync = false; this.revision = 0;
    this.lastActors = new Map(); this.lastProjectiles = new Map();
    this.counters = { applied: 0, duplicateBatches: 0, duplicateEvents: 0, rejected: 0, resyncs: 0, delivered: 0, kinds: {} };
    this.lastStatus = 'waiting'; this.lastReason = 'initial';
  }
  unitId(row) { return this.anchor ? actorKey(this.anchor.source, this.anchor.generation, 'unit', row[7]) : 'unavailable'; }
  enemyId(row) { return this.anchor ? actorKey(this.anchor.source, this.anchor.generation, 'enemy', row[0]) : 'unavailable'; }
  projectileId(row) { return this.anchor ? actorKey(this.anchor.source, this.anchor.generation, 'projectile', row[6]) : 'unavailable'; }
  stats() { return { mode: 'strict-engine-v1', status: this.lastStatus, reason: this.lastReason, source: this.anchor?.source || null, generation: this.anchor?.generation ?? null, engineTick: this.anchor?.tick || null, ...this.counters, kinds: { ...this.counters.kinds }, pendingResync: this.needsResync, windup: false }; }
  result(status, reason, reset, observation = null) { this.lastStatus = status; this.lastReason = reason; return { status, reason, reset, observation }; }
  reject(reason) {
    this.counters.rejected++; this.needsResync = true; this.lastActors.clear(); this.lastProjectiles.clear();
    return this.result('rejected', reason, true);
  }
  validate(snapshot) {
    const p = snapshot?.presentation;
    if (!p || p.version !== 1) throw Error('missing-or-unsupported-v1');
    if (!decimal(p.source) || p.source === '0' || !decimal(p.tick) || !unsigned(p.generation)) throw Error('invalid-epoch-or-tick');
    if (p.delivery !== 'latest_tick' || p.windup !== false || typeof p.fullResync !== 'boolean' || !unsigned(p.dropped) || !['complete', 'overflow'].includes(p.status)) throw Error('invalid-delivery-contract');
    if (!Array.isArray(p.events) || p.events.length > 1024) throw Error('invalid-event-budget');
    if (p.status === 'complete' && p.dropped !== 0 || p.status === 'overflow' && (!p.fullResync || p.dropped === 0 || p.events.length !== 0)) throw Error('contradictory-delivery-status');
    for (const key of ['units', 'enemies', 'projectiles']) if (!Array.isArray(snapshot[key])) throw Error(`missing-${key}`);
    if (snapshot.units.length + snapshot.enemies.length > 384 || snapshot.projectiles.length > 128) throw Error('unsupported-snapshot-budget');
    const unique = (rows, idIndex, name, valid) => {
      const seen = new Set();
      for (const row of rows) {
        if (!Array.isArray(row) || row.length !== 8 || !valid(row) || row[idIndex] === 0 || seen.has(row[idIndex])) throw Error(`invalid-${name}-identity-or-row`);
        seen.add(row[idIndex]);
      }
    };
    unique(snapshot.units, 7, 'unit', r => [0, 1, 2, 3, 4, 7].every(i => unsigned(r[i])) && signed(r[5]) && signed(r[6]));
    unique(snapshot.enemies, 0, 'enemy', r => [0, 1, 2, 3, 4, 5].every(i => unsigned(r[i])) && signed(r[6]) && signed(r[7]));
    unique(snapshot.projectiles, 6, 'projectile', r => [0, 1, 2, 5, 6, 7].every(i => unsigned(r[i])) && r[7] > 0 && r[2] > 0 && signed(r[3]) && signed(r[4]));
    const keys = new Map(), events = [], sequences = new Map(); let sequenceGap = false, duplicateEvents = 0;
    for (const e of p.events) {
      if (!e || !decimal(e.tick) || e.tick !== p.tick || !unsigned(e.system, 65535) || !unsigned(e.seq) || !KINDS.has(e.kind) || e.generation !== p.generation) throw Error('invalid-event-key-or-kind');
      if (!['unitId', 'otherUnitId', 'enemyId', 'projectileId', 'board', 'role', 'value', 'hpAfter'].every(k => unsigned(e[k])) || !signed(e.x) || !signed(e.y)) throw Error('invalid-event-payload');
      if (['summon', 'move', 'merge', 'release', 'hit', 'death', 'projectile_expired'].includes(e.kind) && e.unitId === 0) throw Error('missing-event-unit-id');
      if (['release', 'hit', 'death', 'leak', 'enemy_spawn', 'projectile_expired'].includes(e.kind) && e.enemyId === 0) throw Error('missing-event-enemy-id');
      if (['release', 'hit', 'death', 'projectile_expired'].includes(e.kind) && e.projectileId === 0) throw Error('missing-event-projectile-id');
      if (e.kind === 'merge' && (e.otherUnitId === 0 || e.otherUnitId === e.unitId)) throw Error('invalid-merge-identity');
      if (e.kind !== 'merge' && e.otherUnitId !== 0) throw Error('unexpected-consumed-unit-id');
      if (['leak', 'enemy_spawn', 'wave', 'finish', 'restart'].includes(e.kind) && e.unitId !== 0) throw Error('unexpected-event-unit-id');
      if (['summon', 'move', 'merge', 'wave', 'finish', 'restart'].includes(e.kind) && e.enemyId !== 0) throw Error('unexpected-event-enemy-id');
      if (!['release', 'hit', 'death', 'projectile_expired'].includes(e.kind) && e.projectileId !== 0) throw Error('unexpected-event-projectile-id');
      if (e.kind === 'death' && e.hpAfter !== 0) throw Error('death-with-live-hp');
      const key = eventKey(p.source, p.generation, e), fingerprint = JSON.stringify(fingerprintEvent(e));
      if (keys.has(key)) { if (keys.get(key) !== fingerprint) throw Error('conflicting-duplicate-event-key'); duplicateEvents++; continue; }
      const expected = sequences.has(e.system) ? sequences.get(e.system) + 1 : 0;
      if (e.seq !== expected) sequenceGap = true;
      sequences.set(e.system, e.seq); keys.set(key, fingerprint); events.push({ ...e });
    }
    const signature = JSON.stringify([p.version, p.source, p.generation, p.tick, p.status, p.dropped, p.fullResync, events.map(fingerprintEvent), snapshot.units, snapshot.enemies, snapshot.projectiles]);
    return { p, events, signature, sequenceGap, duplicateEvents };
  }
  normalizedActor(kind, row, p, moving = false) {
    const isUnit = kind === 'unit', id = isUnit ? row[7] : row[0], board = isUnit ? row[0] : row[1], key = `${kind}:${row[2]}`;
    const point = this.project(row[isUnit ? 5 : 6], row[isUnit ? 6 : 7], board), factor = this.scale(board);
    if (!Array.isArray(point) || point.length !== 2 || !point.every(Number.isFinite) || !Number.isFinite(factor) || factor <= 0) throw Error('invalid-view-transform');
    return { id: actorKey(p.source, p.generation, kind, id), x: point[0], y: point[1], scale: factor, color: this.colorFor(key), visualKey: key, moving, renderData: [...row] };
  }
  normalize(snapshot, p, events, allowEvents) {
    const actors = [], map = new Map();
    for (const row of snapshot.units) { const a = this.normalizedActor('unit', row, p); actors.push(a); map.set(a.id, a); }
    for (const row of snapshot.enemies) {
      const a = this.normalizedActor('enemy', row, p), prior = this.lastActors.get(a.id);
      // Movement is a render pose, not a hit/death/attack decision.
      a.moving = !!prior && (prior.x !== a.x || prior.y !== a.y);
      actors.push(a); map.set(a.id, a);
    }
    const projectiles = snapshot.projectiles.map(row => {
      const position = this.project(row[3], row[4], row[0]);
      if (!Array.isArray(position) || position.length !== 2 || !position.every(Number.isFinite)) throw Error('invalid-projectile-view-transform');
      // Exactly the observed projectile position and ID. No nearest shooter or
      // target interpolation is guessed to compensate for the legacy map.
      return { id: actorKey(p.source, p.generation, 'projectile', row[6]), sourceActorId: actorKey(p.source, p.generation, 'unit', row[7]), targetActorId: actorKey(p.source, p.generation, 'enemy', row[2]), x: position[0], y: position[1], color: this.colorFor(`unit:${row[1]}`), size: 3.2 * Math.max(.65, this.scale(row[0])) };
    });
    const output = [], lethalHits = new Map();
    const projectileRows = new Map(snapshot.projectiles.map(row => [row[6], row]));
    const eventActors = new Map();
    for (const e of events) if (['enemy_spawn', 'death', 'leak'].includes(e.kind)) {
      const a = this.normalizedActor('enemy', [e.enemyId, e.board, e.value, e.hpAfter, Math.max(1, e.hpAfter), 0, e.x, e.y], p);
      eventActors.set(a.id, a);
    }
    if (allowEvents) for (const e of events) {
      const id = eventKey(p.source, p.generation, e), position = this.project(e.x, e.y, e.board);
      if (!Array.isArray(position) || position.length !== 2 || !position.every(Number.isFinite)) throw Error('invalid-event-view-transform');
      const unitId = actorKey(p.source, p.generation, 'unit', e.unitId), enemyId = actorKey(p.source, p.generation, 'enemy', e.enemyId);
      const shared = { id, engineKind: e.kind, engineKey: { source: p.source, generation: p.generation, tick: e.tick, system: e.system, seq: e.seq }, x: position[0], y: position[1], projectileId: actorKey(p.source, p.generation, 'projectile', e.projectileId), sourceActorId: unitId, targetActorId: enemyId, value: e.value, hpAfter: e.hpAfter };
      if (e.kind === 'release') {
        const target = map.get(enemyId), unit = map.get(unitId);
        if (!unit || !target) throw Error('release-not-backed-by-live-actors');
        const projectile = projectileRows.get(e.projectileId);
        if (!projectile || projectile[7] !== e.unitId || projectile[2] !== e.enemyId || projectile[0] !== e.board || projectile[1] !== e.role || projectile[5] !== e.value || projectile[3] !== e.x || projectile[4] !== e.y) throw Error('release-projectile-mismatch');
        output.push({ ...shared, type: 'attack', actorId: unitId, phase: 'release', dx: target.x - position[0], dy: target.y - position[1] });
      } else if (e.kind === 'hit') {
        const victim = map.get(enemyId) || this.lastActors.get(enemyId) || eventActors.get(enemyId), projectile = this.lastProjectiles.get(e.projectileId);
        if (!victim || !projectile || projectile[7] !== e.unitId || projectile[1] !== e.role) throw Error('hit-attribution-mismatch');
        if (e.hpAfter === 0) lethalHits.set(`${e.enemyId}/${e.projectileId}/${e.unitId}`, e);
        output.push({ ...shared, type: 'hit', actorId: enemyId, actor: victim });
      } else if (e.kind === 'death' || e.kind === 'leak') {
        // Explicit event contains the victim kind and final position even when
        // an actor spawned and retired entirely inside this same verified tick.
        const fallbackActor = this.normalizedActor('enemy', [e.enemyId, e.board, e.value, e.hpAfter, Math.max(1, e.hpAfter), 0, e.x, e.y], p);
        if (map.has(enemyId)) throw Error('retired-enemy-still-active');
        if (e.kind === 'death') {
          const key = `${e.enemyId}/${e.projectileId}/${e.unitId}`, lethal = lethalHits.get(key);
          if (!lethal || lethal.role !== e.role || lethal.board !== e.board || lethal.x !== e.x || lethal.y !== e.y) throw Error('death-without-preceding-lethal-hit');
          lethalHits.delete(key);
        }
        output.push({ ...shared, type: e.kind === 'death' ? 'death' : 'remove', actorId: enemyId, actor: map.get(enemyId) || this.lastActors.get(enemyId) || fallbackActor });
      } else if (e.kind === 'projectile_expired') {
        const projectile = this.lastProjectiles.get(e.projectileId);
        if (!projectile || projectile[7] !== e.unitId || projectile[2] !== e.enemyId || projectileRows.has(e.projectileId)) throw Error('projectile-retirement-mismatch');
        output.push({ ...shared, type: 'projectile_retire' });
      } else if (e.kind === 'move') {
        const unit = map.get(unitId);
        if (!unit || !this.lastActors.has(unitId) || unit.renderData[1] !== e.value) throw Error('move-identity-mismatch');
        output.push({ ...shared, type: 'move', actorId: unitId });
      } else if (e.kind === 'merge') {
        const otherActorId = actorKey(p.source, p.generation, 'unit', e.otherUnitId), unit = map.get(unitId);
        if (!unit || !this.lastActors.has(unitId) || !this.lastActors.has(otherActorId) || map.has(otherActorId) || unit.renderData[3] !== e.value) throw Error('merge-identity-mismatch');
        output.push({ ...shared, type: 'merge', actorId: unitId, otherActorId });
      }
      else if (e.kind === 'summon' || e.kind === 'enemy_spawn') {
        const spawnActor = e.kind === 'enemy_spawn' ? this.normalizedActor('enemy', [e.enemyId, e.board, e.value, e.hpAfter, Math.max(1, e.hpAfter), 0, e.x, e.y], p) : map.get(unitId);
        output.push({ ...shared, type: 'spawn', actorId: e.kind === 'summon' ? unitId : enemyId, actor: spawnActor });
      }
      else output.push({ ...shared, type: 'status', statusKind: e.kind });
    }
    return { observation: { tick: ++this.revision, engineTick: p.tick, actors, projectiles, events: output }, map };
  }
  consume(snapshot) {
    let checked;
    try { checked = this.validate(snapshot); } catch (error) { return this.reject(error.message); }
    const { p, events, signature, sequenceGap, duplicateEvents } = checked, last = this.anchor;
    const tick = BigInt(p.tick);
    if (last && p.source === last.source && p.generation === last.generation && p.tick === last.tick && !this.needsResync) {
      if (signature !== this.signature) return this.reject('same-tick-conflict');
      this.counters.duplicateBatches++; this.counters.duplicateEvents += duplicateEvents;
      return this.result('duplicate', 'same-verified-batch', false);
    }
    let reason = null;
    if (!last) reason = 'initial-snapshot';
    else if (p.source !== last.source) reason = 'new-source';
    else if (p.generation !== last.generation) reason = events.some(e => e.kind === 'restart') ? 'restart-generation' : 'generation-discontinuity';
    else if (tick < BigInt(last.tick)) reason = 'backward-engine-tick';
    else if (tick !== BigInt(last.tick) + 1n) reason = 'engine-tick-gap';
    if (this.needsResync) reason = 'recover-after-rejection';
    if (p.fullResync || p.status === 'overflow') reason = p.status === 'overflow' ? 'event-overflow' : 'explicit-full-resync';
    if (sequenceGap) reason = 'event-sequence-gap';
    const hasRestart = events.some(e => e.kind === 'restart');
    if (hasRestart && last && p.source === last.source && p.generation === last.generation) return this.reject('restart-without-generation-change');
    // A complete batch is staged before handing anything to the FX module.
    let normalized;
    try { normalized = this.normalize(snapshot, p, events, !reason); } catch (error) { return this.reject(error.message); }
    this.anchor = { source: p.source, generation: p.generation, tick: p.tick }; this.signature = signature;
    this.needsResync = false; this.lastActors = normalized.map; this.lastProjectiles = new Map(snapshot.projectiles.map(row => [row[6], [...row]]));
    this.counters.duplicateEvents += duplicateEvents;
    if (reason) { this.counters.resyncs++; return this.result('resync', reason, true, normalized.observation); }
    this.counters.applied++; this.counters.delivered += normalized.observation.events.length;
    for (const event of events) this.counters.kinds[event.kind] = (this.counters.kinds[event.kind] || 0) + 1;
    return this.result('applied', 'contiguous-verified-tick', false, normalized.observation);
  }
}

export function applyStrictSnapshot(presentation, adapter, snapshot) {
  const result = adapter.consume(snapshot);
  if (result.reset) presentation.reset();
  if (result.observation) presentation.observe(result.observation);
  return result;
}
