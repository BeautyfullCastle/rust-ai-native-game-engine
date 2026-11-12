#!/usr/bin/env python3
"""Cone extension mutation fixtures only. These never certify a GPU execution."""
import ast
import copy
import hashlib
import inspect
import json
import math
import os
import pathlib
import re
import sys
import tempfile
import types
import unittest
from unittest import mock
import static_model_cylinder_gate as gate
import static_model_cone_gate as cone

ROOT=pathlib.Path(__file__).resolve().parents[1]


def fixture_model(n=8,r=.55,h=2.):
    recipe={'format':'orr_static_model_cone','version':1,'radius':r,'height':h,'segments':n}
    identity=[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]]
    model={'format':'orr_static_model','version':1,'asset_id':'props/key.cone.json',
           'dependencies':[{'uri':'$source','sha256':'a'*64}], 'primitives':[],
           'materials':[{'base_color':[1.]*4,'image':0,'linear_filter':False,'wrap_s':'clamp','wrap_t':'clamp'} for _ in range(2)],
           'images':[{'width':1,'height':1,'rgba8':[255]*4}]}
    ring=[]
    for i in range(n):
        angle=math.tau*i/n
        ring.append([gate.f32(gate.f32(r)*gate.f32(math.cos(angle))),-h/2,gate.f32(gate.f32(r)*gate.f32(math.sin(angle)))])
    side=[]
    for i in range(n):
        mid=math.tau*(i+.5)/n
        length=math.hypot(h,r*math.cos(math.pi/n))
        normal=[h*math.cos(mid)/length,r*math.cos(math.pi/n)/length,h*math.sin(mid)/length]
        for point,uv in zip((ring[i],[0.,h/2,0.],ring[(i+1)%n]),((i/n,1.),((i+.5)/n,0.),((i+1)/n,1.))):
            side.append({'position':point,'normal':normal,'uv':list(uv)})
    bottom=[{'position':[0.,-h/2,0.],'normal':[0.,-1.,0.],'uv':[.5,.5]}]
    bottom += [{'position':point,'normal':[0.,-1.,0.],'uv':[.5+point[0]/(2*r),.5+point[2]/(2*r)]} for point in ring]
    for slot,(vertices,indices) in enumerate(((side,list(range(3*n))),(bottom,[v for i in range(n) for v in (0,i+1,(i+1)%n+1)]))):
        model['primitives'].append({'id':f'props/key.cone.json#node=0/mesh=0/primitive={slot}',
                                   'vertices':vertices,'indices':indices,'material':slot,'transform':identity})
    # The authoring contract has exact cardinal zeros. Mathematical fixtures
    # normalize libm axis residue without copying producer ring constants.
    for primitive in model['primitives']:
        for vertex in primitive['vertices']:
            vertex['position']=[0. if abs(v)<1e-14 else v for v in vertex['position']]
    # Contract normals are cross products of the stored binary32 positions.
    for i in range(n):
        vertices=model['primitives'][0]['vertices'][i*3:i*3+3]
        a,b,c=([gate.f32(v) for v in vertex['position']] for vertex in vertices)
        u,v=[b[k]-a[k] for k in range(3)],[c[k]-a[k] for k in range(3)]
        normal=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]
        length=math.sqrt(sum(x*x for x in normal))
        for vertex in vertices:vertex['normal']=[x/length for x in normal]
    return model,recipe


