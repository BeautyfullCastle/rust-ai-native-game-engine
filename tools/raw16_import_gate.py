#!/usr/bin/env python3
"""Required RAW16 inventories, strict features and explicit software-device evidence.

Self-contained stdlib gate. No explicit dependency acquisition, retries, target
cleanup or native-window claims. Hosted Cargo uses locked dependency resolution;
local work requires a separate offline plan. The existing job target is reused.
"""
import argparse
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import stat
import struct
import subprocess
import sys
import threading
import time
import zlib

sys.dont_write_bytecode = True
LOG_CAP = 16 * 1024**2
DISK_FLOOR = 4 * 1024**3
CAPTURE_CAP = 24 * 1024**2
CACHE_CAP = 32 * 1024**2
PACKAGE = 'orr_terrain_import'
DEVICE = 'raw16_package_reload_oblique_side_and_orientation_controls_render'
EDITOR = sorted([
    'imported_local_open_query_edit_undo_save_reopen_preserves_host_frame_history',
    'installed_import_opens_read_only_and_copy_edit_does_not_change_package',
    'production_editor_rejects_missing_terrain_declaration_and_precision_transactionally',
])
PUBLISH = sorted('publish::tests::' + name for name in [
    'failures_before_link_leave_destination_absent_and_remove_owned_stage',
    'corrupt_stage_readback_is_not_published',
    'concurrent_destination_creation_is_never_replaced',
    'late_cleanup_failure_keeps_exact_published_bytes_and_one_owned_stage',
    'late_directory_sync_failure_keeps_published_result',
    'failed_commit_and_cleanup_report_owned_leftover_without_destination',
])
CLI = sorted([
    'actual_cli_imports_canonical_bytes_and_keeps_source_unchanged',
    'help_and_usage_statuses_are_explicit',
    'duplicate_missing_unknown_and_malformed_parameters_do_not_create_output',
    'explicit_big_endian_and_flip_z_are_used_by_actual_binary',
    'input_lengths_directories_and_missing_files_are_rejected_without_staging',
    'no_clobber_aliases_traversal_package_store_and_missing_parent_are_rejected',
    'output_basename_is_portable_for_existing_editor_open',
    'independent_cli_processes_race_one_destination_without_overwriting',
    'public_file_api_rejects_display_failure_before_staging',
])
UNIX_CLI = ['symlinks_dangling_links_and_fifo_are_rejected_before_open',
            'socket_input_is_rejected_before_open']
CONVERSION = sorted([
    'asymmetric_little_big_endian_floor_and_endpoints_match_integer_oracle',
    'rectangular_two_by_three_and_flip_z_do_not_transpose_x',
    'zero_scale_and_negative_offset_are_exact',
    'conversion_floors_fractional_raw_product_after_decimal_rounding_once',
    'canonical_repeated_imports_and_identity_revision_are_deterministic',
    'maximum_dimensions_and_exact_byte_budget_are_admitted',
    'dimension_identity_and_scale_bounds_fail_before_allocation',
    'spacing_coordinate_overflow_and_display_precision_are_rejected',
    'nonexact_spacing_is_rejected_even_with_exact_displayable_vertices',
    'truncated_odd_padded_and_oversized_input_lengths_fail',
    'canonical_surface_uses_triangle_plane_not_bilinear_interpolation',
])
PACKAGE_TESTS = sorted([
    'imported_terrain_installs_reopens_and_survives_original_raw_and_source_removal',
    'package_capability_and_undeclared_asset_rejections_are_real_api_gates',
    'installed_tamper_fails_verification_and_read_asset',
    'incompatible_engine_and_missing_declared_file_fail_installation',
])
FRAMES = ['oblique-baseline', 'oblique-flip-z', 'oblique-transpose',
          'side-baseline', 'side-flip-z', 'side-transpose', 'top-baseline',
          'top-markers', 'side-markers', 'side-wrong-height']
HELPER_TESTS = [
    'test_platform_and_command_inventory', 'test_exact_names_and_duplicates',
    'test_zero_ignored_filtered_and_failed_results', 'test_artifact_selection',
    'test_actual_cli_producer_fence', 'test_source_and_mode_fence',
    'test_software_adapter_required', 'test_png_integrity_and_bounds',
    'test_capture_census_and_controls', 'test_exclusive_evidence_and_budgets',
    'test_workflow_each_inherited_and_private_action_dependency_is_mandatory',
    'test_mandatory_workflow_and_preserved_gates', 'test_bounded_process_receipts',
]


