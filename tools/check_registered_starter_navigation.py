#!/usr/bin/env python3
"""Exact-candidate CPU gate and downloaded-executable replay; never opens a window.

Producer: EVIDENCE=/outside/fresh python3 tools/check_registered_starter_navigation.py --expected-head SHA
Consumer: EVIDENCE=/outside/fresh python3 tools/check_registered_starter_navigation.py --expected-head SHA --replay /download/acceptance
Inventories are reviewed source, never generated or accepted from runtime output.
"""
import argparse
import hashlib
import json
import os
import py_compile
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
INVENTORY = 'tools/registered_starter_navigation_inventory.json'
SCHEMA = 'orr.registered-starter-navigation/1'
PACKAGE = 'orr_static_starter'
STARTER = 'examples/static-game-starter'
# Each producer is compiled separately. Do not unify core, gameplay and standalone.
SUITES = {
    'core': {'features': '', 'target': ['--lib'], 'args': []},
    'gameplay': {'features': 'gameplay', 'target': ['--lib'],
                 'args': ['--skip', 'gameplay::ws_acceptance::']},
    'acceptance': {'features': 'gameplay', 'target': ['--test', 'acceptance'], 'args': []},
    'standalone-lib': {'features': 'standalone', 'target': ['--lib'], 'args': []},
    'standalone-contract': {'features': 'standalone', 'target': ['--test', 'standalone_contract'], 'args': []},
    'combined-contract': {'features': 'standalone,gameplay', 'target': ['--test', 'standalone_contract'], 'args': []},
}
GAMES = {'delivery': ('StarterDelivery', '0x5354415254563201'),
         'switchboard': ('StarterSwitchboard', '0x5354415254563202')}
BINARY_PATHS = {**{label: 'bin/' + label + '-tests' for label in SUITES},
                **{label: 'bin/' + label for label in ['player', 'delivery-player', 'switchboard-player']}}
ASSET_PATHS = {alias: f'scenes/{alias}.scene.yaml' for alias in GAMES}
SUMMARY = re.compile(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in .+$', re.M)
CASE = re.compile(r'^test (\S+) \.\.\. (ok|FAILED|ignored)(?: .*)?$', re.M)


def require(value, reason):
    if not value:
        raise ValueError(reason)


def sha(path):
    require(path.is_file() and not path.is_symlink(), f'Missing or linked file: {path}')
    digest = hashlib.sha256()
    with path.open('rb') as file:
        for block in iter(lambda: file.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def strict_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'Duplicate JSON key: ' + key)
            result[key] = value
        return result
    return json.loads(text, object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('Nonfinite JSON')))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n', encoding='utf-8')


def read_json(path):
    require(path.is_file() and not path.is_symlink(), f'Missing or linked JSON: {path}')
    require(path.stat().st_size <= 16 * 1024 * 1024, 'Oversize JSON')
    return strict_json(path.read_text(encoding='utf-8'))


def validate_names(expected):
    require(isinstance(expected, list) and expected and all(isinstance(n, str) and re.fullmatch(r'[a-zA-Z_][a-zA-Z_0-9:]*', n) for n in expected), 'Empty or invalid test inventory')
    require(len(expected) == len(set(expected)), 'Duplicate expected test inventory')


def validate_listing(output, expected):
    validate_names(expected)
    output = output.replace('\r\n', '\n')
    names = re.findall(r'^(\S+): test$', output, re.M)
    summaries = re.findall(r'^(\d+) tests?, (\d+) benchmarks?$', output, re.M)
    require(summaries == [(str(len(expected)), '0')] and len(names) == len(set(names))
            and set(names) == set(expected), 'Missing, extra, duplicate or changed listed test inventory')
    require(not re.search(r'^\S+: benchmark$', output, re.M), 'Unexpected benchmark inventory')


def validate_tests(output, expected):
    validate_names(expected)
    output = output.replace('\r\n', '\n')
    summaries = SUMMARY.findall(output)
    cases = CASE.findall(output)
    names = [name for name, _ in cases]
    require(len(summaries) == 1 and len(re.findall(r'^test result:.*$', output, re.M)) == 1
            and len(re.findall(r'^test \S+ \.\.\. ', output, re.M)) == len(cases)
            and tuple(map(int, summaries[0][:4])) == (len(expected), 0, 0, 0)
            and re.findall(r'^running (\d+) tests?$', output, re.M) == [str(len(expected))]
            and len(names) == len(set(names)) and set(names) == set(expected)
            and all(status == 'ok' for _, status in cases),
            'Incomplete, zero, skipped, duplicate or replaced passing test inventory')


