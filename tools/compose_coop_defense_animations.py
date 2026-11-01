#!/usr/bin/env python3
"""Compose role manifests into one optional Stormglass web animation manifest."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from pack_coop_defense_offline import (
    MAX_ANIMATION, MAX_IMAGE, MAX_IMAGES_TOTAL, animation_names, publish_new,
    snapshot_file, strict_json, validate_animations,
)


def compose_animations(assets_dir, output, manifests):
    """Keep image paths relative to one explicit asset root; never replace roles."""
    manifests = list(manifests)
    if not 1 <= len(manifests) <= 11:
        raise ValueError("Composition requires 1..11 input manifests")
    combined, sources = {}, []
    for path in manifests:
        data = snapshot_file(Path(path), MAX_ANIMATION)
        manifest = strict_json(data)
        animation_names(manifest)  # Validate each input before image path lookup.
        if combined.keys() & manifest.keys():
            raise ValueError("Duplicate animation role across manifests")
        combined.update(manifest)
        sources.append({"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    names = animation_names(combined)
    text = json.dumps(combined, ensure_ascii=True, sort_keys=True,
                      separators=(",", ":"), allow_nan=False) + "\n"
    if len(text.encode("utf-8")) > MAX_ANIMATION:
        raise ValueError("Composed animation manifest exceeds 64 KiB")
    images, total = {}, 0
    for name in names:
        image = snapshot_file(Path(assets_dir) / name, MAX_IMAGE)
        total += len(image)
        if total > MAX_IMAGES_TOTAL:
            raise ValueError("Animation images exceed total byte limit")
        images[name] = image
    receipt = validate_animations(text.encode("utf-8"), images)
    publish_new(Path(output), text)
    return {**receipt, "roles": sorted(combined), "sources": sources}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets-dir", required=True, type=Path,
                        help="one common PNG root; references are not rebased")
    parser.add_argument("--output", required=True, type=Path,
                        help="new manifest in an existing directory; never overwritten")
    parser.add_argument("--manifest", action="append", required=True, type=Path,
                        help="input manifest, repeat for each file (maximum 11)")
    args = parser.parse_args(argv)
    try:
        receipt = compose_animations(args.assets_dir, args.output, args.manifest)
    except (OSError, ValueError, subprocess.SubprocessError, ImportError) as error:
        print(f"Animation composition failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
