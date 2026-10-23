#!/usr/bin/env python3
"""Independent Room wall gate fixtures; never a substitute for real consumers.

--semantic-only excludes only actual workflow integration, has a distinct
non-acceptance marker, and cannot satisfy room_wall_gate.begin(). Default runs
the complete census, including current mandatory hosted wiring.
"""
import argparse
import ast
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import zlib
import sys
import tempfile
import unittest
from unittest import mock

import room_wall_gate as gate

ROOT = Path(__file__).resolve().parents[1]
EXPECTED_SEMANTIC_TESTS = ('test_actual_cpu_and_consumer_source_census', 'test_canonical_wall_operation_rejects_nonwall_rotation_and_filter', 'test_capture_inventory_rejects_extra_missing_and_symlinks', 'test_capture_references_bind_exact_file_bytes_and_hashes', 'test_commands_use_reviewed_bounded_runner_and_source_fences', 'test_complete_source_contract', 'test_exact_test_results_reject_ignored_filtered_and_duplicate', 'test_exact_unoptimized_bytecode_runtime_required', 'test_feature_flags_and_cli_interface_are_frozen', 'test_finish_keeps_original_failure_and_final_fence_evidence', 'test_fixture_marker_cannot_launder_missing_skipped_or_failed_rows', 'test_frame_layout_and_collision_los_controls_are_independent', 'test_full_cpu_retained_adoption_requires_real_layout_and_provenance', 'test_full_export_requires_frame_controls_namespace_and_manifest_identity', 'test_full_gpu_retained_oracle_rejects_opaque_receipt_claims', 'test_full_snapshot_covers_reviewed_runner_and_feature_sources', 'test_initialization_failure_retains_partial_receipt', 'test_mixed_ignored_census_executes_every_test_without_filtering', 'test_mode_coverage_never_credits_unrun_consumers', 'test_numeric_gpu_oracle_checks_expected_geometry_and_models', 'test_numeric_gpu_oracle_rejects_wrong_geometry_and_vacuity', 'test_ops_require_one_ordinary_explicit_spawn', 'test_package_model_closure_and_unrelated_binding_preservation', 'test_pinned_parallel_workflow_rejects_weakened_rows_and_retention', 'test_ppm_bytes_must_match_exact_raw_pixels', 'test_private_fresh_targets_and_outside_evidence', 'test_producer_requires_current_fresh_exact_build_identity', 'test_receipt_provenance_binds_source_and_fresh_retained_producer', 'test_runtime_and_probe_require_actual_checksum_and_namespace_markers', 'test_source_fence_bytes_modes_and_paths', 'test_source_inventory_rejects_ignored_and_duplicate_paths', 'test_source_rejects_symlink_files_and_directories', 'test_strict_json_rejects_duplicates_nonfinite_and_nonregular', 'test_workflow_pending_is_fail_closed_and_jobs_are_unique')


def listing(names):
    return ''.join(name + ': test\n' for name in names) + f'\n{len(names)} tests, 0 benchmarks\n'


def passed(names):
    return (f'running {len(names)} test' + ('' if len(names) == 1 else 's') + '\n'
            + ''.join('test ' + name + ' ... ok\n' for name in names)
            + f'\ntest result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')


def cpu_census(source):
    declarations = re.findall(r'((?:^#\[[^\n]+\]\n)+)^fn ([A-Za-z0-9_]+)\(', source, re.M)
    actual = {name: attributes.splitlines() for attributes, name in declarations
              if '#[test]' in attributes.splitlines()}
    require_names = set(gate.CPU_TESTS)
    if set(actual) != require_names or len(actual) != len(gate.CPU_TESTS):
        raise AssertionError('complete exact CPU source census')
    for name, attributes in actual.items():
        expected = ['#[test]', '#[ignore = "mandatory gate: freshly retained Room wall CLI and CPU evidence directory required"]'] if name in gate.CPU_IGNORED else ['#[test]']
        if attributes != expected:
            raise AssertionError('exact CPU test attributes: ' + name)


class RoomWallSemanticTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.repo = self.root / 'checkout'
        self.repo.mkdir()
        patch = mock.patch.dict(os.environ, {}, clear=False)
        patch.start()
        self.addCleanup(patch.stop)
        gate.initialize(self.repo, self.root / 'evidence', 'cpu')

    def test_exact_unoptimized_bytecode_runtime_required(self):
        baseline = gate.python_runtime()
        self.assertTrue(baseline['__debug__'])
        self.assertTrue(baseline['dont_write_bytecode'])
        self.assertIsNone(baseline['PYTHONOPTIMIZE'])
        for key, value in (('optimize', 1), ('__debug__', False), ('PYTHONOPTIMIZE', '0'),
                           ('PYTHONOPTIMIZE', ''), ('PYTHONOPTIMIZE', '1'),
                           ('dont_write_bytecode', False), ('PYTHONDONTWRITEBYTECODE', None)):
            with self.subTest(key=key, value=value), mock.patch.object(gate, 'python_runtime', return_value=dict(baseline, **{key: value})), mock.patch('sys.stderr'), self.assertRaises(SystemExit):
                gate.require_python()
        tree = ast.parse((ROOT / 'tools/room_wall_gate.py').read_text())
        guard = next(index for index, node in enumerate(tree.body) if isinstance(node, ast.Expr)
                     and isinstance(node.value, ast.Call) and getattr(node.value.func, 'id', '') == 'require_python')
        imported = next(index for index, node in enumerate(tree.body) if isinstance(node, ast.Import)
                        and any(item.name == 'static_model_extract_gate' for item in node.names))
        self.assertLess(guard, imported)

    def test_private_fresh_targets_and_outside_evidence(self):
        report = gate.runner.report
        self.assertTrue(report['private_target']['fresh_empty'])
        self.assertEqual(list(gate.runner.target_dir.iterdir()), [])
        self.assertFalse(gate.runner.evidence.is_relative_to(self.repo))
        self.assertEqual(os.environ['TMPDIR'], str(gate.runner.evidence / 'temporary'))
        for bad in (self.repo / 'evidence', self.root / 'evidence', Path('relative-evidence')):
            with self.subTest(bad=bad), self.assertRaises(AssertionError):
                gate.initialize(self.repo, bad, 'cpu')

    def test_complete_source_contract(self):
        value = {name: {} for name in gate.FEATURE_PATHS + gate.INFRA_PATHS}
        for package in ('orr_remote', 'orr_sample', 'orr_edit', 'orr_reflect', 'orr_games'):
            for name in ('Cargo.toml', 'src/lib.rs'):
                value[f'crates/{package}/{name}'] = {}
        gate.require_sources(value)
        for name in value:
            changed = dict(value)
            del changed[name]
            with self.subTest(name=name), self.assertRaises(AssertionError):
                gate.require_sources(changed)

    def test_source_fence_bytes_modes_and_paths(self):
        path = self.repo / 'source.rs'
        path.write_text('before')
        gate.frozen = gate.snapshot()
        gate.source_fence()
        path.write_text('after')
        with self.assertRaises(AssertionError):
            gate.source_fence()
        path.write_text('before')
        with mock.patch.object(gate, 'snapshot', return_value={'source.rs': dict(gate.frozen['source.rs'], mode=0)}), self.assertRaises(AssertionError):
            gate.source_fence()
        (self.repo / 'new.rs').write_text('new')
        with self.assertRaises(AssertionError):
            gate.source_fence()
        self.assertEqual(gate.runner.report['source_path_inventories']['source-fence']['extra'], ['new.rs'])

    def test_source_rejects_symlink_files_and_directories(self):
        (self.repo / 'source.rs').write_text('source')
        with mock.patch.object(Path, 'is_symlink', return_value=True), self.assertRaises(AssertionError):
            gate.snapshot()
        (self.repo / 'subdir').mkdir()
        original = Path.is_symlink
        with mock.patch.object(Path, 'is_symlink', lambda path: path.name == 'subdir' or original(path)), self.assertRaises(AssertionError):
            gate.snapshot()

    def test_source_inventory_rejects_ignored_and_duplicate_paths(self):
        path = self.repo / 'source.rs'
        path.write_text('source')
        gate.check_source_inventory(['source.rs'], 'valid')
        with self.assertRaises(AssertionError):
            gate.check_source_inventory(['source.rs', 'source.rs'], 'duplicate')
        ignored = self.repo / 'tools/__pycache__/hidden.pyc'
        ignored.parent.mkdir(parents=True)
        ignored.write_bytes(b'not legitimate source')
        with self.assertRaises(AssertionError):
            gate.check_source_inventory(['source.rs'], 'ignored')
        self.assertTrue(ignored.exists())
        self.assertEqual(gate.runner.report['source_path_inventories']['ignored']['extra'],
                         ['tools/__pycache__/hidden.pyc'])

    def test_full_snapshot_covers_reviewed_runner_and_feature_sources(self):
        with mock.patch.object(gate, 'source_root', ROOT):
            current = gate.snapshot()
        gate.require_sources(current)
        for path in gate.FEATURE_PATHS + gate.INFRA_PATHS:
            self.assertEqual(current[path]['sha256'], hashlib.sha256((ROOT / path).read_bytes()).hexdigest())
        self.assertIn('crates/orr_sample/src/room_project.rs', current)
        self.assertIn('crates/orr_sample/src/bin/room_escape.rs', current)
        self.assertIn('crates/orr_sample/src/bin/orr_export_room.rs', current)

    def test_commands_use_reviewed_bounded_runner_and_source_fences(self):
        order = []
        with mock.patch.object(gate, 'source_fence', side_effect=lambda: order.append('fence')), mock.patch.object(gate.runner, 'run', side_effect=lambda *args, **kwargs: order.append('run') or 'output'):
            self.assertEqual(gate.run('fixture', ['no-execution']), 'output')
        self.assertEqual(order, ['fence', 'run', 'fence'])
        source = (ROOT / 'tools/room_wall_gate.py').read_text()
        tree = ast.parse(source)
        self.assertFalse(any(isinstance(node, (ast.Import, ast.ImportFrom))
                             and ((isinstance(node, ast.ImportFrom) and node.module == 'subprocess')
                                  or (isinstance(node, ast.Import) and any(item.name == 'subprocess' for item in node.names)))
                             for node in ast.walk(tree)))
        self.assertIn('runner.run(label, command, expected=expected, seconds=seconds)', source)

    def producer(self):
        package = self.repo / 'crates/orr_remote'
        package.mkdir(parents=True)
        (package / 'Cargo.toml').write_text('fixture manifest')
        executable = gate.runner.target_dir / 'release/examples/room_wall'
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b'not executable; pin-only fixture')
        return {'reason': 'compiler-artifact', 'fresh': False,
                'manifest_path': str(package / 'Cargo.toml'),
                'package_id': 'path+' + package.as_uri() + '#orr_remote@0.0.1',
                'target': {'name': 'room_wall', 'src_path': str(package / 'examples/room_wall.rs'),
                           'kind': ['example'], 'crate_types': ['bin']},
                'profile': {'test': False, 'opt_level': '3'},
                'features': ['room-project', 'sample-host'], 'executable': str(executable)}

    def test_producer_requires_current_fresh_exact_build_identity(self):
        item = self.producer()
        for changed in (dict(item, fresh=True), dict(item, features=['room-project']),
                        dict(item, package_id='registry+https://example.invalid#orr_remote'),
                        dict(item, manifest_path=str(self.repo / 'Cargo.toml')),
                        dict(item, profile={'test': True, 'opt_level': '3'})):
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.runner.pin(json.dumps(changed), 'invalid', 'orr_remote', 'room_wall', 'example', ['room-project', 'sample-host'])
        gate.runner.report.update(source_inventory_sha256='a' * 64, head='b' * 40, tree='c' * 40)
        binary = gate.pin(json.dumps(item), 'room-wall-cli', 'orr_remote', 'room_wall', 'example', ['room-project', 'sample-host'])
        self.assertEqual(Path(binary).read_bytes(), Path(item['executable']).read_bytes())
        record = gate.runner.report['producers']['room-wall-cli']
        self.assertEqual(record['source_inventory_sha256'], 'a' * 64)
        self.assertEqual(record['source_head'], 'b' * 40)
        self.assertEqual(record['source_tree'], 'c' * 40)

    def test_fixture_marker_cannot_launder_missing_skipped_or_failed_rows(self):
        marker = 'ROOM_WALL_GATE_FIXTURES: 1 passed; 0 failed; 0 skipped'
        value = ('test_one (__main__.Fixture.test_one) ... ok\n\n' + '-' * 70
                 + '\nRan 1 tests in 0.01s\n\nOK\n' + marker + '\n')
        gate.fixture_result(value, 1, marker)
        noisy = value.replace(' ... ok\n', ' ... deadline: failed\n\nexpected-two: passed\n'
                              'unexpected-seven: failed\nparent complete\n\nok\n', 1)
        gate.fixture_result(noisy, 1, marker)
        gate.fixture_result(noisy.replace('\n', '\r\n'), 1, marker)
        gate.fixture_result(value.replace(' ... ok', ' ... \nok'), 1, marker)
        class_only = noisy.replace('(__main__.Fixture.test_one)', '(__main__.Fixture)')
        gate.fixture_result(class_only, 1, marker)
        gate.fixture_result(class_only.replace('\n', '\r\n'), 1, marker)
        hosted_shape = value.replace('test_one (__main__.Fixture.test_one)',
                                    'test_actual_model_census_matches_test_source (__main__.StaticPropGateTests)')
        gate.fixture_result(hosted_shape, 1, marker)
        cases = {
            'marker-only': marker,
            'wrong-total': value.replace('Ran 1', 'Ran 0'),
            'malformed-duration': value.replace('0.01s', '0..01s'),
            'missing-total': value.replace('Ran 1 tests in 0.01s\n', ''),
            'duplicate-stream': value + value,
            'duplicate-name': value.replace('\n\n' + '-' * 70, '\ntest_one (__main__.Fixture.test_one) ... ok\n\n' + '-' * 70),
            'duplicate-method-other-class': value.replace('\n\n' + '-' * 70, '\ntest_one (__main__.Other.test_one) ... ok\n\n' + '-' * 70),
            'mismatched-identity': value.replace('Fixture.test_one', 'Fixture.test_two'),
            'class-only-explicit-mismatch': class_only.replace('(__main__.Fixture)', '(__main__.Fixture.test_two)'),
            'extra-identity-segment': value.replace('Fixture.test_one', 'Fixture.test_one.extra'),
            'missing-class-module': class_only.replace('__main__.Fixture', 'Fixture'),
            'unverified-class-module': class_only.replace('__main__.Fixture', 'other.Fixture'),
            'malformed-header': value.replace(' ... ok', ' .. ok'),
            'missing-record': value.split('\n', 1)[1],
            'missing-completion': noisy.replace('\nok\n', '\n'),
            'nested-record': noisy.replace('\nok\n', '\ntest_two (__main__.Fixture.test_two) ... ok\nok\n'),
            'nested-inline-record': noisy.replace('deadline: failed', 'test_two (__main__.Fixture.test_two) ... ok'),
            'orphan-initial-completion': 'ok\n' + value,
            'duplicate-completion': value.replace(' ... ok\n', ' ... ok\nok\n'),
            'payload-before-record': 'child output\n' + value,
            'payload-after-record': value.replace(' ... ok\n', ' ... ok\nchild output\n'),
            'missing-OK': value.replace('\nOK\n', '\n'),
            'duplicate-OK': value.replace('\nOK\n', '\nOK\nOK\n'),
            'false-OK': value.replace('\nOK\n', '\nOK (skipped=1)\n'),
            'failed-footer': value.replace('\nOK\n', '\nFAILED (failures=1)\n'),
            'missing-marker': value.replace(marker, ''),
            'duplicate-marker': value + marker,
            'payload-after-marker': value + 'child output\n',
            'bare-carriage-return': noisy.replace('parent complete', 'parent\rcomplete'),
        }
        for status in ('skipped', "skipped 'unavailable'", 'FAIL', 'ERROR', 'FAILED (failures=1)',
                       'expected failure', 'unexpected success', 'ok extra', 'OK', 'SKIP: unavailable'):
            cases['inline-status-' + status] = value.replace(' ... ok', ' ... ' + status)
            cases['noisy-status-' + status] = noisy.replace('parent complete', status)
        for status in ('skipped', 'FAIL', 'ERROR', 'expected failure', 'unexpected success'):
            cases['embedded-status-' + status] = noisy.replace('parent complete', 'child ... ' + status)
        for label, changed in cases.items():
            with self.subTest(label=label), self.assertRaises(AssertionError):
                gate.fixture_result(changed, 1, marker)
        two_marker = 'ROOM_WALL_GATE_FIXTURES: 2 passed; 0 failed; 0 skipped'
        two = (noisy.replace('\n\n' + '-' * 70, '\ntest_two (__main__.Fixture.test_two) ... ok\n\n' + '-' * 70)
               .replace('Ran 1', 'Ran 2').replace(marker, two_marker))
        gate.fixture_result(two, 2, two_marker)
        mixed = two.replace('(__main__.Fixture.test_one)', '(__main__.Fixture)')
        gate.fixture_result(mixed, 2, two_marker)
        gate.fixture_result(two.replace('(__main__.Fixture.test_two)', '(__main__.Fixture)'), 2, two_marker)
        for changed in (two.replace('test_two', 'test_one'),
                        mixed.replace('test_two', 'test_one'),
                        two.replace('(__main__.Fixture.test_two)', '(__main__.Fixture)').replace('test_two', 'test_one'),
                        two.replace('test_two (__main__.Fixture.test_two) ... ok\n', ''),
                        two.replace('\nok\n', '\n')):
            with self.subTest(two_records=changed), self.assertRaises(AssertionError):
                gate.fixture_result(changed, 2, two_marker)

    def test_exact_test_results_reject_ignored_filtered_and_duplicate(self):
        names = ['one', 'two']
        gate.runner.inventory(listing(names), names)
        gate.runner.result(passed(names), names)
        for value in (passed(names).replace('0 ignored', '1 ignored'),
                      passed(names).replace('0 filtered out', '1 filtered out'),
                      passed(names).replace('test two', 'test one'),
                      passed(names).replace(' ... ok', ' ... ignored', 1),
                      passed(names).replace(' ... ok', ' ... SKIP: missing adapter\nok', 1)):
            with self.subTest(value=value), self.assertRaises(AssertionError):
                gate.runner.result(value, names)

    def test_mixed_ignored_census_executes_every_test_without_filtering(self):
        calls = []
        outputs = [listing(gate.CPU_TESTS), listing(gate.CPU_IGNORED), passed(gate.CPU_TESTS)]
        with mock.patch.object(gate, 'run', side_effect=lambda label, command: calls.append(command) or outputs.pop(0)):
            gate.test('cpu', 'retained-producer', gate.CPU_TESTS, gate.CPU_IGNORED)
        self.assertEqual(calls, [['retained-producer', '--list'],
                                 ['retained-producer', '--ignored', '--list'],
                                 ['retained-producer', '--include-ignored', '--test-threads=1']])

    def test_strict_json_rejects_duplicates_nonfinite_and_nonregular(self):
        path = self.root / 'receipt.json'
        path.write_text('{"a":1}')
        self.assertEqual(gate.strict_json(path), {'a': 1})
        for text in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":1e999}', '{"a":Infinity}'):
            path.write_text(text)
            with self.subTest(text=text), self.assertRaises(AssertionError):
                gate.strict_json(path)
        with self.assertRaises(AssertionError):
            gate.strict_json(self.root)

    def test_capture_inventory_rejects_extra_missing_and_symlinks(self):
        root = self.root / 'captures'
        root.mkdir()
        (root / 'one').write_bytes(b'one')
        self.assertEqual(set(gate.capture_files(root, {'one'})), {'one'})
        for expected in ({'missing'}, set(), {'one', 'missing'}):
            with self.subTest(expected=expected), self.assertRaises(AssertionError):
                gate.capture_files(root, expected)
        with mock.patch.object(Path, 'is_symlink', return_value=True), self.assertRaises(AssertionError):
            gate.capture_files(root, {'one'})

    def test_capture_references_bind_exact_file_bytes_and_hashes(self):
        path = self.root / 'one'
        path.write_bytes(b'one')
        entry = {'file': 'one', 'bytes': 3, 'sha256': gate.runner.digest(path)}
        gate.file_reference(self.root, entry, 'one')
        for changed in (dict(entry, file='../one'), dict(entry, bytes=4), dict(entry, sha256='0' * 64),
                        dict(entry, extra=True)):
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.file_reference(self.root, changed, 'one')
        fnv = {'file': 'one', 'bytes': 3, 'fnv1a64': gate.fnv1a64(b'one')}
        gate.fnv_reference(self.root, fnv, 'one')
        with self.assertRaises(AssertionError):
            gate.fnv_reference(self.root, dict(fnv, fnv1a64='0' * 16), 'one')

    def test_receipt_provenance_binds_source_and_fresh_retained_producer(self):
        for path in gate.FEATURE_PATHS[:3]:
            file = self.repo / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(path)
        producer = gate.runner.evidence / 'room-wall-cli'
        producer.write_bytes(b'fresh retained CLI')
        gate.runner.report['producers']['room-wall-cli'] = {'file': producer.name, 'sha256': gate.runner.digest(producer)}
        receipt = {'source_fnv1a64': {name: gate.fnv1a64((self.repo / path).read_bytes())
                                   for name, path in zip(('cli', 'authoring', 'test'), gate.FEATURE_PATHS[:3])},
                   'producer_fnv1a64': {'room-wall-cli': gate.fnv1a64(producer.read_bytes())}}
        gate.fnv_provenance(receipt, gate.FEATURE_PATHS[2], ['room-wall-cli'])
        changed = copy.deepcopy(receipt)
        changed['source_fnv1a64']['test'] = '0' * 16
        with self.assertRaises(AssertionError):
            gate.fnv_provenance(changed, gate.FEATURE_PATHS[2], ['room-wall-cli'])
        producer.write_bytes(b'old producer swapped in')
        with self.assertRaises(AssertionError):
            gate.fnv_provenance(receipt, gate.FEATURE_PATHS[2], ['room-wall-cli'])

    @staticmethod
    def frames():
        baseline = bytearray(bytes([20, 30, 40, 255]) * (512 * 512))
        authored = bytearray(baseline)
        wrong = bytearray(baseline)
        for y in range(80, 176):
            for x in range(384, 448):
                offset = (y * 512 + x) * 4
                authored[offset:offset + 4] = bytes([90, 110, 140, 255])
        for y in range(336, 432):
            for x in range(384, 448):
                offset = (y * 512 + x) * 4
                wrong[offset:offset + 4] = bytes([90, 110, 140, 255])
        absent = bytearray(baseline)
        for offset in range(100 * 4, 140 * 4, 4):
            absent[offset:offset + 4] = bytes([0, 0, 0, 255])
        return {'baseline': bytes(baseline), 'authored': bytes(authored),
                'wrong-position': bytes(wrong), 'models-off': bytes(absent)}

    def test_numeric_gpu_oracle_checks_expected_geometry_and_models(self):
        value = gate.room_wall_pixel_oracle(self.frames())
        self.assertEqual(value['wall_interior_pixels'], 5520)
        self.assertEqual(value['wall_exterior_pixels'], 255344)
        self.assertEqual(value['preserved_model_pixels'], 40)
        self.assertEqual(value['control_pixel_differences']['baseline'], 6144)

    def test_numeric_gpu_oracle_rejects_wrong_geometry_and_vacuity(self):
        frames = self.frames()
        mutations = []
        mutations.append(dict(frames, authored=frames['baseline']))
        mutations.append(dict(frames, authored=frames['wrong-position']))
        mutations.append(dict(frames, **{'models-off': frames['authored']}))
        changed = bytearray(frames['authored'])
        changed[:4] = b'\xff\xff\xff\xff'
        mutations.append(dict(frames, authored=bytes(changed)))
        for changed in mutations:
            with self.subTest(), self.assertRaises(AssertionError):
                gate.room_wall_pixel_oracle(changed)

    def test_ppm_bytes_must_match_exact_raw_pixels(self):
        pixels = self.frames()['baseline']
        (self.root / 'frame.rgba').write_bytes(pixels)
        ppm = self.root / 'frame.ppm'
        ppm.write_bytes(b'P6\n512 512\n255\n' + bytes(value for index, value in enumerate(pixels) if index % 4 != 3))
        self.assertEqual(gate.read_rgba_ppm(self.root, 'frame'), pixels)
        ppm.write_bytes(ppm.read_bytes() + b'x')
        with self.assertRaises(AssertionError):
            gate.read_rgba_ppm(self.root, 'frame')

    def test_ops_require_one_ordinary_explicit_spawn(self):
        path = self.root / 'ops.json'
        operation = {'op': 'spawn', 'guid': 'e_00000080', 'name': 'Authored wall', 'components': {'fixture': {}}}
        path.write_text(json.dumps([operation]))
        self.assertEqual(gate.validate_ops(path), operation)
        for changed in ([], [operation, operation], [dict(operation, op='patch')],
                        [dict(operation, source_digest='0' * 64)], [dict(operation, name='')]):
            path.write_text(json.dumps(changed))
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.validate_ops(path)

    def test_mode_coverage_never_credits_unrun_consumers(self):
        gate.runner.report['workflow_contract'] = 'PASSED'
        gate.runner.report['coverage']['cpu'] = 'PASSED'
        gate.check_coverage()
        gate.runner.report['coverage']['gpu'] = 'PASSED'
        with self.assertRaises(AssertionError):
            gate.check_coverage()
        gate.runner.report['coverage']['gpu'] = 'NOT RUN'
        gate.runner.report['workflow_contract'] = 'PENDING'
        with self.assertRaises(AssertionError):
            gate.check_coverage()

    def test_finish_keeps_original_failure_and_final_fence_evidence(self):
        with self.assertRaises(SystemExit):
            gate.finish(ValueError('first failure preserved'))
        receipt = json.loads((gate.runner.evidence / 'gate-receipt.json').read_text())
        self.assertEqual(receipt['status'], 'failed')
        self.assertIn('first failure preserved', receipt['failure'])
        self.assertIn('prebuild source snapshot missing', receipt['final_fence_error'])

    def test_actual_cpu_and_consumer_source_census(self):
        cpu_census((ROOT / gate.FEATURE_PATHS[2]).read_text())
        for path, name in ((gate.FEATURE_PATHS[3], gate.GPU_TEST), (gate.FEATURE_PATHS[4], gate.EXPORT_TEST)):
            source = (ROOT / path).read_text()
            actual = re.findall(r'^#\[test\]\n(?:#\[ignore[^\n]*\]\n)?fn (\w+)\(', source, re.M)
            self.assertEqual(actual, [name])
            self.assertEqual(len(re.findall(r'^#\[ignore', source, re.M)), 1)
            self.assertNotIn('SKIP:', source)

    def test_workflow_pending_is_fail_closed_and_jobs_are_unique(self):
        with mock.patch.object(gate, 'WORKFLOW_JOB', None), self.assertRaisesRegex(AssertionError, 'pending'):
            gate.workflow_contract('jobs:\n')
        sample = 'jobs:\n  one:\n    steps: []\n\n  two:\n    steps: []\n'
        self.assertEqual(gate.workflow_job(sample, 'one'), '  one:\n    steps: []\n\n')
        with self.assertRaises(AssertionError):
            gate.workflow_job(sample + '  one:\n', 'one')
        with self.assertRaises(AssertionError):
            gate.workflow_job(sample, 'absent')

    def test_initialization_failure_retains_partial_receipt(self):
        destination = self.root / 'partial'
        with mock.patch.object(gate.runner, 'new_target', side_effect=RuntimeError('target allocation failed')), self.assertRaises(RuntimeError):
            gate.initialize(self.repo, destination, 'cpu')
        receipt = json.loads((destination / 'gate-receipt.json').read_text())
        self.assertEqual(receipt['status'], 'failed')
        self.assertIn('target allocation failed', receipt['initialization_error'])

    def test_feature_flags_and_cli_interface_are_frozen(self):
        self.assertEqual(gate.cargo_flags('orr_remote'), ['--locked', '--release', '-p', 'orr_remote', '--no-default-features'])
        source = (ROOT / gate.FEATURE_PATHS[0]).read_text()
        self.assertIn('#[cfg(not(feature = "room-project"))]', source)
        authoring = (ROOT / gate.FEATURE_PATHS[1]).read_text()
        for option in ('--project', '--guid', '--name', '--center', '--half-extents', '--output'):
            self.assertIn(option, authoring)
        self.assertIn('PreparedProject::open', authoring)
        self.assertIn('orr_remote::room_escape::document', authoring)
        self.assertIn('.apply(', authoring)

    def prepare_sources(self):
        for name in (*gate.FEATURE_PATHS[:5], *('assets/imported_scene_demo/' + name for name in ('orr.package.json', *gate.PACKAGE_FILES))):
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        for name in ('room-wall-cli', 'room_escape', 'orr_export_room', 'export-test'):
            path = gate.runner.evidence / name
            path.write_bytes(('synthetic pin fixture: ' + name).encode())
            gate.runner.report['producers'][name] = {'file': name, 'bytes': path.stat().st_size,
                                                     'sha256': gate.runner.digest(path)}

    @staticmethod
    def save(path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, indent=2) + '\n')

    def project_fixture(self, root, scene='baseline', off=False):
        root.mkdir()
        entry = {'game': 'room-escape-v1', 'scene': 'room.scene.yaml', 'models': 'room.models.json', 'camera': 'room.camera.json'}
        self.save(root / 'orr.project.json', {'schema': 2, 'engine': '*', 'entry': entry})
        (root / 'room.scene.yaml').write_text(scene)
        self.save(root / 'room.camera.json', {'camera': 'synthetic checker fixture'})
        object_root = root / '.orr/packages/objects' / gate.PACKAGE_DIGEST
        object_root.mkdir(parents=True)
        hashes = {}
        for name in ('orr.package.json', *gate.PACKAGE_FILES):
            path = object_root / name
            path.write_bytes((ROOT / 'assets/imported_scene_demo' / name).read_bytes())
            if name != 'orr.package.json':
                hashes[name] = gate.runner.digest(path)
        manifest = {'schema': 1, 'name': gate.PACKAGE_NAME, 'version': '1.0.0', 'engine': '^0.0.1',
                    'capabilities': ['models'], 'dependencies': {}, 'files': list(gate.PACKAGE_FILES)}
        self.save(root / 'orr.packages.lock.json', {'schema': 1, 'direct': {gate.PACKAGE_NAME: '1.0.0'},
                 'packages': {gate.PACKAGE_NAME: {'manifest': manifest, 'digest': gate.PACKAGE_DIGEST, 'files': hashes}}})
        binding = {'kind': 'static', 'package': gate.PACKAGE_NAME, 'asset': 'foreground.glb',
                   'package_digest': gate.PACKAGE_DIGEST, 'source_hash': hashes['foreground.glb'],
                   'transform': {'translation': [1000, 1000, 1000] if off else [0, 0, 0],
                                 'rotation': [0, 0, 0, 1], 'scale': [1, 1, 1]}}
        self.save(root / 'room.models.json', {'version': 2, 'scene': 'room.scene.yaml', 'project': '.',
                                             'bindings': {'e_00000000': binding, 'e_00000001': binding}})

    @staticmethod
    def operation(center, half):
        return {'op': 'spawn', 'guid': gate.WALL_GUID, 'name': 'authored wall', 'components': {
            'RoomEscapeV1::Actor': {'kind': 4, 'ordinal': 0},
            'orr_physics3d::Body': {'pos': [center[0], .5, center[1]], 'rot': {'kind': 'unit', 'x': 0, 'y': 0, 'z': 0, 'w': 1},
                'vel': [0, 0, 0], 'omega': [0, 0, 0], 'inv_mass': 0, 'inv_inertia': [0, 0, 0],
                'linear_damping': 0, 'angular_damping': 0, 'kind': 'static'},
            'orr_physics3d::Collider': {'shape': {'kind': 'box', 'half_extents': [half[0], 1, half[1]]},
                                      'restitution': 0, 'friction': .5, 'layer': 1, 'mask': 0}}}

    def provenance(self, test, names, fnv=False):
        function = gate.fnv1a64 if fnv else lambda data: hashlib.sha256(data).hexdigest()
        suffix = 'fnv1a64' if fnv else 'sha256'
        return {'source_' + suffix: {key: function((self.repo / name).read_bytes()) for key, name in
                (('test', test), ('cli', gate.FEATURE_PATHS[0]), ('authoring', gate.FEATURE_PATHS[1]))},
                'producer_' + suffix: {name: function((gate.runner.evidence / name).read_bytes()) for name in names}}

    @staticmethod
    def frame_fixture(tick=0, wall=(40960, 32768, 0), key=0, player_x=None):
        count = 8 if wall is not None else 7
        out = bytearray(b'ORRF' + struct.pack('<IQ', 2, tick))
        for section in gate.FRAME_SCHEMA:
            out.extend(struct.pack('<I', len(section)))
            for name, size in section:
                out.extend(struct.pack('<I', len(name)) + name.encode() + struct.pack('<I', size))
        out.extend(struct.pack('<II', count, count) + bytes(count * 4) + b'\x01' * count + struct.pack('<I', 0))
        kinds = [1, 2, 3] + [4] * (count - 3)
        walls = [(327680, 32768, 0), (-327680, 32768, 0), (0, 32768, 327680), (0, 32768, -327680)]
        if wall is not None:
            walls.append(wall)
        for name, size in gate.FRAME_SCHEMA[0]:
            out.extend(struct.pack('<I', count))
            for index in range(count):
                out.extend(struct.pack('<II', index, 0))
            for index, kind in enumerate(kinds):
                value = bytearray(size)
                if name.endswith('Body'):
                    x = (0 if tick == 0 else 5000 if wall == (40960, 32768, 0) else 4 * 65536) if player_x is None else player_x
                    center = walls[index - 3] if kind == 4 else ((x if kind == 1 else index * 65536), 32768, 0)
                    struct.pack_into('<3q', value, 0, *center)
                    struct.pack_into('<4q', value, 24, 0, 0, 0, 65536)
                elif name.endswith('Collider'):
                    struct.pack_into('<IIq3qqqIIII', value, 0, 2, 0, 0, 8192, 65536, 49152, 0, 32768, 1, 0, 0, 0)
                else:
                    struct.pack_into('<II', value, 0, kind, 0)
                out.extend(value)
        out.extend(bytes(16 + 120) + struct.pack('<II', 0, 1) + struct.pack('<4I', key, 0, int(key != 0), 0))
        out.extend(struct.pack('<II', 1, 1) + b'\x01' + struct.pack('<II', 0, 0))
        checksum = 1000 + count + tick + key + (0 if wall is None else abs(wall[0]))
        out.extend(struct.pack('<Q', checksum))
        return bytes(out), f'0x{checksum:016x}'

    def test_frame_layout_and_collision_los_controls_are_independent(self):
        for wall, tick, key in ((None, 0, 0), ((40960, 32768, 0), 0, 0),
                                ((40960, 32768, 0), 120, 0), (None, 120, 0),
                                (None, 1, 1), ((-131072, 32768, 0), 1, 1)):
            data, checksum = self.frame_fixture(tick, wall, key)
            gate.check_frame(data, tick, checksum, wall, key)
        data, checksum = self.frame_fixture()
        for changed in (b'XXXX' + data[4:], data[:-1], data + b'x', data[:16] + struct.pack('<I', 9999) + data[20:],
                        data[:-21] + struct.pack('<I', 0) + data[-17:]):
            with self.subTest(), self.assertRaises(AssertionError):
                gate.room_frame(changed)
        for tick, wall, key, x in ((120, (40960, 32768, 0), 0, 65536), (120, None, 0, 0)):
            data, checksum = self.frame_fixture(tick, wall, key, player_x=x)
            with self.subTest(), self.assertRaises(AssertionError):
                gate.check_frame(data, tick, checksum, wall, key)
        data, checksum = self.frame_fixture(1, None, 0)
        with self.assertRaises(AssertionError):
            gate.check_frame(data, 1, checksum, None, 1)
        for tick, key in ((True, 0), (1.0, 0), (1, False), (1, 0.0)):
            with self.subTest(tick=tick, key=key), self.assertRaises(AssertionError):
                gate.check_frame(data, tick, checksum, None, key)

    def test_package_model_closure_and_unrelated_binding_preservation(self):
        self.prepare_sources()
        before, after = self.root / 'before', self.root / 'after'
        self.project_fixture(before)
        self.project_fixture(after, 'authored')
        gate.compare_projects(before, after, {'room.scene.yaml'})
        for name, keys in (('orr.project.json', ('schema',)),
                           ('orr.packages.lock.json', ('schema',)),
                           ('orr.packages.lock.json', ('packages', gate.PACKAGE_NAME, 'manifest', 'schema')),
                           ('room.models.json', ('version',))):
            path = after / name
            original = path.read_bytes()
            changed = json.loads(original)
            target = changed
            for key in keys[:-1]:
                target = target[key]
            target[keys[-1]] = float(target[keys[-1]])
            self.save(path, changed)
            with self.subTest(name=name, keys=keys), self.assertRaises(AssertionError):
                gate.check_project(after)
            path.write_bytes(original)
        path = after / 'room.models.json'
        original = path.read_bytes()
        value = json.loads(original)
        value['bindings']['e_00000000']['transform']['translation'] = [1, 0, 0]
        self.save(path, value)
        with self.assertRaises(AssertionError):
            gate.compare_projects(before, after, {'room.scene.yaml'})
        path.write_bytes(original)
        (after / '.orr/packages/objects' / gate.PACKAGE_DIGEST / 'foreground.glb').write_bytes(b'wrong')
        with self.assertRaises(AssertionError):
            gate.check_project(after)

    def test_canonical_wall_operation_rejects_nonwall_rotation_and_filter(self):
        path = self.root / 'wall.ops.json'
        operation = self.operation([.625, 0], [.125, .75])
        self.save(path, [operation])
        gate.check_wall_operation(path, [.625, 0], [.125, .75])
        for component, field, value in (('RoomEscapeV1::Actor', 'kind', 1),
                                        ('orr_physics3d::Body', 'rot', {'kind': 'unit', 'x': 1, 'y': 0, 'z': 0, 'w': 0}),
                                        ('orr_physics3d::Collider', 'mask', 1)):
            changed = copy.deepcopy(operation)
            changed['components'][component][field] = value
            self.save(path, [changed])
            with self.subTest(), self.assertRaises(AssertionError):
                gate.check_wall_operation(path, [.625, 0], [.125, .75])
        for keys, value in (
                (('RoomEscapeV1::Actor', 'ordinal'), False),
                (('RoomEscapeV1::Actor', 'kind'), 4.0),
                (('RoomEscapeV1::Actor', 'ordinal'), 0.0),
                (('orr_physics3d::Collider', 'layer'), True),
                (('orr_physics3d::Collider', 'mask'), False),
                (('orr_physics3d::Collider', 'layer'), 1.0),
                (('orr_physics3d::Collider', 'mask'), 0.0),
                (('orr_physics3d::Collider', 'shape', 'half_extents', 1), True),
                (('orr_physics3d::Collider', 'restitution'), False),
                (('orr_physics3d::Body', 'rot', 'w'), True),
                (('orr_physics3d::Body', 'rot', 'x'), False),
                (('orr_physics3d::Body', 'pos', 2), False),
                (('orr_physics3d::Body', 'vel', 0), False),
                (('orr_physics3d::Body', 'omega', 1), False),
                (('orr_physics3d::Body', 'inv_inertia', 2), False),
                (('orr_physics3d::Body', 'inv_mass'), False),
                (('orr_physics3d::Body', 'linear_damping'), False),
                (('orr_physics3d::Body', 'angular_damping'), False)):
            changed = copy.deepcopy(operation)
            target = changed['components']
            for key in keys[:-1]:
                target = target[key]
            target[keys[-1]] = value
            self.save(path, [changed])
            with self.subTest(keys=keys, value=value), self.assertRaises(AssertionError):
                gate.check_wall_operation(path, [.625, 0], [.125, .75])

    @staticmethod
    def runtime_output(tick, checksum, key=0, initial=None):
        return (f'room initial checksum: {initial or checksum}\nroom tick: {tick} checksum: {checksum}\n'
                f'room key: {key} won: 0\nroom capture adapter: synthetic checker adapter\n')

    @staticmethod
    def probe_output(tick, checksum):
        return (f'running 1 test\ntest {gate.EXPORT_TEST} ... {gate.NAMESPACE_MARKER}\n'
                f'PreparedProject frame tick={tick} checksum={checksum}\nok\n'
                'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')

    def test_runtime_and_probe_require_actual_checksum_and_namespace_markers(self):
        # The strict result parser requires the named header before child stdout.
        def check_child_invocation(source):
            commands = re.findall(
                r'fn probe\([^{}]*\) -> String \{\s*'
                r'let mut c = self\.isolated\(&self\.probe, project, bundle\);\s*'
                r'c\.args\(\[([^\]]*)\]\)\s*'
                r'\.env\("ORR_ROOM_WALL_CHILD", "source-hidden-v1"\)', source, re.S)
            self.assertEqual(len(commands), 1)
            self.assertEqual([arg.strip() for arg in commands[0].split(',') if arg.strip()],
                             ['"--ignored"', '"--exact"', 'TEST', '"--nocapture"', '"--test-threads=1"'])
        source = (ROOT / gate.FEATURE_PATHS[4]).read_text()
        check_child_invocation(source)
        changed = source.replace('"--test-threads=1",', '', 1)
        self.assertNotEqual(changed, source)
        with self.assertRaises(AssertionError):
            check_child_invocation(changed)
        checksum = '0x0000000000000001'
        runtime = self.runtime_output(1, checksum)
        probe = self.probe_output(1, checksum)
        gate.check_runtime_output(runtime, 1, checksum, 0)
        gate.check_probe_output(probe, 1, checksum)
        for changed in (runtime.replace('room capture adapter:', 'missing adapter:'), runtime.replace('room key: 0', 'room key: 1'), runtime + runtime):
            with self.subTest(), self.assertRaises(AssertionError):
                gate.check_runtime_output(changed, 1, checksum, 0)
        for changed in (probe.replace(gate.NAMESPACE_MARKER, 'namespace omitted'),
                        probe.replace('0 filtered out', '1 filtered out'), probe.replace(checksum, '0x0000000000000002')):
            with self.subTest(), self.assertRaises(AssertionError):
                gate.check_probe_output(changed, 1, checksum)

    def test_full_cpu_retained_adoption_requires_real_layout_and_provenance(self):
        self.prepare_sources()
        captures = self.root / 'captures'
        adoption = captures / 'adoption'
        adoption.mkdir(parents=True)
        self.project_fixture(adoption / 'before-project')
        self.project_fixture(adoption / 'after-project', 'authored')
        for name, data in (('before.scene.yaml', b'baseline'), ('after.scene.yaml', b'authored'), ('reopened.scene.yaml', b'authored')):
            (adoption / name).write_bytes(data)
        (adoption / 'initial.frame').write_bytes(self.frame_fixture(wall=None)[0])
        (adoption / 'reopened.frame').write_bytes(self.frame_fixture()[0])
        operation = self.operation([.625, 0], [.125, .75])
        self.save(adoption / 'wall.ops.json', [operation])
        self.save(captures / 'real-cli.ops.json', [operation])
        (captures / 'real-cli.stdout').write_bytes(b'')
        (captures / 'real-cli-no-overwrite.stderr').write_text('output already exists')
        receipt = {'schema': 1, 'status': 'passed', **self.provenance(gate.FEATURE_PATHS[2], ['room-wall-cli'], fnv=True)}
        receipt['cli_rejections'] = {name: {'exit': 1, 'stdout': '', 'stderr': 'room_wall: invalid fixture option'}
                                     for name in ('malformed-number', 'duplicate-option', 'overlong-number', 'extra-arguments', 'missing-output')}
        self.save(captures / 'cpu-receipt.json', receipt)
        self.save(adoption / 'verified-report.json', {'ticks': 4, 'passed': True, 'first_divergence': 0, 'identical': False,
            'checksums': {'base_start': self.frame_fixture(wall=None)[1], 'candidate_start': self.frame_fixture()[1]},
            'metric_sampling': {'requested_interval': 1, 'sample_count': 5, 'scope': 'sampled_tick_boundaries', 'every_tick_boundary_observed': True},
            'checks': {'passed': True, 'results': [{'check': 'entities.delta == 1', 'passed': True}]},
            'verified_state': {'document_id': 'one', 'id': 'p1', 'document_revision': 0, 'proposal_revision': 1}})
        self.save(adoption / 'history.json', {'entries': [{'op_count': 1}]})
        gate.check_cpu(captures)
        report_path = adoption / 'verified-report.json'
        report = json.loads(report_path.read_text())
        for keys, value in (
                (('checksums', 'base_start'), '0x0000000000000000'),
                (('checksums', 'candidate_start'), '0x0000000000000000'),
                (('checksums', 'base_start'), False),
                (('ticks',), 4.0), (('first_divergence',), False),
                (('passed',), 1), (('identical',), 0),
                (('metric_sampling', 'requested_interval'), True),
                (('metric_sampling', 'sample_count'), 5.0),
                (('metric_sampling', 'every_tick_boundary_observed'), 1),
                (('verified_state', 'document_revision'), False),
                (('verified_state', 'proposal_revision'), True),
                (('verified_state', 'proposal_revision'), 1.0)):
            changed = copy.deepcopy(report)
            target = changed
            for key in keys[:-1]:
                target = target[key]
            target[keys[-1]] = value
            self.save(report_path, changed)
            with self.subTest(keys=keys, value=value), self.assertRaises(AssertionError):
                gate.check_cpu(captures)
        self.save(report_path, report)
        for value in (True, 1.0):
            self.save(adoption / 'history.json', {'entries': [{'op_count': value}]})
            with self.subTest(op_count=value), self.assertRaises(AssertionError):
                gate.check_cpu(captures)
        self.save(adoption / 'history.json', {'entries': [{'op_count': 1}]})
        for value in (True, 1.0):
            changed = dict(receipt, schema=value)
            self.save(captures / 'cpu-receipt.json', changed)
            with self.subTest(schema=value), self.assertRaises(AssertionError):
                gate.check_cpu(captures)
        self.save(captures / 'cpu-receipt.json', receipt)
        (adoption / 'reopened.scene.yaml').write_bytes(b'baseline')
        with self.assertRaises(AssertionError):
            gate.check_cpu(captures)

    def test_full_gpu_retained_oracle_rejects_opaque_receipt_claims(self):
        self.prepare_sources()
        captures = self.root / 'captures'
        captures.mkdir()
        for name, scene, off in (('baseline-project', 'baseline', False), ('authored-project', 'authored', False),
                                  ('wrong-position-project', 'wrong', False), ('models-off-project', 'baseline', True)):
            self.project_fixture(captures / name, scene, off)
        adapter = {'name': 'synthetic checker adapter', 'backend': 'Vulkan', 'software': True}
        self.save(captures / 'adapter.json', adapter)
        frames = self.frames()
        refs = {}
        for name, raw in frames.items():
            ppm = b'P6\n512 512\n255\n' + bytes(value for index, value in enumerate(raw) if index % 4 != 3)
            refs[name.replace('-', '_')] = {}
            for key, suffix, data in (('raw', '.rgba', raw), ('image', '.ppm', ppm)):
                path = captures / (name + suffix)
                path.write_bytes(data)
                refs[name.replace('-', '_')][key] = {'file': path.name, 'bytes': len(data), 'fnv1a64': gate.fnv1a64(data)}
        self.save(captures / 'wall.ops.json', [self.operation([2.5, -2], [.5, .75])])
        self.save(captures / 'wrong-position.ops.json', [self.operation([2.5, 2], [.5, .75])])
        receipt = {'schema': 1, 'status': 'passed', 'width': 512, 'height': 512, 'adapter': adapter, 'frames': refs,
                   'wall_rect': [384, 80, 448, 176], 'interior_rect': [386, 82, 446, 174],
                   'exterior_exclusion_rect': [382, 78, 450, 178], 'wrong_position_rect': [384, 336, 448, 432],
                   'interior_pixels': 5520, 'unchanged_model_pixels': 40, 'wrong_position_pixels': 5520,
                   **self.provenance(gate.FEATURE_PATHS[3], ['room-wall-cli'], fnv=True)}
        self.save(captures / 'receipt.json', receipt)
        gate.check_gpu(captures)
        for key in ('schema', 'width', 'height', 'interior_pixels', 'unchanged_model_pixels', 'wrong_position_pixels'):
            changed = copy.deepcopy(receipt)
            changed[key] = float(changed[key])
            self.save(captures / 'receipt.json', changed)
            with self.subTest(key=key), self.assertRaises(AssertionError):
                gate.check_gpu(captures)
        for key in ('wall_rect', 'interior_rect', 'exterior_exclusion_rect', 'wrong_position_rect'):
            changed = copy.deepcopy(receipt)
            changed[key][0] = float(changed[key][0])
            self.save(captures / 'receipt.json', changed)
            with self.subTest(rectangle=key), self.assertRaises(AssertionError):
                gate.check_gpu(captures)
        changed = copy.deepcopy(receipt)
        changed['adapter']['software'] = 1
        self.save(captures / 'receipt.json', changed)
        with self.assertRaises(AssertionError):
            gate.check_gpu(captures)
        receipt['adapter']['software'] = False
        self.save(captures / 'receipt.json', receipt)
        self.save(captures / 'adapter.json', receipt['adapter'])
        with self.assertRaises(AssertionError):
            gate.check_gpu(captures)

    def test_pinned_parallel_workflow_rejects_weakened_rows_and_retention(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        if '  room-wall-authoring:\n' not in text:
            text = text.replace('  cross_platform_checksum_compare:\n', gate.WORKFLOW_JOB + '  cross_platform_checksum_compare:\n')
            text = text.replace('sample-room, static-model-authoring]', 'sample-room, static-model-authoring, room-wall-authoring]')
            previous = gate.workflow_job(text, 'cross_platform_checksum_compare')
            text = text.replace(previous, previous + gate.WORKFLOW_AGGREGATE_STEP)
        gate.workflow_contract(text)
        for removed in (', generated-user-game-native',
                        "          GENERATED_USER_NATIVE_RESULT: ${{ needs['generated-user-game-native'].result }}\n",
                        ' GENERATED_USER_NATIVE_RESULT PRIVATE_ACTION_NATIVE_RESULT; do',
                        ', static-game-actions-native',
                        "          PRIVATE_ACTION_NATIVE_RESULT: ${{ needs['static-game-actions-native'].result }}\n",
                        ' PRIVATE_ACTION_NATIVE_RESULT; do'):
            replacement = '; do' if removed.endswith('; do') else ''
            self.assertIn(removed, text)
            with self.subTest(removed=removed), self.assertRaises(AssertionError):
                gate.workflow_contract(text.replace(removed, replacement, 1))
        for original, changed in (('if-no-files-found: error', 'if-no-files-found: warn'),
                                   ('fail-fast: false', 'fail-fast: true'),
                                   ('unset PYTHONOPTIMIZE', 'echo skipped'),
                                   ('python -B tools/room_wall_gate.py', 'python -O tools/room_wall_gate.py')):
            mutated = gate.WORKFLOW_JOB.replace(original, changed)
            self.assertNotEqual(mutated, gate.WORKFLOW_JOB)
            with self.subTest(original=original), self.assertRaises(AssertionError):
                gate.workflow_contract(text.replace(gate.WORKFLOW_JOB, mutated))


    @staticmethod
    def write_png(path, color):
        def chunk(kind, data):
            return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
        row = b'\0' + bytes([color, 20, 40, 255]) * 1024
        raw = row * 768
        path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1024, 768, 8, 6, 0, 0, 0))
                         + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))

    def test_full_export_requires_frame_controls_namespace_and_manifest_identity(self):
        self.prepare_sources()
        captures = self.root / 'captures'
        captures.mkdir()
        for name, scene in (('baseline-project', 'baseline'), ('authored-project', 'authored'), ('wrong-position-project', 'wrong')):
            self.project_fixture(captures / name, scene)
        self.save(captures / 'wall.ops.json', [self.operation([.625, 0], [.125, .75])])
        self.save(captures / 'wrong-position.ops.json', [self.operation([-2, 0], [.125, .75])])
        observations, controls = {}, {}
        initial_checksum = self.frame_fixture()[1]
        for name, ticks, held in gate.EXPORT_STAGES:
            data, checksum = self.frame_fixture(ticks)
            item = {'ticks': ticks, 'held': held, 'checksum': checksum}
            for route in ('source', 'export'):
                frame = captures / f'prepared-project-{name}-{route}.frame'
                png = captures / f'runtime-{name}-{route}.png'
                frame.write_bytes(data)
                self.write_png(png, 20 if name in ('initial', 'restarted') else 40)
                item.update({route + '_stdout': self.runtime_output(ticks, checksum, initial=initial_checksum),
                             route + '_probe_stdout': self.probe_output(ticks, checksum),
                             route + '_frame_sha256': gate.runner.digest(frame), route + '_png_sha256': gate.runner.digest(png)})
            observations[name] = item
        for control in ('baseline', 'wrong-position'):
            for stage, ticks, held in gate.CONTROL_STAGES:
                key = int(stage == 'interacted')
                data, checksum = self.frame_fixture(ticks, wall=(-131072, 32768, 0) if control == 'wrong-position' else None, key=key)
                frame = captures / f'control-{control}-{stage}.frame'
                png = captures / f'control-{control}-{stage}.png'
                frame.write_bytes(data)
                self.write_png(png, 60)
                controls[control + '-' + stage] = {'ticks': ticks, 'held': held, 'checksum': checksum,
                    'stdout': self.runtime_output(ticks, checksum, key), 'probe_stdout': self.probe_output(ticks, checksum),
                    'frame_sha256': gate.runner.digest(frame), 'png_sha256': gate.runner.digest(png)}
        negative = {'capture-no-overwrite': 'already exists', 'runtime-read-only': 'Read-only file system',
                    'exporter-read-only': 'Read-only file system', 'export-no-overwrite': 'output destination already exists'}
        self.save(captures / 'negative-controls.json', {name: {'exit': 1, 'stdout': '', 'stderr': error} for name, error in negative.items()})
        exported = {'project/' + name: [path.stat().st_size, gate.runner.digest(path)]
                    for name in gate.project_files() for path in [captures / 'authored-project' / name]}
        runtime = gate.runner.report['producers']['room_escape']
        exported.update({'bin/room_escape': [runtime['bytes'], runtime['sha256']],
                         'run-room-escape': [10, 'a' * 64]})
        payload = {'exporter_version': '0.0.1', 'profile': 'room-escape-authored-linux-x86_64-v1',
                   'entry': {'game': 'room-escape-v1', 'scene': 'room.scene.yaml', 'models': 'room.models.json', 'camera': 'room.camera.json'},
                   'packages': {gate.PACKAGE_NAME: gate.PACKAGE_DIGEST},
                   'files': [{'path': name, 'role': 'fixture', 'mode': 0o644 if name.startswith('project/') else 0o755,
                              'bytes': value[0], 'sha256': value[1]} for name, value in sorted(exported.items())],
                   'runtime': {'sha256': runtime['sha256'], 'bytes': runtime['bytes']},
                   'declared': {'target': 'linux-x86_64', 'source_revision': None}, 'initial_checksum': initial_checksum}
        content = b'orrery.room-escape.export.content.v1\0' + json.dumps(payload, separators=(',', ':'), ensure_ascii=False).encode()
        manifest = {'schema': 1, 'payload': payload, 'content_digest': hashlib.sha256(content).hexdigest()}
        self.save(captures / 'orr.export.json', manifest)
        exported['orr.export.json'] = [(captures / 'orr.export.json').stat().st_size, gate.runner.digest(captures / 'orr.export.json')]
        receipt = {'schema': 1, 'status': 'passed', **self.provenance(gate.FEATURE_PATHS[4], ['room-wall-cli', 'room_escape', 'orr_export_room', 'export-test']),
                   'observations': observations, 'controls': controls, 'exported_file_inventory': exported,
                   'frame_evidence_boundary': 'synthetic fixture tests validator semantics only'}
        self.save(captures / 'production-receipt.json', receipt)
        def refresh():
            entries = {path.relative_to(captures).as_posix(): [path.stat().st_size, gate.runner.digest(path)]
                       for path in captures.rglob('*') if path.is_file() and path.name != 'capture-inventory.json'}
            self.save(captures / 'capture-inventory.json', entries)
        refresh()
        gate.check_export(captures)
        for section, stage, value in (('observations', 'initial', False), ('observations', 'moved', True),
                                       ('observations', 'collision', 120.0), ('controls', 'baseline-initial', False)):
            changed = copy.deepcopy(receipt)
            changed[section][stage]['ticks'] = value
            self.save(captures / 'production-receipt.json', changed)
            refresh()
            with self.subTest(section=section, stage=stage), self.assertRaises(AssertionError):
                gate.check_export(captures)
        self.save(captures / 'production-receipt.json', receipt)
        refresh()
        for value in (True, 1.0):
            changed = dict(manifest, schema=value)
            self.save(captures / 'orr.export.json', changed)
            with self.subTest(manifest_schema=value), self.assertRaises(AssertionError):
                gate.check_export_manifest(captures, receipt)
        self.save(captures / 'orr.export.json', manifest)
        for value in (True, 1.0):
            with self.subTest(inventory_bytes=value), self.assertRaises(AssertionError):
                gate.inventory_numbers({'one-byte-file': [value, 'a' * 64]})
        gate.inventory_numbers({'one-byte-file': [1, 'a' * 64]})
        refresh()
        receipt['observations']['initial']['source_probe_stdout'] = receipt['observations']['initial']['source_probe_stdout'].replace(gate.NAMESPACE_MARKER, 'namespace bypassed')
        self.save(captures / 'production-receipt.json', receipt)
        refresh()
        with self.assertRaises(AssertionError):
            gate.check_export(captures)
        receipt['observations']['initial']['source_probe_stdout'] = self.probe_output(0, initial_checksum)
        self.save(captures / 'production-receipt.json', receipt)
        manifest['payload']['runtime']['sha256'] = '0' * 64
        self.save(captures / 'orr.export.json', manifest)
        with self.assertRaises(AssertionError):
            gate.check_export_manifest(captures, receipt)



