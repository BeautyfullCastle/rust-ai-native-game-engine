// Actual Rust WASM with a mock DOM/Image/Canvas, not browser/compositor evidence.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash, webcrypto } from 'node:crypto';
import { deflateSync } from 'node:zlib';
import { CombatPresentation } from '../combat-presentation.mjs';
import { StrictEventAdapter, applyStrictSnapshot } from '../strict-event-adapter.mjs';

const binary=process.env.ORR_DEFENSE_WASM;
const gameSource=fs.readFileSync(new URL('../game.js',import.meta.url),'utf8').replace(/^import .*$/mg,'').replaceAll('import.meta.url',"'file:///game.js'");
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const plain=value=>JSON.parse(JSON.stringify(value));
const b64=bytes=>Buffer.from(bytes).toString('base64');
function crc32(bytes){let crc=0xffffffff;for(const byte of bytes){crc^=byte;for(let bit=0;bit<8;bit++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}return(crc^0xffffffff)>>>0;}
function chunk(type,data){const name=Buffer.from(type),size=Buffer.alloc(4),crc=Buffer.alloc(4);size.writeUInt32BE(data.length);crc.writeUInt32BE(crc32(Buffer.concat([name,data])));return Buffer.concat([size,name,data,crc]);}
function spritePng(){
 const header=Buffer.alloc(13);header.writeUInt32BE(4);header.writeUInt32BE(2,4);header[8]=8;header[9]=6;
 const row=Buffer.from([0,...[255,0,0,255,255,0,0,255,0,255,0,255,0,255,0,255]]);
 return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(Buffer.concat([row,row]))),chunk('IEND',Buffer.alloc(0))]);
}
const png=spritePng(),imageName='authored/two-poses.png';
function manifest(){
 const definition={image:imageName,anchor:[.5,1],pivot:[1,2],logicalSize:[2,2],clips:{}};
 for(const name of ['idle','move','attack','hit','death'])definition.clips[name]={frames:[[0,0,2,2],[2,0,2,2]],pivots:[[1,2],[1.5,1.75]],fps:10,loop:['idle','move'].includes(name)};
 Object.assign(definition.clips.attack,{windupMs:100,releaseMs:100,recoveryMs:100});
 return Object.fromEntries([...Array.from({length:6},(_,i)=>`unit:${i}`),...Array.from({length:5},(_,i)=>`enemy:${i}`)].map(key=>[key,structuredClone(definition)]));
}
function payload(spec=manifest(),bytes=png){
 const encoded=Buffer.from(typeof spec==='string'?spec:JSON.stringify(spec));
 return {manifest:encoded,bytes,receipt:{version:1,manifestBytes:encoded.length,manifestSha256:sha(encoded),images:[{name:imageName,bytes:bytes.length,sha256:sha(bytes),width:4,height:2,id:'authored-animation-image-0'}]}};
}
function node(id,textContent,type='application/octet-stream'){return {id,textContent,tagName:'SCRIPT',type};}
async function harness({mode='offline',fixture=payload(),alter,failImage=false,startup=true,shippedHtml=null}={}){
 const elements=[],ui=new Map(),draws=[],imageSources=[],fetches=[],listeners=new Map(),raf=[];
 const add=(id,text,type)=>{const value=node(id,text,type);elements.push(value);return value;};
 const context=new Proxy({drawImage:(...args)=>draws.push(args)},{get:(object,key)=>object[key]??(()=>{})});
 const uiNode=id=>{if(!ui.has(id))ui.set(id,{id,style:{},span:{},getContext:()=>context,querySelector(){return this.span;},addEventListener(){},setAttribute(){},hasPointerCapture:()=>false});return ui.get(id);};
 for(const id of ['game','codex','scene-file','summon','upgrade','invest','again','reset','begin','pause','sound','ready','result','notice','health','healthbar','wave','campaign-intro','gold','kills','selected','detail','outcome','summary','event-status','scene-source','loading'])uiNode(id);
 if(binary)add('wasm-core',b64(fs.readFileSync(binary)));
 for(const [id,file]of [['tower-atlas','units.png'],['enemy-atlas','enemies.png']])add(id,b64(fs.readFileSync(new URL('../assets/'+file,import.meta.url))));
 add('character-atlas-meta',fs.readFileSync(new URL('../assets/characters.json',import.meta.url),'utf8'),'application/json');
 if(mode==='offline'){
  add('authored-animation-manifest',b64(fixture.manifest));add('authored-animation-receipt',JSON.stringify(fixture.receipt),'application/json');
  for(const entry of fixture.receipt.images)add(entry.id,b64(fixture.bytes));
 }else if(mode==='served')add('combat-animation-manifest',fixture.manifest.toString(),'application/json');
 else if(mode==='source')add('combat-animation-manifest',JSON.stringify({src:'authored.json'}),'application/json');
 let source=gameSource;
 if(shippedHtml!==null){
  elements.length=0;
  for(const match of shippedHtml.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)){
   const id=/\bid="([^"]*)"/.exec(match[1])?.[1],type=/\btype="([^"]*)"/.exec(match[1])?.[1];
   if(id){assert.ok(!elements.some(element=>element.id===id),'generated IDs must be unique');add(id,match[2],type);}
  }
  const module=elements.find(element=>element.id==='game-module');assert.ok(module);assert.equal(module.type,'module');
  source=module.textContent.replace(/^export (const|class|function) /mg,'$1 ').replaceAll('import.meta.url',"'file:///game.js'");
 }
 alter?.(elements,fixture);
 const document={hidden:false,baseURI:'https://example.invalid/game/',getElementById:id=>elements.find(n=>n.id===id)||ui.get(id)||null,querySelectorAll(selector){const prefix=selector.match(/^\[id\^="(.*)"\]$/),exact=selector.match(/^\[id="(.*)"\]$/);return elements.filter(n=>prefix?n.id.startsWith(prefix[1]):exact?n.id===exact[1]:false);},addEventListener:(name,fn)=>listeners.set(name,fn)};
 class MockImage{
  set src(value){
   this._src=value;imageSources.push(value);let bytes;
   if(value.startsWith('data:image/png;base64,'))bytes=Buffer.from(value.slice(22),'base64');
   else{if(mode==='offline'||mode==='none')throw Error('Network image forbidden');assert.equal(value,'https://example.invalid/game/assets/'+imageName);bytes=fixture.bytes;}
   this.contentSha=sha(bytes);this.naturalWidth=bytes.readUInt32BE(16);this.naturalHeight=bytes.readUInt32BE(20);
   queueMicrotask(()=>{if(failImage&&sha(bytes)===sha(fixture.bytes))this.onerror?.();else this.onload?.();});
  }
  get src(){return this._src;}
 }
 const fetch=async url=>{fetches.push(String(url));if(mode!=='source')throw Error('Network fetch forbidden');assert.equal(String(url),'https://example.invalid/game/authored.json');return new Response(fixture.manifest);};
 const sandbox=vm.createContext({CombatPresentation,StrictEventAdapter,applyStrictSnapshot,document,Image:MockImage,URL,fetch,TextDecoder,TextEncoder,Uint8Array,DataView,HTMLButtonElement:class{},WebAssembly,crypto:webcrypto,atob,btoa,window:{},console:{error(){},warn(){}},requestAnimationFrame:fn=>raf.push(fn),addEventListener:(name,fn)=>listeners.set(name,fn)});
 const run=code=>vm.runInContext(code,sandbox),prelude=source.split('try{let bytes;')[0];run(prelude);
 if(startup){assert.ok(binary,'ORR_DEFENSE_WASM is required');await run('(async()=>{try{let bytes;'+source.split('try{let bytes;')[1]+'})()');}
 return {run,sandbox,document,elements,ui,draws,imageSources,fetches,listeners,raf,fixture};
}
function currentFrames(h){return plain(h.run('[...presentation.actors].map(([id])=>[id,presentation.pose(id),presentation.frame(id)])'));}
function authoredDraws(h){return h.draws.filter(([image])=>image.contentSha===sha(png)).map(([, ...args])=>args);}
function assertReady(h){assert.equal(h.run('state.tick'),0);assert.equal(h.run('ready&&paused'),true);assert.equal(h.ui.get('loading').hidden,true);}

