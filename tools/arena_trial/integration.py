"""Actual-host local mock scenarios; separate from model-quality measurements."""

from __future__ import annotations

import copy
import ctypes
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import time
import traceback
from typing import Any

from .runner import TrialRunner, freeze_mock_manifest, write_json


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def read_state(path: Path) -> dict[str, Any]:
    return json.loads((path / "trial.json").read_text(encoding="utf-8"))


def process_alive(pid: int, owned_started_at: str) -> bool:
    if os.name == "nt":
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.WaitForSingleObject.restype = wintypes.DWORD
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle.restype = wintypes.BOOL
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        kernel.GetProcessTimes.restype = wintypes.BOOL
        handle = kernel.OpenProcess(0x00101000, False, pid)  # SYNCHRONIZE | QUERY_LIMITED_INFORMATION
        if not handle:
            error = ctypes.get_last_error()
            if error == 87:  # ERROR_INVALID_PARAMETER: no process with this PID
                return False
            raise OSError(error, f"cannot verify process cleanup for pid {pid}")
        try:
            created, exited, system, user = (wintypes.FILETIME() for _ in range(4))
            if not kernel.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited),
                                          ctypes.byref(system), ctypes.byref(user)):
                raise OSError(ctypes.get_last_error(), f"cannot verify process identity for pid {pid}")
            created_unix = ((created.dwHighDateTime << 32) | created.dwLowDateTime) / 10_000_000 - 11_644_473_600
            if created_unix > datetime.fromisoformat(owned_started_at).timestamp():
                # Windows can reuse an exited PID during a later task. The
                # newer process is not the instance owned by this experiment.
                return False
            result = kernel.WaitForSingleObject(handle, 0)
            if result not in (0, 0x102):  # signalled or timeout
                raise OSError(ctypes.get_last_error(), f"cannot wait on pid {pid}")
            return result == 0x102
        finally:
            kernel.CloseHandle(handle)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def check_cleanup(output: Path) -> dict[str, list[int]]:
    hosts: set[int] = set()
    started: dict[int, str] = {}
    for path in output.rglob("commands.jsonl"):
        for line in path.read_text(encoding="utf-8").splitlines():
            item = json.loads(line)
            if item.get("kind") == "host_spawn":
                hosts.add(item["pid"])
                started[item["pid"]] = item["started_at"]
    require(bool(hosts), "actual-host integration did not preserve any host PID")
    solver_spawns = [item for item in
                     (json.loads(line) for line in (output / "events.jsonl").read_text(encoding="utf-8").splitlines())
                     if item.get("kind") == "solver_spawn"]
    solvers = {item["pid"] for item in solver_spawns}
    started.update({item["pid"]: item["utc"] for item in solver_spawns})
    require(bool(solvers), "actual-host integration did not preserve any solver PID")
    cleanup = {item["pid"]: item for attempt in read_state(output)["attempts"]
               for item in attempt.get("cleanup", []) if "pid" in item}
    require(solvers <= cleanup.keys(), "solver spawn has no retained cleanup result")
    require(all(cleanup[pid].get("exit_code") is not None for pid in solvers),
            "solver cleanup did not retain an exit code")
    for kind, pids in (("host", hosts), ("solver", solvers)):
        for pid in pids:
            require(not process_alive(pid, started[pid]), f"owned {kind} pid {pid} survived runner cleanup")
    return {"host_pids": sorted(hosts), "solver_pids": sorted(solvers)}


def check_success(state: dict[str, Any], attempts: int) -> None:
    require(state["status"] == "passed", "mock success experiment did not pass")
    require(len(state["attempts"]) == attempts, "scheduled attempt omitted")
    counts = state["counts"]
    require(counts["passed_attempts"] == attempts and counts["failed_attempts"] == 0,
            "success attempt denominator differs")
    require(counts["scheduled_task_cells"] == attempts * 5 and
            counts["completed_task_cells"] == attempts * 5 and
            counts["passed_task_cells"] == attempts * 5 and
            counts["failed_task_cells"] == 0 and counts["unstarted_task_cells"] == 0,
            "success task denominator differs")
    require(state["usage"] == {"tokens": "unknown", "cost_usd": "unknown"},
            "mock usage must remain unknown")
    for attempt in state["attempts"]:
        facts = attempt["facts"]
        require(facts["checksum_ticks_matched"] == 51, "all 51 live checksums were not compared")
        require(facts["recording"]["checked"] == 51 and
                facts["recording"]["mismatches"] == 0 and
                facts["debug_commands_replayed"] == 0,
                "original replay was not independently verified")


