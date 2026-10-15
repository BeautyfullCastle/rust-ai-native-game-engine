#!/usr/bin/env python3
"""Independent semantic and real-process fixtures for the static prop gate."""
import copy
import hashlib
import json
import os
import pathlib
import re
import struct
import sys
import tempfile
import time
import unittest
from unittest import mock
import zlib

import static_model_extract_gate as gate

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXPECTED_TESTS = [
    'test_success_reaps_changed_group_descendants',
    'test_timeout_reaps_separate_session_and_retained_pipe',
    'test_final_revision_and_path_inventory_are_fenced',
    'test_actual_model_census_matches_test_source',
    'test_cached_or_missing_freshness_is_rejected',
    'test_private_target_cannot_be_reused',
    'test_success_reaps_separate_session_descendants',
    'test_timeout_reaps_changed_group_descendants',
    'test_exact_inventory',
    'test_inventory_rejects_wrong_count_duplicates_and_extras',
    'test_exact_result',
    'test_result_accepts_hosted_linux_interleaved_cli_stdout',
    'test_result_accepts_windows_crlf_interleaved_cli_stdout',
    'test_result_rejects_ambiguous_interleaved_records',
    'test_result_rejects_skip_failure_missing_names_and_counts',
    'test_exact_producer',
    'test_producer_rejects_package_source_profile_features_and_path',
    'test_retained_executable_mutation',
    'test_source_snapshot_bytes_modes_and_file_types',
    'test_real_exit_and_durable_receipt',
    'test_bounded_output_and_failure_receipt',
    'test_bounded_deadline_and_cleanup',
    'test_success_reaps_descendants',
    'test_gpu_capture_census_and_pixels',
    'test_gpu_adapter_hash_and_control_negatives',
    'test_export_capture_inventory_and_provenance',
    'test_export_rejects_source_producer_frame_and_inventory_mutations',
    'test_workflow_required_modes_and_artifacts',
    'test_workflow_rejects_disabled_or_unretained_modes',
]

def listing(names):
    return ''.join(name + ': test\n' for name in names) + f'\n{len(names)} tests, 0 benchmarks\n'

def passed(names):
    return f'running {len(names)} test' + ('' if len(names) == 1 else 's') + '\n' + ''.join('test ' + name + ' ... ok\n' for name in names) + (
        f'\ntest result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')

# Exact retained Linux model-run.log, SHA-256 cf5407b915ad29e201679ad009cca137e266147b2ffe8fdf9fc62dc7e4bc184b.
# Direct authoring::run stdout bypasses libtest's normal print capture.
HOSTED_MODEL_LOG = '''
running 20 tests
test absent_out_of_scene_and_empty_subtree_selections_reject ... ok
test cli_bad_arguments_fail_without_output_or_staging ... published 2418 bytes to /tmp/orr-static-prop-test-2973-0/prop.model.json
ok
test cli_invalid_missing_and_oversized_inputs_leave_no_output ... ok
test cli_listing_and_help_do_not_publish_or_change_input ... Usage: static_model_extract --input FILE.glb --asset-id LOGICAL/SOURCE.glb --node N --output NEW.model.json
List selectable source nodes: static_model_extract --input FILE.glb --asset-id LOGICAL/SOURCE.glb --list
Requires import; embedded static GLB only. Subtrees keep source-world coordinates and all materials/images. Trusted stable link-free parents; no overwrite/install. Exit 0: help/list/published (check warnings); 2: error/not published.
[{"index":0,"name":"assembly","primitive_count":6},{"index":1,"name":"prop","primitive_count":4},{"index":2,"name":"sibling","primitive_count":2},{"index":3,"name":"prop","primitive_count":2}]
[{"index":0,"name":"assembly","primitive_count":6},{"index":1,"name":"prop","primitive_count":4},{"index":2,"name":"sibling","primitive_count":2},{"index":3,"name":"prop","primitive_count":2}]
ok
test cli_nonregular_socket_input_rejects_without_opening_or_publication ... ok
test cli_publishes_reloaded_model_once_and_preserves_source_and_existing_targets ... published 2418 bytes to /tmp/orr-static-prop-test-2973-4/prop.model.json
ok
test cli_symlink_sources_destinations_and_ancestors_reject ... ok
test cli_unsafe_names_ancestry_and_package_store_outputs_reject ... ok
test explicit_and_implicit_default_scenes_control_selection ... ok
test external_data_uri_and_unknown_container_resources_reject ... ok
test full_source_admission_rejects_bad_siblings_before_selection ... ok
test listing_uses_default_scene_nonempty_subtrees_and_unambiguous_indices ... ok
test logical_identity_and_exact_input_digest_survive_extraction ... ok
test malformed_envelopes_truncation_and_duplicate_fields_reject ... ok
test nested_selection_keeps_ancestor_mirror_nonuniform_transform_without_recentering ... ok
test root_selection_roundtrips_exact_full_import_and_is_deterministic ... ok
test source_json_and_node_budgets_reject_before_output ... ok
test subtree_preserves_geometry_uv_material_tables_and_source_slot_order ... ok
test unsupported_animation_skin_extensions_and_attributes_are_never_dropped ... ok
test unused_material_and_image_slots_are_retained_without_remapping ... ok

test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

'''

