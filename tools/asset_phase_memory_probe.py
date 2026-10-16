#!/usr/bin/env python3
"""Bounded, device-free asset phase evidence, not a heap or RSS budget.

The fixed campaign is six cooker commands, two inspections, and one existing
audio boundary test. Preparation/builds are separate. No source/cache is added
to the runtime process. The runtime selector includes max/+1 and digest checks;
it does not successfully construct a maximum-bank mixer.

Use --self-test for pure/fake-process unit checks. --prepare creates a new
worktree target directory. --build runs the two pinned preparation commands.
--run consumes that immutable preparation once. No command kills a child:
watchdog/quota/error stops future launches, while the owned child is drained
and naturally reaped. A permanently live child remains pending and needs
coordination. Windows memory is PSAPI; Linux peak is wait4 child ru_maxrss.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock
from datetime import datetime, timezone

INPUT_CAP = 1 << 20
SOURCE_CAP = 1 << 16
DATA_CAP = 32 << 20
RAW_CAP = 64 << 20
METADATA_RESERVE = 1 << 20
WATCHDOG = 30.0
BUILD_WATCHDOG = 900.0  # Separate preparation bound; record explicitly.
SELECTOR = "audio::tests::decoded_budget_exact_max_and_one_frame_over_are_checked_before_conversion"
SIM_TYPE, VIEW_TYPE = 1, 2
SOURCE_FILES = (
    "tools/asset_phase_memory_probe.py", "Cargo.lock", "Cargo.toml",
    "crates/orr_asset/src/manifest.rs", "crates/orr_asset_cook/src/authoring.rs",
    "crates/orr_asset_cook/src/pipeline.rs", "crates/orr_asset_cook/src/main.rs",
    "crates/orr_asset_fixture/Cargo.toml", "crates/orr_asset_fixture/src/audio.rs",
    "crates/orr_asset_fixture/src/audio/tests.rs", "crates/orr_audio/src/lib.rs",
)


def require(ok, message):
    if not ok:
        raise ValueError(message)


def utc():
    return datetime.now(timezone.utc).isoformat()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def fingerprint(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return {"bytes": Path(path).stat().st_size, "sha256": h.hexdigest()}


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_new(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as f:
        f.write(data)


def no_links(path):
    """Reject existing symlink/reparse ancestors; not an OS sandbox."""
    for p in (Path(path), *Path(path).parents):
        if p.exists() or p.is_symlink():
            s = p.lstat()
            require(not p.is_symlink() and not (getattr(s, "st_file_attributes", 0) & 0x400),
                    f"symlink/reparse path: {p}")


def inventory(root):
    root = Path(root)
    no_links(root)
    result = {}
    if root.exists():
        for p in sorted(root.rglob("*")):
            no_links(p)
            if p.is_file():
                result[p.relative_to(root).as_posix()] = fingerprint(p)
    return result


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE).decode().strip()


def source_identity(root, head):
    require(re.fullmatch(r"[0-9a-f]{40}", head) is not None, "expected-head must be full immutable SHA")
    require(git(root, "rev-parse", "HEAD") == head, "source HEAD changed")
    require(git(root, "status", "--porcelain", "--untracked-files=all") == "", "source must be clean")
    return {"head": head, "tree": git(root, "rev-parse", "HEAD^{tree}"),
            "files": {rel: fingerprint(Path(root) / rel) for rel in SOURCE_FILES}}


def trunc_div(n, d):
    return (abs(n) // d) * (-1 if n < 0 else 1)


def pcm(peak, frames=48000, period=4):
    out = bytearray(struct.pack("<II", 48000, frames))
    for i in range(frames):
        phase = i % period
        triangle = 4 * phase - period if phase < period // 2 else 3 * period - 4 * phase
        out.extend(struct.pack("<h", trunc_div(peak * triangle * (frames - i), period * frames)))
    return bytes(out)


def cache_key(type_id, source):
    return sha(b"orr.asset-cook.cache\0" + struct.pack("<IIIII", 1, 1, type_id, 1, 0)
               + hashlib.sha256(source).digest())


def fixture_data(variant):
    require(variant in ("max_index", "max_source"), "unknown fixture")
    entries, sources, payloads = [], {}, {}
    for i in range(80):
        path = f"sources/asset_{i:03}.json"
        # Quarter increments 0.25..16 exactly map to 64 distinct Q48.16 values.
        if i < 64:
            quarters = i + 1
            decimal = f"{quarters // 4}.{(quarters % 4) * 25:02}"
            source = json_bytes({"speed_per_tick": decimal})
            payload = struct.pack("<q", quarters * 16384)
            asset_type, type_id = "sim.motion_profile", SIM_TYPE
        else:
            peak = 8192 - (i - 64)
            source = json_bytes({"generator": "triangle_decay_v1", "sample_rate": 48000,
                                 "frames": 48000, "period_frames": 4, "peak_pcm16": peak})
            payload = pcm(peak)
            asset_type, type_id = "view.impact_pcm16", VIEW_TYPE
        entries.append({"id": f"a_{i + 1:016x}", "type": asset_type,
                        "schema_version": 1, "source": path})
        sources[path] = source
        payloads[sha(payload)] = {"bytes": len(payload), "type_id": type_id,
                                  "id": i + 1, "sha256": sha(payload)}
    entries.extend({"id": f"a_{i:016x}", "tombstone": True} for i in range(81, 1025))
    index = json_bytes({"format": "orr.asset-index/1", "entries": entries})
    remaining = INPUT_CAP - len(index) - sum(map(len, sources.values()))
    require(remaining >= 0, "base fixture exceeds aggregate limit")
    if variant == "max_index":
        index += b" " * remaining
    else:
        # Last live record is at max source size after earlier outputs accumulate.
        order = ["sources/asset_079.json"] + list(sources)[:-1]
        for path in order:
            added = min(remaining, SOURCE_CAP - len(sources[path]))
            sources[path] += b" " * added
            remaining -= added
        require(remaining == 0, "source padding capacity too small")
    require(len(entries) == 1024 and len(sources) == len(payloads) == 80, "deduplicated fixture")
    require(len(index) + sum(map(len, sources.values())) == INPUT_CAP, "aggregate must be exactly1MiB")
    require(all(len(v) <= SOURCE_CAP for v in sources.values()), "source cap exceeded")
    require(json.loads(index)["entries"] == entries, "index padding changed semantic records")
    if variant == "max_source":
        require(len(sources["sources/asset_079.json"]) == SOURCE_CAP, "last source not64KiB")
    keys = [cache_key(SIM_TYPE if i < 64 else VIEW_TYPE, v) for i, v in enumerate(sources.values())]
    require(len(set(keys)) == 80, "cache key deduplication")
    return index, sources, payloads, keys


def prepare(root, head, output):
    no_links(root)
    root, output = Path(root).resolve(), Path(output).absolute()
    no_links(output)
    require(output.resolve().is_relative_to(root / "target"), "output must be inside own worktree target")
    require(not output.exists(), "fresh output only; never reuse a campaign")
    before = source_identity(root, head)
    output.mkdir(parents=True)
    fixtures = {}
    for variant in ("max_index", "max_source"):
        index, sources, payloads, keys = fixture_data(variant)
        case = output / "cases" / variant
        write_new(case / "inputs" / "index.json", index)
        for path, data in sources.items():
            write_new(case / "inputs" / path, data)
        fixtures[variant] = {"input": inventory(case / "inputs"), "expected_objects": payloads,
                             "cache_keys": keys, "index_records": 1024, "sim": 64, "view": 16,
                             "aggregate_input_bytes": INPUT_CAP}
    # Two inputs/caches, four final bundles and one staging bundle. A bounded
    # text allowance accompanies each bundle; build/raw/FS overhead is separate.
    forecast = 2 * INPUT_CAP + 2 * 1543680 + 5 * (1536640 + 3600 + 912 + (1 << 20))
    require(forecast <= DATA_CAP, "planned data with staging exceeds cap")
    plan = {"format": "orr.asset-phase-probe/1", "prepared_at": utc(), "source_root": str(root),
            "source": before, "fixtures": fixtures, "campaign_child_limit": 9,
            "watchdog_s": WATCHDOG, "raw_limit_bytes": RAW_CAP, "data_limit_bytes": DATA_CAP,
            "python": sys.version, "platform": sys.platform,
            "runtime_selector": SELECTOR, "measurements": "whole-child observations; internal/private phases unknown"}
    plan["data_admission_with_one_staging_bundle_bytes"] = forecast
    plan["build_watchdog_s"] = BUILD_WATCHDOG
    require(source_identity(root, head) == before, "source changed during fixture preparation")
    write_new(output / "plan.json", json_bytes(plan))
    return plan


class RawBudget:
    def __init__(self, cap=RAW_CAP - METADATA_RESERVE):
        self.remaining = cap
        self.lock = threading.Lock()
        self.exceeded = threading.Event()
        self.metadata_remaining = METADATA_RESERVE

    def take(self, wanted):
        with self.lock:
            n = min(wanted, self.remaining)
            self.remaining -= n
            if n != wanted:
                self.exceeded.set()
            return n

    def metadata(self, path, value):
        # Prefix text is transient validation input, not duplicated raw data.
        data = json_bytes(without_prefix(value))
        with self.lock:
            require(len(data) <= self.metadata_remaining, "reserved metadata quota exceeded")
            self.metadata_remaining -= len(data)
        write_new(path, data)


def without_prefix(value):
    if isinstance(value, dict):
        return {k: without_prefix(v) for k, v in value.items() if k not in ("stdout_text", "stderr_text")}
    if isinstance(value, list):
        return [without_prefix(v) for v in value]
    return value


def raw_usage(output):
    output = Path(output)
    files = list(output.glob("*.json"))
    for name in ("evidence", "build-evidence"):
        folder = output / name
        if folder.exists():
            files += [folder / rel for rel in inventory(folder)]
    return sum(p.stat().st_size for p in files)


def new_budget(output):
    used = raw_usage(output)
    require(used <= RAW_CAP - METADATA_RESERVE, "existing raw evidence exhausts campaign allowance")
    return RawBudget(RAW_CAP - METADATA_RESERVE - used)


class WindowsMemory:
    """Duplicate the handle held by Popen, never reopen a recycled numeric PID."""
    def __init__(self, proc):
        from ctypes import wintypes as w
        self.w = w
        self.k = ctypes.WinDLL("kernel32", use_last_error=True)
        self.p = ctypes.WinDLL("psapi", use_last_error=True)
        self.k.GetCurrentProcess.restype = w.HANDLE
        self.k.DuplicateHandle.argtypes = [w.HANDLE, w.HANDLE, w.HANDLE, ctypes.POINTER(w.HANDLE), w.DWORD, w.BOOL, w.DWORD]
        self.k.DuplicateHandle.restype = w.BOOL
        self.k.GetProcessId.argtypes, self.k.GetProcessId.restype = [w.HANDLE], w.DWORD
        self.k.CloseHandle.argtypes, self.k.CloseHandle.restype = [w.HANDLE], w.BOOL
        self.handle = w.HANDLE()
        current = self.k.GetCurrentProcess()
        if not self.k.DuplicateHandle(current, w.HANDLE(int(proc._handle)), current,
                                      ctypes.byref(self.handle), 0, False, 2):
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            require(self.k.GetProcessId(self.handle) == proc.pid, "held-handle PID mismatch")
            self.k.GetProcessTimes.argtypes = [w.HANDLE] + [ctypes.POINTER(w.FILETIME)] * 4
            self.k.GetProcessTimes.restype = w.BOOL
            times = [w.FILETIME() for _ in range(4)]
            if not self.k.GetProcessTimes(self.handle, *(ctypes.byref(t) for t in times)):
                raise ctypes.WinError(ctypes.get_last_error())
            size, buf = w.DWORD(32768), ctypes.create_unicode_buffer(32768)
            self.k.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
            self.k.QueryFullProcessImageNameW.restype = w.BOOL
            if not self.k.QueryFullProcessImageNameW(self.handle, 0, buf, ctypes.byref(size)):
                raise ctypes.WinError(ctypes.get_last_error())
            self.identity = {"pid": proc.pid, "creation_filetime": times[0].dwLowDateTime | times[0].dwHighDateTime << 32,
                             "image": buf.value, "attribution": "duplicate_of_Popen_held_handle"}
            class Counters(ctypes.Structure):
                _fields_ = [("cb", w.DWORD), ("PageFaultCount", w.DWORD)] + [(n, ctypes.c_size_t) for n in
                    ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
                     "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]
            self.Counters = Counters
            self.p.GetProcessMemoryInfo.argtypes = [w.HANDLE, ctypes.POINTER(Counters), w.DWORD]
            self.p.GetProcessMemoryInfo.restype = w.BOOL
        except BaseException:
            self.close()
            raise

    def sample(self):
        c = self.Counters()
        c.cb = ctypes.sizeof(c)
        if not self.p.GetProcessMemoryInfo(self.handle, ctypes.byref(c), c.cb):
            # A naturally exited process may no longer provide counters.
            return {"working_set_bytes": None, "peak_working_set_bytes": None, "private_usage_bytes": None,
                    "win_error": ctypes.get_last_error()}
        return {"working_set_bytes": c.WorkingSetSize, "peak_working_set_bytes": c.PeakWorkingSetSize,
                "private_usage_bytes": c.PrivateUsage}

    def close(self):
        if self.handle:
            if not self.k.CloseHandle(self.handle):
                raise ctypes.WinError(ctypes.get_last_error())
            self.handle = None


def child_capture(argv, cwd, folder, budget, watchdog=WATCHDOG, factory=subprocess.Popen, memory_factory=None):
    """One outer lifetime guard includes spawn/setup/read/observe/reap/close."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=False)
    files = {}
    try:
        for name in ("stdout", "stderr", "memory"):
            files[name] = (folder / (name + ".raw")).open("xb")
    except BaseException:
        for f in files.values():
            f.close()
        raise
    counters = {name: {"observed": 0, "saved": 0, "discarded": 0, "observed_sha": hashlib.sha256(),
                       "saved_sha": hashlib.sha256(), "prefix": bytearray()} for name in files}
    errors, error_lock = [], threading.Lock()
    gate = threading.Event()
    owned_exit = threading.Event()
    stream_eof = {"stdout": False, "stderr": False}
    proc = observer = None
    streams = {}
    start = time.monotonic()
    record = {"argv": list(argv), "cwd": str(cwd), "started_at": utc(), "pid": None,
              "watchdog_s": watchdog, "watchdog_triggered": False, "memory_supported": None,
              "memory_identity": None, "ru_maxrss_kib": None, "sampling_target_ms": 10,
              "sampling_count": 0, "interval_min_ms": None, "interval_max_ms": None,
              "max_observed_working_set_bytes": None, "max_reported_peak_working_set_bytes": None,
              "max_observed_private_usage_bytes": None}
    record["memory_handle_closed"] = None

    def error(value):
        with error_lock:
            if len(errors) < 32:
                errors.append(str(value)[:500])

    def save(name, data):
        c = counters[name]
        c["observed"] += len(data)
        c["observed_sha"].update(data)
        n = budget.take(len(data))
        if n:
            files[name].write(data[:n])
            c["saved_sha"].update(data[:n])
            c["saved"] += n
        c["discarded"] += len(data) - n
        if name != "memory" and len(c["prefix"]) < 1048576:
            c["prefix"].extend(data[:1048576 - len(c["prefix"])])

    def reader(name):
        gate.wait()
        if proc is None:
            return
        drain_only, read_error_reported = False, False
        while True:
            try:
                chunk = streams[name].read(8192)
                if not chunk:
                    stream_eof[name] = True
                    break
            except BaseException as exc:
                if not read_error_reported:
                    error(f"{name} pipe-read pending; EOF not proven: {exc!r}")
                    print(json.dumps({"pending_owned_pid": proc.pid, "reason": "pipe-read failure; drain pending"}), flush=True)
                    read_error_reported = True
                if owned_exit.is_set():
                    error(f"{name} drain incomplete after proven owned exit")
                    break
                # No replacement/cleanup claim if an OS pipe never recovers.
                time.sleep(0.01)
                continue
            if drain_only:
                c = counters[name]
                c["observed"] += len(chunk)
                c["observed_sha"].update(chunk)
                c["discarded"] += len(chunk)
            else:
                try:
                    save(name, chunk)
                except BaseException as exc:
                    error(f"{name} evidence-write failed; continuing drain/discard: {exc!r}")
                    c = counters[name]
                    c["discarded"] = c["observed"] - c["saved"]
                    drain_only = True

    threads = [threading.Thread(target=reader, args=(name,), daemon=False) for name in ("stdout", "stderr")]
    returned = None
    linux_reap = sys.platform.startswith("linux") and factory is subprocess.Popen
    try:
        # Readers/buffers/files exist before spawn. No postspawn exception escapes
        # without the same finally guard draining and actually reaping ownership.
        for t in threads:
            t.start()  # Threads block on gate; startup failure occurs before Popen.
        proc = factory(list(argv), cwd=str(cwd), stdin=subprocess.DEVNULL,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False)
        record["pid"] = proc.pid
        streams = {"stdout": proc.stdout, "stderr": proc.stderr}
        gate.set()
        if memory_factory is not None:
            observer = memory_factory(proc)
        elif sys.platform == "win32":
            observer = WindowsMemory(proc)
        record["memory_supported"] = observer is not None or linux_reap
        if observer:
            record["memory_identity"] = observer.identity
            if sys.platform == "win32" and memory_factory is None:
                expected = Path(shutil.which(argv[0]) or argv[0]).resolve()
                require(os.path.normcase(str(expected)) == os.path.normcase(observer.identity["image"]),
                        "held process image differs from command executable")
        last = None
        while True:
            if linux_reap:
                pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
                if pid:
                    returned = os.waitstatus_to_exitcode(status)
                    proc.returncode = returned  # wait4 is the sole reaper.
                    record["ru_maxrss_kib"] = usage.ru_maxrss
                    break
            else:
                returned = proc.poll()
                if returned is not None:
                    break
            now = time.monotonic()
            if now - start > watchdog and not record["watchdog_triggered"]:
                record["watchdog_triggered"] = True
                error("observation watchdog exceeded; waiting for owned natural exit")
                print(json.dumps({"pending_owned_pid": proc.pid, "reason": "watchdog", "folder": str(folder)}), flush=True)
            if observer:
                sample = {"at_utc": utc(), "elapsed_s": now - start, **observer.sample()}
                save("memory", json_bytes(sample))
                for key, source in (("max_observed_working_set_bytes", "working_set_bytes"),
                                    ("max_reported_peak_working_set_bytes", "peak_working_set_bytes"),
                                    ("max_observed_private_usage_bytes", "private_usage_bytes")):
                    value = sample.get(source)
                    if value is not None:
                        record[key] = max(record[key] or 0, value)
                record["sampling_count"] += 1
                if last is not None:
                    delta = (now - last) * 1000
                    record["interval_min_ms"] = min(record["interval_min_ms"] or delta, delta)
                    record["interval_max_ms"] = max(record["interval_max_ms"] or delta, delta)
                last = now
            time.sleep(0.01)
    except BaseException as exc:
        error(f"supervisor: {exc!r}")
        if proc is not None and returned is None:
            print(json.dumps({"pending_owned_pid": proc.pid, "reason": "supervisor error; natural exit pending",
                              "folder": str(folder)}), flush=True)
    finally:
        # Recover pipe references if interruption occurred after Popen returned
        # but before normal setup published them to the gated readers.
        if proc is not None and not streams:
            streams = {"stdout": proc.stdout, "stderr": proc.stderr}
        gate.set()
        if proc is not None:
            reaper_error_reported = False
            # Natural exit is required even on setup/interrupt/reader failures.
            while returned is None:
                try:
                    if linux_reap:
                        _, status, usage = os.wait4(proc.pid, 0)
                        returned = os.waitstatus_to_exitcode(status)
                        proc.returncode = returned
                        record["ru_maxrss_kib"] = usage.ru_maxrss
                    else:
                        returned = proc.wait()
                except BaseException as exc:
                    error(f"reaper pending; actual exit not proven: {exc!r}")
                    if not reaper_error_reported:
                        print(json.dumps({"pending_owned_pid": proc.pid, "reason": "reaper error; exit pending"}), flush=True)
                        reaper_error_reported = True
                    time.sleep(0.01)
            owned_exit.set()
        for t in threads:
            if t.ident is not None:
                while t.is_alive():
                    try:
                        t.join(0.1)
                    except BaseException as exc:
                        error(f"reader join pending: {exc!r}")
        for stream in streams.values():
            try:
                stream.close()
            except BaseException as exc:
                error(f"pipe close: {exc!r}")
        if observer:
            try:
                observer.close()
                record["memory_handle_closed"] = True
            except BaseException as exc:
                record["memory_handle_closed"] = False
                error(f"observer close: {exc!r}")
        for f in files.values():
            try:
                f.close()
            except BaseException as exc:
                error(f"evidence close: {exc!r}")
    if budget.exceeded.is_set():
        error("managed raw quota exceeded; streams drained with saved-prefix/discard accounting")
    # A failed buffered write/close can persist only part of a chunk. Derive
    # retained bytes/hash from the closed file, rather than claiming intent.
    for name, c in counters.items():
        try:
            actual = fingerprint(folder / (name + ".raw"))
            require(actual["bytes"] <= c["observed"], "retained stream exceeds observed bytes")
            c["saved"] = actual["bytes"]
            c["discarded"] = c["observed"] - actual["bytes"]
            c["saved_sha"] = actual["sha256"]
        except BaseException as exc:
            error(f"retained stream verification: {exc!r}")
    record.update({"actual_exit": returned, "ended_at": utc(), "command_wall_s": time.monotonic() - start,
                   "errors": errors, "status": "passed" if returned == 0 and not errors else "failed",
                   "drain_complete": all(stream_eof.values()) if proc is not None else None,
                   "cleanup": ("actual_exit_drained_reaped" if all(stream_eof.values()) else "actual_exit_reaped_streams_incomplete") if proc is not None else "not_spawned",
                   "streams": {n: {k: (v.hexdigest() if k.endswith("sha") and not isinstance(v, str) else v) for k, v in c.items() if k != "prefix"}
                               for n, c in counters.items()}})
    record["stdout_text"] = counters["stdout"]["prefix"].decode("utf-8", "replace")
    record["stderr_text"] = counters["stderr"]["prefix"].decode("utf-8", "replace")
    try:
        record["process_metadata_persisted"] = True
        budget.metadata(folder / "process.json", record)
    except BaseException as exc:
        error(f"process metadata not persisted: {exc!r}")
        record["status"] = "failed"
        record["process_metadata_persisted"] = False
        print(json.dumps({"failed_process_metadata": str(folder), "pid": record["pid"],
                          "actual_exit": returned, "reason": str(exc)[:300]}), flush=True)
    return record