def run(repo: Path, host: Path, cli: Path, output: Path) -> int:
    output.mkdir(parents=True, exist_ok=False)
    summary: dict[str, Any] = {
        "format": "orr.arena-agent-trial-integration/1",
        "kind": "mock_harness_validation_not_model_quality",
        "started_at": datetime.now(timezone.utc).isoformat(),
        "scenarios": [],
    }
    start = time.monotonic()
    try:
        frozen = freeze_mock_manifest(repo, host, cli)
        write_json(output / "frozen-provenance.json", frozen)
        for scenario in ("success", "task2_failure", "infra_once"):
            manifest = copy.deepcopy(frozen)
            manifest["experiment_id"] = "integration-" + scenario
            manifest["mock_scenario"] = scenario
            manifest["attempt_count"] = 2 if scenario == "infra_once" else 1
            # The mock is prompt, while finite limits still catch hangs.
            manifest["task_timeout_seconds"] = 60
            manifest["attempt_timeout_seconds"] = 300
            path = output / scenario
            record: dict[str, Any] = {"scenario": scenario, "exit_codes": []}
            summary["scenarios"].append(record)
            write_json(output / "summary.json", summary)
            runner = TrialRunner(path, manifest, repo=repo, host_bin=host, cli_bin=cli)
            code = runner.run()
            record["exit_codes"].append(code)
            state = read_state(path)
            if scenario == "success":
                require(code == 0, "success scenario failed")
                check_success(state, 1)
            elif scenario == "task2_failure":
                require(code == 1 and state["status"] == "failed", "ordinary task failure misclassified")
                require(state["counts"]["failed_attempts"] == 1 and
                        state["counts"]["passed_task_cells"] == 1 and
                        state["counts"]["failed_task_cells"] == 1 and
                        state["counts"]["unstarted_task_cells"] == 3,
                        "ordinary task failure denominator differs")
                require(state["counts"]["infrastructure_interrupted"] == 0,
                        "task failure must not become infrastructure exclusion")
                require(len(state["attempts"][0]["tasks"]) == 2,
                        "failure attempt raw task records omitted")
            else:
                require(code == 2 and state["status"] == "interrupted", "outage must interrupt experiment")
                first = copy.deepcopy(state)
                require(first["counts"]["scheduled_task_cells"] == 10 and
                        first["counts"]["completed_task_cells"] == 1 and
                        first["counts"]["started_task_cells"] == 2 and
                        first["counts"]["unstarted_task_cells"] == 8 and
                        first["counts"]["unresolved_task_cells"] == 9,
                        "interrupted ledger confused unresolved with unstarted task cells")
                prefix = (path / "events.jsonl").read_bytes()
                while code == 2 and len(record["exit_codes"]) <= 3:
                    check_cleanup(path)
                    before = read_state(path)
                    pending = next(a for a in before["attempts"] if a["status"] == "interrupted")
                    number, requests, active = pending["number"], before["counts"]["solver_requests"], pending["active_seconds"]
                    resumed = TrialRunner(path, manifest, resume=True, repo=repo, host_bin=host, cli_bin=cli)
                    code = resumed.run()
                    record["exit_codes"].append(code)
                    state = read_state(path)
                    same = next(a for a in state["attempts"] if a["number"] == number)
                    require(same.get("resume_count", 0) >= 1 and same["active_seconds"] >= active,
                            "resume reset attempt identity or active duration")
                    require(state["counts"]["solver_requests"] > requests,
                            "resume reset request budget accounting")
                    require((path / "events.jsonl").read_bytes().startswith(prefix),
                            "resume replaced the interrupted transcript")
                require(code == 0, "resumed scheduled attempts did not finish")
                check_success(state, 2)
                require(state["counts"]["infrastructure_interrupted"] >= 1,
                        "resumed success erased interruption")
                require(first["attempts"][0]["number"] == state["attempts"][0]["number"],
                        "resume created a replacement attempt")
                record["preserved_initial_interrupted_attempt"] = first
            record["stopped_processes"] = check_cleanup(path)
            record["status"] = "passed"
            write_json(output / "summary.json", summary)
        summary["status"] = "passed"
        return 0
    except (Exception, KeyboardInterrupt) as exc:
        summary["status"] = "failed"
        summary["error"] = str(exc)
        summary["traceback"] = traceback.format_exc()
        print(summary["traceback"], flush=True)
        return 1
    finally:
        summary["elapsed_seconds"] = time.monotonic() - start
        summary["finished_at"] = datetime.now(timezone.utc).isoformat()
        write_json(output / "summary.json", summary)
        print(json.dumps({"integration_status": summary["status"],
                          "output": str(output), "elapsed_seconds": summary["elapsed_seconds"]}), flush=True)
