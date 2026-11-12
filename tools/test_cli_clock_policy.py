#!/usr/bin/env python3
"""Verify the CLI-only monotonic clock exception without relaxing simulation.

Run --config-only for the structural checks without Cargo. Full fixtures use
real Clippy from a workspace root, with a CLI package depending on a strict peer.
Generated sources and raw diagnostics are retained under CARGO_TARGET_DIR.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
INSTANT = "std::time::Instant"
RETAINED = ("std::collections::HashMap", "std::collections::HashSet", "std::time::SystemTime")
REQUIRED = {INSTANT, *RETAINED}


def paths(config):
    entries = config["disallowed-types"]
    result = [entry if isinstance(entry, str) else entry["path"] for entry in entries]
    if len(result) != len(set(result)):
        raise ValueError("duplicate disallowed type")
    return set(result)


def check_policy(workspace, cli, simulation):
    """Fail closed on missing bans, changed entries/options or stale CLI copies."""
    if not REQUIRED <= paths(workspace):
        raise ValueError("workspace must retain all four deterministic type bans")
    expected = deepcopy(workspace)
    expected["disallowed-types"] = [
        entry for entry in expected["disallowed-types"]
        if (entry if isinstance(entry, str) else entry["path"]) != INSTANT
    ]
    if cli != expected:
        raise ValueError("CLI policy must equal the full workspace policy minus only Instant")
    if not (paths(workspace) | {"f32", "f64"}) <= paths(simulation):
        raise ValueError("simulation policy must retain workspace bans and both float types")


def structural_checks():
    configs = [tomllib.loads(path.read_text(encoding="utf-8")) for path in (
        ROOT / "clippy.toml", ROOT / "crates/orr_cli/clippy.toml",
        ROOT / "tools/sim-float-guard/clippy.toml",
    )]
    check_policy(*configs)

    def reject(label, values):
        try:
            check_policy(*values)
        except ValueError:
            print("PASS structural:", label, flush=True)
        else:
            raise AssertionError("weakened policy accepted: " + label)

    for banned in REQUIRED:
        values = deepcopy(configs)
        # Even coordinated deletion from both copied policies cannot lower the floor.
        for config in values[:2]:
            config["disallowed-types"] = [entry for entry in config["disallowed-types"]
                                          if entry["path"] != banned]
        reject("root floor " + banned, values)
    for banned in RETAINED:
        values = deepcopy(configs)
        values[1]["disallowed-types"] = [entry for entry in values[1]["disallowed-types"]
                                         if entry["path"] != banned]
        reject("CLI retains " + banned, values)
    values = deepcopy(configs)
    values[1]["disallowed-types"][0]["reason"] = "changed"
    reject("full entries, not only names", values)
    values = deepcopy(configs)
    values[1]["disallowed-types"].append({"path": INSTANT})
    reject("exception is precisely Instant", values)
    values = deepcopy(configs)
    added = {"path": "std::collections::BinaryHeap", "reason": "future policy fixture"}
    values[0]["disallowed-types"].append(added)
    reject("new workspace ban requires CLI propagation", values)
    values[1]["disallowed-types"].append(deepcopy(added))
    reject("new workspace ban requires simulation propagation", values)
    values[2]["disallowed-types"].append(deepcopy(added))
    check_policy(*values)
    values = deepcopy(configs)
    values[0]["msrv"] = "1.85"
    reject("new workspace option requires CLI propagation", values)
    for banned in REQUIRED | {"f32", "f64"}:
        values = deepcopy(configs)
        values[2]["disallowed-types"] = [entry for entry in values[2]["disallowed-types"]
                                         if entry["path"] != banned]
        reject("simulation retains " + banned, values)
    return configs


def type_name(path):
    return path + ({"std::collections::HashMap": "<u32, u32>",
                    "std::collections::HashSet": "<u32>"}.get(path, ""))


def fixtures():
    cargo = shutil.which("cargo")
    if cargo is None:
        raise RuntimeError("cargo was not found on PATH")
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    target.mkdir(parents=True, exist_ok=True)
    evidence = Path(tempfile.mkdtemp(prefix="cli-clock-policy-", dir=target))
    print("Fixture evidence:", evidence, flush=True)
    env = os.environ.copy()
    for key in ("CLIPPY_CONF_DIR", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        env.pop(key, None)
    env["CARGO_NET_OFFLINE"] = "true"
    cases = [("cli-instant", INSTANT, None, False, False, False),
             ("default-peer-instant", INSTANT, INSTANT, False, False, False),
             ("peer-from-cli-cwd", INSTANT, INSTANT, False, False, True)]
    cases += [("cli-" + path.rsplit("::", 1)[1], path, None, False, False, False) for path in RETAINED]
    cases += [("peer-" + path.rsplit("::", 1)[1], INSTANT, path, False, False, False) for path in RETAINED]
    cases += [("simulation-" + path.rsplit("::", 1)[1], None, path, True, False, False)
              for path in (INSTANT, *RETAINED)]
    cases += [("simulation-forbid-local-allow", None, INSTANT, True, True, False),
              ("simulation-config-overrides-cli", INSTANT, None, True, False, False)]
    for label, cli_type, peer_type, sim_config, local_allow, cli_cwd in cases:
        work = evidence / label
        work.mkdir()
        (work / "Cargo.toml").write_text('[workspace]\nresolver = "2"\nmembers = ["cli", "peer"]\n', encoding="utf-8")
        shutil.copyfile(ROOT / "clippy.toml", work / "clippy.toml")
        (work / "simulation").mkdir()
        shutil.copyfile(ROOT / "tools/sim-float-guard/clippy.toml", work / "simulation/clippy.toml")
        for package, path in (("cli", cli_type), ("peer", peer_type)):
            directory = work / package
            (directory / "src").mkdir(parents=True)
            manifest = ('[package]\nname = "clock-policy-' + package + '"\nversion = "0.0.0"\nedition = "2021"\n')
            if package == "cli":
                manifest += '[dependencies]\nclock-policy-peer = { path = "../peer" }\n'
            (directory / "Cargo.toml").write_text(manifest, encoding="utf-8")
            selected_type = type_name(path) if path else "std::time::Duration"
            source = "pub fn value() -> Option<" + selected_type + "> { None }\n"
            if local_allow and package == "peer":
                source = "#![allow(clippy::disallowed_types)]\n" + source
            (directory / "src/lib.rs").write_text(source, encoding="utf-8")
        shutil.copyfile(ROOT / "crates/orr_cli/clippy.toml", work / "cli/clippy.toml")
        case_env = dict(env, CARGO_TARGET_DIR=str(work / "target"))
        cwd = work / "cli" if cli_cwd else work
        common = ["--offline", "--workspace", "--all-targets", "--manifest-path", str(work / "Cargo.toml")]

        def run(step, command):
            print("+", label, subprocess.list2cmdline(command), flush=True)
            result = subprocess.run(command, cwd=cwd, env=case_env, text=True,
                                    encoding="utf-8", errors="replace", capture_output=True, check=False)
            (work / (step + ".command.json")).write_text(json.dumps({"command": command, "cwd": str(cwd),
                "simulation_config": sim_config if step == "clippy" else False, "exit_code": result.returncode}, indent=2), encoding="utf-8")
            (work / (step + ".stdout.log")).write_text(result.stdout, encoding="utf-8")
            (work / (step + ".stderr.log")).write_text(result.stderr, encoding="utf-8")
            return result

        compiled = run("check", [cargo, "check", *common])
        if compiled.returncode:
            raise AssertionError(f"{label}: fixture must compile before linting; see {work}")
        if sim_config:
            case_env["CLIPPY_CONF_DIR"] = str(work / "simulation")
        flags = ["-F", "clippy::disallowed_types"] if sim_config else ["-D", "warnings"]
        result = run("clippy", [cargo, "clippy", *common, "--message-format=json", "--", *flags])
        diagnostics = [record["message"] for line in result.stdout.splitlines()
                       if (record := json.loads(line)).get("reason") == "compiler-message"]
        errors = [item for item in diagnostics if item["level"] == "error"]
        expected_package = "peer" if peer_type else "cli"
        should_fail = bool(peer_type or cli_type in RETAINED or sim_config)
        expected_path = work / expected_package / "src/lib.rs"
        # Cargo diagnostics are workspace-relative, even when invoked below it.
        expected_line = 2 if local_allow else 1
        matching = [item for item in errors if (item.get("code") or {}).get("code") == "clippy::disallowed_types"]
        if not should_fail:
            if result.returncode or errors or matching:
                raise AssertionError(f"{label}: allowed CLI Instant must pass; see {work}")
        else:
            allowed_codes = {"clippy::disallowed_types", "E0453"} if local_allow else {"clippy::disallowed_types"}
            if result.returncode == 0 or (not local_allow and not matching) or any((item.get("code") or {}).get("code") not in allowed_codes for item in errors):
                raise AssertionError(f"{label}: expected only policy errors; see {work}")
            for item in matching:
                primary = [span for span in item["spans"] if span["is_primary"]]
                if not any((work / span["file_name"]).resolve() == expected_path.resolve()
                           and span["line_start"] == expected_line for span in primary):
                    raise AssertionError(f"{label}: diagnostic must identify intended source and line; see {work}")
            # rustc may abort on E0453 before Clippy reaches the forbidden type.
            if local_allow and not any((item.get("code") or {}).get("code") == "E0453"
                                      and any(span["is_primary"] and (work / span["file_name"]).resolve() == expected_path.resolve()
                                              and span["line_start"] == 1 for span in item["spans"]) for item in errors):
                raise AssertionError(f"{label}: -F must reject local allow at its source; see {work}")
        print("PASS fixture:", label, flush=True)
    print(f"All {len(cases)} CLI clock policy fixtures passed.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config-only", action="store_true")
    args = parser.parse_args()
    structural_checks()
    if not args.config_only:
        fixtures()
    print("CLI clock policy passed: narrow CLI Instant exception; default and simulation bans retained.")


if __name__ == "__main__":
    main()
