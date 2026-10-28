// Actual WASM ABI integration; no reimplementation of game rules.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const binary=process.argv[2]||'dist/coop-defense-web/game.wasm';
const campaignTracePath=process.argv[3];
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
// Native legacy_seed_trace_matches_native_simulation produces be2bdbfedfabd614 for
// seed71825, default Scenario+Campaign, 2 input slots and this same 1200-tick trace.
assert.equal(s.checksum,'be2bdbfedfabd614','native Scenario+Campaign schema parity fixture');
assert.equal(s.wave,2);assert.equal(s.hp,30);assert.ok(s.projectiles.length>0);assert.ok(s.units.some(u=>u[0]===1));
if(campaignTracePath){
 // Replay only exact consumed inputs from the native seeded strategy. No frame state
 // is edited or inferred by this WASM checker.
 const trace=JSON.parse(fs.readFileSync(campaignTracePath,'utf8'));
 assert.equal(trace.lastWave,3);assert.match(trace.buildId,/^[0-9a-f]{16}$/);
 const scene=fs.readFileSync('examples/static-game-starter/scenes/stormglass-short.scene.yaml');
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
