#!/usr/bin/env python3
"""Copy the existing offline packer's web inputs into a generated build directory."""
import argparse
from pathlib import Path
import shutil
import sys

from pack_coop_defense_offline import ASSETS


def copy_assets(source, output):
    """Keep served and offline builds aligned; Cargo supplies game.wasm separately.

    This retains the build script's replace-in-build-directory semantics, not an
    atomic release publisher. Extra output files are not removed. Repository and
    generated directories must be trusted and quiescent, as in the build script.
    """
    source, output = Path(source), Path(output)
    required = tuple(name for name in ASSETS if name != "game.wasm")
    # Detect incomplete source trees before creating or modifying generated files.
    for name in required:
        if not (source / name).is_file():
            raise FileNotFoundError(f"Required CoopDefense web source is missing: {name}")
    output.mkdir(parents=True, exist_ok=True)
    for name in required:
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / name, destination)
    # Preserve optional authored atlas/metadata/license files served by the web
    # build, not just the fixed assets needed by the default offline package.
    shutil.copytree(source / "assets", output / "assets", dirs_exist_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args(argv)
    source = Path(__file__).resolve().parents[1] / "crates/orr_coop_defense_web/web"
    try:
        copy_assets(source, args.output)
    except (OSError, shutil.Error) as error:
        print(f"CoopDefense web asset copy failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
