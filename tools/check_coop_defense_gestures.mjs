import { CombatPresentation } from '../crates/orr_coop_defense_web/web/combat-presentation.mjs';
import { StrictEventAdapter, applyStrictSnapshot } from '../crates/orr_coop_defense_web/web/strict-event-adapter.mjs';
import * as RunSave from '../crates/orr_coop_defense_web/web/run-save.mjs';
// Actual WASM plus simulated DOM PointerEvents. This is not browser/manual QA.
import fs from 'node:fs';import vm from 'node:vm';import assert from 'node:assert/strict';
const root=process.argv[2]||'dist/coop-defense-web',src=process.argv[3]||`${root}/game.js`;
const core=(await WebAssembly.instantiate(fs.readFileSync(`${root}/game.wasm`),{})).instance.exports;
let width=375;const nodes=new Map(),events=new Map(),globalEvents=new Map(),captured=new Set();
const ctx=new Proxy({},{get:(o,p)=>o[p]||(()=>{})});
const canvas={width:500,height:720,getContext:()=>ctx,getBoundingClientRect:()=>({left:0,top:0,width,height:width*720/500}),addEventListener:(k,f)=>events.set(k,f),setPointerCapture:id=>captured.add(id),hasPointerCapture:id=>captured.has(id),releasePointerCapture:id=>captured.delete(id)};
const get=id=>{if(id==='game')return canvas;if(!nodes.has(id))nodes.set(id,{style:{},span:{},querySelector(){return this.span;},setAttribute(k,v){this[k]=v;}});return nodes.get(id);};
const sandbox=vm.createContext({CombatPresentation,StrictEventAdapter,applyStrictSnapshot,...RunSave,document:{getElementById:get,addEventListener:(k,f)=>globalEvents.set(k,f)},addEventListener:(k,f)=>globalEvents.set(k,f),TextDecoder,Uint8Array,HTMLButtonElement:class{},core,requestAnimationFrame(){},console});
vm.runInContext(fs.readFileSync(src,'utf8').split('try{let bytes;')[0].replace(/^import .*$/mg,''),sandbox);const run=s=>vm.runInContext(s,sandbox);
run('wasm=core;dimensions();start();');assert.equal(run('canvas.width'),500);assert.ok(68*width/500>=44,'mobile effective target >=44px');
const event=(name,cell,extra={})=>{const p=run(`project(...state.cells.find(a=>a[0]===0&&a[1]===${cell}).slice(2),0)`);events.get(name)({pointerId:1,button:0,clientX:p[0]*width/canvas.width,clientY:p[1]*width/canvas.width,preventDefault(){},...extra});};
assert.equal(run('command(1)'),false);get('begin').onclick();assert.equal(run('command(1)'),true);assert.equal(run('state.units.filter(u=>u[0]===0).length'),0,'input not fabricated before WASM');run('execute(queue.shift());');assert.equal(run('state.units.filter(u=>u[0]===0).length'),1);
event('pointerdown',0);event('pointermove',1);assert.equal(run('gesture.drag'),true);assert.equal(run('queue.length'),0);event('pointerup',1);assert.equal(run('queue.length'),1);assert.equal(run('selected'),0,'selection stays source pending acceptance');run('execute(queue.shift());update();');assert.equal(run('selected'),1);assert.equal(run('unitAt(1) !== undefined'),true);assert.ok(get('selected').textContent.includes('★'),'identity shown after move');
event('pointerdown',1);event('pointermove',2);events.get('pointerup')({pointerId:1,clientX:-100,clientY:-100});assert.equal(run('queue.length'),0);assert.equal(run('selected'),1);
event('pointerdown',1);event('pointermove',2);events.get('pointercancel')({pointerId:1});assert.equal(run('queue.length'),0);assert.equal(run('gesture'),null);
// Tap alternative: selected unit plus destination.
event('pointerdown',2);event('pointerup',2);assert.equal(run('queue.length'),1);run('execute(queue.shift());');assert.equal(run('selected'),2);
run('togglePause();');event('pointerdown',2);event('pointerup',3);assert.equal(run('queue.length'),0);assert.equal(run('selected'),2);assert.equal(run('command(1)'),false);run('togglePause();');
// Invalid merge preserves source and emits no action (synthetic snapshot for exact pair fixture).
run('state.units.push([0,3,5,5,0,375,265]);');event('pointerdown',2);event('pointermove',3);event('pointerup',3);assert.equal(run('selected'),2,'invalid drag retains source');assert.equal(run('queue.length'),0);assert.ok(get('notice').textContent.includes('같은 역할'));event('pointerdown',3);event('pointerup',3);assert.equal(run('selected'),3,'tap nonmatching occupied unit selects it');assert.equal(run('queue.length'),0);run('state=snapshot();');
// Real deterministic core merge: summon until matching pair appears.
run('start();ready=false;paused=false;');for(let i=0;i<6;i++)run('command(1);execute(queue.shift());');const pair=run(`(()=>{const a=state.units.filter(u=>u[0]===0);for(const u of a)for(const v of a)if(u[1]!==v[1]&&u[2]===v[2]&&u[3]===v[3])return[u[1],v[1]];return null;})()`);assert.ok(pair,'seed supplies real matching pair');event('pointerdown',pair[0]);event('pointermove',pair[1]);event('pointerup',pair[1]);const merges=run('state.merges');run('execute(queue.shift());');assert.equal(run('state.merges'),merges+1);assert.equal(run('selected'),pair[1]);
// Keyboard and button spending use the same rejection path; losing focus cancels drag.
run('start();');get('begin').onclick();get('summon').onclick();run('execute(queue.shift());');event('pointerdown',0);event('pointermove',1);run('command(1);');globalEvents.get('blur')();assert.equal(run('gesture'),null);assert.equal(run('queue.length'),0);assert.equal(run('paused'),true,'window blur pauses simulation');
globalEvents.get('keydown')({code:'KeyQ',repeat:false,target:{tagName:'CANVAS'},preventDefault(){}});get('summon').onclick();assert.equal(run('queue.length'),0);assert.ok(get('notice').textContent.includes('일시정지'));
run('start();');assert.equal(run('queue.length'),0);assert.equal(run('state.tick'),0);assert.equal(run('ready'),true);assert.equal(run('selected'),-1);
width=(768-245)*500/720;sandbox.innerWidth=1000;run('dimensions();draw();');assert.equal(canvas.width,500,'desktop stays portrait');assert.ok(68*width/500>=44,'desktop768-height keeps 44px target');assert.ok(width*720/500+245<=768.01,'desktop768-height arena plus reserved chrome fits');get('begin').onclick();run('command(1);execute(queue.shift());');event('pointerdown',0);event('pointermove',1);event('pointerup',1);run('execute(queue.shift());');assert.ok(run('unitAt(1)'),'desktop portrait drag maps correctly');
// Cancellation must not replace selection from a move accepted after pointerdown.
const cancellationCases=[];
const cancellationState=()=>run(`JSON.stringify({live:snapshot(),view:state,queue,seq,
  recording:{initial:runRecorder.initial,steps:runRecorder.steps,count:runRecorder.stepCount,disabledReason:runRecorder.disabledReason}})`);
