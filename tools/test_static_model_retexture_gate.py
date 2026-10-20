#!/usr/bin/env python3
"""Independent texture-variant gate semantics; lifecycle fixtures run separately."""
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

import static_model_retexture_gate as gate
from test_room_project_gate import check_room_workflow
from test_static_model_extract_gate import listing, passed, workflow_job

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXPECTED_TESTS = [
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
    assert 'static_model_retexture_gate.py' not in room
    header = sample.split('    steps:', 1)[0]
    assert 'continue-on-error' not in header
    assert re.findall(r'^    if:[ \t]*(.*)$', header, re.M) == [], 'sample job is unconditional'
    body = sample.split('    steps:\n', 1)[1]
    steps = list(re.finditer(r'^      - ', body, re.M))
    assert len(steps) > 1 and body[:steps[1].start()] == BYTECODE_SETUP, 'exact unconditional first setup step'
    assert text.count(BYTECODE_SETUP) == 1, 'unique bytecode setup'
    inherited_bytecode_contract(sample)
    assert 'os: [ubuntu-22.04, windows-latest]' in header
    assert 'needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, sample-linked-prefabs, sample-room, static-model-authoring, room-wall-authoring]' in aggregate
    assert re.findall(r'^    if:[ \t]*(.*)$', aggregate, re.M) == ['${{ always() }}'], 'one failure-aware aggregate condition'
    prerequisites = {
        'gpu': "        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n",
        'export': "        run: sudo apt-get update -q && sudo apt-get install -y -q bubblewrap\n",
    }
    entries = [
        ('cpu', 'CPU CLI and bindings', 'CPU', None, 45),
        ('gpu', 'actual software GPU', 'GPU', "runner.os == 'Linux'", 45),
        ('export', 'source-hidden Room export', 'Room export', "runner.os == 'Linux' && runner.arch == 'X64'", 60),
    ]
    for mode, title, artifact, condition, timeout in entries:
        marker = '      - name: Required static texture variant ' + title + ' acceptance\n'
        assert text.count(marker) == 1 and marker in sample
        block = sample.split(marker, 1)[1].split('      - name:', 1)[0]
        assert f'        run: python -B tools/static_model_retexture_gate.py {mode}\n' in block
        assert '        shell: bash\n' in block and f'        timeout-minutes: {timeout}\n' in block
        assert 'continue-on-error' not in block and 'ORR_STATIC_RETEXTURE_EVIDENCE:' in block
        conditions = re.findall(r'^        if:[ \t]*(.*)$', block, re.M)
        assert conditions == ([] if condition is None else [condition]), 'exact acceptance condition census'
        if condition is not None:
            assert sample.index(prerequisites[mode]) < sample.index(marker)
        marker = '      - name: Retain static texture variant ' + artifact + ' and partial failure evidence\n'
        assert text.count(marker) == 1 and marker in sample
        block = sample.split(marker, 1)[1].split('      - name:', 1)[0]
        expected_condition = 'always()' + (' && ' + condition if condition else '')
        assert re.findall(r'^        if:[ \t]*(.*)$', block, re.M) == [expected_condition], 'exact retention condition census'
        assert '        uses: actions/upload-artifact@v4\n' in block
        assert '          if-no-files-found: error\n' in block and '          retention-days: 7\n' in block
        if mode == 'export':
            assert '          include-hidden-files: true\n' in block
    for title in ['Required static prop CPU CLI and bindings acceptance',
                  'Required static prop actual software GPU acceptance',
                  'Required static prop source-hidden Room export acceptance',
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
    assert len(names) == len(set(names)) == len(actual) == 29
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
    assert len(common) == 25
    unix = {'cli_symlink_inputs_ancestors_and_destinations_reject', 'cli_non_utf8_option_names_and_logical_ids_reject'}
    assert censuses['linux'] == common | unix | {'cli_non_utf8_filesystem_inputs_work'}
    assert censuses['darwin'] == common | unix
    assert censuses['win32'] == common | {'cli_windows_nonlocal_prefixes_reject'}


class RetextureGateTests(unittest.TestCase):
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
        source = (ROOT / 'crates/orr_model/tests/static_model_retexture.rs').read_text()
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
        for package, target, name in [('orr_model_bindings', 'static_model_retexture', gate.BINDINGS_TEST),
                                      ('orr_render', 'static_model_retexture', gate.GPU_TEST),
                                      ('orr_sample', 'static_model_retexture_export', gate.EXPORT_TEST)]:
            source = (ROOT / f'crates/{package}/tests/{target}.rs').read_text()
            actual = re.findall(r'#\[test\]\s*(?:#\[[^\n]*\]\s*)*fn ([A-Za-z0-9_]+)\(', source)
            self.assertEqual(actual, [name])
            self.assertRegex(source, r'#\[ignore[^\n]*\]')
            self.assertIn('ORR_STATIC_MODEL_RETEXTURE_BIN', source)

    def producer(self):
        crate = self.repo / 'crates/orr_model'
        (crate / 'examples').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname="orr_model"\n')
        source = crate / 'examples/static_model_retexture.rs'; source.write_text('fn main() {}')
        directory = gate.runner.target_dir / 'release/examples'; directory.mkdir(parents=True)
        binary = directory / ('static_model_retexture.exe' if os.name == 'nt' else 'static_model_retexture')
        binary.write_bytes(b'exact compiler artifact'); binary.chmod(0o755)
        gate.runner.report['source_inventory_sha256'] = 'a' * 64
        return {'reason': 'compiler-artifact', 'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_model@0.0.1',
                'target': {'name': 'static_model_retexture', 'src_path': str(source),
                           'kind': ['example'], 'crate_types': ['bin']},
                'profile': {'test': False, 'opt_level': '3'}, 'features': ['import'],
                'executable': str(binary), 'fresh': False}

    def pin(self, producer):
        return gate.pin(json.dumps(producer) + '\n', 'producer', 'orr_model',
                        'static_model_retexture', 'example', ['import'])

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
                     'static_model_retexture', 'example', ['import'])

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

    def variant_fixture(self, directory=None, stem='magenta', source_name='source.model.json'):
        root = directory or (self.root / 'variant'); root.mkdir(exist_ok=True)
        material = {'base_color': [1, 1, 1, 1], 'image': 0, 'linear_filter': False, 'wrap_s': 'repeat', 'wrap_t': 'clamp'}
        primitive = {'id': 'models/source.glb#node=1/mesh=0/primitive=0', 'vertices': [],
                     'indices': [0, 1, 2], 'material': 0, 'transform': [[1, 0, 0, 0]] * 4}
        source = {'format': 'orr_static_model', 'version': 1, 'asset_id': 'models/source.glb',
                  'dependencies': [{'uri': '$source', 'sha256': 'f' * 64}],
                  'primitives': [primitive, dict(primitive, id='models/source.glb#node=2/mesh=0/primitive=0'),
                                 dict(primitive, id='models/source.glb#node=3/mesh=0/primitive=1', material=1)],
                  'materials': [material, dict(material, image=1)],
                  'images': [{'width': 1, 'height': 1, 'rgba8': [255, 0, 0, 255]},
                             {'width': 1, 'height': 1, 'rgba8': [0, 255, 0, 255]}]}
        (root / source_name).write_text(json.dumps(source))
        recipe_file = stem + '.retexture.json'; texture_file = stem + '.png'; cooked_file = stem + '.model.json'
        recipe = {'format': 'orr_static_material_texture', 'version': 1,
                  'source': 'inputs/source.model.json', 'texture': 'images/' + texture_file, 'material': 0}
        (root / recipe_file).write_text(json.dumps(recipe)); (root / texture_file).write_bytes(b'fixture PNG ' + stem.encode())
        asset_id = 'variants/' + recipe_file
        cooked = copy.deepcopy(source); cooked['asset_id'] = asset_id
        for primitive in cooked['primitives']:
            primitive['id'] = asset_id + primitive['id'][len(source['asset_id']):]
        cooked['images'].append({'width': 1, 'height': 1, 'rgba8': [255, 0, 255, 0]})
        cooked['materials'][0]['image'] = 2
        cooked['dependencies'] = [{'uri': uri, 'sha256': gate.runner.digest(root / file)} for uri, file in
                                  [('$source', recipe_file), (recipe['source'], source_name), (recipe['texture'], texture_file)]]
        (root / cooked_file).write_text(json.dumps(cooked))
        return root, (source_name, recipe_file, texture_file, cooked_file, asset_id), cooked

    def test_variant_keeps_source_geometry_and_slots(self):
        root, args, cooked = self.variant_fixture()
        self.assertEqual(gate.check_variant(root, *args), cooked)

    def test_variant_rejects_geometry_material_and_dependency_mutations(self):
        root, args, original = self.variant_fixture()
        for field, value in [('asset_id', 'different.retexture.json'), ('dependencies', list(reversed(original['dependencies']))),
                             ('images', original['images'][1:]), ('primitives', original['primitives'][:1]),
                             ('materials', [dict(original['materials'][0], base_color=[0, 0, 0, 0]), original['materials'][1]])]:
            cooked = dict(original); cooked[field] = value
            (root / args[3]).write_text(json.dumps(cooked))
            with self.subTest(field=field), self.assertRaises(AssertionError):
                gate.check_variant(root, *args)

    def test_gpu_capture_inventory_and_pixels(self):
        root, _ = self.gpu_fixture()
        gate.check_gpu(root)

    def test_gpu_rejects_receipt_pixel_and_provenance_mutations(self):
        root, receipt = self.gpu_fixture()
        cases = []
        changed = copy.deepcopy(receipt); changed['shared_material_users_checked'] = False; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['control_pixel_differences']['asymmetric_uv'] = 0; cases.append(changed)
        changed = copy.deepcopy(receipt); changed['assets']['variants'][0]['cooked']['fnv1a64'] = '0' * 16; cases.append(changed)
        for changed in cases:
            (root / 'receipt.json').write_text(json.dumps(changed))
            with self.assertRaises(AssertionError):
                gate.check_gpu(root)
        # Coherently rehash all affected captures and recompute reported control
        # counts: an actually changed marker must fail semantic pixel checks,
        # rather than just a stale hash or stale receipt count.
        changed = copy.deepcopy(receipt)
        path = root / 'magenta.rgba'; rgba = bytearray(path.read_bytes())
        offset = gate.marker_offsets()[0]; rgba[offset:offset + 4] = b'\x40\x40\x40\xff'
        path.write_bytes(rgba)
        (root / 'magenta.ppm').write_bytes(b'P6\n256 256\n255\n' + bytes(value for index, value in enumerate(rgba) if index % 4 != 3))
        for entry in changed['frames']['magenta'].values():
            data = (root / entry['file']).read_bytes(); value = 0xcbf29ce484222325
            for byte in data:
                value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
            entry.update(bytes=len(data), fnv1a64=f'{value:016x}')
        for name, (first, second) in changed['control_pairs'].items():
            changed['control_pixel_differences'][name] = gate.difference(
                (root / changed['frames'][first]['raw']['file']).read_bytes(),
                (root / changed['frames'][second]['raw']['file']).read_bytes())
        (root / 'receipt.json').write_text(json.dumps(changed))
        with self.assertRaises(AssertionError):
            gate.check_gpu(root)
        (root / 'receipt.json').write_text(json.dumps(receipt))
        (root / 'magenta.rgba').write_bytes(b'wrong dimensions')
        with self.assertRaises(AssertionError):
            gate.check_gpu(root)

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
        (root / 'prepared-project-initial-export.frame').write_bytes(b'changed')
        self.export_inventory(root)
        with self.assertRaises(AssertionError):
            gate.check_export(root)

    def gpu_fixture(self):
        root = self.root / 'gpu'; root.mkdir()
        def reference(name):
            data = (root / name).read_bytes(); value = 0xcbf29ce484222325
            for byte in data:
                value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
            return {'file': name, 'bytes': len(data), 'fnv1a64': f'{value:016x}'}
        def save(name, data):
            (root / name).write_bytes(data)
            return reference(name)
        assets = {'original_glb': save('source.glb', b'fixture GLB'), 'variants': [],
                  'source_hidden_after_install': True, 'cooked_reload_before_render': True, 'installed_files': 2}
        for color, rgba in [('magenta', [255, 0, 255, 0]), ('cyan', [0, 255, 255, 64])]:
            _, args, cooked = self.variant_fixture(root, stem=color)
            cooked['images'][-1] = {'width': 2, 'height': 2, 'rgba8': rgba * 4}
            (root / args[3]).write_text(json.dumps(cooked))
            assets['variants'].append({'asset_id': args[4], 'texture': reference(args[2]),
                                       'recipe': reference(args[1]), 'cooked': reference(args[3])})
        assets['source'] = reference('source.model.json')
        values = {'source': 0, 'magenta': 10, 'cyan': 20, 'variants-together': 30, 'variants-reversed': 30,
                  'wrong-transform': 40, 'wrong-uv': 50, 'wrong-material': 60,
                  'only-first-user': 70, 'changed-unselected-material': 80}
        frames = {}; raw_frames = {}
        markers = gate.marker_offsets()
        for name, value in values.items():
            rgba = bytearray(b'\0\0\0\xff' * (256 * 256))
            for y in range(64, 192):
                for x in range(64, 192):
                    rgba[(y * 256 + x) * 4:(y * 256 + x + 1) * 4] = bytes([value, value, value, 255])
            if name in {'source', 'magenta', 'cyan'}:
                for offset in markers:
                    rgba[offset:offset + 4] = bytes([188, 255, 0, 255])
            for frame, center, color in [('magenta', 0, [255, 0, 255, 255]), ('cyan', 0, [0, 255, 255, 255]),
                                         ('variants-together', -64, [255, 0, 255, 255]),
                                         ('variants-together', 64, [0, 255, 255, 255]),
                                         ('variants-reversed', -64, [255, 0, 255, 255]),
                                         ('variants-reversed', 64, [0, 255, 255, 255])]:
                if name == frame:
                    for x, y in ((140, 88), (116, 168)):
                        offset = (y * 256 + x + center) * 4
                        rgba[offset:offset + 4] = bytes(color)
            raw_frames[name.replace('-', '_')] = bytes(rgba)
            frames[name.replace('-', '_')] = {'raw': save(name + '.rgba', rgba),
                'image': save(name + '.ppm', b'P6\n256 256\n255\n' + bytes(value for index, value in enumerate(rgba) if index % 4 != 3))}
        pairs = {'source_to_magenta': ['source', 'magenta'], 'source_to_cyan': ['source', 'cyan'],
                 'distinct_variants': ['magenta', 'cyan'], 'world_transform': ['wrong_transform', 'magenta'],
                 'asymmetric_uv': ['wrong_uv', 'source'], 'material_slot': ['wrong_material', 'magenta'],
                 'shared_material_user': ['only_first_user', 'magenta'],
                 'unselected_material_mutation': ['changed_unselected_material', 'magenta']}
        coverage = {}
        for name in ('source', 'magenta', 'cyan', 'variants_together'):
            count = 2 if name == 'variants_together' else 1
            coverage[name] = {'selected_pixels_per_model_node': [[501, 502]] * count,
                              'unaffected_pixels_per_model_node': [[41, 42]] * count,
                              'source_checker_pixels': [101, 102, 103, 104], 'clear_checked': 40001}
        adapter = {'software': True, 'backend': 'Vulkan'}
        (root / 'adapter.json').write_text(json.dumps(adapter))
        accepted = {'schema': 1, 'status': 'passed', 'width': 256, 'height': 256, 'adapter': adapter,
                    'assets': assets, 'frames': frames, 'coverage': coverage, 'control_pairs': pairs,
                    'control_pixel_differences': {name: gate.difference(raw_frames[first], raw_frames[second])
                                                 for name, (first, second) in pairs.items()},
                    'shared_material_users_checked': True, 'unaffected_material_byte_identical': True,
                    'distinct_recipe_namespaces': True, 'reversed_batch_order_byte_identical': True,
                    'opaque_alpha_checked': True, 'unaffected_material_pixels_checked': len(markers),
                    'tolerances': {'color_bytes': 3, 'minimum_control_differences': 500,
                                   'minimum_unaffected_differences': 100}}
        (root / 'receipt.json').write_text(json.dumps(accepted))
        return root, accepted

    def export_inventory(self, root):
        files = {path.relative_to(root).as_posix(): [path.stat().st_size, gate.runner.digest(path)]
                 for path in root.rglob('*') if path.is_file() and path.name != 'capture-inventory.json'}
        (root / 'capture-inventory.json').write_text(json.dumps(files))

    def export_fixture(self):
        root = self.root / 'export'; root.mkdir()
        for path in ('crates/orr_sample/tests/static_model_retexture_export.rs',
                     'crates/orr_model/examples/static_model_retexture.rs',
                     'crates/orr_model/examples/static_model_retexture/authoring.rs'):
            file = self.repo / path; file.parent.mkdir(parents=True, exist_ok=True); file.write_text('current source ' + path)
        variants = []
        for color, rgba in [('red', [255, 32, 32, 0]), ('blue', [32, 64, 255, 0])]:
            _, args, cooked = self.variant_fixture(root, stem=color, source_name='authoring-source.model.json')
            cooked['images'][-1] = {'width': 2, 'height': 2, 'rgba8': rgba * 4}
            (root / args[3]).write_text(json.dumps(cooked))
            variants.append({'name': color, 'asset_id': args[4], 'material': 0, 'rgba': rgba,
                             'recipe_sha256': gate.runner.digest(root / args[1]),
                             'png_sha256': gate.runner.digest(root / args[2]),
                             'cooked_sha256': gate.runner.digest(root / args[3]), 'cli_stdout': 'published'})
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
        accepted = {'source_hidden': True, 'read_only_bundle': True, 'visible_changed_pixels': 1024 * 768,
                    'retexture_changed_pixels': 1024 * 768, 'observations': observations,
                    'control_stdout': [observations['initial']['source_stdout']] * 2}
        for field, label in [('runtime_sha256', 'room_escape'), ('exporter_sha256', 'orr_export_room'),
                             ('retexture_cli_sha256', 'retexture-cli'), ('acceptance_executable_sha256', 'export-test')]:
            value = hashlib.sha256(label.encode()).hexdigest()
            accepted[field] = value; gate.runner.report['producers'][label] = {'sha256': value}
        package = 'b' * 64
        project = root / 'authored-project'; project.mkdir()
        object_path = '.orr/packages/objects/' + package + '/'
        files = {}
        for name in ['orr.project.json', 'room.scene.yaml', 'room.models.json', 'room.camera.json',
                     'orr.packages.lock.json', object_path + 'orr.package.json',
                     object_path + 'red.model.json', object_path + 'blue.model.json']:
            file = project / name; file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes((root / pathlib.Path(name).name).read_bytes() if name.endswith('.model.json') else b'{}')
            files['project/' + name] = [file.stat().st_size, gate.runner.digest(file)]
        files['bin/room_escape'] = [10, accepted['runtime_sha256']]
        files['run-room-escape'] = [4, hashlib.sha256(b'run!').hexdigest()]
        manifest = {'schema': 1, 'payload': {'profile': 'room-escape-authored-linux-x86_64-v1',
                    'runtime': {'sha256': accepted['runtime_sha256']}, 'packages': {'retextured-static-props': package},
                    'files': [{'path': name, 'bytes': value[0], 'sha256': value[1]} for name, value in files.items()]}}
        (root / 'orr.export.json').write_text(json.dumps(manifest))
        files['orr.export.json'] = [(root / 'orr.export.json').stat().st_size, gate.runner.digest(root / 'orr.export.json')]
        accepted['exported_file_inventory'] = files
        source = {'test': gate.EXPORT_TEST, 'source_asset_id': 'models/scene.glb', 'source_path': 'inputs/source.model.json',
                  'glb_sha256': gate.runner.digest(root / 'scene.glb'),
                  'source_cooked_sha256': gate.runner.digest(root / 'authoring-source.model.json'),
                  'variants': variants, 'original_source_exported': False,
                  'initial_frame_sha256': gate.runner.digest(root / 'prepared-project-initial-source.frame')}
        for field, name in [('test_source_sha256', 'crates/orr_sample/tests/static_model_retexture_export.rs'),
                            ('cli_source_sha256', 'crates/orr_model/examples/static_model_retexture.rs'),
                            ('authoring_source_sha256', 'crates/orr_model/examples/static_model_retexture/authoring.rs')]:
            source[field] = gate.runner.digest(self.repo / name)
        (root / 'acceptance-source.json').write_text(json.dumps(source))
        (root / 'production-receipt.json').write_text(json.dumps(accepted))
        self.export_inventory(root)
        return root, accepted

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
        for old, new in [('run: python -B tools/static_model_retexture_gate.py cpu', 'run: echo skipped'),
                         ('run: python -B tools/static_model_retexture_gate.py gpu', 'run: echo skipped'),
                         ('run: python -B tools/static_model_retexture_gate.py export', 'run: echo skipped'),
                         ('Retain static texture variant CPU and partial failure evidence', 'missing retention'),
                         ('os: [ubuntu-22.04, windows-latest]', 'os: [ubuntu-22.04]')]:
            with self.subTest(old=old), self.assertRaises(AssertionError):
                workflow_contract(text.replace(old, new, 1))
        setup_mutations = {
            'missing': '',
            'duplicate': BYTECODE_SETUP * 2,
            'conditional': BYTECODE_SETUP.replace('        shell:', '        if: false\n        shell:'),
            'continue on error': BYTECODE_SETUP.replace('        shell:', '        continue-on-error: true\n        shell:'),
            'wrong shell': BYTECODE_SETUP.replace('shell: bash', 'shell: pwsh'),
            'wrong value': BYTECODE_SETUP.replace('PYTHONDONTWRITEBYTECODE=1', 'PYTHONDONTWRITEBYTECODE=0'),
            'wrong file': BYTECODE_SETUP.replace('$GITHUB_ENV', '$GITHUB_OUTPUT'),
            'export only': BYTECODE_SETUP.split('        run:', 1)[0] + '        run: export PYTHONDONTWRITEBYTECODE=1\n',
            'noop': BYTECODE_SETUP.split('        run:', 1)[0] + '        run: ":"\n',
        }
        for label, setup in setup_mutations.items():
            with self.subTest(setup=label), self.assertRaises(AssertionError):
                workflow_contract(text.replace(BYTECODE_SETUP, setup, 1))
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
        old = '      - name: Required static texture variant CPU CLI and bindings acceptance\n'
        with self.assertRaises(AssertionError):
            workflow_contract(text.replace(old, old + '        if: false\n'))
        for key in ('PYTHONDONTWRITEBYTECODE', "'PYTHONDONTWRITEBYTECODE'", '"PYTHONDONTWRITEBYTECODE"'):
            for value in ('""', '"0"', '"1"'):
                overridden = text.replace('          ORR_STATIC_GATE_EVIDENCE:',
                    '          ' + key + ': ' + value + '\n          ORR_STATIC_GATE_EVIDENCE:', 1)
                with self.subTest(step_override_key=key, value=value), self.assertRaises(AssertionError):
                    workflow_contract(overridden)
        for title, condition, middle in [
            ('Required static texture variant CPU CLI and bindings acceptance', None, '        shell: bash\n'),
            ('Required static texture variant actual software GPU acceptance', "runner.os == 'Linux'", '        shell: bash\n'),
            ('Required static texture variant source-hidden Room export acceptance', "runner.os == 'Linux' && runner.arch == 'X64'", '        shell: bash\n'),
            ('Retain static texture variant CPU and partial failure evidence', 'always()', '        uses: actions/upload-artifact@v4\n'),
            ('Retain static texture variant GPU and partial failure evidence', "always() && runner.os == 'Linux'", '        uses: actions/upload-artifact@v4\n'),
            ('Retain static texture variant Room export and partial failure evidence', "always() && runner.os == 'Linux' && runner.arch == 'X64'", '        uses: actions/upload-artifact@v4\n'),
        ]:
            marker = '      - name: ' + title + '\n'
            block = text.split(marker, 1)[1].split('      - name:', 1)[0]
            mutations = {
                'first conditional': '        if: false\n' + block,
                'middle conditional': block.replace(middle, middle + '        if: false\n', 1),
                'duplicate conditional': '        if: ' + (condition or 'false') + '\n' + block,
            }
            if condition is None:
                mutations['duplicate conditional'] = '        if: false\n        if: false\n' + block
            else:
                line = '        if: ' + condition + '\n'
                mutations['missing conditional'] = block.replace(line, '', 1)
                mutations['wrong conditional'] = block.replace(line, '        if: false\n', 1)
                mutations['middle duplicate conditional'] = block.replace(middle, middle + line, 1)
            for label, changed in mutations.items():
                with self.subTest(title=title, mutation=label), self.assertRaises(AssertionError):
                    workflow_contract(text.replace(marker + block, marker + changed, 1))
        start = text.index(old)
        end = text.index('      - name: Required animation clip extraction cpu acceptance\n', start)
        pair = text[start:end]
        relocated = (text[:start] + text[end:]).replace('  sample-room:\n', '  sample-room:\n' + pair, 1)
        with self.assertRaises(AssertionError):
            workflow_contract(relocated)

    def test_workflow_preserves_required_room_and_aggregate(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        for old, new in [('needs: [native, wasm32, wasm32-browser, android-arm64, sample-build, sample-linked-prefabs, sample-room, static-model-authoring, room-wall-authoring]', 'needs: [native]'),
                         ('Required generated Room GPU and source-hidden production export', 'missing Room gate'),
                         ('  sample-room:\n', '  sample-room:\n    # static_model_retexture_gate.py forbidden\n'),
                         ('  sample-build:\n', '  sample-build:\n    if: false\n'),
                         ('    name: sample build (${{ matrix.os }})\n', '    name: sample build (${{ matrix.os }})\n    if: false\n'),
                         ('  cross_platform_checksum_compare:\n', '  cross_platform_checksum_compare:\n    if: ${{ always() }}\n')]:
            with self.subTest(old=old), self.assertRaises(AssertionError):
                workflow_contract(text.replace(old, new, 1))


def main():
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RetextureGateTests)
    names = sorted(test.id().rsplit('.', 1)[-1] for test in suite)
    assert names == sorted(EXPECTED_TESTS) and len(names) == gate.FIXTURE_COUNT == 20
    result = unittest.TextTestRunner(verbosity=2, failfast=True).run(suite)
    if result.testsRun != 20 or result.skipped or result.expectedFailures or result.unexpectedSuccesses or not result.wasSuccessful():
        raise SystemExit(1)
    print('STATIC_RETEXTURE_GATE_FIXTURES: 20 passed; 0 failed; 0 skipped', flush=True)


if __name__ == '__main__':
    main()
