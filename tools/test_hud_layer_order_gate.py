#!/usr/bin/env python3
"""Pure/mocked maintained HUD gate controls. No payload or namespace probes."""
import ast,copy,hashlib,importlib.util,json,pathlib,sys,tempfile,types,unittest
from unittest import mock
import yaml
sys.dont_write_bytecode=True
HERE=pathlib.Path(__file__).resolve().parent;ROOT=HERE.parent
sys.path.insert(0,str(HERE))
from hud_layer_order import contracts as c,evidence as e,producer as p,filesystem as f,owned_process as owner
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
    def test_all_python_parses(self):
        for path in [HERE/'hud_layer_order_gate.py',*list((HERE/'hud_layer_order').glob('*.py'))]:ast.parse(path.read_text())

if __name__=='__main__':unittest.main()
