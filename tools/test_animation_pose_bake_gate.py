#!/usr/bin/env python3
"""Independent failure-injection tests for source and pixel evidence admission."""
import copy
import hashlib
import json
import pathlib
import tempfile
import unittest
from unittest import mock
import animation_pose_bake_gate as gate

class GateFixtures(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.root=pathlib.Path(self.temp.name)
        self.saved=(gate.source_root,gate.frozen,gate.mode,gate.runner.report,gate.runner.evidence)
        gate.source_root=self.root;gate.frozen=None;gate.runner.report={};gate.runner.evidence=self.root
    def tearDown(self):
        gate.source_root,gate.frozen,gate.mode,gate.runner.report,gate.runner.evidence=self.saved
        self.temp.cleanup()
    def test_source_bytes_modes_and_inventory_fenced(self):
        p=self.root/'source.rs';p.write_text('first');gate.frozen=gate.snapshot();gate.source_fence()
        p.write_text('other')
        with self.assertRaises(AssertionError):gate.source_fence()
        p.write_text('first')
        original=p.stat().st_mode
        try:
            p.chmod(0o444 if gate.os.name=='nt' else 0o700)
            with self.assertRaises(AssertionError):gate.source_fence()
        finally:
            p.chmod(original)
    def test_untracked_and_symlink_source_rejected(self):
        p=self.root/'source.rs';p.write_text('source')
        with mock.patch.object(gate.runner,'record'):
            gate.check_source_inventory(['source.rs'],'good');(self.root/'.ignored').write_text('injected')
            with self.assertRaises(AssertionError):gate.check_source_inventory(['source.rs'],'bad')
        if gate.os.name!='nt':
            (self.root/'alias').symlink_to(p)
            with self.assertRaises(AssertionError):gate.snapshot()
    def test_exact_feature_inventory_required(self):
        required=set(gate.FEATURE_PATHS+gate.INFRA_PATHS)
        for package in ('orr_model','orr_render'):required.update([f'crates/{package}/Cargo.toml',f'crates/{package}/src/lib.rs'])
        gate.require_sources(required)
        for path in gate.FEATURE_PATHS:
            with self.assertRaises(AssertionError):gate.require_sources(required-{path})
    def test_lanes_cannot_claim_other_coverage(self):
        for mode in ('cpu','gpu'):
            gate.mode=mode;gate.runner.report['coverage']={k:('PASSED' if k=={'cpu':'model','gpu':'gpu'}[mode] else 'NOT RUN') for k in ('model','gpu')};gate.check_coverage()
            gate.runner.report['coverage']={'model':'PASSED','gpu':'PASSED'}
            with self.assertRaises(AssertionError):gate.check_coverage()
    def test_platform_test_census_retains_socket(self):
        linux=gate.model_tests('linux');windows=gate.model_tests('win32')
        self.assertEqual(len(linux),25);self.assertEqual(len(windows),22)
        self.assertIn('cli_unix_socket_inputs_reject_without_opening_or_publishing',linux)
        self.assertNotIn('cli_unix_socket_inputs_reject_without_opening_or_publishing',windows)
        self.assertEqual(len(linux),len(set(linux)))
    def test_independent_pixel_oracle_rejects_clear_and_corruption(self):
        pixels,interior,clear=gate.pixel_oracle();self.assertEqual(interior,[2126,1207]);self.assertEqual(clear,56987)
        good=bytes(v for p in pixels for v in (p or (0,0,0,255)))
        self.assertEqual(gate.mismatches(good,pixels),0)
        self.assertGreater(gate.mismatches(bytes([0,0,0,255])*65536,pixels),100)
        bad=bytearray(good);i=next(i for i,p in enumerate(pixels) if p is not None);bad[i*4]^=255
        self.assertEqual(gate.mismatches(bad,pixels),1)
        with self.assertRaises(AssertionError):gate.mismatches(good[:-1],pixels)
    def test_capture_inventory_rejects_missing_extra_and_symlink(self):
        with self.assertRaises(AssertionError):gate.check_gpu(self.root)
        (self.root/'extra').write_bytes(b'')
        with self.assertRaises(AssertionError):gate.check_gpu(self.root)
    def captures(self):
        pixels,interior,clear=gate.pixel_oracle();data=bytes(v for p in pixels for v in (p or (0,0,0,255)))
        bad=bytearray(data)
        for i,p in enumerate(pixels):
            if p is not None:bad[i*4]^=255
        count=gate.mismatches(bad,pixels)
        report={'adapter':{'software':True,'backend':'Vulkan','name':'llvmpipe'},'size':256,'tolerance':3,'interior':interior,'clear':clear,'static_oracle_mismatches':0,'skinned_oracle_mismatches':0,'controls':[{'name':n,'oracle_mismatches':count} for n in gate.CONTROLS]}
        for name in gate.FRAMES:
            frame=bytes(bad) if name in gate.CONTROLS else data
            (self.root/(name+'.rgba')).write_bytes(frame);(self.root/(name+'.ppm')).write_bytes(b'P6\n256 256\n255\n'+b''.join(frame[i:i+3] for i in range(0,len(frame),4)))
        for name in ('midpoint','wrong-pose','wrong-weights'):
            source={'materials':[],'images':[],'primitives':[{'indices':[0,1,2],'material':0,'vertices':[{'vertex':{'uv':[0,0]}}]}]*2}
            recipe={'source':name+'.animated.json'};rb=json.dumps(recipe).encode();sb=json.dumps(source).encode()
            output={'format':'orr_static_model','version':1,'materials':[],'images':[],'primitives':[{'indices':[0,1,2],'material':0,'vertices':[{'uv':[0,0]}],'transform':[[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]}]*2,'dependencies':[{'uri':'$source','sha256':hashlib.sha256(rb).hexdigest()},{'uri':recipe['source'],'sha256':hashlib.sha256(sb).hexdigest()}]}
            for ext,content in [('animated.json',sb),('pose.json',rb),('model.json',json.dumps(output).encode()),('stdout',b'published output'),('stderr',b'')]: (self.root/(name+'.'+ext)).write_bytes(content)
        (self.root/'report.json').write_text(json.dumps(report));return report
    def test_capture_report_cannot_forge_backend_pixels_or_controls(self):
        report=self.captures()
        with mock.patch.object(gate.runner,'record'):
            gate.check_gpu(self.root)
            for field,value in [('tolerance',4),('clear',0),('static_oracle_mismatches',1),('controls',[])]:
                bad=copy.deepcopy(report);bad[field]=value;(self.root/'report.json').write_text(json.dumps(bad))
                with self.assertRaises(AssertionError):gate.check_gpu(self.root)
            bad=copy.deepcopy(report);bad['adapter']['backend']='Gl';(self.root/'report.json').write_text(json.dumps(bad))
            with self.assertRaises(AssertionError):gate.check_gpu(self.root)
            (self.root/'report.json').write_text(json.dumps(report));p=self.root/'static-midpoint.rgba';p.write_bytes(bytes(p.stat().st_size))
            with self.assertRaises(AssertionError):gate.check_gpu(self.root)
    def test_cooked_provenance_and_ppm_tampering_rejected(self):
        self.captures()
        p=self.root/'midpoint.model.json';saved=p.read_bytes();doc=json.loads(saved);doc['dependencies'][0]['sha256']='0'*64;p.write_text(json.dumps(doc))
        with self.assertRaises(AssertionError):gate.check_gpu(self.root)
        p.write_bytes(saved);p=self.root/'static-midpoint.ppm';p.write_bytes(b'wrong')
        with self.assertRaises(AssertionError):gate.check_gpu(self.root)

if __name__=='__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(GateFixtures)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.skipped or result.testsRun!=gate.FIXTURE_COUNT:raise SystemExit(1)
    print(f'ANIMATION_POSE_BAKE_GATE_FIXTURES: {result.testsRun} passed; 0 failed; 0 skipped')
