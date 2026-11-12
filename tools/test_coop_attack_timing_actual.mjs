// Generated/composed/packed authoring -> unchanged bundled presentation + real WASM.
// DOM, Image, Canvas and File are mocks; cue calls are observed, not audible playback.
// This does not establish browser pixels, GPU, native-window or real-audio behavior.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';

const [htmlPath, defaultHtmlPath, manifestPath, defaultManifestPath, pngPath, wasmPath, scenePath] = process.argv.slice(2);
assert.ok(process.argv.length === 9 && scenePath,
  'usage: node tools/test_coop_attack_timing_actual.mjs AUTHORED_HTML DEFAULT_HTML COMPOSED_MANIFEST DEFAULT_MANIFEST PNG TRUSTED_WASM SCENE');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const plain = value => JSON.parse(JSON.stringify(value));
const png = fs.readFileSync(pngPath), wasmBytes = fs.readFileSync(wasmPath), sceneBytes = fs.readFileSync(scenePath);
assert.deepEqual(wasmBytes.subarray(0, 8), Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]), 'actual trusted WASM is required');
const authoredBytes = fs.readFileSync(manifestPath), defaultBytes = fs.readFileSync(defaultManifestPath);
const authoredManifest = JSON.parse(authoredBytes), defaultManifest = JSON.parse(defaultBytes);
const timing = {windupMs: 200, releaseMs: 50, recoveryMs: 150};
const defaultTiming = {windupMs: 110, releaseMs: 70, recoveryMs: 160};
const definition = authoredManifest['unit:0'];
assert.deepEqual(Object.keys(authoredManifest), ['enemy:0', 'unit:0']);
assert.deepEqual(Object.keys(defaultManifest), Object.keys(authoredManifest));
assert.deepEqual(definition.clips.attack.frames, [0, 2, 4, 6].map(x => [x, 0, 2, 2]));
assert.equal(definition.clips.attack.fps, 10);
assert.equal(definition.clips.attack.loop, false);
assert.deepEqual(definition.pivot, [1, 2]);
for (const [key, value] of Object.entries(timing)) {
  assert.equal(definition.clips.attack[key], value);
  assert.equal(Object.hasOwn(defaultManifest['unit:0'].clips.attack, key), false);
}
const withoutTiming = plain(authoredManifest);
for (const key of Object.keys(timing)) delete withoutTiming['unit:0'].clips.attack[key];
assert.deepEqual(withoutTiming, defaultManifest, 'control differs only by omitted attack timing');

