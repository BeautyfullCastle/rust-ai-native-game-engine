// Focused authored-CLI delivery acceptance: real WASM, mock DOM/Image/Canvas.
// This does not establish browser pixels, GPU or native-window behavior.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';

const [htmlPath, manifestPath, pngPath, wasmPath] = process.argv.slice(2);
assert.ok(htmlPath && manifestPath && pngPath && wasmPath,
  'usage: node test_create_coop_defense_animation_actual.mjs HTML MANIFEST PNG TRUSTED_WASM');
const html = fs.readFileSync(htmlPath, 'utf8');
const manifestBytes = fs.readFileSync(manifestPath), png = fs.readFileSync(pngPath);
const manifest = JSON.parse(manifestBytes), definition = manifest['unit:0'];
assert.deepEqual(Object.keys(manifest), ['unit:0']);
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
  for (const id of ['game','codex','scene-file','summon','upgrade','invest','again','reset','begin','pause','sound','ready','result','notice','health','healthbar','wave','campaign-intro','gold','kills','selected','detail','outcome','summary','event-status','scene-source','loading']) {
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
    crypto: webcrypto, atob, btoa, window: {}, console,
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
const checksum = authored.run('state.checksum');
const frameSamples = [];
for (let frame = 0; frame < 2; frame++) {
  if (frame) authored.run('presentation.advance(100);');
  const frames = plain(authored.run('[...presentation.actors].map(([id]) => presentation.frame(id))'));
  assert.ok(frames.some(value => value?.image === definition.image && value.index === frame));
  authored.draws.length = 0; authored.run('draw();');
  const selected = authored.draws.filter(([image]) => image.contentSha === sha(png));
  assert.ok(selected.some(([, x,y,w,h,dx,dy,dw,dh]) =>
    x === frame * 2 && y === 0 && w === 2 && h === 2 &&
    dx === -35 && dy === -50 && dw === 70 && dh === 70), 'authored rectangle and pixel pivot reach drawImage');
  assert.equal(authored.run('state.checksum'), checksum);
  frameSamples.push(frame);
}
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
assert.deepEqual(plain(authored.run('presentation.manifest["unit:0"]')), definition);
assert.equal(authored.fetches.length + baseline.fetches.length, 0);
console.log(JSON.stringify({version: 1, manifestSha256: sha(manifestBytes), pngSha256: sha(png),
  wasmSha256: sha(fs.readFileSync(wasmPath)), frameSamples, presentationChecksum: checksum,
  restartChecksum: authored.run('state.checksum'), networkRequests: 0,
  coverage: 'actual WASM + bundled JS; mock DOM/Image/Canvas, not browser/GPU'}));