def inventory():
    result = read_json(ROOT / INVENTORY)
    require(result.get('schema') == SCHEMA and result.get('reviewed') is True, 'Inventory is not reviewed')
    require(set(result['suites']) == set(SUITES), 'Changed suite inventory')
    for expected in result['suites'].values():
        validate_names(expected)
    require(not any('ws_acceptance::' in name or 'gpu_api_acceptance::' in name
                    for names in result['suites'].values() for name in names), 'Device/socket test in CPU inventory')
    return result['suites']


def run(command, directory, label, env, timeout=2700, expected_exit=0):
    """Bound every command and preserve argv, output, exit, and timeout failures."""
    require(re.fullmatch(r'[a-z0-9-]+', label), 'Invalid log label')
    log_path = directory / (label + '.log')
    record_path = directory / (label + '.command.json')
    require(not log_path.exists() and not record_path.exists(), 'Refusing to overwrite command evidence')
    command = list(map(str, command))
    record = {'argv': command, 'cwd': str(ROOT), 'timeout_seconds': timeout,
              'expected_exit': expected_exit, 'exit_code': None, 'status': 'running'}
    write_json(record_path, record)
    print('+ ' + ' '.join(command), flush=True)
    start = time.monotonic()
    try:
        with log_path.open('w', encoding='utf-8') as log:
            log.write('+ ' + ' '.join(command) + '\n')
            log.flush()
            process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                record['exit_code'] = process.wait()
                record['status'] = 'timeout'
                log.write('\nTIMED OUT; process group terminated\n')
                raise
            record['exit_code'] = code
            record['status'] = 'passed' if code == expected_exit else 'failed'
            log.write(f'\nEXIT_CODE={code}\n')
        require(code == expected_exit, f'{label}: exit {code}; see {log_path}')
        require(log_path.stat().st_size <= 128 * 1024 * 1024, f'{label}: excessive output')
        return log_path.read_text(encoding='utf-8', errors='replace')
    except Exception as error:
        record['error'] = str(error)[:4096]
        if record['status'] == 'running':
            record['status'] = 'failed'
        raise
    finally:
        record['elapsed_seconds'] = round(time.monotonic() - start, 3)
        write_json(record_path, record)


def source_fence(expected_head, directory, label, root=None):
    root = Path(root or ROOT).resolve()
    status = subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=all'], cwd=root, text=True)
    (directory / (label + '-status.txt')).write_text(status, encoding='utf-8')
    require(not status, 'Source must be clean, including index and untracked files')
    head, tree = subprocess.check_output(['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'], cwd=root, text=True).splitlines()
    if expected_head is not None:
        require(head == expected_head, 'Checked-out HEAD differs from exact expected candidate')
    # Independently compare reviewed Git blobs, not merely Git's stat cache.
    # This follows static_game_native_gate.clean_checkout and rejects changes
    # hidden by update-index --assume-unchanged / --skip-worktree as well.
    entries = subprocess.check_output(['git', 'ls-tree', '-rz', '--full-tree', 'HEAD'], cwd=root)
    files = {}
    for entry in entries.split(b'\0'):
        if not entry:
            continue
        header, name = entry.split(b'\t', 1)
        mode, kind, object_id = header.decode().split()
        name = name.decode()
        require(kind == 'blob' and mode in ('100644', '100755'), 'Unsupported source link/submodule: ' + name)
        path = root / name
        require(path.is_file() and not path.is_symlink(), 'Missing/linked tracked source: ' + name)
        for parent in path.parents:
            if parent == root:
                break
            require(not parent.is_symlink(), 'Linked source directory: ' + str(parent))
        require(bool(path.stat().st_mode & 0o111) == (mode == '100755'), 'Tracked source mode differs: ' + name)
        data = path.read_bytes()
        blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        require(blob == object_id, 'Tracked source bytes differ from reviewed tree: ' + name)
        files[name] = hashlib.sha256(data).hexdigest()
    require(files, 'Empty tracked source inventory')
    # --others WITHOUT --exclude-standard includes ignored inputs as well.
    # Generated target outputs may remain; new source/configuration may not.
    unknown = subprocess.check_output(['git', 'ls-files', '-z', '--others'], cwd=root)
    reserved_roots = ('crates/', 'examples/', 'tools/', 'assets/', 'scenes/', 'tests/', '.cargo/')
    reserved_names = ('crates', 'examples', 'tools', 'assets', 'scenes', 'tests', '.cargo',
                      'Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain', 'rust-toolchain.toml')
    for raw in unknown.split(b'\0'):
        if not raw:
            continue
        name = raw.decode()
        # Python import caches are generated output, not additional gate source.
        cache = name.startswith('tools/') and '__pycache__/' in name
        require(cache or not (name.startswith(reserved_roots) or name in reserved_names),
                'Untracked compiler/gate input: ' + name)
    return {'head': head, 'tree': tree, 'files': dict(sorted(files.items()))}


