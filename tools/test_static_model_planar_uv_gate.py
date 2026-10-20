#!/usr/bin/env python3
"""Focused grouped positive/negative receipt checks, without Cargo or a GPU."""
import copy
import hashlib
import json
import pathlib
import struct
import tempfile
import zlib
import static_model_planar_uv_gate as gate


def reject(action):
    try:
        action()
    except (AssertionError, KeyError, ValueError):
        return
    raise AssertionError('negative fixture admitted')


def png(raw):
    def chunk(kind, data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
    scan=b''.join(b'\0'+raw[y*1024:(y+1)*1024] for y in range(256))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',256,256,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(scan))+chunk(b'IEND',b'')


def gpu_fixture(root):
    source=root/'source'; source.mkdir(); captures=root/'captures'; captures.mkdir()
    for name in gate.PATHS:
        path=source/name; path.parent.mkdir(exist_ok=True,parents=True); path.write_bytes(name.encode())
    binary=root/'producer'; binary.write_bytes(b'fixture-not-a-real-producer')
    def write(name,data):
        (captures/name).write_bytes(data)
        return {'file':name,'bytes':len(data),'fnv1a64':gate.fnv(data)}
    def js(name,value):
        (captures/name).write_text(json.dumps(value))
    provenance={'producer_path':str(binary),'producer_fnv1a64':gate.fnv(binary.read_bytes())}
    for key,index in [('test_source_fnv1a64',3),('oracle_source_fnv1a64',4),('cli_source_fnv1a64',0),('authoring_source_fnv1a64',1)]:
        provenance[key]=gate.fnv((source/gate.PATHS[index]).read_bytes())
    adapter={'software':True,'backend':'Vulkan','name':'llvmpipe fixture'}
    frames={}
    for index,name in enumerate(('source','projected','scene','wrong_axis','wrong_repeat','local_space','affine_collapsed')):
        raw=bytes([1 if name=='scene' else index,17,91,255])*(256*256)
        frames[name]={'rgba':write(name+'.rgba',raw),'png':write(name+'.png',png(raw))}
    recipe=b'fixture recipe'; original=b'fixture source'
    cooked={'asset_id':'variants/tile.planar-uv.json','dependencies':[{'uri':'$source','sha256':hashlib.sha256(recipe).hexdigest()},{'uri':'inputs/source.model.json','sha256':hashlib.sha256(original).hexdigest()}]}
    assets={'imported_glb':write('source.glb',b'glb fixture'),'source':write('source.model.json',original),'recipe':write('recipe.json',recipe),'cooked':write('cooked.model.json',json.dumps(cooked).encode()),'manifest':write('orr.package.json',b'{}'),'asset_id':cooked['asset_id'],'source_path':'inputs/source.model.json'}
    receipt={'schema':1,'status':'passed','width':256,'height':256,'adapter':adapter,'provenance':provenance,'frames':frames,'assets':assets,
             'control_order':['original_allzero','wrong_axis','wrong_repeat','local_space','affine_collapsed'], 'control_pixel_differences':[3001]*5,'non_target_identical_pixels':501,
             'coverage':{name:{'target':5001,'non_target':501,'background':50001,'target_texels':[151]*16} for name in ('source','projected','wrong_axis','wrong_repeat','local_space')}}
    for name in ('installed_exact_cli_bytes','authoring_hidden','source_unchanged','geometry_preserved','production_renderers_equal'): receipt[name]=True
    for name,value in [('adapter',adapter),('provenance',provenance),('frames',frames),('assets',assets),('receipt',receipt)]: js(name+'.json',value)
    return source,captures,binary,receipt


def editor_fixture(root, binary):
    root.mkdir()
    def js(path, value):
        path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(value))
    authoring=root/'authoring-hidden';package=root/'package-source-hidden'
    (authoring/'inputs').mkdir(parents=True);package.mkdir()
    recipe=b'editor recipe';original=b'editor source'
    (authoring/'project.planar-uv.json').write_bytes(recipe)
    (authoring/'inputs/source.model.json').write_bytes(original)
    model={'asset_id':'variants/projected.planar-uv.json','dependencies':[{'uri':'$source','sha256':hashlib.sha256(recipe).hexdigest()},{'uri':'inputs/source.model.json','sha256':hashlib.sha256(original).hexdigest()}]}
    data=json.dumps(model).encode();(authoring/'projected.model.json').write_bytes(data)
    (package/'projected.model.json').write_bytes(data);(package/'foreground.glb').write_bytes(b'unrelated glb')
    digest='a'*64;installed=root/'room/.orr/packages/objects'/digest;installed.mkdir(parents=True)
    files={}
    for path in package.iterdir():
        (installed/path.name).write_bytes(path.read_bytes());files[path.name]=hashlib.sha256(path.read_bytes()).hexdigest()
    binding={'asset':'projected.model.json','source_hash':files['projected.model.json'],'kind':'static','package':'planar','package_digest':digest}
    js(root/'room/room.models.json',{'bindings':{'guid':binding}})
    js(root/'room/orr.packages.lock.json',{'packages':{'planar':{'digest':digest,'files':files}}})
    js(root/'started.json',{'cli':str(binary)})
    js(root/'adapter.json',{'backend':'Vulkan','device_type':'Cpu','name':'llvmpipe fixture'})
    for prefix in ('01-assigned','02-undo','03-redo','04-saved','05-fresh-reopen','06-source-hidden-play-off','07-source-hidden-play-on'):
        js(root/(prefix+'-dimensions.json'),{'ui':[256,256],'viewport':[256,256],'adapter':'llvmpipe fixture'})
        raw=bytes([1 if prefix.startswith('07') else 0,17,91,255])*65536
        for kind in ('ui','viewport'):
            frame = bytes([15,15,15,64])*65536 if kind == 'ui' else raw
            (root/(prefix+'-'+kind+'.rgba')).write_bytes(frame);(root/(prefix+'-'+kind+'.png')).write_bytes(png(frame))
    for side in ('off','on'): (root/('presentation-'+side+'.frame')).write_bytes(b'opaque-Rust-decoded-frame')
    observed={'off_tick':12,'on_tick':12,'off_checksum':123,'on_checksum':123,'off_frame_bytes':len(b'opaque-Rust-decoded-frame'),'on_frame_bytes':len(b'opaque-Rust-decoded-frame'),'off_viewport_bytes':262144,'on_viewport_bytes':262144}
    js(root/'parity-observed.json',observed)
    receipt={'status':'passed','actual_cli':True,'actual_egui_widgets_and_gpu':True,'os_native_input':False,'undo_redo_save_fresh_reopen':True,'authoring_and_package_source_hidden':True,'authoritative_frame_byte_parity':True,'installed_asset':'projected.model.json','recipe_identity':model['asset_id'],'hidden_inputs':['recipe','source cooked model','original CLI output','package source'],'retained_installed_files':['projected.model.json','foreground.glb'],'source_hash':binding['source_hash'],'assigned_guid':'guid','tick':12,'checksum':123,'actual_viewport_changed_pixels':65536}
    def record(path): return {'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'length':path.stat().st_size}
    inputs={'algorithm':'sha256','cli':record(binary),'recipe':record(authoring/'project.planar-uv.json'),'source':record(authoring/'inputs/source.model.json'),'source_files':{}}
    for key,index in (('test',5),('oracle',4),('creator',0),('helper',1)):
        path=root.parent/gate.PATHS[index];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(gate.PATHS[index])
        inputs['source_files'][key]=record(path)
    js(root/'provenance-inputs.json',inputs)
    js(root/'provenance.json',dict(inputs,output=record(authoring/'projected.model.json'),installed=record(installed/'projected.model.json')))
    js(root/'acceptance.json',receipt)
    return receipt


def main():
    with tempfile.TemporaryDirectory() as directory:
        source,captures,binary,receipt=gpu_fixture(pathlib.Path(directory))
        def check(): return gate.check_gpu(captures,source,binary)
        check()
        for mutate in (
            lambda r:r.update(status='failed'),
            lambda r:r.update(width=255),
            lambda r:r.update(control_pixel_differences=[3000]*5),
            lambda r:r.update(non_target_identical_pixels=500),
            lambda r:r['coverage']['projected'].update(target_texels=[150]*16),
            lambda r:r.update(authoring_hidden=False),
            lambda r:r['provenance'].update(producer_fnv1a64='0'*16),
            lambda r:r.update(control_order=list(reversed(r['control_order']))),
        ):
            altered=copy.deepcopy(receipt);mutate(altered)
            (captures/'receipt.json').write_text(json.dumps(altered));reject(check)
        (captures/'receipt.json').write_text(json.dumps(receipt))
        for path in (binary,source/gate.PATHS[1],captures/'projected.rgba',captures/'source.png',captures/'recipe.json'):
            saved=path.read_bytes();path.write_bytes(saved+b'corrupt');reject(check);path.write_bytes(saved)
        adapter=receipt['adapter'];adapter['name']='SwiftShader fixture'
        (captures/'adapter.json').write_text(json.dumps(adapter));(captures/'receipt.json').write_text(json.dumps(receipt))
        reject(check)
        gate.check_gpu(captures,source,binary,local_adapter='SwiftShader')
        # Local validation cannot enter mandatory main: it has no arguments,
        # batch calls software() unconditionally and checker without override.
        observed=gate.manifest(captures)
        assert set(observed)=={p.name for p in captures.iterdir()}
        assert all(len(item['sha256'])==64 for item in observed.values())
    with tempfile.TemporaryDirectory() as directory:
        root=pathlib.Path(directory);binary=root/'cli';binary.write_bytes(b'fixture')
        captures=root/'editor';receipt=editor_fixture(captures,binary)
        def check_editor(): return gate.check_editor(captures,root,binary)
        check_editor()
        for key,value in [('actual_cli',False),('os_native_input',True),('authoritative_frame_byte_parity',False),('checksum',124),('actual_viewport_changed_pixels',20),('source_hash','0'*64)]:
            altered=copy.deepcopy(receipt);altered[key]=value
            (captures/'acceptance.json').write_text(json.dumps(altered));reject(check_editor)
        (captures/'acceptance.json').write_text(json.dumps(receipt))
        for path in (captures/'presentation-on.frame',captures/'07-source-hidden-play-on-viewport.rgba',captures/'authoring-hidden/project.planar-uv.json'):
            saved=path.read_bytes();path.write_bytes(saved+b'corrupt');reject(check_editor);path.write_bytes(saved)
        # Explicit real-world regression: ColorImage conversion used to
        # premultiply UI RGB15/alpha64 into RGB4/alpha64 before PNG writing.
        path=captures/'01-assigned-ui.png';saved=path.read_bytes()
        path.write_bytes(png(bytes([4,4,4,64])*65536));reject(check_editor);path.write_bytes(saved)
        check_editor()
    output='running 1 test\ntest test_name ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.01s\n'
    gate.checked_result(output,['test_name'],4)
    reject(lambda:gate.checked_result(output,['test_name'],0))
    reject(lambda:gate.checked_result(output.replace('1 passed','0 passed'),['test_name'],4))
    reject(lambda:gate.checked_result(output+output,['test_name'],4))
    workflow=(pathlib.Path(__file__).resolve().parents[1]/'.github/workflows/static-model-sampler.yml').read_text()
    block=workflow.split('      - name: Required grouped planar UV acceptance\n')[1].split('      - name: Retain evidence and failures')[0]
    assert "if: matrix.mode == 'ui'" in block
    assert block.count('python -B tools/static_model_planar_uv_gate.py') == 1
    assert 'ORR_STATIC_PLANAR_UV_EVIDENCE="$RUNNER_TEMP/orr-static-sampler-' in block
    assert '/planar-uv"' in block and '--' not in block
    assert workflow.count('${{ runner.temp }}') == 2
    assert 'timeout-minutes: 90' in workflow
    print('STATIC_PLANAR_UV_GATE_FIXTURES: passed')

if __name__=='__main__': main()
