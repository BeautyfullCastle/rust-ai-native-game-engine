#!/usr/bin/env python3
"""Fail-closed inventory/hash gate for the actual common-canvas GPU acceptance."""

import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import struct
import tempfile
import unittest
import zlib

TEST = 'cli_canvas_sequence_installed_anchor_loop_once_and_wrong_offsets_gpu_readback'
MARKER = 'PNG canvas sequence: 300 production GPU captures'
PACKAGES = ('canvas-loop', 'canvas-once', 'canvas-wrong-x', 'canvas-wrong-y')
TIMES = (0, 99, 100, 249, 250, 449, 450, 550, 900, 18446744073709551615)
COUNT = 300
SIZE = (128, 128)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def validate_log(output):
    output = output.replace('\r\n', '\n')
    require(re.findall(r'^running (\d+) tests?$', output, re.M) == ['1'], 'exactly one GPU test required')
    require(re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in .+$', output, re.M) == [('1', '0', '0', '0', '0')], 'incomplete GPU test summary')
    # --nocapture can put renderer diagnostics between the test name and its ok.
    require(re.findall(r'^test (\S+) \.\.\. ', output, re.M) == [TEST], 'unexpected GPU test')
    require(output.count(MARKER) == 1 and not re.search(r'\b(?:SKIP|FAILED|ignored)\b(?!;)', output.replace('0 ignored;', '')), 'GPU evidence missing or skipped')


def png_size(data):
    require(data.startswith(b'\x89PNG\r\n\x1a\n'), 'capture is not PNG')
    offset, chunks = 8, []
    while offset < len(data):
        require(offset + 12 <= len(data), 'truncated PNG chunk')
        length = struct.unpack('>I', data[offset:offset + 4])[0]
        end = offset + 12 + length
        require(end <= len(data), 'truncated PNG payload')
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:end - 4]
        require(zlib.crc32(kind + payload) == struct.unpack('>I', data[end - 4:end])[0], 'PNG CRC mismatch')
        chunks.append((kind, payload))
        offset = end
    require(chunks and chunks[0][0] == b'IHDR' and len(chunks[0][1]) == 13, 'missing PNG header')
    require(chunks[-1] == (b'IEND', b'') and any(k == b'IDAT' for k, _ in chunks), 'incomplete PNG')
    width, height, depth, color, compression, filtering, interlace = struct.unpack('>IIBBBBB', chunks[0][1])
    require((width, height) == SIZE and (depth, color, compression, filtering, interlace) == (8, 6, 0, 0, 0), 'unexpected capture format')
    compressed = b''.join(p for k, p in chunks if k == b'IDAT')
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, (width * 4 + 1) * height + 1)
    require(decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail and len(raw) == (width * 4 + 1) * height, 'invalid PNG image payload')
    require(all(raw[y * (width * 4 + 1)] <= 4 for y in range(height)), 'invalid PNG filter')


def expected_inventory():
    return {(package, 'direct', time, transformed) for package in PACKAGES for time in TIMES for transformed in (False, True)} | {(package, 'host', tick, False) for package in PACKAGES for tick in range(55)}


def validate_receipt(directory):
    directory = Path(directory)
    receipt = json.loads((directory / 'receipt.json').read_text())
    require(type(receipt.get('schema')) is int and receipt['schema'] == 1, 'unexpected receipt schema')
    require(receipt.get('test') == TEST and type(receipt.get('capture_count')) is int and receipt['capture_count'] == COUNT, 'wrong test or capture count')
    require(all(isinstance(receipt.get(key), str) and receipt[key].strip() for key in ('adapter', 'backend')), 'missing adapter/backend')
    require(receipt['backend'] == 'Vulkan', 'required Vulkan backend missing')
    captures = receipt.get('captures')
    require(isinstance(captures, list) and len(captures) == COUNT, 'incomplete captures')
    inventory, names, proofs = set(), set(), set()
    for capture in captures:
        filename = capture.get('file')
        require(isinstance(filename, str) and re.fullmatch(r'[A-Za-z0-9_-]+\.png', filename), 'unsafe capture path')
        require(filename not in names, 'duplicate capture file')
        names.add(filename)
        path = directory / filename
        require(not path.is_symlink() and path.is_file(), 'missing or symlink capture')
        data = path.read_bytes()
        require(capture.get('sha256') == hashlib.sha256(data).hexdigest(), 'capture hash mismatch')
        png_size(data)
        require(type(capture.get('time_or_tick')) is int and type(capture.get('transformed')) is bool and type(capture.get('frame')) is int, 'invalid capture metadata')
        require(0 <= capture['frame'] <= 2, 'invalid source frame')
        key = (capture.get('package'), capture.get('phase'), capture['time_or_tick'], capture['transformed'])
        require(key in expected_inventory(), 'unexpected capture inventory')
        require(filename == f"{key[0]}-{key[1]}-{key[2]}-{int(key[3])}.png", 'capture filename does not match metadata')
        elapsed = key[2] if key[1] == 'direct' else key[2] * 1000 // 60
        sampled = min(elapsed, 449) if key[0] == 'canvas-once' else elapsed % 450
        expected_frame = 0 if sampled < 100 else (1 if sampled < 250 else 2)
        require(capture['frame'] == expected_frame, 'source frame does not match clock')
        require(key not in inventory, 'duplicate capture inventory')
        inventory.add(key)
        require(type(capture.get('wrong_offset_proof')) is bool, 'missing wrong-offset proof')
        require(capture['wrong_offset_proof'] == (capture['package'] in PACKAGES[2:]), 'wrong-offset proof mismatch')
        if capture['wrong_offset_proof']:
            require(capture['package'] in PACKAGES[2:], 'proof attached to correct package')
            proofs.add(capture['package'])
    require(inventory == expected_inventory(), 'capture inventory mismatch')
    require(proofs == set(PACKAGES[2:]), 'missing x/y negative controls')
    require({p.name for p in directory.glob('*.png')} == names, 'unreceipted PNG captures')
    return receipt