def reject_ancestor_config(root):
    for ancestor in Path(root).parents:
        for name in ('.cargo/config', '.cargo/config.toml', 'rust-toolchain', 'rust-toolchain.toml'):
            path = ancestor / name
            # Rustup ignores ordinary directories at these selection-file names.
            # Verified with `rustup show active-toolchain`; symlinks and every
            # existing non-directory remain forbidden, including special files.
            if name in ('rust-toolchain', 'rust-toolchain.toml') and path.is_dir() and not path.is_symlink():
                continue
            require(not path.exists() and not path.is_symlink(), 'Ancestor compiler configuration: ' + str(path))


def validate_replay_source(producer, consumer, expected_head):
    # API publication can preserve the reviewed tree under a new commit parent.
    # Only replay permits a different local HEAD; bytes and tree must be exact.
    require(producer.get('head') == expected_head, 'Downloaded producer HEAD differs from supplied remote HEAD')
    require(producer.get('tree') == consumer.get('tree'), 'Downloaded source tree differs from clean local tree')
    require(producer.get('files') == consumer.get('files') and producer.get('files'), 'Downloaded tracked source hashes differ')


def cargo(verb, features, targets):
    return ['cargo', verb, '--release', '--locked', '-p', PACKAGE, '--no-default-features'] + (
        ['--features', features] if features else []) + targets


def artifact(output, name, kind, source, test):
    matches = []
    for line in output.splitlines():
        if not line.startswith('{'):
            continue
        value = strict_json(line)
        target = value.get('target', {})
        if (value.get('reason') == 'compiler-artifact' and target.get('name') == name
                and target.get('kind') == [kind] and value.get('profile', {}).get('test') is test
                and value.get('executable')):
            require(Path(target['src_path']).resolve() == ROOT / source, 'Wrong compiled target source')
            require(value.get('manifest_path') == str(ROOT / STARTER / 'Cargo.toml'), 'Wrong compiled package manifest')
            matches.append(Path(value['executable']))
    require(len(matches) == 1, f'Expected exactly one {name} executable producer')
    return matches[0]


def copy_file(source, destination, executable=False):
    expected = sha(source)
    require(not destination.exists(), 'Refusing to overwrite artifact')
    shutil.copyfile(source, destination)
    if executable:
        destination.chmod(0o755)
    require(sha(destination) == expected, 'Copied artifact hash mismatch')
    return expected


def build_binaries(directory, env):
    binaries = {}
    for label, features in [('player', ''), ('delivery-player', 'standalone'), ('switchboard-player', 'standalone')]:
        output = run(cargo('build', features, ['--bin', label, '--message-format=json']), directory, label + '-build', env)
        producer = artifact(output, label, 'bin', f'{STARTER}/src/bin/{label}.rs', False)
        destination = directory / 'bin' / label
        binaries[label] = {'path': 'bin/' + label, 'sha256': copy_file(producer, destination, True)}
    for label, suite in SUITES.items():
        output = run(cargo('test', suite['features'], suite['target']) + ['--no-run', '--message-format=json'], directory, label + '-build', env)
        lib = suite['target'] == ['--lib']
        name = PACKAGE if lib else suite['target'][1]
        source = f'{STARTER}/src/lib.rs' if lib else f'{STARTER}/tests/{name}.rs'
        producer = artifact(output, name, 'lib' if lib else 'test', source, True)
        destination = directory / 'bin' / (label + '-tests')
        binaries[label] = {'path': 'bin/' + label + '-tests', 'sha256': copy_file(producer, destination, True)}
    return binaries


