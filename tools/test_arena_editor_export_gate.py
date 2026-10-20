#!/usr/bin/env python3
"""Independent negative controls; fixtures are never native acceptance."""
import copy
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import tempfile
import unittest
from unittest import mock
import zlib

import arena_editor_export_gate as gate


class Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.saved = gate.runner.repo, gate.runner.evidence, gate.runner.target_dir, gate.runner.report
        self.addCleanup(self.restore)
        gate.runner.repo = self.root / 'repo'; gate.runner.repo.mkdir()
        gate.runner.evidence = self.root / 'evidence'; gate.runner.evidence.mkdir()
        gate.runner.target_dir = self.root / 'target'
        gate.runner.report = {'head':'a'*40, 'tree':'b'*40, 'producers':{}, 'coverage':{}}

    def restore(self):
        gate.runner.repo, gate.runner.evidence, gate.runner.target_dir, gate.runner.report = self.saved

    def artifact(self):
        binary = gate.runner.target_dir / 'release/deps/orr_editor-fixture'
        binary.parent.mkdir(parents=True, exist_ok=True); binary.write_bytes(b'fixture only')
        crate = gate.runner.repo / 'crates/orr_editor'
        return {'reason':'compiler-artifact', 'fresh':False, 'package_id':'path+'+crate.as_uri()+'#orr_editor@0.0.1',
            'manifest_path':str(crate/'Cargo.toml'), 'target':{'name':'orr_editor','kind':['lib'],'crate_types':['lib'],'src_path':str(crate/'src/lib.rs')},
            'profile':{'test':True,'opt_level':'3'},'features':[], 'executable':str(binary)}

    def cargo(self, item):
        return json.dumps(item)+'\n{"reason":"build-finished","success":true}\n'

    def pin(self, text):
        return gate.producer(text, 'test', 'orr_editor', 'orr_editor', 'lib', [])

    def list_text(self, names):
        return ''.join(n+': test\n' for n in names)+f'{len(names)} tests, 0 benchmarks\n'

    def receipt(self):
        root = gate.runner.evidence/'acceptance'; root.mkdir()
        project = root/'project'; project.mkdir()
        (project/'orr.project.json').write_text(json.dumps({'entry':{'game':'arena','sprites':'arena.sprites.json'}}))
        (project/'arena.sprites.json').write_text(json.dumps({'bindings':{'hero':{'package':'sample-sprites','document':'sprites.json'}}}))
        source=Path(__file__).resolve().parents[1]/'assets/sprite_demo'
        installed=project/'.orr/packages/objects'/('c'*64);shutil.copytree(source,installed)
        hashes={name:gate.runner.digest(installed/name) for name in ['sprites.json','lantern_keeper.png','lantern_keeper.rgba','LICENSE.txt']}
        (project/'orr.packages.lock.json').write_text(json.dumps({'packages':{'sample-sprites':{'digest':'c'*64,'files':hashes}}}))
        rgba=(installed/'lantern_keeper.rgba').read_bytes()
        color=next(rgba[i:i+4] for i in range(0,len(rgba),4) if rgba[i+3]==255 and (abs(rgba[i]-rgba[i+1])>20 or abs(rgba[i+1]-rgba[i+2])>20))
        bundle = root/'bundle'; (bundle/'bin').mkdir(parents=True)
        (bundle/'bin/arena').write_bytes(b'runtime'); (bundle/'run-arena').write_bytes(b'launcher'); (bundle/'LICENSE').write_bytes(b'license')
        runtime = gate.runner.evidence/'runtime-files/arena'; runtime.parent.mkdir(); runtime.write_bytes(b'runtime')
        harness = gate.runner.evidence/'native-harness'; harness.write_bytes(b'harness')
        for label, path in [('runtime',runtime),('native-harness',harness)]:
            gate.runner.report['producers'][label]={'file':path.relative_to(gate.runner.evidence).as_posix(),'sha256':gate.runner.digest(path),'bytes':path.stat().st_size}
        payload = gate.files(bundle)
        manifest = {'schema':1,'payload':{'files':payload, 'runtime':{'sha256':gate.runner.digest(runtime),'bytes':7},'entry':{'game':'arena'},'profile':'arena-authored-linux-x86_64-v1','initial_checksum':'0x0000000000000001'}}
        manifest['content_digest']=hashlib.sha256(b'orrery.arena.export.content.v1\0'+json.dumps(manifest['payload'],ensure_ascii=False,separators=(',',':')).encode()).hexdigest()
        (bundle/'orr.export.json').write_text(json.dumps(manifest))
        captures=[]
        for index,name in enumerate(['initial.png','moved.png']):
            def chunk(kind,data):
                return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
            row=b'\x00'+color*32+bytes([index,0,0,255])*480
            raw=row*512
            data=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',512,512,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
            path=root/name;path.write_bytes(data)
            captures.append({'path':name,'width':512,'height':512,'sha256':gate.runner.digest(path)})
        before=gate.files(project)
        r={'schema':1,'source_head':'a'*40,'source_tree':'b'*40,'runtime':{'path':str(runtime),'sha256':gate.runner.digest(runtime)},'test_producer':{'path':str(harness),'sha256':gate.runner.digest(harness)},
           'terminal_joined_success':True,'source_hidden':True,'bundle_read_only':True,'source_preserved':True,'source_files':before,'source_files_before':before,'source_files_after':before,'bundle_files':gate.files(bundle),'captures':captures,'commands':list(gate.STAGES)}
        (root/'source-files-before.json').write_text(json.dumps(before))
        (root/'producer-receipt.json').write_text(json.dumps({k:r[k] for k in ['source_head','source_tree','runtime','test_producer']}))
        (root/'stage-outcomes.json').write_text(json.dumps({'schema':1,'source_head':'a'*40,'source_tree':'b'*40,'stage':'complete','terminal_joined_success':True}))
        commands=[]
        for stage in gate.STAGES:
            mounts=['--ro-bind','/','/','--bind',str(root),str(root),'--ro-bind',str(bundle),str(bundle),'--tmpfs',str(gate.runner.repo),'--tmpfs',str(project),'--tmpfs',str(runtime.parent)]
            hidden=[str(gate.runner.repo/'Cargo.toml'),str(project/'orr.project.json'),str(runtime)]
            if stage=='isolated-proof':
                args=mounts+['--','/bin/sh','-c',gate.PROBE_SCRIPT,'sh']+hidden+[str(bundle)]
            else:
                ticks='0' if stage=='capture-initial' else '3'
                hold='idle' if stage=='capture-initial' else 'right' if stage in ['capture-moved','relocated-move'] else 'right,fire'
                args=mounts+['--','/bin/sh','-c',gate.HIDE_SCRIPT,'sh']+hidden+[str(bundle/'run-arena'),'--headless','--ticks',ticks,'--hold',hold]
                if stage.startswith('capture-'):
                    args+=['--capture',str(root/('initial.png' if stage=='capture-initial' else 'moved.png'))]
            cmd={'stage':stage,'program':'bwrap','args':args,'cwd':str(root),'status':0,'stdout_file':stage+'.stdout.txt','stderr_file':stage+'.stderr.txt'}
            if stage=='source-move-fire':
                cmd['program']=str(runtime);cmd['args']=['--project',str(project),'--headless','--ticks','3','--hold','right,fire']
            commands.append(cmd)
            (root/(stage+'.command-state.json')).write_text(json.dumps(dict(cmd,state="completed",error=None)))
            if stage=='isolated-proof':
                text='source-hidden=true bundle-read-only=true write-denied=true\n'
            else:
                final='1' if stage=='capture-initial' else '3' if stage in ['capture-moved','relocated-move'] else '2'
                text=f'project initial checksum: 0x0000000000000001\nproject tick: {ticks} checksum: 0x000000000000000{final}\nproject entity: hero handle:0v1 position:0,0\nproject entity: peer handle:1v1 position:1,1\n'
                if stage.startswith('capture-'):
                    text+='project capture adapter: llvmpipe (software: true)\n'
            (root/cmd['stdout_file']).write_text(text);(root/cmd['stderr_file']).write_text('')
        (root/'commands.jsonl').write_text(''.join(json.dumps(c)+'\n' for c in commands))
        (root/'receipt.json').write_text(json.dumps(r))
        return root,r

    def test_exact_source_names_and_cfg(self):
        self.assertEqual(len(gate.BASELINE),24)
        self.assertEqual(len(gate.APP+gate.EDITOR+gate.NEW_APP),30)
        self.assertEqual(len(gate.SAMPLE),16)
        self.assertEqual(len(set(gate.SAMPLE)),16)
        gate.runner.repo=Path(__file__).resolve().parents[1]
        self.assertFalse(gate.structural_proof()['executed_old_new_inventory_proof'])

    def test_inventory_rejects_missing_duplicate_extra(self):
        gate.inventory(self.list_text([gate.NATIVE]))
        for output in ['',self.list_text([gate.NATIVE,gate.NATIVE]),self.list_text([gate.NATIVE]).replace('1 tests','0 tests')]:
            with self.assertRaises(AssertionError): gate.inventory(output)
        with self.assertRaises(AssertionError):gate.close.inventory(self.list_text([]),[gate.NATIVE])

    def test_results_reject_zero_ignored_failed_missing_duplicate(self):
        good=f'running 1 test\ntest {gate.NATIVE} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n'
        gate.runner.result(good,[gate.NATIVE])
        for bad in ['',good+good,good.replace('1 passed','0 passed'),good.replace('0 ignored','1 ignored'),good.replace('... ok','... FAILED'),good.replace('... ok','... ignored'),good.replace('... ok','... diagnostic'),good.replace(f'test {gate.NATIVE} ... ok\n','')]:
            with self.assertRaises(AssertionError):gate.runner.result(bad,[gate.NATIVE])

    def test_producer_rejects_stale_missing_extra_failed(self):
        item=self.artifact();good=self.cargo(item)
        for bad in ['',good+good,good.replace('"fresh": false','"fresh": true'),good.replace('"success":true','"success":false'),good.splitlines()[0]]:
            with self.assertRaises((AssertionError,ValueError)):self.pin(bad)
        self.pin(good)

    def test_producer_rejects_features_source_target_path(self):
        item=self.artifact()
        for key,value in [('features',['default']),('manifest_path',str(self.root/'wrong')),('package_id','registry+wrong'),('executable',str(self.root/'missing'))]:
            bad=copy.deepcopy(item);bad[key]=value
            with self.assertRaises(AssertionError):self.pin(self.cargo(bad))
        bad=copy.deepcopy(item);bad['target']['src_path']=str(self.root/'other.rs')
        with self.assertRaises(AssertionError):self.pin(self.cargo(bad))

    def test_receipt_requires_full_source_and_producer_binding(self):
        root,r=self.receipt();gate.verify_receipt(root)
        for key,value in [('source_head','c'*40),('terminal_joined_success',False),('source_files_after',[]),('bundle_files',[])]:
            bad=copy.deepcopy(r);bad[key]=value;(root/'receipt.json').write_text(json.dumps(bad))
            with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_receipt_missing_or_tampered_capture_fails(self):
        root,r=self.receipt();(root/'moved.png').write_bytes(b'bad')
        with self.assertRaises(AssertionError):gate.verify_receipt(root)
        (root/'receipt.json').unlink()
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_receipt_rejects_non_lavapipe_both_captures(self):
        root,r=self.receipt()
        for stage in ['capture-initial','capture-moved']:
            path=root/(stage+'.stdout.txt');original=path.read_text()
            for bad in ['project capture adapter: SwiftShader (software: true)','project capture adapter: llvmpipe (software: false)','adapter unavailable']:
                path.write_text(original.replace('project capture adapter: llvmpipe (software: true)',bad))
                with self.assertRaises(AssertionError):gate.verify_receipt(root)
            path.write_text(original)

    def test_receipt_rejects_failed_unfinished_children(self):
        root,r=self.receipt();path=root/'commands.jsonl';original=path.read_text()
        rows=[json.loads(l) for l in original.splitlines()]
        for status in [None,1,True]:
            rows[0]['status']=status;path.write_text(''.join(json.dumps(c)+'\n' for c in rows))
            with self.assertRaises(AssertionError):gate.verify_receipt(root)
        path.write_text(original)
        state_path=root/'source-move-fire.command-state.json';state=state_path.read_text()
        for bad in [{},dict(json.loads(state),state='started',status=None),dict(json.loads(state),state='wait_failed',error='failed')]:
            state_path.write_text(json.dumps(bad))
            with self.assertRaises(AssertionError):gate.verify_receipt(root)
        state_path.write_text(state)
        (root/'stage-outcomes.json').write_text('{"schema":1,"stage":"running"}')
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_receipt_rejects_source_hiding_write_proof_and_extra_files(self):
        root,r=self.receipt();path=root/'isolated-proof.stdout.txt';path.write_text('source-hidden=true')
        with self.assertRaises(AssertionError):gate.verify_receipt(root)
        path.write_text('source-hidden=true write-denied=true');(root/'bundle/extra').write_bytes(b'new')
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_census_old_subset_exact_additions_and_ignored(self):
        old=['old::test'];new=old+list(gate.SAMPLE)
        paths=[self.root/n for n in ['new','old','ignored','old-ignored']]
        for path,names in zip(paths,[new,old,[],[]]):path.write_text(self.list_text(names))
        gate.census(paths[0],1,paths[2],paths[1],paths[3])
        paths[0].write_text(self.list_text(['renamed::test']+list(gate.SAMPLE)))
        with self.assertRaises(AssertionError):gate.census(paths[0],1,paths[2],paths[1],paths[3])
        paths[0].write_text(self.list_text(new));paths[2].write_text(self.list_text([gate.SAMPLE[0]]))
        with self.assertRaises(AssertionError):gate.census(paths[0],1,paths[2])

    def test_exact_namespace_probe_and_launcher_required(self):
        root,r=self.receipt();path=root/'commands.jsonl';rows=[json.loads(line) for line in path.read_text().splitlines()]
        for index,mutate in [(1,lambda a:a[:14]),(1,lambda a:[x.replace('touch "$4/.write-probe"','true') for x in a]),(2,lambda a:a+['--bind','/tmp',str(root/'bundle')]),(3,lambda a:[x.replace('right,fire','idle') for x in a]),(6,lambda a:a[:-1]+['/tmp/wrong.png'])]:
            bad=copy.deepcopy(rows);bad[index]['args']=mutate(bad[index]['args']);path.write_text(''.join(json.dumps(c)+'\n' for c in bad))
            with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_simulation_checksums_repeat_fire_and_movement_required(self):
        root,r=self.receipt()
        for name in ['source-move-fire','relocated-fire-1','relocated-fire-2','relocated-move','capture-moved']:
            path=root/(name+'.stdout.txt');original=path.read_text();path.write_text(original.replace('checksum: 0x000000000000000','checksum: 0x111111111111111'))
            with self.assertRaises(AssertionError):gate.verify_receipt(root)
            path.write_text(original)
        path=root/'relocated-move.stdout.txt';path.write_text((root/'source-move-fire.stdout.txt').read_text())
        with self.assertRaises(AssertionError):gate.verify_receipt(root)

    def test_installed_sprite_oracle_rejects_monochrome_and_wrong_palette(self):
        root,r=self.receipt()
        for color in [bytes([0,0,0,255]),bytes([255,0,255,255])]:
            with self.assertRaises(AssertionError):gate.sprite_oracle(root/'project',color*(512*512))
        path=next((root/'project/.orr/packages/objects').glob('*/lantern_keeper.rgba'));path.write_bytes(b'changed')
        with self.assertRaises(AssertionError):gate.sprite_oracle(root/'project',bytes([0,0,0,255])*(512*512))

    def test_forged_tokens_without_checksum_records_fail(self):
        root,r=self.receipt()
        for stage in ['source-move-fire','capture-initial']:
            path=root/(stage+'.stdout.txt');original=path.read_text()
            path.write_text('project capture adapter: llvmpipe (software: true)\nsource-hidden=true write-denied=true\n')
            with self.assertRaises(AssertionError):gate.verify_receipt(root)
            path.write_text(original)

    def test_workflow_requires_both_pairs_and_preserves_matrix(self):
        text=(Path(__file__).resolve().parents[1]/'.github/workflows/determinism.yml').read_text()
        for mode in ['cpu','native']:
            self.assertIn('run: python -B tools/arena_editor_export_gate.py '+mode,text)
            self.assertIn('Retain Arena editor export '+('CPU' if mode=='cpu' else 'native')+' evidence',text)
        self.assertIn("if: always() && runner.os == 'Linux' && runner.arch == 'X64'",text)
        self.assertNotIn('continue-on-error:',text[text.index('      - name: Required Arena editor export native acceptance'):text.index('      - name: Required authored Collect CPU and admission counts')])


if __name__=='__main__':
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    assert result.wasSuccessful() and result.testsRun==gate.FIXTURE_COUNT and not result.skipped
    print(f'ARENA_EXPORT_GATE_FIXTURES: {gate.FIXTURE_COUNT} passed; 0 failed; 0 skipped',flush=True)
