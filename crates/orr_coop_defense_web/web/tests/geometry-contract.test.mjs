// Static contracts against the actual stylesheet, not DOM/rendered-browser QA.
// This small parser supports this sheet's qualified rules and min/max-width media.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
const css = fs.readFileSync(new URL('../style.css', import.meta.url), 'utf8');
const html = fs.readFileSync(new URL('../index.html', import.meta.url), 'utf8');

function rules(text, viewportWidth, output = []) {
  text = text.replace(/\/\*[\s\S]*?\*\//g, '');
  let cursor = 0;
  while (cursor < text.length) {
    const open = text.indexOf('{', cursor);
    if (open < 0) break;
    const head = text.slice(cursor, open).trim(); let depth = 1, close = open + 1;
    for (; close < text.length && depth; close++) { if (text[close] === '{') depth++; if (text[close] === '}') depth--; }
    assert.equal(depth, 0, 'balanced stylesheet braces');
    const body = text.slice(open + 1, close - 1);
    if (head.startsWith('@media')) {
      const min = /min-width\s*:\s*([\d.]+)px/.exec(head), max = /max-width\s*:\s*([\d.]+)px/.exec(head);
      assert.ok(min || max, `unsupported media condition in contract parser: ${head}`);
      if ((!min || viewportWidth >= +min[1]) && (!max || viewportWidth <= +max[1])) rules(body, viewportWidth, output);
    } else {
      assert.ok(!head.startsWith('@'), `unsupported at-rule: ${head}`);
      const declarations = body.split(';').map(declaration => { const colon = declaration.indexOf(':'); return colon < 0 ? null : [declaration.slice(0, colon).trim(), declaration.slice(colon + 1).trim()]; }).filter(Boolean);
      for (const selector of head.split(',')) output.push({ selector: selector.trim(), declarations });
    }
    cursor = close;
  }
  return output;
}
function style(target, viewportWidth) {
  const result = {}, weights = {};
  for (const rule of rules(css, viewportWidth)) {
    const s = rule.selector;
    const matches = s === '*' || s === target.tag || s === `.${target.className}` || s === `#${target.id}` || s === 'main > *' && target.parent === 'main';
    if (!matches) continue;
    const weight = s.startsWith('#') ? 100 : s.startsWith('.') ? 10 : s === '*' ? 0 : 1;
    for (const [property, value] of rule.declarations) if ((weights[property] ?? -1) <= weight) { result[property] = value; weights[property] = weight; }
  }
  return result;
}

test('actual CSS never allocates a positional or height-flexed HUD/arena row', () => {
  for (const width of [1920, 1366, 1024, 701, 700, 375, 320]) {
    const main = style({ tag: 'main' }, width), arena = style({ tag: 'section', className: 'arena', parent: 'main' }, width), hud = style({ tag: 'section', className: 'hud', parent: 'main' }, width);
    assert.equal(main.display, 'flex'); assert.equal(main['flex-direction'], 'column');
    assert.equal(main.height, 'auto'); assert.equal(main['min-height'], '0'); assert.equal(main['grid-template-rows'], undefined);
    assert.equal(arena.height, 'auto'); assert.equal(arena.display, 'block'); assert.equal(arena.flex, '0 0 auto');
    assert.equal(hud.flex, '0 0 auto'); assert.equal(hud.height, undefined, 'HUD height is content-owned');
    assert.equal(hud['min-height'], '48px');
  }
});

test('actual canvas rules preserve 25:36 without independent forced height/min-width', () => {
  assert.match(html, /<canvas id="game" width="500" height="720"/);
  for (const width of [1920, 1024, 375, 320]) {
    const canvas = style({ tag: 'canvas', id: 'game' }, width), arena = style({ tag: 'section', className: 'arena', parent: 'main' }, width);
    assert.equal(canvas.width, '100%'); assert.equal(canvas.height, 'auto'); assert.equal(canvas['max-height'], 'none');
    assert.equal(canvas['min-width'], '0'); assert.equal(canvas['max-width'], '100%'); assert.equal(canvas['aspect-ratio'], '25/36');
    assert.equal(arena['aspect-ratio'], '25/36'); assert.equal(arena['border-width'], '0');
  }
});

test('declared width formula keeps target geometry and allows short-screen scrolling', () => {
  const samples = [[1920, 855], [1366, 768], [1024, 600], [375, 812], [320, 568]];
  for (const [vw, vh] of samples) {
    const main = style({ tag: 'main' }, vw), body = style({ tag: 'body' }, vw);
    assert.equal(main.width, 'min(var(--play-width),100%)');
    let preferred;
    if (vw > 700) {
      const match = /^clamp\(([\d.]+)px,calc\(\(100svh - ([\d.]+)px\)\*([\d.]+)\),([\d.]+)px\)$/.exec(main['--play-width']);
      assert.ok(match, 'width formula is parsed from the actual stylesheet');
      preferred = Math.max(+match[1], Math.min((vh - +match[2]) * +match[3], +match[4]));
    } else preferred = parseFloat(main['--play-width']);
    const canvasWidth = Math.min(vw, preferred), canvasHeight = canvasWidth * 36 / 25;
    assert.ok(canvasWidth <= vw); assert.ok(Math.abs(canvasWidth / canvasHeight - 25 / 36) < 1e-10);
    assert.notEqual(body.overflow, 'hidden'); assert.notEqual(body['overflow-y'], 'hidden');
    assert.equal(main.height, 'auto', 'content may extend below viewport instead of distorting the arena');
  }
});

test('QA disclosure is collapsed but preserves caveats; overlays can scroll safely', () => {
  assert.match(html, /<details class="slice-warning"><summary>/);
  assert.doesNotMatch(html, /<details class="slice-warning"[^>]*\bopen\b/);
  assert.ok(html.includes('기존 규칙/맵은 원작 재현 미확정'));
  assert.ok(html.includes('현재 캐릭터: 임시 단일 이미지 + 절차적 동작 · 프레임 클립 교체 지원'));
  for (const id of ['ready', 'loading', 'result']) {
    const overlay = style({ tag: 'div', id }, 1920);
    assert.equal(overlay.overflow, 'auto'); assert.equal(overlay['justify-content'], 'safe center');
  }
});
