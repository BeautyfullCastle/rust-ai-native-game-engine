#!/usr/bin/env python3
"""Independent cylinder gate contracts and rejection fixtures; no Cargo/GPU use."""
import ast
import copy
import hashlib
import inspect
import json
import os
import pathlib
import re
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib
from unittest import mock
import static_model_cylinder_gate as gate

ROOT = pathlib.Path(__file__).resolve().parents[1]


def workflow_contract(text):
    assert text.startswith("name: Static model cylinder\non:\n  pull_request:\n    branches: [main, su/engine-correctness-hardening]\n    types: [ready_for_review]\n  workflow_dispatch:\npermissions:\n")
    assert 'permissions:\n  contents: read\n' in text
    assert re.findall(r'^  ([a-z]+):$', text[text.index('jobs:\n'):], re.M) == ['acceptance', 'required']
    assert '    runs-on: ${{ matrix.os }}\n' in text
    assert '    timeout-minutes: 90\n' in text
    assert text.count('      matrix:\n') == 1
    assert re.findall(r'          - \{mode: (\w+), os: ([\w.-]+)\}', text) == list(gate.WORKFLOW_ROWS)
    assert 'continue-on-error' not in text and 'paths-ignore' not in text and 'branches-ignore:' not in text
    assert '      fail-fast: false\n' in text
    assert '      PYTHONDONTWRITEBYTECODE: "1"\n' in text
    assert '      CARGO_PROFILE_RELEASE_DEBUG: "0"\n' in text
    assert "        run: python -B tools/static_model_cylinder_gate.py '${{ matrix.mode }}'" in text
    assert '          ORR_STATIC_CYLINDER_EVIDENCE: '+gate.WORKFLOW_EVIDENCE in text
    assert '        shell: bash\n' in text
    assert '        if: always()\n        uses: actions/upload-artifact@v4' in text
    assert '          if-no-files-found: error\n' in text
    assert '          include-hidden-files: true\n' in text
    assert '          name: '+gate.WORKFLOW_ARTIFACT in text
    assert '          path: '+gate.WORKFLOW_EVIDENCE in text
    assert '    if: always()\n    needs: acceptance\n' in text
    assert '          ACCEPTANCE: ${{ needs.acceptance.result }}\n' in text
    assert '        run: test "$ACCEPTANCE" = success\n' in text
    assert [line.strip() for line in text.splitlines() if line.strip().startswith('if:')] == [
        "if: matrix.mode != 'cpu'", 'if: always()', 'if: always()']
    assert [line.strip() for line in text.splitlines() if line.strip().startswith('run:')] == [
        'run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers libxkbcommon-dev',
        "run: python -B tools/static_model_cylinder_gate.py '${{ matrix.mode }}'",
        'run: test "$ACCEPTANCE" = success']
    for line in text.splitlines():
        if 'runner.temp' in line:
            assert line.startswith('          ORR_STATIC_CYLINDER_EVIDENCE:') or line.startswith('          path:')


def fixture_model(segments=8, radius=.5, height=1.):
    recipe={'format':'orr_static_model_cylinder','version':1,'radius':radius,'height':height,'segments':segments}
    identity=[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]]
    model={'format':'orr_static_model','version':1,'asset_id':'props/key.cylinder.json',
           'dependencies':[{'uri':'$source','sha256':'a'*64}],
           'materials':[{'base_color':[1.]*4,'image':0,'linear_filter':False,'wrap_s':'clamp','wrap_t':'clamp'} for _ in range(3)],
           'images':[{'width':1,'height':1,'rgba8':[255]*4}], 'primitives':[]}
    for slot,(vertices,indices) in enumerate(gate.geometry_oracle(radius,height,segments)):
        model['primitives'].append({'id':f'props/key.cylinder.json#node=0/mesh=0/primitive={slot}',
                                    'vertices':vertices,'indices':indices,'material':slot,'transform':copy.deepcopy(identity)})
    return model,recipe


