"""Offline fixtures; optional exact-tree end-to-end gate via ORR_BOOTSTRAP_ENGINE_SOURCE."""
import errno
import hashlib
import json
import os
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import bootstrap_user_game as boot

MANIFEST = b'''[package]
name = "orr_static_starter"
version.workspace = true
edition.workspace = true
license.workspace = true
[lints]
workspace = true
[dependencies]
orr_editor = { optional = true, path = "../../crates/orr_editor", features = ["static-games"] }
[features]
default = ["editor"]
editor = ["dep:orr_editor"]
standalone-native-acceptance = ["editor"]
[[test]]
name = "standalone_x11"
path = "tests/standalone_x11.rs"
required-features = ["standalone-native-acceptance"]
'''


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.engine = self.root / 'engine with spaces'
        self.engine.mkdir()
        self.destination = self.root / 'my game'
        self.sources = {
            'Cargo.toml': b'[workspace]\nmembers = ["crates/*"]\n',
            'crates/orr_editor/Cargo.toml': b'[package]\nname = "orr_editor"\n',
            boot.STARTER + 'Cargo.toml': MANIFEST,
            boot.STARTER + 'README.md': b'Original repository instructions\n',
            boot.STARTER + 'src/lib.rs': b'pub const SCENE: &str = include_str!("../scenes/delivery.scene.yaml");\n',
            boot.STARTER + 'src/bin/player.rs': b'fn main() {}\n',
            boot.STARTER + 'scenes/delivery.scene.yaml': b'schema: orr.scene/1\n',
            boot.STARTER + 'tests/acceptance.rs': b'// portable\n',
            boot.STARTER + 'tests/standalone_x11.rs': b'// repository-only\n',
            boot.STARTER + 'tests/support/standalone_x11.rs': b'// repository-only support\n',
        }
        entries = []
        for name, data in self.sources.items():
            path = self.engine / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            entries.append({'path': name, 'mode': '100644', 'blob': boot.git_hash('blob', data)})
        self.inventory = {'schema': 1, 'commit': boot.PINNED_COMMIT,
                          'tree': boot.inventory_tree(entries), 'files': entries}
        pin = patch.object(boot, 'PINNED_TREE', self.inventory['tree'])
        pin.start()
        self.addCleanup(pin.stop)

    def run_bootstrap(self, **kwargs):
        return boot.bootstrap(kwargs.get('engine', self.engine), kwargs.get('destination', self.destination),
                              kwargs.get('name', 'my-game'), kwargs.get('inventory', self.inventory))

    def assert_clean(self):
        self.assertFalse(self.destination.exists())
        self.assertEqual(list(self.root.glob('.orr-bootstrap-*')), [])

    def test_relocated_owned_project(self):
        result = self.run_bootstrap()
        manifest = tomllib.loads((self.destination / 'Cargo.toml').read_text())
        self.assertEqual(manifest['package'], {'name': 'my-game', 'version': '0.1.0', 'edition': '2021', 'license': 'MIT OR Apache-2.0'})
        self.assertEqual(manifest['lib']['name'], 'orr_static_starter')
        self.assertEqual(manifest['workspace']['resolver'], '2')
        self.assertEqual(manifest['dependencies']['orr_editor']['path'], str(self.engine / 'crates/orr_editor'))
        self.assertNotIn('standalone-native-acceptance', manifest['features'])
        self.assertNotIn('test', manifest)
        for path, data in self.sources.items():
            relative = path.removeprefix(boot.STARTER)
            if path.startswith(boot.STARTER) and relative not in boot.OMITTED | {'Cargo.toml', 'README.md'}:
                self.assertEqual((self.destination / relative).read_bytes(), data)
        for path in boot.OMITTED:
            self.assertFalse((self.destination / path).exists())
        self.assertFalse((self.destination / 'Cargo.lock').exists())
        receipt = json.loads((self.destination / 'provenance.json').read_text())
        self.assertIsNone(receipt['user_source_commit'])
        for path, digest in receipt['files_sha256'].items():
            self.assertEqual(hashlib.sha256((self.destination / path).read_bytes()).hexdigest(), digest)
        self.assertFalse(result['cargo_ran'])

    def test_existing_directory_not_overwritten(self):
        self.destination.mkdir()
        (self.destination / 'owned').write_text('keep')
        with self.assertRaisesRegex(ValueError, 'already exists'):
            self.run_bootstrap()
        self.assertEqual((self.destination / 'owned').read_text(), 'keep')

    def test_existing_file_not_overwritten(self):
        self.destination.write_text('keep')
        with self.assertRaises(ValueError):
            self.run_bootstrap()
        self.assertEqual(self.destination.read_text(), 'keep')

    def test_broken_destination_symlink_rejected(self):
        self.destination.symlink_to(self.root / 'missing')
        with self.assertRaises(ValueError):
            self.run_bootstrap()
        self.assertTrue(self.destination.is_symlink())

    def test_symlink_engine_root(self):
        alias = self.root / 'alias'
        alias.symlink_to(self.engine, target_is_directory=True)
        with self.assertRaises(OSError):
            self.run_bootstrap(engine=alias)
        self.assert_clean()

    def test_symlink_engine_ancestor(self):
        alias = self.root / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            self.run_bootstrap(engine=alias / self.engine.name)
        self.assert_clean()

    def test_symlink_destination_parent(self):
        alias = self.root / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            self.run_bootstrap(destination=alias / 'output')
        self.assertFalse((self.root / 'output').exists())

    def test_symlink_input_file(self):
        path = self.engine / 'Cargo.toml'
        path.unlink()
        path.symlink_to(self.engine / 'crates/orr_editor/Cargo.toml')
        with self.assertRaises(ValueError):
            self.run_bootstrap()
        self.assert_clean()

    def test_symlink_input_directory(self):
        (self.engine / 'extra').symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.run_bootstrap()
        self.assert_clean()

    def test_engine_dependency_changed(self):
        (self.engine / 'crates/orr_editor/Cargo.toml').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'differs'):
            self.run_bootstrap()
        self.assert_clean()

    def test_mode_changed(self):
        (self.engine / 'Cargo.toml').chmod(0o755)
        with self.assertRaises(ValueError):
            self.run_bootstrap()
        self.assert_clean()

    def test_missing_input(self):
        (self.engine / 'Cargo.toml').unlink()
        with self.assertRaises(FileNotFoundError):
            self.run_bootstrap()
        self.assert_clean()

    def test_untracked_build_script_rejected(self):
        (self.engine / 'crates/orr_editor/build.rs').write_text('malicious')
        with self.assertRaisesRegex(ValueError, 'untracked'):
            self.run_bootstrap()
        self.assert_clean()

    def test_untracked_cargo_configuration_rejected(self):
        (self.engine / '.cargo').mkdir()
        (self.engine / '.cargo/config.toml').write_text('[build]\nrustc-wrapper="bad"')
        with self.assertRaises(ValueError):
            self.run_bootstrap()
        self.assert_clean()

    def test_git_metadata_build_output_ignored(self):
        (self.engine / '.git').write_text('worktree metadata')
        (self.engine / 'target').mkdir()
        (self.engine / 'target/file').write_text('build output')
        self.run_bootstrap()

    def test_destination_inside_engine(self):
        with self.assertRaisesRegex(ValueError, 'outside'):
            self.run_bootstrap(destination=self.engine / 'new-project')
        self.assertFalse((self.engine / 'new-project').exists())

    def test_traversal_rejected(self):
        with self.assertRaisesRegex(ValueError, 'traversal'):
            self.run_bootstrap(destination=self.root / 'unused/../output')
        self.assert_clean()

    def test_invalid_name_rejected(self):
        for name in ('../bad', 'hello\nworld', 'UPPER', 'test', '0project', 'a' * 65):
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.run_bootstrap(name=name)
        self.assert_clean()

    def test_bad_inventory_refused(self):
        for key, value in [('tree', '0'*40), ('commit', '0'*40), ('schema', 2)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.run_bootstrap(inventory={**self.inventory, key: value})
        self.assert_clean()

    def test_inventory_path_attacks(self):
        item = self.inventory['files'][0]
        for path in ('../escape', '/escape', 'a//b', 'a/./b', '.git/config', 'a\\b'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                boot.inventory_tree([{**item, 'path': path}])
        with self.assertRaises(ValueError):
            boot.inventory_tree([item, item])

    def test_destination_race_no_replace(self):
        real = boot.rename_noreplace
        def race(fd, source, destination):
            os.mkdir(destination, dir_fd=fd)
            real(fd, source, destination)
        with patch.object(boot, 'rename_noreplace', race), self.assertRaises(FileExistsError):
            self.run_bootstrap()
        self.assertTrue(self.destination.is_dir())
        self.assertEqual(list(self.destination.iterdir()), [])
        self.assertEqual(list(self.root.glob('.orr-bootstrap-*')), [])

    def test_unsupported_atomic_publish_cleans_stage(self):
        with patch.object(boot, 'rename_noreplace', side_effect=OSError(errno.ENOSYS, 'unsupported')), self.assertRaises(OSError):
            self.run_bootstrap()
        self.assert_clean()

    def test_partial_write_cleans_stage(self):
        real_open = boot.os.open
        def fail_output(path, flags, *args, **kwargs):
            if flags & os.O_WRONLY:
                raise OSError('write failed')
            return real_open(path, flags, *args, **kwargs)
        with patch.object(boot.os, 'open', fail_output), self.assertRaises(OSError):
            self.run_bootstrap()
        self.assert_clean()

    def test_nonlinux_fails_closed(self):
        with patch.object(boot.sys, 'platform', 'darwin'), self.assertRaisesRegex(ValueError, 'Linux'):
            self.run_bootstrap()
        self.assert_clean()


class PinnedInventoryTests(unittest.TestCase):
    def test_bundled_tree_identity(self):
        inventory = json.loads(Path(boot.__file__).with_name('user_game_bootstrap_source.json').read_text())
        self.assertEqual(len(inventory['files']), 2026)
        self.assertEqual(boot.inventory_tree(inventory['files']), boot.PINNED_TREE)
        self.assertEqual(inventory['commit'], boot.PINNED_COMMIT)

    @unittest.skipUnless(os.environ.get('ORR_BOOTSTRAP_ENGINE_SOURCE'), 'set ORR_BOOTSTRAP_ENGINE_SOURCE for exact-tree gate')
    def test_exact_engine_materialization(self):
        inventory = json.loads(Path(boot.__file__).with_name('user_game_bootstrap_source.json').read_text())
        engine = Path(os.environ['ORR_BOOTSTRAP_ENGINE_SOURCE'])
        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp) / 'independent game'
            boot.bootstrap(engine, destination, 'independent-game', inventory)
            manifest = tomllib.loads((destination / 'Cargo.toml').read_text())
            self.assertEqual(manifest['package']['name'], 'independent-game')
            for path in (destination / 'src').rglob('*.rs'):
                self.assertEqual(path.read_bytes(), (engine / boot.STARTER / path.relative_to(destination)).read_bytes())
            self.assertTrue((destination / 'src/bin/bundle.rs').is_file())
            self.assertTrue((destination / 'tests/bundle.rs').is_file())
            self.assertFalse((destination / 'tests/standalone_x11.rs').exists())
            self.assertEqual(manifest['dependencies']['orr_remote']['default-features'], False)


if __name__ == '__main__':
    unittest.main()
