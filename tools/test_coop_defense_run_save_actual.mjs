// Focused Run Save v1 acceptance: actual supplied WASM, bundled JS, mock DOM.
// Does not establish browser file-picker, pixels, GPU, or a preemptive WASM timeout.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';
const [htmlPath, wasmPath, scenePath] = process.argv.slice(2);
assert.ok(htmlPath && wasmPath && scenePath,
  'usage: node test_coop_defense_run_save_actual.mjs HTML TRUSTED_WASM AUTHORED_SCENE');
const html = fs.readFileSync(htmlPath, 'utf8');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const plain = value => JSON.parse(JSON.stringify(value));
const sceneBytes = fs.readFileSync(scenePath);
assert.ok(sceneBytes.length <= 65536);
async function boot() {
  const elements = [], ui = new Map(), draws = [], imageSources = [], fetches = [], raf = [], canvasListeners = new Map();
  for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    const id = /\bid="([^"]*)"/.exec(match[1])?.[1];
    if (!id || id.startsWith('authored-animation-') || id === 'authored-scene' || id === 'authored-scene-receipt') continue;
    assert.ok(!elements.some(element => element.id === id), 'duplicate generated ID');
    elements.push({id, type: /\btype="([^"]*)"/.exec(match[1])?.[1],
      tagName: 'SCRIPT', textContent: match[2]});
  }
  const element = id => elements.find(value => value.id === id);
  assert.equal(sha(Buffer.from(element('wasm-core').textContent.trim(), 'base64')),
    sha(fs.readFileSync(wasmPath)));
  const context = new Proxy({drawImage: (...args) => draws.push(args)},
    {get: (object, key) => object[key] ?? (() => {})});
  for (const id of ['game','codex','scene-file','summon','upgrade','invest','again','reset','begin','pause','sound','ready','result','notice','health','healthbar','wave','campaign-intro','gold','kills','selected','detail','outcome','summary','event-status','scene-source','loading','run-save','run-file','run-status']) {
    ui.set(id, {id, style: {}, span: {}, getContext: () => context,
      querySelector() { return this.span; },
      addEventListener(name, fn) { if (id === 'game') canvasListeners.set(name, fn); }, setAttribute() {},
      hasPointerCapture: () => false});
  }
  const blobs = new Map(), downloads = [], revoked = [], listeners = new Map();
  class MockURL extends URL {
    static createObjectURL(blob) { const url = 'blob:run-' + blobs.size; blobs.set(url, blob); return url; }
    static revokeObjectURL(url) { revoked.push(url); }
  }
  const document = {hidden: false, baseURI: 'https://example.invalid/game/',
    getElementById: id => element(id) || ui.get(id) || null,
    querySelectorAll(selector) {
      const prefix = selector.match(/^\[id\^="(.*)"\]$/), exact = selector.match(/^\[id="(.*)"\]$/);
      return elements.filter(value => prefix ? value.id.startsWith(prefix[1]) : exact ? value.id === exact[1] : false);
    }, addEventListener(name, fn) { listeners.set(name, fn); },
    createElement(tag) { assert.equal(tag, 'a'); return {click() { downloads.push({href: this.href, download: this.download}); }, remove() {}}; },
    body: {appendChild() {}}};
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
  const sandbox = vm.createContext({document, Image: MockImage, URL: MockURL, Blob, performance: {now: () => performance.now()}, setTimeout, clearTimeout,
    fetch: async url => { fetches.push(String(url)); throw Error('network forbidden'); },
    TextDecoder, TextEncoder, Uint8Array, DataView, HTMLButtonElement: class {}, WebAssembly: {instantiate: WebAssembly.instantiate},
    crypto: webcrypto, atob, btoa, window: {}, console,
    requestAnimationFrame: fn => raf.push(fn), addEventListener(name, fn) { listeners.set(name, fn); }});
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
  return {run, ui, draws, fetches, imageSources, raf, sandbox, blobs, downloads, revoked, listeners, canvasListeners};
}

