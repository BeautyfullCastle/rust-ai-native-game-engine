"""Synthetic lock preparation regression tests, not Cargo/native acceptance."""
import ast
import inspect
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import user_game_native_gate as native
import user_game_action_preview_gate as preview


class OwnLockTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.project = Path(self.tmp.name)
        self.lock = self.project/'Cargo.lock'
        self.calls = []

    def execute(self, args, name, timeout):
        self.calls.append((args, name, timeout))
        if name == 'own-lock':
            self.lock.write_text('# independently resolved compatible dependency\n')

    def test_new_resolution_then_locked_fetch_share_original_budget(self):
        with patch.object(native.time, 'monotonic', side_effect=[100, 101, 140]):
            digest = native.prepare_own_lock(self.project, self.execute)
        self.assertEqual(self.calls, [(['cargo', 'generate-lockfile'], 'own-lock', 179),
                                     (['cargo', 'fetch', '--locked'], 'own-lock-fetch', 140)])
        self.assertEqual(digest, native.editor.digest(self.lock))

    def test_yanked_only_offline_cache_is_not_used_for_new_resolution(self):
        def registry(args, name, timeout):
            if args[:2] == ['cargo', 'generate-lockfile'] and '--offline' in args:
                raise ValueError('only cached compatible version is yanked')
            self.execute(args, name, timeout)
        native.prepare_own_lock(self.project, registry)
        self.assertEqual(len(self.calls), 2)
        self.assertNotIn('--ignore-rust-version', self.calls[0][0])

    def test_existing_or_dangling_link_lock_is_rejected_before_cargo(self):
        for linked in (False, True):
            with self.subTest(linked=linked):
                if linked: self.lock.symlink_to(self.project/'missing')
                else: self.lock.write_text('# copied engine lock')
                with self.assertRaisesRegex(ValueError, 'start absent'):
                    native.prepare_own_lock(self.project, self.execute)
                self.assertEqual(self.calls, [])
                self.lock.unlink()

    def test_resolution_and_fetch_failures_propagate_without_fallback(self):
        for failure in ('own-lock', 'own-lock-fetch'):
            with self.subTest(failure=failure):
                self.calls.clear()
                if self.lock.exists(): self.lock.unlink()
                def execute(args, name, timeout):
                    if name == failure: raise ValueError(name+' original failure')
                    self.execute(args, name, timeout)
                with self.assertRaisesRegex(ValueError, failure+' original failure'):
                    native.prepare_own_lock(self.project, execute)
                self.assertEqual(len(self.calls), 0 if failure == 'own-lock' else 1)

    def test_generated_missing_and_linked_locks_fail_before_fetch(self):
        for linked in (False, True):
            with self.subTest(linked=linked):
                calls = []
                def execute(args, name, timeout):
                    calls.append(name)
                    if linked: self.lock.symlink_to(self.project/'missing')
                with self.assertRaisesRegex(ValueError, 'missing/linked'):
                    native.prepare_own_lock(self.project, execute)
                self.assertEqual(calls, ['own-lock'])
                if linked: self.lock.unlink()

    def test_fetch_cannot_mutate_delete_or_replace_lock_with_link(self):
        for mutation in ('bytes', 'delete', 'link'):
            with self.subTest(mutation=mutation):
                def execute(args, name, timeout):
                    self.execute(args, name, timeout)
                    if name == 'own-lock-fetch':
                        self.lock.unlink()
                        if mutation == 'bytes': self.lock.write_text('different')
                        elif mutation == 'link': self.lock.symlink_to(self.project/'missing')
                with self.assertRaisesRegex(ValueError, 'own lock'):
                    native.prepare_own_lock(self.project, execute)
                if self.lock.exists() or self.lock.is_symlink(): self.lock.unlink()

    def test_exhausted_resolution_budget_cannot_start_fetch(self):
        with patch.object(native.time, 'monotonic', side_effect=[100, 101, 280]):
            with self.assertRaisesRegex(ValueError, 'preparation timeout'):
                native.prepare_own_lock(self.project, self.execute)
        self.assertEqual([x[1] for x in self.calls], ['own-lock'])

    def test_both_gates_prepare_lock_before_own_commit(self):
        for module in (native, preview):
            tree = ast.parse(inspect.getsource(module.run))
            calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)]
            def named(node, name):
                return ((isinstance(node.func, ast.Name) and node.func.id == name) or
                        (isinstance(node.func, ast.Attribute) and node.func.attr == name))
            prepare = [x for x in calls if named(x, 'prepare_own_lock')]
            commit = [x for x in calls if named(x, 'own_commit')]
            self.assertEqual(len(prepare), 1); self.assertEqual(len(commit), 1)
            self.assertLess(prepare[0].lineno, commit[0].lineno)


if __name__ == '__main__':
    unittest.main()