async function boot(path, manifestBytes) {
  const html = fs.readFileSync(path, 'utf8'), elements = [], ui = new Map();
  const draws = [], imageSources = [], fetches = [], raf = [], cues = [], listeners = new Map();
  for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    const id = /\bid="([^"]*)"/.exec(match[1])?.[1];
    if (!id) continue;
    assert.ok(!elements.some(element => element.id === id), 'duplicate generated ID');
    elements.push({id, type: /\btype="([^"]*)"/.exec(match[1])?.[1], tagName: 'SCRIPT', textContent: match[2]});
  }
  const element = id => elements.find(value => value.id === id);
  assert.deepEqual(Buffer.from(element('wasm-core').textContent.trim(), 'base64'), wasmBytes);
  assert.deepEqual(Buffer.from(element('authored-animation-manifest').textContent.trim(), 'base64'), manifestBytes);
  const receipt = JSON.parse(element('authored-animation-receipt').textContent);
  assert.equal(receipt.version, 1);
  assert.equal(receipt.manifestBytes, manifestBytes.length);
  assert.equal(receipt.manifestSha256, sha(manifestBytes));
  assert.equal(receipt.images.length, 1);
  assert.deepEqual(receipt.images[0], {name: definition.image, bytes: png.length,
    sha256: sha(png), width: 8, height: 2, id: 'authored-animation-image-0'});
  assert.deepEqual(Buffer.from(element(receipt.images[0].id).textContent.trim(), 'base64'), png);
  const context = new Proxy({drawImage: (...args) => draws.push(args)},
    {get: (object, key) => object[key] ?? (() => {})});
  for (const id of ['game','codex','scene-file','summon','upgrade','invest','again','reset','begin','pause','sound','ready','result','notice','health','healthbar','wave','campaign-intro','gold','kills','selected','detail','outcome','summary','event-status','scene-source','loading','run-save','run-file','run-status']) {
    ui.set(id, {id, style: {}, span: {}, getContext: () => context,
      querySelector() { return this.span; }, addEventListener() {}, setAttribute() {}, hasPointerCapture: () => false});
  }
  const document = {hidden: false, baseURI: 'https://example.invalid/offline/',
    getElementById: id => element(id) || ui.get(id) || null,
    querySelectorAll(selector) {
      const prefix = selector.match(/^\[id\^="(.*)"\]$/), exact = selector.match(/^\[id="(.*)"\]$/);
      return elements.filter(value => prefix ? value.id.startsWith(prefix[1]) : exact ? value.id === exact[1] : false);
    }, addEventListener: (name, fn) => listeners.set(name, fn)};
  class MockImage {
    set src(value) {
      assert.ok(value.startsWith('data:image/png;base64,'), 'network image forbidden');
      imageSources.push(value);
      const bytes = Buffer.from(value.slice(22), 'base64');
      this.contentSha = sha(bytes);
      this.naturalWidth = bytes.readUInt32BE(16); this.naturalHeight = bytes.readUInt32BE(20);
      queueMicrotask(() => this.onload?.());
    }
  }
  const sandbox = vm.createContext({document, Image: MockImage, URL, Blob, performance, setTimeout, clearTimeout,
    fetch: async url => { fetches.push(String(url)); throw Error('network forbidden'); },
    TextDecoder, TextEncoder, Uint8Array, DataView, HTMLButtonElement: class {}, WebAssembly,
    crypto: webcrypto, atob, btoa, window: {}, console, testCues: cues,
    requestAnimationFrame: fn => raf.push(fn), addEventListener: (name, fn) => listeners.set(name, fn)});
  const run = code => vm.runInContext(code, sandbox);
  const source = element('game-module').textContent.replace(/^export (const|class|function) /mg, '$1 ')
    .replaceAll('import.meta.url', "'file:///offline/game.js'");
  const parts = source.split('try{let bytes;');
  assert.equal(parts.length, 2, 'review startup boundary if bundled source changes');
  run(parts[0]); await run('(async()=>{try{let bytes;' + parts[1] + '})()');
  assert.equal(run('state?.tick'), 0, ui.get('loading').textContent);
  assert.equal(run('ready && paused'), true);
  assert.equal(ui.get('loading').hidden, true);
  assert.deepEqual(plain(run('ATTACK_TIMING')), defaultTiming, 'existing fallback defaults remain unchanged');
  for (const [role, spec] of Object.entries(JSON.parse(manifestBytes))) {
    assert.deepEqual(plain(run(`presentation.manifest[${JSON.stringify(role)}]`)), spec);
  }
  // Forward every call to the real (muted) audio owner, only recording dispatch.
  run('const originalCue = presentation.audio.cue.bind(presentation.audio); presentation.audio.cue = (...args) => {testCues.push(args); return originalCue(...args);};');
  return {run, ui, draws, imageSources, fetches, raf, cues, listeners, sandbox};
}

