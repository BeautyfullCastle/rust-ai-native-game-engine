#!/usr/bin/env python3
"""Exact-candidate CPU/PCM acceptance; never opens a device, browser or window."""
import hashlib
import json
import math
import os
from pathlib import Path
import re
import runpy
import shutil
import struct
import subprocess
import sys
import unittest

EXTERNAL = 'defense_audio::tests::cli_authored_bank_source_hidden'
ROOT = Path(__file__).resolve().parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate(output, expected):
    # The pre-existing package FIFO regression deliberately self-executes. With
    # one test thread its child's successful output interrupts the parent's line.
    # Admit exactly that bounded nested case, not arbitrary extra test summaries.
    fifo = 'tests::fifo_inputs_fail_without_blocking'
    if fifo in expected:
        child = (r'^test ' + re.escape(fifo) + r' \.\.\. \n+'
                 r'running 1 test\n'
                 r'test ' + re.escape(fifo) + r' \.\.\. ok\n+'
                 r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; '
                 + str(len(expected) - 1) + r' filtered out; finished in [^\n]+\n+'
                 r'ok\n')
        output, count = re.subn(child, 'test ' + fifo + ' ... ok\n', output, flags=re.M)
        if count != 1:
            raise ValueError('Missing or invalid bounded package FIFO child result')
    summaries = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in .+$', output, re.M)
    cases = re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?: .*)?$', output, re.M)
    names = [name for name, _ in cases]
    if (len(summaries) != 1 or tuple(map(int, summaries[0][:4])) != (len(expected), 0, 0, 0)
            or re.findall(r'^running (\d+) tests?$', output, re.M) != [str(len(expected))]
            or not expected or len(names) != len(set(names)) or set(names) != set(expected)
            or any(status != 'ok' for _, status in cases)):
        raise ValueError('Incomplete, skipped, duplicate or changed test inventory')


