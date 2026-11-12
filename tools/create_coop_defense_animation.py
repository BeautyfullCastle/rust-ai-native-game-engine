#!/usr/bin/env python3
"""Create a new optional Stormglass web animation manifest from frame rectangles."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from pack_coop_defense_offline import (
    MAX_IMAGE, animation_names, publish_new, snapshot_file, validate_animations,
)


def animation_text(role, image, frames, clips=(), pivot=None):
    """Emit the existing role/FPS schema, leaving omitted sampler defaults intact."""
    definition = {"image": image, "clips": {}}
    for name, x, y, width, height in frames:
        definition["clips"].setdefault(name, {"frames": []})["frames"].append(
            [x, y, width, height])
    configured = set()
    for name, fps, mode in clips:
        if name in configured or name not in definition["clips"]:
            raise ValueError("Clip settings require a unique clip with authored frames")
        if mode not in ("loop", "once"):
            raise ValueError("Clip mode must be loop or once")
        configured.add(name)
        definition["clips"][name].update(fps=fps, loop=mode == "loop")
    if pivot is not None:
        definition["pivot"] = list(pivot)
    manifest = {role: definition}
    animation_names(manifest)  # Validate references before any filesystem lookup.
    return json.dumps(manifest, ensure_ascii=True, sort_keys=True, indent=2,
                      allow_nan=False) + "\n"


def create_animation(assets_dir, output, role, image, frames, clips=(), pivot=None):
    text = animation_text(role, image, frames, clips, pivot)
    # The image reference was validated above: no absolute/traversal components.
    # snapshot_file walks without symlinks and bounds the exact bytes admitted.
    images = {image: snapshot_file(Path(assets_dir) / image, MAX_IMAGE)}
    receipt = validate_animations(text.encode("utf-8"), images)
    publish_new(Path(output), text)  # Publish these exact admitted manifest bytes.
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets-dir", required=True, type=Path,
                        help="existing PNG asset root; symlinks are rejected")
    parser.add_argument("--output", required=True, type=Path,
                        help="new animations.json in an existing directory; never overwritten")
    parser.add_argument("--role", required=True, help="unit:0..5 or enemy:0..4")
    parser.add_argument("--image", required=True, help="relative PNG path under assets-dir")
    parser.add_argument("--frame", action="append", nargs=5, required=True,
                        metavar=("CLIP", "X", "Y", "W", "H"))
    parser.add_argument("--clip", action="append", nargs=3, default=[],
                        metavar=("CLIP", "FPS", "MODE"), help="MODE is loop or once")
    parser.add_argument("--pivot", nargs=2, type=float, metavar=("X", "Y"))
    args = parser.parse_args(argv)
    try:
        frames = [(name, *(float(v) for v in rect)) for name, *rect in args.frame]
        clips = [(name, float(fps), mode) for name, fps, mode in args.clip]
        receipt = create_animation(args.assets_dir, args.output, args.role, args.image,
                                   frames, clips, args.pivot)
    except (OSError, ValueError, subprocess.SubprocessError, ImportError) as error:
        print(f"Animation creation failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
