"""Real native WAV importer acceptance; requires ORR_WAV_IMPORTER, never skips.

Synthetic user WAVs only. Full HTML test is packaging, not executable WASM,
WebAudio, browser or hardware playback coverage.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import wave

import create_coop_defense_audio_bank_from_wav as author
from pack_coop_defense_audio import CUES, snapshot_audio
from pack_coop_defense_offline import snapshot_file, strict_json


def wav_bytes(samples, channels=1, rate=48000, width=2):
    stream = io.BytesIO()
    with wave.open(stream, 'wb') as output:
        output.setnchannels(channels)
        output.setsampwidth(width)
        output.setframerate(rate)
        output.writeframes(struct.pack('<' + 'h' * len(samples), *samples))
    return stream.getvalue()


class FromWav(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        value = os.environ.get('ORR_WAV_IMPORTER')
        if not value:
            raise RuntimeError('ORR_WAV_IMPORTER must name an explicit trusted native wav_audio_package binary')
        cls.importer = Path(value).absolute()
        if not cls.importer.is_file() or not os.access(cls.importer, os.X_OK):
            raise RuntimeError(f'ORR_WAV_IMPORTER is not an executable file: {cls.importer}')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'user'
        self.source.mkdir()
        self.inputs = []
        self.expected = {}
        for index, cue in enumerate(CUES):
            samples = [0, index + 1, -index - 1, 8192, -8192]
            path = self.source / (cue + '.wav')
            path.write_bytes(wav_bytes(samples))
            self.inputs.append(f'{cue}={path}')
            self.expected[cue + '.pcm'] = struct.pack('<II', 48000, len(samples)) + struct.pack('<5h', *samples)
        self.license = self.source / 'LICENSE.txt'
        self.license.write_bytes(b'Synthetic user fixture: CC0-1.0\n')
        self.output = self.root / 'bank'
        self.work = self.root / 'private-work'
        self.work.mkdir()
        real_temporary_directory = tempfile.TemporaryDirectory
        def scoped_temporary_directory(*args, **kwargs):
            kwargs['dir'] = self.work
            return real_temporary_directory(*args, **kwargs)
        self.patch = patch.object(author.tempfile, 'TemporaryDirectory', side_effect=scoped_temporary_directory)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def create(self, **kwargs):
        return author.create_from_wav(kwargs.get('inputs', self.inputs),
                                     kwargs.get('license', self.license),
                                     kwargs.get('output', self.output), self.importer)

    def assert_clean(self):
        self.assertFalse(self.output.exists())
        self.assertEqual(list(self.work.iterdir()), [], 'private conversion scratch must be removed')

    def rejected(self, **kwargs):
        with self.assertRaises((ValueError, OSError, RuntimeError, subprocess.SubprocessError)):
            self.create(**kwargs)
        self.assert_clean()

    def test_real_conversion_exact_pcm_digests_determinism_and_unchanged_sources(self):
        before = {p.name: p.read_bytes() for p in self.source.iterdir()}
        result = self.create()
        self.assertEqual(Path(result), self.output)
        files = {p.name: p.read_bytes() for p in self.output.iterdir()}
        self.assertEqual(set(files), {'bank.json', 'LICENSE.txt'} | set(self.expected))
        for name, pcm in self.expected.items():
            self.assertEqual(files[name], pcm)
        self.assertEqual(files['LICENSE.txt'], before['LICENSE.txt'])
        manifest = json.loads(files['bank.json'])
        self.assertEqual([r['cue'] for r in manifest['cues']], list(CUES))
        for index, record in enumerate(manifest['cues']):
            self.assertEqual(record['assetId'], f'a_000000005444{index + 1:04x}')
            self.assertEqual(record['sha256'], hashlib.sha256(files[record['file']]).hexdigest())
        other = self.root / 'second'
        relocated = self.root / 'relocated-source'
        shutil.copytree(self.source, relocated)
        moved_inputs = [f'{cue}={relocated / (cue + ".wav")}' for cue in reversed(CUES)]
        self.create(inputs=moved_inputs, license=relocated / 'LICENSE.txt', output=other)
        self.assertEqual(files, {p.name: p.read_bytes() for p in other.iterdir()})
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.source.iterdir()})
        self.assertEqual(list(self.work.iterdir()), [])
        self.assertIn('authored-audio-bank', snapshot_audio(self.output / 'bank.json', snapshot_file, strict_json))

    def test_real_cli_entrypoint(self):
        command = [os.environ.get('PYTHON', 'python3'), str(Path(author.__file__).resolve()),
                   '--license', str(self.license), '--output', str(self.output), '--importer', str(self.importer)]
        for assignment in self.inputs:
            command.extend(['--cue', assignment])
        result = subprocess.run(command, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.output / 'summon.pcm').read_bytes(), self.expected['summon.pcm'])

    def test_full_offline_packer_embeds_generated_bank_without_source_paths(self):
        import pack_coop_defense_offline as packer
        self.create()
        web = self.root / 'web'
        web.mkdir()
        source = Path(__file__).resolve().parents[1] / 'crates/orr_coop_defense_web/web'
        for name in (*packer.MODULES, *packer.EXTRA_MODULES, 'game.js', 'index.html', 'style.css'):
            target = web / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / name, target)
        (web / 'game.wasm').write_bytes(b'not executable packaging fixture')
        (web / 'assets').mkdir()
        for name, data in {'characters.json': b'{"heroes":[],"enemies":[]}',
                           'units.png': b'fixture heroes', 'enemies.png': b'fixture enemies'}.items():
            (web / 'assets' / name).write_bytes(data)
        html = packer.package(web, output=self.root / 'delivery.html', audio=self.output / 'bank.json').read_text()
        self.assertIn(snapshot_audio(self.output / 'bank.json', snapshot_file, strict_json), html)
        self.assertNotIn(str(self.source), html)
        self.assertNotIn(str(self.output), html)

    def test_invalid_cue_sets_and_paths_fail_before_importer(self):
        for inputs in [self.inputs[:-1], self.inputs + [self.inputs[0]],
                       ['unknown=x'] + self.inputs[1:], ['attack-light='] + self.inputs[1:],
                       ['attack-light'] + self.inputs[1:],
                       [f'attack-light={self.source}/../user/attack-light.wav'] + self.inputs[1:]]:
            with self.subTest(inputs=inputs), patch.object(author.subprocess, 'run') as run:
                self.rejected(inputs=inputs)
                run.assert_not_called()

    def test_all_sources_snapshotted_before_any_native_conversion(self):
        (self.source / (CUES[-1] + '.wav')).unlink()
        with patch.object(author.subprocess, 'run') as run:
            self.rejected()
            run.assert_not_called()

    def test_symlink_fifo_and_directory_wav_inputs_reject_without_blocking(self):
        first = self.source / (CUES[0] + '.wav')
        good = first.read_bytes()
        target = self.source / 'target.wav'
        target.write_bytes(good)
        first.unlink()
        first.symlink_to(target)
        self.rejected()
        first.unlink()
        os.mkfifo(first)
        self.rejected()
        first.unlink()
        first.mkdir()
        self.rejected()

    def test_bad_wav_contracts_and_size_leave_no_output(self):
        first = self.source / (CUES[0] + '.wav')
        for data in [wav_bytes([1, 2], channels=2), wav_bytes([1], rate=44100),
                     wav_bytes([8193]), wav_bytes([-8193]), wav_bytes([]),
                     wav_bytes([1])[:-1], b'not wav', b'x' * (1024 * 1024 + 1),
                     wav_bytes([0] * 48001)]:
            with self.subTest(length=len(data), prefix=data[:16]):
                first.write_bytes(data)
                self.rejected()

    def test_license_rejections_and_symlink_leave_no_output(self):
        for data in [b' ', b'\xff', b'x' * (64 * 1024 + 1)]:
            self.license.write_bytes(data)
            self.rejected()
        target = self.source / 'license-target'
        target.write_bytes(b'license')
        self.license.unlink()
        self.license.symlink_to(target)
        self.rejected()

    def test_output_collisions_preserve_existing_file_directory_and_symlink(self):
        self.output.write_bytes(b'keep file')
        with self.assertRaises((ValueError, OSError, RuntimeError)):
            self.create()
        self.assertEqual(self.output.read_bytes(), b'keep file')
        self.output.unlink()
        self.output.mkdir()
        sentinel = self.output / 'keep'
        sentinel.write_bytes(b'keep directory')
        with self.assertRaises((ValueError, OSError, RuntimeError)):
            self.create()
        self.assertEqual(list(self.output.iterdir()), [sentinel])
        self.assertEqual(sentinel.read_bytes(), b'keep directory')
        shutil.rmtree(self.output)
        target = self.root / 'untouched'
        target.mkdir()
        self.output.symlink_to(target, target_is_directory=True)
        with self.assertRaises((ValueError, OSError, RuntimeError)):
            self.create()
        self.assertTrue(self.output.is_symlink())
        self.assertEqual(list(target.iterdir()), [])
        self.assertEqual(list(self.work.iterdir()), [])

    def test_injected_timeout_cleans_private_scratch_without_output(self):
        with patch.object(author.subprocess, 'run',
                          side_effect=subprocess.TimeoutExpired('trusted-importer', 30)):
            self.rejected()

    def test_injected_native_output_mismatch_is_rejected_before_publication(self):
        actual_run = subprocess.run
        for fault in ('extra-object', 'digest'):
            def corrupt_output(command, **kwargs):
                result = actual_run(command, **kwargs)
                self.assertEqual(result.returncode, 0)
                objects = Path(command[command.index('--out') + 1]) / 'cooked' / 'objects'
                if fault == 'extra-object':
                    (objects / 'unexpected.bin').write_bytes(b'extra')
                else:
                    payload = next(objects.iterdir())
                    payload.write_bytes(payload.read_bytes() + b'changed')
                return result
            with self.subTest(fault=fault), patch.object(author.subprocess, 'run',
                                                       side_effect=corrupt_output):
                self.rejected()

    def test_last_native_conversion_failure_cleans_scratch_without_publishing(self):
        # Last fixed cue fails in the real native decoder, after nine successful
        # conversions. This is not a mocked process or copied decoder.
        (self.source / (CUES[-1] + '.wav')).write_bytes(wav_bytes([1], rate=44100))
        before = {p.name: p.read_bytes() for p in self.source.iterdir()}
        actual_run = subprocess.run
        with patch.object(author.subprocess, 'run', wraps=actual_run) as run:
            self.rejected()
            self.assertEqual(run.call_count, 10)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.source.iterdir()})


if __name__ == '__main__':
    unittest.main()
