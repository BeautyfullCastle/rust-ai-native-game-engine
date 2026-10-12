"""Bounded Linux capture for the fixed hosted material diagnostic profiles."""
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time


def run(argv, cwd, env, stdout_path, stderr_path, timeout, cap, check_resources):
    if not hasattr(os,'WNOWAIT') or signal.getsignal(signal.SIGCHLD)!=signal.SIG_DFL:
        raise RuntimeError('WNOWAIT/default SIGCHLD required')
    if cap<=0 or timeout<=0: raise ValueError('positive finite bounds required')
    start=time.monotonic(); proc=None; interrupted=None; lost=False; total=0; stopped=None
    selector=selectors.DefaultSelector(); outputs=[]; original_handlers={}; terminal=None
    def interrupt(signum,frame):
        nonlocal interrupted
        if interrupted is None: interrupted=signum
    def exited():
        nonlocal lost
        try: return os.waitid(os.P_PID,proc.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT) is not None
        except ChildProcessError:
            lost=True
            raise RuntimeError('child identity ownership lost')
    def cleanup():
        if proc is None or proc.returncode is not None or lost:return
        exited() # retain unreaped leader to fence PID/PGID reuse through final signal
        for sig in (signal.SIGTERM,signal.SIGKILL):
            try:os.killpg(proc.pid,sig)
            except ProcessLookupError:pass
            if sig==signal.SIGTERM:
                deadline=time.monotonic()+5
                while (remaining:=deadline-time.monotonic())>0:time.sleep(min(.05,remaining))
                exited()
        proc.wait(timeout=10)
    try:
        for sig in (signal.SIGINT,signal.SIGTERM):
            original_handlers[sig]=signal.getsignal(sig);signal.signal(sig,interrupt)
        for path in (stdout_path,stderr_path):
            fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
            outputs.append(os.fdopen(fd,'wb'))
        check_resources()
        if interrupted:raise RuntimeError('interrupted before launch')
        proc=subprocess.Popen(argv,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
        for stream,out in zip((proc.stdout,proc.stderr),outputs):
            os.set_blocking(stream.fileno(),False);selector.register(stream,selectors.EVENT_READ,out)
        while True:
            try:
                check_resources()
                if interrupted:stopped='interrupt'
                elif time.monotonic()-start>timeout:stopped='timeout'
                for key,_ in selector.select(timeout=.2):
                    block=os.read(key.fileobj.fileno(),65536)
                    if not block:
                        selector.unregister(key.fileobj);key.fileobj.close();continue
                    remaining=max(0,cap-total)
                    key.data.write(block[:remaining]);key.data.flush();total+=len(block)
                    if total>cap:stopped='log-cap'
                if stopped:break
                if exited() and not selector.get_map():break
            except BaseException as error:
                stopped='resource-or-supervisor-error:'+type(error).__name__
                break
        cleanup()
        terminal = {'returncode':proc.returncode,'stopped':stopped,'observed_bytes':total,
                'retained_bytes':min(total,cap),'elapsed_seconds':time.monotonic()-start,
                'group_cleanup':'identity lost; no group signals or reap claimed' if lost else 'TERM/grace/KILL while leader unreaped; leader reaped',
                'pid':proc.pid}
        return terminal
    finally:
        cleanup()
        for key in list(selector.get_map().values()):key.fileobj.close()
        selector.close()
        for index, stream in enumerate(outputs):
            try:
                stream.close()
            except Exception as error:
                # Do not replace a known child failure with a capture close error.
                # If no terminal exists, the original setup/supervisor error wins.
                if terminal is not None:
                    evidence_error(terminal, ("stdout-close", "stderr-close")[index], error)
        for sig,handler in original_handlers.items():signal.signal(sig,handler)


COMMAND = ["cargo", "test", "--locked", "--release", "-p", "orr_editor",
           "--features", "models", "--test", "material_overrides", "--", "--skip",
           "production_main_viewport_changes_after_material_override_edit", "--show-output"]
LIGHTING_COMMAND = [
    "cargo", "test", "--locked", "--release", "-p", "orr_editor", "--features",
    "room-lighting,room-character,room-checkpoint,project-create",
    "--test", "room_lighting", "--test", "room_project_workflow",
    "--test", "room_character_workflow", "--test", "material_overrides", "--",
    "--skip", "generated_room_lighting_actual_viewport_gpu_static_and_character",
    "--show-output",
]
PROFILES = {
    "models": (COMMAND, "material-evidence", "models-reload-trace"),
    "lighting-mixed": (LIGHTING_COMMAND, "lighting-evidence", "mixed-editor-reload-trace"),
}
CAP_BYTES = 16 * 1024 * 1024
TIMEOUT_SECONDS = 2400


def evidence_error(terminal, phase, error):
    errors = terminal.setdefault("evidence_errors", [])
    if len(errors) < 8:
        errors.append({"phase": phase, "type": type(error).__name__[:64]})


def exit_code(terminal):
    code = terminal.get("returncode")
    if isinstance(code, int) and code != 0:
        return code if code > 0 else 128 - code
    if code != 0 or terminal.get("stopped") is not None or terminal.get("evidence_errors"):
        return 1
    return 0


def diagnostic(message):
    import sys
    try:
        print(message, file=sys.stderr)
    except Exception:
        # A later interpreter flush must not replace the preserved exit status.
        try:
            sys.stderr.close()
        except Exception:
            pass


def main(argv=None):
    import json
    import sys

    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        profile = "models"
    elif len(argv) == 2 and argv[0] == "--profile" and argv[1] in PROFILES:
        profile = argv[1]
    else:
        diagnostic("Usage: material_trace_gate.py [--profile models|lighting-mixed]")
        return 2
    command, directory, prefix = PROFILES[profile]
    evidence = Path(directory)
    assert evidence.is_dir() and not evidence.is_symlink()
    env = os.environ.copy()
    env["ORR_MATERIAL_RELOAD_TRACE"] = "1"
    out = evidence / (prefix + ".stdout")
    err = evidence / (prefix + ".stderr")
    terminal = run(list(command), Path.cwd(), env, out, err, TIMEOUT_SECONDS, CAP_BYTES, lambda: None)
    # --show-output retains per-test attribution. No test scheduling flag changes;
    # opt-in logging itself can perturb timing and does not prove a race is fixed.
    for phase, path, stream in (("stdout-emit", out, sys.stdout.buffer),
                                ("stderr-emit", err, sys.stderr.buffer)):
        try:
            with path.open("rb") as source:
                while block := source.read(65536):
                    stream.write(block)
            stream.flush()
        except Exception as error:
            evidence_error(terminal, phase, error)
            # Avoid a failed buffered stream changing the exit code again during
            # interpreter shutdown. The other stream and terminal are still tried.
            try:
                stream.close()
            except Exception:
                pass
    try:
        fd = os.open(evidence / (prefix + "-terminal.json"),
                     os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w") as stream:
            json.dump(terminal, stream, indent=2)
            stream.write("\n")
    except Exception as error:
        evidence_error(terminal, "terminal-write", error)
    if terminal.get("stopped") is not None or terminal.get("evidence_errors"):
        diagnostic("Material diagnostic capture has incomplete evidence")
    return exit_code(terminal)


if __name__ == "__main__":
    raise SystemExit(main())
