"""Rust attack-timing authoring and real offline-consumer acceptance.

Required: ORR_RUST_PACKAGE_BIN (candidate), ORR_BASELINE_PACKAGE_BIN (pre-change
trusted native CLI), ORR_COOP_WEB_BUILD_DIR (fresh actual WASM + current web assets),
Node on PATH and Pillow for the unchanged independent offline checker. Missing
fixtures fail, never skip. This driver never builds, downloads or installs tools.

Run: python3 -B -m unittest discover -s tools -p test_rust_coop_attack_timing.py -v
Optional ORR_ATTACK_TIMING_EVIDENCE_DIR is an existing absolute directory outside
this checkout; then each test retains its inputs, exact argv/stdout/stderr and
consumer receipt there. Otherwise temporary files are cleaned after each test.
Legacy Python authoring is deliberately not an oracle for the new option.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

import pack_coop_defense_offline as offline

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parent
TIMING = ('windupMs', 'releaseMs', 'recoveryMs')
WEB_SOURCE = REPO / 'crates/orr_coop_defense_web/web'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def png():
    """Four real 2x2 RGBA frames in an 8x2 atlas, no Pillow fixture generator."""
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    row = b''.join(bytes(color) * 2 for color in
                   [(255, 0, 0, 255), (0, 255, 0, 255), (0, 0, 255, 255), (255, 255, 0, 255)])
    pixels = (b'\0' + row) * 2
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 8, 2, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(pixels)) + chunk(b'IEND', b''))


def executable(name):
    value = os.environ.get(name)
    if not value or not Path(value).is_absolute() or not Path(value).is_file() or not os.access(value, os.X_OK):
        raise RuntimeError(f'{name} must identify an explicit trusted executable')
    return Path(value)


class RustCoopAttackTiming(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = executable('ORR_RUST_PACKAGE_BIN')
        cls.baseline = executable('ORR_BASELINE_PACKAGE_BIN')
        value = os.environ.get('ORR_COOP_WEB_BUILD_DIR')
        if not value or not Path(value).is_absolute() or not Path(value).is_dir():
            raise RuntimeError('ORR_COOP_WEB_BUILD_DIR must identify the actual reviewed fresh WASM/web build')
        cls.build = Path(value)
        if not shutil.which('node'):
            raise RuntimeError('Node is required for actual native admission and offline consumption')
        # Fail on stale/missing modules, rather than passing with a mock or a
        # different consumer implementation accidentally copied from another tree.
        cls.build_bytes = {name: (cls.build / name).read_bytes() for name in offline.ASSETS}
        if cls.build_bytes['game.wasm'][:8] != b'\0asm\x01\0\0\0':
            raise RuntimeError('Actual trusted WASM is required, not placeholder bytes')
        for name in ('game.js', *offline.MODULES, *offline.EXTRA_MODULES):
            if cls.build_bytes[name] != (WEB_SOURCE / name).read_bytes():
                raise RuntimeError(f'Fresh build module differs from this checkout: {name}')
        value = os.environ.get('ORR_ATTACK_TIMING_EVIDENCE_DIR')
        cls.evidence = None
        if value:
            cls.evidence = Path(value).resolve()
            if not Path(value).is_absolute() or not cls.evidence.is_dir() or cls.evidence.is_relative_to(REPO):
                raise RuntimeError('ORR_ATTACK_TIMING_EVIDENCE_DIR must be an existing absolute directory outside the checkout')

    @classmethod
    def tearDownClass(cls):
        for name, data in cls.build_bytes.items():
            if (cls.build / name).read_bytes() != data:
                raise AssertionError(f'Supplied trusted build changed during test: {name}')

    def setUp(self):
        if self.evidence:
            self.root = Path(tempfile.mkdtemp(prefix=self._testMethodName + '-', dir=self.evidence))
        else:
            temp = tempfile.TemporaryDirectory(prefix='orr-attack-timing-')
            self.addCleanup(temp.cleanup)
            self.root = Path(temp.name)
        self.assets = self.root / 'assets'
        (self.assets / 'art').mkdir(parents=True)
        self.image = self.assets / 'art/hero.png'
        self.original_png = png()
        self.image.write_bytes(self.original_png)
        self.sources = {self.image: self.original_png}
        self.invocations = 0

    def tearDown(self):
        for path, data in self.sources.items():
            self.assertEqual(path.read_bytes(), data, f'authored source changed: {path}')

    def execute(self, arguments, timeout=60):
        self.invocations += 1
        stem = self.root / f'command-{self.invocations:03}'
        argv = list(map(str, arguments))
        stem.with_suffix('.argv.json').write_text(json.dumps(argv, indent=2) + '\n')
        result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout,
                                env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
        stem.with_suffix('.stdout').write_text(result.stdout)
        stem.with_suffix('.stderr').write_text(result.stderr)
        stem.with_suffix('.exit').write_text(str(result.returncode) + '\n')
        return result

    def create_args(self, output, extra=(), *, binary=None, role='unit:0', attack=True, configured=True):
        args = [binary or self.binary, 'create-coop-animation', '--assets-dir', self.assets,
                '--output', output, '--role', role, '--image', 'art/hero.png',
                '--frame', 'idle', '0', '0', '2', '2']
        if attack:
            for x in (0, 2, 4, 6):
                args += ['--frame', 'attack', str(x), '0', '2', '2']
            if configured:
                args += ['--clip', 'attack', '10', 'once']
        return args + ['--pivot', '1', '2'] + list(map(str, extra))

    def receipt(self, result, output, *, sources=None):
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = json.loads(result.stdout)
        data = output.read_bytes()
        self.assertEqual(receipt['version'], 1)
        self.assertEqual(receipt['manifestBytes'], len(data))
        self.assertEqual(receipt['manifestSha256'], digest(data))
        if sources is not None:
            self.assertEqual(receipt['roles'], sorted(json.loads(data)))
        else:
            self.assertNotIn('roles', receipt)
            self.assertNotIn('sources', receipt)
        self.assertEqual(receipt['images'], [{'name': 'art/hero.png', 'bytes': len(self.original_png),
                         'sha256': digest(self.original_png), 'width': 8, 'height': 2,
                         'id': 'authored-animation-image-0'}])
        if sources is not None:
            self.assertEqual(receipt['sources'], [{'bytes': path.stat().st_size,
                             'sha256': digest(path.read_bytes())} for path in sources])
        output.with_suffix('.receipt.json').write_text(result.stdout)
        return receipt

    def create(self, name, extra=(), **kwargs):
        output = self.root / name
        self.receipt(self.execute(self.create_args(output, extra, **kwargs)), output)
        self.sources[output] = output.read_bytes()
        return output

    def compose(self, output, manifests, binary=None):
        args = [binary or self.binary, 'compose-coop-animations', '--assets-dir', self.assets, '--output', output]
        for path in manifests:
            args += ['--manifest', path]
        self.receipt(self.execute(args), output, sources=manifests)
        self.sources[output] = output.read_bytes()
        return output

    def assert_rejected(self, args, output):
        result = self.execute(args)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertTrue(result.stderr.strip(), 'refusal must explain the error')
        self.assertFalse(os.path.lexists(output), 'failed admission must not publish output')
        self.assertEqual(self.image.read_bytes(), self.original_png)

    def copy_build(self):
        build = self.root / 'fresh-web'
        shutil.copytree(self.build, build)
        (build / 'assets/art').mkdir(parents=True, exist_ok=True)
        (build / 'assets/art/hero.png').write_bytes(self.original_png)
        return build

    def pack_args(self, build, manifest, output):
        return [self.binary, 'pack-coop-offline', '--build-dir', build,
                '--animations', manifest, '--output', output]

    def test_omitted_timing_exact_prechange_native_bytes(self):
        # The trusted old Rust binary, not a hand-written expected serializer or
        # legacy Python writer, defines exact backward-compatible omitted bytes.
        for index, options in enumerate([{}, {'configured': False}, {'attack': False}]):
            with self.subTest(options=options):
                old = self.create(f'old-{index}.json', binary=self.baseline, **options)
                new = self.create(f'new-{index}.json', **options)
                self.assertEqual(new.read_bytes(), old.read_bytes())
                self.assertEqual(json.loads(new.with_suffix('.receipt.json').read_text()),
                                 json.loads(old.with_suffix('.receipt.json').read_text()))
                for clip in json.loads(new.read_bytes())['unit:0']['clips'].values():
                    self.assertTrue(set(TIMING).isdisjoint(clip))
                old_composed = self.compose(self.root / f'old-composed-{index}.json', [old], self.baseline)
                new_composed = self.compose(self.root / f'new-composed-{index}.json', [new])
                self.assertEqual(new_composed.read_bytes(), old_composed.read_bytes())
                self.assertEqual(json.loads(new_composed.with_suffix('.receipt.json').read_text()),
                                 json.loads(old_composed.with_suffix('.receipt.json').read_text()))

    def test_independent_bounds_fractions_and_exponents(self):
        values = [(1, 1, 1), (2000, 2000, 2000), (1, 2000, 12.5), (2000, 1, 1999.75),
                  (200.25, 50.5, 150.75), ('1e2', '5e1', '1.5e2'), (2000, 1999.5, 1)]
        for index, numbers in enumerate(values):
            with self.subTest(numbers=numbers):
                role = 'enemy:4' if index == len(values) - 1 else 'unit:0'
                source = self.create(f'accepted-{index}.json', ['--attack-timing', *numbers], role=role)
                value = json.loads(source.read_bytes())[role]
                self.assertEqual({key: value['clips']['attack'][key] for key in TIMING},
                                 dict(zip(TIMING, map(float, numbers))))
                self.assertTrue(set(TIMING).isdisjoint(value['clips']['idle']))
                composed = self.compose(self.root / f'composed-{index}.json', [source])
                self.assertEqual(json.loads(composed.read_bytes())[role], value)

    def test_invalid_each_position_arity_duplicate_and_missing_attack(self):
        invalid = ('0', '-1', '0.999', '2000.001', 'NaN', 'nan', 'inf', '-inf', 'Infinity', '1e309', 'true', 'nope')
        case = 0
        for slot in range(3):
            for bad in invalid:
                case += 1
                values = ['200', '50', '150']; values[slot] = bad
                with self.subTest(slot=slot, value=bad):
                    output = self.root / f'invalid-{case}.json'
                    self.assert_rejected(self.create_args(output, ['--attack-timing', *values]), output)
        for index, extra in enumerate([
            ['--attack-timing'], ['--attack-timing', '200'], ['--attack-timing', '200', '50'],
            ['--attack-timing', '200', '50', '150', '1'],
            ['--attack-timing', '200', '50', '150', '--attack-timing', '200', '50', '150'],
        ]):
            with self.subTest(extra=extra):
                output = self.root / f'invalid-argv-{index}.json'
                self.assert_rejected(self.create_args(output, extra), output)
        output = self.root / 'missing-attack.json'
        self.assert_rejected(self.create_args(output, ['--attack-timing', '200', '50', '150'], attack=False), output)
        self.assertFalse(list(self.root.glob('.orr-offline-*')), 'failed calls must not leave publication temporary files')

    def test_no_overwrite_or_source_alias_in_create_compose_and_pack(self):
        manifest = self.create('source.json', ['--attack-timing', '200', '50', '150'])
        build = self.copy_build()
        for operation in ('create', 'compose', 'pack'):
            for target in ('file', 'dangling-symlink'):
                output = self.root / f'{operation}-{target}'
                if target == 'file':
                    output.write_bytes(b'keep existing bytes')
                else:
                    output.symlink_to('missing-target')
                args = (self.create_args(output, ['--attack-timing', '200', '50', '150']) if operation == 'create' else
                        [self.binary, 'compose-coop-animations', '--assets-dir', self.assets,
                         '--manifest', manifest, '--output', output] if operation == 'compose' else
                        self.pack_args(build, manifest, output))
                with self.subTest(operation=operation, target=target):
                    result = self.execute(args)
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    if target == 'file':
                        self.assertEqual(output.read_bytes(), b'keep existing bytes')
                    else:
                        self.assertTrue(output.is_symlink())
                        self.assertEqual(os.readlink(output), 'missing-target')
                        self.assertFalse((self.root / 'missing-target').exists())
        for args in [self.create_args(self.image, ['--attack-timing', '200', '50', '150']),
                     [self.binary, 'compose-coop-animations', '--assets-dir', self.assets,
                      '--manifest', manifest, '--output', manifest]]:
            result = self.execute(args)
            self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(manifest.read_bytes(), self.sources[manifest])
        self.assertEqual(self.image.read_bytes(), self.original_png)

    def test_generated_composed_packed_actual_wasm_source_hidden_consumer(self):
        unit = self.create('authored-unit.json', ['--attack-timing', '200', '50', '150'])
        default_unit = self.create('default-unit.json')
        enemy = self.create('enemy.json', role='enemy:0', attack=False)
        authored = self.compose(self.root / 'composed.json', [unit, enemy])
        default = self.compose(self.root / 'default-composed.json', [default_unit, enemy])
        for role, source in [('unit:0', unit), ('enemy:0', enemy)]:
            self.assertEqual(json.loads(authored.read_bytes())[role], json.loads(source.read_bytes())[role])
        build = self.copy_build()
        build_before = {path.relative_to(build): path.read_bytes() for path in build.rglob('*') if path.is_file()}
        html, default_html = self.root / 'authored.html', self.root / 'default.html'
        for manifest, output in [(authored, html), (default, default_html)]:
            result = self.execute(self.pack_args(build, manifest, output), timeout=120)
            self.assertEqual(result.returncode, 0, result.stderr)
            checked = self.execute([sys.executable, '-B', TOOLS / 'check_coop_defense_offline.py', build, output], timeout=120)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            self.assertNotIn(str(self.root), output.read_text(), 'local authoring paths must not leak into HTML')
        for name, data in build_before.items():
            self.assertEqual((build / name).read_bytes(), data, f'packaging changed build source: {name}')
        scene = self.root / 'scene.yaml'
        created = self.execute([self.binary, 'create-coop-scene', '--wasm', self.build / 'game.wasm',
                                '--output', scene, '--starting-gold', '250', '--reactor-hp', '75', '--last-wave', '3'])
        self.assertEqual(created.returncode, 0, created.stderr)
        self.sources[scene] = scene.read_bytes()
        # Only external verification snapshots remain at supplied argv paths.
        # Original generated manifests/art/build locations disappear before boot;
        # the real offline loader must use embedded bytes, with all fetch denied.
        snapshots = self.root / 'verification-snapshots'; snapshots.mkdir()
        png_snapshot = snapshots / 'hero.png'; png_snapshot.write_bytes(self.original_png)
        authored_snapshot = snapshots / 'authored.json'; authored_snapshot.write_bytes(authored.read_bytes())
        default_snapshot = snapshots / 'default.json'; default_snapshot.write_bytes(default.read_bytes())
        build.rename(self.root / 'source-hidden-web')
        self.assets.rename(self.root / 'source-hidden-assets')
        self.sources[self.root / 'source-hidden-assets/art/hero.png'] = self.sources.pop(self.image)
        hidden_manifests = self.root / 'source-hidden-manifests'; hidden_manifests.mkdir()
        for path in [unit, default_unit, enemy, authored, default]:
            hidden = hidden_manifests / path.name
            path.rename(hidden)
            self.sources[hidden] = self.sources.pop(path)
            self.assertFalse(path.exists())
        self.assertFalse(build.exists())
        self.assertFalse(self.assets.exists())
        consumed = self.execute(['node', TOOLS / 'test_coop_attack_timing_actual.mjs', html, default_html,
                                 authored_snapshot, default_snapshot, png_snapshot, self.build / 'game.wasm', scene], timeout=120)
        self.assertEqual(consumed.returncode, 0, consumed.stderr)
        report = json.loads(consumed.stdout)
        (self.root / 'actual-consumer-receipt.json').write_text(json.dumps(report, indent=2) + '\n')
        self.assertTrue(report['passed'])
        self.assertEqual(report['manifestSha256'], digest(authored_snapshot.read_bytes()))
        self.assertEqual(report['wasmSha256'], digest(self.build_bytes['game.wasm']))
        self.assertEqual(report['networkRequests'], 0)
        self.assertEqual(len(report['realRelease']['releaseSamples']), 6)
        self.assertEqual(len(report['syntheticOnset']), 7)
        self.assertGreater(report['comparedTicks'], 3)
        self.assertIn('not browser/GPU', report['coverage'])


if __name__ == '__main__':
    unittest.main()
