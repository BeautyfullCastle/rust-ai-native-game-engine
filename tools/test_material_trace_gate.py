"""Mocked supervisor controls. No child process, signal or Cargo is launched."""
import signal
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
import material_trace_gate as bounded_process


class EntryPointTests(unittest.TestCase):
    def invoke(self, code, stopped=None, argv=(), fault=None, broken_diagnostic=False):
        import io
        import json
        import os
        captured = {"attempts": []}
        attempts = captured["attempts"]
        real_open = Path.open
        real_os_open = os.open
        real_fdopen = os.fdopen

        class Reader:
            def __init__(self, source, name):
                self.source, self.name = source, name
            def __enter__(self):
                return self
            def __exit__(self, *args):
                self.source.close()
            def read(self, size):
                attempts.append(self.name + "-read")
                if fault == self.name + "-read":
                    raise OSError("private exception text must not be copied")
                return self.source.read(size)

        class Writer(io.BytesIO):
            def __init__(self, name):
                super().__init__()
                self.name = name
            def write(self, block):
                attempts.append(self.name + "-write")
                if fault == self.name + "-write":
                    raise BrokenPipeError("private write details")
                return super().write(block)
            def flush(self):
                attempts.append(self.name + "-flush")
                if fault == self.name + "-flush":
                    raise OSError("private flush details")
                return super().flush()

        class TextStream:
            def __init__(self, name):
                self.buffer = Writer(name)
            def write(self, text):
                attempts.append("diagnostic-write")
                if broken_diagnostic:
                    raise BrokenPipeError("private diagnostic details")
                return len(text)
            def flush(self):
                pass
            def close(self):
                attempts.append("diagnostic-close")
                self.buffer.close()

        class Terminal:
            def __init__(self, stream):
                self.stream = stream
            def __enter__(self):
                return self
            def __exit__(self, *args):
                self.stream.close()
                if fault == "terminal-close":
                    raise OSError("private terminal close details")
            def write(self, text):
                attempts.append("terminal-write")
                if fault == "terminal-write":
                    raise OSError("private terminal write details")
                return self.stream.write(text)

        def open_source(path, mode="r", *args, **kwargs):
            name = path.suffix[1:]
            if mode == "rb" and name in ("stdout", "stderr"):
                attempts.append(name + "-open")
                if fault == name + "-open":
                    raise OSError("private open details")
                return Reader(real_open(path, mode, *args, **kwargs), name)
            return real_open(path, mode, *args, **kwargs)

        def open_terminal(path, flags, mode=0o777, **kwargs):
            if str(path).endswith("-terminal.json"):
                attempts.append("terminal-open")
                captured.update(terminal_flags=flags, terminal_mode=mode)
                if fault == "terminal-open":
                    raise OSError("private terminal open details")
            return real_os_open(path, flags, mode, **kwargs)

        def fdopen(fd, mode="r", *args, **kwargs):
            stream = real_fdopen(fd, mode, *args, **kwargs)
            return Terminal(stream) if mode == "w" else stream

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("material-evidence", "lighting-evidence"):
                (root / name).mkdir()
            previous = Path.cwd()
            os.chdir(root)
            def fake(command, cwd, env, out, err, timeout, cap, check):
                captured.update(argv=command, env=env, timeout=timeout, cap=cap,
                                out=str(out), err=str(err), cwd=cwd)
                out.write_bytes(b"captured test output\n")
                err.write_bytes(b"captured compiler output\n")
                terminal = {"returncode": code, "stopped": stopped, "retained_bytes": 45}
                captured["terminal"] = terminal
                return terminal
            try:
                with patch.object(bounded_process, "run", side_effect=fake), \
                     patch.object(Path, "open", open_source), \
                     patch.object(os, "open", side_effect=open_terminal), \
                     patch.object(os, "fdopen", side_effect=fdopen), \
                     patch.dict(os.environ, {"ORR_MATERIAL_RELOAD_TRACE": "parent-value"}), \
                     patch("sys.stdout", TextStream("stdout")), \
                     patch("sys.stderr", TextStream("stderr")):
                    actual = bounded_process.main(list(argv))
                    self.assertEqual(os.environ["ORR_MATERIAL_RELOAD_TRACE"], "parent-value")
                captured["files"] = sorted(str(p.relative_to(root)) for p in root.rglob("*") if p.is_file())
                records = list(root.rglob("*-terminal.json"))
                if records and fault != "terminal-write":
                    captured["saved_terminal"] = json.loads(records[0].read_text())
                for p in root.rglob("*.stdout"):
                    self.assertEqual(p.read_bytes(), b"captured test output\n")
                for p in root.rglob("*.stderr"):
                    self.assertEqual(p.read_bytes(), b"captured compiler output\n")
                return actual, captured
            finally:
                os.chdir(previous)

    def test_exact_original_command_with_show_output_and_optin(self):
        result, captured = self.invoke(0)
        self.assertEqual(result, 0)
        self.assertEqual(captured["argv"], ["cargo", "test", "--locked", "--release", "-p", "orr_editor", "--features", "models", "--test", "material_overrides", "--", "--skip", "production_main_viewport_changes_after_material_override_edit", "--show-output"])
        self.assertEqual(captured["out"], "material-evidence/models-reload-trace.stdout")
        self.assertEqual(captured["err"], "material-evidence/models-reload-trace.stderr")
        self.assertIn("material-evidence/models-reload-trace-terminal.json", captured["files"])
        self.assertEqual(captured["env"]["ORR_MATERIAL_RELOAD_TRACE"], "1")
        self.assertEqual(captured["cap"], 16 * 1024**2)
        self.assertEqual(captured["timeout"], 2400)

    def test_explicit_models_profile_matches_default(self):
        default = self.invoke(0)[1]
        explicit = self.invoke(0, argv=("--profile", "models"))[1]
        for field in ("argv", "out", "err", "files", "cap", "timeout"):
            self.assertEqual(default[field], explicit[field])

    def test_lighting_profile_preserves_exact_original_argv_plus_show_output(self):
        original = ["cargo", "test", "--locked", "--release", "-p", "orr_editor", "--features", "room-lighting,room-character,room-checkpoint,project-create", "--test", "room_lighting", "--test", "room_project_workflow", "--test", "room_character_workflow", "--test", "material_overrides", "--", "--skip", "generated_room_lighting_actual_viewport_gpu_static_and_character"]
        result, captured = self.invoke(0, argv=("--profile", "lighting-mixed"))
        self.assertEqual(result, 0)
        self.assertEqual(captured["argv"][:-1], original)
        self.assertEqual(captured["argv"][-1], "--show-output")
        self.assertEqual(captured["out"], "lighting-evidence/mixed-editor-reload-trace.stdout")
        self.assertEqual(captured["err"], "lighting-evidence/mixed-editor-reload-trace.stderr")
        self.assertIn("lighting-evidence/mixed-editor-reload-trace-terminal.json", captured["files"])
        self.assertEqual(captured["env"]["ORR_MATERIAL_RELOAD_TRACE"], "1")
        self.assertEqual(captured["cap"], 16 * 1024**2)
        self.assertEqual(captured["timeout"], 2400)

    def test_closed_cli_rejects_invalid_arguments_before_files_or_child(self):
        invalid = [("--profile",), ("--profile", "unknown"), ("--profile=lighting-mixed",),
                   ("--prof", "models"), ("--profile", "models", "--profile", "models"),
                   ("--profile", "lighting-mixed", "cargo"), ("--cap", "1"),
                   ("--timeout", "1"), ("--output", "/tmp/elsewhere"), ("cargo",)]
        for argv in invalid:
            with self.subTest(argv=argv), patch.object(bounded_process, "run") as run, \
                 patch.object(Path, "is_dir") as is_dir, patch.object(bounded_process.os, "open") as open_file, \
                 patch("sys.stderr", SimpleNamespace(write=lambda text: len(text))):
                self.assertEqual(bounded_process.main(list(argv)), 2)
                run.assert_not_called()
                is_dir.assert_not_called()
                open_file.assert_not_called()

    def test_failed_cargo_signal_and_incomplete_capture_remain_nonzero(self):
        self.assertEqual(self.invoke(101)[0], 101)
        self.assertEqual(self.invoke(-15)[0], 143)
        for stopped in ("log-cap", "timeout", "interrupt"):
            with self.subTest(stopped=stopped):
                self.assertEqual(self.invoke(0, stopped)[0], 1)
                self.assertEqual(self.invoke(101, stopped)[0], 101)

    def test_missing_returncode_and_lost_ownership_fail_safely(self):
        self.assertEqual(self.invoke(None)[0], 1)
        self.assertEqual(self.invoke(None, "resource-or-supervisor-error:RuntimeError")[0], 1)

    def test_each_stream_failure_keeps_primary_and_attempts_other_evidence(self):
        for name in ("stdout", "stderr"):
            for phase in ("open", "read", "write", "flush"):
                for code in (0, 101):
                    with self.subTest(name=name, phase=phase, code=code):
                        result, captured = self.invoke(code, fault=name + "-" + phase)
                        self.assertEqual(result, code or 1)
                        other = "stderr" if name == "stdout" else "stdout"
                        self.assertIn(other + "-flush", captured["attempts"])
                        self.assertIn("terminal-write", captured["attempts"])
                        errors = captured["saved_terminal"]["evidence_errors"]
                        self.assertEqual(len(errors), 1)
                        self.assertEqual(errors[0]["phase"], name + "-emit")
                        self.assertEqual(set(errors[0]), {"phase", "type"})
                        self.assertNotIn("private", repr(errors))

    def test_terminal_failure_keeps_primary_and_raw_output(self):
        for phase in ("open", "write", "close"):
            for code in (0, 101):
                with self.subTest(phase=phase, code=code):
                    result, captured = self.invoke(code, fault="terminal-" + phase)
                    self.assertEqual(result, code or 1)
                    self.assertIn("stdout-flush", captured["attempts"])
                    self.assertIn("stderr-flush", captured["attempts"])
                    self.assertEqual(captured["terminal"]["evidence_errors"][0]["phase"], "terminal-write")

    def test_broken_diagnostic_stream_cannot_replace_exit_status(self):
        for code in (0, 101):
            with self.subTest(code=code):
                self.assertEqual(self.invoke(code, "timeout", broken_diagnostic=True)[0], code or 1)
                self.assertEqual(self.invoke(code, fault="terminal-write", broken_diagnostic=True)[0], code or 1)

    def test_terminal_creation_remains_exclusive_nofollow_private(self):
        import os
        captured = self.invoke(0)[1]
        self.assertEqual(captured["terminal_flags"], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW)
        self.assertEqual(captured["terminal_mode"], 0o600)

    def test_evidence_error_metadata_is_bounded(self):
        terminal = {}
        error = type("E" * 200, (OSError,), {})("private details")
        for _ in range(20):
            bounded_process.evidence_error(terminal, "terminal-write", error)
        self.assertEqual(len(terminal["evidence_errors"]), 8)
        self.assertTrue(all(len(e["type"]) == 64 for e in terminal["evidence_errors"]))
        self.assertNotIn("private", repr(terminal))

    def test_workflow_uses_closed_profile_and_immediate_pipeline_status(self):
        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/room-point-lighting.yml").read_text()
        command = "python3 tools/material_trace_gate.py --profile lighting-mixed 2>&1 | tee lighting-evidence/mixed-editor.log"
        self.assertEqual(workflow.count(command), 1)
        lines = [line.strip() for line in workflow.splitlines()]
        index = lines.index(command)
        self.assertEqual(lines[index - 1], "set +e")
        self.assertEqual(lines[index + 1], 'trace_status=("${PIPESTATUS[@]}")')
        self.assertEqual(lines[index + 2], "set -e")
        self.assertEqual(lines[index + 3], 'if (( trace_status[0] != 0 )); then exit "${trace_status[0]}"; fi')
        self.assertEqual(lines[index + 4], 'if (( trace_status[1] != 0 )); then exit "${trace_status[1]}"; fi')
        self.assertIn("grep -F 'test character_room_lighting_actual_editor_lifecycle ... ok'", lines[index + 5])
        self.assertIn("python3 -m unittest discover -s tools -p test_material_trace_gate.py -v", lines)


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
    def __init__(self,code=0):self.code=code;self.pid=4321;self.returncode=None;self.stdout=Pipe(10);self.stderr=Pipe(11);self.waited=0
    def wait(self,timeout):self.returncode=self.code;self.waited+=1;return self.returncode