def load_plan(output):
    no_links(output)
    output = Path(output).resolve()
    no_links(output)
    plan = json.loads((output / "plan.json").read_bytes())
    require(plan["format"] == "orr.asset-phase-probe/1", "bad plan format")
    root, head = Path(plan["source_root"]), plan["source"]["head"]
    no_links(root)
    require(set(plan["fixtures"]) == {"max_index", "max_source"}, "fixed two fixtures required")
    require(plan["campaign_child_limit"] == 9 and plan["watchdog_s"] == WATCHDOG
            and plan["raw_limit_bytes"] == RAW_CAP and plan["data_limit_bytes"] == DATA_CAP,
            "fixed campaign limits changed")
    require(output.is_relative_to(root / "target"), "plan escaped own target")
    require(source_identity(root, head) == plan["source"], "source identity drift")
    for variant, f in plan["fixtures"].items():
        require(inventory(output / "cases" / variant / "inputs") == f["input"], "fixture changed")
        index, sources, objects, keys = fixture_data(variant)
        expected = {"index.json": {"bytes": len(index), "sha256": sha(index)},
                    **{p: {"bytes": len(v), "sha256": sha(v)} for p, v in sources.items()}}
        require(f["input"] == expected and f["expected_objects"] == objects and f["cache_keys"] == keys,
                "fixture does not match fixed max-input generator")
    return output, plan, root, head