const releaseOffboard=()=>events.get('pointerup')({pointerId:1,clientX:-100,clientY:-100});
const assertReleased=label=>{assert.equal(run('gesture'),null,`${label}: gesture cleared`);assert.equal(captured.size,0,`${label}: capture released`);};
const queueInitialMove=()=>{
  run('start();');get('begin').onclick();get('summon').onclick();
  assert.equal(run('queue.length'),1,'normal summon queued');run('execute(queue.shift());');
  assert.equal(run('state.units.filter(u=>u[0]===0).length'),1,'real core supplied one unit');
  assert.ok(run('unitAt(0)'),'summon occupies source');assert.equal(run('unitAt(1)'),undefined);
  event('pointerdown',0);event('pointermove',1);event('pointerup',1);
  assert.equal(run('queue.length'),1,'only initial move queued');
  assert.equal(run('JSON.stringify(queue[0].slice(0,3))'),'[2,0,1]');
  assert.equal(run('selected'),0,'source selected while move awaits consumption');
};
const acceptInitialMove=()=>{
  const before=JSON.parse(run('JSON.stringify({seq,steps:runRecorder.stepCount,unit:unitAt(0).slice(2,4)})'));
  run('execute(queue.shift());update();');
  assert.equal(run('queue.length'),0,'initial move consumed');assert.equal(run('unitAt(0)'),undefined,'source vacated');
  assert.equal(run('state.units.filter(u=>u[0]===0).length'),1,'move preserves unit count');
  assert.deepEqual(JSON.parse(run('JSON.stringify(unitAt(1).slice(2,4))')),before.unit,'destination retains role and tier');
  assert.equal(run('seq'),before.seq,'consumption does not allocate another command');
  assert.equal(run('runRecorder.stepCount'),before.steps+1,'accepted move recorded once');
  assert.equal(run('selected'),1,'accepted move selects destination');
};
const startSecondDrag=()=>{event('pointerdown',0);event('pointermove',2);assert.equal(run('gesture.source'),0);assert.equal(run('gesture.drag'),true);};

