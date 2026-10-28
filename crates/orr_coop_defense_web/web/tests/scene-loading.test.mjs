import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';
import { CombatPresentation } from '../combat-presentation.mjs';
import { StrictEventAdapter, applyStrictSnapshot } from '../strict-event-adapter.mjs';
const binary=process.env.ORR_DEFENSE_WASM;
const authored='schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n';
const encode=s=>new TextEncoder().encode(s);
async function core(){return (await WebAssembly.instantiate(fs.readFileSync(binary),{})).instance.exports;}
function snapshot(w){const p=w.defense_snapshot();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.defense_snapshot_len())));}
function load(w,bytes){new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),bytes.length).set(bytes);return w.defense_init_scene(bytes.length);}
test('actual WASM authored Scene, native trace parity, maxHp and restart',{skip:!binary},async()=>{
 const w=await core();assert.equal(w.defense_scene_capacity(),65536);assert.equal(load(w,encode(authored)),0);
 let s=snapshot(w);assert.equal(s.hp,75);assert.equal(s.maxHp,75);assert.deepEqual(s.gold,[250,250]);assert.deepEqual(s.presentation.events,[]);
 for(const [action,sequence]of [[1,1],[3,2],[5,3],[1,4]])w.defense_step(action,0,0,sequence);
 s=snapshot(w);assert.equal(s.checksum,'ccfc379122860c2e','native Rust independently consumed same trace');assert.equal(s.hp,75);assert.equal(s.maxHp,75);assert.equal(s.gold[0],225);
 const source=s.presentation.source;assert.equal(load(w,encode(authored)),0);assert.notEqual(snapshot(w).presentation.source,source);
 w.defense_init(0x0c002026);const canonical=snapshot(w).checksum;assert.equal(load(w,encode('schema: orr.scene/1\nentities: {}\n')),0);assert.equal(snapshot(w).checksum,canonical);
 w.defense_init(71825);for(let t=0,seq=0;t<1200;t++)w.defense_step(t%30===0?1:0,0,0,t%30===0?++seq:0);assert.equal(snapshot(w).checksum,'9056df75782da6ab','legacy seed unchanged (Scenario schema baseline a6844afb)');
});
test('actual WASM rejects before replacing checksum/source/events',{skip:!binary},async()=>{
 const w=await core();w.defense_init(71825);w.defense_step(1,0,0,1);const before=snapshot(w);
 assert.equal(w.defense_init_scene(65537),1);assert.deepEqual(snapshot(w),before);
 assert.equal(w.defense_init_scene(-1),1);assert.deepEqual(snapshot(w),before);
 assert.equal(load(w,new Uint8Array([255])),2);assert.deepEqual(snapshot(w),before);
 for(const bad of ['', 'schema: wrong',authored.replace('75','0'),authored.replace('250','10001'),authored.replace('reactor_hp: 75','unknown: 75'),authored.replace('entities: {}','entities: {e_00000001: {name: injected}}')]){
  assert.equal(load(w,encode(bad)),3);assert.deepEqual(snapshot(w),before);
 }
 const exact=encode(authored+'#'+'x'.repeat(65536-encode(authored).length-1));assert.equal(load(w,exact),0,'exact capacity valid');
});
async function dom(initialize=true){
 const w=await core(),nodes=new Map();const ctx=new Proxy({},{get:(o,p)=>o[p]||(()=>{})});
 const get=id=>{if(['authored-scene','authored-scene-receipt','wasm-core'].includes(id)&&!nodes.has(id))return null;if(!nodes.has(id))nodes.set(id,{style:{},span:{},getContext:()=>ctx,querySelector(){return this.span;},addEventListener(){},setAttribute(){},hasPointerCapture:()=>false});return nodes.get(id);};
 const sandbox=vm.createContext({CombatPresentation,StrictEventAdapter,applyStrictSnapshot,document:{getElementById:get,addEventListener(){}},addEventListener(){},TextDecoder,Uint8Array,HTMLButtonElement:class{},core:w,console:{error(){}},WebAssembly,crypto:webcrypto,atob,btoa,window:{},requestAnimationFrame(){}});
 vm.runInContext(fs.readFileSync(new URL('../game.js',import.meta.url),'utf8').split('try{let bytes;')[0].replace(/^import .*$/mg,''),sandbox);
 const run=s=>vm.runInContext(s,sandbox);run('wasm=core;dimensions();');if(initialize)run('start();');
 return {w,get,run,sandbox,nodes};
}
const file=(text,name='saved.yaml')=>({name,size:encode(text).length,arrayBuffer:async()=>encode(text).buffer});
test('mock DOM + actual WASM success clears pending input and ready/restart uses authored HP',{skip:!binary},async()=>{
 const {w,get,run,sandbox}=await dom();run('ready=false;paused=false;command(1);selected=3;pending={old:true};gesture={id:1};');sandbox.file=file(authored);
 await run('loadSceneFile(file)');assert.equal(run('queue.length'),0);assert.equal(run('pending'),null);assert.equal(run('gesture'),null);assert.equal(run('selected'),-1);assert.equal(run('ready'),true);assert.equal(run('paused'),true);assert.equal(get('healthbar').style.width,'100%');assert.equal(get('scene-source').textContent,'saved.yaml');
 run('start()');assert.equal(snapshot(w).hp,75);assert.equal(snapshot(w).gold[0],250);
});
test('mock DOM rejection/read failure/stale selection/restart keep current game and pending commands',{skip:!binary},async()=>{
 const {w,run,sandbox}=await dom();run('ready=false;paused=false;command(1);selected=2;pending={old:true};');const before=snapshot(w);
 for(const f of [file('bad'),{size:65537,arrayBuffer(){throw Error('must not read oversize');}},{size:2,arrayBuffer:async()=>{throw Error('read failure');}}]){sandbox.file=f;await run('loadSceneFile(file)');assert.deepEqual(snapshot(w),before);assert.equal(run('queue.length'),1);assert.equal(run('selected'),2);assert.equal(run('pending.old'),true);}
 let resolve;sandbox.file={name:'stale.yaml',size:encode(authored).length,arrayBuffer:()=>new Promise(r=>resolve=r)};const old=run('loadSceneFile(file)');sandbox.file=file(authored.replace('250','350'),'latest.yaml');await run('loadSceneFile(file)');const latest=snapshot(w);resolve(encode(authored).buffer);await old;assert.deepEqual(snapshot(w),latest);
 sandbox.file={size:encode(authored).length,arrayBuffer:()=>new Promise(r=>resolve=r)};const restarted=run('loadSceneFile(file)');run('start()');const after=snapshot(w);resolve(encode(authored).buffer);await restarted;assert.deepEqual(snapshot(w),after);
});

