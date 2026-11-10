#!/usr/bin/env python3
"""Opt-in low-footprint launcher for the unchanged Linux-local baseline."""
import argparse
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys

GIB = 1024 ** 3


def preflight(target, minimum_gib):
    if minimum_gib < 1:
        raise ValueError("minimum free space must be at least 1 GiB")
    target = Path(target).expanduser()
    if not target.is_absolute():
        raise ValueError("target directory must be absolute and reused across runs")
    target = target.resolve()
    ancestor = target
    while not ancestor.exists():
        ancestor = ancestor.parent
    if not ancestor.is_dir():
        raise ValueError("target directory or its existing ancestor is not a directory")
    free = shutil.disk_usage(ancestor).free
    if free < minimum_gib * GIB:
        raise ValueError(f"only {free / GIB:.2f} GiB free; {minimum_gib} GiB required; no build started or files deleted")
    return target, free


def compact_environment(environment, target):
    result = dict(environment)
    result.update(CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0",
                  CARGO_PROFILE_DEV_DEBUG="1", CARGO_PROFILE_TEST_DEBUG="1")
    return result


def run(target, minimum_gib, root):
    if sys.platform != "linux":
        raise ValueError("this launcher requires Linux")
    gate = root / "tools/check-linux-local.sh"
    if not gate.is_file():
        raise ValueError("unchanged tools/check-linux-local.sh is required in this checkout")
    target, free = preflight(target, minimum_gib)
    target.mkdir(parents=True, exist_ok=True)
    # Serialize compact launchers sharing this target. Cargo retains its own lock.
    # Never remove the lock file: unlinking a held lock permits a second writer.
    with (target / ".orr-compact.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("another compact validation is using this target directory") from None
        target, free = preflight(target, minimum_gib)
        print(f"Compact local validation: target={target}; free={free / GIB:.2f} GiB", flush=True)
        print("CARGO_INCREMENTAL=0; dev/test debug=1; release profile unchanged. "
              "No test selection, optimization, assertions, or gate commands changed.", flush=True)
        result = subprocess.run(["bash", str(gate)], cwd=root,
                                env=compact_environment(os.environ, target),
                                pass_fds=(lock.fileno(),))
        return result.returncode if result.returncode >= 0 else 128 - result.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", required=True,
                        help="absolute, stable Cargo target directory; do not create one per attempt")
    parser.add_argument("--min-free-gib", type=int, default=8,
                        help="preflight reserve (default 8 GiB, minimum 1); not a build-size guarantee")
    args = parser.parse_args()
    try:
        return run(args.target_dir, args.min_free_gib, Path(__file__).resolve().parents[1])
    except (OSError, ValueError) as error:
        print(f"Compact validation refused: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
