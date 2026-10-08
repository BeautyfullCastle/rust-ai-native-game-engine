#!/usr/bin/env python3
"""Synthetic/static adversarial fixtures, not GPU, Cargo, isolation or CI proof."""
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock

import check_navigation_playground_gate as gate

ROOT = Path(__file__).resolve().parents[1]
INVENTORIES = gate.inventories()


def listing(names):
    return ''.join(name + ': test\n' for name in names) + f'\n{len(names)} tests, 0 benchmarks\n'


def result(spec):
    count = spec['passed']
    return (f'\nrunning {count} tests\n' + ''.join(f'test {name} ... ok\n' for name in spec['names'])
            + '\nsuccesses:\n' + ''.join(f'    {name}\n' for name in spec['names'])
            + f'\ntest result: ok. {count} passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.01s\n')


class InventoryChecks(unittest.TestCase):
    def test_every_exact_positive_inventory_and_result(self):
        for lane, spec in INVENTORIES.items():
            with self.subTest(lane=lane):
                gate.check_log(spec, 'list', listing(spec['names']))
                gate.check_log(spec, 'ignored', listing(spec['ignored_names']))
                gate.check_log(spec, 'result', result(spec))

    def test_missing_extra_duplicate_substituted_and_malformed_names_fail(self):
        for lane, spec in INVENTORIES.items():
            cases = [spec['names'][1:], spec['names'] + ['unrelated_test'],
                     spec['names'] + [spec['names'][0]], ['wrong_filter'] + spec['names'][1:]]
            for names in cases:
                with self.subTest(lane=lane, names=names), self.assertRaises(AssertionError):
                    gate.check_log(spec, 'list', listing(names))
            with self.assertRaises(AssertionError):
                gate.check_log(spec, 'list', listing(spec['names']) + 'wrong-name: test\n')

    def test_absent_wrong_and_duplicate_list_counts_fail(self):
        for spec in INVENTORIES.values():
            good = listing(spec['names'])
            for text in (good.rsplit('\n', 2)[0], good + f'{spec["passed"]} tests, 0 benchmarks\n',
                         good.replace(f'{spec["passed"]} tests', '0 tests'), good.replace('0 benchmarks', '1 benchmarks')):
                with self.assertRaises(AssertionError):
                    gate.check_log(spec, 'list', text)

    def test_ignored_inventory_cannot_drift(self):
        for spec in INVENTORIES.values():
            wrong = [] if spec['run_ignored'] else [spec['names'][0]]
            with self.assertRaises(AssertionError):
                gate.check_log(spec, 'ignored', listing(wrong))

    def test_missing_extra_failed_ignored_and_zero_results_fail(self):
        for lane, spec in INVENTORIES.items():
            good = result(spec)
            bad = ['', good + good, good + 'test result: malformed\n',
                   good.replace('0 failed;', '1 failed;'), good.replace('0 ignored;', '1 ignored;'),
                   re.sub(r'\d+ passed;', '0 passed;', good), good.replace('0 measured;', '1 measured;'),
                   good.replace(f'running {spec["passed"]} tests', 'running 0 tests')]
            for text in bad:
                with self.subTest(lane=lane, text=text), self.assertRaises(AssertionError):
                    gate.check_log(spec, 'result', text)

    def test_passing_summary_alone_never_proves_execution(self):
        for spec in INVENTORIES.values():
            good = result(spec)
            for text in (re.sub(r'^test [^\n]+ \.\.\. ok\n', '', good, flags=re.M),
                         good.replace(spec['names'][0], 'wrong_filter_test', 1),
                         good.replace(' ... ok', ' ... ignored', 1),
                         good.replace(' ... ok', ' ... FAILED', 1),
                         good + f'test {spec["names"][0]} ... ok\n',
                         good + 'test malformed-name ... ok\n'):
                with self.assertRaises(AssertionError):
                    gate.check_log(spec, 'result', text)

    def test_native_stderr_interleave_requires_exact_final_successes(self):
        for spec in INVENTORIES.values():
            good = result(spec).replace(' ... ok\n', ' ... Mesa: cache diagnostic\nok\n', 1)
            gate.check_log(spec, 'result', good)
            for text in (good.replace('\nok\n', '\nFAILED\n', 1),
                         good.replace('    ' + spec['names'][0] + '\n', '', 1),
                         good.replace('    ' + spec['names'][0] + '\n', '    wrong_test\n', 1),
                         good.replace('\nsuccesses:\n', '\nsuccesses:\n    extra_test\n', 1)):
                with self.assertRaises(AssertionError):
                    gate.check_log(spec, 'result', text)

    def test_skip_and_fallback_diagnostics_fail(self):
        for text in ('SKIP: no adapter', 'software GPU unavailable', 'skipping GPU',
                     'fallback enabled', 'without source hiding', 'source_hiding=false'):
            for spec in INVENTORIES.values():
                with self.assertRaises(AssertionError):
                    gate.check_log(spec, 'result', result(spec) + text + '\n')

    def test_inventory_itself_must_have_positive_zero_ignored_contract(self):
        spec = INVENTORIES['app-gpu']
        changes = [{'names': [], 'passed': 0}, {'ignored': 1}, {'passed': 0},
                   {'names': spec['names'] * 2}, {'ignored_names': spec['names']}, {'filter': 'wrong'}]
        for change in changes:
            bad = dict(spec, **change)
            with self.assertRaises(AssertionError):
                gate.validate_spec(bad)

    def test_source_test_functions_are_all_declared_and_owned(self):
        gate.check_source_inventory(INVENTORIES, ROOT)

    def test_wrong_declared_module_prefix_is_rejected_even_when_short_names_match(self):
        for lane, spec in INVENTORIES.items():
            if spec['target'] != ['--lib']:
                continue
            wrong = copy.deepcopy(INVENTORIES)
            old_prefix = spec['names'][0].rsplit('::', 1)[0] + '::'
            new_prefix = spec['names'][0].split('::')[0] + '::wrong_module::'
            for field in ('names', 'ignored_names', 'skip'):
                wrong[lane][field] = [name.replace(old_prefix, new_prefix) for name in spec[field]]
            wrong[lane]['filter'] = spec['filter'].replace(old_prefix, new_prefix)
            with self.subTest(lane=lane), self.assertRaises(AssertionError):
                gate.check_source_inventory(wrong, ROOT)
        wrong = copy.deepcopy(INVENTORIES)
        spec = wrong['pure-admission']
        spec['names'] = [name.replace('::admission_tests::', '::tests::') for name in spec['names']]
        spec['filter'] = spec['filter'].replace('::admission_tests::', '::tests::')
        with self.assertRaises(AssertionError):
            gate.check_source_inventory(wrong, ROOT)

    def test_module_scanner_handles_nested_scopes_comments_and_literals(self):
        source = r'''// mod fake { #[test] fn nope() {} }
mod actual {
    /* outer /* nested */ mod fake { #[test] fn nope() {} } */
    const FAKE: &str = r#"mod wrong { #[test] fn nope() {} }"#;
    #[test] fn direct() { let s = "} mod wrong {"; let c = '}'; }
    mod nested { #[test] #[ignore = "reason"] fn inside() {} }
}
'''
        self.assertEqual(gate.rust_test_names(source, 'file_module'),
                         ['file_module::actual::direct', 'file_module::actual::nested::inside'])

    def test_app_capture_inventory_matches_every_production_evidence_call(self):
        source = (ROOT / 'crates/orr_sample/src/navigation_app.rs').read_text()
        actual = re.findall(r'evidence\(&app, &mut renderer, &gpu, "([^"\n]+)"\)', source)
        self.assertEqual(actual, list(gate.APP_CAPTURE_NAMES))
        self.assertIn('07-view-controls', actual)

    def test_commands_lock_features_target_and_exact_filters(self):
        for lane, spec in INVENTORIES.items():
            args = gate.command(spec)
            self.assertEqual(args[:5], ['cargo', 'test', '--release', '--locked', '--no-default-features'])
            self.assertIn('--', args)
            self.assertNotIn('--ignored', args, 'only explicit result invocation adds --ignored')
            if spec['exact']:
                self.assertIn('--exact', args)
                self.assertIn(spec['names'][0], args)
        self.assertEqual(INVENTORIES['editor-default-off']['features'], [])
        self.assertEqual(INVENTORIES['editor-cpu']['features'], ['navigation-project', 'project-create'])
        self.assertEqual(INVENTORIES['readonly-export']['ignored_names'], INVENTORIES['readonly-export']['names'])
        self.assertEqual(INVENTORIES['app-cpu']['skip'], INVENTORIES['app-gpu']['names'])
        self.assertEqual(INVENTORIES['editor-cpu']['skip'], INVENTORIES['editor-gpu']['names'])


class ArtifactChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)

    def rows(self, group):
        rows = []
        for package, name, kind, test, dest in gate.ARTIFACTS[group]:
            manifest = self.repo / 'crates' / package / 'Cargo.toml'
            src = manifest.parent / ('src/lib.rs' if kind == 'lib' else f'{"src/bin" if kind == "bin" else "tests"}/{name}.rs')
            executable = self.repo / dest
            executable.write_bytes((name + ' fixture, not an executable build').encode())
            executable.chmod(0o755)
            rows.append({'reason': 'compiler-artifact', 'package_id': 'path+' + manifest.parent.as_uri() + '#0.0.1',
                         'manifest_path': str(manifest),
                         'target': {'kind': [kind], 'crate_types': ['lib'] if kind == 'lib' else ['bin'],
                                    'name': name, 'src_path': str(src), 'edition': '2024',
                                    'doc': True, 'doctest': kind == 'lib', 'test': True},
                         'profile': {'opt_level': '3', 'debuginfo': 0, 'debug_assertions': False,
                                     'overflow_checks': False, 'test': test},
                         'features': sorted(gate.EDITOR_FEATURES if package == 'orr_editor' else gate.SAMPLE_FEATURES),
                         'filenames': [str(executable)], 'executable': str(executable), 'fresh': False})
        return rows + [{'reason': 'build-finished', 'success': True}]

    def test_exact_production_and_harness_artifacts(self):
        for group, wanted in gate.ARTIFACTS.items():
            rows = self.rows(group)
            self.assertEqual(len(gate.check_artifacts(rows, self.repo, group)), len(wanted))

    def test_missing_duplicate_failed_and_wrong_package_artifacts_fail(self):
        for group in gate.ARTIFACTS:
            rows = self.rows(group)
            cases = [rows[1:], rows[:-1], rows[:1] + rows,
                     rows[:-1] + [{'reason': 'build-finished', 'success': False}]]
            for field, value in [('package_id', 'path+file:///wrong#0.0.1'), ('manifest_path', '/wrong/Cargo.toml'),
                                 ('features', ['navigation-project']), ('executable', None)]:
                bad = copy.deepcopy(rows)
                bad[0][field] = value
                cases.append(bad)
            for bad in cases:
                with self.subTest(group=group), self.assertRaises(AssertionError):
                    gate.check_artifacts(bad, self.repo, group)

    def test_wrong_target_source_profile_and_aliased_artifacts_fail(self):
        rows = self.rows('production')
        for section, field, value in [('target', 'name', 'wrong_target'), ('target', 'kind', ['test']),
                                      ('target', 'src_path', '/wrong/source.rs'), ('profile', 'test', True),
                                      ('profile', 'opt_level', '0')]:
            bad = copy.deepcopy(rows)
            bad[0][section][field] = value
            with self.assertRaises(AssertionError):
                gate.check_artifacts(bad, self.repo, 'production')
        second = Path(rows[1]['executable'])
        second.unlink()
        os.link(rows[0]['executable'], second)
        with self.assertRaises(AssertionError):
            gate.check_artifacts(rows, self.repo, 'production')

    def test_duplicate_json_fields_and_nonfinite_constants_fail(self):
        for text in ('{"success":true,"success":false}', '{"value":NaN}', '{"value":Infinity}'):
            with self.assertRaises(AssertionError):
                gate.shared.load_json(text)


