// Focused authored-CLI delivery acceptance: real WASM, mock DOM/Image/Canvas.
// This does not establish browser pixels, GPU or native-window behavior.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';

const [htmlPath, manifestPath, pngPath, wasmPath] = process.argv.slice(2);
assert.ok(htmlPath && manifestPath && pngPath && wasmPath,
  'usage: node test_compose_coop_defense_animations_actual.mjs HTML MANIFEST PNG TRUSTED_WASM');
const html = fs.readFileSync(htmlPath, 'utf8');
const manifestBytes = fs.readFileSync(manifestPath), png = fs.readFileSync(pngPath);
const manifest = JSON.parse(manifestBytes), definition = manifest['unit:0'];
assert.deepEqual(Object.keys(manifest), ['enemy:0', 'unit:0']);
assert.deepEqual(manifest['enemy:0'], definition);
assert.deepEqual(definition.clips.idle.frames, [[0, 0, 2, 2], [2, 0, 2, 2]]);
assert.deepEqual(definition.pivot, [1, 2]);
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const plain = value => JSON.parse(JSON.stringify(value));

async function boot(authored = true) {
  const elements = [], ui = new Map(), draws = [], imageSources = [], fetches = [], raf = [];
  for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    const id = /\bid="([^"]*)"/.exec(match[1])?.[1];
    if (!id || (!authored && id.startsWith('authored-animation-'))) continue;
    assert.ok(!elements.some(element => element.id === id), 'duplicate generated ID');
    elements.push({id, type: /\btype="([^"]*)"/.exec(match[1])?.[1],
      tagName: 'SCRIPT', textContent: match[2]});
  }
  const element = id => elements.find(value => value.id === id);
  assert.equal(sha(Buffer.from(element('wasm-core').textContent.trim(), 'base64')),
    sha(fs.readFileSync(wasmPath)));
  if (authored) {
    assert.deepEqual(Buffer.from(element('authored-animation-manifest').textContent.trim(), 'base64'), manifestBytes);
    const receipt = JSON.parse(element('authored-animation-receipt').textContent);
    assert.equal(receipt.manifestSha256, sha(manifestBytes));
    assert.equal(receipt.images.length, 1);
    assert.equal(receipt.images[0].name, definition.image);
    assert.equal(receipt.images[0].sha256, sha(png));
    assert.deepEqual(Buffer.from(element(receipt.images[0].id).textContent.trim(), 'base64'), png);
  }
  const context = new Proxy({drawImage: (...args) => draws.push(args)},
    {get: (object, key) => object[key] ?? (() => {})});
  for (const id of ['game','codex','scene-file','summon','upgrade','invest','again','reset','begin','pause','sound','ready','result','notice','health','healthbar','wave','campaign-intro','gold','kills','selected','detail','outcome','summary','event-status','scene-source','loading','run-save','run-file','run-status']) {
    ui.set(id, {id, style: {}, span: {}, getContext: () => context,
      querySelector() { return this.span; }, addEventListener() {}, setAttribute() {},
      hasPointerCapture: () => false});
  }
  const document = {hidden: false, baseURI: 'https://example.invalid/game/',
    getElementById: id => element(id) || ui.get(id) || null,
    querySelectorAll(selector) {
      const prefix = selector.match(/^\[id\^="(.*)"\]$/), exact = selector.match(/^\[id="(.*)"\]$/);
      return elements.filter(value => prefix ? value.id.startsWith(prefix[1]) : exact ? value.id === exact[1] : false);
    }, addEventListener() {}};
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
  const sandbox = vm.createContext({document, Image: MockImage, URL,
    fetch: async url => { fetches.push(String(url)); throw Error('network forbidden'); },
    TextDecoder, TextEncoder, Uint8Array, DataView, HTMLButtonElement: class {}, WebAssembly,
    crypto: webcrypto, performance, Blob, setTimeout, atob, btoa, window: {}, console,
    requestAnimationFrame: fn => raf.push(fn), addEventListener() {}});
  const run = code => vm.runInContext(code, sandbox);
  const source = element('game-module').textContent
    .replace(/^export (const|class|function) /mg, '$1 ')
    .replaceAll('import.meta.url', "'file:///game.js'");
  const parts = source.split('try{let bytes;');
  assert.equal(parts.length, 2, 'review startup boundary if bundled source changes');
  run(parts[0]); await run('(async()=>{try{let bytes;' + parts[1] + '})()');
  assert.equal(run('state.tick'), 0);
  assert.equal(run('ready && paused'), true);
  assert.equal(ui.get('loading').hidden, true);
  return {run, ui, draws, fetches, imageSources, raf};
}

