#!/usr/bin/env python3
"""Independent UV-variant gate semantics; lifecycle fixtures run separately."""
import copy
import hashlib
import json
import os
import pathlib
import re
import struct
import sys
import tempfile
import unittest
from unittest import mock
import zlib

import static_model_uv_gate as gate
from test_room_project_gate import check_room_workflow
from test_static_model_extract_gate import listing, passed, workflow_job, mutate_workflow_job

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXPECTED_TESTS = [
    'test_uv_oracle_hand_calculated_sampling',
    'test_capture_json_rejects_duplicate_and_nonfinite',
    'test_variant_rejects_uv_order_zero_sign_and_nonfinite',
    'test_prebuild_source_inventory_rejects_ignored_and_duplicate_paths',
    'test_command_fences_order_and_mutation',
    'test_complete_feature_source_contract',
    'test_source_snapshot_fences_bytes_modes_and_new_paths',
    'test_source_snapshot_rejects_symlinks',
    'test_source_snapshot_covers_helper_and_fixture_producers',
    'test_actual_model_census_matches_current_source',
    'test_actual_consumer_census_matches_current_source',
    'test_producer_binds_complete_prebuild_source',
    'test_producer_rejects_cached_wrong_source_and_duplicate_artifacts',
    'test_inventory_and_result_fail_closed',
    'test_required_ignored_consumer_runs_exactly',
    'test_unrun_modes_cannot_be_claimed',
    'test_gpu_capture_inventory_and_pixels',
    'test_gpu_rejects_receipt_pixel_and_provenance_mutations',
    'test_variant_keeps_source_geometry_and_slots',
    'test_variant_rejects_geometry_material_and_dependency_mutations',
    'test_export_capture_inventory_and_provenance',
    'test_export_rejects_receipt_frame_and_inventory_mutations',
    'test_workflow_required_modes_and_retention',
    'test_workflow_rejects_disabled_moved_or_unretained_modes',
    'test_workflow_preserves_required_room_and_aggregate',
]
BYTECODE_SETUP = '''      - name: Prevent Python bytecode in source checkout
        shell: bash
        run: printf '%s\\n' 'PYTHONDONTWRITEBYTECODE=1' >> "$GITHUB_ENV"
'''

# Pin the complete reviewed matrix, including routing, isolation, and retention.
AUTHORING_JOB = '  static-model-authoring:\n    # Independent feature routes own separate runners, private targets and evidence.\n    # Keep all eight rows mandatory; the final aggregate requires this matrix.\n    name: static model authoring (${{ matrix.feature }}, ${{ matrix.mode }}, ${{ matrix.os }})\n    runs-on: ${{ matrix.os }}\n    timeout-minutes: 90\n    env:\n      PYTHONDONTWRITEBYTECODE: "1"\n    strategy:\n      fail-fast: false\n      matrix:\n        include:\n          - feature: uv\n            mode: cpu\n            os: ubuntu-22.04\n            timeout: 45\n          - feature: uv\n            mode: cpu\n            os: windows-latest\n            timeout: 45\n          - feature: uv\n            mode: gpu\n            os: ubuntu-22.04\n            timeout: 45\n          - feature: uv\n            mode: export\n            os: ubuntu-22.04\n            timeout: 60\n          - feature: pivot\n            mode: cpu\n            os: ubuntu-22.04\n            timeout: 45\n          - feature: pivot\n            mode: cpu\n            os: windows-latest\n            timeout: 45\n          - feature: pivot\n            mode: gpu\n            os: ubuntu-22.04\n            timeout: 45\n          - feature: pivot\n            mode: export\n            os: ubuntu-22.04\n            timeout: 60\n    steps:\n      - uses: actions/checkout@v4\n      - uses: dtolnay/rust-toolchain@stable\n      - uses: Swatinem/rust-cache@v2\n      - name: Install static authoring software Vulkan dependency\n        if: matrix.mode != \'cpu\'\n        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n      - name: Install static authoring namespace acceptance dependency\n        if: matrix.mode == \'export\'\n        run: sudo apt-get update -q && sudo apt-get install -y -q bubblewrap\n      - name: Required independent static model authoring acceptance\n        shell: bash\n        timeout-minutes: ${{ matrix.timeout }}\n        env:\n          ORR_STATIC_UV_EVIDENCE: ${{ matrix.mode == \'export\' && \'/tmp\' || runner.temp }}/orr-static-${{ matrix.feature }}-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}\n          ORR_STATIC_PIVOT_EVIDENCE: ${{ matrix.mode == \'export\' && \'/tmp\' || runner.temp }}/orr-static-${{ matrix.feature }}-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}\n        run: python -B tools/static_model_${{ matrix.feature }}_gate.py ${{ matrix.mode }}\n      - name: Retain static authoring and partial failure evidence\n        if: always()\n        uses: actions/upload-artifact@v4\n        with:\n          name: static-${{ matrix.feature }}-${{ matrix.mode }}-${{ matrix.os }}\n          path: ${{ matrix.mode == \'export\' && \'/tmp\' || runner.temp }}/orr-static-${{ matrix.feature }}-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}\n          include-hidden-files: true\n          if-no-files-found: error\n          retention-days: 7\n\n'
AUTHORING_AGGREGATE_STEP = '      - name: Require every independent static authoring matrix row to pass\n        shell: bash\n        env:\n          AUTHORING_RESULT: ${{ needs[\'static-model-authoring\'].result }}\n        run: |\n          echo "AUTHORING_RESULT=$AUTHORING_RESULT"\n          if [[ "$AUTHORING_RESULT" != "success" ]]; then\n            echo "::error::AUTHORING_RESULT must be success, got \'$AUTHORING_RESULT\'"\n            exit 1\n          fi\n          echo "All eight independent static authoring routes passed."\n'


ROOM_WALL_AGGREGATE_STEP = '      - name: Require every independent Room wall authoring matrix row to pass\n        if: always()\n        shell: bash\n        env:\n          ROOM_WALL_RESULT: ${{ needs[\'room-wall-authoring\'].result }}\n        run: |\n          echo "ROOM_WALL_RESULT=$ROOM_WALL_RESULT"\n          if [[ "$ROOM_WALL_RESULT" != "success" ]]; then\n            echo "::error::ROOM_WALL_RESULT must be success, got \'$ROOM_WALL_RESULT\'"\n            exit 1\n          fi\n          echo "All four independent Room wall authoring routes passed."\n'

# These independently owned Arena steps repeat the inherited value without
# changing it. Pin complete executable blocks: this is not a general exemption
# for same-value overrides on the protected static-model producers. Unsupported
# YAML/shell spellings fail closed and require an explicit contract review.
ARENA_BYTECODE_STEPS = (
    ("Required Arena editor export CPU acceptance", "arena_editor_export_gate.py cpu",
     "ORR_ARENA_EXPORT_GATE_EVIDENCE", "${{ runner.temp }}/arena-editor-export-cpu-", ""),
    ("Required Arena editor export native acceptance", "arena_editor_export_gate.py native",
     "ORR_ARENA_EXPORT_GATE_EVIDENCE", "/tmp/orr-arena-editor-export-",
     "        if: runner.os == 'Linux' && runner.arch == 'X64'\n"),
    ("Required Arena editor UI export native acceptance", "arena_editor_ui_export_gate.py native",
     "ORR_ARENA_UI_EXPORT_GATE_EVIDENCE", "/tmp/orr-arena-editor-ui-export-",
     "        if: runner.os == 'Linux' && runner.arch == 'X64'\n"),
)


def arena_bytecode_step(title, command, evidence_key, evidence_prefix, condition):
    return (f"      - name: {title}\n" + condition +
            "        shell: bash\n        timeout-minutes: 90\n        env:\n" +
            f"          {evidence_key}: {evidence_prefix}" +
            "${{ github.run_id }}-${{ github.run_attempt }}\n" +
            "          PYTHONDONTWRITEBYTECODE: '1'\n" +
            "          CARGO_BUILD_JOBS: '2'\n" +
            f"        run: python -B tools/{command}\n")


def inherited_bytecode_contract(sample):
    header, body = sample.split('    steps:\n', 1)
    assert 'PYTHONDONTWRITEBYTECODE' not in header, 'no job override of inherited setup'
    steps = re.split(r'(?=^      - )', body, flags=re.M)
    assert steps.pop(0) == ''
    assert steps.pop(0) == BYTECODE_SETUP, 'exact unconditional inherited setup'
    allowed = {arena_bytecode_step(*entry) for entry in ARENA_BYTECODE_STEPS}
    seen = set()
    for step in steps:
        if 'PYTHONDONTWRITEBYTECODE' not in step:
            continue
        # Comments separating the next step are not part of this command.
        lines = step.splitlines(keepends=True)
        while lines and (not lines[-1].strip() or lines[-1].lstrip().startswith('#')):
            lines.pop()
        executable = ''.join(lines)
        assert executable in allowed, 'inherited setting must not be overridden later'
        assert executable not in seen, 'duplicate Arena bytecode exception'
        seen.add(executable)


