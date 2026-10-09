#!/usr/bin/env python3
"""Semantic checker fixtures, not execution of the Rust or GPU acceptance gates."""
import copy
import json
from pathlib import Path
import re
import shutil
import tempfile
import unittest

from check_navigation_playground_gate import rust_test_names
import room_project_gate as gate
import test_room_template_gate as fixtures
from test_room_template_gate import log_for, png

ROOT = Path(__file__).resolve().parents[1]
INVENTORIES = json.loads(gate.INVENTORY.read_text())
CHARACTER_VIEW_TESTS = [
    'room_view::tests::character::authored_character_poses_are_read_only_and_share_exact_generation_placement',
    'room_view::tests::character::character_requires_one_player_binding_and_every_mapped_clip',
    'room_view::tests::character::mixed_character_depth_visibility_repeat_resize_and_failure_are_atomic',
]


def check_room_view_source_inventory(source):
    """Keep default Room tests distinct from the explicitly gated character lane."""
    actual = rust_test_names(source, 'room_view')
    assert len(actual) == len(set(actual)), 'duplicate qualified Room test'
    # Bind the required attribute to the actual module, not a matching comment,
    # string, or unrelated module. The lexical scanner identifies this inserted
    # test's scope and ignores comments and literals without executing Rust.
    guarded = list(re.finditer(
        r'#\[cfg\(feature\s*=\s*"room-character"\)\]\s*mod\s+character\s*\{', source))
    assert len(guarded) == 1, 'one explicit room-character module guard required'
    probe = '__room_character_inventory_scope_probe'
    assert probe not in source
    position = guarded[0].end()
    probed = rust_test_names(source[:position] + f'\n#[test] fn {probe}() {{}}\n' + source[position:], 'room_view')
    assert probed.count('room_view::tests::character::' + probe) == 1, 'guard must own the nested character test module'
    character = [name for name in actual if name.startswith('room_view::tests::character::')]
    assert sorted(character) == CHARACTER_VIEW_TESTS, 'exact character lane source inventory'
    default = [name for name in actual if name not in character]
    assert sorted(default) == INVENTORIES['room-view-contracts']['names'], 'exact default Room source inventory'
    declarations = re.findall(r'#\[test\]\s*((?:#\[[^\n]*\]\s*)*)fn\s+(\w+)\s*\(', source)
    ignored = []
    for qualified in actual:
        attributes = [attributes for attributes, name in declarations if name == qualified.rsplit('::', 1)[1]]
        assert len(attributes) == 1, ('ambiguous Room test attributes', qualified)
        if '#[ignore' in attributes[0]:
            ignored.append(qualified)
    assert sorted(name for name in ignored if name in default) == INVENTORIES['room-view-contracts']['ignored_names']
    assert sorted(name for name in ignored if name in character) == [CHARACTER_VIEW_TESTS[2]], 'character GPU test must remain explicitly ignored'


def unified_log(mode):
    pieces = []
    expected = INVENTORIES['camera-schema-unified']
    for harness in expected['harness_rows']:
        pieces.append(f"     Running unittests src/lib.rs (/tmp/target/release/deps/{harness['package']}-abc123)")
        if mode == 'result':
            pieces.append(log_for({'names': harness['names'], 'ignored_names': [], 'run_ignored': False,
                                   'expected_summary_rows': [harness['summary']]}))
        elif mode == 'list':
            pieces.append(''.join(name + ': test\n' for name in harness['names']))
    return '\n'.join(pieces) + '\n'


