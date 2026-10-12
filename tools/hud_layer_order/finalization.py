"""Bounded, descriptor-bound ELF finalization/diagnosis under the existing owner.

This module never launches processes. Its CLI is called only through Runner.run.
One os.fsync invocation per proven inode; CPython may internally retry EINTR.
Neither successful fsync nor stable metadata establishes the old failure's cause.
"""
import hashlib
import json
import os
import pathlib
import re
import stat
import struct
import sys

MIB = 1024 ** 2
JSON_CAP = 64 * 1024
ELF_CAP = 192 * MIB
INODES_CAP = 8
ALIASES_CAP = 16
TOTAL_CAP = 768 * MIB
STAT_KEYS = {'bytes', 'mode', 'allocated', 'device', 'inode', 'mtime_ns', 'uid', 'nlink', 'ctime_ns'}
ROOT_KEYS = {'device', 'inode', 'uid', 'mode'}


def require(value, message):
    if not value:
        raise RuntimeError(message)


def snapshot(info):
    require(stat.S_ISREG(info.st_mode), 'ELF must be regular')
    return {'bytes': info.st_size, 'mode': stat.S_IMODE(info.st_mode),
            'allocated': info.st_blocks * 512, 'device': str(info.st_dev),
            'inode': str(info.st_ino), 'mtime_ns': str(info.st_mtime_ns),
            'ctime_ns': str(info.st_ctime_ns), 'uid': info.st_uid, 'nlink': info.st_nlink}


def root_identity(info):
    require(stat.S_ISDIR(info.st_mode), 'parent must be directory')
    return {'device': str(info.st_dev), 'inode': str(info.st_ino),
            'uid': info.st_uid, 'mode': stat.S_IMODE(info.st_mode)}


def relative_parts(name):
    require(isinstance(name, str) and 0 < len(name.encode('utf-8')) <= 512, 'path byte bound')
    path = pathlib.PurePosixPath(name)
    require(path.parts and not path.is_absolute() and str(path) == name and
            all(part not in ('', '.', '..') for part in path.parts), 'relative path required')
    return path.parts


def validate_request(request):
    require(isinstance(request, dict) and set(request) ==
            {'version', 'mode', 'phase', 'root', 'root_identity', 'groups'}, 'request keys')
    require(request['version'] == 1 and type(request['version']) is int, 'request version')
    require(request['mode'] in ('finalize', 'diagnose'), 'request mode')
    require(isinstance(request['phase'], str) and re.fullmatch('[a-zA-Z0-9_-]{1,80}', request['phase']), 'phase')
    require(isinstance(request['root'], str) and len(request['root'].encode()) <= 4096, 'root path bound')
    root = pathlib.PurePosixPath(request['root'])
    require(root.is_absolute() and str(root) == request['root'] and '..' not in root.parts,
            'canonical target root required')
    require(isinstance(request['root_identity'], dict) and set(request['root_identity']) == ROOT_KEYS,
            'root identity keys')
    identity = request['root_identity']
    require(all(type(identity[k]) is int and identity[k] >= 0 for k in ('uid', 'mode')) and
            all(isinstance(identity[k], str) and re.fullmatch('[0-9]{1,24}', identity[k])
                for k in ('device', 'inode')), 'root identity fields')
    require(identity['uid'] == os.geteuid(), 'target root owner')
    groups = request['groups']
    cap = INODES_CAP if request['mode'] == 'finalize' else 2
    total_cap = TOTAL_CAP if request['mode'] == 'finalize' else 384 * MIB
    require(isinstance(groups, list) and 0 < len(groups) <= cap, 'inode count bound')
    seen = set(); names = set(); total = 0
    for group in groups:
        require(isinstance(group, dict) and set(group) == {'aliases', 'stat', 'sha256', 'role'}, 'group keys')
        value = group['stat']
        require(isinstance(value, dict) and set(value) == STAT_KEYS, 'stat keys')
        require(all(type(value[k]) is int and value[k] >= 0 for k in ('bytes', 'mode', 'allocated', 'uid', 'nlink')),
                'integer stat fields')
        require(all(isinstance(value[k], str) and re.fullmatch('[0-9]{1,24}', value[k])
                    for k in ('device', 'inode', 'mtime_ns', 'ctime_ns')), 'string stat fields')
        require(64 <= value['bytes'] <= ELF_CAP and value['mode'] & 0o111 and value['uid'] == os.geteuid(),
                'owned ELF size/mode')
        require(isinstance(group['sha256'], str) and re.fullmatch('[0-9a-f]{64}', group['sha256']), 'producer hash')
        require(group['role'] in ('test', 'canonical-bin'), 'ELF role')
        aliases = group['aliases']
        require(isinstance(aliases, list) and len(aliases) == (1 if group['role'] == 'test' else 2)
                and len(aliases) == value['nlink'], 'complete link closure required')
        for name in aliases:
            relative_parts(name)
            require(name not in names, 'duplicate alias')
            names.add(name)
        key = value['device'], value['inode']
        require(key not in seen, 'inode must be deduplicated')
        seen.add(key); total += value['bytes']
    require(len(names) <= ALIASES_CAP and total <= total_cap, 'closure byte/alias bound')
    require(len(json.dumps(request, sort_keys=True).encode()) <= JSON_CAP, 'request byte cap')