def require(condition, message='contract failed'):
    if not condition:
        raise AssertionError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def save(path, value):
    with path.open('x', encoding='utf-8') as output:
        json.dump(value, output, indent=2)
        output.write('\n')


def inventory(text, expected):
    names = re.findall(r'^([A-Za-z0-9_:]+): test$', text, re.M)
    require(sorted(names) == sorted(expected) and len(set(names)) == len(names), 'exact inventory')
    require(re.findall(r'^\s*(\d+) tests?, 0 benchmarks\s*$', text, re.M) == [str(len(expected))], 'inventory total')


def result(text, expected):
    require(not re.search(r'(?im)(?:SKIP:|\bskipping\b|adapter unavailable)', text), 'skip diagnostic')
    require(sorted(re.findall(r'^test (.+) \.\.\. ok$', text, re.M)) == sorted(expected), 'exact passed names')
    require(not re.search(r'^test .+ \.\.\. (?:ignored|FAILED)', text, re.M), 'failed or ignored result')
    summaries = re.findall(r'^test result: (.+)$', text, re.M)
    require(len(summaries) == 1 and re.fullmatch(
        rf'ok\. {len(expected)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in [0-9.]+s',
        summaries[0]) is not None, 'exact result total')


def native_lanes(platform):
    require(platform in ('linux', 'darwin', 'win32'), 'unsupported platform')
    return [('lib', PACKAGE, PUBLISH), ('bin', PACKAGE, []),
            ('test', 'cli', sorted(CLI + ([] if platform == 'win32' else UNIX_CLI))),
            ('test', 'conversion', CONVERSION), ('test', 'package', PACKAGE_TESTS)]


def command_inventory(phase):
    if phase == 'native':
        labels = ['helper-tests', 'native-compile']
        for label in ('lib', 'bin', 'cli', 'conversion', 'package'):
            labels += [label + '-list', label + '-ignored-list', label + '-run']
        return labels + ['default-strict', 'feature-off-strict', 'default-tree', 'feature-off-tree']
    if phase == 'editor':
        return ['helper-tests', 'editor-compile', 'editor-list', 'editor-ignored-list', 'editor-run',
                'editor-strict', 'gpu-strict', 'combined-strict']
    require(phase == 'device', 'phase')
    return ['device-compile', 'render-list', 'render-ignored-list', 'render-run']


def helper_result(text):
    rows = re.findall(r'^(test_\w+) \(__main__\.Raw16GateTests(?:\.(test_\w+))?\) \.\.\. ok$', text, re.M)
    require(sorted(name for name, _ in rows) == sorted(HELPER_TESTS), 'helper names')
    require(len(re.findall(r'^test_\w+ .* \.\.\. ', text, re.M)) == len(HELPER_TESTS), 'extra helper status')
    require(all(not suffix or name == suffix for name, suffix in rows), 'helper suffix')
    require(not re.search(r'(?im)(?:\.\.\. (?:skipped|FAIL|ERROR|expected failure|unexpected success)|^FAILED|^ERROR:|^FAIL:)', text), 'helper failure or skip')
    require(re.findall(r'^Ran (\d+) tests in [0-9.]+s$', text, re.M) == ['13'] and text.rstrip().endswith('OK'), 'helper total')


def dependency_tree(text):
    names = [line.split(' ', 1)[0] for line in text.splitlines() if line]
    require(PACKAGE in names, 'normal dependency root')
    blocked = {'orr_editor', 'orr_render', 'orr_rhi', 'wgpu', 'egui', 'eframe', 'winit'}
    require(not blocked.intersection(names), 'default dependency leakage')


def adapter(text):
    rows = re.findall(r'^RAW16 acceptance adapter: (.+) \(software: (true|false)\)$', text, re.M)
    require(len(rows) == 1 and rows[0][1] == 'true' and rows[0][0].strip(), 'one observed software adapter')
    return {'name': rows[0][0], 'software': True, 'backend_identity_reported': False}