def verify_bindings(directory, bindings):
    require(isinstance(bindings, dict) and bindings, 'Empty artifact bindings')
    for name, entry in bindings.items():
        relative = Path(entry['path'])
        require(not relative.is_absolute() and '..' not in relative.parts and relative.as_posix() == entry['path'], 'Escaping artifact path')
        target = directory / relative
        require(target.resolve().is_relative_to(directory.resolve()), 'Artifact path escapes root')
        require(sha(target) == entry['sha256'], 'Changed executable/asset binding: ' + name)


def validate_role_paths(binaries, assets):
    for bindings, paths in [(binaries, BINARY_PATHS), (assets, ASSET_PATHS)]:
        require(set(bindings) == set(paths), 'Missing/extra executable or scene roles')
        require(all(bindings[name].get('path') == path for name, path in paths.items()), 'Changed executable or scene role path')


def json_output(output):
    lines = [line for line in output.splitlines() if line.startswith('{')]
    require(len(lines) == 1, 'Expected one JSON output')
    return strict_json(lines[0])


def execute_binaries(directory, binaries, assets, expected, head, env):
    validate_role_paths(binaries, assets)
    verify_bindings(directory, binaries)
    verify_bindings(directory, assets)
    env = env.copy()
    env['ORR_STATIC_STARTER_PLAYER_TEST_BIN'] = str(directory / binaries['player']['path'])
    for label, suite in SUITES.items():
        executable = directory / binaries[label]['path']
        listing = run([executable, *suite['args'], '--list', '--format=pretty'], directory, label + '-inventory', env, 60)
        validate_listing(listing, expected[label])
        output = run([executable, *suite['args'], '--test-threads=1', '--format=pretty', '--color=never', '--show-output'], directory, label + '-tests', env, 600)
        validate_tests(output, expected[label])
        verify_bindings(directory, binaries)
    identities = {}
    for alias, (game, build) in GAMES.items():
        binary = directory / binaries[alias + '-player']['path']
        scene = directory / assets[alias]['path']
        provenance = json_output(run([binary, '--provenance'], directory, alias + '-provenance', env, 30))
        require(provenance == {'source_sha': head}, 'Native player provenance differs from exact HEAD')
        value = json_output(run([binary, '--identity', '--scene', scene], directory, alias + '-identity', env, 30))
        require(value.get('game') == game and value.get('alias') == alias and value.get('build_id') == build,
                'Native player game/build identity mismatch')
        require(value.get('seed') == 42 and value.get('tick_rate') == 60 and value.get('player_count') == 1,
                'Native player session identity mismatch')
        require(re.fullmatch(r'0x[0-9a-f]{16}', value.get('initial_checksum', '')) and isinstance(value.get('schema'), dict)
                and isinstance(value.get('input_contract'), dict) and value['input_contract'].get('max_players') == 1,
                'Incomplete native schema/input identity')
        identities[alias] = value
    verify_bindings(directory, binaries)
    verify_bindings(directory, assets)
    return identities


def seal(directory):
    files = sorted(path for path in directory.rglob('*') if path.is_file() and path.name != 'artifact-manifest.json')
    write_json(directory / 'artifact-manifest.json', {path.relative_to(directory).as_posix(): sha(path) for path in files})


def verify_seal(directory):
    require(not any(path.is_symlink() for path in directory.rglob('*')), 'Linked artifact entry')
    manifest = read_json(directory / 'artifact-manifest.json')
    require(isinstance(manifest, dict) and manifest, 'Missing artifact manifest')
    actual = {path.relative_to(directory).as_posix() for path in directory.rglob('*') if path.is_file() and path.name != 'artifact-manifest.json'}
    require(actual == set(manifest), 'Missing or extra artifact files')
    for name, digest in manifest.items():
        path = Path(name)
        require(not path.is_absolute() and '..' not in path.parts, 'Invalid artifact manifest path')
        require(sha(directory / path) == digest, 'Modified artifact: ' + name)


def execution_environment(directory, head):
    env = os.environ.copy()
    cache = directory / 'python-cache'
    cache.mkdir()  # Fresh and empty: -B alone still permits reading stale pyc.
    env.update(ORR_STATIC_STANDALONE_BUILD_SHA=head, PYTHONDONTWRITEBYTECODE='1',
               PYTHONPYCACHEPREFIX=str(cache))
    # No inherited module lookup path or alternate compiler wrapper participates.
    for key in ['DISPLAY', 'WAYLAND_DISPLAY', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'PYTHONPATH']:
        env.pop(key, None)
    return env


