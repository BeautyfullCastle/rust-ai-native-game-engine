#!/usr/bin/env python3
"""Real exporter/runtime + source-hidden readonly bundle + production App process gate.

The App harness exercises the exact runtime host without a native window. This
is not a physical-GPU or native-window claim. --syscall is a mandatory separate
CI mode; failure/permission denial is never downgraded to a pass.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

from room_checkpoint_syscall import check_syscall_proof

P = argparse.ArgumentParser()
for name in ('creator', 'runtime', 'exporter', 'app-tests', 'evidence', 'workspace'):
    P.add_argument('--' + name, required=True, type=Path)
P.add_argument('--syscall', action='store_true')
a = P.parse_args()
evidence = a.evidence.resolve()
evidence.mkdir(parents=True, exist_ok=True)
workspace = a.workspace.resolve()

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def tree(path):
    result = {}
    for file in sorted(path.rglob('*')):
        assert not file.is_symlink(), file
        if file.is_file():
            result[str(file.relative_to(path))] = [file.stat().st_size, digest(file)]
    return result

def run(name, cmd, env=None):
    result = subprocess.run([str(v) for v in cmd], env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
    (evidence / (name + '.log')).write_text(result.stdout)
    assert result.returncode == 0, f'{name}: {result.returncode}\n{result.stdout}'
    return result.stdout

TRACE_NAMES = frozenset(('readonly-export-smoke', 'acquire', 'resume', 'newgame', 'empty'))
TRACE_MAX_BYTES = 16 * 1024 * 1024  # Five known traces, at most 80 MiB total.

def retain_trace(name, command_failed):
    """Keep bounded raw evidence before fixture cleanup, never certify a trace."""
    assert name in TRACE_NAMES, 'unexpected trace name'
    filename = name + '.trace'
    report = {'trace': filename, 'limit_bytes': TRACE_MAX_BYTES, 'retained_bytes': None,
              'byte_capture_complete': False, 'command_failed': command_failed,
              'syscall_validation': 'not performed during retention'}
    if command_failed:
        report['incomplete_reason'] = 'isolated command failed; trace may be incomplete'
    failure = None
    evidence_fd = None
    try:
        # Both directories were created by the gate. Do not follow a replaced
        # directory or source symlink, or open a FIFO/device in the trace slot.
        assert evidence == evidence.resolve(strict=True), 'evidence directory is not canonical'
        assert not evidence.is_symlink()
        evidence_fd = os.open(evidence, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        assert traces == traces.resolve(strict=True), 'trace directory is not canonical'
        assert not traces.is_symlink()
        assert (traces/filename).parent == traces
        assert (evidence/filename).parent == evidence
        traces_fd = os.open(traces, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            with os.fdopen(os.open(filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=traces_fd), 'rb') as source:
                before = os.fstat(source.fileno())
                assert stat.S_ISREG(before.st_mode), 'trace is not a regular file'
                raw = source.read(TRACE_MAX_BYTES + 1)
                after = os.fstat(source.fileno())
        finally:
            os.close(traces_fd)
        payload = raw[:TRACE_MAX_BYTES]
        # Exclusive creation refuses every preexisting destination, including
        # symlinks. Unexpected files in traces are never enumerated or copied.
        with os.fdopen(os.open(filename, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=evidence_fd), 'wb') as output:
            output.write(payload)
        report.update(source_bytes=after.st_size, retained_bytes=len(payload),
                      sha256=hashlib.sha256(payload).hexdigest())
        assert max(before.st_size, after.st_size, len(raw)) <= TRACE_MAX_BYTES, 'trace exceeds retention byte limit'
        assert (before.st_size, before.st_mtime_ns, before.st_ctime_ns) == (after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'trace changed during retention'
        assert len(raw) == after.st_size, 'incomplete trace byte capture'
        report['byte_capture_complete'] = True
    except Exception as error:
        failure = error
        report['incomplete_reason'] = str(error)
        raise
    finally:
        try:
            if evidence_fd is None:
                # An invalid destination must not be followed for diagnostics.
                print(f'{filename}: retention diagnostic: {json.dumps(report)}', file=sys.stderr)
            else:
                with os.fdopen(os.open(filename + '.json', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=evidence_fd), 'w') as output:
                    json.dump(report, output, indent=2)
                    output.write('\n')
        except Exception as error:
            if failure is None:
                raise
            print(f'{filename}: could not retain diagnostic: {error}', file=sys.stderr)
        finally:
            if evidence_fd is not None:
                os.close(evidence_fd)

with tempfile.TemporaryDirectory(prefix='orr-checkpoint-export-') as temporary:
    root = Path(temporary).resolve()
    assert not root.is_relative_to(workspace), 'proof tools must live outside hidden workspace'
    tools = root / 'tools'; tools.mkdir()
    hashes = {}
    for name, path in [('creator',a.creator),('room_escape',a.runtime),('exporter',a.exporter),('app-tests',a.app_tests)]:
        source = path.resolve(strict=True)
        assert source.is_file() and os.access(source, os.X_OK)
        target = tools / name
        shutil.copyfile(source, target); target.chmod(0o555)
        assert digest(source) == digest(target)
        hashes[name] = digest(target)
    (evidence/'tools.sha256.json').write_text(json.dumps(hashes,indent=2)+'\n')
    source = root/'source'
    bundle = root/'bundle'
    # Keep checkpoint ancestry inside the private fixture. In an unprivileged
    # bwrap namespace host-root-owned ancestors can have an unmapped uid.
    # Bind only profile data at an existing top-level mount, as in the Collect
    # progress export gate; production ancestor ownership checks stay intact.
    profiles_root = root/'profiles'; profiles_root.mkdir(mode=0o700)
    data = profiles_root/'data'; data.mkdir(mode=0o700)
    home = profiles_root/'home'; home.mkdir(mode=0o700)
    profile_mount = Path('/opt')
    assert profile_mount.is_dir() and not profile_mount.is_symlink()
    assert not root.is_relative_to(profile_mount)
    assert not workspace.is_relative_to(profile_mount)
    assert not profile_mount.is_relative_to(workspace)
    isolated_data = profile_mount/'data'
    isolated_home = profile_mount/'home'
    captures = root/'captures'; captures.mkdir()
    traces = root/'traces'; traces.mkdir()
    env = dict(os.environ, XDG_DATA_HOME=str(data), HOME=str(home))
    run('create', [tools/'creator','--output',source,'--template','room-escape-ui-3d-v1','--seed','checkpoint-export','--room-checkpoint','12345678-1234-4234-9234-123456789abc'],env)
    run('export', [tools/'exporter','--project',source,'--runtime',tools/'room_escape','--runtime-sha256',hashes['room_escape'],'--output',bundle,'--trusted-runtime'],env)
    assert not list(data.iterdir()), 'creator/export smoke touched profile'
    before = tree(bundle)
    assert before and 'project/orr.project.json' in before
    for item in sorted(bundle.rglob('*'),reverse=True):
        item.chmod(0o555 if item.is_dir() or os.access(item,os.X_OK) else 0o444)
    bundle.chmod(0o555)
    base = ['bwrap','--ro-bind','/','/','--tmpfs',workspace,'--tmpfs',source,
            '--bind',profiles_root,profile_mount,'--ro-bind',home,isolated_home,
            '--bind',captures,captures,'--bind',traces,traces,
            '--setenv','HOME',isolated_home,'--setenv','XDG_DATA_HOME',isolated_data,'--chdir','/']
    def isolated(name, command):
        traced = ['strace','-f','-qq','-yy','-o',traces/(name+'.trace'),'-e','trace=%file,fchdir,write,writev,pwrite64,pwritev,pwritev2,fsync,fdatasync,flock'] if a.syscall else []
        failure = None
        try:
            return run(name, base+['--']+traced+command,env)
        except BaseException as error:
            failure = error
            raise
        finally:
            if a.syscall:
                try:
                    retain_trace(name, command_failed=failure is not None)
                except Exception as error:
                    if failure is None:
                        raise
                    # A retention problem must never replace the process error.
                    print(f'{name}: trace retention failed: {error}', file=sys.stderr)
    smoke = isolated('readonly-export-smoke', [bundle/'run-room-escape','--headless','--ticks','0','--capture',captures/'smoke.png'])
    assert 'room key: 0 won: 0' in smoke and (captures/'smoke.png').is_file()
    assert not list(data.iterdir()), 'headless/capture discovered profile'
    child = 'room_app::room_checkpoint_acceptance_tests::checkpoint_exported_app_process'
    for mode in ('acquire','resume','newgame','empty'):
        command = ['env','ORR_ROOM_CHECKPOINT_PROJECT='+str(bundle/'project'),
                   'ORR_ROOM_CHECKPOINT_MODE='+mode,tools/'app-tests','--ignored','--exact',child,'--nocapture']
        output = isolated(mode,command)
        assert 'test result: ok. 1 passed; 0 failed;' in output, f'zero-test {mode}'
        assert f'mode={mode}' in output and 'profile='+str(isolated_data) in output
    after = tree(bundle)
    assert after == before, 'readonly bundle bytes changed'
    assert all(digest(tools/name)==value for name,value in hashes.items()), 'copied executable changed'
    profiles = list(data.rglob('checkpoint.json'))
    assert len(profiles)==1 and json.loads(profiles[0].read_text())['key_collected'] is False
    (evidence/'bundle-before.json').write_text(json.dumps(before,indent=2)+'\n')
    (evidence/'bundle-after.json').write_text(json.dumps(after,indent=2)+'\n')
    shutil.copyfile(bundle/'orr.export.json', evidence/'orr.export.json')
    shutil.copyfile(captures/'smoke.png', evidence/'smoke.png')
    if a.syscall:
        profile_match = re.search(r' profile=(\S+) status=', (evidence/'acquire.log').read_text())
        assert profile_match and Path(profile_match[1]).is_relative_to(isolated_data)
        check_syscall_proof((traces/'readonly-export-smoke.trace').read_text(),
                            (traces/'acquire.trace').read_text(), isolated_data, isolated_home,
                            Path(profile_match[1]))
    (evidence/'result.json').write_text(json.dumps({'source_hidden':True,'bundle_readonly':True,'bundle_hashes_unchanged':True,'app_processes':4,'native_window':False,'physical_gpu':False,'syscall_proof':a.syscall},indent=2)+'\n')
print('PASS: exported runtime smoke and 4 source-hidden App checkpoint processes; bundle unchanged')
