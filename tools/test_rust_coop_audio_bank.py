"""Actual Rust/Python differential authoring and unchanged consumer admission.

Requires ORR_RUST_PACKAGE_BIN, ORR_WAV_IMPORTER (original native baseline),
and ORR_NODE_BIN. Python is INTERNAL test infrastructure, not a Rust CLI dependency.
WebAudio/events are mocks; packaging WASM/art are explicitly nonexecutable fixtures.
No browser, physical playback, or crash-atomic publication coverage is claimed.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import wave

from pack_coop_defense_audio import CUES, snapshot_audio
from pack_coop_defense_offline import snapshot_file, strict_json

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent


def wav_bytes(samples, channels=1, rate=48000, width=2):
    stream = io.BytesIO()
    with wave.open(stream, 'wb') as out:
        out.setnchannels(channels)
        out.setsampwidth(width)
        out.setframerate(rate)
        out.writeframes(struct.pack('<' + 'h' * len(samples), *samples))
    return stream.getvalue()


def files(path):
    return {p.name: p.read_bytes() for p in path.iterdir()}


class RustAudioBank(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        for name in ('ORR_RUST_PACKAGE_BIN', 'ORR_WAV_IMPORTER', 'ORR_NODE_BIN'):
            value = os.environ.get(name)
            if not value or not Path(value).is_absolute() or not Path(value).is_file() or not os.access(value, os.X_OK):
                raise RuntimeError(f'{name} must identify an explicit trusted executable')
        cls.rust = os.environ['ORR_RUST_PACKAGE_BIN']
        cls.importer = os.environ['ORR_WAV_IMPORTER']
        cls.node = os.environ['ORR_NODE_BIN']

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        # .orr is a valid source component for this tool; no model-path rule is inferred.
        self.source = self.root / '.orr'
        self.source.mkdir()
        self.license = self.source / 'LICENSE.txt'
        self.license.write_bytes(b'User supplied synthetic fixture license\r\nCC0-1.0\r\n')
        self.samples = {}
        for index, cue in enumerate(CUES):
            values = [0, index + 1, -index - 1, 8192, -8192]
            self.samples[cue] = values
            (self.source / (cue + '.pcm')).write_bytes(struct.pack('<II5h', 48000, 5, *values))
            (self.source / (cue + '.wav')).write_bytes(wav_bytes(values))

    def inputs(self, kind):
        return [f'{cue}={self.source / (cue + "." + kind)}' for cue in CUES]

    def command(self, native, kind, output, inputs=None, license=None):
        if native:
            command = [self.rust, 'create-coop-audio-bank' + ('-from-wav' if kind == 'wav' else '')]
        else:
            script = 'create_coop_defense_audio_bank' + ('_from_wav' if kind == 'wav' else '') + '.py'
            command = [sys.executable, '-B', str(TOOLS / script)]
            if kind == 'wav':
                command += ['--importer', self.importer]
        command += ['--license', str(license or self.license), '--output', str(output)]
        for cue in self.inputs(kind) if inputs is None else inputs:
            command += ['--cue', cue]
        return command

    def run_cli(self, command):
        return subprocess.run(command, capture_output=True, text=True, timeout=40,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def success(self, command):
        result = self.run_cli(command)
        self.assertEqual(result.returncode, 0, result.stderr)

    def rejected_both(self, kind, **kw):
        for native in (False, True):
            out = self.root / ('rejected-rust' if native else 'rejected-legacy')
            result = self.run_cli(self.command(native, kind, out, **kw))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse(os.path.lexists(out), 'admission failure published output')

    def differential(self, kind):
        before = files(self.source)
        legacy, rust = self.root / ('legacy-' + kind), self.root / ('rust-' + kind)
        self.success(self.command(False, kind, legacy))
        self.success(self.command(True, kind, rust, inputs=list(reversed(self.inputs(kind)))))
        expected = files(legacy)
        self.assertEqual(files(rust), expected)
        self.assertEqual(set(expected), {'bank.json', 'LICENSE.txt'} | {cue + '.pcm' for cue in CUES})
        self.assertEqual(len(expected), 12)
        self.assertEqual(before, files(self.source))
        manifest = json.loads(expected['bank.json'])
        self.assertEqual([r['cue'] for r in manifest['cues']], list(CUES))
        for index, record in enumerate(manifest['cues']):
            self.assertEqual(record['assetId'], f'a_000000005444{index + 1:04x}')
            self.assertEqual(record['sha256'], hashlib.sha256(expected[record['file']]).hexdigest())
            self.assertEqual(expected[record['file']], struct.pack('<II5h', 48000, 5, *self.samples[CUES[index]]))
        self.assertEqual(expected['LICENSE.txt'], self.license.read_bytes())
        self.assertIn('authored-audio-bank', snapshot_audio(rust / 'bank.json', snapshot_file, strict_json))
        return rust

    def test_pcm_and_wav_exact_legacy_bytes_order_digests_and_dot_orr_sources(self):
        pcm = self.differential('pcm')
        wav = self.differential('wav')
        self.assertEqual(files(pcm), files(wav))

    def test_pcm_header_clipping_and_bounds_reject_before_publication(self):
        path = self.source / (CUES[-1] + '.pcm')
        for data in [b'x', struct.pack('<IIh', 44100, 1, 1), struct.pack('<IIh', 48000, 2, 1),
                     struct.pack('<IIh', 48000, 1, 8193), struct.pack('<IIh', 48000, 1, -8193),
                     struct.pack('<II', 48000, 0), b'x' * (96008 + 1)]:
            with self.subTest(data=data[:12], length=len(data)):
                path.write_bytes(data)
                self.rejected_both('pcm')
                self.assertEqual(path.read_bytes(), data)

    def test_wav_channels_rate_clipping_truncation_chunks_and_late_cue(self):
        path = self.source / (CUES[-1] + '.wav')
        good = wav_bytes([1])
        bad_chunk = bytearray(good)
        struct.pack_into('<I', bad_chunk, 16, 0xffffffff)  # fmt chunk length exceeds RIFF
        bad_riff = bytearray(good)
        struct.pack_into('<I', bad_riff, 4, 1)
        for data in [wav_bytes([1, 2], channels=2), wav_bytes([1], rate=44100),
                     wav_bytes([8193]), wav_bytes([-8193]), wav_bytes([]), good[:-1],
                     b'not wav', bytes(bad_chunk), bytes(bad_riff), b'x' * (1024 * 1024 + 1),
                     wav_bytes([0] * 48001)]:
            with self.subTest(length=len(data), prefix=data[:16]):
                path.write_bytes(data)
                self.rejected_both('wav')
                self.assertEqual(path.read_bytes(), data)

    def test_missing_duplicate_unknown_cues_and_license_admission(self):
        for kind in ('pcm', 'wav'):
            cues = self.inputs(kind)
            for inputs in [cues[:-1], cues + [cues[0]], ['unknown=x'] + cues[1:],
                           [CUES[0] + '='] + cues[1:], [CUES[0]] + cues[1:]]:
                self.rejected_both(kind, inputs=inputs)
            for data in [b' ', b'\xff', b'x' * (64 * 1024 + 1)]:
                self.license.write_bytes(data)
                self.rejected_both(kind)
            self.license.write_bytes(b'Valid user supplied license\n')

    def test_symlink_fifo_traversal_and_foreign_output_preservation(self):
        for kind in ('pcm', 'wav'):
            first = self.source / (CUES[0] + '.' + kind)
            original = first.read_bytes()
            target = self.source / ('target.' + kind)
            target.write_bytes(original)
            first.unlink()
            first.symlink_to(target)
            self.rejected_both(kind)
            first.unlink()
            os.mkfifo(first)
            self.rejected_both(kind)  # bounded subprocess timeout detects any blocking open
            first.unlink()
            first.mkdir()
            self.rejected_both(kind)
            first.rmdir()
            first.write_bytes(original)
            link = self.root / ('link-' + kind)
            link.symlink_to(self.source, target_is_directory=True)
            self.rejected_both(kind, inputs=[f'{CUES[0]}={link}/{CUES[0]}.{kind}'] + self.inputs(kind)[1:])
            self.rejected_both(kind, license=self.source / '..' / '.orr' / 'LICENSE.txt')
            for native in (False, True):
                output = self.root / f'existing-{kind}-{native}'
                output.write_bytes(b'keep')
                self.assertNotEqual(self.run_cli(self.command(native, kind, output)).returncode, 0)
                self.assertEqual(output.read_bytes(), b'keep')
                output.unlink()
                output.mkdir()
                (output / 'sentinel').write_bytes(b'keep')
                self.assertNotEqual(self.run_cli(self.command(native, kind, output)).returncode, 0)
                self.assertEqual(files(output), {'sentinel': b'keep'})
                dangling = self.root / f'dangling-{kind}-{native}'
                dangling.symlink_to('missing')
                self.assertNotEqual(self.run_cli(self.command(native, kind, dangling)).returncode, 0)
                self.assertEqual(os.readlink(dangling), 'missing')
                self.assertNotEqual(self.run_cli(self.command(native, kind, link / 'new-output')).returncode, 0)
                self.assertFalse((self.source / 'new-output').exists())

    def test_new_rust_rejects_external_execution_flags_without_execution(self):
        sentinel = self.root / 'executed'
        fake = self.root / 'must-not-run'
        fake.write_text(f'#!/bin/sh\ntouch "{sentinel}"\n')
        fake.chmod(0o755)
        for kind in ('pcm', 'wav'):
            for flag in ('--importer', '--command'):
                output = self.root / 'no-output'
                command = self.command(True, kind, output) + [flag, str(fake)]
                result = self.run_cli(command)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(sentinel.exists())
                self.assertFalse(output.exists())
        # New argv/path (4096 byte, 128 component) limits are intentional safety
        # limits, not claims of complete negative compatibility with Python.
        # Oversized argv exec can hit OS ARG_MAX; Rust parser unit tests own that bound.

    def test_actual_snapshot_node_consumer_and_full_packer(self):
        bank = self.root / 'actual-rust-bank'
        self.success(self.command(True, 'wav', bank))
        script = snapshot_audio(bank / 'bank.json', snapshot_file, strict_json)
        prefix = '<script type="application/json" id="authored-audio-bank">'
        self.assertTrue(script.startswith(prefix) and script.endswith('</script>'))
        encoded = script[len(prefix):-len('</script>')]
        payload = self.root / 'consumer.json'
        payload.write_text(json.dumps({'encoded': encoded, 'samples': self.samples}))
        js = NODE_CONSUMER.replace('@AUDIO@', (ROOT / 'crates/orr_coop_defense_web/web/audio/coop-defense-audio.mjs').as_uri()).replace('@ADAPTER@', (ROOT / 'crates/orr_coop_defense_web/web/strict-event-adapter.mjs').as_uri())
        result = self.run_cli([self.node, '--input-type=module', '-e', js, str(payload)])
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = json.loads(result.stdout)
        self.assertEqual(receipt['result'], 'passed')
        self.assertEqual(receipt['strictSummonPlayed'], 2)
        self.assertEqual(receipt['suppressedWithoutReplay'], 2)
        self.assertEqual(receipt['webAudio'], 'mock')
        import pack_coop_defense_offline as packer
        web = self.root / 'web'
        web.mkdir()
        source = ROOT / 'crates/orr_coop_defense_web/web'
        for name in (*packer.MODULES, *packer.EXTRA_MODULES, 'game.js', 'index.html', 'style.css'):
            target = web / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / name, target)
        (web / 'game.wasm').write_bytes(b'not executable packaging fixture')
        (web / 'assets').mkdir()
        for name, data in {'characters.json': b'{"heroes":[],"enemies":[]}',
                           'units.png': b'fixture heroes', 'enemies.png': b'fixture enemies'}.items():
            (web / 'assets' / name).write_bytes(data)
        html = packer.package(web, output=self.root / 'delivery.html', audio=bank / 'bank.json').read_text()
        self.assertIn(script, html)
        self.assertNotIn(str(self.source), html)
        self.assertNotIn(str(bank), html)


# Actual existing JS consumers; only WebAudio and authoritative-event input are mocked.
NODE_CONSUMER = r'''import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { CoopDefenseAudio, loadAuthoredAudio } from '@AUDIO@';
import { StrictEventAdapter } from '@ADAPTER@';
const input = JSON.parse(readFileSync(process.argv[1], 'utf8'));
const cues = Object.keys(input.samples);
function context() {
  return { state: 'suspended', currentTime: 0, destination: {}, sources: [], buffers: [],
    createGain() { return { gain: { value: 0 }, connect() {}, disconnect() {} }; },
    createBuffer(channels, length, rate) {
      const buffer = { channels, length, rate, copyToChannel(pcm) { this.samples = pcm.slice(); } };
      this.buffers.push(buffer); return buffer;
    },
    createBufferSource() {
      const source = { connect() {}, disconnect() {}, start() { this.started = true; }, stop() { this.stopped = true; } };
      this.sources.push(source); return source;
    },
    async resume() { this.state = 'running'; }, async suspend() { this.state = 'suspended'; },
    async close() { this.state = 'closed'; },
  };
}
function snapshot(tick, kind = null) {
  return { units: tick ? [[0, 0, 0, 1, 0, 100, 200, 1]] : [], enemies: [], projectiles: [],
    presentation: { version: 1, source: '1', generation: 0, tick: String(tick),
      delivery: 'latest_tick', status: 'complete', dropped: 0, fullResync: false, windup: false,
      events: kind ? [{ tick: String(tick), system: 0, seq: 0, kind, generation: 0,
        unitId: 1, otherUnitId: 0, enemyId: 0, projectileId: 0, board: 0, role: 0,
        value: 0, hpAfter: 0, x: 100, y: 200 }] : [] } };
}
  const ctx = context();
  const audio = await loadAuthoredAudio({
    getElementById: id => id === 'authored-audio-bank' ? { textContent: input.encoded } : null,
    querySelectorAll: () => [{ id: 'authored-audio-bank' }],
  }, { contextFactory: () => ctx });
  assert.ok(audio instanceof CoopDefenseAudio);
  assert.equal(audio.stats().contextState, 'not-created');
  await audio.setMuted(false);
  assert.equal(await audio.unlock({ isTrusted: true, type: 'click' }), true);
  assert.equal(ctx.buffers.length, 10);
  for (const [index, cue] of cues.entries()) {
    assert.equal(ctx.buffers[index].channels, 1); assert.equal(ctx.buffers[index].rate, 48000);
    assert.deepEqual(Array.from(ctx.buffers[index].samples), input.samples[cue].map(value => value / 32768));
  }
  const adapter = new StrictEventAdapter();
  const send = value => {
    const before = JSON.stringify(value), result = adapter.consume(value);
    audio.observe(result, value.presentation);
    assert.equal(JSON.stringify(value), before, 'consumer must not mutate input snapshots');
    return result;
  };
  send(snapshot(0));
  const first = snapshot(1, 'summon');
  assert.equal(send(first).status, 'applied');
  assert.equal(ctx.sources.length, 1); assert.equal(ctx.sources[0].buffer, ctx.buffers[5]);
  assert.deepEqual(Array.from(ctx.sources[0].buffer.samples), input.samples.summon.map(value => value / 32768));
  assert.equal(send(first).status, 'duplicate'); assert.equal(ctx.sources.length, 1);
  for (const [method, tick] of [['setMuted', 2], ['setPaused', 3]]) {
    await audio[method](true); assert.equal(audio.stats().voices, 0);
    const suppressed = snapshot(tick, 'summon');
    assert.equal(send(suppressed).status, 'applied'); assert.equal(ctx.sources.length, 1);
    await audio[method](false);
    assert.equal(send(suppressed).status, 'duplicate'); assert.equal(ctx.sources.length, 1, 'suppressed event must not replay');
  }
  assert.equal(audio.stats().suppressed, 2);
  assert.equal(send(snapshot(4, 'summon')).status, 'applied'); assert.equal(ctx.sources.length, 2);
  assert.equal(audio.cue('ui-confirm'), false); assert.equal(ctx.sources.length, 2);
  assert.equal(send(snapshot(5, 'ui-confirm')).status, 'rejected', 'UI confirm is not an engine event');
  assert.equal(ctx.sources.length, 2); assert.equal(audio.stats().voices, 0);
  assert.equal(audio.stats().adapterError, null);
  audio.dispose(); assert.equal(ctx.state, 'closed');
  console.log(JSON.stringify({ result: 'passed', cues: 10, strictSummonPlayed: 2,
    suppressedWithoutReplay: 2, authoringCLI: 'actual', offlineSnapshot: 'actual',
    runtimeConsumer: 'actual', eventInput: 'synthetic', webAudio: 'mock' }));'''

if __name__ == '__main__':
    unittest.main()
