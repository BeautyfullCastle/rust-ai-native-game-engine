#!/usr/bin/env python3
"""Fail-closed checks for the checkpoint gate's Linux strace output.

This is a bounded acceptance parser for the gate's own trace selection and
namespace (initial cwd /), not a general strace interpreter. Child processes
whose creation is outside the file trace inherit all observed cwd candidates;
that conservative over-approximation may reject an ambiguous trace, never omit
a possible profile probe. A real positive CI run remains mandatory.
"""
import ast
from pathlib import PurePosixPath
import posixpath
import re


def arguments(text):
    result = []
    start = 0
    quoted = escaped = False
    stack = []
    pairs = {')': '(', ']': '[', '}': '{', '>': '<'}
    for index, char in enumerate(text):
        if quoted:
            if escaped: escaped = False
            elif char == '\\': escaped = True
            elif char == '"': quoted = False
        elif char == '"': quoted = True
        elif char in '([{<': stack.append(char)
        elif char in ')]}>':
            assert stack and stack.pop() == pairs[char], 'unbalanced syscall arguments'
        elif char == ',' and not stack:
            result.append(text[start:index].strip())
            start = index + 1
    assert not quoted and not stack, 'incomplete syscall arguments'
    result.append(text[start:].strip())
    return result


def calls(text):
    """Rejoin per-process unfinished/resumed calls before inspecting results."""
    result, pending = [], {}
    for line in text.splitlines():
        match = re.fullmatch(r'(?:(?:\[pid\s+(\d+)\]|(\d+))\s+)?(.*)', line.strip())
        pid, body = match[1] or match[2] or 'main', match[3]
        if not body or body.startswith(('--- ', '+++ ')):
            continue
        if body.endswith('<unfinished ...>'):
            assert pid not in pending, 'overlapping unfinished syscall'
            pending[pid] = body[:-len('<unfinished ...>')]
            continue
        resumed = re.match(r'<\.\.\. ([A-Za-z0-9_]+) resumed>(.*)', body)
        if resumed:
            assert pid in pending and pending[pid].startswith(resumed[1] + '('), 'unmatched resumed syscall'
            body = pending.pop(pid) + resumed[2]
        call = re.fullmatch(r'([A-Za-z0-9_]+)\((.*)\)\s+=\s+(.*)', body)
        assert call, ('unsupported trace record', body)
        result.append((pid, call[1], arguments(call[2]), call[3]))
    assert not pending, 'incomplete traced syscall'
    assert result, 'empty syscall trace'
    return result


def quoted(text):
    assert re.fullmatch(r'"(?:[^"\\]|\\.)*"', text), ('unresolved quoted path', text)
    value = ast.literal_eval(text)
    assert isinstance(value, str) and '\0' not in value
    return value


def normalized(path):
    assert path.startswith('/') and ' (deleted)' not in path, ('unresolved descriptor path', path)
    return PurePosixPath(posixpath.normpath(path))


def descriptor(text):
    match = re.fullmatch(r'(?:\d+|AT_FDCWD)<(/[^<>]*?)(?:<[^<>]+>)?>', text)
    assert match, ('missing descriptor annotation', text)
    return normalized(match[1])


def retval(text):
    match = re.match(r'(-?\d+)(?:\s|$)', text)
    return int(match[1]) if match else None