const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
async function embeddedFixture(text=authored){
 const bytes=encode(text),wasmBytes=fs.readFileSync(binary),w=await core();
 assert.equal(load(w,bytes),0);
 return {encoded:Buffer.from(bytes).toString('base64'),wasmBytes,receipt:{version:1,sceneSha256:sha(bytes),sceneBytes:bytes.length,wasmSha256:sha(wasmBytes),initialChecksum:snapshot(w).checksum}};
}
async function embeddedDom(fixture){
 const d=await dom(false);d.sandbox.wasmBytes=fixture.wasmBytes;
 d.nodes.set('authored-scene',{textContent:fixture.encoded});
 d.nodes.set('authored-scene-receipt',{textContent:JSON.stringify(fixture.receipt)});
 return d;
}
test('actual WASM embedded and local-file paths have identical tick-zero and input traces',{skip:!binary},async()=>{
 const f=await embeddedFixture(authored+'# Unicode 한글 </script>\n'),embedded=await embeddedDom(f),local=await dom();
 assert.equal(await embedded.run('loadEmbeddedScene(wasmBytes)'),true);
 local.sandbox.file=file(authored+'# Unicode 한글 </script>\n');await local.run('loadSceneFile(file)');
 assert.equal(embedded.run('state.tick'),0);assert.equal(embedded.run('seq'),0);assert.equal(embedded.run('ready&&paused'),true);
 assert.equal(embedded.get('healthbar').style.width,'100%');
 for(const [action,sequence]of [[1,1],[3,2],[5,3],[1,4]]){
  embedded.w.defense_step(action,0,0,sequence);local.w.defense_step(action,0,0,sequence);
  const a=snapshot(embedded.w),b=snapshot(local.w);
  // Source is a transient client-replacement epoch, not deterministic game data.
  assert.equal(a.presentation.source,'1');assert.equal(b.presentation.source,'2');
  delete a.presentation.source;delete b.presentation.source;assert.deepEqual(a,b);
 }
 embedded.run('seq=4;state=snapshot();start()');assert.equal(snapshot(embedded.w).hp,75);assert.equal(snapshot(embedded.w).gold[0],250);
});
test('embedded receipt, byte, ABI and checksum errors throw instead of default fallback',{skip:!binary},async()=>{
 const f=await embeddedFixture();
 const variants=[
  {...f,encoded:'!'}, {...f,encoded:'A'.repeat(87388)},
  {...f,receipt:{...f.receipt,version:2}}, {...f,receipt:{...f.receipt,sceneBytes:0}},
  {...f,receipt:{...f.receipt,sceneSha256:'0'.repeat(64)}}, {...f,receipt:{...f.receipt,wasmSha256:'0'.repeat(64)}},
  {...f,receipt:{...f.receipt,initialChecksum:'0'.repeat(16)}},
 ];
 for(const variant of variants){const d=await embeddedDom(variant);await assert.rejects(d.run('loadEmbeddedScene(wasmBytes)'));}
 for(const bytes of [new Uint8Array([255]),encode('bad scene')]){
  const d=await embeddedDom({...f,encoded:Buffer.from(bytes).toString('base64'),receipt:{...f.receipt,sceneBytes:bytes.length,sceneSha256:sha(bytes)}});
  await assert.rejects(d.run('loadEmbeddedScene(wasmBytes)'));
 }
 const absent=await dom(false);assert.equal(await absent.run('loadEmbeddedScene(new Uint8Array())'),false);
 const missing=await embeddedDom(f);missing.nodes.delete('authored-scene-receipt');await assert.rejects(missing.run('loadEmbeddedScene(wasmBytes)'));
 const abi=await embeddedDom(f);abi.run('wasm={}');await assert.rejects(abi.run('loadEmbeddedScene(wasmBytes)'));
 const cryptoMissing=await embeddedDom(f);cryptoMissing.sandbox.crypto=undefined;await assert.rejects(cryptoMissing.run('loadEmbeddedScene(wasmBytes)'));
 const oversizedReceipt=await embeddedDom(f);oversizedReceipt.nodes.get('authored-scene-receipt').textContent=' '.repeat(1025);await assert.rejects(oversizedReceipt.run('loadEmbeddedScene(wasmBytes)'));
});
test('full mock DOM startup rejects corrupt embedded receipt and disables restart controls',{skip:!binary},async()=>{
 const f=await embeddedFixture();f.receipt.initialChecksum='0'.repeat(16);const d=await embeddedDom(f);
 d.nodes.set('wasm-core',{textContent:Buffer.from(f.wasmBytes).toString('base64')});
 d.run('loadAtlas=async()=>{}');
 const startup=fs.readFileSync(new URL('../game.js',import.meta.url),'utf8').split('try{let bytes;')[1].replaceAll('import.meta.url',"'file:///game.js'");
 await d.run('(async()=>{try{let bytes;'+startup+'})()');
 assert.equal(d.run('wasm'),null);assert.equal(d.run('state'),undefined);assert.equal(d.get('ready').hidden,true);
 assert.match(d.get('loading').textContent,/체크섬/);d.get('reset').onclick();d.get('begin').onclick();assert.equal(d.run('state'),undefined);
});

