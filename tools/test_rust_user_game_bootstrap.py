"""Actual Python/Rust bootstrap differential against both fixed reviewed inputs.

Required: ORR_RUST_PACKAGE_BIN, ORR_BOOTSTRAP_ENGINE_SOURCE and
ORR_BOOTSTRAP_ACTION_ENGINE_SOURCE. No silent skips. No Cargo/Git/game launches.
TOML parsing and literal include-path closure are static consumption proofs only,
not compilation, runtime, native-window or source-hidden deployment acceptance.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import unittest

import bootstrap_user_game as legacy

SCRIPT = Path(__file__).resolve().with_name('bootstrap_user_game.py')


def snapshot(root):
    """All owned paths, bytes and permission bits; top Git/build metadata excluded."""
    result = {}
    for directory, dirs, names in os.walk(root, followlinks=False):
        if Path(directory) == root:
            dirs[:] = [d for d in dirs if d not in ('.git', 'target')]
            names = [n for n in names if n not in ('.git', 'target')]
        for name in sorted(dirs + names):
            path = Path(directory) / name
            info = path.lstat()
            key = path.relative_to(root).as_posix()
            mode = stat.S_IMODE(info.st_mode)
            if stat.S_ISLNK(info.st_mode):
                result[key] = ('link', mode, os.readlink(path))
            elif stat.S_ISDIR(info.st_mode):
                result[key] = ('directory', mode)
            elif stat.S_ISREG(info.st_mode):
                result[key] = ('file', mode, path.read_bytes())
            else:
                result[key] = ('nonregular', mode)
    return result


class RustUserGameBootstrap(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        value = os.environ.get('ORR_RUST_PACKAGE_BIN')
        if not value or not Path(value).is_absolute() or not Path(value).is_file() or not os.access(value, os.X_OK):
            raise RuntimeError('ORR_RUST_PACKAGE_BIN must identify an explicit trusted executable')
        cls.binary = value
        cls.engines = {}
        for profile, variable in [('legacy', 'ORR_BOOTSTRAP_ENGINE_SOURCE'),
                                  ('multi-action-preview', 'ORR_BOOTSTRAP_ACTION_ENGINE_SOURCE')]:
            value = os.environ.get(variable)
            if not value or not Path(value).is_absolute() or not Path(value).is_dir():
                raise RuntimeError(f'{variable} must identify the exact reviewed source directory')
            cls.engines[profile] = Path(value)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sentinel = self.root / 'unrelated-user-data'
        self.sentinel.write_bytes(b'never change')

    def command(self, native, engine, output, profile='legacy', name='my-user-game'):
        prefix = [self.binary, 'bootstrap-user-game'] if native else [sys.executable, '-B', str(SCRIPT)]
        return prefix + ['--engine', str(engine), '--destination', str(output), '--name', name,
                         '--profile', profile]

    def execute(self, command):
        return subprocess.run(command, capture_output=True, text=True, timeout=90,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def refused(self, engine, profile='legacy', output=None, name='my-user-game', both=True):
        output = output if output is not None else self.root / 'refused-output'
        before = snapshot(self.root)
        for native in (False, True) if both else (True,):
            result = self.execute(self.command(native, engine, output, profile, name))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertEqual(snapshot(self.root), before, (native, result.stderr))
            self.assertEqual(self.sentinel.read_bytes(), b'never change')
        return output

    def assert_manifest_consumers(self, output, engine, profile):
        manifest = tomllib.loads((output / 'Cargo.toml').read_text())
        self.assertEqual(manifest['package']['name'], 'my-user-game')
        self.assertEqual(manifest['package']['version'], '0.1.0')
        self.assertEqual(manifest['lib']['name'], 'orr_static_starter')
        self.assertEqual(manifest['workspace']['resolver'], '2')
        self.assertNotIn('standalone-native-acceptance', manifest['features'])
        count = 0
        for section in ('dependencies', 'dev-dependencies', 'build-dependencies'):
            for dependency in manifest.get(section, {}).values():
                if isinstance(dependency, dict) and 'path' in dependency:
                    path = Path(dependency['path'])
                    self.assertTrue(path.is_absolute())
                    self.assertTrue(path.is_relative_to(engine))
                    self.assertTrue((path / 'Cargo.toml').is_file())
                    count += 1
        self.assertGreater(count, 0)
        includes = 0
        for source in output.rglob('*.rs'):
            for text in re.findall(r'include_(?:str|bytes)!\(\s*"([^"\\]+)"\s*\)', source.read_text()):
                path = (source.parent / text).resolve()
                self.assertTrue(path.is_relative_to(output), (source, text))
                self.assertTrue(path.is_file(), (source, text))
                includes += 1
        self.assertGreaterEqual(includes, 2)
        if profile == 'multi-action-preview':
            self.assertEqual(manifest['features']['static-game-actions'],
                             ['dep:orr_input', 'orr_editor?/static-game-actions'])
            self.assertNotIn('static-game-actions', manifest['features']['default'])
            self.assertTrue((output / 'tests/multi_action_contract.rs').is_file())
        return manifest

    def test_both_real_pins_every_generated_byte_mode_path_and_receipt(self):
        for profile, engine in self.engines.items():
            with self.subTest(profile=profile):
                before = snapshot(engine)
                outputs = []
                receipts = []
                for native in (False, True):
                    output = self.root / f'{profile}-{native}'
                    result = self.execute(self.command(native, engine, output, profile))
                    self.assertEqual(result.returncode, 0, result.stderr)
                    receipt = json.loads(result.stdout)
                    self.assertEqual(receipt['destination'], str(output))
                    self.assertIs(receipt['cargo_ran'], False)
                    self.assertEqual(receipt['engine_tree'], legacy.profile_contract(profile)[1])
                    self.assertEqual(receipt['files'], sum(p.is_file() for p in output.rglob('*')))
                    receipts.append({**receipt, 'destination': '<output>'})
                    outputs.append(output)
                self.assertEqual(receipts[0], receipts[1])
                self.assertEqual(snapshot(outputs[0]), snapshot(outputs[1]))
                self.assertEqual(stat.S_IMODE(outputs[0].stat().st_mode),
                                 stat.S_IMODE(outputs[1].stat().st_mode))
                self.assertEqual(snapshot(engine), before, 'bootstrap changed the pinned input')
                output = outputs[1]
                self.assert_manifest_consumers(output, engine, profile)
                self.assertEqual((output / 'UPSTREAM_README.md').read_bytes(),
                                 (engine / legacy.STARTER / 'README.md').read_bytes())
                provenance = json.loads((output / 'provenance.json').read_bytes())
                self.assertEqual(provenance['engine_path'], str(engine))
                self.assertEqual(provenance['engine_commit'], legacy.profile_contract(profile)[0])
                self.assertIsNone(provenance['user_source_commit'])
                self.assertEqual(list(provenance['files_sha256']), sorted(provenance['files_sha256']))
                self.assertEqual(set(provenance['files_sha256']),
                                 {p.relative_to(output).as_posix() for p in output.rglob('*') if p.is_file()} - {'provenance.json'})
                for name, digest in provenance['files_sha256'].items():
                    self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), digest)
                for forbidden in ('Cargo.lock', '.git', 'target'):
                    self.assertFalse((output / forbidden).exists())
                for name in legacy.profile_contract(profile)[2]:
                    self.assertFalse((output / name).exists())
                if profile == 'multi-action-preview':
                    self.assertEqual(provenance['stability'], 'preview')
                    self.assertEqual(provenance['hosted_native_acceptance'], 'unverified')
                    self.assertIs(provenance['action_feature_default'], False)

    def test_default_profile_matches_explicit_legacy(self):
        engine = self.engines['legacy']
        outputs = [self.root / 'default', self.root / 'explicit']
        command = self.command(True, engine, outputs[0])[:-2]
        for cmd in (command, self.command(True, engine, outputs[1])):
            result = self.execute(cmd)
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(snapshot(outputs[0]), snapshot(outputs[1]))

    def test_changed_pin_mode_untracked_code_symlink_and_fifo_are_refused(self):
        source = self.engines['legacy']
        engine = self.root / 'copied engine'
        # One private copy is reused and restored. Never mutate the supplied pin.
        shutil.copytree(source, engine, symlinks=True,
                        ignore=lambda directory, names: [n for n in names if Path(directory) == source and n in ('.git', 'target')])
        target = engine / 'Cargo.toml'
        original, mode = target.read_bytes(), stat.S_IMODE(target.stat().st_mode)
        baseline = snapshot(engine)
        try:
            target.write_bytes(original + b'\n# altered reviewed bytes\n')
            self.refused(engine)
        finally:
            target.write_bytes(original)
        try:
            target.chmod(mode ^ stat.S_IXUSR)
            self.refused(engine)
        finally:
            target.chmod(mode)
        for name, data in [('.cargo/config.toml', b'[build]\nrustflags = ["--cfg", "injected"]\n'),
                           ('build.rs', b'fn main() { panic!("must not execute"); }\n')]:
            extra = engine / name
            self.assertFalse(extra.exists())
            parent_existed = extra.parent.exists()
            extra.parent.mkdir(parents=True, exist_ok=True)
            try:
                extra.write_bytes(data)
                self.refused(engine)
            finally:
                extra.unlink()
                if not parent_existed:
                    extra.parent.rmdir()
        for kind in ('symlink', 'fifo', 'missing'):
            target.unlink()
            try:
                if kind == 'symlink':
                    target.symlink_to(engine / 'crates/orr_editor/Cargo.toml')
                elif kind == 'fifo':
                    os.mkfifo(target)
                self.refused(engine)
            finally:
                if os.path.lexists(target):
                    target.unlink()
                target.write_bytes(original)
                target.chmod(mode)
        self.assertEqual(snapshot(engine), baseline)

    def test_existing_output_file_directory_link_and_inside_engine_preserved(self):
        engine = self.engines['legacy']
        out = self.root / 'existing'
        out.write_bytes(b'foreign file')
        self.refused(engine, output=out)
        out.unlink()
        out.mkdir()
        (out / 'sentinel').write_bytes(b'foreign directory')
        self.refused(engine, output=out)
        dangling = self.root / 'dangling'
        dangling.symlink_to('absent')
        self.refused(engine, output=dangling)
        # No engine writes are attempted: both tools must reject containment first.
        inside = engine / 'must-not-be-created-by-bootstrap-test'
        self.assertFalse(os.path.lexists(inside))
        self.refused(engine, output=inside)
        self.assertFalse(os.path.lexists(inside))
        self.refused(engine, output=engine)

    def test_engine_alias_destination_alias_traversal_and_profile_mismatch(self):
        engine = self.engines['legacy']
        alias = self.root / 'alias-engine'
        alias.symlink_to(engine, target_is_directory=True)
        self.refused(alias)
        destination_alias = self.root / 'alias-parent'
        destination_alias.symlink_to(self.root, target_is_directory=True)
        self.refused(engine, output=destination_alias / 'new')
        self.refused(engine, output=self.root / 'existing-parent' / '..' / 'new')
        self.refused(engine, profile='multi-action-preview')
        self.refused(self.engines['multi-action-preview'], profile='legacy')
        self.refused(engine, profile='unreviewed')
        for name in ('UPPER', 'test', 'build', 'deps', 'examples', 'a' * 65, '../game'):
            self.refused(engine, name=name)

    def test_native_cannot_override_pinned_inventory_or_execute_commands(self):
        engine = self.engines['legacy']
        for flag in ('--inventory', '--commit', '--tree', '--command'):
            out = self.root / 'override-refused'
            result = self.execute(self.command(True, engine, out) + [flag, str(self.sentinel)])
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(os.path.lexists(out))
            self.assertEqual(self.sentinel.read_bytes(), b'never change')


if __name__ == '__main__':
    unittest.main()
