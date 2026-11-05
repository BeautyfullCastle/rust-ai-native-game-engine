#!/usr/bin/env python3
"""Exact-source Stormglass CPU gate and downloaded test-executable replay.

EVIDENCE=/outside/fresh python3 -B tools/check_stormglass_live_observation.py --expected-head SHA
Add --replay /authenticated/download/acceptance to execute the downloaded binaries.
Artifact retrieval authenticates the workflow/run/head separately. This gate binds
its source tree, reviewed inventory and executable bytes; it invents no provenance.
"""
import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import types
import unittest
from unittest.mock import patch


def source_module(path):
    """Read helper source directly; -B alone still permits stale .pyc reads."""
    module = types.ModuleType('stormglass_cpu_shared')
    module.__file__ = str(path)
    exec(compile(path.read_bytes(), str(path), 'exec'), module.__dict__)
    return module


ROOT = Path(__file__).resolve().parent.parent
g = source_module(ROOT / 'tools/check_registered_starter_navigation.py')
SCHEMA = 'orr.stormglass-live-observation/1'
INVENTORY = 'tools/stormglass_live_observation_inventory.json'
WS = 'gameplay::ws_acceptance::'
GAMEPLAY_SOCKET_TESTS = [
    WS + 'gameplay_cli_two_registered_games_consume_and_release',
    WS + 'two_registered_games_accept_authenticated_websocket_semantic_input',
]
STORMGLASS_SOCKET_TESTS = GAMEPLAY_SOCKET_TESTS + [
    WS + 'gameplay_cli_stormglass_consumes_summon_and_preserves_source',
    WS + 'stormglass_authored_project_accepts_authenticated_websocket_summon_and_preserves_source',
]
REMOTE = 'crates/orr_remote'
STARTER = 'examples/static-game-starter'
SUITES = {
    'remote-coop': {'package': 'orr_remote', 'directory': REMOTE,
                    'features': 'coop-defense', 'compiled_features': ['coop-defense'],
                    'list_args': ['coop_defense::'], 'excluded': []},
    'starter-core': {'package': 'orr_static_starter', 'directory': STARTER,
                     'features': '', 'compiled_features': [], 'list_args': [], 'excluded': []},
    'starter-gameplay': {'package': 'orr_static_starter', 'directory': STARTER,
                         'features': 'gameplay', 'compiled_features': ['editor', 'gameplay'],
                         'list_args': [], 'excluded': GAMEPLAY_SOCKET_TESTS},
    'starter-stormglass': {'package': 'orr_static_starter', 'directory': STARTER,
                           'features': 'stormglass', 'compiled_features': ['editor', 'gameplay', 'stormglass'],
                           'list_args': [], 'excluded': STORMGLASS_SOCKET_TESTS},
}
BINARY_PATHS = {label: 'bin/' + label + '-tests' for label in SUITES}
SCOPE = {'cpu_in_process': True, 'socket': False, 'window': False, 'browser': False,
         'gpu': False, 'audio_device': False, 'physical_input': False,
         'model_execution': False, 'full_workspace': False}
CLIPPY_CONFIGS = [('remote-core', 'orr_remote', ''),
                 ('remote-coop', 'orr_remote', 'coop-defense'),
                 ('remote-sample-host', 'orr_remote', 'sample-host'),
                 ('starter-core', 'orr_static_starter', ''),
                 ('starter-gameplay', 'orr_static_starter', 'gameplay'),
                 ('starter-stormglass', 'orr_static_starter', 'stormglass')]


def inventory(value=None):
    value = g.read_json(ROOT / INVENTORY) if value is None else value
    g.require(value.get('schema') == SCHEMA and value.get('reviewed') is True,
              'Inventory is not reviewed for this gate schema')
    g.require(set(value['suites']) == set(SUITES), 'Changed suite inventory')
    for label, names in value['suites'].items():
        g.validate_names(names)
        g.require(not any('ws_acceptance::' in name or 'gpu_api_acceptance::' in name
                          for name in names), 'Device/socket test in CPU inventory')
        if label == 'remote-coop':
            g.require(all(name.startswith('coop_defense::') for name in names),
                      'Remote inventory escaped game-owned namespace')
    return value['suites']


