// Actual authoring CLI -> offline snapshot -> runtime consumer acceptance.
// Synthetic user input only; WebAudio and strict event snapshots are mocked.
// No browser, hardware playback or WASM coverage is claimed by this check.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { CoopDefenseAudio, loadAuthoredAudio } from '../crates/orr_coop_defense_web/web/audio/coop-defense-audio.mjs';
import { StrictEventAdapter } from '../crates/orr_coop_defense_web/web/strict-event-adapter.mjs';

const tools = fileURLToPath(new URL('.', import.meta.url));
const cues = ['attack-light', 'attack-heavy', 'attack-magic', 'impact-light',
  'impact-heavy', 'summon', 'merge', 'ui-confirm', 'ui-reject', 'death'];
function python(args) {
  const result = spawnSync(process.env.PYTHON || 'python3', args, {
    cwd: tools, encoding: 'utf8', env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, `Python failed: ${result.stderr}\n${result.stdout}`);
  return result.stdout;
}
function context() {
  return { state: 'suspended', currentTime: 0, destination: {}, sources: [], buffers: [],
    createGain() { return { gain: { value: 0 }, connect() {}, disconnect() {} }; },
    createBuffer(channels, length, rate) {
      const buffer = { channels, length, rate, copyToChannel(pcm) { this.samples = pcm.slice(); } };
      this.buffers.push(buffer); return buffer;
    },
    createBufferSource() {
      const source = { connect() {}, disconnect() {}, start() { this.started = true; }, stop() { this.stopped = true; } };
      this.sources.push(source); return source;
    },
    async resume() { this.state = 'running'; }, async suspend() { this.state = 'suspended'; },
    async close() { this.state = 'closed'; },
  };
}
function snapshot(tick, kind = null) {
  return { units: tick ? [[0, 0, 0, 1, 0, 100, 200, 1]] : [], enemies: [], projectiles: [],
    presentation: { version: 1, source: '1', generation: 0, tick: String(tick),
      delivery: 'latest_tick', status: 'complete', dropped: 0, fullResync: false, windup: false,
      events: kind ? [{ tick: String(tick), system: 0, seq: 0, kind, generation: 0,
        unitId: 1, otherUnitId: 0, enemyId: 0, projectileId: 0, board: 0, role: 0,
        value: 0, hpAfter: 0, x: 100, y: 200 }] : [] } };
}
const temporary = mkdtempSync(join(tmpdir(), 'orr-audio-authoring-actual-'));
try {
  const output = join(temporary, 'new-bank'), license = join(temporary, 'user-license.txt');
  const licenseBytes = Buffer.from('Synthetic test PCM authored solely for this test. CC0-1.0.\n');
  writeFileSync(license, licenseBytes);
  const inputs = new Map(), samples = new Map();
  const args = [resolve(tools, 'create_coop_defense_audio_bank.py'), '--license', license, '--output', output];
  for (const [index, cue] of cues.entries()) {
    const values = [0, index + 1, -(index + 1), 8192 - index, -8192 + index, 317 + index];
    const pcm = Buffer.alloc(8 + values.length * 2);
    pcm.writeUInt32LE(48000); pcm.writeUInt32LE(values.length, 4);
    values.forEach((value, frame) => pcm.writeInt16LE(value, 8 + frame * 2));
    const path = join(temporary, `user-${index}.pcm`);
    writeFileSync(path, pcm); inputs.set(cue, { path, pcm }); samples.set(cue, values);
    args.push('--cue', `${cue}=${path}`);
  }
  python(args);
  assert.deepEqual(readdirSync(output).sort(), ['bank.json', 'LICENSE.txt', ...cues.map(cue => `${cue}.pcm`)].sort());
  assert.deepEqual(readFileSync(join(output, 'LICENSE.txt')), licenseBytes);
  for (const [cue, input] of inputs) {
    assert.deepEqual(readFileSync(input.path), input.pcm, 'input must remain unchanged');
    assert.deepEqual(readFileSync(join(output, `${cue}.pcm`)), input.pcm, 'authored samples must not be replaced');
  }
  // Use the real offline packer's bounded no-follow readers and strict JSON parser.
  const script = python(['-c', `import sys\nfrom pack_coop_defense_audio import snapshot_audio\nfrom pack_coop_defense_offline import snapshot_file, strict_json\nprint(snapshot_audio(sys.argv[1], snapshot_file, strict_json))`, join(output, 'bank.json')]);
  const match = /^<script type="application\/json" id="authored-audio-bank">([\s\S]*)<\/script>\s*$/.exec(script);
  assert.ok(match, 'actual packer must produce one embedded bank script');
  const ctx = context();
  const audio = await loadAuthoredAudio({
    getElementById: id => id === 'authored-audio-bank' ? { textContent: match[1] } : null,
    querySelectorAll: () => [{ id: 'authored-audio-bank' }],
  }, { contextFactory: () => ctx });
  assert.ok(audio instanceof CoopDefenseAudio);
  assert.equal(audio.stats().contextState, 'not-created');
  await audio.setMuted(false);
  assert.equal(await audio.unlock({ isTrusted: true, type: 'click' }), true);
  assert.equal(ctx.buffers.length, 10);
  for (const [index, cue] of cues.entries()) {
    assert.equal(ctx.buffers[index].channels, 1); assert.equal(ctx.buffers[index].rate, 48000);
    assert.deepEqual(Array.from(ctx.buffers[index].samples), samples.get(cue).map(value => value / 32768));
  }
  const adapter = new StrictEventAdapter();
  const send = value => {
    const before = JSON.stringify(value), result = adapter.consume(value);
    audio.observe(result, value.presentation);
    assert.equal(JSON.stringify(value), before, 'consumer must not mutate input snapshots');
    return result;
  };
  send(snapshot(0));
  const first = snapshot(1, 'summon');
  assert.equal(send(first).status, 'applied');
  assert.equal(ctx.sources.length, 1); assert.equal(ctx.sources[0].buffer, ctx.buffers[5]);
  assert.deepEqual(Array.from(ctx.sources[0].buffer.samples), samples.get('summon').map(value => value / 32768));
  assert.equal(send(first).status, 'duplicate'); assert.equal(ctx.sources.length, 1);
  for (const [method, tick] of [['setMuted', 2], ['setPaused', 3]]) {
    await audio[method](true); assert.equal(audio.stats().voices, 0);
    const suppressed = snapshot(tick, 'summon');
    assert.equal(send(suppressed).status, 'applied'); assert.equal(ctx.sources.length, 1);
    await audio[method](false);
    assert.equal(send(suppressed).status, 'duplicate'); assert.equal(ctx.sources.length, 1, 'suppressed event must not replay');
  }
  assert.equal(audio.stats().suppressed, 2);
  assert.equal(send(snapshot(4, 'summon')).status, 'applied'); assert.equal(ctx.sources.length, 2);
  assert.equal(audio.cue('ui-confirm'), false); assert.equal(ctx.sources.length, 2);
  assert.equal(send(snapshot(5, 'ui-confirm')).status, 'rejected', 'UI confirm is not an engine event');
  assert.equal(ctx.sources.length, 2); assert.equal(audio.stats().voices, 0);
  assert.equal(audio.stats().adapterError, null);
  audio.dispose(); assert.equal(ctx.state, 'closed');
  console.log(JSON.stringify({ result: 'passed', cues: 10, strictSummonPlayed: 2,
    suppressedWithoutReplay: 2, authoringCLI: 'actual', offlineSnapshot: 'actual',
    runtimeConsumer: 'actual', eventInput: 'synthetic', webAudio: 'mock' }));
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
