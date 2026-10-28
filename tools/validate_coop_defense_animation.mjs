#!/usr/bin/env node
// Trusted local tool: exercise exact authored bytes through the existing FPS sampler.
// Structural and PNG admission belong to pack_coop_defense_offline.py; no conversion.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';
// argv[3] is the packer's trusted build-source snapshot, never an authored path.
const { sampleClip }=await import(pathToFileURL(process.argv[3]).href);
const bytes=fs.readFileSync(process.argv[2]);
assert.ok(bytes.length>0&&bytes.length<=65536,'manifest byte limit');
const manifest=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));
for(const definition of Object.values(manifest))for(const [name,clip]of Object.entries(definition.clips)){
 const fps=clip.fps??10,loop=clip.loop??(name==='idle'||name==='move');
 for(let index=0;index<clip.frames.length;index++){
  const frame=sampleClip(definition,name,(index+.25)*1000/fps);
  assert.equal(frame.index,index);assert.deepEqual(frame.rect,clip.frames[index]);
  assert.equal(frame.image,definition.image);
  assert.deepEqual(frame.pivot,clip.pivots?.[index]??definition.pivot??null);
  assert.deepEqual(frame.logicalSize,definition.logicalSize??null);
  assert.deepEqual(frame.anchor,definition.anchor??[.5,.86]);
 }
 assert.equal(sampleClip(definition,name,(clip.frames.length+.25)*1000/fps).index,loop?0:clip.frames.length-1);
}
console.log(JSON.stringify({version:1,manifestBytes:bytes.length,manifestSha256:createHash('sha256').update(bytes).digest('hex')}));