# Bounded PNG and process primitives are copied into this gate for independent
# review; there is no runtime import of development-only helper infrastructure.
def png_rgba(data, shape):
    """Decode only bounded, noninterlaced RGBA8 PNG; reject malformed chunks."""
    require(len(data) <= 2 * 1024**2 and data[:8] == b"\x89PNG\r\n\x1a\n", "PNG signature/size")
    width, height = shape
    require(0 < width <= 640 and 0 < height <= 480, "PNG dimensions")
    pos, compressed, state, seen_header = 8, bytearray(), 0, False
    while pos < len(data):
        require(pos + 12 <= len(data), "truncated chunk")
        length = struct.unpack_from(">I", data, pos)[0]
        require(length <= 2 * 1024**2 and pos + 12 + length <= len(data), "chunk length")
        kind = data[pos+4:pos+8]; payload = data[pos+8:pos+8+length]
        require(all(65 <= v <= 90 or 97 <= v <= 122 for v in kind) and 65 <= kind[2] <= 90, "chunk type/reserved bit")
        crc = struct.unpack_from(">I", data, pos+8+length)[0]
        require(zlib.crc32(kind+payload) & 0xffffffff == crc, "chunk CRC")
        pos += length + 12
        if kind == b"IHDR":
            require(not seen_header and pos == 33 and length == 13, "IHDR order")
            require(struct.unpack(">IIBBBBB", payload) == (width, height, 8, 6, 0, 0, 0), "RGBA8 format")
            seen_header = True
        elif kind == b"IDAT":
            require(seen_header and state in (0, 1), "IDAT order")
            state = 1; compressed.extend(payload)
        elif kind == b"IEND":
            require(state in (1, 2) and length == 0 and pos == len(data), "IEND/order/trailing bytes")
            state = 3
        else:
            require(seen_header and kind[0] & 32, "unknown critical chunk")
            if state == 1:
                state = 2
    require(state == 3, "missing IEND")
    count = height * (1 + width * 4)
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, count + 1)
    require(len(raw) == count and decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail, "bounded zlib stream")
    stride = width * 4; pixels = bytearray(); previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1); filt = raw[start]
        require(filt <= 4, "PNG filter")
        row = bytearray(raw[start+1:start+1+stride])
        for x in range(stride):
            a = row[x-4] if x >= 4 else 0; b = previous[x]; c = previous[x-4] if x >= 4 else 0
            if filt == 1: prediction = a
            elif filt == 2: prediction = b
            elif filt == 3: prediction = (a+b)//2
            elif filt == 4:
                q = a+b-c; pa,pb,pc = abs(q-a),abs(q-b),abs(q-c)
                prediction = a if pa <= pb and pa <= pc else b if pb <= pc else c
            else: prediction = 0
            row[x] = (row[x]+prediction) & 255
        pixels.extend(row); previous = row
    return bytes(pixels)


class WindowsJob:
    """Place a gated launcher in a kill-on-close job before it starts cargo/tests."""
    def __init__(self):
        from ctypes import wintypes as w
        class Basic(ctypes.Structure):
            _fields_ = [("process_time",ctypes.c_int64),("job_time",ctypes.c_int64),("flags",w.DWORD),("min_ws",ctypes.c_size_t),("max_ws",ctypes.c_size_t),("active",w.DWORD),("affinity",ctypes.c_size_t),("priority",w.DWORD),("scheduling",w.DWORD)]
        class Io(ctypes.Structure):
            _fields_ = [(name,ctypes.c_uint64) for name in ("ro","wo","oo","rt","wt","ot")]
        class Extended(ctypes.Structure):
            _fields_ = [("basic",Basic),("io",Io),("process_mem",ctypes.c_size_t),("job_mem",ctypes.c_size_t),("peak_process",ctypes.c_size_t),("peak_job",ctypes.c_size_t)]
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p,w.LPCWSTR]; self.api.CreateJobObjectW.restype = w.HANDLE
        self.api.SetInformationJobObject.argtypes = [w.HANDLE,ctypes.c_int,ctypes.c_void_p,w.DWORD]
        self.api.AssignProcessToJobObject.argtypes = [w.HANDLE,w.HANDLE]
        self.api.TerminateJobObject.argtypes = [w.HANDLE,w.UINT]
        self.api.CloseHandle.argtypes = [w.HANDLE]
        self.api.QueryInformationJobObject.argtypes = [w.HANDLE,ctypes.c_int,ctypes.c_void_p,w.DWORD,ctypes.c_void_p]
        self.handle = self.api.CreateJobObjectW(None,None)
        require(bool(self.handle), "CreateJobObjectW")
        info = Extended(); info.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle,9,ctypes.byref(info),ctypes.sizeof(info)):
            self.api.CloseHandle(self.handle); raise OSError(ctypes.get_last_error(), "SetInformationJobObject")
    def assign(self, process):
        require(bool(self.api.AssignProcessToJobObject(self.handle,int(process._handle))), "AssignProcessToJobObject")
    def close(self):
        if self.handle:
            require(bool(self.api.TerminateJobObject(self.handle,1)), "TerminateJobObject")
            accounting = ctypes.create_string_buffer(48)
            deadline = time.monotonic()+10
            while True:
                require(bool(self.api.QueryInformationJobObject(self.handle,1,accounting,48,None)), "QueryInformationJobObject")
                active = struct.unpack_from("<I",accounting.raw,40)[0]
                if active == 0: break
                require(time.monotonic() < deadline, "Windows job descendants not reaped")
                time.sleep(.01)
            require(bool(self.api.CloseHandle(self.handle)), "CloseHandle job"); self.handle = None