test('actual WASM no-network startup adopts exact embedded inventory and keeps legacy no-animation path',{skip:!binary},async()=>{
 for(const mode of ['offline','none']){const h=await harness({mode});assertReady(h);assert.equal(h.fetches.length,0);assert.ok(h.imageSources.every(src=>src.startsWith('data:image/png;base64,')));assert.equal(h.run('presentation.manifest["unit:0"].clips.idle.frames.length'),mode==='offline'?2:1);assert.equal(h.raf.length,1);}
});

test('actual WASM served and offline use identical inputs/time, rect/pivot samples and draw each actor once',{skip:!binary},async()=>{
 const offline=await harness(),served=await harness({mode:'served'}),phases=new Set();
 for(const h of [offline,served]){assertReady(h);h.ui.get('begin').onclick({});}
 for(let tick=0;tick<1500;tick++){
  for(const h of [offline,served])h.run(`execute([${tick%30===0?1:0},0,0,${tick%30===0?tick/30+1:0}]);presentation.advance(1000/60);`);
  assert.equal(offline.run('state.checksum'),served.run('state.checksum'));
  assert.deepEqual(currentFrames(offline),currentFrames(served));
  if(tick%20===0){
   for(const h of [offline,served]){h.draws.length=0;h.run('draw()');assert.equal(h.draws.length,h.run('state.units.length+state.enemies.length+presentation.ghosts.length'));}
   assert.deepEqual(authoredDraws(offline),authoredDraws(served));
   for(const frame of currentFrames(offline)){if(frame[2])phases.add(frame[2].index);}
  }
 }
 assert.deepEqual([...phases].sort(),[0,1]);assert.ok(offline.run('presentation.counts.attacks')>0);assert.ok(offline.run('presentation.counts.deaths')>0);
 assert.ok(authoredDraws(offline).some(args=>args[0]===2),'second rectangle must reach Canvas.drawImage');
 assert.ok(authoredDraws(offline).some(args=>args[4]===-1.5*18.5||args[4]===-1.5*35),'per-frame pixel pivot must reach destination');
 assert.equal(offline.fetches.length,0);
});

