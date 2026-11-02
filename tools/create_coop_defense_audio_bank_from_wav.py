#!/usr/bin/env python3
"""Bridge explicit user WAV cues through the existing strict native importer."""
import argparse
import hashlib
from itertools import islice
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile

from create_coop_defense_audio_bank import MAX_LICENSE, MAX_PCM, checked_path, create_bank
from pack_coop_defense_audio import CUES
from pack_coop_defense_offline import open_directory, snapshot_file

MAX_WAV = 1_048_576
IMPORT_TIMEOUT_SECONDS = 30


def create_from_wav(cue_inputs, license_path, output, importer):
    """Decode all cues privately, then delegate exclusive bank publication.

    The supplied importer must be the trusted local wav_audio_package executable.
    This is not a sandbox for arbitrary executables or hostile directory owners.
    """
    output = checked_path(output)
    if output.name in ('', '.', '..'):
        raise ValueError('Output must name a new directory')
    parent_fd = open_directory(output.parent)
    try:
        try:
            os.stat(output.name, dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError('Output already exists; choose a new directory')
    finally:
        os.close(parent_fd)
    sources = {}
    for assignment in cue_inputs:
        cue, separator, filename = assignment.partition('=')
        if not separator or cue not in CUES or cue in sources or not filename:
            raise ValueError('Each supported cue must be supplied exactly once as NAME=WAV')
        sources[cue] = checked_path(filename)
    if set(sources) != set(CUES):
        raise ValueError('Exactly all ten supported cues are required')
    license_bytes = snapshot_file(checked_path(license_path), MAX_LICENSE)
    if not license_bytes.decode('utf-8').strip():
        raise ValueError('License must be nonempty UTF-8 text')
    wavs = {cue: snapshot_file(sources[cue], MAX_WAV) for cue in CUES}
    executable = checked_path(importer).resolve(strict=True)
    if not stat.S_ISREG(executable.stat().st_mode) or not os.access(executable, os.X_OK):
        raise ValueError('Importer must be a trusted local executable file')

    with tempfile.TemporaryDirectory(prefix='orr-wav-bank-') as directory:
        scratch = Path(directory)
        license_copy = scratch / 'LICENSE.txt'
        license_copy.write_bytes(license_bytes)
        cooked_inputs = []
        for index, cue in enumerate(CUES):
            source = scratch / (cue + '.wav')
            source.write_bytes(wavs[cue])
            package = scratch / ('package-' + cue)
            # No shell or PATH lookup; discard bounded-tool diagnostics instead of
            # buffering arbitrary subprocess output. No final output exists yet.
            try:
                result = subprocess.run(
                    [str(executable), '--wav', str(source), '--asset-id',
                     f'a_000000005444{index + 1:04x}', '--name', 'wav-' + cue,
                     '--version', '1.0.0', '--license', str(license_copy),
                     '--out', str(package)],
                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL, timeout=IMPORT_TIMEOUT_SECONDS,
                    check=False)
            except subprocess.TimeoutExpired as error:
                raise RuntimeError(f'WAV importer timed out for cue {cue}') from error
            if result.returncode:
                raise ValueError(f'Strict WAV importer rejected cue {cue} (exit {result.returncode})')
            # Recheck the native result as bounded regular PCM; never trust stdout
            # paths. The existing bank admission validates its header and samples.
            objects = package / 'cooked' / 'objects'
            fd = open_directory(objects)
            try:
                with os.scandir(fd) as entries:
                    names = [entry.name for entry in islice(entries, 2)]
            finally:
                os.close(fd)
            if len(names) != 1 or not re.fullmatch(r'[0-9a-f]{64}\.bin', names[0]):
                raise ValueError(f'Unexpected cooked object set for cue {cue}')
            payload = objects / names[0]
            data = snapshot_file(payload, MAX_PCM)
            if hashlib.sha256(data).hexdigest() + '.bin' != names[0]:
                raise ValueError(f'Cooked object digest mismatch for cue {cue}')
            cooked_inputs.append(f'{cue}={payload}')
        # Existing publisher alone owns final output creation/collision/cleanup.
        return create_bank(cooked_inputs, license_copy, output)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cue', action='append', required=True, metavar='NAME=WAV',
                        help='repeat once per cue: ' + ', '.join(CUES))
    parser.add_argument('--license', required=True, help='explicit user-supplied UTF-8 license')
    parser.add_argument('--output', required=True, help='new bank directory; parent must exist')
    parser.add_argument('--importer', required=True,
                        help='trusted local wav_audio_package executable, not an arbitrary program')
    args = parser.parse_args(argv)
    try:
        output = create_from_wav(args.cue, args.license, args.output, args.importer)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f'WAV bank creation failed: {error}\n')
    print(output)


if __name__ == '__main__':
    main()
