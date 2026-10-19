#!/usr/bin/env python3
"""Independent checker negatives: lifecycle, coverage, source and pixel admission."""
import contextlib
import copy
import hashlib
import json
import os
import pathlib
import re
import struct
import tempfile
import unittest
from unittest import mock
import zlib

import static_model_recolor_gate as gate


class GateFixtures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.saved = (gate.source_root, gate.frozen, gate.mode, gate.runner.report, gate.runner.evidence)
        gate.source_root = self.root
        gate.frozen = None
        gate.runner.report = {}
        gate.runner.evidence = self.root

    def tearDown(self):
        gate.source_root, gate.frozen, gate.mode, gate.runner.report, gate.runner.evidence = self.saved
        self.temp.cleanup()

    def test_source_bytes_modes_and_inventory_fenced(self):
        path = self.root / 'source.rs'; path.write_text('first')
        gate.frozen = gate.snapshot(); gate.source_fence()
        path.write_text('other')
        with self.assertRaises(AssertionError): gate.source_fence()
        path.write_text('first'); gate.source_fence()
        path.chmod(0o700)
        with self.assertRaises(AssertionError): gate.source_fence()

    def test_untracked_ignored_and_symlink_source_rejected(self):
        (self.root / 'source.rs').write_text('source')
        with mock.patch.object(gate.runner, 'record'):
            gate.check_source_inventory(['source.rs'], 'good')
            (self.root / '.ignored').write_text('injected')
            with self.assertRaises(AssertionError): gate.check_source_inventory(['source.rs'], 'bad')
        if os.name != 'nt':
            (self.root / 'alias').symlink_to(self.root / 'source.rs')
            with self.assertRaises(AssertionError): gate.snapshot()

    def test_exact_feature_inventory_required(self):
        required = set(gate.FEATURE_PATHS + gate.INFRA_PATHS)
        for package in ('orr_model', 'orr_model_bindings', 'orr_render', 'orr_editor'):
            required.update([f'crates/{package}/Cargo.toml', f'crates/{package}/src/lib.rs'])
        gate.require_sources(required)
        for path in gate.FEATURE_PATHS:
            with self.assertRaises(AssertionError): gate.require_sources(required - {path})

    def test_each_lane_cannot_claim_other_lanes(self):
        for phase, passed in [('cpu', {'model','bindings'}),('gpu', {'gpu'}),('ui', {'ui'})]:
            gate.mode = phase
            correct = {name: 'PASSED' if name in passed else 'NOT RUN' for name in ('model','bindings','gpu','ui')}
            gate.runner.report['coverage'] = correct.copy(); gate.check_coverage()
            for key in correct:
                gate.runner.report['coverage'] = correct.copy()
                gate.runner.report['coverage'][key] = 'NOT RUN' if key in passed else 'PASSED'
                with self.assertRaises(AssertionError): gate.check_coverage()

    def test_exact_rust_census(self):
        root = pathlib.Path(__file__).resolve().parents[1]
        source = (root / 'crates/orr_model/tests/static_model_recolor.rs').read_text()
        declared = set(re.findall(r'#\[test\]\s*fn ([a-z0-9_]+)', source))
        platform_only = {'cli_windows_nonlocal_prefixes_reject', 'cli_symlink_inputs_ancestors_and_destinations_reject', 'cli_non_utf8_option_names_and_logical_ids_reject', 'cli_non_utf8_filesystem_inputs_work'}
        common = declared - platform_only
        for platform, extra in [('linux', platform_only - {'cli_windows_nonlocal_prefixes_reject'}), ('darwin', {'cli_symlink_inputs_ancestors_and_destinations_reject', 'cli_non_utf8_option_names_and_logical_ids_reject'}), ('win32', {'cli_windows_nonlocal_prefixes_reject'})]:
            names = gate.model_tests(platform)
            self.assertEqual(len(names), len(set(names)))
            self.assertEqual(set(names), common | extra)
        for package, target, name in [('orr_model_bindings','static_model_recolor',gate.BINDINGS_TEST), ('orr_render','static_model_recolor',gate.GPU_TEST), ('orr_editor','static_model_recolor_package_preview_ui',gate.UI_TEST)]:
            text = (root / f'crates/{package}/tests/{target}.rs').read_text()
            self.assertEqual(re.findall(r'#\[ignore[^\n]*\]\s*fn ([a-z0-9_]+)', text), [name])

    def test_inherited_bytecode_prevention_required(self):
        with mock.patch.object(gate.runner, 'record'), mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(AssertionError): gate.require_inherited_bytecode_prevention()
            os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
            gate.require_inherited_bytecode_prevention()

    def test_reference_hash_size_name_and_inventory(self):
        data = b'independent test bytes'; path = self.root / 'proof'; path.write_bytes(data)
        value = 0xcbf29ce484222325
        for byte in data: value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
        entry = {'file':'proof','bytes':len(data),'fnv1a64':f'{value:016x}'}
        gate.capture_reference(self.root, entry)
        gate.capture_files(self.root, {'proof'})
        for field, invalid in [('file','../proof'),('bytes',0),('fnv1a64','0'*16)]:
            bad = entry.copy(); bad[field] = invalid
            with self.assertRaises(AssertionError): gate.capture_reference(self.root,bad)
        with self.assertRaises(AssertionError): gate.capture_files(self.root, {'proof','missing'})

    def test_png_decoder_rejects_crc_truncation_dimensions_and_extra_data(self):
        def chunk(kind,data): return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
        pixels = bytes([13,37,89,255])*4
        raw = b'\0'+pixels[:8]+b'\0'+pixels[8:]
        data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',2,2,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
        path = self.root / 'frame.png'; path.write_bytes(data)
        self.assertEqual(gate.png_pixels(path,2,2),pixels)
        with self.assertRaises(AssertionError): gate.png_pixels(path,3,2)
        for bad in [data[:-1],data+b'x',data[:40]+bytes([data[40]^1])+data[41:]]:
            path.write_bytes(bad)
            with self.assertRaises(AssertionError): gate.png_pixels(path,2,2)

    def test_ui_empty_receipt_and_missing_pixels_rejected(self):
        (self.root/'receipt.json').write_text('{}')
        with self.assertRaises((AssertionError,KeyError)): gate.check_ui(self.root)
        (self.root/'receipt.json').write_text(json.dumps({'test':gate.UI_TEST,'adapter':'hardware'}))
        with self.assertRaises(AssertionError): gate.check_ui(self.root)

    def test_workflow_all_lanes_and_failure_aggregate(self):
        source = (pathlib.Path(__file__).resolve().parents[1]/'.github/workflows/static-model-recolor.yml').read_text()
        for row in ['{mode: cpu, os: ubuntu-22.04}', '{mode: cpu, os: windows-latest}', '{mode: gpu, os: ubuntu-22.04}', '{mode: ui, os: ubuntu-22.04}']:
            self.assertIn(row,source)
        self.assertIn('needs: acceptance',source)
        self.assertIn('if: always()',source)
        self.assertIn('test "$ACCEPTANCE" = success',source)
        self.assertNotIn('continue-on-error',source)
        self.assertNotIn('paths-ignore:',source)