test('authored ready/pause/hidden freeze, resume, restart and source replacement retain the shared clock',{skip:!binary},async()=>{
 const h=await harness();h.run('draw();');const readyChecksum=h.run('state.checksum');h.run('animate(1000);animate(60000)');assert.equal(h.run('state.checksum'),readyChecksum);assert.equal(h.run('presentation.time'),0);
 h.ui.get('begin').onclick({});h.run('seq=1;execute([1,0,0,1]);presentation.advance(70);update();draw();');
 const before=currentFrames(h);h.run('togglePause();');const time=h.run('presentation.time'),checksum=h.run('state.checksum');
 for(const stamp of [60008,60040,100000])h.run(`animate(${stamp});`);
 assert.equal(h.run('presentation.time'),time);assert.equal(h.run('state.checksum'),checksum);assert.deepEqual(currentFrames(h),before);
 h.run('togglePause();animate(100008);');assert.equal(h.run('presentation.time'),time+8);assert.equal(h.run('state.checksum'),checksum);
 h.document.hidden=true;h.listeners.get('visibilitychange')();const hidden=currentFrames(h),hiddenTime=h.run('presentation.time');h.run('animate(200000)');assert.deepEqual(currentFrames(h),hidden);assert.equal(h.run('presentation.time'),hiddenTime);
 h.document.hidden=false;h.listeners.get('visibilitychange')();assert.equal(h.run('paused'),true);h.run('togglePause();animate(200008);');assert.equal(h.run('presentation.time'),hiddenTime+8);
 const generation=h.run('state.presentation.generation');h.run('start()');assert.equal(h.run('presentation.time'),0);assert.ok(h.run('state.presentation.generation')>generation);assertReady(h);assert.equal(h.run('presentation.manifest["unit:0"].clips.idle.frames.length'),2);
 const source=h.run('state.presentation.source');h.sandbox.bytes=new TextEncoder().encode('schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n');h.run('loadSceneBytes(bytes)');assert.notEqual(h.run('state.presentation.source'),source);assert.equal(h.run('presentation.time'),0);assert.equal(h.run('state.hp'),75);assert.equal(h.run('animationImages.has("'+imageName+'")'),true);
});

test('actual death event reaches terminal authored frame then retires without duplicate sprite draws',{skip:!binary},async()=>{
 const h=await harness();h.ui.get('begin').onclick({});let found=false;
 for(let tick=0;tick<2500;tick++){
  h.run(`execute([${tick%30===0?1:0},0,0,${tick%30===0?tick/30+1:0}]);presentation.advance(1000/60);`);
  if(h.run('presentation.ghosts.some(g=>g.death)')){found=true;break;}
 }
 assert.ok(found,'fixture must observe real Rust death');const checksum=h.run('state.checksum');
 h.run('presentation.advance(100);');h.draws.length=0;h.run('draw();');const ghosts=plain(h.run('presentation.ghosts.filter(g=>g.death).map(g=>presentation.frameFor(g.record.actor,presentation.pose(null,g)))'));
 assert.ok(ghosts.some(frame=>frame.clip==='death'&&frame.index===1));assert.equal(h.draws.length,h.run('state.units.length+state.enemies.length+presentation.ghosts.length'));
 const drawCount=h.draws.length;h.run('draw();');assert.equal(h.draws.length,drawCount*2);assert.equal(h.run('state.checksum'),checksum);
 h.run('presentation.advance(100);');assert.equal(h.run('presentation.ghosts.length'),0);assert.equal(h.run('state.checksum'),checksum);
});

