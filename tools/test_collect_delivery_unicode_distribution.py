#!/usr/bin/env python3
"""Source-hidden packaged CPU gameplay under Unicode paths.

This exercises only the supported --trace CPU gameplay route. It does not test
the production run-game launcher or native window/GPU. Requires existing trusted
builds; it never builds or installs software.

  python3 tools/test_collect_delivery_unicode_distribution.py \\
    --bundle /path/collect-delivery-bundle --player /path/collect-delivery-player \\
    --source-sha 70ed3f755952e0b6d53623a61d8966e047b76a66 \\
    --scene /path/collect-delivery.scene.yaml --evidence /path/evidence

Equivalent environment variables: ORR_COLLECT_BUNDLE, ORR_COLLECT_PLAYER,
ORR_COLLECT_SOURCE_SHA, ORR_COLLECT_SCENE, ORR_COLLECT_EVIDENCE. All are required.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest

EXPECTED_GAME = "CollectDeliveryV1"
TIMEOUT_SECONDS = 45
OUTPUT_LIMIT = 65536
ARGS: argparse.Namespace | None = None
RECORD: dict[str, object] = {}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def regular_file(value: str, label: str) -> Path:
    path = Path(value).expanduser().absolute()
    try:
        info = path.lstat()
    except OSError as exc:
        raise ValueError(f"{label} unavailable: {path}: {exc}") from exc
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise ValueError(f"{label} must be a regular, non-symlink file: {path}")
    return path.resolve(strict=True)


def bounded(data: bytes | None) -> dict[str, object]:
    raw = data or b""
    return {"text": raw[:OUTPUT_LIMIT].decode("utf-8", "replace"),
            "truncated": len(raw) > OUTPUT_LIMIT, "bytes": len(raw)}


def run(argv: list[str], *, cwd: Path, label: str) -> subprocess.CompletedProcess[bytes]:
    event: dict[str, object] = {"label": label, "argv": argv, "cwd": str(cwd),
                                "timeout_seconds": TIMEOUT_SECONDS}
    try:
        result = subprocess.run(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=TIMEOUT_SECONDS, check=False)
    except subprocess.TimeoutExpired as exc:
        event.update({"timed_out": True, "stdout": bounded(exc.stdout), "stderr": bounded(exc.stderr)})
        RECORD.setdefault("subprocesses", []).append(event)
        raise AssertionError(f"{label} timed out after {TIMEOUT_SECONDS}s") from exc
    event.update({"returncode": result.returncode, "stdout": bounded(result.stdout),
                  "stderr": bounded(result.stderr)})
    RECORD.setdefault("subprocesses", []).append(event)
    return result


def assert_success(result: subprocess.CompletedProcess[bytes], label: str) -> None:
    if result.returncode != 0:
        raise AssertionError(f"{label} exited {result.returncode}; stderr:\n"
                             f"{result.stderr.decode('utf-8', 'replace')}\nstdout:\n"
                             f"{result.stdout.decode('utf-8', 'replace')}")


def trace_inputs() -> list[dict[str, int]]:
    # Public contract: movement pulse (2), neutral release (0), stationary act (1).
    def pulse(dx: int) -> list[dict[str, int]]:
        return [{"dx": dx, "dy": 0, "action": 2}, {"dx": 0, "dy": 0, "action": 0}]
    return (pulse(1) + pulse(1) +
            [{"dx": 0, "dy": 0, "action": 1}, {"dx": 0, "dy": 0, "action": 0}] +
            pulse(1) + pulse(1) + pulse(1) + [{"dx": 0, "dy": 0, "action": 1}])


def finish_hashes(record: dict[str, object], paths: dict[str, Path]) -> None:
    hashes: dict[str, str | dict[str, str]] = {}
    for label, path in paths.items():
        try:
            hashes[label] = sha256(path)
        except Exception as exc:  # evidence collection must not mask the test failure
            hashes[label] = {"error": f"{type(exc).__name__}: {exc}"}
    record["input_sha256_after"] = hashes


def write_record(path: Path, record: dict[str, object]) -> None:
    # Run directory is uniquely created by tempfile; x-mode adds a second no-overwrite guard.
    with path.open("x", encoding="utf-8") as stream:
        json.dump(record, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", default=os.getenv("ORR_COLLECT_BUNDLE"))
    parser.add_argument("--player", default=os.getenv("ORR_COLLECT_PLAYER"))
    parser.add_argument("--source-sha", default=os.getenv("ORR_COLLECT_SOURCE_SHA"))
    parser.add_argument("--scene", default=os.getenv("ORR_COLLECT_SCENE"))
    parser.add_argument("--evidence", default=os.getenv("ORR_COLLECT_EVIDENCE"))
    args = parser.parse_args()  # reject unittest selectors and every unknown option
    absent = [name for name in ("bundle", "player", "source_sha", "scene", "evidence")
              if not getattr(args, name)]
    if absent:
        parser.error("missing required inputs: " + ", ".join("--" + n.replace("_", "-") for n in absent))
    if len(args.source_sha) != 40 or any(c not in "0123456789abcdef" for c in args.source_sha):
        parser.error("--source-sha must be 40 lowercase hexadecimal characters")
    return args


class UnicodeDistribution(unittest.TestCase):
    def test_real_bundle_and_source_hidden_cpu_gameplay(self) -> None:
        args = ARGS
        assert args is not None
        bundle_tool = regular_file(args.bundle, "bundle tool")
        player = regular_file(args.player, "player runtime")
        source_scene = regular_file(args.scene, "approved scene")
        source_sha = args.source_sha
        evidence = Path(args.evidence).expanduser().absolute()
        evidence.mkdir(parents=True, exist_ok=True)
        if not evidence.is_dir():
            self.fail(f"evidence path is not a directory: {evidence}")
        input_paths = {"bundle_tool": bundle_tool, "player": player, "approved_scene": source_scene}
        input_before = {name: sha256(path) for name, path in input_paths.items()}
        RECORD.update({"test": "source-hidden packaged CPU gameplay under Unicode paths",
                       "limitations": ["CPU --trace only", "does not exercise production run-game launcher",
                                       "does not exercise native window/GPU"],
                       "source_sha_expected": source_sha, "inputs": {k: str(v) for k, v in input_paths.items()},
                       "input_sha256_before": input_before, "started_unix": time.time()})
        run_dir = Path(tempfile.mkdtemp(prefix="unicode-distribution-run-", dir=evidence))
        record_path = run_dir / "run-evidence.json"
        base = run_dir / "work"
        base.mkdir()
        project = base / "프로젝트 with spaces"
        trusted_dir = base / "신뢰 runtime with spaces"
        output = base / "배포 bundle with spaces"
        consumer = base / "소비자 cwd with spaces"
        project.mkdir()
        trusted_dir.mkdir()
        consumer.mkdir()
        trusted_copy = trusted_dir / "collect player"
        authored_scene = project / "collect-delivery.scene.yaml"
        trace = consumer / "성공 trace with spaces.json"
        relocated = consumer / "재배치 배포 with spaces"
        RECORD.update({"run_directory": str(run_dir), "workspace": str(base), "project": str(project),
                       "bundle_output": str(output), "consumer_cwd": str(consumer),
                       "trusted_runtime_copy": str(trusted_copy)})
        try:
            shutil.copyfile(player, trusted_copy)
            trusted_copy.chmod(0o755)
            if sha256(trusted_copy) != input_before["player"]:
                self.fail("trusted runtime copy changed bytes")
            provenance = run([str(trusted_copy), "--provenance"], cwd=base, label="trusted player provenance")
            assert_success(provenance, "trusted player --provenance")
            provenance_json = json.loads(provenance.stdout)
            RECORD["runtime_provenance"] = provenance_json
            if provenance_json != {"source_sha": source_sha}:
                self.fail(f"runtime provenance mismatch: expected {source_sha}, got {provenance_json!r}")

            raw_scene = source_scene.read_bytes()
            if raw_scene.count(b"destination: [4, 0]") != 1:
                self.fail("approved default scene does not contain exactly one destination [4, 0]")
            authored_bytes = raw_scene.replace(b"destination: [4, 0]", b"destination: [5, 0]", 1)
            authored_scene.write_bytes(authored_bytes)
            trace.write_text(json.dumps(trace_inputs(), separators=(",", ":")) + "\n", encoding="utf-8")
            player_hash = str(input_before["player"])
            bundled = run([str(bundle_tool), "--runtime", str(trusted_copy), "--runtime-sha256", player_hash,
                           "--scene", str(authored_scene), "--output", str(output), "--source-sha", source_sha,
                           "--trusted-runtime"], cwd=base, label="production bundle CLI")
            assert_success(bundled, "production bundle CLI")
            receipt = json.loads(bundled.stdout)
            RECORD["bundle_receipt"] = receipt
            if receipt.get("source_sha") != source_sha or receipt.get("runtime_sha256") != player_hash:
                self.fail("bundle receipt source/runtime SHA mismatch")
            if not (output / "bin/standalone").is_file() or sha256(output / "bin/standalone") != player_hash:
                self.fail("bundle runtime missing or differs from trusted runtime")
            if (output / "scene/game.scene.yaml").read_bytes() != authored_bytes:
                self.fail("bundle did not preserve exact authored scene bytes")

            # Remove only these test-created source inputs before packaged gameplay.
            shutil.rmtree(project)
            trusted_copy.unlink()
            if project.exists() or trusted_copy.exists():
                self.fail("authored project or trusted runtime copy survived removal")
            output.rename(relocated)
            packaged = relocated / "bin/standalone"
            child = run([str(packaged), "--scene", str(relocated / "scene/game.scene.yaml"),
                         "--trace", str(trace)], cwd=consumer, label="relocated packaged CPU trace")
            assert_success(child, "relocated packaged CPU trace")
            state = json.loads(child.stdout)
            RECORD["game_result"] = state
            expected = {"game": EXPECTED_GAME,
                        "state": {"destination": [5 << 16, 0], "position": [5 << 16, 0],
                                  "item_available": 0, "carrying": 1, "won": 1, "lost": 0}}
            if state.get("game") != expected["game"]:
                self.fail(f"wrong game identity: {state.get('game')!r}")
            for key, value in expected["state"].items():
                actual = state.get("state", {}).get(key)
                if actual != value:
                    self.fail(f"final {key} mismatch: expected {value!r}, got {actual!r}")
            if project.exists() or trusted_copy.exists():
                self.fail("original project/runtime source remained available to consumer")
            RECORD.update({"status": "passed", "distribution": str(relocated), "trace": str(trace)})
            after = {}
            for name, path in input_paths.items():
                after[name] = sha256(path)
            RECORD["input_sha256_after"] = after
            if after != input_before:
                self.fail("an original bundle/player/scene input changed during test")
        except BaseException as exc:
            RECORD["status"] = "failed"
            RECORD["error"] = f"{type(exc).__name__}: {exc}"
            finish_hashes(RECORD, input_paths)
            RECORD["finished_unix"] = time.time()
            write_record(record_path, RECORD)
            print(f"failure evidence: {record_path}", file=sys.stderr)
            raise
        else:
            RECORD["finished_unix"] = time.time()
            write_record(record_path, RECORD)
            print(json.dumps({"status": "passed", "evidence": str(record_path),
                              "distribution": str(relocated), "trace": str(trace)}, ensure_ascii=False))


def main() -> int:
    global ARGS
    ARGS = parse_args()
    suite = unittest.TestSuite([UnicodeDistribution("test_real_bundle_and_source_hidden_cpu_gameplay")])
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if result.testsRun != 1 or result.skipped or not result.wasSuccessful():
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
