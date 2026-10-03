#!/usr/bin/env python3
"""Run the Arena trial harness with the explicitly identified local mock solver."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys
import unittest

# Permit invocation from any working directory without installing a package.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from tools.arena_trial.protocol import ProtocolError
from tools.arena_trial.runner import TrialError, TrialRunner, freeze_mock_manifest


def binary_paths(repo: Path, host: Path | None, cli: Path | None) -> tuple[Path, Path]:
    suffix = ".exe" if os.name == "nt" else ""
    return ((host or repo / "target" / "release" / ("orr_remote_host" + suffix)).resolve(),
            (cli or repo / "target" / "release" / ("orr" + suffix)).resolve())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument("--self-test", action="store_true")
    actions.add_argument("--integration-test", action="store_true")
    actions.add_argument("--write-mock-manifest", type=Path)
    actions.add_argument("--manifest", type=Path)
    actions.add_argument("--resume", type=Path, help="existing interrupted experiment directory")
    parser.add_argument("--output", type=Path, help="new experiment/integration evidence directory")
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--host-bin", type=Path)
    parser.add_argument("--cli-bin", type=Path)
    args = parser.parse_args(argv)
    repo = args.repo.resolve()
    if repo != Path(__file__).resolve().parents[1]:
        parser.error("--repo must be the worktree containing this runner; invoke that worktree's script")
    host, cli = binary_paths(repo, args.host_bin, args.cli_bin)
    if args.self_test:
        from tools.arena_trial import tests
        suite = unittest.defaultTestLoader.loadTestsFromModule(tests)
        if suite.countTestCases() == 0:
            print("trial self-test suite is empty", file=sys.stderr)
            return 1
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    try:
        if args.integration_test:
            if args.output is None:
                parser.error("--integration-test requires --output")
            from tools.arena_trial.integration import run
            return run(repo, host, cli, args.output.resolve())
        if args.write_mock_manifest:
            path = args.write_mock_manifest.resolve()
            path.parent.mkdir(parents=True, exist_ok=True)
            # Freeze before creating the destination; avoid replacing existing evidence.
            manifest = freeze_mock_manifest(repo, host, cli)
            with path.open("x", encoding="utf-8") as stream:
                json.dump(manifest, stream, indent=2, sort_keys=True, allow_nan=False)
                stream.write("\n")
            print(json.dumps({"manifest": str(path), "kind": "local_mock_only"}))
            return 0
        if args.resume:
            if args.output is not None:
                parser.error("--resume uses its existing directory; omit --output")
            output = args.resume.resolve()
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            provenance = manifest["provenance"]
            host = args.host_bin.resolve() if args.host_bin else Path(provenance["host_path"])
            cli = args.cli_bin.resolve() if args.cli_bin else Path(provenance["cli_path"])
            runner = TrialRunner(output, manifest, resume=True, repo=repo, host_bin=host, cli_bin=cli)
        else:
            if args.output is None:
                parser.error("--manifest requires --output")
            manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
            runner = TrialRunner(args.output, manifest, repo=repo, host_bin=host, cli_bin=cli)
        code = runner.run()
        print(json.dumps({"output": str(runner.output), "status": runner.state["status"],
                          "counts": runner.state["counts"], "exit_code": code}, sort_keys=True))
        return code
    except (OSError, ValueError, KeyError, TrialError, ProtocolError) as exc:
        print(f"trial runner error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
