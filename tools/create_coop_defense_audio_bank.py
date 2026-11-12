#!/usr/bin/env python3
"""Create an opt-in CoopDefense bank from explicit user-owned cooked PCM files."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct

from pack_coop_defense_audio import CUES, snapshot_audio
from pack_coop_defense_offline import open_directory, snapshot_file, strict_json

MAX_PCM = 8 + 48000 * 2
MAX_LICENSE = 64 * 1024


def checked_path(value):
    path = Path(value)
    if '..' in path.parts:
        raise ValueError('Parent traversal is not allowed')
    return path


def create_bank(cue_inputs, license_path, output):
    """Snapshot/validate first; publish a new directory, never replace an output.

    Linux descriptor-relative no-follow IO is inherited from the existing packer.
    Trusted/quiescent directories are required; this is not crash-atomic publication.
    """
    output = checked_path(output)
    if output.name in ('', '.', '..'):
        raise ValueError('Output must name a new directory')
    sources = {}
    for assignment in cue_inputs:
        cue, separator, filename = assignment.partition('=')
        if not separator or cue not in CUES or cue in sources or not filename:
            raise ValueError('Each supported cue must be supplied exactly once as NAME=PCM')
        sources[cue] = checked_path(filename)
    if set(sources) != set(CUES):
        raise ValueError('Exactly all ten supported cues are required')
    license_bytes = snapshot_file(checked_path(license_path), MAX_LICENSE)
    if not license_bytes.decode('utf-8').strip():
        raise ValueError('License must be nonempty UTF-8 text')
    objects, records = {}, []
    for index, cue in enumerate(CUES):
        data = snapshot_file(sources[cue], MAX_PCM)
        if len(data) < 8:
            raise ValueError('PCM header is truncated')
        _, frames = struct.unpack_from('<II', data)
        records.append({'cue': cue, 'assetId': f'a_000000005444{index + 1:04x}',
                        'file': cue + '.pcm', 'frames': frames, 'bytes': len(data),
                        'sha256': hashlib.sha256(data).hexdigest()})
        objects[cue + '.pcm'] = data
    manifest = {'version': 1, 'scope': 'user-authored-coop-defense-bank',
                'sampleRate': 48000, 'cues': records}
    objects['bank.json'] = (json.dumps(manifest, indent=2) + '\n').encode('utf-8')
    # Use the unchanged production admission before creating any output.
    def snapshot_memory(path, limit):
        data = objects[Path(path).name]
        if len(data) > limit:
            raise ValueError('Input exceeds byte limit')
        return data
    snapshot_audio(Path('bank.json'), snapshot_memory, strict_json)
    objects['LICENSE.txt'] = license_bytes

    parent_fd = open_directory(output.parent)
    directory_fd = None
    created = False
    written = []
    try:
        os.mkdir(output.name, dir_fd=parent_fd)
        created = True
        directory_fd = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                               dir_fd=parent_fd)
        identity = os.fstat(directory_fd)
        for name, data in objects.items():
            fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o644, dir_fd=directory_fd)
            written.append(name)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
        os.fsync(directory_fd)
        os.fsync(parent_fd)
    except BaseException as error:
        # Only our exclusively created files, never recursively traverse anything.
        cleanup_error = None
        if directory_fd is not None:
            try:
                for name in written:
                    os.unlink(name, dir_fd=directory_fd)
                current = os.stat(output.name, dir_fd=parent_fd, follow_symlinks=False)
                if (current.st_dev, current.st_ino) != (identity.st_dev, identity.st_ino):
                    raise OSError('Output directory identity changed; refusing cleanup')
                os.rmdir(output.name, dir_fd=parent_fd)
            except OSError as cleanup:
                cleanup_error = cleanup
        elif created:
            # If opening the new directory failed, its identity is not established.
            cleanup_error = OSError('New output could not be opened; inspect partial output')
        if cleanup_error is not None:
            raise RuntimeError(f'{error}; cleanup incomplete: {cleanup_error}') from error
        raise
    finally:
        if directory_fd is not None:
            os.close(directory_fd)
        os.close(parent_fd)
    return output


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cue', action='append', required=True, metavar='NAME=PCM',
                        help='repeat once for every cue: ' + ', '.join(CUES))
    parser.add_argument('--license', required=True, help='explicit user-supplied UTF-8 license')
    parser.add_argument('--output', required=True, help='new output directory; parent must exist')
    args = parser.parse_args(argv)
    try:
        output = create_bank(args.cue, args.license, args.output)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f'Audio bank creation failed: {error}\n')
    print(output)


if __name__ == '__main__':
    main()