def cargo(verb, package, features, targets):
    return ['cargo', verb, '--release', '--locked', '-p', package, '--no-default-features'] + (
        ['--features', features] if features else []) + targets


def artifact(output, suite):
    matches = []
    package = suite['package']
    directory = ROOT / suite['directory']
    # Cargo's canonical path ID omits the name when the directory already names
    # the package (orr_remote); renamed directories retain it (static starter).
    fragment = '0.0.1' if directory.name == package else package + '@0.0.1'
    for line in output.splitlines():
        if not line.startswith('{'):
            continue
        value = g.strict_json(line)
        target = value.get('target', {})
        if (value.get('reason') == 'compiler-artifact' and target.get('name') == package
                and target.get('kind') == ['lib'] and value.get('profile', {}).get('test') is True
                and value.get('executable')):
            g.require(value.get('manifest_path') == str(directory / 'Cargo.toml')
                      and target.get('src_path') == str(directory / 'src/lib.rs'),
                      'Wrong compiled package manifest or target source')
            g.require(value.get('package_id') == 'path+' + directory.as_uri() + '#' + fragment,
                      'Wrong compiled package identity')
            g.require(value.get('features') == suite['compiled_features'], 'Wrong compiled features')
            path = Path(value['executable'])
            g.require(path.is_absolute(), 'Relative compiler executable')
            matches.append(path)
    g.require(len(matches) == 1, f'Expected exactly one {package} lib-test executable producer')
    return matches[0]


def validate_roles(binaries):
    g.require(isinstance(binaries, dict) and set(binaries) == set(BINARY_PATHS),
              'Missing/extra executable roles')
    g.require(all(binaries[name].get('path') == path for name, path in BINARY_PATHS.items()),
              'Changed executable role path')


def build_binaries(directory, env):
    binaries = {}
    for label, suite in SUITES.items():
        command = cargo('test', suite['package'], suite['features'],
                        ['--lib', '--no-run', '--message-format=json'])
        output = g.run(command, directory, label + '-build', env)
        relative = BINARY_PATHS[label]
        binaries[label] = {'path': relative,
                           'sha256': g.copy_file(artifact(output, suite), directory / relative, True)}
    return binaries


def execute_binaries(directory, binaries, expected, env):
    validate_roles(binaries)
    g.verify_bindings(directory, binaries)
    for label, suite in SUITES.items():
        executable = directory / binaries[label]['path']
        # Admit the entire starter harness (remote: only its game-owned namespace)
        # before executing any test. New tests cannot silently evade review.
        listing = g.run([executable, *suite['list_args'], '--list', '--format=pretty'],
                        directory, label + '-inventory', env, 60)
        g.validate_listing(listing, expected[label] + suite['excluded'])
        # Explicit exact names cannot accidentally execute a newly added socket test.
        output = g.run([executable, *expected[label], '--exact', '--test-threads=1',
                        '--format=pretty', '--color=never', '--show-output'],
                       directory, label + '-tests', env, 600)
        g.validate_tests(output, expected[label])
        g.verify_bindings(directory, binaries)


def checks(directory, env):
    rust = g.run(['rustc', '--version', '--verbose'], directory, 'rustc', env, 60)
    g.require(re.search(r'^release: 1\.99\.1$', rust, re.M), 'Expected supported Rust 1.99.1')
    g.run(['cargo', '--version'], directory, 'cargo', env, 60)
    g.run([sys.executable, '-B', str(Path(__file__).resolve()), '--self-test'],
          directory, 'gate-self-tests', env, 120)
    for label, package, features in CLIPPY_CONFIGS:
        g.run(cargo('clippy', package, features, ['--lib', '--tests', '--bins']) + ['--', '-D', 'warnings'],
              directory, label + '-strict-clippy', env)
    for package in ['orr_remote', 'orr_static_starter']:
        g.run(cargo('check', package, '', ['--lib']), directory,
              package.replace('_', '-') + '-feature-off-check', env)


