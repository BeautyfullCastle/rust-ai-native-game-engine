// Focused authored PCM + Run Save acceptance: actual WASM, bundled JS, mock DOM/Web Audio.
// Does not establish browser file-picker, pixels, GPU, or a preemptive WASM timeout.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';
const [audioHtmlPath, proceduralHtmlPath] = process.argv.slice(2);
const wasmPath = process.env.ORR_DEFENSE_WASM;
assert.ok(audioHtmlPath && proceduralHtmlPath && wasmPath,
  'usage: ORR_DEFENSE_WASM=TRUSTED_WASM node test_coop_defense_authored_restore_actual.mjs AUDIO_HTML NO_AUDIO_HTML');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const plain = value => JSON.parse(JSON.stringify(value));
const trustedWasmHash = sha(fs.readFileSync(wasmPath));
async function boot(htmlPath, corruptAudio=false, startupHide=null) {
  const html = fs.readFileSync(htmlPath, 'utf8');
  const audioContexts = [];
  class MockAudioContext {
    constructor() { this.state='suspended';this.currentTime=0;this.destination={};this.sources=[];this.buffers=[];audioContexts.push(this); }
    createGain() { return {gain:{value:0},connect(){},disconnect(){}}; }
    createBuffer(channels,length,rate) { assert.equal(channels,1);assert.equal(rate,48000);const buffer={length,rate,copyToChannel(pcm){this.samples=pcm.slice();}};this.buffers.push(buffer);return buffer; }
    createBufferSource() { const source={connect(){},disconnect(){},start(){this.started=true;},stop(){this.stopped=true;}};this.sources.push(source);return source; }
    async resume(){this.state='running';} async suspend(){this.state='suspended';} async close(){this.state='closed';}
  }
  const elements = [], ui = new Map(), draws = [], imageSources = [], fetches = [], raf = [];
  for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    const id = /\bid="([^"]*)"/.exec(match[1])?.[1];
    if (!id || id.startsWith('authored-animation-') || id === 'authored-scene' || id === 'authored-scene-receipt') continue;
    assert.ok(!elements.some(element => element.id === id), 'duplicate generated ID');
    elements.push({id, type: /\btype="([^"]*)"/.exec(match[1])?.[1],
      tagName: 'SCRIPT', textContent: match[2]});
  }
  const element = id => elements.find(value => value.id === id);
  if(corruptAudio) element('authored-audio-bank').textContent='{}';
  assert.equal(sha(Buffer.from(element('wasm-core').textContent.trim(), 'base64')),
    sha(fs.readFileSync(wasmPath)));
  const context = new Proxy({drawImage: (...args) => draws.push(args)},
    {get: (object, key) => object[key] ?? (() => {})});
  for (const id of ['game','codex','scene-file','summon','upgrade','invest','again','reset','begin','pause','sound','ready','result','notice','health','healthbar','wave','campaign-intro','gold','kills','selected','detail','outcome','summary','event-status','scene-source','loading','run-save','run-file','run-status']) {
    ui.set(id, {id, style: {}, span: {}, getContext: () => context,
      querySelector() { return this.span; }, addEventListener() {}, setAttribute() {},
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
      if(startupHide==='atlas' && imageSources.length===1) listeners.get('pagehide')?.({persisted:false});
      const bytes = Buffer.from(value.slice(22), 'base64');
      this.contentSha = sha(bytes);
      this.naturalWidth = bytes.readUInt32BE(16); this.naturalHeight = bytes.readUInt32BE(20);
      queueMicrotask(() => this.onload?.());
    }
  }
  let digestCount=0;
  const harnessCrypto={subtle:{digest(...args){if(startupHide==='bank' && ++digestCount===2)listeners.get('pagehide')?.({persisted:false});return webcrypto.subtle.digest(...args);}}};
  const sandbox = vm.createContext({document, Image: MockImage, URL: MockURL, Blob, performance: {now: () => performance.now()}, setTimeout, clearTimeout,
    fetch: async url => { fetches.push(String(url)); throw Error('network forbidden'); },
    TextDecoder, TextEncoder, Uint8Array, DataView, HTMLButtonElement: class {}, WebAssembly: {instantiate: WebAssembly.instantiate},
    crypto: harnessCrypto, atob, btoa, window: {}, console, AudioContext: MockAudioContext,
    requestAnimationFrame: fn => raf.push(fn), addEventListener(name, fn) { listeners.set(name, fn); }});
  const run = code => vm.runInContext(code, sandbox);
  const source = element('game-module').textContent
    .replace(/^export (const|class|function) /mg, '$1 ')
    .replaceAll('import.meta.url', "'file:///game.js'");
  const parts = source.split('try{let bytes;');
  assert.equal(parts.length, 2, 'review startup boundary if bundled source changes');
  run(parts[0]); await run('(async()=>{try{let bytes;' + parts[1] + '})()');
  if(corruptAudio||startupHide){assert.equal(run('wasm'),null);assert.equal(run('state'),undefined);assert.equal(ui.get('loading').hidden,false);assert.equal(audioContexts.length,0);}
  if(startupHide){assert.match(ui.get('loading').textContent,/Startup cancelled by pagehide/);assert.equal(run('authoredAudio?.stats().disposed'),true);assert.equal(raf.length,0);}
  else if(!corruptAudio) {assert.equal(run('state.tick'),0);assert.equal(run('ready && paused'),true);assert.equal(ui.get('loading').hidden,true);}
  return {run, ui, draws, fetches, imageSources, raf, sandbox, blobs, downloads, revoked, listeners, audioContexts};
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

const h = await boot(audioHtmlPath);
const stats = () => plain(h.run('presentation.audio.stats()'));
assert.equal(stats().mode, 'coop-authored-pcm');
assert.equal(stats().muted, true);assert.equal(stats().unlocked, false);
assert.equal(h.audioContexts.length, 0);
const initial = h.run('runSaveText()');
const initialChecksum = h.run('state.checksum');
h.run('globalThis.observations=0;globalThis.audioIdentity=presentation.audio;globalThis.observeAudio=presentation.audio.observe.bind(presentation.audio);presentation.audio.observe=(...args)=>{observations++;return observeAudio(...args);};');
// The mock trusted event tests activation policy, not actual browser permission.
h.ui.get('sound').onclick({type:'click',isTrusted:true});
h.ui.get('begin').onclick({type:'click',isTrusted:true});
await h.run('presentation.audio.setPaused(false)');
await h.run('presentation.audio.unlock({type:"click",isTrusted:true})');
assert.equal(h.audioContexts.length,1);assert.equal(h.audioContexts[0].buffers.length,10);
assert.equal(h.run('command(1)'),true);h.run('execute(queue.shift());');
assert.equal(stats().played,1);assert.equal(stats().adapterError,null);
const withSummon = h.run('runSaveText()');
const afterSummon = h.run('state.checksum');
h.run('start();'); // Same instance generation rises, still ordinary reset semantics.
const upperGeneration = h.run('state.presentation.generation');
assert.ok(upperGeneration>0);
for (let repeat=0;repeat<2;repeat++) {
  const played = stats().played, observed = h.run('observations');
  assert.equal(await load(h,initial),true,'lower-generation restore succeeds');
  assert.equal(h.run('presentation.audio===audioIdentity'),true);
  assert.equal(h.run('state.checksum'),initialChecksum);
  assert.equal(h.run('state.presentation.source'),'1');assert.equal(h.run('state.presentation.generation'),0);
  assert.equal(h.run('observations'),observed+1,'candidate replay never observes audio');
  assert.equal(h.run('paused && !ready'),true);assert.equal(stats().paused,true);
  assert.equal(stats().played,played,'restoration baseline is silent');assert.equal(stats().voices,0);
  assert.equal(stats().unlocked,true);assert.equal(stats().muted,false);assert.equal(stats().adapterError,null);
  assert.equal(h.audioContexts.length,1,'context and decoded PCM retained');
  h.ui.get('pause').onclick();await h.run('presentation.audio.setPaused(false)');
  assert.equal(h.run('command(1)'),true);h.run('execute(queue.shift());');
  assert.equal(h.run('state.checksum'),afterSummon);
  assert.equal(stats().played,played+1,'first resumed new event plays once');
  h.run('observePresentation();');assert.equal(stats().played,played+1,'duplicate cannot play');
  assert.equal(stats().adapterError,null);
}
// A nonzero-step candidate replays its past summon without observing/playing it.
const beforeReplayPlayed=stats().played, beforeReplayObserved=h.run('observations');
assert.equal(await load(h,withSummon),true);
assert.equal(stats().played,beforeReplayPlayed);assert.equal(h.run('observations'),beforeReplayObserved+1);
assert.equal(h.run('state.checksum'),afterSummon);
// All rejected candidates and cancellation preserve live owner, activation and cursor.
h.run('globalThis.originalWasm=wasm;');
const beforeIdentity=identity(h), beforeAudio=JSON.stringify(stats()), beforeObserved=h.run('observations');
const saved=JSON.parse(withSummon);
for(const malformed of ['{',JSON.stringify({...saved,wasmSha256:'0'.repeat(64)}),JSON.stringify({...saved,checksum:'0'.repeat(16)})]) {
  assert.equal(await load(h,malformed),false);assert.equal(identity(h),beforeIdentity);
  assert.equal(JSON.stringify(stats()),beforeAudio);assert.equal(h.run('observations'),beforeObserved);
  assert.equal(h.run('wasm===originalWasm && presentation.audio===audioIdentity'),true);
}
h.sandbox.WebAssembly.instantiate=async(...args)=>{const actual=await WebAssembly.instantiate(...args);return {instance:{exports:{...actual.instance.exports,defense_step(){throw new WebAssembly.RuntimeError('candidate trap');}}}};};
assert.equal(await load(h,withSummon),false);h.sandbox.WebAssembly.instantiate=WebAssembly.instantiate;
assert.equal(identity(h),beforeIdentity);assert.equal(JSON.stringify(stats()),beforeAudio);assert.equal(h.run('observations'),beforeObserved);
let release;const readWait=new Promise(resolve=>{release=resolve;});
h.sandbox.testFile={size:withSummon.length,arrayBuffer:async()=>{await readWait;return new TextEncoder().encode(withSummon).buffer;}};
const pending=h.run('loadRunFile(testFile)');assert.equal(await load(h,'{'),false);release();assert.equal(await pending,false);
assert.equal(identity(h),beforeIdentity);assert.equal(JSON.stringify(stats()),beforeAudio);assert.equal(h.run('observations'),beforeObserved);
// Persistent pagehide keeps the same owner but pauses it; nonpersistent closes it.
h.listeners.get('pagehide')({persisted:true});await h.run('presentation.audio.setPaused(true)');
assert.equal(stats().disposed,false);assert.equal(stats().voices,0);assert.equal(stats().paused,true);
h.listeners.get('pageshow')({persisted:true});assert.equal(stats().played,beforeReplayPlayed);
h.listeners.get('pagehide')({persisted:false});assert.equal(stats().disposed,true);assert.equal(h.audioContexts[0].state,'closed');
assert.equal(h.fetches.length,0);
// Lifecycle changes are independent of failed/cancelled candidate restoration.
// A pending read may not suppress hidden/pagehide pause or disposal.
for(const lifecycle of ['hidden','persisted','dispose']) {
 const live=await boot(audioHtmlPath);
 live.ui.get('sound').onclick({type:'click',isTrusted:true});live.ui.get('begin').onclick({type:'click',isTrusted:true});
 await live.run('presentation.audio.setPaused(false)');await live.run('presentation.audio.unlock({type:"click",isTrusted:true})');
 const savedText=live.run('runSaveText()');
 assert.equal(live.run('command(1)'),true);live.run('execute(queue.shift());globalThis.liveWasm=wasm;');
 const liveChecksum=live.run('state.checksum'), binding=live.run('presentation.audio.stats().bindingSerial');
 let settle;const pendingRead=new Promise(resolve=>{settle=resolve;});
 live.sandbox.testFile={size:savedText.length,arrayBuffer:async()=>{await pendingRead;return new TextEncoder().encode(savedText).buffer;}};
 const loading=live.run('loadRunFile(testFile)');
 if(lifecycle==='hidden'){
  live.sandbox.document.hidden=true;live.listeners.get('visibilitychange')();
  assert.equal(live.run('presentation.audio.stats().hidden'),true);
  // Cancel independently, so restoration does not subsequently commit.
  live.run('invalidateRunImport();');
 }else live.listeners.get('pagehide')({persisted:lifecycle==='persisted'});
 assert.equal(live.run('presentation.audio.stats().voices'),0);
 assert.equal(live.run('paused'),true);
 assert.equal(live.run('presentation.audio.stats().disposed'),lifecycle==='dispose');
 settle();assert.equal(await loading,false);
 assert.equal(live.run('wasm===liveWasm'),true);assert.equal(live.run('state.checksum'),liveChecksum);
 assert.equal(live.run('presentation.audio.stats().bindingSerial'),binding);
 assert.equal(live.fetches.length,0);live.run('presentation.dispose();');
}
for(const phase of ['bank','atlas']){const cancelled=await boot(audioHtmlPath,false,phase);assert.equal(cancelled.fetches.length,0);}
const rejectedBank=await boot(audioHtmlPath,true);assert.equal(rejectedBank.fetches.length,0,'malformed explicit bank cannot fall back to network');
const procedural=await boot(proceduralHtmlPath);
assert.notEqual(procedural.run('presentation.audio.stats().mode'),'coop-authored-pcm');
const proceduralSaved=procedural.run('runSaveText()');assert.equal(await load(procedural,proceduralSaved),true);
assert.equal(procedural.run('paused && !ready'),true);assert.equal(procedural.audioContexts.length,0);assert.equal(procedural.fetches.length,0);
console.log(JSON.stringify({version:1,wasmSha256:trustedWasmHash,audioHtmlSha256:sha(fs.readFileSync(audioHtmlPath)),proceduralHtmlSha256:sha(fs.readFileSync(proceduralHtmlPath)),restores:4,networkRequests:0,coverage:'Actual source-compatible WASM and bundled JS with unchanged real PCM; mock DOM/File/Web Audio only, no browser autoplay or physical hearing claim'}));
