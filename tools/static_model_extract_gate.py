#!/usr/bin/env python3
"""Mandatory, source-bound static prop acceptance. Standard library only.

Each mode has independent current-checkout Cargo producers. Historical evidence,
ignored inventories, and CPU results cannot certify the hosted GPU/export modes.
The module is import-safe for its independent semantic and lifecycle tests.
"""
import uuid
import select
import ctypes, hashlib, json, os, pathlib, queue, re, shutil, signal, stat, struct, subprocess, sys, threading, time, traceback, urllib.parse

# This gate is self-contained: no imports from protected gate helpers.
LOG_CAP = 8 * 1024**2
DISK_FLOOR = 512 * 1024**2
PROCESS_CAP = 65536
repo = evidence = frozen = target_dir = None
report = {}
def initialize(root, destination, phase='fixture'):
    global repo, evidence, report, frozen, target_dir
    repo = pathlib.Path(root).resolve()
    evidence = pathlib.Path(destination)
    assert evidence.is_absolute() and evidence == evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    frozen = None
    report = {'status': 'started', 'platform': sys.platform, 'commands': [], 'producers': {}}
    os.environ['CARGO_TERM_COLOR'] = 'never'
    target_dir = new_target(phase)
    os.environ['CARGO_TARGET_DIR'] = str(target_dir)
    report['private_target'] = {'path': str(target_dir), 'phase': phase, 'fresh_empty': True,
                                'device': target_dir.stat().st_dev, 'inode': target_dir.stat().st_ino}
    record()
def new_target(phase):
    assert phase in {'cpu', 'gpu', 'export', 'fixture'}
    parent = repo / 'target'; parent.mkdir(exist_ok=True)
    assert parent.resolve() == parent and not parent.is_symlink()
    target = parent / ('static-prop-' + phase + '-' + uuid.uuid4().hex)
    target.mkdir(mode=0o700, exist_ok=False)
    assert target.resolve() == target and not list(target.iterdir())
    return target
def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024**2), b''):
            h.update(block)
    return h.hexdigest()
