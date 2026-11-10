#!/usr/bin/env python3
"""Create a new Room ActionMap v1 sidecar without modifying a project manifest."""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import string
import sys
import tempfile

ACTIONS = ("move_left", "move_right", "move_forward", "move_backward", "interact")
KEYS = frozenset(string.ascii_lowercase) | {
    "arrow_left", "arrow_right", "arrow_up", "arrow_down", "space", "escape", "enter", "tab"
}
RESERVED = {"r", "escape"}
RESERVED_FILENAMES = {"orr.project.json", "orr.packages.lock.json"}
MAX_BYTES = 65536


def encode(bindings):
    """Match orr_input ActionMap and orr_sample RoomControls, not logical keys."""
    if set(bindings) != set(ACTIONS):
        raise ValueError("exactly the five Room actions are required")
    seen = set()
    actions = []
    for name in ACTIONS:
        value = bindings[name]
        if not isinstance(value, str) or not 1 <= len(value) <= 128:
            raise ValueError(f"{name}: expected 1..128 characters")
        keys = value.split(",")
        if not 1 <= len(keys) <= 8:
            raise ValueError(f"{name}: expected 1..8 comma-separated physical keys")
        for key in keys:
            if key not in KEYS:
                raise ValueError(f"{name}: unsupported physical key {key!r}")
            if key in RESERVED:
                raise ValueError("r and escape are reserved for restart and exit")
            if key in seen:
                raise ValueError(f"duplicate physical key {key!r}")
            seen.add(key)
        actions.append({"name": name, "bindings": [
            {"device": "Keyboard", "key": key} for key in sorted(keys)
        ]})
    data = (json.dumps({"version": 1, "actions": actions}, indent=2) + "\n").encode("ascii")
    if len(data) > MAX_BYTES:
        raise ValueError("input map exceeds 64 KiB")
    return data


def publish_new(output, data):
    """Stage, sync, hard-link without replacement, clean up. No fallback overwrite."""
    if not isinstance(output, str) or not 1 <= len(output) <= 4096:
        raise ValueError("output path must be 1..4096 characters")
    if not isinstance(data, bytes) or not 1 <= len(data) <= MAX_BYTES:
        raise ValueError("output must be 1..65536 bytes")
    path = Path(output)
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,122}\.json", path.name):
        raise ValueError("output needs a simple ASCII .json filename (at most 128 characters)")
    if path.name.lower() in RESERVED_FILENAMES:
        raise ValueError("output filename is reserved for a project manifest or lock")
    # Reject symlink parents before staging; concurrent hostile directory replacement
    # is outside this local authoring tool's filesystem boundary.
    parent = path.absolute().parent
    for component in (*reversed(parent.parents), parent):
        if not stat.S_ISDIR(component.lstat().st_mode):
            raise ValueError("output parent must be an existing directory without symlinks")
    if os.path.lexists(path):
        raise FileExistsError("output already exists; choose a new filename")
    fd, staging = tempfile.mkstemp(prefix=".orr-room-controls-", suffix=".tmp", dir=parent)
    published = False
    failure = None
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(staging, path)
        published = True
    except OSError as error:
        failure = error
        raise
    finally:
        try:
            os.unlink(staging)
        except OSError as error:
            if published:
                raise OSError("output was published, but staging cleanup failed") from error
            if failure is not None:
                raise OSError(f"{failure}; staging cleanup also failed: {error}") from failure
            raise


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--output", required=True, action="append", help="new JSON sidecar; parent must exist")
    for action in ACTIONS:
        parser.add_argument("--" + action.replace("_", "-"), required=True, action="append",
                            help="1..8 exact lowercase physical keys, comma-separated")
    if len(argv) > 12 or any(len(arg) > 4096 for arg in argv):
        parser.error("too many or oversized arguments")
    args = vars(parser.parse_args(argv))
    if any(len(values) != 1 for values in args.values()):
        parser.error("each option must occur exactly once")
    output = args.pop("output")[0]
    try:
        data = encode({name: values[0] for name, values in args.items()})
        publish_new(output, data)
    except (OSError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    print(f"Created {output} ({len(data)} bytes).")
    print(f'Explicitly set entry.controls to "{Path(output).name}" in orr.project.json,')
    print("with this sidecar in the project root. The manifest was NOT changed.")
    print("Runtime/export require the optional orr_sample feature room-input-actions.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