def command_contract(root, directory, expected, replay, python):
    commands = {}
    self_test = [python, '-B', str(root / 'tools/check_stormglass_live_observation.py'), '--self-test']
    if replay:
        commands['replay-gate-self-tests'] = (self_test, 120)
    else:
        commands.update(rustc=(['rustc', '--version', '--verbose'], 60), cargo=(['cargo', '--version'], 60))
        commands['gate-self-tests'] = (self_test, 120)
        for label, package, features in CLIPPY_CONFIGS:
            commands[label + '-strict-clippy'] = (
                cargo('clippy', package, features, ['--lib', '--tests', '--bins']) + ['--', '-D', 'warnings'], 2700)
        for package in ['orr_remote', 'orr_static_starter']:
            commands[package.replace('_', '-') + '-feature-off-check'] = (cargo('check', package, '', ['--lib']), 2700)
    for label, suite in SUITES.items():
        if not replay:
            commands[label + '-build'] = (cargo('test', suite['package'], suite['features'],
                                                  ['--lib', '--no-run', '--message-format=json']), 2700)
        executable = str(directory / BINARY_PATHS[label])
        commands[label + '-inventory'] = ([executable, *suite['list_args'], '--list', '--format=pretty'], 60)
        commands[label + '-tests'] = ([executable, *expected[label], '--exact', '--test-threads=1',
                                      '--format=pretty', '--color=never', '--show-output'], 600)
    return commands


def validate_commands(directory, expected, replay, source_root, evidence_root, python):
    # Downloaded logs retain the hosted paths. Audit their recorded argv against
    # the same closed contract, without resolving those paths on this machine.
    for path in [source_root, evidence_root, python]:
        g.require(isinstance(path, str) and Path(path).is_absolute() and '..' not in Path(path).parts,
                  'Invalid command origin path')
    commands = command_contract(Path(source_root), Path(evidence_root), expected, replay, python)
    g.require({path.name for path in directory.glob('*.command.json')} ==
              {label + '.command.json' for label in commands}, 'Missing/extra command evidence')
    g.require({path.name for path in directory.glob('*.log')} ==
              {label + '.log' for label in commands}, 'Missing/extra command logs')
    for label, (argv, timeout) in commands.items():
        record = g.read_json(directory / (label + '.command.json'))
        g.require(record.get('argv') == argv and record.get('cwd') == source_root
                  and record.get('timeout_seconds') == timeout and record.get('expected_exit') == 0
                  and record.get('exit_code') == 0 and record.get('status') == 'passed'
                  and 'error' not in record, 'Command did not satisfy required contract: ' + label)
        log_path = directory / (label + '.log')
        g.require(log_path.is_file() and not log_path.is_symlink()
                  and log_path.stat().st_size <= 128 * 1024 * 1024, 'Missing or invalid command log')
        log = log_path.read_text(encoding='utf-8')
        g.require(log.startswith('+ ' + ' '.join(argv) + '\n') and log.endswith('\nEXIT_CODE=0\n'),
                  'Log does not bind successful command: ' + label)
        if label.endswith('-inventory'):
            role = label.removesuffix('-inventory')
            g.validate_listing(log, expected[role] + SUITES[role]['excluded'])
        elif label.endswith('-tests') and label.removesuffix('-tests') in SUITES:
            g.validate_tests(log, expected[label.removesuffix('-tests')])
    return sorted(commands)


