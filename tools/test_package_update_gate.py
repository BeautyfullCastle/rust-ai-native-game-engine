#!/usr/bin/env python3
"""Independent package-update gate rejection fixtures; no Cargo/GPU use."""
import ast
import copy
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import package_update_gate as gate

ROOT = pathlib.Path(__file__).resolve().parents[1]


def workflow_contract(text):
    assert text.startswith("name: Checked package update\non:\n  push:\n    branches: [main, su/engine-correctness-hardening]\n    tags: ['**']\n  pull_request:\npermissions:\n")
    assert 'permissions:\n  contents: read\n' in text
    assert re.findall(r'^  ([a-z]+):$', text[text.index('jobs:\n'):], re.M) == ['acceptance', 'required']
    for marker in ('    runs-on: ${{ matrix.os }}\n', '    timeout-minutes: 90\n',
                   '      fail-fast: false\n', '      PYTHONDONTWRITEBYTECODE: "1"\n',
                   '      CARGO_PROFILE_RELEASE_DEBUG: "0"\n', '        shell: bash\n',
                   '          components: clippy\n', '          if-no-files-found: error\n',
                   '          include-hidden-files: true\n',
                   '    if: always()\n    needs: acceptance\n',
                   '          ACCEPTANCE: ${{ needs.acceptance.result }}\n'):
        assert marker in text, marker
    assert re.findall(r'          - \{mode: (\w+), os: ([\w.-]+)\}', text) == list(gate.WORKFLOW_ROWS)
    assert text.count('      matrix:\n') == 1
    for marker in ('continue-on-error', 'paths-ignore', 'paths:', 'branches-ignore:', 'cargo test', 'rust-cache'):
        assert marker not in text
    assert [line.strip() for line in text.splitlines() if line.strip().startswith('if:')] == [
        "if: matrix.mode == 'ui'", 'if: always()', 'if: always()']
    assert [line.strip() for line in text.splitlines() if line.strip().startswith('run:')] == [
        'run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers libxkbcommon-dev',
        "run: python -B tools/package_update_gate.py '${{ matrix.mode }}'",
        'run: test "$ACCEPTANCE" = success']
    assert '        if: always()\n        uses: actions/upload-artifact@v4\n' in text
    assert '          name: ' + gate.WORKFLOW_ARTIFACT + '\n' in text
    assert '          path: ' + gate.WORKFLOW_EVIDENCE + '\n' in text
    assert '        env:\n          ORR_PACKAGE_UPDATE_EVIDENCE: ' + gate.WORKFLOW_EVIDENCE + '\n' in text
    assert [line for line in text.splitlines() if 'runner.temp' in line] == [
        '          ORR_PACKAGE_UPDATE_EVIDENCE: ' + gate.WORKFLOW_EVIDENCE,
        '          path: ' + gate.WORKFLOW_EVIDENCE]


def result(names, filtered=0):
    return ('running ' + str(len(names)) + (' test\n' if len(names) == 1 else ' tests\n')
        + ''.join('test ' + name + ' ... ok\n' for name in names)
        + f'\ntest result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; {filtered} filtered out; finished in 0.01s\n')