class RoomProjectGateTests(unittest.TestCase):
    def test_source_inventories_and_cfg_scopes(self):
        sources = [('room-simulation', 'orr_games/src/room_escape_game.rs', 'room_escape_game::tests::'),
                   ('runtime-input', 'orr_sample/src/room_app.rs', 'room_app::tests::'),
                   ('room-view-contracts', 'orr_sample/src/room_view.rs', 'room_view::tests::'),
                   ('project-admission', 'orr_sample/tests/room_project.rs', ''),
                   ('editor-workflow', 'orr_editor/tests/room_project_workflow.rs', ''),
                   ('camera-schema', 'orr_sample/src/room_camera.rs', 'room_camera::tests::'),
                   ('camera-editor-panel', 'orr_editor/src/room_camera_panel.rs', 'room_camera_panel::tests::'),
                   ('editor-model-admission', 'orr_editor/tests/room_model_admission.rs', '')]
        for lane, path, prefix in sources:
            source = (ROOT / 'crates' / path).read_text()
            if lane == 'room-view-contracts':
                check_room_view_source_inventory(source)
                continue
            if lane == 'runtime-input':
                source = source.split('mod tests {', 1)[1].split('\nmod room_ui_app_tests {', 1)[0]
            names = [(prefix + name, '#[ignore' in attributes)
                     for attributes, name in re.findall(r'#\[test\]\s*((?:#\[[^\n]*\]\s*)*)fn\s+(\w+)\s*\(', source)]
            self.assertEqual(INVENTORIES[lane]['names'], sorted(name for name, _ in names), lane)
            self.assertEqual(INVENTORIES[lane]['ignored_names'], sorted(name for name, ignored in names if ignored), lane)
        source = (ROOT / 'crates/orr_sample/tests/new_room_project.rs').read_text()
        source = re.sub(r'    #\[cfg\(feature = "room-ui"\)\]\n    #\[test\]\n    #\[ignore[^\n]*\]\n    fn generated_room_ui_real_export_source_hidden_gpu_workflow\(\)', '', source)
        names = ['room::' + name for name in re.findall(r'^    #\[test\]\n(?:    #\[ignore[^\n]*\]\n)?    fn (\w+)\(', source, re.M)]
        inherited = json.loads((ROOT / 'tools/room-template-inventories.json').read_text())
        self.assertEqual(inherited['sample-room']['names'], sorted(names))
        self.assertEqual(inherited['sample-room']['expected_summary_rows'], [[2, 0, 1]])
        self.assertIn('#[cfg(all(feature="room-project",feature="project-export"))]', source)
        self.assertEqual(INVENTORIES['generated-room-camera']['names'], sorted(names + ['generated_camera_only_edit_preserves_game_identity']))
        self.assertEqual(INVENTORIES['generated-room-camera']['expected_summary_rows'], [[3, 0, 1]])
        self.assertEqual(INVENTORIES['camera-schema-unified']['harness_rows'], [
            {'package': 'orr_editor', 'names': [], 'summary': [0, 0, 0]},
            {'package': 'orr_sample', 'names': INVENTORIES['camera-schema']['names'], 'summary': [9, 0, 0]}])

    def test_room_view_character_scope_is_explicit_and_cannot_hide_default_growth(self):
        default = ''.join(f'    #[test]\n    fn {name.rsplit("::", 1)[1]}() {{}}\n'
                          for name in INVENTORIES['room-view-contracts']['names'])
        character = ''.join('        #[test]\n' + ('        #[ignore = "software GPU"]\n' if index == 2 else '')
                            + f'        fn {name.rsplit("::", 1)[1]}() {{}}\n'
                            for index, name in enumerate(CHARACTER_VIEW_TESTS))
        guard = '#[cfg(feature = "room-character")]'
        valid = '#[cfg(test)]\nmod tests {\n' + default + '    ' + guard + '\n    mod character {\n' + character + '    }\n}\n'
        check_room_view_source_inventory(valid)
        first = '        #[test]\n        fn ' + CHARACTER_VIEW_TESTS[0].rsplit('::', 1)[1] + '() {}\n'
        moved = valid.replace(first, '').replace('    ' + guard, first.replace('        ', '    ') + '    ' + guard)
        cases = {
            'unguarded': valid.replace(guard, ''),
            'wrong feature': valid.replace('feature = "room-character"', 'feature = "room-project"'),
            'wrong module': valid.replace('mod character {', 'mod other {'),
            'wrong parent module': valid.replace('mod tests {', 'mod unrelated {'),
            'moved to default module': moved,
            'missing nested test': valid.replace(first, ''),
            'extra default test': valid.replace('    ' + guard, '    #[test]\n    fn unexpected_default() {}\n    ' + guard),
            'unrelated unguarded nested module': valid.replace('    ' + guard, '    mod unrelated {\n        #[test]\n        fn unexpected_nested_default() {}\n    }\n    ' + guard),
            'extra nested test': valid.replace('mod character {', 'mod character {\n        #[test]\n        fn unexpected_character() {}'),
            'missing GPU ignore': valid.replace('        #[ignore = "software GPU"]\n', ''),
            'comment guard decoy': '// ' + guard + '\n// mod character {\n' + valid.replace(guard, ''),
            'literal guard decoy': 'const DECOY: &str = r#"' + guard + '\nmod character {"#;\n' + valid.replace(guard, ''),
        }
        for name, source in cases.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                check_room_view_source_inventory(source)

    def test_every_lane_requires_exact_lists_outcomes_and_complete_summaries(self):
        for lane, expected in INVENTORIES.items():
            if 'harness_rows' in expected:
                continue
            with self.subTest(lane=lane):
                valid = log_for(expected)
                gate.check_log(expected, 'result', valid)
                for mode, names in [('list', expected['names']), ('ignored', expected['ignored_names'])]:
                    listed = ''.join(name + ': test\n' for name in names)
                    gate.check_log(expected, mode, listed)
                    for invalid in [listed + 'extra: test\n', listed + 'malformed name: test\n']:
                        with self.assertRaises(AssertionError): gate.check_log(expected, mode, invalid)
                bad = [valid.replace(' ... ok', ' ... FAILED', 1), valid.replace(expected['names'][0], 'wrong_test', 1),
                       valid + 'test unparsed name ... ok\n', valid + 'test extra ... ok\n',
                       valid + 'test result: ok. 0 passed; 0 failed; 0 ignored;\n',
                       valid.replace('0 failed;', '1 failed;', 1), valid.replace(' finished in 0.01s', ' broken tail'),
                       valid.replace(' ... ok', ' ... ignored', 1), '\n'.join(valid.splitlines()[1:]),
                       valid + 'SKIP: unavailable GPU\n', valid + 'source_hiding=false\n']
                for invalid in bad:
                    with self.assertRaises(AssertionError): gate.check_log(expected, 'result', invalid)

    def test_unified_zero_is_bound_to_editor_with_sample_nine_mandatory(self):
        expected = INVENTORIES['camera-schema-unified']
        for mode in ('list', 'ignored', 'result'):
            valid = unified_log(mode)
            gate.check_log(expected, mode, valid)
            bad = [valid.replace('orr_editor-', 'wrong-'), valid.replace('orr_sample-', 'orr_editor-'),
                   valid.replace('Running unittests src/lib.rs', 'Running tests/unknown.rs', 1),
                   valid[valid.index('     Running', 10):], valid + valid,
                   'test outside ... ok\n' + valid, valid + '\n Running tests/unknown.rs (/tmp/unknown-abc)\n']
            if mode != 'ignored':
                bad.extend([valid.replace(expected['names'][0], 'wrong_name', 1),
                            valid.replace('9 passed;', '0 passed;'),
                            valid.replace('0 passed;', '9 passed;', 1)] if mode == 'result' else [valid.replace(expected['names'][0], 'wrong_name', 1)])
            if mode == 'result':
                bad.append(valid.replace('test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n', ''))
            for invalid in bad:
                with self.assertRaises(AssertionError): gate.check_log(expected, mode, invalid)

    def test_camera_capture_pixels_dimensions_and_exact_preservation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = png(96, 64, 40)
            for name in gate.CAMERA_CAPTURES:
                (root / ('authored-camera-' + name + '.png')).write_bytes(
                    png(64, 96, 40) if name == 'portrait' else png(96, 64, 60) if name == 'manual' else first)
            self.assertEqual(len(gate.camera_captures(root)), 7)
            for name, data in [('landscape', png(64, 96)), ('portrait', first), ('manual', first),
                               ('invalid-retained', png(96, 64, 60)), ('reset', png(96, 64, 60)),
                               ('projection-error', png(96, 64, 60)), ('resize-return', png(96, 64, 60)),
                               ('manual', png(96, 64, blank=True))]:
                path = root / ('authored-camera-' + name + '.png')
                saved = path.read_bytes(); path.write_bytes(data)
                with self.assertRaises(AssertionError): gate.camera_captures(root)
                path.write_bytes(saved)
            (root / 'extra.png').write_bytes(first)
            with self.assertRaises(AssertionError): gate.camera_captures(root)
            (root / 'extra.png').unlink()
            (root / 'authored-camera-manual.png').unlink()
            with self.assertRaises(AssertionError): gate.camera_captures(root)

    def test_runtime_capture_exact_parity_and_negative_controls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for phase, shade in [('initial', 40), ('moved', 60), ('interacted', 80)]:
                for kind in ('source', 'export'): (root / f'{phase}-{kind}.png').write_bytes(png(shade=shade))
            (root / 'no-models.png').write_bytes(png(shade=100))
            self.assertEqual(len(gate.runtime_captures(root)), 7)
            for name, data in [('initial-export', png(shade=41)), ('no-models', png(shade=40)),
                               ('initial-source', png(64, 64)), ('initial-source', png(blank=True))]:
                path = root / (name + '.png'); saved = path.read_bytes(); path.write_bytes(data)
                with self.assertRaises(AssertionError): gate.runtime_captures(root)
                path.write_bytes(saved)
            for kind in ('source', 'export'): (root / f'moved-{kind}.png').write_bytes(png(shade=40))
            with self.assertRaises(AssertionError): gate.runtime_captures(root)

    def test_full_composite_gate_validates_both_exports_and_all_23_pngs(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = fixtures.RoomTemplateGateTests().make_evidence(Path(directory))
            shutil.copytree(evidence / 'export', evidence / 'camera-export')
            (evidence / 'captures/editor').rename(evidence / 'captures/legacy-editor')
            camera = evidence / 'captures/camera-editor'
            camera.mkdir()
            first = png(96, 64, 40)
            for name in gate.CAMERA_CAPTURES:
                (camera / ('authored-camera-' + name + '.png')).write_bytes(
                    png(64, 96, 40) if name == 'portrait' else png(96, 64, 60) if name == 'manual' else first)
            # Legacy source has no camera sidecar. Its project and export entry
            # must both remain the exact three-field entry for this control.
            legacy = evidence / 'export'
            manifest_path = legacy / 'orr.export.json'
            manifest = json.loads(manifest_path.read_text())
            manifest['payload']['entry'].pop('camera')
            manifest['payload']['files'] = [row for row in manifest['payload']['files'] if row['role'] != 'camera_sidecar']
            project_path = legacy / 'authored-project/orr.project.json'
            project = json.loads(project_path.read_text())
            project['entry'].pop('camera')
            project_path.write_text(json.dumps(project))
            (legacy / 'authored-project/room.camera.json').unlink()
            item = next(row for row in manifest['payload']['files'] if row['role'] == 'project_manifest')
            item.update(bytes=project_path.stat().st_size, sha256=gate.shared.digest(project_path))
            manifest_path.write_text(json.dumps(manifest))
            gate.check_captures(evidence)
            self.assertEqual(len(json.loads((evidence / 'capture-manifest.json').read_text())), 23)
            camera_path = evidence / 'camera-export/authored-project/room.camera.json'
            camera_path.write_bytes(camera_path.read_bytes() + b' ')
            with self.assertRaises(AssertionError): gate.check_captures(evidence)

    def test_mandatory_gates_cover_six_distinct_ignored_gpu_tests(self):
        template = json.loads((ROOT / 'tools/room-template-inventories.json').read_text())
        names = {row['names'][0] for row in INVENTORIES.values() if row['run_ignored']}
        self.assertEqual(len(names), 5)
        names |= {row['names'][0] for row in template.values() if row['run_ignored']}
        self.assertEqual(len(names), 6)
        script = (ROOT / 'tools/check-room-project.sh').read_text()
        self.assertIn('options+=(--ignored --exact)', script)
        for lane, row in INVENTORIES.items():
            self.assertIn('run_test ' + lane + (' ignored ' if row['run_ignored'] else ' normal '), script)
        for required in ['--message-format=json', 'python3 "$checker" pin "$evidence"', 'python3 "$checker" verify-tools "$evidence"',
                         'ORR_REQUIRE_PROJECT_ISOLATION', 'test "$(id -u)" -ne 0', 'namespace-preflight',
                         'captures/legacy-editor', 'captures/camera-editor', '--all-targets -- -D warnings']:
            self.assertIn(required, script)
        self.assertNotIn('target/release/', script)
        self.assertNotIn('continue-on-error', script)
        workflow = (ROOT / '.github/workflows/determinism.yml').read_text()
        for name in ('check-room-project', 'check-room-template'):
            for mode in ('contracts', 'gpu'): self.assertIn(f'run: tools/{name}.sh {mode}', workflow)


if __name__ == '__main__':
    unittest.main(verbosity=2)