def encode_png(width, height, pixels):
    def chunk(kind, data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
    rows=b''.join(b'\0'+pixels[y*width*4:(y+1)*width*4] for y in range(height))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(rows))+chunk(b'IEND',b'')


class Fixtures(unittest.TestCase):
    def test_workflow_exact_contract(self):
        workflow_contract((ROOT/'.github/workflows/static-model-cylinder.yml').read_text())

    def test_workflow_negative_mutations(self):
        original=(ROOT/'.github/workflows/static-model-cylinder.yml').read_text()
        # Only explicit integration readiness or dispatch may start expensive CI.
        trigger_filters = "    branches: [main, su/engine-correctness-hardening]\n    types: [ready_for_review]\n"
        for old, new in [
            (trigger_filters, ''),
            ('branches: [main, su/engine-correctness-hardening]', "branches: ['**']"),
            ('branches: [main, su/engine-correctness-hardening]', 'branches: [main]'),
            ('branches: [main, su/engine-correctness-hardening]', 'branches: [su/engine-correctness-hardening]'),
            ("    types: [ready_for_review]\n", ''),
            ('types: [ready_for_review]', 'types: [opened, synchronize]'),
            ('  pull_request:\n', ''),
            ('  pull_request:\n', '  pull_request:\n    branches: [main]\n'),
            ('  pull_request:\n', '  pull_request:\n    paths: [src/**]\n'),
        ]:
            with self.subTest(trigger=old), self.assertRaises(AssertionError):
                workflow_contract(original.replace(old, new, 1))
        for old,new in [('mode: gpu','mode: cpu'),('mode: ui','mode: gpu'),('if-no-files-found: error','if-no-files-found: warn'),
                        ('if: always()','if: success()'),('test "$ACCEPTANCE" = success','true'),
                        ('PYTHONDONTWRITEBYTECODE: "1"','PYTHONDONTWRITEBYTECODE: "0"'),
                        ('include-hidden-files: true','include-hidden-files: false'),('needs: acceptance','needs: other'),
                        ('        shell: bash','        shell: pwsh'),('timeout-minutes: 90','timeout-minutes: 1')]:
            with self.subTest(old=old),self.assertRaises(AssertionError): workflow_contract(original.replace(old,new))
        with self.assertRaises(AssertionError):
            workflow_contract(original.replace('    env:\n','    env:\n      BAD: ${{ runner.temp }}\n',1))
        with self.assertRaises(AssertionError):
            workflow_contract(original.replace('    runs-on: ${{ matrix.os }}','    if: false\n    runs-on: ${{ matrix.os }}'))

    def test_required_sources_and_legacy_current_source_fence(self):
        names=set(gate.FEATURE_PATHS+gate.INFRA_PATHS+gate.LEGACY_PATHS)
        for package in ('orr_model','orr_model_bindings','orr_render','orr_editor'):
            names.update((f'crates/{package}/Cargo.toml',f'crates/{package}/src/lib.rs'))
        snapshot={name:{'sha256':'b'*64} for name in names}
        gate.require_sources(snapshot)
        for name in names:
            altered=dict(snapshot);del altered[name]
            with self.assertRaises(AssertionError):gate.require_sources(altered)
        # Current-run source fences protect provenance. Historical Cargo/runtime
        # hashes must not forbid legitimate unrelated changes in future PRs.
        self.assertFalse(hasattr(gate,'PRESERVED_SHA256'))

    def test_source_snapshot_bytes_modes_and_inventory(self):
        with tempfile.TemporaryDirectory() as root:
            root=pathlib.Path(root);p=root/'a';p.write_bytes(b'a')
            with mock.patch.object(gate,'source_root',root):
                first=gate.snapshot();p.write_bytes(b'b');self.assertNotEqual(first,gate.snapshot())
                p.write_bytes(b'a')
                if os.name!='nt':
                    p.chmod(0o700);self.assertNotEqual(first,gate.snapshot())
                (root/'extra').write_bytes(b'x');self.assertNotEqual(set(first),set(gate.snapshot()))

    def test_source_fence_and_inventory_reject_untracked(self):
        with tempfile.TemporaryDirectory() as root:
            root=pathlib.Path(root);p=root/'a';p.write_bytes(b'a')
            with mock.patch.object(gate,'source_root',root),mock.patch.object(gate.runner,'record'):
                frozen=gate.snapshot()
                with mock.patch.object(gate,'frozen',frozen):
                    gate.source_fence();p.write_bytes(b'b')
                    with self.assertRaises(AssertionError):gate.source_fence()
                    p.write_bytes(b'a');(root/'ignored-untracked').write_bytes(b'x')
                    with self.assertRaises(AssertionError):gate.source_fence()
                with self.assertRaises(AssertionError):gate.check_source_inventory(['a'],'fixture')

    def test_exact_inventory_rejects_extra_missing_and_duplicates(self):
        good='one: test\ntwo: test\n\n2 tests, 0 benchmarks\n'
        gate.runner.inventory(good,['one','two'])
        for bad in [good.replace('two: test','three: test'),good.replace('2 tests','1 tests'),good+'three: test\n',good.replace('two: test','one: test')]:
            with self.assertRaises(AssertionError):gate.runner.inventory(bad,['one','two'])

    def test_exact_results_reject_skip_failure_filtered_or_missing(self):
        good='running 2 tests\ntest one ... ok\ntest two ... ok\n\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
        gate.runner.result(good,['one','two'])
        for bad in [good.replace('two ... ok','two ... ignored'),good.replace('2 passed','1 passed'),good.replace('0 filtered','1 filtered'),
                    good.replace('two ... ok','wrong ... ok'),good+'SKIP: missing adapter\n']:
            with self.assertRaises(AssertionError):gate.runner.result(bad,['one','two'])

    def test_optimized_python_rejects_before_work(self):
        environment=dict(os.environ,PYTHONDONTWRITEBYTECODE='1')
        completed=subprocess.run([sys.executable,'-B','-O',str(ROOT/'tools/static_model_cylinder_gate.py'),'cpu'],
                                 capture_output=True,text=True,env=environment,check=False)
        self.assertNotEqual(completed.returncode,0)
        self.assertIn('requires nonoptimized Python',completed.stderr)

    def test_inherited_bytecode_prevention_is_mandatory(self):
        with mock.patch.object(gate.runner,'record'):
            with mock.patch.dict(os.environ,{'PYTHONDONTWRITEBYTECODE':'1'}):gate.require_inherited_bytecode_prevention()
            for value in ('0',''):
                with mock.patch.dict(os.environ,{'PYTHONDONTWRITEBYTECODE':value}),self.assertRaises(AssertionError):gate.require_inherited_bytecode_prevention()

    def test_exact_model_and_executable_test_census(self):
        source=(ROOT/'crates/orr_model/tests/static_model_cylinder.rs').read_text()
        names=set(re.findall(r'#\[test\](?:\n#\[[^\n]+\])*\nfn (\w+)',source))
        all_names=set().union(*(gate.model_tests(platform) for platform in ('linux','darwin','win32')))|{gate.CLI_TEST}
        self.assertEqual(names,all_names)
        self.assertIn('actual_cli_publishes_validated_output_and_rejects_second_write',all_names)
        for package,target,name in [('orr_model_bindings','static_model_cylinder',gate.BINDINGS_TEST),
                                    ('orr_render','static_model_cylinder',gate.GPU_TEST),
                                    ('orr_editor','static_model_cylinder_package_preview_ui',gate.UI_TEST)]:
            source=(ROOT/f'crates/{package}/tests/{target}.rs').read_text()
            names=re.findall(r'#\[test\](?:\n#\[[^\n]+\])*\nfn (\w+)',source)
            self.assertEqual(names,[name])
            self.assertRegex(source,r'#\[ignore[^\n]*\]')

    def test_producer_lifecycle_and_no_bypass_flags(self):
        source=(ROOT/'tools/static_model_cylinder_gate.py').read_text();ast.parse(source)
        for marker in ('runner.fence()', 'source_fence()', '--message-format=json', 'expected=2', "'--', '-D', 'warnings'",'check_coverage()', 'NOT RUN here'):
            self.assertIn(marker,source)
        for marker in ("'--skip'",'continue-on-error','CARGO_TARGET_DIR=target/release'):
            self.assertNotIn(marker,source)

    def test_strict_json_rejects_duplicate_nonfinite_and_huge_exponents(self):
        with tempfile.TemporaryDirectory() as root:
            path=pathlib.Path(root)/'receipt.json'
            for text in ('{"a":1,"a":2}','{"value":NaN}','{"value":Infinity}','{"value":1e999}'):
                path.write_text(text)
                with self.assertRaises(AssertionError):gate.strict_json(path)
            path.write_text('{"value":1}');self.assertEqual(gate.strict_json(path),{'value':1})

    def test_capture_inventory_and_hash_reference(self):
        with tempfile.TemporaryDirectory() as root:
            root=pathlib.Path(root);path=root/'frame.rgba';path.write_bytes(b'1234')
            self.assertEqual(set(gate.capture_files(root,{'frame.rgba'})),{'frame.rgba'})
            gate.check_reference(root,{'file':'frame.rgba','bytes':4,'fnv1a64':gate.fnv1a64(b'1234')})
            for reference in ({'file':'../frame.rgba','bytes':4,'fnv1a64':'0'*16},{'file':'frame.rgba','bytes':3,'fnv1a64':gate.fnv1a64(b'1234')}):
                with self.assertRaises(AssertionError):gate.check_reference(root,reference)
            (root/'extra').write_bytes(b'x')
            with self.assertRaises(AssertionError):gate.capture_files(root,{'frame.rgba'})

    def test_png_decoding_rejects_corruption_truncation_or_wrong_extent(self):
        with tempfile.TemporaryDirectory() as root:
            path=pathlib.Path(root)/'a.png';data=encode_png(2,1,b'\x01\x02\x03\xff'*2);path.write_bytes(data)
            self.assertEqual(gate.png_pixels(path,2,1),b'\x01\x02\x03\xff'*2)
            for bad in (data[:-1],data+b'x',data[:50]+bytes([data[50]^1])+data[51:]):
                path.write_bytes(bad)
                with self.assertRaises(AssertionError):gate.png_pixels(path,2,1)
            path.write_bytes(data)
            with self.assertRaises(AssertionError):gate.png_pixels(path,1,2)

    def test_geometry_oracle_all_supported_sizes(self):
        for segments in (8,16,32,64,128):
            for radius,height in ((.001,.001),(.5,1.),(1000000.,1000000.)):
                model,recipe=fixture_model(segments,radius,height)
                gate.validate_cylinder(model,recipe,'a'*64,'props/key.cylinder.json')
                self.assertEqual(sum(len(p['vertices']) for p in model['primitives']),4*segments+4)
                self.assertEqual(sum(len(p['indices']) for p in model['primitives']),12*segments)

    def test_geometry_oracle_rejects_owned_field_mutations(self):
        mutations=[lambda m:m['primitives'][0]['vertices'][3]['position'].__setitem__(0,.2),
                   lambda m:m['primitives'][1]['vertices'][1]['normal'].__setitem__(1,-1.),
                   lambda m:m['primitives'][0]['vertices'][0]['uv'].__setitem__(0,.5),
                   lambda m:m['primitives'][2]['indices'].__setitem__(1,2),
                   lambda m:m['primitives'][1].__setitem__('material',0),
                   lambda m:m['images'][0]['rgba8'].__setitem__(0,0),
                   lambda m:m['dependencies'][0].__setitem__('sha256','b'*64),
                   lambda m:m['primitives'][0].__setitem__('id','wrong')]
        for mutation in mutations:
            model,recipe=fixture_model();mutation(model)
            with self.assertRaises(AssertionError):gate.validate_cylinder(model,recipe,'a'*64,'props/key.cylinder.json')

    def test_geometry_recipe_rejects_schema_and_limits(self):
        for key,value in [('segments',True),('segments',24),('radius',.0009),('height',1000001),('radius',float('inf')),('version',True)]:
            model,recipe=fixture_model();recipe[key]=value
            with self.assertRaises(AssertionError):gate.validate_cylinder(model,recipe,'a'*64,'props/key.cylinder.json')
        model,recipe=fixture_model();recipe['unexpected']=1
        with self.assertRaises(AssertionError):gate.validate_cylinder(model,recipe,'a'*64,'props/key.cylinder.json')

    def test_exact_recipe_byte_hash_is_bound_to_retained_model(self):
        with tempfile.TemporaryDirectory() as root:
            root=pathlib.Path(root);model,recipe=fixture_model();r=root/'recipe.json';m=root/'cooked.json'
            r.write_text(json.dumps(recipe));model['dependencies'][0]['sha256']=gate.runner.digest(r);m.write_text(json.dumps(model))
            gate.check_cylinder(r,m,'props/key.cylinder.json')
            r.write_text(json.dumps(recipe)+'\n')
            with self.assertRaises(AssertionError):gate.check_cylinder(r,m,'props/key.cylinder.json')

    def test_coverage_cannot_promote_unrun_rows(self):
        keys=('model','bindings','gpu','ui')
        for mode,passed in [('cpu',{'model','bindings'}),('gpu',{'gpu'}),('ui',{'ui'})]:
            with mock.patch.object(gate,'mode',mode),mock.patch.object(gate.runner,'report',{'coverage':{key:'PASSED' if key in passed else 'NOT RUN' for key in keys}}):
                gate.check_coverage();gate.runner.report['coverage'][next(key for key in keys if key not in passed)]='PASSED'
                with self.assertRaises(AssertionError):gate.check_coverage()

    def test_pixel_comparisons_are_measured_not_receipt_booleans(self):
        self.assertEqual(gate.difference(bytes([0,0,0,255]*4),bytes([0,0,0,255]*3+[5,0,0,255])),1)
        self.assertEqual(gate.difference(bytes([0,0,0,255]),bytes([3,0,0,255]),3),0)
        with self.assertRaises(AssertionError):gate.difference(b'123',b'123')

    def test_explicit_ignored_partition_never_accepts_ignored_as_pass(self):
        ordinary='running 2 tests\ntest one ... ok\ntest pinned ... ignored, producer required\n\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
        gate.executed(ordinary,['one'],ignored_names=['pinned'])
        explicit='running 1 test\ntest pinned ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.01s\n'
        gate.executed(explicit,['pinned'],filtered=1)
        for output,names,filtered,ignored in [(ordinary,['one'],0,[]),(ordinary,['one','pinned'],0,[]),
                                             (explicit,['pinned'],0,[]),(explicit.replace('pinned ... ok','pinned ... ignored'),['pinned'],1,[])]:
            with self.assertRaises(AssertionError):gate.executed(output,names,filtered,ignored)

    def test_ray_geometry_uses_polygon_planes_and_caps(self):
        spec=(.55,2.,32)
        self.assertEqual(gate.ray_hit(0.,0.,spec,1)[0],0)
        self.assertEqual(gate.ray_hit(0.,.8,spec,1)[0],1)
        self.assertEqual(gate.ray_hit(0.,-.8,spec,-1)[0],2)
        self.assertIsNone(gate.ray_hit(1.5,1.5,spec,1))
        self.assertAlmostEqual(gate.ray_hit(0.,.8,spec,1)[1][0],.5)
        self.assertAlmostEqual(gate.ray_hit(0.,.8,spec,1)[1][1],.5)
        tree=ast.parse(inspect.getsource(gate.ray_hit))
        names={node.id for node in ast.walk(tree) if isinstance(node,ast.Name)}
        for marker in ('geometry_oracle','primitives','indices','QUARTER_SIN'):
            self.assertNotIn(marker,names)

    @staticmethod
    def ray_fixture(kind):
        spec=(.55,2.,32);pixels=bytearray([0,0,0,255]*256*256)
        for offset,surface,uv in gate.ray_samples(spec,1):
            if surface<0:continue
            if kind=='white':color=(255,255,255,255)
            elif kind=='lit':color=([188,231,124][surface],)*3+(255,)
            else:color=gate.TEXELS[min(1,max(0,int(uv[1]*2)))*2+min(1,max(0,int(uv[0]*2)))]
            pixels[offset:offset+4]=bytes(color)
        return pixels

    def test_ray_oracle_positive_blank_wrong_normal_and_missing_cap(self):
        spec=(.55,2.,32)
        for kind in ('white','lit'):
            pixels=self.ray_fixture(kind);gate.check_ray_pixels(pixels,spec,1,kind)
            with self.assertRaises(AssertionError):gate.check_ray_pixels(bytes([0,0,0,255]*256*256),spec,1,kind)
        pixels=self.ray_fixture('lit')
        for offset,surface,_ in gate.ray_samples(spec,1):
            if surface==0:pixels[offset:offset+4]=bytes((231,231,231,255))
        with self.assertRaises(AssertionError):gate.check_ray_pixels(pixels,spec,1,'lit')
        pixels=self.ray_fixture('white')
        for offset,surface,_ in gate.ray_samples(spec,1):
            if surface==1:pixels[offset:offset+4]=bytes((0,0,0,255))
        with self.assertRaises(AssertionError):gate.check_ray_pixels(pixels,spec,1,'white')

    def test_ray_oracle_checker_wrong_colors_background_and_extent(self):
        spec=(.55,2.,32);pixels=self.ray_fixture('checker')
        gate.check_ray_pixels(pixels,spec,1,'checker')
        wrong=bytearray(pixels)
        for offset,surface,uv in gate.ray_samples(spec,1):
            if surface>=0:wrong[offset:offset+4]=bytes((255,255,255,255))
        with self.assertRaises(AssertionError):gate.check_ray_pixels(wrong,spec,1,'checker')
        wrong=bytearray(pixels);wrong[:4]=bytes((255,0,0,255))
        with self.assertRaises(AssertionError):gate.check_ray_pixels(wrong,spec,1,'checker')
        with self.assertRaises(AssertionError):gate.check_ray_pixels(pixels[:-4],spec,1,'checker')

    @staticmethod
    def manifest_fixture(root):
        path=root/'lvp_icd.json'
        value={'file_format_version':'1.0.0','ICD':{'api_version':'1.4.305','library_path':'libvulkan_lvp.so'}}
        path.write_text(json.dumps(value))
        return path,value

    def test_vulkan_manifest_explicit_agreement_and_single_system_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve();path,_=self.manifest_fixture(root)
            for environment in ({'VK_DRIVER_FILES':str(path)},{'VK_ICD_FILENAMES':str(path)},
                                {'VK_DRIVER_FILES':str(path),'VK_ICD_FILENAMES':str(path)},{}):
                selected,raw=gate.software_manifest(environment,root)
                self.assertEqual(selected,path);self.assertEqual(raw,path.read_bytes())
            (root/'lvp_icd.second.json').write_bytes(path.read_bytes())
            with self.assertRaises(AssertionError):gate.software_manifest({},root)
            # An explicit validated selection is independent of other installed ICDs.
            self.assertEqual(gate.software_manifest({'VK_DRIVER_FILES':str(path)},root)[0],path)
            with self.assertRaises(AssertionError):gate.software_manifest({},root/'absent')

    def test_vulkan_manifest_rejects_lists_disagreement_relative_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve();path,_=self.manifest_fixture(root)
            for environment in ({'VK_DRIVER_FILES':''},{'VK_DRIVER_FILES':str(path)+':'+str(path)},
                                {'VK_DRIVER_FILES':str(path)+';'+str(path)},{'VK_DRIVER_FILES':'lvp_icd.json'},
                                {'VK_DRIVER_FILES':str(path),'VK_ICD_FILENAMES':str(root/'different.json')},
                                {'VK_DRIVER_FILES':str(root/'missing.json')},{'VK_DRIVER_FILES':str(root)}):
                with self.assertRaises(AssertionError):gate.software_manifest(environment,root)
            # These portable fixtures cover Windows without requiring symlink privileges.
            with mock.patch.object(pathlib.Path,'is_symlink',return_value=True),self.assertRaises(AssertionError):
                gate.software_manifest({'VK_DRIVER_FILES':str(path)},root)
            with mock.patch.object(pathlib.Path,'resolve',return_value=root/'elsewhere.json'),self.assertRaises(AssertionError):
                gate.software_manifest({'VK_DRIVER_FILES':str(path)},root)
            if os.name!='nt':
                alias=root/'lvp_icd.alias.json';alias.symlink_to(path.name)
                with self.assertRaises(AssertionError):gate.software_manifest({'VK_DRIVER_FILES':str(alias)},root)
                parent=root/'alias';parent.symlink_to(root,target_is_directory=True)
                with self.assertRaises(AssertionError):gate.software_manifest({'VK_DRIVER_FILES':str(parent/path.name)},root)

    def test_vulkan_manifest_rejects_invalid_oversized_duplicate_and_nonlavapipe(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve();path,value=self.manifest_fixture(root)
            invalid=[b'not JSON',b' '*65537,b'{"ICD":{},"ICD":{}}',b'[]',b'null',b'\xff',
                     json.dumps({**value,'file_format_version':'2.0.0'}).encode(),
                     json.dumps({**value,'ICD':{'api_version':'1.4.0','library_path':'libvulkan_radeon.so'}}).encode(),
                     json.dumps({**value,'ICD':{'api_version':'bogus','library_path':'libvulkan_lvp.so'}}).encode()]
            for raw in invalid:
                path.write_bytes(raw)
                with self.assertRaises((AssertionError,json.JSONDecodeError,UnicodeDecodeError)):
                    gate.software_manifest({'VK_DRIVER_FILES':str(path)},root)
            path.write_text(json.dumps(value));other=root/'hardware.json';other.write_bytes(path.read_bytes())
            with self.assertRaises(AssertionError):gate.software_manifest({'VK_DRIVER_FILES':str(other)},root)

    def test_vulkan_selection_retains_bytes_hash_and_final_manifest_fence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve();path,_=self.manifest_fixture(root)
            evidence=root/'evidence';evidence.mkdir();raw=path.read_bytes();report={}
            with mock.patch.object(gate.runner,'evidence',evidence),mock.patch.object(gate.runner,'report',report),mock.patch.object(gate.runner,'record'),\
                 mock.patch.dict(os.environ,{'VK_DRIVER_FILES':str(path),'LD_LIBRARY_PATH':'unchanged'},clear=True):
                gate.configure_software()
                self.assertEqual(os.environ['VK_DRIVER_FILES'],os.environ['VK_ICD_FILENAMES'])
                self.assertEqual(os.environ['ORR_REQUIRE_GPU'],'1');self.assertEqual(os.environ['WGPU_BACKEND'],'vulkan')
                self.assertEqual(os.environ['LD_LIBRARY_PATH'],'unchanged')
                self.assertEqual((evidence/'lavapipe-icd.json').read_bytes(),raw)
                self.assertEqual(report['icd_sha256'],hashlib.sha256(raw).hexdigest())
                self.assertEqual(report['icd_manifest'],{'path':str(path),'file':'lavapipe-icd.json','bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest()})
                gate.software_fence();path.write_bytes(raw+b'\n')
                with self.assertRaises(AssertionError):gate.software_fence()
                path.write_bytes(raw);(evidence/'lavapipe-icd.json').write_bytes(raw+b'\n')
                with self.assertRaises(AssertionError):gate.software_fence()


if __name__=='__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(Fixtures)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.testsRun != gate.FIXTURE_COUNT or result.skipped:
        raise SystemExit(1)
    print(f'STATIC_CYLINDER_GATE_FIXTURES: {gate.FIXTURE_COUNT} passed; 0 failed; 0 skipped')
