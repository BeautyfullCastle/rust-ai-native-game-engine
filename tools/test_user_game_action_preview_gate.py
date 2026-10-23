"""Synthetic CPU-gate contracts; these tests do not compile or execute Rust."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import user_game_action_preview_gate as gate


def metadata(project, engine, target, features):
    package = {'name': gate.PACKAGE, 'id': 'path+file://project#'+gate.PACKAGE+'@0.1.0',
               'manifest_path': str(project/'Cargo.toml'), 'source': None,
               'targets': [{'name': name, 'kind': [kind], 'src_path': str(project/path)}
                           for name, (path, kind) in gate.TARGETS.items()]}
    packages = [package, {'name': 'orr_input', 'id': 'input', 'source': None,
                         'manifest_path': str(engine/'crates/orr_input/Cargo.toml')}]
    nodes = [{'id': package['id'], 'features': features.split(',')}, {'id': 'input', 'features': []}]
    if 'editor' in features.split(','):
        packages.append({'name': 'orr_editor', 'id': 'editor', 'source': None,
                         'manifest_path': str(engine/'crates/orr_editor/Cargo.toml')})
        nodes.append({'id': 'editor', 'features': ['static-game-actions']})
    return {'packages': packages, 'workspace_root': str(project), 'workspace_members': [package['id']],
            'target_directory': str(target), 'resolve': {'root': package['id'], 'nodes': nodes}}


def creation(engine):
    return {'schema': 1, 'template_profile': gate.PROFILE, 'engine_commit': gate.bootstrap.ACTION_PREVIEW_COMMIT,
            'engine_tree': gate.bootstrap.ACTION_PREVIEW_TREE, 'engine_path': str(engine),
            'project_name': gate.PACKAGE, 'stability': 'preview', 'hosted_native_acceptance': 'unverified',
            'action_feature_default': False, 'files_sha256': {}}


def result(names):
    return '\n'.join('test '+name+' ... ok' for name in sorted(names))+f'\ntest result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'


class ContextTests(unittest.TestCase):
    def setUp(self):
        self.project, self.engine, self.target, self.host = map(Path, ('/project with spaces', '/engine', '/target', '/gate'))

    def check(self, value, features='standalone'):
        return gate.metadata_context(value, self.project, self.engine, self.target, 'b'*40, self.host, 'a'*40, features)

    def test_all_mode_contexts(self):
        for _, features, _, _ in gate.MODES:
            self.check(metadata(self.project, self.engine, self.target, features), features)

    def test_wrong_package_workspace_and_target(self):
        original = metadata(self.project, self.engine, self.target, 'standalone')
        mutations = [
            lambda m: m['packages'].append(copy.deepcopy(m['packages'][0])),
            lambda m: m['packages'][0].update(manifest_path='/wrong/Cargo.toml'),
            lambda m: m.update(workspace_root='/engine'),
            lambda m: m['workspace_members'].append('extra'),
            lambda m: m.update(target_directory='/shared-target'),
            lambda m: m['resolve'].update(root='input'),
            lambda m: m['resolve']['nodes'].append(copy.deepcopy(m['resolve']['nodes'][0])),
            lambda m: m['packages'][0]['targets'][0].update(src_path='/engine/lib.rs'),
            lambda m: m['packages'][0]['targets'][0].update(kind=['test']),
            lambda m: m['packages'][0]['targets'].append(copy.deepcopy(m['packages'][0]['targets'][0])),
            lambda m: m['packages'][1].update(manifest_path='/outside/Cargo.toml'),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                value = copy.deepcopy(original); mutate(value)
                with self.assertRaises(ValueError):
                    self.check(value)

    def test_feature_and_editor_dependency_isolation(self):
        mutations = [
            lambda m: m['resolve']['nodes'][0]['features'].append('static-game-actions'),
            lambda m: m['resolve']['nodes'][0]['features'].append('editor'),
            lambda m: m['resolve']['nodes'][0]['features'].append('default'),
            lambda m: m['resolve']['nodes'][0]['features'].remove('standalone'),
            lambda m: m['packages'][1].update(name='orr_editor'),
        ]
        for mutate in mutations:
            value = metadata(self.project, self.engine, self.target, 'standalone'); mutate(value)
            with self.assertRaises(ValueError):
                self.check(value)
        value = metadata(self.project, self.engine, self.target, 'editor,standalone,static-game-actions')
        value['resolve']['nodes'] = [n for n in value['resolve']['nodes'] if n['id'] != 'editor']
        with self.assertRaisesRegex(ValueError, 'editor dependency'):
            self.check(value, 'editor,standalone,static-game-actions')

    def test_creation_identity_and_no_native_claim(self):
        good = creation(self.engine)
        gate.validate_creation_receipt(good, self.engine)
        for field, bad in [('schema', 2), ('template_profile', 'legacy'), ('engine_commit', '0'*40),
                           ('engine_tree', '0'*40), ('engine_path', '/wrong'), ('project_name', 'orr_static_starter'),
                           ('stability', 'stable'), ('hosted_native_acceptance', 'passed'), ('action_feature_default', True)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                gate.validate_creation_receipt({**good, field: bad}, self.engine)

    def test_exact_executed_test_names_and_counts(self):
        for _, _, _, names in gate.MODES:
            good = result(names)
            gate.test_result(good, names)
            for bad in (good.replace('0 ignored', '1 ignored'), good.replace('0 filtered out', '1 filtered out'),
                        good.replace('test '+sorted(names)[0], 'test wrong'), good+good,
                        good.replace(' ... ok', ' ... ignored', 1), 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'):
                with self.subTest(bad=bad), self.assertRaises(ValueError):
                    gate.test_result(bad, names)


class CompilerModeTests(unittest.TestCase):
    def test_wrong_features_and_library_profiles_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'lib.rs'; path.write_text('source')
            from types import SimpleNamespace
            context = SimpleNamespace(package_id='own', library_artifact=lambda output: {'files': []})
            artifact = {'reason': 'compiler-artifact', 'package_id': 'own',
                        'target': {'kind': ['lib'], 'src_path': str(path)},
                        'features': ['standalone'], 'profile': {'opt_level': '3', 'test': False}}
            result = gate.mode_library(json.dumps(artifact), context, 'standalone')
            self.assertEqual(result['source_sha256'], gate.source.digest(path))
            mutations = [lambda a: a.update(features=[]), lambda a: a.update(features=['editor']),
                         lambda a: a.update(features=['standalone', 'standalone']),
                         lambda a: a['profile'].update(opt_level='0'),
                         lambda a: a['profile'].update(test=True), lambda a: a.update(profile={})]
            for mutate in mutations:
                changed = copy.deepcopy(artifact); mutate(changed)
                with self.assertRaises(Exception):
                    gate.mode_library(json.dumps(changed), context, 'standalone')


class OrchestrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.host, self.engine, self.project, self.target, self.artifacts = [self.root/n for n in ('gate', 'engine', 'project with spaces', 'target', 'artifacts')]
        (self.host/'tools').mkdir(parents=True); self.engine.mkdir()
        (self.host/'tools/user_game_action_preview_source.json').write_text('{}')
        self.env = {'GITHUB_SHA': 'a'*40, 'GITHUB_RUN_ID': '7', 'GITHUB_RUN_ATTEMPT': '1',
                    'PATH': os.environ.get('PATH', ''), 'CARGO_HOME': str(self.root/'cargo-home')}
        self.calls = []
        self.fail_mode = None
        self.mutate_user = False
        self.source_changed = False

    def bootstrap(self, engine, project, name, inventory, *, profile):
        self.assertEqual(profile, gate.PROFILE); self.assertEqual(name, gate.PACKAGE)
        project.mkdir()
        for relative, _ in gate.TARGETS.values():
            path = project/relative; path.parent.mkdir(parents=True, exist_ok=True); path.write_text('// source\n')
        (project/'Cargo.toml').write_text('[workspace]\n')
        (project/'provenance.json').write_text(json.dumps(creation(engine)))
        return {'cargo_ran': False}

    def checkout(self, sha, path, *, complete):
        if path == self.project and self.source_changed:
            raise ValueError('user source changed')
        return {'commit': sha, 'tree': 'c'*40, 'files': []}

    def execute(self, args, artifacts, name, env, seconds, cwd):
        self.calls.append((args, name, env.copy(), cwd))
        self.assertEqual(cwd, self.project)
        if name == self.fail_mode:
            (artifacts/(name+'.log')).write_text('original compiler failure\n')
            raise ValueError(name+' failed')
        if name == 'own-lock':
            (self.project/'Cargo.lock').write_text('# own lock\n'); output = ''
        elif name == 'own-lock-fetch':
            self.assertEqual(args, ['cargo', 'fetch', '--locked']); output = ''
        elif name.endswith('-metadata'):
            output = json.dumps(metadata(self.project, self.engine, self.target, args[args.index('--features')+1]))
        elif name.endswith('-compile'):
            features = args[args.index('--features')+1]; test = args[args.index('--test')+1]
            self.target.mkdir(exist_ok=True)
            names = {test:'test', 'player':'bin', 'delivery-player':'bin', 'switchboard-player':'bin', 'orr_static_starter':'lib'}
            if 'editor' in features.split(','): names['editor'] = 'bin'
            items = []
            for target_name, kind in names.items():
                path = self.target/(name+'-'+target_name); path.write_text('compiled bytes')
                items.append({'reason':'compiler-artifact', 'package_id':'path+file://project#'+gate.PACKAGE+'@0.1.0',
                              'target':{'name':target_name, 'kind':[kind], 'src_path':str(self.project/gate.TARGETS[target_name][0])},
                              'features':features.split(','), 'profile':{'opt_level':'3','test':kind=='test'},
                              'filenames':[str(path)], 'executable':str(path) if kind!='lib' else None})
            output = '\n'.join(json.dumps(v) for v in items)
        else:
            mode = next(m for m in gate.MODES if name == m[0]+'-inventory' or name == m[0]+'-tests')
            names = mode[3]
            if name.endswith('-inventory'):
                output = '\n'.join(n+': test' for n in sorted(names))+f'\n{len(names)} tests, 0 benchmarks\n'
            else:
                output = result(names)
                if self.mutate_user: self.source_changed = True
        (artifacts/(name+'.log')).write_text(output)
        return output

    def run_gate(self):
        with patch.dict(os.environ, self.env, clear=True), \
             patch.object(gate.bootstrap, 'validate_engine') as validate, \
             patch.object(gate.bootstrap, 'bootstrap', self.bootstrap), \
             patch.object(gate.source, 'clean_checkout', self.checkout), \
             patch.object(gate.shared, 'own_commit', return_value='b'*40), \
             patch.object(gate.command_gate, 'command', self.execute):
            gate.run(self.host, self.engine, self.project, self.target, self.artifacts)
            self.assertTrue(all(call.kwargs['profile'] == gate.PROFILE for call in validate.call_args_list))

    def test_all_modes_bind_user_source_not_engine_and_keep_native_false(self):
        self.run_gate()
        receipt = json.loads((self.artifacts/'gate.json').read_text())
        self.assertEqual(receipt['status'], 'passed'); self.assertTrue(receipt['portable_cpu_acceptance'])
        self.assertFalse(receipt['native_acceptance'])
        self.assertEqual([receipt['modes'][m]['tests'] for m, *_ in gate.MODES], [10,3,4])
        for args, name, env, cwd in self.calls:
            if name.endswith('-compile'):
                self.assertIn('--offline', args); self.assertIn('--locked', args); self.assertIn('--no-default-features', args)
                self.assertEqual(env['ORR_STATIC_STANDALONE_BUILD_SHA'], 'b'*40)
                self.assertNotEqual(env['ORR_STATIC_STANDALONE_BUILD_SHA'], env['GITHUB_SHA'])

    def test_lock_fetch_failure_retains_logs_and_prevents_compilation(self):
        self.fail_mode = 'own-lock-fetch'
        with self.assertRaisesRegex(ValueError, 'own-lock-fetch failed'):
            self.run_gate()
        receipt = json.loads((self.artifacts/'gate.json').read_text())
        self.assertEqual(receipt['status'], 'failed')
        self.assertFalse(receipt['native_acceptance'])
        self.assertNotIn('user_source_commit', receipt)
        self.assertNotIn('portable_cpu_acceptance', receipt)
        self.assertEqual([name for _, name, _, _ in self.calls], ['own-lock', 'own-lock-fetch'])
        self.assertEqual((self.artifacts/'own-lock-fetch.log').read_text(), 'original compiler failure\n')

    def test_compiler_failure_preserves_partial_evidence(self):
        self.fail_mode = 'actions-compile'
        with self.assertRaisesRegex(ValueError, 'actions-compile failed'): self.run_gate()
        receipt = json.loads((self.artifacts/'gate.json').read_text())
        self.assertEqual(receipt['status'], 'failed'); self.assertFalse(receipt['native_acceptance'])
        self.assertEqual(receipt['modes']['feature-off']['status'], 'passed')
        self.assertEqual((self.artifacts/'actions-compile.log').read_text(), 'original compiler failure\n')
        self.assertNotIn('portable_cpu_acceptance', receipt)

    def test_user_mutation_prevents_success(self):
        self.mutate_user = True
        with self.assertRaisesRegex(ValueError, 'user source changed'): self.run_gate()
        receipt = json.loads((self.artifacts/'gate.json').read_text())
        self.assertEqual(receipt['status'], 'failed'); self.assertFalse(receipt['native_acceptance'])

    def test_ambient_loader_is_not_removed_or_allowed(self):
        self.env['LD_LIBRARY_PATH'] = '/injected'
        with self.assertRaisesRegex(ValueError, 'ambient compiler configuration'): self.run_gate()
        self.assertFalse(self.project.exists())
        self.assertFalse(json.loads((self.artifacts/'gate.json').read_text())['native_acceptance'])

    def test_inherited_source_identity_rejected(self):
        self.env['ORR_STATIC_STANDALONE_BUILD_SHA'] = 'd'*40
        with self.assertRaisesRegex(ValueError, 'inherited build/context'): self.run_gate()
        self.assertFalse(self.project.exists())


if __name__ == '__main__':
    unittest.main()
