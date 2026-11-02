#!/usr/bin/env python3
"""Package an offline animation sequence through existing strict native tools."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

from create_assembled_model_package import (
    MAX_LICENSE, MAX_MODEL, MAX_RECIPE, checked_path, publish, run_tool, trusted_executable,
)
from pack_coop_defense_offline import open_directory, snapshot_file


def create_package(recipe, asset_id, license_path, name, version, engine, output, sequencer, package_source):
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
    recipe_bytes = snapshot_file(recipe, MAX_RECIPE)
    license_bytes = snapshot_file(checked_path(license_path), MAX_LICENSE)
    if not license_bytes.decode('utf-8').strip():
        raise ValueError('License must be nonempty user-provided UTF-8 text')
    sequencer, package_source = trusted_executable(sequencer), trusted_executable(package_source)
    for value in (asset_id, name, version, engine):
        if not isinstance(value, str) or not value or len(value.encode('utf-8')) > 4096 or '\x00' in value:
            raise ValueError('Metadata arguments must contain 1..4096 UTF-8 bytes')
    with tempfile.TemporaryDirectory(prefix='orr-sequence-package-') as directory:
        stage = Path(directory)
        # The existing sequencer alone resolves/admit sources and constructs clips.
        run_tool([str(sequencer), '--recipe', str(recipe), '--asset-id', asset_id,
                  '--output', str(stage / 'model.animated.json')])
        if snapshot_file(recipe, MAX_RECIPE) != recipe_bytes:
            raise ValueError('Recipe changed during sequence authoring')
        (stage / 'LICENSE.txt').write_bytes(license_bytes)
        run_tool([str(package_source), '--source', str(stage), '--name', name,
                  '--version', version, '--engine', engine, '--capability', 'models',
                  '--capability', 'animation', '--file', 'model.animated.json', '--file', 'LICENSE.txt'])
        expected = {'model.animated.json': MAX_MODEL, 'LICENSE.txt': MAX_LICENSE,
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
    for flag in ('recipe', 'asset-id', 'license', 'name', 'version', 'engine', 'output', 'sequencer', 'package-source'):
        parser.add_argument('--' + flag, required=True)
    args = parser.parse_args()
    try:
        print(create_package(args.recipe, args.asset_id, args.license, args.name, args.version,
                             args.engine, args.output, args.sequencer, args.package_source))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Animation sequence package creation failed: {error}\n')


if __name__ == '__main__':
    main()