class Fixtures(unittest.TestCase):
    def test_workflow_contract(self):
        workflow_contract((ROOT / '.github/workflows/package-update.yml').read_text())

    def test_workflow_negative_mutations(self):
        original = (ROOT / '.github/workflows/package-update.yml').read_text()
        # Trigger changes must preserve every PR and tag while avoiding feature-push duplicates.
        trigger_filters = "    branches: [main, su/engine-correctness-hardening]\n    tags: ['**']\n"
        for old, new in [
            (trigger_filters, ''),
            ('branches: [main, su/engine-correctness-hardening]', "branches: ['**']"),
            ('branches: [main, su/engine-correctness-hardening]', 'branches: [main]'),
            ('branches: [main, su/engine-correctness-hardening]', 'branches: [su/engine-correctness-hardening]'),
            ("    tags: ['**']\n", ''),
            ("tags: ['**']", "tags: ['v*']"),
            ('  pull_request:\n', ''),
            ('  pull_request:\n', '  pull_request:\n    branches: [main]\n'),
            ('  pull_request:\n', '  pull_request:\n    paths: [src/**]\n'),
        ]:
            with self.subTest(trigger=old), self.assertRaises(AssertionError):
                workflow_contract(original.replace(old, new, 1))
        for old, new in [('mode: ui', 'mode: cpu'), ('windows-latest', 'ubuntu-22.04'),
                         ('if-no-files-found: error', 'if-no-files-found: warn'),
                         ('include-hidden-files: true', 'include-hidden-files: false'),
                         ('if: always()', 'if: success()'), ('needs: acceptance', 'needs: missing'),
                         ('test "$ACCEPTANCE" = success', 'true'), ('fail-fast: false', 'fail-fast: true'),
                         ('PYTHONDONTWRITEBYTECODE: "1"', 'PYTHONDONTWRITEBYTECODE: "0"'),
                         ('        shell: bash', '        shell: pwsh')]:
            with self.subTest(old=old), self.assertRaises(AssertionError):
                workflow_contract(original.replace(old, new))
        for altered in (original.replace('    env:\n', '    env:\n      BAD: ${{ runner.temp }}\n', 1),
                        original.replace('    runs-on: ${{ matrix.os }}', '    if: false\n    runs-on: ${{ matrix.os }}')):
            with self.assertRaises(AssertionError): workflow_contract(altered)

    def test_complete_current_source_inventory(self):
        names = set(gate.FEATURE_PATHS + gate.INFRA_PATHS)
        self.assertEqual(len(gate.FEATURE_PATHS), len(set(gate.FEATURE_PATHS)))
        gate.require_sources(dict.fromkeys(names))
        for name in names:
            with self.assertRaises(AssertionError): gate.require_sources(dict.fromkeys(names - {name}))
        self.assertFalse(hasattr(gate, 'PRESERVED_SHA256'))

    def test_source_fence_bytes_modes_and_untracked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); path = root / 'one'; path.write_bytes(b'one')
            with mock.patch.object(gate, 'source_root', root):
                frozen = gate.snapshot()
                with mock.patch.object(gate, 'frozen', frozen):
                    gate.source_fence(); path.write_bytes(b'two')
                    with self.assertRaises(AssertionError): gate.source_fence()
                    path.write_bytes(b'one')
                    if os.name != 'nt':
                        path.chmod(0o700)
                        with self.assertRaises(AssertionError): gate.source_fence()
                        path.chmod(frozen['one']['mode'])
                    (root / 'ignored-source').write_bytes(b'x')
                    with self.assertRaises(AssertionError): gate.source_fence()

    def test_source_directories_refuse_symlinks(self):
        if os.name == 'nt':
            with tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory); (root / 'one').mkdir()
                with mock.patch.object(pathlib.Path, 'is_symlink', return_value=True), self.assertRaises(AssertionError):
                    gate.source_paths(root)
        else:
            with tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory); (root / 'one').symlink_to(root, target_is_directory=True)
                with self.assertRaises(AssertionError): gate.source_paths(root)

    def test_optimized_python_refused(self):
        completed = subprocess.run([sys.executable, '-B', '-O', str(ROOT / 'tools/package_update_gate.py'), 'cpu'],
            capture_output=True, text=True, env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1'), check=False)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn('requires nonoptimized Python', completed.stderr)

    def test_bytecode_prevention_inherited(self):
        for environment in ({}, {'PYTHONDONTWRITEBYTECODE': '0'}, {'PYTHONDONTWRITEBYTECODE': ''}):
            with mock.patch.dict(os.environ, environment, clear=True), self.assertRaises(AssertionError): gate.begin()

    def test_inventory_exact_names_count_and_duplicates(self):
        good = 'one: test\ntwo: test\n\n2 tests, 0 benchmarks\n'
        self.assertEqual(gate.inventory_names(good), ['one', 'two'])
        for bad in (good.replace('two: test', 'one: test'), good.replace('2 tests', '3 tests'), good + 'three: test\n'):
            with self.assertRaises(AssertionError): gate.inventory_names(bad)
        for wanted in (['one'], ['one', 'wrong'], ['one', 'two', 'three']):
            with self.assertRaises(AssertionError): gate.runner.inventory(good, wanted)

    def test_result_exact_partition_rejects_ignored_failure_filter_and_skip(self):
        good = result(['one', 'two'], 10)
        gate.executed(good, ['one', 'two'], 10)
        for bad in (good.replace('two ... ok', 'two ... ignored'), good.replace('2 passed', '1 passed'),
                    good.replace('10 filtered', '9 filtered'), good.replace('two ... ok', 'wrong ... ok'),
                    good + 'SKIP: missing adapter\n', good + good):
            with self.assertRaises(AssertionError): gate.executed(bad, ['one', 'two'], 10)

    def test_new_inventory_separate_from_old_counts(self):
        source = (ROOT / 'tools/package_update_gate.py').read_text(); ast.parse(source)
        self.assertIn("'package_manager::update::tests::'", source)
        self.assertIn("'package_manager_panel::update_tests::'", source)
        self.assertIn("'legacy-removal'", source); self.assertIn("'legacy-panel'", source)
        self.assertTrue(gate.BACKEND_TESTS and gate.PANEL_TESTS, 'explicit authored inventory required')
        for names, prefix in [(gate.BACKEND_TESTS, 'package_manager::update::tests::'),
                              (gate.PANEL_TESTS, 'package_manager_panel::update_tests::')]:
            self.assertEqual(len(names), len(set(names)))
            self.assertTrue(all(name.startswith(prefix) for name in names))
        backend = (ROOT / 'crates/orr_editor/src/package_manager/update_tests.rs').read_text()
        panel = (ROOT / 'crates/orr_editor/src/package_manager_panel.rs').read_text().split('mod update_tests {', 1)[1]
        for source, prefix, expected in [(backend, 'package_manager::update::tests::', gate.BACKEND_TESTS),
                                         (panel, 'package_manager_panel::update_tests::', gate.PANEL_TESTS)]:
            names = re.findall(r'#\[test\]\s+fn (\w+)', source)
            self.assertEqual(set(prefix + name for name in names), set(expected))

    def test_source_build_commands_and_no_bypass(self):
        source = (ROOT / 'tools/package_update_gate.py').read_text()
        for marker in ('--no-default-features', '--message-format=json', 'source_fence()', 'runner.fence()',
                       'checked.software_fence()', "'--', '-D', 'warnings'", 'default-off-check',
                       'minimal-package-manager-check', 'animated-without-package-manager-check'):
            self.assertIn(marker, source)
        for forbidden in ("'--skip'", 'continue-on-error', 'CARGO_TARGET_DIR=target/release'):
            self.assertNotIn(forbidden, source)

    def test_retained_library_producer_validates_cargo_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve(); crate = root / 'crates/orr_editor'
            (crate / 'src').mkdir(parents=True); target = root / 'target/fresh'
            binary = target / 'release/deps/editor-test'; binary.parent.mkdir(parents=True); binary.write_bytes(b'producer')
            evidence = root / 'evidence'; evidence.mkdir()
            artifact = {'reason': 'compiler-artifact', 'fresh': False, 'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_editor@0.0.1', 'features': gate.LIB_FEATURES,
                'target': {'name': 'orr_editor', 'src_path': str(crate / 'src/lib.rs'), 'kind': ['lib'], 'crate_types': ['lib']},
                'profile': {'test': True, 'opt_level': '3'}, 'executable': str(binary)}
            report = {'producers': {}, 'source_inventory_sha256': 'a' * 64}
            with mock.patch.object(gate, 'source_root', root), mock.patch.object(gate.runner, 'target_dir', target), \
                 mock.patch.object(gate.runner, 'evidence', evidence), mock.patch.object(gate.runner, 'report', report), \
                 mock.patch.object(gate.runner, 'record'):
                result_path = gate.pin_library(json.dumps(artifact), 'valid', gate.LIB_FEATURES)
                self.assertEqual(pathlib.Path(result_path).read_bytes(), b'producer')
                self.assertEqual(report['producers']['valid']['source_inventory_sha256'], 'a' * 64)
                for key, value in [('fresh', True), ('features', []), ('manifest_path', str(root / 'Cargo.toml')),
                                   ('package_id', 'registry+https://example.invalid#orr_editor'),
                                   ('profile', {'test': True, 'opt_level': '0'})]:
                    changed = copy.deepcopy(artifact); changed[key] = value
                    with self.subTest(key=key), self.assertRaises(AssertionError):
                        gate.pin_library(json.dumps(changed), 'invalid', gate.LIB_FEATURES)
                with self.assertRaises(AssertionError):
                    gate.pin_library(json.dumps(artifact) + '\n' + json.dumps(artifact), 'duplicate', gate.LIB_FEATURES)

    def test_coverage_unrun_rows_cannot_be_promoted(self):
        for phase in ('cpu', 'ui'):
            report = {'coverage': {name: 'PASSED' if name == phase else 'NOT RUN' for name in ('cpu', 'ui')}}
            with mock.patch.object(gate, 'mode', phase), mock.patch.object(gate.runner, 'report', report):
                gate.check_coverage(); report['coverage']['ui' if phase == 'cpu' else 'cpu'] = 'PASSED'
                with self.assertRaises(AssertionError): gate.check_coverage()

    def test_explicit_local_lavapipe_manifest_is_supported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve(); path = root / 'lvp_icd.json'
            path.write_text(json.dumps({'file_format_version': '1.0.0', 'ICD': {
                'api_version': '1.4.305', 'library_path': '/reviewed/libvulkan_lvp.so'}}))
            selected, raw = gate.checked.software_manifest({'VK_DRIVER_FILES': str(path)}, root / 'absent-system')
            self.assertEqual(selected, path); self.assertEqual(raw, path.read_bytes())
            for environment in ({'VK_DRIVER_FILES': str(path) + ':' + str(path)},
                                {'VK_DRIVER_FILES': str(path), 'VK_ICD_FILENAMES': str(root / 'different.json')},
                                {'VK_DRIVER_FILES': 'lvp_icd.json'}):
                with self.assertRaises(AssertionError): gate.checked.software_manifest(environment, root)

    def test_strict_json_duplicate_nonfinite_receipts_refused(self):
        for text in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":1e999}'):
            with self.assertRaises(AssertionError): gate.checked.strict_json_text(text)

    def test_capture_inventory_rejects_unexpected_and_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); (root / 'receipt.json').write_text('{}')
            self.assertEqual(set(gate.checked.capture_files(root, {'receipt.json'})), {'receipt.json'})
            for inventory in ({'receipt.json', 'missing'}, set()):
                with self.assertRaises(AssertionError): gate.checked.capture_files(root, inventory)

    def test_update_lock_delta_rejects_unrelated_and_dependency_changes(self):
        before = {'schema': 1, 'direct': {'target': '1.0.0', 'other': '1.0.0'}, 'packages': {}}
        for name in ('target', 'other'):
            before['packages'][name] = {'manifest': {'name': name, 'version': '1.0.0', 'dependencies': {}},
                                        'digest': 'a' * 64, 'files': {'model.orrmodel': 'c' * 64}}
        after = copy.deepcopy(before); after['direct']['target'] = '2.0.0'
        after['packages']['target'].update(digest='b' * 64)
        after['packages']['target']['manifest']['version'] = '2.0.0'
        gate.check_lock_delta(before, after, 'target')
        for mutate in (lambda value: value['direct'].__setitem__('other', '2.0.0'),
                       lambda value: value['packages']['other'].__setitem__('digest', 'd' * 64),
                       lambda value: value['packages']['target']['manifest']['dependencies'].__setitem__('new', '1.0.0'),
                       lambda value: value['packages'].pop('other'),
                       lambda value: value['direct'].__setitem__('target', '1.0.0')):
            changed = copy.deepcopy(after); mutate(changed)
            with self.assertRaises(AssertionError): gate.check_lock_delta(before, changed, 'target')

    def test_png_validation_is_measured_not_receipt_only(self):
        self.assertEqual(gate.checked.difference(b'\0\0\0\xff', b'\1\0\0\xff'), 1)
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'bad.png'; path.write_bytes(b'\x89PNG\r\n\x1a\n')
            with self.assertRaises(AssertionError): gate.checked.png_pixels(path, 1, 1)


    def test_ui_receipt_rejects_missing_false_stale_and_nondefault_proof(self):
        receipt = dict.fromkeys(gate.UI_FIELDS)
        receipt.update(dict.fromkeys(gate.UI_TRUE_FIELDS, True))
        receipt.update(schema=1, status='passed', test=gate.UI_TEST, mandatory_test_count=1,
            test_source_fnv1a64=gate.checked.fnv1a64((ROOT / f'crates/orr_editor/tests/{gate.UI_TARGET}.rs').read_bytes()),
            adapter='llvmpipe', renderer_options='default', predictable_texture_filtering=False,
            package='raw-package-update', old_version='1.0.0', new_version='2.0.0', assigned_role='KEY',
            assigned_guid='e_00000001', normalized_viewport_size=[900, 850], key_placements_before_collect=2,
            key_placements_after_collect=1, key_placements_after_restart=2, key_collected=1, restart_key_collected=0,
            recorded_ticks=10, recorded_checksums_checked=2, authoritative_checksum='a' * 16)
        with mock.patch.object(gate, 'source_root', ROOT):
            gate.check_ui_receipt(receipt)
            for field in gate.UI_FIELDS:
                altered = dict(receipt); del altered[field]
                with self.subTest(missing=field), self.assertRaises(AssertionError): gate.check_ui_receipt(altered)
            for field, value in [*[(field, False) for field in gate.UI_TRUE_FIELDS], ('schema', True),
                                 ('test_source_fnv1a64', '0' * 16), ('test', 'historical_test'),
                                 ('mandatory_test_count', 0), ('renderer_options', 'predictable'),
                                 ('predictable_texture_filtering', True), ('adapter', 'hardware'),
                                 ('new_version', '1.0.0'), ('recorded_ticks', 0), ('recorded_checksums_checked', 12)]:
                altered = dict(receipt); altered[field] = value
                with self.subTest(field=field), self.assertRaises(AssertionError): gate.check_ui_receipt(altered)

    def test_package_object_hashes_sources_and_canonical_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); source = root / 'source'; source.mkdir()
            manifest = {'schema': 1, 'name': 'target', 'version': '2.0.0', 'engine': '*',
                        'capabilities': ['models'], 'dependencies': {}, 'files': ['LICENSE.txt', 'model.glb']}
            values = {'LICENSE.txt': b'license', 'model.glb': b'model'}
            hashes = {name: hashlib.sha256(data).hexdigest() for name, data in values.items()}
            digest = hashlib.sha256(json.dumps(['orr-package-v1', manifest, hashes], separators=(',', ':')).encode()).hexdigest()
            package = {'manifest': manifest, 'digest': digest, 'files': hashes}
            objects = root / '.orr/packages/objects' / digest; objects.mkdir(parents=True)
            (source / 'orr.package.json').write_text(json.dumps(manifest))
            (objects / 'orr.package.json').write_text(json.dumps(manifest))
            for name, data in values.items():
                (source / name).write_bytes(data); (objects / name).write_bytes(data)
            gate.check_package_object(root, package, source)
            for path in (source / 'orr.package.json', objects / 'orr.package.json'):
                path.write_text('{}')
                with self.assertRaises(AssertionError): gate.check_package_object(root, package, source)
                path.write_text(json.dumps(manifest))
            for path in (source / 'model.glb', objects / 'model.glb'):
                path.write_bytes(b'corrupted')
                with self.assertRaises(AssertionError): gate.check_package_object(root, package, source)
                path.write_bytes(values['model.glb'])
            altered = copy.deepcopy(package); altered['manifest']['version'] = '3.0.0'
            with self.assertRaises(AssertionError): gate.check_package_object(root, altered, source)
            altered = copy.deepcopy(package); altered['digest'] = '0' * 64
            with self.assertRaises(AssertionError): gate.check_package_object(root, altered, source)

    def test_glb_source_evidence_rejects_malformed_or_changed_header(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'model.glb'
            data = (ROOT / 'assets/imported_scene_demo/foreground.glb').read_bytes(); path.write_bytes(data)
            document, binary = gate.glb_chunks(path)
            self.assertTrue(document['meshes']); self.assertTrue(binary)
            for bad in (b'invalid', data[:-1], data + b'x', b'bad!' + data[4:], data[:4] + b'\x01\0\0\0' + data[8:]):
                path.write_bytes(bad)
                with self.assertRaises(AssertionError): gate.glb_chunks(path)


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Fixtures)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.testsRun != gate.FIXTURE_COUNT or result.skipped:
        raise SystemExit(1)
    print(f'PACKAGE_UPDATE_GATE_FIXTURES: {gate.FIXTURE_COUNT} passed; 0 failed; 0 skipped')