def managed_data(output):
    inv = inventory(Path(output) / "cases")
    count = sum(v["bytes"] for v in inv.values())
    require(count <= DATA_CAP, "fixture/cache/output logical byte cap exceeded")
    return {"logical_bytes": count, "limit": DATA_CAP, "files": inv,
            "excludes": "build/binaries, raw evidence, filesystem metadata; no hard filesystem guarantee"}


def assert_pass(record):
    require(record["status"] == "passed" and record["actual_exit"] == 0, "child did not pass: " + str(record["errors"]))
    for c in record["streams"].values():
        require(c["observed"] == c["saved"] + c["discarded"] and c["discarded"] == 0, "stream incomplete")


def final_check(report, key, check):
    """Retain the first failure and write evidence even when final checks fail."""
    try:
        report[key] = check()
    except BaseException as exc:
        report.setdefault("finalization_errors", []).append(f"{key}: {exc!r}")
        if report.get("status") != "failed_stop":
            report.update(status="failed_stop", failure=f"{key}: {exc!r}")


def cargo_artifact(path, target_name):
    artifacts = []
    with Path(path).open("rb") as stream:
        while True:
            line = stream.readline(524289)
            if not line:
                break
            require(len(line) <= 524288, "Cargo JSON line exceeds bounded parser input")
            if line.startswith(b"{"):
                entry = json.loads(line)
                if (entry.get("reason") == "compiler-artifact" and entry.get("executable")
                        and entry.get("target", {}).get("name") == target_name):
                    artifacts.append(entry)
                    require(len(artifacts) <= 1, "ambiguous executable artifacts")
    require(len(artifacts) == 1, "exact build executable missing")
    return artifacts[0]