queueInitialMove();startSecondDrag();
const beforeEarlyCancel=cancellationState();releaseOffboard();
assert.equal(cancellationState(),beforeEarlyCancel,'pre-consumption cancellation preserves board, queue, sequence and recording');
assertReleased('pre-consumption cancellation');assert.equal(run('selected'),0,'cancellation before consumption retains source');
acceptInitialMove();cancellationCases.push('cancel before consumption then acceptance selects destination');

queueInitialMove();acceptInitialMove();event('pointerdown',1);event('pointermove',2);
const beforeFreshCancel=cancellationState();releaseOffboard();
assert.equal(cancellationState(),beforeFreshCancel,'fresh cancellation preserves board, queue, sequence and recording');
assertReleased('fresh cancellation');assert.equal(run('selected'),1,'fresh cancellation retains accepted destination');
cancellationCases.push('fresh offboard cancellation retains destination');

queueInitialMove();acceptInitialMove();
const beforeFreshTap=cancellationState();event('pointerdown',1);event('pointerup',1);
assert.equal(cancellationState(),beforeFreshTap,'fresh deselection preserves board, queue, sequence and recording');
assertReleased('fresh tap');assert.equal(run('selected'),-1,'intentional fresh tap deselects accepted destination');
cancellationCases.push('fresh destination tap intentionally deselects');

queueInitialMove();startSecondDrag();acceptInitialMove();
const beforeStaleCancel=cancellationState();releaseOffboard();
assert.equal(cancellationState(),beforeStaleCancel,'stale cancellation preserves board, queue, sequence and recording');
assertReleased('stale cancellation');
const staleSelection=run('selected');
if(staleSelection!==1)console.error(JSON.stringify({kind:'stale gesture cancellation diagnostic',case:'obsolete offboard release after accepted move',
  controlsCompleted:cancellationCases,expected:{selected:1},actual:{selected:staleSelection,
    sourceOccupied:run('!!unitAt(0)'),destinationOccupied:run('!!unitAt(1)'),queueLength:run('queue.length'),
    sequence:run('seq'),recordedSteps:run('runRecorder.stepCount')},
  cancellationPreserved:['live snapshot','cached snapshot','queue','sequence','recording'],
  eventOrder:['queue move 0 to 1','start drag 0 toward 2','consume move 0 to 1','release second drag offboard']},null,2));