def checks(directory, env):
    rust = run(['rustc', '--version', '--verbose'], directory, 'rustc', env, 60)
    require(re.search(r'^release: 1\.99\.1$', rust, re.M), 'Expected supported Rust 1.99.1')
    run(['cargo', '--version'], directory, 'cargo', env, 60)
    run([sys.executable, '-B', str(ROOT / 'tools/check_registered_starter_navigation.py'), '--self-test'], directory, 'gate-self-tests', env, 60)
    for test in ['test_static_game_native_gate.py', 'test_static_game_standalone_gate.py']:
        run([sys.executable, '-B', '-m', 'unittest', 'discover', '-s', 'tools', '-p', test, '-v'], directory, test[:-3].replace('_', '-'), env, 120)
    for label, features in [('core', ''), ('gameplay', 'gameplay'), ('standalone', 'standalone'), ('combined', 'standalone,gameplay')]:
        run(cargo('clippy', features, ['--lib', '--tests', '--bins']) + ['--', '-D', 'warnings'], directory, label + '-strict-clippy', env)
    run(cargo('check', '', ['--lib']), directory, 'feature-off-check', env)
    run([sys.executable, '-B', 'tools/check_sim_float_types.py'], directory, 'simulation-float-guard', env)
    run([sys.executable, '-B', 'tools/test_sim_float_guard.py'], directory, 'simulation-float-fixtures', env)


def main(args):
    require(Path.cwd().resolve() == ROOT, 'Run from the exact candidate repository root')
    require(re.fullmatch(r'[0-9a-f]{40}', args.expected_head or ''), 'An explicit full expected HEAD is required')
    if not args.replay and os.environ.get('GITHUB_SHA'):
        require(os.environ['GITHUB_SHA'] == args.expected_head, 'GITHUB_SHA differs from expected HEAD')
    evidence_root = Path(os.environ['EVIDENCE']).resolve()
    require(evidence_root != ROOT and not evidence_root.is_relative_to(ROOT), 'Evidence must be outside source checkout')
    evidence_root.mkdir(parents=True, exist_ok=True)
    directory = evidence_root / ('replay' if args.replay else 'acceptance')
    directory.mkdir()  # Fresh only. Failed or interrupted evidence is never replaced.
    (directory / 'bin').mkdir()
    (directory / 'scenes').mkdir()
    scope = {'schema': SCHEMA, 'status': 'running', 'head': args.expected_head,
             'mode': 'downloaded-artifact-replay' if args.replay else 'producer',
             'cpu_in_process': True, 'native_identity_queries_only': True,
             'socket': False, 'window': False, 'browser': False, 'gpu': False,
             'audio_device': False, 'physical_input': False, 'full_workspace': False,
             'compile_checks_replayed': False, 'compile_checks_performed': not bool(args.replay)}
    source = None
    try:
        expected = inventory()
        source = source_fence(None if args.replay else args.expected_head, directory, 'before')
        scope['consumer_head' if args.replay else 'producer_head'] = source['head']
        write_json(directory / 'source.json', source)
        copy_file(ROOT / INVENTORY, directory / 'inventory.json')
        env = execution_environment(directory, args.expected_head)
        if args.replay:
            origin = args.replay.resolve()
            require(origin != directory and not directory.is_relative_to(origin), 'Replay evidence must be separate from downloaded evidence')
            verify_seal(origin)
            original = read_json(origin / 'scope.json')
            require(original.get('schema') == SCHEMA and original.get('status') == 'passed'
                    and original.get('mode') == 'producer', 'Replay requires completed producer evidence')
            validate_replay_source(read_json(origin / 'source.json'), source, args.expected_head)
            require(original.get('source_sha256') == sha(origin / 'source.json') and original.get('inventory_sha256') == sha(origin / 'inventory.json'), 'Inconsistent source/inventory digest binding')
            require(read_json(origin / 'inventory.json') == read_json(ROOT / INVENTORY), 'Downloaded test inventory differs from source')
            require(original.get('head') == args.expected_head and original.get('tree') == source['tree'], 'Downloaded candidate identity mismatch')
            require(original.get('suite_configurations') == SUITES and original.get('counts') == {k: len(v) for k, v in expected.items()}, 'Downloaded suite contract changed')
            binaries, assets = original['binaries'], original['assets']
            validate_role_paths(binaries, assets)
            verify_bindings(origin, binaries)
            verify_bindings(origin, assets)
            for entry in binaries.values():
                copy_file(origin / entry['path'], directory / entry['path'], True)
            for entry in assets.values():
                copy_file(origin / entry['path'], directory / entry['path'])
            scope['producer_scope_sha256'] = sha(origin / 'scope.json')
            scope['producer_manifest_sha256'] = sha(origin / 'artifact-manifest.json')
            run([sys.executable, '-B', str(ROOT / 'tools/check_registered_starter_navigation.py'), '--self-test'], directory, 'replay-gate-self-tests', env, 60)
        else:
            reject_ancestor_config(ROOT)
            checks(directory, env)
            binaries = build_binaries(directory, env)
            assets = {}
            for alias in GAMES:
                relative = f'scenes/{alias}.scene.yaml'
                assets[alias] = {'path': relative, 'sha256': copy_file(ROOT / STARTER / relative, directory / relative)}
        identities = execute_binaries(directory, binaries, assets, expected, args.expected_head, env)
        if args.replay:
            require(identities == original['identities'], 'Replayed native identity differs from producer')
            verify_seal(origin)
        if not args.replay:
            reject_ancestor_config(ROOT)
        require(source_fence(None if args.replay else args.expected_head, directory, 'after') == source, 'Source HEAD/tree/bytes changed during validation')
        scope.update(status='passed', tree=source['tree'], counts={k: len(v) for k, v in expected.items()},
                     suite_configurations=SUITES, binaries=binaries, assets=assets, identities=identities,
                     source_sha256=sha(directory / 'source.json'), inventory_sha256=sha(directory / 'inventory.json'))
    except Exception as error:
        scope.update(status='failed', error=str(error)[:4096])
        raise
    finally:
        if source is not None and 'after-status.txt' not in {p.name for p in directory.iterdir()}:
            try:
                if not args.replay:
                    reject_ancestor_config(ROOT)
                unchanged = source_fence(None if args.replay else args.expected_head, directory, 'failure-after') == source
                scope['source_unchanged_after_failure'] = unchanged
            except Exception as error:
                scope['source_after_failure_error'] = str(error)[:4096]
        write_json(directory / 'scope.json', scope)
        seal(directory)


