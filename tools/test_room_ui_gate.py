#!/usr/bin/env python3
"""Semantic positive/negative checker fixtures, never fresh Rust/GPU proof."""
import copy
import json
from pathlib import Path
import re
import shutil
import tempfile
import tomllib
import unittest

import room_ui_gate as gate
import test_room_template_gate as fixtures

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = json.loads(gate.INVENTORY.read_text())


def document():
    return {'schema': 1, 'nodes': [
        {'id': binding, 'parent': None, 'kind': {'type': 'label', 'text': 'Room', 'binding': binding},
         'screen': 'playing', 'anchor': [0, 0], 'offset': [0, index * 24], 'size': [160, 24]}
        for index, binding in enumerate(('key_acquired', 'exit_state', 'room_phase'))]}


def make_evidence(root):
    evidence = fixtures.RoomTemplateGateTests().make_evidence(root)
    shutil.rmtree(evidence / 'captures/editor')
    ui = evidence / 'captures/room-ui'; ui.mkdir()
    for width, height in gate.SIZES:
        for state, shade in zip(gate.STATES, (40, 60, 80, 100)):
            (ui / f'{state}-{width}x{height}.png').write_bytes(fixtures.png(width, height, shade))
    project = evidence / 'export/authored-project'
    path = evidence / 'export/orr.export.json'
    manifest = json.loads(path.read_text()); payload = manifest['payload']
    payload['entry']['ui'] = copy.deepcopy(gate.UI)
    payload['packages'] = {'korean-game-ui': 'a' * 64, 'sample-imported-scene': 'b' * 64}
    (project / 'orr.project.json').write_text(json.dumps({'schema': 2, 'engine': '0.0.1', 'entry': payload['entry']}))
    (project / 'room.ui.json').write_text(json.dumps(document()))
    font_dir = project / '.orr/packages/objects' / ('a' * 64)
    font_dir.mkdir(parents=True)
    for name in ('COPYRIGHT.txt', 'OFL.txt', 'OrreryKoreanUI.otf', 'corpus.txt', 'font-manifest.json', 'orr.package.json'):
        shutil.copyfile(ROOT / 'assets/game_ui_font' / name, font_dir / name)
        payload['files'].append({'path': 'project/' + str((font_dir / name).relative_to(project)),
                                 'role': 'package_manifest' if name == 'orr.package.json' else 'package_asset', 'mode': 0o644})
    payload['files'].append({'path': 'project/room.ui.json', 'role': 'ui_document', 'mode': 0o644})
    for item in payload['files']:
        source = project / item['path'].removeprefix('project/')
        item.update(bytes=source.stat().st_size, sha256=gate.shared.digest(source))
    path.write_text(json.dumps(manifest))
    return evidence