def workflow_job(text, name):
    peers = list(re.finditer(r'^  ([A-Za-z0-9_-]+):\n', text, re.M))
    matches = [index for index, peer in enumerate(peers) if peer.group(1) == name]
    assert len(matches) == 1, ('exact workflow job', name)
    index = matches[0]
    end = peers[index + 1].start() if index + 1 < len(peers) else len(text)
    return text[peers[index].start():end]

def workflow_contract(text):
    sample = workflow_job(text, 'sample-build')
    room = workflow_job(text, 'sample-room')
    assert 'os: [ubuntu-22.04, windows-latest]' in sample
    assert 'static_model_extract_gate.py' not in room, 'protected Room job has no new producer'
    export_marker = '      - name: Required static prop source-hidden Room export acceptance\n'
    namespace_setup = ("      - name: Provision repository-hidden authored runtime proof\n"
                       "        if: runner.os == 'Linux'\n"
                       '        run: sudo apt-get update -q && sudo apt-get install -y -q bubblewrap\n')
    vulkan_setup = ("      - if: runner.os == 'Linux'\n"
                    '        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n')
    assert namespace_setup + export_marker in sample, 'export immediately follows namespace setup'
    assert vulkan_setup in sample, 'mandatory Vulkan dependency'
    assert sample.index(vulkan_setup) < sample.index(namespace_setup), 'Vulkan before export'
    for mode, title, artifact, part in [
        ('cpu', 'Required static prop CPU CLI and bindings acceptance',
         'Retain static prop CPU and partial failure evidence', sample),
        ('gpu', 'Required static prop actual software GPU acceptance',
         'Retain static prop GPU and partial failure evidence', sample),
        ('export', 'Required static prop source-hidden Room export acceptance',
         'Retain static prop Room export and partial failure evidence', sample),
    ]:
        marker = '      - name: ' + title + '\n'
        assert text.count(marker) == 1 and marker in part
        block = part.split(marker, 1)[1].split('      - name:', 1)[0]
        assert f'        run: python -B tools/static_model_extract_gate.py {mode}\n' in block
        assert '        shell: bash\n' in block and '        timeout-minutes: ' in block
        assert 'ORR_STATIC_GATE_EVIDENCE:' in block and 'continue-on-error' not in block
        if mode == 'cpu':
            assert '\n        if:' not in block
        else:
            assert "        if: runner.os == 'Linux'" in block
            if mode == 'export':
                assert "&& runner.arch == 'X64'" in block
        marker = '      - name: ' + artifact + '\n'
        assert text.count(marker) == 1 and marker in part
        block = part.split(marker, 1)[1].split('      - name:', 1)[0].split('      #', 1)[0]
        assert '        if: always()' in block and '        uses: actions/upload-artifact@v4' in block
        assert '          if-no-files-found: error\n' in block
        assert '          retention-days: 7\n' in block
        if mode == 'export':
            assert '          include-hidden-files: true\n' in block
    # Existing required routes and the failure-aware aggregate remain mandatory.
    for marker in [
        'Required animation clip extraction cpu acceptance',
        'Required animation clip extraction gpu acceptance',
        'RoomEscapeV1 actual GPU and source-hidden production export',
        'Required generated Room GPU and source-hidden production export',
    ]:
        assert text.count('      - name: ' + marker + '\n') == 1
    assert 'needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, sample-linked-prefabs, sample-room]' in text
    assert '    if: \u0024{{ always() }}\n' in text

class StaticPropGateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name).resolve()
        self.repo = self.root / 'checkout'; self.repo.mkdir()
        environment = mock.patch.dict(os.environ, {}, clear=False)
        environment.start(); self.addCleanup(environment.stop)
        floor = mock.patch.object(gate, 'DISK_FLOOR', 0)
        floor.start(); self.addCleanup(floor.stop)
        gate.initialize(self.repo, self.root / 'evidence')

    def test_exact_inventory(self):
        gate.inventory(listing(['alpha', 'module::beta']), ['module::beta', 'alpha'])
        gate.inventory('0 tests, 0 benchmarks\n', [])

    def test_actual_model_census_matches_test_source(self):
        source = (ROOT / 'crates/orr_model/tests/static_model_extract.rs').read_text()
        actual = re.findall(r'#\[test\]\s+fn ([A-Za-z0-9_]+)\(', source)
        unix = {'cli_symlink_sources_destinations_and_ancestors_reject',
                'cli_nonregular_socket_input_rejects_without_opening_or_publication'}
        linux_names = gate.model_tests()
        windows_names = gate.model_tests(windows=True)
        self.assertEqual(len(actual), 20)
        self.assertEqual(sorted(linux_names), sorted(actual))
        self.assertEqual(len(set(linux_names)), 20)
        self.assertEqual(len(windows_names), 18)
        self.assertEqual(set(windows_names), set(actual) - unix)
        gate.inventory(listing(actual), linux_names)
        gate.inventory(listing(sorted(set(actual) - unix)), windows_names)

    def test_inventory_rejects_wrong_count_duplicates_and_extras(self):
        for value in [listing(['alpha']).replace('1 tests', '0 tests'),
                      listing(['alpha', 'alpha']), listing(['alpha', 'extra']),
                      'alpha: test\n', listing(['alpha']).replace('0 benchmarks', '1 benchmarks')]:
            with self.subTest(value=value), self.assertRaises(AssertionError):
                gate.inventory(value, ['alpha'])

    def test_exact_result(self):
        gate.result(passed(['alpha', 'module::beta']), ['module::beta', 'alpha'])
        gate.result(passed([]), [])
        with mock.patch.object(gate, 'run', side_effect=[listing(['alpha']), listing([]), passed(['alpha'])]) as run:
            gate.test('serial', '/absolute/test-binary', ['alpha'])
        self.assertEqual(run.call_args_list[-1], mock.call('serial-run', ['/absolute/test-binary', '--test-threads=1']))

    def test_result_accepts_hosted_linux_interleaved_cli_stdout(self):
        self.assertEqual(hashlib.sha256(HOSTED_MODEL_LOG.encode()).hexdigest(),
                         'cf5407b915ad29e201679ad009cca137e266147b2ffe8fdf9fc62dc7e4bc184b')
        gate.result(HOSTED_MODEL_LOG, gate.model_tests())

    def test_result_accepts_windows_crlf_interleaved_cli_stdout(self):
        # Synthetic Windows-shaped variant, not a claim of Windows execution.
        output = HOSTED_MODEL_LOG
        for name in ['cli_nonregular_socket_input_rejects_without_opening_or_publication',
                     'cli_symlink_sources_destinations_and_ancestors_reject']:
            output = output.replace('test ' + name + ' ... ok\n', '')
        output = output.replace('running 20 tests', 'running 18 tests').replace('20 passed;', '18 passed;')
        output = output.replace(' ... published', ' ... warning: published; directory sync unavailable on this platform\npublished')
        output = output.replace('/tmp/', 'C:\\Users\\runneradmin\\AppData\\Local\\Temp\\').replace('\n', '\r\n')
        gate.result(output, gate.model_tests(windows=True))

    def test_result_rejects_ambiguous_interleaved_records(self):
        good = passed(['alpha'])
        split = good.replace('test alpha ... ok\n', 'test alpha ... published 2418 bytes to /tmp/prop.model.json\nok\n')
        gate.result(split, ['alpha'])
        cases = {
            'nested-inline-start': split.replace('\nok\n', '\ntest alpha ... ok\nok\n'),
            'nested-prefix-payload': split.replace('published 2418 bytes to /tmp/prop.model.json', 'test alpha ... ok'),
            'orphan-extra-completion': good.replace('test alpha ... ok\n', 'test alpha ... ok\nok\n'),
            'orphan-initial-completion': good.replace('test alpha ... ok\n', 'ok\ntest alpha ... ok\n'),
            'duplicate-name': good.replace('test alpha ... ok\n', 'test alpha ... ok\ntest alpha ... ok\n'),
            'unknown-name': split.replace('test alpha ...', 'test unknown ...'),
            'malformed-name': split.replace('test alpha ...', 'test alpha-beta ...'),
            'malformed-prefix': split.replace('test alpha ...', 'test alpha ..'),
            'unfinished-before-summary': split.replace('\nok\n', '\n'),
            'unfinished-without-summary': split.split('test result:', 1)[0].replace('\nok\n', '\n'),
            'missing-summary': split.split('test result:', 1)[0],
            'duplicate-summary': split + split.split('\n\n', 1)[1],
            'malformed-summary': split.replace('finished in 0.01s', 'finished in 0..1s'),
            'wrong-header-count': split.replace('running 1 test', 'running 2 tests'),
            'duplicate-header': split.replace('running 1 test', 'running 1 test\nrunning 1 test'),
            'missing-header': split.replace('running 1 test\n', ''),
            'payload-outside-record': good.replace('test alpha ... ok\n', 'published 2418 bytes\ntest alpha ... ok\n'),
            'payload-after-summary': split + 'published 2418 bytes\n',
            'bare-carriage-return': split.replace('/tmp/prop.model.json', '/tmp/prop\r.model.json'),
        }
        for status in ['FAILED', 'ignored', 'ignored, reason', 'bench', 'ok extra', 'test result: ok. 1 passed', ' test alpha ... ok']:
            cases['reserved-payload-' + status] = split.replace('published 2418 bytes to /tmp/prop.model.json', status)
            cases['reserved-line-' + status] = split.replace('\nok\n', '\n' + status + '\nok\n')
        for label, output in cases.items():
            with self.subTest(label=label), self.assertRaises(AssertionError):
                gate.result(output, ['alpha'])
        for names in [['alpha', 'alpha'], ['alpha-beta'], ['alpha::'], ['0alpha']]:
            with self.subTest(names=names), self.assertRaises(AssertionError):
                gate.result(passed(names), names)

    def test_result_rejects_skip_failure_missing_names_and_counts(self):
        good = passed(['alpha'])
        for value in [good + 'SKIP: unavailable', good.replace(' ... ok', ' ... ignored'),
                      good.replace('1 passed', '0 passed'), good.replace('0 filtered', '1 filtered'),
                      good.replace('alpha', 'other'), good + good, good + 'source_hiding=false']:
            with self.subTest(value=value), self.assertRaises(AssertionError):
                gate.result(value, ['alpha'])

    def producer(self):
        crate = self.repo / 'crates/orr_model'
        (crate / 'examples').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname="orr_model"\n')
        source = crate / 'examples/static_model_extract.rs'; source.write_text('fn main() {}')
        directory = gate.target_dir / 'release/examples'; directory.mkdir(parents=True)
        binary = directory / ('static_model_extract.exe' if os.name == 'nt' else 'static_model_extract')
        binary.write_bytes(b'exact compiler artifact'); binary.chmod(0o755)
        return {'reason': 'compiler-artifact', 'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_model@0.0.1',
                'target': {'name': 'static_model_extract', 'src_path': str(source),
                           'kind': ['example'], 'crate_types': ['bin']},
                'profile': {'test': False, 'opt_level': '3'}, 'features': ['import'],
                'executable': str(binary), 'fresh': False}

    def pin(self, producer, label='producer'):
        return gate.pin(json.dumps(producer) + '\n', label, 'orr_model',
                        'static_model_extract', 'example', ['import'])

    def test_exact_producer(self):
        producer = self.producer(); copied = pathlib.Path(self.pin(producer))
        self.assertEqual(copied.read_bytes(), pathlib.Path(producer['executable']).read_bytes())
        self.assertEqual(gate.report['producers']['producer']['producer'], producer)
        gate.fence()

    def test_cached_or_missing_freshness_is_rejected(self):
        producer = self.producer()
        for value in [True, None, 0, 'false']:
            changed = dict(producer, fresh=value)
            with self.subTest(fresh=value), self.assertRaises(AssertionError):
                self.pin(changed)
        del producer['fresh']
        with self.assertRaises(AssertionError):
            self.pin(producer)

    def test_private_target_cannot_be_reused(self):
        original = gate.target_dir
        self.assertEqual(list(original.iterdir()), [])
        self.assertEqual(os.environ['CARGO_TARGET_DIR'], str(original))
        self.assertTrue(gate.report['private_target']['fresh_empty'])
        with mock.patch.object(gate.uuid, 'uuid4', return_value=mock.Mock(hex='a'*32)):
            fresh = gate.new_target('fixture')
            self.assertNotEqual(fresh, original)
            with self.assertRaises(FileExistsError):
                gate.new_target('fixture')

    def test_producer_rejects_package_source_profile_features_and_path(self):
        producer = self.producer()
        cases = [
            ('manifest_path', str(self.repo / 'outside/Cargo.toml')),
            ('package_id', 'registry+https://example.invalid#orr_model@0.0.1'),
            ('features', ['import', 'animation']),
            ('profile', {'test': True, 'opt_level': '3'}),
            ('profile', {'test': False, 'opt_level': '0'}),
            ('target', dict(producer['target'], src_path=str(self.repo / 'other.rs'))),
            ('target', dict(producer['target'], kind=['test'])),
        ]
        for key, value in cases:
            changed = copy.deepcopy(producer); changed[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(AssertionError):
                self.pin(changed)
        outside = self.root / 'not-a-target'; outside.write_bytes(b'other')
        changed = dict(producer, executable=str(outside))
        with self.assertRaises(AssertionError):
            self.pin(changed)
        with self.assertRaises(AssertionError):
            gate.pin(json.dumps(producer) + '\n' + json.dumps(producer),
                     'duplicate', 'orr_model', 'static_model_extract', 'example', ['import'])

    def test_retained_executable_mutation(self):
        copied = pathlib.Path(self.pin(self.producer()))
        copied.write_bytes(b'changed')
        with self.assertRaises(AssertionError):
            gate.fence()

    def test_source_snapshot_bytes_modes_and_file_types(self):
        path = self.repo / 'source'; path.write_bytes(b'before')
        initial = gate.source_snapshot(['source'])
        path.write_bytes(b'after')
        self.assertNotEqual(initial, gate.source_snapshot(['source']))
        if os.name != 'nt':
            mode = gate.source_snapshot(['source']); path.chmod(0o700)
            self.assertNotEqual(mode, gate.source_snapshot(['source']))
            alias = self.repo / 'alias'; alias.symlink_to(path)
            with self.assertRaises(AssertionError):
                gate.source_snapshot(['alias'])
        path.unlink(); path.mkdir()
        with self.assertRaises(AssertionError):
            gate.source_snapshot(['source'])

    def test_final_revision_and_path_inventory_are_fenced(self):
        with mock.patch.object(gate, 'frozen', {'one': {}, 'two': {}}):
            gate.report.update(head='a'*40, tree='b'*40)
            gate.source_authority('a'*40, 'b'*40, ['one', 'two'])
            for head, tree, paths in [
                ('c'*40, 'b'*40, ['one', 'two']),
                ('a'*40, 'c'*40, ['one', 'two']),
                ('a'*40, 'b'*40, ['one', 'two', 'new']),
                ('a'*40, 'b'*40, ['two', 'one']),
            ]:
                with self.subTest(head=head, tree=tree, paths=paths), self.assertRaises(AssertionError):
                    gate.source_authority(head, tree, paths)

    def test_real_exit_and_durable_receipt(self):
        output = gate.run('expected-two', [sys.executable, '-B', '-c', "print('known'); raise SystemExit(2)"], expected=2)
        self.assertEqual(output.strip(), 'known')
        receipt = json.loads((gate.evidence / 'gate-receipt.json').read_text())['commands'][-1]
        self.assertEqual(receipt['state'], 'passed')
        self.assertEqual(receipt['exit'], 2)
        self.assertTrue(receipt['owned_children_reaped'])
        with self.assertRaises(gate.GateFailure) as caught:
            gate.run('unexpected-seven', [sys.executable, '-B', '-c', 'raise SystemExit(7)'])
        self.assertEqual(caught.exception.code, 7)
        self.assertEqual(gate.report['commands'][-1]['exit'], 7)

    def test_bounded_output_and_failure_receipt(self):
        with mock.patch.object(gate, 'LOG_CAP', 4096):
            with self.assertRaises(gate.GateFailure) as caught:
                gate.run('overflow', [sys.executable, '-B', '-c',
                                     "import sys; sys.stdout.buffer.write(b'x'*1000000)"])
        self.assertEqual(caught.exception.code, 125)
        self.assertEqual((gate.evidence / 'overflow.log').stat().st_size, 4096)
        receipt = gate.report['commands'][-1]
        self.assertEqual(receipt['state'], 'failed')
        self.assertTrue(receipt['owned_children_reaped'])

    def test_bounded_deadline_and_cleanup(self):
        with self.assertRaises(gate.GateFailure) as caught:
            gate.run('deadline', [sys.executable, '-B', '-c', 'import time; time.sleep(60)'], seconds=.15)
        self.assertEqual(caught.exception.code, 124)
        self.assertTrue(gate.report['commands'][-1]['owned_children_reaped'])
        self.assertLess(gate.report['commands'][-1]['seconds'], 15)
        entries = mock.MagicMock()
        entries.__enter__.side_effect = lambda: iter(pathlib.Path(str(i)) for i in range(1, 4))
        with mock.patch.object(gate.os, 'scandir', return_value=entries):
            with self.assertRaises(AssertionError):
                gate.LinuxTree.snapshot(time.monotonic() - 1)
            with mock.patch.object(gate, 'PROCESS_CAP', 2), mock.patch.object(
                    gate.LinuxTree, 'identity', side_effect=lambda pid: {'pid': pid}):
                with self.assertRaises(AssertionError):
                    gate.LinuxTree.snapshot(time.monotonic() + 10)

    def test_success_reaps_descendants(self):
        escaped = self.root / 'escaped'
        descendant = f"import pathlib,time; time.sleep(1); pathlib.Path({str(escaped)!r}).write_text('escaped')"
        parent = ("import subprocess,sys; subprocess.Popen([sys.executable,'-B','-c',"
                  + repr(descendant) + "],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); print('parent complete')")
        gate.run('descendants', [sys.executable, '-B', '-c', parent], seconds=5)
        time.sleep(1.2)
        self.assertFalse(escaped.exists())
        self.assertTrue(gate.report['commands'][-1]['owned_children_reaped'])

    def changed_group(self, label, timeout, session=False, retain_pipe=False):
        escaped = self.root / (label + '-escaped')
        ready = self.root / (label + '-ready')
        descendant = f"import pathlib,time; time.sleep(1.4); pathlib.Path({str(escaped)!r}).write_text('escaped')"
        if not session and os.name != 'nt':
            option = 'preexec_fn=os.setpgrp'
        else:
            option = 'start_new_session=True'
        streams = 'stdin=subprocess.DEVNULL,' if retain_pipe else 'stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,'
        parent = ("import os,pathlib,subprocess,sys,time; child=subprocess.Popen([sys.executable,'-B','-c',"
                  + repr(descendant) + "]," + streams
                  + option + "); pathlib.Path(" + repr(str(ready)) + ").write_text(str(child.pid)); "
                  + ("time.sleep(60)" if timeout and not retain_pipe else "print('parent complete')"))
        if timeout:
            with self.assertRaises(gate.GateFailure) as caught:
                gate.run(label, [sys.executable, '-B', '-c', parent], seconds=.8)
            self.assertEqual(caught.exception.code, 124)
        else:
            gate.run(label, [sys.executable, '-B', '-c', parent], seconds=5)
        self.assertTrue(ready.is_file(), 'fixture must actually launch its descendant')
        time.sleep(1.6)
        self.assertFalse(escaped.exists())
        self.assertTrue(gate.report['commands'][-1]['owned_children_reaped'])
        if os.name != 'nt':
            self.assertIn(int(ready.read_text()), [item['pid'] for item in gate.report['commands'][-1]['owned_descendants']])

    def test_success_reaps_separate_session_descendants(self):
        self.changed_group('separate-session', timeout=False, session=True)

    def test_timeout_reaps_changed_group_descendants(self):
        self.changed_group('changed-group-timeout', timeout=True)

    def test_success_reaps_changed_group_descendants(self):
        self.changed_group('changed-group-success', timeout=False)

    def test_timeout_reaps_separate_session_and_retained_pipe(self):
        self.changed_group('separate-session-pipe-timeout', timeout=True, session=True, retain_pipe=True)

    def gpu_fixture(self):
        root = self.root / 'gpu'; root.mkdir()
        def save(name, data):
            (root / name).write_bytes(data)
            value = 0xcbf29ce484222325
            for byte in data:
                value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
            return {'file': name, 'bytes': len(data), 'fnv1a64': f'{value:016x}'}
        selected = b'\x01\x02\x03\xff' * (256 * 256)
        full = b''.join(selected[row*1024:row*1024+512] + b'\x14\x15\x16\xff'*128 for row in range(256))
        variants = {'full-scene': full, 'extracted-subtree': selected,
                    'wrong-transform': b'\x07\x08\x09\xff'*(256*256),
                    'wrong-uv': b'\x0a\x0b\x0c\xff'*(256*256),
                    'wrong-material': b'\x0d\x0e\x0f\xff'*(256*256)}
        frames = {}
        for name, data in variants.items():
            frames[name.replace('-', '_')] = {
                'raw': save(name + '.rgba', data),
                'image': save(name + '.ppm', b'P6\n256 256\n255\n' + bytes(v for i, v in enumerate(data) if i % 4 != 3))}
        adapter = {'software': True, 'backend': 'Vulkan'}
        (root / 'adapter.json').write_text(json.dumps(adapter))
        accepted = {'schema': 1, 'status': 'passed', 'width': 256, 'height': 256,
                    'selected_node': 1, 'selected_primitives': 4, 'source_primitives': 8,
                    'adapter': adapter, 'frames': frames,
                    'assets': {'source': save('source.glb', b'fixture glb'), 'cooked': save('selected.model.json', b'{}'),
                               'source_hidden_after_install': True, 'cooked_reload_before_render': True, 'installed_files': 1},
                    'selected_left_half_byte_identical': True,
                    'tolerances': {'color_bytes': 3, 'minimum_control_differences': 500},
                    'control_pixel_differences': {'omitted_geometry': 32768, 'world_transform': 65536,
                                                 'asymmetric_uv': 65536, 'material_slot': 65536}}
        (root / 'receipt.json').write_text(json.dumps(accepted))
        return root, accepted

    def test_gpu_capture_census_and_pixels(self):
        root, _ = self.gpu_fixture(); gate.check_gpu(root)
        (root / 'unexpected').write_bytes(b'not allowed')
        with self.assertRaises(AssertionError):
            gate.check_gpu(root)
        (root / 'unexpected').unlink()
        (root / 'wrong-uv.rgba').write_bytes(b'wrong dimensions')
        with self.assertRaises(AssertionError):
            gate.check_gpu(root)

    def test_gpu_adapter_hash_and_control_negatives(self):
        root, accepted = self.gpu_fixture()
        cases = [
            dict(accepted, adapter={'software': False, 'backend': 'Vulkan'}),
            dict(accepted, control_pixel_differences=dict(accepted['control_pixel_differences'], asymmetric_uv=0)),
            dict(accepted, assets=dict(accepted['assets'], source_hidden_after_install=False)),
            dict(accepted, frames=dict(accepted['frames'], wrong_uv={'raw':dict(accepted['frames']['wrong_uv']['raw'], fnv1a64='0'*16),
                                                                  'image':accepted['frames']['wrong_uv']['image']})),
        ]
        for changed in cases:
            (root / 'receipt.json').write_text(json.dumps(changed))
            with self.assertRaises(AssertionError):
                gate.check_gpu(root)

    def export_fixture(self):
        root = self.root / 'export'; root.mkdir()
        source_path = self.repo / 'crates/orr_sample/tests/static_model_extract_export.rs'
        source_path.parent.mkdir(parents=True); source_path.write_text('current source')
        (root / 'authored-project').mkdir()
        (root / 'authored-project/chair.model.json').write_bytes(b'{}')
        def chunk(kind, data):
            return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
        png = (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1024, 768, 8, 6, 0, 0, 0))
               + chunk(b'IDAT', zlib.compress((b'\0' + b'\0\0\0\xff'*1024)*768)) + chunk(b'IEND', b''))
        observations = {}
        for name, ticks, held in [('initial', 0, ''), ('moved', 18, 'right'), ('collision', 180, 'right'), ('interacted', 18, 'right,interact')]:
            text = f'room capture adapter: fixture\nroom tick: {ticks} checksum: 0x123\n'
            observation = {'ticks': ticks, 'held': held, 'checksum': '0x123', 'source_stdout': text, 'export_stdout': text}
            for route in ['source', 'export']:
                for suffix, stem, data in [('png', 'runtime', png), ('frame', 'prepared-project', name.encode())]:
                    path = root / f'{stem}-{name}-{route}.{suffix}'; path.write_bytes(data)
                    observation[f'{route}_{suffix}_sha256'] = gate.digest(path)
            observations[name] = observation
        for name in ['control-no-model.png', 'control-whole-scene.png']:
            (root / name).write_bytes(png)
        (root / 'scene.glb').write_bytes(b'GLB')
        (root / 'chair.model.json').write_bytes(b'{}')
        (root / 'orr.export.json').write_bytes(b'{}')
        accepted = {'source_hidden': True, 'read_only_bundle': True, 'selected_changed_pixels': 21,
                    'omitted_changed_pixels': 21, 'observations': observations}
        for field, label in [('runtime_sha256', 'room_escape'), ('exporter_sha256', 'orr_export_room'),
                             ('extractor_sha256', 'extractor'), ('acceptance_executable_sha256', 'export-test')]:
            accepted[field] = hashlib.sha256(label.encode()).hexdigest()
            gate.report['producers'][label] = {'sha256': accepted[field]}
        (root / 'production-receipt.json').write_text(json.dumps(accepted))
        source = {'test_source_sha256': gate.digest(source_path), 'asset_id': 'models/scene.glb', 'node': 3,
                  'selected_primitives': 2, 'full_scene_primitives': 4, 'retained_materials': 2, 'retained_images': 2,
                  'unreferenced_material_slot_retained': 0, 'original_source_digest_retained': True,
                  'source_sha256': gate.digest(root / 'scene.glb'), 'cooked_sha256': gate.digest(root / 'chair.model.json'),
                  'initial_frame_sha256': gate.digest(root / 'prepared-project-initial-source.frame')}
        (root / 'acceptance-source.json').write_text(json.dumps(source))
        self.export_inventory(root)
        return root, accepted

    def export_inventory(self, root):
        inventory = {str(path.relative_to(root)): [path.stat().st_size, gate.digest(path)]
                     for path in root.rglob('*') if path.is_file() and path.name != 'capture-inventory.json'}
        (root / 'capture-inventory.json').write_text(json.dumps(inventory))

    def test_export_capture_inventory_and_provenance(self):
        root, _ = self.export_fixture()
        gate.check_export(root)

    def test_export_rejects_source_producer_frame_and_inventory_mutations(self):
        root, accepted = self.export_fixture()
        for changed in [dict(accepted, source_hidden=False),
                        dict(accepted, runtime_sha256='0'*64), dict(accepted, selected_changed_pixels=0)]:
            (root / 'production-receipt.json').write_text(json.dumps(changed)); self.export_inventory(root)
            with self.assertRaises(AssertionError):
                gate.check_export(root)
        (root / 'production-receipt.json').write_text(json.dumps(accepted)); self.export_inventory(root)
        frame = root / 'prepared-project-initial-export.frame'; frame.write_bytes(b'different')
        self.export_inventory(root)
        with self.assertRaises(AssertionError):
            gate.check_export(root)
        frame.write_bytes(b'initial'); self.export_inventory(root)
        (root / 'scene.glb').write_bytes(b'changed without inventory update')
        with self.assertRaises(AssertionError):
            gate.check_export(root)

    def test_workflow_required_modes_and_artifacts(self):
        workflow_contract((ROOT / '.github/workflows/determinism.yml').read_text())

    def test_workflow_rejects_disabled_or_unretained_modes(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        for old, new in [
            ('run: python -B tools/static_model_extract_gate.py gpu', 'run: echo skipped'),
            ('run: python -B tools/static_model_extract_gate.py export', 'run: echo skipped'),
            ('Retain static prop CPU and partial failure evidence', 'missing retention'),
            ('os: [ubuntu-22.04, windows-latest]', 'os: [ubuntu-22.04]'),
            ('needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, sample-linked-prefabs, sample-room]', 'needs: [native]'),
            ('sudo apt-get update -q && sudo apt-get install -y -q bubblewrap', 'echo namespace skipped'),
            ("      - if: runner.os == 'Linux'\n        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n",
             "      - if: runner.os == 'Linux'\n        run: echo Vulkan skipped\n"),
        ]:
            with self.subTest(old=old), self.assertRaises(AssertionError):
                workflow_contract(text.replace(old, new, 1))
        first = text.index('      - name: Required static prop source-hidden Room export acceptance\n')
        last = text.index('      - name: Mandatory authored runtime compositor readback\n', first)
        pair = text[first:last]
        without = text[:first] + text[last:]
        for destination in ['  sample-room:\n', '  sample-build:\n']:
            relocated = without.replace(destination, destination + pair, 1)
            with self.subTest(destination=destination), self.assertRaises(AssertionError):
                workflow_contract(relocated)

def main():
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(StaticPropGateTests)
    names = sorted(test.id().rsplit('.', 1)[-1] for test in suite)
    assert names == sorted(EXPECTED_TESTS) and len(names) == 29, 'exact checker inventory'
    result = unittest.TextTestRunner(verbosity=2, failfast=True).run(suite)
    if result.testsRun != 29 or result.skipped or result.expectedFailures or result.unexpectedSuccesses or not result.wasSuccessful():
        raise SystemExit(1)
    print('STATIC_PROP_GATE_FIXTURES: 29 passed; 0 failed; 0 skipped', flush=True)

if __name__ == '__main__':
    main()

