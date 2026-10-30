#!/usr/bin/env python3
"""Author irregular atlas rectangles through the existing trusted Rust packer."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

# Allocation guards matching the current orr_sprite v1 resource ceilings.
# Semantic admission always belongs to SpriteDocument and the PNG packer.
MAX_JSON_BYTES = 4 * 1024 * 1024
MAX_REGIONS = 4096
MAX_CLIPS = 256
MAX_FRAMES = 4096
MAX_TOTAL_FRAMES = 16384


def integer(text):
    if len(text) > 21 or not text.lstrip("-").isascii() or not text.lstrip("-").isdigit():
        raise ValueError("expected a bounded decimal integer")
    return int(text)


def document_bytes(size, regions, clips):
    if len(regions) > MAX_REGIONS or len(clips) > MAX_CLIPS:
        raise ValueError("too many regions or clips")
    document = {"format": "orr_sprite", "version": 1,
                "atlas": {"image": "atlas.png", "width": integer(size[0]), "height": integer(size[1])},
                "regions": [], "clips": []}
    for region in regions:
        document["regions"].append(dict(zip(("id", "x", "y", "width", "height"), map(integer, region))))
    total = 0
    for name, mode, sequence in clips:
        count = sequence.count(",") + 1
        total += count
        if count > MAX_FRAMES or total > MAX_TOTAL_FRAMES:
            raise ValueError("too many frame references")
        frames = []
        for pair in sequence.split(","):
            parts = pair.split(":")
            if len(parts) != 2:
                raise ValueError("frames must be REGION:DURATION_MS pairs separated by commas")
            frames.append({"region": integer(parts[0]), "duration_ms": integer(parts[1])})
        document["clips"].append({"id": name, "mode": mode, "frames": frames})
    data = (json.dumps(document, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    if len(data) > MAX_JSON_BYTES:
        raise ValueError("sprite JSON exceeds 4 MiB")
    return data


def create_package(args):
    data = document_bytes(args.atlas_size, args.region, args.clip)
    # An explicit local path only: never PATH lookup, shell interpolation or build.
    packer = args.packer.absolute()
    if not packer.is_file() or not os.access(packer, os.X_OK):
        raise ValueError("--packer must name an existing trusted local executable")
    with tempfile.TemporaryDirectory(prefix="orr-sprite-rectangles-") as directory:
        document = Path(directory) / "sprites.json"
        document.write_bytes(data)
        result = subprocess.run([
            str(packer), "--png", str(args.png), "--json", str(document),
            "--license", str(args.license), "--name", args.name,
            "--version", args.version, "--out", str(args.out),
        ], check=False)
    # Never clean up the destination: the existing packer owns its publication.
    return result.returncode if result.returncode >= 0 else 1


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--packer", required=True, type=Path,
                        help="trusted prebuilt png_sprite_package executable; executes local code")
    parser.add_argument("--png", required=True, type=Path)
    parser.add_argument("--license", required=True, type=Path)
    parser.add_argument("--name", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True, type=Path,
                        help="new package directory under an existing trusted parent; never overwritten")
    parser.add_argument("--atlas-size", required=True, nargs=2, metavar=("WIDTH", "HEIGHT"))
    parser.add_argument("--region", action="append", default=[], nargs=5,
                        metavar=("ID", "X", "Y", "WIDTH", "HEIGHT"))
    parser.add_argument("--clip", action="append", default=[], nargs=3,
                        metavar=("ID", "MODE", "REGION:DURATION,..."))
    try:
        if sum(len(arg.encode("utf-8")) + 1 for arg in argv) > MAX_JSON_BYTES:
            raise ValueError("arguments exceed 4 MiB")
        args = parser.parse_args(argv)
        return create_package(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Sprite package creation failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
