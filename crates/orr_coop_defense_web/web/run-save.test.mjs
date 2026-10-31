import test from 'node:test';
import assert from 'node:assert/strict';
import {webcrypto} from 'node:crypto';
import {RUN_SAVE_LIMITS,parseRunSave,sceneInitial,createRunRecorder,replayRunSave,sha256} from './run-save.mjs';
const hash='a'.repeat(64), checksum='0123456789abcdef';
const valid=()=>({format:'orr.stormglass-run',version:1,wasmSha256:hash,initial:{kind:'seed',seed:71825},steps:[[1,0,0,1],[0,0,0,0],[5,0,0,4]],sequence:7,checksum});
const parse=s=>parseRunSave(JSON.stringify(s));
const reject=mutate=>{const s=valid();mutate(s);assert.throws(()=>parse(s));};
function mockCandidate({trap=false,sceneStatus=0,resultChecksum=checksum}={}){
 const memory={buffer:new ArrayBuffer(131072)},calls=[];const data=new TextEncoder().encode(JSON.stringify({checksum:resultChecksum,tick:3}));new Uint8Array(memory.buffer,65536,data.length).set(data);
 const wasm={memory,defense_init:seed=>calls.push(['seed',seed]),defense_scene_capacity:()=>65536,defense_scene_buffer:()=>0,defense_init_scene:n=>{calls.push(['scene',...new Uint8Array(memory.buffer,0,n)]);return sceneStatus;},defense_step:(...step)=>{calls.push(step);if(trap)throw Error('original trap');},defense_snapshot:()=>65536,defense_snapshot_len:()=>data.length};
 return {wasm,calls,instantiate:async()=>({instance:{exports:wasm}})};
}
const replayOptions=candidate=>({wasmBytes:new Uint8Array([1]),wasmSha256:hash,instantiate:candidate.instantiate,now:()=>0,yieldTask:async()=>{}});
test('valid seed, zero steps, discarded queued sequence high-water and Restart retain exact steps',()=>{
 const s=parse(valid());assert.equal(s.sequence,7);assert.deepEqual(s.steps,valid().steps);
 const r=createRunRecorder(s.initial,s.steps);assert.deepEqual(parseRunSave(r.save(hash,9,checksum)).steps,s.steps);
 const empty=valid();empty.steps=[];empty.sequence=4294967295;assert.equal(parse(empty).sequence,4294967295);
});
test('strict duplicate, escaped duplicate, unknown fields, depth, trailing syntax and file byte bounds',()=>{
 const text=JSON.stringify(valid());assert.throws(()=>parseRunSave(text.replace('"version":1','"version":1,"version":1')),/duplicate/);
 assert.throws(()=>parseRunSave(text.replace('"version":1','"version":1,"vers\\u0069on":1')),/duplicate/);
 reject(s=>s.extra=0);reject(s=>s.initial.extra=0);
 assert.throws(()=>parseRunSave('['.repeat(10)+'0'+']'.repeat(10)),/nesting/);
 assert.throws(()=>parseRunSave(text.slice(0,-1)+',}'));
 for(const suffix of ['x',',','{}'])assert.throws(()=>parseRunSave(text+suffix));
 assert.throws(()=>parseRunSave(' '.repeat(RUN_SAVE_LIMITS.fileBytes+1)));
 assert.throws(()=>parseRunSave('é'.repeat(RUN_SAVE_LIMITS.fileBytes/2+1)));
 assert.throws(()=>parseRunSave(text.replace('"sequence":7','"sequence":1e999')),/nonfinite/);
});
test('format, version, lowercase hash/checksum, seed, sequence, action and cells strictly bounded',()=>{
 for(const key of ['format','version','wasmSha256','checksum'])reject(s=>s[key]='invalid');
 reject(s=>s.wasmSha256='A'.repeat(64));reject(s=>s.checksum='A'.repeat(16));
 for(const n of [-1,-0,0.1,4294967296,NaN,Infinity]){if(!Object.is(n,-0)){reject(s=>s.sequence=n);reject(s=>s.initial.seed=n);}}
 assert.throws(()=>parseRunSave(JSON.stringify(valid()).replace('"sequence":7','"sequence":-0')));
 reject(s=>s.sequence=3);reject(s=>s.steps[2][3]=1);
 for(const step of [[6,0,0,1],[1,12,0,1],[1,0,12,1],[1,0,0,0],[0,1,0,0],[0,0,0,1],[1,0,0,4294967296],[1,0,0], [1,0,0,1,0]])reject(s=>s.steps=[step]);
});
test('authored UTF-8 scene preserved exactly; canonical base64 and byte cap enforced',()=>{
 const bytes=new TextEncoder().encode('Scene:\n  name: 눈\n  seed: 42\n');const initial=sceneInitial(bytes);const s=valid();s.initial=initial;assert.deepEqual(parse(s).initial, Object.assign(Object.create(null),initial));
 for(const b64 of ['YQ=','YR==','@@==','AA==\n','/w=='])reject(s=>s.initial={kind:'scene',base64:b64});
 assert.throws(()=>sceneInitial(new Uint8Array([255])));assert.throws(()=>sceneInitial(new Uint8Array(65537)));
 const edge=sceneInitial(new Uint8Array(65536).fill(32));s.initial=edge;assert.equal(parse(s).initial.base64.length,87384);
});
test('recording cap explicitly disables save while subsequent game calls can continue',()=>{
 const r=createRunRecorder({kind:'seed',seed:0});for(let i=0;i<36000;i++)assert.equal(r.record([0,0,0,0]),true);
 assert.equal(parseRunSave(r.save(hash,0,checksum)).steps.length,36000);assert.equal(r.record([0,0,0,0]),false);assert.match(r.disabledReason,/limit/);assert.throws(()=>r.save(hash,0,checksum));assert.equal(r.steps.length,36000);
 const s=valid();s.steps=Array.from({length:36001},()=>[0,0,0,0]);assert.throws(()=>parse(s));
});
test('recorder copies inputs and snapshots; invalid executed input disables save without throwing',()=>{
 const initial={kind:'seed',seed:1},r=createRunRecorder(initial),step=[1,0,0,1];r.record(step);step[0]=5;initial.seed=2;const steps=r.steps;steps[0][0]=4;assert.equal(r.steps[0][0],1);assert.equal(r.stepCount,1);const restored=[[5,0,0,8]],copy=createRunRecorder({kind:'seed',seed:1},restored);restored[0][0]=4;assert.equal(copy.steps[0][0],5);assert.equal(r.initial.seed,1);assert.equal(r.record([1,0,0,1]),false);assert.match(r.disabledReason,/sequence/);
});
test('candidate replay preserves order including Restart and scene bytes; zero-step works',async()=>{
 const c=mockCandidate(),result=await replayRunSave(valid(),replayOptions(c));assert.equal(result.wasm,c.wasm);assert.deepEqual(c.calls,[['seed',71825],...valid().steps]);
 const scene=valid();scene.initial=sceneInitial(new TextEncoder().encode('seed: 999\n'));scene.steps=[];const d=mockCandidate();await replayRunSave(scene,replayOptions(d));assert.deepEqual(d.calls,[['scene',...new TextEncoder().encode('seed: 999\n')]]);
});
test('hash mismatch before instantiation; checksum, scene rejection and traps remain failures',async()=>{
 const c=mockCandidate();await assert.rejects(replayRunSave(valid(),{...replayOptions(c),wasmSha256:'b'.repeat(64)}),/hash mismatch/);assert.equal(c.calls.length,0);
 await assert.rejects(replayRunSave(valid(),replayOptions(mockCandidate({trap:true}))),/original trap/);
 await assert.rejects(replayRunSave(valid(),replayOptions(mockCandidate({resultChecksum:'0'.repeat(16)}))),/checksum mismatch/);
 const s=valid();s.initial=sceneInitial(new TextEncoder().encode('bad'));await assert.rejects(replayRunSave(s,replayOptions(mockCandidate({sceneStatus:3}))),/scene rejected/);
});
test('cooperative yield after 64 steps; fake-clock deadline and stale fence reject candidate',async()=>{
 const s=valid();s.steps=Array.from({length:65},()=>[0,0,0,0]);s.sequence=0;let yields=0;await replayRunSave(s,{...replayOptions(mockCandidate()),yieldTask:async()=>{yields++;}});assert.equal(yields,1);
 let time=0;const c=mockCandidate();const original=c.wasm.defense_step;c.wasm.defense_step=(...args)=>{original(...args);time=5000;};await assert.rejects(replayRunSave(valid(),{...replayOptions(c),now:()=>time}),/deadline/);assert.equal(c.calls.length,2);
 let pending=true;const e=mockCandidate();await assert.rejects(replayRunSave(valid(),{...replayOptions(e),isCurrent:()=>pending,instantiate:async()=>{pending=false;return {instance:{exports:e.wasm}};}}),/stale/);assert.equal(e.calls.length,0);
 let current=true;await assert.rejects(replayRunSave(s,{...replayOptions(mockCandidate()),isCurrent:()=>current,yieldTask:async()=>{current=false;}}),/stale/);
 let elapsed=0,elapsedYields=0;const d=mockCandidate();d.wasm.defense_step=()=>{elapsed+=9;};await replayRunSave(valid(),{...replayOptions(d),now:()=>elapsed,yieldTask:async()=>{elapsedYields++;}});assert.equal(elapsedYields,3);
});
test('SHA-256 exact byte digest',async()=>{const old=globalThis.crypto;Object.defineProperty(globalThis,'crypto',{value:webcrypto,configurable:true});try{assert.equal(await sha256(new Uint8Array()),'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');}finally{Object.defineProperty(globalThis,'crypto',{value:old,configurable:true});}});
