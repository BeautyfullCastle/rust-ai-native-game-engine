#!/usr/bin/env python3
"""Independent pivot gate semantic fixtures. Synthetic captures are checker tests,
never device, Rust, workflow-integration, or production acceptance evidence.
The complete suite requires the real current-source parallel authoring matrix
and the inherited UV, texture, Room and aggregate contracts.
"""
import copy
import hashlib
import json
import os
import pathlib
import re
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zlib

if not __debug__ or sys.flags.optimize or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
    raise SystemExit('static pivot fixtures require nonoptimized Python')

import static_model_pivot_gate as gate
from test_static_model_extract_gate import listing, passed, workflow_job
from test_static_model_retexture_gate import workflow_contract as published_workflow_contract
from test_static_model_uv_gate import workflow_contract as published_uv_workflow_contract

ROOT = pathlib.Path(__file__).resolve().parents[1]
CONDITIONAL = {
    'cli_symlink_inputs_ancestors_and_destinations_reject':'#[cfg(unix)]',
    'cli_non_utf8_option_names_and_logical_ids_reject':'#[cfg(unix)]',
    'cli_unix_socket_inputs_reject_without_opening_or_publishing':'#[cfg(unix)]',
    'cli_non_utf8_filesystem_inputs_work':'#[cfg(target_os = "linux")]',
    'cli_windows_nonlocal_prefixes_reject':'#[cfg(windows)]',
}


def model_census_contract(source):
    declarations = re.findall(r'((?:^#\[[^\n]+\]\n)+)^fn ([A-Za-z0-9_]+)\(',source,re.M)
    actual = {name:attributes.splitlines() for attributes,name in declarations if '#[test]' in attributes.splitlines()}
    names = re.findall(r'^#\[test\]\s+fn ([A-Za-z0-9_]+)\(',source,re.M)
    assert len(names) == len(set(names)) == len(actual) == 26
    censuses = {platform:set(gate.model_tests(platform)) for platform in ('linux','darwin','win32')}
    assert set(actual) == set.union(*censuses.values())
    for name,attrs in actual.items():
        assert attrs == ([CONDITIONAL[name],'#[test]'] if name in CONDITIONAL else ['#[test]']),name
    common = set(actual)-set(CONDITIONAL)
    assert len(common) == 21
    unix = {name for name,attr in CONDITIONAL.items() if attr == '#[cfg(unix)]'}
    assert censuses['linux'] == common|unix|{'cli_non_utf8_filesystem_inputs_work'}
    assert censuses['darwin'] == common|unix
    assert censuses['win32'] == common|{'cli_windows_nonlocal_prefixes_reject'}


def protected_workflow_contract(text):
    published_workflow_contract(text)
    published_uv_workflow_contract(text)
    assert 'static_model_pivot_gate.py' not in workflow_job(text, 'sample-room')
    assert 'static_model_pivot_gate.py' not in workflow_job(text, 'sample-build')


def workflow_lines(text):
    """Ignore only empty lines and comments, never executable YAML or shell."""
    return [line for line in text.splitlines() if line.strip() and not line.lstrip().startswith('#')]


def matrix_row(feature, mode, os_name, timeout):
    return (f'          - feature: {feature}\n            mode: {mode}\n'
            f'            os: {os_name}\n            timeout: {timeout}\n')


def retention_binding_contract(acceptance, retention):
    """Exact generic matrix data binding, including duplicate quoted-key rejection."""
    for feature in ('UV', 'PIVOT'):
        key = 'ORR_STATIC_' + feature + '_EVIDENCE'
        assert acceptance.count(key) == 1, 'one inherited feature evidence destination'
        assert re.findall(r'^          '+key+r':[ \t]*(.*)$', acceptance, re.M) == [gate.WORKFLOW_EVIDENCE]
    for key in ('path', 'name'):
        forms = re.escape(key)+"|'"+re.escape(key)+"'|\""+re.escape(key)+"\""
        assert len(re.findall(r'^          (?:'+forms+r')[ \t]*:', retention, re.M)) == 1, 'one upload field key'
    assert re.findall(r'^          path:[ \t]*(.*)$', retention, re.M) == [gate.WORKFLOW_EVIDENCE]
    assert re.findall(r'^          name:[ \t]*(.*)$', retention, re.M) == [gate.WORKFLOW_ARTIFACT]


