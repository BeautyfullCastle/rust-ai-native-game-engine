"""One bounded release build, then one CPU-0 phase diagnostic invocation."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

BASE = Path('/workspace/scratch/edd3c9d10b8b')
REPO = BASE / 'su-physics-phase-costs'
OUT = BASE / 'su-physics-phase-logs'
TARGET = BASE / 'su-tui-wss-target'
FLOOR = 1024 ** 3
ENV = os.environ.copy()
ENV.update(RUSTUP_HOME=str(BASE / 'rust-setup/rustup'),
           CARGO_HOME=str(BASE / 'rust-setup/cargo'),
           CARGO_TARGET_DIR=str(TARGET), RUSTUP_TOOLCHAIN='1.97.1',
           PATH=str(BASE / 'rust-setup/cargo/bin') + ':' + ENV['PATH'])

def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def capture(cmd):
    p = subprocess.run(cmd, cwd=REPO, env=ENV, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return {'command': cmd, 'exit_code': p.returncode, 'output': p.stdout}

def write(name, data):
    (OUT / name).write_text(json.dumps(data, indent=2) + '\n')

def free():
    st = os.statvfs(BASE)
    return st.f_bavail * st.f_frsize

def snapshot(stage):
    result = {'utc': utc(), 'free_bytes': free(),
              'loadavg': Path('/proc/loadavg').read_text().strip(),
              'allowed_cpus': sorted(os.sched_getaffinity(0)),
              'source': capture(['git', 'rev-parse', 'HEAD']),
              'source_status': capture(['git', 'status', '--porcelain=v1', '--untracked-files=all']),
              'processes': capture(['ps', '-eo', 'pid,ppid,stat,psr,pcpu,pmem,comm,args', '--sort=-pcpu'])}
    write(stage + '.environment.json', result)
    return result

def run(stage, cmd, limit, expected):
    before = snapshot(stage + '.before')
    if before['source']['output'].strip() != expected or before['source_status']['output']:
        raise SystemExit('Source not clean at expected commit; refusing ' + stage)
    if before['free_bytes'] < FLOOR:
        raise SystemExit('Disk floor already violated; refusing ' + stage)
    result = {'stage': stage, 'command': cmd, 'cwd': str(REPO),
              'limit_seconds': limit, 'free_floor_bytes': FLOOR,
              'utc_start': utc(), 'free_start_bytes': before['free_bytes'],
              'minimum_free_bytes': before['free_bytes'], 'aborted': False,
              'abort_reason': None}
    start = time.monotonic()
    with (OUT / (stage + '.log')).open('w') as log, (OUT / (stage + '.monitor.jsonl')).open('w') as monitor:
        proc = subprocess.Popen(cmd, cwd=REPO, env=ENV, stdout=log,
                                stderr=subprocess.STDOUT, start_new_session=True)
        result['pid'] = proc.pid
        result['initial_affinity'] = sorted(os.sched_getaffinity(proc.pid))
        next_sample = 0
        while proc.poll() is None:
            elapsed = time.monotonic() - start
            remaining = free()
            result['minimum_free_bytes'] = min(result['minimum_free_bytes'], remaining)
            if elapsed >= next_sample:
                row = {'utc': utc(), 'elapsed_seconds': elapsed, 'free_bytes': remaining,
                       'loadavg': Path('/proc/loadavg').read_text().strip(),
                       'processes': capture(['ps', '-eo', 'pid,ppid,stat,psr,pcpu,pmem,comm,args', '--sort=-pcpu'])}
                try:
                    row['child_affinity'] = sorted(os.sched_getaffinity(proc.pid))
                except ProcessLookupError:
                    pass
                monitor.write(json.dumps(row) + '\n')
                monitor.flush()
                next_sample = elapsed + 5
            if remaining < FLOOR or elapsed >= limit:
                result['aborted'] = True
                result['abort_reason'] = 'disk_floor' if remaining < FLOOR else 'wall_clock_limit'
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
                break
            time.sleep(min(.25, max(.001, limit - (time.monotonic() - start))))
        result.update(exit_code=proc.wait(), elapsed_seconds=time.monotonic() - start,
                      utc_end=utc(), free_end_bytes=free())
    snapshot(stage + '.after')
    write(stage + '.result.json', result)
    print(json.dumps(result, indent=2), flush=True)
    if result['exit_code'] or result['aborted']:
        raise SystemExit('Incomplete ' + stage + '; preserved evidence and no retry')
    return result

if __name__ == '__main__':
    if (OUT / 'metadata.json').exists():
        raise SystemExit('Unique diagnostic output already exists; refusing repeat')
    expected = capture(['git', 'rev-parse', 'HEAD'])['output'].strip()
    metadata = {'utc': utc(), 'source_sha': expected,
                'source_tree': capture(['git', 'rev-parse', 'HEAD^{tree}'])['output'].strip(),
                'baseline_code_sha': 'a9e5ccbcc86ecf282179aa0b908ab5efd6d097c0',
                'base_docs_sha': 'd9fb5e0bd758ebed05990be904b106a17bbd07e7',
                'environment': {k: ENV[k] for k in ('RUSTUP_HOME', 'CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_TOOLCHAIN', 'PATH')},
                'other_build_variables': {k: v for k, v in ENV.items() if k.startswith(('CARGO_PROFILE', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'))},
                'profile': 'Existing release: default opt-level 3, thin LTO, codegen-units 1, debug 1; no overrides',
                'cpu_claim': 'Logical CPU 0 affinity only, not physical-core isolation; visible process namespace cannot exclude host work; cgroup quota/throttle files unavailable'}
    for label, command in [('rustc', ['rustc', '-Vv']), ('cargo', ['cargo', '-Vv']),
                           ('kernel', ['uname', '-a']), ('taskset', ['taskset', '--version']),
                           ('targets', ['rustup', 'target', 'list', '--installed'])]:
        metadata[label] = capture(command)
    sources = ['Cargo.toml', 'Cargo.lock', 'crates/orr_physics3d/Cargo.toml',
               'crates/orr_physics3d/examples/phase_costs.rs',
               'crates/orr_physics3d/benches/physics3d.rs', 'crates/orr_physics3d/tests/common/mod.rs',
               'crates/orr_physics3d/tests/common/scenes.rs']
    sources += [str(p.relative_to(REPO)) for p in sorted((REPO / 'crates/orr_physics3d/src').glob('*.rs'))]
    metadata['source_sha256'] = {p: digest(REPO / p) for p in sources}
    metadata['source_delta_from_base'] = capture(['git', 'diff', '--stat', 'd9fb5e0bd758ebed05990be904b106a17bbd07e7..HEAD'])
    metadata['source_patch_sha256'] = hashlib.sha256(capture(['git', 'diff', 'd9fb5e0bd758ebed05990be904b106a17bbd07e7..HEAD'])['output'].encode()).hexdigest()
    metadata['runner_sha256'] = digest(__file__)
    write('metadata.json', metadata)
    (OUT / 'proc-cpuinfo.txt').write_text(Path('/proc/cpuinfo').read_text())
    (OUT / 'source.patch').write_text(capture(['git', 'diff', 'd9fb5e0bd758ebed05990be904b106a17bbd07e7..HEAD'])['output'])
    run('build', ['cargo', 'build', '--release', '-p', 'orr_physics3d', '--example', 'phase_costs', '--locked', '--offline', '-j1'], 300, expected)
    built = TARGET / 'release/examples/phase_costs'
    preserved = OUT / 'phase_costs'
    if free() - built.stat().st_size < FLOOR:
        raise SystemExit('Preserving executable would breach disk floor; no run')
    shutil.copy2(built, preserved)
    if digest(built) != digest(preserved):
        raise SystemExit('Executable copy hash mismatch')
    if 0 not in os.sched_getaffinity(0):
        raise SystemExit('Logical CPU 0 unavailable')
    exe = {'build_path': str(built), 'preserved_path': str(preserved),
           'sha256_before': digest(preserved), 'size_bytes': preserved.stat().st_size,
           'cpu': 0, 'recorded_at': utc()}
    write('executable.json', exe)
    run('diagnostic', ['taskset', '-c', '0', str(preserved)], 120, expected)
    exe['sha256_after'] = digest(preserved)
    assert exe['sha256_after'] == exe['sha256_before']
    write('executable.json', exe)
    final = snapshot('final')
    for source, sha in metadata['source_sha256'].items():
        if digest(REPO / source) != sha:
            raise SystemExit('Source changed during diagnostic: ' + source)
