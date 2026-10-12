#!/usr/bin/env python3
"""Pure/mocked maintained HUD gate controls. No payload or namespace probes."""
import ast,copy,errno,os,hashlib,importlib.util,json,pathlib,sys,tempfile,types,unittest
from unittest import mock
import yaml
sys.dont_write_bytecode=True
HERE=pathlib.Path(__file__).resolve().parent;ROOT=HERE.parent
sys.path.insert(0,str(HERE))
from hud_layer_order import contracts as c,evidence as e,producer as p,filesystem as f,owned_process as owner,finalization as z
import hud_layer_order_gate as gate

class GateContracts(unittest.TestCase):
    def test_workflow_required_exact_head(self):
        jobs=yaml.safe_load((ROOT/'.github/workflows/determinism.yml').read_text())['jobs'];job=jobs['hud-layer-order'];aggregate=jobs['cross_platform_checksum_compare']
        self.assertEqual(job['uses'],'./.github/workflows/hud-layer-order.yml')
        self.assertEqual(job['with']['expected_head'],'${{ github.event.pull_request.head.sha || github.sha }}')
        self.assertIn('hud-layer-order',aggregate['needs']);self.assertEqual(aggregate['if'],'${{ always() }}')
        step=aggregate['steps'][0];self.assertEqual(step['env']['HUD_LAYER_ORDER_RESULT'],"${{ needs['hud-layer-order'].result }}")
        self.assertIn('HUD_LAYER_ORDER_RESULT; do',step['run']);self.assertIn('"$result" != "success"',step['run'])
        workflow=yaml.safe_load((ROOT/'.github/workflows/hud-layer-order.yml').read_text());job=workflow['jobs']['combined-hud']
        self.assertEqual(job['runs-on'],'ubuntu-22.04');self.assertEqual(job['timeout-minutes'],180)
        self.assertEqual(job['steps'][0]['with']['ref'],'${{ inputs.expected_head }}')
        run=[s for s in job['steps'] if s.get('run')=='python3 tools/hud_layer_order_gate.py --run'];self.assertEqual(len(run),1)
        for value in [job,run[0]]:self.assertNotIn('if',value);self.assertNotIn('continue-on-error',value)
        self.assertNotIn('systemd',(ROOT/'.github/workflows/hud-layer-order.yml').read_text())
    def test_aggregate_accepts_only_all_success(self):
        job=yaml.safe_load((ROOT/'.github/workflows/determinism.yml').read_text())['jobs']['cross_platform_checksum_compare'];env=job['steps'][0]['env']
        self.assertEqual(len(env),7)
        for name in env:
            for state in ['failure','cancelled','skipped','',None]:
                results={key:'success' for key in env};results[name]=state
                self.assertFalse(all(value=='success' for value in results.values()))
    def test_exact_git_tree_requires_all_bytes_and_modes(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);data=b'tracked source';(root/'file').write_bytes(data)
            baseline={'file':{'bytes':len(data),'mode':0o644,'sha256':hashlib.sha256(data).hexdigest()}}
            listing='100644 blob '+hashlib.sha1(b'blob 14\0'+data).hexdigest()+'\tfile\0'
            self.assertEqual(c.source_matches_tree(root,listing,baseline)['leaves'],1)
            for bad in [listing.replace('100644','100755'),listing+listing,listing[:-1],listing.replace('file','../file')]:
                with self.assertRaises(RuntimeError):c.source_matches_tree(root,bad,baseline)
            (root/'file').write_bytes(b'tracked change')
            with self.assertRaises(RuntimeError):c.source_matches_tree(root,listing,baseline)
    def test_command_roles(self):
        self.assertEqual(c.command('clippy','orr_editor')[-3:],['--','-D','warnings'])
        self.assertIn('collect-ui,project-create',c.command('clippy','orr_editor'))
        self.assertIn('--all-targets',c.command('clippy','orr_editor'))
        cmd=c.command('build','orr_sample');self.assertEqual([cmd[i+1] for i,v in enumerate(cmd) if v=='--bin'],c.TOOLS)
    def test_exact_inventories_reject_zero_extra_duplicate(self):
        expected=c.CPU_TESTS;text=''.join(n+': test\n' for n in expected)+'3 tests, 0 benchmarks\n';e.inventory(text,expected)
        for bad in ['0 tests, 0 benchmarks\n',text+'extra: test\n',text+expected[0]+': test\n']:
            with self.assertRaises(ValueError):e.inventory(bad,expected)
        e.inventory(''.join(n+': test\n' for n in c.GPU_TESTS)+'2 tests, 0 benchmarks\n',c.GPU_TESTS)
        with self.assertRaises(ValueError):e.inventory('0 tests, 0 benchmarks\n',c.GPU_TESTS)
    def test_negative_exact_full_operands(self):
        pin=json.loads((HERE/'hud_layer_order/negative-oracle.json').read_text());name=c.GPU_MAIN
        text=f"running 1 test\ntest {name} ... FAILED\nfailures:\n---- {name} stdout ----\nthread '{name}' (4) panicked at {pin['file']}:{pin['line']}:{pin['column']}:\nassertion `left == right` failed\n {pin['diagnostics'][0]}\n {pin['diagnostics'][1]}\nfailures:\n    {name}\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 1 filtered out;\n"
        oracle={'path':pin['file'],'line':pin['line'],'column':pin['column'],'diagnostics':pin['diagnostics']};e.result(text,[name],101,1,oracle)
        for before,after in [('left: Document','left: Other'),('right: Document','right: Other'),(':5:',':6:'),('1 filtered out','0 filtered out')]:
            with self.assertRaises(ValueError):e.result(text.replace(before,after),[name],101,1,oracle)
        source=ROOT/pin['file'];self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),pin['sha256']);self.assertEqual(source.read_text().splitlines()[pin['line']-1],pin['text'])
    def test_rust_bounded_copy_and_retention_contract(self):
        source=(ROOT/'crates/orr_editor/tests/ui_layer_order_gpu_export.rs').read_text()
        self.assertNotIn('fs::copy(',source)
        self.assertIn('written <= maximum',source);self.assertLess(source.index('written <= maximum'),source.index('output.write_all(&buffer[..count])'))
        self.assertIn('self.reserve_retained(count);\n        bounded_copy(source, &destination, count);',source)
        self.assertIn('charged <= 120 * 1024 * 1024',source)
        self.assertIn('Cell::new(0)',source);self.assertIn('192 * 1024 * 1024',source)
        self.assertIn('"/tmp/user-runtime"',source)
    def test_mixed_multiline_cargo_framing(self):
        text='[foo 1.2.3] {"reason":"build-finished","success":false}\n{"reason":"build-finished","success":true}\n'
        rows,summary=p.producer_messages('test',text);self.assertEqual(rows,[{'reason':'build-finished','success':True}]);self.assertEqual(summary['counts']['verbose_build_script_lines'],1)
        command=" Running `CARGO_PKG_DESCRIPTION='line one\nline two' rustc --crate-name example src/lib.rs`\n"
        tokens=list(p.running_commands(command));self.assertEqual(len(tokens),1);self.assertIn('CARGO_PKG_DESCRIPTION=line one\nline two',tokens[0])
    def test_producer_malformed_json_and_command_rejected(self):
        for text in ['{"reason":"build-finished","reason":"forged"}\n','{"reason":NaN}\n','arbitrary output\n']:
            with self.assertRaises((RuntimeError,AssertionError)):p.producer_messages('bad',text)
        with self.assertRaises(RuntimeError):list(p.running_commands("Running `CARGO_PKG_DESCRIPTION='unterminated\n"))
    def test_real_package_id_variants(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'crates/orr_sample').mkdir(parents=True)
            (root/'Cargo.toml').write_text('[workspace.package]\nversion="0.0.1"\n')
            (root/'crates/orr_sample/Cargo.toml').write_text('[package]\nname="orr_sample"\nversion.workspace=true\n')
            (root/'Cargo.lock').write_text('version=4\n[[package]]\nname="orr_sample"\nversion="0.0.1"\n')
            ids=f.package_identities(root,['Cargo.toml','crates/orr_sample/Cargo.toml']);prefix='path+'+(root/'crates/orr_sample').as_uri()
            self.assertIn(prefix+'#0.0.1',ids);self.assertIn(prefix+'#orr_sample@0.0.1',ids)
    def test_generic_root_selection_rejects_wrong_kind_and_root(self):
        rows=[{'manifest_path':'/source/crates/orr_sample/Cargo.toml','features':c.TOOL_FEATURES,'target':{'name':n,'kind':['bin']},'profile':{'test':False},'fresh':False,'executable':'/target/'+n} for n in c.TOOLS]
        selected=lambda values:c.selected_roots(values,pathlib.Path('/source'),'orr_sample',{n:('bin',False) for n in c.TOOLS},c.TOOL_FEATURES)
        self.assertEqual(set(selected(rows)),set(c.TOOLS))
        for key,value in [('manifest_path','/source/crates/orr_editor/Cargo.toml'),('profile',{'test':True}),('fresh',True),('features',['default'])]:
            bad=copy.deepcopy(rows);bad[0][key]=value
            with self.assertRaises(RuntimeError):selected(bad)
    def hud_root_rows(self,target):
        def row(name,kind,is_test,source):
            return {'manifest_path':'/source/crates/orr_editor/Cargo.toml','features':list(c.EDITOR_FEATURES),
                    'target':{'name':name,'kind':[kind],'src_path':'/source/crates/orr_editor/'+source},
                    'profile':{'test':is_test},'fresh':False,'executable':'/target/'+name}
        return [row(target,'test',True,'tests/'+target+'.rs'),row('orr_editor','bin',False,'src/main.rs')]
    def select_hud_roots(self,rows,target):
        return c.selected_roots(rows,pathlib.Path('/source'),'orr_editor',{target:('test',True)},c.EDITOR_FEATURES)
    def test_hud_selects_only_requested_test_with_audited_companion(self):
        for target in ['ui_layer_order_acceptance','ui_layer_order_gpu_export']:
            rows=self.hud_root_rows(target)
            for values in [rows,rows[::-1],rows[:1]]:
                with self.subTest(target=target,order=[r['target']['name'] for r in values]):
                    selected=self.select_hud_roots(values,target)
                    self.assertEqual(selected,{target:rows[0]})
                    self.assertIs(selected[target],rows[0])
    def test_hud_companion_cannot_replace_missing_requested_test(self):
        target='ui_layer_order_acceptance';rows=self.hud_root_rows(target)
        for values in [[],rows[1:]]:
            with self.assertRaises(RuntimeError):self.select_hud_roots(values,target)
        rows[0]['executable']=None
        with self.assertRaisesRegex(RuntimeError,'missing selected executable'):self.select_hud_roots(rows,target)
    def test_hud_companion_rejects_wrong_identity_and_cached_output(self):
        target='ui_layer_order_acceptance';rows=self.hud_root_rows(target)
        mutations=[('manifest_path','/source/crates/other/Cargo.toml'),('features',['default']),
                   ('fresh',True),('profile',{'test':True}),('profile',{'test':0})]
        for key,value in mutations:
            bad=copy.deepcopy(rows);bad[1][key]=value
            with self.subTest(key=key,value=value),self.assertRaises(RuntimeError):self.select_hud_roots(bad,target)
        for key,value in [('name','arbitrary'),('kind',['test']),('kind',['bin','test']),
                          ('src_path','/source/crates/orr_editor/src/other.rs'),
                          ('src_path','/source/crates/orr_editor/src/../src/main.rs')]:
            bad=copy.deepcopy(rows);bad[1]['target'][key]=value
            with self.subTest(key=key,value=value),self.assertRaises(RuntimeError):self.select_hud_roots(bad,target)
        with self.assertRaisesRegex(RuntimeError,'duplicate/cached auxiliary'):self.select_hud_roots(rows+[copy.deepcopy(rows[1])],target)
    def test_hud_requested_test_still_rejects_wrong_identity_and_duplicates(self):
        target='ui_layer_order_acceptance';rows=self.hud_root_rows(target)
        for key,value in [('fresh',True),('profile',{'test':False}),('profile',{'test':1}),('features',['default']),
                          ('manifest_path','/source/crates/other/Cargo.toml')]:
            bad=copy.deepcopy(rows);bad[0][key]=value
            with self.subTest(key=key,value=value),self.assertRaises(RuntimeError):self.select_hud_roots(bad,target)
        bad=copy.deepcopy(rows);bad[0]['target']['kind']=['bin']
        with self.assertRaisesRegex(RuntimeError,'wrong target kind/test profile'):self.select_hud_roots(bad,target)
        with self.assertRaisesRegex(RuntimeError,'duplicate/cached selected'):self.select_hud_roots(rows+[copy.deepcopy(rows[0])],target)
    def test_hud_companion_exception_is_not_generic_extra_executable_permission(self):
        for target in ['unrequested_test','ui_layer_order_acceptance']:
            rows=self.hud_root_rows(target)
            selections={target:('test',True)}
            if target=='ui_layer_order_acceptance':selections['another_test']=('test',True)
            with self.assertRaisesRegex(RuntimeError,'unexpected executable'):
                c.selected_roots(rows,pathlib.Path('/source'),'orr_editor',selections,c.EDITOR_FEATURES)
        rows=[{'manifest_path':'/source/crates/orr_sample/Cargo.toml','features':c.TOOL_FEATURES,
               'target':{'name':n,'kind':['bin']},'profile':{'test':False},'fresh':False,
               'executable':'/target/'+n} for n in [*c.TOOLS,'orr_editor']]
        with self.assertRaisesRegex(RuntimeError,'unexpected executable'):
            c.selected_roots(rows,pathlib.Path('/source'),'orr_sample',{n:('bin',False) for n in c.TOOLS},c.TOOL_FEATURES)
    def test_hud_audit_and_attribution_precede_selection_and_only_selected_rows_are_pinned(self):
        source=(HERE/'hud_layer_order_gate.py').read_text()
        self.assertLess(source.index('rows=audit.audit_producer('),source.index('chosen=c.selected_roots('))
        self.assertLess(source.index('attribute(name,stdout)'),source.index('chosen=c.selected_roots('))
        self.assertIn('for key,row in chosen.items():',source)
    def test_archive_full_equality_and_corruption(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);src=root/'producer';src.write_bytes(b'exact toy binary'*100);dest=root/'producer.gz';receipt=e.archive(src,dest)
            e.readback(dest,receipt['raw_sha256'],receipt['raw_size'])
            with self.assertRaises(ValueError):e.readback(dest,'0'*64,receipt['raw_size'])
            dest.write_bytes(dest.read_bytes()[:-4])
            with self.assertRaises((ValueError,EOFError,OSError)):e.readback(dest,receipt['raw_sha256'],receipt['raw_size'])
    def test_restored_identity_never_timestamp_only(self):
        positive={'bytes':100,'sha256':'a'*64};c.restored_identity(positive,dict(positive))
        with self.assertRaises(RuntimeError):c.restored_identity(positive,{'bytes':100,'sha256':'b'*64})
    def test_helper_requires_name_outcome_and_filtered_count(self):
        text=f'ORR_LAYER_ORDER_ISOLATION_OK\nrunning 1 test\ntest {c.GPU_HELPER} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;\n';c.helper_result(text)
        for before,after in [('running 1','running 0'),(c.GPU_HELPER,'wrong'),('... ok','... ignored'),('1 filtered','0 filtered')]:
            with self.assertRaises(RuntimeError):c.helper_result(text.replace(before,after))
    def test_helper_rejects_every_extra_malformed_frame(self):
        text=f'ORR_LAYER_ORDER_ISOLATION_OK\nrunning 1 test\ntest {c.GPU_HELPER} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;\n'
        for extra in ['test unexpected ... ok malformed', ' test unexpected ... ok', 'test unexpected ... ok', 'running 1 test malformed', ' running 1 test', 'running 1 test', 'test result: malformed', ' test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;']:
            with self.subTest(extra=extra),self.assertRaises(RuntimeError):c.helper_result(text+extra+'\n')
    def test_helper_rejects_stderr_panic_on_success(self):
        text=f'ORR_LAYER_ORDER_ISOLATION_OK\nrunning 1 test\ntest {c.GPU_HELPER} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;\n'
        with self.assertRaises(RuntimeError):c.helper_result(text,"thread 'other' panicked at wrong.rs:1:1:")
        source=(HERE/'hud_layer_order/contracts.py').read_text()
        self.assertIn("helper_result((root/'07-relocated-proof.stdout.txt').read_text(),(root/'07-relocated-proof.stderr.txt').read_text())",source)
    def test_rust_private_runtime_leaf_checks_owner_and_mode(self):
        source=(ROOT/'crates/orr_editor/tests/ui_layer_order_gpu_export.rs').read_text()
        self.assertIn('fs::DirBuilder::new().mode(0o700).create(&path)',source)
        self.assertIn('metadata.file_type().is_dir()',source)
        self.assertIn('metadata.uid(), profile_owner',source)
        self.assertIn('metadata.mode() & 0o7777, 0o700',source)
        self.assertLess(source.index('"user-runtime"]'),source.index('metadata.uid(), profile_owner'))
    def test_copy_budget_runtime_twice(self):
        values={n:{'bytes':4097,'sha256':'a'*64} for n in ['generator','runtime','exporter','acceptance']};self.assertEqual(sum(c.fixture_budget(values)['copies'].values()),5*8192)
    def test_evidence_nested_archive_bypass_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'nested').mkdir();(root/'nested/a.elf.gz').write_bytes(b'x')
            with self.assertRaises(RuntimeError):c.evidence_usage(root)
    def test_no_local_execution(self):
        with mock.patch.object(sys,'argv',['gate','--run']),mock.patch.dict('os.environ',{'GITHUB_ACTIONS':'false'}),self.assertRaises(RuntimeError):gate.main()
    def test_owner_kernel_empty_not_a_poll_guess(self):
        for value in [None,types.SimpleNamespace(si_pid=1)]:
            with mock.patch.object(owner.os,'waitid',return_value=value):self.assertFalse(owner.kernel_no_children())
        with mock.patch.object(owner.os,'waitid',side_effect=ChildProcessError):self.assertTrue(owner.kernel_no_children())
        with mock.patch.object(owner.os,'waitid',side_effect=PermissionError),self.assertRaises(PermissionError):owner.kernel_no_children()
    def test_owner_reparented_and_new_session_descendants(self):
        # A synthetic escaped-session parent owns a child. Reaping it adopts that
        # child; each signal is pidfd+kernel-child owned before eventual ECHILD.
        children={52:True};signals=[];records=[]
        def wait(kind,identifier,flags):
            if kind==owner.os.P_ALL:
                if not children:raise ChildProcessError()
                return None
            pid=identifier-1000
            if pid not in children:raise ChildProcessError()
            if children[pid]:return None
            result=types.SimpleNamespace(si_pid=pid,si_code=2,si_status=9)
            if not flags & owner.os.WNOWAIT:
                del children[pid]
                if pid==52:children[53]=True
            return result
        def send(fd,sig):signals.append((fd,sig));children[fd-1000]=False
        with mock.patch.object(owner.os,'waitid',side_effect=wait),mock.patch.object(owner,'adopted_ids',side_effect=lambda:list(children)),mock.patch.object(owner.os,'pidfd_open',side_effect=lambda pid:pid+1000),mock.patch.object(owner.os,'close'),mock.patch.object(owner.signal,'pidfd_send_signal',side_effect=send),mock.patch.object(owner.time,'sleep'):
            owner.reap_descendants(owner.time.monotonic()+2,records)
        self.assertEqual([fd for fd,_ in signals],[1052,1053]);self.assertFalse(children);self.assertEqual(sum('reaped_pid' in r for r in records),2)
    def test_owner_stale_pid_never_signaled(self):
        def wait(kind,identifier,flags):
            if kind==owner.os.P_ALL:return None
            raise ChildProcessError('no longer an owned child')
        with mock.patch.object(owner.os,'waitid',side_effect=wait),mock.patch.object(owner,'adopted_ids',return_value=[52]),mock.patch.object(owner.os,'pidfd_open',return_value=1052),mock.patch.object(owner.os,'close'),mock.patch.object(owner.signal,'pidfd_send_signal') as send,self.assertRaises(owner.OwnershipUnknown):
            owner.reap_descendants(owner.time.monotonic()+2,[])
        send.assert_not_called()
    def test_owner_live_descendant_timeout_never_quiescence(self):
        with mock.patch.object(owner,'kernel_no_children',return_value=False),mock.patch.object(owner,'adopted_ids',return_value=[]),mock.patch.object(owner.time,'monotonic',side_effect=[0,2]),mock.patch.object(owner.time,'sleep'),self.assertRaises(owner.OwnershipUnknown):owner.reap_descendants(1,[])
    def test_owner_cleanup_exception_propagates(self):
        with mock.patch.object(owner,'kernel_no_children',return_value=False),mock.patch.object(owner,'adopted_ids',side_effect=PermissionError),self.assertRaises(PermissionError):owner.reap_descendants(owner.time.monotonic()+1,[])
    def test_owner_source_does_not_claim_aggregate_memory(self):
        source=(HERE/'hud_layer_order/owned_process.py').read_text();self.assertIn("'aggregate_memory_hard_limit':False",source);self.assertIn('os.WNOWAIT',source);self.assertNotIn('systemd',source)
    def test_release_locks_are_retained_audited_metadata(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'release').mkdir()
            for lock in f.CARGO_RELEASE_LOCKS:(root/lock).touch(mode=0o600)
            for hash_content in [False,True]:
                after,_=f.disk_inventory(root,hash_content=hash_content)
                delta=f.attribute_output_changes({},after,[],root,{})
                self.assertEqual(set(delta['changed']),f.CARGO_RELEASE_LOCKS)
                self.assertEqual(delta['removed'],[]);self.assertEqual(delta['classes'],['cargo-metadata'])
                for lock in f.CARGO_RELEASE_LOCKS:
                    entry=delta['changed'][lock]
                    self.assertEqual(entry['after'],after[lock]);self.assertIsNone(entry['before'])
                    self.assertEqual(entry['owner']['metadata'],'empty-release-lock')
                    self.assertEqual(entry['owner']['sha256'],hashlib.sha256(b'').hexdigest())
                    self.assertEqual(entry['owner']['uid'],f.os.geteuid());self.assertEqual(entry['owner']['nlink'],1)
                self.assertEqual(f.attribute_output_changes(after,after,[],root,{})['changed'],{})
    def test_release_locks_reject_nonempty_and_unsafe_modes(self):
        for lock in f.CARGO_RELEASE_LOCKS:
            for content,mode in [(b'payload',0o600),(b'',0o755),(b'',0o666),(b'',0o400),(b'',0o4644)]:
                with self.subTest(lock=lock,mode=mode),tempfile.TemporaryDirectory() as name:
                    root=pathlib.Path(name);(root/'release').mkdir();path=root/lock
                    path.write_bytes(content);path.chmod(mode);after,_=f.disk_inventory(root)
                    with self.assertRaisesRegex(RuntimeError,'must be empty'):
                        f.attribute_output_changes({},after,[],root,{})
    def test_release_locks_reject_aliases_special_files_and_hardlinks(self):
        for kind in ['symlink','hardlink','directory','fifo']:
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as name:
                root=pathlib.Path(name);(root/'release').mkdir();path=root/'release/.cargo-build-lock'
                other=root/'other';other.touch(mode=0o600)
                if kind=='symlink':path.symlink_to(other)
                elif kind=='hardlink':f.os.link(other,path)
                elif kind=='directory':path.mkdir()
                else:f.os.mkfifo(path)
                with self.assertRaises(RuntimeError):f.cargo_release_lock_owner(root,'release/.cargo-build-lock',{})
                if kind in ['symlink','fifo']:
                    with self.assertRaises(RuntimeError):f.disk_inventory(root)
    def test_release_locks_reject_parent_alias(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'actual').mkdir();(root/'release').symlink_to(root/'actual',target_is_directory=True)
            (root/'actual/.cargo-build-lock').touch()
            with self.assertRaisesRegex(RuntimeError,'path alias'):
                f.cargo_release_lock_owner(root,'release/.cargo-build-lock',{})
    def test_release_locks_reject_foreign_owner_and_parent(self):
        for foreign in ['target','release','lock']:
            with self.subTest(foreign=foreign),tempfile.TemporaryDirectory() as name:
                root=pathlib.Path(name);(root/'release').mkdir();path=root/'release/.cargo-build-lock';path.touch(mode=0o600)
                after,_=f.disk_inventory(root);original=pathlib.Path.lstat
                changed={'target':root,'release':root/'release','lock':path}[foreign]
                def foreign_stat(p):
                    result=original(p)
                    if p==changed:
                        values={key:getattr(result,key) for key in dir(result) if key.startswith('st_')}
                        values['st_uid']=f.os.geteuid()+1;return types.SimpleNamespace(**values)
                    return result
                with mock.patch.object(pathlib.Path,'lstat',foreign_stat),self.assertRaisesRegex(RuntimeError,'owner|owned'):
                    f.attribute_output_changes({},after,[],root,{})
    def test_release_locks_reject_inventory_forgery_or_staleness(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'release').mkdir();lock='release/.cargo-build-lock';path=root/lock;path.touch(mode=0o600)
            after,_=f.disk_inventory(root)
            for key,value in [('bytes',1),('mode',0o755),('allocated',999),('device','bad'),('inode','bad'),('mtime_ns','bad'),('uid',-1),('nlink',2),('sha256','0'*64)]:
                with self.subTest(key=key):
                    forged=copy.deepcopy(after);forged[lock][key]=value
                    with self.assertRaisesRegex(RuntimeError,'inventory|hash'):
                        f.attribute_output_changes({},forged,[],root,{})
            path.write_bytes(b'changed after inventory')
            with self.assertRaisesRegex(RuntimeError,'must be empty'):
                f.attribute_output_changes({},after,[],root,{})
    def test_release_locks_validate_even_unchanged_and_emitted_names(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'release').mkdir();lock='release/.cargo-build-lock';path=root/lock;path.write_bytes(b'payload')
            after,_=f.disk_inventory(root)
            row={'reason':'compiler-artifact','package_id':'known','filenames':[str(path)],'target':{'name':'test','kind':['lib']}}
            for before,rows in [(after,[]),({},[row])]:
                with self.assertRaisesRegex(RuntimeError,'must be empty'):
                    f.attribute_output_changes(before,after,rows,root,{'known':{'package':'test','class':'local'}})
    def test_release_locks_do_not_allow_unknown_names_locations_or_removal(self):
        for lock in ['.cargo-build-lock','debug/.cargo-build-lock','release/.cargo-build-lock.bak','release/.cargo-other-lock','release/deps/.cargo-build-lock','other/release/.cargo-build-lock']:
            with self.subTest(lock=lock),tempfile.TemporaryDirectory() as name:
                root=pathlib.Path(name);path=root/lock;path.parent.mkdir(parents=True,exist_ok=True);path.touch(mode=0o600)
                after,_=f.disk_inventory(root)
                with self.assertRaisesRegex(RuntimeError,'unrecognized retained Cargo output'):
                    f.attribute_output_changes({},after,[],root,{})
        for lock in f.CARGO_RELEASE_LOCKS:
            with self.assertRaisesRegex(RuntimeError,'unrecognized target removal'):
                f.attribute_output_changes({lock:{}},{},[],pathlib.Path('/target'),{})
    def test_release_locks_do_not_mask_unknown_payload(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'release').mkdir();(root/'release/.cargo-build-lock').touch(mode=0o600);(root/'release/unknown').write_bytes(b'unowned')
            after,_=f.disk_inventory(root)
            with self.assertRaisesRegex(RuntimeError,'unrecognized retained Cargo output change: release/unknown'):
                f.attribute_output_changes({},after,[],root,{})
    def test_release_lock_inventory_tracks_owner_and_links_without_hashing(self):
        with tempfile.TemporaryDirectory() as name:
            root=pathlib.Path(name);(root/'release').mkdir();lock='release/.cargo-build-lock';path=root/lock;path.touch(mode=0o600)
            before,_=f.disk_inventory(root,hash_content=False);f.os.link(path,root/'release/external-link')
            after,_=f.disk_inventory(root,hash_content=False)
            self.assertEqual(before[lock]['nlink'],1);self.assertEqual(after[lock]['nlink'],2)
            with self.assertRaisesRegex(RuntimeError,'owner/link count'):
                f.attribute_output_changes(before,{lock:after[lock]},[],root,{})
    def test_all_python_parses(self):
        for path in [HERE/'hud_layer_order_gate.py',*list((HERE/'hud_layer_order').glob('*.py'))]:ast.parse(path.read_text())

