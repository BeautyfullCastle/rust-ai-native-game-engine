"""Explicit preview selection and source-only generation; no native claims."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import bootstrap_user_game as boot
import test_bootstrap_user_game as legacy

PROFILE = 'multi-action-preview'


class PreviewSafetyTests(legacy.BootstrapTests):
    # Repeat every existing filesystem/atomic-publication attack against preview.
    def setUp(self):
        super().setUp()
        path = boot.STARTER + 'Cargo.toml'
        self.sources[path] = self.sources[path].replace(
            b'[features]\n', b'[features]\nstatic-game-actions = ["dep:orr_input", "orr_editor?/static-game-actions"]\n')
        self.sources[boot.STARTER + 'tests/multi_action_contract.rs'] = b'// portable action fixture\n'
        for name in boot.ACTION_NATIVE_OMITTED:
            self.sources[boot.STARTER + name] = b'// engine-only native harness\n'
        for name, data in self.sources.items():
            output = self.engine / name
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(data)
        self.inventory = {'schema': 1, 'commit': boot.ACTION_PREVIEW_COMMIT,
                          'files': [{'path': p, 'mode': '100644', 'blob': boot.git_hash('blob', b)}
                                    for p, b in self.sources.items()]}
        self.inventory['tree'] = boot.inventory_tree(self.inventory['files'])
        pin = patch.object(boot, 'ACTION_PREVIEW_TREE', self.inventory['tree'])
        pin.start()
        self.addCleanup(pin.stop)

    def run_bootstrap(self, **kwargs):
        return boot.bootstrap(kwargs.get('engine', self.engine), kwargs.get('destination', self.destination),
                              kwargs.get('name', 'my-game'), kwargs.get('inventory', self.inventory), profile=PROFILE)

    def test_relocated_owned_project(self):
        # Base test's copy loop is legacy-specific; check preview's explicit omissions.
        result = self.run_bootstrap()
        for path, data in self.sources.items():
            if path.startswith(boot.STARTER):
                relative = path.removeprefix(boot.STARTER)
                if relative in boot.OMITTED | boot.ACTION_NATIVE_OMITTED:
                    self.assertFalse((self.destination / relative).exists())
                elif relative not in ('Cargo.toml', 'README.md'):
                    self.assertEqual((self.destination / relative).read_bytes(), data)
        receipt = json.loads((self.destination / 'provenance.json').read_text())
        self.assertEqual(receipt['engine_commit'], boot.ACTION_PREVIEW_COMMIT)
        self.assertEqual(receipt['engine_tree'], self.inventory['tree'])
        self.assertEqual(receipt['template_profile'], PROFILE)
        self.assertEqual(receipt['hosted_native_acceptance'], 'unverified')
        self.assertFalse(receipt['action_feature_default'])
        for path, digest in receipt['files_sha256'].items():
            self.assertEqual(hashlib.sha256((self.destination / path).read_bytes()).hexdigest(), digest)
        self.assertFalse(result['cargo_ran'])

    def test_no_implicit_profile_switch(self):
        with self.assertRaisesRegex(ValueError, 'pinned reviewed'):
            boot.bootstrap(self.engine, self.destination, 'game', self.inventory)
        self.assert_clean()

    def test_wrong_legacy_commit_rejected(self):
        with self.assertRaisesRegex(ValueError, 'pinned reviewed'):
            self.run_bootstrap(inventory={**self.inventory, 'commit': boot.PINNED_COMMIT})
        self.assert_clean()

    def test_unknown_profile_rejected_before_io(self):
        for value in ('../multi-action-preview', '', 'main', 'latest', 'MULTI-ACTION-PREVIEW', None):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'unknown bootstrap profile'):
                boot.bootstrap(self.engine, self.destination, 'game', self.inventory, profile=value)
        self.assert_clean()


class PreviewContractTests(unittest.TestCase):
    def test_inventory_exact_tree_and_pin(self):
        inv = json.loads(Path(boot.__file__).with_name('user_game_action_preview_source.json').read_text())
        self.assertEqual(inv['commit'], boot.ACTION_PREVIEW_COMMIT)
        self.assertEqual(inv['tree'], boot.ACTION_PREVIEW_TREE)
        self.assertEqual(boot.inventory_tree(inv['files']), boot.ACTION_PREVIEW_TREE)
        self.assertEqual(len(inv['files']), 2053)
        modified = copy.deepcopy(inv['files'])
        modified[0]['blob'] = '0' * 40
        self.assertNotEqual(boot.inventory_tree(modified), boot.ACTION_PREVIEW_TREE)

    def test_real_starter_transformation(self):
        root = Path(boot.__file__).resolve().parents[1] / boot.STARTER
        snapshot = {str(p.relative_to(root)): p.read_bytes() for p in root.rglob('*') if p.is_file()}
        files = boot.generated_files(snapshot, Path('/engine with spaces'), 'my-actions', profile=PROFILE)
        manifest = tomllib.loads(files['Cargo.toml'].decode())
        self.assertEqual(manifest['features']['default'], ['editor'])
        self.assertEqual(manifest['features']['static-game-actions'], ['dep:orr_input', 'orr_editor?/static-game-actions'])
        self.assertFalse(manifest['dependencies']['orr_remote']['default-features'])
        self.assertEqual(manifest['package']['name'], 'my-actions')
        for entry in manifest.get('bin', []) + manifest.get('test', []):
            self.assertIn(entry['path'], files)
            for feature in entry.get('required-features', []):
                self.assertIn(feature, manifest['features'])
        self.assertIn('tests/multi_action_contract.rs', files)
        for name in boot.ACTION_NATIVE_OMITTED | boot.OMITTED:
            self.assertNotIn(name, files)
        for path, data in snapshot.items():
            if path.endswith('.rs') and path not in boot.ACTION_NATIVE_OMITTED | boot.OMITTED:
                self.assertEqual(files[path], data)
        self.assertNotIn('multi-action-editor-acceptance', manifest['features'])
        self.assertNotIn('multi-action-standalone-acceptance', manifest['features'])
        self.assertIn(b'UNVERIFIED', files['README.md'])
        self.assertIn(b'not a third playable game', files['README.md'])

    def test_missing_portable_contract_fails_closed(self):
        root = Path(boot.__file__).resolve().parents[1] / boot.STARTER
        snapshot = {'Cargo.toml': (root / 'Cargo.toml').read_bytes(), 'README.md': b'upstream'}
        with self.assertRaisesRegex(ValueError, 'portable action contract missing'):
            boot.generated_files(snapshot, Path('/engine'), 'game', profile=PROFILE)

    def test_cli_rejects_unknown_profile_without_writing(self):
        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp) / 'game'
            result = subprocess.run([sys.executable, '-B', boot.__file__, '--engine', temp,
                                     '--destination', str(destination), '--name', 'game',
                                     '--profile', '../latest'], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(destination.exists())


if __name__ == '__main__':
    unittest.main()
