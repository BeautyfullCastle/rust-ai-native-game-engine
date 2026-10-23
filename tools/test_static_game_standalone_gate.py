"""Synthetic checker fixtures only. Never native, isolation or GPU evidence."""
import copy
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zlib

import static_game_standalone_gate as gate


def write(path, value):
    path.write_text(json.dumps(value,separators=(',',':')))


def image(path, alias, phase):
    w,h = 640,480
    pixels = bytearray([0,0,0,255])*(w*h)
    def paint(world,color):
        camera=gate.expected_camera(alias)
        x,y = round(w/2+(world[0]-camera['center'][0])*min(w,h)/(2*camera['half_extent'])),round(h/2-(world[1]-camera['center'][1])*min(w,h)/(2*camera['half_extent']))
        for yy in range(y-10,y+11):
            for xx in range(x-10,x+11):
                pixels[(yy*w+xx)*4:(yy*w+xx)*4+4] = bytes((*color,255))
    if alias == 'delivery':
        paint([2,2],[255,210,80])
        if phase == 'restarted':
            paint([0,0],[80,150,255])
    else:
        paint([0,0],[80,150,255])
        paint([0,2],[80,255,80] if phase == 'won' else [150,150,150])
    def chunk(kind,data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    raw = b''.join(b'\0'+pixels[y*w*4:(y+1)*w*4] for y in range(h))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+
                    chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))


