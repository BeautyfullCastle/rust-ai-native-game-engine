#!/usr/bin/env python3
"""Synthetic acceptance-checker fixtures. No subprocess or real strace is used."""
import ast
from pathlib import Path, PurePosixPath
from types import SimpleNamespace
import unittest

import room_checkpoint_syscall as gate


DATA = PurePosixPath('/proof/data')
HOME = PurePosixPath('/proof/home')
PROFILE = DATA / 'orrery/checkpoints/fixture'
SMOKE = '11 openat(AT_FDCWD, "/usr/lib/library.so", O_RDONLY) = 3</usr/lib/library.so>\n'


def publication():
    return [f'22 flock(3<{PROFILE}/checkpoint.lock>, LOCK_EX|LOCK_NB) = 0',
            f'22 write(4<{PROFILE}/.checkpoint-stage-one>, "{{...}}", 7) = 7',
            f'22 fsync(4<{PROFILE}/.checkpoint-stage-one>) = 0',
            f'22 renameat(5<{PROFILE}>, ".checkpoint-stage-one", 5<{PROFILE}>, "checkpoint.json") = 0',
            f'22 fsync(5<{PROFILE}>) = 0']


def check(rows=None, smoke=SMOKE):
    gate.check_syscall_proof(smoke, '\n'.join(publication() if rows is None else rows) + '\n', DATA, HOME, PROFILE)


def exporter_trace_selection(source=None):
    """Run only the actual command-construction function with a recording stub."""
    if source is None:
        source = Path(__file__).with_name('check-room-checkpoint-export.py').read_text()
    functions = [node for node in ast.walk(ast.parse(source)) if isinstance(node, ast.FunctionDef) and node.name == 'isolated']
    assert len(functions) == 1
    namespace = {'a': SimpleNamespace(syscall=True), 'traces': PurePosixPath('/proof/traces'),
                 'base': ['bwrap', '--chdir', '/'], 'env': {},
                 'run': lambda name, command, env: command}
    exec(compile(ast.fix_missing_locations(ast.Module(body=functions, type_ignores=[])), '<exporter-command-construction>', 'exec'), namespace)
    command = namespace['isolated']('readonly-export-smoke', ['runtime-fixture'])
    selection = command[command.index('-e') + 1]
    assert selection.startswith('trace=')
    return set(selection.removeprefix('trace=').split(','))


