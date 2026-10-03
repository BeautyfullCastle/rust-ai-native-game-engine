"""Coordinator, broker, oracle, persistence, and attempt accounting."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import queue
import signal
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from typing import Any

from tools import arena_agent_benchmark as baseline
from . import FORMAT
from .protocol import TASKS, ProtocolError, decode, encode, validate_manifest, validate_request


MAX_PROTOCOL_FRAME_BYTES = 64 * 1024
PROTOCOL_QUEUE_DEPTH = 4

def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def digest_file(path: Path) -> str:
    return baseline.sha256(path)


def canonical_hash(value: Any) -> str:
    data = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(data).hexdigest()


def require_exists(path: Path) -> None:
    baseline.require(path.is_file() and path.stat().st_size > 0, f"runner-owned replay is missing or empty: {path.name}")


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def default_manifest() -> dict[str, Any]:
    return {
        "format": "orr.arena-agent-trial-manifest/1",
        "experiment_id": "arena-mock-local-v1",
        "solver": {"name": "mock", "version": "1", "argv": ["{python}", "{repo}/tools/arena_trial/mock_solver.py"],
                   "script_sha256": "generated-by-write-mock-manifest"},
        "attempt_count": 1,
        "seed": 42,
        "players": 2,
        "tick_rate": 60,
        "task_timeout_seconds": 600,
        "attempt_timeout_seconds": 2400,
        "command_timeout_seconds": 30,
        "readiness_timeout_seconds": 20,
        "max_requests": 2000,
        "max_interventions": 0,
        "intervention_policy": "zero",
        "model": {"provider": "mock", "version": "scripted-protocol-driver-v1", "temperature": None},
        "token_budget": None,
        "cost_budget_usd": None,
        "usage_policy": "unavailable_is_unknown",
        "allowed_actions": ["typed_arena_broker_v1"],
        "resume_policy": "resume_same_attempt_from_last_accepted_task_boundary",
        "trust_boundary": "protocol_only_same_account_not_an_os_sandbox",
        "mock_scenario": "success",
    }


def _git(repo: Path, *args: str) -> str:
    result = subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, check=True)
    return result.stdout


def frozen_provenance(repo: Path, host_bin: Path, cli_bin: Path) -> dict[str, Any]:
    repo = repo.resolve()
    paths = sorted(set(_git(repo, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")) - {""})
    digest = hashlib.sha256()
    file_count = 0
    for name in paths:
        path = repo / name
        if not path.is_file() or any(part in {"target", ".git"} for part in path.parts[len(repo.parts):]):
            continue
        digest.update(name.replace("\\", "/").encode("utf-8") + b"\0")
        digest.update(bytes.fromhex(digest_file(path)))
        file_count += 1
    fixture = repo / "scenes" / "arena_blank.scene.yaml"
    return {
        "commit": _git(repo, "rev-parse", "HEAD").strip(),
        "tree": _git(repo, "rev-parse", "HEAD^{tree}").strip(),
        "dirty_status": _git(repo, "status", "--porcelain=v1").strip(),
        "source_sha256": digest.hexdigest(), "source_file_count": file_count,
        "fixture_sha256": digest_file(fixture),
        "host_path": str(host_bin.resolve()), "host_sha256": digest_file(host_bin),
        "cli_path": str(cli_bin.resolve()), "cli_sha256": digest_file(cli_bin),
    }


def freeze_mock_manifest(repo: Path, host_bin: Path, cli_bin: Path) -> dict[str, Any]:
    manifest = default_manifest()
    mock_path = repo / "tools" / "arena_trial" / "mock_solver.py"
    manifest["solver"]["script_sha256"] = digest_file(mock_path)
    manifest["solver"]["interpreter"] = sys.version.splitlines()[0]
    manifest["solver"]["interpreter_path"] = str(Path(sys.executable).resolve())
    manifest["solver"]["interpreter_sha256"] = digest_file(Path(sys.executable))
    manifest["provenance"] = frozen_provenance(repo, host_bin, cli_bin)
    with tempfile.TemporaryDirectory(prefix="orr-arena-trial-manifest-") as temp:
        args = type("BenchmarkArgs", (), {})()
        args.repo = repo.resolve()
        args.output = Path(temp) / "control"
        args.build_mode = "prebuilt"
        args.host_bin = host_bin.resolve()
        args.cli_bin = cli_bin.resolve()
        args.command_timeout = manifest["command_timeout_seconds"]
        args.readiness_timeout = manifest["readiness_timeout_seconds"]
        args.build_timeout = 1800
        args.human_interventions = 0
        args.intervention_note = "none reported"
        probe = baseline.Benchmark(args)
        try:
            probe.prepare()
            status = probe.start_host()
            schema = probe.cli("schema", "--input")
            manifest["provenance"]["engine_identity"] = status["engine"]
            manifest["provenance"]["input_schema_sha256"] = canonical_hash(schema)
        finally:
            probe.stop_host()
    return manifest


class TrialError(RuntimeError):
    pass


def _task_actions(attempt: dict[str, Any], number: int) -> list[dict[str, Any]]:
    segments = attempt.get("task_action_segments", {}).get(str(number), [])
    return segments[-1] if segments else []


def _expect_actions(attempt: dict[str, Any], number: int, expected: list[str]) -> list[dict[str, Any]]:
    actions = _task_actions(attempt, number)
    actual = [item["op"] for item in actions]
    baseline.require(actual == expected, f"task {number} operation sequence differs: {actual}")
    return actions


class Broker:
    """Strict typed adapter over the existing baseline's CLI/lifecycle helpers."""

    def __init__(self, runner: "TrialRunner", attempt: dict[str, Any], benchmark: baseline.Benchmark):
        self.runner = runner
        self.attempt = attempt
        self.benchmark = benchmark
        self.task_number = attempt["next_task"]
        self.playing = False
        self.trace: list[dict[str, Any]] = []
        self.baseline_checksum = attempt.get("facts", {}).get("baseline_checksum")
        self.modified_checksum = attempt.get("facts", {}).get("modified_checksum")
        segments = attempt.setdefault("task_action_segments", {}).setdefault(str(self.task_number), [])
        segments.append([])
        self.actions = segments[-1]

    def record(self, op: str, **details: Any) -> None:
        self.actions.append({"op": op, **details})

    def _cli(self, *args: str) -> Any:
        return self.benchmark.cli(*args)

    def _reply(self, request_id: str, result: Any = None, error: str | None = None) -> dict[str, Any]:
        return {"type": "reply", "id": request_id, "ok": error is None,
                "result": result if error is None else None, "error": error}

    def dispatch(self, message: dict[str, Any]) -> tuple[dict[str, Any], bool]:
        request_id = str(message.get("id", ""))
        try:
            op, args = validate_request(message)
            result, done = self._dispatch(op, args)
            return self._reply(request_id, result), done
        except (ProtocolError, TrialError, baseline.BenchmarkFailure, KeyError, ValueError) as exc:
            return self._reply(request_id, error=str(exc)), False

    def _dispatch(self, op: str, args: dict[str, Any]) -> tuple[Any, bool]:
        if op == "task_done":
            try:
                verdict = self.runner.judge_task(self, self.task_number)
            except (baseline.BenchmarkFailure, TrialError, KeyError, ValueError) as exc:
                verdict = {"number": self.task_number, "status": "task_failed", "finished_at": utc_now(),
                           "reason": "independent_oracle_failed", "error": str(exc), "facts": {}}
            self.attempt["tasks"].append(verdict)
            self.attempt["facts"].update(verdict.get("facts", {}))
            self.attempt["next_task"] += 1
            self.runner.checkpoint(self.attempt, self.benchmark, self.attempt["next_task"])
            self.runner.persist()
            self.runner.event({"kind": "task_verdict", "attempt": self.attempt["number"], **verdict})
            if verdict["status"] != "passed":
                self.attempt["status"] = "task_failed"
                self.runner.persist()
                return {"task_failed": True}, True
            self.task_number = self.attempt["next_task"]
            self.trace = []
            if self.task_number > 5:
                return {"next": None}, True
            return {"next_task": self.task_number, "description": TASKS[self.task_number]}, True

        if op == "read":
            what = args["what"]
            if what == "status":
                result = self._cli("status")
            elif what == "scene":
                result = self._cli("scene", "--components")
            elif what == "history":
                result = self._cli("history")
            elif what == "schema":
                result = self._cli("schema", "--input")
            else:
                result = self._cli("sim", "state") if self.playing else self._cli("status")["state"]
            self.record("read", what=what)
            return result, False

        if op == "apply":
            if self.playing or self.task_number not in (1, 4):
                raise ProtocolError("guarded authoring is allowed only in tasks 1 and 4 while stopped")
            ops = args["operations"]
            if self.task_number == 1:
                if len(ops) != 2 or [item.get("op") for item in ops] != ["spawn_player", "spawn_player"]:
                    raise ProtocolError("task 1 requires one guarded two-player creation apply")
                if [(item["name"], item["slot"], item["pos"]) for item in ops] != [
                    ("hero", 0, [-300, 0]), ("target", 1, [300, 0])]:
                    raise ProtocolError("task 1 player slots/positions differ from the frozen goal")
                argv: list[str] = ["apply", args["label"]]
                for item in ops:
                    argv += ["spawn", "--name", item["name"],
                             "Position={\"pos\":" + json.dumps(item["pos"], separators=(",", ":")) + "}",
                             "PlayerTag={\"slot\":" + str(item["slot"]) + "}"]
                argv += ["--idle", "1"]
                for check in ("players.final == 2", "bullets.final == 0", "score_0.final == 0",
                              "score_1.final == 0", "out_of_bounds.final == 0"):
                    argv += ["--check", check]
            else:
                if len(ops) != 1 or ops[0].get("op") != "set_position":
                    raise ProtocolError("task 4 permits one target Position.pos edit")
                target = ops[0]
                scene = baseline.scene_facts(self._cli("scene", "--components"))
                target_player = next((p for p in scene["players"] if p["slot"] == 1), None)
                if target_player is None or target["entity"] != target_player["id"]:
                    raise ProtocolError("task 4 may move only discovered slot 1")
                if target["pos"] != [900, 0]:
                    raise ProtocolError("task 4 target position differs from the frozen goal")
                replay = self.benchmark.output / "baseline20.orrp"
                if not replay.is_file():
                    raise ProtocolError("task 4 requires task 3's runner-owned baseline replay")
                argv = ["apply", args["label"], "set", target["entity"], "Position.pos=[900,0]",
                        "--replay", str(replay)]
                for check in ("base:score_0.final == 1", "score_0.final == 0", "players.final == 2",
                              "score_1.final == 0", "out_of_bounds.final == 0", "recording_matches"):
                    argv += ["--check", check]
            report = self._cli(*argv)
            if report.get("accepted") is not True or report.get("outcome") != "accepted":
                raise TrialError("guarded apply did not pass")
            if self.task_number == 4:
                self.attempt["facts"]["task4_apply_verify"] = report.get("verify")
                self.attempt["facts"]["task4_apply_accepted"] = True
            if self.task_number == 1:
                self.baseline_checksum = self._cli("status")["state"]["doc_checksum"]
                self.attempt["facts"]["baseline_checksum"] = self.baseline_checksum
                self.attempt["facts"]["task1_apply_receipt"] = report.get("history_id")
            else:
                self.modified_checksum = self._cli("status")["state"]["doc_checksum"]
                self.attempt["facts"]["modified_checksum"] = self.modified_checksum
            self.record("apply", task=self.task_number, accepted=True)
            # The independent verdict remains private; return only command acceptance.
            return {"accepted": True, "history_id": report.get("history_id")}, False

        if op == "sim_start":
            if self.playing or self.task_number not in (2, 3, 4, 5):
                raise ProtocolError("sim_start is not allowed in this task state")
            result = self._cli("sim", "start")
            self.playing = True
            self.benchmark.playing = True
            self.trace = [{"tick": 0, "checksum": result["checksum"]}]
            self.record("sim_start")
            return {"mode": result["mode"], "head_tick": result["head_tick"], "playing": result["playing"]}, False

        if op == "sim_input":
            if not self.playing:
                raise ProtocolError("sim input requires an active play session")
            value = args["value"]
            self._cli("sim", "input", "--player", str(args["player"]), json.dumps(value, separators=(",", ":")))
            self.record("sim_input", player=args["player"], value=value)
            return {"accepted": True}, False

        if op == "sim_step":
            if not self.playing:
                raise ProtocolError("sim_step requires an active play session")
            ticks = args["ticks"]
            if self.task_number in (4, 5) and ticks != 1:
                raise ProtocolError("tasks 4 and 5 require one-tick steps for all 51 checksums")
            result = self._cli("sim", "step", str(ticks))
            if self.task_number in (4, 5):
                self.trace.append({"tick": result["head_tick"], "checksum": result["checksum"]})
                # Persist raw evidence after every tick so process interruption cannot erase it.
                segment = self.attempt.get("resume_count", 0)
                write_json(self.runner.attempt_dir(self.attempt["number"]) /
                           f"task{self.task_number}-segment{segment}-checksums-partial.json", self.trace)
            elif self.task_number == 2 and result["head_tick"] in (10, 15):
                # Capture live simulation positions before sim_stop restores the
                # authoring document to its pre-play positions.
                snapshot = baseline.scene_facts(self._cli("scene", "--components"))
                self.attempt.setdefault("facts", {}).setdefault("task2_live_snapshots", []).append(
                    {"tick": result["head_tick"], "scene": snapshot})
            if ((self.task_number == 3 and result["head_tick"] == 20) or
                  (self.task_number in (4, 5) and result["head_tick"] == 50)):
                snapshot = baseline.scene_facts(self._cli("scene", "--components"))
                self.attempt.setdefault("facts", {})[f"task{self.task_number}_live_final"] = {
                    "tick": result["head_tick"], "scene": snapshot}
            self.record("sim_step", ticks=ticks)
            return {"head_tick": result["head_tick"], "checksum": result["checksum"]}, False

        if op == "verify_replay":
            allowed_task = (self.task_number == 3 and args["replay"] == "baseline20") or (
                self.task_number == 4 and args["replay"] == "modified50") or (
                self.task_number == 5 and args["replay"] == "modified50")
            if not allowed_task or self.playing:
                raise ProtocolError("replay verification is not allowed in this task state")
            ticks = 20 if args["replay"] == "baseline20" else 50
            replay = self.benchmark.output / f"{args['replay']}.orrp"
            self.benchmark.verify(replay, ticks, f"solver-request-task{self.task_number}-verify.json")
            self.record("verify_replay", replay=args["replay"])
            return {"verification_ran": True}, False

        if op == "sim_stop":
            if not self.playing:
                raise ProtocolError("sim_stop requires an active play session")
            label = args.get("replay")
            if label:
                replay_path = self.benchmark.output / f"{label}.orrp"
                result = self.benchmark.stop_play(replay_path)
            else:
                result = self._cli("sim", "stop")
                self.benchmark.playing = False
            self.playing = False
            self.record("sim_stop", replay=label)
            return {"stopped": True, "replay": label}, False

        if op in {"undo", "redo"}:
            if self.playing or self.task_number != 4:
                raise ProtocolError(f"{op} is allowed only in task 4 while stopped")
            self._cli(op)
            state = self._cli("status")["state"]
            scene = baseline.scene_facts(self._cli("scene", "--components"))
            self.attempt["facts"][f"task4_{op}_checksum"] = state["doc_checksum"]
            self.attempt["facts"][f"task4_{op}_scene"] = scene
            self.attempt["facts"][f"task4_{op}_seen"] = True
            self.record(op, checksum=state["doc_checksum"])
            return {"accepted": True}, False

        if op == "save":
            if self.playing or self.task_number != 5:
                raise ProtocolError("save is allowed only in task 5 while stopped")
            self._cli("save", "--write")
            self.record("save", write=True)
            return {"saved": True}, False

        if op == "restart_host":
            if self.playing or self.task_number != 5:
                raise ProtocolError("restart_host is allowed only in task 5 while stopped")
            old_process = self.benchmark.process
            self.benchmark.stop_host()
            old_job_closed = self.runner._close_job(old_process) if old_process is not None else False
            if old_process is not None:
                host_cleanup = {"pid": old_process.pid, "exit_code": old_process.returncode,
                                "job_closed": old_job_closed, "planned_restart": True, "finished_at": utc_now()}
                self.attempt.setdefault("cleanup", []).append(host_cleanup)
                self.runner.event({"kind": "host_cleanup", "attempt": self.attempt["number"], **host_cleanup})
            status = self.benchmark.start_host()
            self.benchmark.process._trial_job = self.runner._attach_windows_job(self.benchmark.process)  # type: ignore[attr-defined]
            if status["engine"] != self.benchmark.engine:
                raise TrialError("restarted host identity differs")
            self.runner.state["counts"]["planned_host_restarts"] += 1
            self.record("restart_host", generation=self.benchmark.host_generation)
            self.runner.persist()
            return {"restarted": True, "generation": self.benchmark.host_generation}, False

        raise ProtocolError("operation not implemented")