def producer_context(root):
    source=root/'source';evidence=root/'evidence';target=root/'target'
    evidence.mkdir();source.mkdir();target.mkdir()
    source_file=source/'crates/orr_editor/tests/static_model_cone_package_preview_ui.rs'
    source_file.parent.mkdir(parents=True);source_file.write_bytes(b'current source fixture only')
    report={'producers':{},'source_inventory_sha256':'b'*64}
    for label,package,name,kind,features in [
        ('cone-cli','orr_model',cone.CLI_TARGET,'example',['import']),
        ('cylinder-cli','orr_model','static_model_cylinder','example',['import']),
        ('cone-editor-test','orr_editor',cone.UI_TARGET,'test',['models','package-manager','room-project'])]:
        crate=source/'crates'/package
        retained=evidence/(label+('.exe' if os.name=='nt' else ''));retained.write_bytes(('fixture '+label).encode())
        original=target/'release'/('examples' if kind=='example' else 'deps')/name
        metadata={'reason':'compiler-artifact','fresh':False,'manifest_path':str(crate/'Cargo.toml'),
                  'package_id':'path+'+crate.as_uri()+'#0.0.1','executable':str(original),
                  'target':{'name':name,'kind':[kind],'crate_types':['bin'],'src_path':str(crate/('examples' if kind=='example' else 'tests')/(name+'.rs'))},
                  'profile':{'test':kind=='test','opt_level':'3'},'features':features}
        report['producers'][label]={'producer':metadata,'file':retained.name,'bytes':retained.stat().st_size,
            'sha256':gate.runner.digest(retained),'source_inventory_sha256':report['source_inventory_sha256'],'source_root':str(source)}
    runner=types.SimpleNamespace(evidence=evidence,target_dir=target,report=report,digest=gate.runner.digest,record=lambda:None)
    context=types.SimpleNamespace(source_root=source,runner=runner,regular=gate.regular,fnv1a64=gate.fnv1a64,
                                 UI_FIELDS=gate.UI_FIELDS,UI_TRUE_FIELDS=gate.UI_TRUE_FIELDS,UI_PNGS=gate.UI_PNGS,GPU_SPECS=gate.GPU_SPECS,
                                 capture_files=gate.capture_files,strict_json=gate.strict_json,f32=gate.f32)
    receipt={key:True for key in gate.UI_TRUE_FIELDS}
    receipt.update(dict.fromkeys(gate.UI_FIELDS,0));receipt.update({key:True for key in gate.UI_TRUE_FIELDS})
    receipt.update(schema=1,status='passed',test=cone.UI_TEST,mandatory_test_count=1,
        test_source_fnv1a64=gate.fnv1a64(source_file.read_bytes()),
        producer_fnv1a64=gate.fnv1a64(cone.retained_producer(context,'cone-cli').read_bytes()),
        cylinder_control_producer_fnv1a64=gate.fnv1a64(cone.retained_producer(context,'cylinder-cli').read_bytes()),
        cone_triangle_oracle=True,native_gpu=False,native_os_input=False,adapter='llvmpipe fixture only',
        renderer_options='default',predictable_texture_filtering=False,assigned_role='KEY',assigned_guid='e_00000002',
        normalized_viewport_size=[900,850],key_placements_before_collect=2,key_placements_after_collect=1,
        key_placements_after_restart=2,key_collected=1,restart_key_collected=0,recorded_ticks=4,
        recorded_checksums_checked=1,authoritative_checksum='a'*16,
        tall_silhouette_pixels_checked=140000,wide_silhouette_pixels_checked=140000,cylinder_negative_control_mismatches=5000)
    return context,receipt


def scalar_canvas(radius, height, segments, cylinder=False):
    """Synthetic checker-only image from an independent projected convex hull.

    This is not a renderer execution or a GPU acceptance artifact.
    """
    scale=336/(2*math.sqrt(2*radius*radius+height*height/4)*1.2)
    def project(p):
        x,y,z=p
        return (224+(math.cos(.45)*x-math.sin(.45)*z)*scale,
                168-(-math.sin(.45)*math.sin(.25)*x+math.cos(.25)*y-math.cos(.45)*math.sin(.25)*z)*scale)
    points=[project((radius*math.cos(math.tau*i/segments),-height/2,radius*math.sin(math.tau*i/segments))) for i in range(segments)]
    if cylinder:
        points += [project((radius*math.cos(math.tau*i/segments),height/2,radius*math.sin(math.tau*i/segments))) for i in range(segments)]
    else:points.append(project((0,height/2,0)))
    def cross(o,a,b):return (a[0]-o[0])*(b[1]-o[1])-(a[1]-o[1])*(b[0]-o[0])
    def chain(items):
        hull=[]
        for p in items:
            while len(hull)>=2 and cross(hull[-2],hull[-1],p)<=0:hull.pop()
            hull.append(p)
        return hull
    points=sorted(set(points));hull=chain(points)[:-1]+chain(reversed(points))[:-1]
    pixels=bytearray()
    for y in range(336):
        for x in range(448):
            inside=all(cross(hull[i],hull[(i+1)%len(hull)],(x+.5,y+.5))>=0 for i in range(len(hull)))
            pixels.extend((230,230,230,255) if inside else (20,20,20,255))
    return bytes(pixels)


