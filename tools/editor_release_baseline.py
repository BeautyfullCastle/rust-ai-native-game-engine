#!/usr/bin/env python3
"""Collect fixed release editor and renderer measurements without hiding failures."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shlex
import signal
import statistics
import subprocess
import sys
import threading
import time
from typing import Any, Iterable


REPO = Path(__file__).resolve().parents[1]
PREFIX = "ORR_BASELINE "
MAX_RUN_SECONDS = 40 * 60
MAX_LOG_BYTES = 128 * 1024 * 1024
MAX_RECORD_LINE = 64 * 1024
MAX_RECORDS = 600
GPU_RUNS = (
    ("gpu_2d_default", ["cargo", "test", "-p", "orr_render", "--test", "gpu",
                        "release_baseline_2d", "--release", "--locked", "--", "--ignored", "--exact",
                        "--nocapture", "--test-threads=1"]),
    ("gpu_3d_low", ["cargo", "test", "-p", "orr_render", "--test", "gpu3d",
                    "release_baseline_3d_low", "--release", "--locked", "--", "--ignored", "--exact",
                    "--nocapture", "--test-threads=1"]),
    ("gpu_3d_default", ["cargo", "test", "-p", "orr_render", "--test", "gpu3d",
                        "release_baseline_3d_default", "--release", "--locked", "--", "--ignored", "--exact",
                        "--nocapture", "--test-threads=1"]),
)
BUILD_RUNS = (
    ("build_editor_measure", ["cargo", "test", "-p", "orr_editor", "--test", "measure", "--release", "--locked", "--no-run", "--message-format=json"]),
    ("build_renderer_2d", ["cargo", "test", "-p", "orr_render", "--test", "gpu", "--release", "--locked", "--no-run", "--message-format=json"]),
    ("build_renderer_3d", ["cargo", "test", "-p", "orr_render", "--test", "gpu3d", "--release", "--locked", "--no-run", "--message-format=json"]),
)
EDITOR_RUN = ("editor_measure", ["cargo", "test", "-p", "orr_editor", "--test", "measure",
                                 "--release", "--locked", "--", "--nocapture", "--test-threads=1"])
SAFE_ENV_KEYS = (
    "ORR_REQUIRE_GPU", "ORR_BASELINE_GPU_MODE", "ORR_BASELINE_SOFTWARE",
    "RUST_BACKTRACE", "CARGO_TERM_COLOR", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET",
    "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "WGPU_BACKEND", "WGPU_ADAPTER_NAME",
    "VK_ICD_FILENAMES", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL",
    "CARGO_HOME", "RUSTUP_HOME", "RUSTC", "RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER",
    "RUSTDOCFLAGS", "CARGO_ENCODED_RUSTDOCFLAGS", "CARGO_NET_OFFLINE", "CARGO_NET_RETRY",
    "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_DEBUG", "CARGO_PROFILE_RELEASE_LTO",
    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO",
    "CARGO_PROFILE_RELEASE_STRIP", "CARGO_PROFILE_RELEASE_INCREMENTAL", "CARGO_PROFILE_RELEASE_PANIC",
    "CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS", "CARGO_PROFILE_RELEASE_RPATH",
)
GPU_COMMON = {
    "schema_version": 1,
    "suite": "renderer",
    "instance_count": 1000,
    "resolution_px": [640, 360],
}
GPU_EXPECTED = {
    "gpu_2d_default": {"renderer": "2d", "preset": "default", "scene_id": "grid_1000_v1",
                        "target_format": "Rgba8Unorm"},
    "gpu_3d_low": {"renderer": "3d", "preset": "low", "scene_id": "sphere_grid_1000_v1",
                    "target_format": "Rgba8UnormSrgb",
                    "settings": {"requested_msaa": 1, "shadow_map_size": 512, "mesh_segments": 12},
                    "shadows": True},
    "gpu_3d_default": {"renderer": "3d", "preset": "default", "scene_id": "sphere_grid_1000_v1",
                        "target_format": "Rgba8UnormSrgb",
                        "settings": {"requested_msaa": 4, "shadow_map_size": 2048, "mesh_segments": 32},
                        "shadows": True},
}


class BaselineError(RuntimeError):
    pass


def _run_text(argv: list[str], *, cwd: Path = REPO, timeout: int = 30) -> str:
    completed = subprocess.run(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", errors="replace",
                               timeout=timeout, check=False)
    if completed.returncode != 0:
        raise BaselineError(f"command failed ({completed.returncode}): {shlex.join(argv)}\n{completed.stderr[-2000:]}")
    return completed.stdout.strip()


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _source_identity(repo: Path) -> dict[str, Any]:
    head = _run_text(["git", "rev-parse", "HEAD"], cwd=repo)
    tree = _run_text(["git", "rev-parse", "HEAD^{tree}"], cwd=repo)
    status = _run_text(["git", "status", "--porcelain=v1", "--untracked-files=all"], cwd=repo)
    files_raw = subprocess.run(["git", "ls-files", "-z", "--", "crates/orr_editor", "crates/orr_render",
                                "scenes", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"], cwd=repo,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if files_raw.returncode:
        raise BaselineError("git ls-files could not inventory measured sources")
    names = [part.decode("utf-8", "strict") for part in files_raw.stdout.split(b"\0") if part]
    digest = hashlib.sha256()
    for name in sorted(names):
        path = repo / name
        if not path.is_file():
            raise BaselineError(f"tracked source disappeared: {name}")
        digest.update(name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(bytes.fromhex(_sha256_file(path)))
    return {"head": head, "tree": tree, "dirty_status": status,
            "source_file_count": len(names), "source_sha256": digest.hexdigest()}


def _require_clean(identity: dict[str, Any]) -> None:
    if identity["dirty_status"]:
        raise BaselineError("release baseline requires a clean exact commit; git status is not empty")


def _output_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    if not path.is_absolute():
        path = repo / path
    path = path.resolve(strict=False)
    try:
        relative = path.relative_to(repo.resolve())
    except ValueError as exc:
        raise BaselineError("--output must be inside this repository's ignored output area") from exc
    if path.exists() or path.is_symlink():
        raise BaselineError(f"--output already exists; refusing overwrite: {path}")
    checked = subprocess.run(["git", "check-ignore", "--", relative.as_posix()], cwd=repo,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if checked.returncode != 0:
        raise BaselineError("--output must be covered by .gitignore")
    return path


def _toolchain() -> dict[str, str]:
    rustc = _run_text(["rustc", "--version", "--verbose"])
    cargo = _run_text(["cargo", "--version"])
    git = _run_text(["git", "--version"])
    return {"rustc_verbose": rustc, "cargo": cargo, "git": git}


def _cpu_metadata() -> dict[str, Any]:
    model = platform.processor().strip()
    if os.name == "nt":
        try:
            import winreg
            key = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                                 r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
            try:
                model, _ = winreg.QueryValueEx(key, "ProcessorNameString")
            finally:
                winreg.CloseKey(key)
        except OSError:
            pass
    else:
        try:
            with Path("/proc/cpuinfo").open(encoding="utf-8", errors="replace") as stream:
                for line in stream:
                    label, separator, value = line.partition(":")
                    if separator and label.strip().lower() in {"model name", "hardware", "processor"} and value.strip():
                        model = value.strip()
                        break
        except OSError:
            pass
    return {"model": model or None, "logical_cpu_count": os.cpu_count()}


def _power_metadata() -> dict[str, str] | None:
    if os.name == "nt":
        try:
            return {"active_scheme": _run_text(["powercfg", "/getactivescheme"], timeout=10)}
        except (BaselineError, OSError, subprocess.TimeoutExpired):
            return None
    for path in (Path("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                 Path("/sys/devices/system/cpu/intel_pstate/no_turbo")):
        try:
            return {"source": str(path), "value": path.read_text(encoding="ascii").strip()}
        except OSError:
            continue
    return None


def _safe_environment(env: dict[str, str]) -> dict[str, str | None]:
    return {key: env.get(key) for key in SAFE_ENV_KEYS}


def _command_environment(suite_name: str, gpu_mode: str) -> dict[str, str]:
    env = os.environ.copy()
    if suite_name.startswith("gpu_"):
        env["ORR_REQUIRE_GPU"] = "1"
        env["ORR_BASELINE_GPU_MODE"] = "software" if gpu_mode == "software" else "hardware"
        env["ORR_BASELINE_SOFTWARE"] = "1" if gpu_mode == "software" else "0"
    else:
        for key in ("ORR_REQUIRE_GPU", "ORR_BASELINE_GPU_MODE", "ORR_BASELINE_SOFTWARE"):
            env.pop(key, None)
    return env


def _planned_commands(repetitions: int, gpu_mode: str, repo: Path) -> list[dict[str, Any]]:
    planned: list[dict[str, Any]] = []
    for name, command in BUILD_RUNS:
        env = _command_environment(name, gpu_mode)
        planned.append({"id": name, "phase": "build", "suite_name": name,
                        "argv": command, "cwd": str(repo), "env_allowlist": _safe_environment(env),
                        "default_features": True})
    for repetition in range(1, repetitions + 1):
        for name, command in (EDITOR_RUN, *GPU_RUNS):
            env = _command_environment(name, gpu_mode)
            planned.append({"id": f"rep-{repetition:02d}-{name}", "phase": "measurement",
                            "suite_name": name, "repetition": repetition, "argv": command,
                            "cwd": str(repo), "env_allowlist": _safe_environment(env),
                            "default_features": True})
    return planned


def _atomic_json(path: Path, value: Any) -> None:
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n", encoding="utf-8")
    os.replace(temp, path)


def _terminate_owned(process: subprocess.Popen[bytes]) -> dict[str, Any]:
    action = "already_exited"
    try:
        if os.name == "nt":
            # Cargo owns its rustc/test descendants. Terminate only the tree rooted at
            # this collector-created PID; never enumerate or kill unrelated processes.
            if process.poll() is None:
                killed = subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, timeout=8, check=False)
                action = "taskkill_owned_tree" if killed.returncode == 0 else "taskkill_owned_tree_failed"
                process.wait(timeout=5)
                verified = killed.returncode == 0
            else:
                action = "parent_exited_before_tree_cleanup"
                verified = False
            return {"action": action, "exit_code": process.poll(), "tree_cleanup_verified": verified,
                    **({} if verified else {"cleanup_error": "owned tree could not be verified terminated"})}
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return {"action": "owned_group_already_exited", "exit_code": process.poll(),
                    "tree_cleanup_verified": True}
        action = "terminate_owned_group"
        deadline = time.monotonic() + 5
        try:
            process.wait(timeout=0.2)
        except subprocess.TimeoutExpired:
            pass
        while time.monotonic() < deadline:
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.05)
        try:
            os.killpg(process.pid, 0)
            os.killpg(process.pid, signal.SIGKILL)
            action = "kill_owned_group"
        except ProcessLookupError:
            pass
        process.wait(timeout=5)
        try:
            os.killpg(process.pid, 0)
            verified = False
        except ProcessLookupError:
            verified = True
        return {"action": action, "exit_code": process.poll(), "tree_cleanup_verified": verified}
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"action": action, "cleanup_error": str(exc), "exit_code": process.poll(),
                "tree_cleanup_verified": False}


def _launch(argv: list[str], env: dict[str, str], cwd: Path, run_dir: Path) -> dict[str, Any]:
    out_path, err_path = run_dir / "stdout.log", run_dir / "stderr.log"
    started = utc_now()
    start_clock = time.monotonic()
    kwargs: dict[str, Any] = {}
    if os.name == "nt":
        kwargs["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        kwargs["start_new_session"] = True
    timed_out = False
    interrupted = False
    launch_error = None
    cleanup = {"action": "not_needed", "exit_code": None}
    overflow = threading.Event()
    stream_errors: list[str] = []

    def drain(source: Any, path: Path) -> None:
        written = 0
        try:
            with path.open("wb") as destination:
                while chunk := source.read(64 * 1024):
                    remaining = MAX_LOG_BYTES - written
                    if remaining > 0:
                        kept = chunk[:remaining]
                        destination.write(kept)
                        written += len(kept)
                    if len(chunk) > max(remaining, 0):
                        overflow.set()
        except OSError as exc:
            stream_errors.append(f"{path.name}: {exc}")
            overflow.set()
        finally:
            try:
                source.close()
            except OSError:
                pass

    try:
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0, **kwargs)
    except OSError as exc:
        launch_error = str(exc)
        process = None
    if process is not None:
        assert process.stdout is not None and process.stderr is not None
        readers = [threading.Thread(target=drain, args=(process.stdout, out_path), daemon=True),
                   threading.Thread(target=drain, args=(process.stderr, err_path), daemon=True)]
        for reader in readers:
            reader.start()
        deadline = start_clock + MAX_RUN_SECONDS
        try:
            while process.poll() is None:
                if overflow.is_set():
                    launch_error = f"raw process log exceeded {MAX_LOG_BYTES} byte per-stream limit"
                    cleanup = _terminate_owned(process)
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    cleanup = _terminate_owned(process)
                    break
                try:
                    process.wait(timeout=min(0.2, remaining))
                except subprocess.TimeoutExpired:
                    continue
        except KeyboardInterrupt:
            interrupted = True
            cleanup = _terminate_owned(process)
        for reader in readers:
            reader.join(timeout=10)
        if any(reader.is_alive() for reader in readers):
            launch_error = launch_error or "raw log reader did not stop after child termination"
            if not timed_out and not interrupted:
                cleanup = _terminate_owned(process)
                for reader in readers:
                    reader.join(timeout=5)
        if stream_errors:
            launch_error = "; ".join(([launch_error] if launch_error else []) + stream_errors)
        if overflow.is_set() and launch_error is None:
            launch_error = f"raw process log exceeded {MAX_LOG_BYTES} byte per-stream limit"
        if cleanup["action"] == "not_needed" and process.poll() is not None:
            cleanup = {"action": "normal_exit", "exit_code": process.returncode,
                       "parent_exit_verified": True,
                       "stdio_closed": not any(reader.is_alive() for reader in readers)}
    elapsed = time.monotonic() - start_clock
    return {"argv": argv, "cwd": str(cwd), "started_at": started,
            "finished_at": utc_now(), "elapsed_seconds": elapsed,
            "timeout_seconds": MAX_RUN_SECONDS, "timed_out": timed_out,
            "interrupted": interrupted,
            "exit_code": process.returncode if process is not None else None,
            "child_cleanup_verified": cleanup.get(
                "tree_cleanup_verified",
                process is None or (cleanup.get("parent_exit_verified", False)
                                    and cleanup.get("stdio_closed", False))),
            "launch_error": launch_error, "cleanup": cleanup,
            "stdout_file": out_path.name, "stderr_file": err_path.name}


def _records(run_dir: Path) -> tuple[list[dict[str, Any]], list[str]]:
    found: list[dict[str, Any]] = []
    errors: list[str] = []
    for name in ("stdout.log", "stderr.log"):
        path = run_dir / name
        if not path.is_file():
            errors.append(f"missing raw log {name}")
            continue
        if path.stat().st_size > MAX_LOG_BYTES:
            errors.append(f"raw log exceeds {MAX_LOG_BYTES} bytes: {name}")
            continue
        with path.open("rb") as stream:
            for number, raw in enumerate(stream, 1):
                if len(found) >= MAX_RECORDS:
                    errors.append(f"ORR_BASELINE record count exceeds parser limit {MAX_RECORDS}")
                    break
                if len(raw) > MAX_RECORD_LINE:
                    if raw.startswith(PREFIX.encode()):
                        errors.append(f"oversize ORR_BASELINE record in {name}:{number}")
                    continue
                line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                pos = line.find(PREFIX)
                if pos < 0:
                    continue
                try:
                    value = json.loads(line[pos + len(PREFIX):], parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))
                except (json.JSONDecodeError, ValueError, RecursionError) as exc:
                    errors.append(f"malformed ORR_BASELINE in {name}:{number}: {exc}")
                    continue
                if not isinstance(value, dict):
                    errors.append(f"ORR_BASELINE must be a JSON object in {name}:{number}")
                    continue
                found.append(value)
    return found, errors


def _is_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _finite_numeric(value: Any) -> bool:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    try:
        return math.isfinite(value)
    except OverflowError:
        return False


def _finite(value: Any, label: str, errors: list[str], *, nullable: bool = False) -> None:
    if nullable and value is None:
        return
    if not _finite_numeric(value) or value < 0:
        errors.append(f"{label} must be finite nonnegative numeric" + (" or null" if nullable else ""))


def _aggregate(values: list[Any]) -> dict[str, Any]:
    usable = [float(value) for value in values if _finite_numeric(value)]
    if not usable:
        return {"count": 0, "available": False, "mean": None, "median": None, "p95": None, "max": None}
    ordered = sorted(usable)
    return {"count": len(usable), "available": True, "mean": statistics.fmean(usable),
            "median": statistics.median(ordered), "p95": ordered[max(0, math.ceil(len(ordered) * .95) - 1)],
            "max": ordered[-1]}


def _validate_editor(records: list[dict[str, Any]], errors: list[str]) -> dict[str, Any]:
    editor = [r for r in records if r.get("suite") == "editor"]
    for r in editor:
        if r.get("schema_version") != 1:
            errors.append("editor record has unsupported schema_version")
        if r.get("backend") != "egui_kittest" or r.get("erp_connected") is not False or r.get("gpu") is not False:
            errors.append("editor record has unexpected backend/ERP/GPU metadata")
        if r.get("window_px") != [1500, 900]:
            errors.append("editor record has unexpected window resolution")
    drag = sorted((r for r in editor if r.get("case") == "drag_move"), key=lambda x: x.get("move_index", -1))
    frames = [r for r in editor if r.get("case") == "ui_frame"]
    diagnostics = [r for r in editor if r.get("case") == "diagnostics"]
    expected_cases = {"drag_move": 40, "ui_frame": 540, "diagnostics": 5}
    if len(editor) != sum(expected_cases.values()):
        errors.append("editor record count differs from fixed 40+540+5 sample schedule")
    for case, count in expected_cases.items():
        actual = sum(1 for r in editor if r.get("case") == case)
        if actual != count:
            errors.append(f"editor {case} records: expected {count}, got {actual}")
    if len(drag) != 40:
        errors.append(f"editor drag_move records: expected 40, got {len(drag)}")
    if [r.get("move_index") for r in drag] != list(range(1, 41)):
        errors.append("editor drag_move indices must be exactly 1..40")
    if any(r.get("group") != ("first" if i == 1 else "steady") for i, r in enumerate(drag, 1)):
        errors.append("editor drag groups must be one first plus 39 steady samples")
    drag_body_counts = [r.get("body_count") for r in drag]
    if any(not _is_int(value) or value <= 0 for value in drag_body_counts):
        errors.append("editor drag body_count must be a positive integer")
    elif len(set(drag_body_counts)) > 1:
        errors.append("editor drag scene body_count changed during the sample")
    expected_phase = {"edit": 60, "play_1x": 240, "play_4x": 240}
    for phase, count in expected_phase.items():
        selected = [r for r in frames if r.get("phase") == phase]
        if len(selected) != count:
            errors.append(f"editor {phase} frame records: expected {count}, got {len(selected)}")
        if [r.get("frame_index") for r in selected] != list(range(count)):
            errors.append(f"editor {phase} frame indices must be exactly 0..{count - 1}")
    if not diagnostics:
        errors.append("editor diagnostics records are missing")
    expected_diagnostic_phases = {"drag", "edit", "play 1x", "play 4x", "600-tick step"}
    if len(diagnostics) != 5 or {r.get("phase") for r in diagnostics} != expected_diagnostic_phases:
        errors.append("editor diagnostics phases are missing or duplicated")
    diagnostic_availability: dict[str, Any] = {}
    metric_names = {"ui_frame", "pump", "sync_erp_wait", "async_request", "snapshot_extract"}
    for d in diagnostics:
        expected_body_count = (drag_body_counts[0] if d.get("phase") == "drag" and drag_body_counts
                               else (drag_body_counts[0] + 1000 if drag_body_counts else None))
        if d.get("body_count") != expected_body_count:
            errors.append(f"editor diagnostics {d.get('phase')!r} body_count differs from scene")
        expected_scene = "demo_editor" if d.get("phase") == "drag" else "demo_editor_1000_bodies"
        if d.get("scene_id") != expected_scene:
            errors.append(f"editor diagnostics {d.get('phase')!r} scene_id differs from phase")
        metrics = d.get("metrics")
        if not isinstance(metrics, dict) or set(metrics) != metric_names:
            errors.append(f"editor diagnostics {d.get('phase')!r} has incomplete metrics")
            continue
        phase_result: dict[str, Any] = {}
        for metric_name, stats in metrics.items():
            if not isinstance(stats, dict):
                errors.append(f"editor diagnostic {metric_name} is not an object")
                continue
            samples, total = stats.get("samples"), stats.get("total_samples")
            if not _is_int(samples) or samples < 0 or not _is_int(total) or total < 0:
                errors.append(f"editor diagnostic {metric_name} sample counts must be nonnegative integers")
                continue
            if total < samples:
                errors.append(f"editor diagnostic {metric_name} total_samples is smaller than recent samples")
            values = {key: stats.get(key) for key in ("last_ms", "max_ms", "p95_ms")}
            for key, value in values.items():
                _finite(value, f"editor diagnostic {metric_name}.{key}", errors)
            if samples == 0 and any(value != 0 for value in values.values()):
                errors.append(f"editor diagnostic {metric_name} has timings without samples")
            phase_result[metric_name] = {"available": samples > 0, "samples": samples,
                                         "total_samples": total, "raw_ms": values}
        diagnostic_availability[d.get("phase", "unknown")] = phase_result
    for index, r in enumerate(drag):
        if not _is_int(r.get("frames")) or r["frames"] <= 0:
            errors.append(f"editor drag frame sample {index} must be a positive integer")
        _finite(r.get("wall_ms"), f"editor drag wall_ms[{index}]", errors)
    for index, r in enumerate(frames):
        if not _is_int(r.get("frame_index")) or r["frame_index"] < 0:
            errors.append(f"editor ui frame index[{index}] must be a nonnegative integer")
        _finite(r.get("wall_ms"), f"editor ui wall_ms[{index}]", errors)
        if not drag_body_counts or not _is_int(r.get("body_count")) or r.get("body_count") != drag_body_counts[0] + 1000:
            errors.append("editor play/edit frame body_count differs from the demo scene plus 1000")
        if r.get("scene_id") != "demo_editor_1000_bodies":
            errors.append("editor frame sample has unexpected scene_id")
    if any(r.get("scene_id") != "demo_editor" for r in drag):
        errors.append("editor drag sample has unexpected scene_id")
    return {
        "identity": {"body_count": drag_body_counts[0] if drag_body_counts else None,
                     "window_px": [1500, 900], "backend": "egui_kittest"},
        "drag_first": {"frames": drag[0].get("frames"), "wall_ms": drag[0].get("wall_ms")} if drag else None,
        "drag_steady_39_wall_ms_summary": _aggregate([r.get("wall_ms") for r in drag[1:]]),
        "drag_steady_39_frames_summary": _aggregate([r.get("frames") for r in drag[1:]]),
        "drag_steady_39_wall_ms": [r.get("wall_ms") for r in drag[1:]],
        "drag_steady_39_frames": [r.get("frames") for r in drag[1:]],
        "ui_frame_wall_ms_by_phase": {
            phase: {"samples": [r.get("wall_ms") for r in frames if r.get("phase") == phase],
                    "summary": _aggregate([r.get("wall_ms") for r in frames if r.get("phase") == phase])}
            for phase in expected_phase
        },
        "diagnostic_records": len(diagnostics),
        "diagnostic_availability": diagnostic_availability,
    }


def _gpu_signature(record: dict[str, Any]) -> dict[str, Any]:
    keys = ("renderer", "preset", "scene_id", "instance_count", "resolution_px", "target_format",
            "adapter_name", "adapter_backend", "adapter_device_type", "adapter_vendor", "adapter_device",
            "driver", "driver_info", "software", "settings", "shadows")
    return {key: record.get(key) for key in keys}


GPU_FRAME_IDENTITY_KEYS = ("renderer", "preset", "scene_id", "instance_count", "resolution_px",
                           "adapter_name", "adapter_backend", "adapter_device_type", "adapter_vendor",
                           "adapter_device", "driver", "driver_info", "software")


def _validate_gpu(run_name: str, records: list[dict[str, Any]], errors: list[str],
                  gpu_mode: str = "default") -> dict[str, Any] | None:
    expected = GPU_EXPECTED[run_name]
    if len(records) != 43 or any(r.get("case") not in {"metadata", "frame", "readback"} for r in records):
        errors.append(f"{run_name}: expected exactly 1 metadata + 41 frame + 1 readback records")
    if any(r.get("suite") != "renderer" for r in records):
        errors.append(f"{run_name}: unexpected non-renderer ORR_BASELINE record")
    metadata = [r for r in records if r.get("case") == "metadata"]
    frames = [r for r in records if r.get("case") == "frame"]
    readbacks = [r for r in records if r.get("case") == "readback"]
    if len(metadata) != 1:
        errors.append(f"{run_name}: expected one adapter metadata record, got {len(metadata)}")
        meta = metadata[0] if metadata else {}
    else:
        meta = metadata[0]
    for key, value in {**GPU_COMMON, **expected}.items():
        if meta.get(key) != value:
            errors.append(f"{run_name}: metadata {key} mismatch (expected {value!r}, got {meta.get(key)!r})")
    required_text = ("adapter_name", "adapter_backend", "adapter_device_type", "driver", "driver_info")
    for key in required_text:
        if not isinstance(meta.get(key), str) or not meta[key].strip():
            errors.append(f"{run_name}: metadata {key} is missing")
    if not _is_int(meta.get("adapter_vendor")) or not _is_int(meta.get("adapter_device")):
        errors.append(f"{run_name}: adapter vendor/device ids must be integers")
    if not isinstance(meta.get("software"), bool):
        errors.append(f"{run_name}: software must be a bool")
    if meta.get("case") != "metadata":
        errors.append(f"{run_name}: invalid metadata case")
    if meta.get("renderer_initialization_included") is not False or meta.get("target_creation_included") is not False:
        errors.append(f"{run_name}: renderer/target construction must be excluded from frame timing")
    if meta.get("cold_scope") != "first_frame_after_renderer_construction":
        errors.append(f"{run_name}: cold frame scope is missing or unexpected")
    if gpu_mode == "software" and meta.get("software") is not True:
        errors.append(f"{run_name}: software mode did not resolve to a software adapter")
    if gpu_mode == "default" and meta.get("software") is True:
        errors.append(f"{run_name}: default mode resolved to software; hardware baseline is unverified")
    required_frames = [("cold", 1), ("warmup", 10), ("steady", 30)]
    expected_pairs = [(kind, index) for kind, count in required_frames for index in range(count)]
    actual_pairs = [(r.get("frame_class"), r.get("frame_index")) for r in frames]
    if sorted(actual_pairs) != sorted(expected_pairs):
        errors.append(f"{run_name}: expected 1 cold, 10 warmup, 30 steady frames with exact indices")
    base_sig = _gpu_signature(meta)
    frame_sig = {key: meta.get(key) for key in GPU_FRAME_IDENTITY_KEYS}
    for i, r in enumerate(frames):
        if r.get("schema_version") != 1 or r.get("case") != "frame":
            errors.append(f"{run_name}: unsupported frame schema at index {i}")
        for key, value in frame_sig.items():
            if r.get(key) != value:
                errors.append(f"{run_name}: frame metadata drift for {key} at index {i}")
        _finite(r.get("wall_ms"), f"{run_name} wall_ms[{i}]", errors)
        for key in ("cpu_prepare_ms", "cpu_encode_ms", "cpu_submit_ms"):
            _finite(r.get(key), f"{run_name} {key}[{i}]", errors, nullable=True)
        for key in ("shape_instances", "mesh_instances", "line_instances", "upload_calls", "upload_bytes",
                    "buffer_reallocations", "attachment_allocations", "attachment_reallocations", "msaa_samples"):
            if not _is_int(r.get(key)) or r[key] < 0:
                errors.append(f"{run_name} {key}[{i}] must be a nonnegative integer")
        if not isinstance(r.get("software"), bool):
            errors.append(f"{run_name} software[{i}] must be a bool")
        for counter_group in ("main", "shadow"):
            counters = r.get(counter_group)
            if not isinstance(counters, dict) or any(
                    not _is_int(counters.get(key)) or counters[key] < 0
                    for key in ("passes", "draw_calls", "instances")):
                errors.append(f"{run_name} {counter_group}[{i}] counters are missing or invalid")
        expected_work = ((1000, 0, 0) if expected["renderer"] == "2d" else (0, 1000, 0))
        if (r.get("shape_instances"), r.get("mesh_instances"), r.get("line_instances")) != expected_work:
            errors.append(f"{run_name}: submitted instance-kind counters differ from workload at frame {i}")
        main = r.get("main")
        if isinstance(main, dict) and main.get("instances") != 1000:
            errors.append(f"{run_name}: main pass must submit 1000 instances at frame {i}")
        shadow = r.get("shadow")
        if expected["renderer"] == "3d" and isinstance(shadow, dict) and shadow.get("instances") != 1000:
            errors.append(f"{run_name}: shadow pass must submit 1000 instances at frame {i}")
        if r.get("wall_scope") != "render_call_return_not_gpu_completion":
            errors.append(f"{run_name}: wall time must be labelled render-call return, not GPU completion")
    if len(readbacks) != 1:
        errors.append(f"{run_name}: expected one readback record, got {len(readbacks)}")
    readback = readbacks[0] if readbacks else {}
    if readback.get("schema_version") != 1 or readback.get("suite") != "renderer" or readback.get("case") != "readback":
        errors.append(f"{run_name}: invalid readback record schema")
    for key in ("renderer", "preset", "scene_id", "resolution_px"):
        if readback.get(key) != meta.get(key):
            errors.append(f"{run_name}: readback {key} differs from metadata")
    _finite(readback.get("readback_ms"), f"{run_name} readback_ms", errors)
    if readback.get("timing_included_in_frame_samples") is not False:
        errors.append(f"{run_name}: readback timing must be excluded from frame samples")
    if readback.get("pixel_bytes") != 640 * 360 * 4:
        errors.append(f"{run_name}: unexpected readback byte count")
    if not isinstance(readback.get("fnv1a64"), str) or not re.fullmatch(r"[0-9a-f]{16}", readback["fnv1a64"]):
        errors.append(f"{run_name}: invalid readback checksum")
    groups = {kind: [r.get("wall_ms") for r in frames if r.get("frame_class") == kind]
              for kind, _ in required_frames}
    cpu_groups = {
        field: {kind: _aggregate([r.get(field) for r in frames if r.get("frame_class") == kind])
                for kind, _ in required_frames}
        for field in ("cpu_prepare_ms", "cpu_encode_ms", "cpu_submit_ms")
    }
    return {"signature": base_sig, "frame_wall_ms": groups,
            "frame_wall_ms_summary": {kind: _aggregate(values) for kind, values in groups.items()},
            "cpu_stage_ms_summary": cpu_groups,
            "cpu_frame_fields_are_not_gpu_completion": True,
            "readback_ms_excluded": readback.get("timing_included_in_frame_samples") is False}


def _validate_repetitions(run_results: list[dict[str, Any]], errors: list[str],
                          repetitions: int | None = None) -> dict[str, Any]:
    grouped: dict[str, list[dict[str, Any]]] = {}
    for item in run_results:
        grouped.setdefault(item["suite_name"], []).append(item)
    summaries: dict[str, Any] = {}
    expected_names = {"editor_measure", *GPU_EXPECTED.keys()}
    if repetitions is not None:
        for name in expected_names:
            actual_count = len(grouped.get(name, []))
            if actual_count != repetitions:
                errors.append(f"{name}: expected {repetitions} fixed repetitions, got {actual_count}")
    for name, runs in grouped.items():
        if repetitions is not None and len(runs) != repetitions:
            errors.append(f"{name}: expected {repetitions} fixed repetitions, got {len(runs)}")
        if any(r.get("exit_code") != 0 or r.get("timed_out") or r.get("launch_error") for r in runs):
            errors.append(f"{name}: one or more scheduled process runs failed or timed out")
        if name == "editor_measure":
            for i, r in enumerate(runs):
                if not r.get("validation", {}).get("valid"):
                    errors.extend(f"editor run {i + 1}: {e}" for e in r.get("validation", {}).get("errors", []))
            editor_summaries = [r.get("validation", {}).get("summary") for r in runs]
            identities = [s.get("identity") if isinstance(s, dict) else None for s in editor_summaries]
            if identities and any(identity != identities[0] for identity in identities[1:]):
                errors.append("editor scene/body-count/window/backend differ across repetitions")
            summaries[name] = editor_summaries
        else:
            signatures = [r.get("validation", {}).get("summary", {}).get("signature") for r in runs]
            if signatures and any(signature != signatures[0] for signature in signatures[1:]):
                errors.append(f"{name}: scene/resolution/adapter/software/backend/settings differ across repetitions")
            for i, r in enumerate(runs):
                if not r.get("validation", {}).get("valid"):
                    errors.extend(f"{name} run {i + 1}: {e}" for e in r.get("validation", {}).get("errors", []))
            summaries[name] = [r.get("validation", {}).get("summary") for r in runs]
    return summaries


def _compiled_binary(run_dir: Path, command: list[str]) -> dict[str, Any]:
    expected = command[command.index("--test") + 1]
    matches = []
    with (run_dir / "stdout.log").open("r", encoding="utf-8", errors="strict") as stream:
        for line in stream:
            try:
                item = json.loads(line)
            except (ValueError, RecursionError):
                continue
            if (isinstance(item, dict) and item.get("reason") == "compiler-artifact"
                    and item.get("target", {}).get("name") == expected
                    and item.get("profile", {}).get("test") is True
                    and item.get("executable")):
                matches.append(Path(item["executable"]).resolve())
    if len(matches) != 1 or not matches[0].is_file():
        raise BaselineError(f"expected exactly one existing Cargo test executable for {expected}")
    executable = matches[0]
    return {"target": expected, "path": str(executable),
            "sha256": _sha256_file(executable), "bytes": executable.stat().st_size}


def _write_manifest(output: Path, manifest: dict[str, Any]) -> None:
    _atomic_json(output / "manifest.json", manifest)


def collect(args: argparse.Namespace) -> int:
    repo = REPO.resolve()
    before = _source_identity(repo)
    _require_clean(before)
    toolchain = _toolchain()
    power = _power_metadata()
    output = _output_path(args.output, repo)
    output.mkdir(parents=True, exist_ok=False)
    manifest: dict[str, Any] = {
        "schema_version": 1, "status": "running", "started_at": utc_now(),
        "repetitions": args.repetitions, "gpu_mode": args.gpu_mode,
        "source_before": before, "source_after": None,
        "platform": {"system": platform.system(), "release": platform.release(),
                     "machine": platform.machine(), "python": platform.python_version(),
                     "cpu": _cpu_metadata()},
        "toolchain": toolchain, "power": power, "default_features": True,
        "output_directory": str(output), "planned_commands": _planned_commands(args.repetitions, args.gpu_mode, repo),
        "build_runs": [], "runs": [], "errors": [],
    }
    _write_manifest(output, manifest)
    aborted = False
    for build_index, (build_name, command) in enumerate(BUILD_RUNS):
        run_dir = output / build_name
        run_dir.mkdir()
        env = _command_environment(build_name, args.gpu_mode)
        _atomic_json(run_dir / "argv.json", {"argv": command, "cwd": str(repo)})
        outcome = _launch(command, env, repo, run_dir)
        outcome.update({"suite_name": build_name, "env_allowlist": _safe_environment(env)})
        if outcome["exit_code"] == 0 and not outcome["launch_error"]:
            try:
                outcome["binary_identity"] = _compiled_binary(run_dir, command)
            except (BaselineError, OSError, UnicodeError) as exc:
                outcome["launch_error"] = f"test executable provenance failed: {exc}"
        manifest["build_runs"].append(outcome)
        if outcome["exit_code"] != 0 or outcome["timed_out"] or outcome["launch_error"]:
            manifest["errors"].append({"run": build_name, "exit_code": outcome["exit_code"],
                                        "timed_out": outcome["timed_out"],
                                        "launch_error": outcome["launch_error"]})
        _write_manifest(output, manifest)
        if outcome.get("interrupted") or outcome.get("timed_out") or not outcome["child_cleanup_verified"]:
            aborted = True
            manifest["errors"].append({"infra_abort": build_name, "timed_out": outcome["timed_out"],
                                        "interrupted": outcome["interrupted"],
                                        "child_cleanup_verified": outcome["child_cleanup_verified"]})
            completed_ids = {item["suite_name"] for item in manifest["build_runs"]}
            manifest["unstarted"] = [item["id"] for item in manifest["planned_commands"]
                                        if item["id"] not in completed_ids]
            break
    suite_plan = [EDITOR_RUN, *GPU_RUNS]
    for repetition in range(1, args.repetitions + 1):
        if aborted:
            break
        for suite_index, (suite_name, command) in enumerate(suite_plan):
            run_id = f"rep-{repetition:02d}-{suite_name}"
            run_dir = output / run_id
            run_dir.mkdir()
            env = _command_environment(suite_name, args.gpu_mode)
            _atomic_json(run_dir / "argv.json", {"argv": command, "cwd": str(repo)})
            outcome = _launch(command, env, repo, run_dir)
            outcome.update({"suite_name": suite_name, "repetition": repetition,
                            "env_allowlist": _safe_environment(env)})
            records, parse_errors = _records(run_dir)
            validation_errors = list(parse_errors)
            if suite_name == "editor_measure":
                summary = _validate_editor(records, validation_errors)
                if any(r.get("suite") != "editor" for r in records):
                    validation_errors.append("editor process emitted a non-editor ORR_BASELINE record")
                if any(r.get("case") == "metadata" for r in records):
                    pass
            else:
                summary = _validate_gpu(suite_name, records, validation_errors, args.gpu_mode)
                if outcome.get("exit_code") == 0 and not records:
                    validation_errors.append("GPU test emitted no records (skip/missing output is not baseline data)")
            outcome["record_count"] = len(records)
            with (run_dir / "records.jsonl").open("w", encoding="utf-8", newline="\n") as record_file:
                for record in records:
                    record_file.write(json.dumps(record, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n")
            outcome["record_file"] = "records.jsonl"
            outcome["validation"] = {"valid": not validation_errors and outcome.get("exit_code") == 0
                                      and not outcome.get("timed_out") and not outcome.get("launch_error"),
                                      "errors": validation_errors, "summary": summary}
            manifest["runs"].append(outcome)
            if validation_errors or outcome["exit_code"] != 0 or outcome["timed_out"] or outcome["launch_error"]:
                manifest["errors"].append({"run": run_id, "errors": validation_errors,
                                            "exit_code": outcome["exit_code"], "timed_out": outcome["timed_out"],
                                            "launch_error": outcome["launch_error"]})
            _write_manifest(output, manifest)
            if outcome.get("interrupted") or outcome.get("timed_out") or not outcome["child_cleanup_verified"]:
                aborted = True
                manifest["errors"].append({"infra_abort": run_id, "timed_out": outcome["timed_out"],
                                            "interrupted": outcome["interrupted"],
                                            "child_cleanup_verified": outcome["child_cleanup_verified"]})
                executed_ids = {item["suite_name"] for item in manifest["build_runs"]}
                executed_ids.update(f"rep-{item['repetition']:02d}-{item['suite_name']}"
                                    for item in manifest["runs"])
                manifest["unstarted"] = [item["id"] for item in manifest["planned_commands"]
                                            if item["id"] not in executed_ids]
                break
    after = _source_identity(repo)
    manifest["source_after"] = after
    if after != before:
        manifest["errors"].append({"source_changed_during_collection": True,
                                   "before": before, "after": after})
    manifest["summary"] = _validate_repetitions(manifest["runs"], manifest["errors"], args.repetitions)
    manifest["finished_at"] = utc_now()
    manifest["status"] = "complete" if not manifest["errors"] and after == before else "incomplete"
    _write_manifest(output, manifest)
    print(json.dumps({"status": manifest["status"], "output": str(output),
                      "run_count": len(manifest["runs"]), "errors": len(manifest["errors"])}, indent=2))
    return 0 if manifest["status"] == "complete" else 1


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--output", help="new directory under an ignored repository path")
    result.add_argument("--repetitions", type=int, default=3)
    result.add_argument("--gpu-mode", choices=("default", "software"), default="default")
    result.add_argument("--self-test", action="store_true")
    return result


def main(argv: Iterable[str] | None = None) -> int:
    args = parser().parse_args(argv)
    if args.self_test:
        import unittest
        suite = unittest.defaultTestLoader.discover(str(Path(__file__).parent), pattern="test_editor_release_baseline.py")
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    if args.repetitions < 3 or args.repetitions > 10:
        parser().error("--repetitions must be between 3 and 10")
    if not args.output:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        args.output = f"target/editor-release-baseline/{stamp}-{os.getpid()}"
    try:
        return collect(args)
    except (BaselineError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"editor release baseline: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