class ValidatorTests(unittest.TestCase):
    def output(self):
        return ('running 2 tests\ntest one ... ok\ntest two ... ok\n'
                'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s\n')

    def test_accept_exact_and_crlf(self):
        for output in [self.output(), self.output().replace('\n', '\r\n')]:
            validate_tests(output, ['one', 'two'])
        validate_listing('one: test\ntwo: test\n\n2 tests, 0 benchmarks\n', ['one', 'two'])

    def test_reject_zero_missing_extra_skipped_duplicate_replaced_and_multiple_runs(self):
        good = self.output()
        for bad in ['', good.replace('test two ... ok\n', ''), good.replace('test two', 'test extra'),
                    good.replace('test two', 'test one'), good.replace('... ok', '... ignored', 1),
                    good.replace('0 ignored', '1 ignored'), good.replace('2 passed', '0 passed'),
                    good.replace('running 2', 'running 0'), good + good,
                    good + 'test result: FAILED. 0 passed; 1 failed\n',
                    good + 'test extra ... unknown\n',
                    good.replace('test two', 'test extra ... ok\ntest two')]:
            with self.subTest(output=bad), self.assertRaises(ValueError):
                validate_tests(bad, ['one', 'two'])
        for names in [[], ['one', 'one']]:
            with self.assertRaises(ValueError):
                validate_tests(good, names)

    def test_listing_rejects_missing_extra_duplicate_and_benchmark(self):
        good = 'one: test\ntwo: test\n\n2 tests, 0 benchmarks\n'
        for bad in ['', good.replace('two: test\n', ''), good.replace('two', 'three'),
                    good.replace('two', 'one'), good.replace('0 benchmarks', '1 benchmark'),
                    good.replace('two: test', 'two: benchmark'), good + good]:
            with self.subTest(output=bad), self.assertRaises(ValueError):
                validate_listing(bad, ['one', 'two'])

    def test_changed_executable_and_source_bindings_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            executable, source = root / 'test-binary', root / 'source.rs'
            executable.write_bytes(b'executable')
            source.write_bytes(b'source')
            bindings = {name: {'path': path.name, 'sha256': sha(path)} for name, path in [('executable', executable), ('source', source)]}
            verify_bindings(root, bindings)
            for path in [executable, source]:
                original = path.read_bytes()
                path.write_bytes(original + b'changed')
                with self.assertRaises(ValueError):
                    verify_bindings(root, bindings)
                path.write_bytes(original)
            bindings['source']['path'] = '../source.rs'
            with self.assertRaises(ValueError):
                verify_bindings(root, bindings)

    def test_role_bindings_cannot_replace_or_omit_a_producer(self):
        binaries = {name: {'path': path} for name, path in BINARY_PATHS.items()}
        assets = {name: {'path': path} for name, path in ASSET_PATHS.items()}
        validate_role_paths(binaries, assets)
        for bad in [dict(binaries, core={'path': binaries['gameplay']['path']}),
                    {name: value for name, value in binaries.items() if name != 'core'},
                    dict(binaries, other={'path': 'bin/other'})]:
            with self.assertRaises(ValueError):
                validate_role_paths(bad, assets)

    def test_seal_rejects_missing_extra_changed_and_linked_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            item = root / 'receipt.json'
            item.write_text('{}')
            seal(root)
            verify_seal(root)
            item.write_text('{"changed":true}')
            with self.assertRaises(ValueError):
                verify_seal(root)
            item.write_text('{}')
            extra = root / 'extra'
            extra.write_text('extra')
            with self.assertRaises(ValueError):
                verify_seal(root)
            extra.unlink()
            item.unlink()
            with self.assertRaises(ValueError):
                verify_seal(root)
            target = root / 'target'
            target.write_text('{}')
            item.symlink_to(target)
            with self.assertRaises(ValueError):
                verify_bindings(root, {'receipt': {'path': 'receipt.json', 'sha256': hashlib.sha256(b'{}').hexdigest()}})

    def test_python_children_ignore_timestamp_valid_unreviewed_cache(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            module_dir, evidence = root / 'modules', root / 'evidence'
            module_dir.mkdir()
            evidence.mkdir()
            module = module_dir / 'fenced_helper.py'
            module.write_text("value = 'stale'\n")
            timestamp = module.stat().st_mtime_ns
            cache_file = module_dir / '__pycache__' / f'fenced_helper.{sys.implementation.cache_tag}.pyc'
            py_compile.compile(str(module), cfile=str(cache_file), doraise=True,
                               invalidation_mode=py_compile.PycInvalidationMode.TIMESTAMP)
            module.write_text("value = 'fresh'\n")
            os.utime(module, ns=(timestamp, timestamp))
            command = [sys.executable, '-B', '-c',
                       'import sys; sys.path.insert(0, sys.argv[1]); import fenced_helper; print(fenced_helper.value)',
                       str(module_dir)]
            unfenced = os.environ.copy()
            unfenced.pop('PYTHONPYCACHEPREFIX', None)
            unfenced.pop('PYTHONPATH', None)
            self.assertEqual(subprocess.check_output(command, cwd=root, env=unfenced, text=True, timeout=30).strip(), 'stale')
            env = execution_environment(evidence, 'a' * 40)
            self.assertEqual(env['PYTHONPYCACHEPREFIX'], str(evidence / 'python-cache'))
            self.assertNotIn('PYTHONPATH', env)
            self.assertEqual(list((evidence / 'python-cache').iterdir()), [])
            output = subprocess.check_output(command, cwd=root, env=env, text=True, timeout=30)
            self.assertEqual(output.strip(), 'fresh')
            self.assertEqual(list((evidence / 'python-cache').iterdir()), [])

    def test_source_fence_rejects_ignored_compiler_inputs_and_hidden_mutations(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'source'
            evidence = Path(temp) / 'evidence'
            root.mkdir()
            evidence.mkdir()
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            source = root / 'source.rs'
            source.write_text('reviewed source')
            (root / '.gitignore').write_text('/target/\n')
            subprocess.run(['git', '-C', str(root), 'add', '.'], check=True)
            subprocess.run(['git', '-C', str(root), '-c', 'user.name=fixture', '-c',
                            'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
            head = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
            baseline = source_fence(head, evidence, 'clean', root)
            (root / 'target').mkdir()
            (root / 'target/generated').write_text('ordinary build output')
            self.assertEqual(source_fence(head, evidence, 'target-output', root), baseline)
            paths = ['rust-toolchain', 'rust-toolchain.toml', '.cargo/config', '.cargo/config.toml',
                     'examples/game/.cargo/config.toml', 'examples/game/Cargo.toml',
                     'crates/game/hidden.rs', 'tools/hidden.py', 'assets/hidden.scene.yaml']
            for index, name in enumerate(paths):
                (root / '.git/info/exclude').write_text('/' + name + '\n')
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('unreviewed input')
                with self.subTest(path=name), self.assertRaisesRegex(ValueError, 'Untracked compiler/gate input'):
                    source_fence(head, evidence, 'ignored-' + str(index), root)
                path.unlink()
            (root / '.git/info/exclude').write_text('')
            for flag, clear in [('--assume-unchanged', '--no-assume-unchanged'),
                                ('--skip-worktree', '--no-skip-worktree')]:
                subprocess.run(['git', '-C', str(root), 'update-index', flag, 'source.rs'], check=True)
                source.write_text('hidden mutation')
                with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, 'Tracked source bytes differ'):
                    source_fence(head, evidence, 'hidden-' + flag[2:], root)
                source.write_text('reviewed source')
                subprocess.run(['git', '-C', str(root), 'update-index', clear, 'source.rs'], check=True)
            self.assertEqual(source_fence(head, evidence, 'restored', root), baseline)
            with self.assertRaisesRegex(ValueError, 'expected candidate'):
                source_fence('f' * 40, evidence, 'wrong-head', root)

    def test_producer_rejects_ancestor_compiler_configuration(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'project'
            root.mkdir()
            reject_ancestor_config(root)
            for name in ['rust-toolchain', 'rust-toolchain.toml']:
                path = Path(temp) / name
                path.mkdir()
                reject_ancestor_config(root)
                path.rmdir()
                path.symlink_to(root, target_is_directory=True)
                with self.subTest(directory_symlink=name), self.assertRaisesRegex(ValueError, 'Ancestor compiler configuration'):
                    reject_ancestor_config(root)
                path.unlink()
                path.symlink_to(Path(temp) / 'missing')
                with self.subTest(dangling_symlink=name), self.assertRaisesRegex(ValueError, 'Ancestor compiler configuration'):
                    reject_ancestor_config(root)
                path.unlink()
            for name in ['.cargo/config', '.cargo/config.toml', 'rust-toolchain', 'rust-toolchain.toml']:
                path = Path(temp) / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('unreviewed ancestor configuration')
                with self.subTest(path=name), self.assertRaisesRegex(ValueError, 'Ancestor compiler configuration'):
                    reject_ancestor_config(root)
                path.unlink()

    def test_replay_source_allows_only_identical_tree_with_expected_remote_head(self):
        producer = {'head': 'a' * 40, 'tree': 'b' * 40, 'files': {'source.rs': 'c' * 64}}
        consumer = dict(producer, head='d' * 40)
        validate_replay_source(producer, consumer, 'a' * 40)
        for changed, expected_head in [
                (dict(producer, head='e' * 40), 'a' * 40),
                (dict(producer, tree='e' * 40), 'a' * 40),
                (dict(producer, files={'source.rs': 'e' * 64}), 'a' * 40),
                (dict(producer, files={}), 'a' * 40),
                (producer, 'e' * 40)]:
            with self.subTest(producer=changed, expected_head=expected_head), self.assertRaises(ValueError):
                validate_replay_source(changed, consumer, expected_head)

    def test_duplicate_json_keys_rejected(self):
        with self.assertRaises(ValueError):
            strict_json('{"head":1,"head":2}')


if __name__ == '__main__':
    if '--self-test' in sys.argv:
        unittest.main(argv=[sys.argv[0]])
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument('--expected-head', default=os.environ.get('GITHUB_SHA'))
        parser.add_argument('--replay', type=Path)
        try:
            main(parser.parse_args())
        except Exception as error:
            print('Registered starter navigation gate FAILED: ' + str(error), file=sys.stderr)
            sys.exit(1)