class AppCaptureChecks(unittest.TestCase):
    """Synthetic images exercise inventory/view-only checks, not GPU pixels."""
    def run_fixture(self, change=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'source-revision.txt').write_text('a' * 40 + '\n' + 'b' * 40 + '\n')
            captures = root / 'app-captures'
            captures.mkdir()
            for name in gate.APP_CAPTURE_NAMES:
                phase = (1 if name in ('02-mid', '03-paused', '04b-sought-mid')
                         else 2 if name in ('04-reached', '05-portrait') else 0)
                pixels = 3 if name == '07-view-controls' else phase
                (captures / (name + '.png')).write_bytes(bytes([pixels]))
                metadata = {'source_sha': 'a' * 40, 'software': True, 'adapter': 'synthetic CPU',
                            'terrain_models': 1, 'overlay_models': 1,
                            'tick': (0, 20, 260)[phase], 'frame_checksum': '0x' + str(phase) * 16,
                            'navigator_checksum': str(phase) * 64, 'position_raw': [phase, 0, 0],
                            'terrain_revision': 'd' * 64, 'graph_revision': 'e' * 64,
                            'status': 'Arrived' if phase == 2 else 'Moving',
                            'size': [480, 720] if name == '05-portrait' else [800, 600]}
                (captures / (name + '.json')).write_text(json.dumps(metadata))
            if change:
                change(captures)
            def read_png(path):
                size = (480, 720) if path.stem == '05-portrait' else (800, 600)
                return *size, bytearray(path.read_bytes() * 512)
            with mock.patch.object(gate.shared, 'read_png', side_effect=read_png):
                gate.check_app_captures(root)

    def test_view_controls_capture_is_required_and_changes_only_view(self):
        self.run_fixture()
        for suffix in ('.png', '.json'):
            with self.assertRaises(AssertionError):
                self.run_fixture(lambda root: (root / ('07-view-controls' + suffix)).unlink())
        with self.assertRaises(AssertionError):
            self.run_fixture(lambda root: (root / '07-view-controls.png').write_bytes(b'\0'))

    def test_view_controls_cannot_change_tick_frame_or_navigation_state(self):
        for field, value in [('tick', 1), ('frame_checksum', '0x' + 'f' * 16),
                             ('navigator_checksum', 'f' * 64), ('position_raw', [1, 2, 3]),
                             ('terrain_revision', 'f' * 64), ('graph_revision', 'f' * 64),
                             ('status', 'Arrived')]:
            def change(root):
                path = root / '07-view-controls.json'
                data = json.loads(path.read_text())
                data[field] = value
                path.write_text(json.dumps(data))
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.run_fixture(change)


