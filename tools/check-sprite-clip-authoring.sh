#!/usr/bin/env bash
# Mandatory Linux acceptance. Local execution additionally needs an admitted lease.
# Python is standard-library-only; every producer refreshes the complete local
# tracked source closure, never claiming cached local dependencies as fresh proof.
set -euo pipefail
test "$#" -le 1
cd "$(dirname "${BASH_SOURCE[0]}")/.."
exec python3 - "${1:-all}" "$PWD" <<'PY'
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import time
import tomllib
import traceback
import zlib

MIB = 1024 * 1024
GIB = 1024 * MIB
if sys.flags.optimize != 0:
    raise RuntimeError('Python optimization is forbidden: parser and platform asserts must remain active')
MODE = sys.argv[1]
ROOT = Path(sys.argv[2]).resolve(strict=True)
assert MODE in {'contracts', 'acceptance', 'all'}, 'usage: check-sprite-clip-authoring.sh [contracts|acceptance|all]'
assert sys.version_info[:2] == (3, 12), 'the gate requires the CI-pinned Python 3.12'
assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
CORE = '''admission_limits_and_closed_package_inventory_are_enforced
bounded_history_and_branching_leave_other_content_identical
changed_stage_bytes_are_rejected_before_publication
concurrent_destination_and_postpublish_failure_have_honest_outcomes
filesystem_save_publishes_exact_new_bytes_and_never_replaces_existing_paths
filesystem_save_rejects_symlink_ancestors_destinations_and_traversal
injected_partial_write_retains_only_private_stage_and_never_deletes_old_files
invalid_edits_and_noops_preserve_document_and_redo
new_version_save_plan_preserves_original_pixels_license_and_other_clips
ordered_frames_and_exact_loop_once_boundaries
oversized_frame_and_version_inputs_reject_without_history_change
save_plan_rejects_old_versions_changed_snapshots_and_foreign_documents
sha256_digest_has_exact_lowercase_64_character_known_vectors
unexpected_stage_entry_rejects_closed_inventory
unsafe_destination_ancestry_rejects_without_writes'''.splitlines()
CORE = ['sprite_clip_authoring::tests::' + name for name in CORE]
PANEL = '''installed_package_admission_decodes_real_png_and_rejects_bad_atlas
invalid_widget_drafts_do_not_commit_or_destroy_redo
real_widgets_edit_order_duration_mode_and_undo_redo
widget_history_is_bounded_and_region_choice_is_transactional
widget_save_as_preserves_pixels_license_and_existing_destination
widget_save_as_rejects_old_version_before_filesystem_write'''.splitlines()
BOUNDARY = ['sprite_clip_registration_is_default_off_and_bin_is_gated']
OFF_BOUNDARY = ['default_profile_does_not_enable_sprite_clip_feature', *BOUNDARY]
ORDINARY = 'authored_package_widgets_install_assign_reopen_and_native_frames'
GPU = 'authored_clip_composed_gpu_and_source_hidden_readonly_export'
APP = 'tests::production_app_controls_render_edit_and_save_admitted_package'
GUARDS = ['bins_without_test_harnesses_have_no_tests_or_local_modules',
          'the_testless_bin_guard_catches_attributes_modules_and_generated_sources']
FEATURES = ['default', 'sprite-clip-editor', 'sprites']
PROJECT = ['default', 'project', 'sprites']
EXPORT = ['default', 'project', 'project-export', 'sprites']
FEATURE_ARGS = ['--features', 'sprite-clip-editor']
SKIP = re.compile(r'(?i)\bskip(?:s|ped|ping)?\b|GPU unavailable|no GPU adapter|fallback execution')
CURRENT = None
PINS = []


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(MIB), b''):
            h.update(chunk)
    return h.hexdigest()


