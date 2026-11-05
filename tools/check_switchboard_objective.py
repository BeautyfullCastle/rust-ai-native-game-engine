#!/usr/bin/env python3
"""Exact-head CPU acceptance and authenticated-download replay; no window/device runs.

The caller authenticates the downloaded workflow/run/head before --replay. A hash
manifest proves integrity, not origin. Inventory preparation is separate from this
gate: compile/list/review, then commit reviewed inventories before production use.
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

ROOT = Path(__file__).resolve().parents[1]


def source_module(path, name):
    module = types.ModuleType(name)
    module.__file__ = str(path)
    sys.modules[name] = module
    exec(compile(path.read_bytes(), str(path), 'exec'), module.__dict__)
    return module


h = source_module(ROOT / 'tools/check_stormglass_live_observation.py', 'objective_shared')
g = h.g
SCHEMA = 'orr.delivery-reachability-focused/1'
INVENTORY = 'tools/switchboard_objective_inventory.json'
PACKAGE = 'orr_static_starter'
STARTER = 'examples/static-game-starter'
SELF = 'tools/check_switchboard_objective.py'
SCOPE = dict(h.SCOPE, native_identity_query=True, native_harness_compile_only=True,
             synthetic_pixel_oracles=True, genuine_visual_acceptance=False,
             cpu_in_process=False, cpu_filesystem=True, controlled_fixture_subprocesses=True,
             subprocess_scope=("copied headless-player acceptance cases; copied native provenance, identity and "
                               "reachability queries; reviewed bundle launcher, timeout and process-group shell fixtures"),
             native_reachability_query=True, reachability_scope="admitted_initial_scene")


def suite(features, target=None):
    expanded = set(features.split(',')) if features else set()
    if 'gameplay' in expanded:
        expanded.add('editor')
    if 'native-acceptance' in expanded:
        expanded.add('editor')
    if 'standalone-native-acceptance' in expanded:
        expanded.update(['standalone', 'bundle'])
    return {'features': features, 'compiled_features': sorted(expanded),
            'target': ['--test', target] if target else ['--lib'],
            'excluded': list(h.GAMEPLAY_SOCKET_TESTS) if features == 'gameplay' and target is None else []}


SUITES = {
    'reachability-core': suite('', 'delivery_authored_reachability'),
    'reachability-gameplay': suite('gameplay', 'delivery_authored_reachability'),
    'bundle-lib': suite('bundle'), 'bundle-contract': suite('bundle', 'bundle'),
    'core': suite(''), 'standalone': suite('standalone'),
    'standalone-actions': suite('standalone,static-game-actions'),
    'gameplay': suite('gameplay'),
    'controller': suite('standalone', 'standalone_contract'),
    'controller-actions': suite('standalone,static-game-actions', 'standalone_contract'),
    'objectives': suite('standalone', 'switchboard_objective'),
    'objectives-actions': suite('standalone,static-game-actions', 'switchboard_objective'),
    'action-compatibility': suite('standalone,static-game-actions', 'multi_action_contract'),
    'editor-acceptance': suite('editor', 'acceptance'),
    'gameplay-acceptance': suite('gameplay', 'acceptance'),
}
NATIVE = {'native-window': suite('native-acceptance', 'native_window'),
          'standalone-x11': suite('standalone-native-acceptance', 'standalone_x11')}
PLAYERS = {alias + '-player': dict(suite('standalone'), target=['--bin', alias + '-player'])
           for alias in g.GAMES}
HEADLESS = {'player': dict(suite(''), target=['--bin', 'player'])}
BUILDS = dict(SUITES, **NATIVE, **PLAYERS, **HEADLESS)
BINARY_PATHS = {role: 'bin/' + role for role in BUILDS}
ASSET_SOURCES = {alias: STARTER + '/scenes/' + alias + '.scene.yaml' for alias in g.GAMES}
ASSET_PATHS = dict(g.ASSET_PATHS)
for fixture in ['current-unreachable', 'restart-unreachable', 'both-unreachable']:
    role = 'delivery-' + fixture
    ASSET_SOURCES[role] = STARTER + '/tests/fixtures/delivery-reachability/' + fixture + '.scene.yaml'
    ASSET_PATHS[role] = 'scenes/' + role + '.scene.yaml'

# Independently derived residue cases: (current, origin) TT, FT, TF, FF.
# Negative -0.25 has Euclidean residue 49152, matching goal 1.75.
CURRENT_WARNING = ('Goal unreachable from the current position. Match the courier and goal '
                   'fractional coordinates on both axes; whole-unit coordinates work.')
ORIGIN_WARNING = ('Goal unreachable after restart at the origin. Use whole-unit goal '
                  'coordinates for origin restart.')
QUERY_CASES = {
    'delivery-stock': ('delivery', 'delivery', True, True),
    'delivery-current-unreachable': ('delivery', 'delivery-current-unreachable', False, True),
    'delivery-restart-unreachable': ('delivery', 'delivery-restart-unreachable', True, False),
    'delivery-both-unreachable': ('delivery', 'delivery-both-unreachable', False, False),
    'switchboard-unsupported': ('switchboard', 'switchboard', None, None),
}


def report_expectations():
    return {case: {'schema': 'orr.delivery-reachability/1', 'game': g.GAMES[alias][0],
                   'scope': 'admitted_initial_scene', 'analysis': 'integer_unit_exact_position',
                   'supported': alias == 'delivery', 'current_to_goal': current, 'origin_to_goal': origin,
                   'warnings': ([CURRENT_WARNING] if current is False else []) +
                               ([ORIGIN_WARNING] if origin is False else [])}
            for case, (alias, _, current, origin) in QUERY_CASES.items()}


def validate_report(case, actual):
    g.require(case in QUERY_CASES, 'Unreviewed reachability query')
    g.require(json.dumps(actual, sort_keys=True, separators=(',', ':')) ==
              json.dumps(report_expectations()[case], sort_keys=True, separators=(',', ':')),
              'Reachability report differs from independent expectation: ' + case)


def validate_asset_sources(source, assets):
    g.require(set(assets) == set(ASSET_SOURCES), 'Changed scene source roles')
    for role, entry in assets.items():
        g.require(entry['sha256'] == source['files'][ASSET_SOURCES[role]],
                  'Scene asset differs from reviewed source: ' + role)

PYTHON_MODULES = ['test_static_game_native_gate', 'test_static_game_standalone_gate',
                  'test_static_game_native_ui_settle_gate']
STRICT = {'bundle': 'bundle', 'core': '', 'standalone': 'standalone', 'standalone-actions': 'standalone,static-game-actions',
          'editor': 'editor', 'gameplay': 'gameplay'}


def inventory(value=None):
    value = g.read_json(ROOT / INVENTORY) if value is None else value
    g.require(value.get('schema') == SCHEMA and value.get('reviewed') is True, 'Inventory must be reviewed')
    g.require(set(value['suites']) == set(SUITES), 'Changed CPU role inventory')
    g.require(value.get('excluded') == {k: v['excluded'] for k, v in SUITES.items()}, 'Changed fixed exclusions')
    for names in value['suites'].values():
        g.validate_names(names)
        g.require(not any('ws_acceptance::' in n or 'gpu_api_acceptance::' in n for n in names), 'Non-CPU test')
    g.require(json.dumps(value.get('reports'), sort_keys=True, separators=(',', ':')) ==
              json.dumps(report_expectations(), sort_keys=True, separators=(',', ':')),
              'Changed independent report pins')
    validate_identity_expectations(value.get('identities'))
    names = value['python_tests']
    g.require(isinstance(names, list) and names and len(names) == len(set(names)), 'Invalid Python inventory')
    g.require(all(isinstance(n, str) and re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*\.[A-Za-z_][A-Za-z_0-9]*\.test_[A-Za-z_0-9]+', n)
                  and n.split('.')[0] in PYTHON_MODULES for n in names), 'Unexpected Python test identity')
    return value['suites']


def validate_identity_expectations(identities):
    g.require(isinstance(identities, dict) and set(identities) == set(g.GAMES), 'Missing/extra reviewed identities')
    fields = {'game', 'alias', 'build_id', 'schema', 'input_contract', 'seed', 'tick_rate', 'player_count', 'initial_checksum'}
    for alias, (game, build) in g.GAMES.items():
        value = identities[alias]
        g.require(isinstance(value, dict) and set(value) == fields, 'Incomplete reviewed identity')
        g.require(value['game'] == game and value['alias'] == alias and value['build_id'] == build,
                  'Changed reviewed game/build identity')
        g.require(value['seed'] == 42 and value['tick_rate'] == 60 and value['player_count'] == 1
                  and re.fullmatch(r'0x[0-9a-f]{16}', value['initial_checksum']), 'Invalid reviewed session identity')
        schema, contract = value['schema'], value['input_contract']
        g.require(isinstance(schema, dict) and schema.get('properties') and schema.get('$defs') and schema.get('required'),
                  'Empty reviewed scene schema')
        g.require(isinstance(contract, dict) and set(contract) == {'schema', 'value_format', 'max_players'}
                  and contract['max_players'] == 1 and isinstance(contract['value_format'], str) and contract['value_format'],
                  'Incomplete reviewed input contract')
        g.require(isinstance(contract['schema'], dict) and contract['schema'].get('properties')
                  and contract['schema'].get('required'), 'Empty reviewed input schema')


def validate_identity(alias, actual, identities):
    validate_identity_expectations(identities)
    # Canonical JSON equality also distinguishes booleans from integer fields.
    g.require(json.dumps(actual, sort_keys=True, separators=(',', ':')) ==
              json.dumps(identities[alias], sort_keys=True, separators=(',', ':')),
              'Native identity differs from reviewed schema/input/checksum: ' + alias)


def execution_environment(directory, head):
    env = g.execution_environment(directory, head)
    # Never fall back to Cargo's original producer path or an inherited override.
    env['ORR_STATIC_STARTER_PLAYER_TEST_BIN'] = str(directory / BINARY_PATHS['player'])
    return env


def cargo(verb, config, targets):
    return h.cargo(verb, PACKAGE, config, targets)


def artifact(output, spec):
    target = spec['target']
    kind = 'lib' if target == ['--lib'] else 'test' if target[0] == '--test' else 'bin'
    name = PACKAGE if kind == 'lib' else target[1]
    source = STARTER + ('/src/lib.rs' if kind == 'lib' else '/tests/' + name + '.rs' if kind == 'test' else '/src/bin/' + name + '.rs')
    matches = []
    for line in output.splitlines():
        if not line.startswith('{'):
            continue
        value = g.strict_json(line)
        actual = value.get('target', {})
        # Cargo can emit a same-name bin while building an integration test.
        # Only the exact requested target kind participates in producer selection.
        if (value.get('reason') != 'compiler-artifact' or actual.get('name') != name
                or actual.get('kind') != [kind] or not value.get('executable')):
            continue
        g.require(actual.get('src_path') == str(ROOT / source), 'Wrong target source')
        g.require(value.get('profile', {}).get('test') is (kind != 'bin'), 'Wrong test profile')
        g.require(value.get('manifest_path') == str(ROOT / STARTER / 'Cargo.toml'), 'Wrong package manifest')
        g.require(value.get('package_id') == 'path+' + (ROOT / STARTER).as_uri() + '#orr_static_starter@0.0.1', 'Wrong package identity')
        g.require(value.get('features') == spec['compiled_features'], 'Wrong compiled features')
        path = Path(value['executable'])
        g.require(path.is_absolute(), 'Relative compiler executable')
        matches.append(path)
    g.require(len(matches) == 1, 'Missing or ambiguous compiled producer')
    return matches[0]


def build_command(spec):
    binary = spec['target'][0] == '--bin'
    return cargo('build' if binary else 'test', spec['features'], spec['target']) + ([] if binary else ['--no-run']) + ['--message-format=json']


def command_contract(root, directory, expected, replay, python):
    commands = {}
    commands['replay-gate-self-tests' if replay else 'gate-self-tests'] = ([python, '-B', str(root / SELF), '--self-test'], 120)
    commands['python-oracle-tests'] = ([python, '-B', str(root / SELF), '--python-tests'], 180)
    if not replay:
        commands.update(rustc=(['rustc', '--version', '--verbose'], 60), cargo=(['cargo', '--version'], 60))
        for label, features in STRICT.items():
            commands[label + '-strict-clippy'] = (cargo('clippy', features, ['--lib', '--tests', '--bins']) + ['--', '-D', 'warnings'], 2700)
        commands['feature-off-check'] = (cargo('check', '', ['--lib']), 2700)
        for label, spec in NATIVE.items():
            commands[label + '-strict-clippy'] = (cargo('clippy', spec['features'], spec['target']) + ['--', '-D', 'warnings'], 2700)
        # Stable grouping avoids relinking shared Cargo bin paths on feature toggles.
        # Each producer is still copied immediately, with an independent role/hash.
        for label, spec in sorted(BUILDS.items(), key=lambda entry: entry[1]['features']):
            commands[label + '-build'] = (build_command(spec), 2700)
    for label, spec in SUITES.items():
        executable = str(directory / BINARY_PATHS[label])
        commands[label + '-inventory'] = ([executable, '--list', '--format=pretty'], 60)
        commands[label + '-tests'] = ([executable, *expected[label], '--exact', '--test-threads=1', '--format=pretty', '--color=never', '--show-output'], 600)
    for alias in g.GAMES:
        binary = str(directory / BINARY_PATHS[alias + '-player'])
        commands[alias + '-provenance'] = ([binary, '--provenance'], 30)
        commands[alias + '-identity'] = ([binary, '--identity', '--scene', str(directory / ASSET_PATHS[alias])], 30)
    for case, (alias, asset, _, _) in QUERY_CASES.items():
        commands[case + '-reachability'] = ([str(directory / BINARY_PATHS[alias + '-player']),
                                            '--reachability', '--scene', str(directory / ASSET_PATHS[asset])], 30)
    return commands


# Reuse closed command-record auditing without copying its implementation.
h.ROOT, h.SUITES, h.command_contract = ROOT, SUITES, command_contract


def validate_commands(directory, expected, replay, source_root, evidence_root, python):
    labels = h.validate_commands(directory, expected, replay, source_root, evidence_root, python)
    for alias, (game, build) in g.GAMES.items():
        provenance = g.json_output((directory / (alias + '-provenance.log')).read_text())
        identity = g.json_output((directory / (alias + '-identity.log')).read_text())
        source = g.read_json(directory / 'source.json')
        head = g.read_json(directory / 'scope.json')['head']
        g.require(provenance == {'source_sha': head}, 'Native provenance differs from candidate')
        validate_identity(alias, identity, g.read_json(ROOT / INVENTORY)['identities'])
        g.require(source.get('files'), 'Missing source identity')
    for case in QUERY_CASES:
        validate_report(case, g.json_output((directory / (case + '-reachability.log')).read_text()))
    python_log = (directory / 'python-oracle-tests.log').read_text()
    names = g.read_json(ROOT / INVENTORY)['python_tests']
    g.require('SOURCE_PYTHON_INVENTORY=' + json.dumps(names, separators=(',', ':')) + '\n' in python_log,
              'Python tests differ from reviewed inventory')
    g.require(f'\nRan {len(names)} tests in ' in python_log and '\nOK\n' in python_log, 'Python suite did not pass')
    return labels


def validate_roles(binaries, assets):
    g.require(set(binaries) == set(BINARY_PATHS) and set(assets) == set(ASSET_PATHS), 'Changed artifact roles')
    for entries, paths in [(binaries, BINARY_PATHS), (assets, ASSET_PATHS)]:
        g.require(all(entries[k].get('path') == v for k, v in paths.items()), 'Changed artifact role path')


def replay_contract(origin, source, head, expected):
    g.verify_seal(origin)
    scope = g.read_json(origin / 'scope.json')
    g.require(all(scope.get(k) == v for k, v in SCOPE.items()), 'Changed producer scope')
    g.require(scope.get('schema') == SCHEMA and scope.get('status') == 'passed' and scope.get('mode') == 'producer'
              and scope.get('compile_checks_performed') is True and scope.get('compile_checks_replayed') is False,
              'Requires complete producer evidence')
    g.validate_replay_source(g.read_json(origin / 'source.json'), source, head)
    g.require(scope.get('head') == head and scope.get('tree') == source['tree'], 'Wrong producer candidate')
    g.require(scope.get('source_sha256') == g.sha(origin / 'source.json') and scope.get('inventory_sha256') == g.sha(origin / 'inventory.json'), 'Wrong source/inventory binding')
    g.require(g.read_json(origin / 'inventory.json') == g.read_json(ROOT / INVENTORY), 'Different reviewed inventory')
    g.require(scope.get('configurations') == BUILDS and scope.get('counts') == {k: len(v) for k, v in expected.items()}, 'Changed build/test matrix')
    binaries, assets = scope['binaries'], scope['assets']
    validate_roles(binaries, assets)
    g.verify_bindings(origin, binaries)
    g.verify_bindings(origin, assets)
    validate_asset_sources(source, assets)
    labels = validate_commands(origin, expected, False, scope.get('source_directory'), scope.get('evidence_directory'), scope.get('python_executable'))
    g.require(scope.get('command_labels') == labels, 'Changed producer command inventory')
    return binaries, assets


def python_tests():
    inventory()
    for name in ['static_game_native_gate', 'static_game_standalone_gate']:
        source_module(ROOT / 'tools' / (name + '.py'), name)
    all_tests = []
    def flatten(suite):
        for entry in suite:
            if isinstance(entry, unittest.TestSuite):
                yield from flatten(entry)
            else:
                yield entry
    for name in PYTHON_MODULES:
        module = source_module(ROOT / 'tools' / (name + '.py'), name)
        all_tests.extend(flatten(unittest.defaultTestLoader.loadTestsFromModule(module)))
    names = g.read_json(ROOT / INVENTORY)['python_tests']
    g.require(sorted(t.id() for t in all_tests) == sorted(names), 'Python inventory drift')
    by_name = {t.id(): t for t in all_tests}
    print('SOURCE_PYTHON_INVENTORY=' + json.dumps(names, separators=(',', ':')), flush=True)
    result = unittest.TextTestRunner(verbosity=2).run(unittest.TestSuite(by_name[n] for n in names))
    g.require(result.wasSuccessful() and result.testsRun == len(names) and not result.skipped and not result.expectedFailures,
              'Python tests failed, skipped or incomplete')


def main(args):
    g.require(Path.cwd().resolve() == ROOT, 'Run at candidate root')
    g.require(re.fullmatch(r'[0-9a-f]{40}', args.expected_head or ''), 'Full expected head required')
    if not args.replay and os.environ.get('GITHUB_SHA'):
        g.require(os.environ['GITHUB_SHA'] == args.expected_head, 'Wrong hosted head')
    evidence = Path(os.environ['EVIDENCE']).resolve()
    g.require(evidence != ROOT and not evidence.is_relative_to(ROOT), 'Evidence must be external')
    evidence.mkdir(parents=True, exist_ok=True)
    directory = evidence / ('replay' if args.replay else 'acceptance')
    directory.mkdir()
    (directory / 'bin').mkdir()
    (directory / 'scenes').mkdir()
    scope = dict(SCOPE, schema=SCHEMA, status='running', head=args.expected_head,
                 mode='downloaded-artifact-replay' if args.replay else 'producer', compile_checks_performed=not bool(args.replay),
                 compile_checks_replayed=False, source_directory=str(ROOT), evidence_directory=str(directory), python_executable=sys.executable)
    g.write_json(directory / 'scope.json', scope)
    source = None
    try:
        expected = inventory()
        source = g.source_fence(None if args.replay else args.expected_head, directory, 'before')
        g.write_json(directory / 'source.json', source)
        g.copy_file(ROOT / INVENTORY, directory / 'inventory.json')
        env = execution_environment(directory, args.expected_head)
        commands = command_contract(ROOT, directory, expected, bool(args.replay), sys.executable)
        binaries, assets = {}, {}
        if args.replay:
            origin = args.replay.resolve()
            g.require(origin != directory and not directory.is_relative_to(origin), 'Replay must be separate')
            binaries, assets = replay_contract(origin, source, args.expected_head, expected)
            for entries, executable in [(binaries, True), (assets, False)]:
                for entry in entries.values():
                    g.copy_file(origin / entry['path'], directory / entry['path'], executable)
            scope.update(producer_scope_sha256=g.sha(origin / 'scope.json'), producer_manifest_sha256=g.sha(origin / 'artifact-manifest.json'))
        else:
            g.reject_ancestor_config(ROOT)
            for alias, relative in ASSET_PATHS.items():
                assets[alias] = {'path': relative, 'sha256': g.copy_file(ROOT / ASSET_SOURCES[alias], directory / relative)}
        for label, (command, timeout) in commands.items():
            if binaries and all(k in binaries for k in BINARY_PATHS):
                g.verify_bindings(directory, binaries)
                g.verify_bindings(directory, assets)
            output = g.run(command, directory, label, env, timeout)
            if label == 'rustc':
                g.require(re.search(r'^release: 1\.99\.1$', output, re.M), 'Rust 1.99.1 required')
            if label.endswith('-build'):
                role = label.removesuffix('-build')
                relative = BINARY_PATHS[role]
                binaries[role] = {'path': relative, 'sha256': g.copy_file(artifact(output, BUILDS[role]), directory / relative, True)}
            elif label.endswith('-inventory'):
                role = label.removesuffix('-inventory')
                g.validate_listing(output, expected[role] + SUITES[role]['excluded'])
            elif label.endswith('-tests') and label.removesuffix('-tests') in SUITES:
                g.validate_tests(output, expected[label.removesuffix('-tests')])
        validate_roles(binaries, assets)
        validate_asset_sources(source, assets)
        g.verify_bindings(directory, binaries)
        g.verify_bindings(directory, assets)
        scope['command_labels'] = validate_commands(directory, expected, bool(args.replay), str(ROOT), str(directory), sys.executable)
        if args.replay:
            g.verify_seal(origin)
        else:
            g.reject_ancestor_config(ROOT)
        g.require(g.source_fence(None if args.replay else args.expected_head, directory, 'after') == source, 'Source changed during gate')
        scope.update(status='passed', tree=source['tree'], configurations=BUILDS, counts={k: len(v) for k, v in expected.items()},
                     binaries=binaries, assets=assets, source_sha256=g.sha(directory / 'source.json'), inventory_sha256=g.sha(directory / 'inventory.json'))
    except Exception as error:
        scope.update(status='failed', error=str(error)[:4096])
        raise
    finally:
        if source is not None and not (directory / 'after-status.txt').exists():
            try:
                scope['source_unchanged_after_failure'] = g.source_fence(None if args.replay else args.expected_head, directory, 'failure-after') == source
            except Exception as error:
                scope['source_after_failure_error'] = str(error)
        g.write_json(directory / 'scope.json', scope)
        g.seal(directory)


class GateTests(unittest.TestCase):
    def test_matrix_and_closed_commands(self):
        expected = {k: ['test_case'] for k in SUITES}
        producer = command_contract(ROOT, Path('/tmp/evidence'), expected, False, sys.executable)
        replay = command_contract(ROOT, Path('/tmp/evidence'), expected, True, sys.executable)
        self.assertEqual(len(SUITES), 15)
        self.assertEqual(len(BUILDS), 20)
        self.assertEqual(len(STRICT) + len(NATIVE), 8)
        self.assertEqual(len(ASSET_PATHS), 5)
        self.assertEqual(set(ASSET_PATHS), set(ASSET_SOURCES))
        self.assertEqual(len(producer), 72)
        self.assertEqual(len(replay), 41)
        build_roles = [label.removesuffix('-build') for label in producer if label.endswith('-build')]
        self.assertEqual(set(build_roles), set(BUILDS))
        self.assertEqual([BUILDS[role]['features'] for role in build_roles],
                         sorted(spec['features'] for spec in BUILDS.values()))
        self.assertFalse(any('--skip' in argv for argv, _ in producer.values()))
        self.assertFalse(any('cargo' == argv[0] for argv, _ in replay.values()))
        for role in NATIVE:
            self.assertNotIn(role + '-tests', producer)
            self.assertNotIn(role + '-inventory', producer)
            self.assertNotIn(role + '-inventory', replay)
            self.assertNotIn(role + '-tests', replay)
            self.assertIn('--no-run', producer[role + '-build'][0])

    def test_inventory_fail_closed(self):
        good = {'schema': SCHEMA, 'reviewed': True, 'suites': {k: ['case'] for k in SUITES},
                'python_tests': ['test_static_game_native_gate.NativeGateTests.test_case'], 'reports': report_expectations(),
                'identities': g.read_json(ROOT / INVENTORY)['identities'],
                'excluded': {k: v['excluded'] for k, v in SUITES.items()}}
        inventory(good)
        for mutation in [lambda v: v.update(reviewed=False), lambda v: v['suites'].update(core=[]),
                         lambda v: v['suites'].update(core=['case', 'case']), lambda v: v['suites'].pop('core'),
                         lambda v: v['suites'].update(core=h.GAMEPLAY_SOCKET_TESTS), lambda v: v.update(python_tests=[])]:
            value = json.loads(json.dumps(good))
            mutation(value)
            with self.assertRaises(ValueError):
                inventory(value)

    def test_compiler_contract_all_roles(self):
        for role, spec in BUILDS.items():
            target = spec['target']
            kind = 'lib' if target == ['--lib'] else 'test' if target[0] == '--test' else 'bin'
            name = PACKAGE if kind == 'lib' else target[1]
            source = STARTER + ('/src/lib.rs' if kind == 'lib' else '/tests/' + name + '.rs' if kind == 'test' else '/src/bin/' + name + '.rs')
            value = {'reason': 'compiler-artifact', 'target': {'name': name, 'kind': [kind], 'src_path': str(ROOT / source)},
                     'manifest_path': str(ROOT / STARTER / 'Cargo.toml'), 'package_id': 'path+' + (ROOT / STARTER).as_uri() + '#orr_static_starter@0.0.1',
                     'profile': {'test': kind != 'bin'}, 'features': spec['compiled_features'], 'executable': '/tmp/binary'}
            with self.subTest(role=role):
                self.assertEqual(artifact(json.dumps(value), spec), Path('/tmp/binary'))
                for key, bad in [('manifest_path', '/wrong'), ('package_id', 'wrong'), ('features', ['unknown']), ('executable', 'relative'), ('profile', {'test': kind == 'bin'})]:
                    with self.assertRaises(ValueError):
                        artifact(json.dumps(dict(value, **{key: bad})), spec)
                with self.assertRaises(ValueError):
                    artifact(json.dumps(value) + '\n' + json.dumps(value), spec)

    def test_same_name_bin_does_not_shadow_integration_test_producer(self):
        spec = BUILDS['bundle-contract']
        test = {'reason': 'compiler-artifact',
                'target': {'name': 'bundle', 'kind': ['test'],
                           'src_path': str(ROOT / STARTER / 'tests/bundle.rs')},
                'manifest_path': str(ROOT / STARTER / 'Cargo.toml'),
                'package_id': 'path+' + (ROOT / STARTER).as_uri() + '#orr_static_starter@0.0.1',
                'profile': {'test': True}, 'features': ['bundle'], 'executable': '/tmp/bundle-test'}
        binary = dict(test, target={'name': 'bundle', 'kind': ['bin'],
                                   'src_path': str(ROOT / STARTER / 'src/bin/bundle.rs')},
                      profile={'test': False}, executable='/tmp/bundle-bin')
        for stream in [[binary, test], [test, binary]]:
            self.assertEqual(artifact('\n'.join(json.dumps(v) for v in stream), spec), Path('/tmp/bundle-test'))
        wrong_source = dict(test, target=dict(test['target'], src_path='/wrong/tests/bundle.rs'))
        for stream in [[binary], [], [binary, wrong_source], [binary, test, test]]:
            with self.assertRaises(ValueError):
                artifact('\n'.join(json.dumps(v) for v in stream), spec)

    def test_replay_rejects_missing_commands_failed_commands_and_changed_artifacts(self):
        with tempfile.TemporaryDirectory() as temp:
            origin = Path(temp)
            (origin / 'bin').mkdir()
            (origin / 'scenes').mkdir()
            head = 'a' * 40
            expected = {k: ['cpu_case'] for k in SUITES}
            document = {'schema': SCHEMA, 'reviewed': True, 'suites': expected,
                        'python_tests': ['test_static_game_native_gate.NativeGateTests.test_case'], 'reports': report_expectations(),
                'identities': g.read_json(ROOT / INVENTORY)['identities'],
                'excluded': {k: v['excluded'] for k, v in SUITES.items()}}
            source = {'head': head, 'tree': 'b' * 40, 'files': {}}
            binaries, assets = {}, {}
            for entries, paths in [(binaries, BINARY_PATHS), (assets, ASSET_PATHS)]:
                for role, relative in paths.items():
                    (origin / relative).write_text(role)
                    entries[role] = {'path': relative, 'sha256': g.sha(origin / relative)}
                    if entries is assets:
                        source['files'][ASSET_SOURCES[role]] = entries[role]['sha256']
            g.write_json(origin / 'source.json', source)
            g.write_json(origin / 'inventory.json', document)
            scope = dict(SCOPE, schema=SCHEMA, status='passed', mode='producer', head=head, tree=source['tree'],
                         compile_checks_performed=True, compile_checks_replayed=False, configurations=BUILDS,
                         counts={k: len(v) for k, v in expected.items()}, binaries=binaries, assets=assets,
                         source_sha256=g.sha(origin / 'source.json'), inventory_sha256=g.sha(origin / 'inventory.json'),
                         source_directory=str(ROOT), evidence_directory=str(origin), python_executable=sys.executable)
            commands = command_contract(ROOT, origin, expected, False, sys.executable)
            scope['command_labels'] = sorted(commands)
            for label, (argv, timeout) in commands.items():
                g.write_json(origin / (label + '.command.json'), dict(argv=argv, cwd=str(ROOT), timeout_seconds=timeout,
                             expected_exit=0, exit_code=0, status='passed'))
                log = '+ ' + ' '.join(argv) + '\n'
                if label.endswith('-inventory'):
                    role = label.removesuffix('-inventory')
                    names = expected[role] + SUITES[role]['excluded']
                    log += ''.join(n + ': test\n' for n in names) + f'\n{len(names)} tests, 0 benchmarks\n'
                elif label.endswith('-tests') and label.removesuffix('-tests') in SUITES:
                    log += 'running 1 test\ntest cpu_case ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
                elif label.endswith('-provenance'):
                    log += json.dumps({'source_sha': head}) + '\n'
                elif label.endswith('-identity'):
                    alias = label.removesuffix('-identity')
                    log += json.dumps(document['identities'][alias]) + '\n'
                elif label.endswith('-reachability'):
                    log += json.dumps(report_expectations()[label.removesuffix('-reachability')]) + '\n'
                elif label == 'python-oracle-tests':
                    log += 'SOURCE_PYTHON_INVENTORY=' + json.dumps(document['python_tests'], separators=(',', ':')) + '\nRan 1 tests in 0.01s\n\nOK\n'
                (origin / (label + '.log')).write_text(log + '\nEXIT_CODE=0\n')
            g.write_json(origin / 'scope.json', scope)
            original_read = g.read_json
            def read(path):
                return document if path == ROOT / INVENTORY else original_read(path)
            with patch.object(g, 'read_json', side_effect=read):
                g.seal(origin)
                replay_contract(origin, source, head, expected)
                # Re-sealing cannot turn tampered reports, scope or source binding into proof.
                for case in QUERY_CASES:
                    log_path = origin / (case + '-reachability.log')
                    original_log = log_path.read_text()
                    for changed in [dict(report_expectations()[case], supported=1),
                                    dict(report_expectations()[case], scope='live_frame'),
                                    dict(report_expectations()[case], current_to_goal='true')]:
                        log_path.write_text(original_log.replace(json.dumps(report_expectations()[case]), json.dumps(changed)))
                        g.seal(origin)
                        with self.assertRaises(ValueError):
                            replay_contract(origin, source, head, expected)
                    log_path.write_text(original_log)
                for change in [dict(cpu_in_process=True), dict(window=True),
                               dict(reachability_scope='all_game_states'),
                               dict(subprocess_scope='arbitrary executables')]:
                    g.write_json(origin / 'scope.json', dict(scope, **change))
                    g.seal(origin)
                    with self.assertRaises(ValueError):
                        replay_contract(origin, source, head, expected)
                g.write_json(origin / 'scope.json', scope)
                source_path = origin / 'source.json'
                changed_source = json.loads(json.dumps(source))
                changed_source['files'][ASSET_SOURCES['delivery-current-unreachable']] = 'f' * 64
                g.write_json(source_path, changed_source)
                changed_scope = dict(scope, source_sha256=g.sha(source_path))
                g.write_json(origin / 'scope.json', changed_scope)
                g.seal(origin)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
                g.write_json(source_path, source)
                g.write_json(origin / 'scope.json', scope)
                query_command = origin / 'delivery-stock-reachability.command.json'
                original_query = original_read(query_command)
                changed_query = dict(original_query, argv=[str(origin / BINARY_PATHS['delivery-player']),
                                                          '--identity', '--scene', str(origin / ASSET_PATHS['delivery'])])
                g.write_json(query_command, changed_query)
                g.seal(origin)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
                g.write_json(query_command, original_query)
                query_log = origin / 'switchboard-unsupported-reachability.log'
                saved_log = query_log.read_text()
                query_log.unlink()
                g.seal(origin)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
                query_log.write_text(saved_log)
                g.seal(origin)
                command_path = origin / 'feature-off-check.command.json'
                good = original_read(command_path)
                for change in [dict(argv=['cargo', 'check']), dict(exit_code=1), dict(status='running'), dict(timeout_seconds=1)]:
                    g.write_json(command_path, dict(good, **change))
                    g.seal(origin)
                    with self.assertRaises(ValueError):
                        replay_contract(origin, source, head, expected)
                g.write_json(command_path, good)
                command_path.unlink()
                g.seal(origin)
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)
                g.write_json(command_path, good)
                g.seal(origin)
                (origin / BINARY_PATHS['core']).write_text('changed')
                with self.assertRaises(ValueError):
                    replay_contract(origin, source, head, expected)

    def test_reports_reject_partial_extra_wrong_game_boolean_and_warning_contracts(self):
        for case, original in report_expectations().items():
            validate_report(case, original)
            for mutate in [lambda v: v.pop('warnings'), lambda v: v.update(extra='unreviewed'),
                           lambda v: v.update(supported=int(v['supported'])),
                           lambda v: v.update(game='OtherGame'),
                           lambda v: v.update(schema='orr.delivery-reachability/2'),
                           lambda v: v.update(scope='current_frame'),
                           lambda v: v.update(analysis='bounded_path_search'),
                           lambda v: v.update(warnings=['success']),
                           lambda v: v.update(current_to_goal='false'),
                           lambda v: v.update(origin_to_goal='true')]:
                value = json.loads(json.dumps(original))
                mutate(value)
                with self.subTest(case=case, value=value), self.assertRaises(ValueError):
                    validate_report(case, value)
        unsupported = report_expectations()['switchboard-unsupported']
        self.assertIs(unsupported['supported'], False)
        self.assertIsNone(unsupported['current_to_goal'])
        self.assertIsNone(unsupported['origin_to_goal'])
        self.assertEqual(unsupported['warnings'], [])
        with self.assertRaises(ValueError):
            validate_report('unknown', unsupported)

    def test_custom_scene_source_bindings_reject_mixed_missing_and_wrong_assets(self):
        source = {'files': {path: str(i) * 64 for i, path in enumerate(ASSET_SOURCES.values())}}
        assets = {role: {'path': ASSET_PATHS[role], 'sha256': source['files'][path]}
                  for role, path in ASSET_SOURCES.items()}
        validate_asset_sources(source, assets)
        mixed = json.loads(json.dumps(assets))
        a, b = 'delivery-current-unreachable', 'delivery-restart-unreachable'
        mixed[a]['sha256'], mixed[b]['sha256'] = mixed[b]['sha256'], mixed[a]['sha256']
        with self.assertRaises(ValueError):
            validate_asset_sources(source, mixed)
        for role in ASSET_SOURCES:
            changed = json.loads(json.dumps(assets))
            changed.pop(role)
            with self.assertRaises(ValueError):
                validate_asset_sources(source, changed)
            changed = json.loads(json.dumps(assets))
            changed[role]['sha256'] = 'f' * 64
            with self.assertRaises(ValueError):
                validate_asset_sources(source, changed)

    def test_query_commands_are_separate_read_only_copied_artifact_queries(self):
        directory = Path('/tmp/copied evidence')
        commands = command_contract(ROOT, directory, {k: ['case'] for k in SUITES}, True, sys.executable)
        for case, (alias, asset, _, _) in QUERY_CASES.items():
            argv, timeout = commands[case + '-reachability']
            self.assertEqual(argv, [str(directory / BINARY_PATHS[alias + '-player']), '--reachability',
                                    '--scene', str(directory / ASSET_PATHS[asset])])
            self.assertEqual(timeout, 30)
            self.assertNotIn('--reachability', commands[alias + '-identity'][0])
            self.assertNotIn('--reachability', commands[alias + '-provenance'][0])
        with tempfile.TemporaryDirectory() as temporary:
            with patch.dict(os.environ, {'DISPLAY': ':99', 'WAYLAND_DISPLAY': 'fake-display'}):
                env = execution_environment(Path(temporary), 'a' * 40)
            self.assertNotIn('DISPLAY', env)
            self.assertNotIn('WAYLAND_DISPLAY', env)

    def test_identity_rejects_checksum_schema_input_domain_and_partial_contract(self):
        identities = g.read_json(ROOT / INVENTORY)['identities']
        for alias in g.GAMES:
            validate_identity(alias, identities[alias], identities)
            for mutation in [lambda v: v.update(initial_checksum='0x0000000000000000'),
                             lambda v: v.update(schema={}),
                             lambda v: v['schema'].update(additionalProperties=True),
                             lambda v: v.update(input_contract={'max_players': 1}),
                             lambda v: v['input_contract'].update(value_format='changed'),
                             lambda v: v['input_contract']['schema']['properties'].clear(),
                             lambda v: v['input_contract']['schema']['properties'].update(unreviewed={'type': 'number'}),
                             lambda v: v.update(player_count=True)]:
                value = json.loads(json.dumps(identities[alias]))
                mutation(value)
                with self.assertRaises(ValueError):
                    validate_identity(alias, value, identities)

    def test_headless_player_override_is_copied_artifact(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            with patch.dict(os.environ, {'ORR_STATIC_STARTER_PLAYER_TEST_BIN': '/untrusted/inherited/player'}):
                env = execution_environment(directory, 'a' * 40)
            self.assertEqual(env['ORR_STATIC_STARTER_PLAYER_TEST_BIN'], str(directory / 'bin/player'))
            self.assertEqual(BUILDS['player']['features'], '')
            self.assertEqual(BUILDS['player']['compiled_features'], [])
            self.assertEqual(BUILDS['player']['target'], ['--bin', 'player'])

    def test_source_loader_ignores_stale_cache(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'helper.py'
            path.write_text("value = 'stale'\n")
            stamp = path.stat().st_mtime_ns
            g.py_compile.compile(str(path), doraise=True, invalidation_mode=g.py_compile.PycInvalidationMode.TIMESTAMP)
            path.write_text("value = 'fresh'\n")
            os.utime(path, ns=(stamp, stamp))
            self.assertEqual(source_module(path, 'objective_loader_fixture').value, 'fresh')


if __name__ == '__main__':
    try:
        if '--self-test' in sys.argv:
            result = unittest.TextTestRunner(verbosity=2).run(unittest.TestSuite([
                unittest.defaultTestLoader.loadTestsFromTestCase(g.ValidatorTests),
                unittest.defaultTestLoader.loadTestsFromTestCase(GateTests)]))
            sys.exit(not result.wasSuccessful())
        if '--python-tests' in sys.argv:
            python_tests()
        else:
            parser = argparse.ArgumentParser(description=__doc__)
            parser.add_argument('--expected-head', default=os.environ.get('GITHUB_SHA'))
            parser.add_argument('--replay', type=Path)
            main(parser.parse_args())
    except Exception as error:
        print('Delivery reachability gate FAILED: ' + str(error), file=sys.stderr)
        sys.exit(1)