def bounded_run(argv, cwd, env, log_path, timeout, expected_exit=0, disk_floor=DISK_FLOOR):
    """Bound output with a reader queue, reap descendants on success and failure."""
    require(not log_path.exists(), "fresh command log")
    start = time.monotonic(); reason = None; count = 0; job = None; process = None; reaped = False; assigned = False
    events = queue.Queue(maxsize=16); stopped = threading.Event(); reader = None
    def pump(pipe):
        try:
            while True:
                chunk = pipe.read1(65536)
                while not stopped.is_set():
                    try: events.put(chunk, timeout=.05); break
                    except queue.Full: pass
                if not chunk or stopped.is_set(): break
        finally:
            pipe.close()
    try:
        if os.name == "nt":
            job = WindowsJob()
            # Waiting for GO prevents the target or any descendants escaping
            # before assignment to the job; argv is JSON, never a shell string.
            launcher = "import sys,json,subprocess; a=json.loads(sys.argv[1]); assert sys.stdin.readline()=='GO\\n'; sys.exit(subprocess.call(a))"
            process = subprocess.Popen([sys.executable,"-c",launcher,json.dumps(argv)],cwd=cwd,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            job.assign(process); assigned = True; process.stdin.write(b"GO\n"); process.stdin.flush(); process.stdin.close()
        else:
            process = subprocess.Popen(argv,cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,start_new_session=True)
        reader = threading.Thread(target=pump,args=(process.stdout,),daemon=True); reader.start()
        with log_path.open("xb") as log:
            while True:
                if time.monotonic()-start > timeout: reason = "timeout"; break
                if shutil.disk_usage(log_path.parent).free < disk_floor: reason = "disk floor"; break
                try: chunk = events.get(timeout=.05)
                except queue.Empty: continue
                if not chunk: break
                if count+len(chunk) > LOG_CAP:
                    retained = chunk[:LOG_CAP-count]; log.write(retained); log.flush(); count += len(retained)
                    reason = "log cap"; break
                log.write(chunk); log.flush(); count += len(chunk)
        if reason is None:
            process.wait(timeout=max(.01,timeout-(time.monotonic()-start)))
    except BaseException as error:
        reason = reason or repr(error)
    finally:
        cleanup_errors = []; group_error = None; group_cleanup = None
        try:
            if job is not None:
                if process is not None and not assigned:
                    process.kill(); process.wait(timeout=10)
                job.close()
            elif process is not None:
                try: os.killpg(process.pid,signal.SIGKILL)
                except ProcessLookupError: pass
                except PermissionError as error: group_error = error
        except BaseException as error:
            cleanup_errors.append(repr(error))
        child_reaped = False
        try:
            if process is not None:
                process.wait(timeout=10)
            child_reaped = True
        except BaseException as error:
            cleanup_errors.append(repr(error))
        if group_error is not None:
            group_cleanup = {"signal_error":repr(group_error), "child_reaped":child_reaped,
                             "group_absent_after_reap":False}
            # Darwin can report EPERM for a group containing only zombies.
            # Reaping the known child is not sufficient: prove the entire group
            # absent, without sending another destructive signal after reap.
            if sys.platform == "darwin" and group_error.errno == errno.EPERM and child_reaped:
                try: os.killpg(process.pid,0)
                except ProcessLookupError: group_cleanup["group_absent_after_reap"] = True
                except BaseException as error: group_cleanup["probe_error"] = repr(error)
            if not group_cleanup["group_absent_after_reap"]:
                cleanup_errors.append(repr(group_error))
        try:
            if process is not None:
                if process.stdin is not None and not process.stdin.closed: process.stdin.close()
                if reader is None and process.stdout is not None: process.stdout.close()
        except BaseException as error:
            cleanup_errors.append(repr(error))
        stopped.set()
        try:
            if reader is not None:
                reader.join(timeout=10)
                require(not reader.is_alive(), "output reader not reaped")
        except BaseException as error:
            cleanup_errors.append(repr(error))
        reaped = not cleanup_errors
        if cleanup_errors:
            reason = (reason or "") + " cleanup: " + "; ".join(cleanup_errors)
        receipt = {"argv":argv,"exit":process.returncode if process else None,"stop_reason":reason,"seconds":time.monotonic()-start,"log_bytes":count,"log_sha256":digest(log_path.read_bytes()) if log_path.exists() else None,"owned_children_reaped":reaped}
        if group_cleanup is not None:
            receipt["posix_group_cleanup"] = group_cleanup
        save(log_path.with_suffix(".json"),receipt)
    require(reason is None and process.returncode == expected_exit, f"command failed: {receipt}")
    return log_path.read_text(encoding="utf-8",errors="replace")



def regular_bytes(path, maximum):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, 'regular bounded file')
    data = path.read_bytes()
    require(len(data) == info.st_size, 'stable file length')
    return data