def replay_contract(origin, source, expected_head, expected):
    g.verify_seal(origin)
    original = g.read_json(origin / 'scope.json')
    g.require(original.get('schema') == SCHEMA and original.get('status') == 'passed'
              and original.get('mode') == 'producer' and original.get('compile_checks_performed') is True
              and original.get('compile_checks_replayed') is False,
              'Replay requires completed producer evidence')
    g.require(all(original.get(key) == value for key, value in SCOPE.items()), 'Changed producer scope')
    g.validate_replay_source(g.read_json(origin / 'source.json'), source, expected_head)
    g.require(original.get('source_sha256') == g.sha(origin / 'source.json')
              and original.get('inventory_sha256') == g.sha(origin / 'inventory.json'),
              'Inconsistent source/inventory digest binding')
    g.require(g.read_json(origin / 'inventory.json') == g.read_json(ROOT / INVENTORY),
              'Downloaded test inventory differs from source')
    g.require(original.get('head') == expected_head and original.get('tree') == source['tree'],
              'Downloaded candidate identity mismatch')
    g.require(original.get('suite_configurations') == SUITES
              and original.get('counts') == {k: len(v) for k, v in expected.items()},
              'Downloaded suite contract changed')
    binaries = original['binaries']
    validate_roles(binaries)
    g.verify_bindings(origin, binaries)
    labels = validate_commands(origin, expected, False, original.get('source_directory'),
                               original.get('evidence_directory'), original.get('python_executable'))
    g.require(original.get('command_labels') == labels, 'Changed producer command inventory')
    return binaries


def main(args):
    g.require(Path.cwd().resolve() == ROOT, 'Run from the exact candidate repository root')
    g.require(re.fullmatch(r'[0-9a-f]{40}', args.expected_head or ''), 'Explicit full expected HEAD required')
    if not args.replay and os.environ.get('GITHUB_SHA'):
        g.require(os.environ['GITHUB_SHA'] == args.expected_head, 'GITHUB_SHA differs from expected HEAD')
    evidence_root = Path(os.environ['EVIDENCE']).resolve()
    g.require(evidence_root != ROOT and not evidence_root.is_relative_to(ROOT), 'Evidence must be outside source')
    evidence_root.mkdir(parents=True, exist_ok=True)
    directory = evidence_root / ('replay' if args.replay else 'acceptance')
    directory.mkdir()  # Never replace failed or interrupted evidence.
    (directory / 'bin').mkdir()
    scope = dict(SCOPE, schema=SCHEMA, status='running', head=args.expected_head,
                 mode='downloaded-artifact-replay' if args.replay else 'producer',
                 compile_checks_replayed=False, compile_checks_performed=not bool(args.replay),
                 source_directory=str(ROOT), evidence_directory=str(directory), python_executable=sys.executable)
    source = None
    try:
        expected = inventory()
        source = g.source_fence(None if args.replay else args.expected_head, directory, 'before')
        scope['consumer_head' if args.replay else 'producer_head'] = source['head']
        g.write_json(directory / 'source.json', source)
        g.copy_file(ROOT / INVENTORY, directory / 'inventory.json')
        env = g.execution_environment(directory, args.expected_head)
        env.pop('ORR_STATIC_STANDALONE_BUILD_SHA', None)  # No native identity/build claim.
        if args.replay:
            origin = args.replay.resolve()
            g.require(origin != directory and not directory.is_relative_to(origin), 'Separate replay evidence required')
            binaries = replay_contract(origin, source, args.expected_head, expected)
            for entry in binaries.values():
                g.copy_file(origin / entry['path'], directory / entry['path'], True)
            scope['producer_scope_sha256'] = g.sha(origin / 'scope.json')
            scope['producer_manifest_sha256'] = g.sha(origin / 'artifact-manifest.json')
            g.run([sys.executable, '-B', str(Path(__file__).resolve()), '--self-test'],
                  directory, 'replay-gate-self-tests', env, 120)
        else:
            g.reject_ancestor_config(ROOT)
            checks(directory, env)
            binaries = build_binaries(directory, env)
        execute_binaries(directory, binaries, expected, env)
        scope['command_labels'] = validate_commands(directory, expected, bool(args.replay),
                                                     str(ROOT), str(directory), sys.executable)
        if args.replay:
            g.verify_seal(origin)
        else:
            g.reject_ancestor_config(ROOT)
        g.require(g.source_fence(None if args.replay else args.expected_head, directory, 'after') == source,
                  'Source HEAD/tree/bytes changed during validation')
        scope.update(status='passed', tree=source['tree'], counts={k: len(v) for k, v in expected.items()},
                     suite_configurations=SUITES, binaries=binaries,
                     source_sha256=g.sha(directory / 'source.json'),
                     inventory_sha256=g.sha(directory / 'inventory.json'))
    except Exception as error:
        scope.update(status='failed', error=str(error)[:4096])
        raise
    finally:
        if source is not None and not (directory / 'after-status.txt').exists():
            try:
                if not args.replay:
                    g.reject_ancestor_config(ROOT)
                scope['source_unchanged_after_failure'] = g.source_fence(
                    None if args.replay else args.expected_head, directory, 'failure-after') == source
            except Exception as error:
                scope['source_after_failure_error'] = str(error)[:4096]
        g.write_json(directory / 'scope.json', scope)
        g.seal(directory)