const authored = await boot(htmlPath, authoredBytes), baseline = await boot(defaultHtmlPath, defaultBytes);
const both = [authored, baseline];
const gameplay = h => h.run('JSON.stringify((({presentation,...gameplay})=>gameplay)(state))');
let comparedTicks = 0;
function compareGameplay() {
  assert.equal(gameplay(authored), gameplay(baseline), 'same actual WASM and gameplay inputs remain identical');
  for (const h of both) {
    assert.equal(h.run('state.presentation.windup'), false, 'real WASM does not produce windup');
    assert.equal(h.run('visualAdapter.stats().rejected'), 0, 'real strict adapter must accept every snapshot');
  }
  comparedTicks++;
}
for (const h of both) {
  h.ui.get('begin').onclick({});
  assert.equal(h.run('command(1)'), true);
  h.run('execute(queue.shift());update();');
}
compareGameplay();
assert.ok(authored.run('state.units.some(u => u[0] === 0 && u[2] === 0)'), 'actual first Summon must produce unit:0');
let release;
for (let ticks = 0; ticks < 1800; ticks++) {
  for (const h of both) h.run('execute([0,0,0,0]);');
  compareGameplay();
  release = plain(authored.run('state.presentation.events.find(e => e.kind === "release" && e.board === 0 && e.role === 0) || null'));
  if (release) break;
}
assert.ok(release, 'real unit:0 release required within 1800 idle inputs; no synthetic engine fallback');
const releaseChecksum = authored.run('state.checksum');
for (const h of both) {
  h.sandbox.targetNumericId = release.unitId;
  h.run('globalThis.targetId = visualAdapter.unitId(state.units.find(u => u[7] === targetNumericId));');
  assert.equal(h.run('presentation.actors.get(targetId).attack.released'), true, 'real release cues immediately');
  assert.equal(h.run('presentation.counts.attacks'), h.cues.filter(([kind]) => kind === 'attack').length,
    'every strictly delivered real release dispatches exactly one attack cue');
  const cueCount = h.cues.length;
  assert.equal(h.run('observePresentation().status'), 'duplicate');
  assert.equal(h.cues.length, cueCount, 'duplicate strict snapshot cannot replay cue');
}
assert.deepEqual(plain(authored.run('presentation.actors.get(targetId).attack.timing')), timing);
assert.deepEqual(plain(baseline.run('presentation.actors.get(targetId).attack.timing')), defaultTiming);
const releaseCueCounts = both.map(h => h.cues.length), releaseSamples = [];
function sample(h, phase, clipTime, index) {
  const pose = plain(h.run('presentation.pose(targetId)')), frame = plain(h.run('presentation.frame(targetId)'));
  assert.equal(pose.phase, phase);
  if (clipTime !== null) assert.equal(pose.clipTime, clipTime);
  assert.equal(frame.clip, phase === 'idle' ? 'idle' : 'attack');
  assert.equal(frame.index, index);
  h.draws.length = 0;
  h.run('drawHero(state.units.find(u => u[7] === targetNumericId));');
  const selected = h.draws.filter(([image]) => image.contentSha === sha(png));
  assert.equal(selected.length, 1, 'target actor uses the exact authored atlas');
  assert.deepEqual(selected[0].slice(1), [...frame.rect, -35, -50, 70, 70], 'authored rectangle/pivot reach real paintFrame/drawImage');
  assert.equal(h.run('state.checksum'), releaseChecksum, 'presentation-only sampling must not step gameplay');
  return {phase: pose.phase, clipTime: pose.clipTime, frame: frame.index};
}
function sampleBoth(offset, authoredExpected, defaultExpected) {
  releaseSamples.push({offset, authored: sample(authored, ...authoredExpected), baseline: sample(baseline, ...defaultExpected)});
  both.forEach((h, i) => assert.equal(h.cues.length, releaseCueCounts[i], 'advance cannot replay release cue'));
}
function advanceBoth(ms) {
  assert.ok(ms >= 0 && ms <= 100, 'consumer advance clamps each call to 100 ms');
  both.forEach(h => h.run(`presentation.advance(${ms});`));
}
sampleBoth(0, ['release', 200, 2], ['release', 110, 1]);
advanceBoth(49);
sampleBoth(49, ['release', 249, 2], ['release', 159, 1]);
// Pause through the real UI/RAF boundary while the authored release is active.
for (const h of both) {
  h.run('togglePause();');
  const time = h.run('presentation.time'), pose = h.run('JSON.stringify(presentation.pose(targetId))');
  h.run('animate(1000);animate(60000);');
  assert.equal(h.run('presentation.time'), time);
  assert.equal(h.run('JSON.stringify(presentation.pose(targetId))'), pose);
  assert.equal(h.run('state.checksum'), releaseChecksum);
  h.run('togglePause();');
}
advanceBoth(1);
sampleBoth(50, ['recovery', 250, 2], ['release', 160, 1]);
advanceBoth(50);
sampleBoth(100, ['recovery', 300, 3], ['recovery', 210, 2]);
advanceBoth(100);
sampleBoth(200, ['idle', null, 0], ['recovery', 310, 3]);
advanceBoth(30);
sampleBoth(230, ['idle', null, 0], ['idle', null, 0]);

// Separate, explicitly synthetic onset exercises authored windup, which the real
// engine intentionally never emits. The bundled class and authored data are real.
authored.run(`globalThis.onsetCues = [];
  globalThis.onset = new CombatPresentation({manifest: presentation.manifest,
    audio: {reset(){}, setPaused(){}, setHidden(){}, cue(...args){onsetCues.push(args);}}});
  onset.observe({tick: 1, actors: [{id: 'synthetic-onset-unit', x: 0, y: 0, visualKey: 'unit:0'}],
    events: [{id: 'synthetic-onset-event', type: 'attack', actorId: 'synthetic-onset-unit', elapsedMs: 0}]});`);