test('full mock DOM embedded startup is ready at tick zero without legacy initialization',{skip:!binary},async()=>{
 const f=await embeddedFixture(),d=await embeddedDom(f);
 d.nodes.set('wasm-core',{textContent:Buffer.from(f.wasmBytes).toString('base64')});d.run('loadAtlas=async()=>{}');
 const startup=fs.readFileSync(new URL('../game.js',import.meta.url),'utf8').split('try{let bytes;')[1].replaceAll('import.meta.url',"'file:///game.js'");
 await d.run('(async()=>{try{let bytes;'+startup+'})()');
 assert.equal(d.run('state.tick'),0);assert.equal(d.run('seq'),0);assert.equal(d.run('state.checksum'),f.receipt.initialChecksum);
 assert.equal(d.run('state.presentation.source'),'1');assert.equal(d.run('ready&&paused'),true);assert.equal(d.get('loading').hidden,true);
});

test('startup atlas and hash awaits reject reset/local file actions until authored bootstrap is ready',{skip:!binary},async()=>{
 for(const stage of ['atlas','hash']){
  const f=await embeddedFixture(),d=await embeddedDom(f);
  d.nodes.set('wasm-core',{textContent:Buffer.from(f.wasmBytes).toString('base64')});
  let release,entered;const waiting=new Promise(r=>entered=r),gate=new Promise(r=>release=r);
  if(stage==='atlas'){d.sandbox.atlasGate=async()=>{entered();await gate;};d.run('loadAtlas=atlasGate');}
  else{d.run('loadAtlas=async()=>{}');let first=true;d.sandbox.crypto={subtle:{digest:async(...args)=>{if(first){first=false;entered();await gate;}return webcrypto.subtle.digest(...args);}}};}
  const startup=fs.readFileSync(new URL('../game.js',import.meta.url),'utf8').split('try{let bytes;')[1].replaceAll('import.meta.url',"'file:///game.js'");
  const pending=d.run('(async()=>{try{let bytes;'+startup+'})()');await waiting;
  assert.equal(d.run('startupPending'),true);let reads=0;
  d.sandbox.file={...file(authored.replace('250','350')),arrayBuffer:async()=>{reads++;return encode(authored.replace('250','350')).buffer;}};
  d.get('reset').onclick();d.get('again').onclick();d.get('begin').onclick();await d.run('loadSceneFile(file)');
  assert.equal(reads,0);assert.equal(d.run('state'),undefined);assert.equal(d.run('sceneLoadGeneration'),0);
  release();await pending;
  assert.equal(d.run('startupPending'),false);assert.equal(d.run('state.tick'),0);assert.equal(d.run('state.checksum'),f.receipt.initialChecksum);
  assert.equal(d.run('state.gold[0]'),250);assert.equal(d.run('state.presentation.source'),'1');
  await d.run('loadSceneFile(file)');assert.equal(reads,1);assert.equal(d.run('state.gold[0]'),350);
  d.get('reset').onclick();assert.equal(d.run('state.gold[0]'),350);
 }
});