def fixture(root):
    provenance = {'source_root':'/hidden/source','toolchain_root':'/hidden/toolchain','cargo_home':'/hidden/cargo','source_sha':'a'*40,'test_sha256':'b'*64,'binaries':{'standalone_x11':{'path':'/hidden/target/release/deps/standalone_x11'}}}
    result = {'schema':gate.SCHEMA,'status':'passed','source_sha':'a'*40,'games':[]}
    for alias,(game,build) in gate.GAMES.items():
        directory = root/alias
        execution = directory/'execution'
        execution.mkdir(parents=True)
        bundle = directory/'relocated bundle with spaces'
        (bundle/'bin').mkdir(parents=True)
        (bundle/'scene').mkdir()
        (bundle/'bin/standalone').write_bytes(b'synthetic checker-only '+alias.encode())
        (bundle/'scene/game.scene.yaml').write_text('synthetic non-runnable scene\n')
        (bundle/'run-game').write_text('synthetic non-runnable launcher\n')
        (directory/'authored.scene.yaml').write_bytes((bundle/'scene/game.scene.yaml').read_bytes())
        files=[]
        for name,mode in [('bin/standalone',0o755),('run-game',0o755),('scene/game.scene.yaml',0o644)]:
            path=bundle/name
            path.chmod(mode)
            files.append({'path':name,'mode':mode,'bytes':path.stat().st_size,'sha256':gate.digest(path)})
        identity={'game':game,'build_id':build,'schema':{'fixture':alias},'input_contract':{'fixture':alias},
                  'seed':42,'tick_rate':60,'player_count':1,'initial_checksum':'0x0000000000000001'}
        runtime=gate.digest(bundle/'bin/standalone')
        provenance['binaries'][alias+'-player']={'sha256':runtime,'path':'/hidden/target/release/'+alias+'-player'}
        manifest={'schema':1,'source_sha':'a'*40,'target':'x86_64-unknown-linux-gnu',
                  'runtime_provenance':{'source_sha':'a'*40},'runtime_sha256':runtime,'identity':identity,'files':files}
        for key,target in [('schema','scene_descriptor_sha256'),('input_contract','input_contract_sha256')]:
            manifest[target]=hashlib.sha256(json.dumps(identity[key],separators=(',',':')).encode()).hexdigest()
        write(bundle/'bundle.json',manifest)
        neutral={'dx':0,'dy':0,'deliver':0} if alias=='delivery' else {'selection':0,'toggle':0}
        action={**neutral, 'deliver' if alias=='delivery' else 'toggle':1}
        move={**neutral, 'dx' if alias=='delivery' else 'selection':1}
        def state(tick):
            if alias=='delivery':
                return {'position':[131072,131072] if tick<4 else [max(0,(tick-3)//2)*65536,0],
                        'goal':[131072,131072],'won':int(2<=tick<4),'held':int(tick in (2,4,5,7))}
            return {'position':[max(0,(tick-3)//2)*131072,0], 'selected':max(0,(tick-3)//2),
                    'held':int(tick in (2,4,5,7)),'bits':int(2<=tick<4),'target':1,'moves':int(2<=tick<4),'won':int(2<=tick<4)}
        ticks=[{'tick':t,'checksum':f'0x{t+1:016x}','state':state(t),
                **({'input':action if t in (2,4) else {**move,'deliver' if alias=='delivery' else 'toggle':1} if t in (5,7) else neutral} if t else {})} for t in range(9)]
        # Neutral ticks preserve all state except the held bit.
        for t in (3,6,8):
            ticks[t]['state']={**ticks[t-1]['state'],'held':0}
        observations=[]
        def observe(tick,paused,consumed=False,resume_from=0,neutral_tick=1):
            value={'schema':'orr.static-standalone-observation/1','sequence':len(observations),
                   'tick':tick,'checksum':ticks[tick]['checksum'],'state':ticks[tick]['state'],
                   'identity':identity,'source_sha':'a'*40,'process_id':2,'paused':paused,
                   'focused':True,'resume_from_tick':resume_from,'neutral_consumed_tick':neutral_tick,
                   'consumed':[ticks[tick]] if consumed else [],
                   'adapter':{'backend':'Vulkan','device_type':'Cpu','name':'synthetic checker fixture'},
                   'camera':gate.expected_camera(alias),'size':[640,480]}
            observations.append(value)
            return value
        initial=observe(0,True)
        observe(1,False,True)
        observe(2,False,True)
        won=observe(2,True)
        observe(3,False,True)
        observe(4,False,True)
        restarted=observe(4,True)
        proofs=[]
        for t in (5,7):
            observe(t,False,True)
            paused=observe(t,True)
            paused['focused'] = t != 5
            for _ in range(6):
                no_auto=observe(t,True)
            resumed=observe(t+1,False,True,t,t+1)
            observe(t+1,True,False,t,t+1)
            proofs.append({'paused':paused,'no_auto_resume':no_auto,'resumed':resumed,'first_consumed':ticks[t+1]})
        (execution/'observations.ndjson').write_text(''.join(json.dumps(v)+'\n' for v in observations))
        (execution/'isolation.txt').write_text('source-hidden\n')
        (execution/'mountinfo.txt').write_text('1 0 0:1 / / ro - tmpfs tmpfs ro\n2 1 0:2 / /bundle\\040with\\040spaces ro - ext4 fixture ro\n3 1 0:3 / /evidence rw - ext4 fixture rw\n4 1 0:4 / /proc rw - proc proc rw\n5 1 0:5 / /tmp rw - tmpfs tmpfs rw\n')
        (execution/'process-status.txt').write_text('NSpid:\t2\n')
        (execution/'record.orrp').write_bytes(b'ORRP synthetic checker fixture, not replay evidence')
        write(execution/'source-replay.json',{'ticks':ticks})
        captures=[]
        for phase,observation in [('initial',initial),('won',won),('restarted',restarted)]:
            image(execution/(phase+'.png'),alias,phase)
            captures.append({'phase':phase,'observation':observation,'png':phase+'.png',
                             'sha256':gate.digest(execution/(phase+'.png')),
                             'x11':{'width':640,'height':480,'depth':24,'bits_per_pixel':32}})
        receipt={'schema':gate.SCHEMA,'status':'passed','alias':alias,'source_sha':'a'*40,
                 'test_executable_sha256':'b'*64,'input_source':'XTEST','framebuffer_source':'XGetImage',
                 'physical_gpu':False,'source_hidden':True,'network_disabled':True,'pid_namespace':True,
                 'process_id':2,'mountinfo_sha256':gate.digest(execution/'mountinfo.txt'),'process_status_sha256':gate.digest(execution/'process-status.txt'),
                 'process_binding':{'namespaces':{kind:{'host':kind+':[1]','sandbox':kind+':[2]'} for kind in ('pid','net','mnt')},'interfaces':['lo'],'namespace_pid':2,'namespace_ids':[999,2],'host_pid':999,'owned_launcher_pid':998,'executed_path':'/bundle with spaces/bin/standalone','sha256':runtime},'hidden_paths':['/hidden/source','/hidden/toolchain','/hidden/cargo','/hidden/target','/hidden/target/release/deps/standalone_x11','/hidden/target/release/'+alias+'-player',str((directory/'authored.scene.yaml').resolve()),str((directory/'trusted runtime copy').resolve()),str(bundle.resolve())],
                 'runtime_sha256':runtime,'bundle_sha256':gate.digest(bundle/'bundle.json'),
                 'scene_sha256':gate.digest(directory/'authored.scene.yaml'),
                 'observations_sha256':gate.digest(execution/'observations.ndjson'),'captures':captures,
                 'focus_clear':proofs[0],'escape_clear':proofs[1],
                 'replay':{'path':'record.orrp','sha256':gate.digest(execution/'record.orrp'),'ticks':8,
                           'source_replay_sha256':gate.digest(execution/'source-replay.json'),
                           'initial_checksum':ticks[0]['checksum'],'final_checksum':ticks[-1]['checksum']}}
        write(directory/'receipt.json',receipt)
        result['games'].append({'alias':alias,'receipt_sha256':gate.digest(directory/'receipt.json')})
    write(root/'result.json',result)
    provenance.update(run_id='1',run_attempt='1')
    editor={'schema':'orr.static-standalone-editor-trace/1','source_sha':provenance['source_sha'],
            'run_id':'1','run_attempt':'1','native_gui_proof':False,'games':[],
            'status':'passed','native_result_sha256':gate.digest(root/'result.json')}
    for alias in gate.GAMES:
        receipt=gate.json_file(root/alias/'receipt.json')
        editor['games'].append({'alias':alias,'identity':gate.json_file(root/alias/'relocated bundle with spaces'/'bundle.json')['identity'],
            'receipt_sha256':gate.digest(root/alias/'receipt.json'),'scene_sha256':receipt['scene_sha256'],
            'replay_sha256':receipt['replay']['sha256'],'source_replay_sha256':receipt['replay']['source_replay_sha256'],
            'ticks':receipt['replay']['ticks'],'initial_checksum':receipt['replay']['initial_checksum'],
            'final_checksum':receipt['replay']['final_checksum'],'all_ticks_match':True,
            'typed_state_match':True,'checked_registered_host':True,'compared_frames':receipt['replay']['ticks']+1})
    write(root/'editor-trace.json',editor)
    return provenance


class ValidatorTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        self.provenance=fixture(self.root)
    def receipt_change(self,change):
        path=self.root/'delivery/receipt.json'
        value=gate.json_file(path)
        change(value)
        write(path,value)
        result=gate.json_file(self.root/'result.json')
        result['games'][0]['receipt_sha256']=gate.digest(path)
        write(self.root/'result.json',result)
    def test_checker_fixture_only(self):
        self.assertTrue(gate.validate(self.root,self.provenance)['source_hidden'])
    def test_editor_checker_fixture_only(self):
        self.assertTrue(gate.validate_editor(self.root,self.provenance)['same_native_trace'])
    def test_editor_unrelated_trace(self):
        path=self.root/'editor-trace.json';value=gate.json_file(path)
        value['games'][0]['replay_sha256']='c'*64;write(path,value)
        with self.assertRaises(ValueError):gate.validate_editor(self.root,self.provenance)
    def test_editor_missing_game(self):
        path=self.root/'editor-trace.json';value=gate.json_file(path)
        value['games'].pop();write(path,value)
        with self.assertRaises(ValueError):gate.validate_editor(self.root,self.provenance)
    def test_editor_partial_ticks(self):
        path=self.root/'editor-trace.json';value=gate.json_file(path)
        value['games'][0]['ticks']-=1;write(path,value)
        with self.assertRaises(ValueError):gate.validate_editor(self.root,self.provenance)
    def test_missing_game(self):
        value=gate.json_file(self.root/'result.json');value['games'].pop();write(self.root/'result.json',value)
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_wrong_producer(self):
        self.provenance['test_sha256']='c'*64
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_extra_bundle_file(self):
        (self.root/'delivery/relocated bundle with spaces/source.rs').write_text('forbidden')
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_bundle_mutation(self):
        (self.root/'delivery/relocated bundle with spaces/bin/standalone').write_bytes(b'changed')
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_direct_input_substitute(self):
        self.receipt_change(lambda v:v.update(input_source='direct_controller'))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_offscreen_substitute(self):
        self.receipt_change(lambda v:v.update(framebuffer_source='offscreen'))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_no_isolation(self):
        self.receipt_change(lambda v:v.update(source_hidden=False))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_missing_first_consumed_tick(self):
        self.receipt_change(lambda v:v['focus_clear']['first_consumed'].update(tick=7))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_raw_replay_mutation(self):
        (self.root/'delivery/execution/record.orrp').write_bytes(b'JSON substitute')
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_missing_actual_pixels(self):
        path=self.root/'delivery/execution/restarted.png'
        image(path,'delivery','won')
        self.receipt_change(lambda v:v['captures'][2].update(sha256=gate.digest(path)))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_vacuous_action_clear(self):
        self.receipt_change(lambda v:v['focus_clear']['paused']['state'].update(held=0))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_wrong_camera(self):
        self.receipt_change(lambda v:v['captures'][0]['observation']['camera'].update(center=[0,0]))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_writable_bundle_mount(self):
        path=self.root/'delivery/execution/mountinfo.txt'
        path.write_text(path.read_text().replace('spaces ro','spaces rw'))
        self.receipt_change(lambda v:v.update(mountinfo_sha256=gate.digest(path)))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_missing_toolchain_hidden_path(self):
        self.receipt_change(lambda v:v['hidden_paths'].remove('/hidden/toolchain'))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_external_network_namespace(self):
        self.receipt_change(lambda v:v['process_binding'].update(interfaces=['lo','eth0']))
        with self.assertRaises(ValueError):gate.validate(self.root,self.provenance)
    def test_png_crc(self):
        path=self.root/'delivery/execution/initial.png'
        value=bytearray(path.read_bytes());value[-1]^=1;path.write_bytes(value)
        with self.assertRaises(ValueError):gate.png(path,640,480)
    def test_duplicate_json(self):
        with self.assertRaises(ValueError):gate.strict_json('{"tick":1,"tick":2}')
    def test_zero_or_filtered_tests(self):
        for text in ('test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out',
                     'test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 3 filtered out'):
            with self.assertRaises(ValueError):gate.test_summary(text,1,4)


class CommandBounds(unittest.TestCase):
    def test_success_records_duration(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            self.assertEqual(gate.command([gate.sys.executable,'-c','pass'],root,'success',{},2),'')
            timing=gate.strict_json((root/'success-timing.json').read_text())
            self.assertEqual(timing['exit_code'],0)
            self.assertGreaterEqual(timing['seconds'],0)
    def test_failure_preserves_diagnostics(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(ValueError):
                gate.command([gate.sys.executable,'-c','print("diagnostic");raise SystemExit(2)'],root,'failure',{},2)
            self.assertEqual((root/'failure.log').read_text(),'diagnostic\n')
            self.assertEqual(gate.strict_json((root/'failure-timing.json').read_text())['exit_code'],2)
    def test_timeout_is_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(ValueError):
                gate.command([gate.sys.executable,'-c','import time;time.sleep(2)'],root,'timeout',{},0.1)
            self.assertTrue((root/'timeout-timing.json').is_file())
    def test_output_overflow_is_bounded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            with self.assertRaises(ValueError):
                gate.command([gate.sys.executable,'-c','import sys;sys.stdout.buffer.write(b"x"*(33*1024*1024))'],root,'overflow',{},5)
            self.assertLessEqual((root/'overflow.log').stat().st_size,32*1024*1024)


if __name__=='__main__':unittest.main()
