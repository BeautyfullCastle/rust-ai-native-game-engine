"""Bounded Linux capture for the exact hosted models-only material diagnostic."""
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
    selector=selectors.DefaultSelector(); outputs=[]; original_handlers={}
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
        return {'returncode':proc.returncode,'stopped':stopped,'observed_bytes':total,
                'retained_bytes':min(total,cap),'elapsed_seconds':time.monotonic()-start,
                'group_cleanup':'identity lost; no group signals or reap claimed' if lost else 'TERM/grace/KILL while leader unreaped; leader reaped',
                'pid':proc.pid}
    finally:
        cleanup()
        for key in list(selector.get_map().values()):key.fileobj.close()
        selector.close()
        for stream in outputs:stream.close()
        for sig,handler in original_handlers.items():signal.signal(sig,handler)


COMMAND = ["cargo", "test", "--locked", "--release", "-p", "orr_editor",
           "--features", "models", "--test", "material_overrides", "--", "--skip",
           "production_main_viewport_changes_after_material_override_edit", "--show-output"]
CAP_BYTES = 16 * 1024 * 1024
TIMEOUT_SECONDS = 2400


def main():
    import json
    import sys

    evidence = Path("material-evidence")
    assert evidence.is_dir() and not evidence.is_symlink()
    env = os.environ.copy()
    env["ORR_MATERIAL_RELOAD_TRACE"] = "1"
    out = evidence / "models-reload-trace.stdout"
    err = evidence / "models-reload-trace.stderr"
    terminal = run(COMMAND, Path.cwd(), env, out, err, TIMEOUT_SECONDS, CAP_BYTES, lambda: None)
    fd = os.open(evidence / "models-reload-trace-terminal.json",
                 os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(terminal, stream, indent=2)
        stream.write("\n")
    # Libtest --show-output preserves per-test attribution without altering event
    # scheduling. Retained output is already bounded across both streams.
    for path, stream in ((out, sys.stdout.buffer), (err, sys.stderr.buffer)):
        with path.open("rb") as source:
            while block := source.read(65536):
                stream.write(block)
        stream.flush()
    if terminal["stopped"] is not None:
        print("Material diagnostic capture stopped before complete evidence", file=sys.stderr)
        return 1
    code = terminal["returncode"]
    return code if code >= 0 else 128 - code


if __name__ == "__main__":
    raise SystemExit(main())