def workflow_contract(text):
    """Validate the actual file; negative mutations are checker controls only."""
    protected_workflow_contract(text)
    authoring = workflow_job(text, gate.WORKFLOW_JOB)
    header = '  static-model-authoring:\n    name: static model authoring (${{ matrix.feature }}, ${{ matrix.mode }}, ${{ matrix.os }})\n    runs-on: ${{ matrix.os }}\n    timeout-minutes: 90\n    env:\n      PYTHONDONTWRITEBYTECODE: "1"\n    strategy:\n      fail-fast: false\n      matrix:\n        include:\n'
    expected_rows = (
        ('uv', 'cpu', 'ubuntu-22.04', 45), ('uv', 'cpu', 'windows-latest', 45),
        ('uv', 'gpu', 'ubuntu-22.04', 45), ('uv', 'export', 'ubuntu-22.04', 60),
        ('pivot', 'cpu', 'ubuntu-22.04', 45), ('pivot', 'cpu', 'windows-latest', 45),
        ('pivot', 'gpu', 'ubuntu-22.04', 45), ('pivot', 'export', 'ubuntu-22.04', 60),
    )
    assert gate.WORKFLOW_ROWS == expected_rows and len(set(expected_rows)) == 8
    assert gate.WORKFLOW_JOB == 'static-model-authoring'
    assert gate.WORKFLOW_STEP == 'Required independent static model authoring acceptance'
    assert gate.WORKFLOW_RETENTION == 'Retain static authoring and partial failure evidence'
    assert gate.WORKFLOW_EVIDENCE == "${{ matrix.mode == 'export' && '/tmp' || runner.temp }}/orr-static-${{ matrix.feature }}-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}"
    assert gate.WORKFLOW_ARTIFACT == 'static-${{ matrix.feature }}-${{ matrix.mode }}-${{ matrix.os }}'
    header += ''.join(matrix_row(*row) for row in expected_rows)
    steps = "    steps:\n      - uses: actions/checkout@v4\n      - uses: dtolnay/rust-toolchain@stable\n      - uses: Swatinem/rust-cache@v2\n      - name: Install static authoring software Vulkan dependency\n        if: matrix.mode != 'cpu'\n        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n      - name: Install static authoring namespace acceptance dependency\n        if: matrix.mode == 'export'\n        run: sudo apt-get update -q && sudo apt-get install -y -q bubblewrap\n      - name: Required independent static model authoring acceptance\n        shell: bash\n        timeout-minutes: ${{ matrix.timeout }}\n        env:\n          ORR_STATIC_UV_EVIDENCE: @EVIDENCE@\n          ORR_STATIC_PIVOT_EVIDENCE: @EVIDENCE@\n        run: python -B tools/static_model_${{ matrix.feature }}_gate.py ${{ matrix.mode }}\n      - name: Retain static authoring and partial failure evidence\n        if: always()\n        uses: actions/upload-artifact@v4\n        with:\n          name: @ARTIFACT@\n          path: @EVIDENCE@\n          include-hidden-files: true\n          if-no-files-found: error\n          retention-days: 7\n"
    steps = steps.replace('@EVIDENCE@', gate.WORKFLOW_EVIDENCE).replace('@ARTIFACT@', gate.WORKFLOW_ARTIFACT)
    # An exact executable contract rejects extra keys/rows, inherited overrides,
    # job/step conditions, allowed failures, command bypass and missing retention.
    assert workflow_lines(authoring) == workflow_lines(header + steps), 'exact eight-row authoring matrix contract'
    acceptance_marker = '      - name: '+gate.WORKFLOW_STEP+'\n'
    retention_marker = '      - name: '+gate.WORKFLOW_RETENTION+'\n'
    assert text.count(acceptance_marker) == text.count(retention_marker) == 1
    acceptance = authoring.split(acceptance_marker, 1)[1].split('      - name:', 1)[0]
    retention = authoring.split(retention_marker, 1)[1]
    retention_binding_contract(acceptance, retention)
    aggregate = workflow_job(text, 'cross_platform_checksum_compare')
    aggregate_header = aggregate.split('    steps:\n', 1)[0]
    expected_needs = '[native, wasm32, wasm32-browser, android-arm64, sample-build, generated-user-game-native, static-game-actions-native, sample-arena-cpu, sample-fixture-preflight, sample-linked-prefabs, sample-room, static-model-authoring, room-wall-authoring]'
    assert re.findall(r'^    needs:[ \t]*(.*)$', aggregate_header, re.M) == [expected_needs]
    assert re.findall(r'^    if:[ \t]*(.*)$', aggregate_header, re.M) == ['${{ always() }}']
    assert 'continue-on-error' not in aggregate_header
    authoring_result = '      - name: Require every independent static authoring matrix row to pass\n        shell: bash\n        env:\n          AUTHORING_RESULT: ${{ needs[\'static-model-authoring\'].result }}\n        run: |\n          echo "AUTHORING_RESULT=$AUTHORING_RESULT"\n          if [[ "$AUTHORING_RESULT" != "success" ]]; then\n            echo "::error::AUTHORING_RESULT must be success, got \'$AUTHORING_RESULT\'"\n            exit 1\n          fi\n          echo "All eight independent static authoring routes passed."\n'
    result_marker = authoring_result.split('\n', 1)[0]+'\n'
    assert text.count(result_marker) == 1
    assert re.findall(r'^      - name: (.*)$', aggregate, re.M) == [
        'Require every checksum and sample runner to pass',
        'Require every independent static authoring matrix row to pass',
        'Require every independent Room wall authoring matrix row to pass',
    ]
    authoring_tail = aggregate.split(result_marker, 1)[1].split('      - name:', 1)[0]
    assert workflow_lines(result_marker + authoring_tail) == workflow_lines(authoring_result), 'required authoring aggregate result gate'


