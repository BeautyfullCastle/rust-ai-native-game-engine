#!/usr/bin/env python3
"""Semantic fixtures for the actual checker; never count these as Rust/GPU runs."""
import copy
import json
import os
from pathlib import Path
import re
import struct
import tempfile
import unittest
import zlib

import room_template_gate as gate

ROOT = Path(__file__).resolve().parents[1]
INVENTORIES = json.loads(gate.INVENTORY.read_text())


def log_for(expected):
    rows = ['test ' + name + ' ... ' + ('ignored, explicit GPU' if name in expected['ignored_names'] and not expected['run_ignored'] else 'ok')
            for name in expected['names']]
    rows += [f'test result: ok. {passed} passed; {failed} failed; {ignored} ignored; 0 measured; 0 filtered out; finished in 0.01s'
             for passed, failed, ignored in expected['expected_summary_rows']]
    return '\n'.join(rows) + '\n'


def png(width=1024, height=768, shade=40, blank=False):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload)) + kind + payload + struct.pack('>I', zlib.crc32(kind + payload) & 0xffffffff)
    background = bytes((0, 0, 0, 255))
    foreground = background if blank else bytes((shade, 80, 120, 255))
    row = b'\0' + background * (width // 2) + foreground * (width - width // 2)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(row * height)) + chunk(b'IEND', b''))


def artifacts(repo, output):
    manifest = repo / 'crates/orr_sample/Cargo.toml'
    output.mkdir(parents=True)
    rows = []
    for name in gate.TOOLS:
        path = output / name
        path.write_bytes(b'fixture executable ' + name.encode())
        path.chmod(0o755)
        rows.append({'reason': 'compiler-artifact', 'manifest_path': str(manifest),
                     'package_id': 'path+' + manifest.parent.as_uri() + '#orr_sample@0.0.1',
                     'target': {'name': name, 'kind': ['bin'], 'crate_types': ['bin'],
                                'src_path': str(manifest.parent / 'src/bin' / (name + '.rs')),
                                'edition': '2021', 'doc': True, 'doctest': False, 'test': True},
                     'features': sorted(gate.FEATURES),
                     'profile': {'test': False, 'opt_level': '3', 'debuginfo': 0,
                                 'debug_assertions': False, 'overflow_checks': False},
                     'filenames': [str(path)], 'executable': str(path), 'fresh': False})
    rows.append({'reason': 'build-finished', 'success': True})
    return rows


def camera_document():
    return {'schema': 1, 'target': [0.0, 0.5, 0.0], 'yaw': 0.65, 'pitch': 1.02, 'distance': 24.0,
            'projection': {'type': 'orthographic', 'half_height': 8.0}}


class RoomTemplateGateTests(unittest.TestCase):
    def test_exact_source_derived_creator_and_acceptance_inventories(self):
        source = (ROOT / 'crates/orr_sample/src/project_create/tests.rs').read_text()
        collect_start = source.index('#[cfg(feature = "collect-dodge")]\nmod collect {')
        ui_start = source.index('#[cfg(feature = "collect-ui")]\nmod collect_ui_template_tests {')
        disabled_ui = source.index('#[cfg(all(feature = "collect-dodge", not(feature = "collect-ui")))]')
        disabled_room = source.index('#[cfg(not(feature = "room-project"))]\n#[test]\nfn room_template_requires_feature_before_staging()')
        room_start = source.index('#[cfg(feature = "room-project")]\nmod room_template_tests {')
        prefix = 'project_create::tests::'
        base = [prefix + name for name in re.findall(r'^#\[test\]\nfn (\w+)\(', source[:collect_start], re.M)]
        def nested(start, end, module):
            return [prefix + module + '::' + name for name in re.findall(r'^    #\[test\]\n    fn (\w+)\(', source[start:end], re.M)]
        collect = nested(collect_start, ui_start, 'collect')
        ui = nested(ui_start, disabled_ui, 'collect_ui_template_tests')
        room = nested(room_start, len(source), 'room_template_tests')
        self.assertEqual((len(base), len(collect), len(ui), len(room)), (13, 4, 3, 3))
        disabled = [prefix + name for name in re.findall(r'^#\[test\]\nfn (\w+)\(', source[disabled_room:room_start], re.M)]
        audio_source = (ROOT / 'crates/orr_sample/src/project_create/audio_tests.rs').read_text()
        audio_attributes = dict((name, attributes) for attributes, name in re.findall(
            r'^((?:#\[[^\n]+\]\n)+)fn (\w+)\(', audio_source, re.M))
        self.assertEqual(audio_attributes, {
            'original_arena_and_collect_templates_do_not_gain_audio_implicitly': '#[test]\n',
            'missing_audio_feature_rejects_template_before_any_stage_exists': '#[cfg(not(feature = "collect-audio"))]\n#[test]\n',
        })
        self.assertIn('#[cfg(test)]\nmod audio_tests;',
                      (ROOT / 'crates/orr_sample/src/project_create.rs').read_text())
        audio = ['project_create::audio_tests::' + name for name in audio_attributes]
        for lane, names, count in [('creator-default', base + disabled + audio, 16), ('creator-room', base + room + audio, 18), ('creator-joint', base + room + collect + ui + audio, 25)]:
            self.assertEqual(INVENTORIES[lane]['names'], sorted(names))
            self.assertEqual(INVENTORIES[lane]['expected_summary_rows'], [[count, 0, 0]])
        sample = (ROOT / 'crates/orr_sample/tests/new_room_project.rs').read_text()
        self.assertIn('#[cfg(not(feature = "room-project"))]', sample)
        self.assertIn('#[cfg(feature = "room-project")]\nmod room {', sample)
        sample = re.sub(r'    #\[cfg\(feature = "room-ui"\)\]\n    #\[test\]\n    #\[ignore[^\n]*\]\n    fn generated_room_ui_real_export_source_hidden_gpu_workflow\(\)', '', sample)
        sample_names = re.findall(r'^    #\[test\]\n(?:    #\[ignore[^\n]*\]\n)?    fn (\w+)\(', sample, re.M)
        self.assertEqual(INVENTORIES['sample-room']['names'], sorted('room::' + name for name in sample_names))
        editor = (ROOT / 'crates/orr_editor/tests/new_room_project.rs').read_text()
        editor_names = re.findall(r'^#\[test\]\n(?:#\[ignore[^\n]*\]\n)?fn (\w+)\(', editor, re.M)
        self.assertEqual(INVENTORIES['editor-room']['names'], sorted(editor_names))
        cli = (ROOT / 'crates/orr_sample/tests/new_arena_cli.rs').read_text()
        cli_names = re.findall(r'^#\[test\]\nfn (\w+)\(', cli, re.M)
        self.assertEqual(INVENTORIES['arena-room']['names'], sorted(set(cli_names) - {'collect_cli_requires_identity_and_matches_library'}))
        self.assertEqual(INVENTORIES['arena-joint']['names'], sorted(set(cli_names) - {'generator_without_collect_feature_rejects_collect_template'}))

    def test_production_feature_identity_comes_from_current_cargo_features(self):
        source = (ROOT / 'crates/orr_sample/Cargo.toml').read_text()
        section = source.split('[features]\n', 1)[1].split('\n[dependencies]', 1)[0]
        features = {name: json.loads(values) for name, values in re.findall(r'^([a-z][a-z0-9-]*) = (\[[^\n]*\])$', section, re.M)}
        closure = {'default', 'project-create', 'project-export', 'room-project'}
        pending = list(closure)
        while pending:
            for dependency in features[pending.pop()]:
                if dependency in features and dependency not in closure:
                    closure.add(dependency)
                    pending.append(dependency)
        self.assertEqual(closure, gate.FEATURES)
        editor = (ROOT / 'crates/orr_editor/Cargo.toml').read_text()
        self.assertEqual(re.findall(r'^project-create = (\[[^\n]*\])$', editor, re.M), ['["orr_sample/project-create"]'])

    def test_all_lane_logs_require_exact_executions_and_summary_values(self):
        for lane, expected in INVENTORIES.items():
            with self.subTest(lane=lane):
                valid = log_for(expected)
                gate.check_log(expected, 'result', valid)
                for mode, names in [('list', expected['names']), ('ignored', expected['ignored_names'])]:
                    listed = ''.join(name + ': test\n' for name in names)
                    gate.check_log(expected, mode, listed)
                    with self.assertRaises(AssertionError):
                        gate.check_log(expected, mode, listed + 'unexpected: test\n')
                    with self.assertRaises(AssertionError):
                        gate.check_log(expected, mode, listed + 'malformed name: test\n')
                invalids = [valid + 'test result: FAILED. 1 passed; 1 failed; 0 ignored;\n',
                            valid + 'test result: bad summary\n', valid + 'test result: ok. 0 passed; 0 failed; 0 ignored;\n',
                            valid.replace('test result: ok.', 'test result: FAILED.'),
                            valid.replace(expected['names'][0], 'wrong_test_name', 1),
                            valid.replace('0 failed;', '1 failed;'),
                            valid.replace(f"{expected['expected_summary_rows'][0][0]} passed;", '0 passed;'),
                            valid.replace(' ... ok', ' ... ignored', 1),
                            valid + 'test injected_extra ... ok\n',
                            '\n'.join(valid.splitlines()[1:]) + '\n']
                invalids += [valid + diagnostic + '\n' for diagnostic in ['SKIP: GPU', 'skipping unavailable GPU', 'source_hiding=false', 'without source hiding', 'software GPU unavailable']]
                for invalid in invalids:
                    with self.assertRaises(AssertionError):
                        gate.check_log(expected, 'result', invalid)

    def test_real_artifact_fields_and_unique_executables_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = artifacts(root, root / 'target/release')
            self.assertEqual(len(gate.check_artifacts(rows, root)), 3)
            modifications = [
                lambda r: r.pop(0), lambda r: r.append(copy.deepcopy(r[0])),
                lambda r: r[-1].update(success=False), lambda r: r.append(copy.deepcopy(r[-1])),
                lambda r: r[0]['features'].remove('room-project'), lambda r: r[0]['features'].append('collect-ui'),
                lambda r: r[0]['profile'].update(test=True), lambda r: r[0]['profile'].update(opt_level='0'),
                lambda r: r[0]['target'].update(kind=['lib']), lambda r: r[0]['target'].update(crate_types=['lib']),
                lambda r: r[0]['target'].update(src_path=str(root / 'wrong.rs')),
                lambda r: r[0].update(manifest_path=str(root / 'wrong/Cargo.toml')),
                lambda r: r[0].update(package_id='path+file:///wrong#orr_sample@0.0.1'),
                lambda r: r[0].update(executable=r[1]['executable']),
                lambda r: r[-1].update(success=1), lambda r: r[-1].update(success='true'),
                lambda r: r[0]['features'].append(r[0]['features'][0]),
                lambda r: r[0]['profile'].update(test=0), lambda r: r[0]['profile'].update(opt_level=3),
                lambda r: r[0].update(filenames=[]), lambda r: r[0].update(fresh=1),
                lambda r: r[0].update(unknown='ignored before'),
                lambda r: r[0]['target'].update(unknown=True), lambda r: r[0]['profile'].update(unknown=True),
                lambda r: r[0]['target'].pop('src_path'), lambda r: r[0].update(target=[]),
                lambda r: r[0].update(features='room-project'), lambda r: r[0].update(package_id=None),
                lambda r: r.insert(0, {'reason': 'unrecognized-record'}),
                lambda r: r.insert(0, {'reason': 'build-script-executed'}),
                lambda r: r.insert(0, {'reason': 'compiler-message', 'message': False}),
                lambda r: r.insert(0, []), lambda r: r.insert(0, {'reason': []}),
                lambda r: r.reverse(),
            ]
            for modify in modifications:
                altered = copy.deepcopy(rows)
                modify(altered)
                with self.assertRaises(AssertionError):
                    gate.check_artifacts(altered, root)

    def test_supported_unrelated_cargo_records_are_accepted(self):
        # The Cargo Book's external-tools JSON schema includes all four reasons.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = artifacts(root, root / 'target/release')
            message = {'reason': 'compiler-message', 'package_id': rows[0]['package_id'],
                       'manifest_path': rows[0]['manifest_path'], 'target': copy.deepcopy(rows[0]['target']),
                       'message': {'message': 'fixture warning', 'code': None, 'level': 'warning',
                                   'spans': [], 'children': [], 'rendered': 'warning: fixture\n'}}
            message['target']['required-features'] = ['room-project']
            script = {'reason': 'build-script-executed', 'package_id': 'registry+https://example.invalid/index#fixture@1.0.0',
                      'linked_libs': ['static=fixture'], 'linked_paths': ['native=/tmp/fixture'],
                      'cfgs': ['fixture'], 'env': [['FIXTURE', 'value']], 'out_dir': str(root / 'out')}
            dependency = copy.deepcopy(rows[0])
            dependency.update(package_id=script['package_id'], manifest_path=str(root / 'dependency/Cargo.toml'),
                              features=[], executable=None, filenames=[str(root / 'libfixture.rlib')], fresh=True)
            dependency['target'].update(name='fixture', kind=['lib'], crate_types=['lib'])
            for debug in (None, 0, 1, 2, 'line-directives-only', 'line-tables-only'):
                dependency['profile']['debuginfo'] = debug
                valid = [script, message, dependency] + rows
                self.assertEqual(len(gate.check_artifacts(valid, root)), 3)
            for record, key, bad in [(script, 'env', [['only-one']]), (script, 'linked_libs', 'lib'),
                                     (message, 'message', False), (message, 'unexpected', True)]:
                altered = copy.deepcopy(record)
                altered[key] = bad
                with self.assertRaises(AssertionError):
                    gate.check_artifacts([altered] + rows, root)

    def test_raw_cargo_json_rejects_duplicate_keys_constants_and_malformed_lines(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = artifacts(root / 'repo', root / 'target/release')
            valid = '\n'.join(json.dumps(row) for row in rows)
            cases = [valid.replace('"success": true', '"success": false, "success": true'),
                     valid.replace('"test": false', '"test": true, "test": false', 1),
                     valid.replace('"opt_level": "3"', '"opt_level": "0", "opt_level": "3"', 1),
                     valid.replace('"debuginfo": 0', '"debuginfo": NaN', 1),
                     valid + '\nnot JSON', valid + '\nnull']
            evidence = root / 'evidence'
            evidence.mkdir()
            for text in cases:
                (evidence / 'production-tools.jsonl').write_text(text)
                with self.assertRaises((AssertionError, json.JSONDecodeError)):
                    gate.pin_tools(evidence, root / 'repo')
                self.assertFalse((evidence / 'tools').exists())

    def test_artifacts_reject_hardlink_alias_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = artifacts(root, root / 'target/release')
            first, second = (Path(row['executable']) for row in rows[:2])
            second.unlink()
            os.link(first, second)
            with self.assertRaises(AssertionError):
                gate.check_artifacts(rows, root)
            second.unlink()
            second.symlink_to(first)
            with self.assertRaises(AssertionError):
                gate.check_artifacts(rows, root)

    def make_evidence(self, root):
        rows = artifacts(root / 'repo', root / 'target/release')
        evidence = root / 'evidence'
        evidence.mkdir()
        (evidence / 'production-tools.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))
        gate.pin_tools(evidence, root / 'repo')
        gate.verify_tools(evidence)
        runtime = evidence / 'export/captures'
        editor = evidence / 'captures/editor'
        runtime.mkdir(parents=True)
        editor.mkdir(parents=True)
        for phase, shade in [('initial', 40), ('moved', 60), ('interacted', 80)]:
            for kind in ('source', 'export'):
                (runtime / f'{phase}-{kind}.png').write_bytes(png(shade=shade))
        (runtime / 'no-models.png').write_bytes(png(shade=100))
        for name, shade in [('room-editor-bound', 40), ('room-editor-models-offscreen', 60)]:
            (editor / (name + '.png')).write_bytes(png(64, 64, shade))
        project = evidence / 'export/authored-project'
        project.mkdir()
        entry = {'game': 'room-escape-v1', 'scene': 'room.scene.yaml', 'models': 'room.models.json',
                 'camera': 'room.camera.json'}
        (project / 'orr.project.json').write_text(json.dumps({'schema': 2, 'engine': '0.0.1', 'entry': entry}))
        (project / 'room.camera.json').write_text(json.dumps(camera_document(), indent=2) + '\n')
        runtime_path = evidence / 'tools/room_escape'
        manifest = {'schema': 1, 'payload': {'profile': 'room-escape-authored-linux-x86_64-v1',
                    'entry': entry,
                    'runtime': {'sha256': gate.digest(runtime_path), 'bytes': runtime_path.stat().st_size},
                    'initial_checksum': '0x94ab9f06da16ca7f',
                    'files': [{'path': 'project/' + name, 'role': role, 'mode': 0o644,
                               'bytes': (project / name).stat().st_size, 'sha256': gate.digest(project / name)}
                              for name, role in [('orr.project.json', 'project_manifest'), ('room.camera.json', 'camera_sidecar')]]}}
        (evidence / 'export/orr.export.json').write_text(json.dumps(manifest))
        return evidence

    def test_pins_detect_mutation_and_writable_tools(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            tool = evidence / 'tools/room_escape'
            tool.chmod(0o755)
            with self.assertRaises(AssertionError):
                gate.verify_tools(evidence)
            tool.write_bytes(b'wrong binary')
            tool.chmod(0o555)
            with self.assertRaises(AssertionError):
                gate.verify_tools(evidence)

    def test_valid_full_capture_values_and_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            gate.check_captures(evidence)
            self.assertEqual(len(json.loads((evidence / 'capture-manifest.json').read_text())), 9)

    def test_camera_schema_accepts_both_projections_and_exact_byte_boundary(self):
        document = camera_document()
        for projection in [{'type': 'orthographic', 'half_height': 1},
                           {'type': 'orthographic', 'half_height': 64},
                           {'type': 'perspective', 'fov_y_degrees': 20},
                           {'type': 'perspective', 'fov_y_degrees': 100}]:
            document['projection'] = projection
            document.update(target=[-64, 0, 64], yaw=3.1415927, pitch=1.4, distance=128)
            raw = json.dumps(document).encode()
            self.assertEqual(gate.check_camera(raw), document)
            padded = raw + b' ' * (4096 - len(raw))
            self.assertEqual(gate.check_camera(padded), document)
            with self.assertRaises(AssertionError):
                gate.check_camera(padded + b' ')

    def test_camera_closure_rejects_invalid_documents_even_with_matching_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            export_path = evidence / 'export/orr.export.json'
            manifest = json.loads(export_path.read_text())
            camera_path = evidence / 'export/authored-project/room.camera.json'
            camera_record = manifest['payload']['files'][1]
            valid = camera_document()
            raw = json.dumps(valid)
            invalids = [b'', b'{', b'[]', b'null', b'true', json.dumps(list(valid.values())).encode(),
                        raw.encode('utf-16'), raw.encode('utf-32'), b'\xef\xbb\xbf' + raw.encode(),
                        raw.replace('"schema": 1', '"schema": 1, "schema": 1').encode(),
                        raw.replace('"half_height": 8.0', '"half_height": 8.0, "half_height": 8.0').encode(),
                        raw.replace('"yaw": 0.65', '"yaw": NaN').encode(),
                        raw.replace('"yaw": 0.65', '"yaw": 1e999').encode(),
                        raw.encode() + b' ' * 4096]
            for key in valid:
                changed = copy.deepcopy(valid)
                del changed[key]
                invalids.append(json.dumps(changed).encode())
                changed = copy.deepcopy(valid)
                changed[key] = None
                invalids.append(json.dumps(changed).encode())
            changes = [('schema', True), ('schema', 1.0), ('schema', 2), ('unknown', 0),
                       ('target', [0, 0]), ('target', [0, 0, 0, 0]), ('target', [True, 0, 0]),
                       ('target', [65, 0, 0]), ('target', {'x': 0, 'y': 0, 'z': 0}),
                       ('yaw', '0.65'), ('yaw', False), ('yaw', 3.15), ('pitch', -1.41),
                       ('distance', 0), ('distance', 129), ('projection', ['orthographic', 8]),
                       ('projection', {'type': {'orthographic': None}, 'half_height': 8}),
                       ('projection', {'type': 'Orthographic', 'half_height': 8}),
                       ('projection', {'type': 'orthographic'}),
                       ('projection', {'type': 'orthographic', 'half_height': True}),
                       ('projection', {'type': 'orthographic', 'half_height': 0}),
                       ('projection', {'type': 'orthographic', 'half_height': 65}),
                       ('projection', {'type': 'orthographic', 'half_height': 8, 'fov_y_degrees': None}),
                       ('projection', {'type': 'orthographic', 'half_height': 8, 'extra': 0}),
                       ('projection', {'type': 'perspective', 'fov_y_degrees': 19}),
                       ('projection', {'type': 'perspective', 'fov_y_degrees': 101}),
                       ('projection', {'type': 'perspective', 'fov_y_degrees': 50, 'half_height': 8})]
            for key, value in changes:
                changed = copy.deepcopy(valid)
                changed[key] = value
                invalids.append(json.dumps(changed).encode())
            for data in invalids:
                with self.subTest(camera=data[:100]):
                    camera_path.write_bytes(data)
                    camera_record.update(bytes=len(data), sha256=gate.digest(camera_path))
                    export_path.write_text(json.dumps(manifest))
                    with self.assertRaises((AssertionError, ValueError, UnicodeDecodeError)):
                        gate.check_export(evidence, evidence / 'export')

    def test_camera_export_rejects_missing_aliased_duplicate_or_mismatched_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            export_path = evidence / 'export/orr.export.json'
            original = json.loads(export_path.read_text())
            modifications = [
                lambda p: p['entry'].pop('camera'), lambda p: p['entry'].update(camera=None),
                lambda p: p['entry'].update(camera='nested/room.camera.json'),
                lambda p: p['entry'].update(extra=True), lambda p: p['files'].pop(),
                lambda p: p['files'].append(copy.deepcopy(p['files'][1])),
                lambda p: p['files'][1].update(role='model_sidecar'),
                lambda p: p['files'][1].update(path='project/other.camera.json'),
                lambda p: p['files'][1].update(path='project/../room.camera.json'),
                lambda p: p['files'][1].update(path='project/./room.camera.json'),
                lambda p: p['files'][1].update(path='project//room.camera.json'),
                lambda p: p['files'][1].update(path='project\\room.camera.json'),
                lambda p: p['files'][1].update(path='/project/room.camera.json'),
                lambda p: p['files'][1].update(bytes=True), lambda p: p['files'][1].update(bytes=4097),
                lambda p: p['files'][1].update(sha256='0' * 64), lambda p: p['files'][1].update(mode=0o755),
                lambda p: p['files'][0].update(role='camera_sidecar'),
                lambda p: p['files'][1].update(extra=0),
            ]
            for change in modifications:
                altered = copy.deepcopy(original)
                change(altered['payload'])
                export_path.write_text(json.dumps(altered))
                with self.assertRaises(AssertionError):
                    gate.check_export(evidence, evidence / 'export')
            raw = json.dumps(original)
            for altered in [raw.replace('"camera": "room.camera.json"', '"camera": "room.camera.json", "camera": "room.camera.json"'),
                            raw.replace('"role": "camera_sidecar"', '"role": "model_sidecar", "role": "camera_sidecar"')]:
                export_path.write_text(altered)
                with self.assertRaises(AssertionError):
                    gate.check_export(evidence, evidence / 'export')
            export_path.write_text(raw)
            camera_path = evidence / 'export/authored-project/room.camera.json'
            original_bytes = camera_path.read_bytes()
            camera_path.write_bytes(original_bytes.replace(b'0.65', b'0.75'))
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, evidence / 'export')
            camera_path.unlink()
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, evidence / 'export')
            other = evidence / 'outside.camera.json'
            other.write_bytes(original_bytes)
            camera_path.symlink_to(other)
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, evidence / 'export')
            camera_path.unlink()
            camera_path.mkdir()
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, evidence / 'export')

    def test_export_helper_supports_separate_directory_and_explicit_legacy_entry(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            export = evidence / 'camera-export'
            (evidence / 'export').rename(export)
            gate.check_export(evidence, export)
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, export, require_camera=False)
            manifest_path = export / 'orr.export.json'
            manifest = json.loads(manifest_path.read_text())
            project_path = export / 'authored-project/orr.project.json'
            project = json.loads(project_path.read_text())
            del project['entry']['camera']
            project_path.write_text(json.dumps(project))
            record = manifest['payload']['files'][0]
            record.update(bytes=project_path.stat().st_size, sha256=gate.digest(project_path))
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, export)
            del manifest['payload']['entry']['camera']
            manifest['payload']['files'].pop()
            manifest_path.write_text(json.dumps(manifest))
            (export / 'authored-project/room.camera.json').unlink()
            gate.check_export(evidence, export, require_camera=False)
            with self.assertRaises(AssertionError):
                gate.check_export(evidence, export)

    def test_capture_checker_rejects_parity_blank_dimensions_and_model_controls(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            runtime = evidence / 'export/captures'
            editor = evidence / 'captures/editor'
            cases = [(runtime / 'initial-export.png', png(shade=41)),
                     (runtime / 'initial-source.png', png(blank=True)),
                     (runtime / 'initial-source.png', png(64, 64)),
                     (runtime / 'no-models.png', (runtime / 'initial-source.png').read_bytes()),
                     (editor / 'room-editor-models-offscreen.png', (editor / 'room-editor-bound.png').read_bytes()),
                     (runtime / 'initial-source.png', b'not a PNG')]
            for path, invalid in cases:
                original = path.read_bytes()
                path.write_bytes(invalid)
                with self.assertRaises((AssertionError, struct.error, zlib.error)):
                    gate.check_captures(evidence)
                path.write_bytes(original)
            path = runtime / 'extra.png'
            path.write_bytes(png())
            with self.assertRaises(AssertionError):
                gate.check_captures(evidence)
            path.unlink()
            (runtime / 'no-models.png').unlink()
            with self.assertRaises(AssertionError):
                gate.check_captures(evidence)

    def test_capture_checker_rejects_wrong_runtime_and_authored_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = self.make_evidence(Path(directory))
            path = evidence / 'export/orr.export.json'
            manifest = json.loads(path.read_text())
            bad = copy.deepcopy(manifest)
            bad['payload']['runtime']['sha256'] = '0' * 64
            path.write_text(json.dumps(bad))
            with self.assertRaises(AssertionError):
                gate.check_captures(evidence)
            path.write_text(json.dumps(manifest))
            (evidence / 'export/authored-project/orr.project.json').write_text('tampered')
            with self.assertRaises(AssertionError):
                gate.check_captures(evidence)

    def test_gpu_gate_is_mandatory_after_gpu_and_namespace_provision(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        gate_name = '- name: Required generated Room GPU and source-hidden production export'
        start = text.index(gate_name)
        stop = text.index('- name: Upload Room creator acceptance evidence', start)
        block = text[start:stop]
        self.assertIn("if: runner.os == 'Linux' && runner.arch == 'X64'", block)
        for required in ['ORR_REQUIRE_GPU: "1"', 'ORR_REQUIRE_PROJECT_ISOLATION: "1"', 'WGPU_BACKEND: vulkan', 'run: tools/check-room-template.sh gpu']:
            self.assertIn(required, block)
        self.assertNotIn('continue-on-error', block)
        self.assertLess(text.index('mesa-vulkan-drivers'), start)
        self.assertLess(text.index('install -y -q bubblewrap'), start)
        script = (ROOT / 'tools/check-room-template.sh').read_text()
        for command in ['run_test creator-default normal', 'run_test creator-room normal', 'run_test creator-joint normal', 'run_test sample-disabled normal', 'run_test sample-room normal', 'run_test editor-room normal', 'run_test source-hidden-gpu ignored', 'run_test editor-gpu ignored']:
            self.assertIn(command, script)
        for mode in ['contracts', 'gpu']:
            self.assertIn(f'run: tools/check-room-project.sh {mode}', text)
        self.assertIn('Required RoomEscapeV1 gameplay core', text)


if __name__ == '__main__':
    unittest.main(verbosity=2)