# Add these standard-library helpers and mixin to the independent gate fixtures.
# The constructor never reads production Rust, real captures, or gate oracle helpers.
def recolor_gpu_fixture(root):
    import math
    root.mkdir(exist_ok=True)
    checker = [[225,83,41,0],[47,198,112,63],[108,56,237,149],[173,219,91,231]]
    factors = [[0,.4,.8],[.65,.22,.48],[.18,.72,.34]]
    points = [(-1,-1),(1,-1),(1,.4),(.4,1),(-1,1),(1.1,-.75),(1.8,-.75),(1.4,.4)]
    triangles = [[0,1,2],[0,2,3],[0,3,4],[5,6,7]]
    def write(name, value):
        data = value if isinstance(value, bytes) else json.dumps(value).encode()
        (root/name).write_bytes(data)
        return recolor_ref(root/name)
    material = {'base_color':[0,.4,.8,.25], 'image':0, 'linear_filter':False,'wrap_s':'mirror','wrap_t':'repeat'}
    source = {'format':'orr_static_model','version':1,'asset_id':'scene/source.glb',
              'dependencies':[{'uri':'$source','sha256':'f'*64}], 'primitives':[],
              'materials':[material,dict(material,base_color=[.5,.9,.3,.75]),dict(material,image=2)],
              'images':[{'width':2,'height':2,'rgba8':sum(checker,[])},
                        {'width':1,'height':1,'rgba8':[1,2,3,4]}, {'width':1,'height':1,'rgba8':[17,31,47,63]}]}
    for node, sign, cy in [(1,-1,1.25),(7,1,-1.25)]:
        for slot in [0,1]:
            indices = list(range(5)) if slot == 0 else [5,6,7]
            vertices = [{'position':[*points[i],0],'normal':[0,0,1],
                         'uv':[(points[i][0]+1)/2,(1-points[i][1])/2] if slot == 0 else [.25,.25]} for i in indices]
            source['primitives'].append({'id':f'scene/source.glb#node={node}/mesh=0/primitive={slot}',
                'material':slot,'vertices':vertices,'indices':[0,1,2,0,2,3,0,3,4] if slot == 0 else [0,1,2],
                'transform':[[.8*sign,0,0,0],[0,.65,0,0],[0,0,1,0],[0,cy,.1,1]]})
    assets = {'original_glb':write('source.glb',b'independent fixture GLB'),
              'source':write('source.model.json',source),'variants':[],
              'source_hidden_after_install':True,'cooked_reload_before_render':True,'installed_files':2}
    for name, factor in zip(['rose','teal'],factors[1:]):
        recipe = {'format':'orr_static_material_color','version':1,'source':'inputs/prop.model.json','material':0,'linear_rgb':factor}
        recipe_ref = write(name+'.recolor.json',recipe)
        cooked = copy.deepcopy(source); cooked['asset_id'] = 'variants/'+name+'.recolor.json'
        for primitive in cooked['primitives']:
            primitive['id'] = cooked['asset_id']+primitive['id'][len(source['asset_id']):]
        cooked['materials'][0]['base_color'] = factor+[.25]
        cooked['dependencies'] = [{'uri':'$source','sha256':hashlib.sha256((root/(name+'.recolor.json')).read_bytes()).hexdigest()},
                                  {'uri':recipe['source'],'sha256':hashlib.sha256((root/'source.model.json').read_bytes()).hexdigest()}]
        assets['variants'].append({'asset_id':cooked['asset_id'],'recipe':recipe_ref,'cooked':write(name+'.model.json',cooked)})
    def shade(texel,factor):
        result = []
        for channel,multiplier in zip(texel,factor):
            v = channel/255
            v = (v/12.92 if v<=.04045 else ((v+.055)/1.055)**2.4)*multiplier*.6
            v = v*12.92 if v<=.0031308 else 1.055*v**(1/2.4)-.055
            result.append(int(v*255+.5))
        return bytes(result+[255])
    colors = [[shade(texel,factor) for texel in checker] for factor in factors]
    marker = shade(checker[0],[.5,.9,.3])
    def barycentric(p,triangle):
        a,b,c = [points[i] for i in triangle]
        def cross(v,w): return v[0]*w[1]-v[1]*w[0]
        def sub(v,w): return v[0]-w[0],v[1]-w[1]
        area = cross(sub(b,a),sub(c,a))
        u = cross(sub(b,p),sub(c,p))/area
        v = cross(sub(c,p),sub(a,p))/area
        return u,v,1-u-v
    frames = {}; coverage = {}; raw_frames = {}
    configs = {'source':[(0,0)],'rose':[(0,1)],'teal':[(0,2)],'variants_together':[(-2,1),(2,2)]}
    for name,placements in configs.items():
        raw = bytearray(b'\0\0\0\xff'*(256*256))
        selected = [[0,0] for _ in placements]; markers = [[0,0] for _ in placements]; quadrants=[0]*4; clear=0
        for y in range(256):
            for x in range(256):
                world = ((x+.5-128)/32,(128-y-.5)/32); near=False; matched=False
                for model,(cx,mode) in enumerate(placements):
                    for node,(sign,cy) in enumerate([(-1,1.25),(1,-1.25)]):
                        p=((world[0]-cx)/(.8*sign),(world[1]-cy)/.65)
                        for ti,triangle in enumerate(triangles):
                            weights=barycentric(p,triangle); near |= min(weights)>=-.08
                            if min(weights)<=.08: continue
                            if ti==3: color=marker; markers[model][node]+=1
                            else:
                                texel=(p[0]+1,1-p[1])
                                if any(not .02<=v%1<=.98 for v in texel): continue
                                slot=int(texel[1])*2+int(texel[0]); color=colors[mode][slot]
                                quadrants[slot]+=1; selected[model][node]+=1
                            raw[(y*256+x)*4:(y*256+x)*4+4]=color; matched=True
                clear += not matched and not near
        raw_frames[name]=bytes(raw)
        coverage[name]={'selected_pixels_per_model_node':selected,'unaffected_pixels_per_model_node':markers,
                        'source_checker_pixels':quadrants,'clear_checked':clear}
    raw_frames['variants_reversed']=raw_frames['variants_together']
    controls = ['unchanged_factor','old_factor_multiplied','srgb_input','wrong_material','only_first_user',
                'all_material_tint','swapped_channels','wrong_uv','wrong_transform','offscreen']
    # Deliberately synthetic control frames establish checker difference requirements;
    # the actual mandatory Rust lane, separately, renders each physical counterfactual.
    for name in controls: raw_frames[name]=b'\0\0\0\xff'*(256*256)
    for name,data in raw_frames.items():
        filename=name.replace('_','-') if name.startswith('variants_') else name
        frames[name]={'raw':write(filename+'.rgba',data),'image':write(filename+'.ppm',b'P6\n256 256\n255\n'+bytes(v for i,v in enumerate(data) if i%4!=3))}
    adapter={'software':True,'backend':'Vulkan','name':'independent synthetic fixture'}
    write('adapter.json',adapter)
    receipt={'schema':1,'status':'passed','width':256,'height':256,'adapter':adapter,'assets':assets,'frames':frames,'coverage':coverage,
             'shared_texture_materials':True,'shared_material_users_checked':True,'source_images_byte_identical':True,
             'source_factor_alpha_bits_preserved_cpu':True,'distinct_recipe_namespaces':True,'reversed_batch_order_byte_identical':True,
             'unaffected_material_pixels_checked':sum(coverage['source']['unaffected_pixels_per_model_node'][0]),
             'tolerances':{'color_bytes':3,'minimum_control_differences':500,'minimum_unaffected_differences':100},
             'control_pixel_differences':{name:recolor_difference(raw_frames[name],raw_frames['rose']) for name in controls}}
    write('receipt.json',receipt)
    return receipt


