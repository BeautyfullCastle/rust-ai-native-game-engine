#!/usr/bin/env python3
"""Run the real fixture with mocked processes; no Cargo, bwrap or GPU runs.

The ancestry model represents an unmapped host ancestor, not an observation of
hosted namespace metadata. Production checkpoint rejection remains unchanged.
"""
import contextlib
import ast
import hashlib
import io
import json
import os
from pathlib import Path
import runpy
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import room_checkpoint_syscall


class ExportFixture(unittest.TestCase):
    def drive(self, syscall=False, reject=False, timeout=False, proof_failure=False, missing_trace=False):
        source = Path(__file__).with_name('check-room-checkpoint-export.py')
        with tempfile.TemporaryDirectory(prefix='checkpoint-fixture-test-') as temporary:
            work = Path(temporary)
            binary = work/'tool'; binary.write_text('mock tool'); binary.chmod(0o700)
            evidence = work/'evidence'
            calls = []
            proof_calls = []
            host_data = None
            modes = []
            trace_payloads = {}
            original_proof = room_checkpoint_syscall.check_syscall_proof

            def proof(*args):
                proof_calls.append(args)
                # Every raw trace must already be retained when validation
                # starts, including the trace responsible for its rejection.
                for name, raw in trace_payloads.items():
                    self.assertEqual((evidence/name).read_bytes(), raw)
                if proof_failure:
                    original_proof(*args)

            def fake_run(command, **kwargs):
                nonlocal host_data
                calls.append(command)
                env = kwargs['env']
                if command[0] != 'bwrap':
                    if '--template' in command:
                        Path(command[command.index('--output')+1]).mkdir()
                    else:
                        bundle = Path(command[command.index('--output')+1]); bundle.mkdir()
                        (bundle/'project').mkdir()
                        (bundle/'project/orr.project.json').write_text('{}')
                        (bundle/'orr.export.json').write_text('{}')
                        (bundle/'run-room-escape').write_text('mock runtime')
                    host_data = Path(env['XDG_DATA_HOME'])
                    return SimpleNamespace(returncode=0, stdout='mock exporter\n')
                self.assertEqual(command[:4], ['bwrap','--ro-bind','/','/'])
                settings = {command[i+1]:command[i+2] for i,v in enumerate(command) if v == '--setenv'}
                binds = {command[i+2]:command[i+1] for i,v in enumerate(command) if v == '--bind'}
                data = Path(settings['XDG_DATA_HOME']); home = Path(settings['HOME'])
                if syscall:
                    trace = Path(command[command.index('-o')+1])
                    raw = ('raw fixture ' + trace.name + '\n').encode()
                    if proof_failure and '--headless' in command:
                        raw = b'11 openat(AT_FDCWD, "/opt/data", O_RDONLY) = -1 ENOENT\n'
                    if not (missing_trace and '--headless' not in command):
                        trace.write_bytes(raw)
                        trace_payloads[trace.name] = raw
                    # A directory sweep must not copy unrelated files.
                    (trace.parent/'unexpected.trace').write_bytes(b'not authorized evidence')
                if '--headless' in command:
                    Path(command[command.index('--capture')+1]).write_bytes(b'mock PNG')
                    return SimpleNamespace(returncode=0, stdout='room key: 0 won: 0\n')
                if timeout:
                    raise subprocess.TimeoutExpired(command, 180, output='fixture timeout')
                # Model the production rejection when traversal still crosses
                # an unmapped host ancestor. A direct, private bind removes it.
                if data.parent != Path('/opt') or reject:
                    return SimpleNamespace(returncode=101, stdout='Checkpoint not saved: checkpoint ancestor must be trusted and not writable by other users\n')
                self.assertEqual(home, Path('/opt/home'))
                readonly = {command[i+2]:command[i+1] for i,v in enumerate(command) if v == '--ro-bind'}
                self.assertEqual(Path(readonly['/opt/home']),host_data.parent/'home')
                self.assertEqual(set(binds),{'/opt',str(host_data.parent.parent/'captures'),str(host_data.parent.parent/'traces')})
                hidden = [command[i+1] for i,v in enumerate(command) if v == '--tmpfs']
                self.assertEqual(hidden,[str(work/'hidden'),str(host_data.parent.parent/'source')])
                self.assertEqual(Path(binds['/opt']), host_data.parent)
                self.assertEqual(host_data.parent.stat().st_mode & 0o777, 0o700)
                self.assertEqual(host_data.stat().st_mode & 0o777, 0o700)
                self.assertEqual((host_data.parent/'home').stat().st_mode & 0o777, 0o700)
                self.assertNotIn(str(host_data), binds)
                mode = next(x.split('=',1)[1] for x in command if x.startswith('ORR_ROOM_CHECKPOINT_MODE='))
                modes.append(mode)
                (host_data/'checkpoint.json').write_text(json.dumps({'key_collected': mode in ('acquire','resume')}))
                return SimpleNamespace(returncode=0, stdout=f'mode={mode} profile={data}/checkpoint.json status=fixture\ntest result: ok. 1 passed; 0 failed;\n')

            args = [str(source)]
            for name in ('creator','runtime','exporter','app-tests'):
                args += ['--'+name,str(binary)]
            args += ['--evidence',str(evidence),'--workspace',str(work/'hidden')]
            if syscall: args += ['--syscall']
            stderr = io.StringIO()
            with patch.object(sys,'argv',args), patch('subprocess.run',side_effect=fake_run), \
                 patch.object(room_checkpoint_syscall,'check_syscall_proof',side_effect=proof), \
                 contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stderr):
                try:
                    runpy.run_path(str(source),run_name='__main__')
                finally:
                    self.retained = {p.name:p.read_bytes() for p in evidence.iterdir()}
                    self.trace_payloads = trace_payloads
                    self.assertFalse(host_data.parent.parent.exists(), 'fixture cleanup did not run')
                    self.retention_stderr = stderr.getvalue()
            self.assertEqual(modes,['acquire','resume','newgame','empty'])
            result = json.loads((evidence/'result.json').read_text())
            self.assertEqual(result['app_processes'],4)
            self.assertTrue(result['bundle_hashes_unchanged'])
            if syscall:
                self.assertEqual(len(proof_calls),1)
                self.assertEqual(proof_calls[0][2:],(Path('/opt/data'),Path('/opt/home'),Path('/opt/data/checkpoint.json')))
                self.assertEqual(len(trace_payloads),5)
                self.assertNotIn('unexpected.trace',self.retained)
                for name, raw in trace_payloads.items():
                    self.assertEqual(self.retained[name],raw)
                    report = json.loads(self.retained[name+'.json'])
                    self.assertTrue(report['byte_capture_complete'])
                    self.assertFalse(report['command_failed'])
                    self.assertEqual(report['sha256'],hashlib.sha256(raw).hexdigest())
            else:
                self.assertFalse(proof_calls)
            return calls

    def test_unmapped_host_ancestor_is_not_in_profile_path(self):
        self.drive()

    def test_syscall_checker_uses_namespace_paths(self):
        self.drive(syscall=True)

    def test_failed_checkpoint_process_still_fails_gate(self):
        with self.assertRaisesRegex(AssertionError,'ancestor must be trusted'):
            self.drive(reject=True)

    def check_failed_evidence(self, names):
        self.assertNotIn('result.json',self.retained)
        self.assertNotIn('unexpected.trace',self.retained)
        self.assertEqual({n for n in self.retained if n.endswith('.trace')},set(names))
        for name in names:
            self.assertEqual(self.retained[name],self.trace_payloads[name])

    def test_actual_profile_rejection_keeps_raw_traces_after_cleanup(self):
        with self.assertRaisesRegex(AssertionError,'headless/capture referenced profile path'):
            self.drive(syscall=True,proof_failure=True)
        self.check_failed_evidence([n+'.trace' for n in ('readonly-export-smoke','acquire','resume','newgame','empty')])
        self.assertIn(b'/opt/data',self.retained['readonly-export-smoke.trace'])

    def test_nonzero_process_keeps_raw_traces_and_original_failure(self):
        with self.assertRaisesRegex(AssertionError,'acquire: 101'):
            self.drive(syscall=True,reject=True)
        self.check_failed_evidence(['readonly-export-smoke.trace','acquire.trace'])
        report = json.loads(self.retained['acquire.trace.json'])
        self.assertTrue(report['command_failed'])
        self.assertIn('may be incomplete',report['incomplete_reason'])

    def test_timeout_keeps_raw_traces_and_original_timeout(self):
        with self.assertRaises(subprocess.TimeoutExpired) as raised:
            self.drive(syscall=True,timeout=True)
        self.assertEqual(raised.exception.output,'fixture timeout')
        self.check_failed_evidence(['readonly-export-smoke.trace','acquire.trace'])
        self.assertTrue(json.loads(self.retained['acquire.trace.json'])['command_failed'])

    def test_retention_error_does_not_mask_nonzero_process(self):
        with self.assertRaisesRegex(AssertionError,'ancestor must be trusted'):
            self.drive(syscall=True,reject=True,missing_trace=True)
        self.assertIn('trace retention failed',self.retention_stderr)
        report = json.loads(self.retained['acquire.trace.json'])
        self.assertFalse(report['byte_capture_complete'])
        self.assertTrue(report['command_failed'])
        self.assertIn('No such file',report['incomplete_reason'])

    def test_retention_error_does_not_mask_timeout(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            self.drive(syscall=True,timeout=True,missing_trace=True)
        self.assertIn('trace retention failed',self.retention_stderr)
        self.assertNotIn('result.json',self.retained)

    def test_successful_process_with_missing_trace_fails_gate(self):
        with self.assertRaises(FileNotFoundError):
            self.drive(syscall=True,missing_trace=True)
        self.assertNotIn('result.json',self.retained)


class TraceRetention(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='checkpoint-trace-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.traces = self.root/'traces'; self.traces.mkdir()
        self.evidence = self.root/'evidence'; self.evidence.mkdir()
        self.trace = self.traces/'acquire.trace'
        tree = ast.parse(Path(__file__).with_name('check-room-checkpoint-export.py').read_text())
        nodes = [n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='retain_trace'
                 or isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id in ('TRACE_NAMES','TRACE_MAX_BYTES') for t in n.targets)]
        self.namespace = dict(os=os,stat=stat,json=json,hashlib=hashlib,sys=sys,
                              traces=self.traces,evidence=self.evidence)
        exec(compile(ast.Module(body=nodes,type_ignores=[]),'<actual-retain-trace>','exec'),self.namespace)
        self.assertEqual(len(self.namespace['TRACE_NAMES']),5)
        self.assertEqual(self.namespace['TRACE_MAX_BYTES'],16*1024*1024)
        self.namespace['TRACE_MAX_BYTES'] = 8  # Exercise the production bound cheaply.
        self.retain = self.namespace['retain_trace']

    def report(self):
        return json.loads((self.evidence/'acquire.trace.json').read_text())

    def test_exact_limit_is_retained_byte_for_byte(self):
        self.trace.write_bytes(b'raw\x00\xff123')
        self.retain('acquire',command_failed=False)
        self.assertEqual((self.evidence/self.trace.name).read_bytes(),b'raw\x00\xff123')
        self.assertTrue(self.report()['byte_capture_complete'])

    def test_overflow_retains_bounded_prefix_and_reports_failure(self):
        self.trace.write_bytes(b'123456789012345')
        with self.assertRaisesRegex(AssertionError,'byte limit'):
            self.retain('acquire',command_failed=False)
        self.assertEqual((self.evidence/self.trace.name).read_bytes(),b'12345678')
        self.assertEqual(self.report()['retained_bytes'],8)
        self.assertEqual(self.report()['source_bytes'],15)
        self.assertFalse(self.report()['byte_capture_complete'])

    def test_missing_trace_reports_incomplete(self):
        with self.assertRaises(FileNotFoundError): self.retain('acquire',command_failed=True)
        self.assertFalse(self.report()['byte_capture_complete'])

    def test_source_symlink_is_not_followed(self):
        other = self.root/'other'; other.write_bytes(b'private')
        self.trace.symlink_to(other)
        with self.assertRaises(OSError): self.retain('acquire',command_failed=False)
        self.assertFalse((self.evidence/self.trace.name).exists())
        self.assertFalse(self.report()['byte_capture_complete'])

    def test_nonregular_trace_is_rejected_without_blocking(self):
        os.mkfifo(self.trace)
        with self.assertRaisesRegex(AssertionError,'regular file'):
            self.retain('acquire',command_failed=False)
        self.assertFalse((self.evidence/self.trace.name).exists())

    def test_symlinked_trace_directory_is_rejected(self):
        alias = self.root/'alias'; alias.symlink_to(self.traces,target_is_directory=True)
        self.namespace['traces'] = alias
        with self.assertRaisesRegex(AssertionError,'not canonical'):
            self.retain('acquire',command_failed=False)

    def test_destination_symlink_is_not_followed(self):
        self.trace.write_bytes(b'trace')
        other = self.root/'other'; other.write_bytes(b'private')
        (self.evidence/self.trace.name).symlink_to(other)
        with self.assertRaises(FileExistsError): self.retain('acquire',command_failed=False)
        self.assertEqual(other.read_bytes(),b'private')
        self.assertFalse(self.report()['byte_capture_complete'])

    def test_symlinked_destination_directory_receives_no_trace_or_diagnostic(self):
        self.trace.write_bytes(b'trace')
        other = self.root/'other'; other.mkdir()
        alias = self.root/'alias'; alias.symlink_to(other,target_is_directory=True)
        self.namespace['evidence'] = alias
        with contextlib.redirect_stderr(io.StringIO()) as stderr, self.assertRaisesRegex(AssertionError,'not canonical'):
            self.retain('acquire',command_failed=False)
        self.assertFalse(list(other.iterdir()))
        self.assertIn('retention diagnostic',stderr.getvalue())

    def test_retention_uses_pinned_destination_directory(self):
        self.trace.write_bytes(b'trace')
        original_open = os.open
        other = self.root/'other'; other.mkdir()
        pinned = self.root/'pinned-evidence'
        def replace_after_open(path, flags, *args, **kwargs):
            fd = original_open(path,flags,*args,**kwargs)
            if path == self.evidence:
                self.evidence.rename(pinned)
                self.evidence.symlink_to(other,target_is_directory=True)
            return fd
        with patch.object(os,'open',side_effect=replace_after_open):
            self.retain('acquire',command_failed=False)
        self.assertFalse(list(other.iterdir()))
        self.assertEqual((pinned/self.trace.name).read_bytes(),b'trace')
        self.assertTrue(json.loads((pinned/'acquire.trace.json').read_text())['byte_capture_complete'])

    def test_changed_trace_is_retained_but_not_claimed_complete(self):
        self.trace.write_bytes(b'trace')
        original_fstat = os.fstat
        calls = 0
        def changed(fd):
            nonlocal calls
            result = original_fstat(fd)
            calls += 1
            if calls == 2:
                return SimpleNamespace(st_size=result.st_size,st_mtime_ns=result.st_mtime_ns+1,st_ctime_ns=result.st_ctime_ns)
            return result
        with patch.object(os,'fstat',side_effect=changed), self.assertRaisesRegex(AssertionError,'changed during retention'):
            self.retain('acquire',command_failed=False)
        self.assertEqual((self.evidence/self.trace.name).read_bytes(),b'trace')
        self.assertFalse(self.report()['byte_capture_complete'])

    def test_partial_write_reports_unknown_retained_size(self):
        self.trace.write_bytes(b'trace')
        original_fdopen = os.fdopen
        class PartialWrite:
            def __init__(self, output): self.output = output
            def __enter__(self): return self
            def __exit__(self, *args): self.output.close()
            def write(self, raw):
                self.output.write(raw[:2]); self.output.flush()
                raise OSError('fixture partial write failure')
        def fail_output(fd, mode):
            output = original_fdopen(fd,mode)
            return PartialWrite(output) if mode=='wb' else output
        with patch.object(os,'fdopen',side_effect=fail_output), self.assertRaisesRegex(OSError,'partial write failure'):
            self.retain('acquire',command_failed=False)
        self.assertEqual((self.evidence/self.trace.name).read_bytes(),b'tr')
        self.assertIsNone(self.report()['retained_bytes'])
        self.assertFalse(self.report()['byte_capture_complete'])

    def test_unexpected_name_cannot_escape_or_expand_capture_count(self):
        for name in ('other','../acquire','/acquire'):
            with self.subTest(name=name), self.assertRaisesRegex(AssertionError,'unexpected trace name'):
                self.retain(name,command_failed=False)
        self.assertFalse(list(self.evidence.iterdir()))

    def test_diagnostic_failure_does_not_replace_capture_failure(self):
        (self.evidence/'acquire.trace.json').write_text('existing evidence')
        with contextlib.redirect_stderr(io.StringIO()) as stderr, self.assertRaises(FileNotFoundError):
            self.retain('acquire',command_failed=False)
        self.assertIn('could not retain diagnostic',stderr.getvalue())
        self.assertEqual((self.evidence/'acquire.trace.json').read_text(),'existing evidence')

    def test_diagnostic_failure_cannot_pass_successful_capture(self):
        self.trace.write_bytes(b'raw')
        (self.evidence/'acquire.trace.json').write_text('existing evidence')
        with self.assertRaises(FileExistsError): self.retain('acquire',command_failed=False)


if __name__ == '__main__':
    unittest.main(verbosity=2)