def write_json(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def outside(path, parent):
    return not path.is_relative_to(parent)


def directory(name, fallback):
    supplied = Path(os.environ.get(name, fallback))
    require(supplied.is_absolute(), f'{name} must be absolute')
    supplied.mkdir(parents=True, exist_ok=True)
    require(not supplied.is_symlink(), f'{name} must not be a symlink')
    path = supplied.resolve(strict=True)
    require(outside(path, ROOT) and outside(ROOT, path), f'{name} must be disjoint from the source checkout')
    return path


EVIDENCE_BASE = directory('ORR_SPRITE_EVIDENCE_ROOT', '/tmp/orr-sprite-clip-evidence')
WORK_BASE = directory('ORR_SPRITE_WORK_ROOT', '/tmp/orr-sprite-clip-work')
require(outside(EVIDENCE_BASE, WORK_BASE) and outside(WORK_BASE, EVIDENCE_BASE), 'evidence and work roots must be disjoint')
EVIDENCE = Path(tempfile.mkdtemp(prefix=MODE + '-', dir=EVIDENCE_BASE))
WORK = Path(tempfile.mkdtemp(prefix=MODE + '-', dir=WORK_BASE))
TARGET = Path(os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target'))).resolve()
TARGET.mkdir(parents=True, exist_ok=True)
for path in (EVIDENCE, WORK):
    require(outside(TARGET, path) and outside(path, TARGET), 'build target and evidence/work roots must be disjoint')
LOCK = (TARGET / '.orr-sprite-clip-gate.lock').open('a')
fcntl.flock(LOCK, fcntl.LOCK_EX | fcntl.LOCK_NB)
ENV = os.environ.copy()
# Profile/runner injection makes recorded release identity ambiguous. No wrapper,
# custom rustflags or profile override is silently trusted by this dedicated gate.
for key, value in ENV.items():
    if value and (key in {'RUSTC', 'CARGO', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER',
                         'RUSTC_WORKSPACE_WRAPPER', 'CARGO_BUILD_TARGET'}
                  or key.startswith('CARGO_PROFILE_RELEASE_')
                  or (key.startswith('CARGO_TARGET_') and key.endswith('_RUNNER'))):
        require(False, f'unsupported identity override: {key}')
ENV.update(CARGO_TARGET_DIR=str(TARGET), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0',
           CARGO_TERM_COLOR='never', RUST_BACKTRACE='1', PYTHONUNBUFFERED='1')
print(f'Sprite clip {MODE} evidence: {EVIDENCE}', flush=True)


def reserve(extra=0):
    for path in {ROOT, TARGET, EVIDENCE, WORK, Path('/tmp')}:
        require(shutil.disk_usage(path).free >= GIB + extra,
                f'fixed 1 GiB reserve plus {extra} bytes required at {path}')


def stop(signum, _frame):
    if CURRENT is not None and CURRENT.poll() is None:
        os.killpg(CURRENT.pid, signal.SIGKILL)
    raise KeyboardInterrupt(f'interrupted by signal {signum}')


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)


def run(name, argv, cwd=ROOT, expected=0, timeout=1800, extra_env=None, build=False):
    global CURRENT
    reserve(1536 * MIB if build else 0)
    env = ENV | (extra_env or {})
    # Only task-specific, non-secret knobs are persisted, never the inherited env.
    recorded = {key: value for key, value in env.items()
                if key.startswith('ORR_SPRITE_') or key in {
                    'ORR_REQUIRE_GPU', 'CARGO_TARGET_DIR', 'CARGO_BUILD_JOBS',
                    'CARGO_INCREMENTAL', 'CARGO_TERM_COLOR', 'WGPU_BACKEND',
                    'VK_ICD_FILENAMES', 'VK_DRIVER_FILES', 'LIBGL_ALWAYS_SOFTWARE', 'XDG_RUNTIME_DIR'}}
    start = time.time_ns()
    print(f'[{name}] {shlex.join(map(str, argv))}', flush=True)
    status = None
    failure = None
    try:
        with (EVIDENCE / f'{name}.stdout').open('xb') as stdout, (EVIDENCE / f'{name}.stderr').open('xb') as stderr:
            CURRENT = subprocess.Popen(list(map(str, argv)), cwd=cwd, env=env,
                                       stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                       start_new_session=True)
            deadline = time.monotonic() + timeout
            while CURRENT.poll() is None:
                # Preserve the fixed floor throughout compilation and every
                # child runtime, not merely at the command's endpoints. Any
                # breach follows the same group-kill/reap/failure-receipt path.
                reserve()
                if time.monotonic() >= deadline:
                    os.killpg(CURRENT.pid, signal.SIGKILL)
                    CURRENT.wait()
                    raise RuntimeError(f'{name}: timeout after {timeout}s')
                if max(stdout.tell(), stderr.tell()) > 128 * MIB:
                    os.killpg(CURRENT.pid, signal.SIGKILL)
                    CURRENT.wait()
                    raise RuntimeError(f'{name}: log exceeds 128 MiB cap')
                time.sleep(0.1)
            status = CURRENT.returncode
    except BaseException as error:
        failure = f'{type(error).__name__}: {error}'
        raise
    finally:
        if CURRENT is not None and CURRENT.poll() is None:
            try:
                os.killpg(CURRENT.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            status = CURRENT.wait()
        CURRENT = None
        write_json(EVIDENCE / f'{name}.command.json', {
            'argv': list(map(str, argv)), 'cwd': str(cwd), 'environment': recorded,
            'expected_exit': expected, 'actual_exit': status, 'termination': failure, 'timeout_seconds': timeout,
            'started_ns': start, 'finished_ns': time.time_ns(),
            'stdout_sha256': digest(EVIDENCE / f'{name}.stdout'),
            'stderr_sha256': digest(EVIDENCE / f'{name}.stderr')})
    require(status == expected, f'{name}: exit {status}, expected {expected}; inspect retained stderr')
    reserve()
    return ((EVIDENCE / f'{name}.stdout').read_text(),
            (EVIDENCE / f'{name}.stderr').read_text())


def source_inventory(root, names):
    entries = {}
    for name in names:
        path = root / name
        require(path.is_file() and not path.is_symlink(), f'non-regular tracked input: {path}')
        require(path.resolve().is_relative_to(root), f'input escapes source root: {path}')
        info = path.stat()
        entries[name] = {'sha256': digest(path), 'bytes': info.st_size, 'mode': stat.S_IMODE(info.st_mode)}
    return entries


files, _ = run('tracked-inputs', ['git', 'ls-files', '-z'])
NAMES = sorted(files.rstrip('\0').split('\0'))
require(NAMES and len(NAMES) == len(set(NAMES)), 'tracked source list is empty or duplicated')
for name in NAMES:
    require(not Path(name).is_absolute() and '..' not in Path(name).parts and '.git' not in Path(name).parts,
            f'unsafe source inventory entry: {name}')
REQUIRED = {
    'Cargo.toml', 'Cargo.lock', 'crates/orr_editor/Cargo.toml',
    'crates/orr_editor/src/sprite_clip_authoring.rs', 'crates/orr_editor/src/sprite_clip_panel.rs',
    'crates/orr_editor/src/bin/orr_sprite_clip.rs', 'crates/orr_editor/src/sprite_clip_app_tests.rs',
    'crates/orr_editor/tests/sprite_clip_panel.rs', 'crates/orr_editor/tests/sprite_clip_feature_boundary.rs',
    'crates/orr_editor/tests/sprite_clip_workflow.rs', 'crates/orr_editor/tests/common/sprite_clip_export.rs',
    'tools/check-sprite-clip-authoring.sh', '.github/workflows/sprite-clip-authoring.yml'}
require(REQUIRED <= set(NAMES), 'all new gate and acceptance sources must be tracked before execution')
untracked, _ = run('untracked-inputs', ['git', 'ls-files', '--others', '--exclude-standard', '-z'])
require(not [p for p in untracked.split('\0') if p and (p.startswith(('crates/', 'assets/', '.cargo/'))
        or p in {'Cargo.toml', 'Cargo.lock', 'build.rs'})], 'untracked compiler/fixture inputs are not admitted')
BEFORE = source_inventory(ROOT, NAMES)
write_json(EVIDENCE / 'source-before.json', BEFORE)
run('source-revision', ['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'])
run('source-diff', ['git', 'diff', '--no-ext-diff', '--binary', 'HEAD', '--'])
run('rustc-version', ['rustc', '-vV'])
run('cargo-version', ['cargo', '-V'])
run('python-version', [sys.executable, '--version'])
PACKAGES = {}
for name in NAMES:
    if name.endswith('/Cargo.toml') or name == 'Cargo.toml':
        content = tomllib.loads((ROOT / name).read_text())
        if 'package' in content:
            PACKAGES[name] = content['package']['name']
LOCAL_NAMES = set(PACKAGES.values())
require(PACKAGES.get('crates/orr_editor/Cargo.toml') == 'orr_editor', 'wrong workspace')
profile = tomllib.loads((ROOT / 'Cargo.toml').read_text())['profile']['release']
require(profile == {'debug': 1, 'codegen-units': 1, 'lto': 'thin'}, 'review changed release profile before updating the gate')


def refresh(name, root, baseline):
    require(source_inventory(root, NAMES) == baseline, f'{name}: source closure changed before refresh')
    cached = []
    fingerprint = TARGET / 'release/.fingerprint'
    if fingerprint.exists():
        for entry in fingerprint.iterdir():
            if any(entry.name.startswith(package + '-') for package in LOCAL_NAMES):
                cached.extend(p.stat().st_mtime_ns for p in entry.rglob('*') if p.is_file())
    cached_max = max(cached, default=0)
    now = time.time_ns()
    require(cached_max < now, f'{name}: local cached fingerprints are not older than the current clock')
    records = []
    for relative in NAMES:
        path = root / relative
        prior = path.stat().st_mtime_ns
        require(prior <= now, f'{name}: future source mtime: {relative}')
        os.utime(path, ns=(now, now), follow_symlinks=False)
        observed = path.stat().st_mtime_ns
        require(cached_max < observed <= time.time_ns(), f'{name}: refresh did not exceed cached local fingerprints')
        records.append({'path': relative, 'prior_mtime_ns': prior, 'mtime_ns': observed,
                        'sha256': baseline[relative]['sha256']})
    require(source_inventory(root, NAMES) == baseline, f'{name}: refresh changed source bytes or modes')
    write_json(EVIDENCE / f'{name}.refresh.json', {
        'root': str(root), 'scope': 'complete tracked local workspace, dependency, fixture and config inputs',
        'registry_and_other_worktrees_untouched': True, 'cached_max_mtime_ns': cached_max,
        'clock_ns': now, 'verified_at_ns': time.time_ns(), 'files': records})


def options(tokens, name):
    return [tokens[i + 1] for i, token in enumerate(tokens[:-1]) if token == name]


def running_commands(stderr):
    # Cargo shell-quotes environment values, including literal newlines/backticks.
    # A trailing backtick closes a record only outside an unfinished shell quote.
    pending = None
    start = None
    for number, line in enumerate(stderr.splitlines(), 1):
        match = re.match(r'^\s*Running `', line)
        if match:
            require(pending is None, f'nested verbose command at line {number}; unfinished line {start}')
            pending = line[match.end():]
            start = number
        elif pending is not None:
            pending += '\n' + line
        else:
            continue
        closed = pending.endswith('`')
        try:
            tokens = shlex.split(pending[:-1] if closed else pending)
        except ValueError as error:
            require(str(error) == 'No closing quotation',
                    f'malformed verbose command at line {start}: {error}')
            continue
        require(closed, f'missing verbose command delimiter at line {start}')
        require(tokens, f'empty verbose command at line {start}')
        yield tokens
        pending = None
    require(pending is None, f'unterminated verbose command at line {start}')


def commands(stderr, root):
    result = []
    for tokens in running_commands(stderr):
        if '--crate-name' not in tokens:
            continue
        settings = dict(t.split('=', 1) for t in tokens if re.match(r'^CARGO_[A-Z_]+=.*', t))
        if settings.get('CARGO_PKG_NAME') not in LOCAL_NAMES:
            continue
        manifest = settings.get('CARGO_MANIFEST_PATH')
        if manifest is None:
            manifest = str(Path(settings['CARGO_MANIFEST_DIR']) / 'Cargo.toml')
        manifest = Path(manifest).resolve()
        require(manifest.is_relative_to(root), 'verbose local compile belongs to another worktree')
        sources = [str((root / token).resolve()) for token in tokens
                   if token.endswith('.rs') and '=' not in token]
        require(len(sources) == 1, f'ambiguous verbose source: {sources}')
        result.append({'tokens': tokens, 'manifest': str(manifest), 'source': sources[0],
                       'crate': options(tokens, '--crate-name')[0], 'test': '--test' in tokens,
                       'features': sorted(x[len('feature="'):-1] for x in options(tokens, '--cfg')
                                          if x.startswith('feature="') and x.endswith('"'))})
    require(result, 'producer has no verbose local rustc/clippy invocations')
    return result


def producer_messages(name, stdout):
    # Cargo -vv forwards build-script output alongside its JSON messages.
    # Only the anchored Cargo verbose prefix is accepted as unrelated output;
    # its payload is never parsed, even if it resembles a successful build.
    verbose = re.compile(r'\[[A-Za-z0-9_][A-Za-z0-9_-]* '
                         r'[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?'
                         r'(?:\+[0-9A-Za-z.-]+)?\] [^\r\n]*')
    def unique_object(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError('duplicate JSON object key')
            value[key] = item
        return value

    def invalid_constant(value):
        raise ValueError('non-finite JSON constant')

    rows = []
    counts = {'json_objects': 0, 'verbose_build_script_lines': 0, 'blank_lines': 0}
    verbose_digest = hashlib.sha256()
    for number, raw_line in enumerate(stdout.splitlines(keepends=True), 1):
        line = raw_line.rstrip('\r\n')
        if not line.strip():
            counts['blank_lines'] += 1
        elif line.lstrip().startswith('{'):
            try:
                row = json.loads(line, object_pairs_hook=unique_object, parse_constant=invalid_constant)
            except ValueError as error:
                raise AssertionError(f'{name}: malformed producer JSON at line {number}') from error
            require(isinstance(row, dict), f'{name}: non-object producer JSON at line {number}')
            rows.append(row)
            counts['json_objects'] += 1
        else:
            require(verbose.fullmatch(line) is not None,
                    f'{name}: unrecognized producer output at line {number}')
            counts['verbose_build_script_lines'] += 1
            verbose_digest.update(raw_line.encode('utf-8'))
    return rows, {'counts': counts, 'verbose_build_script_sha256': verbose_digest.hexdigest()}


def audit_producer(name, stdout, stderr, root, package, features, action):
    rows, stream_summary = producer_messages(name, stdout)
    require([row for row in rows if row.get('reason') == 'build-finished'] ==
            [{'reason': 'build-finished', 'success': True}], f'{name}: missing unique successful JSON build-finished')
    local = [row for row in rows if row.get('reason') == 'compiler-artifact'
             and row['package_id'].startswith('path+')]
    require(local, f'{name}: missing local compiler artifacts')
    invocations = commands(stderr, root)
    emitted = {str(Path(filename).resolve()): row for row in local for filename in row['filenames']}
    proof = []
    for row in local:
        manifest = Path(row['manifest_path']).resolve()
        require(manifest.is_relative_to(root) and str(manifest.relative_to(root)) in PACKAGES,
                f'{name}: local artifact outside complete tracked source closure')
        require(row.get('fresh') is False, f'{name}: cached local artifact is not fresh provenance: {manifest}')
        matches = [entry for entry in invocations if entry['manifest'] == str(manifest)
                   and entry['source'] == str(Path(row['target']['src_path']).resolve())
                   and entry['crate'] == row['target']['name'].replace('-', '_')
                   and entry['test'] == row['profile']['test']
                   and entry['features'] == sorted(row['features'])]
        require(matches, f'{name}: no exact verbose source/test/feature identity for {row["target"]["name"]}')
        externs = []
        for match in matches:
            for argument in options(match['tokens'], '--extern'):
                dependency, sep, filename = argument.partition('=')
                if sep and (dependency in LOCAL_NAMES or str(Path(filename).resolve()) in emitted):
                    path = Path(filename).resolve(strict=True)
                    require(str(path) in emitted, f'{name}: local extern has no producer JSON artifact: {argument}')
                    require(path.is_file() and not path.is_symlink(), f'{name}: invalid local extern')
                    externs.append({'dependency': dependency, 'path': str(path),
                                    'bytes': path.stat().st_size, 'sha256': digest(path)})
        proof.append({'artifact': row, 'verbose': matches, 'local_externs': externs})
    roots = [row for row in local if Path(row['manifest_path']).resolve() == root / f'crates/{package}/Cargo.toml']
    require(roots, f'{name}: selected package has no artifacts')
    for row in roots:
        require(sorted(row['features']) == sorted(features), f'{name}: wrong selected feature closure: {row["features"]}')
        expected = {'opt_level': '3', 'debuginfo': 1, 'debug_assertions': False,
                    'overflow_checks': False, 'test': row['profile']['test']}
        require(row['profile'] == expected, f'{name}: exact release profile changed: {row["profile"]}')
        matching = [entry for entry in invocations if entry['manifest'] == str(Path(row['manifest_path']).resolve())
                    and entry['source'] == str(Path(row['target']['src_path']).resolve())
                    and entry['test'] == row['profile']['test'] and entry['features'] == sorted(row['features'])]
        require(all({'opt-level=3', 'codegen-units=1', 'debuginfo=1'} <= set(options(entry['tokens'], '-C'))
                    for entry in matching), f'{name}: selected verbose release flags differ')
        if row.get('executable'):
            require(all('lto=thin' in options(entry['tokens'], '-C') for entry in matching),
                    f'{name}: selected executable was not linked with declared thin LTO')
    write_json(EVIDENCE / f'{name}.provenance.json', {
        'action': action, 'package': package, 'expected_features': features,
        'root': str(root), 'local_artifacts': proof, 'selected_artifacts': roots,
        'producer_stdout_stream': stream_summary,
        'producer_stdout_sha256': digest(EVIDENCE / f'{name}.stdout'),
        'producer_stderr_sha256': digest(EVIDENCE / f'{name}.stderr')})
    return roots


def elf(path, cap, expected=None):
    require(path.is_file() and not path.is_symlink(), f'not a regular ELF file: {path}')
    info = path.stat()
    require(64 <= info.st_size <= cap and info.st_mode & 0o111, f'executable cap/mode mismatch: {path}')
    with path.open('rb') as stream:
        header = stream.read(64)
    require(header[:7] == b'\x7fELF\x02\x01\x01' and struct.unpack_from('<H', header, 16)[0] in (2, 3)
            and struct.unpack_from('<H', header, 18)[0] == 62
            and struct.unpack_from('<H', header, 52)[0] == 64, f'not Linux x86_64 full ELF: {path}')
    measured = {'bytes': info.st_size, 'sha256': digest(path)}
    require(expected is None or measured == expected, f'full executable bytes changed: {path}')
    return measured


def producer(name, package, features, feature_args, selector, *, action='test', root=ROOT,
             baseline=BEFORE, cap=128*MIB, freeze=True):
    refresh(name, root, baseline)
    argv = ['cargo', action, '--release', '--locked', '-j1', '-p', package,
            *feature_args, *selector, '--message-format=json', '-vv']
    if action == 'test':
        argv.append('--no-run')
    if action == 'clippy':
        argv += ['--', '-D', 'warnings']
    stdout, stderr = run(name, argv, cwd=root, build=True)
    require(source_inventory(root, NAMES) == baseline, f'{name}: source bytes changed during producer')
    roots = audit_producer(name, stdout, stderr, root, package, features, action)
    if not freeze:
        return roots
    kind, target_name = (('lib', package) if selector == ['--lib'] else (selector[0][2:], selector[1]))
    found = [row for row in roots if row.get('executable') and row['target']['name'] == target_name
             and row['target']['kind'] == [kind] and row['profile']['test'] == (action == 'test')]
    require(len(found) == 1, f'{name}: expected one exact selected executable, got {len(found)}')
    row = found[0]
    original = Path(row['executable'])
    require(original.is_absolute() and original.resolve(strict=True) == original
            and original.is_relative_to(TARGET / 'release'), f'{name}: wrong producer executable path')
    identity = elf(original, cap)
    reserve(identity['bytes'])
    folder = EVIDENCE / 'pinned-tools' / name
    folder.mkdir(parents=True)
    frozen = folder / original.name
    with original.open('rb') as src, frozen.open('xb') as dst:
        shutil.copyfileobj(src, dst, MIB)
    frozen.chmod(0o555)
    elf(frozen, cap, identity)
    pin = {'name': name, 'root': root, 'package': package, 'feature_args': feature_args,
           'selector': selector, 'original': original, 'frozen': frozen, 'identity': identity,
           'cap': cap, 'producer': row, 'baseline': baseline}
    write_json(EVIDENCE / f'{name}.artifact.json', {
        'producer': row, 'original': str(original), 'frozen': str(frozen), **identity,
        'cap': cap, 'producer_provenance_sha256': digest(EVIDENCE / f'{name}.provenance.json')})
    PINS.append(pin)
    return pin


def fence(pin):
    elf(pin['original'], pin['cap'], pin['identity'])
    elf(pin['frozen'], pin['cap'], pin['identity'])
    require(source_inventory(pin['root'], NAMES) == pin['baseline'], 'source changed between build and execution')


def test_command(pin, test_filter='', arguments=()):
    command = ['cargo', 'test', '--offline', '--release', '--locked', '-j1', '-p', pin['package'],
               *pin['feature_args'], *pin['selector']]
    if test_filter:
        command.append(test_filter)
    return command + ['--', *arguments]


def running(pin, stderr):
    candidates = [line for line in stderr.splitlines() if re.match(r'^\s*Running\b', line)]
    require(len(candidates) == 1, f'exactly one Cargo Running entry required: {candidates}')
    selector = pin['selector']
    if selector == ['--lib']:
        label = 'unittests src/lib.rs'
    elif selector[0] == '--bin':
        label = f'unittests src/bin/{selector[1]}.rs'
    else:
        label = f'tests/{selector[1]}.rs'
    expected = re.fullmatch(r'\s*Running ' + re.escape(label) + r' \(([^\r\n]+)\)', candidates[0])
    require(expected and expected[1] == str(pin['original']), f'Cargo Running label/path mismatch: {candidates[0]}')
    require(not re.search(r'^\s*(Compiling|Checking|Dirty)\b', stderr, re.M), 'execution unexpectedly rebuilt its producer')
    return {'line': candidates[0], 'label': label, 'path': expected[1], **pin['identity']}


def inventory(pin, expected, ignored=(), test_filter='', exact=False):
    require(len(expected) == len(set(expected)) and set(ignored) <= set(expected), 'invalid fixed inventory')
    if not test_filter:
        pin['inventory_total'] = len(expected)
    for ignored_only in (False, True):
        suffix = 'ignored' if ignored_only else 'list'
        name = pin['name'] + '-' + suffix
        args = ['--list', '--format=pretty'] + (['--ignored'] if ignored_only else []) + (['--exact'] if exact else [])
        fence(pin)
        stdout, stderr = run(name, test_command(pin, test_filter, args), cwd=pin['root'])
        fence(pin)
        identity = running(pin, stderr)
        names = re.findall(r'^([^\s]+): test$', stdout, re.M)
        wanted = list(ignored if ignored_only else expected)
        require(sorted(names) == sorted(wanted) and len(names) == len(set(names)), f'{name}: exact names differ: {names}')
        require(re.findall(r'^(\d+) tests?, (\d+) benchmarks?$', stdout, re.M) == [(str(len(wanted)), '0')],
                f'{name}: missing exact list cardinality')
        require(not re.search(r': benchmark$|^test result:', stdout, re.M), f'{name}: non-test inventory output')
        require(not SKIP.search(stdout + stderr), f'{name}: skip/fallback output')
        write_json(EVIDENCE / f'{name}.verified.json', {'names': names, 'ignored_only': ignored_only, 'running': identity})


def execute(pin, name, expected, ignored=(), test_filter='', *, exact=False, ignored_only=False, negative=False, extra_env=None):
    args = ['--test-threads=1', '--nocapture', '--format=pretty']
    if exact:
        args.append('--exact')
    if ignored_only:
        args.append('--ignored')
    fence(pin)
    stdout, stderr = run(name, test_command(pin, test_filter, args), cwd=pin['root'],
                         expected=101 if negative else 0, extra_env=extra_env)
    fence(pin)
    identity = running(pin, stderr)
    # With --nocapture a panic/error can split a libtest status line across
    # stderr; stdout still has one canonical named line and one final summary.
    results = re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?:, [^\r\n]*)?$', stdout, re.M)
    wanted_status = 'FAILED' if negative else 'ok'
    wanted = sorted([(test, wanted_status) for test in expected] + [(test, 'ignored') for test in ignored])
    require(sorted(results) == wanted, f'{name}: wrong executed identities: {results}')
    summaries = re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in [^\r\n]+$', stdout, re.M)
    require(len(summaries) == 1 and stdout.count('test result:') == 1, f'{name}: missing unique summary')
    require(summaries[0][:5] == (('FAILED' if negative else 'ok'), '0' if negative else str(len(expected)),
            str(len(expected)) if negative else '0', str(len(ignored)), '0'), f'{name}: wrong result counts: {summaries}')
    require(re.findall(r'^running (\d+) tests?$', stdout, re.M) == [(str(len(expected) + len(ignored)))],
            f'{name}: wrong running-test cardinality')
    filtered = (pin.get('inventory_total', 0) - len(expected) - len(ignored)) if test_filter else 0
    if not test_filter or 'inventory_total' in pin:
        require(int(summaries[0][5]) == filtered, f'{name}: wrong filtered-test cardinality')
    require(not SKIP.search(stdout + stderr), f'{name}: skip/fallback is not mandatory acceptance')
    if negative:
        require(expected == ['real_widgets_edit_order_duration_mode_and_undo_redo'] and not ignored,
                'the negative proof is the exact panel singleton only')
        require(re.search(r'panicked at crates/orr_editor/tests/sprite_clip_panel\.rs:60:[0-9]+:', stderr),
                'omitted Apply must fail at the unchanged intended line-60 duration assertion')
        require('assertion `left == right` failed' in stderr
                and re.search(r'left:\s+100\b', stderr) and re.search(r'right:\s+125\b', stderr),
                'omitted Apply must fail at the intended unchanged-duration assertion')
    write_json(EVIDENCE / f'{name}.verified.json', {
        'results': results, 'summary': summaries[0], 'expected_negative': negative, 'running': identity})
    return stdout + stderr


def cpu_target(name, selector, expected, ignored=(), *, feature_args=FEATURE_ARGS, features=FEATURES,
               package='orr_editor', test_filter='', exact=False, root=ROOT, baseline=BEFORE, cap=128*MIB):
    pin = producer(name, package, features, feature_args, selector, root=root, baseline=baseline, cap=cap)
    inventory(pin, expected, ignored, test_filter, exact)
    execute(pin, name + '-full', [n for n in expected if n not in ignored], ignored, test_filter, exact=exact)
    return pin


def metadata_boundary(name, no_default):
    args = ['cargo', 'metadata', '--locked', '--format-version=1', '--filter-platform',
            'x86_64-unknown-linux-gnu', '--manifest-path', str(ROOT / 'crates/orr_editor/Cargo.toml')]
    if no_default:
        args.append('--no-default-features')
    stdout, _ = run(name, args)
    value = json.loads(stdout)
    package = next(p for p in value['packages'] if Path(p['manifest_path']).resolve() == ROOT / 'crates/orr_editor/Cargo.toml')
    require(package['features']['default'] == [], 'default editor features must remain empty')
    bins = [t for t in package['targets'] if t['name'] == 'orr_sprite_clip']
    require(len(bins) == 1 and bins[0]['kind'] == ['bin']
            and bins[0]['required-features'] == ['sprite-clip-editor'] and bins[0]['test'] is True,
            'standalone binary registration/harness boundary changed')
    node = next(n for n in value['resolve']['nodes'] if n['id'] == package['id'])
    require(set(node['features']) <= {'default'}, f'default-off metadata activated optional editor features: {node["features"]}')
    enabled_dependencies = sorted(d['name'] for d in node['deps'])
    require(not {'orr_sprite', 'orr_package', 'sha2', 'rustix', 'tempfile', 'serde'} & set(enabled_dependencies),
            'default-off metadata activated an optional authoring dependency')
    write_json(EVIDENCE / f'{name}.verified.json', {
        'manifest': package['manifest_path'], 'features': node['features'],
        'dependencies': enabled_dependencies, 'standalone_bin': bins[0],
        'metadata_sha256': digest(EVIDENCE / f'{name}.stdout')})


def no_apply_control():
    # A byte-identical isolated workspace protects the checkout even on SIGKILL.
    # Its own positive/restored ELF identity is compared: debug source paths are
    # never silently transferred from the original worktree's producer.
    control = WORK / 'no-apply-source'
    control.mkdir()
    reserve(sum(entry['bytes'] for entry in BEFORE.values()) + 384*MIB)
    for relative in NAMES:
        destination = control / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, destination)
    require(source_inventory(control, NAMES) == BEFORE, 'negative-control source copy is not exact')
    positive = cpu_target('control-positive', ['--test', 'sprite_clip_panel'], PANEL, root=control)
    path = control / 'crates/orr_editor/tests/sprite_clip_panel.rs'
    original = path.read_bytes()
    start = original.index(b'fn real_widgets_edit_order_duration_mode_and_undo_redo() {')
    end = original.index(b'\n#[test]', start)
    needle = b'    click(&mut h, "Apply frame draft");\n'
    require(original[start:end].count(needle) == 1, 'no-Apply mutation is no longer an exact singleton')
    at = original.index(needle, start, end)
    replacement = b'    // Mandatory negative control: omit this one Apply click.\n'
    changed = original[:at] + replacement + original[at + len(needle):]
    write_json(EVIDENCE / 'no-apply-mutation.json', {
        'source': str(path), 'offset': at, 'removed': needle.decode(), 'replacement': replacement.decode(),
        'intended_failure_location': 'crates/orr_editor/tests/sprite_clip_panel.rs:60',
        'original_sha256': hashlib.sha256(original).hexdigest(), 'negative_sha256': hashlib.sha256(changed).hexdigest()})
    try:
        path.write_bytes(changed)
        negative_source = source_inventory(control, NAMES)
        differences = [n for n in NAMES if negative_source[n] != BEFORE[n]]
        require(differences == ['crates/orr_editor/tests/sprite_clip_panel.rs'], 'negative modified more than its owned test file')
        negative = producer('control-negative', 'orr_editor', FEATURES, FEATURE_ARGS,
                            ['--test', 'sprite_clip_panel'], root=control, baseline=negative_source)
        require(negative['identity'] != positive['identity'], 'negative-control executable did not change')
        inventory(negative, PANEL)
        execute(negative, 'control-negative-singleton', ['real_widgets_edit_order_duration_mode_and_undo_redo'],
                test_filter='real_widgets_edit_order_duration_mode_and_undo_redo', exact=True, negative=True)
    finally:
        path.write_bytes(original)
        require(source_inventory(control, NAMES) == BEFORE, 'control cleanup failed to restore original bytes')
        write_json(EVIDENCE / 'no-apply-restored-source.json', {'root': str(control), 'source': BEFORE})
    restored = cpu_target('control-restored', ['--test', 'sprite_clip_panel'], PANEL, root=control)
    require(restored['identity'] == positive['identity'], 'restored panel full ELF differs from its exact-source positive producer')
    write_json(EVIDENCE / 'no-apply-complete.json', {
        'omitted_clicks': 1, 'negative_test': 'real_widgets_edit_order_duration_mode_and_undo_redo',
        'negative_exit': 101, 'positive_and_restored_tests': PANEL, 'full_elf_equal': True,
        'positive': positive['identity'], 'negative': negative['identity'], 'restored': restored['identity'],
        'original_checkout_untouched': source_inventory(ROOT, NAMES) == BEFORE})


def contracts():
    cpu_target('core15', ['--lib'], CORE, test_filter='sprite_clip_authoring::tests::')
    cpu_target('panel6', ['--test', 'sprite_clip_panel'], PANEL)
    cpu_target('workflow-cpu', ['--test', 'sprite_clip_workflow'], [ORDINARY, GPU], [GPU])
    cpu_target('boundary-enabled1', ['--test', 'sprite_clip_feature_boundary'], BOUNDARY, cap=32*MIB)
    cpu_target('boundary-default2', ['--test', 'sprite_clip_feature_boundary'], OFF_BOUNDARY,
               feature_args=[], features=['default'], cap=32*MIB)
    cpu_target('boundary-no-default2', ['--test', 'sprite_clip_feature_boundary'], OFF_BOUNDARY,
               feature_args=['--no-default-features'], features=[], cap=32*MIB)
    for target in ('sprite_clip_panel', 'sprite_clip_workflow'):
        off = producer('default-off-' + target, 'orr_editor', ['default'], [], ['--test', target], cap=32*MIB)
        inventory(off, [])
    app = producer('standalone-inventory', 'orr_editor', FEATURES, FEATURE_ARGS, ['--bin', 'orr_sprite_clip'])
    inventory(app, [APP], [APP])
    for index, guard in enumerate(GUARDS):
        cpu_target(f'tui-guard-{index}', ['--test', 'deps'], [guard], package='orr_tui',
                   feature_args=[], features=['default', 'ffi'], test_filter=guard, exact=True, cap=32*MIB)
    metadata_boundary('metadata-default', False)
    metadata_boundary('metadata-no-default', True)
    for name, action, package, features, args in [
        ('check-default', 'check', 'orr_editor', ['default'], []),
        ('check-no-default', 'check', 'orr_editor', [], ['--no-default-features']),
        ('strict-default', 'clippy', 'orr_editor', ['default'], []),
        ('strict-no-default', 'clippy', 'orr_editor', [], ['--no-default-features']),
        ('strict-sprite-feature', 'clippy', 'orr_editor', FEATURES, FEATURE_ARGS),
        ('strict-sample-project', 'clippy', 'orr_sample', PROJECT, ['--features', 'project']),
        ('strict-sample-export', 'clippy', 'orr_sample', EXPORT, ['--features', 'project-export'])]:
        rows = producer(name, package, features, args, ['--all-targets'], action=action, freeze=False)
        if name in {'check-default', 'check-no-default', 'strict-default', 'strict-no-default'}:
            require(not any(row['target']['name'] == 'orr_sprite_clip' for row in rows),
                    f'{name}: disabled standalone binary was activated')
    no_apply_control()


def png(path, expected_size):
    data = path.read_bytes()
    require(data[:8] == b'\x89PNG\r\n\x1a\n' and 32 < len(data) <= 32*MIB, f'invalid PNG: {path}')
    offset, compressed, chunks, header = 8, bytearray(), [], None
    while offset < len(data):
        require(offset + 12 <= len(data), 'truncated PNG chunk')
        length = int.from_bytes(data[offset:offset+4], 'big')
        kind = data[offset+4:offset+8]
        payload = data[offset+8:offset+8+length]
        require(offset + 12 + length <= len(data), 'truncated PNG payload')
        crc = int.from_bytes(data[offset+8+length:offset+12+length], 'big')
        require(zlib.crc32(kind + payload) & 0xffffffff == crc, 'PNG CRC mismatch')
        chunks.append(kind)
        if kind == b'IHDR':
            require(header is None and len(chunks) == 1 and length == 13, 'bad PNG IHDR')
            header = struct.unpack('>IIBBBBB', payload)
        elif kind == b'IDAT':
            compressed.extend(payload)
        elif kind == b'IEND':
            require(length == 0 and offset + 12 == len(data), 'PNG has trailing data')
        offset += length + 12
    require(header is not None and chunks[-1] == b'IEND' and chunks.count(b'IEND') == 1, 'incomplete PNG')
    width, height, depth, color, compression, filtering, interlace = header
    require((width, height) == expected_size and (depth, compression, filtering, interlace) == (8, 0, 0, 0)
            and color in (2, 6), f'PNG layout changed: {path}: {header}')
    channels = 4 if color == 6 else 3
    row_bytes = width * channels
    limit = height * (row_bytes + 1)
    decoder = zlib.decompressobj()
    raw = decoder.decompress(bytes(compressed), limit + 1)
    require(len(raw) == limit and decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail,
            'PNG stream has wrong decoded byte count')
    previous = bytearray(row_bytes)
    pixels = bytearray()
    for row in range(height):
        at = row * (row_bytes + 1)
        filtering = raw[at]
        current = bytearray(raw[at+1:at+1+row_bytes])
        require(filtering in range(5), 'invalid PNG row filter')
        if filtering:
            for index in range(row_bytes):
                left = current[index-channels] if index >= channels else 0
                up = previous[index]
                diagonal = previous[index-channels] if index >= channels else 0
                if filtering == 1:
                    predictor = left
                elif filtering == 2:
                    predictor = up
                elif filtering == 3:
                    predictor = (left + up) // 2
                else:
                    p = left + up - diagonal
                    distances = [abs(p-left), abs(p-up), abs(p-diagonal)]
                    predictor = [left, up, diagonal][distances.index(min(distances))]
                current[index] = (current[index] + predictor) & 255
        pixels.extend(current)
        previous = current
    first = pixels[:channels]
    require(any(pixels[i:i+channels] != first for i in range(0, len(pixels), channels)), f'blank PNG: {path}')
    require(any(pixels[i] > 40 for i in range(len(pixels)) if i % channels < 3), f'black PNG: {path}')
    return {'width': width, 'height': height, 'channels': channels,
            'decoded_sha256': hashlib.sha256(pixels).hexdigest(), 'sha256': digest(path), 'bytes': len(data)}


def capture_manifest(directory, expected):
    require({path.name for path in directory.iterdir()} == set(expected), f'exact capture inventory differs: {directory}')
    result = {}
    for name in sorted(expected):
        path = directory / name
        require(path.is_file() and not path.is_symlink(), f'non-regular evidence: {path}')
        result[name] = {'sha256': digest(path), 'bytes': path.stat().st_size}
    return result


def export_captures(directory, arena, exporter, bwrap):
    children = list(directory.iterdir())
    require(len(children) == 1 and children[0].is_dir() and not children[0].is_symlink()
            and children[0].name.startswith('sprite-export-evidence-'), 'exactly one retained export evidence directory required')
    folder = children[0]
    samples = [(0, 21), (8, 21), (9, 20), (15, 20), (16, 20), (60, 20)]
    labels = {f'{kind}-{index:02}-tick-{tick}': (kind, tick, region)
              for kind in ('source', 'export') for index, (tick, region) in enumerate(samples)}
    successful = [*labels, 'restored-source', 'restored-positive-export']
    negative = ['tampered-runtime', 'tampered-export']
    expected = {'pins.json', 'source-before.json', 'tampered-before.json', 'tampered-after.json',
                'export-manifest.json', 'export-package-lock.json', 'export-readonly-baseline.json', 'complete.json'}
    for label in successful + negative:
        expected.update(label + suffix for suffix in ('.command.json', '.stdout', '.stderr', '.status.json'))
    for label, (kind, _, _) in labels.items():
        expected.add(label + '.png')
        expected.add(label + '.inventory-after.json')
        expected.add(label + ('.inventory-before.json' if kind == 'export' else '.oracle.json'))
    expected.add('restored-source.png')
    manifest = capture_manifest(folder, expected)
    read = lambda name: json.loads((folder / name).read_text())
    complete = read('complete.json')
    require(complete == {
        'schema': 1, 'samples': [{'tick': tick, 'region': region} for tick, region in samples], 'tick_rate': 60,
        'runtime_sha256': arena['identity']['sha256'], 'exporter_sha256': exporter['identity']['sha256'],
        'export_content_digest': read('export-manifest.json')['content_digest'], 'source_independent': True,
        'export_mount_readonly': True, 'export_inventory_unchanged': True,
        'tamper_rejected_before_play_and_export': True, 'restored_positive': True,
        'capture_kind': 'actual Arena headless WGPU compositor; not a native-window capture'}, 'export completion receipt differs')
    baseline = read('export-readonly-baseline.json')
    require(baseline and all(entry['mode'] & 0o222 == 0 for entry in baseline.values()), 'export inventory has write permissions')
    require(read('tampered-before.json') == read('tampered-after.json'), 'tampered admission changed source inventory')
    images = {}
    for label in successful + negative:
        status = read(label + '.status.json')
        require(status['termination'] is None and status['code'] == (1 if label in negative else 0)
                and status['success'] is (label not in negative), f'{label}: child exit or timeout mismatch')
        command = read(label + '.command.json')
        require(command['program'] == str(bwrap) and command['environment_cleared'] is True
                and command['environment']['PATH'] == '/nonexistent'
                and command['environment']['ORR_REQUIRE_GPU'] == '1', f'{label}: child program/environment differs')
        require('--unshare-net' in command['arguments'], f'{label}: child networking is not isolated')
        text = (folder / (label + '.stdout')).read_text() + (folder / (label + '.stderr')).read_text()
        require(not SKIP.search(text), f'{label}: skip/fallback output')
        if label in negative:
            require('installed content changed: lantern_keeper.png' in text
                    and 'project initial checksum:' not in text and 'project capture adapter:' not in text,
                    f'{label}: tamper rejection occurred after play or for another reason')
        if (folder / (label + '.png')).exists():
            require(re.findall(r'^project capture adapter: (.+)$', (folder / (label + '.stdout')).read_text(), re.M)
                    and len(re.findall(r'^project capture adapter: (.+)$', (folder / (label + '.stdout')).read_text(), re.M)) == 1
                    and re.search(r'^project capture adapter: .+ \(software: true\)$',
                                  (folder / (label + '.stdout')).read_text(), re.M),
                    f'{label}: runtime must identify the pinned software CPU adapter')
            images[label] = png(folder / (label + '.png'), (512, 512))
    for label, (kind, tick, region) in labels.items():
        if kind == 'source':
            require(read(label + '.inventory-after.json') == read('source-before.json'), 'source runtime changed project inventory')
            oracle = read(label + '.oracle.json')
            require(oracle['sample'] == {'tick': tick, 'region': region} and oracle['tick_rate'] == 60
                    and oracle['elapsed_ms'] == tick * 1000 // 60
                    and oracle['png_sha256'] == images[label]['sha256'], 'source oracle/capture identity differs')
        else:
            require(read(label + '.inventory-before.json') == baseline == read(label + '.inventory-after.json'),
                    'source-hidden export changed bytes, modes, locks or paths')
            original_label = label.replace('export-', 'source-', 1)
            require(images[label]['decoded_sha256'] == images[original_label]['decoded_sha256'], 'export framebuffer differs from original')
            require('sprite export isolation: source absent; export write rejected' in (folder / (label + '.stderr')).read_text(),
                    'missing mandatory source-hidden/read-only probe')
    require(images['restored-source']['decoded_sha256'] == images['source-00-tick-0']['decoded_sha256'], 'restored framebuffer differs')
    require(images['source-00-tick-0']['decoded_sha256'] != images['source-02-tick-9']['decoded_sha256'], 'authored frame captures did not change')
    return {'directory': str(folder), 'files': manifest, 'images': images, 'complete': complete}


def acceptance():
    require(ENV.get('ORR_REQUIRE_GPU') == '1', 'acceptance requires explicit ORR_REQUIRE_GPU=1, never a skip path')
    require(ENV.get('WGPU_BACKEND') == 'vulkan', 'dedicated CI acceptance pins WGPU_BACKEND=vulkan')
    xdg = WORK / 'xdg-runtime'
    xdg.mkdir(mode=0o700)
    ENV['XDG_RUNTIME_DIR'] = str(xdg)
    # The official runner's packaged bubblewrap is used as-is. Namespace/security
    # restrictions fail this lane; no sysctl, AppArmor, setuid or sudo workaround.
    bwrap_found = shutil.which('bwrap')
    require(bwrap_found is not None, 'mandatory bubblewrap is missing')
    bwrap = Path(bwrap_found).resolve(strict=True)
    require(outside(bwrap, ROOT), 'bubblewrap must survive source hiding')
    run('bwrap-version', [bwrap, '--version'])
    run('bwrap-preflight', [bwrap, '--die-with-parent', '--unshare-net', '--ro-bind', '/', '/', '--', '/bin/true'])
    arena = producer('arena-project', 'orr_sample', PROJECT, ['--features', 'project'], ['--bin', 'arena'], action='build', cap=96*MIB)
    exporter = producer('arena-exporter', 'orr_sample', EXPORT, ['--features', 'project-export'],
                        ['--bin', 'orr_export_arena'], action='build', cap=48*MIB)
    runtime_work = WORK / 'runtime-work'
    runtime_work.mkdir()
    gpu_capture = EVIDENCE / 'composed-captures'
    gpu_capture.mkdir()
    export_capture = EVIDENCE / 'export-captures'
    export_capture.mkdir()
    gpu_env = {
        'ORR_REQUIRE_GPU': '1', 'ORR_SPRITE_ARENA_BIN': str(arena['frozen']),
        'ORR_SPRITE_ARENA_SHA256': arena['identity']['sha256'],
        'ORR_SPRITE_EXPORTER_BIN': str(exporter['frozen']),
        'ORR_SPRITE_EXPORTER_SHA256': exporter['identity']['sha256'],
        'ORR_SPRITE_BWRAP': str(bwrap), 'ORR_SPRITE_HIDE_ROOT': str(ROOT),
        'ORR_SPRITE_WORK_ROOT': str(runtime_work), 'ORR_SPRITE_CAPTURE_DIR': str(export_capture),
        'ORR_SPRITE_CLIP_CAPTURE_DIR': str(gpu_capture)}
    for key, value in gpu_env.items():
        if key.endswith(('_BIN', '_ROOT', '_DIR')) and key != 'ORR_SPRITE_HIDE_ROOT':
            require(outside(Path(value).resolve(), ROOT), f'{key} must survive source hiding')
    standalone = producer('standalone-gui', 'orr_editor', FEATURES, FEATURE_ARGS,
                          ['--bin', 'orr_sprite_clip'], action='build')
    elf(standalone['frozen'], standalone['cap'], standalone['identity'])
    stdout, stderr = run('standalone-invalid-cli', [standalone['frozen'], '--invalid'], expected=2, timeout=30)
    require(stdout == '' and stderr == 'sprite clip authoring: usage: orr_sprite_clip PROJECT PACKAGE DOCUMENT CLIP NEW_VERSION\n',
            'standalone invalid arguments did not fail at its exact pre-GUI CLI boundary')
    elf(standalone['frozen'], standalone['cap'], standalone['identity'])
    app = producer('standalone-app', 'orr_editor', FEATURES, FEATURE_ARGS, ['--bin', 'orr_sprite_clip'])
    inventory(app, [APP], [APP])
    log = execute(app, 'standalone-app-mandatory-gpu', [APP], test_filter=APP, exact=True, ignored_only=True, extra_env=gpu_env)
    require(re.search(r'actual standalone App adapter: .*device_type: Cpu\b', log),
            'standalone actual App must identify the pinned software CPU adapter')
    workflow = producer('workflow-gpu', 'orr_editor', FEATURES, FEATURE_ARGS, ['--test', 'sprite_clip_workflow'])
    inventory(workflow, [ORDINARY, GPU], [GPU])
    log = execute(workflow, 'workflow-mandatory-gpu-export', [GPU], test_filter=GPU, exact=True, ignored_only=True, extra_env=gpu_env)
    require(re.search(r'actual EditorApp adapter: .*device_type: Cpu\b', log),
            'EditorApp must identify the pinned software CPU adapter')
    # Pixel/region/control oracles are asserted by actual production consumers.
    # Independently decode retained PNG bytes and enforce the exact output set.
    names = ['standalone-01-controls', 'standalone-02-rejected-duration', 'standalone-03-published',
             'standalone-04-existing-destination-error', 'clip-01-frame21', 'clip-02-frame20',
             'clip-03-once-end', 'clip-04-stop-restored', 'clip-05-reopened-frame21']
    files = capture_manifest(gpu_capture, [name + '.png' for name in names])
    images = {name: png(gpu_capture / (name + '.png'), (1000, 900) if name.startswith('standalone') else (1500, 1400)) for name in names}
    require(images['standalone-01-controls']['decoded_sha256'] != images['standalone-02-rejected-duration']['decoded_sha256'],
            'standalone rejected-duration control pixels did not change')
    require(images['clip-01-frame21']['decoded_sha256'] != images['clip-02-frame20']['decoded_sha256'],
            'EditorApp composed frame captures did not change')
    write_json(EVIDENCE / 'composed-capture-manifest.json', {'files': files, 'decoded_images': images,
        'kind': 'actual offscreen eframe App and EditorApp FullOutput WGPU composition; no native OS-window claim'})
    write_json(EVIDENCE / 'export-capture-manifest.json', export_captures(export_capture, arena, exporter, bwrap))
    for pin in (arena, exporter):
        elf(pin['frozen'], pin['cap'], pin['identity'])


try:
    if MODE in {'contracts', 'all'}:
        contracts()
    if MODE in {'acceptance', 'all'}:
        acceptance()
    require(source_inventory(ROOT, NAMES) == BEFORE, 'tracked checkout changed during gate')
    for pin in PINS:
        elf(pin['frozen'], pin['cap'], pin['identity'])
    write_json(EVIDENCE / 'complete.json', {
        'mode': MODE, 'source_unchanged': True, 'frozen_elf_count': len(PINS),
        'source_manifest_sha256': digest(EVIDENCE / 'source-before.json'),
        'contracts_complete': MODE in {'contracts', 'all'},
        'mandatory_gpu_export_complete': MODE in {'acceptance', 'all'},
        'evidence': str(EVIDENCE), 'finished_ns': time.time_ns()})
    print(f'Sprite clip {MODE}: complete; exact evidence retained at {EVIDENCE}', flush=True)
except BaseException as error:
    write_json(EVIDENCE / 'failed.json', {'mode': MODE, 'error': str(error), 'type': type(error).__name__,
               'traceback': traceback.format_exc(), 'finished_ns': time.time_ns()})
    raise
finally:
    # Preserve logs, captures, source snapshots and frozen ELFs for inspection.
    # Isolated control sources (never the checkout) are already restored above.
    fcntl.flock(LOCK, fcntl.LOCK_UN)
    LOCK.close()
PY