def check_bytecode_boundary_fixtures(test, text, contract):
    # Each independently owned command is checked, not just the first occurrence.
    for entry in ARENA_BYTECODE_STEPS:
        block = arena_bytecode_step(*entry)
        test.assertEqual(text.count(block), 1)
        unrelated = "      - name: Unrelated command boundary\n        run: echo unrelated\n"
        for side in ('before', 'after'):
            changed = unrelated + block if side == 'before' else block + unrelated
            with test.subTest(arena_command=entry[1], unrelated=side):
                contract(text.replace(block, changed, 1))
        mutations = {
            'empty': block.replace("PYTHONDONTWRITEBYTECODE: '1'", "PYTHONDONTWRITEBYTECODE: ''"),
            'disabled': block.replace("PYTHONDONTWRITEBYTECODE: '1'", "PYTHONDONTWRITEBYTECODE: '0'"),
            'dynamic': block.replace("PYTHONDONTWRITEBYTECODE: '1'", "PYTHONDONTWRITEBYTECODE: ${{ env.BYTECODE }}"),
            'duplicate key': block.replace("          PYTHONDONTWRITEBYTECODE: '1'\n", "          PYTHONDONTWRITEBYTECODE: '1'\n" * 2),
            'renamed step': block.replace(entry[0], 'Unrelated acceptance'),
            'lookalike command': block.replace(entry[1], entry[1] + '-lookalike'),
            'additional command': block.replace(entry[1], entry[1] + '; echo extra'),
            'shell reassignment': block.replace('run: python', 'run: PYTHONDONTWRITEBYTECODE=0 python'),
            'shell unset': block.replace('run: python', 'run: env -u PYTHONDONTWRITEBYTECODE python'),
            'multiline command': block.replace('run: python', 'run: |\n          unset PYTHONDONTWRITEBYTECODE\n          python'),
            'not step env': block.replace('        env:\n', '        with:\n'),
            'duplicate step': block + block,
        }
        for label, changed in mutations.items():
            with test.subTest(arena_command=entry[1], mutation=label), test.assertRaises(AssertionError):
                contract(text.replace(block, changed, 1))
        # A following unrelated step cannot borrow the prior step's exemption.
        for command in ('unset PYTHONDONTWRITEBYTECODE',
                        "echo 'PYTHONDONTWRITEBYTECODE=0' >> \"$GITHUB_ENV\"",
                        'PYTHONDONTWRITEBYTECODE=1 python -B unrelated.py'):
            extra = unrelated.replace('echo unrelated', command)
            with test.subTest(arena_command=entry[1], following=command), test.assertRaises(AssertionError):
                contract(text.replace(block, block + extra, 1))


def workflow_contract(text):
    # Reuse the protected checker unchanged: its exact sample header is part
    # of the Room/RAW16 contract, even though this setup belongs to sample-build.
    check_room_workflow(text)
    sample = workflow_job(text, 'sample-build')
    room = workflow_job(text, 'sample-room')
    aggregate = workflow_job(text, 'cross_platform_checksum_compare')
    assert 'static_model_uv_gate.py' not in room
    header = sample.split('    steps:', 1)[0]
    assert 'continue-on-error' not in header
    assert re.findall(r'^    if:[ \t]*(.*)$', header, re.M) == [], 'sample job is unconditional'
    body = sample.split('    steps:\n', 1)[1]
    steps = list(re.finditer(r'^      - ', body, re.M))
    assert len(steps) > 1 and body[:steps[1].start()] == BYTECODE_SETUP, 'exact unconditional first setup step'
    assert sample.count(BYTECODE_SETUP) == 1, 'unique bytecode setup in sample-build'
    arena = workflow_job(text, 'sample-arena-cpu')
    assert arena.count(BYTECODE_SETUP) == 1, 'unique bytecode setup in Arena CPU'
    inherited_bytecode_contract(arena)
    inherited_bytecode_contract(sample)
    assert 'os: [ubuntu-22.04, windows-latest]' in header
    assert 'needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, generated-user-game-native, static-game-actions-native, sample-arena-cpu, sample-fixture-preflight, sample-linked-prefabs, sample-room, static-model-authoring, room-wall-authoring]' in aggregate
    assert re.findall(r'^    if:[ \t]*(.*)$', aggregate, re.M) == ['${{ always() }}'], 'one failure-aware aggregate condition'
    authoring = workflow_job(text, 'static-model-authoring')
    assert authoring == AUTHORING_JOB, 'exact independent eight-row authoring matrix'
    assert aggregate.endswith(AUTHORING_AGGREGATE_STEP + ROOM_WALL_AGGREGATE_STEP), 'required fail-closed authoring results'
    assert aggregate.count(AUTHORING_AGGREGATE_STEP) == aggregate.count(ROOM_WALL_AGGREGATE_STEP) == 1
    assert text.count(AUTHORING_AGGREGATE_STEP) == 1, 'unique authoring result owner'
    assert 'static_model_uv_gate.py' not in sample and 'static_model_pivot_gate.py' not in sample
    assert 'Required static UV variant ' not in text, 'old serial UV routes must be relocated, not duplicated'
    assert 'Required static pivot ' not in sample, 'pivot must use independent matrix only'
    for title in ['Required static prop CPU CLI and bindings acceptance',
                  'Required static prop actual software GPU acceptance',
                  'Required static prop source-hidden Room export acceptance',
                  'Required static texture variant CPU CLI and bindings acceptance',
                  'Required static texture variant actual software GPU acceptance',
                  'Required static texture variant source-hidden Room export acceptance',
                  'Required animation clip extraction cpu acceptance',
                  'Required animation clip extraction gpu acceptance',
                  'RoomEscapeV1 actual GPU and source-hidden production export',
                  'Required generated Room GPU and source-hidden production export']:
        assert text.count('      - name: ' + title + '\n') == 1


def model_census_contract(source):
    # Inspect the complete test attribute block, so a narrowed/broadened cfg or
    # an ignored test cannot be concealed by an otherwise correct name census.
    declarations = re.findall(r'((?:^#\[[^\n]+\]\n)+)^fn ([A-Za-z0-9_]+)\(', source, re.M)
    actual = {name: attributes.splitlines() for attributes, name in declarations
              if '#[test]' in attributes.splitlines()}
    names = re.findall(r'^#\[test\]\s+fn ([A-Za-z0-9_]+)\(', source, re.M)
    assert len(names) == len(set(names)) == len(actual) == 32
    censuses = {platform: set(gate.model_tests(platform)) for platform in ('linux', 'darwin', 'win32')}
    assert set(actual) == set.union(*censuses.values())
    conditional = {
        'cli_symlink_inputs_ancestors_and_destinations_reject': '#[cfg(unix)]',
        'cli_non_utf8_option_names_and_logical_ids_reject': '#[cfg(unix)]',
        'cli_non_utf8_filesystem_inputs_work': '#[cfg(target_os = "linux")]',
        'cli_windows_nonlocal_prefixes_reject': '#[cfg(windows)]',
    }
    for name, attributes in actual.items():
        assert attributes == ([conditional[name], '#[test]'] if name in conditional else ['#[test]']), name
    common = set(actual) - set(conditional)
    assert len(common) == 28
    unix = {'cli_symlink_inputs_ancestors_and_destinations_reject', 'cli_non_utf8_option_names_and_logical_ids_reject'}
    assert censuses['linux'] == common | unix | {'cli_non_utf8_filesystem_inputs_work'}
    assert censuses['darwin'] == common | unix
    assert censuses['win32'] == common | {'cli_windows_nonlocal_prefixes_reject'}


class UvGateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name).resolve()
        self.repo = self.root / 'checkout'; self.repo.mkdir()
        environment = mock.patch.dict(os.environ, {}, clear=False)
        environment.start(); self.addCleanup(environment.stop)
        gate.initialize(self.repo, self.root / 'evidence', 'cpu')

    def test_complete_feature_source_contract(self):
        value = {name: {} for name in gate.FEATURE_PATHS + gate.INFRA_PATHS}
        for package in ('orr_model', 'orr_model_bindings', 'orr_render', 'orr_sample'):
            for name in ('Cargo.toml', 'src/lib.rs'):
                value[f'crates/{package}/{name}'] = {}
        gate.require_sources(value)
        self.assertEqual(len(gate.FEATURE_PATHS), 10)
        for name in value:
            missing = dict(value); del missing[name]
            with self.subTest(name=name), self.assertRaises(AssertionError):
                gate.require_sources(missing)

    def test_source_snapshot_fences_bytes_modes_and_new_paths(self):
        path = self.repo / 'source.rs'; path.write_text('before')
        gate.frozen = gate.snapshot()
        gate.source_fence()
        path.write_text('after')
        with self.assertRaises(AssertionError):
            gate.source_fence()
        path.write_text('before')
        if os.name != 'nt':
            original = path.stat().st_mode; path.chmod(0o700)
            with self.assertRaises(AssertionError):
                gate.source_fence()
            path.chmod(original)
        added = self.repo / 'new-test.rs'; added.write_text('new')
        with self.assertRaises(AssertionError):
            gate.source_fence()
        recorded = json.loads((gate.runner.evidence / 'gate-receipt.json').read_text())
        self.assertEqual(recorded['source_path_inventories']['source-fence'],
                         {'tracked': ['source.rs'], 'observed': ['new-test.rs', 'source.rs'],
                          'extra': ['new-test.rs'], 'missing': [], 'matches': False})
        # A Git-ignored bytecode artifact is still an extra source path, and its
        # exact name must be retained before rejection without deleting it.
        added.unlink()
        ignored = self.repo / 'tools/__pycache__/fixture.cpython-312.pyc'
        ignored.parent.mkdir(parents=True); ignored.write_bytes(b'ignored bytecode')
        path.unlink()
        with self.assertRaises(AssertionError):
            gate.check_source_inventory(['source.rs'], 'ignored-extra')
        self.assertTrue(ignored.is_file())
        recorded = json.loads((gate.runner.evidence / 'gate-receipt.json').read_text())
        self.assertEqual(recorded['source_path_inventories']['ignored-extra'],
                         {'tracked': ['source.rs'], 'observed': ['tools/__pycache__/fixture.cpython-312.pyc'],
                          'extra': ['tools/__pycache__/fixture.cpython-312.pyc'], 'missing': ['source.rs'], 'matches': False})

    def test_source_snapshot_rejects_symlinks(self):
        path = self.repo / 'regular'; path.write_text('source')
        if os.name == 'nt':
            # No symlink privilege assumption: enforce the same classification
            # rejection with an observed path result instead of skipping.
            with mock.patch.object(pathlib.Path, 'is_symlink', return_value=True):
                with self.assertRaises(AssertionError):
                    gate.snapshot()
        else:
            (self.repo / 'alias').symlink_to(path)
            with self.assertRaises(AssertionError):
                gate.snapshot()
            (self.repo / 'alias').unlink()
            (self.repo / 'directory-alias').symlink_to(self.root, target_is_directory=True)
            with self.assertRaises(AssertionError):
                gate.snapshot()

    def test_source_snapshot_covers_helper_and_fixture_producers(self):
        with mock.patch.object(gate, 'source_root', ROOT):
            source = gate.snapshot()
        gate.require_sources(source)
        for name in gate.FEATURE_PATHS + gate.INFRA_PATHS:
            self.assertEqual(source[name]['sha256'], hashlib.sha256((ROOT / name).read_bytes()).hexdigest())
        self.assertIn('crates/orr_model/src/import.rs', source)
        self.assertIn('crates/orr_model/tests/fixtures/model.glb', source)
        self.assertIn('crates/orr_sample/src/bin/room_escape.rs', source)
        self.assertIn('crates/orr_sample/src/bin/orr_export_room.rs', source)

    def test_actual_model_census_matches_current_source(self):
        source = (ROOT / 'crates/orr_model/tests/static_model_uv.rs').read_text()
        model_census_contract(source)
        for platform in (None, True, False, '', 'windows', 'macos', 'freebsd', 'linux2'):
            with self.subTest(platform=platform), self.assertRaises(AssertionError):
                gate.model_tests(platform)
        for original, changed in [
            ('#[cfg(target_os = "linux")]\n#[test]\nfn cli_non_utf8_filesystem_inputs_work(',
             '#[cfg(unix)]\n#[test]\nfn cli_non_utf8_filesystem_inputs_work('),
            ('#[cfg(target_os = "linux")]\n#[test]\nfn cli_non_utf8_filesystem_inputs_work(',
             '#[test]\nfn cli_non_utf8_filesystem_inputs_work('),
            ('#[cfg(unix)]\n#[test]\nfn cli_non_utf8_option_names_and_logical_ids_reject(',
             '#[cfg(target_os = "linux")]\n#[test]\nfn cli_non_utf8_option_names_and_logical_ids_reject('),
            ('#[cfg(windows)]\n#[test]\nfn cli_windows_nonlocal_prefixes_reject(',
             '#[cfg(unix)]\n#[test]\nfn cli_windows_nonlocal_prefixes_reject('),
            ('#[test]\nfn recipe_accepts_exact_required_object_and_whitespace(',
             '#[test]\n#[ignore]\nfn recipe_accepts_exact_required_object_and_whitespace('),
        ]:
            self.assertIn(original, source)
            with self.subTest(original=original, changed=changed), self.assertRaises(AssertionError):
                model_census_contract(source.replace(original, changed, 1))
        self.assertNotIn('UnixListener', source, 'fixtures must not require local sockets')

    def test_actual_consumer_census_matches_current_source(self):
        for package, target, name in [('orr_model_bindings', 'static_model_uv', gate.BINDINGS_TEST),
                                      ('orr_render', 'static_model_uv', gate.GPU_TEST),
                                      ('orr_sample', 'static_model_uv_export', gate.EXPORT_TEST)]:
            source = (ROOT / f'crates/{package}/tests/{target}.rs').read_text()
            actual = re.findall(r'#\[test\]\s*(?:#\[[^\n]*\]\s*)*fn ([A-Za-z0-9_]+)\(', source)
            self.assertEqual(actual, [name])
            self.assertRegex(source, r'#\[ignore[^\n]*\]')
            self.assertIn('ORR_STATIC_MODEL_UV_BIN', source)

    def producer(self):
        crate = self.repo / 'crates/orr_model'
        (crate / 'examples').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname="orr_model"\n')
        source = crate / 'examples/static_model_uv.rs'; source.write_text('fn main() {}')
        directory = gate.runner.target_dir / 'release/examples'; directory.mkdir(parents=True)
        binary = directory / ('static_model_uv.exe' if os.name == 'nt' else 'static_model_uv')
        binary.write_bytes(b'exact compiler artifact'); binary.chmod(0o755)
        gate.runner.report['source_inventory_sha256'] = 'a' * 64
        return {'reason': 'compiler-artifact', 'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_model@0.0.1',
                'target': {'name': 'static_model_uv', 'src_path': str(source),
                           'kind': ['example'], 'crate_types': ['bin']},
                'profile': {'test': False, 'opt_level': '3'}, 'features': ['import'],
                'executable': str(binary), 'fresh': False}

    def pin(self, producer):
        return gate.pin(json.dumps(producer) + '\n', 'producer', 'orr_model',
                        'static_model_uv', 'example', ['import'])

    def test_producer_binds_complete_prebuild_source(self):
        producer = self.producer(); copied = pathlib.Path(self.pin(producer))
        self.assertEqual(copied.read_bytes(), pathlib.Path(producer['executable']).read_bytes())
        self.assertEqual(gate.runner.report['producers']['producer']['source_inventory_sha256'], 'a' * 64)
        self.assertEqual(gate.runner.report['producers']['producer']['source_root'], str(self.repo))
        copied.write_bytes(b'changed')
        with self.assertRaises(AssertionError):
            gate.runner.fence()

    def test_producer_rejects_cached_wrong_source_and_duplicate_artifacts(self):
        producer = self.producer()
        for changed in [dict(producer, fresh=True), dict(producer, fresh=None),
                        dict(producer, features=[]), dict(producer, package_id='registry+unknown'),
                        dict(producer, target=dict(producer['target'], src_path=str(self.repo / 'wrong.rs')))]:
            with self.subTest(changed=changed), self.assertRaises(AssertionError):
                self.pin(changed)
        with self.assertRaises(AssertionError):
            gate.pin((json.dumps(producer) + '\n') * 2, 'duplicate', 'orr_model',
                     'static_model_uv', 'example', ['import'])

    def test_inventory_and_result_fail_closed(self):
        gate.runner.inventory(listing(['alpha']), ['alpha'])
        gate.runner.result(passed(['alpha']), ['alpha'])
        for text in [listing(['other']), listing(['alpha', 'alpha']), listing(['alpha']).replace('1 tests', '0 tests')]:
            with self.assertRaises(AssertionError):
                gate.runner.inventory(text, ['alpha'])
        for text in [passed(['alpha']) + 'SKIP: unavailable', passed(['alpha']).replace('0 ignored', '1 ignored'),
                     passed(['alpha']).replace('alpha', 'other')]:
            with self.assertRaises(AssertionError):
                gate.runner.result(text, ['alpha'])

    def test_required_ignored_consumer_runs_exactly(self):
        with mock.patch.object(gate, 'run', side_effect=[listing(['alpha']), listing(['alpha']), passed(['alpha'])]) as run:
            gate.test('consumer', '/exact/binary', ['alpha'], ignored=True)
        self.assertEqual(run.call_args_list[-1], mock.call('consumer-run',
                         ['/exact/binary', '--ignored', '--exact', 'alpha', '--test-threads=1']))

    def test_unrun_modes_cannot_be_claimed(self):
        for phase, completed in [('cpu', ('model', 'bindings')), ('gpu', ('gpu',)), ('export', ('export',))]:
            gate.mode = phase
            expected = {name: ('PASSED' if name in completed else 'NOT RUN') for name in ('model', 'bindings', 'gpu', 'export')}
            gate.runner.report['coverage'] = expected.copy(); gate.check_coverage()
            for name in expected:
                changed = expected.copy(); changed[name] = 'NOT RUN' if changed[name] == 'PASSED' else 'PASSED'
                gate.runner.report['coverage'] = changed
                with self.assertRaises(AssertionError):
                    gate.check_coverage()

    def variant_fixture(self, directory=None, stem='rotate', source_name='source.model.json'):
        root = directory or (self.root / 'variant'); root.mkdir(exist_ok=True)
        material = {'base_color': [1.0, 1.0, 1.0, 0.25], 'image': 0, 'linear_filter': False,
                    'wrap_s': 'repeat', 'wrap_t': 'mirror'}
        vertices = [{'position': [x, y, 0.0], 'normal': [0.0, 0.0, 1.0], 'uv': uv}
                    for x, y, uv in [(0.0, 0.0, [0.0, 0.0]), (1.0, 0.0, [1.0, 0.0]),
                                     (0.0, 1.0, [0.0, 1.0])]]
        primitive = {'id': 'models/source.glb#node=1/mesh=0/primitive=0', 'vertices': vertices,
                     'indices': [0, 1, 2], 'material': 0,
                     'transform': [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0],
                                   [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]}
        source = {'format': 'orr_static_model', 'version': 1, 'asset_id': 'models/source.glb',
                  'dependencies': [{'uri': '$source', 'sha256': 'f' * 64}],
                  'primitives': [primitive, dict(primitive, id='models/source.glb#node=7/mesh=0/primitive=0'),
                                 dict(primitive, id='models/source.glb#node=7/mesh=0/primitive=1', material=1),
                                 dict(primitive, id='models/source.glb#node=1/mesh=0/primitive=1', material=1)],
                  'materials': [material, dict(material, base_color=[0.25, 0.5, 0.75, 0.5]), dict(material, image=2)],
                  'images': [{'width': 2, 'height': 2, 'rgba8': [255, 0, 0, 255, 0, 255, 0, 255,
                                                               0, 0, 255, 255, 128, 128, 128, 255]},
                             {'width': 1, 'height': 1, 'rgba8': [255, 255, 0, 255]},
                             {'width': 1, 'height': 1, 'rgba8': [17, 31, 47, 63]}]}
        source['primitives'] = [copy.deepcopy(primitive) for primitive in source['primitives']]
        (root / source_name).write_text(json.dumps(source))
        recipe_file, cooked_file = stem + '.uv.json', stem + '.model.json'
        if stem == 'rotate':
            offset, scale, turns = [0.25, 0.125], [1.5, 0.5], 1
            transformed = [[0.25, 0.125], [0.25, 1.625], [-0.25, 0.125]]
        else:
            assert stem == 'counter'
            offset, scale, turns = [-0.375, 0.75], [0.5, 1.5], 3
            transformed = [[-0.375, 0.75], [-0.375, 0.25], [1.125, 0.75]]
        recipe = {'format': 'orr_static_material_uv', 'version': 1, 'source': 'inputs/source.model.json',
                  'material': 0, 'offset': offset, 'scale': scale, 'quarter_turns': turns}
        (root / recipe_file).write_text(json.dumps(recipe))
        asset_id = 'variants/' + recipe_file
        cooked = copy.deepcopy(source); cooked['asset_id'] = asset_id
        for primitive in cooked['primitives']:
            primitive['id'] = asset_id + primitive['id'][len(source['asset_id']):]
            if primitive['material'] == 0:
                for vertex, uv in zip(primitive['vertices'], transformed):
                    vertex['uv'] = uv[:]
        cooked['dependencies'] = [{'uri': uri, 'sha256': gate.runner.digest(root / file)} for uri, file in
                                  [('$source', recipe_file), (recipe['source'], source_name)]]
        (root / cooked_file).write_text(json.dumps(cooked))
        return root, (source_name, recipe_file, cooked_file, asset_id), cooked

    def test_variant_keeps_source_geometry_and_slots(self):
        root, args, cooked = self.variant_fixture()
        self.assertEqual(gate.check_variant(root, *args), cooked)
        root, args, cooked = self.variant_fixture(root, 'counter')
        self.assertEqual(gate.check_variant(root, *args), cooked)

    def test_variant_rejects_geometry_material_and_dependency_mutations(self):
        root, args, original = self.variant_fixture()
        mutations = []
        for field, value in [('asset_id', 'different.uv.json'), ('dependencies', list(reversed(original['dependencies']))),
                             ('images', original['images'] + original['images']), ('primitives', original['primitives'][:1]),
                             ('materials', [dict(original['materials'][0], base_color=[0, 0, 0, 0]), original['materials'][1]])]:
            changed = copy.deepcopy(original); changed[field] = value; mutations.append(changed)
        for index in range(3):
            changed = copy.deepcopy(original); changed['primitives'][index]['vertices'][0]['position'][0] = 1.0
            mutations.append(changed)
            changed = copy.deepcopy(original); changed['primitives'][index]['vertices'][0]['uv'][0] += 0.125
            mutations.append(changed)
        for changed in mutations:
            (root / args[2]).write_text(json.dumps(changed))
            with self.assertRaises(AssertionError):
                gate.check_variant(root, *args)

    def test_gpu_capture_inventory_and_pixels(self):
        root, _ = self.gpu_fixture()
        gate.check_gpu(root)

    def test_gpu_rejects_receipt_pixel_and_provenance_mutations(self):
        root, receipt = self.gpu_fixture()
        first = gate.GPU_CASES[0]
        cases = []
        changed = copy.deepcopy(receipt); del changed['cases'][first]; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['producer_fnv1a64'] = '0' * 16; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['cases'][first]['shared_material_users_checked'] = False; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['cases'][first]['control_pixel_differences']['wrong_order'] = 0; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['cases'][first]['assets']['variants'][0]['cooked']['fnv1a64'] = '0' * 16; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['cases'][first]['coverage']['rotate']['selected_pixels_per_model_node'][0][1] += 1; cases.append(changed)
        for changed in cases:
            (root / 'receipt.json').write_text(json.dumps(changed))
            with self.assertRaises(AssertionError):
                gate.check_gpu(root)
        # Rehash RGBA/PPM and recompute controls after corrupting an independently
        # sampled selected UV pixel. Only the separate pixel oracle can reject it.
        changed = copy.deepcopy(receipt); item = changed['cases'][first]
        path = root / (first + '-rotate.rgba'); original = path.read_bytes(); rgba = bytearray(original)
        expected, _ = gate.oracle_pixels(((0, 1),), False, 'repeat')
        markers = set(gate.marker_offsets())
        offset = next(offset for offset, color in expected if color != (0, 0, 0, 255) and offset not in markers)
        rgba[offset:offset + 4] = bytes([61, 79, 97, 255]); path.write_bytes(rgba)
        ppm_path = root / (first + '-rotate.ppm'); original_ppm = ppm_path.read_bytes()
        ppm_path.write_bytes(b'P6\n256 256\n255\n' + bytes(value for index, value in enumerate(rgba) if index % 4 != 3))
        for entry in item['frames']['rotate'].values():
            data = (root / entry['file']).read_bytes(); entry.update(bytes=len(data), fnv1a64=gate.fnv1a64(data))
        for name, (a, b) in item['control_pairs'].items():
            item['control_pixel_differences'][name] = gate.difference(
                (root / item['frames'][a]['raw']['file']).read_bytes(), (root / item['frames'][b]['raw']['file']).read_bytes())
        (root / 'receipt.json').write_text(json.dumps(changed))
        with self.assertRaisesRegex(AssertionError, 'independent UV pixel oracle'):
            gate.check_gpu(root)
        path.write_bytes(original); ppm_path.write_bytes(original_ppm)
        (root / 'receipt.json').write_text(json.dumps(receipt))
        extra = root / 'unexpected.rgba'; extra.write_bytes(b'uninventoried')
        with self.assertRaisesRegex(AssertionError, 'exact capture inventory'):
            gate.check_gpu(root)
        extra.unlink()
        path.write_bytes(b'wrong dimensions')
        with self.assertRaises(AssertionError):
            gate.check_gpu(root)

    def test_uv_oracle_hand_calculated_sampling(self):
        for wrap in ('repeat', 'mirror', 'clamp'):
            self.assertEqual(gate.uv_sample((.25, .25), False, wrap)[0], (255, 0, 0, 255))
            self.assertEqual(gate.uv_sample((.75, .25), False, wrap)[0], (0, 255, 0, 255))
            self.assertEqual(gate.uv_sample((.25, .75), False, wrap)[0], (0, 0, 255, 255))
            self.assertEqual(gate.uv_sample((.75, .75), True, wrap)[0], (128, 128, 128, 255))
            self.assertEqual(gate.uv_sample((.5, .25), True, wrap)[0], (188, 188, 0, 255))
            self.assertEqual(gate.uv_sample((.25, .5), True, wrap)[0], (188, 0, 188, 255))
            self.assertIsNone(gate.uv_sample((.5, .25), False, wrap))
        self.assertEqual(gate.uv_sample((-.25, .25), False, 'repeat')[0], (0, 255, 0, 255))
        self.assertEqual(gate.uv_sample((-.25, .25), False, 'mirror')[0], (255, 0, 0, 255))
        self.assertEqual(gate.uv_sample((1.25, .25), False, 'mirror')[0], (0, 255, 0, 255))
        self.assertEqual(gate.uv_sample((1.25, .25), False, 'repeat')[0], (255, 0, 0, 255))

    def test_export_capture_inventory_and_provenance(self):
        root, _ = self.export_fixture()
        gate.check_export(root)
        # Known independent byte vectors cover every PNG predictor, including
        # the nonzero left/above/corner case in the second row of Paeth.
        rows = [(list(range(1, 9)), list(range(9, 17))),
                ([1, 2, 3, 4, 4, 4, 4, 4], [9, 10, 11, 12, 4, 4, 4, 4]),
                (list(range(1, 9)), [8] * 8),
                ([1, 2, 3, 4, 5, 5, 6, 6], [9, 9, 10, 10, 6, 6, 6, 6]),
                ([1, 2, 3, 4, 4, 4, 4, 4], [8, 8, 8, 8, 4, 4, 4, 4])]
        def chunk(kind, value):
            return struct.pack('>I', len(value)) + kind + value + struct.pack('>I', zlib.crc32(kind + value) & 0xffffffff)
        file = self.root / 'predictors.png'
        for filter_type, (first, second) in enumerate(rows):
            raw = bytes([filter_type, *first, filter_type, *second])
            data = (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 6, 0, 0, 0))
                    + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))
            file.write_bytes(data)
            self.assertEqual(gate.png_pixels(file, 2, 2), bytes(range(1, 17)))
        for corrupt in [data[:-1], data + b'trailing', data[:29] + b'wrong CRC' + data[33:]]:
            file.write_bytes(corrupt)
            with self.assertRaises(AssertionError):
                gate.png_pixels(file, 2, 2)

    def test_export_rejects_receipt_frame_and_inventory_mutations(self):
        root, accepted = self.export_fixture()
        for field, value in [('source_hidden', False), ('runtime_sha256', '0' * 64), ('visible_changed_pixels', 21)]:
            (root / 'production-receipt.json').write_text(json.dumps(dict(accepted, **{field: value})))
            self.export_inventory(root)
            with self.assertRaises(AssertionError):
                gate.check_export(root)
        (root / 'production-receipt.json').write_text(json.dumps(accepted))
        original = (root / 'control-original.png').read_bytes()
        (root / 'control-original.png').write_bytes((root / 'runtime-initial-source.png').read_bytes())
        self.export_inventory(root)
        with self.assertRaises(AssertionError):
            gate.check_export(root)
        (root / 'control-original.png').write_bytes(original)
        control = root / 'control-offscreen-project'
        sidecar = control / 'room.models.json'; original_sidecar = sidecar.read_bytes()
        for mutation in ('empty bindings', 'visible translation'):
            document = json.loads(original_sidecar)
            if mutation == 'empty bindings':
                document['bindings'] = {}
            else:
                next(iter(document['bindings'].values()))['transform']['translation'] = [-2.0, 1.0, 0.0]
            sidecar.write_text(json.dumps(document))
            changed = copy.deepcopy(accepted)
            changed['control_project_inventories']['offscreen'] = {
                file.relative_to(control).as_posix(): [file.stat().st_size, gate.runner.digest(file)]
                for file in control.rglob('*') if file.is_file()}
            (root / 'production-receipt.json').write_text(json.dumps(changed))
            self.export_inventory(root)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                gate.check_export(root)
        sidecar.write_bytes(original_sidecar)
        (root / 'production-receipt.json').write_text(json.dumps(accepted))
        extra = control / 'uninventoried-empty'; extra.mkdir()
        self.export_inventory(root)
        with self.assertRaisesRegex(AssertionError, 'unexpected empty capture directories'):
            gate.check_export(root)
        extra.rmdir()
        (root / 'prepared-project-initial-export.frame').write_bytes(b'changed')
        self.export_inventory(root)
        with self.assertRaises(AssertionError):
            gate.check_export(root)

    def gpu_fixture(self):
        root = self.root / 'gpu'; root.mkdir()
        for name in ('crates/orr_render/tests/static_model_uv.rs',
                     'crates/orr_model/examples/static_model_uv.rs',
                     'crates/orr_model/examples/static_model_uv/authoring.rs',
                     'crates/orr_model/tests/fixtures/model.glb'):
            path = self.repo / name; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(('current source ' + name).encode())
        cli = gate.runner.evidence / 'uv-cli'; cli.write_bytes(b'current CLI producer')
        gate.runner.report['producers']['uv-cli'] = {'file': cli.name, 'sha256': gate.runner.digest(cli)}
        def reference(name):
            data = (root / name).read_bytes(); value = 0xcbf29ce484222325
            for byte in data:
                value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
            return {'file': name, 'bytes': len(data), 'fnv1a64': f'{value:016x}'}
        def save(name, data):
            (root / name).write_bytes(data)
            return reference(name)
        original = save('source.glb', (self.repo / 'crates/orr_model/tests/fixtures/model.glb').read_bytes())
        cases = {}
        markers = gate.marker_offsets()
        for case in gate.GPU_CASES:
            filter_, wrap = case.split('-'); linear = filter_ == 'linear'
            assets = {'original_glb': original, 'variants': [], 'source_hidden_after_install': True,
                      'cooked_reload_before_render': True, 'installed_files': 2}
            source_name = case + '-source.model.json'
            for name in ('rotate', 'counter'):
                _, args, cooked = self.variant_fixture(root, name, source_name)
                source = json.loads((root / source_name).read_text())
                for document in (source, cooked):
                    for material in document['materials']:
                        material.update(linear_filter=linear, wrap_s=wrap, wrap_t=wrap)
                    for material in document['materials'][:2]:
                        material['base_color'][:3] = [1.0, 1.0, 1.0]
                (root / source_name).write_text(json.dumps(source))
                cooked['dependencies'][1]['sha256'] = gate.runner.digest(root / source_name)
                (root / args[2]).write_text(json.dumps(cooked))
                for suffix in ('.uv.json', '.model.json'):
                    (root / (name + suffix)).rename(root / (case + '-' + name + suffix))
                assets['variants'].append({'asset_id': args[3], 'recipe': reference(case + '-' + name + '.uv.json'),
                                           'cooked': reference(case + '-' + name + '.model.json')})
            assets['source'] = reference(source_name)
            pixels, coverage = {}, {}
            arrangements = {'source': ((0, 0),), 'rotate': ((0, 1),), 'counter': ((0, 2),),
                            'variants_together': ((-2, 1), (2, 2))}
            for name, arrangement in arrangements.items():
                # Synthetic accepted fixture pixels use the checker mask. Hand
                # vectors below independently verify the sampler; corruption
                # tests prove rehashed pixels cannot defeat semantic checks.
                expected, counts = gate.oracle_pixels(arrangement, linear, wrap)
                data = bytearray(b'\0\0\0\xff' * 65536)
                for offset, color in expected:
                    data[offset:offset + 4] = bytes(color)
                pixels[name] = bytes(data); coverage[name] = counts
            # Nearest boundary omissions may differ by variant; actual untouched
            # material readbacks still agree byte-for-byte across those pixels.
            for name in ('rotate', 'counter'):
                data = bytearray(pixels[name])
                for offset in markers:
                    data[offset:offset + 4] = pixels['source'][offset:offset + 4]
                pixels[name] = bytes(data)
            pixels['variants_reversed'] = pixels['variants_together']
            for index, name in enumerate(('wrong_order', 'all_material', 'only_first_user', 'wrong_transform')):
                data = bytearray(pixels['rotate'])
                if name == 'all_material':
                    offsets = markers
                else:
                    offsets = [(y * 256 + x) * 4 for y in range(64, 192) for x in range(64, 192)]
                for offset in offsets:
                    data[offset:offset + 4] = bytes([250, 15 + index * 10, 240, 255])
                pixels[name] = bytes(data)
            frames = {}
            for name in gate.GPU_FRAMES:
                stem = case + '-' + name.replace('_', '-')
                rgba = pixels[name]
                frames[name] = {'raw': save(stem + '.rgba', rgba),
                                'image': save(stem + '.ppm', b'P6\n256 256\n255\n' + bytes(value for index, value in enumerate(rgba) if index % 4 != 3))}
            cases[case] = {'linear_filter': linear, 'wrap_s': wrap, 'wrap_t': wrap, 'assets': assets,
                           'frames': frames, 'coverage': coverage, 'control_pairs': copy.deepcopy(gate.CONTROL_PAIRS),
                           'control_pixel_differences': {key: gate.difference(pixels[a], pixels[b]) for key, (a, b) in gate.CONTROL_PAIRS.items()},
                           'shared_material_users_checked': True, 'unaffected_material_byte_identical': True,
                           'same_image_unselected_material': True, 'distinct_recipe_namespaces': True,
                           'reversed_batch_order_byte_identical': True, 'opaque_alpha_checked': True,
                           'unaffected_material_pixels_checked': len(markers)}
        adapter = {'software': True, 'backend': 'Vulkan'}
        (root / 'adapter.json').write_text(json.dumps(adapter))
        receipt = {'schema': 1, 'status': 'passed', 'width': 256, 'height': 256, 'adapter': adapter, 'cases': cases,
                   'tolerances': {'color_bytes': 3, 'minimum_control_differences': 500, 'minimum_unaffected_differences': 30}}
        for field, path in [('test_source_fnv1a64', self.repo / 'crates/orr_render/tests/static_model_uv.rs'),
                            ('cli_source_fnv1a64', self.repo / 'crates/orr_model/examples/static_model_uv.rs'),
                            ('authoring_source_fnv1a64', self.repo / 'crates/orr_model/examples/static_model_uv/authoring.rs'),
                            ('producer_fnv1a64', cli)]:
            receipt[field] = gate.fnv1a64(path.read_bytes())
        (root / 'receipt.json').write_text(json.dumps(receipt))
        return root, receipt

    def export_inventory(self, root):
        files = {path.relative_to(root).as_posix(): [path.stat().st_size, gate.runner.digest(path)]
                 for path in root.rglob('*') if path.is_file() and path.name != 'capture-inventory.json'}
        (root / 'capture-inventory.json').write_text(json.dumps(files))

    def export_fixture(self):
        root = self.root / 'export'; root.mkdir()
        for path in ('crates/orr_sample/tests/static_model_uv_export.rs',
                     'crates/orr_model/examples/static_model_uv.rs',
                     'crates/orr_model/examples/static_model_uv/authoring.rs'):
            file = self.repo / path; file.parent.mkdir(parents=True, exist_ok=True); file.write_text('current source ' + path)
        variants = []
        for name in ('rotate', 'counter'):
            _, args, _ = self.variant_fixture(root, stem=name, source_name='authoring-source.model.json')
            recipe = json.loads((root / args[1]).read_text())
            variants.append({'name': name, 'asset_id': args[3], 'material': 0,
                             'offset': recipe['offset'], 'scale': recipe['scale'], 'quarter_turns': recipe['quarter_turns'],
                             'recipe_sha256': gate.runner.digest(root / args[1]),
                             'cooked_sha256': gate.runner.digest(root / args[2]), 'cli_stdout': 'published'})
        (root / 'scene.glb').write_bytes(b'GLB')
        def png(value):
            def chunk(kind, data):
                return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
            return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1024, 768, 8, 6, 0, 0, 0))
                    + chunk(b'IDAT', zlib.compress((b'\0' + bytes([value, value, value, 255]) * 1024) * 768)) + chunk(b'IEND', b''))
        observations = {}
        for name, ticks, held in [('initial', 0, ''), ('moved', 18, 'right'), ('collision', 180, 'right'), ('interacted', 18, 'right,interact')]:
            output = f'room capture adapter: fixture\nroom tick: {ticks} checksum: 0x123\nroom key: 1 won: 0\n'
            item = {'ticks': ticks, 'held': held, 'checksum': '0x123', 'source_stdout': output, 'export_stdout': output}
            for route in ('source', 'export'):
                for suffix, stem, data in [('png', 'runtime', png(ticks)), ('frame', 'prepared-project', name.encode())]:
                    file = root / f'{stem}-{name}-{route}.{suffix}'; file.write_bytes(data)
                    item[f'{route}_{suffix}_sha256'] = gate.runner.digest(file)
            observations[name] = item
        (root / 'control-offscreen.png').write_bytes(png(1))
        (root / 'control-original.png').write_bytes(png(2))
        accepted = {'source_hidden': True, 'read_only_bundle': True, 'no_visible_models_control': True, 'offscreen_control_preserves_bindings': True, 'visible_changed_pixels': 1024 * 768,
                    'uv_changed_pixels': 1024 * 768, 'observations': observations,
                    'control_stdout': [observations['initial']['source_stdout']] * 2}
        for field, label in [('runtime_sha256', 'room_escape'), ('exporter_sha256', 'orr_export_room'),
                             ('uv_cli_sha256', 'uv-cli'), ('acceptance_executable_sha256', 'export-test')]:
            value = hashlib.sha256(label.encode()).hexdigest()
            accepted[field] = value; gate.runner.report['producers'][label] = {'sha256': value}
        def fixture_project(folder, slug, assets, hidden=False):
            project = root / folder; project.mkdir()
            hashes = {name: hashlib.sha256(data).hexdigest() for name, data in sorted(assets.items())}
            manifest = {'schema': 1, 'name': slug, 'version': '1.0.0', 'engine': '*',
                        'capabilities': ['models'], 'dependencies': {}, 'files': sorted(assets)}
            identity = hashlib.sha256(json.dumps(['orr-package-v1', manifest, hashes], separators=(',', ':')).encode()).hexdigest()
            object_path = '.orr/packages/objects/' + identity + '/'
            bindings = {}
            for index, name in enumerate(('rotate.model.json', 'counter.model.json') if len(assets) == 2 else ('original.model.json',) * 2):
                bindings[f'e_{index + 1:08x}'] = {'kind': 'static', 'package': slug, 'asset': name,
                    'package_digest': identity, 'source_hash': hashes[name], 'transform': {
                        'translation': [1000.0] * 3 if hidden else [-2.0 if index == 0 else 2.0, 1.0, 0.0],
                        'rotation': [0.0, 0.0, 0.0, 1.0], 'scale': [1.0, 1.0, 1.0]}}
            documents = {'orr.project.json': {}, 'room.camera.json': {},
                         'room.models.json': {'version': 2, 'scene': 'room.scene.yaml', 'project': '.', 'bindings': bindings},
                         'orr.packages.lock.json': {'schema': 1, 'direct': {slug: '1.0.0'}, 'packages': {
                             slug: {'manifest': manifest, 'digest': identity, 'files': hashes}}},
                         object_path + 'orr.package.json': manifest}
            for name, document in documents.items():
                file = project / name; file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(json.dumps(document))
            (project / 'room.scene.yaml').write_bytes(b'canonical unchanged scene')
            for name, data in assets.items():
                (project / object_path / name).write_bytes(data)
            inventory = {file.relative_to(project).as_posix(): [file.stat().st_size, gate.runner.digest(file)]
                         for file in project.rglob('*') if file.is_file()}
            return identity, inventory
        assets = {name + '.model.json': (root / (name + '.model.json')).read_bytes() for name in ('rotate', 'counter')}
        package, authored_inventory = fixture_project('authored-project', 'uv-static-props', assets)
        _, offscreen_inventory = fixture_project('control-offscreen-project', 'uv-static-props', assets, hidden=True)
        _, original_inventory = fixture_project('control-original-project', 'original-uv-control',
                                                {'original.model.json': (root / 'authoring-source.model.json').read_bytes()})
        accepted['control_project_inventories'] = {'offscreen': offscreen_inventory, 'original': original_inventory}
        files = {'project/' + name: value for name, value in authored_inventory.items()}
        files['bin/room_escape'] = [10, accepted['runtime_sha256']]
        files['run-room-escape'] = [4, hashlib.sha256(b'run!').hexdigest()]
        manifest = {'schema': 1, 'payload': {'profile': 'room-escape-authored-linux-x86_64-v1',
                    'runtime': {'sha256': accepted['runtime_sha256']}, 'packages': {'uv-static-props': package},
                    'files': [{'path': name, 'bytes': value[0], 'sha256': value[1]} for name, value in files.items()]}}
        (root / 'orr.export.json').write_text(json.dumps(manifest))
        files['orr.export.json'] = [(root / 'orr.export.json').stat().st_size, gate.runner.digest(root / 'orr.export.json')]
        accepted['exported_file_inventory'] = files
        source = {'test': gate.EXPORT_TEST, 'source_asset_id': 'models/scene.glb', 'source_path': 'inputs/source.model.json',
                  'glb_sha256': gate.runner.digest(root / 'scene.glb'),
                  'source_cooked_sha256': gate.runner.digest(root / 'authoring-source.model.json'),
                  'variants': variants, 'original_source_exported': False,
                  'initial_frame_sha256': gate.runner.digest(root / 'prepared-project-initial-source.frame')}
        for field, name in [('test_source_sha256', 'crates/orr_sample/tests/static_model_uv_export.rs'),
                            ('cli_source_sha256', 'crates/orr_model/examples/static_model_uv.rs'),
                            ('authoring_source_sha256', 'crates/orr_model/examples/static_model_uv/authoring.rs')]:
            source[field] = gate.runner.digest(self.repo / name)
        (root / 'acceptance-source.json').write_text(json.dumps(source))
        (root / 'production-receipt.json').write_text(json.dumps(accepted))
        self.export_inventory(root)
        return root, accepted

    def test_command_fences_order_and_mutation(self):
        self.assertEqual(gate.runner.report['python_runtime'], gate.python_runtime())
        self.assertEqual(gate.runner.report['python_runtime']['optimize'], 0)
        self.assertIs(gate.runner.report['python_runtime']['__debug__'], True)
        # Keep the reviewed lifecycle and its Windows admission launcher in the
        # normal parent environment. Only its already-owned bootstrap changes
        # the environment and launches the fresh interpreter being tested.
        # subprocess.call explicitly propagates its exit status on Windows too.
        bootstrap = (
            'import json,os,subprocess,sys; '
            'environment=os.environ.copy(); '
            'environment["PYTHONDONTWRITEBYTECODE"]="1"; '
            'environment["ORR_STATIC_UV_EVIDENCE"]=sys.argv[2]; '
            'environment.pop("PYTHONOPTIMIZE",None); '
            'environment.update(json.loads(sys.argv[4])); '
            'sys.exit(subprocess.call([sys.executable,"-B",*json.loads(sys.argv[3]),sys.argv[1],"cpu"],env=environment))'
        )
        cases = [('flag-O', ['-O'], None, 1), ('flag-OO', ['-OO'], None, 2),
                 ('inherited-1', [], '1', 1), ('inherited-2', [], '2', 2),
                 ('ignored-inherited-1', ['-E'], '1', 0)]
        for label, flags, inherited, optimized in cases:
            candidate = self.root / ('candidate-' + label)
            source = candidate / 'tools'; source.mkdir(parents=True)
            for name in ('static_model_uv_gate.py', 'static_model_extract_gate.py'):
                (source / name).write_bytes((ROOT / 'tools' / name).read_bytes())
            before = {path.relative_to(candidate).as_posix(): path.read_bytes()
                      for path in candidate.rglob('*') if path.is_file()}
            destination = self.root / ('rejected-evidence-' + label)
            environment = {} if inherited is None else {'PYTHONOPTIMIZE': inherited}
            output = gate.runner.run('optimized-' + label,
                [sys.executable, '-B', '-c', bootstrap, str(source / 'static_model_uv_gate.py'),
                 str(destination), json.dumps(flags), json.dumps(environment)], expected=2, seconds=20)
            lines = output.splitlines()
            self.assertEqual(len(lines), 1)
            prefix = 'STATIC_UV_GATE_PYTHON_REJECTED '
            self.assertTrue(lines[0].startswith(prefix))
            observed = json.loads(lines[0][len(prefix):])
            self.assertEqual(observed['optimize'], optimized)
            self.assertIs(observed['__debug__'], optimized == 0)
            self.assertEqual(observed['PYTHONOPTIMIZE'], inherited)
            self.assertIs(observed['dont_write_bytecode'], True)
            self.assertEqual(observed['ignore_environment'], int('-E' in flags))
            self.assertFalse(destination.exists(), 'rejection must precede evidence creation')
            self.assertFalse((candidate / 'target').exists(), 'rejection must precede target creation')
            self.assertEqual(before, {path.relative_to(candidate).as_posix(): path.read_bytes()
                                     for path in candidate.rglob('*') if path.is_file()})
            self.assertEqual(gate.runner.report['producers'], {})
            self.assertTrue(gate.runner.report['commands'][-1]['owned_children_reaped'])
            print('OPTIMIZED_PYTHON_REJECTION ' + json.dumps(
                {'case': label, 'runtime': observed, 'exit': 2, 'candidate_evidence_created': False,
                 'candidate_target_created': False, 'candidate_source_unchanged': True}, sort_keys=True), flush=True)
        path = self.repo / 'source.rs'; path.write_bytes(b'before')
        gate.frozen = gate.snapshot()
        with mock.patch.object(gate.runner, 'run', return_value='ok') as run:
            self.assertEqual(gate.run('one', ['command']), 'ok')
            run.assert_called_once_with('one', ['command'], expected=0, seconds=1200)
        path.write_bytes(b'after')
        with mock.patch.object(gate.runner, 'run') as run:
            with self.assertRaises(AssertionError):
                gate.run('two', ['command'])
            run.assert_not_called()
        path.write_bytes(b'before')
        def mutate(*args, **kwargs):
            path.write_bytes(b'after'); return 'success claim'
        with mock.patch.object(gate.runner, 'run', side_effect=mutate):
            with self.assertRaises(AssertionError):
                gate.run('three', ['command'])

    def test_prebuild_source_inventory_rejects_ignored_and_duplicate_paths(self):
        (self.repo / 'known.rs').write_text('known')
        gate.check_source_inventory(['known.rs'], 'matching')
        for tracked in [[], ['known.rs', 'known.rs'], ['known.rs', 'missing.rs']]:
            with self.assertRaises(AssertionError):
                gate.check_source_inventory(tracked, 'mismatch')
        ignored = self.repo / '.hidden-untracked'; ignored.write_text('ignored bytes')
        with self.assertRaises(AssertionError):
            gate.check_source_inventory(['known.rs'], 'ignored')
        self.assertEqual(gate.runner.report['source_path_inventories']['ignored']['extra'], ['.hidden-untracked'])
        self.assertTrue(ignored.exists())

    def test_variant_rejects_uv_order_zero_sign_and_nonfinite(self):
        recipe = {'offset': [0.25, 0.125], 'scale': [1.5, 0.5], 'quarter_turns': 1}
        self.assertEqual(gate.transformed_uv([1.0, 0.0], recipe), [0.25, 1.625])
        # Rotate-before-scale would produce y=.625, whereas scale-before-turn is 1.625.
        self.assertFalse(gate.bit_equal([0.25, 0.625], gate.transformed_uv([1.0, 0.0], recipe)))
        identity = {'offset': [-0.0, 0.0], 'scale': [1.0, 1.0], 'quarter_turns': 0}
        self.assertTrue(gate.bit_equal(gate.transformed_uv([-0.0, 0.0], identity), [-0.0, 0.0]))
        self.assertFalse(gate.bit_equal([-0.0, 0.0], [0.0, 0.0]))
        rounded_identity = {'offset': [-1e-100, 1e-100], 'scale': [1.00000001, .99999999], 'quarter_turns': 0}
        self.assertTrue(gate.bit_equal(gate.transformed_uv([-0.0, 0.0], rounded_identity), [-0.0, 0.0]))
        for scale in ([1e-100, 1.0], [1.0, 1e-100]):
            with self.assertRaisesRegex(AssertionError, 'strictly positive admitted f32 scales'):
                gate.transformed_uv([0.0, 1.0], dict(identity, scale=scale))
        for uv in ([float('nan'), 0.0], [float('inf'), 0.0], [65536.0, 65536.0]):
            with self.assertRaises((AssertionError, OverflowError)):
                gate.transformed_uv(uv, recipe)
        root, args, original = self.variant_fixture()
        changed = copy.deepcopy(original)
        changed['primitives'][0]['vertices'][1]['uv'] = [0.25, 0.625]
        (root / args[2]).write_text(json.dumps(changed))
        with self.assertRaises(AssertionError):
            gate.check_variant(root, *args)

    def test_capture_json_rejects_duplicate_and_nonfinite(self):
        path = self.root / 'receipt.json'
        for content in ('{"status":"failed","status":"passed"}', '{"number":NaN}',
                        '{"number":Infinity}', '{"number":-Infinity}', '{"number":1e9999}'):
            path.write_text(content)
            with self.assertRaises(AssertionError):
                gate.strict_json(path)

    def test_workflow_required_modes_and_retention(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        workflow_contract(text)
        check_bytecode_boundary_fixtures(self, text, workflow_contract)
        for value, dont_write in ((None, False), (None, True), ('', True), ('0', True), ('true', True)):
            with self.subTest(inherited=value, dont_write_bytecode=dont_write), mock.patch.dict(os.environ, {}, clear=False), \
                    mock.patch.object(sys, 'dont_write_bytecode', dont_write), \
                    mock.patch.object(gate, 'run') as run, mock.patch.object(gate, 'build_cli') as build:
                if value is None:
                    os.environ.pop('PYTHONDONTWRITEBYTECODE', None)
                else:
                    os.environ['PYTHONDONTWRITEBYTECODE'] = value
                with self.assertRaisesRegex(AssertionError, 'inherited PYTHONDONTWRITEBYTECODE=1 required'):
                    gate.begin()
                run.assert_not_called()
                build.assert_not_called()
                self.assertEqual(os.environ.get('PYTHONDONTWRITEBYTECODE'), value, 'preflight must not self-set')
                self.assertIs(sys.dont_write_bytecode, dont_write)
                recorded = json.loads((gate.runner.evidence / 'gate-receipt.json').read_text())
                self.assertEqual(recorded['python_bytecode_prevention'],
                                 {'PYTHONDONTWRITEBYTECODE': value, 'sys_dont_write_bytecode': dont_write})
                self.assertEqual(recorded['commands'], [])
                self.assertEqual(recorded['producers'], {})
        with mock.patch.dict(os.environ, {'PYTHONDONTWRITEBYTECODE': '1'}), \
                mock.patch.object(sys, 'dont_write_bytecode', True), \
                mock.patch.object(gate, 'run') as run:
            gate.require_inherited_bytecode_prevention()
            run.assert_not_called()
            recorded = json.loads((gate.runner.evidence / 'gate-receipt.json').read_text())
            self.assertEqual(recorded['python_bytecode_prevention'],
                             {'PYTHONDONTWRITEBYTECODE': '1', 'sys_dont_write_bytecode': True})

    def test_workflow_rejects_disabled_moved_or_unretained_modes(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        workflow_contract(text)
        matrix = workflow_job(text, 'static-model-authoring')
        mutations = [
            ('  static-model-authoring:\n', '  static-model-authoring:\n    if: false\n'),
            ('    runs-on: ${{ matrix.os }}\n', '    runs-on: ubuntu-latest\n'),
            ('    timeout-minutes: 90\n', '    continue-on-error: true\n    timeout-minutes: 90\n'),
            ('      PYTHONDONTWRITEBYTECODE: "1"\n', ''),
            ('      PYTHONDONTWRITEBYTECODE: "1"\n', '      PYTHONDONTWRITEBYTECODE: "0"\n'),
            ('      fail-fast: false\n', '      fail-fast: true\n'),
            ('        include:\n', '        exclude:\n'),
            ('            os: windows-latest\n', '            os: ubuntu-22.04\n'),
            ('            mode: gpu\n', '            mode: cpu\n'),
            ('            mode: export\n', '            mode: cpu\n'),
            ('          - feature: pivot\n', '          - feature: uv\n'),
            ("        if: matrix.mode != 'cpu'\n", '        if: false\n'),
            ("        if: matrix.mode == 'export'\n", '        if: false\n'),
            ('mesa-vulkan-drivers', 'unavailable-gpu'),
            ('bubblewrap', 'unavailable-namespace'),
            ('      - name: Required independent static model authoring acceptance\n',
             '      - name: Required independent static model authoring acceptance\n        if: false\n'),
            ('        shell: bash\n', '        shell: pwsh\n'),
            ('        timeout-minutes: ${{ matrix.timeout }}\n', '        timeout-minutes: 1\n'),
            ('          ORR_STATIC_UV_EVIDENCE:', '          OTHER_EVIDENCE:'),
            ('          ORR_STATIC_PIVOT_EVIDENCE:', '          OTHER_EVIDENCE:'),
            ("matrix.mode == 'export' && '/tmp' || runner.temp", 'github.workspace'),
            ('python -B tools/static_model_${{ matrix.feature }}_gate.py ${{ matrix.mode }}', 'echo skipped'),
            ('        if: always()\n', '        if: success()\n'),
            ('actions/upload-artifact@v4', 'actions/upload-artifact@v3'),
            ('static-${{ matrix.feature }}-${{ matrix.mode }}-${{ matrix.os }}', 'shared-evidence'),
            ('          include-hidden-files: true\n', ''),
            ('          if-no-files-found: error\n', '          if-no-files-found: ignore\n'),
            ('          retention-days: 7\n', '          retention-days: 1\n'),
        ]
        for old, new in mutations:
            self.assertIn(old, matrix)
            changed = matrix.replace(old, new, 1)
            self.assertNotEqual(changed, matrix)
            with self.subTest(matrix_mutation=old), self.assertRaises(AssertionError):
                workflow_contract(text.replace(matrix, changed, 1))
        rows = list(re.finditer(r'^          - feature:.*(?:\n            .*?){3}\n', matrix, re.M))
        self.assertEqual(len(rows), 8)
        for row in rows:
            for change in ('', row.group() * 2):
                with self.subTest(row=row.group(), replacement=change), self.assertRaises(AssertionError):
                    workflow_contract(text.replace(matrix, matrix[:row.start()] + change + matrix[row.end():], 1))
        with self.assertRaises(AssertionError):
            workflow_contract(text.replace(matrix, '', 1).replace('  sample-room:\n', '  sample-room:\n' + matrix, 1))
        with self.assertRaises(AssertionError):
            workflow_contract(text.replace('  sample-build:\n', matrix + '  sample-build:\n', 1))
        setup_mutations = {
            'missing': '', 'duplicate': BYTECODE_SETUP * 2,
            'conditional': BYTECODE_SETUP.replace('        shell:', '        if: false\n        shell:'),
            'continue on error': BYTECODE_SETUP.replace('        shell:', '        continue-on-error: true\n        shell:'),
            'wrong shell': BYTECODE_SETUP.replace('shell: bash', 'shell: pwsh'),
            'wrong value': BYTECODE_SETUP.replace('PYTHONDONTWRITEBYTECODE=1', 'PYTHONDONTWRITEBYTECODE=0'),
            'wrong file': BYTECODE_SETUP.replace('$GITHUB_ENV', '$GITHUB_OUTPUT'),
            'export only': BYTECODE_SETUP.split('        run:', 1)[0] + '        run: export PYTHONDONTWRITEBYTECODE=1\n',
            'noop': BYTECODE_SETUP.split('        run:', 1)[0] + '        run: ":"\n',
        }
        for label, setup in setup_mutations.items():
            for owner in ('sample-build', 'sample-arena-cpu'):
                changed = mutate_workflow_job(text, owner, BYTECODE_SETUP, setup)
                with self.subTest(job=owner, setup=label), self.assertRaises(AssertionError):
                    workflow_contract(changed)
        sample = workflow_job(text, 'sample-build')
        for action in ('actions/checkout@v4', 'dtolnay/rust-toolchain@stable', 'Swatinem/rust-cache@v2'):
            anchor = '      - uses: ' + action + '\n'
            moved = sample.replace(BYTECODE_SETUP, '', 1).replace(anchor, anchor + BYTECODE_SETUP, 1)
            with self.subTest(moved_after=action), self.assertRaises(AssertionError):
                workflow_contract(text.replace(sample, moved, 1))
        job_environment = sample.replace('    strategy:\n',
            '    env:\n      PYTHONDONTWRITEBYTECODE: "1"\n    strategy:\n', 1)
        with self.assertRaises(AssertionError):
            workflow_contract(text.replace(sample, job_environment, 1))
        for key in ('PYTHONDONTWRITEBYTECODE', "'PYTHONDONTWRITEBYTECODE'", '"PYTHONDONTWRITEBYTECODE"'):
            for value in ('""', '"0"', '"1"'):
                overridden = text.replace('          ORR_STATIC_GATE_EVIDENCE:',
                    '          ' + key + ': ' + value + '\n          ORR_STATIC_GATE_EVIDENCE:', 1)
                with self.subTest(step_override_key=key, value=value), self.assertRaises(AssertionError):
                    workflow_contract(overridden)

    def test_workflow_preserves_required_room_and_aggregate(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        workflow_contract(text)
        for old, new in [('needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, generated-user-game-native, static-game-actions-native, sample-arena-cpu, sample-fixture-preflight, sample-linked-prefabs, sample-room, static-model-authoring, room-wall-authoring]', 'needs: [native]'),
                         (', generated-user-game-native', ''),
                         ('Required generated Room GPU and source-hidden production export', 'missing Room gate'),
                         ('  sample-room:\n', '  sample-room:\n    # static_model_uv_gate.py forbidden\n'),
                         ('  sample-build:\n', '  sample-build:\n    if: false\n'),
                         ('    name: sample build (${{ matrix.os }})\n', '    name: sample build (${{ matrix.os }})\n    if: false\n'),
                         ('  cross_platform_checksum_compare:\n', '  cross_platform_checksum_compare:\n    if: ${{ always() }}\n')]:
            with self.subTest(old=old), self.assertRaises(AssertionError):
                workflow_contract(text.replace(old, new, 1))


def main():
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(UvGateTests)
    names = sorted(test.id().rsplit('.', 1)[-1] for test in suite)
    assert names == sorted(EXPECTED_TESTS) and len(names) == gate.FIXTURE_COUNT == 25
    result = unittest.TextTestRunner(verbosity=2, failfast=True).run(suite)
    if result.testsRun != 25 or result.skipped or result.expectedFailures or result.unexpectedSuccesses or not result.wasSuccessful():
        raise SystemExit(1)
    print('STATIC_UV_GATE_FIXTURES: 25 passed; 0 failed; 0 skipped', flush=True)


if __name__ == '__main__':
    main()
