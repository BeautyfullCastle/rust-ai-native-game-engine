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


if __name__ == '__main__':
    unittest.main()