def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        width, height = SIZE
        png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes((width * 4 + 1) * height))) + chunk(b'IEND', b'')
        captures = []
        for package, phase, time, transformed in sorted(expected_inventory()):
            filename = f'{package}-{phase}-{time}-{int(transformed)}.png'
            (self.root / filename).write_bytes(png)
            elapsed = time if phase == 'direct' else time * 1000 // 60
            sampled = min(elapsed, 449) if package == 'canvas-once' else elapsed % 450
            frame = 0 if sampled < 100 else (1 if sampled < 250 else 2)
            captures.append(dict(file=filename, sha256=hashlib.sha256(png).hexdigest(), package=package, phase=phase, time_or_tick=time, transformed=transformed, frame=frame, wrong_offset_proof=package in PACKAGES[2:]))
        self.receipt = dict(schema=1, test=TEST, adapter='fixture only', backend='Vulkan', capture_count=COUNT, captures=captures)

    def write(self, receipt):
        (self.root / 'receipt.json').write_text(json.dumps(receipt))

    def test_complete_fixture(self):
        self.write(self.receipt)
        validate_receipt(self.root)

    def test_reject_bad_receipts(self):
        mutations = [lambda r: r.update(capture_count=0), lambda r: r.update(adapter=''), lambda r: r.update(backend='Metal'), lambda r: r['captures'][0].update(frame=2), lambda r: r['captures'].pop(), lambda r: r['captures'][0].update(sha256='0' * 64), lambda r: r['captures'][0].update(file='../outside.png'), lambda r: r['captures'][0].update(time_or_tick=True), lambda r: r['captures'][0].update(phase='fake'), lambda r: r['captures'].__setitem__(0, r['captures'][1]), lambda r: [c.update(wrong_offset_proof=False) for c in r['captures']]]
        for mutation in mutations:
            with self.subTest(mutation=mutations.index(mutation)):
                receipt = copy.deepcopy(self.receipt)
                mutation(receipt)
                self.write(receipt)
                with self.assertRaises(ValueError):
                    validate_receipt(self.root)

    def test_missing_symlink_and_corrupt_capture(self):
        self.write(self.receipt)
        capture = self.receipt['captures'][0]
        path = self.root / capture['file']
        original = path.read_bytes()
        path.unlink()
        with self.assertRaises(ValueError):
            validate_receipt(self.root)
        other = self.root / self.receipt['captures'][1]['file']
        path.symlink_to(other.name)
        with self.assertRaises(ValueError):
            validate_receipt(self.root)
        path.unlink()
        # Even a matching SHA256 must not admit an invalid image payload.
        corrupt = original[:-1] + bytes([original[-1] ^ 1])
        path.write_bytes(corrupt)
        capture['sha256'] = hashlib.sha256(corrupt).hexdigest()
        self.write(self.receipt)
        with self.assertRaises(ValueError):
            validate_receipt(self.root)

    def test_reject_invalid_png(self):
        for data in (b'', b'not PNG', b'\x89PNG\r\n\x1a\n', b'\x89PNG\r\n\x1a\n' + chunk(b'IEND', b'')):
            with self.assertRaises(ValueError):
                png_size(data)

    def test_exact_log(self):
        output = f'running 1 test\ntest {TEST} ... adapter output\n{MARKER}\nok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n'
        validate_log(output)
        for bad in ('', output.replace('running 1', 'running 0'), output.replace('1 passed', '0 passed'), output.replace(TEST, 'other_test'), output.replace(MARKER, ''), output + 'SKIP: no adapter\n', output.replace('0 filtered', '1 filtered')):
            with self.assertRaises(ValueError):
                validate_log(bad)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=['check-png-canvas-receipt.py'])
    elif args.directory:
        validate_receipt(args.directory)
        print('Verified 300 hashed PNG GPU captures, complete inventory and wrong-offset controls')
    else:
        parser.error('provide receipt directory or --self-test')