def build(output):
    output, plan, root, head = load_plan(output)
    require(not (output / "build-report.json").exists(), "preparation build may only run once")
    write_new(output / "build-started.json", json_bytes({"at": utc(), "source": plan["source"]}))
    target = output / "build"
    commands = [
        ["cargo", "build", "--release", "--locked", "--offline", "-p", "orr_asset_cook", "--target-dir", str(target), "--message-format=json"],
        ["cargo", "test", "--release", "--locked", "--offline", "-p", "orr_asset_fixture", "--features", "audio", "--lib", "--no-run", "--target-dir", str(target), "--message-format=json"],
    ]
    report = {"kind": "preparation_not_campaign_measurement", "source": plan["source"], "records": [], "binaries": {}}
    budget = new_budget(output)
    try:
        for i, cmd in enumerate(commands):
            rec = child_capture(cmd, root, output / "build-evidence" / str(i), budget, BUILD_WATCHDOG)
            report["records"].append(rec)
            assert_pass(rec)
            artifact = cargo_artifact(output / "build-evidence" / str(i) / "stdout.raw",
                                      "orr_asset_cook" if i == 0 else "orr_asset_fixture")
            no_links(artifact["executable"])
            exe = Path(artifact["executable"]).resolve()
            require(exe.is_relative_to(target.resolve()), "build executable escaped target")
            report["binaries"]["cook" if i == 0 else "runtime"] = {"path": str(exe), **fingerprint(exe)}
            require(source_identity(root, head) == plan["source"], "source changed during build")
        report["status"] = "completed_two_builds"
    except BaseException as exc:
        report.update(status="failed_stop", failure=repr(exc))
    final_check(report, "source_after", lambda: source_identity(root, head))
    budget.metadata(output / "build-report.json", report)
    require(raw_usage(output) <= RAW_CAP, "total preparation/raw cap exceeded")
    return report