class TrialRunner:
    def __init__(self, output: Path, manifest: dict[str, Any], *, resume: bool = False,
                 repo: Path | None = None, host_bin: Path | None = None, cli_bin: Path | None = None):
        self.output = output.resolve()
        self.manifest = validate_manifest(manifest)
        self.manifest_hash = canonical_hash(self.manifest)
        self.repo = (repo or Path(__file__).resolve().parents[2]).resolve()
        self.host_bin = host_bin
        self.cli_bin = cli_bin
        if self.host_bin is None:
            self.host_bin = Path(self.manifest["provenance"]["host_path"])
        if self.cli_bin is None:
            self.cli_bin = Path(self.manifest["provenance"]["cli_path"])
        self.resume_mode = resume
        self.state_path = self.output / "trial.json"
        self.transcript_path = self.output / "events.jsonl"
        self.last_event_hash = "0" * 64
        if resume:
            if not self.state_path.is_file():
                raise TrialError("--resume requires an existing trial.json")
            self.state = json.loads(self.state_path.read_text(encoding="utf-8"))
            if self.state.get("manifest_sha256") != self.manifest_hash:
                raise TrialError("frozen manifest differs from the interrupted experiment")
            if self.state.get("status") != "interrupted":
                raise TrialError("only an interrupted experiment can resume")
            self.expected_event_hash = self.state.get("last_event_hash", self.last_event_hash)
        else:
            self.output.mkdir(parents=True, exist_ok=False)
            write_json(self.output / "manifest.json", self.manifest)
            self.state = {
                "format": FORMAT, "experiment_id": self.manifest["experiment_id"],
                "manifest_sha256": self.manifest_hash, "status": "running", "started_at": utc_now(),
                "attempts": [], "counts": {"scheduled_attempts": self.manifest["attempt_count"],
                    "passed_attempts": 0, "failed_attempts": 0, "infrastructure_interrupted": 0,
                    "interventions": 0, "planned_host_restarts": 0, "solver_requests": 0},
                "usage": {"tokens": "unknown", "cost_usd": "unknown"},
                "trust_boundary": "Protocol-only cooperative solver; arbitrary same-account subprocess is not sandboxed.",
            }
            self.persist()
        self._replay_event_hash_chain()
        self.solver_request_count = self.state.get("counts", {}).get("solver_requests", 0)
        self.current_attempt_dir: Path | None = None
        self.last_task_clock = time.monotonic()

    def attempt_dir(self, number: int) -> Path:
        return self.output / f"attempt-{number:04d}"

    def _replay_event_hash_chain(self) -> None:
        if not self.transcript_path.exists():
            if getattr(self, "resume_mode", False) and self.expected_event_hash != "0" * 64:
                raise TrialError("event transcript is missing for saved hash-chain head")
            return
        for line in self.transcript_path.read_text(encoding="utf-8").splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                raise TrialError("event transcript has a malformed line")
            expected = event.pop("event_sha256", None)
            if event.get("previous_sha256") != self.last_event_hash:
                raise TrialError("event transcript chain is broken")
            actual = canonical_hash(event)
            if actual != expected:
                raise TrialError("event transcript hash mismatch")
            self.last_event_hash = actual
        if getattr(self, "resume_mode", False) and self.last_event_hash != self.expected_event_hash:
            raise TrialError("event transcript head differs from trial.json")

    def event(self, event: dict[str, Any]) -> None:
        record = {"utc": utc_now(), "previous_sha256": self.last_event_hash, **event}
        digest = canonical_hash(record)
        record["event_sha256"] = digest
        with self.transcript_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(record, sort_keys=True) + "\n")
        self.last_event_hash = digest
        self.state["last_event_hash"] = digest
        self.persist()

    def persist(self) -> None:
        self.state["updated_at"] = utc_now()
        write_json(self.state_path, self.state)

    def _refresh_ledger(self) -> None:
        """Recompute denominators from terminal task records, retaining unresolved cells."""
        attempts = self.state["attempts"]
        counts = self.state["counts"]
        attempt_numbers = [item.get("number") for item in attempts]
        if len(set(attempt_numbers)) != len(attempt_numbers) or any(
                isinstance(number, bool) or not isinstance(number, int) or
                not 1 <= number <= self.manifest["attempt_count"] for number in attempt_numbers):
            raise TrialError("attempt ledger contains duplicate or out-of-range attempt numbers")
        counts["passed_attempts"] = sum(1 for item in attempts if item["status"] == "passed")
        counts["failed_attempts"] = sum(
            1 for item in attempts if item["status"] in {"task_failed", "intervened"})
        task_cells = [task for attempt in attempts for task in attempt["tasks"]]
        task_keys = [(attempt["number"], task.get("number"))
                     for attempt in attempts for task in attempt["tasks"]]
        if len(set(task_keys)) != len(task_keys) or any(
                isinstance(number, bool) or not isinstance(number, int) or not 1 <= number <= 5
                for _, number in task_keys):
            raise TrialError("terminal task ledger contains duplicate or out-of-range task cells")
        passed_tasks = sum(1 for task in task_cells if task["status"] == "passed")
        failed_tasks = sum(1 for task in task_cells if task["status"] == "task_failed")
        scheduled = self.manifest["attempt_count"] * 5
        completed = passed_tasks + failed_tasks
        interrupted_cells = {
            (attempt["number"], event.get("task"))
            for attempt in attempts for event in attempt.get("task_events", [])
            if event.get("status") in {"infrastructure_interrupted", "runner_interrupted"}
        }
        completed_cells = {
            (attempt["number"], task["number"])
            for attempt in attempts for task in attempt["tasks"]
        }
        started_cells = set(completed_cells)
        started_cells.update(
            (attempt["number"], event.get("task"))
            for attempt in attempts for event in attempt.get("task_events", [])
            if event.get("task") is not None
        )
        counts["scheduled_task_cells"] = scheduled
        counts["completed_task_cells"] = completed
        counts["passed_task_cells"] = passed_tasks
        counts["failed_task_cells"] = failed_tasks
        counts["started_task_cells"] = len(started_cells)
        if completed > scheduled or len(started_cells) > scheduled:
            raise TrialError("task ledger exceeds the frozen scheduled-cell denominator")
        counts["unresolved_task_cells"] = scheduled - completed
        counts["unstarted_task_cells"] = scheduled - len(started_cells)
        counts["infrastructure_interrupted_task_cells"] = len(interrupted_cells)
        counts["resumed_task_cells"] = len(interrupted_cells & completed_cells)
        times = [task.get("active_seconds") for task in task_cells if task.get("active_seconds") is not None]
        self.state["timing"] = {
            "task_active_seconds": times,
            "attempt_wall_seconds": [item.get("wall_seconds", 0) for item in attempts],
            "attempt_infrastructure_downtime_seconds": [item.get("infra_downtime_seconds", 0)
                                                         for item in attempts],
        }

    def _write_artifact_manifest(self) -> None:
        """Hash persisted evidence without recursively including mutable indexes."""
        items = []
        for path in sorted(self.output.rglob("*")):
            if not path.is_file() or path.name in {"trial.json", "artifacts.json"}:
                continue
            relative = path.relative_to(self.output).as_posix()
            items.append({"path": relative, "size_bytes": path.stat().st_size,
                          "sha256": digest_file(path)})
        write_json(self.output / "artifacts.json", {
            "format": "orr.arena-agent-trial-artifacts/1",
            "manifest_sha256": self.manifest_hash,
            "event_chain_head": self.last_event_hash,
            "files": items,
        })

    @staticmethod
    def _record_infrastructure_downtime(attempt: dict[str, Any], resumed_at: str) -> float:
        intervals = attempt.setdefault("infra_intervals", [])
        if not intervals or "finished_at" in intervals[-1]:
            return 0.0
        interval = intervals[-1]
        interval["finished_at"] = resumed_at
        # Detection time precedes process cleanup. Downtime begins only after
        # this execution segment has ended, otherwise cleanup is double-counted.
        origin = interval.get("downtime_started_at", interval.get(
            "segment_ended_at", interval.get("started_at")))
        try:
            seconds = (datetime.fromisoformat(resumed_at) - datetime.fromisoformat(origin)).total_seconds()
        except (TypeError, ValueError):
            raise TrialError("interruption interval lacks valid UTC downtime boundaries")
        if seconds < 0:
            raise TrialError("interruption resume timestamp precedes downtime boundary")
        interval["downtime_seconds"] = seconds
        attempt["infra_downtime_seconds"] = attempt.get("infra_downtime_seconds", 0.0) + seconds
        attempt["wall_seconds"] = attempt.get("wall_seconds", 0.0) + seconds
        return seconds

    def _benchmark(self, number: int) -> baseline.Benchmark:
        attempt_dir = self.attempt_dir(number)
        resume_index = next((item.get("resume_count", 0) for item in self.state["attempts"]
                             if item.get("number") == number), 0)
        control = attempt_dir / ("control" if resume_index == 0 else f"control-resume-{resume_index}")
        args = type("BenchmarkArgs", (), {})()
        args.repo = self.repo
        args.output = control
        args.build_mode = "prebuilt"
        args.host_bin = self.host_bin
        args.cli_bin = self.cli_bin
        if args.host_bin is None:
            default = self.repo / "target" / "release" / ("orr_remote_host.exe" if os.name == "nt" else "orr_remote_host")
            args.host_bin = default
        if args.cli_bin is None:
            default = self.repo / "target" / "release" / ("orr.exe" if os.name == "nt" else "orr")
            args.cli_bin = default
        args.command_timeout = self.manifest["command_timeout_seconds"]
        args.readiness_timeout = self.manifest["readiness_timeout_seconds"]
        args.build_timeout = 1800
        args.human_interventions = 0
        args.intervention_note = "none reported"
        args.human_interventions = 0
        return baseline.Benchmark(args)

    def checkpoint(self, attempt: dict[str, Any], benchmark: baseline.Benchmark, next_task: int) -> None:
        attempt_dir = self.attempt_dir(attempt["number"])
        checkpoint = attempt_dir / "checkpoints" / f"task-{next_task}-before.scene.yaml"
        if next_task <= 5:
            response = benchmark.cli("save")
            checkpoint.parent.mkdir(parents=True, exist_ok=True)
            checkpoint.write_text(response["text"], encoding="utf-8")
            attempt["resume_scene"] = str(checkpoint.relative_to(self.output))
        attempt["checkpoint_task"] = next_task
        attempt["last_accepted_task"] = next_task - 1
        self.persist()

    def _prepare_attempt(self, attempt: dict[str, Any], *, fresh: bool) -> baseline.Benchmark:
        attempt_dir = self.attempt_dir(attempt["number"])
        current_provenance = frozen_provenance(self.repo, self.host_bin, self.cli_bin)
        frozen = self.manifest["provenance"]
        for field in ("commit", "tree", "dirty_status", "source_sha256", "source_file_count",
                      "fixture_sha256", "host_sha256", "cli_sha256"):
            if current_provenance[field] != frozen[field]:
                raise TrialError(f"frozen provenance changed: {field}")
        if digest_file(self.repo / "tools" / "arena_trial" / "mock_solver.py") != self.manifest["solver"]["script_sha256"]:
            raise TrialError("frozen mock solver script changed")
        if str(Path(sys.executable).resolve()) != self.manifest["solver"]["interpreter_path"] or \
                sys.version.splitlines()[0] != self.manifest["solver"]["interpreter"] or \
                digest_file(Path(sys.executable)) != self.manifest["solver"]["interpreter_sha256"]:
            raise TrialError("solver interpreter differs from the frozen manifest")
        benchmark = self._benchmark(attempt["number"])
        benchmark.prepare()
        if not fresh:
            previous_resume = attempt.get("resume_count", 0) - 1
            prior = attempt_dir / ("control" if previous_resume == 0 else f"control-resume-{previous_resume}")
            if prior.is_dir():
                for replay in prior.glob("*.orrp"):
                    shutil.copy2(replay, benchmark.output / replay.name)
            scene_rel = attempt.get("resume_scene")
            if not scene_rel:
                raise TrialError("interrupted attempt has no task-boundary scene checkpoint")
            scene = (self.output / scene_rel).read_text(encoding="utf-8")
            benchmark.scene_path.write_text(scene, encoding="utf-8")
        status = benchmark.start_host()
        benchmark.engine = status["engine"]
        schema = benchmark.cli("schema", "--input")
        if status["engine"] != frozen["engine_identity"] or canonical_hash(schema) != frozen["input_schema_sha256"]:
            raise TrialError("host engine or input schema differs from the frozen manifest")
        if attempt.get("last_accepted_task", 0) >= 1:
            attempt["facts"].setdefault("baseline_checksum", status["state"]["doc_checksum"])
        if attempt.get("last_accepted_task", 0) >= 4:
            attempt["facts"].setdefault("modified_checksum", status["state"]["doc_checksum"])
        return benchmark

    def _solver_command(self) -> list[str]:
        mock_path = self.repo / "tools" / "arena_trial" / "mock_solver.py"
        template = self.manifest["solver"]["argv"]
        expected = ["{python}", "{repo}/tools/arena_trial/mock_solver.py"]
        if template != expected:
            raise TrialError("only the frozen bundled mock argv is supported by this runner version")
        return [self.manifest["solver"]["interpreter_path"], str(mock_path),
                "--scenario", self.manifest.get("mock_scenario", "success")]

    def _spawn_solver(self, attempt: dict[str, Any], task_number: int, attempt_dir: Path) -> subprocess.Popen[str]:
        stderr_path = attempt_dir / f"solver-task-{task_number}.stderr.log"
        stderr_file = stderr_path.open("a", encoding="utf-8")
        command = self._solver_command()
        resume = next((item.get("resume_count", 0) for item in self.state["attempts"]
                       if item.get("number") == attempt["number"]), 0)
        workdir = attempt_dir / "solver-work" / f"task-{task_number}-segment-{resume}"
        workdir.mkdir(parents=True, exist_ok=False)
        env = {key: os.environ[key] for key in ("PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT") if key in os.environ}
        env.update({"PYTHONUNBUFFERED": "1", "TEMP": str(workdir), "TMP": str(workdir)})
        kwargs: dict[str, Any] = {}
        if os.name == "nt":
            kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
        else:
            kwargs["start_new_session"] = True
        process = subprocess.Popen(command, cwd=workdir, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=stderr_file, text=True, encoding="utf-8", bufsize=1,
                                   close_fds=True, **kwargs)
        process._trial_argv = command  # type: ignore[attr-defined]
        process._trial_workdir = str(workdir)  # type: ignore[attr-defined]
        process._trial_job = self._attach_windows_job(process)  # type: ignore[attr-defined]
        self.event({"kind": "solver_spawn", "attempt": attempt["number"], "task": task_number,
                    "argv": command, "workdir": str(workdir), "pid": process.pid,
                    "python_path": str(Path(sys.executable).resolve()),
                    "python_sha256": digest_file(Path(sys.executable)),
                    "windows_job_attached": bool(getattr(process, "_trial_job", None))})
        # Keep the handle alive on the Popen object for deterministic cleanup.
        process._trial_stderr_file = stderr_file  # type: ignore[attr-defined]
        return process

    @staticmethod
    def _attach_windows_job(process: subprocess.Popen[str]) -> Any:
        if os.name != "nt":
            return None
        try:
            import ctypes
            kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel.CreateJobObjectW.restype = ctypes.c_void_p
            job = kernel.CreateJobObjectW(None, None)
            if not job:
                return None
            kernel.AssignProcessToJobObject.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
            if not kernel.AssignProcessToJobObject(ctypes.c_void_p(job), ctypes.c_void_p(int(process._handle))):
                kernel.CloseHandle(ctypes.c_void_p(job))
                return None
            return {"kernel": kernel, "handle": job, "assigned": True}
        except Exception:
            return None

    def _send(self, process: subprocess.Popen[str], message: dict[str, Any]) -> None:
        assert process.stdin is not None
        process.stdin.write(encode(message) + "\n")
        process.stdin.flush()

    def _read_solver(self, process: subprocess.Popen[str], timeout: float) -> dict[str, Any] | None:
        assert process.stdout is not None
        response_queue: queue.Queue[str | None] = process._trial_lines  # type: ignore[attr-defined]
        try:
            line = response_queue.get(timeout=timeout)
        except queue.Empty:
            return None
        if line is None:
            return None
        if line == "\0FRAME_TOO_LARGE":
            raise ProtocolError(f"solver protocol frame exceeds {MAX_PROTOCOL_FRAME_BYTES} bytes")
        return decode(line)

    def _start_reader(self, process: subprocess.Popen[str]) -> None:
        lines: queue.Queue[str | None] = queue.Queue(maxsize=PROTOCOL_QUEUE_DEPTH)
        process._trial_lines = lines  # type: ignore[attr-defined]
        def read_lines() -> None:
            assert process.stdout is not None
            while True:
                line = process.stdout.readline(MAX_PROTOCOL_FRAME_BYTES + 1)
                if not line:
                    break
                if len(line.encode("utf-8", errors="replace")) > MAX_PROTOCOL_FRAME_BYTES:
                    lines.put("\0FRAME_TOO_LARGE")
                    break
                lines.put(line)
            lines.put(None)
        threading.Thread(target=read_lines, daemon=True).start()

    def _solver_exchange(self, process: subprocess.Popen[str], broker: Broker, attempt: dict[str, Any], task: int) -> bool:
        self._start_reader(process)
        attempt.setdefault("task_events", []).append({"task": task, "status": "started", "utc": utc_now()})
        self.persist()
        self._send(process, {"type": "start", "attempt": attempt["number"], "task": task,
                             "description": TASKS[task], "seed": 42, "players": 2, "tick_rate": 60,
                             "guide": broker.benchmark.cli("agents-md")["text"],
                             "resume_count": attempt.get("resume_count", 0)})
        task_started = time.monotonic()
        initial_task_active = attempt.get("current_task_active_seconds", 0.0)
        self._active_task_clock = (task_started, initial_task_active, attempt, task)
        attempt_remaining = self.manifest["attempt_timeout_seconds"] - attempt.get("active_seconds", 0.0)
        deadline = task_started + min(self.manifest["task_timeout_seconds"] - initial_task_active,
                                      max(0.0, attempt_remaining))
        while True:
            host = broker.benchmark.process
            if host is None or host.poll() is not None:
                raise TrialError("owned Arena host exited during an active solver task")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self.event({"kind": "task_timeout", "attempt": attempt["number"], "task": task,
                            "active_seconds": time.monotonic() - task_started})
                attempt["tasks"].append({"number": task, "status": "task_failed", "reason": "solver_timeout",
                                         "active_seconds": initial_task_active + time.monotonic() - task_started})
                attempt["active_seconds"] += time.monotonic() - task_started
                attempt["status"] = "task_failed"
                return False
            try:
                message = self._read_solver(process, min(remaining, 0.25))
            except ProtocolError as exc:
                self.event({"kind": "protocol_error", "attempt": attempt["number"], "task": task, "error": str(exc)})
                elapsed = time.monotonic() - task_started
                attempt["tasks"].append({"number": task, "status": "task_failed", "reason": "malformed_solver_protocol",
                                         "active_seconds": initial_task_active + elapsed})
                attempt["active_seconds"] += elapsed
                attempt["status"] = "task_failed"
                return False
            if message is None:
                exit_code = process.poll()
                if exit_code is None:
                    continue
                if exit_code == 75 and self.manifest["solver"]["name"] == "mock":
                    attempt["status"] = "interrupted"
                    attempt["current_task_active_seconds"] = initial_task_active + time.monotonic() - task_started
                    attempt["active_seconds"] += time.monotonic() - task_started
                    attempt["interruption"] = {"kind": "mock_injected_interruption_signal", "task": task,
                                                "exit_code": exit_code, "utc": utc_now()}
                    attempt.setdefault("task_events", []).append({"task": task, "status": "infrastructure_interrupted",
                                                                  "reason": "mock_injected_interruption_signal",
                                                                  "active_seconds": attempt["current_task_active_seconds"]})
                    attempt["infra_intervals"].append({"started_at": utc_now(), "task": task})
                    self.state["counts"]["infrastructure_interrupted"] += 1
                    self.event({"kind": "infrastructure_interrupted", "attempt": attempt["number"], "task": task,
                                "source": "mock solver injected interruption signal; host remained available"})
                    return False
                if attempt.get("status") != "task_failed":
                    attempt["tasks"].append({"number": task, "status": "task_failed", "reason": "solver_exit",
                                             "exit_code": exit_code,
                                             "active_seconds": initial_task_active + time.monotonic() - task_started})
                attempt["active_seconds"] += time.monotonic() - task_started
                attempt["status"] = "task_failed"
                self.event({"kind": "solver_exit", "attempt": attempt["number"], "task": task, "exit_code": exit_code})
                return False
            kind = message.get("type")
            if kind == "request":
                self.solver_request_count += 1
                self.state["counts"]["solver_requests"] = self.solver_request_count
                if self.solver_request_count > self.manifest["max_requests"]:
                    elapsed = time.monotonic() - task_started
                    attempt["tasks"].append({"number": task, "status": "task_failed", "reason": "request_budget_exhausted",
                                             "active_seconds": initial_task_active + elapsed})
                    attempt["active_seconds"] += elapsed
                    attempt["status"] = "task_failed"
                    self.event({"kind": "request_budget_exhausted", "attempt": attempt["number"], "task": task})
                    return False
                reply, complete = broker.dispatch(message)
                if message.get("op") == "task_done" and reply.get("ok"):
                    duration = time.monotonic() - task_started
                    if attempt["tasks"] and attempt["tasks"][-1].get("number") == task:
                        attempt["tasks"][-1]["active_seconds"] = initial_task_active + duration
                    attempt["active_seconds"] += duration
                    attempt["current_task_active_seconds"] = 0.0
                    self.persist()
                self.event({"kind": "broker_request", "attempt": attempt["number"], "task": task,
                            "request_id": message.get("id"), "op": message.get("op"), "args": message.get("args"),
                            "reply": reply, "active_seconds": time.monotonic() - task_started})
                self._send(process, reply)
                if complete:
                    return attempt.get("status") != "task_failed"
            elif kind == "intervention":
                self.state["counts"]["interventions"] += 1
                attempt["interventions"].append({"task": task, "detail": message.get("detail"), "utc": utc_now()})
                self.event({"kind": "intervention", "attempt": attempt["number"], "task": task,
                            "detail": message.get("detail")})
                self.persist()
                if self.state["counts"]["interventions"] > self.manifest["max_interventions"]:
                    attempt["status"] = "intervened"
                    elapsed = time.monotonic() - task_started
                    attempt["tasks"].append({"number": task, "status": "task_failed", "reason": "intervention_limit",
                                             "active_seconds": initial_task_active + elapsed})
                    attempt["active_seconds"] += elapsed
                    return False
            else:
                elapsed = time.monotonic() - task_started
                attempt["tasks"].append({"number": task, "status": "task_failed", "reason": "unknown_protocol_message",
                                         "active_seconds": initial_task_active + elapsed})
                attempt["active_seconds"] += elapsed
                attempt["status"] = "task_failed"
                return False

    def judge_task(self, broker: Broker, number: int) -> dict[str, Any]:
        b = broker.benchmark
        facts: dict[str, Any] = {}
        if number == 1:
            actions = _expect_actions(broker.attempt, 1, ["read", "read", "read", "apply", "read"])
            baseline.require([item.get("what") for item in actions if item["op"] == "read"] ==
                             ["status", "schema", "scene", "history"], "task 1 omitted initial discovery")
            receipt = broker.attempt["facts"].get("task1_apply_receipt")
            baseline.require(isinstance(receipt, int) and not isinstance(receipt, bool) and receipt > 0,
                             "task 1 lacks a positive guarded-apply history receipt")
            initial = baseline.scene_facts(b.cli("scene", "--components"))
            baseline.assert_scene(initial, -300, 300)
            history = b.cli("history")["entries"]
            baseline.require(len(history) == 1, "task 1 must have exactly one accepted history entry")
            baseline.require(history[0].get("id") == receipt,
                             "task 1 accepted history entry does not match the guarded-apply receipt")
            checksum = b.cli("status")["state"]["doc_checksum"]
            facts.update(baseline_checksum=checksum, player_ids=[p["id"] for p in initial["players"]],
                         baseline_scene_checksum=initial["checksum"])
        elif number == 2:
            actions = _expect_actions(broker.attempt, 2,
                                      ["sim_start", "sim_input", "sim_step", "sim_input", "sim_step", "sim_stop"])
            inputs = [item for item in actions if item["op"] == "sim_input"]
            steps = [item["ticks"] for item in actions if item["op"] == "sim_step"]
            baseline.require([item["value"] for item in inputs] == [
                {"axis_x": 1, "axis_y": 0, "buttons": []},
                {"axis_x": 0, "axis_y": 0, "buttons": []}], "task 2 must issue right then neutral input")
            baseline.require(steps == [10, 5], "task 2 must step 10 ticks then 5 neutral ticks")
            snapshots = broker.attempt["facts"].get("task2_live_snapshots", [])
            baseline.require([item.get("tick") for item in snapshots] == [10, 15],
                             "task 2 lacks live position observations after both step phases")
            baseline.assert_scene(snapshots[0]["scene"], -240, 300)
            baseline.assert_scene(snapshots[1]["scene"], -240, 300)
            state = b.cli("status")["state"]
            baseline.require(state["mode"] == "edit", "task 2 did not stop simulation")
            baseline.require(state["doc_checksum"] == broker.baseline_checksum, "task 2 changed authoring document")
            stopped = baseline.scene_facts(b.cli("scene", "--components"))
            baseline.assert_scene(stopped, -300, 300)
            facts.update(movement_final=[-240, 0], document_unchanged=True)
        elif number == 3:
            actions = _expect_actions(broker.attempt, 3,
                                      ["sim_start", "sim_input", "sim_input", "sim_step", "sim_input",
                                       "sim_step", "sim_stop", "verify_replay"])
            inputs = [item for item in actions if item["op"] == "sim_input"]
            steps = [item["ticks"] for item in actions if item["op"] == "sim_step"]
            expected_shot_inputs = [
                {"player": 1, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
                {"player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}},
                {"player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
            ]
            baseline.require([{"player": item["player"], "value": item["value"]} for item in inputs] ==
                             expected_shot_inputs,
                             "task 3 must keep player 1 neutral, fire once as player 0, then release")
            baseline.require(steps == [1, 19], "task 3 must execute a 20-tick recording")
            baseline.require(actions[-1].get("replay") == "baseline20", "task 3 must request baseline replay verification")
            live = broker.attempt["facts"].get("task3_live_final", {})
            baseline.require(live.get("tick") == 20, "task 3 lacks its live tick-20 scene observation")
            baseline.assert_scene(live.get("scene", {}), -300, 300, score=1)
            replay = b.output / "baseline20.orrp"
            require_exists(replay)
            report = b.verify(replay, 20, "trial-task3-verdict.json")
            facts.update(baseline_replay=str(replay.name), recording=report["recording"],
                         debug_commands_replayed=report["debug_commands_replayed"])
        elif number == 4:
            actions = _expect_actions(broker.attempt, 4,
                                      ["read", "apply", "sim_start", "sim_input", "sim_input", "sim_step",
                                       "sim_input", *(["sim_step"] * 49),
                                       "sim_stop", "verify_replay", "undo", "redo"])
            inputs = [item for item in actions if item["op"] == "sim_input"]
            expected_shot_inputs = [
                {"player": 1, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
                {"player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}},
                {"player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
            ]
            baseline.require([{"player": item["player"], "value": item["value"]} for item in inputs] ==
                             expected_shot_inputs,
                             "task 4 must keep player 1 neutral, fire once as player 0, then release")
            baseline.require([item.get("ticks") for item in actions if item["op"] == "sim_step"] == [1] * 50,
                             "task 4 must collect a checksum after each of 50 ticks")
            live = broker.attempt["facts"].get("task4_live_final", {})
            baseline.require(live.get("tick") == 50, "task 4 lacks its live tick-50 scene observation")
            baseline.assert_scene(live.get("scene", {}), -300, 900, score=1)
            baseline.require(broker.attempt["facts"].get("task4_undo_seen") is True and
                             broker.attempt["facts"].get("task4_redo_seen") is True,
                             "task 4 must undo and redo the verified edit")
            baseline.require(broker.attempt["facts"].get("task4_undo_checksum") == broker.baseline_checksum,
                             "task 4 undo did not restore the exact baseline document")
            baseline.require(broker.attempt["facts"].get("task4_redo_checksum") == broker.modified_checksum,
                             "task 4 redo did not restore the exact modified document")
            baseline.assert_scene(broker.attempt["facts"]["task4_undo_scene"], -300, 300)
            baseline.assert_scene(broker.attempt["facts"]["task4_redo_scene"], -300, 900)
            apply_report = broker.attempt["facts"].get("task4_apply_verify")
            baseline.require(broker.attempt["facts"].get("task4_apply_accepted") is True and isinstance(apply_report, dict),
                             "task 4 did not make the guarded replay edit")
            baseline.check_report(apply_report, 20,
                                  {"base:score_0": 1, "score_0": 0, "players": 2,
                                   "score_1": 0, "out_of_bounds": 0}, recording=True)
            replay50 = b.output / "modified50.orrp"
            verify50 = b.verify(replay50, 50, "trial-task4-verdict.json")
            trace = broker.trace
            baseline.require(len(trace) == 51 and [item["tick"] for item in trace] == list(range(51)),
                             "task 4 must retain all 51 checksums")
            write_json(b.output / "task4-checksums.json", trace)
            baseline.require(b.cli("status")["state"]["doc_checksum"] == broker.modified_checksum,
                             "task 4 must finish with modified document after redo")
            facts.update(modified_checksum=broker.modified_checksum, task4_trace=trace,
                         recording=verify50["recording"], debug_commands_replayed=verify50["debug_commands_replayed"])
        elif number == 5:
            actions = _expect_actions(broker.attempt, 5,
                                      ["save", "restart_host", "sim_start", "sim_input", "sim_input", "sim_step",
                                       "sim_input", *(["sim_step"] * 49),
                                       "sim_stop", "verify_replay"])
            baseline.require(actions[0].get("write") is True and actions[1].get("generation", 0) >= 2,
                             "task 5 must save then restart the host")
            inputs = [item for item in actions if item["op"] == "sim_input"]
            expected_shot_inputs = [
                {"player": 1, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
                {"player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}},
                {"player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
            ]
            baseline.require([{"player": item["player"], "value": item["value"]} for item in inputs] ==
                             expected_shot_inputs,
                             "task 5 must keep player 1 neutral, fire once as player 0, then release")
            baseline.require([item.get("ticks") for item in actions if item["op"] == "sim_step"] == [1] * 50,
                             "task 5 must collect fresh-process checksums at every tick")
            baseline.require(actions[-1].get("replay") == "modified50", "task 5 must verify task 4's original replay")
            live = broker.attempt["facts"].get("task5_live_final", {})
            baseline.require(live.get("tick") == 50, "task 5 lacks its live tick-50 scene observation")
            baseline.assert_scene(live.get("scene", {}), -300, 900, score=1)
            status = b.cli("status")
            baseline.require(status["state"]["doc_checksum"] == broker.modified_checksum,
                             "fresh host did not load the saved modified scene")
            baseline.require(b.cli("history")["entries"] == [], "fresh host undo history is not empty")
            current = baseline.scene_facts(b.cli("scene", "--components"))
            baseline.assert_scene(current, -300, 900, score=0)
            trace = broker.trace
            reference = broker.attempt["facts"].get("task4_trace")
            baseline.require(len(trace) == 51 and [item["tick"] for item in trace] == list(range(51)),
                             "task 5 must retain all 51 checksums")
            baseline.require(trace == reference, "fresh-process live checksums differ from task 4")
            original = b.output / "modified50.orrp"
            verify = b.verify(original, 50, "trial-task5-original-replay-verdict.json")
            baseline.require(verify["recording"]["checked"] == 51 and verify["recording"]["mismatches"] == 0,
                             "original replay failed fresh-host verification")
            write_json(b.output / "task5-checksums.json", trace)
            facts.update(checksum_ticks_matched=51, recording=verify["recording"],
                         debug_commands_replayed=verify["debug_commands_replayed"])
        else:
            raise TrialError(f"unknown task {number}")
        return {"number": number, "status": "passed", "finished_at": utc_now(), "facts": facts}

    def _create_attempt(self, number: int) -> dict[str, Any]:
        return {"number": number, "status": "running", "started_at": utc_now(), "tasks": [],
                "facts": {}, "interventions": [], "infra_intervals": [], "next_task": 1,
                "active_seconds": 0.0, "wall_seconds": 0.0}

    @staticmethod
    def _clear_restarted_task_evidence(attempt: dict[str, Any]) -> None:
        """Discard partial facts for a task replayed from its pre-task checkpoint."""
        task = attempt["next_task"]
        facts = attempt.setdefault("facts", {})
        if task == 2:
            facts.pop("task2_live_snapshots", None)
        if task in (3, 4, 5):
            facts.pop(f"task{task}_live_final", None)
        if task == 4:
            for key in tuple(facts):
                if key.startswith("task4_"):
                    facts.pop(key, None)
            facts.pop("modified_checksum", None)

    def run_attempt(self, attempt: dict[str, Any], *, fresh: bool) -> None:
        attempt_wall_started = time.monotonic()
        segment_started_at = utc_now()
        attempt.setdefault("execution_segments", []).append({"started_at": segment_started_at})
        attempt_dir = self.attempt_dir(attempt["number"])
        attempt_dir.mkdir(parents=True, exist_ok=True)
        self.current_attempt_dir = attempt_dir
        benchmark: baseline.Benchmark | None = None
        process: subprocess.Popen[str] | None = None
        try:
            if not fresh:
                self._clear_restarted_task_evidence(attempt)
            if fresh:
                initial_checkpoint = attempt_dir / "checkpoints" / "task-1-before.scene.yaml"
                initial_checkpoint.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(self.repo / "scenes" / "arena_blank.scene.yaml", initial_checkpoint)
                attempt["resume_scene"] = str(initial_checkpoint.relative_to(self.output))
                attempt["checkpoint_task"] = 1
                attempt["last_accepted_task"] = 0
            benchmark = self._prepare_attempt(attempt, fresh=fresh)
            if benchmark.process is not None:
                benchmark.process._trial_job = self._attach_windows_job(benchmark.process)  # type: ignore[attr-defined]
            if fresh:
                self.checkpoint(attempt, benchmark, 1)
            while attempt["next_task"] <= 5:
                task = attempt["next_task"]
                self.checkpoint(attempt, benchmark, task)
                process = self._spawn_solver(attempt, task, attempt_dir)
                try:
                    done = self._solver_exchange(process, Broker(self, attempt, benchmark), attempt, task)
                except BaseException:
                    self._charge_interrupted_task_clock()
                    raise
                self._active_task_clock = None
                cleanup = self._stop_solver(process)
                attempt.setdefault("cleanup", []).append(cleanup)
                self.event({"kind": "solver_stop", "attempt": attempt["number"], "task": task, **cleanup})
                process = None
                if not done:
                    if attempt["status"] == "running":
                        attempt["status"] = "task_failed"
                    break
            if attempt["next_task"] > 5 and attempt["status"] == "running":
                attempt["status"] = "passed"
                attempt["finished_at"] = utc_now()
        except (Exception, KeyboardInterrupt) as exc:
            attempt["status"] = "interrupted"
            attempt["interruption"] = {"kind": "runner_or_infrastructure_exception", "error": str(exc),
                                        "traceback": traceback.format_exc(), "utc": utc_now()}
            self.state["counts"]["infrastructure_interrupted"] += 1
            attempt.setdefault("infra_intervals", []).append({"started_at": utc_now(),
                                                               "task": attempt.get("next_task"),
                                                               "reason": "runner_or_infrastructure_exception"})
            self.event({"kind": "runner_interrupted", "attempt": attempt["number"], "error": str(exc)})
        finally:
            if process is not None:
                cleanup = self._stop_solver(process)
                attempt.setdefault("cleanup", []).append(cleanup)
                self.event({"kind": "solver_stop", "attempt": attempt["number"],
                            "task": attempt.get("next_task"), **cleanup})
            if benchmark is not None:
                host_process = benchmark.process
                benchmark.stop_host()
                if host_process is not None:
                    job_closed = self._close_job(host_process)
                    host_cleanup = {"pid": host_process.pid, "exit_code": host_process.returncode,
                                    "job_closed": job_closed, "finished_at": utc_now()}
                    attempt.setdefault("cleanup", []).append({"host": host_cleanup})
                    self.event({"kind": "host_cleanup", "attempt": attempt["number"], **host_cleanup})
                attempt["hosts"] = benchmark.report.get("hosts", [])
                # Preserve the baseline helper's raw transcript and host logs.
                if benchmark.host_log is not None:
                    try:
                        benchmark.host_log.close()
                    except Exception:
                        pass
                write_json(benchmark.output / "lifecycle.json", {
                    "kind": "trial_runner_lifecycle_only",
                    "note": "Host lifecycle evidence; no scripted baseline solver was run.",
                    "report": benchmark.report,
                })
            segment_ended_at = utc_now()
            segment_elapsed = max(0.0, time.monotonic() - attempt_wall_started)
            attempt["execution_segments"][-1].update(ended_at=segment_ended_at,
                                                      wall_seconds=segment_elapsed)
            if attempt.get("status") == "interrupted":
                intervals = attempt.setdefault("infra_intervals", [])
                if intervals and "downtime_started_at" not in intervals[-1]:
                    intervals[-1]["segment_ended_at"] = segment_ended_at
                    intervals[-1]["downtime_started_at"] = segment_ended_at
            attempt["wall_seconds"] = attempt.get("wall_seconds", 0) + segment_elapsed
            attempt["finished_at"] = utc_now() if attempt["status"] != "interrupted" else attempt.get("finished_at")
            self.persist()

    def _charge_interrupted_task_clock(self) -> None:
        clock = getattr(self, "_active_task_clock", None)
        if clock is None:
            return
        started, initial_active, attempt, task = clock
        elapsed = max(0.0, time.monotonic() - started)
        attempt["current_task_active_seconds"] = initial_active + elapsed
        attempt["active_seconds"] = attempt.get("active_seconds", 0.0) + elapsed
        attempt.setdefault("task_events", []).append({"task": task, "status": "runner_interrupted",
                                                       "active_seconds": initial_active + elapsed})
        self._active_task_clock = None

    @classmethod
    def _close_job(cls, process: subprocess.Popen[str]) -> dict[str, Any]:
        job = getattr(process, "_trial_job", None)
        if not job:
            return {"attached": False, "terminated": False}
        try:
            kernel, handle = job["kernel"], job["handle"]
            kernel.TerminateJobObject.argtypes = [__import__("ctypes").c_void_p, __import__("ctypes").c_uint]
            kernel.TerminateJobObject.restype = __import__("ctypes").c_int
            terminated = bool(kernel.TerminateJobObject(handle, 1))
            kernel.CloseHandle.argtypes = [__import__("ctypes").c_void_p]
            closed = bool(kernel.CloseHandle(handle))
            return {"attached": bool(job.get("assigned")), "terminated": terminated, "closed": closed}
        except Exception:
            return {"attached": bool(job.get("assigned")), "terminated": False, "closed": False}

    @classmethod
    def _stop_solver(cls, process: subprocess.Popen[str]) -> dict[str, Any]:
        terminated_group = False
        if os.name != "nt":
            try:
                os.killpg(process.pid, signal.SIGTERM)
                terminated_group = True
            except OSError:
                pass
        elif process.poll() is None:
            try:
                process.terminate()
            except OSError:
                pass
        if process.poll() is None:
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                if os.name != "nt":
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                        terminated_group = True
                    except OSError:
                        process.kill()
                else:
                    process.kill()
                process.wait(timeout=2)
        if os.name != "nt":
            # Kill descendants that outlived an already-exited protocol parent.
            time.sleep(0.05)
            try:
                os.killpg(process.pid, signal.SIGKILL)
                terminated_group = True
            except OSError:
                pass
        job_closed = cls._close_job(process)
        for stream_name in ("stdin", "stdout"):
            stream = getattr(process, stream_name, None)
            if stream:
                stream.close()
        stderr_file = getattr(process, "_trial_stderr_file", None)
        if stderr_file:
            stderr_file.close()
        return {"pid": process.pid, "exit_code": process.returncode,
                "group_signal_used": terminated_group, "job_closed": job_closed,
                "workdir": getattr(process, "_trial_workdir", None), "finished_at": utc_now()}

    def run(self) -> int:
        pending = None
        if self.resume_mode:
            pending = next((a for a in reversed(self.state["attempts"]) if a.get("status") == "interrupted"), None)
            if pending is None:
                raise TrialError("no interrupted attempt found")
            pending["status"] = "running"
            pending.setdefault("resume_count", 0)
            pending["resume_count"] += 1
            pending["resumed_at"] = utc_now()
            pending.setdefault("infra_intervals", [])
            self._record_infrastructure_downtime(pending, pending["resumed_at"])
            pending.setdefault("resume_events", []).append({"resumed_at": utc_now(), "task": pending["next_task"]})
            self.event({"kind": "attempt_resumed", "attempt": pending["number"], "task": pending["next_task"]})
            self.run_attempt(pending, fresh=False)
            if pending["status"] == "interrupted":
                self.state["status"] = "interrupted"
                self._refresh_ledger()
                self.persist()
                self._write_artifact_manifest()
                return 2
            next_number = max((item["number"] for item in self.state["attempts"]), default=0) + 1
            for number in range(next_number, self.manifest["attempt_count"] + 1):
                attempt = self._create_attempt(number)
                self.state["attempts"].append(attempt)
                self.persist()
                self.run_attempt(attempt, fresh=True)
                if attempt["status"] == "interrupted":
                    self.state["status"] = "interrupted"
                    self._refresh_ledger()
                    self.persist()
                    self._write_artifact_manifest()
                    return 2
        else:
            for number in range(1, self.manifest["attempt_count"] + 1):
                attempt = self._create_attempt(number)
                self.state["attempts"].append(attempt)
                self.persist()
                self.run_attempt(attempt, fresh=True)
                if attempt["status"] == "interrupted":
                    self.state["status"] = "interrupted"
                    self._refresh_ledger()
                    self.persist()
                    self._write_artifact_manifest()
                    return 2
        self._finalize()
        return 0 if self.state["status"] == "passed" else 1

    def _finalize(self) -> None:
        self._refresh_ledger()
        counts = self.state["counts"]
        self.state["status"] = "passed" if counts["passed_attempts"] == self.manifest["attempt_count"] else "failed"
        self.state["finished_at"] = utc_now()
        self.persist()
        self._write_artifact_manifest()
