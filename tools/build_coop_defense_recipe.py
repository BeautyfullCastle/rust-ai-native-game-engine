#!/usr/bin/env python3
"""Build one optional CoopDefense delivery from a portable, closed recipe."""
import argparse
import os
from pathlib import Path
import stat
import subprocess

from pack_coop_defense_offline import open_directory, package, snapshot_file, strict_json

MAX_RECIPE = 8192
FIELDS = {'format', 'version', 'build', 'scene', 'animations', 'audio'}


def recipe_path(root, value, directory=False):
    """Admit canonical relative components without resolving symlink aliases."""
    if (not isinstance(value, str) or not value or len(value) > 1024
            or '\\' in value or ':' in value or '\x00' in value
            or any(part in ('', '.', '..') for part in value.split('/'))):
        raise ValueError('Recipe paths must be nonempty canonical relative paths')
    path = root / value
    # open_directory checks every ancestor from / without following symlinks.
    parent = open_directory(path if directory else path.parent)
    try:
        if not directory:
            info = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
            if not stat.S_ISREG(info.st_mode):
                raise ValueError('Recipe input must be a regular non-symlink file')
    finally:
        os.close(parent)
    return path


def build_recipe(recipe, output):
    """No build execution: retain the existing packer's full input admission."""
    recipe = Path(os.path.abspath(recipe))
    data = strict_json(snapshot_file(recipe, MAX_RECIPE))
    if (not isinstance(data, dict) or set(data) - FIELDS
            or not {'format', 'version', 'build'} <= set(data)
            or data.get('format') != 'orr_coop_project'
            or type(data.get('version')) is not int or data['version'] != 1):
        raise ValueError('Expected closed orr_coop_project version 1 recipe')
    root = recipe.parent
    build = recipe_path(root, data['build'], directory=True)
    optional = {key: recipe_path(root, data[key]) for key in ('scene', 'animations', 'audio') if key in data}
    # Explicit output always selects the packer's exclusive hard-link publisher.
    return package(build, output=Path(output), **optional)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('recipe', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    try:
        print(build_recipe(args.recipe, args.output))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Recipe build failed: {error}\n')


if __name__ == '__main__':
    main()
