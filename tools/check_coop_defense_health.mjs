// Bounded actual-WASM migration and authored-combat evidence; no simulated JS rules.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const [currentPath, baselinePath, baselineTracePath, healthTracePath] = process.argv.slice(2);
assert.ok(healthTracePath, 'usage: node tools/check_coop_defense_health.mjs v5.wasm v4.wasm v4-campaign-trace.json health-trace.json');
async function open(path) {
 const w=(await WebAssembly.instantiate(fs.readFileSync(path),{})).instance.exports;
 return {
  init(seed){w.defense_init(seed);},
  scene(text){const b=new TextEncoder().encode(text);assert.ok(b.length<=w.defense_scene_capacity());new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),b.length).set(b);assert.equal(w.defense_init_scene(b.length),0);},
  step(input){w.defense_step(...input);},
  read(){const p=w.defense_snapshot();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.defense_snapshot_len())));},
 };
}
const current=await open(currentPath),baseline=await open(baselinePath),replay=await open(currentPath);
// Only checksum changes with appended Frame data/build identity. All gameplay,
// complete event payloads, event keys, and source/reset metadata must still match.
const gameplay=({checksum,...snapshot})=>snapshot;
let compared=0;
function compare(){const s=current.read();assert.deepEqual(gameplay(s),gameplay(baseline.read()),`v4/v5 gameplay and events at sample ${compared}`);assert.deepEqual(s,replay.read(),'v5 exact reset/replay checksum and events');compared++;return s;}
function step(input){for(const g of [current,baseline,replay])g.step(input);return compare();}
for(const g of [current,baseline,replay])g.init(71825);
compare();
for(let tick=0,seq=0;tick<1200;tick++)step(tick%30===0?[1,0,0,++seq]:[0,0,0,0]);
step([5,0,0,1000]);
let s;
for(let tick=0;tick<20000;tick++){s=step([0,0,0,0]);if(s.phase!==0)break;}
assert.equal(s.phase,2,'bounded no-input terminal loss');
for(let tick=0;tick<6;tick++)step([0,0,0,0]);
step([5,0,0,1001]);assert.equal(current.read().tick,0);
// A pristine v4 native strategy supplies the exact consumed inputs for a full
// authored default-health campaign, including terminal ticks and restart.
const oldTrace=JSON.parse(fs.readFileSync(baselineTracePath,'utf8'));
assert.ok(oldTrace.rows.length>0&&oldTrace.rows.length<=20000);
const shortScene=fs.readFileSync('examples/static-game-starter/scenes/stormglass-short.scene.yaml','utf8');
for(const g of [current,baseline,replay])g.scene(shortScene);
assert.equal(baseline.read().checksum,oldTrace.initialChecksum);compare();
for(const row of oldTrace.rows){step(row.slice(0,4));assert.equal(baseline.read().checksum,row[4],'pristine v4 native/WASM oracle');}
assert.equal(current.read().phase,1);
// Exact same health-trace inputs drive health1 and health4. Do not assume equal
// completion times, universal winnability, or unchanged downstream targeting.
const healthTrace=JSON.parse(fs.readFileSync(healthTracePath,'utf8'));
assert.ok(healthTrace.rows.length>0&&healthTrace.rows.length<=20000);
const healthScene=fs.readFileSync('examples/static-game-starter/scenes/stormglass-health.scene.yaml','utf8');
const low=await open(currentPath),high=await open(currentPath);
low.scene(healthScene.replace('{multiplier: 4}','{multiplier: 1}'));high.scene(healthScene);
const combat=[[],[]],spawns=[[],[]];
for(const row of healthTrace.rows){
 for(const [i,g] of [low,high].entries()){
  g.step(row.slice(0,4));const state=g.read();
  if(i===1)assert.equal(state.checksum,row[4],'health trace native checksum');
  for(const e of state.presentation.events){
   if(e.kind==='enemy_spawn')spawns[i].push([e.generation,e.board,e.value,e.hpAfter]);
   if(e.kind==='hit'||e.kind==='death')combat[i].push([e.kind,e.tick,e.generation,e.enemyId,e.value,e.hpAfter]);
  }
 }
}
assert.equal(high.read().phase,1,'authored bounded fixture finishes after restart');
assert.equal(spawns[1].length,24,'two complete 12-enemy runs');
assert.deepEqual(spawns[1].map(([generation,board,kind])=>[generation,board,kind]),spawns[0].map(([generation,board,kind])=>[generation,board,kind]),'same authored family/order/count across both runs');
for(let i=0;i<spawns[1].length;i++)assert.equal(spawns[1][i][3],spawns[0][i][3]*4);
for(const events of combat){assert.ok(events.some(e=>e[0]==='hit'));assert.ok(events.some(e=>e[0]==='death'));}
assert.notDeepEqual(combat[0],combat[1],'authored HP changes actual hit/death outcomes');
console.log(JSON.stringify({passed:true,defaultSnapshotsCompared:compared,healthSpawns:spawns.map(s=>s.length),combatEvents:combat.map(s=>s.length),checks:['complete v4/v5 default gameplay and event parity','native v4 oracle','v5 exact replay','terminal and restart','native health oracle','authored HP changes real combat']},null,2));
