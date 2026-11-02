"""Real Rust/legacy CLI deployment differential; no described programs are launched.

Requires explicit ORR_RUST_PACKAGE_BIN. Python is internal test infrastructure.
New bounded/no-follow I/O limits are intentional improvements, not old-tool parity.
Provenance is release-owner supplied metadata, never a signature/build attestation.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import deployment_manifest as legacy

SCRIPT = Path(__file__).resolve().with_name('deployment_manifest.py')


class RustDeploymentManifest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        value = os.environ.get('ORR_RUST_PACKAGE_BIN')
        if not value or not Path(value).is_absolute() or not Path(value).is_file() or not os.access(value, os.X_OK):
            raise RuntimeError('ORR_RUST_PACKAGE_BIN must identify an explicit trusted executable')
        cls.binary = value

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.manifest = legacy.create_manifest('arena', 'release-1', '9007199254740993')

    def run_cli(self, native, *args):
        prefix = [self.binary, 'deployment'] if native else [sys.executable, '-B', str(SCRIPT)]
        return subprocess.run(prefix + list(map(str, args)), capture_output=True, timeout=20,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def ok(self, native, *args):
        result = self.run_cli(native, *args)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def create_args(self, output, raw='9007199254740993', identity='release-1', game='arena'):
        return ['create', '--game', game, '--game-code-identity', identity,
                '--game-code-id', raw, '--output', str(output)]

    def write_manifest(self, value=None, raw=None):
        path = self.root / 'input.json'
        path.write_bytes(raw if raw is not None else json.dumps(self.manifest if value is None else value).encode())
        return path

    def reject_both(self, path):
        before = path.read_bytes()
        for native in (False, True):
            for command in ('validate', 'export', 'id'):
                result = self.run_cli(native, command, path)
                self.assertEqual(result.returncode, 1, (native, command, result.stderr))
                self.assertEqual(result.stdout, b'')
        self.assertEqual(path.read_bytes(), before)

    def test_exact_create_export_id_ascii_json_and_precision_boundaries(self):
        for index, raw in enumerate(['1', '9007199254740993', str(legacy.MAX_U64), '0Xffffffffffffffff']):
            for game in ('arena', 'physics'):
                # Controls are internal, so identity remains trimmed. Non-BMP checks surrogate escaping.
                identity = '게임😀quote"slash\\tab\tline\nend'
                paths = [self.root / f'{index}-{game}-{native}.json' for native in (False, True)]
                for native, path in zip((False, True), paths):
                    self.ok(native, *self.create_args(path, raw, identity, game))
                self.assertEqual(paths[0].read_bytes(), paths[1].read_bytes())
                data = paths[1].read_bytes()
                data.decode('ascii')
                self.assertTrue(data.endswith(b'\n'))
                parsed = json.loads(data)
                number = int(raw, 16) if raw.lower().startswith('0x') else int(raw)
                self.assertEqual(parsed['game_code_id'], str(number))
                self.assertEqual(parsed['build_id'], str(legacy.frame_build_id(number)))
                self.assertIsNone(parsed['provenance']['source_revision'])
                for command in ('validate', 'export', 'id'):
                    expected = self.ok(False, command, paths[0], '--expect-game', game,
                                       '--expect-game-code-identity', identity)
                    self.assertEqual(self.ok(True, command, paths[1], '--expect-game', game,
                                             '--expect-game-code-identity', identity), expected)
                plan = json.loads(self.ok(True, 'export', paths[1]))
                self.assertIsInstance(plan['browser']['opts']['build_id'], str)
                self.assertEqual(plan['native_client']['args'], ['--build-id', parsed['build_id']])

    def test_binary_hash_order_and_previous_identity_guard(self):
        first, second = self.root / 'a.bin', self.root / 'b.bin'
        first.write_bytes(b'not executable\x00\xffA')
        second.write_bytes(b'other provenance bytes')
        previous = self.write_manifest()
        for order in ((first, second), (second, first)):
            outputs = [self.root / f'order-{order[0].name}-{n}.json' for n in (False, True)]
            for native, output in zip((False, True), outputs):
                args = self.create_args(output) + ['--source-revision', '소스😀\tend', '--previous', str(previous)]
                for path in order:
                    args += ['--binary', f' {path.name} ={path}']
                self.ok(native, *args)
            self.assertEqual(outputs[0].read_bytes(), outputs[1].read_bytes())
            records = json.loads(outputs[1].read_bytes())['provenance']['binaries']
            self.assertEqual([r['label'] for r in records], [p.name for p in order])
            self.assertEqual([r['sha256'] for r in records], [hashlib.sha256(p.read_bytes()).hexdigest() for p in order])
        for native in (False, True):
            for identity, game in [('changed', 'arena'), ('release-1', 'physics')]:
                output = self.root / 'refused.json'
                args = self.create_args(output, identity=identity, game=game) + ['--previous', str(previous)]
                self.assertEqual(self.run_cli(native, *args).returncode, 1)
                self.assertFalse(output.exists())
            output = self.root / f'new-id-{native}.json'
            self.ok(native, *(self.create_args(output, raw='2', identity='changed') + ['--previous', str(previous)]))

    def test_strict_shape_nullable_fields_duplicate_keys_and_canonical_ids(self):
        variants = []
        for key in self.manifest:
            v = copy.deepcopy(self.manifest)
            del v[key]
            variants.append(v)
        for key in ('source_revision', 'binaries'):
            v = copy.deepcopy(self.manifest)
            del v['provenance'][key]
            variants.append(v)
        for field, values in {
            'frame_format_version': [True, False, 2.0, 3, '2', None],
            'game_code_id': [1, True, None, '0', '01', '+1', '1 ', '0x1', str(1 << 64)],
            'build_id': [1, None, '0', '01', '42'],
            'format': [None, 'wrong'], 'game': ['phys', None],
            'game_code_identity': ['', ' leading', 'trailing ', None],
        }.items():
            for value in values:
                v = copy.deepcopy(self.manifest)
                v[field] = value
                variants.append(v)
        v = copy.deepcopy(self.manifest)
        v['unknown'] = 1
        variants.append(v)
        for records in [None, {}, [{'label': 'x', 'sha256': 'A' * 64}],
                        [{'label': 'x', 'sha256': 'a' * 64}] * 2,
                        [{'label': 'x', 'sha256': 'a' * 64, 'extra': 1}]]:
            v = copy.deepcopy(self.manifest)
            v['provenance']['binaries'] = records
            variants.append(v)
        for value in variants:
            with self.subTest(value=value):
                self.reject_both(self.write_manifest(value))
        valid = json.dumps(self.manifest)
        for raw in [b'\xff', b'{', ('{"format":"orr.deployment/1",' + valid[1:]).encode(),
                    valid.replace('"source_revision": null', '"source_revision": null, "source_revision": null').encode()]:
            self.reject_both(self.write_manifest(raw=raw))
        path = self.write_manifest()
        for native in (False, True):
            for args in [('--expect-game', 'physics'), ('--expect-game-code-identity', 'other')]:
                self.assertEqual(self.run_cli(native, 'validate', path, *args).returncode, 1)

    def test_create_bad_ids_duplicate_binary_and_existing_output_preserved(self):
        binary = self.root / 'binary'
        binary.write_bytes(b'supplied bytes')
        for native in (False, True):
            for raw in ('0', '0x0', '01', '+1', '-1', '1.0', str(1 << 64), '0x10000000000000000'):
                out = self.root / 'bad-id'
                self.assertNotEqual(self.run_cli(native, *self.create_args(out, raw=raw)).returncode, 0)
                self.assertFalse(out.exists())
            out = self.root / 'duplicate-binary'
            self.assertNotEqual(self.run_cli(native, *(self.create_args(out) + ['--binary', f'x={binary}', '--binary', f'x={binary}'])).returncode, 0)
            self.assertFalse(out.exists())
            out = self.root / f'foreign-{native}'
            out.write_bytes(b'keep')
            self.assertEqual(self.run_cli(native, *self.create_args(out)).returncode, 1)
            self.assertEqual(out.read_bytes(), b'keep')
            dangling = self.root / f'dangling-{native}'
            dangling.symlink_to('missing')
            self.assertEqual(self.run_cli(native, *self.create_args(dangling)).returncode, 1)
            self.assertEqual(os.readlink(dangling), 'missing')

    def test_new_native_nofollow_fifo_and_path_preservation(self):
        # Legacy follows symlinks; do not launch it on FIFOs or assert negative parity.
        path = self.write_manifest()
        link = self.root / 'link.json'
        link.symlink_to(path)
        fifo = self.root / 'fifo'
        os.mkfifo(fifo)
        for source in (link, fifo, self.root):
            self.assertNotEqual(self.run_cli(True, 'validate', source).returncode, 0)
            out = self.root / 'never-created'
            self.assertNotEqual(self.run_cli(True, *(self.create_args(out) + ['--binary', f'x={source}'])).returncode, 0)
            self.assertFalse(out.exists())
        directory = self.root / 'directory'
        directory.mkdir()
        alias = self.root / 'alias'
        alias.symlink_to(directory, target_is_directory=True)
        self.assertNotEqual(self.run_cli(True, *self.create_args(alias / 'output')).returncode, 0)
        self.assertEqual(list(directory.iterdir()), [])
        traversing = directory / '..' / 'output'
        self.assertNotEqual(self.run_cli(True, *self.create_args(traversing)).returncode, 0)
        self.assertFalse((self.root / 'output').exists())
        self.assertEqual(path.read_bytes(), json.dumps(self.manifest).encode())

    def test_new_native_resource_caps_without_reading_large_sparse_binary(self):
        path = self.write_manifest(raw=b' ' * (1024 * 1024 + 1))
        self.assertNotEqual(self.run_cli(True, 'validate', path).returncode, 0)
        binary = self.root / 'sparse'
        with binary.open('wb') as stream:
            stream.truncate(512 * 1024 * 1024 + 1)
        out = self.root / 'oversize-output'
        self.assertNotEqual(self.run_cli(True, *(self.create_args(out) + ['--binary', f'large={binary}'])).returncode, 0)
        self.assertFalse(out.exists())
        binary.write_bytes(b'tiny')
        args = self.create_args(out)
        for i in range(33):
            args += ['--binary', f'label-{i}={binary}']
        self.assertNotEqual(self.run_cli(True, *args).returncode, 0)
        self.assertFalse(out.exists())
        self.assertNotEqual(self.run_cli(True, *self.create_args(out, identity='x' * 4097)).returncode, 0)
        self.assertFalse(out.exists())
        # 64KiB argv, 4096-byte paths and 128 components are deliberate new bounds;
        # parser unit tests own extremes that could instead fail at the OS exec layer.


if __name__ == '__main__':
    unittest.main()
