#!/usr/bin/env python3
"""Independent fail-closed sampler gate fixtures; no Cargo or GPU required."""
import copy
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zlib

import static_model_sampler_gate as gate
from test_static_model_extract_gate import listing, passed

ROOT = Path(__file__).resolve().parents[1]


def png(width=2, height=2):
    def chunk(kind,data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
    return (b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,6,0,0,0))+
            chunk(b'IDAT',zlib.compress((b'\0'+b'\x11\x22\x33\xff'*width)*height))+chunk(b'IEND',b''))


def fixture(root):
    source={'format':'orr_static_model','version':1,'asset_id':'source.glb', 'dependencies':[],
            'materials':[{'linear_filter':False,'wrap_s':'clamp','wrap_t':'clamp','image':0,'base_color':[1.0,-0.0,.5,.25]},
                         {'linear_filter':False,'wrap_s':'clamp','wrap_t':'clamp','image':0,'base_color':[.5,.5,.5,1.0]}],
            'images':[{'width':1,'height':1,'rgba8':[17,31,47,63]}],
            'primitives':[{'id':f'source.glb#node={i}/mesh=0/primitive=0','material':slot,'vertices':[{'uv':[-0.0,1.0]}]} for i,slot in enumerate([0,0,1])]}
    recipe={'format':'orr_static_material_sampler','version':1,'source':'source.model.json','material':0,'linear_filter':True,'wrap_s':'repeat','wrap_t':'mirror'}
    for name,data in [('source.model.json',source),('recipe.json',recipe)]: (root/name).write_text(json.dumps(data))
    cooked=copy.deepcopy(source);cooked['asset_id']='variants/test.sampler.json'
    for primitive in cooked['primitives']: primitive['id']=cooked['asset_id']+primitive['id'][len(source['asset_id']):]
    cooked['materials'][0].update({k:recipe[k] for k in ('linear_filter','wrap_s','wrap_t')})
    cooked['dependencies']=[{'uri':'$source','sha256':gate.runner.digest(root/'recipe.json')},{'uri':recipe['source'],'sha256':gate.runner.digest(root/'source.model.json')}]
    (root/'cooked.json').write_text(json.dumps(cooked))
    return cooked