def validate_bundle(case, fixture, output_name):
    folder = Path(case) / output_name
    inv = inventory(folder)
    require(set(inv) == {"sim.manifest.bin", "view.manifest.bin", "generated_sim.rs", "inspect.json", "report.json"}
            | {f"objects/{d}.bin" for d in fixture["expected_objects"]}, "bundle file inventory mismatch")
    for d, expected in fixture["expected_objects"].items():
        require(inv[f"objects/{d}.bin"] == {"bytes": expected["bytes"], "sha256": d}, "cooked payload mismatch")
    inspection = json.loads((folder / "inspect.json").read_bytes())
    require(set(inspection) == {"format", "sim_manifest_sha256", "view_manifest_sha256", "assets"}, "inspect v1 keys mismatch")
    require(inspection["format"] == "orr.asset-inspect/1" and len(inspection["assets"]) == 80, "inspect shape")
    require({v["payload_sha256"] for v in inspection["assets"]} == set(fixture["expected_objects"]), "inspection hashes")
    require({v["id"] for v in inspection["assets"]} == {f"a_{i:016x}" for i in range(1, 81)}, "inspection IDs")
    ordered = sorted(fixture["expected_objects"].values(), key=lambda v: v["id"])
    expected_inspect = [{"id": f"a_{v['id']:016x}", "type": "sim.motion_profile" if v["type_id"] == SIM_TYPE else "view.impact_pcm16",
                         "schema_version": 1, "payload_len": v["bytes"], "payload_sha256": v["sha256"]} for v in ordered]
    require(inspection["assets"] == expected_inspect, "inspection typed entries/order mismatch")
    for domain, count, domain_id in (("sim", 64, SIM_TYPE), ("view", 16, VIEW_TYPE)):
        info = inv[f"{domain}.manifest.bin"]
        require(info["bytes"] == 16 + count * 56, "manifest count/length")
        require(info["sha256"] == inspection[f"{domain}_manifest_sha256"], "manifest digest binding")
        expected_manifest = struct.pack("<4sIII", b"ORAM", 1, domain_id, count)
        for v in ordered:
            if v["type_id"] == domain_id:
                expected_manifest += struct.pack("<QIIQ", v["id"], v["type_id"], 1, v["bytes"]) + bytes.fromhex(v["sha256"])
        require((folder / f"{domain}.manifest.bin").read_bytes() == expected_manifest,
                "manifest header/typed identity/payload mismatch")
    expected_sources = [{"id": f"a_{i + 1:016x}", "source": f"sources/asset_{i:03}.json",
                         "source_sha256": fixture["input"][f"sources/asset_{i:03}.json"]["sha256"],
                         "cache_key": fixture["cache_keys"][i]} for i in range(80)]
    require(json.loads((folder / "report.json").read_bytes()) ==
            {"format": "orr.asset-cook-report/1", "cooker_format_version": 1,
             "importer_version": 1, "sources": expected_sources}, "provenance/source/cache attribution mismatch")
    require(sum(v["bytes"] for k, v in inv.items() if not k.startswith("objects/")) <= 1 << 20,
            "manifest/generated/JSON allowance exceeded")
    return inv


def validate_cache(case, fixture):
    folder = Path(case) / "cache"
    inv = inventory(folder)
    require(set(inv) == {f"{k}.bin" for k in fixture["cache_keys"]}, "touched cache key inventory")
    ordered = sorted(fixture["expected_objects"].values(), key=lambda v: v["id"])
    for key, v in zip(fixture["cache_keys"], ordered):
        data = (folder / (key + ".bin")).read_bytes()
        expected = struct.pack("<4sIII", b"ORAC", 1, v["type_id"], 1) + bytes.fromhex(key)
        expected += struct.pack("<Q", v["bytes"]) + bytes.fromhex(v["sha256"])
        require(len(data) == 88 + v["bytes"] and data[:88] == expected and sha(data[88:]) == v["sha256"],
                "cache header/body/type/source key mismatch")
    require(sum(v["bytes"] for v in inv.values()) == 1543680, "cache payload/header accounting")
    return inv


def validate_cook(record, fixture, hits):
    assert_pass(record)
    text = record["stdout_text"]
    require(re.search(rf"(?m)^assets 80 cache_hits {hits}$", text) is not None, "cook count/cache_hits mismatch")
    for domain in ("sim", "view"):
        require(re.search(rf"(?m)^{domain} [0-9a-f]{{64}}$", text) is not None, "missing manifest hash")
    lines = text.strip().splitlines()
    require(len(lines) == 3 and lines[0].startswith("sim ") and lines[1].startswith("view "), "cook stdout shape changed")
    return {domain: lines[i].split()[1] for i, domain in enumerate(("sim", "view"))}


def validate_runtime(record):
    assert_pass(record)
    text = record["stdout_text"]
    require(text.splitlines().count(f"test {SELECTOR} ... ok") == 1,
            "exact runtime test name was not reported once as ok")
    require(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", text) is not None,
            "exact runtime selector not executed once")