function fileOf(bytes) {
  const data = Uint8Array.from(bytes);
  return {name: 'local-run.json', size: data.length,
    arrayBuffer: async () => data.buffer.slice(0),
    text: async () => new TextDecoder('utf-8', {fatal: true}).decode(data)};
}
async function load(h, text) {
  h.sandbox.testFile = fileOf(new TextEncoder().encode(text));
  return await h.run('loadRunFile(testFile)');
}
const identity = h => h.run('JSON.stringify([state,seq,queue,runRecorder.initial,runRecorder.steps,paused,ready])');
const results = [];
for (const initialKind of ['seed', 'scene']) {
  const h = await boot();
  if (initialKind === 'scene') {
    h.sandbox.testFile = fileOf(sceneBytes);
    await h.run('loadSceneFile(testFile)');
    assert.equal(h.run('state.tick'), 0);
  }
  const initial = JSON.parse(h.run('runSaveText()'));
  assert.equal(initial.initial.kind, initialKind);
  if (initialKind === 'scene') assert.deepEqual(Buffer.from(initial.initial.base64, 'base64'), sceneBytes);
  assert.equal(initial.steps.length, 0);
  assert.equal(await load(h, JSON.stringify(initial)), true, 'zero-step restore');
  assert.equal(h.run('paused && !ready'), true);
  h.run('paused=false;');
  assert.equal(h.run('command(1)'), true);
  h.run('execute(queue.shift());execute([0,0,0,0]);');
  // Execute through the real boundary: game acceptance/refusal is recorded alike.
  h.run('execute([2,0,1,++seq]);execute([3,0,0,++seq]);');
  h.run('start();');
  h.ui.get('begin').onclick({});
  assert.equal(h.run('command(1)'), true);
  h.run('execute(queue.shift());');
  // Queue high-water survives discarding a queued action; it is not replayed.
  assert.equal(h.run('command(1)'), true);
  h.run('queue=[];');
  const text = h.run('runSaveText()'), saved = JSON.parse(text);
  assert.equal(saved.steps.length, 6);
  assert.deepEqual(saved.steps.map(step => step[0]), [1,0,2,3,5,1]);
  assert.ok(saved.sequence > Math.max(...saved.steps.map(step => step[3])));
  assert.equal(saved.wasmSha256, sha(fs.readFileSync(wasmPath)));
  const savedChecksum = h.run('state.checksum');
  h.run('execute([0,0,0,0]);');
  const nextChecksum = h.run('state.checksum');
  // Real download bytes and filename, then the local File-shaped read boundary.
  assert.equal(await load(h, text), true);
  h.run('downloadRun();');
  assert.equal(h.downloads.length, 1);
  assert.equal(h.downloads[0].download, 'stormglass-run-v1.json');
  const downloaded = h.blobs.get(h.downloads[0].href);
  assert.ok(downloaded instanceof Blob);
  assert.deepEqual(JSON.parse(await downloaded.text()), saved);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(h.revoked.includes(h.downloads[0].href), 'download URL revoked');
  // Populate UI-only transient data to prove a successful swap clears it.
  h.run('queue=[[1,0,0,++seq]];pending={kind:"test"};selected=1;acc=55;last=42;visualAdapter.counters.rejected=99;presentation.advance(100);');
  h.sandbox.testFile = fileOf(new Uint8Array(await downloaded.arrayBuffer()));
  assert.equal(await h.run('loadRunFile(testFile)'), true);
  assert.equal(h.run('state.checksum'), savedChecksum);
  assert.equal(h.run('paused && !ready && queue.length===0 && pending===null && selected===-1 && acc===0 && last===0'), true);
  assert.equal(h.run('presentation.time'), 0);
  assert.equal(h.run('visualAdapter.stats().rejected'), 0);
  assert.equal(h.run('visualAdapter.stats().source === state.presentation.source'), true);
  assert.equal(h.run('seq'), saved.sequence);
  h.run('execute([0,0,0,0]);');
  assert.equal(h.run('state.checksum'), nextChecksum);
  // Invalid candidates must not mutate the current live instance or queued input.
  h.run('queue=[[1,0,0,++seq]];');
  const before = identity(h);
  h.run('globalThis.originalWasm = wasm;');
  for (const malformed of [
    '{',
    JSON.stringify({...saved, version: 2}),
    JSON.stringify({...saved, wasmSha256: '0'.repeat(64)}),
    JSON.stringify({...saved, checksum: '0'.repeat(16)}),
    JSON.stringify({...saved, initial: {kind: 'scene', base64: btoa('not a Scene')}}),
  ]) {
    assert.equal(await load(h, malformed), false);
    assert.equal(identity(h), before);
    assert.equal(h.run('wasm===originalWasm'), true);
  }
  // Candidate-only WASM trap: the original instance and queued command stay intact.
  h.sandbox.WebAssembly.instantiate = async (...args) => {
    const actual = await WebAssembly.instantiate(...args);
    return {instance: {exports: {...actual.instance.exports,
      defense_step() { throw new WebAssembly.RuntimeError('controlled candidate trap'); }}}};
  };
  assert.equal(await load(h, text), false);
  assert.match(h.ui.get('notice').textContent, /controlled candidate trap/);
  assert.equal(identity(h), before);
  assert.equal(h.run('wasm===originalWasm'), true);
  h.sandbox.WebAssembly.instantiate = WebAssembly.instantiate;
  // Advance the cooperative clock, not 36,000 real combat ticks.
  let clock = 0;
  h.sandbox.performance.now = () => (clock += 6000);
  assert.equal(await load(h, text), false);
  assert.match(h.ui.get('notice').textContent, /deadline/);
  assert.equal(identity(h), before);
  assert.equal(h.run('wasm===originalWasm'), true);
  h.sandbox.performance.now = () => performance.now();
  // Invalidate a pending read via a newer file selection. Neither failed read mutates live state.
  let release;
  const blockedRead = new Promise(resolve => { release = resolve; });
  h.sandbox.testFile = {name:'pending.json',size:text.length,
    arrayBuffer:async()=>{await blockedRead;return new TextEncoder().encode(text).buffer;},
    text:async()=>{await blockedRead;return text;}};
  const stale = h.run('loadRunFile(testFile)');
  assert.equal(h.run('command(1)'), false);
  h.run('execute([0,0,0,0]);');
  assert.equal(identity(h), before, 'pending load guards live mutation without clearing the queue');
  assert.equal(await load(h, '{'), false);
  release();
  assert.equal(await stale, false);
  assert.equal(identity(h), before);
  assert.equal(h.run('wasm===originalWasm'), true);
  assert.equal(h.fetches.length, 0);
  // Focus loss is a lifecycle event, not part of candidate admission rollback.
  // Exercise the actual WASM with a deferred file read, never an OS/browser claim.
  for (const outcome of ['failure', 'success', 'cancel', 'restart', 'ordinary']) {
    const live = await boot();
    assert.equal(await load(live, text), true);
    live.ui.get('pause').onclick();
    assert.equal(live.run('paused'), false);
    assert.equal(live.run('command(1)'), true);
    live.run('gesture={id:7};hover=1;acc=12;');
    let releasedCapture = 0;
    live.ui.get('game').hasPointerCapture = id => id === 7;
    live.ui.get('game').releasePointerCapture = id => {
      assert.equal(id, 7); releasedCapture++;
    };
    const authoritative = () => live.run('JSON.stringify([state,seq,runRecorder.initial,runRecorder.steps])');
    const prior = authoritative();
    let finishRead, loading;
    if (outcome !== 'ordinary') {
      const reading = new Promise(resolve => { finishRead = resolve; });
      live.sandbox.testFile = {name:'pending.json',size:text.length,arrayBuffer:()=>reading};
      loading = live.run('loadRunFile(testFile)');
      assert.equal(live.run('runImportActive'), true);
    }
    live.listeners.get('blur')();
    assert.equal(live.run('paused'), true, `${outcome}: focus loss must pause pending restore`);
    assert.equal(live.run('queue.length'), 0, `${outcome}: discard pending purchase`);
    assert.equal(live.run('gesture===null && hover===-1 && acc===0'), true);
    assert.equal(releasedCapture, 1);
    assert.equal(authoritative(), prior, 'blur preserves simulation, sequence and replay history');
    if (outcome === 'cancel') assert.equal(await load(live, '{'), false);
    if (outcome === 'restart') live.ui.get('reset').onclick();
    const afterLifecycle = authoritative();
    if (loading) {
      finishRead(new TextEncoder().encode(outcome === 'failure' ? '{' : text).buffer);
      assert.equal(await loading, outcome === 'success');
    }
    assert.equal(live.run('paused'), true, `${outcome}: restoration must not undo focus pause`);
    assert.equal(live.run('queue.length'), 0);
    assert.equal(live.run('runImportActive || runImportBusy'), false);
    if (outcome === 'success') assert.equal(live.run('state.checksum'), savedChecksum);
    else assert.equal(authoritative(), afterLifecycle);
    const pausedState = live.run('JSON.stringify(state)');
    live.run('animate(1);animate(35);');
    assert.equal(live.run('JSON.stringify(state)'), pausedState, 'frames cannot advance before explicit resume');
    const humanUnits = live.run('state.units.filter(u=>u[0]===0).length');
    const tickBeforeResume = live.run('state.tick');
    if (outcome === 'restart') live.ui.get('begin').onclick({});
    else live.ui.get('pause').onclick();
    live.run('animate(100);animate(135);');
    assert.ok(live.run('state.tick') > tickBeforeResume, 'explicit resume advances combat');
    assert.equal(live.run('state.units.filter(u=>u[0]===0).length'), humanUnits,
      'resume must not replay the discarded Summon');
    assert.equal(live.fetches.length, 0);
  }
  const pointerReleaseCases = [];
  for (const eventName of ['pointerup', 'pointercancel', 'lostpointercapture']) {
    for (const outcome of ['failure', 'cancel', 'success', 'restart']) {
      const live = await boot();
      if (initialKind === 'scene') {
        live.sandbox.testFile = fileOf(sceneBytes);
        await live.run('loadSceneFile(testFile)');
      }
      live.ui.get('begin').onclick({});
      assert.equal(live.run('command(1)'), true);
      live.run('execute(queue.shift());');
      const restoreText = live.run('runSaveText()');
      const canvas = live.ui.get('game');
      canvas.getBoundingClientRect = () => ({left:0, top:0, width:500, height:720});
      const captured = new Set();
      canvas.setPointerCapture = id => captured.add(id);
      canvas.hasPointerCapture = id => captured.has(id);
      let releases = 0;
      const releaseStates = [];
      canvas.releasePointerCapture = id => {
        releases++;
        releaseStates.push(live.run('gesture===null && hover===-1'));
        captured.delete(id);
        live.canvasListeners.get('lostpointercapture')({pointerId:id});
        if (eventName === 'pointerup' && outcome === 'failure') throw Error('capture already lost');
      };
      const pointer = (name, id, cell, dx=0) => {
        const [x,y] = live.run(`project(...state.cells.find(c=>c[0]===0&&c[1]===${cell}).slice(2),0)`);
        live.canvasListeners.get(name)({pointerId:id, button:0, clientX:x+dx,
          clientY:y, preventDefault(){}});
      };
      pointer('pointerdown', 7, 0);pointer('pointermove', 7, 0, 20);
      assert.equal(live.run('gesture.drag'), true, 'real handlers create the drag');
      assert.equal(live.run('command(1)'), true, 'unexecuted purchase must survive cleanup');
      let finishRead;
      live.sandbox.testFile = {name:'pending.json',size:restoreText.length,
        arrayBuffer:()=>new Promise(resolve=>{finishRead=resolve;})};
      const loading = live.run('loadRunFile(testFile)');
      const prior = identity(live);
      assert.equal(live.run('command(1)'), false, 'import still blocks gameplay commands');
      assert.equal(identity(live), prior);
      const transient = live.run('JSON.stringify([gesture,hover,selected])');
      pointer(eventName, 99, 1);
      assert.equal(live.run('JSON.stringify([gesture,hover,selected])'), transient,
        `${eventName}: unrelated pointer cannot cancel active drag`);
      assert.equal(releases, 0);
      if (eventName === 'lostpointercapture') captured.delete(7);
      pointer(eventName, 7, 1);
      assert.equal(live.run('gesture===null && hover===-1'), true,
        `${eventName}/${outcome}: pending import must not suppress pointer cleanup`);
      assert.equal(identity(live), prior, 'cleanup preserves state, queue, sequence and recorder');
      assert.equal(releases, eventName === 'lostpointercapture' ? 0 : 1);
      assert.ok(releaseStates.every(Boolean), 'logical state clears before capture callbacks');
      if (outcome === 'cancel') assert.equal(await load(live, '{'), false);
      if (outcome === 'restart') live.ui.get('reset').onclick();
      const afterLifecycle = identity(live);
      finishRead(new TextEncoder().encode(outcome === 'failure' ? '{' : restoreText).buffer);
      assert.equal(await loading, outcome === 'success');
      assert.equal(live.run('runImportActive || runImportBusy'), false);
      if (outcome !== 'success') assert.equal(identity(live), afterLifecycle);
      assert.equal(live.run('gesture===null && hover===-1'), true);
      if (outcome === 'restart') {
        live.ui.get('begin').onclick({});
        assert.equal(live.run('command(1)'), true);
        live.run('execute(queue.shift());');
      } else if (outcome === 'success') live.ui.get('pause').onclick();
      // A fresh down/up with different endpoints is not a drag. Neither a new
      // nor a recycled pointer ID can release the old drag as a phantom move.
      for (const id of [8,7]) {
        const beforePointer = identity(live);
        pointer('pointerdown', id, 0);
        assert.equal(live.run('gesture.id'), id, 'new pointer is accepted');
        if (id === 8) {
          const activeGesture = live.run('JSON.stringify([gesture,hover,selected])');
          const activeState = identity(live);
          const activeReleases = releases;
          for (const staleEvent of ['lostpointercapture', 'pointercancel', 'pointerup']) {
            pointer(staleEvent, 7, 1);
            assert.equal(live.run('JSON.stringify([gesture,hover,selected])'), activeGesture,
              `${staleEvent}: old pointer cannot cancel a new gesture outside import`);
            assert.equal(identity(live), activeState);
            assert.equal(releases, activeReleases);
          }
        }
        pointer('pointerup', id, 1);
        assert.equal(identity(live), beforePointer, 'no stale move command');
        assert.equal(live.run('gesture'), null);
      }
      // A deliberate new drag still reaches the real game after restoration.
      const queued = live.run('queue.length');
      pointer('pointerdown', 9, 0);pointer('pointermove', 9, 11);pointer('pointerup', 9, 11);
      assert.equal(live.run('queue.length'), queued + 1);
      assert.equal(live.run('queue[queue.length-1][0]'), 2);
      live.run('while(queue.length)execute(queue.shift());');
      assert.equal(live.run('!!unitAt(11) && !unitAt(0)'), true, 'fresh move is consumed by WASM');
      assert.equal(live.fetches.length, 0);
      assert.ok(releaseStates.every(Boolean), 'all capture releases observe cleared logical state');
      pointerReleaseCases.push({event:eventName, outcome});
    }
  }
  results.push({initialKind, steps: saved.steps.length, sequence: saved.sequence,
    savedChecksum, nextChecksum, networkRequests: h.fetches.length,
    focusLossOutcomes: ['failure', 'success', 'cancel', 'restart', 'ordinary'], pointerReleaseCases});
}
console.log(JSON.stringify({version:1, wasmSha256:sha(fs.readFileSync(wasmPath)),
  sceneSha256:sha(sceneBytes), results,
  coverage:'actual supplied WASM + bundled JavaScript; mock DOM/Blob/File, not a real browser picker or GPU; cooperative timeout is not synchronous-call preemption'}));
