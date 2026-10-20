#!/usr/bin/env python3
"""Pure Python hostile gate fixtures. No Rust compilation, GPU or subprocesses."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zlib

SPEC = importlib.util.spec_from_file_location('tilemap_gate', Path(__file__).with_name('tilemap_authoring_gate.py'))
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


def png_bytes(pixels, size=(2, 2), method=0):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    stride = size[0] * 4
    raw = bytearray()
    previous = bytes(stride)
    for y in range(size[1]):
        row = pixels[y * stride:(y + 1) * stride]
        raw.append(method)
        for x, value in enumerate(row):
            a, b, c = row[x - 4] if x >= 4 else 0, previous[x], previous[x - 4] if x >= 4 else 0
            if method == 1: predictor = a
            elif method == 2: predictor = b
            elif method == 3: predictor = (a + b) // 2
            elif method == 4:
                q = a + b - c
                d = (abs(q - a), abs(q - b), abs(q - c))
                predictor = (a, b, c)[d.index(min(d))]
            else: predictor = 0
            raw.append((value - predictor) & 255)
        previous = row
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', *size, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')


ACTUAL_INTERLEAVE = '\nrunning 1 test\ntest gpu_asymmetric_three_by_two_blank_shape_actor_and_wrong_region ... Failed to create /home/agent/.cache/mesa_shader_cache for shader cache (Read-only file system)---disabling.\nFailed to create /home/agent/.cache/mesa_shader_cache_db for shader cache (Read-only file system)---disabling.\nok\n\nsuccesses:\n\n---- gpu_asymmetric_three_by_two_blank_shape_actor_and_wrong_region stdout ----\ntile background GPU: SwiftShader Device (Subzero)\n\n\nsuccesses:\n    gpu_asymmetric_three_by_two_blank_shape_actor_and_wrong_region\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.30s\n\n'

class GateTests(unittest.TestCase):
    good = 'running 2 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.01s\n'

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='tilemap-gate-fixture-', dir=Path(__file__).resolve().parent.parent)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runtime, self.editor = self.root / 'runtime', self.root / 'editor'
        self.runtime.mkdir(); self.editor.mkdir()
        self.pixels = bytes(range(16))
        self.digest = hashlib.sha256(self.pixels).hexdigest()
        names = ['saved.png', 'fresh-reopen.png', 'actor-moving.png', 'tile-only-wrong-region.png']
        captures = [{'file': n, 'sha256': self.digest} for n in names]
        self.r = {'completed': True, 'captures': captures, 'native_os_input': False, 'export': False, 'windows': False}
        self.rp = {'completed': False, 'captures': copy.deepcopy(captures), 'adapter': 'llvmpipe (LLVM 15.0)', 'creator_sha256': 'a' * 64, 'source_sha256': 'b' * 64}
        self.e = {'completed': True, 'captures': ['dirty-paint.png', 'saved.png', 'fresh-reopen.png', 'play.png'], 'native_os_input': False, 'fresh_host_reopen': True}
        fingerprint = 0xcbf29ce484222325
        for b in self.pixels:
            fingerprint = ((fingerprint ^ b) * 0x100000001b3) & 0xffffffffffffffff
        self.ep = {'last_capture': 'play.png', 'size': [1800, 2000], 'adapter': 'llvmpipe (LLVM 15.0)', 'rgba_fnv1a64': f'{fingerprint:016x}'}
        for directory, names in [(self.runtime, names), (self.editor, self.e['captures'])]:
            for name in names:
                (directory / name).write_bytes(png_bytes(self.pixels))

    def check_receipts(self, loader=None):
        for path, obj in [(self.runtime / 'receipt.json', self.r), (self.runtime / 'partial-receipt.json', self.rp), (self.editor / 'editor-receipt.json', self.e), (self.editor / 'partial-editor-receipt.json', self.ep)]:
            path.write_text(json.dumps(obj))
        # Receipt fixtures use 2x2 images; real decoder/dimension validation tested separately.
        def small(path, _expected_size):
            return gate.capture(path, (2, 2))
        return gate.validate_receipts(self.runtime, self.editor, self.root, 'a' * 64, 'b' * 64, loader or small)

    def test_fill_census_and_explicit_fingerprint_contract(self):
        self.assertEqual(len(gate.FILL), 5)
        self.assertEqual(len(set(gate.FILL)), 5)
        self.assertTrue(all(name.startswith('tilemap_panel::fill') for name in gate.FILL))
        self.assertTrue(set(gate.FILL).isdisjoint(gate.PANEL))
        self.assertEqual(gate.CPU_COUNT, sum(map(len, [gate.CONSUMER, gate.PACKAGE,
            gate.CORE, gate.ADMISSION, gate.PANEL, gate.FILL, gate.RESIZE, gate.RECTANGLE])) + 2)
        self.assertEqual(gate.fnv1a64(b'hello'), 'a430d84680aabd0b')
        self.assertNotEqual(gate.fnv1a64(b'hello'), hashlib.sha256(b'hello').hexdigest())

    def test_resize_inventory_preserves_existing_cpu_thirty(self):
        self.assertEqual(len(gate.RESIZE), 5)
        self.assertEqual(len(set(gate.RESIZE)), 5)
        self.assertTrue(all(n.startswith('tilemap_panel::resize') for n in gate.RESIZE))
        self.assertTrue(set(gate.RESIZE).isdisjoint(gate.PANEL + gate.FILL))
        self.assertEqual(gate.CPU_COUNT - len(gate.RECTANGLE) - len(gate.RESIZE), 30)

    def test_rectangle_inventory_is_additive_exact_and_not_ignored(self):
        self.assertEqual(len(gate.RECTANGLE), 13)
        self.assertEqual(gate.CPU_COUNT - len(gate.RECTANGLE), 35)
        self.assertEqual(len(set(gate.RECTANGLE)), len(gate.RECTANGLE))
        self.assertTrue(all(n.startswith('tilemap_panel::rectangle::') for n in gate.RECTANGLE))
        self.assertTrue(set(gate.RECTANGLE).isdisjoint(gate._BASE_CPU))
        output = ''.join(n + ': test\n' for n in gate.RECTANGLE)
        gate.validate_inventory(output, gate.RECTANGLE)
        gate.validate_inventory('', [])
        for missing in (output.splitlines()[0] + '\n', ''):
            with self.assertRaises(RuntimeError): gate.validate_inventory(missing, gate.RECTANGLE)
        with self.assertRaises(RuntimeError): gate.validate_inventory(output, [])
        singleton = gate.RECTANGLE_EDITOR + ': test\n'
        gate.validate_inventory(singleton, [gate.RECTANGLE_EDITOR])
        with self.assertRaises(RuntimeError): gate.validate_inventory('', [gate.RECTANGLE_EDITOR])
        with self.assertRaises(RuntimeError): gate.validate_inventory(singleton * 2, [gate.RECTANGLE_EDITOR])
        # Exercise the exact named rectangle producer, not an anonymous pass count.
        good = ('running 1 test\ntest ' + gate.RECTANGLE_EDITOR + ' ... ok\n'
            'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n')
        gate.validate_cases(good, [gate.RECTANGLE_EDITOR])
        for bad in (good.replace(' ... ok', ' ... ignored'),
                    good.replace('1 passed', '0 passed'), good.replace(gate.RECTANGLE_EDITOR, 'other')):
            with self.assertRaises(RuntimeError): gate.validate_cases(bad, [gate.RECTANGLE_EDITOR])

    def test_frozen_producer_and_source_fences_fail_before_or_after_execution(self):
        import ast
        tree = ast.parse(Path(gate.__file__).read_text())
        main = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'main')
        fence = next(n for n in main.body if isinstance(n, ast.FunctionDef) and n.name == 'fenced_cases')
        code = compile(ast.Module(body=[fence], type_ignores=[]), '<actual gate fence>', 'exec')
        binary = self.root / 'producer'; binary.write_bytes(b'fresh-native-producer')
        manifest = self.root / 'manifest.json'
        manifest.write_text(json.dumps({'orr_editor': {'path': str(binary), 'sha256': gate.sha(binary)}}))
        sources = {'source': 'fresh'}
        ns = {'Path': Path, 'root': self.root, 'initial_sources': dict(sources),
            'source_fence': lambda _: dict(sources), 'manifest': manifest,
            'evidence_json': gate.evidence_json, 'sha': gate.sha}
        calls = []
        def cases(*args, **kwargs):
            calls.append(args)
            return 'passed'
        ns['cases'] = cases
        exec(code, ns)
        run = lambda: ns['fenced_cases']('rectangle', str(binary), gate.RECTANGLE_EDITOR, [gate.RECTANGLE_EDITOR])
        self.assertEqual(run(), 'passed')
        self.assertEqual(len(calls), 1)
        binary.write_bytes(b'stale producer')
        with self.assertRaisesRegex(RuntimeError, 'before execution'): run()
        self.assertEqual(len(calls), 1)
        binary.write_bytes(b'fresh-native-producer')
        sources['source'] = 'forged source'
        with self.assertRaisesRegex(RuntimeError, 'before test execution'): run()
        self.assertEqual(len(calls), 1)
        sources['source'] = 'fresh'
        def mutate_binary(*_args, **_kwargs):
            binary.write_bytes(b'changed during test')
            return 'passed'
        ns['cases'] = mutate_binary
        with self.assertRaisesRegex(RuntimeError, 'during execution'): run()
        binary.write_bytes(b'fresh-native-producer')
        def mutate_source(*_args, **_kwargs):
            sources['source'] = 'changed during test'
            return 'passed'
        ns['cases'] = mutate_source
        with self.assertRaisesRegex(RuntimeError, 'during test execution'): run()

    def test_scoped_source_fence_detects_changes_and_new_sources(self):
        root = self.root / 'source'; root.mkdir()
        for name in ('orr_editor', 'orr_tilemap', 'orr_sample', 'orr_package',
                     'orr_sprite', 'orr_sim', 'orr_session', 'orr_render', 'orr_rhi',
                     'orr_view', 'orr_bridge', 'orr_ecs', 'orr_fp'):
            folder = root / 'crates' / name; folder.mkdir(parents=True)
            (folder / 'Cargo.toml').write_text('manifest')
        for name in ('Cargo.toml', 'Cargo.lock', 'tools/tilemap_authoring_gate.py',
                     'tools/test_tilemap_authoring_gate.py',
                     'crates/orr_editor/src/app/tilemap_fill_tests.rs',
                     'crates/orr_editor/src/app/tilemap_resize_tests.rs',
                     'crates/orr_editor/src/app/tilemap_rectangle_tests.rs',
                     'crates/orr_editor/src/tilemap_panel/rectangle.rs',
                     'crates/orr_editor/src/tilemap_panel/resize.rs',
                     'crates/orr_editor/src/tilemap_panel/fill.rs'):
            path = root / name; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('source')
        before = gate.source_fence(root)
        (root / 'Cargo.lock').write_text('changed')
        self.assertNotEqual(before, gate.source_fence(root))
        (root / 'Cargo.lock').write_text('source')
        self.assertEqual(before, gate.source_fence(root))
        (root / 'crates/orr_editor/src/extra.rs').write_text('new source')
        self.assertNotEqual(before, gate.source_fence(root))

    def test_fill_json_and_file_integrity_fail_closed(self):
        path = self.root / 'fill.json'
        path.write_text('{"complete": true, "complete": false}')
        with self.assertRaisesRegex(RuntimeError, 'Duplicate'):
            gate.evidence_json(path)
        path.write_bytes(b'')
        with self.assertRaises(RuntimeError): gate.evidence_bytes(path)
        path.unlink()
        with self.assertRaises(RuntimeError): gate.evidence_bytes(path)
        path.symlink_to(self.runtime / 'saved.png')
        with self.assertRaises(RuntimeError): gate.evidence_bytes(path)


    def test_native_stderr_interleave_actual_static_gpu_log(self):
        text = ACTUAL_INTERLEAVE
        name = 'gpu_asymmetric_three_by_two_blank_shape_actor_and_wrong_region'
        gate.validate_inventory(name + ': test\n', [name])
        gate.validate_cases(text, [name])
        for bad in [text.replace('\nok\n', '\nFAILED\n'),
                    text.replace('\nok\n', '\nignored\n'),
                    text.replace('\nok\n', '\n'),
                    text.replace('\nok\n', '\nok\nok\n'),
                    text.replace('1 passed', '0 passed'),
                    text.replace('test ' + name, 'test wrong_name'),
                    text.replace('    ' + name, '    wrong_name'),
                    text.replace('\nok\n', '\nwarning only\n'),
                    text + 'SKIP GPU unavailable\n']:
            with self.subTest(text=bad), self.assertRaises(RuntimeError):
                gate.validate_cases(bad, [name])

    def test_valid_counts_and_crlf(self):
        gate.validate_cases(self.good, ['a', 'b'])
        gate.validate_cases(self.good.replace('\n', '\r\n'), ['a', 'b'])

    def test_bad_count_summary_duplicate_and_missing_case_rejected(self):
        for text in [self.good.replace('2 passed', '1 passed'), self.good.replace('running 2', 'running 1'), self.good.replace('test b ... ok\n', ''), self.good.replace('test b', 'test a'), self.good + self.good]:
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                gate.validate_cases(text, ['a', 'b'])

    def test_ignored_failed_skipped_and_empty_rejected(self):
        for text in [self.good.replace('0 ignored', '1 ignored'), self.good.replace('test b ... ok', 'test b ... ignored'), self.good.replace('test b ... ok', 'test b ... FAILED'), self.good + 'SKIP GPU unavailable\n', self.good + 'skipping GPU\n', '']:
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                gate.validate_cases(text, ['a', 'b'])
        with self.assertRaises(RuntimeError):
            gate.validate_cases('running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n', [])

    def test_inventory_exact_and_ignored_drift(self):
        gate.validate_inventory('a: test\nb: test\n', ['b', 'a'])
        gate.validate_inventory('', [])
        for output, expected in [('a: test\n', ['a', 'b']), ('a: test\na: test\n', ['a']), ('a: test\n', []), ('', ['gpu'])]:
            with self.subTest(output=output), self.assertRaises(RuntimeError):
                gate.validate_inventory(output, expected)

    def test_png_all_filters_round_trip(self):
        path = self.root / 'capture.png'
        for method in range(5):
            with self.subTest(method=method):
                data = png_bytes(self.pixels, method=method)
                path.write_bytes(data)
                digest, pixels = gate.capture(path, (2, 2))
                self.assertEqual(pixels, self.pixels)
                self.assertEqual(digest, hashlib.sha256(data).hexdigest())

    def test_png_corruption_truncation_size_and_payload_rejected(self):
        path = self.root / 'capture.png'
        data = png_bytes(self.pixels)
        for bad in [data[:-1], data[:40] + bytes([data[40] ^ 1]) + data[41:], b'not png', data + b'junk', png_bytes(self.pixels[:-1])]:
            with self.subTest(length=len(bad)), self.assertRaises(RuntimeError):
                path.write_bytes(bad); gate.capture(path, (2, 2))
        path.write_bytes(data)
        with self.assertRaises(RuntimeError):
            gate.capture(path, (3, 2))

    def test_valid_receipts_and_all_eight_capture_hashes(self):
        result = self.check_receipts()
        self.assertEqual(len(result), 8)
        self.assertTrue(all(len(value) == 64 for value in result.values()))

    def test_incomplete_final_receipt_rejected(self):
        self.r['completed'] = False
        with self.assertRaises(RuntimeError): self.check_receipts()
        self.r['completed'] = True
        self.e['completed'] = False
        with self.assertRaises(RuntimeError): self.check_receipts()

    def test_wrong_adapter_rejected_on_each_route(self):
        for receipt in [self.rp, self.ep]:
            old = receipt['adapter']
            for adapter in ['SwiftShader Device (Subzero)', 'NVIDIA Vulkan', '']:
                receipt['adapter'] = adapter
                with self.subTest(adapter=adapter), self.assertRaises(RuntimeError): self.check_receipts()
            receipt['adapter'] = old

    def test_missing_duplicate_and_reordered_capture_receipt_rejected(self):
        original = copy.deepcopy(self.r['captures'])
        for captures in [original[:-1], original[:1] * 4, list(reversed(original))]:
            self.r['captures'] = captures
            self.rp['captures'] = copy.deepcopy(captures)
            with self.subTest(captures=captures), self.assertRaises(RuntimeError): self.check_receipts()

    def test_source_and_creator_hash_mismatch_rejected(self):
        for key in ['source_sha256', 'creator_sha256']:
            old = self.rp[key]; self.rp[key] = 'c' * 64
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check_receipts()
            self.rp[key] = old

    def test_capture_hash_mismatch_rejected_even_matching_partial(self):
        self.r['captures'][0]['sha256'] = 'c' * 64
        self.rp['captures'][0]['sha256'] = 'c' * 64
        with self.assertRaisesRegex(RuntimeError, 'raw-pixel hash'): self.check_receipts()

    def test_editor_pixel_fingerprint_mismatch_rejected(self):
        self.ep['rgba_fnv1a64'] = '0' * 16
        with self.assertRaisesRegex(RuntimeError, 'fingerprint'): self.check_receipts()

    def test_missing_capture_rejected(self):
        (self.editor / 'play.png').unlink()
        with self.assertRaises(FileNotFoundError): self.check_receipts()

    def test_reopen_pixels_mismatch_rejected_even_consistent_hash(self):
        different = bytes(reversed(self.pixels))
        (self.runtime / 'fresh-reopen.png').write_bytes(png_bytes(different))
        digest = hashlib.sha256(different).hexdigest()
        self.r['captures'][1]['sha256'] = digest; self.rp['captures'][1]['sha256'] = digest
        with self.assertRaisesRegex(RuntimeError, 'reopen capture differs'): self.check_receipts()

    def test_overclaim_or_bad_editor_size_rejected(self):
        self.r['windows'] = True
        with self.assertRaises(RuntimeError): self.check_receipts()
        self.r['windows'] = False; self.ep['size'] = [1800, 1200]
        with self.assertRaises(RuntimeError): self.check_receipts()


class FillReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='fill-gate-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fill = self.root / 'fill'; self.fill.mkdir()
        self.identities = {'creator_fnv1a64': b'creator', 'test_source_fnv1a64': b'source'}
        self.rows = []
        for index, name in enumerate(gate.FILL_PHASES):
            size = [1, 1] if index < 8 else [224, 224]
            rgba = bytearray(size[0] * size[1] * 4)
            if index == 9:
                for x in range(72, 80): rgba[(64 * 224 + x) * 4] = 255
            frame = b'actual-frame-zero' if index < 7 else b'actual-frame-one'
            row = {'name': name, 'size': size, 'files': [name + x for x in
                (['.rgba', '.png', '.frame', '-app.png'] if index < 8 else ['.rgba', '.png', '.frame'])],
                'rgba_fnv1a64': gate.fnv1a64(rgba), 'frame_fnv1a64': gate.fnv1a64(frame),
                'tick': 0 if index < 7 else 1, 'checksum': 100 if index < 7 else 101}
            (self.fill / (name + '.rgba')).write_bytes(rgba)
            (self.fill / (name + '.png')).write_bytes(png_bytes(rgba, tuple(size)))
            (self.fill / (name + '.frame')).write_bytes(frame)
            if index < 8:
                row.update(scope='actual App viewport; authoring grid, no pixel verdict', adapter='llvmpipe (LLVM)', backend='Vulkan', device_type='Cpu',
                    frame_source='watch.subscribe source=view matched to predicted tick/checksum',
                    cells=copy.deepcopy(gate.FILL_BEFORE if index in (0, 1, 3) else gate.FILL_CELLS))
                (self.fill / (name + '-app.png')).write_bytes(b'app-png-fixture')
            else:
                row.update(scope='actual saved PreparedRuntime PlayHost production compositor', pixel_verdict_pending=False, oracle_pixels_checked=10752)
            self.rows.append(row)
        self.receipt = {'completed': True, 'captures': self.rows, 'literal_cells': gate.FILL_CELLS,
            'oracle_pixels_per_runtime_capture': 10752, 'wrong_region_visible_pixels': 8,
            'native_os_input': False, 'export': False, 'windows': False}
        self.receipt.update({k: gate.fnv1a64(v) for k, v in self.identities.items()})
        for filename, cells in [('saved.tilemap.json', gate.FILL_CELLS),
                ('wrong-region.tilemap.json', [*gate.FILL_CELLS[:8], 10, *gate.FILL_CELLS[9:]])]:
            (self.fill / filename).write_text(json.dumps({'width': 7, 'height': 6,
                'origin': [-56., 144.], 'cells': cells}))
        (self.fill / 'creator.log').write_bytes(b'')

    def write(self):
        (self.fill / 'receipt.json').write_text(json.dumps(self.receipt))
        partial = copy.deepcopy(self.rows)
        partial[-1]['pixel_verdict_pending'] = True
        partial[-1].pop('oracle_pixels_checked', None)
        (self.fill / 'partial-receipt.json').write_text(json.dumps(
            {'completed': False, 'captures': partial, 'native_os_input': False}))

    def check(self, adapter='llvmpipe'):
        self.write()
        def loader(path, size):
            # Whole-App PNG decoder is already tested above; avoid a 14MB fixture.
            if path.name.endswith('-app.png'):
                self.assertEqual(size, (1800, 2000)); return 'fixture', b''
            return gate.capture(path, size)
        return gate.validate_fill(self.fill, self.root, self.identities, loader, adapter)

    def test_all_retained_artifacts_have_external_sha256(self):
        hashes = self.check()
        self.assertEqual(len(hashes), 43)
        for name, digest in hashes.items():
            self.assertEqual(digest, hashlib.sha256((self.root / name).read_bytes()).hexdigest())

    def test_incomplete_census_sources_and_unsupported_scope_rejected(self):
        for field, bad in [('completed', False), ('windows', True), ('creator_fnv1a64', '0'*16),
                           ('literal_cells', [None]*42), ('wrong_region_visible_pixels', 0)]:
            old = self.receipt[field]; self.receipt[field] = bad
            with self.subTest(field=field), self.assertRaises(RuntimeError): self.check()
            self.receipt[field] = old
        old = self.rows[-1]['name']; self.rows[-1]['name'] = 'runtime-saved'
        with self.assertRaises(RuntimeError): self.check()
        self.rows[-1]['name'] = old
        (self.fill / 'unexpected').write_bytes(b'x')
        with self.assertRaisesRegex(RuntimeError, 'census'): self.check()

    def test_local_adapter_never_substitutes_hosted_acceptance(self):
        for row in self.rows[:8]: row['adapter'] = 'SwiftShader Device (Subzero)'
        with self.assertRaisesRegex(RuntimeError, 'llvmpipe'): self.check()
        self.assertEqual(len(self.check(adapter='SwiftShader')), 43)

    def test_frame_parity_png_and_visible_pixel_tamper_rejected(self):
        path = self.fill / 'redo.frame'; old = path.read_bytes()
        path.write_bytes(b'changed-frame')
        self.rows[4]['frame_fnv1a64'] = gate.fnv1a64(path.read_bytes())
        with self.assertRaisesRegex(RuntimeError, 'parity'): self.check()
        path.write_bytes(old); self.rows[4]['frame_fnv1a64'] = gate.fnv1a64(old)
        path = self.fill / 'saved.png'; old = path.read_bytes()
        path.write_bytes(png_bytes(bytes([1, 2, 3, 4]), (1, 1)))
        with self.assertRaisesRegex(RuntimeError, 'PNG/raw'): self.check()
        path.write_bytes(old)
        self.receipt['wrong_region_visible_pixels'] = 9
        with self.assertRaisesRegex(RuntimeError, 'pixel verdict'): self.check()

    def test_missing_evidence_and_pending_runtime_verdict_rejected(self):
        self.rows[-1]['pixel_verdict_pending'] = True
        with self.assertRaisesRegex(RuntimeError, 'oracle incomplete'): self.check()
        self.rows[-1]['pixel_verdict_pending'] = False
        (self.fill / 'runtime-wrong-region.frame').unlink()
        with self.assertRaises(RuntimeError): self.check()


class ResizeReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='resize-gate-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.resize = self.root / 'resize'; self.resize.mkdir()
        self.identities = {k: k.encode() for k in gate.RESIZE_IDENTITIES}
        self.rows = []
        for name in gate.RESIZE_PHASES:
            app = name in gate.RESIZE_APP_PHASES
            tick = int(name in ('play', 'runtime-saved', 'runtime-wrong-stride'))
            size = [1, 1] if app else [224, 224]
            rgba = bytearray(size[0] * size[1] * 4)
            if name == 'runtime-wrong-stride':
                for x in range(104, 112): rgba[(48 * 224 + x) * 4] = 255
            frame = b'actual-frame-one' if tick else b'actual-frame-zero'
            row = {'name': name, 'size': size, 'files': [name + x for x in
                (['.rgba', '.png', '.frame', '-app.png'] if app else ['.rgba', '.png', '.frame'])],
                'rgba_fnv1a64': gate.fnv1a64(rgba), 'frame_fnv1a64': gate.fnv1a64(frame),
                'tick': tick, 'checksum': 100 + tick}
            (self.resize / (name + '.rgba')).write_bytes(rgba)
            (self.resize / (name + '.png')).write_bytes(png_bytes(rgba, tuple(size)))
            (self.resize / (name + '.frame')).write_bytes(frame)
            if app:
                if name in ('before', 'resize-undo', 'noop-preserves-redo'):
                    literal, dims = gate.RESIZE_BEFORE, [3, 2]
                elif name in ('paint-new-cell', 'paint-redo', 'occupied-crop-rejected'):
                    literal, dims = gate.RESIZE_PAINTED, [5, 3]
                elif name in ('empty-trim', 'saved', 'fresh-reopen', 'play'):
                    literal, dims = gate.RESIZE_CELLS, [4, 2]
                else:
                    literal, dims = gate.RESIZE_GROWN, [5, 3]
                row.update(scope='actual App viewport; authoring grid, no pixel verdict',
                    adapter='llvmpipe (LLVM)', backend='Vulkan', device_type='Cpu',
                    frame_source='watch.subscribe source=view matched to predicted tick/checksum',
                    cells=copy.deepcopy(literal), dimensions=dims)
                (self.resize / (name + '-app.png')).write_bytes(b'app-png-fixture')
            else:
                row.update(scope='actual saved PreparedRuntime PlayHost production compositor',
                    pixel_verdict_pending=False, oracle_pixels_checked=3840)
            self.rows.append(row)
        (self.resize / 'presentation-free.frame').write_bytes(b'actual-frame-one')
        self.receipt = {'completed': True, 'captures': self.rows,
            'literal_before': gate.RESIZE_BEFORE, 'literal_grown': gate.RESIZE_GROWN,
            'literal_painted': gate.RESIZE_PAINTED, 'literal_cells': gate.RESIZE_CELLS,
            'literal_wrong_stride': gate.RESIZE_WRONG,
            'oracle_pixels_per_runtime_capture': 3840, 'wrong_stride_visible_pixels': 8,
            'frame_parity': gate.RESIZE_FRAME_SCOPE,
            'presentation_free_frame_fnv1a64': gate.fnv1a64(b'actual-frame-one'),
            'fixture_file_bindings': [{'path': n, 'bytes': 1, 'fnv1a64': gate.fnv1a64(b'x')}
                for n in ['arena.tilemap.json', 'orr.project.json']],
            'native_os_input': False, 'export': False, 'windows': False}
        self.receipt.update({k: gate.fnv1a64(v) for k, v in self.identities.items()})
        for filename, literal in [('saved.tilemap.json', gate.RESIZE_CELLS),
                ('wrong-stride.tilemap.json', gate.RESIZE_WRONG)]:
            (self.resize / filename).write_text(json.dumps({'version': 1,
                'sprite': {'package': 'sample-sprites', 'document': 'sprites.json'},
                'width': 4, 'height': 2, 'tile_size': [16, 16], 'origin': [-56., 144.],
                'units_per_pixel': 1., 'cells': literal}))
        (self.resize / 'creator.log').write_bytes(b'')

    def write(self):
        (self.resize / 'receipt.json').write_text(json.dumps(self.receipt))
        partial = copy.deepcopy(self.rows)
        partial[-1]['pixel_verdict_pending'] = True
        partial[-1].pop('oracle_pixels_checked', None)
        (self.resize / 'partial-receipt.json').write_text(json.dumps(
            {'completed': False, 'captures': partial, 'native_os_input': False}))

    def check(self, adapter='llvmpipe', rewrite=True):
        if rewrite: self.write()
        def loader(path, size):
            if path.name.endswith('-app.png'):
                self.assertEqual(size, (1800, 2000)); return 'fixture', b''
            return gate.capture(path, size)
        return gate.validate_resize(self.resize, self.root, self.identities, loader, adapter)

    def test_exact_sixty_eight_retained_hashes(self):
        hashes = self.check()
        self.assertEqual(len(hashes), 68)
        self.assertEqual(set(hashes), {'resize/' + p.name for p in self.resize.iterdir()})
        for name, digest in hashes.items():
            self.assertEqual(digest, hashlib.sha256((self.root / name).read_bytes()).hexdigest())

    def test_every_receipt_and_row_key_is_required_and_extras_rejected(self):
        for item in [self.receipt, self.rows[0], self.rows[-1]]:
            for key in list(item):
                old = item.pop(key)
                with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
                item[key] = old
            item['surprise'] = True
            with self.assertRaises(RuntimeError): self.check()
            del item['surprise']

    def test_literals_dimensions_crop_noop_and_provenance_fail_closed(self):
        for row in self.rows[:14]:
            old = row['dimensions']; row['dimensions'] = [5, 99]
            with self.subTest(name=row['name']), self.assertRaisesRegex(RuntimeError, 'literal grid'): self.check()
            row['dimensions'] = old
            old = row['cells']; row['cells'] = old[::-1]
            with self.subTest(name=row['name']), self.assertRaises(RuntimeError): self.check()
            row['cells'] = old
        old = self.rows[0]['cells']; self.rows[0]['cells'] = [10, None, 20, 11, 20, False]
        with self.assertRaises(RuntimeError): self.check()
        self.rows[0]['cells'] = old
        for key in gate.RESIZE_IDENTITIES:
            old = self.receipt[key]; self.receipt[key] = '0' * 16
            with self.subTest(key=key), self.assertRaisesRegex(RuntimeError, 'fingerprint'): self.check()
            self.receipt[key] = old
        self.identities.pop('creator_fnv1a64')
        with self.assertRaisesRegex(RuntimeError, 'identity census'): self.check()

    def test_scope_runtime_pending_census_and_types_fail_closed(self):
        for key, value in [('completed', False), ('windows', True), ('export', True),
                           ('native_os_input', True), ('oracle_pixels_per_runtime_capture', 0),
                           ('wrong_stride_visible_pixels', True), ('frame_parity', 'predicted')]:
            old = self.receipt[key]; self.receipt[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
            self.receipt[key] = old
        for key, value in [('tick', True), ('checksum', False), ('pixel_verdict_pending', True),
                           ('oracle_pixels_checked', 0), ('size', [224, 223])]:
            old = self.rows[-1][key]; self.rows[-1][key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
            self.rows[-1][key] = old
        self.rows.reverse()
        with self.assertRaisesRegex(RuntimeError, 'census'): self.check()
        self.rows.reverse()
        (self.resize / 'unexpected').write_bytes(b'x')
        with self.assertRaisesRegex(RuntimeError, 'census'): self.check()

    def test_saved_metadata_and_strict_json_rejected(self):
        path = self.resize / 'saved.tilemap.json'
        original = path.read_text(); doc = json.loads(original)
        for key, value in [('width', 5), ('height', True), ('origin', [0, 0]), ('tile_size', [True, 16]),
                ('version', True), ('units_per_pixel', True), ('sprite', {'package': 'other', 'document': 'sprites.json'}),
                ('cells', gate.RESIZE_WRONG), ('extra', 'forbidden')]:
            altered = dict(doc); altered[key] = value; path.write_text(json.dumps(altered))
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
        path.write_text(original[:-1] + ',"version":1}')
        with self.assertRaisesRegex(RuntimeError, 'Duplicate'): self.check()

    def test_partial_and_fixture_binding_manifest_rejected(self):
        self.write()
        path = self.resize / 'partial-receipt.json'
        partial = json.loads(path.read_text())
        for bad in (True, 0):
            partial['completed'] = bad
            path.write_text(json.dumps(partial))
            with self.assertRaisesRegex(RuntimeError, 'partial'): self.check(rewrite=False)
        original = copy.deepcopy(self.receipt['fixture_file_bindings'])
        for bad in [[], original + original[:1], list(reversed(original)),
                    [{'path': '../escape', 'bytes': 1, 'fnv1a64': '0'*16}],
                    [{'path': 'arena.tilemap.json', 'bytes': True, 'fnv1a64': '0'*16}]]:
            self.receipt['fixture_file_bindings'] = bad
            with self.subTest(bad=bad), self.assertRaises(RuntimeError): self.check()

    def test_frame_png_and_visible_wrong_stride_tampering_rejected(self):
        path = self.resize / 'grown.frame'; old = path.read_bytes()
        path.write_bytes(b'changed-frame'); self.rows[1]['frame_fnv1a64'] = gate.fnv1a64(path.read_bytes())
        with self.assertRaisesRegex(RuntimeError, 'parity'): self.check()
        path.write_bytes(old); self.rows[1]['frame_fnv1a64'] = gate.fnv1a64(old)
        path = self.resize / 'saved.png'; old = path.read_bytes()
        path.write_bytes(png_bytes(bytes([1, 2, 3, 4]), (1, 1)))
        with self.assertRaisesRegex(RuntimeError, 'PNG/raw'): self.check()
        path.write_bytes(old)
        self.receipt['wrong_stride_visible_pixels'] = 9
        with self.assertRaisesRegex(RuntimeError, 'pixel verdict'): self.check()
        self.receipt['wrong_stride_visible_pixels'] = 8
        (self.resize / 'presentation-free.frame').write_bytes(b'not-authoritative')
        self.receipt['presentation_free_frame_fnv1a64'] = gate.fnv1a64(b'not-authoritative')
        with self.assertRaisesRegex(RuntimeError, 'presentation-free'): self.check()

    def test_regular_files_required(self):
        path = self.resize / 'before-app.png'; old = path.read_bytes(); path.unlink()
        with self.assertRaises(RuntimeError): self.check()
        target = self.root / 'elsewhere'; target.write_bytes(old); path.symlink_to(target)
        with self.assertRaises(RuntimeError): self.check()

    def test_local_adapter_audit_never_replaces_hosted_default(self):
        for row in self.rows[:14]: row['adapter'] = 'SwiftShader Device (Subzero)'
        with self.assertRaisesRegex(RuntimeError, 'llvmpipe'): self.check()
        self.assertEqual(len(self.check(adapter='SwiftShader')), 68)
        with self.assertRaisesRegex(RuntimeError, 'empty adapter'): self.check(adapter='')
        # Main has no argument/environment escape hatch for hosted acceptance.
        import ast
        tree = ast.parse(Path(gate.__file__).read_text())
        main = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'main')
        calls = [n for n in ast.walk(main) if isinstance(n, ast.Call) and isinstance(n.func, ast.Name)
                 and n.func.id == 'validate_resize']
        self.assertEqual(len(calls), 1)
        self.assertEqual(len(calls[0].args), 3)
        self.assertEqual(calls[0].keywords, [])


class RectangleReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rectangle-gate-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.rectangle = self.root / 'rectangle'; self.rectangle.mkdir()
        self.identities = {k: k.encode() for k in gate.RECTANGLE_IDENTITIES}
        self.rows = []
        for name in gate.RECTANGLE_PHASES:
            app = name in gate.RECTANGLE_APP_PHASES
            tick = int(name in gate.RECTANGLE_PLAY_PHASES)
            size = [1, 1] if app else [224, 224]
            rgba = bytearray(size[0] * size[1] * 4)
            if name == 'runtime-wrong-stride':
                for x in range(136, 144): rgba[(48 * 224 + x) * 4] = 255
            if name == 'runtime-extra-cell':
                for x in range(152, 160): rgba[(96 * 224 + x) * 4] = 255
            frame = b'actual-frame-one' if tick else b'actual-frame-zero'
            row = {'name': name, 'size': size, 'files': [name + x for x in
                (['.rgba', '.png', '.frame', '-app.png'] if app else ['.rgba', '.png', '.frame'])],
                'rgba_fnv1a64': gate.fnv1a64(rgba), 'frame_fnv1a64': gate.fnv1a64(frame),
                'tick': tick, 'checksum': 100 + tick}
            (self.rectangle / (name + '.rgba')).write_bytes(rgba)
            (self.rectangle / (name + '.png')).write_bytes(png_bytes(rgba, tuple(size)))
            (self.rectangle / (name + '.frame')).write_bytes(frame)
            if app:
                literal, dims = gate.rectangle_literal(name), [7, 4]
                undo, redo, dirty = gate.rectangle_history(name)
                row['history'] = {'undo': undo, 'redo': redo, 'dirty': dirty, 'saved_fnv1a64': '1' * 16}
                row.update(scope='actual App viewport; authoring grid, no pixel verdict',
                    adapter='llvmpipe (LLVM)', backend='Vulkan', device_type='Cpu',
                    frame_source='watch.subscribe source=view matched to predicted tick/checksum',
                    cells=copy.deepcopy(literal), dimensions=dims)
                (self.rectangle / (name + '-app.png')).write_bytes(b'app-png-fixture')
            else:
                row.update(scope='actual saved PreparedRuntime PlayHost production compositor',
                    pixel_verdict_pending=False, oracle_pixels_checked=7168)
            self.rows.append(row)
        (self.rectangle / 'presentation-free.frame').write_bytes(b'actual-frame-one')
        self.receipt = {'completed': True, 'captures': self.rows,
            'literal_before': gate.RECTANGLE_BEFORE, 'literal_extra_cell': gate.RECTANGLE_EXTRA,
            'literal_painted': gate.RECTANGLE_PAINTED, 'literal_cells': gate.RECTANGLE_CELLS,
            'literal_wrong_stride': gate.RECTANGLE_WRONG,
            'oracle_pixels_per_runtime_capture': 7168, 'wrong_stride_visible_pixels': 8,
            'extra_cell_visible_pixels': 8, 'preview_visible_pixels': 8, 'preview_pixel_rect': [0, 0, 8, 1],
            'frame_parity': gate.RECTANGLE_FRAME_SCOPE,
            'presentation_free_frame_fnv1a64': gate.fnv1a64(b'actual-frame-one'),
            'fixture_file_bindings': [{'path': n, 'bytes': 1, 'fnv1a64': gate.fnv1a64(b'x')}
                for n in ['arena.tilemap.json', 'orr.project.json']],
            'native_os_input': False, 'export': False, 'windows': False}
        self.receipt.update({k: gate.fnv1a64(v) for k, v in self.identities.items()})
        for filename, literal in [('saved.tilemap.json', gate.RECTANGLE_CELLS),
                ('wrong-stride.tilemap.json', gate.RECTANGLE_WRONG), ('extra-cell.tilemap.json', gate.RECTANGLE_EXTRA)]:
            (self.rectangle / filename).write_text(json.dumps({'version': 1,
                'sprite': {'package': 'sample-sprites', 'document': 'sprites.json'},
                'width': 7, 'height': 4, 'tile_size': [16, 16], 'origin': [-56., 144.],
                'units_per_pixel': 1., 'cells': literal}))
        (self.rectangle / 'creator.log').write_bytes(b'')
        (self.rectangle / 'preview-baseline-app.png').write_bytes(b'baseline-app-png-fixture')
        final_hash = gate.fnv1a64((self.rectangle / 'saved.tilemap.json').read_bytes())
        for row in self.rows:
            if row['name'] in ('saved', 'saved-noop', 'saved-undo-noop', 'saved-redo', 'fresh-reopen', 'play'):
                row['history']['saved_fnv1a64'] = final_hash
        self.app_pixels = bytes(1800 * 2000 * 4)
        self.preview_pixels = bytes([255, 0, 0, 0]) * 8 + self.app_pixels[32:]

    def write(self):
        (self.rectangle / 'receipt.json').write_text(json.dumps(self.receipt))
        partial = copy.deepcopy(self.rows)
        partial[-1]['pixel_verdict_pending'] = True
        partial[-1].pop('oracle_pixels_checked', None)
        (self.rectangle / 'partial-receipt.json').write_text(json.dumps(
            {'completed': False, 'captures': partial, 'native_os_input': False}))

    def check(self, adapter='llvmpipe', rewrite=True):
        if rewrite: self.write()
        def loader(path, size):
            if path.name.endswith('-app.png'):
                self.assertEqual(size, (1800, 2000))
                return 'fixture', self.preview_pixels if path.name == 'preview-app.png' else self.app_pixels
            return gate.capture(path, size)
        return gate.validate_rectangle(self.rectangle, self.root, self.identities, loader, adapter)

    def test_exact_one_hundred_thirty_seven_retained_hashes(self):
        hashes = self.check()
        self.assertEqual(len(hashes), 137)
        self.assertEqual(set(hashes), {'rectangle/' + p.name for p in self.rectangle.iterdir()})
        for name, digest in hashes.items():
            self.assertEqual(digest, hashlib.sha256((self.root / name).read_bytes()).hexdigest())

    def test_every_receipt_and_row_key_is_required_and_extras_rejected(self):
        for item in [self.receipt, self.rows[0], self.rows[-1]]:
            for key in list(item):
                old = item.pop(key)
                with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
                item[key] = old
            item['surprise'] = True
            with self.assertRaises(RuntimeError): self.check()
            del item['surprise']

    def test_literals_dimensions_crop_noop_and_provenance_fail_closed(self):
        for row in self.rows[:len(gate.RECTANGLE_APP_PHASES)]:
            old = row['dimensions']; row['dimensions'] = [5, 99]
            with self.subTest(name=row['name']), self.assertRaisesRegex(RuntimeError, 'literal grid'): self.check()
            row['dimensions'] = old
            old = row['cells']; row['cells'] = old[::-1]
            with self.subTest(name=row['name']), self.assertRaises(RuntimeError): self.check()
            row['cells'] = old
        old = self.rows[0]['cells']; self.rows[0]['cells'] = [10, None, 20, 11, 20, False]
        with self.assertRaises(RuntimeError): self.check()
        self.rows[0]['cells'] = old
        for key in gate.RECTANGLE_IDENTITIES:
            old = self.receipt[key]; self.receipt[key] = '0' * 16
            with self.subTest(key=key), self.assertRaisesRegex(RuntimeError, 'fingerprint'): self.check()
            self.receipt[key] = old
        self.identities.pop('creator_fnv1a64')
        with self.assertRaisesRegex(RuntimeError, 'identity census'): self.check()

    def test_scope_runtime_pending_census_and_types_fail_closed(self):
        for key, value in [('completed', False), ('windows', True), ('export', True),
                           ('native_os_input', True), ('oracle_pixels_per_runtime_capture', 0),
                           ('wrong_stride_visible_pixels', True), ('extra_cell_visible_pixels', True),
                           ('preview_visible_pixels', True), ('frame_parity', 'predicted')]:
            old = self.receipt[key]; self.receipt[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
            self.receipt[key] = old
        for key, value in [('tick', True), ('checksum', False), ('pixel_verdict_pending', True),
                           ('oracle_pixels_checked', 0), ('size', [224, 223])]:
            old = self.rows[-1][key]; self.rows[-1][key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
            self.rows[-1][key] = old
        self.rows.reverse()
        with self.assertRaisesRegex(RuntimeError, 'census'): self.check()
        self.rows.reverse()
        (self.rectangle / 'unexpected').write_bytes(b'x')
        with self.assertRaisesRegex(RuntimeError, 'census'): self.check()

    def test_saved_metadata_and_strict_json_rejected(self):
        path = self.rectangle / 'saved.tilemap.json'
        original = path.read_text(); doc = json.loads(original)
        for key, value in [('width', 5), ('height', True), ('origin', [0, 0]), ('tile_size', [True, 16]),
                ('version', True), ('units_per_pixel', True), ('sprite', {'package': 'other', 'document': 'sprites.json'}),
                ('cells', gate.RECTANGLE_WRONG), ('extra', 'forbidden')]:
            altered = dict(doc); altered[key] = value; path.write_text(json.dumps(altered))
            for row in self.rows:
                if row['name'] in ('saved', 'saved-noop', 'saved-undo-noop', 'saved-redo', 'fresh-reopen', 'play'):
                    row['history']['saved_fnv1a64'] = gate.fnv1a64(path.read_bytes())
            with self.subTest(key=key), self.assertRaises(RuntimeError): self.check()
        path.write_text(original[:-1] + ',"version":1}')
        for row in self.rows:
            if row['name'] in ('saved', 'saved-noop', 'saved-undo-noop', 'saved-redo', 'fresh-reopen', 'play'):
                row['history']['saved_fnv1a64'] = gate.fnv1a64(path.read_bytes())
        with self.assertRaisesRegex(RuntimeError, 'Duplicate'): self.check()

    def test_partial_and_fixture_binding_manifest_rejected(self):
        self.write()
        path = self.rectangle / 'partial-receipt.json'
        partial = json.loads(path.read_text())
        for bad in (True, 0):
            partial['completed'] = bad
            path.write_text(json.dumps(partial))
            with self.assertRaisesRegex(RuntimeError, 'partial'): self.check(rewrite=False)
        original = copy.deepcopy(self.receipt['fixture_file_bindings'])
        for bad in [[], original + original[:1], list(reversed(original)),
                    [{'path': '../escape', 'bytes': 1, 'fnv1a64': '0'*16}],
                    [{'path': 'arena.tilemap.json', 'bytes': True, 'fnv1a64': '0'*16}]]:
            self.receipt['fixture_file_bindings'] = bad
            with self.subTest(bad=bad), self.assertRaises(RuntimeError): self.check()

    def test_frame_png_and_visible_wrong_stride_tampering_rejected(self):
        path = self.rectangle / 'outside-origin.frame'; old = path.read_bytes()
        path.write_bytes(b'changed-frame'); self.rows[1]['frame_fnv1a64'] = gate.fnv1a64(path.read_bytes())
        with self.assertRaisesRegex(RuntimeError, 'parity'): self.check()
        path.write_bytes(old); self.rows[1]['frame_fnv1a64'] = gate.fnv1a64(old)
        path = self.rectangle / 'saved.png'; old = path.read_bytes()
        path.write_bytes(png_bytes(bytes([1, 2, 3, 4]), (1, 1)))
        with self.assertRaisesRegex(RuntimeError, 'PNG/raw'): self.check()
        path.write_bytes(old)
        self.receipt['wrong_stride_visible_pixels'] = 9
        with self.assertRaisesRegex(RuntimeError, 'pixel verdict'): self.check()
        self.receipt['wrong_stride_visible_pixels'] = 8
        (self.rectangle / 'presentation-free.frame').write_bytes(b'not-authoritative')
        self.receipt['presentation_free_frame_fnv1a64'] = gate.fnv1a64(b'not-authoritative')
        with self.assertRaisesRegex(RuntimeError, 'presentation-free'): self.check()

    def test_history_preview_and_both_flat_negative_controls_rejected(self):
        for name in ('preview', 'escape-cancelled', 'rectangle-painted', 'noop-preserves-redo',
                     'saved-noop', 'saved-undo-noop', 'fresh-reopen'):
            row = next(r for r in self.rows if r['name'] == name)
            history = row['history']
            for key, bad in [('undo', history['undo'] + 1), ('redo', True),
                             ('dirty', not history['dirty']), ('saved_fnv1a64', 'f' * 16)]:
                original = history[key]; history[key] = bad
                with self.subTest(name=name, key=key), self.assertRaises(RuntimeError): self.check()
                history[key] = original
        original = self.preview_pixels; self.preview_pixels = self.app_pixels
        with self.assertRaisesRegex(RuntimeError, 'preview pixel verdict'): self.check()
        self.preview_pixels = original
        for bad in (True, 0, 7, 1800 * 2000 + 1):
            self.receipt['preview_visible_pixels'] = bad
            with self.subTest(bad=bad), self.assertRaises(RuntimeError): self.check()
        self.receipt['preview_visible_pixels'] = 8
        for bad in ([True, 0, 8, 1], [0, 0, 1801, 1], [0, 0, 0, 1]):
            self.receipt['preview_pixel_rect'] = bad
            with self.assertRaises(RuntimeError): self.check()
        self.receipt['preview_pixel_rect'] = [0, 0, 8, 1]
        for name in ('runtime-wrong-stride', 'runtime-extra-cell'):
            path = self.rectangle / (name + '.rgba'); original = path.read_bytes()
            flat = bytes(len(original)); path.write_bytes(flat)
            (self.rectangle / (name + '.png')).write_bytes(png_bytes(flat, (224, 224)))
            row = next(r for r in self.rows if r['name'] == name)
            row['rgba_fnv1a64'] = gate.fnv1a64(flat)
            with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, 'pixel verdict'): self.check()
            path.write_bytes(original)
            (self.rectangle / (name + '.png')).write_bytes(png_bytes(original, (224, 224)))
            row['rgba_fnv1a64'] = gate.fnv1a64(original)

    def test_regular_files_required(self):
        path = self.rectangle / 'before-app.png'; old = path.read_bytes(); path.unlink()
        with self.assertRaises(RuntimeError): self.check()
        target = self.root / 'elsewhere'; target.write_bytes(old); path.symlink_to(target)
        with self.assertRaises(RuntimeError): self.check()

    def test_local_adapter_audit_never_replaces_hosted_default(self):
        for row in self.rows[:len(gate.RECTANGLE_APP_PHASES)]: row['adapter'] = 'SwiftShader Device (Subzero)'
        with self.assertRaisesRegex(RuntimeError, 'llvmpipe'): self.check()
        self.assertEqual(len(self.check(adapter='SwiftShader')), 137)
        with self.assertRaisesRegex(RuntimeError, 'empty adapter'): self.check(adapter='')
        # Main has no argument/environment escape hatch for hosted acceptance.
        import ast
        tree = ast.parse(Path(gate.__file__).read_text())
        main = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'main')
        calls = [n for n in ast.walk(main) if isinstance(n, ast.Call) and isinstance(n.func, ast.Name)
                 and n.func.id == 'validate_rectangle']
        self.assertEqual(len(calls), 1)
        self.assertEqual(len(calls[0].args), 3)
        self.assertEqual(calls[0].keywords, [])


if __name__ == '__main__':
    unittest.main()
