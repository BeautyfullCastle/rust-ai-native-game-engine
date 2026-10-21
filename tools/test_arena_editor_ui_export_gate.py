#!/usr/bin/env python3
"""Independent adversarial validator fixtures, explicitly NOT native acceptance."""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import tempfile
import unittest
from unittest import mock
import zlib

import arena_editor_ui_export_gate as gate
import test_arena_editor_export_gate as old


def write(path, obj):
    path.write_text(json.dumps(obj,ensure_ascii=False), encoding="utf-8")


def png(path, pixels, width=512, height=512):
    def chunk(kind,data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
    rows = b''.join(b'\0'+pixels[y*width*4:(y+1)*width*4] for y in range(height))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(rows))+chunk(b'IEND',b''))


def summary(name,total):
    return f'running 1 test\ntest {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; {total-1} filtered out; finished in 0.1s\n'


class Tests(unittest.TestCase):
    def setUp(self):
        self.old = old.Tests('test_exact_source_names_and_cfg'); self.old.setUp()
        self.addCleanup(self.old.doCleanups)
        self.root = self.old.root

    def fixture(self):
        root,r = self.old.receipt()
        # Upgrade from the deliberately legacy fixture without changing its tests.
        report = gate.runner.report
        report['producers'] = {}
        for number,spec in enumerate(gate.SPECS):
            label,package,target,kind,requested,features = spec
            directory = gate.runner.evidence/'producers'/label; directory.mkdir(parents=True)
            path = directory/label; path.write_bytes(('fixture executable '+label).encode()); path.chmod(0o755)
            original = gate.runner.repo/'target'/label/'release'/('' if kind == 'bin' else 'deps')/label
            original.parent.mkdir(parents=True); shutil.copy2(path,original)
            report['producers'][label] = {'producer':{'executable':str(original)},'file':path.relative_to(gate.runner.evidence).as_posix(),
                'sha256':gate.runner.digest(path),'bytes':path.stat().st_size}
        report['gpu_environment'] = {'VK_ICD_FILENAMES':'/fixture/lvp.json','VK_DRIVER_FILES':'/fixture/lvp.json','WGPU_BACKEND':'vulkan','ORR_REQUIRE_GPU':'1','XDG_RUNTIME_DIR':'/fixture/xdg'}
        report['app_inventory_total'] = 2
        r['runtime'] = gate.bound('ui-runtime'); r['test_producer'] = gate.bound('ui-harness')
        project = root/'project'; shutil.rmtree(project/'.orr/packages/objects')
        lock = {'schema':1,'direct':{},'packages':{}}
        for name, directory in [('sample-sprites','sprite_demo'),('korean-game-ui','game_ui_font')]:
            source = Path(__file__).resolve().parents[1]/'assets'/directory
            manifest = gate.read(source/'orr.package.json')
            manifest = {key:manifest[key] for key in ('schema','name','version','engine','capabilities','dependencies','files')}
            manifest['files'] = sorted(manifest['files'])
            hashes = {name:gate.runner.digest(source/name) for name in manifest['files']}
            identity = hashlib.sha256(json.dumps(['orr-package-v1',manifest,dict(sorted(hashes.items()))],ensure_ascii=False,separators=(',',':')).encode()).hexdigest()
            destination = project/'.orr/packages/objects'/identity; destination.mkdir(parents=True)
            write(destination/'orr.package.json',manifest)
            for filename in hashes: shutil.copyfile(source/filename,destination/filename)
            lock['packages'][name] = {'manifest':manifest,'digest':identity,'files':hashes}
        write(project/'orr.packages.lock.json',lock)
        source_manifest = gate.read(project/'orr.project.json')
        source_manifest['entry']['scene'] = 'arena.scene.yaml'
        (project/'arena.scene.yaml').write_text('fixture scene', encoding="utf-8")
        (project/'README.md').write_text('source-only documentation is preserved, not exported', encoding="utf-8")
        source_manifest['entry']['ui'] = {'profile':'arena-korean-v1','font':{'package':'korean-game-ui','asset':'OrreryKoreanUI.otf'}}
        write(project/'orr.project.json',source_manifest)
        bundle = root/'bundle'; shutil.rmtree(bundle); (bundle/'bin').mkdir(parents=True)
        shutil.copy2(gate.producer_path('ui-runtime'),bundle/'bin/arena')
        (bundle/'run-arena').write_bytes(b'fixture launcher'); (bundle/'run-arena').chmod(0o755)
        shutil.copytree(project,bundle/'project')
        (bundle/'project/README.md').unlink()
        for path in (bundle/'project').rglob('*'):
            if path.is_file(): path.chmod(0o644)
        payload = {'profile':'arena-authored-linux-x86_64-v1','entry':source_manifest['entry'],
            'packages':{name:package['digest'] for name,package in lock['packages'].items()},
            'files':[dict(item,mode=0o755 if item['path'] in ('bin/arena','run-arena') else 0o644,role='fixture') for item in gate.legacy.files(bundle)],
            'runtime':{key:report['producers']['ui-runtime'][key] for key in ('sha256','bytes')},'initial_checksum':'0x0000000000000001'}
        roles={'bin/arena':'runtime','run-arena':'launcher','project/orr.project.json':'project_manifest','project/orr.packages.lock.json':'package_lock','project/arena.scene.yaml':'scene','project/arena.sprites.json':'sprite_sidecar'}
        for entry in payload['files']:
            entry['role']=roles.get(entry['path'],'package_manifest' if entry['path'].endswith('/orr.package.json') else 'package_asset')
        manifest = {'schema':1,'payload':payload,'content_digest':hashlib.sha256(b'orrery.arena.export.content.v1\0'+json.dumps(payload,ensure_ascii=False,separators=(',',':')).encode()).hexdigest()}
        write(bundle/'orr.export.json',manifest)
        (root/'terminal-joined-success.txt').write_text(f"Actual export widget reached terminal joined success for captured scene {root/'project/arena.scene.yaml'} at checksum {payload['initial_checksum']}\n", encoding="utf-8")
        r.update(source_files=gate.legacy.files(project),source_files_before=gate.legacy.files(project),source_files_after=gate.legacy.files(project),bundle_files=gate.legacy.files(bundle),commands=list(gate.STAGES))
        write(root/'source-files-before.json',r['source_files'])
        write(root/'producer-receipt.json',{key:r[key] for key in ('source_head','source_tree','runtime','test_producer')})
        for label,name in [('ui-harness','ui-consumer'),('app-harness','app-ui-consumer')]: shutil.copy2(gate.producer_path(label),root/name)
        write(root/'app-consumer-producer.json',{'source_head':r['source_head'],'source_tree':r['source_tree'],'original':str(gate.producer_path('app-harness')),'copied':str(root/'app-ui-consumer'),'sha256':report['producers']['app-harness']['sha256'],'test':gate.APP})
        write(root/'app-restart-receipt.json',{'bundle':str(bundle),'initial_checksum':payload['initial_checksum'],'replayed_checksums':list(range(2,14)),
            'actual_widget_dispatch':True,'restart_tick':0,'original_entities_positions_players_restored':True})
        (root/'project-only.stdout.txt').write_text('', encoding="utf-8"); (root/'project-only.stderr.txt').write_text(gate.DIAGNOSTIC+'\n', encoding="utf-8")
        (root/'project-only-export-rejection.txt').write_text(gate.EXPORT_DIAGNOSTIC, encoding="utf-8")
        write(root/'project-only-runtime.json',dict(gate.bound('project-runtime'),status=1,stdout='project-only.stdout.txt',stderr='project-only.stderr.txt',export_error=gate.EXPORT_DIAGNOSTIC,published=False))
        for stage,name in [('isolated-ui-controls',gate.MODULE),('isolated-app-restart',gate.APP)]:
            (root/(stage+'.stdout.txt')).write_text(gate.CONSUMER_PROOF+summary(name,3 if name == gate.MODULE else 2), encoding="utf-8"); (root/(stage+'.stderr.txt')).write_text('', encoding="utf-8")
        commands=[]
        for stage in gate.STAGES:
            program,args,environment,status = gate.expected_command(root,stage)
            row = {'stage':stage,'program':program,'args':args,'environment':environment,'gpu_environment':report['gpu_environment'],
                'cwd':str(root),'status':status,'expected_status':status,'stdout_file':stage+'.stdout.txt','stderr_file':stage+'.stderr.txt'}
            for stream in ('stdout','stderr'): row[stream+'_sha256'] = gate.runner.digest(root/row[stream+'_file'])
            commands.append(row)
        self.commands(root,commands)
        # Construct independent raster controls, not success text or precomputed claims.
        base = bytearray(bytes([10,10,10,255])*(512*512))
        playing = bytearray(base)
        for y in range(10,20):
            for x in range(10,30): playing[(y*512+x)*4:(y*512+x)*4+4] = bytes([255,255,255,255])
        control = bytearray(base)
        control[(400*512+400)*4] = 20
        playing[(400*512+400)*4] = 20
        glyphs = bytearray(base)
        for i in range(5):
            for y in range(8+i):
                for x in range(8): glyphs[((100+y)*512+10+i*42+x)*4:((100+y)*512+10+i*42+x)*4+4] = bytes([255,255,255,255])
        buffers = {'ui-negative-no-overlay.png':base,'ui-negative-no-text.png':control,'ui-glyphs-before.png':base,'ui-glyphs-after.png':glyphs,
            'ui-title.png':playing,'ui-playing.png':playing,'ui-menu.png':base,'ui-continued.png':playing,'ui-restart-action.png':playing}
        for name,buffer in buffers.items(): png(root/name,buffer)
        r['captures'] = [{'path':name,'width':512,'height':512,'sha256':gate.runner.digest(root/name)} for name in gate.CAPTURES]
        write(root/'ui-consumer-adapter.json',{'name':'llvmpipe fixture','software':True})
        t={'positive':'ui-playing.png','control':'ui-negative-no-text.png','no_overlay':'ui-negative-no-overlay.png','width':512,'height':512,
           'region_xyxy':[10,10,30,20],'region_text':'오러리 투기장','suppressed_text_shapes':1,'control_method':'clone exact output and override only production HUD TextShape glyph color',
           'changed_pixels':200,'bright_different_pixels':200,'minimum_changed_exclusive':100,'minimum_bright_exclusive':30,'bright_channel_threshold_exclusive':100,
           'positive_buttons':[{'action':'Menu','min':[60,0],'max':[90,30]}],'control_buttons':[{'action':'Menu','min':[60,0],'max':[90,30]}],
           'matching_layout':True,'control_against_itself':False,'control_text_color_rgba':[0,0,0,0]}
        for key in ('positive','control','no_overlay'): t[key+'_sha256']=gate.runner.digest(root/t[key])
        write(root/'ui-text-oracle.json',t)
        g={'before':'ui-glyphs-before.png','after':'ui-glyphs-after.png','width':512,'height':512,'glyphs':['오','러','리','메','뉴'],
           'regions_xywh':[[10+i*42,100,40,40] for i in range(5)],'changed_pixels':gate.glyph_oracle(base,glyphs),'minimum_changed_exclusive':40,
           'all_masks_pairwise_distinct':True,'pair_comparisons':10,'unchanged_pixel_xy':[300,300],'font_source':'relocated bundle admission'}
        for key in ('before','after'): g[key+'_sha256']=gate.runner.digest(root/g[key])
        font=bundle/'project/.orr/packages/objects'/payload['packages']['korean-game-ui']/'OrreryKoreanUI.otf'
        g.update(descriptor=source_manifest['entry']['ui'],font_sha256=gate.runner.digest(font),font_bytes=font.stat().st_size)
        write(root/'ui-glyph-oracle.json',g)
        write(root/'ui-acceptance.json',{'source_head':r['source_head'],'source_tree':r['source_tree'],'font_closure':True,'actual_app_dispatch':True,'actual_app_restart':True,
            'independent_no_ui_runtime_sha256':report['producers']['project-runtime']['sha256'],'consumer_sha256':report['producers']['ui-harness']['sha256'],'app_consumer_sha256':report['producers']['app-harness']['sha256']})
        write(root/'receipt.json',r)
        return root,r

    def commands(self,root,rows):
        (root/'commands.jsonl').write_text(''.join(json.dumps(row)+'\n' for row in rows), encoding="utf-8")
        for row in rows: write(root/(row['stage']+'.command-state.json'),dict(row,state='completed',error=None))

    def mutate_json(self,path,key,values,check):
        original=gate.read(path)
        for value in values:
            bad=copy.deepcopy(original);bad[key]=value;write(path,bad)
            with self.assertRaises((AssertionError,KeyError,ValueError)): check()
        write(path,original)

    def test_full_synthetic_receipt_passes_validator_only(self):
        root,_=self.fixture();gate.verify_receipt(root)

    def test_exact_feature_profiles_and_legacy_inventories(self):
        self.assertEqual((len(gate.legacy.SAMPLE),len(gate.legacy.UI_SAMPLE_BASE),len(gate.legacy.UI_SAMPLE)),(16,3,6))
        self.assertEqual((len(gate.legacy.UI_EDITOR),len(gate.legacy.UI_IDENTITY)),(2,3))
        self.assertEqual(gate.legacy.UI_CENSUS['image-reimport'],(200,5,25,True))
        for label,package,target,kind,requested,features in gate.SPECS:
            self.assertEqual(len(features),len(set(features)))
        self.assertEqual(len({(s[1],s[2],s[3],tuple(s[5])) for s in gate.SPECS}),4)

    def test_wrong_feature_profile_freshness_rejected(self):
        item=self.old.artifact()
        for field,value in [('features',['game-ui']),('fresh',True),('profile',{'test':False,'opt_level':'3'}),('profile',{'test':True,'opt_level':'0'})]:
            bad=copy.deepcopy(item);bad[field]=value
            with self.assertRaises(AssertionError):self.old.pin(self.old.cargo(bad))

    def test_duplicate_producer_rejected(self):
        root,_=self.fixture()
        with self.assertRaises(AssertionError):gate.pin(gate.SPECS[0],'')

    def test_original_or_copied_producer_mutation_rejected(self):
        root,_=self.fixture()
        for entry in gate.runner.report['producers'].values():
            for path in [Path(entry['producer']['executable']),gate.runner.evidence/entry['file']]:
                original=path.read_bytes();path.write_bytes(original+b'x')
                with self.assertRaises(AssertionError):gate.producer_fence()
                path.write_bytes(original)

    def test_source_head_and_tree_rejected(self):
        root,_=self.fixture()
        for key in ['source_head','source_tree']:
            self.mutate_json(root/'receipt.json',key,['f'*40],lambda:gate.verify_receipt(root))

    def test_changed_source_and_bundle_rejected(self):
        root,_=self.fixture()
        for path in [root/'project/orr.project.json',root/'bundle/project/orr.project.json']:
            original=path.read_bytes();path.write_bytes(original+b' ')
            with self.assertRaises(AssertionError):gate.verify_receipt(root)
            path.write_bytes(original)

    def test_unexpected_ignored_inventory_rejected(self):
        profile='authored-ui';passed,count,_,_=gate.legacy.UI_CENSUS[profile]
        names=list(gate.legacy.SAMPLE+gate.legacy.UI_SAMPLE_BASE+gate.legacy.UI_SAMPLE)+( [gate.APP] )
        names += [f'legacy::case_{i}' for i in range(passed+count-len(names))]
        path,ignored=self.root/'list',self.root/'ignored'
        path.write_text(self.old.list_text(names), encoding="utf-8");ignored.write_text(self.old.list_text([gate.APP]), encoding="utf-8")
        gate.legacy.profile_census(profile,path,ignored)
        for bad in [[],[gate.APP,names[0]],[names[0]]]:
            ignored.write_text(self.old.list_text(bad), encoding="utf-8")
            with self.assertRaises(AssertionError):gate.legacy.profile_census(profile,path,ignored)

    def test_missing_explicit_child_execution_rejected(self):
        root,_=self.fixture()
        for stage in ['isolated-ui-controls','isolated-app-restart']:
            path=root/(stage+'.stdout.txt');original=path.read_text(encoding="utf-8")
            rows=[json.loads(line) for line in (root/'commands.jsonl').read_text(encoding="utf-8").splitlines()]
            path.write_text(gate.CONSUMER_PROOF+'UI controls passed\n', encoding="utf-8")
            for row in rows:
                if row['stage'] == stage: row['stdout_sha256']=gate.runner.digest(path)
            self.commands(root,rows)
            with self.assertRaises((AssertionError,ValueError)):gate.verify_receipt(root)
            path.write_text(original, encoding="utf-8")
            for row in rows:
                if row['stage'] == stage: row['stdout_sha256']=gate.runner.digest(path)
            self.commands(root,rows)

    def test_command_order_argv_environment_mounts_rejected(self):
        root,r=self.fixture();rows=[json.loads(line) for line in (root/'commands.jsonl').read_text(encoding="utf-8").splitlines()]
        changes=[(0,'args',[]),(1,'cwd','/tmp/elsewhere'),(2,'args',[]),(8,'environment',{}),(9,'gpu_environment',{}),(9,'program','sh')]
        for index,key,value in changes:
            bad=copy.deepcopy(rows);bad[index][key]=value;self.commands(root,bad)
            with self.assertRaises(AssertionError):gate.verify_commands(root,r)
        self.commands(root,list(reversed(rows)))
        with self.assertRaises(AssertionError):gate.verify_commands(root,r)

    def test_missing_durable_completed_state_rejected(self):
        root,_=self.fixture()
        self.mutate_json(root/'capture-initial.command-state.json','state',['started','wait_failed'],lambda:gate.verify_receipt(root))
        (root/'capture-initial.command-state.json').unlink()
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_negative_status_and_diagnostic_rejected(self):
        root,r=self.fixture();rows=[json.loads(line) for line in (root/'commands.jsonl').read_text(encoding="utf-8").splitlines()]
        for status in [0,2,None,True,-9]:
            bad=copy.deepcopy(rows);bad[0]['status']=status;self.commands(root,bad)
            with self.assertRaises(AssertionError):gate.verify_commands(root,r)
        self.commands(root,rows)
        for bad_text in ['','segfault',gate.DIAGNOSTIC+' extra']:
            (root/'project-only.stderr.txt').write_text(bad_text, encoding="utf-8")
            bad=copy.deepcopy(rows);bad[0]['stderr_sha256']=gate.runner.digest(root/'project-only.stderr.txt');self.commands(root,bad)
            with self.assertRaises(AssertionError):gate.verify_commands(root,r)

    def test_fake_adapter_and_software_rejected(self):
        for name,software in [('SwiftShader',True),('llvmpipe',False),('physical',True),('lavapipe',1)]:
            with self.assertRaises(AssertionError):gate.software_adapter(name,software)

    def test_png_dimensions_and_corruption_rejected(self):
        root,_=self.fixture();path=root/'ui-title.png'
        png(path,bytes([0,0,0,255])*(256*256),256,256)
        with self.assertRaises(AssertionError):gate.fence.png_pixels(path,512,512)
        path.write_bytes(b'PNG')
        with self.assertRaises(AssertionError):gate.fence.png_pixels(path,512,512)

    def test_blank_identical_glyph_masks_rejected(self):
        blank=bytes([0,0,0,255])*(512*512)
        with self.assertRaises(AssertionError):gate.glyph_oracle(blank,blank)

    def test_repeated_tofu_masks_rejected(self):
        blank=bytes([0,0,0,255])*(512*512);after=bytearray(blank)
        for i in range(5):
            for y in range(10):
                for x in range(10):after[((100+y)*512+10+i*42+x)*4:((100+y)*512+10+i*42+x)*4+4]=bytes([255,255,255,255])
        with self.assertRaises(AssertionError):gate.glyph_oracle(blank,after)

    def test_text_control_and_layout_rejected(self):
        root,_=self.fixture()
        for key,values in [('control_buttons',[[]]),('region_text',['123']),('changed_pixels',[0]),('suppressed_text_shapes',[0,2]),('control_against_itself',[True])]:
            self.mutate_json(root/'ui-text-oracle.json',key,values,lambda:gate.verify_receipt(root))
        blank=bytes([0,0,0,255])*(512*512)
        with self.assertRaises(AssertionError):gate.text_oracle(blank,blank,[0,0,320,48])

    def test_missing_font_license_and_closure_rejected(self):
        root,_=self.fixture();manifest=gate.read(root/'bundle/orr.export.json')
        directory=root/'bundle/project/.orr/packages/objects'/manifest['payload']['packages']['korean-game-ui']
        for name in ['OrreryKoreanUI.otf','OFL.txt','COPYRIGHT.txt','font-manifest.json']:
            path=directory/name;original=path.read_bytes();path.write_bytes(b'corrupt')
            with self.assertRaises(AssertionError):gate.verify_receipt(root)
            path.write_bytes(original)
        (directory/'unrelated-extra.txt').write_bytes(b'extra')
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_app_restart_binding_and_replay_rejected(self):
        root,_=self.fixture()
        for key,values in [('bundle',['/unrelated']),('initial_checksum',['0x0000000000000002']),('restart_tick',[1,False]),('replayed_checksums',[[],[1]*12]),('actual_widget_dispatch',[False]),('original_entities_positions_players_restored',[False])]:
            self.mutate_json(root/'app-restart-receipt.json',key,values,lambda:gate.verify_receipt(root))
        self.mutate_json(root/'app-consumer-producer.json','sha256',['0'*64],lambda:gate.verify_receipt(root))

    def test_readonly_probe_and_completed_export_required(self):
        root,_=self.fixture()
        self.mutate_json(root/'stage-outcomes.json','stage',['started'],lambda:gate.verify_receipt(root))
        (root/'bundle/.ui-consumer-write-probe').write_bytes(b'unexpected')
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_finite_native_selector_fresh_roots_and_order(self):
        with mock.patch.dict(os.environ):
            gate.runner.initialize(gate.runner.repo,self.root/'phase','cpu')
            for spec in gate.SPECS: gate.select_target(spec)
            self.assertEqual(len({row['target'] for row in gate.runner.report['ui_target_selections']}),4)
            with self.assertRaises(AssertionError):gate.select_target(gate.SPECS[0])

    def test_native_selector_rejects_wrong_order_and_stale_root(self):
        with mock.patch.dict(os.environ):
            gate.runner.initialize(gate.runner.repo,self.root/'phase','cpu')
            with self.assertRaises(AssertionError):gate.select_target(gate.SPECS[1])
            (gate.runner.target_dir/'stale').write_text('old', encoding="utf-8")
            with self.assertRaises(AssertionError):gate.select_target(gate.SPECS[0])

    def test_nonadjacent_repeated_masks_rejected(self):
        blank=bytes([0,0,0,255])*(512*512);after=bytearray(blank)
        for i in range(5):
            for y in range(8+(i if i < 4 else 0)):
                for x in range(8):after[((100+y)*512+10+i*42+x)*4:((100+y)*512+10+i*42+x)*4+4]=bytes([255,255,255,255])
        with self.assertRaises(AssertionError):gate.glyph_oracle(blank,after)

    def test_font_descriptor_hash_and_pair_count_rejected(self):
        root,_=self.fixture()
        for key,values in [('descriptor',[{}]),('font_sha256',['0'*64]),('font_bytes',[0]),('pair_comparisons',[4]),('all_masks_pairwise_distinct',[False])]:
            self.mutate_json(root/'ui-glyph-oracle.json',key,values,lambda:gate.verify_receipt(root))

    def test_cpu_production_profile_keeps_old_and_new_exact_selections(self):
        g=gate.legacy
        with mock.patch.dict(os.environ):
            g.runner.initialize(g.runner.repo,self.root/'phase','cpu');g.runner.report.update(mode='cpu',coverage={})
            initial=g.runner.target_dir;observed=[]
            inventories={'default':(), 'sprites':(), 'baseline':g.BASELINE,
                'extended':g.APP+g.EDITOR+g.NEW_APP+g.UI_EDITOR+g.UI_IDENTITY,
                'sample':g.SAMPLE+g.UI_SAMPLE_BASE,
                'close':g.BASELINE+tuple(n for n in g.EXTENDED_ONLY if 'loaded_' in n)+(g.NEW_APP[1],)+g.UI_EDITOR+g.UI_IDENTITY,
                'sample-ui':g.SAMPLE+g.UI_SAMPLE_BASE+g.UI_SAMPLE+(g.UI_APP,), 'ui-only':(),
                'minimal-ui':g.BASELINE+(g.EXTENDED_ONLY[1],)+g.UI_EDITOR+g.UI_IDENTITY,'export-only':g.BASELINE}
            def inventory(label):
                if label.endswith('-full-ignored'):return [g.UI_APP] if label == 'sample-ui-full-ignored' else []
                return list(inventories[label.removesuffix('-full-list')])
            def producer(output,label,*args):observed.append((label,g.runner.target_dir));return str(g.runner.evidence/label)
            with mock.patch.object(g.sys,'platform','linux'),mock.patch.object(g.platform,'machine',return_value='x86_64'),mock.patch.object(g,'run',side_effect=lambda label,*args:label),mock.patch.object(g,'producer',side_effect=producer),mock.patch.object(g,'inventory',side_effect=inventory),mock.patch.object(g.close,'inventory'),mock.patch.object(g,'selected') as selected:
                g.cpu(ui=True)
            self.assertEqual([label for label,_ in observed],list(inventories)+['ui-cpu-harness'])
            self.assertEqual({target for _,target in observed},{initial})
            calls={call.args[0]:call.args for call in selected.call_args_list}
            self.assertEqual(len(calls['extended'][2]),32)
            self.assertEqual(len(calls['close'][2]),29)
            self.assertEqual(len(calls['minimal-ui'][2]),27)
            self.assertEqual(len(calls['sample'][2]),19)
            self.assertEqual(len(calls['sample-ui'][2]),25)
            self.assertEqual(calls['ui-cpu-regression'][2],[gate.REGRESSION])
            self.assertIn('close-interaction',calls)
            self.assertEqual(len(g.runner.report['producer_target_selections']),11)



    def test_real_rust_raw_export_error_terminal_newline_passes(self):
        root,_=self.fixture()
        (root/'project-only-export-rejection.txt').write_text(gate.EXPORT_DIAGNOSTIC+'\n', encoding="utf-8")
        receipt=gate.read(root/'project-only-runtime.json')
        receipt['export_error']=gate.EXPORT_DIAGNOSTIC+'\n'
        write(root/'project-only-runtime.json',receipt)
        gate.verify_receipt(root)

    def test_raw_export_error_normalization_is_exact_and_exit_one(self):
        root,r=self.fixture()
        path=root/'project-only-runtime.json';original=gate.read(path)
        for value in ['wrong diagnostic',gate.EXPORT_DIAGNOSTIC+'\nextra output',
                      ' '+gate.EXPORT_DIAGNOSTIC,gate.EXPORT_DIAGNOSTIC+' ',
                      gate.EXPORT_DIAGNOSTIC+'\n\n',gate.EXPORT_DIAGNOSTIC+'\t',gate.EXPORT_DIAGNOSTIC+'\r\n']:
            bad=dict(original,export_error=value);write(path,bad)
            (root/'project-only-export-rejection.txt').write_text(value, encoding="utf-8")
            with self.assertRaises(AssertionError):gate.verify_commands(root,r)
        (root/'project-only-export-rejection.txt').write_text(gate.EXPORT_DIAGNOSTIC+'\n', encoding="utf-8")
        # Individually permitted spellings must still match the retained raw file.
        write(path,original)
        with self.assertRaises(AssertionError):gate.verify_commands(root,r)
        for status in [0,2,None,True,-9]:
            write(path,dict(original,export_error=gate.EXPORT_DIAGNOSTIC+'\n',status=status))
            with self.assertRaises(AssertionError):gate.verify_commands(root,r)



if __name__ == '__main__':
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    assert result.wasSuccessful() and result.testsRun == gate.FIXTURE_COUNT and not result.skipped
    print(f'ARENA_UI_EXPORT_GATE_FIXTURES: {gate.FIXTURE_COUNT} passed; 0 failed; 0 skipped',flush=True)