def check_no_profile_io(text, data, home):
    forbidden = (normalized(str(data)), normalized(str(home) + '/.local'))
    # Retain the source gate's direct absolute-path and reserved-slot rejection.
    assert not any(str(path) in text for path in forbidden), 'headless/capture referenced profile path'
    assert 'checkpoint.json' not in text and 'checkpoint.lock' not in text, 'headless/capture referenced checkpoint slot'
    known_cwds = {PurePosixPath('/')}
    process_cwds = {}
    def admit(paths):
        for path in paths:
            assert not any(path == root or path.is_relative_to(root) for root in forbidden), ('headless/capture profile probe', path)
    for pid, name, args, returned in calls(text):
        # Include observed sibling cwd states because file-only tracing omits
        # clone flags (CLONE_FS can share cwd). Ambiguity is conservative.
        cwd = set(process_cwds.setdefault(pid, set(known_cwds))) | known_cwds
        # Input pathname positions only: readlink/getcwd output buffers and
        # write payloads may legitimately be abbreviated by strace.
        relative = {
            'openat': [(1, 0)], 'openat2': [(1, 0)], 'newfstatat': [(1, 0)],
            'fstatat64': [(1, 0)], 'statx': [(1, 0)], 'faccessat': [(1, 0)],
            'faccessat2': [(1, 0)], 'readlinkat': [(1, 0)], 'execveat': [(1, 0)],
            'unlinkat': [(1, 0)], 'mkdirat': [(1, 0)], 'mknodat': [(1, 0)],
            'fchmodat': [(1, 0)], 'fchmodat2': [(1, 0)], 'fchownat': [(1, 0)],
            'futimesat': [(1, 0)], 'utimensat': [(1, 0)],
            'name_to_handle_at': [(1, 0)], 'renameat': [(1, 0), (3, 2)],
            'renameat2': [(1, 0), (3, 2)], 'linkat': [(1, 0), (3, 2)],
            'symlinkat': [(0, None), (2, 1)],
        }
        plain = {
            'open', 'creat', 'stat', 'stat64', 'lstat', 'lstat64', 'access',
            'readlink', 'chdir', 'execve', 'unlink', 'mkdir', 'rmdir', 'mknod',
            'chmod', 'chown', 'lchown', 'utime', 'utimes', 'truncate', 'truncate64',
            'statfs', 'statfs64', 'chroot', 'umount', 'umount2', 'swapon', 'swapoff',
        }
        fields = relative.get(name, [(0, None)] if name in plain else [])
        if name in ('rename', 'link', 'symlink', 'pivot_root'): fields = [(0, None), (1, None)]
        if name == 'inotify_add_watch': fields = [(1, None)]
        if not fields and any(arg.startswith('"') for arg in args) and name not in ('write', 'writev', 'pwrite64', 'pwritev', 'pwritev2', 'getcwd'):
            raise AssertionError(('unsupported pathname syscall', name))
        for path_index, dir_index in fields:
            value = quoted(args[path_index])
            if value.startswith('/'):
                paths = {normalized(value)}
            else:
                handle = args[dir_index] if dir_index is not None else 'AT_FDCWD'
                bases = cwd if handle == 'AT_FDCWD' else {descriptor(handle)}
                paths = {normalized(posixpath.join(str(base), value)) for base in bases}
            admit(paths)
        if name == 'chdir' and retval(returned) == 0:
            assert len(args) == 1
            value = quoted(args[0])
            next_cwds = {normalized(value)} if value.startswith('/') else {
                normalized(posixpath.join(str(base), value)) for base in cwd}
            admit(next_cwds)
            process_cwds[pid] = next_cwds
            known_cwds.update(next_cwds)
        elif name == 'fchdir' and retval(returned) == 0:
            assert len(args) == 1
            next_cwds = {descriptor(args[0])}
            admit(next_cwds)
            process_cwds[pid] = next_cwds
            known_cwds.update(next_cwds)


def check_publication(text, profile):
    profile = normalized(str(profile))
    locks, writes, synced, renames, directory_syncs = [], {}, {}, [], []
    for index, (_, name, args, returned) in enumerate(calls(text)):
        value = retval(returned)
        if name == 'flock' and value == 0:
            assert len(args) == 2
            operations = set(args[1].split('|'))
            if descriptor(args[0]) == profile / 'checkpoint.lock' and operations in ({'LOCK_EX'}, {'LOCK_EX', 'LOCK_NB'}):
                locks.append(index)
        elif name in ('write', 'writev', 'pwrite64', 'pwritev', 'pwritev2') and value is not None and value > 0:
            match = re.fullmatch(r'\d+<(/[^<>]*?)(?:<[^<>]+>)?>', args[0])
            if not match: continue  # stdout pipes/sockets never prove a file write
            path = normalized(match[1])
            if path.parent == profile and path.name.startswith('.checkpoint-stage-'):
                writes.setdefault(path, []).append(index)
        elif name == 'fsync' and value == 0:
            assert len(args) == 1
            path = descriptor(args[0])
            if path == profile: directory_syncs.append(index)
            elif path.parent == profile and path.name.startswith('.checkpoint-stage-'):
                synced.setdefault(path, []).append(index)
        elif name == 'renameat' and value == 0:
            assert len(args) == 4
            if descriptor(args[0]) == profile and descriptor(args[2]) == profile and quoted(args[3]) == 'checkpoint.json':
                staged = quoted(args[1])
                if '/' not in staged and staged.startswith('.checkpoint-stage-'):
                    renames.append((profile / staged, index))
    assert locks, 'no successful exclusive profile lock acquisition'
    assert writes, 'no successful positive-byte staged checkpoint write'
    # One actual stage must have been written, synced, renamed and followed by
    # directory sync while preceded by an exclusive lock. Unrelated stdout,
    # another namespace, another stage, failed/zero writes and unlocks cannot pass.
    assert any(lock < write < sync < rename < directory_sync
               for stage, rename in renames for lock in locks
               for write in writes.get(stage, []) for sync in synced.get(stage, [])
               for directory_sync in directory_syncs), 'no ordered durable checkpoint publication'


def check_syscall_proof(smoke_trace, acquire_trace, data, home, profile):
    data = normalized(str(data))
    profile = normalized(str(profile))
    assert profile != data and profile.is_relative_to(data), 'profile escaped external data root'
    check_no_profile_io(smoke_trace, data, home)
    check_publication(acquire_trace, profile)
