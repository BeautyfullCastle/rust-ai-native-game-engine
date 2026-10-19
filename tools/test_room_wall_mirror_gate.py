#!/usr/bin/env python3
"""Independent adversarial mirror-gate fixtures; never engine/UI execution proof."""
import ast
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import sys
import tempfile
import types
import unittest
from unittest import mock
import zlib

import room_wall_mirror_gate as gate

ROOT = Path(__file__).resolve().parents[1]
EXPECTED_TESTS = ('test_all_commands_use_bounded_runner_between_source_fences', 'test_bad_explicit_vulkan_configuration_never_falls_back', 'test_capture_census_rejects_extra_missing_unsafe_or_duplicate_entries', 'test_configured_local_lavapipe_is_supported_and_recorded', 'test_exact_cpu_source_census_is_frozen', 'test_exact_inventory_rejects_missing_duplicate_extra_and_diagnostics', 'test_executed_accepts_only_known_filtered_census_and_crlf', 'test_execution_rejects_skip_missing_failed_duplicate_and_wrong_names', 'test_final_fence_failure_preserves_original_failure', 'test_fixture_marker_cannot_launder_skipped_failed_or_missing_cases', 'test_fresh_private_evidence_and_target', 'test_gameplay_rejects_wrong_controls_vacuity_nonfinite_and_boolean_numbers', 'test_geometry_and_mirror_order_oracles_are_independent_literals', 'test_gpu_source_census_and_fresh_evidence_are_explicit', 'test_mode_coverage_never_certifies_unrun_windows_ui_or_merge', 'test_png_decoder_rejects_truncation_crc_and_wrong_dimensions', 'test_python_guard_precedes_assertion_helpers', 'test_receipt_requires_all_actual_ui_and_windows_limitation_fields', 'test_retained_producer_binds_current_source_and_artifact_bytes', 'test_retained_producer_rejects_stale_wrong_features_manifest_and_profile', 'test_retained_project_and_hidden_source_exact_bytes_are_required', 'test_source_fences_paths_bytes_and_modes', 'test_source_inventory_includes_ignored_and_ignores_worktree_metadata', 'test_source_rejects_symlink_files_and_directories', 'test_strict_json_rejects_duplicates_nonfinite_and_nonregular', 'test_test_runner_cannot_shrink_selected_or_ignored_census', 'test_viewport_regions_require_real_composed_pixels_and_exact_bounds', 'test_workflow_and_all_prior_regressions_are_pinned', 'test_workflow_weakened_matrix_skip_or_artifact_retention_rejected')


def listing(names):
    return ''.join(name + ': test\n' for name in names) + f'\n{len(names)} tests, 0 benchmarks\n'


def passed(names, filtered=0):
    return (f'running {len(names)} test' + ('' if len(names) == 1 else 's') + '\n'
            + ''.join('test ' + name + ' ... ok\n' for name in names)
            + f'\ntest result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; {filtered} filtered out; finished in 0.01s\n')


def receipt():
    value = {'schema': 1, 'platform': 'linux-x86_64', 'physical_os_input': False,
             'windows_hardware_ui_tested': False, 'adapter': 'llvmpipe fixture only',
             'scene_checksum': '0x0000000000000001',
             'test_source_sha256': hashlib.sha256((ROOT / 'crates/orr_editor/tests/room_wall_stamp_mirror_ui.rs').read_bytes()).hexdigest(),
             'renderer_options': 'native egui_wgpu::RendererOptions::default',
             'pixel_source': 'full composed EditorApp framebuffer; exact interior viewport ROI 890x797',
             'wrong_axis_and_order_equivalence': 'R90 Mz = Mx R90 = R270 Mx about the same pivot, offset last',
             'mirrored_centers': [[['2', '6'], ['0', '7.5'], ['3', '9']],
                                  [['2', '0'], ['0.5', '-2'], ['-1', '1']],
                                  [['8', '0'], ['10', '-1.5'], ['7', '-3']],
                                  [['8', '6'], ['9.5', '8'], ['11', '5']]],
             'unchecked_90_centers': [['2', '6'], ['0.5', '8'], ['-1', '5']]}
    value.update(dict.fromkeys(gate.TRUE_FIELDS, True))
    value['gameplay_controls'] = [
        {'mirror_x': mirror, 'degrees': degrees, 'start': [0, .5, 0],
         'end': [1 if mirror and degrees == 90 else 3, .5, 0],
         'blocked': mirror and degrees == 90,
         'wrong_axis_and_wrong_order_control': mirror and degrees == 270,
         'unchecked_compatibility_control': not mirror}
        for mirror, degrees in [(False, 90), (True, 0), (True, 180), (True, 270), (True, 90)]]
    value['reopened_gameplay'] = {'start': [0, .5, 0], 'end': [1, .5, 0]}
    return value


