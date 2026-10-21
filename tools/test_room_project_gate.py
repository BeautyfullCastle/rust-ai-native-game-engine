#!/usr/bin/env python3
"""Semantic checker fixtures, not execution of the Rust or GPU acceptance gates."""
import copy
import json
from pathlib import Path
import re
import shutil
import tempfile
import unittest

import room_project_gate as gate
import test_room_template_gate as fixtures
from test_room_template_gate import log_for, png

ROOT = Path(__file__).resolve().parents[1]
INVENTORIES = json.loads(gate.INVENTORY.read_text())


ROOM_CONTRACTS = 'RoomEscapeV1 contracts and optional feature combinations'
ROOM_GPU = 'RoomEscapeV1 actual GPU and source-hidden production export'
TEMPLATE_CONTRACTS = 'Required Room creator API CLI editor and joint inventories'
TEMPLATE_GPU = 'Required generated Room GPU and source-hidden production export'
ROOM_CONDITION = "runner.os == 'Linux' && runner.arch == 'X64'"
ROOM_MIGRATED_NAMES = (
    ROOM_CONTRACTS, ROOM_GPU, 'Upload RoomEscapeV1 acceptance evidence',
    'Room creator checker semantic fixtures', TEMPLATE_CONTRACTS, TEMPLATE_GPU,
    'Upload Room creator acceptance evidence',
)
ROOM_SETUP = '''      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install Room software Vulkan dependency
        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers
      - name: Install Room namespace acceptance dependency
        run: sudo apt-get install -y -q bubblewrap
'''


def workflow_section(text, header, indent):
    """Read one exact indentation-scoped section, not a whole-file substring."""
    lines = text.splitlines()
    assert lines.count(header) == 1, ('missing/duplicate workflow section', header)
    start = lines.index(header)
    end = start + 1
    while end < len(lines):
        line = lines[end]
        if line.strip() and not line.lstrip().startswith('#'):
            if len(line) - len(line.lstrip()) <= indent:
                break
        end += 1
    while end > start + 1 and (not lines[end - 1].strip() or lines[end - 1].lstrip().startswith('#')):
        end -= 1
    return '\n'.join(lines[start:end]) + '\n'


def room_workflow_steps(workflow):
    """The supported CI shape is deliberately strict; changed gates need review."""
    trigger = workflow_section(workflow, 'on:', 0)
    assert trigger == 'on:\n  pull_request:\n    branches: [main, su/engine-correctness-hardening]\n    types: [ready_for_review]\n  workflow_dispatch:\n', 'workflow triggers'
    jobs = workflow_section(workflow, 'jobs:', 0)
    sample = workflow_section(jobs, '  sample-build:', 2)
    assert sample.count('    steps:\n') == 1, 'sample-build steps'
    header, body = sample.split('    steps:\n', 1)
    assert all(len(line) - len(line.lstrip()) >= 6
               for line in body.splitlines() if line.strip() and not line.lstrip().startswith('#')), 'trailing job fields'
    header = '\n'.join(line for line in header.splitlines() if line.strip() and not line.lstrip().startswith('#'))
    assert header == '\n'.join([
        '  sample-build:', '    needs: [sample-fixture-preflight]', '    name: sample build (${{ matrix.os }})',
        '    runs-on: ${{ matrix.os }}', '    strategy:', '      fail-fast: false',
        '      matrix:', '        os: [ubuntu-22.04, windows-latest]',
    ]), 'sample-build runner/trigger must remain mandatory'
    assert not any(name in sample for name in ROOM_MIGRATED_NAMES), 'Room steps must have one owner'
    for helper in ('tools/check-room-project.sh', 'tools/check-room-template.sh'):
        assert helper not in sample, 'renamed duplicate Room helper in sample-build'

    job = workflow_section(jobs, '  sample-room:', 2)
    assert job.count('    steps:\n') == 1, 'sample-room steps'
    header, body = job.split('    steps:\n', 1)
    assert all(len(line) - len(line.lstrip()) >= 6
               for line in body.splitlines() if line.strip() and not line.lstrip().startswith('#')), 'trailing Room job fields'
    header = '\n'.join(line for line in header.splitlines() if line.strip() and not line.lstrip().startswith('#'))
    assert header == '\n'.join([
        '  sample-room:', '    name: sample room acceptance (ubuntu-22.04)',
        '    runs-on: ubuntu-22.04',
    ]), 'Room runner/trigger must remain mandatory and independent'
    assert body.startswith(ROOM_SETUP), 'Room must provision its own compiler, cache, GPU and namespace'
    assert len(re.findall(r'^      - ', body, re.M)) == 12, 'exact Room setup and migrated steps'
    assert re.findall(r'^      - name: (.+)$', body, re.M) == [
        'Install Room software Vulkan dependency', 'Install Room namespace acceptance dependency',
        *ROOM_MIGRATED_NAMES,
    ], 'Room step order and unique producer ownership'
    fixture = workflow_section(job, '      - name: Room creator checker semantic fixtures', 6)
    assert fixture == ('      - name: Room creator checker semantic fixtures\n'
                       '        if: ' + ROOM_CONDITION + '\n        shell: bash\n'
                       '        run: python3 tools/test_room_template_gate.py\n'), 'Room checker fixture remains mandatory'
    for name, root, artifact in [
        ('Upload RoomEscapeV1 acceptance evidence', 'orr-room-project', 'room-project-evidence'),
        ('Upload Room creator acceptance evidence', 'orr-room-template', 'room-template-evidence'),
    ]:
        expected = ('      - name: ' + name + '\n'
                    '        if: always() && ' + ROOM_CONDITION + '\n'
                    '        uses: actions/upload-artifact@v4\n        with:\n'
                    '          name: ' + artifact + '-ubuntu-22.04\n          path: |\n')
        expected += ''.join('            /tmp/' + root + '-${{ github.run_id }}-${{ github.run_attempt }}/**/*.'
                            + extension + '\n' for extension in ('log', 'sha256', 'txt', 'png', 'json', 'jsonl', 'stderr'))
        expected += '          if-no-files-found: error\n          retention-days: 7\n'
        assert workflow_section(job, '      - name: ' + name, 6) == expected, ('mandatory Room evidence upload', name)
    names = (ROOM_CONTRACTS, ROOM_GPU, TEMPLATE_CONTRACTS, TEMPLATE_GPU)
    return {name: workflow_section(job, '      - name: ' + name, 6) for name in names}


