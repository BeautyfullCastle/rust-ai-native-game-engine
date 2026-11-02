"""Real Rust/Python FPS-animation differential and actual offline packaging.

Requires ORR_RUST_PACKAGE_BIN, ORR_COOP_WEB_BUILD_DIR and Node on PATH.
Python/Pillow are legacy-oracle test dependencies, not new Rust dependencies.
Node sampler/validator and final packer/checker run unmocked. No browser/GPU claim.
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

import pack_coop_defense_offline as packer

TOOLS = Path(__file__).resolve().parent


def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


def png(idat=None, extra=b''):
    header = chunk(b'IHDR', struct.pack('>IIBBBBB', 4, 2, 8, 6, 0, 0, 0))
    pixels = b''.join(b'\0' + bytes([255, 0, 0, 255]) * 4 for _ in range(2))
    return b'\x89PNG\r\n\x1a\n' + header + extra + chunk(b'IDAT', zlib.compress(pixels) if idat is None else idat) + chunk(b'IEND', b'')


class RustCoopWebAnimation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binary = os.environ.get('ORR_RUST_PACKAGE_BIN')
        build = os.environ.get('ORR_COOP_WEB_BUILD_DIR')
        if not binary or not Path(binary).is_absolute() or not Path(binary).is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError('ORR_RUST_PACKAGE_BIN must identify an explicit trusted executable')
        if not build or not Path(build).is_absolute() or not Path(build).is_dir():
            raise RuntimeError('ORR_COOP_WEB_BUILD_DIR must identify the actual reviewed clean WASM build')
        if not shutil.which('node'):
            raise RuntimeError('Actual Node sampler required on PATH')
        cls.binary, cls.build = binary, Path(build)
        if (cls.build / 'game.wasm').read_bytes()[:8] != b'\0asm\x01\0\0\0':
            raise RuntimeError('Actual WASM build required, not a placeholder fixture')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.assets = self.root / 'assets'
        (self.assets / 'art').mkdir(parents=True)
        self.image = self.assets / 'art/hero.png'
        self.image.write_bytes(png())
        (self.assets / 'art/enemy.png').write_bytes(png())
        self.definition = {'image': 'art/hero.png', 'clips': {
            'idle': {'frames': [[0, 0, 2, 2], [2, 0, 2, 2]], 'fps': 12, 'loop': True}}}
        self.unit = self.manifest('unit.json', {'unit:0': self.definition})
        self.enemy = self.manifest('enemy.json', {'enemy:0': {**self.definition, 'image': 'art/enemy.png'}})

    def manifest(self, name, value=None, raw=None):
        path = self.root / name
        path.write_bytes(raw if raw is not None else json.dumps(value).encode())
        return path

    def run_cli(self, native, compose, output, *, extra=(), manifests=None):
        name = 'compose-coop-animations' if compose else 'create-coop-animation'
        script = 'compose_coop_defense_animations.py' if compose else 'create_coop_defense_animation.py'
        command = [self.binary, name] if native else [sys.executable, '-B', str(TOOLS / script)]
        command += ['--assets-dir', str(self.assets), '--output', str(output)]
        if compose:
            for path in [self.unit, self.enemy] if manifests is None else manifests:
                command += ['--manifest', str(path)]
        else:
            command += ['--role', 'unit:0', '--image', 'art/hero.png',
                        '--frame', 'idle', '0', '0', '2', '2', '--frame', 'idle', '2', '0', '2', '2']
        command += list(map(str, extra))
        return subprocess.run(command, capture_output=True, text=True, timeout=40,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def success(self, native, compose, output, **kw):
        result = self.run_cli(native, compose, output, **kw)
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = json.loads(result.stdout)
        data = output.read_bytes()
        self.assertEqual(receipt['manifestBytes'], len(data))
        self.assertEqual(receipt['manifestSha256'], hashlib.sha256(data).hexdigest())
        self.assertEqual(receipt['version'], 1)
        names = [item['name'] for item in receipt['images']]
        self.assertEqual(names, sorted(names))
        for index, record in enumerate(receipt['images']):
            image = (self.assets / record['name']).read_bytes()
            self.assertEqual(record, {'name': record['name'], 'bytes': len(image),
                                     'sha256': hashlib.sha256(image).hexdigest(), 'width': 4,
                                     'height': 2, 'id': f'authored-animation-image-{index}'})
        # Re-run the existing production Node sampler against precisely published bytes.
        expected = packer.validate_animations(data, {name: (self.assets / name).read_bytes() for name in names})
        self.assertEqual({k: receipt[k] for k in expected}, expected)
        return receipt

    def rejected_both(self, compose, **kw):
        for native in (False, True):
            out = self.root / f'refused-{native}'
            result = self.run_cli(native, compose, out, **kw)
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse(os.path.lexists(out), 'failed admission published output')

    def test_create_defaults_fps_pivot_and_float_semantics(self):
        cases = [[], ['--clip', 'idle', '12', 'loop'],
                 ['--clip', 'idle', '12.5', 'once', '--pivot', '0.5', '1'],
                 ['--clip', 'idle', '1e2', 'loop', '--pivot', '-0.0', '1e-5']]
        for index, extra in enumerate(cases):
            outputs = [self.root / f'create-{index}-{native}.json' for native in (False, True)]
            receipts = [self.success(native, False, path, extra=extra) for native, path in zip((False, True), outputs)]
            self.assertEqual(json.loads(outputs[0].read_bytes()), json.loads(outputs[1].read_bytes()))
            self.assertEqual(receipts[0]['images'], receipts[1]['images'])
            spec = json.loads(outputs[1].read_bytes())['unit:0']
            if not extra:
                self.assertNotIn('pivot', spec)
                self.assertEqual(set(spec['clips']['idle']), {'frames'})
            # Float serialization is not generally byte-identical to Python; each
            # receipt above is bound to its own exact bytes instead of a foreign hash.

    def test_compose_exact_ordinary_bytes_sorted_roles_and_source_order(self):
        outputs = [self.root / f'compose-{native}.json' for native in (False, True)]
        receipts = [self.success(native, True, path) for native, path in zip((False, True), outputs)]
        self.assertEqual(outputs[0].read_bytes(), outputs[1].read_bytes(), 'integer-only ordinary JSON serialization')
        self.assertEqual(receipts[0], receipts[1])
        self.assertEqual(receipts[1]['roles'], ['enemy:0', 'unit:0'])
        self.assertEqual(receipts[1]['sources'], [{'bytes': p.stat().st_size, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in [self.unit, self.enemy]])
        reverse = self.root / 'reverse.json'
        receipt = self.success(True, True, reverse, manifests=[self.enemy, self.unit])
        self.assertEqual(reverse.read_bytes(), outputs[1].read_bytes())
        self.assertEqual(receipt['sources'], list(reversed(receipts[1]['sources'])))

    def test_compose_strict_schema_duplicates_bounds_and_late_bad_image(self):
        for manifests in [[], [self.unit] * 12, [self.unit, self.unit]]:
            self.rejected_both(True, manifests=manifests)
        for raw in [b'{"unit:0":{},"unit:0":{}}', b'\xff', b'{', b' ' * (65536 + 1)]:
            self.rejected_both(True, manifests=[self.manifest('bad.json', raw=raw)])
        for value in [
            {'unit:6': self.definition},
            {'unit:0': {**self.definition, 'unexpected': True}},
            {'unit:0': {'image': 'art/hero.png', 'clips': {'idle': {'frames': [[3, 0, 2, 2]]}}}},
            {'unit:0': {'image': 'art/hero.png', 'clips': {'idle': {'frames': [[0, 0, 0, 2]]}}}},
            {'unit:0': {'image': 'art/hero.png', 'clips': {'idle': {'frames': [[0, 0, 1, 1]] * 257}}}},
            {'unit:0': {'image': 'art/hero.png', 'clips': {'idle': {'frames': [[0, 0, 1, 1]], 'fps': True}}}},
        ]:
            self.rejected_both(True, manifests=[self.manifest('bad.json', value)])
        for extra in [['--clip', 'idle', '0', 'loop'], ['--clip', 'idle', '121', 'loop'],
                      ['--clip', 'idle', 'NaN', 'loop'], ['--clip', 'idle', 'inf', 'loop'],
                      ['--clip', 'move', '12', 'loop'], ['--clip', 'idle', '12', 'bad'],
                      ['--clip', 'idle', '12', 'loop', '--clip', 'idle', '10', 'once'],
                      ['--pivot', '4097', '1'], ['--frame', 'jump', '0', '0', '1', '1']]:
            self.rejected_both(False, extra=extra)
        # All source manifests parse; a later sorted image fails before publication.
        (self.assets / 'art/hero.png').write_bytes(b'late invalid PNG')
        self.rejected_both(True)

    def test_png_crc_apng_and_complete_pixel_decoder_rejections(self):
        crc = bytearray(png())
        crc[29] ^= 1
        for data in [b'not PNG', png()[:-1], bytes(crc),
                     png(extra=chunk(b'acTL', struct.pack('>II', 1, 0))),
                     png(idat=b'not a zlib stream')]:
            with self.subTest(prefix=data[:16], length=len(data)):
                self.image.write_bytes(data)
                self.rejected_both(False)
                self.assertEqual(self.image.read_bytes(), data)

    def test_native_rejects_missing_scanline_that_legacy_pillow_zero_fills(self):
        # Exact observed migration difference: valid PNG chunks/CRC and a complete
        # zlib stream describe only ONE scanline for a 4x2 RGBA image. Pillow 13.0.0
        # loads the absent second row as transparent zeros; png17 fails closed.
        # Preserve legacy acceptance as evidence, not a native decoder exemption.
        data = png(idat=zlib.compress(b'\0' + b'\xff' * 16))
        self.assertEqual(hashlib.sha256(data).hexdigest(),
                         'e2b4088466554c0f061d0918c28fc11c3c5a40c79e861b7e72ec2ae548bb5786')
        self.image.write_bytes(data)
        old_output, new_output = self.root / 'legacy-padded.json', self.root / 'native-refused.json'
        old = self.run_cli(False, False, old_output)
        self.assertEqual(old.returncode, 0, old.stderr)
        self.assertTrue(old_output.is_file())
        self.assertEqual(json.loads(old.stdout)['images'][0]['sha256'], hashlib.sha256(data).hexdigest())
        new = self.run_cli(True, False, new_output)
        self.assertEqual(new.returncode, 1, new.stderr)
        self.assertIn('not have enough data', new.stderr)
        self.assertFalse(os.path.lexists(new_output))
        self.assertEqual(self.image.read_bytes(), data)

    def test_symlinks_fifo_paths_and_existing_outputs_preserved(self):
        for image in ('../hero.png', '/hero.png', 'art//hero.png', 'art/../hero.png', './hero.png', 'https://x/a.png'):
            bad = self.manifest('bad-path.json', {'unit:0': {**self.definition, 'image': image}})
            self.rejected_both(True, manifests=[bad])
        alias = self.root / 'manifest-link'
        alias.symlink_to(self.unit)
        self.rejected_both(True, manifests=[alias])
        fifo = self.root / 'manifest-fifo'
        os.mkfifo(fifo)
        self.rejected_both(True, manifests=[fifo])
        good = self.image.read_bytes()
        self.image.unlink()
        self.image.symlink_to(self.assets / 'art/enemy.png')
        self.rejected_both(False)
        self.image.unlink()
        self.image.write_bytes(good)
        for native in (False, True):
            for compose in (False, True):
                output = self.root / f'existing-{native}-{compose}'
                output.write_bytes(b'keep')
                self.assertNotEqual(self.run_cli(native, compose, output).returncode, 0)
                self.assertEqual(output.read_bytes(), b'keep')
                output.unlink()
                output.symlink_to('missing')
                self.assertNotEqual(self.run_cli(native, compose, output).returncode, 0)
                self.assertEqual(os.readlink(output), 'missing')

    def test_actual_clean_wasm_build_consumes_rust_composition_in_offline_html(self):
        web = self.root / 'actual-web'
        shutil.copytree(self.build, web)
        original_wasm = (web / 'game.wasm').read_bytes()
        (web / 'assets/art').mkdir(parents=True, exist_ok=True)
        for name in ('hero.png', 'enemy.png'):
            shutil.copyfile(self.assets / 'art' / name, web / 'assets/art' / name)
        output = self.root / 'authored-animations.json'
        self.success(True, True, output)
        html = self.root / 'game.html'
        packer.package(web, animations=output, output=html)
        self.assertEqual((web / 'game.wasm').read_bytes(), original_wasm)
        check = subprocess.run([sys.executable, '-B', str(TOOLS / 'check_coop_defense_offline.py'), str(web), str(html)],
                               capture_output=True, text=True, timeout=60,
                               env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
        self.assertEqual(check.returncode, 0, check.stderr)
        text = html.read_text()
        self.assertIn('authored-animation-manifest', text)
        self.assertNotIn(str(self.assets), text)
        self.assertNotIn(str(output), text)
        # Checker verifies exact embedded build/art/modules and invokes Node --check
        # plus the existing sampler, but does not certify browser rendering/gameplay.


if __name__ == '__main__':
    unittest.main()