def recolor_ref(path):
    data=path.read_bytes(); value=0xcbf29ce484222325
    for byte in data: value=((value^byte)*0x100000001b3)&0xffffffffffffffff
    return {'file':path.name,'bytes':len(data),'fnv1a64':f'{value:016x}'}


def recolor_difference(a,b):
    return sum(any(abs(a[i+c]-b[i+c])>3 for c in range(4)) for i in range(0,len(a),4))


class RecolorGpuFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture_dir=tempfile.TemporaryDirectory()
        cls.fixture_root=pathlib.Path(cls.fixture_dir.name)/'fixture'
        cls.receipt=recolor_gpu_fixture(cls.fixture_root)
        cls.files={path.name:path.read_bytes() for path in cls.fixture_root.iterdir()}

    @classmethod
    def tearDownClass(cls): cls.fixture_dir.cleanup()

    def test_gpu_synthetic_complete_capture_passes(self):
        with mock.patch.object(gate.runner,'report',{}): gate.check_gpu(self.fixture_root)

    def test_gpu_rejects_coherently_rehashed_pixel_mutations(self):
        for name in ['rose','source','variants_together','unchanged_factor']:
            with self.subTest(frame=name), tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory)
                for filename,data in self.files.items(): (root/filename).write_bytes(data)
                receipt=copy.deepcopy(self.receipt)
                frame=receipt['frames'][name]
                raw=bytearray((root/frame['raw']['file']).read_bytes())
                # For the control, replace it with the exact right answer and
                # recompute its claimed difference. For positive frames, alter
                # every colored interior and recompute all hashes and counts.
                if name=='unchanged_factor': raw=bytearray((root/'rose.rgba').read_bytes())
                else:
                    for i in range(0,len(raw),4):
                        if raw[i:i+3]!=b'\0\0\0': raw[i:i+3]=b'\xff\xff\xff'
                (root/frame['raw']['file']).write_bytes(raw)
                (root/frame['image']['file']).write_bytes(b'P6\n256 256\n255\n'+bytes(v for i,v in enumerate(raw) if i%4!=3))
                for field in ['raw','image']: frame[field]=recolor_ref(root/frame[field]['file'])
                if name=='variants_together':
                    reverse=receipt['frames']['variants_reversed']
                    for field in ['raw','image']:
                        (root/reverse[field]['file']).write_bytes((root/frame[field]['file']).read_bytes())
                        reverse[field]=recolor_ref(root/reverse[field]['file'])
                rose=(root/'rose.rgba').read_bytes()
                for control in receipt['control_pixel_differences']:
                    receipt['control_pixel_differences'][control]=recolor_difference((root/(control+'.rgba')).read_bytes(),rose)
                (root/'receipt.json').write_text(json.dumps(receipt))
                with self.assertRaises(AssertionError), mock.patch.object(gate.runner,'report',{}): gate.check_gpu(root)

    def test_gpu_rejects_coherently_rehashed_asset_mutations(self):
        for kind in ['factor','alpha','image','unselected','geometry','dependency']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory)
                for filename,data in self.files.items(): (root/filename).write_bytes(data)
                receipt=copy.deepcopy(self.receipt); cooked=json.loads((root/'rose.model.json').read_text())
                if kind=='factor': cooked['materials'][0]['base_color'][0]=0
                elif kind=='alpha': cooked['materials'][0]['base_color'][3]=.5
                elif kind=='image': cooked['images'][0]['rgba8'][3]=255
                elif kind=='unselected': cooked['materials'][1]['base_color'][0]=.65
                elif kind=='geometry': cooked['primitives'][2]['vertices'][0]['uv'][0]=.8
                else: cooked['dependencies'].reverse()
                (root/'rose.model.json').write_text(json.dumps(cooked))
                receipt['assets']['variants'][0]['cooked']=recolor_ref(root/'rose.model.json')
                (root/'receipt.json').write_text(json.dumps(receipt))
                with self.assertRaises(AssertionError), mock.patch.object(gate.runner,'report',{}): gate.check_gpu(root)



