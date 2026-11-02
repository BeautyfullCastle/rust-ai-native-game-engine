#!/usr/bin/env python3
"""Create an optional static-model package using the existing native authoring tools."""
import argparse
import os
from pathlib import Path
import stat
import subprocess
import tempfile

from pack_coop_defense_offline import open_directory, snapshot_file

MAX_RECIPE = 16 * 1024
MAX_MODEL = 64 * 1024 * 1024
MAX_LICENSE = 1024 * 1024
TIMEOUT = 60


def checked_path(value):
    path = Path(value)
    if '..' in path.parts or '.orr' in path.parts:
        raise ValueError('Traversal and internal .orr store paths are not supported')
    return Path(os.path.abspath(path))


def trusted_executable(value):
    path = checked_path(value)
    parent = open_directory(path.parent)
    try:
        info = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        if not stat.S_ISREG(info.st_mode) or not os.access(path, os.X_OK):
            raise ValueError('Expected an explicit trusted native executable')
    finally:
        os.close(parent)
    return path


def run_tool(arguments):
    result = subprocess.run(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, shell=False, timeout=TIMEOUT, check=False)
    if result.returncode:
        raise ValueError(f'Native {Path(arguments[0]).name} rejected input (exit {result.returncode})')


def publish(output, objects):
    parent = open_directory(output.parent)
    directory = None
    created = False
    written = []
    try:
        os.mkdir(output.name, dir_fd=parent)
        created = True
        directory = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
        identity = os.fstat(directory)
        for name, data in objects.items():
            fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o644, dir_fd=directory)
            written.append(name)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
        os.fsync(directory)
        os.fsync(parent)
    except BaseException as error:
        cleanup_error = None
        if directory is not None:
            try:
                for name in written:
                    os.unlink(name, dir_fd=directory)
                current = os.stat(output.name, dir_fd=parent, follow_symlinks=False)
                if (current.st_dev, current.st_ino) != (identity.st_dev, identity.st_ino):
                    raise OSError('Output identity changed; refusing directory cleanup')
                os.rmdir(output.name, dir_fd=parent)
            except OSError as cleanup:
                cleanup_error = cleanup
        elif created:
            cleanup_error = OSError('Created directory could not be opened; inspect partial output')
        if cleanup_error is not None:
            raise RuntimeError(f'{error}; cleanup incomplete: {cleanup_error}') from error
        raise
    finally:
        if directory is not None:
            os.close(directory)
        os.close(parent)


def create_package(recipe, asset_id, license_path, name, version, engine, output, assembler, package_source):
    output = checked_path(output)
    parent = open_directory(output.parent)
    try:
        try:
            os.stat(output.name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise FileExistsError('Output already exists; choose a new directory')
    finally:
        os.close(parent)
    recipe = checked_path(recipe)
    # Native assembler remains the sole recipe/model decoder and part resolver.
    recipe_bytes = snapshot_file(recipe, MAX_RECIPE)
    license_bytes = snapshot_file(checked_path(license_path), MAX_LICENSE)
    if not license_bytes.decode('utf-8').strip():
        raise ValueError('License must be nonempty user-provided UTF-8 text')
    assembler, package_source = trusted_executable(assembler), trusted_executable(package_source)
    for value in (asset_id, name, version, engine):
        if not isinstance(value, str) or not value or len(value.encode('utf-8')) > 4096 or '\x00' in value:
            raise ValueError('Metadata arguments must contain 1..4096 UTF-8 bytes')
    with tempfile.TemporaryDirectory(prefix='orr-assembled-package-') as directory:
        stage = Path(directory)
        run_tool([str(assembler), '--recipe', str(recipe), '--asset-id', asset_id,
                  '--output', str(stage / 'model.model.json')])
        if snapshot_file(recipe, MAX_RECIPE) != recipe_bytes:
            raise ValueError('Recipe changed during assembly')
        (stage / 'LICENSE.txt').write_bytes(license_bytes)
        run_tool([str(package_source), '--source', str(stage), '--name', name,
                  '--version', version, '--engine', engine, '--capability', 'models',
                  '--file', 'model.model.json', '--file', 'LICENSE.txt'])
        expected = {'model.model.json': MAX_MODEL, 'LICENSE.txt': MAX_LICENSE,
                    'orr.package.json': 1024 * 1024}
        if {p.name for p in stage.iterdir()} != set(expected):
            raise ValueError('Unexpected native package output')
        objects = {key: snapshot_file(stage / key, limit) for key, limit in expected.items()}
        if objects['LICENSE.txt'] != license_bytes:
            raise ValueError('Native tool changed supplied license')
        publish(output, objects)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('recipe', 'asset-id', 'license', 'name', 'version', 'engine', 'output', 'assembler', 'package-source'):
        parser.add_argument('--' + flag, required=True)
    args = parser.parse_args()
    try:
        print(create_package(args.recipe, args.asset_id, args.license, args.name, args.version,
                             args.engine, args.output, args.assembler, args.package_source))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Assembled package creation failed: {error}\n')


if __name__ == '__main__':
    main()
