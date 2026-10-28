import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
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
async function dom(){
 const w=await core(),nodes=new Map();const ctx=new Proxy({},{get:(o,p)=>o[p]||(()=>{})});
 const get=id=>{if(!nodes.has(id))nodes.set(id,{style:{},span:{},getContext:()=>ctx,querySelector(){return this.span;},addEventListener(){},setAttribute(){},hasPointerCapture:()=>false});return nodes.get(id);};
 const sandbox=vm.createContext({CombatPresentation,StrictEventAdapter,applyStrictSnapshot,document:{getElementById:get,addEventListener(){}},addEventListener(){},TextDecoder,Uint8Array,HTMLButtonElement:class{},core:w,console,requestAnimationFrame(){}});
 vm.runInContext(fs.readFileSync(new URL('../game.js',import.meta.url),'utf8').split('try{let bytes;')[0].replace(/^import .*$/mg,''),sandbox);
 const run=s=>vm.runInContext(s,sandbox);run('wasm=core;dimensions();start();');
 return {w,get,run,sandbox};
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