def campaign(output, capture=child_capture):
    output, plan, root, head = load_plan(output)
    require(not (output / "campaign-started.json").exists(), "campaign cannot be reused/rerun")
    builds = json.loads((output / "build-report.json").read_bytes())
    require(builds["status"] == "completed_two_builds" and len(builds["records"]) == 2, "prebuild incomplete")
    for rec in builds["records"]:
        assert_pass(rec)
    require(builds["source"] == plan["source"], "build/source relation changed")
    for b in builds["binaries"].values():
        require(fingerprint(b["path"]) == {k: b[k] for k in ("bytes", "sha256")}, "binary drift")
    cook = builds["binaries"]["cook"]["path"]
    run = builds["binaries"]["runtime"]["path"]
    write_new(output / "campaign-started.json", json_bytes({"at": utc(), "source": plan["source"], "binary": builds["binaries"]}))
    report = {"kind": "whole-child_phase_observation", "source_before": plan["source"], "binary_before": builds["binaries"],
              "planned_children": 9, "attempted_children": 0, "records": [], "validations": [],
              "unknown": ["internal-stage heap/peak", "simultaneous source/cache/cooked/decoded process", "max-bank mixer",
                          "hard preemption", "RSS ceiling", "p95", "performance ratio", "future lane isolation"]}
    budget = new_budget(output)
    semantic = []
    try:
        for variant, fixture in plan["fixtures"].items():
            case = output / "cases" / variant
            require(not (case / "cache").exists(), "cold cache not fresh")
            cold = None
            for phase in ("cold", "warm", "check", "inspect"):
                managed_data(output)
                require(source_identity(root, head) == plan["source"], "source drift before child")
                for other, fixed in plan["fixtures"].items():
                    require(inventory(output / "cases" / other / "inputs") == fixed["input"], "fixture drift before child")
                for b in builds["binaries"].values():
                    require(fingerprint(b["path"])["sha256"] == b["sha256"], "binary drift before child")
                if phase == "inspect":
                    argv = [cook, "inspect", "--bundle", str(case / "cold")]
                else:
                    argv = [cook, "cook", "--index", str(case / "inputs" / "index.json"),
                            "--out", str(case / ("warm" if phase == "warm" else "cold"))]
                    argv += ["--check"] if phase == "check" else ["--cache", str(case / "cache")]
                report["attempted_children"] += 1
                rec = capture(argv, root, output / "evidence" / f"{len(report['records']):02}-{variant}-{phase}", budget)
                report["records"].append(rec)
                assert_pass(rec)
                require(inventory(case / "inputs") == fixture["input"], "fixture drift after child")
                cache = validate_cache(case, fixture)
                if phase == "inspect":
                    require(json.loads(rec["stdout_text"]) == json.loads((case / "cold" / "inspect.json").read_bytes()), "inspect CLI mismatch")
                else:
                    printed_hashes = validate_cook(rec, fixture, 80 if phase == "warm" else 0)
                    inv = validate_bundle(case, fixture, "warm" if phase == "warm" else "cold")
                    require(all(inv[f"{domain}.manifest.bin"]["sha256"] == value for domain, value in printed_hashes.items()),
                            "CLI manifest hashes differ from actual output")
                    if phase == "cold":
                        cold = inv
                    else:
                        require(inv == cold, "same-fixture cold/warm/check changed bytes")
                report["validations"].append({"variant": variant, "phase": phase, "cache_files": len(cache),
                                               "data_logical_bytes": managed_data(output)["logical_bytes"]})
            semantic.append({k: v for k, v in cold.items() if k != "report.json"})
        require(semantic[0] == semantic[1], "padding variants changed semantic cooked artifacts")
        require(source_identity(root, head) == plan["source"], "source drift before runtime child")
        for variant, fixed in plan["fixtures"].items():
            require(inventory(output / "cases" / variant / "inputs") == fixed["input"], "fixture drift before runtime child")
        for b in builds["binaries"].values():
            require(fingerprint(b["path"]) == {k: b[k] for k in ("bytes", "sha256")}, "binary drift before runtime child")
        report["attempted_children"] += 1
        rec = capture([run, SELECTOR, "--exact", "--nocapture", "--test-threads=1"], root,
                      output / "evidence" / "08-runtime-boundary", budget)
        report["records"].append(rec)
        validate_runtime(rec)
        require(source_identity(root, head) == plan["source"], "source drift after campaign")
        for b in builds["binaries"].values():
            require(fingerprint(b["path"]) == {k: b[k] for k in ("bytes", "sha256")}, "binary drift after campaign")
        report["status"] = "completed_all_nine"
    except BaseException as exc:
        report.update(status="failed_stop", failure=repr(exc))
    report["ended_at"] = utc()
    final_check(report, "data_after", lambda: managed_data(output))
    final_check(report, "source_after", lambda: source_identity(root, head))
    budget.metadata(output / "report.json", report)
    require(raw_usage(output) <= RAW_CAP, "whole preparation/campaign raw cap exceeded")
    return report