def complete_ui_fixture(root):
    # Complete acceptance-checker fixture only. Never use it as hosted evidence.
    from test_static_model_cylinder_gate import encode_png, fixture_model as cylinder_model
    g,receipt=producer_context(root)
    for name in ('GPU_SPECS','UI_PNGS','png_pixels','difference','validate_installed','check_cylinder'):
        setattr(g,name,getattr(gate,name))
    captures=root/'captures';captures.mkdir()
    hidden=captures/'hidden-package-source';hidden.mkdir()
    authoring=captures/'hidden-authoring';authoring.mkdir()
    room=captures/'room';room.mkdir()
    for name,spec in gate.GPU_SPECS.items():
        radius,height,n=spec
        model,recipe=fixture_model(n,radius,height)
        recipe_path=authoring/(name+'.json');recipe_path.write_text(json.dumps(recipe))
        identity=f'props/{name}.cone.json';model['asset_id']=identity
        model['dependencies'][0]['sha256']=gate.runner.digest(recipe_path)
        for slot,primitive in enumerate(model['primitives']):primitive['id']=f'{identity}#node=0/mesh=0/primitive={slot}'
        raw=json.dumps(model).encode();(hidden/(name+'.orrmodel')).write_bytes(raw);(authoring/(name+'.orrmodel')).write_bytes(raw)
    model,recipe=cylinder_model(32,.55,2.)
    recipe_path=authoring/'cylinder-control.json';recipe_path.write_text(json.dumps(recipe))
    identity='props/cylinder-control.cylinder.json';model['asset_id']=identity;model['dependencies'][0]['sha256']=gate.runner.digest(recipe_path)
    for slot,primitive in enumerate(model['primitives']):primitive['id']=f'{identity}#node=0/mesh=0/primitive={slot}'
    raw=json.dumps(model).encode();(hidden/'cylinder-control.orrmodel').write_bytes(raw);(authoring/'cylinder-control.model.json').write_bytes(raw)
    (hidden/'original.orrmodel').write_bytes(b'fixture original binding bytes')
    package_digest='d'*64
    installed=room/'.orr/packages/objects'/package_digest;installed.mkdir(parents=True)
    entries={}
    for file in hidden.iterdir():
        (installed/file.name).write_bytes(file.read_bytes());entries[file.name]=gate.runner.digest(file)
    (room/'orr.packages.lock.json').write_text(json.dumps({'packages':{'static-cone-preview':{'digest':package_digest,'files':entries}}}))
    binding={'kind':'static','package':'static-cone-preview','asset':'tall.orrmodel','package_digest':package_digest,
             'source_hash':entries['tall.orrmodel'],'transform':{'translation':[0.,0.,0.],'rotation':[0.,0.,0.,1.],'scale':[1.,1.,1.]}}
    (room/'room.models.json').write_text(json.dumps({'version':2,'scene':'room.scene.yaml','project':'.','bindings':{receipt['assigned_guid']:binding,'e_00000001':binding}}))
    (room/'room.scene.yaml').write_text(f"  {receipt['assigned_guid']}:\n    RoomEscapeV1::Actor: {{ kind: 2, ordinal: 0 }}\n")
    (room/'orr.project.json').write_text(json.dumps({'entry':{'game':'room-escape-v1','models':'room.models.json'}}))
    receipt.update(assigned_binding=binding,installed_package_digest=package_digest,variant_source_hash=entries['tall.orrmodel'],variant_asset_id='props/tall.cone.json')
    for name in ('tall','wide','cylinder-control'):
        for stream in ('stdout','stderr'):(captures/f'cli-{name}.{stream}.log').write_bytes(b'')
    tall=scalar_canvas(.55,2.,32);wide=scalar_canvas(1.,.65,16);control=scalar_canvas(.55,2.,32,True)
    original=bytes([100,80,60,255])*(448*336);orbit=bytes([90,100,110,255])*(448*336);zoom=bytes([120,100,90,255])*(448*336)
    canvas={'02-original-canvas':original,'03-tall-canvas':tall,'03-cylinder-negative-control':control,'04-wide-canvas':wide,
            '05-orbit-canvas':orbit,'06-zoom-canvas':zoom,'07-reset-canvas':tall}
    viewport=bytes([1,2,3,255])+bytes([90,70,50,255])*(900*850-1)
    collected=bytes([1,2,3,255])+bytes([50,70,90,255])*(900*850-1)
    frames={}
    for name in (*gate.UI_PNGS,'03-cylinder-negative-control'):
        if name in canvas:width,height=448,336;pixels=canvas[name];(captures/(name+'.rgba')).write_bytes(pixels)
        elif name.endswith('-viewport'):width,height=900,850;pixels=collected if name.startswith('11-') else viewport
        else:width,height=1,1;pixels=bytes([1,2,3,255])
        (captures/(name+'.png')).write_bytes(encode_png(width,height,pixels));frames[name]=pixels
    for key,first,second in [('original_changed_pixels','02-original-canvas','03-tall-canvas'),('proportions_changed_pixels','03-tall-canvas','04-wide-canvas'),
                            ('orbit_changed_pixels','03-tall-canvas','05-orbit-canvas'),('zoom_changed_pixels','05-orbit-canvas','06-zoom-canvas'),
                            ('collect_changed_pixels','10-reopened-viewport','11-collected-viewport')]:receipt[key]=gate.difference(frames[first],frames[second])
    receipt['tall_silhouette_pixels_checked']=cone.check_silhouette(tall,(.55,2.,32))[0]
    receipt['wide_silhouette_pixels_checked']=cone.check_silhouette(wide,(1.,.65,16))[0]
    receipt['cylinder_negative_control_mismatches']=cone.check_silhouette(control,(.55,2.,32))[1]
    (captures/'collected-authoritative.frame').write_bytes(b'fixture-frame'*8)
    (captures/'presentation-free.frame').write_bytes(b'fixture-frame'*8)
    (captures/'actual-gui.orrp').write_bytes(b'fixture-replay'*8)
    (captures/'receipt.json').write_text(json.dumps(receipt))
    return g,captures