class RoomWallWorkflowTests(unittest.TestCase):
    def test_current_workflow_mandatory_parallel_rows_and_retention(self):
        gate.workflow_contract((ROOT / '.github/workflows/determinism.yml').read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--semantic-only', action='store_true')
    parser.add_argument('--workflow-only', action='store_true')
    args = parser.parse_args()
    if args.semantic_only and args.workflow_only:
        parser.error('choose one fixture subset')
    classes = ([RoomWallSemanticTests] if args.semantic_only else
               [RoomWallWorkflowTests] if args.workflow_only else
               [RoomWallSemanticTests, RoomWallWorkflowTests])
    suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromTestCase(cls) for cls in classes)
    actual_names = sorted(name for name in unittest.defaultTestLoader.getTestCaseNames(RoomWallSemanticTests))
    if actual_names != list(EXPECTED_SEMANTIC_TESTS):
        raise SystemExit('Room wall semantic fixture names changed')
    count = suite.countTestCases()
    expected = (gate.SEMANTIC_FIXTURE_COUNT if args.semantic_only else
                gate.FIXTURE_COUNT - gate.SEMANTIC_FIXTURE_COUNT if args.workflow_only else gate.FIXTURE_COUNT)
    if count != expected or not count:
        raise SystemExit('Room wall fixture census changed')
    result = unittest.TextTestRunner(verbosity=2, failfast=True).run(suite)
    if (result.testsRun != count or result.skipped or result.expectedFailures
            or result.unexpectedSuccesses or not result.wasSuccessful()):
        raise SystemExit(1)
    marker = ('ROOM_WALL_GATE_SEMANTIC_ONLY_FIXTURES' if args.semantic_only else
              'ROOM_WALL_GATE_WORKFLOW_ONLY_FIXTURES' if args.workflow_only else 'ROOM_WALL_GATE_FIXTURES')
    print(f'{marker}: {count} passed; 0 failed; 0 skipped', flush=True)


if __name__ == '__main__':
    main()