assert.equal(staleSelection,1,'obsolete offboard cancellation must preserve selection established by accepted move');
cancellationCases.push('obsolete offboard cancellation preserves accepted destination');
// Accepted board changes retire old pointer ownership, including later moves/taps.
const assertStaleEventsNeutral=(label,destination,releaseCell=null)=>{
  assertReleased(`${label}: acceptance`);assert.equal(run('selected'),destination,`${label}: accepted selection`);
  const before=cancellationState();
  if(releaseCell===null)event('pointermove',11);
  assert.equal(run('selected'),destination,`${label}: stale pointermove cannot restore source`);
  if(releaseCell===null)releaseOffboard();else event('pointerup',releaseCell);
  assert.equal(cancellationState(),before,`${label}: stale events preserve board, queue, sequence and recording`);
  assertReleased(label);assert.equal(run('selected'),destination,`${label}: stale release preserves selection`);
};
const completeFreshMove=(source,target,label)=>{
  const before=JSON.parse(run('JSON.stringify({seq,steps:runRecorder.stepCount})'));
  event('pointerdown',source);event('pointermove',target);event('pointerup',target);
  assert.equal(run('queue.length'),1,`${label}: fresh move queued`);
  assert.equal(run('seq'),before.seq+1,`${label}: one new command`);
  assert.equal(run('runRecorder.stepCount'),before.steps,`${label}: not recorded before consumption`);
  run('execute(queue.shift());update();');
  assert.equal(run(`!!unitAt(${source})`),false,`${label}: source vacated`);
  assert.ok(run(`unitAt(${target})`),`${label}: destination occupied`);
  assert.equal(run('selected'),target,`${label}: fresh move accepted`);
  assert.equal(run('queue.length'),0);assert.equal(run('runRecorder.stepCount'),before.steps+1);
  assertReleased(label);
};
queueInitialMove();startSecondDrag();acceptInitialMove();
assertStaleEventsNeutral('stale drag after move',1);
completeFreshMove(1,2,'fresh drag after retired gesture');
cancellationCases.push('accepted move retires drag before stale pointermove and fresh move works');

queueInitialMove();event('pointerdown',0);
assert.equal(run('gesture.selectedBefore'),0);assert.equal(run('gesture.drag'),false);
acceptInitialMove();assertStaleEventsNeutral('stale tap after move',1,0);
cancellationCases.push('accepted move retires tap with obsolete selectedBefore');

for(const anchor of ['source','target']){
  run('start();');get('begin').onclick();
  for(let i=0;i<6;i++){get('summon').onclick();assert.equal(run('queue.length'),1);run('execute(queue.shift());');}
  const matching=JSON.parse(run(`JSON.stringify((()=>{const units=state.units.filter(u=>u[0]===0);
    for(const a of units)for(const b of units)if(a[1]!==b[1]&&a[2]===b[2]&&a[3]===b[3])return[a[1],b[1]];
    return null;})())`));
  assert.ok(matching,'normal summons provide real matching pair');
  const [source,target]=matching;
  const before=JSON.parse(run(`JSON.stringify({merges:state.merges,units:state.units.filter(u=>u[0]===0).length,
    tier:unitAt(${target})[3],role:unitAt(${target})[2]})`));
  event('pointerdown',source);event('pointermove',target);event('pointerup',target);
  assert.equal(run('queue.length'),1,'real merge queued');
  const heldCell=anchor==='source'?source:target;
  event('pointerdown',heldCell);event('pointermove',11);
  assert.equal(run('gesture.source'),heldCell);assert.equal(run('gesture.drag'),true);
  const sequence=run('seq'),steps=run('runRecorder.stepCount');
  run('execute(queue.shift());update();');
  assert.equal(run('state.merges'),before.merges+1,'core accepted merge');
  assert.equal(run('state.units.filter(u=>u[0]===0).length'),before.units-1);
  assert.equal(run(`unitAt(${source})`),undefined);
  assert.equal(run(`unitAt(${target})[2]`),before.role);assert.equal(run(`unitAt(${target})[3]`),before.tier+1);
  assert.equal(run('seq'),sequence);assert.equal(run('runRecorder.stepCount'),steps+1);assert.equal(run('queue.length'),0);
  assertStaleEventsNeutral(`merge with held ${anchor}`,target);
  completeFreshMove(target,11,`fresh move after merge with held ${anchor}`);
  cancellationCases.push(`accepted merge retires gesture on ${anchor} and fresh upgraded unit move works`);
}