class Fixtures(unittest.TestCase):
    def test_explicit_source_paths_and_mode_coverage(self):
        self.assertEqual(len(cone.FEATURE_PATHS),7);self.assertEqual(len(gate.FEATURE_PATHS),17)
        self.assertTrue(set(cone.FEATURE_PATHS)<=set(gate.FEATURE_PATHS))
        for phase,passed in [('cpu',{'model','bindings','cone_model'}),('gpu',{'gpu'}),('ui',{'ui','cone_ui'})]:
            keys=('model','bindings','gpu','ui','cone_model','cone_ui')
            good={key:'PASSED' if key in passed else 'NOT RUN' for key in keys}
            with mock.patch.object(gate,'mode',phase),mock.patch.object(gate.runner,'report',{'coverage':good}):
                gate.check_coverage()
                for key in ('cone_model','cone_ui'):
                    old=good[key];good[key]='NOT RUN' if old=='PASSED' else 'PASSED'
                    with self.assertRaises(AssertionError):gate.check_coverage()
                    good[key]=old

    def test_current_source_exact_test_census_and_matrix(self):
        source=(ROOT/'crates/orr_model/tests/static_model_cone_cli.rs').read_text()
        names=set(re.findall(r'#\[test\](?:\n#\[[^\n]+\])*\nfn (\w+)',source))
        expected=set().union(*(gate.model_tests(p) for p in ('linux','darwin','win32')))|{gate.CLI_TEST}
        self.assertEqual(names,expected)
        ui=(ROOT/'crates/orr_editor/tests/static_model_cone_package_preview_ui.rs').read_text()
        self.assertEqual(re.findall(r'#\[test\](?:\n#\[[^\n]+\])*\nfn (\w+)',ui),[cone.UI_TEST])
        self.assertEqual(gate.WORKFLOW_ROWS,(('cpu','ubuntu-22.04'),('cpu','windows-latest'),('gpu','ubuntu-22.04'),('ui','ubuntu-22.04')))

    def test_hooks_keep_cylinder_before_cone_and_producer_fences(self):
        for fn,legacy,hook in [(gate.cpu,'model()', 'cone.cpu(sys.modules[__name__])'),(gate.ui,'check_ui(captures)','cone.ui(sys.modules[__name__])')]:
            source=inspect.getsource(fn);self.assertLess(source.index(legacy),source.index(hook));self.assertEqual(source.count(hook),1)
        self.assertNotIn('cone.',inspect.getsource(gate.gpu))
        for name in ('cpu','ui','build_cli'):
            source=inspect.getsource(getattr(cone,name));ast.parse(source)
            self.assertIn('g.pin(',source);self.assertIn('--message-format=json',source)
            self.assertNotIn('subprocess',source)
        self.assertIn('cone.FIXTURE_COUNT',inspect.getsource(gate.begin))
        self.assertIn('source_fence()',inspect.getsource(gate.pin))

    def test_analytic_geometry_all_segments(self):
        for n in (8,16,32,64,128):
            model,recipe=fixture_model(n)
            cone.validate_cone(gate,model,recipe,'a'*64,'props/key.cone.json')

    def test_geometry_rejects_caps_normals_uv_winding_and_cylinder(self):
        good,recipe=fixture_model()
        mutations=[lambda m:m['primitives'].pop(),lambda m:m['primitives'].append(copy.deepcopy(m['primitives'][1])),
                   lambda m:m['primitives'][0]['vertices'][1]['position'].__setitem__(0,.55),
                   lambda m:m['primitives'][0]['vertices'][0]['normal'].__setitem__(1,0.),
                   lambda m:m['primitives'][0]['vertices'][1]['uv'].__setitem__(0,0.),
                   lambda m:m['primitives'][1]['indices'].__setitem__(1,2)]
        for mutate in mutations:
            model=copy.deepcopy(good);mutate(model)
            with self.assertRaises(AssertionError):cone.validate_cone(gate,model,recipe,'a'*64,'props/key.cone.json')

    def test_recipe_provenance_rejects_stale_and_wrong_identity(self):
        model,recipe=fixture_model()
        for field,value in [('format','orr_static_model_cylinder'),('version',True),('segments',True),('radius',float('nan'))]:
            bad=copy.deepcopy(recipe);bad[field]=value
            with self.assertRaises(AssertionError):cone.validate_cone(gate,model,bad,'a'*64,'props/key.cone.json')
        for digest,identity in [('b'*64,'props/key.cone.json'),('a'*64,'props/other.cone.json')]:
            with self.assertRaises(AssertionError):cone.validate_cone(gate,model,recipe,digest,identity)

    def test_retained_producers_reject_stale_hash_source_and_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            g,_=producer_context(pathlib.Path(tmp).resolve())
            for label in ('cone-cli','cylinder-cli','cone-editor-test'):
                entry=g.runner.report['producers'][label]
                for field,value in [('sha256','f'*64),('source_inventory_sha256','c'*64),('source_root','wrong'),('bytes',0),('file','../escape')]:
                    old=entry[field];entry[field]=value
                    with self.assertRaises(AssertionError):cone.retained_producer(g,label)
                    entry[field]=old
                path=cone.retained_producer(g,label);raw=path.read_bytes();path.unlink()
                with self.assertRaises(AssertionError):cone.retained_producer(g,label)
                path.write_bytes(raw+b'changed')
                with self.assertRaises(AssertionError):cone.retained_producer(g,label)
                path.write_bytes(raw)

    def test_cargo_metadata_rejects_wrong_package_target_kind_features_fresh(self):
        with tempfile.TemporaryDirectory() as tmp:
            g,_=producer_context(pathlib.Path(tmp).resolve());entry=g.runner.report['producers']['cone-cli'];good=copy.deepcopy(entry['producer'])
            mutations=[lambda m:m.update(fresh=True),lambda m:m.update(features=[]),lambda m:m.update(package_id='registry+forged'),
                       lambda m:m.update(manifest_path='/wrong/Cargo.toml'),lambda m:m['target'].update(name='static_model_cylinder'),
                       lambda m:m['target'].update(kind=['bin']),lambda m:m['target'].update(src_path='/wrong.rs'),
                       lambda m:m['profile'].update(test=True),lambda m:m['profile'].update(opt_level='0'),
                       lambda m:m.update(executable='/wrong/release/examples/static_model_cone')]
            for mutate in mutations:
                entry['producer']=copy.deepcopy(good);mutate(entry['producer'])
                with self.assertRaises(AssertionError):cone.retained_producer(g,'cone-cli')

    def test_ui_identity_rejects_missing_forged_and_promoted_receipts(self):
        with tempfile.TemporaryDirectory() as tmp:
            g,good=producer_context(pathlib.Path(tmp).resolve());cone.check_ui_identity(g,good)
            for field in sorted(cone.EXTRA_FIELDS|{'producer_fnv1a64','test_source_fnv1a64'}):
                bad=copy.deepcopy(good);del bad[field]
                with self.assertRaises(AssertionError):cone.check_ui_identity(g,bad)
            for field,value in [('producer_fnv1a64','0'*16),('cylinder_control_producer_fnv1a64','0'*16),('test_source_fnv1a64','0'*16),
                                ('native_gpu',True),('native_os_input',True),('adapter','SwiftShader'),('cone_triangle_oracle',False),
                                ('mandatory_test_count',True),('status','skipped'),('test',gate.UI_TEST),('recorded_checksums_checked',0)]:
                bad=copy.deepcopy(good);bad[field]=value
                with self.assertRaises(AssertionError):cone.check_ui_identity(g,bad)

    def test_cli_capture_requires_exact_inventory_and_provenance(self):
        with tempfile.TemporaryDirectory() as tmp:
            g,_=producer_context(pathlib.Path(tmp).resolve());captures=pathlib.Path(tmp)/'captures';captures.mkdir()
            model,recipe=fixture_model(32);r=captures/'key.json';m=captures/'key.orrmodel';r.write_text(json.dumps(recipe));model['dependencies'][0]['sha256']=gate.runner.digest(r);m.write_text(json.dumps(model))
            cone.check_cli(g,captures)
            self.assertEqual(g.runner.report['cone_cli_receipt']['producer_sha256'],g.runner.report['producers']['cone-cli']['sha256'])
            r.write_text(json.dumps(recipe)+'\n')
            with self.assertRaises(AssertionError):cone.check_cli(g,captures)
            r.write_text(json.dumps(recipe));(captures/'extra').write_bytes(b'x')
            with self.assertRaises(AssertionError):cone.check_cli(g,captures)
            (captures/'extra').unlink();m.unlink()
            with self.assertRaises(AssertionError):cone.check_cli(g,captures)

    def test_halfspace_oracle_rejects_blank_full_and_wrong_size(self):
        # Pure scalar oracle mutation fixtures, never a rendered GPU artifact.
        samples=cone.silhouette_samples(.55,2.,32)
        self.assertGreater(sum(want for _,want in samples),4000)
        blank=bytes([20,20,20,255])*(448*336)
        self.assertGreater(cone.check_silhouette(blank,(.55,2.,32))[1],4000)
        full=bytearray(bytes([240,240,240,255])*(448*336));full[:4]=blank[:4]
        self.assertGreater(cone.check_silhouette(full,(.55,2.,32))[1],4000)
        with self.assertRaises(AssertionError):cone.check_silhouette(blank[:-4],(.55,2.,32))

    def test_ui_requires_complete_positive_then_rejects_mixed_proof(self):
        # This validates the checker itself, not GPU/hosted execution.
        with tempfile.TemporaryDirectory() as tmp:
            g,captures=complete_ui_fixture(pathlib.Path(tmp).resolve())
            cone.check_ui(g,captures)
            receipt_path=captures/'receipt.json';receipt=receipt_path.read_bytes()
            for key,value in [('tall_silhouette_pixels_checked',1),('wide_silhouette_pixels_checked',1),('cylinder_negative_control_mismatches',0)]:
                bad=json.loads(receipt);bad[key]=value;receipt_path.write_text(json.dumps(bad))
                with self.assertRaises(AssertionError):cone.check_ui(g,captures)
                receipt_path.write_bytes(receipt)
            for name in ('03-tall-canvas.png','03-tall-canvas.rgba','03-cylinder-negative-control.png',
                         'presentation-free.frame','hidden-package-source/tall.orrmodel',
                         'room/.orr/packages/objects/'+('d'*64)+'/tall.orrmodel'):
                path=captures/name;raw=path.read_bytes();path.unlink()
                with self.assertRaises(AssertionError):cone.check_ui(g,captures)
                path.write_bytes(raw)
                mixed=bytearray(raw);mixed[len(mixed)//2]^=1;path.write_bytes(mixed)
                with self.assertRaises((AssertionError,UnicodeDecodeError,json.JSONDecodeError)):cone.check_ui(g,captures)
                path.write_bytes(raw)
            # Current valid shape plus a different valid image is still rejected.
            tall=captures/'03-tall-canvas.png';raw=tall.read_bytes()
            tall.write_bytes((captures/'04-wide-canvas.png').read_bytes())
            with self.assertRaises(AssertionError):cone.check_ui(g,captures)
            tall.write_bytes(raw)
            cone.check_ui(g,captures)


if __name__=='__main__':
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(Fixtures)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.testsRun!=cone.FIXTURE_COUNT or result.skipped:
        raise SystemExit(1)
    print(f'STATIC_CONE_GATE_FIXTURES: {cone.FIXTURE_COUNT} passed; 0 failed; 0 skipped')