class GateTests(unittest.TestCase):
    def test_source_loader_ignores_timestamp_valid_stale_bytecode(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'helper.py'
            path.write_text("value = 'stale'\n")
            stamp = path.stat().st_mtime_ns
            g.py_compile.compile(str(path), doraise=True,
                                 invalidation_mode=g.py_compile.PycInvalidationMode.TIMESTAMP)
            path.write_text("value = 'fresh'\n")
            os.utime(path, ns=(stamp, stamp))
            self.assertEqual(source_module(path).value, 'fresh')

    def test_inventory_rejects_wrong_schema_roles_empty_duplicates_and_socket_tests(self):
        good = {'schema': SCHEMA, 'reviewed': True,
                'suites': {label: ['coop_defense::tests::case'] if label == 'remote-coop' else ['test_case']
                           for label in SUITES}}
        inventory(good)
        for mutate in [lambda v: v.update(schema='other'), lambda v: v.update(reviewed=False),
                       lambda v: v['suites'].pop('remote-coop'),
                       lambda v: v['suites'].update({'starter-core': []}),
                       lambda v: v['suites'].update({'starter-core': ['case', 'case']}),
                       lambda v: v['suites'].update({'starter-core': [GAMEPLAY_SOCKET_TESTS[0]]}),
                       lambda v: v['suites'].update({'remote-coop': ['other::case']})]:
            value = json.loads(json.dumps(good))
            mutate(value)
            with self.assertRaises(ValueError):
                inventory(value)

    def test_roles_reject_omission_addition_and_alias(self):
        good = {label: {'path': path} for label, path in BINARY_PATHS.items()}
        validate_roles(good)
        for bad in [{k: v for k, v in good.items() if k != 'remote-coop'},
                    dict(good, extra={'path': 'bin/extra'}),
                    dict(good, **{'starter-core': good['starter-gameplay']})]:
            with self.assertRaises(ValueError):
                validate_roles(bad)

    def test_compiler_identity_features_and_unique_test_producer(self):
        for label, suite in SUITES.items():
            directory = ROOT / suite['directory']
            package = suite['package']
            fragment = '0.0.1' if label == 'remote-coop' else 'orr_static_starter@0.0.1'
            prefix = 'path+' + directory.as_uri() + '#'
            good = {'reason': 'compiler-artifact', 'manifest_path': str(directory / 'Cargo.toml'),
                    'package_id': prefix + fragment,
                    'target': {'name': package, 'kind': ['lib'], 'src_path': str(directory / 'src/lib.rs')},
                    'profile': {'test': True}, 'features': suite['compiled_features'], 'executable': '/tmp/lib-tests'}
            with self.subTest(suite=label):
                self.assertEqual(artifact(json.dumps(good), suite), Path('/tmp/lib-tests'))
            for bad_id in ['wrong', prefix + 'wrong_name@0.0.1', prefix + fragment.replace('0.0.1', '0.0.2'),
                           'path+' + (directory / 'wrong').as_uri() + '#' + fragment,
                           prefix + (package + '@0.0.1' if label == 'remote-coop' else '0.0.1')]:
                with self.subTest(suite=label, package_id=bad_id), self.assertRaises(ValueError):
                    artifact(json.dumps(dict(good, package_id=bad_id)), suite)
            for mutate in [lambda v: v.update(manifest_path='/tmp/Cargo.toml'),
                           lambda v: v['target'].update(src_path='/tmp/lib.rs'),
                           lambda v: v['target'].update(name='wrong_name'),
                           lambda v: v['target'].update(kind=['bin']), lambda v: v['profile'].update(test=False),
                           lambda v: v.update(features=['unreviewed']), lambda v: v.update(executable='relative')]:
                value = json.loads(json.dumps(good))
                mutate(value)
                with self.subTest(suite=label), self.assertRaises(ValueError):
                    artifact(json.dumps(value), suite)
            for output in ['', json.dumps(good) + '\n' + json.dumps(good)]:
                with self.subTest(suite=label), self.assertRaises(ValueError):
                    artifact(output, suite)

    def test_execution_uses_exact_cpu_names_and_rejects_new_cases_before_running(self):
        expected = {label: ['coop_defense::tests::case'] if label == 'remote-coop' else ['cpu_case']
                    for label in SUITES}
        binaries = {label: {'path': path} for label, path in BINARY_PATHS.items()}
        commands = []

        def output(command, _directory, label, _env, _timeout):
            commands.append(command)
            role = label.rsplit('-', 1)[0]
            if label.endswith('-inventory'):
                names = expected[role] + SUITES[role]['excluded']
                return '\n'.join(name + ': test' for name in names) + f'\n\n{len(names)} tests, 0 benchmarks\n'
            self.assertEqual(command[1:1 + len(expected[role])], expected[role])
            self.assertIn('--exact', command)
            self.assertNotIn('--skip', command)
            self.assertFalse(set(command).intersection(SUITES[role]['excluded']))
            return ('running 1 test\ntest ' + expected[role][0] + ' ... ok\n'
                    'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')

        with patch.object(g, 'verify_bindings'), patch.object(g, 'run', side_effect=output):
            execute_binaries(Path('/tmp/fixture'), binaries, expected, {})
        self.assertEqual(len(commands), 2 * len(SUITES))
        for invalid in ['\n0 tests, 0 benchmarks\n',
                        'coop_defense::tests::case: test\nunreviewed_case: test\n\n2 tests, 0 benchmarks\n']:
            with patch.object(g, 'verify_bindings'), patch.object(g, 'run', return_value=invalid) as run:
                with self.assertRaises(ValueError):
                    execute_binaries(Path('/tmp/fixture'), binaries, expected, {})
                self.assertEqual(run.call_count, 1)

    def test_replay_rejects_changed_producer_contract_source_and_artifact(self):
        with tempfile.TemporaryDirectory() as temp:
            origin = Path(temp)
            (origin / 'bin').mkdir()
            document = g.read_json(ROOT / INVENTORY)
            expected = document['suites']
            head = 'a' * 40
            source = {'head': head, 'tree': 'b' * 40, 'files': {'source.rs': 'c' * 64}}
            g.write_json(origin / 'source.json', source)
            g.write_json(origin / 'inventory.json', document)
            binaries = {}
            for role, path in BINARY_PATHS.items():
                (origin / path).write_text(role)
                binaries[role] = {'path': path, 'sha256': g.sha(origin / path)}
            good = dict(SCOPE, schema=SCHEMA, status='passed', mode='producer', head=head, tree=source['tree'],
                        compile_checks_performed=True, compile_checks_replayed=False,
                        suite_configurations=SUITES, counts={k: len(v) for k, v in expected.items()},
                        binaries=binaries, source_sha256=g.sha(origin / 'source.json'),
                        inventory_sha256=g.sha(origin / 'inventory.json'),
                        source_directory=str(ROOT), evidence_directory=str(origin), python_executable=sys.executable)
            commands = command_contract(ROOT, origin, expected, False, sys.executable)
            good['command_labels'] = sorted(commands)
            self.assertEqual(len(commands), 23)
            self.assertEqual(len(command_contract(ROOT, origin, expected, True, sys.executable)), 9)
            self.assertEqual({name for name in commands if name.endswith('-strict-clippy')}, {
                'remote-core-strict-clippy', 'remote-coop-strict-clippy', 'remote-sample-host-strict-clippy',
                'starter-core-strict-clippy', 'starter-gameplay-strict-clippy', 'starter-stormglass-strict-clippy',
            })
            self.assertEqual(commands['remote-sample-host-strict-clippy'], (
                ['cargo', 'clippy', '--release', '--locked', '-p', 'orr_remote', '--no-default-features',
                 '--features', 'sample-host', '--lib', '--tests', '--bins', '--', '-D', 'warnings'], 2700))
            for label, (argv, timeout) in commands.items():
                g.write_json(origin / (label + '.command.json'),
                             {'argv': argv, 'cwd': str(ROOT), 'timeout_seconds': timeout,
                              'expected_exit': 0, 'exit_code': 0, 'status': 'passed'})
                log = '+ ' + ' '.join(argv) + '\n'
                if label.endswith('-inventory'):
                    role = label.removesuffix('-inventory')
                    names = expected[role] + SUITES[role]['excluded']
                    log += ''.join(name + ': test\n' for name in names) + f'\n{len(names)} tests, 0 benchmarks\n'
                elif label.endswith('-tests') and label.removesuffix('-tests') in SUITES:
                    names = expected[label.removesuffix('-tests')]
                    log += f'running {len(names)} tests\n' + ''.join('test ' + name + ' ... ok\n' for name in names)
                    log += f'test result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
                (origin / (label + '.log')).write_text(log + '\nEXIT_CODE=0\n')

            def seal(value):
                g.write_json(origin / 'scope.json', value)
                g.seal(origin)

            seal(good)
            self.assertEqual(replay_contract(origin, source, head, expected), binaries)
            for mutate in [lambda v: v.update(schema='other'), lambda v: v.update(status='failed'),
                           lambda v: v.update(mode='downloaded-artifact-replay'),
                           lambda v: v.update(compile_checks_performed=False),
                           lambda v: v.update(compile_checks_replayed=True), lambda v: v.update(socket=True),
                           lambda v: v.update(head='d' * 40), lambda v: v.update(tree='d' * 40),
                           lambda v: v.update(source_sha256='d' * 64), lambda v: v.update(inventory_sha256='d' * 64),
                           lambda v: v['counts'].update({'remote-coop': 0}),
                           lambda v: v['suite_configurations']['remote-coop'].update(features=''),
                           lambda v: v['binaries'].update({'starter-core': v['binaries']['starter-gameplay']}),
                           lambda v: v.update(command_labels=[]), lambda v: v.update(source_directory='/other')]:
                value = json.loads(json.dumps(good))
                mutate(value)
                seal(value)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
            seal(good)
            with self.assertRaises(ValueError):
                replay_contract(origin, dict(source, tree='d' * 40), head, expected)
            with self.assertRaises(ValueError):
                replay_contract(origin, source, 'd' * 40, expected)
            path = origin / 'remote-core-strict-clippy.command.json'
            command = g.read_json(path)
            for change in [{'exit_code': 1}, {'status': 'running'}, {'argv': ['cargo', 'check']}, {'timeout_seconds': 0}]:
                g.write_json(path, dict(command, **change))
                g.seal(origin)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
            g.write_json(path, command)
            for label in ['remote-core-strict-clippy', 'remote-sample-host-strict-clippy']:
                path = origin / (label + '.command.json')
                command = g.read_json(path)
                path.unlink()
                g.seal(origin)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
                g.write_json(path, command)
            g.seal(origin)
            (origin / BINARY_PATHS['remote-coop']).write_text('changed')
            with self.assertRaises(ValueError):
                replay_contract(origin, source, head, expected)


if __name__ == '__main__':
    if '--self-test' in sys.argv:
        tests = unittest.TestSuite([
            unittest.defaultTestLoader.loadTestsFromTestCase(g.ValidatorTests),
            unittest.defaultTestLoader.loadTestsFromTestCase(GateTests),
        ])
        sys.exit(not unittest.TextTestRunner(verbosity=2).run(tests).wasSuccessful())
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected-head', default=os.environ.get('GITHUB_SHA'))
    parser.add_argument('--replay', type=Path)
    try:
        main(parser.parse_args())
    except Exception as error:
        print('Stormglass live observation gate FAILED: ' + str(error), file=sys.stderr)
        sys.exit(1)