class SupervisorTests(unittest.TestCase):
    def simulate(self,mode,code=0):
        clock=[0.0];calls=[];proc=Proc(code);chunks={10:[b'abcd',b''],11:[b'']};resource_calls=[0]
        real_fdopen=bounded_process.os.fdopen;closed=[]
        class Capture:
            def __init__(self,stream):self.stream=stream
            def write(self,block):return self.stream.write(block)
            def flush(self):return self.stream.flush()
            def close(self):
                self.stream.close();closed.append(True)
                if mode=='close':raise OSError('capture close failed')
        def fdopen(fd,*args,**kwargs):return Capture(real_fdopen(fd,*args,**kwargs))
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
            with patch.object(bounded_process.os,'fdopen',side_effect=fdopen), \
                 patch.object(bounded_process.subprocess,'Popen',side_effect=launch), \
                 patch.object(bounded_process.selectors,'DefaultSelector',Selector), \
                 patch.object(bounded_process.os,'set_blocking'), \
                 patch.object(bounded_process.os,'read',side_effect=read), \
                 patch.object(bounded_process.os,'waitid',side_effect=waitid), \
                 patch.object(bounded_process.os,'killpg',side_effect=lambda pid,sig:calls.append((pid,sig))), \
                 patch.object(bounded_process.time,'monotonic',side_effect=now), \
                 patch.object(bounded_process.time,'sleep'):
                result=bounded_process.run(['not-a-process'],root,{},root/'out',root/'err',1 if mode=='timeout' else 100,3 if mode=='cap' else 1024,resource)
            retained=(root/'out').read_bytes()+(root/'err').read_bytes()
            self.assertEqual(len(closed),2)
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

    def test_capture_close_errors_preserve_child_failure_and_cleanup(self):
        for code in (0, 101):
            with self.subTest(code=code):
                result,calls,proc,data=self.simulate('close',code)
                self.assertEqual(result['returncode'],code)
                self.assertEqual(bounded_process.exit_code(result),code or 1)
                self.assertEqual([e['phase'] for e in result['evidence_errors']],['stdout-close','stderr-close'])
                self.assertEqual(calls,[(4321,signal.SIGTERM),(4321,signal.SIGKILL)])
                self.assertEqual(proc.waited,1)
                self.assertEqual(data,b'abcd')


if __name__=='__main__':unittest.main(verbosity=2)