def recolor_ui_fixture(root):
    root.mkdir(exist_ok=True)
    receipt = {'test':gate.UI_TEST,'adapter':'llvmpipe Vulkan', 'different_asset_pixels':1600,
               'red_blue_variant_pixels':1600,'red_blue_channel_reversal_pixels':1600,
               'orbit_changed_pixels':1600,'zoom_changed_pixels':1600,
               'real_cli_logs':['cli-blue.stdout.log','cli-blue.stderr.log','cli-red.stdout.log','cli-red.stderr.log']}
    for key in ('authoring_and_package_source_hidden', 'geometry_uv_images_alpha_unselected_materials_preserved', 'removed_cached_asset_refused', 'default_transform_and_full_binding_identity_reopened', 'reset_exact', 'preview_without_selection', 'preview_preserves_project_source_files_frame_history_bindings_and_editor_camera', 'corrupt_cached_asset_refused', 'use_model_save_reopen_preserved'):
        receipt[key] = True
    for name in receipt['real_cli_logs']: (root/name).write_text('fixture')
    def image(name, color):
        def chunk(kind,data): return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
        data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',40,40,8,6,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+bytes(color)*40)*40))+chunk(b'IEND',b'')
        (root/(name+'.png')).write_bytes(data)
    for name,color in [('02-original',[0,100,0,255]),('03-blue',[0,0,200,255]),('03-red',[200,0,0,255]),('04-orbit',[0,0,100,255]),('05-zoom',[0,0,50,255]),('06-reset',[0,0,200,255])]:
        image(name+'-canvas',color)
        image(name,color)
    hidden=root/'hidden-package-source';hidden.mkdir()
    authoring=root/'hidden-authoring';(authoring/'inputs').mkdir(parents=True)
    original={'asset_id':'source.glb','materials':[{'base_color':[.1,.7,.2,.25]}], 'primitives':[{'id':'source.glb#node=0/mesh=0/primitive=0'}], 'images':[], 'dependencies':[]}
    data=json.dumps(original).encode()
    (hidden/'original.orrmodel').write_bytes(data);(authoring/'inputs/original.model.json').write_bytes(data)
    for name,color in [('blue',[.05,.15,1]),('red',[1,.05,.1])]:
        recipe={'format':'orr_static_material_color','version':1,'source':'inputs/original.model.json','material':0,'linear_rgb':color}
        raw=json.dumps(recipe).encode();(authoring/(name+'.json')).write_bytes(raw)
        variant=copy.deepcopy(original);identity='variants/'+name+'.recolor.json';variant['asset_id']=identity
        variant['primitives'][0]['id']=identity+'#node=0/mesh=0/primitive=0'
        variant['materials'][0]['base_color'][:3]=color
        variant['dependencies']=[{'uri':'$source','sha256':hashlib.sha256(raw).hexdigest()},{'uri':recipe['source'],'sha256':hashlib.sha256(data).hexdigest()}]
        (hidden/(name+'.orrmodel')).write_text(json.dumps(variant))
    room=root/'room';room.mkdir();(room/'room.models.json').write_text(json.dumps({'bindings':{'e_1':{'asset':'blue.orrmodel'}}}))
    (root/'receipt.json').write_text(json.dumps(receipt))
    return receipt