class Descriptors:
    """Retain each directory edge and leaf until endpoint proof and close."""
    def __init__(self):
        self.fds = []
        self.edges = []
        self.paths = {}

    def directory(self, parent, name):
        fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                     dir_fd=parent)
        self.fds.append(fd)
        self.paths[fd] = str(pathlib.PurePosixPath(name) if parent is None else pathlib.PurePosixPath(self.paths[parent]) / name)
        identity = root_identity(os.fstat(fd))
        self.edges.append((parent, name, fd, identity, True))
        return fd

    def absolute_directory(self, path):
        parts = pathlib.PurePosixPath(path).parts
        require(parts and parts[0] == '/' and '..' not in parts, 'absolute directory')
        fd = self.directory(None, '/')
        for part in parts[1:]:
            fd = self.directory(fd, part)
        return fd

    def leaf(self, root, name):
        parts = relative_parts(name)
        parent = root
        for part in parts[:-1]:
            parent = self.directory(parent, part)
        # Do not block on a raced FIFO, and never read/fsync before fstat proves type.
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                     dir_fd=parent)
        self.fds.append(fd)
        value = snapshot(os.fstat(fd))
        self.edges.append((parent, parts[-1], fd, value, False))
        return fd, value

    def prove_paths(self):
        for parent, name, fd, expected, directory in self.edges:
            path = os.stat(name, dir_fd=parent, follow_symlinks=False)
            held = os.fstat(fd)
            if directory:
                require(root_identity(path) == root_identity(held) == expected, 'parent path identity changed')
            else:
                require(stat.S_ISREG(path.st_mode) and stat.S_ISREG(held.st_mode), 'leaf path type changed')
                require((path.st_dev, path.st_ino) == (held.st_dev, held.st_ino), 'leaf path identity changed')

    def parent_receipt(self):
        return {self.paths[fd]: identity for _, _, fd, identity, directory in self.edges if directory}

    def close(self):
        errors = []
        for fd in reversed(self.fds):
            try:
                os.close(fd)
            except Exception as error:
                errors.append({'phase': 'close', 'type': type(error).__name__[:64],
                               'errno': error.errno if isinstance(error, OSError) else None})
        self.fds = []
        return errors[:8]


def same_identity(expected, observed):
    require(all(expected[k] == observed[k] for k in STAT_KEYS - {'allocated'}), 'ELF identity changed')


def hash_fd(fd, expected_size):
    os.lseek(fd, 0, os.SEEK_SET)
    h = hashlib.sha256(); count = 0; header = bytearray()
    # Read no extra probe byte: mandatory endpoint fstat rejects growth.
    while count < expected_size:
        data = os.read(fd, min(MIB, expected_size - count))
        if not data:
            break
        count += len(data)
        require(count <= expected_size, 'growing ELF')
        header.extend(data[:max(0, 64 - len(header))])
        h.update(data)
    require(count == expected_size, 'truncated ELF')
    require(header[:7] == b'\x7fELF\x02\x01\x01' and
            struct.unpack_from('<H', header, 16)[0] in (2, 3) and
            struct.unpack_from('<H', header, 18)[0] == 62 and
            struct.unpack_from('<H', header, 52)[0] == 64, 'not Linux x86_64 full ELF')
    return h.hexdigest()


def mount_type(root, device, text):
    """Return selected non-sensitive fields only; ambiguity stays unknown."""
    if len(text.encode()) > MIB:
        return {'state': 'unknown', 'reason': 'mountinfo-cap'}
    matches = []
    try:
        for line in text.splitlines():
            fields = line.split(); split = fields.index('-')
            require(len(fields) > split + 2 and split >= 6, 'mountinfo row')
            point = re.sub(r'\\(040|011|012|134)', lambda m: chr(int(m[1], 8)), fields[4])
            if fields[2] != device or not pathlib.PurePosixPath(root).is_relative_to(point):
                continue
            require(re.fullmatch('[A-Za-z0-9_.+-]{1,64}', fields[split + 1]), 'filesystem type')
            matches.append({'mount_id': int(fields[0]), 'parent_id': int(fields[1]),
                            'point': point, 'type': fields[split + 1]})
        if not matches:
            return {'state': 'unknown', 'reason': 'no-matching-mount'}
        length = max(len(x['point']) for x in matches)
        matches = [x for x in matches if len(x['point']) == length]
        parents = {x['parent_id'] for x in matches}
        tops = [x for x in matches if x['mount_id'] not in parents]
        require(len(tops) == 1, 'ambiguous mount')
        return {'state': 'observed', 'type': tops[0]['type'], 'device': device, 'mount_id': tops[0]['mount_id']}
    except (ValueError, IndexError, RuntimeError):
        return {'state': 'unknown', 'reason': 'invalid-or-ambiguous-mountinfo'}