class TargetMutationDiagnostics(unittest.TestCase):
    def entry(self):
        return dict(bytes=10,mode=0o755,allocated=4096,device='1',inode='2',mtime_ns='3',uid=1001,nlink=1)
    def reject(self,before,after,save=None):
        writer=save if save is not None else mock.Mock();record={}
        original=copy.deepcopy((before,after))
        with self.assertRaisesRegex(RuntimeError,'^nonproducer target mutation:') as caught:
            gate.require_target_inventory(before,after,8192,writer,record)
        self.assertEqual((before,after),original)
        return writer,record['target_mutation'],str(caught.exception)
    def test_equal_is_unchanged_and_writes_nothing(self):
        before={'release/tool':self.entry()};writer=mock.Mock();record={}
        self.assertIsNone(gate.require_target_inventory(before,copy.deepcopy(before),4096,writer,record))
        writer.assert_not_called();self.assertEqual(record,{})
    def test_every_inventory_field_still_fails_and_is_named(self):
        for field in self.entry():
            with self.subTest(field=field):
                before={'release/tool':self.entry()};after=copy.deepcopy(before)
                old=after['release/tool'][field];after['release/tool'][field]=str(int(old)+1) if isinstance(old,str) else old+1
                writer,summary,_=self.reject(before,after)
                self.assertEqual(summary['rows'][0]['fields'],{field:{'before':old,'after':after['release/tool'][field]}})
                self.assertEqual(writer.call_args_list,[mock.call('target-failure-after.json',{'files':after,'allocated':8192}),
                    mock.call('target-failure-before.json',before),mock.call('target-failure-delta.json',summary)])
    def test_added_removed_and_missing_fields_still_fail(self):
        before={'removed':self.entry(),'changed':self.entry()};after={'added':self.entry(),'changed':self.entry()}
        del after['changed']['uid']
        _,summary,_=self.reject(before,after)
        self.assertEqual([summary[k] for k in ['changed_paths','added_paths','removed_paths']],[3,1,1])
        self.assertEqual([row['path'] for row in summary['rows']],['added','changed','removed'])
        self.assertIn('uid',summary['rows'][1]['fields'])
    def test_path_count_and_byte_caps_do_not_drop_full_snapshot(self):
        after={f'path-{i:03}':self.entry() for i in range(100)}
        after['long-'+'x'*10000]=self.entry()
        writer,summary,message=self.reject({},after)
        self.assertLessEqual(len(summary['rows']),16)
        self.assertEqual(summary['changed_paths'],101)
        self.assertEqual(summary['omitted_paths'],101-len(summary['rows']))
        self.assertLessEqual(len(gate.json_bytes(summary)),8192)
        self.assertLessEqual(len(message.encode()),8192)
        self.assertEqual(writer.call_args_list[0].args[1]['files'],after)
    def test_unicode_paths_and_all_write_errors_respect_full_document_cap(self):
        after={f'{i:03}-'+'한'*40:self.entry() for i in range(40)}
        for error in [OSError('full'),type('한'*100,(Exception,),{})(),type('𐐀'*64,(Exception,),{})()]:
            writer=mock.Mock(side_effect=error)
            _,summary,message=self.reject({},after,writer)
            self.assertEqual(writer.call_count,3)
            self.assertEqual(summary['changed_paths'],40)
            self.assertEqual(summary['omitted_paths'],40-len(summary['rows']))
            self.assertLessEqual(len(gate.json_bytes(summary)),8192)
            self.assertLessEqual(len(message.encode()),8192)
            self.assertEqual(writer.call_args_list[0].args[1]['files'],after)
            self.assertTrue(all(len(value)<=64 for value in summary['diagnostic_write_errors'].values()))
    def test_short_unicode_paths_cap_delta_write_and_final_record(self):
        after={str(n)+'界':self.entry() for n in range(40)}
        for fail_delta in [False,True]:
            snapshots=[]
            def save(name,value):
                snapshots.append((name,copy.deepcopy(value)))
                if name!='target-failure-delta.json' or fail_delta:raise OSError('full')
            _,summary,message=self.reject({},after,save)
            self.assertEqual(len(snapshots),3)
            self.assertEqual(snapshots[-1][0],'target-failure-delta.json')
            self.assertLessEqual(len(gate.json_bytes(snapshots[-1][1])),8192)
            self.assertLessEqual(len(gate.json_bytes(summary)),8192)
            self.assertLessEqual(len(message.encode()),8192)
            self.assertEqual(len(summary['diagnostic_write_errors']),3 if fail_delta else 2)
    def test_supplementary_unicode_errors_fit_with_short_ascii_paths(self):
        after={str(n):self.entry() for n in range(40)}
        writer=mock.Mock(side_effect=type('𐐀'*64,(Exception,),{})())
        _,summary,message=self.reject({},after,writer)
        self.assertLessEqual(len(gate.json_bytes(summary)),8192)
        self.assertLessEqual(len(message.encode()),8192)
        self.assertEqual(writer.call_count,3)
    def test_write_errors_preserve_failure_and_attempt_all_snapshots(self):
        for error in [OSError('disk full'),FileExistsError('exists'),RuntimeError('evidence allocation cap')]:
            with self.subTest(error=type(error).__name__):
                writer=mock.Mock(side_effect=error)
                _,summary,message=self.reject({}, {'release/tool':self.entry()},writer)
                self.assertEqual(writer.call_count,3)
                self.assertEqual(set(summary['diagnostic_write_errors'].values()),{type(error).__name__})
                self.assertIn('nonproducer target mutation',message)
                self.assertNotIn(str(error),message)
    def test_one_write_error_does_not_prevent_remaining_evidence(self):
        writer=mock.Mock(side_effect=[OSError('full'),None,None])
        _,summary,_=self.reject({}, {'release/tool':self.entry()},writer)
        self.assertEqual(writer.call_count,3)
        self.assertEqual(summary['diagnostic_write_errors'],{'target-failure-after.json':'OSError'})
    def test_nested_controller_fence_rejects_before_hash_or_command(self):
        tree=ast.parse((HERE/'hud_layer_order_gate.py').read_text())
        fence=next(node for node in ast.walk(tree) if isinstance(node,ast.FunctionDef) and node.name=='target_fence')
        before={'release/tool':self.entry()};after=copy.deepcopy(before);after['release/tool']['allocated']=8192
        inventory=mock.Mock(return_value=(after,8192));digest=mock.Mock();writer=mock.Mock();record={}
        namespace={'fs':types.SimpleNamespace(disk_inventory=inventory),'target':pathlib.Path('/never-read'),
            'target_state':before,'save':writer,'record':record,'retained_hashes':{'release/tool':'digest'},
            'sha':digest,'require':owner.require,'require_target_inventory':gate.require_target_inventory}
        exec(compile(ast.Module(body=[fence],type_ignores=[]),'<controller-fence>','exec'),namespace)
        command=mock.Mock()
        with self.assertRaisesRegex(RuntimeError,'nonproducer target mutation'):
            namespace['target_fence']();command()
        inventory.assert_called_once_with(pathlib.Path('/never-read'),hash_content=False)
        digest.assert_not_called();command.assert_not_called()
        self.assertEqual(record['target_mutation']['rows'][0]['fields']['allocated'],{'before':4096,'after':8192})

