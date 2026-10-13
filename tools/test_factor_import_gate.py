#!/usr/bin/env python3
"""Bounded factor-import gate controls; no compiler or device is launched."""
import json
from pathlib import Path, PurePosixPath, PureWindowsPath
import sys
import tempfile
import unittest
from unittest import mock
sys.dont_write_bytecode = True
import factor_import_gate as gate

BLUE = bytes((137, 188, 225, 255))
PINK = bytes((255, 137, 188, 255))
CLEAR = bytes((0, 0, 0, 255))
ADAPTER = 'name="fixture" backend=Vulkan device_type=Cpu vendor=1 device=2 driver="fixture" driver_info="version" software=true'


def frame(first=200, second=40, shift=0):
    pixels = [CLEAR] * (gate.SIZE * gate.SIZE)
    pixels[shift:shift+first] = [BLUE] * first
    pixels[shift+first:shift+first+second] = [PINK] * second
    return b''.join(pixels)


def log_rows(raws):
    return '\n'.join('factor color controls: counts=[%d, %d] clear=%d' % (raw.count(BLUE), raw.count(PINK), raw.count(CLEAR)) for raw in raws)


class FactorGateTests(unittest.TestCase):
    def reject(self, function, *args):
        with self.assertRaises((AssertionError, RuntimeError)):
            function(*args)

    def test_platform_command_and_work_census(self):
        for platform, count, unique in [('linux', 46, 39), ('win32', 45, 38)]:
            lanes = gate.lanes_for(platform)
            self.assertEqual(sum(len(row['cpu']) for row in lanes), count)
            self.assertEqual(len({name for row in lanes for name in row['cpu']}), unique)
            self.assertEqual(len(gate.command_inventory(platform)), 33)
            self.assertEqual(len(set(gate.command_inventory(platform))), 33)
            self.assertEqual([len(row['ignored']) for row in lanes], [0, 0, 0, 0, 0, 0, 1, 2])
        self.assertEqual(sum(map(len, gate.FRAMES.values())), 8)
        self.assertEqual(8 * gate.SIZE**2, 294912)
        self.assertEqual(44 * gate.SIZE**2, 1622016)

    def test_exact_results_names_and_ignored(self):
        for suffix in ('', '.test_fixed'):
            verbose = 'test_fixed (__main__.FixedTests'+suffix+') ... ok\n\nRan 1 test in 0.01s\n\nOK\n'
            gate.helper_results(verbose, 'FixedTests', ['test_fixed'])
            self.reject(gate.helper_results, verbose.replace('test_fixed (__main__', 'test_other (__main__'), 'FixedTests', ['test_fixed'])
        wrong = 'test_fixed (__main__.FixedTests.test_other) ... ok\n\nRan 1 test in 0.01s\n\nOK\n'
        self.reject(gate.helper_results, wrong, 'FixedTests', ['test_fixed'])
        text = 'running 2 tests\ntest alpha ... ok\ntest device ... ignored\n\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s\n'
        gate.common.result(text, ['alpha'], ['device'], 0)
        for changed in [text.replace('alpha', 'other'), text + 'test alpha ... ok\n', text.replace('device ... ignored', 'device ... ok')]:
            self.reject(gate.common.result, changed, ['alpha'], ['device'], 0)

    def test_transfer_missing_and_double_encoding(self):
        self.assertEqual([gate.encode(x) for x in (0, .25, .5, .75, 1)], [0, 137, 188, 225, 255])
        self.assertTrue(gate.color_matches(BLUE, gate.FACTORS[0]))
        self.assertFalse(gate.color_matches(bytes((64, 128, 191, 255)), gate.FACTORS[0]))
        self.assertFalse(gate.color_matches(bytes(gate.encode(x/255) for x in BLUE), gate.FACTORS[0]))

    def test_raw_shape_and_metadata_admission(self):
        self.reject(gate.raw_controls, frame()[:-1], 'static')
        self.reject(gate.raw_controls, frame(), 'unknown')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in gate.FRAMES['static']:
                (root/(name+'.rgba')).write_bytes(frame())
            self.reject(gate.audit_capture, root, 'static', 'factor color controls: counts=[NaN, 40] clear=36624\n'*2)
            self.reject(gate.audit_capture, root, 'static', log_rows([frame(), frame()]).replace('[200, 40]', '[201, 40]'))

    def test_display_and_linear_channel_bounds(self):
        good = gate.raw_controls(frame(), 'static')
        self.assertEqual(good['counts'], [200, 40])
        for index, value in [(0, 140), (1, 191), (2, 227), (3, 252)]:
            wrong = bytearray(frame()); wrong[index] = value
            self.reject(gate.raw_controls, bytes(wrong), 'static')
        # 227 is within two display codes but exceeds the independent linear bound.
        self.assertLessEqual(abs(227-225), 2)
        self.assertGreater(abs(gate.decode(227)-.75), 2/255)

    def test_nonvacuous_foreground_and_background(self):
        for raw in [frame(99, 40), frame(200, 19), frame(gate.SIZE**2-139, 40)]:
            self.reject(gate.raw_controls, raw, 'static')
        self.assertEqual(gate.raw_controls(frame(100, 20), 'static')['counts'], [100, 20])

    def test_static_pair_equality(self):
        raws = dict.fromkeys(gate.FRAMES['static'], frame())
        gate.check_pairs(raws, 'static')
        raws['static-white'] = frame(shift=1)
        self.reject(gate.check_pairs, raws, 'static')
        self.reject(gate.check_pairs, {'static-factor': frame()}, 'static')

    def test_animation_pairs_and_pose_changes(self):
        raws = {f'animated-{mode}-{pose}': frame(200, 0, index) for mode in ('factor', 'white') for index, pose in enumerate(('start', 'midpoint', 'key'))}
        gate.check_pairs(raws, 'animated')
        broken = dict(raws); broken['animated-white-key'] = frame(200, 0, 3)
        self.reject(gate.check_pairs, broken, 'animated')
        broken = dict(raws)
        for mode in ('factor', 'white'):
            broken[f'animated-{mode}-midpoint'] = broken[f'animated-{mode}-start']
        self.reject(gate.check_pairs, broken, 'animated')

    def test_full_adapter_identity(self):
        text = 'factor adapter: '+ADAPTER+'\n'
        self.assertEqual(gate.observed_adapter(text, ADAPTER), ADAPTER)
        for changed in [text.replace('software=true', 'software=false'), text.replace('backend=Vulkan', 'backend=Gl'), text.replace(' driver_info="version"', ''), text+text]:
            self.reject(gate.observed_adapter, changed)
        self.reject(gate.observed_adapter, text, ADAPTER.replace('device=2', 'device=3'))

    def test_cross_platform_file_census(self):
        self.assertEqual(gate.common.relative_capture_key(PureWindowsPath('C:/owned/sub/frame.rgba'), PureWindowsPath('C:/owned')), 'sub/frame.rgba')
        self.assertEqual(gate.common.relative_capture_key(PurePosixPath('/owned/sub/frame.rgba'), PurePosixPath('/owned')), 'sub/frame.rgba')
        self.assertEqual(gate.common.relative_capture_key(PurePosixPath('/owned/a\\b'), PurePosixPath('/owned')), 'a\\b')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root/'frame').write_bytes(b'x')
            gate.require_files(root, {'frame'})
            self.reject(gate.require_files, root, set())
            self.reject(gate.require_files, root, {'frame', 'missing'})
            with mock.patch.object(Path, 'is_symlink', return_value=True):
                self.reject(gate.require_files, root, {'frame'})

    def test_lossless_png_and_changed_pixels(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); raw = frame()
            for name in gate.FRAMES['static']:
                (root/(name+'.rgba')).write_bytes(raw)
            audit = gate.audit_capture(root, 'static', log_rows([raw, raw]))
            self.assertEqual(len(audit), 4)
            self.assertEqual(gate.common.png_rgba((root/'static-factor.png').read_bytes(), (192, 192)), raw)
            (root/'static-factor.png').write_bytes(gate.encode_png(frame(shift=1)))
            self.reject(gate.audit_capture, root, 'static', log_rows([raw, raw]))

    def test_fences_and_partial_failure_measurements(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); captures = root/'captures'; cache = root/'cache'
            captures.mkdir(); cache.mkdir(); path = captures/'partial.rgba'; path.write_bytes(b'partial')
            records = {str(path): gate.digest(b'partial')}; gate.verify_hashes(records)
            path.write_bytes(b'changed'); self.reject(gate.verify_hashes, records)
            with mock.patch.object(gate, 'CAPTURE_CAP', 1):
                self.reject(gate.record_budgets, captures, cache, root/'failure.json')
            receipt = json.loads((root/'failure.json').read_text())
            self.assertEqual(receipt['capture_bytes'], 7)
            self.assertEqual(path.read_bytes(), b'changed')
            with mock.patch.object(gate.common.Gate, 'run', side_effect=RuntimeError('injected child failure')):
                fake = object.__new__(gate.FactorGate)
                fake.number = 0; fake.captures = captures; fake.cache = cache; fake.evidence = root
                (root/'budgets').mkdir()
                self.reject(fake.run, 'injected', ['never-launched'])
                self.assertEqual(json.loads((root/'budgets/001-injected.json').read_text())['capture_bytes'], 7)


if __name__ == '__main__':
    unittest.main()