class Fixtures(unittest.TestCase):
    def test_exact_current_rust_census(self):
        source=(ROOT/'crates/orr_model/tests/static_model_sampler.rs').read_text()
        declarations=re.findall(r'#\[test\](?:\n#\[ignore[^\n]*\])?\nfn (\w+)\(',source)
        all_names=set().union(*(set(gate.model_tests(p)) for p in ('linux','darwin','win32')))|{gate.CLI_TEST}
        self.assertEqual(set(declarations),all_names);self.assertEqual(len(declarations),22)
        attributes={name:attrs.splitlines() for attrs,name in re.findall(r'((?:^#\[[^\n]+\]\n)+)^fn (\w+)\(',source,re.M) if '#[test]' in attrs.splitlines()}
        conditionals={
            'cli_symlink_inputs_ancestors_and_destinations_reject':'#[cfg(unix)]',
            'cli_non_utf8_option_names_and_logical_ids_reject':'#[cfg(unix)]',
            'cli_non_utf8_filesystem_inputs_work':'#[cfg(target_os = "linux")]',
            'cli_windows_nonlocal_prefixes_reject':'#[cfg(windows)]'}
        for name,attrs in attributes.items():
            if name==gate.CLI_TEST:
                self.assertEqual(attrs,['#[test]','#[ignore = "mandatory current-source CLI requires ORR_STATIC_MODEL_SAMPLER_BIN"]'])
            else:self.assertEqual(attrs,([conditionals[name],'#[test]'] if name in conditionals else ['#[test]']))

        self.assertEqual([len(gate.model_tests(p)) for p in ('linux','darwin','win32')],[20,19,18])
        for package,target,name in [('orr_model_bindings','static_model_sampler',gate.BINDINGS_TEST),('orr_render','static_model_sampler',gate.GPU_TEST),('orr_editor','static_model_sampler_package_preview_ui',gate.UI_TEST)]:
            source=(ROOT/f'crates/{package}/tests/{target}.rs').read_text()
            self.assertEqual(re.findall(r'#\[test\]\n#\[ignore[^\n]*\]\nfn (\w+)\(',source),[name])

    def test_harness_rejects_empty_skip_failure(self):
        gate.runner.result(passed(['real_test']),['real_test'])
        for output in [passed([]),passed(['real_test'])+'\nSKIP: adapter unavailable',passed(['real_test']).replace('ok','FAILED'),passed(['different_test'])]:
            with self.assertRaises(AssertionError): gate.runner.result(output,['real_test'])
        with mock.patch.object(gate,'run'),self.assertRaises(AssertionError):gate.test('empty','binary',[])

    def test_ignored_consumer_is_explicit(self):
        calls=[]
        def run(label,command):
            calls.append(command)
            return listing(['real_test']) if '--list' in command else passed(['real_test'])
        with mock.patch.object(gate,'run',side_effect=run):gate.test('consumer','binary',['real_test'],True)
        self.assertEqual(calls[-1],['binary','--ignored','--exact','real_test','--test-threads=1'])

    def test_unrun_modes_cannot_pass(self):
        with mock.patch.object(gate,'mode','cpu'),mock.patch.object(gate.runner,'report',{'coverage':{k:'NOT RUN' for k in ('model','bindings','gpu','ui')}}):
            with self.assertRaises(AssertionError):gate.check_coverage()

    def test_source_fence_detects_mutation_and_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'a').write_text('one')
            with mock.patch.object(gate,'source_root',root):
                frozen=gate.snapshot()
                with mock.patch.object(gate,'frozen',frozen):
                    gate.source_fence();(root/'a').write_text('two')
                    with self.assertRaises(AssertionError):gate.source_fence()

    def test_capture_inventory_rejects_extra_and_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'a').write_text('one')
            self.assertEqual(set(gate.capture_files(root,{'a'})),{'a'})
            for names in [set(),{'a','b'}]:
                with self.assertRaises(AssertionError):gate.capture_files(root,names)

    def test_json_duplicate_and_nonfinite_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'receipt.json'
            for value in ['{"a":1,"a":2}','{"a":NaN}','{"a":1e999}']:
                path.write_text(value)
                with self.assertRaises(AssertionError):gate.strict_json(path)

    def test_png_pixels_and_mutations(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'frame.png';path.write_bytes(png())
            self.assertEqual(gate.png_pixels(path,2,2),b'\x11\x22\x33\xff'*4)
            for data in [png()[:-1],png()+b'x',png().replace(b'IDAT',b'IDAX'),png(3,2)]:
                path.write_bytes(data)
                with self.assertRaises(AssertionError):gate.png_pixels(path,2,2)

    def test_sampler_hand_calculated_axes_and_filter(self):
        self.assertEqual(gate.uv_sample((.25,.25),False,'clamp','clamp')[0],(239,31,71,255))
        self.assertEqual(gate.uv_sample((1.25,-.25),False,'repeat','mirror')[0],(239,31,71,255))
        self.assertEqual(gate.uv_sample((1.25,-.25),False,'mirror','repeat')[0],(179,137,43,255))
        self.assertEqual(gate.uv_sample((.25,.25),True,'repeat','mirror')[0],(239,31,71,255))
        self.assertIsNone(gate.uv_sample((.5,.25),False,'repeat','mirror'))
        self.assertNotEqual(gate.uv_sample((.5,.5),True,'repeat','mirror')[0],(239,31,71,255))

    def test_variant_preservation_and_negative_controls(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);original=fixture(root)
            def check():gate.check_variant(root,'source.model.json','recipe.json','cooked.json','variants/test.sampler.json')
            check()
            for kind in ('uv','zero','image','unselected','provenance','identity','filter'):
                value=copy.deepcopy(original)
                if kind=='uv':value['primitives'][0]['vertices'][0]['uv'][1]=.75
                if kind=='zero':value['materials'][0]['base_color'][1]=0.0
                if kind=='image':value['images'][0]['rgba8'][0]=0
                if kind=='unselected':value['materials'][1]['wrap_s']='repeat'
                if kind=='provenance':value['dependencies'].reverse()
                if kind=='identity':value['asset_id']='wrong.sampler.json'
                if kind=='filter':value['materials'][0]['linear_filter']=False
                (root/'cooked.json').write_text(json.dumps(value))
                with self.assertRaises(AssertionError):check()

    def test_workflow_all_modes_fail_closed_and_retained(self):
        text=(ROOT/'.github/workflows/static-model-sampler.yml').read_text()
        self.assertNotIn('continue-on-error',text);self.assertNotIn('paths:',text)
        for row in ('{mode: cpu, os: ubuntu-22.04}','{mode: cpu, os: windows-latest}','{mode: gpu, os: ubuntu-22.04}','{mode: ui, os: ubuntu-22.04}'):
            self.assertEqual(text.count(row),1)
        self.assertIn('needs: acceptance',text);self.assertIn('test "$ACCEPTANCE" = success',text)
        self.assertIn('if-no-files-found: error',text);self.assertIn('include-hidden-files: true',text)
        self.assertEqual(text.count('if: always()'),2)
        producer=re.search(r'^          ORR_STATIC_SAMPLER_EVIDENCE: (.+)$',text,re.M).group(1)
        upload=re.search(r'^          path: (.+)$',text,re.M).group(1)
        self.assertEqual(producer,upload);self.assertEqual(text.count('${{ runner.temp }}'),2)
        self.assertNotIn('runner.',text.split('    steps:')[0])

    def test_optimized_python_is_rejected(self):
        environment=dict(os.environ,PYTHONOPTIMIZE='1',PYTHONDONTWRITEBYTECODE='1')
        result=subprocess.run([sys.executable,'-B','-O',str(ROOT/'tools/static_model_sampler_gate.py'),'cpu'],env=environment,capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0);self.assertIn('requires unoptimized Python',result.stderr)


if __name__=='__main__':
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Fixtures))
    assert result.testsRun==gate.FIXTURE_COUNT and not result.skipped
    if not result.wasSuccessful():raise SystemExit(1)
    print(f'STATIC_SAMPLER_GATE_FIXTURES: {result.testsRun} passed; 0 failed; 0 skipped')