def execute(request):
    validate_request(request)
    descriptors = Descriptors()
    result = {'version': 1, 'mode': request['mode'], 'phase': request['phase'], 'groups': [], 'errors': [],
              'filesystem': {'state': 'unknown', 'reason': 'root-unavailable'}, 'parents': {}}
    phase = 'root-open'
    try:
        root = descriptors.absolute_directory(request['root'])
        require(root_identity(os.fstat(root)) == request['root_identity'], 'target root identity changed')
        device = int(request['root_identity']['device'])
        try:
            with open('/proc/self/mountinfo', encoding='utf-8') as stream:
                mountinfo = stream.read(MIB + 1)
            result['filesystem'] = mount_type(request['root'], f'{os.major(device)}:{os.minor(device)}', mountinfo)
        except (OSError, UnicodeError):
            result['filesystem'] = {'state': 'unknown', 'reason': 'mountinfo-unavailable'}
        groups = []
        # Open/prove the complete closure before flushing any inode.
        for group in request['groups']:
            phase = 'alias-open'
            opened = [descriptors.leaf(root, name) for name in group['aliases']]
            for _, value in opened:
                same_identity(group['stat'], value)
            require(all(value == opened[0][1] for _, value in opened), 'alias stat disagreement')
            groups.append((group, opened))
        descriptors.prove_paths()
        for group, opened in groups:
            phase = 'pre-hash'
            fd = opened[0][0]; before = snapshot(os.fstat(fd))
            same_identity(group['stat'], before)
            observed = hash_fd(fd, before['bytes'])
            if request['mode'] == 'finalize':
                require(observed == group['sha256'], 'pre-flush producer content differs')
            same_identity(before, snapshot(os.fstat(fd)))
            descriptors.prove_paths()
            after_hash = observed
            if request['mode'] == 'finalize':
                phase = 'fsync'
                os.fsync(fd)  # One API invocation; CPython-internal EINTR handling is documented.
                phase = 'post-hash'
                after_hash = hash_fd(fd, before['bytes'])
                require(after_hash == observed == group['sha256'], 'post-flush content differs')
            phase = 'endpoint-proof'
            after = snapshot(os.fstat(fd)); same_identity(before, after)
            for alias_fd, _ in opened:
                require(snapshot(os.fstat(alias_fd)) == after, 'alias endpoint disagreement')
            descriptors.prove_paths()
            result['groups'].append({'aliases': group['aliases'], 'role': group['role'],
                'expected_sha256': group['sha256'], 'before': before, 'after': after,
                'sha256': after_hash, 'hash_matches': after_hash == group['sha256'],
                'allocation_stable': before['allocated'] == after['allocated'],
                'fsync_api_calls': 1 if request['mode'] == 'finalize' else 0})
        descriptors.prove_paths()
        result['parents'] = descriptors.parent_receipt()
    except Exception as error:
        result['errors'].append({'phase': phase, 'type': type(error).__name__[:64],
                                 'errno': error.errno if isinstance(error, OSError) else None})
    finally:
        result['errors'].extend(descriptors.close())
        result['errors'] = result['errors'][:8]
    result['ok'] = not result['errors'] and len(result['groups']) == len(request['groups'])
    require(len(json.dumps(result, sort_keys=True).encode()) <= JSON_CAP, 'result byte cap')
    return result


def read_request(path, expected_sha):
    require(re.fullmatch('[0-9a-f]{64}', expected_sha), 'request hash syntax')
    path = pathlib.PurePosixPath(path)
    require(path.is_absolute() and '..' not in path.parts, 'request absolute path')
    descriptors = Descriptors()
    try:
        parent = descriptors.absolute_directory(str(path.parent))
        fd, value = descriptors.leaf(parent, path.name)
        require(value['uid'] == os.geteuid() and value['nlink'] == 1 and value['bytes'] <= JSON_CAP,
                'request ownership/size')
        data = bytearray()
        while len(data) <= JSON_CAP:
            chunk = os.read(fd, min(4096, JSON_CAP + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
        require(len(data) <= JSON_CAP and hashlib.sha256(data).hexdigest() == expected_sha, 'request bytes differ')
        require(snapshot(os.fstat(fd)) == value, 'request changed while read')
        descriptors.prove_paths()
        value = json.loads(data)
    finally:
        errors = descriptors.close()
        require(not errors, 'request close failure')
    return value


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 2:
        return 2
    try:
        result = execute(read_request(args[0], args[1]))
    except Exception as error:
        result = {'ok': False, 'errors': [{'phase': 'request', 'type': type(error).__name__[:64]}]}
    sys.stdout.write(json.dumps(result, sort_keys=True) + '\n')
    return 0 if result['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