const authored = await boot(), baseline = await boot(false);
for (const h of [authored, baseline]) {
  h.ui.get('begin').onclick({});
  assert.equal(h.run('command(1)'), true);
  h.run('execute(queue.shift());update();');
}
assert.equal(authored.run('state.checksum'), baseline.run('state.checksum'));
assert.ok(authored.run('state.units.some(u => u[0] === 0 && u[2] === 0)'), 'real Summon must produce unit:0');
// Obtain live enemies from identical actual WASM idle inputs, not hand-made actors.
for (const h of [authored, baseline]) h.run('for(let i=0;i<320;i++)execute([0,0,0,0]);update();');
assert.equal(authored.run('state.checksum'), baseline.run('state.checksum'));
assert.ok(authored.run('state.enemies.some(e => e[2] === 0)'), 'actual enemy:0 must spawn');
const checksum = authored.run('state.checksum');
const seen = new Map([['unit:0', new Set()], ['enemy:0', new Set()]]);
for (let sample = 0; sample < 4; sample++) {
  if (sample) authored.run('presentation.advance(100);');
  const frames = plain(authored.run('[...presentation.actors].map(([id,actor]) => ({role:actor.actor.visualKey,frame:presentation.frame(id)}))'));
  for (const {role, frame} of frames) {
    if (seen.has(role) && frame?.image === definition.image) seen.get(role).add(frame.index);
  }
  authored.draws.length = 0; authored.run('draw();');
  const selected = authored.draws.filter(([image]) => image.contentSha === sha(png));
  for (const size of [70, 37]) assert.ok(selected.some(([, x,y,w,h,dx,dy,dw,dh]) =>
    [0,2].includes(x) && y === 0 && w === 2 && h === 2 &&
    dx === -size/2 && dy === 20-size && dw === size && dh === size),
    'both live unit and enemy rectangles/pivots reach drawImage');
  assert.equal(authored.run('state.checksum'), checksum);
}
for (const [role, frames] of seen) assert.deepEqual([...frames].sort(), [0,1], role + ' samples both authored frames');
const frameSamples = Object.fromEntries([...seen].map(([role, frames]) => [role,[...frames].sort()]));
authored.run('togglePause();');
const pausedTime = authored.run('presentation.time');
authored.run('animate(1000);animate(60000);');
assert.equal(authored.run('presentation.time'), pausedTime);
assert.equal(authored.run('state.checksum'), checksum);
const generation = authored.run('state.presentation.generation');
for (const h of [authored, baseline]) h.run('start();');
assert.equal(authored.run('presentation.time'), 0);
assert.equal(authored.run('ready && paused'), true);
assert.ok(authored.run('state.presentation.generation') > generation);
assert.equal(authored.run('state.checksum'), baseline.run('state.checksum'));
for(const role of Object.keys(manifest)) assert.deepEqual(plain(authored.run(`presentation.manifest[${JSON.stringify(role)}]`)), manifest[role]);
assert.equal(authored.fetches.length + baseline.fetches.length, 0);
console.log(JSON.stringify({version: 1, manifestSha256: sha(manifestBytes), pngSha256: sha(png),
  wasmSha256: sha(fs.readFileSync(wasmPath)), frameSamples, presentationChecksum: checksum,
  restartChecksum: authored.run('state.checksum'), networkRequests: 0,
  coverage: 'actual WASM + bundled JS; mock DOM/Image/Canvas, not browser/GPU'}));