// Rejected board commands and non-board ticks must leave valid gestures usable.
const finishSurvivingGesture=(label,beforeGesture)=>{
  assert.equal(run('JSON.stringify(gesture)'),beforeGesture,`${label}: gesture preserved`);
  assert.equal(captured.has(1),true,`${label}: capture retained`);assert.equal(run('selected'),1);
  const before=JSON.parse(run('JSON.stringify({seq,steps:runRecorder.stepCount})'));
  event('pointerup',2);
  assert.equal(run('queue.length'),1,`${label}: preserved gesture queues move`);
  assert.equal(run('JSON.stringify(queue[0].slice(0,3))'),'[2,1,2]');
  assert.equal(run('seq'),before.seq+1);assert.equal(run('runRecorder.stepCount'),before.steps);
  run('execute(queue.shift());update();');
  assert.equal(run('unitAt(1)'),undefined);assert.ok(run('unitAt(2)'));assert.equal(run('selected'),2);
  assert.equal(run('queue.length'),0);assert.equal(run('runRecorder.stepCount'),before.steps+1);assertReleased(label);
};
queueInitialMove();assert.equal(run('command(2,0,2)'),true,'second move admitted before first consumption');
run('execute(queue.shift());update();');assert.equal(run('selected'),1);assert.equal(run('queue.length'),1);
event('pointerdown',1);event('pointermove',2);
const failedGesture=run('JSON.stringify(gesture)'),failedSeq=run('seq'),failedSteps=run('runRecorder.stepCount');
run('execute(queue.shift());update();');
assert.equal(run('unitAt(0)'),undefined);assert.ok(run('unitAt(1)'));assert.equal(run('unitAt(2)'),undefined,'obsolete second move rejected by core');
assert.equal(run('queue.length'),0);assert.equal(run('seq'),failedSeq);assert.equal(run('runRecorder.stepCount'),failedSteps+1);
finishSurvivingGesture('rejected move',failedGesture);
cancellationCases.push('failed move preserves still-valid usable gesture');

for(const [label,action,success]of [
  ['idle',0,'state.tick>beforeTick'],['summon',1,'state.units.filter(u=>u[0]===0).length===2'],
  ['upgrade',3,'state.upgrades[0]===1'],['invest',4,'state.investment[0]===1']
]){
  queueInitialMove();acceptInitialMove();event('pointerdown',1);event('pointermove',2);
  const held=run('JSON.stringify(gesture)'),sequence=run('seq'),steps=run('runRecorder.stepCount');
  const beforeTick=run('state.tick');
  if(action){assert.equal(run(`command(${action})`),true,`${label}: normal command admitted`);run('execute(queue.shift());update();');}
  else run('execute([0,0,0,0]);update();');
  assert.ok(run(success.replace('beforeTick',String(beforeTick))),`${label}: core step applied`);
  assert.equal(run('queue.length'),0);assert.equal(run('seq'),sequence+(action?1:0));assert.equal(run('runRecorder.stepCount'),steps+1);
  finishSurvivingGesture(label,held);
  cancellationCases.push(`${label} preserves still-valid usable gesture`);
}
console.log(JSON.stringify({passed:true,kind:'mock DOM pointer events + actual Rust WASM, not browser QA',mobileHitPixels:68*375/500,cancellationCases,coverage:['ready gate','deferred acceptance','drag move','selection follows accepted command','offboard cancellation','pointercancel','tap move','pause rejection','invalid merge retains selection','actual core merge','keyboard/button parity','blur cancels drag','reset','desktop/mobile rendering paths']},null,2));
