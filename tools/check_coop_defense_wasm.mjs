// Actual WASM ABI integration; no reimplementation of game rules.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const binary=process.argv[2]||'dist/coop-defense-web/game.wasm';
const campaignTracePath=process.argv[3];
const campaignScenePath=process.argv[4]||'examples/static-game-starter/scenes/stormglass-short.scene.yaml';
const w=(await WebAssembly.instantiate(fs.readFileSync(binary),{})).instance.exports;
const read=()=>{const p=w.defense_snapshot();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.defense_snapshot_len())));};
const load=text=>{const bytes=new TextEncoder().encode(text);new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),bytes.length).set(bytes);return w.defense_init_scene(bytes.length);};
w.defense_init(71825);let s=read();assert.equal(s.hp,30);assert.equal(s.units.length,0);
assert.equal(s.lastWave,10,'campaign defaults to ten waves');
assert.equal(load('schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Campaign: {last_wave: 3}}\n'),0);
s=read();assert.equal(s.lastWave,3);assert.equal(s.hp,30);assert.deepEqual(s.gold,[150,150],'Campaign-only authoring keeps Scenario defaults');
w.defense_step(1,0,0,1);w.defense_step(5,0,0,2);s=read();assert.equal(s.tick,0);assert.equal(s.lastWave,3);assert.deepEqual(s.gold,[150,150],'restart preserves Campaign and Scenario');
const campaignSnapshot=read();
for(const lastWave of [0,11]){assert.equal(load(`schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Campaign: {last_wave: ${lastWave}}}\n`),3);assert.deepEqual(read(),campaignSnapshot,'invalid Campaign does not replace state');}
assert.equal(load('schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}, CoopDefense::Campaign: {last_wave: 3}}\n'),0);
s=read();assert.equal(s.lastWave,3);assert.equal(s.hp,75);assert.equal(s.maxHp,75);assert.deepEqual(s.gold,[250,250]);
w.defense_step(5,0,0,1);s=read();assert.equal(s.lastWave,3);assert.equal(s.maxHp,75);assert.deepEqual(s.gold,[250,250],'restart preserves both singletons');
assert.equal(load('schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::WavePacing: {spawn_interval_ticks: 70}}\n'),0);
s=read();assert.equal(s.lastWave,10);assert.equal(s.hp,30);assert.deepEqual(s.gold,[150,150]);
const pacingSnapshot=read();
for(const interval of [0,34,351]){assert.equal(load(`schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::WavePacing: {spawn_interval_ticks: ${interval}}}\n`),3);assert.deepEqual(read(),pacingSnapshot,'invalid pacing does not replace state');}
// WaveCounts-only admission keeps the other independently defaulted settings.
const countScene=values=>`schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::WaveCounts: {pair_counts: ${values}}}\n`;
assert.equal(load(countScene('[1,2,3,16,18,20,22,24,26,28]')),0);
s=read();assert.equal(s.lastWave,10);assert.equal(s.hp,30);assert.deepEqual(s.gold,[150,150]);
const countsSnapshot=read();
for(const values of ['[]','null','[1,2,3,16,18,20,22,24,26]','[1,2,3,16,18,20,22,24,26,0]','[1,2,3,16,18,20,22,24,26,29]','[1,2,3,16,18,20,22,24,26,4294967296]']){
 assert.equal(load(countScene(values)),3);assert.deepEqual(read(),countsSnapshot,'invalid count array preserves current state');
}
// Health-only authoring is independent; malformed values cannot replace a live client.
const healthScene=fields=>`schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::EnemyHealth: ${fields}}\n`;
for(const multiplier of [1,2,3,4]){
 assert.equal(load(healthScene(`{multiplier: ${multiplier}}`)),0);
 s=read();assert.equal(s.hp,30);assert.equal(s.lastWave,10);assert.deepEqual(s.gold,[150,150]);
 w.defense_step(1,0,0,1);const before=read();
 for(const fields of ['{}','{multiplier: 0}','{multiplier: 5}','{multiplier: -1}','{multiplier: 1.5}','{multiplier: 4294967296}','{multiplier: null}','{multiplier: 2, extra: 1}','{multiplier: 2, multiplier: 3}']){
  assert.equal(load(healthScene(fields)),3);assert.deepEqual(read(),before,'invalid health preserves the complete live snapshot');
 }
 w.defense_step(5,0,0,2);
 for(let tick=0;tick<210;tick++)w.defense_step(0,0,0,0);
 const spawned=read().presentation.events.filter(e=>e.kind==='enemy_spawn');
 assert.equal(spawned.length,2);assert.ok(spawned.every(e=>e.value===1&&e.hpAfter===39*multiplier),'restart keeps authored multiplier on both lanes');
}
w.defense_init(71825);s=read();
w.defense_step(1,0,0,1);s=read();assert.equal(s.gold[0],125);assert.equal(s.units.filter(u=>u[0]===0).length,1);
w.defense_step(1,0,0,1);assert.equal(read().gold[0],125,'held sample spends only once');
w.defense_step(2,0,11,2);assert.equal(read().units.find(u=>u[0]===0)[1],11,'move actual Rust unit');
w.defense_step(3,0,0,3);assert.equal(read().upgrades[0],1);
w.defense_step(4,0,0,4);assert.equal(read().investment[0],1);
w.defense_step(5,0,0,5);assert.equal(read().tick,0);assert.equal(read().gold[0],150);
w.defense_step(1,0,0,6);assert.equal(read().gold[0],125,'post-restart higher sequence accepted');
w.defense_init(71825);for(let i=1;i<=4;i++)w.defense_step(1,0,0,i);
w.defense_step(2,0,3,5);s=read();assert.equal(s.units.find(u=>u[0]===0&&u[1]===3)[3],2,'two matching units merge');assert.equal(s.merges,1);
w.defense_init(71825);for(let t=0;t<60000&&read().phase===0;t++)w.defense_step(0,0,0,0);assert.equal(read().phase,2,'unattended run reaches actual loss');
w.defense_step(5,0,0,1);assert.equal(read().phase,0,'terminal restart restores playable run');
w.defense_init(71825);let sequence=0;for(let t=0;t<1200;t++)w.defense_step(t%30===0?1:0,0,0,t%30===0?++sequence:0);s=read();
// Scenario singleton schema intentionally added authored defaults to checksummed Frame.
// Historical identity-only fixture: da0ca42edf9cf8a2 (before Scenario).
// Scenario-v1 historical golden: 9056df75782da6ab (before Campaign singleton).
// Campaign-v2 historical golden: be2bdbfedfabd614.
// WavePacing-v3 changes Frame layout; native legacy_seed_trace_matches_native_simulation
// independently reproduced 41a0cf5eb1436a07, then fresh actual WASM matched it.
// Same seed71825/default settings, two input slots, 1200 consumed ticks.
// WaveCounts-v4 native ABI test independently produced fe7ec338439dce34.
// EnemyHealth-v5 independently printed by native legacy_seed_trace_matches_native_simulation.
assert.equal(s.checksum,'5f42bc7d8e18cc35','native EnemyHealth-v5 schema parity fixture');
assert.equal(s.wave,2);assert.equal(s.hp,30);assert.ok(s.projectiles.length>0);assert.ok(s.units.some(u=>u[0]===1));
if(campaignTracePath){
 // Replay only exact consumed inputs from the native seeded strategy. No frame state
 // is edited or inferred by this WASM checker.
 const trace=JSON.parse(fs.readFileSync(campaignTracePath,'utf8'));
 if(trace.eventRows)assert.equal(trace.eventRows.length,trace.rows.length);
 assert.equal(trace.lastWave,3);assert.match(trace.buildId,/^[0-9a-f]{16}$/);
 const scene=fs.readFileSync(campaignScenePath);
 assert.equal(w.defense_scene_capacity(),65536);
 new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),scene.length).set(scene);
 assert.equal(w.defense_init_scene(scene.length),0);s=read();
 assert.equal(s.checksum,trace.initialChecksum,'same authored scene starts from native checksum');assert.equal(s.lastWave,3);
 let victories=0,previousPhase=s.phase,restarts=0;
 for(let i=0;i<trace.rows.length;i++){
  const row=trace.rows[i];assert.equal(row.length,9);
  const [action,source,target,sequence,checksum,wave,phase,tick,generation]=row;
  if(action===5)restarts++;
  w.defense_step(action,source,target,sequence);s=read();
  assert.equal(s.checksum,checksum,`native consumed-input checksum at trace row ${i}`);
  if(trace.eventRows)assert.deepEqual(s.presentation.events.map(e=>[e.tick,e.system,e.seq,e.kind,e.generation,e.unitId,e.otherUnitId,e.enemyId,e.projectileId,e.board,e.role,e.value,e.hpAfter,e.x,e.y]),trace.eventRows[i],`native complete event payloads at trace row ${i}`);
  assert.equal(s.wave,wave);assert.equal(s.phase,phase);assert.equal(s.tick,tick);
  assert.equal(s.presentation.generation,generation);assert.equal(s.lastWave,3);
  assert.ok(s.wave<=3,'short authored campaign never spawns wave four');
  assert.notEqual(s.phase,2,'native winning trace must not lose');
  if(s.phase===1&&previousPhase!==1)victories++;
  previousPhase=s.phase;
 }
 assert.equal(victories,2,'one uninterrupted win before and after restart');assert.equal(restarts,1);
 assert.equal(s.phase,1);assert.equal(s.wave,3);
}
console.log(JSON.stringify({passed:true,checksum:s.checksum,tick:s.tick,wave:s.wave,hp:s.hp,kills:s.kills,coverage:['Campaign default10','Campaign-only Scenario defaults','Campaign validation atomicity','authored max-wave HUD source','summon','sequence deduplication','move','merge','loss','terminal restart','upgrade','invest','restart','AI partner','projectiles','1200-tick native parity',...(campaignTracePath?['complete native-consumed 3-wave victory/restart parity']:[])]},null,2));
