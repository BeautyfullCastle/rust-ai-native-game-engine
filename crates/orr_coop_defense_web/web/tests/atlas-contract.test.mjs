import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { sampleClip, CombatPresentation } from '../combat-presentation.mjs';
const manifest = JSON.parse(fs.readFileSync(new URL('./fixtures/monster-animation-manifest.json', import.meta.url)));
const definition = manifest.moss_spearfighter;
const png = fs.readFileSync(new URL(`./fixtures/${definition.image}`, import.meta.url));
const width = png.readUInt32BE(16), height = png.readUInt32BE(20);

test('temporary actual atlas has 30 in-bounds fixed-size frames and pixel pivots', () => {
  assert.deepEqual([width, height], [1374, 1145]);
  assert.deepEqual(definition.logicalSize, [229, 229]);
  let total = 0;
  for (const [name, clip] of Object.entries(definition.clips)) {
    assert.equal(clip.frames.length, 6); assert.equal(clip.pivots.length, 6);
    for (let i = 0; i < clip.frames.length; i++) {
      const [x, y, w, h] = clip.frames[i];
      assert.deepEqual([w, h], [229, 229]); assert.ok(x >= 0 && y >= 0 && x + w <= width && y + h <= height);
      const frame = sampleClip(definition, name, (i + .1) * 1000 / clip.fps);
      assert.equal(frame.index, i); assert.deepEqual(frame.pivot, clip.pivots[i]); assert.deepEqual(frame.logicalSize, [229, 229]); total++;
    }
  }
  assert.equal(total, 30);
});

test('actual attack, hit and death frame sequences reach their final authored pose', () => {
  for (const clip of ['attack', 'hit', 'death']) {
    const p = new CombatPresentation({ manifest });
    const actor = { id: 'a', visualKey: 'moss_spearfighter', x: 0, y: 0 };
    p.observe({ tick: 1, actors: [actor], projectiles: [], events: [] });
    p.observe({ tick: 2, actors: clip === 'death' ? [] : [actor], projectiles: [], events: [{ id: clip, type: clip, actorId: 'a' }] });
    const indexes = new Set();
    for (let ms = 0; ms < 700; ms += 10) {
      if (clip === 'death') p.drawGhosts((a, pose, frame) => { if (frame.clip === clip) indexes.add(frame.index); });
      else { const pose = p.pose('a'), frame = p.frame('a', pose); if (frame?.clip === clip) indexes.add(frame.index); }
      p.advance(10);
    }
    assert.deepEqual([...indexes], [0, 1, 2, 3, 4, 5], `${clip} must play every authored pose`);
  }
});
