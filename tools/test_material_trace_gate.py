"""Mocked supervisor controls. No child process, signal or Cargo is launched."""
import signal
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import material_trace_gate as bounded_process


class EntryPointTests(unittest.TestCase):
    def invoke(self, code, stopped=None):
        import io
        import os
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'material-evidence').mkdir()
            previous=Path.cwd();os.chdir(root)
            captured={}
            def fake(argv,cwd,env,out,err,timeout,cap,check):
                captured.update(argv=argv,env=env,timeout=timeout,cap=cap)
                out.write_bytes(b'captured test output\n');err.write_bytes(b'captured compiler output\n')
                return {'returncode':code,'stopped':stopped,'retained_bytes':46}
            try:
                with patch.object(bounded_process,'run',side_effect=fake),patch('sys.stdout',SimpleNamespace(buffer=io.BytesIO())),patch('sys.stderr',io.TextIOWrapper(io.BytesIO())):
                    actual=bounded_process.main()
                self.assertTrue((root/'material-evidence/models-reload-trace-terminal.json').is_file())
                return actual,captured
            finally:os.chdir(previous)

    def test_exact_original_command_with_show_output_and_optin(self):
        result,captured=self.invoke(0)
        self.assertEqual(result,0)
        self.assertEqual(captured['argv'],['cargo','test','--locked','--release','-p','orr_editor','--features','models','--test','material_overrides','--','--skip','production_main_viewport_changes_after_material_override_edit','--show-output'])
        self.assertEqual(captured['env']['ORR_MATERIAL_RELOAD_TRACE'],'1')
        self.assertEqual(captured['cap'],16*1024**2)
        self.assertEqual(captured['timeout'],2400)

    def test_failed_cargo_signal_and_incomplete_capture_remain_nonzero(self):
        self.assertEqual(self.invoke(101)[0],101)
        self.assertEqual(self.invoke(-15)[0],143)
        for stopped in ('log-cap','timeout','interrupt'):
            with self.subTest(stopped=stopped):self.assertEqual(self.invoke(0,stopped)[0],1)


class Pipe:
    def __init__(self,fd):self.fd=fd
    def fileno(self):return self.fd
    def close(self):pass


class Selector:
    def __init__(self):self.items={};self.once=False
    def register(self,stream,events,out):self.items[stream]=SimpleNamespace(fileobj=stream,data=out)
    def unregister(self,stream):del self.items[stream]
    def select(self,timeout):return [(key,None) for key in list(self.items.values())]
    def get_map(self):return self.items
    def close(self):pass


class Proc:
    def __init__(self):self.pid=4321;self.returncode=None;self.stdout=Pipe(10);self.stderr=Pipe(11);self.waited=0
    def wait(self,timeout):self.returncode=0;self.waited+=1;return self.returncode


class SupervisorTests(unittest.TestCase):
    def simulate(self,mode):
        clock=[0.0];calls=[];proc=Proc();chunks={10:[b'abcd',b''],11:[b'']};resource_calls=[0]
        def now():clock[0]+=.1;return clock[0]
        def launch(*args,**kwargs):
            self.assertTrue(kwargs['start_new_session'])
            if mode=='signal':signal.getsignal(signal.SIGTERM)(signal.SIGTERM,None)
            return proc
        def waitid(*args):
            if mode=='ownership':raise ChildProcessError()
            return None if mode=='timeout' else object()
        def resource():
            resource_calls[0]+=1
            if mode=='resource' and resource_calls[0]>1:raise AssertionError('floor')
        def read(fd,n):
            items=chunks[fd]
            return items.pop(0) if items else b''
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with patch.object(bounded_process.subprocess,'Popen',side_effect=launch), \
                 patch.object(bounded_process.selectors,'DefaultSelector',Selector), \
                 patch.object(bounded_process.os,'set_blocking'), \
                 patch.object(bounded_process.os,'read',side_effect=read), \
                 patch.object(bounded_process.os,'waitid',side_effect=waitid), \
                 patch.object(bounded_process.os,'killpg',side_effect=lambda pid,sig:calls.append((pid,sig))), \
                 patch.object(bounded_process.time,'monotonic',side_effect=now), \
                 patch.object(bounded_process.time,'sleep'):
                result=bounded_process.run(['not-a-process'],root,{},root/'out',root/'err',1 if mode=='timeout' else 100,3 if mode=='cap' else 1024,resource)
            retained=(root/'out').read_bytes()+(root/'err').read_bytes()
        return result,calls,proc,retained

    def test_normal_exit_cleans_owned_group_and_reaps_leader(self):
        result,calls,proc,data=self.simulate('normal')
        self.assertIsNone(result['stopped']);self.assertEqual(data,b'abcd')
        self.assertEqual(calls,[(4321,signal.SIGTERM),(4321,signal.SIGKILL)])
        self.assertEqual(proc.waited,1)

    def test_log_cap_preserves_only_bounded_prefix(self):
        result,calls,proc,data=self.simulate('cap')
        self.assertEqual(result['stopped'],'log-cap');self.assertEqual(data,b'abc')
        self.assertEqual(len(calls),2);self.assertEqual(proc.waited,1)

    def test_timeout(self):
        result,calls,proc,_=self.simulate('timeout')
        self.assertEqual(result['stopped'],'timeout');self.assertEqual(proc.waited,1)

    def test_resource_exception(self):
        result,calls,proc,_=self.simulate('resource')
        self.assertEqual(result['stopped'],'resource-or-supervisor-error:AssertionError')
        self.assertEqual(len(calls),2);self.assertEqual(proc.waited,1)

    def test_signal_between_launch_and_assignment_keeps_ownership(self):
        result,calls,proc,_=self.simulate('signal')
        self.assertEqual(result['stopped'],'interrupt');self.assertEqual(len(calls),2)
        self.assertEqual(proc.waited,1)

    def test_lost_child_ownership_never_signals_recycled_group(self):
        result,calls,proc,_=self.simulate('ownership')
        self.assertIn('RuntimeError',result['stopped']);self.assertEqual(calls,[])
        self.assertEqual(proc.waited,0)


if __name__=='__main__':unittest.main(verbosity=2)
