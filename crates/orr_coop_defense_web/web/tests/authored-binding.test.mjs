// Unit boundary: mock Web Audio, unchanged admitted sample PCM; no WASM/browser claim.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { AuthoredAssetAudio, loadAuthoredAssetBank } from '../audio/authored-asset-audio.mjs';
import { CoopDefenseAudio } from '../audio/coop-defense-audio.mjs';
const root=new URL('../assets/audio/',import.meta.url);
const manifest=JSON.parse(await readFile(new URL('bank.json',root)));
const bank=await loadAuthoredAssetBank(manifest,async file=>new Uint8Array(await readFile(new URL(file,root))));
const gesture={type:'click',isTrusted:true};
const epoch=(generation,tick,rewindSerial='1')=>({source:'1',generation,tick:String(tick),rewindSerial});
function context(){return {state:'suspended',currentTime:0,destination:{},sources:[],
 createGain(){return {gain:{value:0},connect(){},disconnect(){}};},
 createBuffer(_channels,length,rate){return {length,rate,copyToChannel(){}};},
 createBufferSource(){const s={connect(){},disconnect(){},start(){},stop(){this.stopped=true;}};this.sources.push(s);return s;},
 async resume(){this.state='running';},async suspend(){this.state='suspended';},async close(){this.state='closed';}};}
const attribution={unitId:1,attackId:0,projectileId:0};
const activation=stats=>Object.fromEntries(['unlocked','muted','paused','hidden','contextState','bufferCount','failed','disposed'].map(k=>[k,stats[k]]));
test('explicit successful instance replacement admits lower generation without relaxing ordinary epoch admission',async()=>{
 const ctx=context(),owner=new AuthoredAssetAudio({bank,contextFactory:()=>ctx});
 await owner.setMuted(false);await owner.unlock(gesture);
 owner.beginEpoch(epoch(3,10));owner.playAsset(bank.assetIds[0],'1/3/1/11/0/0',attribution);
 const before=activation(owner.stats());
 assert.throws(()=>owner.beginEpoch(epoch(0,0,'2')),/Stale audio epoch/);
 assert.equal(owner.stats().voices,1,'rejected ordinary rewind does not mutate owner');
 owner.replaceInstanceBinding(epoch(0,0,'2'));
 assert.deepEqual(activation(owner.stats()),before);assert.equal(owner.stats().voices,0);assert.equal(ctx.sources[0].stopped,true);
 assert.equal(owner.stats().epoch.generation,0);assert.equal(owner.stats().played,1);
 owner.playAsset(bank.assetIds[0],'1/0/2/1/0/0',attribution);assert.equal(owner.stats().played,2);
 assert.throws(()=>owner.playAsset(bank.assetIds[0],'1/0/2/1/0/0',attribution),/Stale or duplicate/);
 owner.replaceInstanceBinding(epoch(0,0,'3'));assert.equal(owner.stats().voices,0);
 owner.playAsset(bank.assetIds[0],'1/0/3/1/0/0',attribution);assert.equal(owner.stats().played,3);
 owner.dispose();
});
test('replacement retains mute hidden pause and retires suppressed events',async()=>{
 for(const setting of ['setMuted','setHidden','setPaused']){
  const ctx=context(),owner=new AuthoredAssetAudio({bank,contextFactory:()=>ctx});
  await owner.setMuted(false);await owner.unlock(gesture);await owner[setting](true);
  owner.beginEpoch(epoch(2,50));const before=activation(owner.stats());
  owner.replaceInstanceBinding(epoch(0,0,'2'));assert.deepEqual(activation(owner.stats()),before);
  const suppressed=owner.playAsset(bank.assetIds[0],'1/0/2/1/0/0',attribution);
  assert.equal(suppressed.status,'suppressed');await owner[setting](false);
  assert.throws(()=>owner.playAsset(bank.assetIds[0],'1/0/2/1/0/0',attribution),/Stale or duplicate/);
  assert.equal(owner.stats().played,0);owner.playAsset(bank.assetIds[0],'1/0/2/2/0/0',attribution);assert.equal(owner.stats().played,1);owner.dispose();
 }
});
test('invalid binding validates before changing owner; replacement cannot resurrect failed or disposed owner',async()=>{
 const owner=new AuthoredAssetAudio({bank});owner.beginEpoch(epoch(4,20));const before=owner.stats();
 assert.throws(()=>owner.replaceInstanceBinding({...epoch(0,0),source:'0'}),/Invalid audio epoch/);
 assert.deepEqual(owner.stats(),before);
 owner.cancelAll('runtime-failure');assert.throws(()=>owner.replaceInstanceBinding(epoch(0,0)),/Stopped audio owner/);
 owner.dispose();assert.throws(()=>owner.replaceInstanceBinding(epoch(0,0)),/Stopped audio owner/);
});
test('adapter binding preserves locked activation and fails silent for stopped owner',()=>{
 const audio=new CoopDefenseAudio({bank});assert.equal(audio.replaceInstanceBinding(epoch(0,0)),true);
 assert.equal(audio.stats().unlocked,false);assert.equal(audio.stats().contextState,'not-created');
 assert.equal(audio.stats().muted,true);assert.equal(audio.stats().played,0);
 audio.dispose();assert.equal(audio.replaceInstanceBinding(epoch(0,0)),false);assert.equal(audio.stats().disposed,true);
});