class SelfTests(unittest.TestCase):
    def test_runtime_exact_name_and_summary(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored; 27 filtered out; finished in 0.01s\n"
        record = {"status": "passed", "actual_exit": 0, "errors": [], "streams": {},
                  "stdout_text": f"test {SELECTOR} ... ok\n" + summary}
        validate_runtime(record)
        for text in (summary, "test wrong::selector ... ok\n" + summary,
                     f"test {SELECTOR} ... ok\ntest {SELECTOR} ... ok\n" + summary,
                     f"test {SELECTOR} ... ok\n" + summary.replace("1 passed", "0 passed")):
            record["stdout_text"] = text
            with self.assertRaises(ValueError):
                validate_runtime(record)

    def test_signed_pcm_division(self):
        self.assertEqual(trunc_div(-7, 3), -2)
        self.assertEqual(pcm(8192, 4, 4), struct.pack("<IIhhhh", 48000, 4, -8192, 0, 4096, 0))

    def test_max_index_and_source(self):
        for variant in ("max_index", "max_source"):
            index, sources, objects, keys = fixture_data(variant)
            self.assertEqual(len(index) + sum(map(len, sources.values())), INPUT_CAP)
            self.assertEqual(len(json.loads(index)["entries"]), 1024)
            self.assertEqual(len(objects), 80)
            self.assertEqual(len(set(keys)), 80)
            self.assertEqual(sum(x["bytes"] for x in objects.values()), 1536640)

    def test_padding_changes_cache_not_payload(self):
        a, b = fixture_data("max_index"), fixture_data("max_source")
        self.assertEqual(a[2], b[2])
        self.assertNotEqual(a[3], b[3])
        self.assertEqual(json.loads(a[0]), json.loads(b[0]))

    def test_unknown_variant(self):
        with self.assertRaises(ValueError):
            fixture_data("other")

    def test_quota_prefix_discard(self):
        budget = RawBudget(10)
        self.assertEqual(budget.take(7), 7)
        self.assertEqual(budget.take(5), 3)
        self.assertEqual(budget.take(1), 0)
        self.assertTrue(budget.exceeded.is_set())

    def test_new_file_no_overwrite(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a"
            write_new(p, b"original")
            with self.assertRaises(FileExistsError):
                write_new(p, b"replace")
            self.assertEqual(p.read_bytes(), b"original")

    def test_inventory_uses_hash(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a"
            write_new(p, b"abc")
            before = inventory(d)
            p.write_bytes(b"def")
            self.assertNotEqual(before, inventory(d))

    def test_cook_count_and_exit_are_strict(self):
        streams = {n: {"observed": 1, "saved": 1, "discarded": 0} for n in ("stdout", "stderr", "memory")}
        r = {"status": "passed", "actual_exit": 0, "errors": [], "streams": streams,
             "stdout_text": "sim " + "a" * 64 + "\nview " + "b" * 64 + "\nassets 80 cache_hits 80\n"}
        validate_cook(r, {}, 80)
        with self.assertRaises(ValueError):
            validate_cook(r, {}, 0)
        r["actual_exit"] = 1
        with self.assertRaises(ValueError):
            assert_pass(r)

    def test_fake_child_quota_exit_and_reap(self):
        # Fake unit input has no real OS process or memory observation.
        class Fake:
            pid = 424242
            def __init__(self, *args, **kwargs):
                self.stdout, self.stderr = io.BytesIO(b"x" * 30), io.BytesIO(b"y" * 20)
            def poll(self): return 0
            def wait(self): return 0
        class FakeMemory:
            identity = {"pid": 424242, "attribution": "fake-unit-only"}
            def __init__(self, proc): pass
            def sample(self): return {"working_set_bytes": None}
            def close(self): pass
        with tempfile.TemporaryDirectory() as d:
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(10), factory=Fake, memory_factory=FakeMemory)
            self.assertEqual(r["actual_exit"], 0)
            self.assertEqual(r["status"], "failed")
            self.assertEqual(r["cleanup"], "actual_exit_drained_reaped")
            self.assertEqual(sum(c["saved"] for c in r["streams"].values()), 10)
            for c in r["streams"].values():
                self.assertEqual(c["observed"], c["saved"] + c["discarded"])

    def test_spawn_error_still_closes_files(self):
        def fail(*args, **kwargs): raise OSError("synthetic spawn failure")
        with tempfile.TemporaryDirectory() as d:
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), factory=fail)
            self.assertIsNone(r["actual_exit"])
            self.assertEqual(r["cleanup"], "not_spawned")
            self.assertEqual(r["status"], "failed")

    def test_thread_start_failure_prevents_spawn(self):
        with tempfile.TemporaryDirectory() as d, mock.patch.object(threading.Thread, "start", side_effect=RuntimeError("unit start")):
            factory = mock.Mock()
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), factory=factory)
            factory.assert_not_called()
            self.assertEqual(r["cleanup"], "not_spawned")

    def test_setup_failure_drains_then_reaps(self):
        waits = []
        class Fake:
            pid = 424242
            def __init__(self, *a, **k):
                self.stdout, self.stderr = io.BytesIO(b"stdout"), io.BytesIO(b"stderr")
            def wait(self): waits.append("wait"); return 0
        def fail_memory(proc): raise OSError("unit observer setup")
        with tempfile.TemporaryDirectory() as d, mock.patch("builtins.print"):
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), factory=Fake, memory_factory=fail_memory)
            self.assertEqual(waits, ["wait"])
            self.assertEqual(r["actual_exit"], 0)
            self.assertEqual(r["status"], "failed")
            self.assertEqual(r["streams"]["stdout"]["observed"], 6)

    def test_reaper_transient_error_retains_ownership(self):
        class Fake:
            pid = 424242
            def __init__(self, *a, **k):
                self.stdout, self.stderr = io.BytesIO(b"a"), io.BytesIO(b"b")
                self.calls = 0
            def wait(self):
                self.calls += 1
                if self.calls == 1: raise OSError("unit wait transient")
                return 0
        with tempfile.TemporaryDirectory() as d, mock.patch("builtins.print"):
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), factory=Fake,
                              memory_factory=lambda proc: (_ for _ in ()).throw(OSError("unit setup")))
            self.assertEqual(r["actual_exit"], 0)
            self.assertEqual(r["cleanup"], "actual_exit_drained_reaped")
            self.assertEqual(r["status"], "failed")

    def test_watchdog_observes_without_kill(self):
        class Fake:
            pid = 424242
            def __init__(self, *a, **k):
                self.stdout, self.stderr = io.BytesIO(b"a"), io.BytesIO(b"b")
                self.calls = 0
            def poll(self):
                self.calls += 1
                return None if self.calls == 1 else 0
            def wait(self): return 0
        with tempfile.TemporaryDirectory() as d, mock.patch("builtins.print"):
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), watchdog=-1,
                              factory=Fake, memory_factory=lambda proc: None)
            self.assertTrue(r["watchdog_triggered"])
            self.assertEqual(r["actual_exit"], 0)
            self.assertEqual(r["status"], "failed")

    def test_reader_transient_error_recovers_drain(self):
        class BadRead(io.BytesIO):
            def __init__(self): super().__init__(b"kept"); self.first = True
            def read(self, *a):
                if self.first:
                    self.first = False
                    raise OSError("unit read transient")
                return super().read(*a)
        class Fake:
            pid = 424242
            def __init__(self, *a, **k): self.stdout, self.stderr = BadRead(), io.BytesIO()
            def poll(self): return 0 if self.stdout.tell() == 4 else None
            def wait(self): return 0
        with tempfile.TemporaryDirectory() as d, mock.patch("builtins.print"):
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), factory=Fake, memory_factory=lambda p: None)
            self.assertEqual(r["streams"]["stdout"]["observed"], 4)
            self.assertEqual(r["status"], "failed")

    def test_permanent_pipe_error_stops_after_proven_child_exit(self):
        class Broken(io.BytesIO):
            def read(self, *a): raise OSError("unit persistent pipe failure")
        class Fake:
            pid = 424242
            def __init__(self, *a, **k): self.stdout, self.stderr = Broken(), io.BytesIO()
            def poll(self): return 0
            def wait(self): return 0
        with tempfile.TemporaryDirectory() as d, mock.patch("builtins.print"):
            r = child_capture(["fake"], d, Path(d) / "capture", RawBudget(), factory=Fake, memory_factory=lambda p: None)
            self.assertEqual(r["actual_exit"], 0)
            self.assertFalse(r["drain_complete"])
            self.assertEqual(r["cleanup"], "actual_exit_reaped_streams_incomplete")
            self.assertEqual(r["status"], "failed")

    def test_save_error_continues_bounded_drain_discard(self):
        class Fake:
            pid = 424242
            def __init__(self, *a, **k): self.stdout, self.stderr = io.BytesIO(b"x" * 50000), io.BytesIO()
            def poll(self): return 0
            def wait(self): return 0
        class BadBudget(RawBudget):
            def take(self, wanted): raise OSError("unit evidence save")
        with tempfile.TemporaryDirectory() as d:
            r = child_capture(["fake"], d, Path(d) / "capture", BadBudget(), factory=Fake, memory_factory=lambda p: None)
            self.assertEqual(r["streams"]["stdout"]["observed"], 50000)
            self.assertEqual(r["streams"]["stdout"]["saved"], 0)
            self.assertEqual(r["streams"]["stdout"]["discarded"], 50000)
            self.assertEqual(r["status"], "failed")

    def test_process_metadata_failure_returns_failed_actual_exit(self):
        class Fake:
            pid = 424242
            def __init__(self, *a, **k): self.stdout, self.stderr = io.BytesIO(), io.BytesIO()
            def poll(self): return 0
            def wait(self): return 0
        with tempfile.TemporaryDirectory() as d, mock.patch("builtins.print"):
            budget = RawBudget()
            budget.metadata = mock.Mock(side_effect=OSError("unit metadata"))
            r = child_capture(["fake"], d, Path(d) / "capture", budget, factory=Fake, memory_factory=lambda p: None)
            self.assertEqual(r["actual_exit"], 0)
            self.assertFalse(r["process_metadata_persisted"])
            self.assertEqual(r["status"], "failed")

    def test_finalization_preserves_first_failure(self):
        report = {"status": "failed_stop", "failure": "first"}
        final_check(report, "source_after", lambda: (_ for _ in ()).throw(ValueError("drift")))
        self.assertEqual(report["failure"], "first")
        self.assertEqual(len(report["finalization_errors"]), 1)

    def test_cargo_artifact_beyond_prefix(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "cargo.raw"
            artifact = {"reason": "compiler-artifact", "target": {"name": "wanted"}, "executable": "exact"}
            write_new(p, b"noise\n" * 200000 + json_bytes(artifact))
            self.assertEqual(cargo_artifact(p, "wanted"), artifact)
            with self.assertRaises(ValueError): cargo_artifact(p, "missing")

    def test_cache_typed_header_and_body_reject_mutation(self):
        _, sources, objects, keys = fixture_data("max_index")
        fixture = {"expected_objects": objects, "cache_keys": keys}
        with tempfile.TemporaryDirectory() as d:
            for i, key in enumerate(keys):
                payload = struct.pack("<q", (i + 1) * 16384) if i < 64 else pcm(8192 - (i - 64))
                header = struct.pack("<4sIII", b"ORAC", 1, SIM_TYPE if i < 64 else VIEW_TYPE, 1)
                header += bytes.fromhex(key) + struct.pack("<Q", len(payload)) + hashlib.sha256(payload).digest()
                write_new(Path(d) / "cache" / (key + ".bin"), header + payload)
            self.assertEqual(len(validate_cache(d, fixture)), 80)
            path = Path(d) / "cache" / (keys[0] + ".bin")
            original = path.read_bytes()
            damaged = bytearray(original)
            damaged[8] = VIEW_TYPE
            path.write_bytes(damaged)
            with self.assertRaises(ValueError): validate_cache(d, fixture)
            path.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
            with self.assertRaises(ValueError): validate_cache(d, fixture)

    def test_manifest_and_provenance_reject_self_consistent_bad_header(self):
        index, sources, objects, keys = fixture_data("max_index")
        fixture = {"expected_objects": objects, "cache_keys": keys,
                   "input": {p: {"bytes": len(v), "sha256": sha(v)} for p, v in sources.items()}}
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d) / "cold"
            for i in range(80):
                payload = struct.pack("<q", (i + 1) * 16384) if i < 64 else pcm(8192 - (i - 64))
                write_new(folder / "objects" / (sha(payload) + ".bin"), payload)
            ordered = sorted(objects.values(), key=lambda v: v["id"])
            inspection = {"format": "orr.asset-inspect/1", "assets": []}
            for name, typ, count in (("sim", SIM_TYPE, 64), ("view", VIEW_TYPE, 16)):
                data = struct.pack("<4sIII", b"ORAM", 1, typ, count)
                for v in ordered:
                    if v["type_id"] == typ:
                        data += struct.pack("<QIIQ", v["id"], typ, 1, v["bytes"]) + bytes.fromhex(v["sha256"])
                write_new(folder / (name + ".manifest.bin"), data)
                inspection[name + "_manifest_sha256"] = sha(data)
            inspection["assets"] = [{"id": f"a_{v['id']:016x}", "type": "sim.motion_profile" if v["type_id"] == SIM_TYPE else "view.impact_pcm16",
                                     "schema_version": 1, "payload_len": v["bytes"], "payload_sha256": v["sha256"]} for v in ordered]
            provenance = {"format": "orr.asset-cook-report/1", "cooker_format_version": 1, "importer_version": 1,
                          "sources": [{"id": f"a_{i + 1:016x}", "source": f"sources/asset_{i:03}.json",
                                       "source_sha256": sha(sources[f"sources/asset_{i:03}.json"]), "cache_key": keys[i]} for i in range(80)]}
            write_new(folder / "generated_sim.rs", b"unit-only semantic stub\n")
            write_new(folder / "inspect.json", json_bytes(inspection))
            write_new(folder / "report.json", json_bytes(provenance))
            validate_bundle(d, fixture, "cold")
            original = (folder / "sim.manifest.bin").read_bytes()
            damaged = bytearray(original); damaged[4] = 2
            (folder / "sim.manifest.bin").write_bytes(damaged)
            inspection["sim_manifest_sha256"] = sha(damaged)
            (folder / "inspect.json").write_bytes(json_bytes(inspection))
            with self.assertRaises(ValueError): validate_bundle(d, fixture, "cold")
            (folder / "sim.manifest.bin").write_bytes(original)
            inspection["sim_manifest_sha256"] = sha(original)
            (folder / "inspect.json").write_bytes(json_bytes(inspection))
            provenance["sources"][0]["source_sha256"] = "0" * 64
            (folder / "report.json").write_bytes(json_bytes(provenance))
            with self.assertRaises(ValueError): validate_bundle(d, fixture, "cold")

    def test_first_campaign_failure_stops_and_blocks_rerun(self):
        with tempfile.TemporaryDirectory() as d:
            out, source = Path(d), {"head": "a" * 40, "tree": "b" * 40, "files": {}}
            binary = out / "fake.exe"
            write_new(binary, b"unit-only")
            plan = {"source": source, "fixtures": {"max_index": {"input": {}}}}
            passed = {"status": "passed", "actual_exit": 0, "errors": [], "streams": {}}
            builds = {"status": "completed_two_builds", "source": source, "records": [passed, passed],
                      "binaries": {k: {"path": str(binary), **fingerprint(binary)} for k in ("cook", "runtime")}}
            write_new(out / "build-report.json", json_bytes(builds))
            capture = mock.Mock(return_value={"status": "failed", "actual_exit": 1, "errors": ["first"], "streams": {}})
            with mock.patch(__name__ + ".load_plan", return_value=(out, plan, out, source["head"])), mock.patch(__name__ + ".source_identity", return_value=source):
                report = campaign(out, capture)
                self.assertEqual(report["status"], "failed_stop")
                self.assertEqual(report["attempted_children"], 1)
                self.assertEqual(capture.call_count, 1)
                self.assertTrue((out / "report.json").exists())
                with self.assertRaises(ValueError): campaign(out, capture)
                self.assertEqual(capture.call_count, 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    for mode in ("self-test", "prepare", "build", "run"):
        modes.add_argument("--" + mode, action="store_true")
    parser.add_argument("--root", type=Path)
    parser.add_argument("--expected-head")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(SelfTests))
        return 0 if result.wasSuccessful() else 1
    require(args.output is not None, "--output is required")
    if args.prepare:
        require(args.root is not None and args.expected_head is not None, "--root/--expected-head required")
        prepare(args.root, args.expected_head, args.output)
        print(json.dumps({"status": "prepared_not_measured", "output": str(args.output)}))
        return 0
    result = build(args.output) if args.build else campaign(args.output)
    print(json.dumps({"status": result["status"], "output": str(args.output)}))
    return 0 if result["status"] in ("completed_two_builds", "completed_all_nine") else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"asset phase probe failed: {exc!r}", file=sys.stderr)
        sys.exit(1)