def record():
    (evidence / 'gate-receipt.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
def fence():
    for entry in report['producers'].values():
        path = evidence / entry['file']
        assert path.is_file() and not path.is_symlink() and digest(path) == entry['sha256'], 'producer mutated'
class GateFailure(Exception):
    def __init__(self, message, code):
        super().__init__(message)
        self.code = code if isinstance(code, int) and 0 < code < 256 else 1
class LinuxTree:
    """Own this synchronous runner's descendants, including changed sessions."""
    def __init__(self):
        assert sys.platform == 'linux' and hasattr(os, 'pidfd_open') and hasattr(signal, 'pidfd_send_signal')
        assert pathlib.Path('/proc/self/stat').is_file(), 'Linux process identity support required'
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
        self.libc.prctl.restype = ctypes.c_int
        self.previous = ctypes.c_int()
        assert self.libc.prctl(37, ctypes.addressof(self.previous), 0, 0, 0) == 0, 'read subreaper state'
        assert self.libc.prctl(36, 1, 0, 0, 0) == 0, 'enable owned descendant reaping'
        self.cleaned = []
    @staticmethod
    def identity(pid):
        with pathlib.Path(f'/proc/{pid}/stat').open('rb') as stream:
            data = stream.read(4096)
        fields = data[data.rindex(b')') + 2:].split()
        return {'pid': pid, 'state': fields[0], 'parent': int(fields[1]),
                'session': int(fields[3]), 'start': int(fields[19])}
    @classmethod
    def snapshot(cls, deadline):
        result = {}
        # scandir streams directory entries; Path.iterdir/os.listdir can first
        # materialize the whole process table before the collection cap applies.
        with os.scandir('/proc') as entries:
            for examined, entry in enumerate(entries):
                assert time.monotonic() < deadline, 'process census deadline'
                assert examined < PROCESS_CAP, 'bounded process census'
                if not entry.name.isdigit():
                    continue
                try:
                    item = cls.identity(int(entry.name))
                    result[item['pid']] = item
                except (FileNotFoundError, ProcessLookupError):
                    pass
        return result
    def close(self, process):
        owned = {}
        deadline = time.monotonic() + 10
        try:
            if process is None:
                return
            # This gate is synchronous and creates exactly one direct command.
            # Subreaper adoption keeps double-forked/setsid children in ownership.
            while True:
                assert time.monotonic() < deadline, 'descendant freeze deadline'
                table = self.snapshot(deadline)
                reachable = {os.getpid(), process.pid}
                reachable.update(pid for pid, (_, start) in owned.items()
                                 if pid in table and table[pid]['start'] == start)
                while True:
                    assert time.monotonic() < deadline, 'descendant ancestry deadline'
                    more = {pid for pid, item in table.items()
                            if item['parent'] in reachable or item['session'] == process.pid}
                    more.discard(os.getpid())
                    if more.issubset(reachable):
                        break
                    reachable.update(more)
                candidates = reachable - {os.getpid()}
                added = False
                for pid in sorted(candidates):
                    assert time.monotonic() < deadline, 'descendant admission deadline'
                    if pid in owned or pid not in table:
                        continue
                    try:
                        fd = os.pidfd_open(pid, 0)
                    except ProcessLookupError:
                        continue
                    try:
                        current = self.identity(pid)
                        assert current['start'] == table[pid]['start'], 'process identity raced pidfd admission'
                        owned[pid] = (fd, current['start'])
                        added = True
                        if current['state'] not in {b'Z', b'X'}:
                            signal.pidfd_send_signal(fd, signal.SIGSTOP)
                    except (FileNotFoundError, ProcessLookupError):
                        if pid not in owned:
                            os.close(fd)
                    except BaseException:
                        if pid not in owned:
                            os.close(fd)
                        raise
                # A second census after stopping every newly found process closes
                # the fork race before any termination or reaping can lose ancestry.
                if not added and all(table[pid]['state'] in {b'T', b't', b'Z', b'X'}
                                     for pid in candidates if pid in table):
                    break
                time.sleep(.01)
            for fd, _ in owned.values():
                try:
                    signal.pidfd_send_signal(fd, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            process.wait(timeout=max(.01, deadline - time.monotonic()))
            pending = set(owned)
            while pending:
                assert time.monotonic() < deadline, 'owned descendants not terminated and reaped'
                for pid in list(pending):
                    assert time.monotonic() < deadline, 'descendant reap deadline'
                    fd, start = owned[pid]
                    poller = select.poll(); poller.register(fd, select.POLLIN)
                    if not poller.poll(0):
                        continue
                    if pid != process.pid:
                        try:
                            observed, _ = os.waitpid(pid, os.WNOHANG)
                            if observed == 0:
                                continue
                        except ChildProcessError:
                            try:
                                if self.identity(pid)['start'] == start:
                                    continue
                            except (FileNotFoundError, ProcessLookupError):
                                pass
                    self.cleaned.append({'pid': pid, 'start': start, 'termination_observed': True})
                    pending.remove(pid)
                if pending:
                    time.sleep(.01)
        except BaseException:
            # Never leave already-frozen owned processes stopped on a failed gate.
            for fd, _ in owned.values():
                try:
                    signal.pidfd_send_signal(fd, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if process is not None:
                process.kill()
                process.wait(timeout=2)
            raise
        finally:
            for fd, _ in owned.values():
                os.close(fd)
            assert self.libc.prctl(36, self.previous.value, 0, 0, 0) == 0, 'restore prior subreaper state'

class ProcessTree:
    def __init__(self):
        self.job = None
        self.linux = LinuxTree() if os.name != 'nt' else None
        if os.name != 'nt':
            return
        from ctypes import wintypes as w
        class Limits(ctypes.Structure):
            _fields_ = [('process_time', ctypes.c_int64), ('job_time', ctypes.c_int64),
                        ('flags', w.DWORD), ('minimum', ctypes.c_size_t), ('maximum', ctypes.c_size_t),
                        ('active', w.DWORD), ('affinity', ctypes.c_size_t), ('priority', w.DWORD), ('scheduling', w.DWORD)]
        class Extended(ctypes.Structure):
            _fields_ = [('limits', Limits), ('io', ctypes.c_uint64 * 6),
                        ('process_memory', ctypes.c_size_t), ('job_memory', ctypes.c_size_t),
                        ('peak_process', ctypes.c_size_t), ('peak_job', ctypes.c_size_t)]
        self.api = ctypes.WinDLL('kernel32', use_last_error=True)
        for name, args, result in [
            ('CreateJobObjectW', [ctypes.c_void_p, w.LPCWSTR], w.HANDLE),
            ('SetInformationJobObject', [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD], w.BOOL),
            ('AssignProcessToJobObject', [w.HANDLE, w.HANDLE], w.BOOL),
            ('TerminateJobObject', [w.HANDLE, w.UINT], w.BOOL),
            ('QueryInformationJobObject', [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD, ctypes.c_void_p], w.BOOL),
            ('CloseHandle', [w.HANDLE], w.BOOL),
        ]:
            fn = getattr(self.api, name); fn.argtypes = args; fn.restype = result
        self.job = self.api.CreateJobObjectW(None, None)
        assert self.job, 'create Windows process tree'
        limits = Extended(); limits.limits.flags = 0x2000
        if not self.api.SetInformationJobObject(self.job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            self.api.CloseHandle(self.job); self.job = None
            raise OSError(ctypes.get_last_error(), 'kill-on-close process tree')
    def launch(self, command):
        options = dict(cwd=repo, env=os.environ.copy(), stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if self.job is None:
            return subprocess.Popen(command, stdin=subprocess.DEVNULL, start_new_session=True, **options)
        # The launcher cannot execute the command before assignment to the Job Object.
        launcher = "import json,subprocess,sys; assert sys.stdin.readline()=='GO\\n'; sys.exit(subprocess.call(json.loads(sys.argv[1])))"
        process = subprocess.Popen([sys.executable, '-B', '-c', launcher, json.dumps(command)], stdin=subprocess.PIPE, **options)
        try:
            assert self.api.AssignProcessToJobObject(self.job, int(process._handle)), 'assign Windows process tree'
            process.stdin.write(b'GO\n'); process.stdin.flush(); process.stdin.close()
        except BaseException:
            process.kill(); process.wait(timeout=10)
            raise
        return process
    def close(self, process):
        if self.job is not None:
            try:
                assert self.api.TerminateJobObject(self.job, 1), 'terminate Windows descendants'
                deadline = time.monotonic() + 10
                accounting = ctypes.create_string_buffer(48)
                while True:
                    assert self.api.QueryInformationJobObject(self.job, 1, accounting, 48, None)
                    if struct.unpack_from('<I', accounting.raw, 40)[0] == 0:
                        break
                    assert time.monotonic() < deadline, 'Windows descendants still active'
                    time.sleep(.01)
            finally:
                assert self.api.CloseHandle(self.job), 'close Windows process tree'
                self.job = None
        elif self.linux is not None:
            self.linux.close(process)
        if process is not None:
            process.wait(timeout=10)
    def observed_exit(self, process, seconds):
        if os.name == 'nt':
            return process.wait(timeout=seconds)
        # Keep the POSIX child waitable until its group is killed, preventing
        # PID reuse between observation and descendant cleanup.
        deadline = time.monotonic() + seconds
        while True:
            result = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG)
            if result is not None:
                return result.si_status if result.si_code == os.CLD_EXITED else -result.si_status
            if time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(process.args, seconds)
            time.sleep(.01)
def run(label, command, expected=0, seconds=1200):
    fence()
    log = evidence / (label + '.log')
    entry = {'label': label, 'argv': command, 'expected_exit': expected, 'state': 'running',
             'log': log.name, 'output_limit': LOG_CAP, 'deadline_seconds': seconds}
    report['commands'].append(entry); record()
    started = time.monotonic(); process = None; tree = None; reader = None
    events = queue.Queue(maxsize=16); stopped = threading.Event()
    count = 0; failure = None; cleanup = []
    def pump():
        try:
            while not stopped.is_set():
                chunk = process.stdout.read1(65536)
                while not stopped.is_set():
                    try:
                        events.put(chunk, timeout=.05); break
                    except queue.Full:
                        pass
                if not chunk:
                    return
        except BaseException as error:
            cleanup.append('output reader: ' + repr(error))
        finally:
            process.stdout.close()
    try:
        tree = ProcessTree(); process = tree.launch(command)
        reader = threading.Thread(target=pump, daemon=True); reader.start()
        with log.open('xb') as stream:
            while True:
                if time.monotonic() - started > seconds:
                    raise GateFailure('command deadline exceeded', 124)
                if shutil.disk_usage(evidence).free < DISK_FLOOR:
                    raise GateFailure('evidence disk floor reached', 125)
                try:
                    chunk = events.get(timeout=.05)
                except queue.Empty:
                    continue
                if not chunk:
                    break
                allowed = min(len(chunk), LOG_CAP - count)
                stream.write(chunk[:allowed]); stream.flush(); count += allowed
                if allowed != len(chunk):
                    raise GateFailure('command output cap exceeded', 125)
        try:
            observed_exit = tree.observed_exit(process, max(.01, seconds - (time.monotonic() - started)))
        except subprocess.TimeoutExpired:
            raise GateFailure('command deadline exceeded after EOF', 124)
        if observed_exit != expected:
            raise GateFailure('unexpected command exit', observed_exit)
    except BaseException as error:
        failure = error
    finally:
        try:
            if tree is not None:
                tree.close(process)
        except BaseException as error:
            cleanup.append('process tree: ' + repr(error))
        stopped.set()
        if reader is not None:
            reader.join(timeout=10)
            if reader.is_alive():
                cleanup.append('output reader remains alive')
        entry.update(exit=process.returncode if process is not None else None,
                     seconds=time.monotonic() - started, log_bytes=count,
                     log_sha256=digest(log) if log.exists() else None,
                     owned_children_reaped=not cleanup, cleanup_errors=cleanup)
        if tree is not None and tree.linux is not None:
            entry['owned_descendants'] = tree.linux.cleaned
        if cleanup and failure is None:
            failure = GateFailure('process cleanup failed', 1)
        entry['state'] = 'failed' if failure is not None else 'passed'
        if failure is not None:
            entry['failure'] = repr(failure)
        record()
    fence()
    print(label + ': ' + entry['state'], flush=True)
    if failure is not None:
        if log.exists():
            print(log.read_text(encoding='utf-8', errors='replace')[-16384:], flush=True)
        raise failure
    return log.read_text(encoding='utf-8', errors='strict')
def source_snapshot(paths):
    result = {}
    for name in paths:
        path = repo / name
        assert path.is_file() and not path.is_symlink(), 'non-regular source: ' + name
        result[name] = {'sha256': digest(path), 'bytes': path.stat().st_size,
                        'mode': stat.S_IMODE(path.stat().st_mode)}
    return result
def begin():
    assert run('clean-source-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
    report['head'] = run('checkout-head', ['git', 'rev-parse', 'HEAD']).strip()
    report['tree'] = run('checkout-tree', ['git', 'rev-parse', 'HEAD^{tree}']).strip()
    assert re.fullmatch('[0-9a-f]{40}', report['head']) and re.fullmatch('[0-9a-f]{40}', report['tree'])
    report['github_sha'] = os.environ.get('GITHUB_SHA')
    if report['github_sha'] is not None:
        assert report['github_sha'] == report['head'], 'checkout does not match hosted source SHA'
    paths = run('tracked-source', ['git', 'ls-files', '-z']).split('\0')
    assert paths.pop() == '' and paths and len(paths) == len(set(paths))
    frozen = source_snapshot(paths)
    report['checkout'] = str(repo); report['source_sha256'] = frozen; record()
    checked = run('checker-fixtures', [sys.executable, '-B', str(repo / 'tools/test_static_model_extract_gate.py')])
    assert checked.rstrip().endswith('STATIC_PROP_GATE_FIXTURES: 29 passed; 0 failed; 0 skipped'), 'gate fixture census'
    return frozen
def pin(output, label, package, target_name, kind, features):
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    found = [item for item in artifacts if item.get('reason') == 'compiler-artifact'
             and item.get('target', {}).get('name') == target_name and item.get('executable')]
    assert len(found) == 1, 'one exact executable producer: ' + label
    producer = found[0]; target = producer['target']; crate = repo / 'crates' / package
    assert producer.get('fresh') is False, 'a newly compiled producer is mandatory'
    assert pathlib.Path(producer['manifest_path']).resolve() == crate / 'Cargo.toml'
    assert producer['package_id'].startswith('path+')
    uri = urllib.parse.urlsplit(producer['package_id'][5:].split('#', 1)[0])
    assert uri.scheme == 'file' and uri.netloc == ''
    package_path = urllib.parse.unquote(uri.path)
    if os.name == 'nt':
        package_path = package_path.lstrip('/')
    assert pathlib.Path(package_path).resolve() == crate
    folder = {'example': 'examples', 'test': 'tests', 'bin': 'src/bin'}[kind]
    assert pathlib.Path(target['src_path']).resolve() == crate / folder / (target_name + '.rs')
    assert target['kind'] == [kind] and target['crate_types'] == ['bin']
    assert producer['profile']['test'] is (kind == 'test') and producer['profile']['opt_level'] == '3'
    assert sorted(producer['features']) == sorted(features), producer['features']
    original = pathlib.Path(producer['executable'])
    assert original.is_absolute() and original.is_file() and not original.is_symlink()
    directory = target_dir / 'release' / {'example': 'examples', 'test': 'deps', 'bin': ''}[kind]
    assert original == original.resolve() and original.parent == directory, 'unexpected artifact path authority'
    assert 0 < original.stat().st_size <= 128 * 1024**2
    copied = evidence / (label + ('.exe' if os.name == 'nt' else ''))
    with original.open('rb') as source, copied.open('xb') as destination:
        shutil.copyfileobj(source, destination, 1024**2)
    shutil.copymode(original, copied)
    assert digest(original) == digest(copied)
    report['producers'][label] = {'producer': producer, 'file': copied.name,
                                  'sha256': digest(copied), 'bytes': copied.stat().st_size}
    record()
    return str(copied)
def inventory(output, names):
    assert sorted(re.findall(r'^([A-Za-z0-9_:]+): test$', output, re.M)) == sorted(names)
    assert re.findall(r'^(\d+) tests?, 0 benchmarks$', output, re.M) == [str(len(names))]
def result(output, names):
    assert not re.search(r'SKIP:|\bskipping\b|adapter unavailable|source.hiding.*not requested|without source hiding|source_hiding=false', output, re.I)
    # test() always requests one libtest thread. Direct CLI stdout may appear
    # between the harness's unterminated prefix and its final "ok".
    name_pattern = r'[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*'
    assert len(set(names)) == len(names) and all(re.fullmatch(name_pattern, name) for name in names)
    output = output.replace('\r\n', '\n')
    assert '\r' not in output, 'unsupported bare carriage return in test output'
    completed = []
    pending = None
    phase = 'header'
    def diagnostic(line):
        assert not re.match(r'(?:test\b|running\b|failures:|ok\b|FAILED\b|ignored\b|bench\b)', line.lstrip()), 'malformed or nested harness record'
    for line in output.split('\n'):
        if not line.strip():
            continue
        if phase == 'header':
            assert line == f'running {len(names)} test' + ('' if len(names) == 1 else 's'), 'exact test run header'
            phase = 'records'
            continue
        assert phase == 'records', 'unexpected output after test summary'
        if line.startswith('test result:'):
            assert pending is None and sorted(completed) == sorted(names), 'unfinished or missing test records'
            assert re.fullmatch(r'test result: ok\. ' + str(len(names)) + r' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9]+(?:\.[0-9]+)?s', line), 'single all-pass summary'
            phase = 'finished'
            continue
        start = re.fullmatch(r'test (' + name_pattern + r') \.\.\. (.*)', line)
        if start:
            name, payload = start.groups()
            assert pending is None, 'nested test record'
            assert name in names and name not in completed, 'unexpected or duplicate test name'
            if payload == 'ok':
                completed.append(name)
            else:
                diagnostic(payload)
                pending = name
            continue
        if line == 'ok':
            assert pending is not None, 'orphan test completion'
            completed.append(pending)
            pending = None
            continue
        assert pending is not None, 'diagnostic output outside a test record'
        diagnostic(line)
    assert phase == 'finished' and pending is None, 'missing summary or unfinished test record'
def test(label, binary, names, ignored=False):
    inventory(run(label + '-inventory', [binary, '--list']), names)
    inventory(run(label + '-ignored', [binary, '--ignored', '--list']), names if ignored else [])
    flags = ['--ignored'] if ignored else []
    output = run(label + '-run', [binary, *flags, '--test-threads=1'])
    result(output, names)
def software():
    manifests = sorted(pathlib.Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json'))
    assert len(manifests) == 1, 'one Lavapipe manifest is mandatory'
    os.environ.update(VK_ICD_FILENAMES=str(manifests[0]), VK_DRIVER_FILES=str(manifests[0]),
                      WGPU_BACKEND='vulkan', ORR_REQUIRE_GPU='1')
    runtime = evidence / 'xdg-runtime'; runtime.mkdir(mode=0o700)
    os.environ['XDG_RUNTIME_DIR'] = str(runtime)
    report['gpu_environment'] = {key: os.environ[key] for key in
        ['VK_ICD_FILENAMES', 'VK_DRIVER_FILES', 'WGPU_BACKEND', 'ORR_REQUIRE_GPU', 'XDG_RUNTIME_DIR']}
    report['icd_sha256'] = digest(manifests[0]); record()
def source_authority(head, tree, paths):
    assert head == report['head'], 'checkout HEAD changed during acceptance'
    assert tree == report['tree'], 'checkout tree changed during acceptance'
    assert paths == list(frozen), 'tracked path inventory changed during acceptance'
def finish(error):
    if error is not None:
        report['status'] = 'failed'; report['failure'] = ''.join(traceback.format_exception(error))
    else:
        report['status'] = 'passed'
    try:
        fence()
        report['source_unchanged'] = frozen is not None and source_snapshot(list(frozen)) == frozen
        assert report['source_unchanged'], 'source bytes or modes changed'
        assert run('clean-source-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
        head = run('checkout-head-after', ['git', 'rev-parse', 'HEAD']).strip()
        tree = run('checkout-tree-after', ['git', 'rev-parse', 'HEAD^{tree}']).strip()
        paths = run('tracked-source-after', ['git', 'ls-files', '-z']).split('\0')
        assert paths.pop() == ''
        source_authority(head, tree, paths)
        report['source_authority_unchanged'] = True
    except BaseException as secondary:
        report['status'] = 'failed'; report['final_fence_error'] = repr(secondary)
        if error is None:
            error = secondary
    finally:
        record()
    if error is not None:
        raise SystemExit(error.code if isinstance(error, GateFailure) else 1)
def check_gpu(captures):
    names = {'source.glb', 'selected.model.json', 'adapter.json', 'receipt.json'}
    variants = ['full-scene', 'extracted-subtree', 'wrong-transform', 'wrong-uv', 'wrong-material']
    names.update(variant + '.' + ext for variant in variants for ext in ['rgba', 'ppm'])
    assert len(names) == 14 and {path.name for path in captures.iterdir()} == names
    for path in captures.iterdir():
        assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 8 * 1024**2
        if path.suffix == '.rgba':
            assert path.stat().st_size == 256 * 256 * 4
        if path.suffix == '.ppm':
            data = path.read_bytes(); header = b'P6\n256 256\n255\n'
            assert data.startswith(header) and len(data) == len(header) + 256 * 256 * 3
            rgba = path.with_suffix('.rgba').read_bytes()
            assert data[len(header):] == bytes(value for index, value in enumerate(rgba) if index % 4 != 3)
    accepted = json.loads((captures / 'receipt.json').read_text())
    adapter = json.loads((captures / 'adapter.json').read_text())
    assert accepted['schema'] == 1 and accepted['status'] == 'passed'
    assert accepted['width'] == 256 and accepted['height'] == 256
    assert accepted['selected_node'] == 1 and accepted['selected_primitives'] == 4 and accepted['source_primitives'] == 8
    assert accepted['adapter'] == adapter and adapter['software'] is True and adapter['backend'] == 'Vulkan'
    assert accepted['assets']['source_hidden_after_install'] is True
    assert accepted['assets']['cooked_reload_before_render'] is True and accepted['assets']['installed_files'] == 1
    assert accepted['selected_left_half_byte_identical'] is True
    assert accepted['tolerances'] == {'color_bytes': 3, 'minimum_control_differences': 500}
    controls = accepted['control_pixel_differences']
    assert set(controls) == {'omitted_geometry', 'world_transform', 'asymmetric_uv', 'material_slot'}
    selected = (captures / 'extracted-subtree.rgba').read_bytes()
    for control, variant in zip(['omitted_geometry', 'world_transform', 'asymmetric_uv', 'material_slot'],
                               ['full-scene', 'wrong-transform', 'wrong-uv', 'wrong-material']):
        other = (captures / (variant + '.rgba')).read_bytes()
        measured = sum(any(abs(selected[i+j] - other[i+j]) > 3 for j in range(4))
                       for i in range(0, len(selected), 4))
        assert controls[control] == measured and measured > 500
    full = (captures / 'full-scene.rgba').read_bytes()
    for row in range(256):
        start = row * 256 * 4
        assert selected[start:start + 128 * 4] == full[start:start + 128 * 4]
    def verify_capture(entry):
        data = (captures / entry['file']).read_bytes(); value = 0xcbf29ce484222325
        for byte in data:
            value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
        assert entry['bytes'] == len(data) and entry['fnv1a64'] == f'{value:016x}'
    for field in ['source', 'cooked']:
        verify_capture(accepted['assets'][field])
    assert set(accepted['frames']) == {name.replace('-', '_') for name in variants}
    for frame in accepted['frames'].values():
        verify_capture(frame['raw']); verify_capture(frame['image'])
    report['captures'] = {path.name: {'bytes': path.stat().st_size, 'sha256': digest(path)}
                          for path in sorted(captures.iterdir())}

def check_export(captures):
    expected = {'acceptance-source.json', 'production-receipt.json', 'orr.export.json',
                'scene.glb', 'chair.model.json', 'capture-inventory.json', 'authored-project',
                'control-no-model.png', 'control-whole-scene.png'}
    stages = [('initial', 0, ''), ('moved', 18, 'right'), ('collision', 180, 'right'),
              ('interacted', 18, 'right,interact')]
    for stage, _, _ in stages:
        for route in ['source', 'export']:
            expected.add(f'runtime-{stage}-{route}.png')
            expected.add(f'prepared-project-{stage}-{route}.frame')
    assert {path.name for path in captures.iterdir()} == expected
    files = sorted(path for path in captures.rglob('*') if not path.is_dir())
    assert len(files) < 128
    assert all(path.is_file() and not path.is_symlink() and path.stat().st_size <= 64 * 1024**2 for path in captures.rglob('*') if not path.is_dir())
    assert not any(path.is_symlink() for path in captures.rglob('*'))
    inventory_map = {str(path.relative_to(captures)): [path.stat().st_size, digest(path)]
                     for path in files if path.name != 'capture-inventory.json'}
    assert json.loads((captures / 'capture-inventory.json').read_text()) == inventory_map
    for path in captures.glob('*.png'):
        data = path.read_bytes()
        assert len(data) > 33 and data[:8] == b'\x89PNG\r\n\x1a\n'
        assert data[12:16] == b'IHDR' and struct.unpack('>IIBBBBB', data[16:29]) == (1024, 768, 8, 6, 0, 0, 0)
    assert len(list(captures.glob('*.png'))) == 10 and len(list(captures.glob('*.frame'))) == 8
    accepted = json.loads((captures / 'production-receipt.json').read_text())
    assert accepted['source_hidden'] is True and accepted['read_only_bundle'] is True
    assert accepted['selected_changed_pixels'] > 20 and accepted['omitted_changed_pixels'] > 20
    for field, label in [('runtime_sha256', 'room_escape'), ('exporter_sha256', 'orr_export_room'),
                         ('extractor_sha256', 'extractor'), ('acceptance_executable_sha256', 'export-test')]:
        assert accepted[field] == report['producers'][label]['sha256']
    assert set(accepted['observations']) == {name for name, _, _ in stages}
    for stage, ticks, held in stages:
        observation = accepted['observations'][stage]
        assert observation['ticks'] == ticks and observation['held'] == held
        assert observation['source_stdout'] == observation['export_stdout']
        assert 'room capture adapter:' in observation['source_stdout']
        assert f"room tick: {ticks} checksum: {observation['checksum']}" in observation['source_stdout']
        for suffix, stem in [('png', 'runtime'), ('frame', 'prepared-project')]:
            source = captures / f'{stem}-{stage}-source.{suffix}'
            exported = captures / f'{stem}-{stage}-export.{suffix}'
            assert source.stat().st_size > 0 and source.read_bytes() == exported.read_bytes()
            assert observation[f'source_{suffix}_sha256'] == digest(source)
            assert observation[f'export_{suffix}_sha256'] == digest(exported)
    source = json.loads((captures / 'acceptance-source.json').read_text())
    assert source['test_source_sha256'] == digest(repo / 'crates/orr_sample/tests/static_model_extract_export.rs')
    assert source['asset_id'] == 'models/scene.glb' and source['node'] == 3
    assert source['selected_primitives'] == 2 and source['full_scene_primitives'] == 4
    assert source['retained_materials'] == 2 and source['retained_images'] == 2
    assert source['unreferenced_material_slot_retained'] == 0 and source['original_source_digest_retained'] is True
    assert source['source_sha256'] == digest(captures / 'scene.glb')
    assert source['cooked_sha256'] == digest(captures / 'chair.model.json')
    assert source['initial_frame_sha256'] == digest(captures / 'prepared-project-initial-source.frame')
    assert not any(path.suffix == '.glb' for path in (captures / 'authored-project').rglob('*'))
    report['captures'] = inventory_map

def model_tests(windows=False):
    expected = '''subtree_preserves_geometry_uv_material_tables_and_source_slot_order
nested_selection_keeps_ancestor_mirror_nonuniform_transform_without_recentering
listing_uses_default_scene_nonempty_subtrees_and_unambiguous_indices
root_selection_roundtrips_exact_full_import_and_is_deterministic
explicit_and_implicit_default_scenes_control_selection
absent_out_of_scene_and_empty_subtree_selections_reject
logical_identity_and_exact_input_digest_survive_extraction
full_source_admission_rejects_bad_siblings_before_selection
unsupported_animation_skin_extensions_and_attributes_are_never_dropped
external_data_uri_and_unknown_container_resources_reject
malformed_envelopes_truncation_and_duplicate_fields_reject
source_json_and_node_budgets_reject_before_output
unused_material_and_image_slots_are_retained_without_remapping
cli_publishes_reloaded_model_once_and_preserves_source_and_existing_targets
cli_listing_and_help_do_not_publish_or_change_input
cli_bad_arguments_fail_without_output_or_staging
cli_unsafe_names_ancestry_and_package_store_outputs_reject
cli_invalid_missing_and_oversized_inputs_leave_no_output
cli_symlink_sources_destinations_and_ancestors_reject
cli_nonregular_socket_input_rejects_without_opening_or_publication'''.split()
    if windows:
        expected.remove('cli_symlink_sources_destinations_and_ancestors_reject')
        expected.remove('cli_nonregular_socket_input_rejects_without_opening_or_publication')
    return expected

def build_assemble_regression_cli():
    build = run('build-assemble-regression-cli', [
        'cargo', 'build', '--locked', '--release', '-p', 'orr_model',
        '--no-default-features', '--features', 'import', '--example',
        'static_model_assemble', '--message-format=json'])
    binary = pin(build, 'assemble-regression-cli', 'orr_model',
                 'static_model_assemble', 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_ASSEMBLE_BIN'] = binary
    return binary


def cpu():
    global frozen
    failure = None
    try:
        frozen = begin()
        model = ['--locked', '--release', '-p', 'orr_model', '--no-default-features']
        target = ['--test', 'static_model_extract']
        expected = model_tests(windows=os.name == 'nt')
        build = run('build-feature-off', ['cargo', 'build', *model, '--example', 'static_model_extract', '--message-format=json'])
        disabled = pin(build, 'feature-off-example', 'orr_model', 'static_model_extract', 'example', [])
        assert 'requires --features import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
        build = run('build-cli', ['cargo', 'build', *model, '--features', 'import', '--example', 'static_model_extract', '--message-format=json'])
        extractor = pin(build, 'extractor', 'orr_model', 'static_model_extract', 'example', ['import'])
        build = run('build-model-test', ['cargo', 'test', *model, '--features', 'import', *target, '--no-run', '--message-format=json'])
        binary = pin(build, 'model-test', 'orr_model', 'static_model_extract', 'test', ['import'])
        test('model', binary, expected)
        checked = run('assemble-prerequisite-fixtures', [sys.executable, '-B',
            str(repo / 'tools/test_assemble_cli_prerequisites.py')])
        assert checked.rstrip().endswith('ASSEMBLE_CLI_PREREQUISITES: 11 passed; 0 failed; 0 skipped')
        build_assemble_regression_cli()
        run('model-import-regressions', ['cargo', 'test', *model, '--features', 'import'])
        run('strict-model-import', ['cargo', 'clippy', *model, '--features', 'import', '--all-targets', '--', '-D', 'warnings'])
        run('default-model-check', ['cargo', 'check', '--locked', '--release', '-p', 'orr_model', '--all-targets'])
        run('strict-model-default', ['cargo', 'clippy', '--locked', '--release', '-p', 'orr_model', '--all-targets', '--', '-D', 'warnings'])
        dependency_output = run('model-default-dependencies', ['cargo', 'tree', '--locked', '-p', 'orr_model', '--edges', 'normal', '--prefix', 'none'])
        dependency_names = {line.split()[0] for line in dependency_output.splitlines() if line.strip()}
        assert not dependency_names.intersection({'orr_render', 'orr_rhi', 'orr_editor', 'wgpu', 'egui', 'winit'})
        assert 'Usage: static_model_extract' in run('actual-help', [extractor, '--help'])
        source = repo / 'crates/orr_model/tests/fixtures/model.glb'; before = source.read_bytes()
        listing = json.loads(run('actual-list', [extractor, '--input', str(source), '--asset-id', 'models/source.glb', '--list']))
        assert [(item['index'], item['primitive_count']) for item in listing] == [(0, 2), (1, 2)]
        assert source.read_bytes() == before
        authored = evidence / 'selected.model.json'
        command = [extractor, '--input', str(source), '--asset-id', 'models/source.glb', '--node', '1', '--output', str(authored)]
        run('actual-publish', command)
        cooked = authored.read_bytes(); document = json.loads(cooked)
        assert document['format'] == 'orr_static_model' and document['version'] == 1
        assert document['asset_id'] == 'models/source.glb' and len(document['primitives']) == 2
        assert all(item['id'].startswith('models/source.glb#node=1/') for item in document['primitives'])
        assert len(document['materials']) == 2 and len(document['images']) == 2
        run('actual-no-replace', command, expected=2)
        alias = command.copy(); alias[-1] = str(source)
        run('actual-source-alias', alias, expected=2)
        for label, option, value in [
            ('invalid-node', '--node', '4294967295'),
            ('invalid-asset-id', '--asset-id', '../source.glb'),
        ]:
            rejected = command.copy(); rejected[-1] = str(evidence / (label + '.model.json'))
            rejected[rejected.index(option) + 1] = value
            run('actual-' + label, rejected, expected=2)
            assert not pathlib.Path(rejected[-1]).exists()
        unknown = command.copy(); unknown[-1] = str(evidence / 'unknown.model.json')
        run('actual-unknown-option', [*unknown, '--force'], expected=2)
        assert not pathlib.Path(unknown[-1]).exists()
        assert source.read_bytes() == before and authored.read_bytes() == cooked
        assert not list(evidence.glob('.orr-static-prop-*'))
        report['actual_cli'] = {'source_sha256': digest(source), 'cooked_sha256': digest(authored),
                                'list': listing, 'rejections_preserved_source_and_output': True}
        bindings = ['--locked', '--release', '-p', 'orr_model_bindings', '--no-default-features', *target]
        build = run('build-bindings-test', ['cargo', 'test', *bindings, '--no-run', '--message-format=json'])
        binary = pin(build, 'bindings-test', 'orr_model_bindings', 'static_model_extract', 'test', [])
        test('bindings', binary, ['installed_extracted_static_subtree_binding_is_self_contained_and_tamper_evident'])
        run('strict-bindings', ['cargo', 'clippy', *bindings, '--', '-D', 'warnings'])
        run('strict-render', ['cargo', 'clippy', '--locked', '--release', '-p', 'orr_render',
                             '--no-default-features', '--features', 'models,imported-scene', *target, '--', '-D', 'warnings'])
    except BaseException as error:
        failure = error
    finally:
        finish(failure)

def gpu():
    global frozen
    failure = None
    try:
        frozen = begin()
        software()
        captures = evidence / 'captures'
        assert not captures.exists()
        os.environ['ORR_STATIC_MODEL_EXTRACT_CAPTURE_DIR'] = str(captures)
        command = ['cargo', 'test', '--locked', '--release', '-p', 'orr_render',
                   '--no-default-features', '--features', 'models,imported-scene', '--test', 'static_model_extract']
        build = run('build-render-test', [*command, '--no-run', '--message-format=json'])
        binary = pin(build, 'render-test', 'orr_render', 'static_model_extract', 'test', ['models', 'imported-scene'])
        test('render', binary, ['extracted_static_subtree_matches_independent_gpu_oracle'], ignored=True)
        check_gpu(captures)
    except BaseException as error:
        failure = error
    finally:
        finish(failure)

def export():
    global frozen
    failure = None
    try:
        frozen = begin()
        assert sys.platform == 'linux' and os.uname().machine == 'x86_64' and os.getuid() != 0
        assert not evidence.is_relative_to(repo.parent), 'export evidence must survive full workspace hiding'
        software()
        run('namespace-preflight', ['bwrap', '--die-with-parent', '--ro-bind', '/', '/', '--', '/bin/true'])
        temporary = evidence / 'temporary'; temporary.mkdir()
        os.environ['TMPDIR'] = str(temporary)
        os.environ['ORR_REQUIRE_PROJECT_ISOLATION'] = '1'
        model = ['--locked', '--release', '-p', 'orr_model', '--no-default-features', '--features', 'import']
        build = run('build-cli', ['cargo', 'build', *model, '--example', 'static_model_extract', '--message-format=json'])
        extractor = pin(build, 'extractor', 'orr_model', 'static_model_extract', 'example', ['import'])
        sample = ['--locked', '--release', '-p', 'orr_sample', '--no-default-features', '--features', 'room-project,project-export']
        features = ['project', 'project-export', 'room-project', 'sprites']
        for target in ['room_escape', 'orr_export_room']:
            build = run('build-' + target, ['cargo', 'build', *sample, '--bin', target, '--message-format=json'])
            executable = pin(build, target, 'orr_sample', target, 'bin', features)
            os.environ['ORR_ROOM_RUNTIME' if target == 'room_escape' else 'ORR_ROOM_EXPORTER'] = executable
        build = run('build-export-test', ['cargo', 'test', *sample, '--test', 'static_model_extract_export', '--no-run', '--message-format=json'])
        binary = pin(build, 'export-test', 'orr_sample', 'static_model_extract_export', 'test', features)
        run('strict-export-test', ['cargo', 'clippy', *sample, '--test', 'static_model_extract_export', '--', '-D', 'warnings'])
        captures = evidence / 'captures'; captures.mkdir()
        os.environ['ORR_STATIC_MODEL_EXTRACT_BIN'] = extractor
        os.environ['ORR_STATIC_MODEL_EXTRACT_EVIDENCE'] = str(captures)
        test('source-hidden-export', binary, ['static_model_extract_real_cli_source_hidden_room_export'], ignored=True)
        check_export(captures)
    except BaseException as error:
        failure = error
    finally:
        finish(failure)

def main():
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'gpu', 'export'}, 'usage: static_model_extract_gate.py cpu|gpu|export'
    initialize(pathlib.Path(__file__).resolve().parents[1], pathlib.Path(os.environ['ORR_STATIC_GATE_EVIDENCE']), sys.argv[1])
    {'cpu': cpu, 'gpu': gpu, 'export': export}[sys.argv[1]]()

if __name__ == '__main__':
    main()