def directory_bytes(root):
    total = 0
    for path in root.rglob('*'):
        info = path.lstat()
        require(not stat.S_ISLNK(info.st_mode), 'owned output symlink')
        if stat.S_ISREG(info.st_mode):
            total += info.st_size
        else:
            require(stat.S_ISDIR(info.st_mode), 'owned output special file')
    return total


def verify_producers(records):
    for record in records.values():
        path = Path(record['path'])
        require(digest(regular_bytes(path, 512 * 1024**2)) == record['sha256'], 'producer changed')


def artifact_records(text, root, expected, normal_cli=False):
    manifest = (root / 'crates/orr_terrain_import/Cargo.toml').resolve()
    rows = []
    for line in text.splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get('reason') != 'compiler-artifact' or not row.get('executable'):
            continue
        if Path(row.get('manifest_path', '')).resolve() == manifest:
            rows.append(row)
    selected = {}
    for kind, name in expected:
        matching = [row for row in rows if row.get('profile', {}).get('test') is True
                    and row.get('target', {}).get('kind') == [kind]
                    and row['target'].get('name') == name]
        require(len(matching) == 1, 'exact test producer')
        selected[kind + ':' + name] = matching[0]
    require(sum(row.get('profile', {}).get('test') is True for row in rows) == len(expected), 'unexpected test producer')
    if normal_cli:
        matching = [row for row in rows if row.get('profile', {}).get('test') is False
                    and row.get('target', {}).get('kind') == ['bin']
                    and row['target'].get('name') == PACKAGE]
        require(len(matching) == 1, 'actual embedded CLI producer')
        selected['actual-cli'] = matching[0]
    return selected


def audit_captures(root):
    expected = {name + suffix for name in FRAMES for suffix in ('.png', '.rgba')}
    require({p.name for p in root.iterdir()} == expected, 'exact capture census')
    raws, records = {}, {}
    for name in FRAMES:
        raw = regular_bytes(root / (name + '.rgba'), 320 * 256 * 4)
        require(len(raw) == 320 * 256 * 4, 'RGBA dimensions')
        png = regular_bytes(root / (name + '.png'), 2 * 1024**2)
        require(png_rgba(png, (320, 256)) == raw, 'PNG/RGBA equality')
        raws[name] = raw
        for suffix, data in (('.rgba', raw), ('.png', png)):
            records[name + suffix] = {'sha256': digest(data), 'bytes': len(data),
                                     'width': 320, 'height': 256, 'rgba_sha256': digest(raw)}
    different = lambda a, b: sum(a[i:i+4] != b[i:i+4] for i in range(0, len(a), 4))
    controls = {}
    for view in ('oblique', 'side'):
        base = raws[view + '-baseline']
        green = sum(base[i+1] > base[i] and base[i+1] > base[i+2] for i in range(0, len(base), 4))
        require(green > 200, 'visible terrain')
        controls[view + '-green'] = green
        for change in ('flip-z', 'transpose'):
            count = different(base, raws[view + '-' + change])
            require(count > 100, 'orientation control')
            controls[view + '-' + change] = count
    controls['top-markers'] = different(raws['top-baseline'], raws['top-markers'])
    require(controls['top-markers'] > 30, 'marker control')
    require(raws['side-markers'] != raws['side-wrong-height'], 'height control differs')
    require(directory_bytes(root) <= CAPTURE_CAP, 'capture budget')
    return {'files': records, 'controls': controls, 'visual_review_required': True,
            'fixed_height_masks_enforced_by_original_rust_fixture': True}