class PivotGateTests(unittest.TestCase):
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
        self.assertEqual(len(gate.FEATURE_PATHS), 9)
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
        source = (ROOT/'crates/orr_model/tests/static_model_pivot.rs').read_text()
        model_census_contract(source)
        for platform in (None,True,False,'','windows','macos','freebsd','linux2'):
            with self.subTest(platform=platform), self.assertRaises(AssertionError): gate.model_tests(platform)
        for name,attr in CONDITIONAL.items():
            original = attr+'\n#[test]\nfn '+name+'('
            self.assertIn(original,source)
            for changed in ('#[test]\nfn '+name+'(', '#[cfg(any())]\n#[test]\nfn '+name+'('):
                with self.subTest(name=name), self.assertRaises(AssertionError): model_census_contract(source.replace(original,changed,1))
        original = '#[test]\nfn zero_pivots_preserve_positive_and_negative_zero_translation_bits('
        with self.assertRaises(AssertionError): model_census_contract(source.replace(original,'#[test]\n#[ignore]\nfn zero_pivots_preserve_positive_and_negative_zero_translation_bits(',1))

    def test_actual_consumer_census_matches_current_source(self):
        for package, target, name in [('orr_model_bindings', 'static_model_pivot', gate.BINDINGS_TEST),
                                      ('orr_render', 'static_model_pivot', gate.GPU_TEST),
                                      ('orr_sample', 'static_model_pivot_export', gate.EXPORT_TEST)]:
            source = (ROOT / f'crates/{package}/tests/{target}.rs').read_text()
            actual = re.findall(r'#\[test\]\s*(?:#\[[^\n]*\]\s*)*fn ([A-Za-z0-9_]+)\(', source)
            self.assertEqual(actual, [name])
            self.assertRegex(source, r'#\[ignore[^\n]*\]')
            self.assertIn('ORR_STATIC_MODEL_PIVOT_BIN', source)

    def producer(self):
        crate = self.repo / 'crates/orr_model'
        (crate / 'examples').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname="orr_model"\n')
        source = crate / 'examples/static_model_pivot.rs'; source.write_text('fn main() {}')
        directory = gate.runner.target_dir / 'release/examples'; directory.mkdir(parents=True)
        binary = directory / ('static_model_pivot.exe' if os.name == 'nt' else 'static_model_pivot')
        binary.write_bytes(b'exact compiler artifact'); binary.chmod(0o755)
        gate.runner.report['source_inventory_sha256'] = 'a' * 64
        return {'reason': 'compiler-artifact', 'manifest_path': str(crate / 'Cargo.toml'),
                'package_id': 'path+' + crate.as_uri() + '#orr_model@0.0.1',
                'target': {'name': 'static_model_pivot', 'src_path': str(source),
                           'kind': ['example'], 'crate_types': ['bin']},
                'profile': {'test': False, 'opt_level': '3'}, 'features': ['import'],
                'executable': str(binary), 'fresh': False}

    def pin(self, producer):
        return gate.pin(json.dumps(producer) + '\n', 'producer', 'orr_model',
                        'static_model_pivot', 'example', ['import'])

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
                     'static_model_pivot', 'example', ['import'])

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


    def test_inherited_bytecode_prevention_is_not_self_set(self):
        for value,dont_write in ((None,False),(None,True),('',True),('0',True),('true',True)):
            with self.subTest(value=value), mock.patch.dict(os.environ,{},clear=False), mock.patch.object(sys,'dont_write_bytecode',dont_write), mock.patch.object(gate,'run') as run:
                if value is None: os.environ.pop('PYTHONDONTWRITEBYTECODE',None)
                else: os.environ['PYTHONDONTWRITEBYTECODE']=value
                with self.assertRaisesRegex(AssertionError,'inherited PYTHONDONTWRITEBYTECODE=1 required'): gate.begin()
                run.assert_not_called()
                self.assertEqual(os.environ.get('PYTHONDONTWRITEBYTECODE'),value)
                self.assertIs(sys.dont_write_bytecode,dont_write)
                saved=json.loads((gate.runner.evidence/'gate-receipt.json').read_text())
                self.assertEqual(saved['python_bytecode_prevention'],{'PYTHONDONTWRITEBYTECODE':value,'sys_dont_write_bytecode':dont_write})
        with mock.patch.dict(os.environ,{'PYTHONDONTWRITEBYTECODE':'1'}): gate.require_inherited_bytecode_prevention()

    def test_optimized_python_rejects_before_producers_or_artifacts(self):
        for name in ('static_model_pivot_gate.py','test_static_model_pivot_gate.py'):
            for flags,optimization in [(['-O','-B'],'0'),(['-E','-B'],'1')]:
                out=subprocess.run([sys.executable,*flags,str(ROOT/'tools'/name),'cpu'],cwd=self.root,
                    env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',PYTHONOPTIMIZE=optimization),capture_output=True,timeout=10,check=False)
                self.assertNotEqual(out.returncode,0)
                self.assertIn(b'nonoptimized Python',out.stderr)
        self.assertEqual(sorted(p.name for p in self.root.iterdir()),['checkout','evidence'])

    def variant_fixture(self,directory=None,stem='offset-a',source_name='source.model.json'):
        root=directory or self.root/'variant'; root.mkdir(exist_ok=True)
        material={'base_color':[1,1,1,0.5],'image':0,'linear_filter':False,'wrap_s':'mirror','wrap_t':'repeat'}
        matrix=[[-0.5,0.0,-0.0,0.0],[0.0,0.75,0.0,0.0],[0.0,0.0,1.0,0.0],[0.0,1.25,-0.0,1.0]]
        primitive={'id':'models/scene.glb#node=1/mesh=0/primitive=0','vertices':[{'position':[0,1,0],'normal':[0,0,1],'uv':[0.25,0.75]}],
            'indices':[0,0,0],'material':0,'transform':matrix}
        source={'format':'orr_static_model','version':1,'asset_id':'models/scene.glb',
            'dependencies':[{'uri':'$source','sha256':'f'*64}],
            'primitives':[dict(copy.deepcopy(primitive),id=f'models/scene.glb#node={1+i//2}/mesh=0/primitive={i%2}',material=i%2) for i in range(4)],
            'materials':[material,dict(material,image=1)],'images':[{'width':1,'height':1,'rgba8':[255,0,0,255]},{'width':1,'height':1,'rgba8':[0,255,0,255]}]}
        (root/source_name).write_text(json.dumps(source))
        pivots={'offset-a':gate.PIVOTS[0],'offset-b':gate.PIVOTS[1],'red':(1.0,-0.5,0.25),'blue':(-1.0,0.75,0.0)}
        recipe_file=stem+'.pivot.json'; cooked_file=stem+'.model.json'
        recipe={'format':'orr_static_model_pivot','version':1,'source':'inputs/source.model.json','pivot':pivots[stem]}
        (root/recipe_file).write_text(json.dumps(recipe)); asset_id='variants/'+recipe_file
        cooked=copy.deepcopy(source); cooked['asset_id']=asset_id
        for primitive in cooked['primitives']:
            primitive['id']=asset_id+primitive['id'][len(source['asset_id']):]
            for axis,value in enumerate(recipe['pivot']):
                if value: primitive['transform'][3][axis]=gate.f32(gate.f32(primitive['transform'][3][axis])-gate.f32(value))
        cooked['dependencies']=[{'uri':uri,'sha256':gate.runner.digest(root/file)} for uri,file in [('$source',recipe_file),(recipe['source'],source_name)]]
        (root/cooked_file).write_text(json.dumps(cooked))
        return root,(source_name,recipe_file,cooked_file,asset_id),cooked

    def test_variant_keeps_all_geometry_tables_and_transform_bits(self):
        root,args,cooked=self.variant_fixture()
        self.assertEqual(gate.check_variant(root,*args),cooked)

    def test_variant_rejects_geometry_material_identity_and_dependency_mutations(self):
        root,args,original=self.variant_fixture()
        mutations=[]
        for field,value in [('asset_id','other.pivot.json'),('dependencies',list(reversed(original['dependencies']))),
            ('images',original['images'][1:]),('materials',original['materials'][:1]),('primitives',original['primitives'][:1])]:
            changed=copy.deepcopy(original);changed[field]=value;mutations.append(changed)
        for field,value in [('indices',[0,0,1]),('material',1),('vertices',[]),('id','wrong#node=1')]:
            changed=copy.deepcopy(original);changed['primitives'][0][field]=value;mutations.append(changed)
        for column,row in [(0,0),(3,0),(3,3)]:
            changed=copy.deepcopy(original);changed['primitives'][0]['transform'][column][row]+=0.125;mutations.append(changed)
        for changed in mutations:
            (root/args[2]).write_text(json.dumps(changed))
            with self.assertRaises(AssertionError):gate.check_variant(root,*args)

    def test_variant_zero_axis_and_f32_rounding_fail_closed(self):
        root,args,cooked=self.variant_fixture(stem='offset-b')
        gate.check_variant(root,*args)
        changed=copy.deepcopy(cooked);changed['primitives'][0]['transform'][3][2]=0.0
        (root/args[2]).write_text(json.dumps(changed))
        with self.assertRaisesRegex(AssertionError,'transform bits'):gate.check_variant(root,*args)
        self.assertEqual(gate.f32(16777217),16777216)
        for value in (True,None,'1',float('inf'),float('nan'),1e100):
            with self.assertRaises((AssertionError,OverflowError)):gate.f32(value)

        # Actual Rust f32 JSON uses shortest round-trip decimals, unlike the
        # expanded Python f64 returned by unpacking an f32 arithmetic result.
        # These literal decimal cases reproduce the hosted export failure.
        for stem, short_translation in [('red', [-1.1, 2.4, -0.15]),
                                        ('blue', [0.9, 1.15, 0.1])]:
            root,args,cooked=self.variant_fixture(stem=stem)
            source=json.loads((root/args[0]).read_text())
            for original,variant in zip(source['primitives'],cooked['primitives']):
                original['transform'][3][:3]=[-0.099999994,1.9,0.1]
                variant['transform'][3][:3]=short_translation
            (root/args[0]).write_text(json.dumps(source))
            cooked['dependencies'][1]['sha256']=gate.runner.digest(root/args[0])
            (root/args[2]).write_text(json.dumps(cooked))
            self.assertNotEqual(short_translation[0],gate.f32(short_translation[0]))
            with self.subTest(stem=stem,case='shortest-f32-decimals'):
                self.assertEqual(gate.check_variant(root,*args),cooked)

            # One changed f32 bit anywhere in the full matrix still rejects,
            # including unchanged axes and non-translation matrix columns.
            for column in range(4):
                for row in range(4):
                    changed=copy.deepcopy(cooked)
                    value=changed['primitives'][0]['transform'][column][row]
                    bits=struct.unpack('<I',struct.pack('<f',value))[0]
                    changed['primitives'][0]['transform'][column][row]=struct.unpack('<f',struct.pack('<I',bits^1))[0]
                    (root/args[2]).write_text(json.dumps(changed))
                    with self.subTest(stem=stem,column=column,row=row), self.assertRaisesRegex(AssertionError,'transform bits'):
                        gate.check_variant(root,*args)

            mutations=[]
            for matrix in (cooked['primitives'][0]['transform'][:3],
                           cooked['primitives'][0]['transform']+[cooked['primitives'][0]['transform'][0]],
                           [column[:3] for column in cooked['primitives'][0]['transform']],
                           [column+[0.0] for column in cooked['primitives'][0]['transform']]):
                changed=copy.deepcopy(cooked);changed['primitives'][0]['transform']=matrix;mutations.append(changed)
            changed=copy.deepcopy(cooked);changed['primitives'][0]['transform'][0][1]=False;mutations.append(changed)
            changed=copy.deepcopy(cooked);changed['primitives'][0]['transform'][2][2]=True;mutations.append(changed)
            for matrix in (None,{},'not a matrix'):
                changed=copy.deepcopy(cooked);changed['primitives'][0]['transform']=matrix;mutations.append(changed)
            changed=copy.deepcopy(cooked);changed['primitives'][0]['transform'][0]={};mutations.append(changed)
            for value in (float('inf'),float('nan'),1e100):
                changed=copy.deepcopy(cooked);changed['primitives'][0]['transform'][0][0]=value;mutations.append(changed)
            changed=copy.deepcopy(cooked);changed['primitives'][0]['extra']=0;mutations.append(changed)
            changed=copy.deepcopy(cooked);del changed['primitives'][0]['indices'];mutations.append(changed)
            changed=copy.deepcopy(cooked);changed['primitives'][0]['vertices'][0]['position'][0]=0.125;mutations.append(changed)
            for index,changed in enumerate(mutations):
                (root/args[2]).write_text(json.dumps(changed))
                with self.subTest(stem=stem,mutation=index), self.assertRaises((AssertionError,OverflowError)):
                    gate.check_variant(root,*args)

            source_matrices=[None,{},'not a matrix',source['primitives'][0]['transform'][:3],
                source['primitives'][0]['transform']+[source['primitives'][0]['transform'][0]],
                [column[:3] for column in source['primitives'][0]['transform']],
                [column+[0.0] for column in source['primitives'][0]['transform']]]
            matrix=copy.deepcopy(source['primitives'][0]['transform']);matrix[0][1]=False;source_matrices.append(matrix)
            matrix=copy.deepcopy(source['primitives'][0]['transform']);matrix[2][2]=True;source_matrices.append(matrix)
            for value in (float('inf'),float('nan'),1e100):
                matrix=copy.deepcopy(source['primitives'][0]['transform']);matrix[0][0]=value;source_matrices.append(matrix)
            for index,matrix in enumerate(source_matrices):
                malformed=copy.deepcopy(source);malformed['primitives'][0]['transform']=matrix
                (root/args[0]).write_text(json.dumps(malformed))
                rebound=copy.deepcopy(cooked);rebound['dependencies'][1]['sha256']=gate.runner.digest(root/args[0])
                (root/args[2]).write_text(json.dumps(rebound))
                with self.subTest(stem=stem,source_matrix=index), self.assertRaises((AssertionError,OverflowError)):
                    gate.check_variant(root,*args)

    def test_numeric_oracle_has_nonvacuous_node_material_and_background_coverage(self):
        samples,coverage=gate.oracle_samples([(0,gate.PIVOTS[0]),(1,gate.PIVOTS[0])])
        self.assertGreater(len(samples),50000)
        self.assertGreater(coverage['clear_checked'],50000)
        self.assertEqual(len(coverage['panel_pixels']),2)
        self.assertTrue(all(n>10 for pair in coverage['marker_pixels'] for n in pair))

    def test_numeric_oracle_detects_wrong_sign_and_wrong_order(self):
        good,_=gate.oracle_samples([(0,gate.PIVOTS[0]),(1,gate.PIVOTS[0])])
        wrong,_=gate.oracle_samples([(0,tuple(-v for v in gate.PIVOTS[0])),(1,tuple(-v for v in gate.PIVOTS[0]))])
        self.assertGreater(sum(value!=wrong.get(offset) for offset,value in good.items()),200)
        # Applying -pivot directly in world space has the opposite x/y response
        # from the explicitly rotated/nonuniform external compensation.
        self.assertNotEqual((-0.625,0.375),(0.75*0.625,0.5*-0.375))

    def test_capture_inventory_rejects_extra_nonregular_and_unbounded_files(self):
        root=self.root/'captures';root.mkdir();(root/'one').write_bytes(b'x')
        gate.capture_files(root,{'one'})
        with self.assertRaises(AssertionError):gate.capture_files(root,set())
        if os.name=='nt':
            with mock.patch.object(pathlib.Path,'is_symlink',return_value=True),self.assertRaises(AssertionError):gate.capture_files(root,{'one'})
        else:
            (root/'alias').symlink_to(root/'one')
            with self.assertRaises(AssertionError):gate.capture_files(root,{'one','alias'})
            (root/'alias').unlink()
        # Sparse size-only fixture: no 64 MiB payload allocation or global stat mock.
        with (root/'one').open('r+b') as stream:
            stream.truncate(64*1024**2+1)
        with self.assertRaises(AssertionError):gate.capture_files(root,{'one'})

    def reference(self,root,name,data=None):
        if data is not None:(root/name).write_bytes(data)
        data=(root/name).read_bytes();value=0xcbf29ce484222325
        for byte in data:value=((value^byte)*0x100000001b3)&0xffffffffffffffff
        return {'file':name,'bytes':len(data),'fnv1a64':f'{value:016x}'}

    def gpu_fixture(self):
        root=self.root/'gpu';root.mkdir();assets={'original_glb':self.reference(root,'source.glb',b'checker-only synthetic GLB'),
            'variants':[],'source_hidden_after_install':True,'cooked_reload_before_render':True,'installed_files':2}
        for stem in ('offset-a','offset-b'):
            _,args,_=self.variant_fixture(root,stem)
            assets['variants'].append({'asset_id':args[3],'recipe':self.reference(root,args[1]),'cooked':self.reference(root,args[2])})
        assets['source']=self.reference(root,'source.model.json')
        placements={'offset_a':[(0,gate.PIVOTS[0]),(1,gate.PIVOTS[0])],
            'offset_b':[(0,gate.PIVOTS[1]),(1,gate.PIVOTS[1])],
            'variants_together':[(0,gate.PIVOTS[0]),(1,gate.PIVOTS[1])]}
        raw={};coverage={}
        for name,items in placements.items():
            pixels=bytearray(b'\0\0\0\xff'*(256*256));samples,coverage[name]=gate.oracle_samples(items)
            for offset,value in samples.items():pixels[offset:offset+4]=value
            raw[name]=bytes(pixels)
        raw['compensated_original_a']=raw['offset_a'];raw['compensated_original_b']=raw['offset_b'];raw['variants_reversed']=raw['variants_together']
        for index,name in enumerate(('no_pivot','wrong_sign','wrong_order','wrong_uv','wrong_material')):
            pixels=bytearray(b'\0\0\0\xff'*(256*256))
            for y in range(64,192):
                for x in range(64,192):pixels[(y*256+x)*4:(y*256+x+1)*4]=bytes((40+index,70,90,255))
            raw[name]=bytes(pixels)
        frames={}
        for name in gate.GPU_FRAMES:
            key=name.replace('-','_');data=raw[key]
            frames[key]={'raw':self.reference(root,name+'.rgba',data),'image':self.reference(root,name+'.ppm',b'P6\n256 256\n255\n'+bytes(v for i,v in enumerate(data) if i%4!=3))}
        adapter={'software':True,'backend':'Vulkan'};(root/'adapter.json').write_text(json.dumps(adapter))
        pairs={'no_pivot':'no_pivot','wrong_sign':'wrong_sign','wrong_order':'wrong_order','wrong_uv':'wrong_uv','wrong_material':'wrong_material','distinct_pivots':'offset_b'}
        accepted={'schema':1,'status':'passed','width':256,'height':256,'adapter':adapter,'assets':assets,'frames':frames,'coverage':coverage,
            'control_pixel_differences':{name:gate.difference(raw['offset_a'],raw[other]) for name,other in pairs.items()},
            'immutable_assets':True,'distinct_recipe_namespaces':True,'reversed_batch_order_byte_identical':True,
            'external_instances':'two Rz(pi) rotations with nonuniform scales; node 1 mirrored, node 7 rotated',
            'oracle':'independent compensated original E*T(-pivot)*M and numeric fixture-space interior/background',
            'tolerances':{'color_bytes':3,'minimum_control_differences':200}}
        (root/'receipt.json').write_text(json.dumps(accepted))
        return root,accepted

    def test_gpu_synthetic_checker_fixture_checks_all_oracles(self):
        root,_=self.gpu_fixture();gate.check_gpu(root)

    def test_gpu_rejects_receipt_pixel_control_and_provenance_mutations(self):
        root,receipt=self.gpu_fixture()
        mutations=[]
        for key,value in [('immutable_assets',False),('adapter',{'software':False,'backend':'Vulkan'}),('control_pixel_differences',{})]:
            changed=copy.deepcopy(receipt);changed[key]=value;mutations.append(changed)
        changed=copy.deepcopy(receipt);changed['assets']['variants'][0]['cooked']['fnv1a64']='0'*16;mutations.append(changed)
        for changed in mutations:
            (root/'receipt.json').write_text(json.dumps(changed))
            with self.assertRaises(AssertionError):gate.check_gpu(root)
        # Coherent receipt rehash still cannot hide an incorrect interior pixel.
        changed=copy.deepcopy(receipt);data=bytearray((root/'offset-a.rgba').read_bytes())
        samples,_=gate.oracle_samples([(0,gate.PIVOTS[0]),(1,gate.PIVOTS[0])])
        offset=next(o for o,value in samples.items() if value[:3]!=b'\0\0\0');data[offset:offset+4]=b'\x40\x40\x40\xff'
        for key,stem in [('offset_a','offset-a'),('compensated_original_a','compensated-original-a')]:
            changed['frames'][key]={'raw':self.reference(root,stem+'.rgba',data),
                'image':self.reference(root,stem+'.ppm',b'P6\n256 256\n255\n'+bytes(v for i,v in enumerate(data) if i%4!=3))}
        (root/'receipt.json').write_text(json.dumps(changed))
        with self.assertRaisesRegex(AssertionError,'numeric pixel'):gate.check_gpu(root)

    def export_inventory(self,root):
        files={path.relative_to(root).as_posix():[path.stat().st_size,gate.runner.digest(path)] for path in root.rglob('*') if path.is_file() and path.name!='capture-inventory.json'}
        (root/'capture-inventory.json').write_text(json.dumps(files))

    def export_fixture(self):
        root=self.root/'export';root.mkdir()
        for path in ('crates/orr_sample/tests/static_model_pivot_export.rs','crates/orr_model/examples/static_model_pivot.rs','crates/orr_model/examples/static_model_pivot/authoring.rs'):
            file=self.repo/path;file.parent.mkdir(parents=True,exist_ok=True);file.write_text('checker synthetic source '+path)
        variants=[]
        for stem,pivot in [('red',[1.0,-0.5,0.25]),('blue',[-1.0,0.75,0.0])]:
            _,args,_=self.variant_fixture(root,stem,'authoring-source.model.json')
            variants.append({'name':stem,'asset_id':args[3],'pivot':pivot,'recipe_sha256':gate.runner.digest(root/args[1]),'cooked_sha256':gate.runner.digest(root/args[2]),'cli_stdout':'published'})
        (root/'scene.glb').write_bytes(b'checker-only synthetic GLB')
        def png(value):
            def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
            return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1024,768,8,6,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+bytes([value,value,value,255])*1024)*768))+chunk(b'IEND',b'')
        observations={}
        for name,ticks,held in [('initial',0,''),('moved',18,'right'),('collision',180,'right'),('interacted',18,'right,interact')]:
            output=f'room capture adapter: checker fixture\nroom tick: {ticks} checksum: 0x123\nroom key: 1 won: 0\n'
            item={'ticks':ticks,'held':held,'checksum':'0x123','source_stdout':output,'export_stdout':output}
            for route in ('source','export'):
                for suffix,stem,data in [('png','runtime',png(ticks)),('frame','prepared-project',name.encode())]:
                    file=root/f'{stem}-{name}-{route}.{suffix}';file.write_bytes(data);item[f'{route}_{suffix}_sha256']=gate.runner.digest(file)
            observations[name]=item
        (root/'control-offscreen.png').write_bytes(png(1));(root/'control-original.png').write_bytes(png(2))
        accepted={'source_hidden':True,'read_only_bundle':True,'rejection_before_output_preserved':True,
            'limits':'explicit offline pivot authoring only; unchanged Room simulation/runtime/exporter; no automatic centering, transform authoring, glTF or collision claims',
            'visible_changed_pixels':1024*768,'pivot_changed_pixels':1024*768,'observations':observations,
            'control_stdout':[observations['initial']['source_stdout']]*2}
        for field,label in [('runtime_sha256','room_escape'),('exporter_sha256','orr_export_room'),('pivot_cli_sha256','pivot-cli'),('acceptance_executable_sha256','export-test')]:
            value=hashlib.sha256(label.encode()).hexdigest();accepted[field]=value;gate.runner.report['producers'][label]={'sha256':value}
        package='b'*64;project=root/'authored-project';project.mkdir();prefix='.orr/packages/objects/'+package+'/'
        files={}
        for name in ['orr.project.json','room.scene.yaml','room.models.json','room.camera.json','orr.packages.lock.json',prefix+'orr.package.json',prefix+'red.model.json',prefix+'blue.model.json']:
            file=project/name;file.parent.mkdir(parents=True,exist_ok=True);file.write_bytes((root/pathlib.Path(name).name).read_bytes() if name.endswith('.model.json') else b'{}');files['project/'+name]=[file.stat().st_size,gate.runner.digest(file)]
        files['bin/room_escape']=[10,accepted['runtime_sha256']];files['run-room-escape']=[4,hashlib.sha256(b'run!').hexdigest()]
        manifest={'schema':1,'payload':{'profile':'room-escape-authored-linux-x86_64-v1','runtime':{'sha256':accepted['runtime_sha256']},'packages':{'pivoted-static-props':package},'files':[{'path':name,'bytes':value[0],'sha256':value[1]} for name,value in files.items()]}}
        (root/'orr.export.json').write_text(json.dumps(manifest));files['orr.export.json']=[(root/'orr.export.json').stat().st_size,gate.runner.digest(root/'orr.export.json')];accepted['exported_file_inventory']=files
        source={'test':gate.EXPORT_TEST,'source_asset_id':'models/scene.glb','source_path':'inputs/source.model.json','glb_sha256':gate.runner.digest(root/'scene.glb'),
            'source_cooked_sha256':gate.runner.digest(root/'authoring-source.model.json'),'variants':variants,'original_source_exported':False,
            'initial_frame_sha256':gate.runner.digest(root/'prepared-project-initial-source.frame')}
        for field,name in [('test_source_sha256','crates/orr_sample/tests/static_model_pivot_export.rs'),('cli_source_sha256','crates/orr_model/examples/static_model_pivot.rs'),('authoring_source_sha256','crates/orr_model/examples/static_model_pivot/authoring.rs')]:source[field]=gate.runner.digest(self.repo/name)
        (root/'acceptance-source.json').write_text(json.dumps(source));(root/'production-receipt.json').write_text(json.dumps(accepted));self.export_inventory(root)
        return root,accepted

    def test_export_synthetic_checker_fixture_keeps_exact_capture_and_manifest_sets(self):
        root,_=self.export_fixture();gate.check_export(root)

    def test_export_rejects_frame_provenance_preservation_and_inventory_mutations(self):
        root,receipt=self.export_fixture()
        for key,value in [('source_hidden',False),('rejection_before_output_preserved',False),('runtime_sha256','0'*64),('exported_file_inventory',{})]:
            changed=copy.deepcopy(receipt);changed[key]=value;(root/'production-receipt.json').write_text(json.dumps(changed));self.export_inventory(root)
            with self.assertRaises(AssertionError):gate.check_export(root)
        (root/'production-receipt.json').write_text(json.dumps(receipt));(root/'prepared-project-moved-export.frame').write_bytes(b'changed');self.export_inventory(root)
        with self.assertRaises(AssertionError):gate.check_export(root)

    def test_workflow_matrix_and_required_coexistence(self):
        text = (ROOT / '.github/workflows/determinism.yml').read_text()
        # This is the real source file, not a fabricated positive workflow.
        workflow_contract(text)
        authoring = workflow_job(text, gate.WORKFLOW_JOB)
        acceptance = authoring.split('      - name: '+gate.WORKFLOW_STEP+'\n',1)[1].split('      - name:',1)[0]
        retention = authoring.split('      - name: '+gate.WORKFLOW_RETENTION+'\n',1)[1]
        retention_binding_contract(acceptance,retention)
        for changed_acceptance,changed_retention in [
            ('',retention),(acceptance*2,retention),
            (acceptance.replace(gate.WORKFLOW_EVIDENCE,'wrong-path'),retention),
            (acceptance,retention.replace(gate.WORKFLOW_EVIDENCE,'wrong-path')),
            (acceptance,retention.replace(gate.WORKFLOW_ARTIFACT,'wrong-artifact')),
            (acceptance,retention+'          path: '+gate.WORKFLOW_EVIDENCE+'\n'),
            (acceptance,retention+'          name: '+gate.WORKFLOW_ARTIFACT+'\n'),
            (acceptance,retention+'          "path": '+gate.WORKFLOW_EVIDENCE+'\n'),
            (acceptance,retention+"          'name' : "+gate.WORKFLOW_ARTIFACT+'\n'),
        ]:
            with self.subTest(binding=changed_retention),self.assertRaises(AssertionError):
                retention_binding_contract(changed_acceptance,changed_retention)
        mutations = {}
        for index,row in enumerate(gate.WORKFLOW_ROWS):
            original = matrix_row(*row)
            self.assertEqual(authoring.count(original), 1)
            mutations[f'missing row {index}'] = authoring.replace(original, '', 1)
            mutations[f'duplicate row {index}'] = authoring.replace(original, original*2, 1)
            for field,value in [('feature','other'),('mode','other'),('os','macos-latest'),('timeout',1)]:
                values = dict(zip(('feature','mode','os','timeout'),row));values[field] = value
                replacement = matrix_row(*(values[key] for key in ('feature','mode','os','timeout')))
                mutations[f'wrong {field} row {index}'] = authoring.replace(original,replacement,1)
        for label,old,new in [
            ('fail-fast','      fail-fast: false\n','      fail-fast: true\n'),
            ('job conditional','    runs-on:','    if: false\n    runs-on:'),
            ('job allowed failure','    runs-on:','    continue-on-error: true\n    runs-on:'),
            ('serial dependency','    runs-on:','    needs: sample-build\n    runs-on:'),
            ('wrong bytecode','      PYTHONDONTWRITEBYTECODE: "1"\n','      PYTHONDONTWRITEBYTECODE: "0"\n'),
            ('missing bytecode','      PYTHONDONTWRITEBYTECODE: "1"\n',''),
            ('step bytecode override','          ORR_STATIC_PIVOT_EVIDENCE:','          PYTHONDONTWRITEBYTECODE: "0"\n          ORR_STATIC_PIVOT_EVIDENCE:'),
            ('acceptance conditional','      - name: '+gate.WORKFLOW_STEP+'\n','      - name: '+gate.WORKFLOW_STEP+'\n        if: false\n'),
            ('acceptance allowed failure','      - name: '+gate.WORKFLOW_STEP+'\n','      - name: '+gate.WORKFLOW_STEP+'\n        continue-on-error: true\n'),
            ('wrong command','run: python -B tools/static_model_${{ matrix.feature }}_gate.py ${{ matrix.mode }}','run: echo skipped'),
            ('wrong feature command','tools/static_model_${{ matrix.feature }}_gate.py','tools/static_model_uv_gate.py'),
            ('wrong mode command','_gate.py ${{ matrix.mode }}','_gate.py cpu'),
            ('retention conditional','        if: always()\n','        if: success()\n'),
            ('wrong upload','uses: actions/upload-artifact@v4','run: echo skipped'),
            ('missing hidden files','          include-hidden-files: true\n',''),
            ('missing evidence ignored','          if-no-files-found: error\n','          if-no-files-found: ignore\n'),
            ('wrong artifact','          name: '+gate.WORKFLOW_ARTIFACT+'\n','          name: wrong-artifact\n'),
            ('wrong evidence','          ORR_STATIC_PIVOT_EVIDENCE: '+gate.WORKFLOW_EVIDENCE+'\n','          ORR_STATIC_PIVOT_EVIDENCE: wrong-path\n'),
            ('wrong upload path','          path: '+gate.WORKFLOW_EVIDENCE+'\n','          path: wrong-path\n'),
            ('missing Vulkan','if: matrix.mode != \'cpu\'','if: false'),
            ('missing namespace','if: matrix.mode == \'export\'','if: false'),
        ]:
            self.assertIn(old,authoring,label)
            mutations[label] = authoring.replace(old,new,1)
        mutations['extra matrix row'] = authoring.replace('    steps:\n',matrix_row('other','cpu','ubuntu-22.04',45)+'    steps:\n',1)
        for label,changed in mutations.items():
            with self.subTest(mutation=label),self.assertRaises(AssertionError):
                workflow_contract(text.replace(authoring,changed,1))
        aggregate = workflow_job(text,'cross_platform_checksum_compare')
        for label,old,new in [
            ('missing dependency',', static-model-authoring, room-wall-authoring]',', room-wall-authoring]'),
            ('missing generated user game dependency', ', generated-user-game-native', ''),
            ('aggregate conditional','    if: ${{ always() }}','    if: false'),
            ('aggregate allowed failure','    runs-on:','    continue-on-error: true\n    runs-on:'),
            ('missing result gate','Require every independent static authoring matrix row to pass','missing authoring gate'),
            ('conditional result gate','      - name: Require every independent static authoring matrix row to pass\n','      - name: Require every independent static authoring matrix row to pass\n        if: false\n'),
            ('allowed result failure','      - name: Require every independent static authoring matrix row to pass\n','      - name: Require every independent static authoring matrix row to pass\n        continue-on-error: true\n'),
            ('constant result',"${{ needs['static-model-authoring'].result }}",'success'),
            ('inverted result','if [[ "$AUTHORING_RESULT" != "success" ]]','if [[ "$AUTHORING_RESULT" == "success" ]]'),
            ('bypassed result','          echo "AUTHORING_RESULT=$AUTHORING_RESULT"','          exit 0\n          echo "AUTHORING_RESULT=$AUTHORING_RESULT"'),
            ('ignored result exit',"echo \"::error::AUTHORING_RESULT must be success, got '$AUTHORING_RESULT'\"\n            exit 1", "echo \"::error::AUTHORING_RESULT must be success, got '$AUTHORING_RESULT'\"\n            exit 0"),
        ]:
            self.assertIn(old,aggregate,label)
            with self.subTest(mutation=label),self.assertRaises(AssertionError):
                workflow_contract(text.replace(aggregate,aggregate.replace(old,new,1),1))
        for old,new in [
            ('Required generated Room GPU and source-hidden production export','missing Room gate'),
            ('  sample-room:\n','  sample-room:\n    # static_model_pivot_gate.py forbidden\n'),
            ('  sample-build:\n','  sample-build:\n    if: false\n'),
            ('run: python -B tools/static_model_retexture_gate.py cpu','run: echo skipped'),
        ]:
            with self.subTest(protected=old),self.assertRaises(AssertionError):
                protected_workflow_contract(text.replace(old,new,1))