def png(width, height, pixels):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + pixels[y * width * 4:(y + 1) * width * 4] for y in range(height))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


class MirrorGateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.repo = self.root / 'checkout'
        self.repo.mkdir()
        environ = mock.patch.dict(os.environ, {}, clear=False)
        environ.start()
        self.addCleanup(environ.stop)
        gate.initialize(self.repo, self.root / 'evidence', 'cpu')

    def test_python_guard_precedes_assertion_helpers(self):
        tree = ast.parse((ROOT / 'tools/room_wall_mirror_gate.py').read_text())
        guard = next(i for i, node in enumerate(tree.body) if isinstance(node, ast.Expr)
                     and isinstance(node.value, ast.Call) and getattr(node.value.func, 'id', '') == 'require_python')
        imports = [i for i, node in enumerate(tree.body) if isinstance(node, ast.Import)
                   and any(item.name == 'static_model_extract_gate' for item in node.names)]
        self.assertLess(guard, imports[0])
        for value in ('', '0', '1'):
            with self.subTest(value=value), mock.patch.dict(os.environ, PYTHONOPTIMIZE=value), mock.patch('sys.stderr'), self.assertRaises(SystemExit):
                gate.require_python()
        with mock.patch.object(sys, 'flags', types.SimpleNamespace(optimize=1)), mock.patch('sys.stderr'), self.assertRaises(SystemExit):
            gate.require_python()
        with mock.patch.object(sys, 'dont_write_bytecode', False), mock.patch('sys.stderr'), self.assertRaises(SystemExit):
            gate.require_python()

    def test_fresh_private_evidence_and_target(self):
        self.assertTrue(gate.runner.report['private_target']['fresh_empty'])
        self.assertEqual(list(gate.runner.target_dir.iterdir()), [])
        self.assertFalse(gate.runner.evidence.is_relative_to(self.repo))
        self.assertEqual(os.environ['TMPDIR'], str(gate.runner.evidence / 'temporary'))
        for destination in (Path('relative'), self.repo / 'evidence', self.root / 'evidence', self.root):
            with self.subTest(destination=destination), self.assertRaises(AssertionError):
                gate.initialize(self.repo, destination, 'cpu')

    def test_source_inventory_includes_ignored_and_ignores_worktree_metadata(self):
        (self.repo / '.git').write_text('gitdir: fixture metadata')
        (self.repo / 'file.rs').write_text('source')
        gate.check_inventory(['file.rs'], 'initial')
        with self.assertRaises(AssertionError):
            gate.check_inventory(['file.rs', 'file.rs'], 'duplicate')
        ignored = self.repo / 'tools/__pycache__/injected.pyc'
        ignored.parent.mkdir(parents=True)
        ignored.write_bytes(b'bytecode')
        with self.assertRaises(AssertionError):
            gate.check_inventory(['file.rs'], 'ignored')
        self.assertEqual(gate.runner.report['source_path_inventories']['ignored']['extra'], ['tools/__pycache__/injected.pyc'])

    def test_source_fences_paths_bytes_and_modes(self):
        path = self.repo / 'source.rs'
        path.write_text('source')
        gate.frozen = gate.snapshot()
        gate.source_fence()
        path.write_text('changed')
        with self.assertRaises(AssertionError):
            gate.source_fence()
        path.write_text('source')
        changed = copy.deepcopy(gate.frozen)
        changed['source.rs']['mode'] = 0
        with mock.patch.object(gate, 'snapshot', return_value=changed), self.assertRaises(AssertionError):
            gate.source_fence()
        (self.repo / 'extra.rs').write_text('new')
        with self.assertRaises(AssertionError):
            gate.source_fence()

    def test_source_rejects_symlink_files_and_directories(self):
        (self.repo / 'file.rs').write_text('source')
        with mock.patch.object(Path, 'is_symlink', return_value=True), self.assertRaises(AssertionError):
            gate.snapshot()
        (self.repo / 'subdir').mkdir()
        original = Path.is_symlink
        with mock.patch.object(Path, 'is_symlink', lambda path: path.name == 'subdir' or original(path)), self.assertRaises(AssertionError):
            gate.snapshot()

    def test_all_commands_use_bounded_runner_between_source_fences(self):
        order = []
        with mock.patch.object(gate, 'source_fence', side_effect=lambda: order.append('fence')), mock.patch.object(gate.runner, 'run', side_effect=lambda *args, **kwargs: order.append('run') or 'value'):
            self.assertEqual(gate.run('fixture', ['not-executed']), 'value')
        self.assertEqual(order, ['fence', 'run', 'fence'])
        tree = ast.parse((ROOT / 'tools/room_wall_mirror_gate.py').read_text())
        self.assertFalse(any(isinstance(node, ast.Import) and any(item.name == 'subprocess' for item in node.names) for node in ast.walk(tree)))

    def test_exact_cpu_source_census_is_frozen(self):
        source = (ROOT / 'crates/orr_editor/src/room_wall_stamp/placement/mirror_tests.rs').read_text()
        actual = re.findall(r'^#\[test\]\nfn ([A-Za-z0-9_]+)\(', source, re.M)
        self.assertEqual(sorted(gate.CPU_PREFIX + name for name in actual), sorted(gate.CPU_TESTS))
        self.assertEqual(len(actual), 16)
        self.assertNotIn('#[ignore', source)
        module = (ROOT / 'crates/orr_editor/src/room_wall_stamp.rs').read_text()
        self.assertIn('mod mirror_tests;', module)
        self.assertNotIn('mirror_tests', (ROOT / 'crates/orr_editor/src/room_wall_stamp/placement.rs').read_text())

    def test_gpu_source_census_and_fresh_evidence_are_explicit(self):
        source = (ROOT / 'crates/orr_editor/tests/room_wall_stamp_mirror_ui.rs').read_text()
        actual = re.findall(r'^#\[test\]\n#\[ignore = "requires software Vulkan and fresh ORR_ROOM_WALL_MIRROR_LINUX_DIR"\]\nfn ([A-Za-z0-9_]+)\(', source, re.M)
        self.assertEqual(actual, [gate.GPU_TEST])
        self.assertIn('evidence.is_absolute() && !evidence.exists()', source)
        self.assertEqual(len(gate.CAPTURES), 17)
        self.assertEqual(list(gate.CAPTURES), sorted(set(gate.CAPTURES)))

    def test_workflow_and_all_prior_regressions_are_pinned(self):
        with mock.patch.object(gate, 'source_root', ROOT):
            gate.workflow_contract()
        # runner is available in step env/with, but not job env. Keep both
        # evidence producers and always-run uploads on the identical path.
        for filename, variable, step in (
                ('room-wall-mirror', 'ORR_ROOM_WALL_MIRROR_EVIDENCE',
                 'Require current-source mirror acceptance and preserved regressions'),
                ('png-sequence-clips', 'ORR_PNG_SEQUENCE_CLIPS_EVIDENCE',
                 'Run current-source fail-closed acceptance')):
            path = ('${{ runner.temp }}/' + filename
                    + '-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}')
            header = '      - name: ' + step + '\n'
            step_env = '        env:\n          ' + variable + ': ' + path + '\n'
            artifact_path = '          path: ' + path + '\n'

            def evidence_wiring(text):
                for job_env in re.findall(r'(?m)^    env:\n((?:^ {6}[^\n]*\n)*)', text):
                    self.assertNotRegex(job_env, r'\$\{\{\s*runner(?:\.|\[)')
                self.assertIn(header + step_env, text)
                self.assertIn(artifact_path, text)

            original = (ROOT / '.github/workflows' / (filename + '.yml')).read_text()
            with self.subTest(workflow=filename):
                evidence_wiring(original)
                illegal_job_env = original.replace(
                    '    steps:\n', '      ' + variable + ': ' + path + '\n    steps:\n', 1)
                with self.assertRaises(AssertionError):
                    evidence_wiring(illegal_job_env)
                with self.assertRaises(AssertionError):
                    evidence_wiring(original.replace(step_env, ''))
                with self.assertRaises(AssertionError):
                    evidence_wiring(original.replace(artifact_path,
                        '          path: ${{ env.' + variable + ' }}\n'))
        for required in ('tools/check-room-wall-group-rotation.py',
                         'crates/orr_editor/src/room_wall_stamp_store/linux_tests.rs',
                         'crates/orr_editor/tests/room_wall_stamp_ui.rs'):
            self.assertIn(required, gate.PRESERVED_SHA256)
        source = (ROOT / 'tools/room_wall_mirror_gate.py').read_text()
        self.assertIn("run('retained-rotation-cpu'", source)

    def test_workflow_weakened_matrix_skip_or_artifact_retention_rejected(self):
        for name in gate.PRESERVED_SHA256:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / name).read_bytes())
        workflow = self.repo / '.github/workflows/room-wall-mirror.yml'
        original = (ROOT / '.github/workflows/room-wall-mirror.yml').read_text()
        workflow.write_text(original)
        gate.workflow_contract()
        for before, after in [('os: windows-latest', 'os: ubuntu-22.04'),
                              ('mode: gpu', 'mode: cpu'), ('if: always()', 'if: success()'),
                              ('if-no-files-found: error', 'if-no-files-found: warn'),
                              ('python -B tools/room_wall_mirror_gate.py', 'echo skipped')]:
            workflow.write_text(original.replace(before, after))
            with self.subTest(before=before), self.assertRaises(AssertionError):
                gate.workflow_contract()
        workflow.write_text(original)
        old = self.repo / 'tools/check-room-wall-group-rotation.py'
        old.write_text(old.read_text().replace('17)', '16)'))
        with self.assertRaises(AssertionError):
            gate.workflow_contract()

    def test_exact_inventory_rejects_missing_duplicate_extra_and_diagnostics(self):
        self.assertEqual(gate.inventory(listing(['a', 'b'])), ['a', 'b'])
        for text in (listing(['a', 'a']), listing(['a']).replace('1 tests', '2 tests'),
                     listing(['a']) + listing(['a']), listing(['a']) + 'ignored\n',
                     listing(['a']).replace('\n', '\r')):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                gate.inventory(text)

    def test_executed_accepts_only_known_filtered_census_and_crlf(self):
        names = ['a', 'b']
        gate.executed(passed(names), names)
        gate.executed(passed(names, 18).replace('\n', '\r\n'), names, 18)
        with self.assertRaises(AssertionError):
            gate.executed(passed(names, 19), names, 18)
        with self.assertRaises(AssertionError):
            gate.executed(passed([]), [])

    def test_execution_rejects_skip_missing_failed_duplicate_and_wrong_names(self):
        base = passed(['one'])
        for text in ('', base * 2, base.replace('... ok', '... ignored'),
                     base.replace('1 passed', '0 passed'), base.replace('0 failed', '1 failed'),
                     base.replace('0 ignored', '1 ignored'), base.replace('one', 'other'),
                     base.replace('running 1', 'running 0'), base + 'extra\n'):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                gate.executed(text, ['one'])

    def test_test_runner_cannot_shrink_selected_or_ignored_census(self):
        names = ['suite::one', 'suite::two']
        for responses in ([listing(names[:1]), listing([])],
                          [listing(names), listing(names[:1])]):
            with mock.patch.object(gate, 'run', side_effect=responses), self.assertRaises(AssertionError):
                gate.run_tests('fixture', 'binary', names, prefix='suite::')
        with mock.patch.object(gate, 'run', side_effect=[listing(names + ['other']), listing([]), passed(names, 1)]):
            gate.run_tests('fixture', 'binary', names, prefix='suite::')

    def producer(self):
        crate = self.repo / 'crates/orr_editor'
        crate.mkdir(parents=True, exist_ok=True)
        (crate / 'Cargo.toml').write_text('fixture')
        binary = gate.runner.target_dir / 'debug/deps/orr_editor-test'
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b'not executable; pin fixture only')
        gate.runner.report.update(head='a' * 40, tree='b' * 40, source_inventory_sha256='c' * 64)
        return {'reason': 'compiler-artifact', 'fresh': False,
                'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_editor@0.0.1',
                'target': {'name': 'orr_editor', 'src_path': str(crate / 'src/lib.rs'),
                           'kind': ['lib'], 'crate_types': ['lib']},
                'profile': {'test': True, 'opt_level': '0', 'debug_assertions': True},
                'features': list(gate.FEATURES['Linux'][1]), 'executable': str(binary)}

    def test_retained_producer_binds_current_source_and_artifact_bytes(self):
        producer = self.producer()
        retained = gate.pin(json.dumps(producer), 'unit', 'orr_editor', 'lib', gate.FEATURES['Linux'][1])
        self.assertEqual(Path(retained).read_bytes(), Path(producer['executable']).read_bytes())
        item = gate.runner.report['producers']['unit']
        self.assertEqual(item['source_head'], 'a' * 40)
        self.assertEqual(item['source_tree'], 'b' * 40)
        self.assertEqual(item['source_inventory_sha256'], 'c' * 64)
        Path(retained).write_bytes(b'changed')
        with self.assertRaises(AssertionError):
            gate.runner.fence()

    def test_retained_producer_rejects_stale_wrong_features_manifest_and_profile(self):
        original = self.producer()
        for key, value in [('fresh', True), ('features', ['room-authoring']),
                           ('manifest_path', str(self.repo / 'Cargo.toml')),
                           ('package_id', 'registry+https://example.invalid#orr_editor'),
                           ('profile', {'test': True, 'opt_level': '3', 'debug_assertions': True}),
                           ('profile', {'test': True, 'opt_level': '0', 'debug_assertions': False})]:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                gate.pin(json.dumps(dict(original, **{key: value})), 'invalid', 'orr_editor', 'lib', gate.FEATURES['Linux'][1])
        with self.assertRaises(AssertionError):
            gate.pin(json.dumps(original) + '\n' + json.dumps(original), 'duplicate', 'orr_editor', 'lib', gate.FEATURES['Linux'][1])

    def test_configured_local_lavapipe_is_supported_and_recorded(self):
        manifest = self.root / 'local-icd.json'
        manifest.write_text(json.dumps({'ICD': {'library_path': '/custom/libvulkan_lvp.so'}}))
        with mock.patch.dict(os.environ, {'VK_DRIVER_FILES': str(manifest), 'VK_ICD_FILENAMES': str(manifest)}), mock.patch.object(gate.platform, 'system', return_value='Linux'), mock.patch.object(gate.platform, 'machine', return_value='x86_64'):
            self.assertEqual(gate.configure_software(), manifest)
            self.assertEqual(os.environ['VK_ICD_FILENAMES'], str(manifest))
            self.assertEqual(gate.runner.report['icd_sha256'], hashlib.sha256(manifest.read_bytes()).hexdigest())
            self.assertEqual(os.environ['WGPU_BACKEND'], 'vulkan')

    def test_bad_explicit_vulkan_configuration_never_falls_back(self):
        manifest = self.root / 'wrong.json'
        manifest.write_text(json.dumps({'ICD': {'library_path': 'libother_vendor.so'}}))
        cases = [('', ''), ('relative.json', 'relative.json'),
                 (str(manifest), str(manifest) + '.other'),
                 (str(manifest) + os.pathsep + str(manifest), str(manifest) + os.pathsep + str(manifest)),
                 (str(manifest), str(manifest))]
        with mock.patch.object(gate.platform, 'system', return_value='Linux'), mock.patch.object(gate.platform, 'machine', return_value='x86_64'):
            for first, second in cases:
                with self.subTest(first=first), mock.patch.dict(os.environ, {'VK_DRIVER_FILES': first, 'VK_ICD_FILENAMES': second}), self.assertRaises(AssertionError):
                    gate.configure_software()

    def test_strict_json_rejects_duplicates_nonfinite_and_nonregular(self):
        path = self.root / 'receipt.json'
        for text in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":Infinity}', '{"a":1e999}'):
            path.write_text(text)
            with self.subTest(text=text), self.assertRaises(AssertionError):
                gate.strict_json(path)
        path.write_text('{"a":1}')
        self.assertEqual(gate.strict_json(path), {'a': 1})
        with mock.patch.object(Path, 'is_symlink', return_value=True), self.assertRaises(AssertionError):
            gate.strict_json(path)

    def test_receipt_requires_all_actual_ui_and_windows_limitation_fields(self):
        value = receipt()
        gate.validate_receipt(value)
        for key in list(value):
            changed = copy.deepcopy(value)
            del changed[key]
            with self.subTest(key=key), self.assertRaises(AssertionError):
                gate.validate_receipt(changed)
        for key, replacement in [('schema', True), ('actual_mirror_control', 1),
                                 ('windows_hardware_ui_tested', True), ('adapter', 'NVIDIA hardware')]:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                gate.validate_receipt(dict(value, **{key: replacement}))

    def test_geometry_and_mirror_order_oracles_are_independent_literals(self):
        value = receipt()
        value['mirrored_centers'][1][0][0] = '8'
        with self.assertRaises(AssertionError):
            gate.validate_receipt(value)
        value = receipt()
        value['unchecked_90_centers'] = value['mirrored_centers'][1]
        with self.assertRaises(AssertionError):
            gate.validate_receipt(value)
        value = receipt()
        value['wrong_axis_and_order_equivalence'] = 'mirror after rotation'
        with self.assertRaises(AssertionError):
            gate.validate_receipt(value)

    def test_gameplay_rejects_wrong_controls_vacuity_nonfinite_and_boolean_numbers(self):
        value = receipt()
        for index in range(5):
            for key, replacement in [('mirror_x', 1), ('degrees', True), ('blocked', 'true'),
                                     ('start', [False, 0, 0]), ('end', [float('nan'), 0, 0]),
                                     ('end', [3 if index == 4 else 1, 0, 0])]:
                changed = copy.deepcopy(value)
                changed['gameplay_controls'][index][key] = replacement
                with self.subTest(index=index, key=key), self.assertRaises(AssertionError):
                    gate.validate_receipt(changed)
        changed = copy.deepcopy(value)
        changed['gameplay_controls'][0]['start'][2] = 2
        with self.assertRaises(AssertionError):
            gate.validate_receipt(changed)
        changed = copy.deepcopy(value)
        changed['gameplay_controls'][0]['start'][1] = 4  # X/Z route, not unsettled Y.
        gate.validate_receipt(changed)

    def test_png_decoder_rejects_truncation_crc_and_wrong_dimensions(self):
        path = self.root / 'image.png'
        data = png(2, 2, b'\x11\x22\x33\xff' * 4)
        path.write_bytes(data)
        self.assertEqual(gate.png_pixels(path, 2, 2), b'\x11\x22\x33\xff' * 4)
        for changed in (data[:33], data[:-1], data + b'extra', data[:-1] + b'X'):
            path.write_bytes(changed)
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.png_pixels(path, 2, 2)
        path.write_bytes(data)
        with self.assertRaises(AssertionError):
            gate.png_pixels(path, 3, 2)

    def regions(self):
        values, regions = {}, []
        for prefix in ('01-source', '04-mirrored-saved', '05-fresh-source-hidden'):
            name = prefix + '-viewport.png'
            composed = prefix + '-composed.png'
            full = b'\x11\x22\x33\xff' * (1730 * 1009)
            values[composed] = (1730, 1009, full)
            values[name] = (890, 797, b'\x11\x22\x33\xff' * (890 * 797))
            regions.append({'name': name, 'composed_name': composed, 'x': 235, 'y': 134,
                            'width': 890, 'height': 797, 'viewport_rect': [230, 86, 1130, 936],
                            'framebuffer_width': 1730, 'framebuffer_height': 1009,
                            'panel_widths': {'hierarchy': 230, 'inspector': 600}})
        return values, regions

    def test_viewport_regions_require_real_composed_pixels_and_exact_bounds(self):
        values, regions = self.regions()
        gate.validate_regions(values, regions)
        for key, replacement in [('x', True), ('x', -1), ('y', 100), ('width', 889),
                                 ('composed_name', 'other.png')]:
            changed = copy.deepcopy(regions)
            changed[0][key] = replacement
            with self.subTest(key=key), self.assertRaises(AssertionError):
                gate.validate_regions(values, changed)
        changed = dict(values)
        changed['01-source-viewport.png'] = (890, 797, b'\0' * (890 * 797 * 4))
        with self.assertRaises(AssertionError):
            gate.validate_regions(changed, regions)
        with self.assertRaises(AssertionError):
            gate.validate_regions(values, regions + regions[:1])

    def retained(self):
        directory = self.root / 'captures'
        directory.mkdir()
        root = directory / 'Mirrored Room fixture'
        root.mkdir()
        for name in ('orr.project.json', 'room.scene.yaml', 'room.models.json', 'room.camera.json'):
            (root / name).write_text('fixture bytes ' + name)
        (directory / 'source-hidden.room-stamp.json').write_text('hidden source')
        items = []
        for path in sorted(directory.rglob('*')):
            if path.is_file():
                items.append({'name': path.relative_to(directory).as_posix(), 'bytes': path.stat().st_size,
                              'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
        return directory, sorted(items, key=lambda item: item['name'])

    def test_retained_project_and_hidden_source_exact_bytes_are_required(self):
        directory, items = self.retained()
        gate.validate_retained_files(directory, items)
        for changed in (items[:-1], items + items[:1], list(reversed(items)),
                        [dict(item, bytes=True) for item in items],
                        [dict(item, sha256='0' * 64) for item in items]):
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.validate_retained_files(directory, changed)
        (directory / 'source-hidden.room-stamp.json').write_text('mutated')
        with self.assertRaises(AssertionError):
            gate.validate_retained_files(directory, items)

    def test_capture_census_rejects_extra_missing_unsafe_or_duplicate_entries(self):
        directory = self.root / 'captures'
        directory.mkdir()
        value = {'captures': [{'name': name} for name in gate.CAPTURES]}
        for changed in ([], value['captures'][:-1], value['captures'] + value['captures'][:1],
                        [{'name': '../escape.png'}] + value['captures'][1:]):
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.validate_captures(directory, {'captures': changed})
        with self.assertRaises(AssertionError):
            gate.validate_captures(directory, value)

    def test_mode_coverage_never_certifies_unrun_windows_ui_or_merge(self):
        self.assertEqual(gate.runner.report['coverage'], {'cpu': 'NOT RUN', 'linux_software_vulkan_ui': 'NOT RUN', 'windows_hardware_vulkan_ui': 'NOT RUN'})
        self.assertFalse(gate.runner.report['merge_eligibility'])
        source = (ROOT / 'tools/room_wall_mirror_gate.py').read_text()
        self.assertNotIn("['windows_hardware_vulkan_ui'] = 'passed'", source)

    def test_final_fence_failure_preserves_original_failure(self):
        error = RuntimeError('original GPU or socket failure')
        with mock.patch.object(gate, 'source_fence', side_effect=AssertionError('changed source')), self.assertRaises(SystemExit):
            gate.finish(error)
        self.assertIn('original GPU or socket failure', gate.runner.report['failure'])
        self.assertIn('changed source', gate.runner.report['final_fence_error'])
        self.assertEqual(gate.runner.report['status'], 'failed')

    def test_fixture_marker_cannot_launder_skipped_failed_or_missing_cases(self):
        marker = 'ROOM_WALL_MIRROR_GATE_FIXTURES: 1 passed; 0 failed; 0 skipped'
        value = 'test_one (__main__.Fixture.test_one) ... ok\n\n' + '-' * 70 + '\nRan 1 tests in 0.01s\n\nOK\n' + marker + '\n'
        gate.fixture_result(value, 1, marker)
        for changed in (marker, value.replace('... ok', '... skipped'), value.replace('... ok', '... FAIL'),
                        value.replace('Ran 1', 'Ran 0'), value + value):
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                gate.fixture_result(changed, 1, marker)


def main():
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(MirrorGateTests)
    names = sorted(test.id().rsplit('.', 1)[-1] for test in suite)
    gate.require(names == sorted(EXPECTED_TESTS) and len(names) == gate.FIXTURE_COUNT,
                 'frozen independent mirror checker census')
    result = unittest.TextTestRunner(verbosity=2, failfast=True).run(suite)
    if (result.testsRun != gate.FIXTURE_COUNT or result.skipped or result.expectedFailures
            or result.unexpectedSuccesses or not result.wasSuccessful()):
        raise SystemExit(1)
    print(f'ROOM_WALL_MIRROR_GATE_FIXTURES: {gate.FIXTURE_COUNT} passed; 0 failed; 0 skipped', flush=True)


if __name__ == '__main__':
    main()