def evidence_path(path, phase, env):
    require(env.get('GITHUB_ACTIONS') == 'true', 'hosted gate only; local work needs separate offline plan')
    for key in ('GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT'):
        require(re.fullmatch(r'[1-9][0-9]*', env.get(key, '')) is not None, 'numeric hosted identity')
    parent = Path(env['RUNNER_TEMP']).resolve(strict=True)
    expected = f"raw16-{phase}-{env['GITHUB_RUN_ID']}-{env['GITHUB_RUN_ATTEMPT']}"
    require(path.name == expected and path.parent.resolve(strict=True) == parent, 'owned evidence destination')
    require(not path.exists() and not path.is_symlink(), 'fresh evidence only')
    return parent / expected


def compiler_environment(env):
    for key in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        require(not env.get(key), 'unexpected compiler override')


class Gate:
    def __init__(self, root, evidence, phase):
        self.root = root.resolve(strict=True)
        self.phase = phase
        self.commands, self.producers = [], {}
        self.env = os.environ.copy()
        self.evidence = evidence_path(evidence, phase, self.env)
        self.evidence.mkdir(mode=0o700)
        self.owned = True
        self.env.update(CARGO_TERM_COLOR='never', PYTHONDONTWRITEBYTECODE='1')
        compiler_environment(self.env)
        for key in list(self.env):
            if key.startswith('ORR_') and ('CAPTURE' in key or 'SCREENSHOT' in key):
                del self.env[key]
        self.runtime, self.cache, self.captures = [self.evidence / name for name in ('runtime', 'cache', 'captures')]
        for directory in (self.runtime, self.cache, self.captures, self.cache / 'mesa'):
            directory.mkdir(mode=0o700)
        self.env.update(TMPDIR=str(self.runtime), TEMP=str(self.runtime), TMP=str(self.runtime),
                        XDG_RUNTIME_DIR=str(self.runtime), XDG_CACHE_HOME=str(self.cache),
                        MESA_SHADER_CACHE_DIR=str(self.cache / 'mesa'), MESA_SHADER_CACHE_MAX_SIZE='16M')
        self.identity = {name: self.git(args).strip() for name, args in [
            ('head', ['rev-parse', 'HEAD']), ('tree', ['rev-parse', 'HEAD^{tree}']),
            ('parents', ['show', '-s', '--format=%P', 'HEAD'])]}
        self.identity.update(run_id=self.env['GITHUB_RUN_ID'], run_attempt=self.env['GITHUB_RUN_ATTEMPT'],
                             runner_os=self.env.get('RUNNER_OS'), runner_arch=self.env.get('RUNNER_ARCH'),
                             cargo_target_dir=self.env.get('CARGO_TARGET_DIR', 'target (job default)'),
                             target_cleanup=False, phase=phase)
        self.source = self.snapshot()
        require('tools/raw16_import_gate.py' in self.source and 'tools/test_raw16_import_gate.py' in self.source, 'tracked helper source')
        save(self.evidence / 'identity.json', self.identity)
        save(self.evidence / 'source.json', self.source)

    def git(self, args):
        completed = subprocess.run(['git', *args], cwd=self.root, stdin=subprocess.DEVNULL,
                                   capture_output=True, timeout=10, check=True)
        require(len(completed.stdout) <= 4 * 1024**2 and len(completed.stderr) <= 65536, 'Git metadata bound')
        return completed.stdout.decode('utf-8')

    def snapshot(self):
        self.git(['diff', '--quiet', '--ignore-submodules', '--'])
        self.git(['diff', '--cached', '--quiet', '--ignore-submodules', '--'])
        records = {}
        for row in self.git(['ls-files', '--stage', '-z']).split('\0'):
            if not row:
                continue
            metadata, name = row.split('\t', 1)
            mode, blob, stage = metadata.split(' ')
            require(mode in ('100644', '100755') and stage == '0', 'regular conflict-free tracked source')
            require(name not in records and not Path(name).is_absolute() and '..' not in Path(name).parts, 'tracked path')
            data = regular_bytes(self.root / name, 64 * 1024**2)
            records[name] = {'mode': mode, 'index_blob': blob, 'sha256': digest(data)}
        require(records, 'nonempty source inventory')
        return records

    def fence(self):
        require(self.snapshot() == self.source, 'source fence')
        require(self.git(['rev-parse', 'HEAD']).strip() == self.identity['head'], 'checkout fence')
        verify_producers(self.producers)
        require(directory_bytes(self.captures) <= CAPTURE_CAP, 'capture budget')
        require(directory_bytes(self.cache) <= CACHE_CAP, 'cache budget')

    def run(self, label, argv, timeout=1200):
        self.fence()
        path = self.evidence / f'{len(self.commands)+1:03d}-{label}.log'
        self.commands.append({'label': label, 'argv': argv, 'log': path.name})
        try:
            return bounded_run(argv, self.root, self.env, path, timeout)
        finally:
            self.fence()

    def compile(self, label, features, expected, native=False):
        argv = ['cargo', 'test', '--locked', '--release', '-p', PACKAGE]
        if features:
            argv += ['--no-default-features', '--features', features]
        argv += ['--tests'] if native else ['--test', expected[0][1]]
        argv += ['--no-run', '--message-format=json']
        text = self.run(label, argv)
        rows = artifact_records(text, self.root, expected, normal_cli=native)
        binaries = {}
        for key, row in rows.items():
            original = Path(row['executable']).resolve(strict=True)
            data = regular_bytes(original, 512 * 1024**2)
            stamp = {'path': str(original), 'sha256': digest(data), 'cargo_artifact': row,
                     'role': 'actual CARGO_BIN_EXE producer' if key == 'actual-cli' else 'Cargo test producer'}
            self.producers[key + ':original'] = stamp
            if key == 'actual-cli':
                continue
            saved = self.evidence / (key.replace(':', '-') + ('.exe' if os.name == 'nt' else ''))
            require(not saved.exists(), 'new saved producer')
            shutil.copy2(original, saved)
            self.producers[key + ':saved'] = {'path': str(saved), 'sha256': digest(data), 'role': 'saved test producer'}
            binaries[key] = saved
        verify_producers(self.producers)
        return binaries

    def test(self, label, binary, names, ignored=(), device=False):
        inventory(self.run(label + '-list', [str(binary), '--list']), names)
        inventory(self.run(label + '-ignored-list', [str(binary), '--ignored', '--list']), list(ignored))
        argv = [str(binary)]
        if device:
            argv += ['--exact', DEVICE, '--ignored']
        argv += ['--test-threads=1', '--show-output']
        text = self.run(label + '-run', argv, timeout=180)
        result(text, names)
        return text

    def helper(self):
        helper_result(self.run('helper-tests', [sys.executable, '-B', str(self.root / 'tools/test_raw16_import_gate.py'), '-v'], timeout=120))

    def strict(self, label, features='', feature_off=False):
        argv = ['cargo', 'clippy', '--locked', '--release', '-p', PACKAGE]
        if features or feature_off:
            argv.append('--no-default-features')
        if features:
            argv += ['--features', features]
        self.run(label, argv + ['--all-targets', '--', '-D', 'warnings'])

    def native(self):
        self.helper()
        lanes = native_lanes(sys.platform)
        binaries = self.compile('native-compile', '', [(kind, name) for kind, name, _ in lanes], native=True)
        for kind, name, names in lanes:
            self.test(kind if kind != 'test' else name, binaries[kind + ':' + name], names)
        self.strict('default-strict')
        self.strict('feature-off-strict', feature_off=True)
        for label, flags in [('default-tree', []), ('feature-off-tree', ['--no-default-features'])]:
            dependency_tree(self.run(label, ['cargo', 'tree', '--locked', '-p', PACKAGE,
                                            '--edges', 'normal', '--prefix', 'none', *flags]))
        save(self.evidence / 'native-summary.json', {'passed': sum(len(row[2]) for row in lanes),
             'ignored': 0, 'filtered': 0, 'device_executed': False, 'platform': sys.platform})

    def editor(self):
        self.helper()
        binaries = self.compile('editor-compile', 'editor-acceptance', [('test', 'editor')])
        self.test('editor', binaries['test:editor'], EDITOR)
        for label, features in [('editor', 'editor-acceptance'), ('gpu', 'gpu-acceptance'),
                                ('combined', 'editor-acceptance,gpu-acceptance')]:
            self.strict(label + '-strict', features)
        save(self.evidence / 'editor-summary.json', {'passed': 3, 'ignored': 0, 'filtered': 0,
             'strict_configurations': 3, 'device_executed': False, 'native_window_executed': False})

    def device(self):
        require(sys.platform == 'linux', 'Linux software-device lane')
        for key in ('ORR_REQUIRE_GPU', 'ORR_RAW16_REQUIRE_SOFTWARE'):
            require(self.env.get(key) == '1', 'required software-device environment')
        require(self.env.get('WGPU_BACKEND') == 'vulkan', 'requested backend')
        icds = sorted(Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json'))
        require(len(icds) == 1, 'one installed lavapipe ICD')
        icd = icds[0].resolve(strict=True)
        self.env.update(VK_ICD_FILENAMES=str(icd), VK_DRIVER_FILES=str(icd),
                        LIBGL_ALWAYS_SOFTWARE='1', ORR_RAW16_CAPTURE_DIR=str(self.captures))
        save(self.evidence / 'driver.json', {'path': str(icd), 'sha256': digest(regular_bytes(icd, 65536)),
             'requested_backend': 'vulkan', 'not_a_full_observed_adapter_tuple': True})
        require(not any(self.captures.iterdir()), 'empty device captures')
        binaries = self.compile('device-compile', 'gpu-acceptance', [('test', 'render')])
        text = self.test('render', binaries['test:render'], [DEVICE], [DEVICE], device=True)
        observed = adapter(text)
        captures = audit_captures(self.captures)
        save(self.evidence / 'device-summary.json', {'passed': 1, 'ignored': 0, 'filtered': 0,
             'adapter': observed, 'captures': captures, 'native_window_executed': False,
             'physical_gpu_executed': False, 'cross_adapter_golden_claimed': False})


def main():
    require(sys.flags.optimize == 0, 'Python optimization unsupported')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('native', 'editor', 'device'))
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    gate = Gate.__new__(Gate)
    gate.owned = False
    state = 'STOPPED'
    try:
        gate.__init__(Path(__file__).resolve().parents[1], args.evidence, args.phase)
        getattr(gate, args.phase)()
        require([entry['label'] for entry in gate.commands] == command_inventory(args.phase), 'command census')
        gate.fence()
        state = 'PASSED'
    finally:
        if gate.owned:
            receipts = [gate.evidence / Path(entry['log']).with_suffix('.json') for entry in gate.commands]
            reaped = bool(receipts) and all(path.is_file() and json.loads(path.read_text()).get('owned_children_reaped') for path in receipts)
            source_ok, fence_error = False, None
            try:
                source_ok = gate.snapshot() == gate.source
            except Exception as error:
                fence_error = repr(error)
            if not reaped or not source_ok:
                state = 'STOPPED'
            source_file = gate.evidence / 'source.json'
            save(gate.evidence / 'terminal.json', {'state': state, 'phase': args.phase,
                 'identity': getattr(gate, 'identity', None),
                 'source_manifest_sha256': digest(source_file.read_bytes()) if source_file.is_file() else None,
                 'commands': gate.commands, 'producers': gate.producers, 'owned_children_reaped': reaped,
                 'source_fence': source_ok, 'fence_error': fence_error, 'target_cleanup': False,
                 'capture_cache_budget_supervision': 'pre/post-command; time/log/disk continuously bounded',
                 'old_ci_transfer': False})
            require(state == 'PASSED', 'RAW16 acceptance stopped; retained terminal evidence')


if __name__ == '__main__':
    main()