def main():
    if Path.cwd().resolve() != ROOT:
        raise RuntimeError('Run from repository root')
    evidence_root = Path(os.environ['EVIDENCE']).resolve()
    if evidence_root == ROOT or ROOT in evidence_root.parents:
        raise RuntimeError('Evidence must be outside the repository')
    evidence_root.mkdir(parents=True, exist_ok=True)
    evidence = evidence_root / 'acceptance'
    # A repeat must choose a fresh directory; never overwrite an earlier failure.
    evidence.mkdir()
    def clean_source(label):
        status = subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=all'], text=True)
        (evidence / (label + '-status.txt')).write_text(status)
        if status:
            raise RuntimeError('Source must be clean, including index and untracked files')
        return subprocess.check_output(['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'], text=True)
    source_identity = clean_source('before')
    (evidence / 'source.txt').write_text(source_identity)
    helpers = runpy.run_path(str(ROOT / 'tools/check-authoring-controls.py'))
    run = helpers['run_logged']
    inventory = json.loads((ROOT / 'tools/stormglass_native_audio_inventory.json').read_text())
    cargo = ['cargo']
    def command(verb, package, features, target):
        return cargo + [verb, '--release', '--locked', '-p', package] + (['--features', features] if features else []) + target
    def producer(label, package, features, target):
        output = run(command('test', package, features, target) + ['--no-run', '--message-format=json'], evidence / (label + '-build.log'))
        candidates = set()
        for line in output.splitlines():
            if line.startswith('{'):
                item = json.loads(line)
                if item.get('reason') == 'compiler-artifact' and item.get('profile', {}).get('test') and item.get('executable'):
                    candidates.add(item['executable'])
        if len(candidates) != 1:
            raise RuntimeError('Expected one test producer: ' + label)
        result = evidence / (label + '-producer')
        shutil.copy2(candidates.pop(), result)
        return result
    def suite(label, package, features, target, args=()):
        binary = producer(label, package, features, target)
        output = run([str(binary), *args, '--test-threads=1', '--color=never'], evidence / (label + '.log'))
        validate(output, inventory[label])
        return binary
    # These configurations stay separate: feature unification must not hide a defect.
    binaries = {}
    binaries['mixer'] = suite('mixer', 'orr_audio', '', ['--test', 'pcm'])
    for label, feature in [('admission', 'coop-audio-admission'), ('loader', 'coop-audio-load'), ('authoring', 'audio-authoring'), ('web', 'coop-web-authoring')]:
        binaries[label] = suite(label, 'orr_package', feature, ['--lib'])
    for label, feature in [('audio-save', 'coop-defense-audio,coop-defense-run-save'), ('audio-only', 'coop-defense-audio'), ('save-only', 'coop-defense-run-save'), ('game-only', 'coop-defense')]:
        args = ['defense_'] + (['--skip', EXTERNAL] if label.startswith('audio-') else [])
        binaries[label] = suite(label, 'orr_sample', feature, ['--lib'], args)
    run(command('build', 'orr_package', 'audio-authoring', ['--bin', 'orr_pkg']), evidence / 'cli-build.log')
    target = Path(os.environ.get('CARGO_TARGET_DIR', 'target')).resolve()
    cli = evidence / 'orr_pkg'
    shutil.copy2(target / 'release/orr_pkg', cli)
    fixture = evidence / 'fixture'
    run([sys.executable, 'tools/generate_stormglass_audio_fixture.py', str(fixture)], evidence / 'fixture-generation.log')
    expected = json.loads((fixture / 'expected.json').read_text())
    args = [str(cli), 'create-coop-audio-bank', '--license', str(fixture / 'originals/LICENSE.txt'), '--output', str(fixture / 'published-bank')]
    for cue in expected['cues']:
        args += ['--cue', cue['cue'] + '=' + str(fixture / 'originals' / (cue['cue'] + '.pcm'))]
    run(args, evidence / 'cli-authoring.log')
    consumer = evidence / 'consumer'
    consumer.mkdir()
    (fixture / 'published-bank').rename(consumer / 'bank')
    (fixture / 'originals').rename(fixture / 'originals-preserved-hidden')
    if (fixture / 'originals').exists() or (fixture / 'published-bank').exists():
        raise RuntimeError('Original paths remain present')
    os.environ.update(ORR_STORMGLASS_AUDIO_BANK=str(consumer / 'bank'), ORR_STORMGLASS_AUDIO_EXPECTED=str(fixture / 'expected.json'), ORR_STORMGLASS_AUDIO_ORIGINALS=str(fixture / 'originals'))
    receipts = {}
    for label in ['audio-save', 'audio-only']:
        receipt_dir = consumer / label
        os.environ['ORR_STORMGLASS_AUDIO_EVIDENCE_DIR'] = str(receipt_dir)
        output = run([str(binaries[label]), EXTERNAL, '--ignored', '--exact', '--test-threads=1'], evidence / (label + '-source-hidden.log'))
        validate(output, [EXTERNAL])
        receipt = json.loads((receipt_dir / 'native-authored-receipt.json').read_text())
        pcm_path = receipt_dir / 'native-authored-mixed.f32le'
        pcm = pcm_path.read_bytes()
        values = [value[0] for value in struct.iter_unpack('<f', pcm)]
        if len(pcm) != 720000 * 2 * 4 or not all(math.isfinite(x) for x in values) or not any(values) or max(map(abs, values)) > 0.25001:
            raise RuntimeError('Invalid mixed PCM')
        for key in ['source_hidden', 'mixed_nonzero', 'actual_app_egui']:
            if receipt[key] is not True:
                raise RuntimeError('Missing mandatory scope: ' + key)
        if (receipt['device_playback_verified'] is not False or receipt['producer_sha256'] != sha(binaries[label])
                or receipt['mixed_pcm_sha256'] != sha(pcm_path) or len(receipt['decoded']) != 10 or len(receipt['mapped_cues']) != 11
                or receipt['license_sha256'] != expected['license_sha256'] or receipt['license_sha256'] != sha(consumer / 'bank/LICENSE.txt')):
            raise RuntimeError('Wrong fixture or producer receipt')
        if receipt['mixed_peak'] != max(map(abs, values)) or not isinstance(receipt['started'], int) or receipt['started'] <= 0:
            raise RuntimeError('Mixed PCM peak or voice receipt mismatch')
        if receipt['bank_manifest_sha256'] != sha(consumer / 'bank/bank.json'):
            raise RuntimeError('Manifest receipt mismatch')
        wanted = {cue['cue']: cue for cue in expected['cues']}
        decoded = {cue['cue']: cue for cue in receipt['decoded']}
        if len(wanted) != 10 or set(decoded) != set(wanted):
            raise RuntimeError('Changed decoded cue inventory')
        manifest = json.loads((consumer / 'bank/bank.json').read_text())
        records = manifest['cues']
        if len(records) != 10 or {cue['cue'] for cue in records} != set(wanted):
            raise RuntimeError('Changed manifest cue inventory')
        for cue in records:
            name = cue['cue']
            raw = (consumer / 'bank' / (name + '.pcm')).read_bytes()
            if sha(consumer / 'bank' / (name + '.pcm')) != wanted[name]['pcm_sha256'] or struct.unpack('<II', raw[:8]) != (48000, 4800):
                raise RuntimeError('Published PCM fixture mismatch')
            values_i16 = [v[0] for v in struct.iter_unpack('<h', raw[8:])]
            stereo = b''.join(struct.pack('<ff', value / 32768, value / 32768) for value in values_i16)
            if hashlib.sha256(stereo).hexdigest() != wanted[name]['decoded_stereo_f32le_sha256'] or decoded[name]['decoded_stereo_f32le_sha256'] != wanted[name]['decoded_stereo_f32le_sha256'] or decoded[name]['sample_probes'] != wanted[name]['sample_probes']:
                raise RuntimeError('Decoded authored samples mismatch')
        mappings = [('summon', 0, 'summon'), ('merge', 0, 'merge'), ('death', 0, 'death'), ('release', 0, 'attack-light'), ('release', 1, 'attack-heavy'), ('release', 2, 'attack-heavy'), ('release', 3, 'attack-magic'), ('hit', 0, 'impact-light'), ('hit', 1, 'impact-heavy'), ('hit', 2, 'impact-heavy'), ('hit', 3, 'impact-light')]
        if [(cue['event'], cue['role'], cue['cue']) for cue in receipt['mapped_cues']] != mappings:
            raise RuntimeError('Event mapping receipt mismatch')
        receipts[label] = receipt
    if receipts['audio-save']['mixed_pcm_sha256'] != receipts['audio-only']['mixed_pcm_sha256']:
        raise RuntimeError('Run Save feature changed actual mixed output')
    for label, package, features, targets in [
        ('strict-mixer', 'orr_audio', '', ['--lib', '--test', 'pcm']),
        ('strict-mixer-native', 'orr_audio', 'native', ['--lib']),
        ('strict-admission', 'orr_package', 'coop-audio-admission', ['--lib']),
        ('strict-loader', 'orr_package', 'coop-audio-load', ['--lib']),
        ('strict-authoring', 'orr_package', 'audio-authoring', ['--lib', '--bin', 'orr_pkg']),
        ('strict-web', 'orr_package', 'coop-web-authoring', ['--lib']),
        ('strict-audio-save', 'orr_sample', 'coop-defense-audio,coop-defense-run-save', ['--lib']),
        ('strict-audio-only', 'orr_sample', 'coop-defense-audio', ['--lib']),
        ('strict-save-only', 'orr_sample', 'coop-defense-run-save', ['--lib']),
        ('strict-game-only', 'orr_sample', 'coop-defense', ['--lib']),
        ('strict-native', 'orr_sample', 'coop-defense-audio,audio-native,coop-defense-run-save', ['--lib', '--bin', 'coop_defense'])]:
        run(command('clippy', package, features, targets) + ['--', '-D', 'warnings'], evidence / (label + '.log'))
    for package in ['orr_package', 'orr_sample']:
        run(command('check', package, '', ['--no-default-features', '--lib']), evidence / (package + '-feature-off.log'))
    run(command('build', 'orr_sample', 'coop-defense-audio,audio-native,coop-defense-run-save', ['--bin', 'coop_defense']), evidence / 'native-build.log')
    native = evidence / 'coop_defense'
    shutil.copy2(target / 'release/coop_defense', native)
    scene = evidence / 'stormglass-short.scene.yaml'
    shutil.copy2('examples/static-game-starter/scenes/stormglass-short.scene.yaml', scene)
    output = run([str(native), '--headless', '--audio-bank', str(consumer / 'bank'), '--audio', 'off', '--scene', str(scene), '--ticks', '6000', '--demo'], evidence / 'native-headless-off.log')
    rows = re.findall(r'^Stormglass tick=(\d+) wave=(\d+) phase=(\d+) reactor=(\d+) kills=(\d+) merges=(\d+) checksum=(0x[0-9a-f]{16})$', output, re.M)
    if rows != [('1884', '3', '1', '75', '72', '29', '0x7f44dac0fea52bc1')]:
        raise RuntimeError('Authored gameplay baseline changed')
    if clean_source('after') != source_identity:
        raise RuntimeError('Source commit or tree changed during validation')
    scope = {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(), 'tree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], text=True).strip(), 'counts': {k: len(v) for k, v in inventory.items()}, 'test_producers': {k: sha(v) for k, v in binaries.items()}, 'cli_sha256': sha(cli), 'native_binary_sha256': sha(native), 'scene_sha256': sha(scene), 'native_headless_result': rows[0], 'source_hidden': True, 'cpu_app_egui': True, 'actual_kira_pcm': True, 'native_device_listening': False, 'native_window': False, 'physical_input': False, 'browser': False, 'gpu': False, 'full_workspace': False, 'mixed_pcm_sha256': receipts['audio-save']['mixed_pcm_sha256']}
    (evidence / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')


class ValidatorTests(unittest.TestCase):
    def output(self):
        return 'running 2 tests\ntest one ... ok\ntest two ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.01s\n'
    def test_accept_exact(self):
        validate(self.output(), ['one', 'two'])
    def test_accept_only_exact_fifo_child(self):
        fifo = 'tests::fifo_inputs_fail_without_blocking'
        child = ('test ' + fifo + ' ... \nrunning 1 test\ntest ' + fifo
                 + ' ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; '
                 '0 measured; 1 filtered out; finished in 0.01s\n\nok\n')
        good = self.output().replace('test two ... ok\n', child)
        validate(good, ['one', fifo])
        for bad in [good.replace('1 filtered out', '0 filtered out'),
                    good.replace('1 passed', '0 passed'),
                    good.replace('\n\nok\n', '\n\nFAILED\n')]:
            with self.subTest(output=bad), self.assertRaises(ValueError):
                validate(bad, ['one', fifo])
    def test_reject_empty(self):
        with self.assertRaises(ValueError):
            validate('', [])
    def test_reject_replacement_duplicate_missing_and_skipped(self):
        for text in [self.output().replace('test two', 'test other'), self.output().replace('test two', 'test one'), self.output().replace('test two ... ok\n', ''), self.output().replace('... ok', '... ignored', 1), self.output().replace('0 ignored', '1 ignored'), self.output() * 2]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                validate(text, ['one', 'two'])


if __name__ == '__main__':
    if '--self-test' in sys.argv:
        unittest.main(argv=[sys.argv[0]])
    else:
        main()