EXPECTED_TESTS = ['test_actual_consumer_census_matches_current_source', 'test_actual_model_census_matches_current_source', 'test_capture_inventory_rejects_extra_nonregular_and_unbounded_files', 'test_complete_feature_source_contract', 'test_export_rejects_frame_provenance_preservation_and_inventory_mutations', 'test_export_synthetic_checker_fixture_keeps_exact_capture_and_manifest_sets', 'test_gpu_rejects_receipt_pixel_control_and_provenance_mutations', 'test_gpu_synthetic_checker_fixture_checks_all_oracles', 'test_inherited_bytecode_prevention_is_not_self_set', 'test_inventory_and_result_fail_closed', 'test_numeric_oracle_detects_wrong_sign_and_wrong_order', 'test_numeric_oracle_has_nonvacuous_node_material_and_background_coverage', 'test_optimized_python_rejects_before_producers_or_artifacts', 'test_producer_binds_complete_prebuild_source', 'test_producer_rejects_cached_wrong_source_and_duplicate_artifacts', 'test_required_ignored_consumer_runs_exactly', 'test_source_snapshot_covers_helper_and_fixture_producers', 'test_source_snapshot_fences_bytes_modes_and_new_paths', 'test_source_snapshot_rejects_symlinks', 'test_unrun_modes_cannot_be_claimed', 'test_variant_keeps_all_geometry_tables_and_transform_bits', 'test_variant_rejects_geometry_material_identity_and_dependency_mutations', 'test_variant_zero_axis_and_f32_rounding_fail_closed', 'test_workflow_matrix_and_required_coexistence']


def main():
    if sys.argv[1:] == ['--workflow-handoff']:
        workflow_contract((ROOT/'.github/workflows/determinism.yml').read_text())
        print('STATIC_PIVOT_WORKFLOW_HANDOFF: passed',flush=True)
        return
    assert sys.argv[1:] == [], 'usage: test_static_model_pivot_gate.py [--workflow-handoff]'
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(PivotGateTests)
    names=sorted(test.id().rsplit('.',1)[-1] for test in suite)
    assert names == EXPECTED_TESTS and len(names) == gate.FIXTURE_COUNT == 24
    result=unittest.TextTestRunner(verbosity=2,failfast=True).run(suite)
    if result.testsRun!=24 or result.skipped or result.expectedFailures or result.unexpectedSuccesses or not result.wasSuccessful():raise SystemExit(1)
    print('STATIC_PIVOT_GATE_FIXTURES: 24 passed; 0 failed; 0 skipped',flush=True)


if __name__ == '__main__':main()
