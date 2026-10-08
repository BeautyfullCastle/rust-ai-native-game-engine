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
import subprocess
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
    data = root/'data'; data.mkdir(mode=0o700)
    home = root/'home'; home.mkdir(mode=0o700)
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
            '--bind',data,data,'--bind',captures,captures,'--bind',traces,traces,
            '--setenv','HOME',home,'--setenv','XDG_DATA_HOME',data,'--chdir','/']
    def isolated(name, command):
        traced = ['strace','-f','-qq','-yy','-o',traces/(name+'.trace'),'-e','trace=%file,fchdir,write,writev,pwrite64,pwritev,pwritev2,fsync,fdatasync,flock'] if a.syscall else []
        return run(name, base+['--']+traced+command,env)
    smoke = isolated('readonly-export-smoke', [bundle/'run-room-escape','--headless','--ticks','0','--capture',captures/'smoke.png'])
    assert 'room key: 0 won: 0' in smoke and (captures/'smoke.png').is_file()
    assert not list(data.iterdir()), 'headless/capture discovered profile'
    child = 'room_app::room_checkpoint_acceptance_tests::checkpoint_exported_app_process'
    for mode in ('acquire','resume','newgame','empty'):
        command = ['env','ORR_ROOM_CHECKPOINT_PROJECT='+str(bundle/'project'),
                   'ORR_ROOM_CHECKPOINT_MODE='+mode,tools/'app-tests','--ignored','--exact',child,'--nocapture']
        output = isolated(mode,command)
        assert 'test result: ok. 1 passed; 0 failed;' in output, f'zero-test {mode}'
        assert f'mode={mode}' in output and 'profile='+str(data) in output
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
        assert profile_match and Path(profile_match[1]).is_relative_to(data)
        check_syscall_proof((traces/'readonly-export-smoke.trace').read_text(),
                            (traces/'acquire.trace').read_text(), data, home,
                            Path(profile_match[1]))
        for trace in traces.iterdir(): shutil.copyfile(trace,evidence/trace.name)
    (evidence/'result.json').write_text(json.dumps({'source_hidden':True,'bundle_readonly':True,'bundle_hashes_unchanged':True,'app_processes':4,'native_window':False,'physical_gpu':False,'syscall_proof':a.syscall},indent=2)+'\n')
print('PASS: exported runtime smoke and 4 source-hidden App checkpoint processes; bundle unchanged')