class SyscallChecks(unittest.TestCase):
    def test_actual_exporter_selects_and_checks_descriptor_cwd_transition(self):
        selection = exporter_trace_selection()
        # strace's x86_64 table marks open/openat TF and fchdir TD. %file
        # therefore requires an explicit fchdir selection to emit this event.
        # This projects synthetic records; it never invokes strace or a process.
        events = [('openat', True, f'11 openat(AT_FDCWD, "{HOME}", O_RDONLY) = 3<{HOME}>'),
                  ('fchdir', False, f'11 fchdir(3<{HOME}>) = 0'),
                  ('open', True, '11 open(".local/share", O_RDONLY) = -1 ENOENT')]
        def project(selected):
            return '\n'.join(line for name, file_class, line in events if name in selected or file_class and '%file' in selected) + '\n'
        self.assertIn('%file', selection)
        self.assertIn('fchdir', selection)
        emitted = project(selection)
        self.assertLess(emitted.index('fchdir('), emitted.index('open(".local/share"'))
        with self.assertRaises(AssertionError):
            gate.check_no_profile_io(emitted, DATA, HOME)
        # Reproduce the prior selection gap: an unobserved CWD change cannot
        # be consumed by the parser, so the actual selector must retain it.
        gate.check_no_profile_io(project(selection - {'fchdir'}), DATA, HOME)

    def test_positive_synthetic_trace(self):
        check()

    def test_each_positive_syscall_is_required(self):
        for index in range(5):
            rows = publication(); rows.pop(index)
            with self.subTest(index=index), self.assertRaises(AssertionError): check(rows)

    def test_failed_only_syscalls_do_not_prove_publication(self):
        for index in range(5):
            rows = publication(); rows[index] = rows[index].rsplit(' = ', 1)[0] + ' = -1 EPERM (Operation not permitted)'
            with self.subTest(index=index), self.assertRaises(AssertionError): check(rows)

    def test_zero_byte_and_unrelated_stdout_writes_are_rejected(self):
        for replacement in [f'22 write(4<{PROFILE}/.checkpoint-stage-one>, "", 0) = 0',
                            '22 write(1<pipe:[1234]>, "saved", 5) = 5',
                            '22 write(1</tmp/output.log>, "saved", 5) = 5']:
            rows = publication(); rows[1] = replacement
            with self.assertRaises(AssertionError): check(rows)

    def test_wrong_namespace_stage_and_deleted_descriptor_are_rejected(self):
        for path in [str(PROFILE) + '-other/.checkpoint-stage-one',
                     str(PROFILE) + '/nested/.checkpoint-stage-one',
                     str(PROFILE) + '/checkpoint.json',
                     str(PROFILE) + '/.checkpoint-stage-other',
                     str(PROFILE) + '/.checkpoint-stage-one (deleted)']:
            rows = publication(); rows[1] = f'22 write(4<{path}>, "value", 5) = 5'
            with self.assertRaises(AssertionError): check(rows)

    def test_unlock_shared_and_nonexclusive_lock_do_not_count(self):
        for operation in ['LOCK_UN', 'LOCK_SH', 'LOCK_SH|LOCK_NB', 'LOCK_NB', 'LOCK_EX|LOCK_UN']:
            rows = publication(); rows[0] = rows[0].replace('LOCK_EX|LOCK_NB', operation)
            with self.assertRaises(AssertionError): check(rows)

    def test_write_sync_rename_and_directory_sync_must_be_ordered(self):
        for left, right in [(0, 1), (1, 2), (2, 3), (3, 4)]:
            rows = publication(); rows[left], rows[right] = rows[right], rows[left]
            with self.assertRaises(AssertionError): check(rows)

    def test_absolute_and_descriptor_relative_probes_reject_failed_attempts(self):
        probes = [f'openat(AT_FDCWD, "{DATA}", O_RDONLY)',
                  f'openat(3<{DATA.parent}>, "data/orrery", O_RDONLY)',
                  f'openat(3<{HOME}>, ".local/share", O_RDONLY)',
                  f'openat(3<{HOME}>, "unused/../.local/share", O_RDONLY)',
                  f'openat(3<{DATA.parent}>, "./unrelated/../data/orrery", O_RDONLY)']
        for probe in probes:
            for result in ('3', '-1 ENOENT (No such file or directory)'):
                with self.assertRaises(AssertionError): check(smoke=probe + ' = ' + result + '\n')

    def test_cwd_and_fchdir_relative_home_fallbacks_are_rejected(self):
        for move in [f'chdir("{HOME}") = 0', f'fchdir(3<{HOME}>) = 0',
                     'chdir("/proof") = 0\nchdir("home") = 0']:
            with self.assertRaises(AssertionError):
                check(smoke=move + '\nopenat(AT_FDCWD, ".local/share", O_RDONLY) = -1 ENOENT\n')

    def test_child_and_shared_cwd_candidates_are_conservative(self):
        for prefix in ['', '2 openat(AT_FDCWD, "/lib/file", O_RDONLY) = 3</lib/file>\n']:
            text = prefix + f'1 chdir("{HOME}") = 0\n2 openat(AT_FDCWD, ".local/share", O_RDONLY) = -1 ENOENT\n'
            with self.assertRaises(AssertionError): check(smoke=text)

    def test_unknown_relevant_dirfd_and_fchdir_fail_closed(self):
        for text in ['openat(3, "data/orrery", O_RDONLY) = -1 ENOENT\n',
                     'fchdir(3) = 0\nopenat(AT_FDCWD, ".local/share", O_RDONLY) = -1 ENOENT\n']:
            with self.assertRaises(AssertionError): check(smoke=text)

    def test_failed_chdir_does_not_change_cwd(self):
        check(smoke=f'chdir("{HOME}") = -1 ENOENT\nopenat(AT_FDCWD, ".local/share", O_RDONLY) = -1 ENOENT\n')

    def test_unfinished_write_is_rejoined_and_incomplete_trace_is_rejected(self):
        rows = publication()
        rows[1:2] = [rows[1].replace(' = 7', ' <unfinished ...>'), '22 <... write resumed>) = 7']
        # strace splits before the final closing parenthesis when unfinished.
        rows[1] = rows[1].replace(') <unfinished', ' <unfinished')
        check(rows)
        with self.assertRaises(AssertionError): check(rows[:-1] + ['99 openat(AT_FDCWD, "/lib/file", O_RDONLY <unfinished ...>'])

    def test_truncated_write_payload_and_unrelated_pipe_output_are_supported(self):
        rows = publication(); rows[1] = rows[1].replace('"{...}"', '"a longer JSON checkpoint prefix"...')
        rows.insert(0, '22 write(1<pipe:[1234]>, "long output"..., 200) = 200')
        rows.insert(0, '22 write(1</dev/null<char 1:3>>, "ignored", 7) = 7')
        smoke = SMOKE + '11 write(1<pipe:[4567]>, "long smoke output"..., 200) = 200\n'
        smoke += '11 fchdir(5</>) = 0\n11 openat(AT_FDCWD</>, "usr/lib/library.so", O_RDONLY) = 3</usr/lib/library.so>\n'
        check(rows, smoke=smoke)

    def test_vectored_and_positioned_write_variants_require_same_stage(self):
        variants = [f'22 writev(4<{PROFILE}/.checkpoint-stage-one>, [{{iov_base="value", iov_len=5}}], 1) = 5',
                    f'22 pwrite64(4<{PROFILE}/.checkpoint-stage-one>, "value", 5, 0) = 5',
                    f'22 pwritev(4<{PROFILE}/.checkpoint-stage-one>, [{{iov_base="value", iov_len=5}}], 1, 0) = 5',
                    f'22 pwritev2(4<{PROFILE}/.checkpoint-stage-one>, [{{iov_base="value", iov_len=5}}], 1, 0, 0) = 5']
        for variant in variants:
            rows = publication(); rows[1] = variant; check(rows)

    def test_empty_and_malformed_traces_fail_closed(self):
        for text in ['', 'strace: ptrace denied\n', '99 <... openat resumed>) = -1 ENOENT\n']:
            with self.assertRaises(AssertionError): check(smoke=text)


if __name__ == '__main__': unittest.main()