class ProducerFinalization(unittest.TestCase):
    """Small files, mocked fsync only; no ELF is executed."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=pathlib.Path(self.temp.name);(self.root/'release/deps').mkdir(parents=True)
        data=bytearray(256);data[:7]=b'\x7fELF\x02\x01\x01'
        data[16:20]=b'\x03\x00\x3e\x00';data[52:54]=b'\x40\x00'
        self.data=bytes(data);self.path=self.root/'release/deps/test-fixture'
        self.path.write_bytes(self.data);self.path.chmod(0o700)
    def request(self,paired=False,mode='finalize'):
        aliases=['release/deps/test-fixture'];role='test'
        if paired:
            alias=self.root/'release/tool';os.link(self.path,alias);aliases.append('release/tool');role='canonical-bin'
        return {'version':1,'mode':mode,'phase':'fixture','root':str(self.root),
            'root_identity':z.root_identity(self.root.lstat()),'groups':[{'aliases':aliases,'role':role,
            'stat':z.snapshot(self.path.lstat()),'sha256':hashlib.sha256(self.data).hexdigest()}]}
    def execute(self,request,**kwargs):
        with mock.patch.object(z.os,'fsync',**kwargs) as flush:
            result=z.execute(request)
        return result,flush
    def nested(self,name,namespace):
        tree=ast.parse((HERE/'hud_layer_order_gate.py').read_text())
        node=copy.deepcopy(next(n for n in ast.walk(tree) if isinstance(n,ast.FunctionDef) and n.name==name))
        node.body=[ast.copy_location(ast.Global(names=n.names),n) if isinstance(n,ast.Nonlocal) else n for n in node.body]
        exec(compile(ast.fix_missing_locations(ast.Module(body=[node],type_ignores=[])),'<nested-controller>','exec'),namespace)
        return namespace[name]
    def test_single_link_has_two_hash_passes_and_one_api_call(self):
        request=self.request()
        with mock.patch.object(z,'hash_fd',wraps=z.hash_fd) as digest:
            result,flush=self.execute(request)
        self.assertTrue(result['ok']);self.assertEqual(digest.call_count,2);flush.assert_called_once()
        group=result['groups'][0];self.assertEqual(group['sha256'],request['groups'][0]['sha256'])
        self.assertEqual(group['before'],group['after']);self.assertEqual(group['fsync_api_calls'],1)
    def test_two_aliases_share_one_flush_and_descriptors_close(self):
        request=self.request(True);fds=[];real_open=os.open
        def opened(*args,**kwargs):
            fd=real_open(*args,**kwargs);fds.append(fd);return fd
        with mock.patch.object(z.os,'open',side_effect=opened):result,flush=self.execute(request)
        self.assertTrue(result['ok']);flush.assert_called_once();self.assertEqual(len(result['groups'][0]['aliases']),2)
        for fd in fds:
            with self.assertRaises(OSError):os.fstat(fd)
    def test_schema_and_resource_caps_reject_before_io(self):
        request=self.request();cases=[]
        for key,value in [('version',True),('mode','repair'),('phase','../bad'),('root','relative'),('groups',[])]:
            bad=copy.deepcopy(request);bad[key]=value;cases.append(bad)
        for key,value in [('bytes',z.ELF_CAP+1),('uid',os.geteuid()+1),('mode',0o600),('nlink',2),('device','-1')]:
            bad=copy.deepcopy(request);bad['groups'][0]['stat'][key]=value;cases.append(bad)
        for alias in ['.','/absolute','../escape','release/./x','release//x','', 'x'*513]:
            bad=copy.deepcopy(request);bad['groups'][0]['aliases']=[alias];cases.append(bad)
        bad=copy.deepcopy(request);bad['groups']*=9;cases.append(bad)
        bad=copy.deepcopy(request);bad['groups'][0]['sha256']='wrong';cases.append(bad)
        for bad in cases:
            with self.subTest(bad=bad),mock.patch.object(z.os,'open') as opened,mock.patch.object(z.os,'fsync') as flush:
                with self.assertRaises((RuntimeError,TypeError)):z.execute(bad)
                opened.assert_not_called();flush.assert_not_called()
    def test_combined_byte_cap_and_duplicate_closure_reject(self):
        request=self.request();groups=[]
        for index in range(5):
            item=copy.deepcopy(request['groups'][0]);item['stat'].update(bytes=z.ELF_CAP,inode=str(index+1))
            item['aliases']=[f'release/deps/test-{index}'];groups.append(item)
        request['groups']=groups
        with self.assertRaisesRegex(RuntimeError,'closure byte/alias bound'):z.validate_request(request)
        request=self.request();request['groups']*=2
        with self.assertRaisesRegex(RuntimeError,'duplicate alias'):z.validate_request(request)
    def test_extra_external_hardlink_rejects_before_read_or_flush(self):
        request=self.request();os.link(self.path,self.root/'external')
        with mock.patch.object(z,'hash_fd') as digest:result,flush=self.execute(request)
        self.assertFalse(result['ok']);self.assertEqual(result['errors'][0]['phase'],'alias-open')
        digest.assert_not_called();flush.assert_not_called()
    def test_missing_alias_and_symlink_leaf_reject_before_read(self):
        for kind in ['missing','symlink']:
            with self.subTest(kind=kind):
                request=self.request();self.path.unlink()
                if kind=='symlink':self.path.symlink_to(self.root/'other')
                with mock.patch.object(z,'hash_fd') as digest:result,flush=self.execute(request)
                self.assertFalse(result['ok']);digest.assert_not_called();flush.assert_not_called()
                if self.path.is_symlink():self.path.unlink()
                self.path.write_bytes(self.data);self.path.chmod(0o700)
    def test_symlink_intermediate_and_fifo_reject_before_read(self):
        request=self.request();parent=self.path.parent;parent.rename(parent.with_name('real-deps'))
        parent.symlink_to(parent.with_name('real-deps'),target_is_directory=True)
        with mock.patch.object(z,'hash_fd') as digest:result,flush=self.execute(request)
        self.assertFalse(result['ok']);digest.assert_not_called();flush.assert_not_called()
        parent.unlink();parent.with_name('real-deps').rename(parent)
        request=self.request();self.path.unlink();os.mkfifo(self.path)
        with mock.patch.object(z,'hash_fd') as digest:result,flush=self.execute(request)
        self.assertFalse(result['ok']);digest.assert_not_called();flush.assert_not_called()
    def test_parent_swap_after_open_fails_and_never_flushes(self):
        request=self.request();real=z.Descriptors.prove_paths;swapped=False
        def swap(descriptors):
            nonlocal swapped
            if not swapped:
                parent=self.path.parent;parent.rename(parent.with_name('old-deps'));parent.mkdir();swapped=True
            return real(descriptors)
        with mock.patch.object(z.Descriptors,'prove_paths',swap),mock.patch.object(z,'hash_fd') as digest:
            result,flush=self.execute(request)
        self.assertFalse(result['ok']);digest.assert_not_called();flush.assert_not_called()
    def test_wrong_preflush_hash_fails_before_flush(self):
        request=self.request();request['groups'][0]['sha256']='0'*64
        result,flush=self.execute(request)
        self.assertFalse(result['ok']);self.assertEqual(result['errors'][0]['phase'],'pre-hash');flush.assert_not_called()
    def test_non_elf_and_truncated_stream_reject(self):
        request=self.request();self.path.write_bytes(b'x'*256)
        request['groups'][0].update(stat=z.snapshot(self.path.lstat()),sha256=hashlib.sha256(b'x'*256).hexdigest())
        result,flush=self.execute(request);self.assertFalse(result['ok']);flush.assert_not_called()
        self.path.write_bytes(self.data);request=self.request()
        with mock.patch.object(z.os,'read',return_value=b''):result,flush=self.execute(request)
        self.assertFalse(result['ok']);flush.assert_not_called()
    def test_same_size_content_change_during_flush_rejects(self):
        request=self.request();mtime=self.path.stat().st_mtime_ns
        def changed(fd):
            self.path.write_bytes(self.data[:-1]+b'x');os.utime(self.path,ns=(mtime,mtime))
        result,flush=self.execute(request,side_effect=changed)
        self.assertFalse(result['ok']);self.assertEqual(result['errors'][0]['phase'],'post-hash');flush.assert_called_once()
    def test_every_identity_field_change_during_flush_rejects(self):
        for field in z.STAT_KEYS-{'allocated'}:
            with self.subTest(field=field):
                request=self.request();flushed=False;real=z.snapshot
                def snapshot(info):
                    value=real(info)
                    if flushed:
                        old=value[field];value[field]=str(int(old)+1) if isinstance(old,str) else old+1
                    return value
                def flush(fd):
                    nonlocal flushed
                    flushed=True
                with mock.patch.object(z,'snapshot',side_effect=snapshot):result,called=self.execute(request,side_effect=flush)
                self.assertFalse(result['ok']);called.assert_called_once()
    def test_allocation_only_evolution_is_receipted_then_exactly_fenced(self):
        request=self.request(True);before,allocated=f.disk_inventory(self.root,hash_content=False)
        flushed=False;real=z.snapshot
        def snapshot(info):
            value=real(info)
            if flushed:value['allocated']+=4096
            return value
        def flush(fd):
            nonlocal flushed
            flushed=True
        with mock.patch.object(z,'snapshot',side_effect=snapshot):
            result,called=self.execute(request,side_effect=flush)
            after=copy.deepcopy(before)
            for alias in request['groups'][0]['aliases']:after[alias]['allocated']+=4096
            gate.verify_finalization(request,result,before,allocated,after,allocated+4096,self.root)
        self.assertTrue(result['ok']);called.assert_called_once();self.assertFalse(result['groups'][0]['allocation_stable'])
        with self.assertRaisesRegex(RuntimeError,'nonproducer target mutation'):
            gate.require_target_inventory(before,after,allocated+4096,mock.Mock(),{})
    def test_fsync_errors_surface_once_and_close_every_fd(self):
        for code in [errno.EIO,errno.ENOSPC,errno.EDQUOT,errno.EROFS,errno.EINVAL,errno.EBADF,errno.EINTR]:
            with self.subTest(code=code),mock.patch.object(z.Descriptors,'close',autospec=True,side_effect=z.Descriptors.close) as closed:
                result,flush=self.execute(self.request(),side_effect=OSError(code,'mock'))
                self.assertFalse(result['ok']);flush.assert_called_once();closed.assert_called_once()
                self.assertEqual(result['errors'][0]['phase'],'fsync');self.assertEqual(result['errors'][0]['errno'],code)
    def test_read_and_close_errors_fail_closed(self):
        with mock.patch.object(z,'hash_fd',side_effect=OSError(errno.EIO,'read')):result,flush=self.execute(self.request())
        self.assertFalse(result['ok']);flush.assert_not_called()
        close=os.close;first=True
        def failed(fd):
            nonlocal first
            close(fd)
            if first:first=False;raise OSError(errno.EIO,'close')
        with mock.patch.object(z.os,'close',side_effect=failed):result,flush=self.execute(self.request())
        self.assertFalse(result['ok']);self.assertEqual(result['errors'][-1]['phase'],'close');flush.assert_called_once()
    def test_diagnosis_hashes_once_reports_mismatch_without_fsync(self):
        request=self.request(True,mode='diagnose');request['groups'][0]['sha256']='0'*64
        with mock.patch.object(z,'hash_fd',wraps=z.hash_fd) as digest:result,flush=self.execute(request)
        self.assertTrue(result['ok']);self.assertFalse(result['groups'][0]['hash_matches']);digest.assert_called_once();flush.assert_not_called()
        self.assertEqual(result['groups'][0]['fsync_api_calls'],0)
    def test_request_hash_symlink_and_cap_fail_before_execute(self):
        request=self.request();path=self.root/'request.json';data=gate.json_bytes(request);path.write_bytes(data)
        expected=hashlib.sha256(data).hexdigest();self.assertEqual(z.read_request(str(path),expected),request)
        with self.assertRaisesRegex(RuntimeError,'request bytes differ'):z.read_request(str(path),'0'*64)
        alias=self.root/'alias.json';alias.symlink_to(path)
        with self.assertRaises(OSError):z.read_request(str(alias),expected)
        path.write_bytes(b'x'*(z.JSON_CAP+1))
        with self.assertRaisesRegex(RuntimeError,'request ownership/size'):z.read_request(str(path),expected)
    def test_filesystem_parser_is_bounded_and_omits_source_options(self):
        text='1 0 8:1 / / rw - ext4 private-source secret-option\n2 1 8:1 / /target\\040space rw - xfs secret other\n'
        value=z.mount_type('/target space/sub','8:1',text)
        self.assertEqual(value,{'state':'observed','type':'xfs','device':'8:1','mount_id':2})
        self.assertNotIn('secret',json.dumps(value));self.assertNotIn('source',value)
        for device,source in [('8:2',text),('8:1','bad'),('8:1','x'*(z.MIB+1))]:
            self.assertEqual(z.mount_type('/target space','8:1' if device=='8:1' else device,source)['state'],'unknown')
        stacked=text+'3 2 8:1 / /target\\040space rw - tmpfs hidden opts\n'
        self.assertEqual(z.mount_type('/target space','8:1',stacked)['mount_id'],3)
        ambiguous=text+'4 1 8:1 / /target\\040space rw - tmpfs hidden opts\n'
        self.assertEqual(z.mount_type('/target space','8:1',ambiguous)['state'],'unknown')
    def test_elf_closure_requires_exact_fresh_row_and_all_aliases(self):
        request=self.request();inventory,_=f.disk_inventory(self.root,hash_content=False)
        hashes={name:request['groups'][0]['sha256'] for name in inventory}
        row={'executable':str(self.path),'fresh':False,'target':{'kind':['test']},'profile':{'test':True}}
        args=({},inventory,hashes,self.root,inventory)
        self.assertEqual(gate.elf_groups([row],*args),request['groups'])
        for key,value in [('fresh',True),('profile',{'test':False}),('target',{'kind':['example']}),('executable','/outside')]:
            bad=copy.deepcopy(row);bad[key]=value
            with self.assertRaises(RuntimeError):gate.elf_groups([bad],*args)
        bad=copy.deepcopy(inventory);bad['unknown-alias']=copy.deepcopy(next(iter(inventory.values())))
        with self.assertRaisesRegex(RuntimeError,'unknown ELF hardlink'):gate.elf_groups([row],{},bad,hashes,self.root,bad)
    def test_canonical_closure_requires_proof_and_independent_hash(self):
        request=self.request(True);inventory,_=f.disk_inventory(self.root,hash_content=False)
        hashes={name:request['groups'][0]['sha256'] for name in inventory}
        row={'executable':str(self.root/'release/tool'),'fresh':False,'package_id':'pkg','target':{'kind':['bin'],'name':'tool'},'profile':{'test':False}}
        proof={'artifact':row,'canonical_relative':'release/tool','compiler_relative':'release/deps/test-fixture','sha256':request['groups'][0]['sha256']}
        self.assertEqual(gate.elf_groups([row],{'pkg|tool':proof},inventory,hashes,self.root,inventory),request['groups'])
        proof['sha256']='0'*64
        with self.assertRaisesRegex(RuntimeError,'canonical full hash'):gate.elf_groups([row],{'pkg|tool':proof},inventory,hashes,self.root,inventory)
    def test_receipt_rejects_malformed_content_outside_change_and_endpoint_drift(self):
        request=self.request();result,_=self.execute(request);before,allocated=f.disk_inventory(self.root,hash_content=False)
        gate.verify_finalization(request,result,before,allocated,before,allocated,self.root)
        for edit in ['ok','hash','calls','missing','outside','allocation','endpoint']:
            bad=copy.deepcopy(result);after=copy.deepcopy(before);total=allocated
            if edit=='ok':bad['ok']=False
            if edit=='hash':bad['groups'][0]['sha256']='0'*64
            if edit=='calls':bad['groups'][0]['fsync_api_calls']=2
            if edit=='missing':bad['groups']=[]
            if edit=='outside':after['unknown']={}
            if edit=='allocation':total+=4096
            if edit=='endpoint':bad['groups'][0]['after']['allocated']+=4096
            with self.subTest(edit=edit),self.assertRaises(RuntimeError):gate.verify_finalization(request,bad,before,allocated,after,total,self.root)
    def transaction(self,helper):
        request=self.request();request['phase']='finalize-cpu';result,_=self.execute(request);after,allocated=f.disk_inventory(self.root,hash_content=False)
        hashes={name:request['groups'][0]['sha256'] for name in after}
        delta={'changed':{name:{'after':value,'after_sha256':hashes[name]} for name,value in after.items()},'removed':{}}
        namespace={'target_state':{'old':{}},'retained_hashes':{'old':'old'},'retained_elfs':{},'phase':'old',
            'elf_groups':mock.Mock(return_value=request['groups']),'target':self.root,'finalization':z,
            'helper':helper if helper is not None else mock.Mock(return_value=result),
            'fs':types.SimpleNamespace(disk_inventory=mock.Mock(return_value=(after,allocated))),
            'verify_finalization':gate.verify_finalization,'observe':mock.Mock(),'save':mock.Mock(),'require':owner.require}
        return namespace,self.nested('finalize',namespace), (after,allocated,{},delta,hashes),result
    def test_transaction_commits_only_after_valid_receipt_and_evidence(self):
        namespace,finalize,pending,_=self.transaction(None);events=[]
        namespace['save'].side_effect=lambda *args:events.append(copy.deepcopy(namespace['target_state']))
        finalize('cpu',pending,[])
        self.assertEqual(events,[{'old':{}},{'old':{}}]);self.assertEqual(namespace['target_state'],pending[0])
        self.assertEqual(namespace['retained_hashes'],pending[-1]);self.assertEqual(len(namespace['retained_elfs']),1)
    def test_transaction_failure_timeout_unknown_or_write_never_commits(self):
        for error in [RuntimeError('owner'),TimeoutError('deadline'),owner.OwnershipUnknown('cleanup')]:
            namespace,finalize,pending,_=self.transaction(mock.Mock(side_effect=error))
            with self.assertRaises(type(error)):finalize('cpu',pending,[])
            self.assertEqual(namespace['target_state'],{'old':{}});self.assertEqual(namespace['retained_hashes'],{'old':'old'})
        namespace,finalize,pending,_=self.transaction(None);namespace['save'].side_effect=OSError('full')
        with self.assertRaises(OSError):finalize('cpu',pending,[])
        self.assertEqual(namespace['target_state'],{'old':{}})
    def test_empty_clippy_closure_uses_no_helper(self):
        namespace,finalize,pending,_=self.transaction(None);namespace['elf_groups'].return_value=[]
        finalize('clippy',pending,[]);namespace['helper'].assert_not_called();self.assertEqual(namespace['target_state'],pending[0])
    def test_helper_requires_fence_reap_and_pinned_request(self):
        namespace={'fence':mock.Mock(),'kernel_no_children':mock.Mock(return_value=False),'require':owner.require,
            'finalization':z,'save':mock.Mock(),'json_bytes':gate.json_bytes,'evidence':self.root,'python':'python',
            'work':self.root,'hashlib':hashlib,'run':mock.Mock(),'json':json}
        helper=self.nested('helper',namespace)
        with self.assertRaisesRegex(RuntimeError,'reaped owner'):helper('fixture',self.request(),60)
        namespace['run'].assert_not_called();namespace['save'].assert_not_called()
        namespace['kernel_no_children'].return_value=True
        request=self.request();result,_=self.execute(request);namespace['run'].return_value=(json.dumps(result),'')
        helper('fixture',request,60)
        call=namespace['run'].call_args;self.assertEqual(call.kwargs,{'timeout':60})
        self.assertEqual(call.args[1][-1],hashlib.sha256(gate.json_bytes(request)).hexdigest())
        self.assertEqual(namespace['fence'].call_count,3)
    def test_diagnostic_failure_and_write_do_not_escape_original_failure_handler(self):
        namespace={'phase':'before-cpu-exact-0','time':types.SimpleNamespace(time_ns=lambda:1,monotonic=lambda:1),
            'record':{'target_mutation':{'rows':[{'path':'release/deps/test-fixture'}]}},
            'runner':types.SimpleNamespace(unknown=True,pending=[],deadline=100),
            'kernel_no_children':mock.Mock(return_value=True),'require':owner.require,'save':mock.Mock(side_effect=OSError('full'))}
        diagnose=self.nested('failure_diagnostic',namespace);diagnose()
        self.assertEqual(namespace['record']['target_failure_identity']['state'],'unavailable')
        self.assertEqual(namespace['record']['target_failure_identity']['write_error'],'OSError')
        self.assertIn('target_mutation',namespace['record']);namespace['kernel_no_children'].assert_not_called()
    def test_all_new_elf_reads_and_fsync_remain_inside_owned_helper(self):
        tree=ast.parse((HERE/'hud_layer_order_gate.py').read_text())
        for name in ['elf_groups','verify_finalization','helper','finalize','failure_diagnostic']:
            node=next(n for n in ast.walk(tree) if isinstance(n,ast.FunctionDef) and n.name==name)
            calls=[ast.unparse(n.func) for n in ast.walk(node) if isinstance(n,ast.Call)]
            self.assertFalse(set(calls)&{'sha','os.fsync','finalization.execute','finalization.hash_fd'})
        helper=next(n for n in ast.walk(tree) if isinstance(n,ast.FunctionDef) and n.name=='helper')
        calls=[ast.unparse(n.func) for n in ast.walk(helper) if isinstance(n,ast.Call)]
        self.assertIn('run',calls);self.assertNotIn('command',calls)
        sync=ast.parse((HERE/'hud_layer_order/finalization.py').read_text())
        self.assertEqual(sum(isinstance(n,ast.Call) and ast.unparse(n.func)=='os.fsync' for n in ast.walk(sync)),1)

    def test_diagnostic_uses_retained_auxiliary_baseline_and_deduplicates(self):
        request=self.request(True);item=request['groups'][0];after,_=f.disk_inventory(self.root,hash_content=False)
        baseline={path:copy.deepcopy(item) for path in item['aliases']}
        diagnostic_result={'ok':True,'groups':[{'hash_matches':False}]}
        namespace={'phase':'before-cpu-exact-0','time':types.SimpleNamespace(time_ns=lambda:1,monotonic=lambda:1),
            'record':{'target_mutation':{'rows':[{'path':path} for path in item['aliases']]}},
            'runner':types.SimpleNamespace(unknown=False,pending=[],deadline=100),
            'kernel_no_children':mock.Mock(return_value=True),'require':owner.require,'save':mock.Mock(),
            'retained_elfs':baseline,'retained_hashes':{path:item['sha256'] for path in item['aliases']},
            'fs':types.SimpleNamespace(disk_inventory=mock.Mock(return_value=(after,0))),
            'target':self.root,'finalization':z,'MIB':z.MIB,'helper':mock.Mock(return_value=diagnostic_result)}
        original=copy.deepcopy(namespace['record']['target_mutation'])
        self.nested('failure_diagnostic',namespace)()
        call=namespace['helper'].call_args;self.assertEqual(call.args[2],30)
        self.assertEqual(call.args[1]['mode'],'diagnose');self.assertEqual(call.args[1]['groups'],[item])
        self.assertEqual(namespace['record']['target_failure_identity']['state'],'observed')
        self.assertEqual(namespace['record']['target_mutation'],original)
        namespace['helper'].reset_mock();namespace['retained_hashes'][item['aliases'][0]]='missing'
        self.nested('failure_diagnostic',namespace)();namespace['helper'].assert_not_called()
        self.assertEqual(namespace['record']['target_failure_identity']['stage'],'known-identity-selection')
    def test_diagnostic_exception_cannot_replace_original_fence_exception(self):
        tree=ast.parse((HERE/'hud_layer_order_gate.py').read_text())
        main=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='main')
        handler=next(n.handlers[0] for n in main.body if isinstance(n,ast.Try))
        template=ast.parse('def invoke():\n try:\n  raise original\n except BaseException as error:\n  pass\n')
        template.body[0].body[0].handlers=[copy.deepcopy(handler)]
        original=RuntimeError('original fence')
        for diagnostic_error in [OSError('full'),TimeoutError('deadline'),KeyboardInterrupt()]:
            namespace={'original':original,'record':{'target_mutation':{}},
                'failure_diagnostic':mock.Mock(side_effect=diagnostic_error),'runner':types.SimpleNamespace(unknown=False)}
            exec(compile(ast.fix_missing_locations(template),'<original-error-preservation>','exec'),namespace)
            with self.assertRaises(RuntimeError) as caught:namespace['invoke']()
            self.assertIs(caught.exception,original);self.assertEqual(namespace['record']['error'],repr(original))
    def test_producer_failure_prevents_attribution_and_finalization(self):
        namespace={'fence':mock.Mock(),'target_fence':mock.Mock(),'require':owner.require,
            'shutil':types.SimpleNamespace(disk_usage=lambda path:types.SimpleNamespace(free=100*gate.GIB)),
            'work':self.root,'GIB':gate.GIB,'MIB':gate.MIB,'ARCHIVE_CAP':gate.ARCHIVE_CAP,
            'TEXT_CAP':gate.TEXT_CAP,'CAPTURE_CAP':gate.CAPTURE_CAP,'usage':lambda:{'archives':0,'evidence':0},
            'c':c,'audit':types.SimpleNamespace(refresh=mock.Mock(),audit_producer=mock.Mock()),'active':{},
            'tools':{'cargo':'never-executed'},'command':mock.Mock(side_effect=RuntimeError('producer failed')),
            'attribute':mock.Mock(),'finalize':mock.Mock()}
        with self.assertRaisesRegex(RuntimeError,'producer failed'):
            self.nested('produce',namespace)('cpu','test','orr_editor','ui_layer_order_acceptance',{'ui_layer_order_acceptance':('test',True)})
        namespace['attribute'].assert_not_called();namespace['finalize'].assert_not_called();namespace['audit'].audit_producer.assert_not_called()
    def test_parent_identity_change_after_helper_is_rejected(self):
        request=self.request();result,_=self.execute(request);before,allocated=f.disk_inventory(self.root,hash_content=False)
        result['parents'][str(self.root)]['inode']='0'
        with self.assertRaisesRegex(RuntimeError,'root receipt changed'):
            gate.verify_finalization(request,result,before,allocated,before,allocated,self.root)
    def test_helper_source_tool_fence_and_stderr_drift_fail(self):
        namespace={'fence':mock.Mock(side_effect=RuntimeError('source/tool drift')),
            'kernel_no_children':mock.Mock(return_value=True),'require':owner.require,
            'finalization':z,'save':mock.Mock(),'json_bytes':gate.json_bytes,'evidence':self.root,'python':'python',
            'work':self.root,'hashlib':hashlib,'run':mock.Mock(return_value=('{}','error')),'json':json}
        helper=self.nested('helper',namespace)
        with self.assertRaisesRegex(RuntimeError,'source/tool drift'):helper('fixture',self.request(),60)
        namespace['run'].assert_not_called();namespace['fence'].side_effect=None
        with self.assertRaisesRegex(RuntimeError,'helper result bound/stderr'):helper('fixture',self.request(),60)

    def test_all_groups_are_opened_and_proven_before_first_flush(self):
        request=self.request();other=self.root/'release/deps/other'
        other.write_bytes(self.data);other.chmod(0o700)
        item=copy.deepcopy(request['groups'][0]);item['aliases']=['release/deps/other']
        item['stat']=z.snapshot(other.lstat());item['stat']['inode']=str(int(item['stat']['inode'])+1)
        request['groups'].append(item)
        with mock.patch.object(z,'hash_fd') as digest:result,flush=self.execute(request)
        self.assertFalse(result['ok']);digest.assert_not_called();flush.assert_not_called()
    def test_helper_cli_argument_rejection_precedes_file_access(self):
        with mock.patch.object(z,'read_request') as reader:
            self.assertEqual(z.main([]),2);self.assertEqual(z.main(['one']),2);self.assertEqual(z.main(['one','two','three']),2)
            reader.assert_not_called()

if __name__=='__main__':unittest.main()