test('external src hook keeps the existing format and fails rather than substituting defaults',{skip:!binary},async()=>{
 const good=await harness({mode:'source'});assertReady(good);assert.equal(good.fetches.length,1);assert.equal(good.run('presentation.manifest["unit:0"].clips.idle.frames.length'),2);
 const bad=await harness({mode:'source',fixture:payload('{"src":"again.json"}')});assert.equal(bad.run('wasm'),null);assert.equal(bad.run('state'),undefined);assert.match(bad.ui.get('loading').textContent,/Animation/);
});

test('missing, tampered, oversize, duplicate or invalid authored payload explicitly fails startup',{skip:!binary},async()=>{
 const cases=[
  elements=>elements.splice(elements.findIndex(n=>n.id==='authored-animation-receipt'),1),
  elements=>elements.splice(elements.findIndex(n=>n.id==='authored-animation-image-0'),1),
  elements=>elements.push({...elements.find(n=>n.id==='authored-animation-manifest')}),
  elements=>elements.push({...elements.find(n=>n.id==='wasm-core')}),
  elements=>elements.push(node('combat-animation-manifest','{}','application/json')),
  elements=>elements.find(n=>n.id==='authored-animation-image-0').textContent=b64(Buffer.alloc(40)),
  elements=>elements.find(n=>n.id==='authored-animation-manifest').textContent='A'.repeat(87388),
  elements=>elements.find(n=>n.id==='authored-animation-receipt').textContent=' '.repeat(8193),
  elements=>{const n=elements.find(n=>n.id==='authored-animation-receipt'),r=JSON.parse(n.textContent);r.images[0].bytes=8388609;n.textContent=JSON.stringify(r);},
  elements=>{const n=elements.find(n=>n.id==='authored-animation-receipt'),r=JSON.parse(n.textContent);r.images[0].width=4097;n.textContent=JSON.stringify(r);},
  elements=>{const n=elements.find(n=>n.id==='authored-animation-receipt'),r=JSON.parse(n.textContent);r.images[0].sha256='0'.repeat(64);n.textContent=JSON.stringify(r);},
  elements=>{const n=elements.find(n=>n.id==='authored-animation-receipt'),r=JSON.parse(n.textContent);r.images[0].name='extra.png';n.textContent=JSON.stringify(r);},
 ];
 for(const alter of cases){const h=await harness({alter});assert.equal(h.run('wasm'),null);assert.equal(h.run('state'),undefined);assert.equal(h.ui.get('loading').hidden,false);assert.match(h.ui.get('loading').textContent,/Animation/);assert.equal(h.run('animationImages.size'),0);assert.equal(h.run('Object.keys(presentation.manifest).length'),0);h.ui.get('reset').onclick();h.ui.get('begin').onclick({});assert.equal(h.run('state'),undefined);}
 const decode=await harness({failImage:true});assert.equal(decode.run('wasm'),null);assert.match(decode.ui.get('loading').textContent,/decode failed/);
});

test('runtime admission rejects duplicate JSON keys, nonfinite, paths, schema and rectangle violations before adoption',async()=>{
 const cases=['{"unit:0":{},"unit:0":{}}','{"unit:0":{"image":"x.png","clips":{"idle":{"frames":[[0,0,1,1]],"fps":1e999}}}}'];
 for(const change of [s=>s['unit:0'].image='../escape.png',s=>s['unit:0'].image='https://bad/x.png',s=>s['unit:0'].image='/absolute.png',s=>s['unit:0'].image='dir\\x.png',s=>s['unit:0'].image='dir/%78.png',s=>s['unit:0'].clips.idle.frames=[[3,0,2,2]],s=>s['unit:0'].clips.idle.pivots=[[0,0]],s=>s['unit:0'].clips.idle.fps=0,s=>s['unit:0'].clips.idle.loop=1,s=>s['unit:0'].pivot=[1,4097],s=>s['unit:0'].anchor=[17,0],s=>s['unit:0'].logicalSize=[0,1],s=>s['unit:0'].clips.idle.windupMs=1,s=>s['unit:0'].clips.attack.releaseMs=2001,s=>s['unit:0'].clips.idle.frames=Array(257).fill([0,0,1,1]),s=>s['unit:6']=s['unit:0'],s=>s['unit:0'].src='other.json']){const spec=manifest();change(spec);cases.push(JSON.stringify(spec));}
 for(const text of cases){const h=await harness({startup:false,fixture:payload(text)});await assert.rejects(h.run('loadAtlas()'),/Animation/);assert.equal(h.run('animationImages.size'),0);}
 const duplicateEscaped='{"unit:0":{},"unit:\\u0030":{}}',h=await harness({startup:false});h.sandbox.duplicateEscaped=duplicateEscaped;assert.throws(()=>h.run('parseAnimationJson(duplicateEscaped)'),/duplicate/);
});