def check_room_workflow(workflow):
    steps = room_workflow_steps(workflow)
    for name, script, mode, evidence in [
        (ROOM_CONTRACTS, 'check-room-project', 'contracts', 'ORR_ROOM_EVIDENCE'),
        (ROOM_GPU, 'check-room-project', 'gpu', 'ORR_ROOM_EVIDENCE'),
        (TEMPLATE_CONTRACTS, 'check-room-template', 'contracts', 'ORR_ROOM_TEMPLATE_EVIDENCE'),
        (TEMPLATE_GPU, 'check-room-template', 'gpu', 'ORR_ROOM_TEMPLATE_EVIDENCE'),
    ]:
        prefix = ('      - name: ' + name + '\n        if: ' + ROOM_CONDITION
                  + '\n        shell: bash\n        timeout-minutes: ' + ('90' if mode == 'contracts' else '60') + '\n')
        root = 'orr-room-project' if evidence == 'ORR_ROOM_EVIDENCE' else 'orr-room-template'
        env = ('        env:\n' + ('' if mode == 'contracts' else
               '          WGPU_BACKEND: vulkan\n          ORR_REQUIRE_GPU: "1"\n          ORR_REQUIRE_PROJECT_ISOLATION: "1"\n')
               + '          ' + evidence + ': /tmp/' + root + '-${{ github.run_id }}-${{ github.run_attempt }}\n')
        command = 'tools/' + script + '.sh ' + mode
        step = steps[name]
        if name != ROOM_CONTRACTS:
            run = '        run: ' + command + '\n'
            expected = prefix + (run + env if mode == 'contracts' else env + run)
            assert step == expected, ('mandatory scalar step', name)
            continue
        assert step.startswith(prefix + '        run: |\n') and step.endswith(env), 'Room contracts step fields'
        lines = step[len(prefix + '        run: |\n'):-len(env)].splitlines()
        assert all(not line or line.startswith('          ') for line in lines), 'Room run block indentation'
        run = '\n'.join(line[10:] for line in lines) + '\n'
        before = ('if ' + command + '; then\n  exit 0\nelse\n  room_status=$?\nfi\n'
                  'if python3 - "$room_status" <<\'PY\'\n')
        after = ('PY\nthen\n  :\nelse\n'
                 "  printf '%s\\n' 'Room tracked-metadata collector failed; original gate exit retained' >&2\n"
                 'fi\nexit "$room_status"\n')
        assert run.startswith(before) and run.endswith(after), 'exact gate invocation and original-exit propagation'
        diagnostic = run[len(before):-len(after)]
        assert diagnostic and 'PY' not in diagnostic.splitlines(), 'one diagnostic heredoc; no intervening shell'


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