const onsetSamples = [];
let onsetTime = 0;
for (const [time, phase, frame, cueCount] of [[0,'windup',0,0],[199,'windup',1,0],[200,'release',2,1],[249,'release',2,1],[250,'recovery',2,1],[300,'recovery',3,1],[400,'idle',0,1]]) {
  while (onsetTime < time) {
    const step = Math.min(100, time - onsetTime);
    authored.run(`onset.advance(${step});`); onsetTime += step;
  }
  assert.equal(authored.run('onset.pose("synthetic-onset-unit").phase'), phase);
  assert.equal(authored.run('onset.frame("synthetic-onset-unit").index'), frame);
  assert.equal(authored.run('onsetCues.length'), cueCount);
  onsetSamples.push({time, phase, frame, cueCount});
}
assert.equal(authored.run('state.checksum'), releaseChecksum);

// Hidden-page behavior, ordinary restart and real loadSceneFile stay on their
// existing paths. Neither lifecycle transition reloads authored animation data.
const sourceBeforeReset = authored.run('state.presentation.source');
const generationBeforeReset = authored.run('state.presentation.generation');
for (const h of both) {
  h.sandbox.document.hidden = true; h.listeners.get('visibilitychange')();
  const time = h.run('presentation.time');
  h.run('animate(60100);animate(60200);');
  assert.equal(h.run('presentation.time'), time);
  assert.equal(h.run('state.checksum'), releaseChecksum);
  assert.equal(h.run('presentation.hidden && paused'), true);
  h.sandbox.document.hidden = false; h.listeners.get('visibilitychange')();
  assert.equal(h.run('paused'), true, 'visibility restoration does not auto-resume');
  h.run('start();');
  assert.equal(h.run('presentation.time'), 0);
  assert.equal(h.run('presentation.counts.attacks'), 0);
  assert.equal(h.run('ready && paused'), true);
}
compareGameplay();
assert.equal(authored.run('state.presentation.source'), sourceBeforeReset);
assert.ok(authored.run('state.presentation.generation') > generationBeforeReset);
const restartChecksum = authored.run('state.checksum');
function fileOf(bytes, name) {
  const data = Uint8Array.from(bytes);
  return {name, size: data.length, arrayBuffer: async () => data.buffer.slice(0)};
}
for (const h of both) {
  const before = h.run('JSON.stringify([state,seq,queue,runRecorder.initial])');
  h.sandbox.localScene = fileOf(Buffer.from('not a Scene'), 'invalid.yaml');
  await h.run('loadSceneFile(localScene)');
  assert.equal(h.run('JSON.stringify([state,seq,queue,runRecorder.initial])'), before, 'invalid local scene preserves existing game');
  h.sandbox.localScene = fileOf(sceneBytes, 'timing-scene.yaml');
  await h.run('loadSceneFile(localScene)');
  assert.equal(h.ui.get('scene-source').textContent, 'timing-scene.yaml');
  assert.equal(h.run('state.tick'), 0);
  assert.equal(h.run('ready && paused'), true);
  assert.equal(h.run('presentation.time'), 0);
  assert.equal(h.run('runRecorder.initial.kind'), 'scene');
  assert.deepEqual(Buffer.from(h.run('runRecorder.initial.base64'), 'base64'), sceneBytes);
}
compareGameplay();
assert.notEqual(authored.run('state.presentation.source'), sourceBeforeReset, 'new local scene has a fresh strict identity');
const sceneChecksum = authored.run('state.checksum');
for (const h of both) {
  h.ui.get('begin').onclick({}); assert.equal(h.run('command(1)'), true);
  h.run('execute(queue.shift());update();');
}
compareGameplay();
for (const [h, manifest] of [[authored, authoredManifest], [baseline, defaultManifest]]) {
  for (const [role, spec] of Object.entries(manifest)) assert.deepEqual(plain(h.run(`presentation.manifest[${JSON.stringify(role)}]`)), spec);
  assert.equal(h.fetches.length, 0);
}
console.log(JSON.stringify({version: 1, passed: true, manifestSha256: sha(authoredBytes),
  defaultManifestSha256: sha(defaultBytes), pngSha256: sha(png), wasmSha256: sha(wasmBytes),
  realRelease: {engineTick: release.tick, unitId: release.unitId, presentationStartsAt: 200, releaseSamples},
  syntheticOnset: onsetSamples, comparedTicks, releaseChecksum, restartChecksum, sceneChecksum,
  networkRequests: 0, cueCoverage: 'dispatch spy forwards to real muted audio; no playback claim',
  coverage: 'actual supplied WASM + unchanged bundled JS; mock DOM/Image/Canvas/File, not browser/GPU'}));
