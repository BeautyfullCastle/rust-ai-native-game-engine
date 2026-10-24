// Actual game.js + Rust WASM + native Canvas. Not browser/compositor evidence.
import fs from 'node:fs';
import vm from 'node:vm';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';
import { CombatPresentation } from '../combat-presentation.mjs';
import { StrictEventAdapter, applyStrictSnapshot } from '../strict-event-adapter.mjs';

const require = createRequire(import.meta.url);
const { createCanvas, loadImage } = require(process.env.ORR_CANVAS_MODULE || '@napi-rs/canvas');
const root = fileURLToPath(new URL('..', import.meta.url));
const gameSource = fs.readFileSync(`${root}/game.js`, 'utf8').split('try{let bytes;')[0].replace(/^import .*$/mg, '');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');

async function harness() {
  const core = (await WebAssembly.instantiate(fs.readFileSync(process.env.ORR_DEFENSE_WASM || `${root}/game.wasm`), {})).instance.exports;
  const canvas = createCanvas(500, 720), context = canvas.getContext('2d'), nodes = new Map();
  canvas.getBoundingClientRect = () => ({ left: 0, top: 0, width: 500, height: 720 });
  canvas.addEventListener = () => {};
  const get = id => { if (id === 'game') return canvas; if (!nodes.has(id)) nodes.set(id, { style: {}, span: {}, querySelector() { return this.span; }, setAttribute() {} }); return nodes.get(id); };
  const document = { hidden: false, getElementById: get, addEventListener() {} };
  const sandbox = vm.createContext({ CombatPresentation, StrictEventAdapter, applyStrictSnapshot, core, document, addEventListener() {}, requestAnimationFrame() {}, TextDecoder, Uint8Array, console,
    performance: { now() { throw Error('Rendering must not read the wall clock'); } },
    Date: class { static now() { throw Error('Rendering must not read Date.now'); } },
  });
  const run = code => vm.runInContext(code, sandbox);
  run(gameSource);
  sandbox.heroes = await loadImage(`${root}/assets/units.png`);
  sandbox.enemies = await loadImage(`${root}/assets/enemies.png`);
  sandbox.meta = JSON.parse(fs.readFileSync(`${root}/assets/characters.json`, 'utf8'));
  run(`towerAtlas=heroes; enemyAtlas=enemies; heroRects=meta.heroes; enemyRects=meta.enemies;
    animationImages.set('units.png',heroes); animationImages.set('enemies.png',enemies);
    const fixtureManifest={}; heroRects.forEach((r,i)=>{fixtureManifest['unit:'+i]={image:'units.png',anchor:[.5,1],clips:{idle:{frames:[r],fps:1,loop:true}}};});
    [2,0,1,0,3].forEach((index,i)=>{fixtureManifest['enemy:'+i]={image:'enemies.png',anchor:[.5,1.16],clips:{idle:{frames:[enemyRects[index]],fps:1,loop:true}}};});
    presentation.setManifest(fixtureManifest); wasm=core; dimensions(); start(); ready=false; paused=false; update();
  `);
  const pixels = () => Buffer.from(context.getImageData(0, 0, 500, 720).data);
  const poses = () => run('JSON.stringify([...presentation.actors].map(([id])=>[id,presentation.pose(id),presentation.frame(id)]))');
  const snapshot = () => run('JSON.stringify(snapshot())');
  return { canvas, document, run, pixels, poses, snapshot };
}

test('actual combat frames stay byte-identical during pause across wall-clock gaps', async () => {
  const h = await harness();
  for (let tick = 0; tick < 1200; tick++) {
    if (tick % 90 === 0) h.run('command(1);');
    h.run('execute(queue.shift()||[0,0,0,0]);presentation.advance(1000/60);');
  }
  assert.ok(h.run('state.units.some(u=>u[0]===1)'), 'AI board must contain units');
  assert.ok(h.run('state.phase===0'), 'test must pause an active game');
  h.run('last=1000; togglePause(); draw();');
  const before = h.pixels(), state = h.snapshot(), poses = h.poses(), time = h.run('presentation.time');
  const captures = [];
  for (const wallTime of [1016, 1500, 5000, 60000, 3600000]) {
    h.run(`animate(${wallTime});`);
    assert.equal(h.snapshot(), state, 'pause must not mutate the authoritative snapshot');
    assert.equal(h.run('presentation.time'), time, 'pause must freeze the presentation clock');
    assert.equal(h.poses(), poses, 'all actor poses/frames must remain frozen');
    assert.deepEqual(h.pixels(), before, `paused pixels changed at RAF wall time ${wallTime}`);
    captures.push({ rafTimeMs: wallTime, rgbaSha256: sha(h.pixels()) });
  }
  console.log(JSON.stringify({ kind: 'native Canvas, not browser', wave: h.run('state.wave'), aiUnits: h.run('state.units.filter(u=>u[0]===1).length'), particles: h.run('presentation.particles.length'), presentationTimeMs: time, captures }));
});

test('resume is continuous from frozen presentation time and draw never steps Rust', async () => {
  const h = await harness();
  for (let tick = 0; tick < 240; tick++) {
    if (tick % 60 === 0) h.run('command(1);');
    h.run('execute(queue.shift()||[0,0,0,0]);presentation.advance(1000/60);');
  }
  h.run('last=1000; togglePause(); animate(60000);');
  const time = h.run('presentation.time'), state = h.snapshot(), poses = h.poses();
  h.run('togglePause(); draw();');
  assert.equal(h.run('presentation.time'), time);
  assert.equal(h.poses(), poses, 'unpausing must not jump actor phase');
  assert.equal(h.snapshot(), state, 'draw and unpause must not step Rust');
  h.run('animate(60008);'); // Less than one fixed simulation step.
  assert.equal(h.run('presentation.time'), time + 8);
  assert.equal(h.snapshot(), state, 'sub-step animation must not mutate Rust');
  assert.notEqual(h.poses(), poses, 'animation should resume, not stay stuck');
  const resumed = h.pixels();
  h.run('draw(); draw();');
  assert.deepEqual(h.pixels(), resumed, 'draw is idempotent without clock advancement');
  assert.equal(h.snapshot(), state);
});

test('ready and hidden states freeze the same rendering clock', async () => {
  const h = await harness();
  h.run('start(); draw();');
  const readyPixels = h.pixels(), readyState = h.snapshot();
  h.run('animate(1000); animate(60000);');
  assert.deepEqual(h.pixels(), readyPixels);
  assert.equal(h.snapshot(), readyState);
  h.run('ready=false; paused=true; update(); draw();');
  h.document.hidden = true; h.run('syncPresentation();');
  const hiddenPixels = h.pixels(), time = h.run('presentation.time');
  h.run('animate(3600000);');
  assert.deepEqual(h.pixels(), hiddenPixels);
  assert.equal(h.run('presentation.time'), time);
});