class RoomUiGateTests(unittest.TestCase):
    def test_source_inventory_cfg_and_feature_closure(self):
        paths = {'authored_ui': 'orr_sample/src/authored_ui.rs', 'collect_ui': 'orr_sample/src/collect_ui.rs',
                 'room_app': 'orr_sample/src/room_app.rs', 'collect_ui_panel': 'orr_editor/src/collect_ui_panel.rs'}
        script = (ROOT / 'tools/check-room-ui.sh').read_text()
        for lane, expected in INVENTORY.items():
            if expected['target'] == '--lib':
                file, module = expected['filter'].rstrip(':').split('::')
                source = (ROOT / 'crates' / paths[file]).read_text().split('mod ' + module + ' {', 1)[1].split('\nmod ', 1)[0]
                prefix = file + '::' + module + '::'
            else:
                test = expected['target'].split()[-1]
                source = (ROOT / 'crates' / expected['package'] / 'tests' / (test + '.rs')).read_text()
                prefix = ''
                if lane == 'room-ui-source-hidden-export':
                    self.assertIn('#[cfg(feature = "room-ui")]\n    #[test]\n    #[ignore', source)
                    source = source[source.index('    #[cfg(feature = "room-ui")]'):source.index('    // Camera offsets')]
                    prefix = 'room::'
            found = [(prefix + name, '#[ignore' in attrs) for attrs, name in re.findall(r'#\[test\]\s*((?:#\[[^\n]*\]\s*)*)fn\s+(\w+)\s*\(', source)]
            if expected.get('execution') == 'exact':
                found = [(name, ignored) for name, ignored in found if name == expected['filter']]
            self.assertEqual(expected['names'], sorted(n for n, _ in found), lane)
            self.assertEqual(expected['ignored_names'], sorted(n for n, ignored in found if ignored), lane)
            self.assertEqual(expected['expected_summary_rows'], [[len(found), 0, 0]], lane)
            execution = expected.get('execution', 'ignored' if expected['run_ignored'] else 'normal')
            self.assertIn(f"run_test {lane} {execution} cargo test", script)
            feature_args = f"--features {expected['features']} " if expected['features'] else ''
            self.assertIn(feature_args + expected['target'], script)
        features = tomllib.loads((ROOT / 'crates/orr_sample/Cargo.toml').read_text())['features']
        closure = {'default', *gate.REQUESTED_FEATURES}; pending = list(closure)
        while pending:
            for value in features[pending.pop()]:
                if value in features and value not in closure: closure.add(value); pending.append(value)
        self.assertEqual(closure, gate.FEATURES)
        self.assertEqual(features['default'], [])
        self.assertEqual(sum(len(v['names']) for v in INVENTORY.values() if not v['run_ignored']), 40)
        self.assertIn('test "$(id -u)" -ne 0', script)
        self.assertIn('namespace-preflight', script)
        self.assertIn('--message-format=json', script)
        self.assertNotIn('target/release/', script)
        all_gpu = set()
        for name in ('room-project', 'room-template', 'room-ui'):
            rows = json.loads((ROOT / 'tools' / (name + '-inventories.json')).read_text())
            all_gpu.update(row['names'][0] for row in rows.values() if row['run_ignored'])
        self.assertEqual(len(all_gpu), 8)

    def test_save_aba_producer_and_lifecycle_are_separate_mandatory_positive_gates(self):
        producer = INVENTORY['room-ui-save-transition-producer']
        lifecycle = INVENTORY['room-ui-editor-source-lifecycle']
        self.assertEqual(producer['names'], ['scene_save_path_transition_flag_is_exact_in_list_and_watch'])
        self.assertEqual(producer['expected_summary_rows'], [[1, 0, 0]])
        self.assertEqual(producer['execution'], 'exact')
        self.assertEqual(producer['features'], '')
        self.assertEqual(lifecycle['expected_summary_rows'], [[3, 0, 0]])
        aba = 'actual_external_save_aba_before_editor_pump_permanently_retires_room_ui'
        self.assertIn(aba, lifecycle['names'])
        script = (ROOT / 'tools/check-room-ui.sh').read_text()
        self.assertIn('if [[ "$execution" == exact ]]; then options+=(--exact); fi', script)
        self.assertIn('run_test room-ui-save-transition-producer exact cargo test --release --locked -j2 -p orr_remote --test activity ' + producer['filter'], script)
        self.assertIn('run_test room-ui-editor-source-lifecycle normal cargo test --release --locked -j2 -p orr_editor --features room-ui,project-create --test room_ui_lifecycle', script)
        for expected in (producer, lifecycle):
            valid = fixtures.log_for(expected)
            gate.check_log(expected, 'result', valid)
            with self.assertRaises(AssertionError):
                gate.check_log(expected, 'result', 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')
        historical = copy.deepcopy(lifecycle)
        historical['names'].remove(aba)
        historical['expected_summary_rows'] = [[2, 0, 0]]
        for mode in ('list', 'result'):
            old = ''.join(name + ': test\n' for name in historical['names']) if mode == 'list' else fixtures.log_for(historical)
            with self.assertRaises(AssertionError): gate.check_log(lifecycle, mode, old)

    def test_exact_outcomes_counts_and_skip_rejection(self):
        for lane, expected in INVENTORY.items():
            valid = fixtures.log_for(expected)
            gate.check_log(expected, 'result', valid)
            for mode, names in [('list', expected['names']), ('ignored', expected['ignored_names'])]:
                listing = ''.join(name + ': test\n' for name in names)
                gate.check_log(expected, mode, listing)
                for bad in [listing + 'extra: test\n', listing + 'malformed name: test\n']:
                    with self.assertRaises(AssertionError): gate.check_log(expected, mode, bad)
            for bad in [valid.replace(' ... ok', ' ... FAILED', 1), valid.replace(' ... ok', ' ... ignored', 1),
                        valid.replace(expected['names'][0], 'wrong', 1), valid.replace('0 failed;', '1 failed;', 1),
                        valid.replace('finished in 0.01s', 'malformed'), valid + 'test extra ... ok\n',
                        valid + 'test result: ok. 0 passed; 0 failed; 0 ignored;\n', valid + 'source_hiding=false\n',
                        valid + 'SKIP: no GPU\n', '\n'.join(valid.splitlines()[1:])]:
                with self.assertRaises(AssertionError): gate.check_log(expected, 'result', bad)

    def test_ui_artifacts_require_exact_capability_and_default_stays_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); rows = fixtures.artifacts(root / 'repo', root / 'out')
            for row in rows[:-1]: row['features'] = sorted(gate.FEATURES)
            self.assertEqual(len(gate.shared.check_artifacts(rows, root / 'repo', gate.FEATURES)), 3)
            with self.assertRaises(AssertionError): gate.shared.check_artifacts(rows, root / 'repo')
            for change in [lambda r: r[0]['features'].remove('room-ui'), lambda r: r[0]['features'].append('collect-ui'),
                           lambda r: r[0]['profile'].update(test=True), lambda r: r[-1].update(success=False),
                           lambda r: r[0].update(executable=r[1]['executable'])]:
                bad = copy.deepcopy(rows); change(bad)
                with self.assertRaises(AssertionError): gate.shared.check_artifacts(bad, root / 'repo', gate.FEATURES)
            evidence = root / 'evidence'; evidence.mkdir()
            (evidence / 'production-tools.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))
            gate.shared.pin_tools(evidence, root / 'repo', gate.FEATURES, gate.REQUESTED_FEATURES)
            records = gate.shared.verify_tools(evidence)
            self.assertTrue(all(row['requested_features'] == gate.REQUESTED_FEATURES and set(row['features']) == gate.FEATURES for row in records))

    def test_ui_shapes_duplicate_keys_cross_profiles_and_closed_bounds(self):
        valid = document(); raw = json.dumps(valid).encode(); self.assertEqual(gate.check_ui(raw), valid)
        self.assertEqual(gate.check_ui(raw + b' ' * (65536-len(raw))), valid)
        bads = [b'', b'[]', b'[1,[]]', b'null', raw + b' ' * 65536,
                raw.replace(b'"schema": 1', b'"schema": 1,"schema": 1'),
                raw.replace(b'"size": [160, 24]', b'"size": [true, 24]'),
                raw.replace(b'"key_acquired"', b'"score"'),
                raw.replace(b'"binding": "key_acquired"', b'"binding": NaN')]
        for mutate in [lambda x: x.update(schema=True), lambda x: x.update(callback='run'),
                       lambda x: x.update(nodes=[list(x['nodes'][0].values())]),
                       lambda x: x['nodes'][0].update(kind=['label', 'Room', None]),
                       lambda x: x['nodes'][0].update(parent='later'), lambda x: x['nodes'][0].update(size=[0, 1]),
                       lambda x: x['nodes'][0].update(anchor=[1001, 0]), lambda x: x['nodes'][0].update(offset=[-4097, 0]),
                       lambda x: x['nodes'][0]['kind'].update(text='x\n'), lambda x: x['nodes'][0]['kind'].update(text='\ud800'), lambda x: x['nodes'].append(copy.deepcopy(x['nodes'][0]))]:
            changed = copy.deepcopy(valid); mutate(changed); bads.append(json.dumps(changed).encode())
        for bad in bads:
            with self.assertRaises((AssertionError, ValueError)): gate.check_ui(bad)

    def test_eight_composited_states_dimensions_and_real_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for size in gate.SIZES:
                for state, shade in zip(gate.STATES, (40, 60, 80, 100)):
                    (root / f'{state}-{size[0]}x{size[1]}.png').write_bytes(fixtures.png(*size, shade))
            self.assertEqual(len(gate.ui_captures(root)), 8)
            path = root / 'key-480x800.png'; good = path.read_bytes()
            for bad in [fixtures.png(480, 800, 60), fixtures.png(1024, 768, 80), fixtures.png(480, 800, blank=True)]:
                path.write_bytes(bad)
                with self.assertRaises(AssertionError): gate.ui_captures(root)
            path.write_bytes(good); (root / 'extra.png').write_bytes(good)
            with self.assertRaises(AssertionError): gate.ui_captures(root)

    def test_full_ui_export_closure_rejects_valid_hash_malformed_ui_and_missing_font(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = make_evidence(Path(directory)); export = evidence / 'export'
            gate.check_captures(evidence)
            self.assertEqual(len(json.loads((evidence / 'capture-manifest.json').read_text())), 15)
            with self.assertRaises(AssertionError): gate.shared.check_export(evidence, export)
            manifest_path = export / 'orr.export.json'; good_manifest = manifest_path.read_text()
            ui_path = export / 'authored-project/room.ui.json'; good_ui = ui_path.read_bytes()
            for data in [b'[1,[]]', json.dumps({'schema': 1, 'nodes': [list(document()['nodes'][0].values())]}).encode()]:
                ui_path.write_bytes(data); manifest = json.loads(good_manifest)
                record = next(row for row in manifest['payload']['files'] if row['role'] == 'ui_document')
                record.update(bytes=len(data), sha256=gate.shared.digest(ui_path)); manifest_path.write_text(json.dumps(manifest))
                with self.assertRaises(AssertionError): gate.check_ui_export(evidence, export)
            ui_path.write_bytes(good_ui)
            for mutate in [lambda p: p['entry']['ui'].update(profile='collect-authored-v1'),
                           lambda p: p.update(files=[r for r in p['files'] if not r['path'].endswith('/OFL.txt')]),
                           lambda p: p['files'].append(copy.deepcopy(next(r for r in p['files'] if r['role']=='ui_document'))),
                           lambda p: next(r for r in p['files'] if r['role']=='ui_document').update(mode=0o755),
                           lambda p: p['runtime'].update(sha256='0'*64)]:
                manifest = json.loads(good_manifest); mutate(manifest['payload']); manifest_path.write_text(json.dumps(manifest))
                with self.assertRaises(AssertionError): gate.check_ui_export(evidence, export)


if __name__ == '__main__':
    unittest.main(verbosity=2)