class RecolorUiFixtureTests(unittest.TestCase):
    def test_ui_synthetic_pixels_and_cooked_assets_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);recolor_ui_fixture(root)
            with mock.patch.object(gate.runner,'record'), mock.patch.object(gate.runner,'report',{}): gate.check_ui(root)

    def test_ui_rejects_rehashed_reset_and_asset_mutations(self):
        for mutation in ('reset','factor','provenance','identity','sidecar','count'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);receipt=recolor_ui_fixture(root)
                if mutation=='reset': (root/'06-reset-canvas.png').write_bytes((root/'03-red-canvas.png').read_bytes())
                elif mutation=='sidecar': (root/'room/room.models.json').write_text('{"bindings":{}}')
                elif mutation=='count':
                    receipt['red_blue_channel_reversal_pixels']=1599
                    (root/'receipt.json').write_text(json.dumps(receipt))
                else:
                    path=root/'hidden-package-source/blue.orrmodel';asset=json.loads(path.read_text())
                    if mutation=='factor':asset['materials'][0]['base_color'][0]=.2
                    elif mutation=='provenance':asset['dependencies'].reverse()
                    else:asset['asset_id']='wrong.recolor.json'
                    path.write_text(json.dumps(asset))
                with self.assertRaises(AssertionError), mock.patch.object(gate.runner,'record'), mock.patch.object(gate.runner,'report',{}): gate.check_ui(root)


if __name__ == '__main__':
    suite = unittest.TestSuite([unittest.defaultTestLoader.loadTestsFromTestCase(GateFixtures), unittest.defaultTestLoader.loadTestsFromTestCase(RecolorGpuFixtureTests), unittest.defaultTestLoader.loadTestsFromTestCase(RecolorUiFixtureTests)])
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    assert result.testsRun == gate.FIXTURE_COUNT and not result.skipped
    if not result.wasSuccessful(): raise SystemExit(1)
    print(f'STATIC_RECOLOR_GATE_FIXTURES: {result.testsRun} passed; 0 failed; 0 skipped')
