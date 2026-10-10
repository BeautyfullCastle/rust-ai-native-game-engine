#!/usr/bin/env python3
"""Lint repository-owned simulation libraries without imposing float bans on views.

Run with Python 3.11+ and Rust/Clippy installed:
    python tools/check_sim_float_types.py
"""

import os
from pathlib import Path
import subprocess
import tomllib


ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "tools/sim-float-guard"
PACKAGES = (
    "orr_fp", "orr_ecs", "orr_sim", "orr_session", "orr_testgame",
    "orr_physics", "orr_physics3d", "orr_games", "orr_asset",
)


def check_config():
    """Keep the extra guard at least as strict as the workspace type policy."""
    def types(path):
        config = tomllib.loads(path.read_text(encoding="utf-8"))
        return {
            entry if isinstance(entry, str) else entry["path"]
            for entry in config["disallowed-types"]
        }

    required = types(ROOT / "clippy.toml") | {"f32", "f64"}
    missing = required - types(CONFIG / "clippy.toml")
    if missing:
        raise RuntimeError(f"simulation guard is missing disallowed types: {sorted(missing)}")


def main():
    check_config()
    env = os.environ.copy()
    env["CLIPPY_CONF_DIR"] = str(CONFIG)
    # Do not inherit a caller's sccache wrapper: these lint invocations must
    # inspect this checkout, including the currently selected configuration.
    env.pop("RUSTC_WRAPPER", None)
    common = ["cargo", "clippy", "--lib"]
    packages = [arg for name in PACKAGES if name != "orr_fp" for arg in ("-p", name)]
    # Forbid local lint suppression in simulation code with no float exception.
    commands = [
        common + packages + ["--", "-F", "clippy::disallowed_types"],
        common + ["-p", "orr_fp", "--", "-F", "clippy::disallowed_types"],
        # Workspace view builds unify this feature into orr_fp. Exercise the
        # exact, narrow conversion exception while checking the rest of FP too.
        common + ["-p", "orr_fp", "--features", "float-interop,serde", "--",
                  "-D", "clippy::disallowed_types"],
    ]
    for command in commands:
        print("+ " + " ".join(command), flush=True)
        subprocess.run(command, cwd=ROOT, env=env, check=True)
    print("Simulation float-type guard passed (9 libraries; FP interop also checked).")


if __name__ == "__main__":
    main()