class ExportEvidenceChecks(unittest.TestCase):
    """Exercise persisted provenance logic with synthetic PNG decoding only."""
    def fixture(self, root):
        sha, checksum = 'a' * 40, '0x' + 'c' * 16
        (root / 'source-revision.txt').write_text(sha + '\n' + 'b' * 40 + '\n')
        exported = root / 'export-captures'
        exported.mkdir()
        for prefix in ('source', 'export'):
            for phase in ('start', 'mid', 'reached'):
                (exported / f'{prefix}-{phase}.png').write_bytes(('synthetic ' + phase).encode())
        (exported / 'export-restarted.png').write_bytes(b'synthetic mid')
        hashes = {path: 'd' * 64 for path in ('project/orr.project.json', 'project/terrain.orrt',
                                             'project/navigation.scene.yaml', 'bin/navigation_playground', 'run-navigation-playground')}
        payload = {'exporter_version': '0.0.1', 'profile': 'terrain-point-route-authored-linux-x86_64-v1',
                   'entry': {'game': 'terrain-point-route-3d-v1', 'scene': 'navigation.scene.yaml'},
                   'packages': {}, 'files': [{'path': path, 'role': 'fixture', 'mode': 420, 'bytes': 1, 'sha256': digest}
                                            for path, digest in sorted(hashes.items())],
                   'runtime': {'sha256': 'd' * 64, 'bytes': 1},
                   'declared': {'target': 'linux-x86_64', 'source_revision': sha}, 'initial_checksum': checksum}
        canonical = json.dumps(payload, separators=(',', ':')).encode()
        manifest = {'schema': 1, 'payload': payload, 'content_digest': hashlib.sha256(b'orrery.terrain-point-route.export.content.v1\0' + canonical).hexdigest()}
        (exported / 'orr.export.json').write_text(json.dumps(manifest))
        hashes['orr.export.json'] = gate.shared.digest(exported / 'orr.export.json')
        for phase in ('before', 'after'):
            (exported / f'bundle-hashes-{phase}.json').write_text(json.dumps(hashes))
        result = {'source_sha': sha, 'initial_checksum': checksum, 'source_hidden': True,
                  'readonly': True, 'frame_and_navigation_state_parity_ticks': [0, 24, 300],
                  'restart_after': 37, 'restart_observed_tick': 24, 'tamper_rejected_before_app': True,
                  'missing_rejected_before_app': True, 'bundle_hashes_unchanged': True}
        (exported / 'result.json').write_text(json.dumps(result))
        (root / 'production-artifacts.json').write_text(json.dumps([{'name': 'navigation_playground', 'sha256': 'd' * 64}]))
        return exported

    def run_fixture(self, mutate=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            exported = self.fixture(root)
            if mutate:
                mutate(root, exported)
            with mock.patch.object(gate.shared, 'read_png', return_value=(960, 720, bytearray())):
                gate.check_export(root)

    def test_exact_synthetic_export_evidence(self):
        self.run_fixture()

    def test_each_export_proof_and_provenance_is_required(self):
        def change_result(key, value):
            def change(_, root):
                path = root / 'result.json'
                data = json.loads(path.read_text())
                data[key] = value
                path.write_text(json.dumps(data))
            return change
        for field, value in [('source_hidden', False), ('readonly', False), ('readonly', 1),
                             ('source_sha', '0' * 40), ('restart_after', 0), ('restart_observed_tick', 0),
                             ('frame_and_navigation_state_parity_ticks', [0, 24]),
                             ('tamper_rejected_before_app', False), ('missing_rejected_before_app', False),
                             ('bundle_hashes_unchanged', False)]:
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                self.run_fixture(change_result(field, value))

    def test_missing_extra_changed_hashes_and_nonparity_exports_fail(self):
        changes = [lambda _, root: (root / 'result.json').unlink(),
                   lambda _, root: (root / 'extra.png').write_bytes(b'extra'),
                   lambda _, root: (root / 'bundle-hashes-after.json').write_text('{}'),
                   lambda _, root: (root / 'orr.export.json').write_text('{}'),
                   lambda _, root: (root / 'export-mid.png').write_bytes(b'different'),
                   lambda _, root: (root / 'export-restarted.png').write_bytes(b'different'),
                   lambda root, _: (root / 'production-artifacts.json').write_text('[]')]
        for change in changes:
            with self.assertRaises(AssertionError):
                self.run_fixture(change)


class ScriptChecks(unittest.TestCase):
    def test_required_safety_and_separate_cpu_gpu_invocations(self):
        script = (ROOT / 'tools/check-navigation-playground.sh').read_text()
        for required in ('set -euo pipefail', 'git status --porcelain --untracked-files=all',
                         "'HEAD^{tree}'", 'mktemp -d', 'ORR_REQUIRE_GPU=1', 'ORR_REQUIRE_PROJECT_ISOLATION=1',
                         'WGPU_BACKEND=${WGPU_BACKEND:-vulkan}', 'LIBGL_ALWAYS_SOFTWARE=1', '--tmpfs "$repo"',
                         'NAVIGATION_PLAYGROUND_CAPTURE_DIR=', 'NAVIGATION_PROJECT_CAPTURE_DIR=',
                         'NAVIGATION_EXPORT_CAPTURE_DIR=', 'ORR_NAVIGATION_RUNTIME=', 'ORR_NAVIGATION_EXPORTER=',
                         'tests app-gpu "$evidence/tools/app-tests"',
                         'tests editor-gpu "$evidence/tools/editor-tests"',
                         'tests readonly-export "$evidence/tools/export-tests"',
                         'sha256sum --check', '--bin orr_new_navigation',
                         'XDG_CACHE_HOME="$evidence/xdg-cache"', 'MESA_SHADER_CACHE_DIR="$evidence/mesa-shader-cache"'):
            self.assertIn(required, script)
        self.assertNotIn('|| true', script)
        self.assertNotIn('--nocapture', script)
        self.assertIn('if [[ $name == readonly-export ]]; then options+=(--ignored); fi', script)

    def test_default_dependency_graph_is_positive_and_closed(self):
        for package in ('orr_sample', 'orr_editor'):
            good = package + ' v0.0.1 (/workspace/crates/' + package + ')\norr_ecs v0.0.1\n'
            gate.check_default_graph(package, good)
            for bad in ('', 'unrelated v0.0.1\n', good + 'orr_navigation v0.0.1\n',
                        good + 'orr_navigation_runtime v0.0.1\n', good + 'orr_terrain_view v0.0.1\n'):
                with self.assertRaises(AssertionError):
                    gate.check_default_graph(package, bad)

    def test_source_revision_requires_exact_commit_and_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / 'source-revision.txt'
            path.write_text('a' * 40 + '\n' + 'b' * 40 + '\n')
            self.assertEqual(gate.check_revision(root), 'a' * 40)
            for bad in ('a' * 40 + '\n', 'HEAD\ntree\n', 'a' * 40 + '\n' + 'b' * 40 + '\nextra\n'):
                path.write_text(bad)
                with self.assertRaises(AssertionError):
                    gate.check_revision(root)


class ProducerPinOrdering(unittest.TestCase):
    def test_each_artifact_is_pinned_before_the_next_cargo_group(self):
        script = Path(__file__).with_name('check-navigation-playground.sh').read_text()
        for group, following in [('production', 'app'), ('app', 'editor'), ('editor', 'export')]:
            build = script.index(f'run_json {group}-build ')
            pin = script.index(f'python3 "$checker" pin "$evidence" "$repo" {group}\n')
            next_build = script.index(f'run_json {following}-build ')
            self.assertLess(build, pin)
            self.assertLess(pin, next_build)
        self.assertLess(script.index('run_json export-build '), script.index('python3 "$checker" pin "$evidence" "$repo" export\n'))


if __name__ == '__main__':
    unittest.main()