test('embedded image adoption decodes the exact snapshot even when its DOM node changes during digest',async()=>{
 const h=await harness({startup:false}),alternate=Buffer.concat([png.subarray(0,33),chunk('tEXt',Buffer.from('source\0other bytes')),png.subarray(33)]);
 let changed=false;
 h.sandbox.crypto={subtle:{digest:async(...args)=>{if(sha(args[1])===sha(png)){changed=true;h.elements.find(n=>n.id==='authored-animation-image-0').textContent=b64(alternate);}return webcrypto.subtle.digest(...args);}}};
 await h.run('loadAtlas()');assert.ok(changed);assert.equal(h.run('animationImages.get("'+imageName+'").contentSha'),sha(png));assert.ok(h.imageSources.at(-1).endsWith(b64(png)));
});

test('self-consistent receipt cannot admit APNG, malformed/trailing chunks or unknown critical PNG chunks',async()=>{
 const animation=Buffer.alloc(8);animation.writeUInt32BE(2);const badCrc=Buffer.from(png);badCrc[32]^=1;
 for(const bytes of [Buffer.concat([png.subarray(0,33),chunk('acTL',animation),png.subarray(33)]),Buffer.concat([png.subarray(0,33),chunk('fcTL',Buffer.alloc(26)),png.subarray(33)]),Buffer.concat([png.subarray(0,33),chunk('fdAT',Buffer.alloc(4)),png.subarray(33)]),Buffer.concat([png.subarray(0,33),chunk('BADX',Buffer.alloc(0)),png.subarray(33)]),Buffer.concat([png,Buffer.from([1])]),png.subarray(0,png.length-1),badCrc,Buffer.concat([png.subarray(0,33),png.subarray(8,33),png.subarray(33)])]){
  const h=await harness({startup:false,fixture:payload(manifest(),bytes)});await assert.rejects(h.run('loadAtlas()'),/Animation PNG/);assert.equal(h.run('animationImages.size'),0);
 }
});

const shippedPath=process.env.ORR_DEFENSE_OFFLINE_ANIMATION_HTML||process.env.ORR_DEFENSE_ANIMATION_HTML;
test('actual generated single HTML boots its bundled source/WASM/scene/PNG with all network disabled',{skip:!binary||!shippedPath},async()=>{
 const html=fs.readFileSync(shippedPath,'utf8'),h=await harness({shippedHtml:html});assertReady(h);
 assert.equal(h.run('state.presentation.source'),'1');assert.equal(h.run('state.hp'),75);assert.equal(h.run('state.lastWave'),3);assert.deepEqual(plain(h.run('state.gold')),[250,250]);
 const wasm=h.elements.find(element=>element.id==='wasm-core'),receipt=JSON.parse(h.elements.find(element=>element.id==='authored-animation-receipt').textContent);
 assert.equal(sha(Buffer.from(wasm.textContent.trim(),'base64')),sha(fs.readFileSync(binary)));
 assert.equal(receipt.manifestSha256,sha(Buffer.from(h.elements.find(element=>element.id==='authored-animation-manifest').textContent.trim(),'base64')));
 assert.equal(h.run('presentation.manifest["unit:0"].clips.idle.frames.length'),2);assert.equal(h.run('animationImages.has("'+imageName+'")'),true);
 h.ui.get('begin').onclick({});h.run('command(1);execute(queue.shift());presentation.advance(100);draw();');
 const clips=plain(h.run('[...presentation.actors].map(([id])=>presentation.frame(id))'));assert.ok(clips.some(frame=>frame.index===1));assert.ok(authoredDraws(h).some(args=>args[0]===2));
 assert.equal(h.fetches.length,0);assert.ok(h.imageSources.every(src=>src.startsWith('data:image/png;base64,')));assert.equal(h.raf.length,1);
});