def room_view_source_tests(source, *, animated):
    """Read the bounded Room test-attribute syntax, rejecting unknown cfg forms.

    This is a source guard, not a Rust parser or evidence of test execution.
    Keep the static inventory separate from the opt-in animated tests.
    """
    # Attribute blocks may contain blank lines and comments. Capture multiline
    # attributes too so unsupported cfg syntax cannot disappear before #[test].
    assert not re.search(r'/\*(?:(?!\*/).)*?/\*', source, re.S), 'unsupported nested block comment'
    trivia = r'(?:\s|//[^\n]*(?:\n|$)|/\*.*?\*/)'
    pattern = r'((?:^[ \t]*#\[[^\]]*\]' + trivia + r'*)+)fn\s+(\w+)\s*\('
    rows = []
    seen = 0
    test_names = set()
    for attributes, name in re.findall(pattern, source, re.M | re.S):
        attributes = re.findall(r'#\[[^\]]*\]', attributes)
        if '#[test]' not in attributes:
            continue
        seen += attributes.count('#[test]')
        assert name not in test_names, ('duplicate Room test names', name)
        test_names.add(name)
        assert attributes.count('#[test]') == 1, ('duplicate test attribute', name)
        allowed = {'#[test]', '#[cfg(feature = "room-animated-models")]'}
        assert all(attribute in allowed for attribute in attributes), ('unsupported Room test attributes', name, attributes)
        assert len(attributes) == len(set(attributes)), ('duplicate Room test attributes', name)
        gated = '#[cfg(feature = "room-animated-models")]' in attributes
        if animated or not gated:
            rows.append(('room_view::tests::' + name, False))
    assert seen == len(re.findall(r'#\[test\]', source)), 'unparsed Room test attributes'
    assert len(rows) == len(set(rows)), 'duplicate Room test names'
    return rows


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
                names = room_view_source_tests(source, animated=False)
                animated_names = room_view_source_tests(source, animated=True)
                self.assertEqual(sorted(set(animated_names) - set(names)), [
                    ('room_view::tests::animated_sampling_uses_fixed_room_ticks_and_once_loop_policies', False),
                    ('room_view::tests::animated_world_matrix_composes_body_before_binding_local_transform', False),
                ])
            else:
                names = [(prefix + name, '#[ignore' in attributes)
                         for attributes, name in re.findall(r'#\[test\]\s*((?:#\[[^\n]*\]\s*)*)fn\s+(\w+)\s*\(', source)]
            self.assertEqual(INVENTORIES[lane]['names'], sorted(name for name, _ in names), lane)
            self.assertEqual(INVENTORIES[lane]['ignored_names'], sorted(name for name, ignored in names if ignored), lane)
        source = (ROOT / 'crates/orr_sample/tests/new_room_project.rs').read_text()
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

    def test_room_view_source_reader_keeps_feature_boundaries_and_fails_closed(self):
        static = '#[test]\nfn static_case() {}\n'
        cfg = '#[cfg(feature = "room-animated-models")]\n'
        animated = cfg + '#[test]\nfn animated_case() {}\n'
        for gated in (animated, '#[test]\n' + cfg + 'fn animated_case() {}\n',
                      animated.replace(cfg, cfg + '\n// boundary comment\n/* comment */\n')):
            self.assertEqual(room_view_source_tests(static + gated, animated=False),
                             [('room_view::tests::static_case', False)])
            self.assertEqual(room_view_source_tests(static + gated, animated=True), [
                ('room_view::tests::static_case', False),
                ('room_view::tests::animated_case', False),
            ])
        for invalid in (
            animated.replace('room-animated-models', 'unknown-feature'),
            animated.replace(cfg, '#[cfg(feature = "unknown-feature")]\n\n'),
            animated.replace(cfg, '#[cfg(\nfeature = "unknown-feature"\n)]\n'),
            animated.replace(cfg, '#[cfg(feature = "unknown-feature")]\n// comment\n'),
            animated.replace(cfg, '#[cfg(feature = "unknown-feature")]\n/* comment */\n'),
            animated.replace(cfg, cfg + '/* outer /* nested */ comment */\n'),
            animated.replace(cfg, '#[cfg(not(feature = "room-animated-models"))]\n'),
            animated.replace(cfg, '#[cfg(any(feature = "room-animated-models", test))]\n'),
            animated.replace('#[test]', '#[test]\n#[ignore]'),
            animated.replace('fn animated_case', 'async fn animated_case'),
            animated.replace('#[test]', '#[test]\n#[test]'),
            animated + animated,
        ):
            for enabled in (False, True):
                with self.subTest(source=invalid, animated=enabled), self.assertRaises(AssertionError):
                    room_view_source_tests(invalid, animated=enabled)

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
        check_room_workflow(workflow)

    def test_workflow_checks_reject_missing_disabled_or_nonpropagating_gates(self):
        workflow = (ROOT / '.github/workflows/determinism.yml').read_text()
        check_room_workflow(workflow)
        steps = room_workflow_steps(workflow)
        contracts = steps[ROOM_CONTRACTS]
        command = 'if tools/check-room-project.sh contracts; then'
        mutations = {
            'missing command': (command, 'if true; then'),
            'wrong command': (command, 'if tools/check-room-template.sh contracts; then'),
            'wrong argument': (command, 'if tools/check-room-project.sh gpu; then'),
            'commented command': (command, '# ' + command),
            'extra argument': (command, 'if tools/check-room-project.sh contracts extra; then'),
            'swallowed command failure': (command, 'if tools/check-room-project.sh contracts || true; then'),
            'skipped command': (command, 'if false; then\n          ' + command),
            'late status capture': ('  room_status=$?', '  :\n            room_status=$?'),
            'reassigned status': ('  room_status=$?', '  room_status=0'),
            'swallowed final exit': ('exit "$room_status"', 'exit 0'),
            'missing final exit': ('exit "$room_status"', ':'),
            'shell after heredoc': ('          PY\n', '          PY\n          room_status=0\n'),
            'premature heredoc end': ('          import base64\n', '          PY\n          room_status=0\n          import base64\n'),
            'no step trigger': ('        if: ' + ROOM_CONDITION + '\n', ''),
            'disabled step': ('        if: ' + ROOM_CONDITION, '        if: false'),
            'wrong architecture': ("runner.arch == 'X64'", "runner.arch == 'ARM64'"),
            'wrong shell': ('        shell: bash', '        shell: python'),
            'continue on error': ('        shell: bash', '        continue-on-error: true\n        shell: bash'),
            'missing environment': ('          ORR_ROOM_EVIDENCE:', '          UNUSED_ROOM_EVIDENCE:'),
            'wrong evidence destination': ('/tmp/orr-room-project-', '/tmp/wrong-room-'),
        }
        for label, (old, new) in mutations.items():
            with self.subTest(mutation=label):
                self.assertIn(old, contracts)
                changed = contracts.replace(old, new, 1)
                self.assertNotEqual(changed, contracts)
                with self.assertRaises(AssertionError):
                    check_room_workflow(workflow.replace(contracts, changed, 1))
        for name, step in steps.items():
            for label, replacement in [
                ('removed step', ''), ('renamed step', step.replace(name, 'Unrelated step', 1)),
                ('duplicate step', step + step),
                ('comment-only command', step.replace('        run: tools/', '        # run: tools/', 1)
                 if name != ROOM_CONTRACTS else contracts.replace(command, '# ' + command, 1)),
            ]:
                with self.subTest(step=name, mutation=label), self.assertRaises(AssertionError):
                    check_room_workflow(workflow.replace(step, replacement, 1))
        job = workflow_section(workflow, '  sample-build:', 2)
        for field in ('if: false', 'continue-on-error: true', 'steps: []'):
            with self.subTest(trailing_job_field=field), self.assertRaises(AssertionError):
                check_room_workflow(workflow.replace(job, job + '    ' + field + '\n', 1))
        relocated = workflow.replace(job, '', 1).replace('jobs:\n', 'unrelated:\n' + job + '\njobs:\n', 1)
        with self.subTest(mutation='job outside jobs'), self.assertRaises(AssertionError):
            check_room_workflow(relocated)
        changed = job.replace('os: [ubuntu-22.04, windows-latest]', 'os: [windows-latest]', 1)
        with self.subTest(mutation='no Linux sample runner'), self.assertRaises(AssertionError):
            check_room_workflow(workflow.replace(job, changed, 1))
        for label, old, new in [
            ('missing jobs section', 'jobs:\n', 'unrelated:\n'),
            ('missing PR trigger', '  pull_request:\n', ''),
            ('conditional PR trigger', '  pull_request:\n', '  pull_request:\n    paths: [unrelated]\n'),
            ('missing job', '  sample-build:\n', '  other-job:\n'),
            ('disabled job', '  sample-build:\n', '  sample-build:\n    if: false\n'),
            ('job allows failure', '  sample-build:\n', '  sample-build:\n    continue-on-error: true\n'),
        ]:
            with self.subTest(mutation=label):
                self.assertIn(old, workflow)
                with self.assertRaises(AssertionError):
                    check_room_workflow(workflow.replace(old, new, 1))

    def test_independent_room_job_rejects_placement_provision_and_evidence_changes(self):
        workflow = (ROOT / '.github/workflows/determinism.yml').read_text()
        check_room_workflow(workflow)
        job = workflow_section(workflow, '  sample-room:', 2)
        for field in ('if: false', 'continue-on-error: true', 'needs: [sample-build]',
                      'env: {ORR_REQUIRE_GPU: "0"}', 'steps: []'):
            for changed in (job.replace('    steps:\n', '    ' + field + '\n    steps:\n', 1),
                            job + '    ' + field + '\n'):
                with self.subTest(job_field=field), self.assertRaises(AssertionError):
                    check_room_workflow(workflow.replace(job, changed, 1))
        mutations = [
            ('missing Room job', '  sample-room:\n', '  unrelated-room:\n'),
            ('renamed check', 'name: sample room acceptance (ubuntu-22.04)', 'name: unrelated acceptance'),
            ('wrong runner', '    runs-on: ubuntu-22.04\n', '    runs-on: ubuntu-latest\n'),
            ('missing checkout', '      - uses: actions/checkout@v4\n', ''),
            ('missing toolchain', '      - uses: dtolnay/rust-toolchain@stable\n', ''),
            ('missing cache', '      - uses: Swatinem/rust-cache@v2\n', ''),
            ('missing GPU provision', '        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n', ''),
            ('missing namespace provision', '        run: sudo apt-get install -y -q bubblewrap\n', ''),
            ('disabled provision', '      - name: Install Room namespace acceptance dependency\n',
             '      - name: Install Room namespace acceptance dependency\n        if: false\n'),
            ('disabled fixture', '        run: python3 tools/test_room_template_gate.py\n', '        run: true\n'),
            ('missing failure evidence', '        if: always() && ' + ROOM_CONDITION,
             '        if: success() && ' + ROOM_CONDITION),
            ('missing evidence accepted', '          if-no-files-found: error\n', '          if-no-files-found: ignore\n'),
            ('wrong artifact name', 'name: room-project-evidence-ubuntu-22.04', 'name: unrelated-evidence'),
            ('artifact collision', 'name: room-template-evidence-ubuntu-22.04', 'name: room-project-evidence-ubuntu-22.04'),
            ('missing capture glob', '            /tmp/orr-room-project-${{ github.run_id }}-${{ github.run_attempt }}/**/*.png\n', ''),
        ]
        for label, old, new in mutations:
            self.assertIn(old, job, label)
            with self.subTest(mutation=label), self.assertRaises(AssertionError):
                check_room_workflow(workflow.replace(job, job.replace(old, new, 1), 1))
        for name in ROOM_MIGRATED_NAMES:
            step = workflow_section(job, '      - name: ' + name, 6)
            for changed in (job.replace(step, '', 1), job.replace(step, step + step, 1)):
                with self.subTest(step=name), self.assertRaises(AssertionError):
                    check_room_workflow(workflow.replace(job, changed, 1))
        provision = workflow_section(job, '      - name: Install Room namespace acceptance dependency', 6)
        reordered = job.replace(provision, '', 1) + provision
        with self.subTest(mutation='provision after GPU'), self.assertRaises(AssertionError):
            check_room_workflow(workflow.replace(job, reordered, 1))
        relocated = workflow.replace(job, '', 1).replace('jobs:\n', 'unrelated:\n' + job + '\njobs:\n', 1)
        with self.subTest(mutation='Room outside jobs'), self.assertRaises(AssertionError):
            check_room_workflow(relocated)
        sample = workflow_section(workflow, '  sample-build:', 2)
        for helper in ('check-room-project.sh', 'check-room-template.sh'):
            duplicate = sample + ('      - name: Renamed duplicate Room gate\n'
                                  '        run: tools/' + helper + ' contracts\n')
            with self.subTest(duplicate=helper), self.assertRaises(AssertionError):
                check_room_workflow(workflow.replace(sample, duplicate, 1))


if __name__ == '__main__':
    unittest.main(verbosity=2)
