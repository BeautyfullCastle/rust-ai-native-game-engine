#!/usr/bin/env python3
"""Observe the bounded normal verification fixture; this is not a product budget.

The Rust probe supplies API-boundary timings. Memory samples are whole-process
proxies (including fixture/setup); they do not identify stage allocations.
An observer watchdog never kills a probe. After a fault the driver stops future
launches and retains the current process until it actually exits.
The 64 MiB quota covers managed evidence/metadata file content, including a
report replacement copy; it excludes filesystem overhead and observer console.
Artifact size checks do not detect same-size external tampering; an unrecoverable
filesystem failure may also prevent append rollback.

Use --print-plan to capture the fixed JSON plan as UTF-8, then provide a prebuilt
test executable, --plan, a fresh --output directory below the coherent source
root's target, and --expected-head. The executable/source association and quiet
lane are pinned by the owner before the single campaign; this observer does not
build, acquire a lane, rerun a failed cell, or infer cache/CPU isolation.
"""
from __future__ import annotations

import argparse
import base64
import collections
import ctypes
import datetime
import hashlib
import json
import os
import pathlib
import platform
import queue
import re
import statistics
import subprocess
import sys
import threading
import time
import tempfile
import unittest
from unittest import mock

class BudgetExceeded(ValueError):
    """An evidence or metadata write would exceed the campaign's byte cap."""


class Budget:
    """Reserve bounded metadata while stdout/stderr/samples share one quota.

    Metadata replacement counts both the old file and temporary new file, so
    atomic replacement accounts for its temporary managed-file content copy.
    This is not an allocated-filesystem-byte or observer-console limit.
    """
    def __init__(self, total=64 * 1024 * 1024, reserve=2 * 1024 * 1024):
        if not 0 < reserve < total:
            raise ValueError("invalid campaign evidence/metadata budget")
        self.total, self.reserve = total, reserve
        self.evidence_used = 0
        self.metadata = {}
        self.artifacts = {}
        self.lock = threading.Lock()

    def write_artifact(self, path, data):
        path, data = pathlib.Path(path), bytes(data)
        with self.lock:
            if self.evidence_used + len(data) > self.total - self.reserve:
                raise BudgetExceeded("shared evidence cap exhausted")
            known = self.artifacts.get(path)
            if known is None and path.exists():
                raise ValueError("refusing to append an unowned evidence file")
            old_size = 0 if known is None else known
            if path.exists() and path.stat().st_size != old_size:
                raise ValueError("owned evidence file changed externally")
            with path.open("ab") as stream:
                try:
                    count = stream.write(data)
                    stream.flush()
                    if count != len(data):
                        raise OSError("short evidence write")
                except BaseException:
                    # Roll back only our attempted append. If the filesystem
                    # also refuses this, the IO fault is explicitly retained.
                    stream.truncate(old_size)
                    raise
            self.evidence_used += len(data)
            self.artifacts[path] = old_size + len(data)
            return len(data)

    def replace_report(self, path, data):
        path, data = pathlib.Path(path), bytes(data)
        with self.lock:
            if path not in self.metadata and path.exists():
                raise ValueError("refusing to replace an unowned report")
            if path in self.metadata and (not path.is_file()
                    or path.stat().st_size != self.metadata[path]):
                raise ValueError("owned report size changed externally")
            occupied = sum(self.metadata.values())
            if occupied + len(data) > self.reserve:
                raise BudgetExceeded("metadata reserve exhausted during replacement")
            if self.evidence_used + occupied + len(data) > self.total:
                raise BudgetExceeded("campaign managed-file cap exhausted")
            temporary = path.with_name(path.name + ".tmp")
            if temporary.exists():
                raise ValueError("refusing to overwrite an unexpected report temporary")
            try:
                with temporary.open("xb") as stream:
                    if stream.write(data) != len(data):
                        raise OSError("short report write")
                    stream.flush()
                    os.fsync(stream.fileno())
                os.replace(temporary, path)
                self.metadata[path] = len(data)
            finally:
                if temporary.exists():
                    temporary.unlink()
            return len(data)


PROTOCOL_MARKER = "ORR_VERIFICATION_BUDGET_PROBE "
CELL_IDS = (
    "core_idle15_serial", "core_idle15_parallel", "core_idle16_serial",
    "core_idle16_parallel", "core_replay16_serial", "core_replay16_parallel",
    "local_idle16_series_off", "local_idle16_series_on", "local_replay16_series_on",
)
PHASES = ("snapshot_clone_base", "snapshot_clone_candidate", "replay_parse",
          "core_verify", "call_local", "returned_json_encode")
_UNKNOWN = (
    "call_local is one synchronous public aggregate; internal base64 decode, build_inputs, snapshot clones, core verify, and report assembly cannot be timed separately through public APIs.",
    "The clone phases are explicit public Frame::clone proxies, not instrumentation of ErpServer admission copies.",
    "No allocation attribution, worker scheduling profile, RSS bound, or whole-machine memory ceiling is measured.",
)
_HEX64 = re.compile(r"0x[0-9a-f]{16}\Z")
_CORE_KEYS = ("initial", "base_final", "candidate_final")
_LOCAL_KEYS = ("base_start", "candidate_start", "base_final", "candidate_final")
_COUNT_KEYS = ("checksum_samples", "metric_comparisons", "metric_series_values", "returned_json_bytes")


class ProtocolError(ValueError):
    """Malformed or inconsistent child protocol transcript."""


def cell_spec(cell):
    if not isinstance(cell, str) or cell not in CELL_IDS:
        raise ProtocolError("unknown protocol cell")
    if cell.startswith("core_idle"):
        ticks = 15 if "idle15" in cell else 16
        requested = cell.endswith("parallel")
        return dict(ticks=ticks, requested_parallel=requested,
                    effective_parallel=requested and ticks >= 16, series=False,
                    replay=False, kind="core", phases=("core_verify",))
    if cell.startswith("core_replay"):
        requested = cell.endswith("parallel")
        return dict(ticks=16, requested_parallel=requested,
                    effective_parallel=requested, series=False, replay=True,
                    kind="core", phases=("replay_parse", "core_verify"))
    series = cell != "local_idle16_series_off"
    replay = cell == "local_replay16_series_on"
    phases = (("replay_parse",) if replay else ()) + ("call_local", "returned_json_encode")
    return dict(ticks=16, requested_parallel=None, effective_parallel=True,
                series=series, replay=replay, kind="local", phases=phases)


def _object_no_duplicates(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ProtocolError("duplicate JSON key")
        obj[key] = value
    return obj


def _reject_nonfinite(value):
    raise ProtocolError("non-finite JSON constant: " + value)


def _is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def _keys(obj, expected, label):
    if not isinstance(obj, dict) or set(obj) != set(expected):
        raise ProtocolError(label + " fields mismatch")


def _integer(value, label, positive=False, u64=False):
    if (not _is_int(value) or value < (1 if positive else 0)
            or (u64 and value > 0xffffffffffffffff)):
        raise ProtocolError(label + " must be an integer in range")


def _result_fields():
    return {"status", "execution_ticks", "requested_parallel", "effective_parallel",
            "series", "checks", "recording_checksums_checked", "checksums",
            "timings_ns", "counts", "unknown"}


class ProtocolValidator:
    """Validate marker JSONL. phase is receiver-observed, not stage attribution."""
    def __init__(self, cell):
        self.cell = cell
        self.spec = cell_spec(cell)
        self.events = []
        self.fixture = None
        self.result = None
        self.replay_sha256 = None
        self.phase = None
        self._active = None
        self._phase_elapsed = {}
        self._expected = [("start", None)]
        self._at = 0

    def _fixture(self, obj):
        fields = {"game", "yaml_bytes", "entities", "players", "frame_bytes",
                  "replay_base64_bytes", "replay_file_bytes",
                  "replay_decompressed_body_bytes", "replay_ticks", "keyframes",
                  "initial_checksum"}
        if self.spec["replay"]:
            fields.add("replay_base64")
        _keys(obj, fields, "fixture")
        if obj["game"] != "PhysGame":
            raise ProtocolError("unexpected fixture game")
        for name in ("yaml_bytes", "entities", "players", "frame_bytes"):
            _integer(obj[name], "fixture." + name, positive=True)
        if obj["yaml_bytes"] > 262144 or obj["entities"] != 49 or obj["players"] != 2:
            raise ProtocolError("fixture exceeds reviewed bounds")
        if not isinstance(obj["initial_checksum"], str) or not _HEX64.fullmatch(obj["initial_checksum"]):
            raise ProtocolError("invalid initial checksum")
        if obj["replay_decompressed_body_bytes"] is not None:
            raise ProtocolError("decompressed replay body must remain unknown")
        replay_fields = ("replay_base64_bytes", "replay_file_bytes", "replay_ticks", "keyframes")
        if self.spec["replay"]:
            for name in replay_fields:
                _integer(obj[name], "fixture." + name, positive=True)
            if obj["replay_file_bytes"] > 262144 or obj["replay_ticks"] != 256:
                raise ProtocolError("replay fixture exceeds reviewed bounds")
            encoded = obj["replay_base64"]
            if not isinstance(encoded, str) or not encoded.isascii():
                raise ProtocolError("replay_base64 must be ASCII")
            if len(encoded) != obj["replay_base64_bytes"]:
                raise ProtocolError("base64 length mismatch")
            try:
                decoded = base64.b64decode(encoded, validate=True)
            except ValueError as exc:
                raise ProtocolError("invalid replay base64") from exc
            if len(decoded) != obj["replay_file_bytes"]:
                raise ProtocolError("serialized replay length mismatch")
            self.replay_sha256 = hashlib.sha256(decoded).hexdigest()
        elif any(obj[name] is not None for name in replay_fields):
            raise ProtocolError("idle replay metadata must be null")
        self.fixture = dict(obj)

    def _result(self, obj):
        _keys(obj, _result_fields(), "result")
        spec = self.spec
        if obj["status"] != "passed":
            raise ProtocolError("status or execution ticks mismatch")
        _integer(obj["execution_ticks"], "execution_ticks", positive=True)
        if obj["execution_ticks"] != spec["ticks"]:
            raise ProtocolError("status or execution ticks mismatch")
        if obj["requested_parallel"] is not spec["requested_parallel"]:
            raise ProtocolError("requested parallel mismatch")
        if obj["effective_parallel"] is not spec["effective_parallel"] or obj["series"] is not spec["series"]:
            raise ProtocolError("effective mode mismatch")
        checks = obj["checks"]
        _keys(checks, ("identical", "recording_matches"), "checks")
        if checks["identical"] is not True:
            raise ProtocolError("report is not identical")
        if spec["replay"]:
            if checks["recording_matches"] is not True:
                raise ProtocolError("replay must match all 16 executed checksums")
            _integer(obj["recording_checksums_checked"], "recording_checksums_checked", positive=True)
            if obj["recording_checksums_checked"] != 16:
                raise ProtocolError("replay must match all 16 executed checksums")
        elif checks["recording_matches"] is not None or obj["recording_checksums_checked"] is not None:
            raise ProtocolError("idle cell cannot report recording checks")
        checksums = obj["checksums"]
        _keys(checksums, _CORE_KEYS if spec["kind"] == "core" else _LOCAL_KEYS, "checksums")
        if any(not isinstance(v, str) or not _HEX64.fullmatch(v) for v in checksums.values()):
            raise ProtocolError("malformed checksum")
        if spec["kind"] == "core":
            if checksums["base_final"] != checksums["candidate_final"]:
                raise ProtocolError("core base/candidate final checksums differ")
            if checksums["initial"] != self.fixture["initial_checksum"]:
                raise ProtocolError("core initial checksum differs from fixture")
        else:
            if checksums["base_start"] != checksums["candidate_start"]:
                raise ProtocolError("local base/candidate starting checksums differ")
            if checksums["base_final"] != checksums["candidate_final"]:
                raise ProtocolError("local base/candidate final checksums differ")
            if checksums["base_start"] != self.fixture["initial_checksum"]:
                raise ProtocolError("local starting checksum differs from fixture")
        timings = obj["timings_ns"]
        _keys(timings, PHASES, "timings_ns")
        executed = {"snapshot_clone_base", "snapshot_clone_candidate", *spec["phases"]}
        for name in PHASES:
            if name in executed:
                _integer(timings[name], "timings_ns." + name, u64=True)
                if self._phase_elapsed.get(name) != timings[name]:
                    raise ProtocolError("result timing differs from phase event")
            elif timings[name] is not None:
                raise ProtocolError("unexecuted timing must be null")
        counts = obj["counts"]
        _keys(counts, _COUNT_KEYS, "counts")
        for name in _COUNT_KEYS[:3]:
            _integer(counts[name], "counts." + name)
        if spec["kind"] == "core":
            if counts["returned_json_bytes"] is not None:
                raise ProtocolError("core has no returned JSON encoding")
        else:
            _integer(counts["returned_json_bytes"], "returned_json_bytes", positive=True)
        if spec["kind"] == "local" and not spec["series"] and counts["metric_series_values"] != 0:
            raise ProtocolError("series values present while series is disabled")
        if not isinstance(obj["unknown"], list) or tuple(obj["unknown"]) != _UNKNOWN:
            raise ProtocolError("unknown limitations mismatch")
        self.result = dict(obj)

    def feed(self, raw_line):
        if not isinstance(raw_line, bytes):
            raise ProtocolError("protocol line must be bytes")
        try:
            line = raw_line.decode("utf-8", "strict").rstrip("\r\n")
        except UnicodeDecodeError as exc:
            raise ProtocolError("stdout is not UTF-8") from exc
        pos = line.find(PROTOCOL_MARKER)
        if pos < 0:
            return
        if line.find(PROTOCOL_MARKER, pos + len(PROTOCOL_MARKER)) >= 0:
            raise ProtocolError("multiple markers on one line")
        try:
            rec = json.loads(line[pos + len(PROTOCOL_MARKER):],
                             object_pairs_hook=_object_no_duplicates,
                             parse_constant=_reject_nonfinite)
        except ProtocolError:
            raise
        except (ValueError, RecursionError) as exc:
            raise ProtocolError("invalid protocol JSON") from exc
        if not isinstance(rec, dict):
            raise ProtocolError("record must be an object")
        if not _is_int(rec.get("protocol")) or rec["protocol"] != 1:
            raise ProtocolError("protocol version mismatch")
        if not isinstance(rec.get("cell"), str) or rec["cell"] != self.cell:
            raise ProtocolError("cell mismatch")
        event = rec.get("event")
        if not isinstance(event, str) or self._at >= len(self._expected):
            raise ProtocolError("invalid or extra event")
        want_event, want_phase = self._expected[self._at]
        if event != want_event:
            raise ProtocolError("event order mismatch")
        common = {"protocol", "event", "cell"}
        if event == "start":
            _keys(rec, common | {"fixture"}, "start")
            self._fixture(rec["fixture"])
            phases = ("snapshot_clone_base", "snapshot_clone_candidate") + self.spec["phases"]
            for phase in phases:
                self._expected.extend((("phase_start", phase), ("phase_end", phase)))
            self._expected.append(("result", None))
        elif event == "phase_start":
            _keys(rec, common | {"phase"}, "phase_start")
            if not isinstance(rec["phase"], str) or rec["phase"] != want_phase or self._active is not None:
                raise ProtocolError("phase start mismatch")
            self._active = rec["phase"]
            self.phase = rec["phase"]
        elif event == "phase_end":
            _keys(rec, common | {"phase", "elapsed_ns"}, "phase_end")
            if not isinstance(rec["phase"], str) or rec["phase"] != want_phase or self._active != want_phase:
                raise ProtocolError("phase end mismatch")
            _integer(rec["elapsed_ns"], "elapsed_ns", u64=True)
            self._phase_elapsed[rec["phase"]] = rec["elapsed_ns"]
            self._active = None
            self.phase = rec["phase"]
        elif event == "result":
            _keys(rec, common | _result_fields(), "result")
            if self._active is not None:
                raise ProtocolError("result while phase is active")
            self._result({key: value for key, value in rec.items() if key not in common})
        else:
            raise ProtocolError("unknown event")
        self.events.append(dict(rec))
        self._at += 1

    def finish(self):
        if self._at != len(self._expected) or self.fixture is None or self.result is None:
            raise ProtocolError("incomplete protocol transcript")
        return {"fixture": dict(self.fixture), "result": dict(self.result),
                "replay_sha256": self.replay_sha256, "events": list(self.events)}


# Synthetic protocol records only; no game input or malformed replay is run.
def synthetic_records(cell):
    spec = cell_spec(cell)
    replay_data = b"synthetic serialized ORRP" if spec["replay"] else None
    fixture = {
        "game": "PhysGame", "yaml_bytes": 12000, "entities": 49, "players": 2,
        "frame_bytes": 4096, "initial_checksum": "0x0123456789abcdef",
        "replay_base64_bytes": len(base64.b64encode(replay_data)) if replay_data else None,
        "replay_file_bytes": len(replay_data) if replay_data else None,
        "replay_decompressed_body_bytes": None,
        "replay_ticks": 256 if replay_data else None, "keyframes": 5 if replay_data else None,
    }
    if replay_data:
        fixture["replay_base64"] = base64.b64encode(replay_data).decode("ascii")
    rows = [{"protocol": 1, "event": "start", "cell": cell, "fixture": fixture}]
    phases = ("snapshot_clone_base", "snapshot_clone_candidate") + spec["phases"]
    for phase in phases:
        rows.append({"protocol": 1, "event": "phase_start", "cell": cell, "phase": phase})
        rows.append({"protocol": 1, "event": "phase_end", "cell": cell, "phase": phase, "elapsed_ns": 7})
    checksums = ({"initial": "0x0123456789abcdef", "base_final": "0x1111111111111111",
                  "candidate_final": "0x1111111111111111"} if spec["kind"] == "core" else
                 {"base_start": "0x0123456789abcdef", "candidate_start": "0x0123456789abcdef",
                  "base_final": "0x1111111111111111", "candidate_final": "0x1111111111111111"})
    rows.append({"protocol": 1, "event": "result", "cell": cell,
        "status": "passed", "execution_ticks": spec["ticks"],
        "requested_parallel": spec["requested_parallel"],
        "effective_parallel": spec["effective_parallel"], "series": spec["series"],
        "checks": {"identical": True, "recording_matches": True if spec["replay"] else None},
        "recording_checksums_checked": 16 if spec["replay"] else None,
        "checksums": checksums,
        "timings_ns": {p: 7 if p in phases else None for p in PHASES},
        "counts": {"checksum_samples": 2, "metric_comparisons": 3,
                   "metric_series_values": 12 if spec["series"] else 0,
                   "returned_json_bytes": None if spec["kind"] == "core" else 256},
        "unknown": list(_UNKNOWN)})
    return rows


class ProtocolTests(unittest.TestCase):
    def _valid(self, cell):
        validator = ProtocolValidator(cell)
        for row in synthetic_records(cell):
            validator.feed(("libtest prefix " + PROTOCOL_MARKER + json.dumps(row) + "\n").encode())
        return validator.finish()

    def test_all_cells_and_phase_counts(self):
        for cell in CELL_IDS:
            with self.subTest(cell=cell):
                result = self._valid(cell)
                expected = 8 if cell.startswith("core_idle") else 10 if cell.startswith(("core_replay", "local_idle")) else 12
                self.assertEqual(len(result["events"]), expected)
                self.assertEqual(result["result"]["execution_ticks"], cell_spec(cell)["ticks"])

    def test_replay_hash_is_for_decoded_serialized_bytes(self):
        result = self._valid("local_replay16_series_on")
        raw = base64.b64decode(result["fixture"]["replay_base64"], validate=True)
        self.assertEqual(result["replay_sha256"], hashlib.sha256(raw).hexdigest())
        self.assertIsNone(result["fixture"]["replay_decompressed_body_bytes"])

    def test_idle_omits_base64_and_keeps_null_metadata(self):
        fixture = self._valid("core_idle15_serial")["fixture"]
        self.assertNotIn("replay_base64", fixture)
        self.assertIsNone(fixture["replay_file_bytes"])

    def test_duplicate_nonfinite_and_invalid_utf8_rejected(self):
        inputs = ((PROTOCOL_MARKER + '{"a":1,"a":2}').encode(),
                  (PROTOCOL_MARKER + '{"x":NaN}').encode(), b"\xff")
        for raw in inputs:
            with self.subTest(raw=raw), self.assertRaises(ProtocolError):
                ProtocolValidator(CELL_IDS[0]).feed(raw)

    def test_incomplete_and_out_of_order_transcripts_rejected(self):
        rows = synthetic_records(CELL_IDS[0])
        validator = ProtocolValidator(CELL_IDS[0])
        for row in rows[:-1]:
            validator.feed((PROTOCOL_MARKER + json.dumps(row)).encode())
        with self.assertRaises(ProtocolError):
            validator.finish()
        with self.assertRaises(ProtocolError):
            ProtocolValidator(CELL_IDS[0]).feed((PROTOCOL_MARKER + json.dumps(rows[1])).encode())

    def test_result_mode_recording_and_integer_guards(self):
        cases = (
            ("core_idle15_parallel", "execution_ticks", -1),
            ("core_idle15_parallel", "effective_parallel", True),
            ("local_replay16_series_on", "recording_checksums_checked", 15),
        )
        for cell, key, value in cases:
            rows = synthetic_records(cell)
            rows[-1][key] = value
            validator = ProtocolValidator(cell)
            with self.subTest(cell=cell, key=key), self.assertRaises(ProtocolError):
                for row in rows:
                    validator.feed((PROTOCOL_MARKER + json.dumps(row)).encode())

    def test_phase_time_bool_and_negative_rejected(self):
        for value in (True, -1):
            rows = synthetic_records("core_idle16_serial")
            rows[2]["elapsed_ns"] = value
            validator = ProtocolValidator("core_idle16_serial")
            with self.subTest(value=value), self.assertRaises(ProtocolError):
                for row in rows:
                    validator.feed((PROTOCOL_MARKER + json.dumps(row)).encode())

    def test_result_cannot_rewrite_phase_elapsed(self):
        rows = synthetic_records("core_idle16_serial")
        rows[-1]["timings_ns"]["snapshot_clone_base"] = 8
        validator = ProtocolValidator("core_idle16_serial")
        with self.assertRaises(ProtocolError):
            for row in rows:
                validator.feed((PROTOCOL_MARKER + json.dumps(row)).encode())

    def test_wrong_fixture_shape_and_invalid_replay_encoding_rejected(self):
        for cell, mutate in (
            ("core_idle15_serial", lambda fixture: fixture.update(players=True)),
            ("core_replay16_serial", lambda fixture: fixture.update(replay_base64="***")),
        ):
            rows = synthetic_records(cell)
            mutate(rows[0]["fixture"])
            validator = ProtocolValidator(cell)
            with self.subTest(cell=cell), self.assertRaises(ProtocolError):
                for row in rows:
                    validator.feed((PROTOCOL_MARKER + json.dumps(row)).encode())


# Runtime fragment for the single-file driver. Root assembles this with Budget,
# JSONL validation, and the fixed-cell loop. This file is text-only and unexecuted.

class ProcessIdentityError(RuntimeError):
    """The queried Windows PID does not identify this launched child."""


class _FileTime(ctypes.Structure):
    _fields_ = [("low", ctypes.c_ulong), ("high", ctypes.c_ulong)]


class _ProcessMemoryCountersEx(ctypes.Structure):
    _fields_ = [
        ("cb", ctypes.c_ulong), ("page_fault_count", ctypes.c_ulong),
        ("peak_working_set", ctypes.c_size_t), ("working_set", ctypes.c_size_t),
        ("quota_peak_paged_pool", ctypes.c_size_t), ("quota_paged_pool", ctypes.c_size_t),
        ("quota_peak_nonpaged_pool", ctypes.c_size_t), ("quota_nonpaged_pool", ctypes.c_size_t),
        ("pagefile_usage", ctypes.c_size_t), ("peak_pagefile_usage", ctypes.c_size_t),
        ("private_usage", ctypes.c_size_t),
    ]


class _WindowsProcessMemory:
    """Query a duplicate of the exact process handle retained by Popen."""

    def __init__(self, pid, expected_executable, spawn_utc, owned_handle):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.psapi = ctypes.WinDLL("psapi", use_last_error=True)
        self.kernel.GetCurrentProcess.argtypes = []
        self.kernel.GetCurrentProcess.restype = ctypes.c_void_p
        self.kernel.DuplicateHandle.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                                 ctypes.POINTER(ctypes.c_void_p), ctypes.c_ulong,
                                                 ctypes.c_int, ctypes.c_ulong]
        self.kernel.DuplicateHandle.restype = ctypes.c_int
        self.kernel.GetProcessId.argtypes = [ctypes.c_void_p]
        self.kernel.GetProcessId.restype = ctypes.c_ulong
        self.kernel.GetProcessTimes.argtypes = [ctypes.c_void_p] + [ctypes.POINTER(_FileTime)] * 4
        self.kernel.GetProcessTimes.restype = ctypes.c_int
        self.kernel.QueryFullProcessImageNameW.argtypes = [ctypes.c_void_p, ctypes.c_ulong,
                                                            ctypes.POINTER(ctypes.c_wchar), ctypes.POINTER(ctypes.c_ulong)]
        self.kernel.QueryFullProcessImageNameW.restype = ctypes.c_int
        self.kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        self.kernel.CloseHandle.restype = ctypes.c_int
        self.psapi.GetProcessMemoryInfo.argtypes = [ctypes.c_void_p,
                ctypes.POINTER(_ProcessMemoryCountersEx), ctypes.c_ulong]
        self.psapi.GetProcessMemoryInfo.restype = ctypes.c_int

        if owned_handle is None:
            raise ProcessIdentityError("Popen did not expose its owned Windows process handle")
        current = self.kernel.GetCurrentProcess()
        duplicate = ctypes.c_void_p()
        # DuplicateHandle yields another handle to the same kernel object. Keep
        # Popen's source handle open; never reopen by PID or close that source.
        if not self.kernel.DuplicateHandle(current, ctypes.c_void_p(int(owned_handle)), current,
                                           ctypes.byref(duplicate), 0, 0, 0x00000002):
            raise OSError(ctypes.get_last_error(), "DuplicateHandle failed for owned child")
        self.handle = duplicate
        try:
            if int(self.kernel.GetProcessId(self.handle)) != int(pid):
                raise ProcessIdentityError("owned child handle PID mismatch")
            capacity = ctypes.c_ulong(32768)
            image = ctypes.create_unicode_buffer(capacity.value)
            if not self.kernel.QueryFullProcessImageNameW(self.handle, 0, image, ctypes.byref(capacity)):
                raise OSError(ctypes.get_last_error(), "QueryFullProcessImageNameW failed")
            actual = os.path.normcase(os.path.realpath(image.value))
            expected = os.path.normcase(os.path.realpath(os.fspath(expected_executable)))
            if actual != expected:
                raise ProcessIdentityError("owned child executable identity mismatch")
            created, exited, kernel_time, user_time = _FileTime(), _FileTime(), _FileTime(), _FileTime()
            if not self.kernel.GetProcessTimes(self.handle, ctypes.byref(created), ctypes.byref(exited),
                                               ctypes.byref(kernel_time), ctypes.byref(user_time)):
                raise OSError(ctypes.get_last_error(), "GetProcessTimes failed")
            self.creation_filetime = (int(created.high) << 32) | int(created.low)
            spawn_epoch = datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
            delta = spawn_utc - spawn_epoch
            spawn_filetime = 116444736000000000 + ((delta.days * 86400 + delta.seconds) * 10_000_000) + delta.microseconds * 10
            if self.creation_filetime < spawn_filetime:
                raise ProcessIdentityError("opened PID predates this owned child spawn")
            self.executable = actual
        except BaseException:
            self.close()
            raise

    def sample(self):
        counters = _ProcessMemoryCountersEx()
        counters.cb = ctypes.sizeof(counters)
        if not self.psapi.GetProcessMemoryInfo(self.handle, ctypes.byref(counters), counters.cb):
            raise OSError(ctypes.get_last_error(), "GetProcessMemoryInfo failed")
        return {"working_set_bytes": int(counters.working_set),
                "peak_working_set_bytes": int(counters.peak_working_set),
                "private_usage_bytes": int(counters.private_usage)}

    def close(self):
        handle = getattr(self, "handle", None)
        if handle:
            if not self.kernel.CloseHandle(handle):
                raise OSError(ctypes.get_last_error(), "CloseHandle failed for owned child")
            self.handle = None


_RUNTIME_POPEN = subprocess.Popen
_RUNTIME_CLOCK_NS = time.monotonic_ns
_RUNTIME_MEMORY_OPEN = _WindowsProcessMemory if os.name == "nt" else None
_RUNTIME_THREAD_START = lambda thread: thread.start()
_RUNTIME_AFTER_SPAWN = lambda proc: None


class _CaptureState:
    """Global, all-or-nothing append budget shared by every evidence stream."""
    def __init__(self, budget, cell, paths):
        self.budget = budget
        self.cell = cell
        self.paths = paths
        self.lock = threading.Lock()
        self.counts = {name: {"observed_bytes": 0, "saved_bytes": 0, "discarded_bytes": 0}
                       for name in ("stdout", "stderr")}
        self.hashes = {name: hashlib.sha256() for name in paths}
        self.failure = None
        self.alerted = set()
        self.status = "spawning"
        self.storage_closed = False
        self.parser_closed = False
        self.sample_count = 0
        self.sample_observed_bytes = 0
        self.sample_saved_bytes = 0
        self.sample_discarded = 0
        self.reader_errors = []

    def signal(self, kind, message=None):
        emit = False
        with self.lock:
            if self.failure is None:
                self.failure = {"kind": kind, "message": message or kind}
            if kind not in self.alerted:
                self.alerted.add(kind)
                emit = True
        if emit:
            try:
                print(json.dumps({"event": "probe_observer_alert", "cell": self.cell,
                                  "pid": getattr(self, "pid", None),
                                  "status": self.status,
                                  "kind": kind, "message": message or kind}, separators=(",", ":")), flush=True)
            except BaseException:
                pass

    def parsing_enabled(self):
        with self.lock:
            return not self.parser_closed

    def stop_parsing(self):
        with self.lock:
            self.parser_closed = True

    def append(self, name, data):
        data = bytes(data)
        with self.lock:
            if name in self.counts:
                self.counts[name]["observed_bytes"] += len(data)
            elif name == "samples":
                self.sample_count += 1
                self.sample_observed_bytes += len(data)
            if not data:
                return True
            if self.storage_closed:
                if name in self.counts:
                    self.counts[name]["discarded_bytes"] += len(data)
                elif name == "samples":
                    self.sample_discarded += len(data)
                return False
            try:
                written = self.budget.write_artifact(self.paths[name], data)
                if written != len(data):
                    raise OSError("write_artifact returned a non-full append count")
            except BudgetExceeded:
                self.storage_closed = True
                self.failure = self.failure or {"kind": "output_budget_exceeded", "message": "output budget exhausted"}
                if name in self.counts:
                    self.counts[name]["discarded_bytes"] += len(data)
                elif name == "samples":
                    self.sample_discarded += len(data)
                self.parser_closed = True
                emit = "output_budget_exceeded" not in self.alerted
                self.alerted.add("output_budget_exceeded")
                if emit:
                    try:
                        print(json.dumps({"event": "probe_observer_alert", "cell": self.cell,
                                          "pid": getattr(self, "pid", None), "status": self.status,
                                          "kind": "output_budget_exceeded"}, separators=(",", ":")), flush=True)
                    except BaseException:
                        pass
                return False
            except Exception as exc:
                self.storage_closed = True
                self.failure = self.failure or {"kind": "artifact_write_error", "message": type(exc).__name__}
                if name in self.counts:
                    self.counts[name]["discarded_bytes"] += len(data)
                elif name == "samples":
                    self.sample_discarded += len(data)
                self.parser_closed = True
                emit = "artifact_write_error" not in self.alerted
                self.alerted.add("artifact_write_error")
                if emit:
                    try:
                        print(json.dumps({"event": "probe_observer_alert", "cell": self.cell,
                                          "pid": getattr(self, "pid", None), "status": self.status,
                                          "kind": "artifact_write_error"}, separators=(",", ":")), flush=True)
                    except BaseException:
                        pass
                return False
            if name in self.counts:
                self.counts[name]["saved_bytes"] += len(data)
            elif name == "samples":
                self.sample_saved_bytes += len(data)
            self.hashes[name].update(data)
            return True


def _push_line(line_queue, capture, line):
    while capture.parsing_enabled():
        try:
            line_queue.put(bytes(line), timeout=0.05)
            return True
        except queue.Full:
            continue
    return False


def _frame_stdout(chunk, pending, dropping, line_queue, capture):
    pos = 0
    while pos < len(chunk):
        end = chunk.find(b"\n", pos)
        if end < 0:
            part = chunk[pos:]
            if not dropping:
                if len(pending) + len(part) > 512 * 1024:
                    pending.clear()
                    dropping = True
                    capture.stop_parsing()
                    capture.signal("stdout_line_too_large")
                else:
                    pending.extend(part)
            break
        part = chunk[pos:end + 1]
        pos = end + 1
        if dropping:
            dropping = False
            continue
        if len(pending) + len(part) > 512 * 1024:
            pending.clear()
            capture.stop_parsing()
            capture.signal("stdout_line_too_large")
            continue
        pending.extend(part)
        line = bytes(pending[:-1])
        pending.clear()
        if line.endswith(b"\r"):
            line = line[:-1]
        if not _push_line(line_queue, capture, line):
            break
    return dropping


def run_probe(argv, *, cell, cwd, output_dir, budget, on_line, on_sample=None,
              watchdog=30, sample_ms=10, env=None) -> dict:
    """Launch one exact child; returns only after child exit and owned readers close.

    Budget.write_artifact(path, bytes) must append all bytes or raise BudgetExceeded
    before writing any part. One Budget instance is shared across every probe cell.
    on_line receives one complete UTF-8/JSONL payload without CR/LF; it never sends
    data back to the child. on_sample may annotate its mutable sample dict, or return
    a short latest-observed phase label. Neither callback may block on external work.
    """
    argv = [os.fsdecode(arg) for arg in argv]
    if not argv or not cell or not re.fullmatch(r"[A-Za-z0-9_.-]{1,96}", cell):
        raise ValueError("invalid probe argv or cell id")
    if not 0 <= watchdog <= 1_000_000 or not isinstance(sample_ms, int) or not 1 <= sample_ms <= 60_000:
        raise ValueError("watchdog/sample interval outside supported finite bounds")
    if env is None:
        raise ValueError("caller must provide the allowlisted child environment")
    out = pathlib.Path(output_dir).resolve()
    if not out.is_dir():
        raise FileNotFoundError("output directory must already exist")
    names = {"stdout": out / (cell + ".stdout.bin"), "stderr": out / (cell + ".stderr.bin"),
             "samples": out / (cell + ".memory.jsonl"),
             "process": out / (cell + ".process.json")}
    capture = _CaptureState(budget, cell, names)
    for name in ("stdout", "stderr", "samples"):
        if budget.write_artifact(names[name], b"") != 0:
            raise OSError("could not create empty evidence artifact")
    report = {"cell": cell, "argv": argv, "cwd": str(pathlib.Path(cwd).resolve()),
              "started_at_utc": None, "started_monotonic_ns": None,
              "pid": None, "process_creation_filetime": None, "process_executable_identity": None,
              "memory_source": "unsupported_nonwindows", "memory_error": None,
              "memory_query_errors": 0,
              "samples": 0, "sample_discarded_bytes": 0, "sample_interval_target_ms": sample_ms,
              "sample_observed_bytes": 0, "sample_saved_bytes": 0, "sample_sha256_saved": None,
              "sample_delta_count": 0, "sample_delta_min_ns": None,
              "sample_delta_max_ns": None, "sample_delta_sum_ns": 0,
              "missed_sample_slots": 0, "watchdog_seconds": watchdog,
              "watchdog_elapsed": False, "exit_code": None, "finished_at_utc": None,
              "elapsed_ns": None, "failure": None, "streams": {}, "reader_errors": [],
              "evidence_paths": {name: str(path) for name, path in names.items()},
              "status": "spawning", "outcome": None}

    def persist_process(status, *, finished=False):
        capture.status = status
        report["status"] = status
        if capture.failure is not None:
            report["failure"] = dict(capture.failure)
        if finished and status in ("passed", "failed", "spawn_failed"):
            report["outcome"] = "passed" if status == "passed" else "failed"
        snapshot = dict(report)
        snapshot["status"] = status
        if finished:
            snapshot["finished_at_utc"] = report["finished_at_utc"]
            snapshot["elapsed_ns"] = report["elapsed_ns"]
        retried_write_error = False
        while True:
            try:
                snapshot = dict(report)
                snapshot["status"] = report["status"]
                encoded = (json.dumps(snapshot, ensure_ascii=True, separators=(",", ":")) + "\n").encode("utf-8")
                budget.replace_report(names["process"], encoded)
                return
            except KeyboardInterrupt:
                capture.signal("keyboard_interrupt_observed", "saving process checkpoint; child supervision continues")
                report["failure"] = dict(capture.failure)
                if finished:
                    report["status"] = "failed"
                    report["outcome"] = "failed"
                    capture.status = "failed"
                report["process_report_write_error"] = "KeyboardInterrupt"
                # A checkpoint fault must not prevent the next child poll/reap.
                # Try once more, then retain the in-memory failure and supervise.
                if retried_write_error:
                    return
                retried_write_error = True
            except BaseException as exc:
                capture.signal("process_report_write_failed", type(exc).__name__)
                report["process_report_write_error"] = type(exc).__name__
                report["failure"] = dict(capture.failure)
                if finished:
                    report["status"] = "failed"
                    report["outcome"] = "failed"
                    capture.status = "failed"
                if retried_write_error:
                    return
                retried_write_error = True

    exe_arg = pathlib.Path(argv[0])
    if not exe_arg.is_absolute():
        raise ValueError("probe executable argv[0] must be an absolute path")
    expected_exe = exe_arg.resolve()
    kwargs = {"cwd": cwd, "stdin": subprocess.DEVNULL, "stdout": subprocess.PIPE,
              "stderr": subprocess.PIPE, "shell": False, "env": env}
    if os.name == "nt":
        kwargs["creationflags"] = (getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) |
                                  getattr(subprocess, "CREATE_NO_WINDOW", 0))
    else:
        kwargs["start_new_session"] = True
    spawn_utc = datetime.datetime.now(datetime.timezone.utc)
    started_utc = spawn_utc.isoformat()
    started_ns = _RUNTIME_CLOCK_NS()
    report["started_at_utc"] = started_utc
    report["started_monotonic_ns"] = started_ns

    memory = None
    line_queue = queue.Queue(maxsize=8)
    reader_failures = []
    pipe_refs = {"stdout": None, "stderr": None}
    reader_buffers = {name: {"pending": bytearray(), "dropping": False}
                      for name in ("stdout", "stderr")}
    start_gate = threading.Event()

    def drain(name):
        pipe = None
        state = None
        pending = None
        dropping_long_line = False
        read_failed = False
        try:
            state = reader_buffers[name]
            pending = state["pending"]
            dropping_long_line = state["dropping"]
            start_gate.wait()
            pipe = pipe_refs[name]
            if pipe is None:
                return
            while True:
                try:
                    read_chunk = getattr(pipe, "read1", pipe.read)
                    chunk = read_chunk(64 * 1024)
                except BaseException as exc:
                    read_failed = True
                    capture.signal("pipe_read_failed", name + ":" + type(exc).__name__)
                    reader_failures.append({"stream": name, "error_type": type(exc).__name__,
                                            "pending_line_bytes": len(pending) if pending is not None else None,
                                            "complete": False})
                    capture.stop_parsing()
                    break
                if not chunk:
                    break
                try:
                    capture.append(name, chunk)
                    if name == "stdout" and capture.parsing_enabled():
                        dropping_long_line = _frame_stdout(chunk, pending, dropping_long_line, line_queue, capture)
                except BaseException as exc:
                    capture.signal("pipe_processing_failed", name + ":" + type(exc).__name__)
                    capture.stop_parsing()
                    reader_failures.append({"stream": name, "error_type": type(exc).__name__,
                                            "fallback": "drain_and_discard", "complete": False})
                    # Continue reading the pipe so an observer bug cannot fill
                    # the child's OS pipe and strand the owned process.
            if not read_failed and name == "stdout" and pending and capture.parsing_enabled():
                capture.stop_parsing()
                capture.signal("stdout_final_line_missing_newline")
        except BaseException as exc:
            capture.signal("pipe_reader_setup_failed", name + ":" + type(exc).__name__)
            reader_failures.append({"stream": name, "error_type": type(exc).__name__,
                                    "pending_line_bytes": len(pending) if pending is not None else None,
                                    "complete": False})
            capture.stop_parsing()
            if pipe is None:
                try:
                    pipe = pipe_refs.get(name)
                except BaseException:
                    pipe = None
            if pipe is not None:
                # Bounded fallback: bypass parsing, keep draining, and retain an
                # explicit incomplete-evidence error if the underlying read fails.
                while True:
                    try:
                        read_chunk = getattr(pipe, "read1", pipe.read)
                        chunk = read_chunk(64 * 1024)
                    except BaseException as read_exc:
                        read_failed = True
                        capture.signal("pipe_read_failed", name + ":" + type(read_exc).__name__)
                        break
                    if not chunk:
                        break
                    try:
                        capture.append(name, chunk)
                    except BaseException:
                        pass
        finally:
            if state is not None:
                state["dropping"] = dropping_long_line
            if not read_failed and pipe is not None:
                try:
                    pipe.close()
                except BaseException:
                    pass

    threads = []
    started_threads = []
    try:
        for name in ("stdout", "stderr"):
            thread = threading.Thread(target=drain, args=(name,), name="probe-" + name, daemon=False)
            threads.append(thread)
            started_threads.append(thread)
            _RUNTIME_THREAD_START(thread)
    except BaseException as exc:
        capture.signal("reader_start_failed", type(exc).__name__)
        report.update({"failure": dict(capture.failure), "status": "failed", "outcome": "failed",
                       "finished_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                       "elapsed_ns": 0, "prestart_failure": True})
        start_gate.set()
        for thread in threads:
            while thread.is_alive():
                try:
                    thread.join(timeout=0.05)
                except BaseException:
                    time.sleep(0.05)
        persist_process("failed", finished=True)
        return report

    def emergency_after_spawn(proc, exc):
        """Fail closed after setup errors: drain and wait for natural exit, never kill."""
        report["pid"] = int(proc.pid)
        capture.pid = int(proc.pid)
        report["status"] = "pending_live"
        capture.status = "pending_live"
        capture.stop_parsing()
        capture.signal("post_spawn_runtime_failure", type(exc).__name__)
        report["failure"] = dict(capture.failure)
        report["post_spawn_runtime_error"] = type(exc).__name__
        report["status"] = "pending_live"
        if pipe_refs["stdout"] is None:
            pipe_refs["stdout"] = proc.stdout
        if pipe_refs["stderr"] is None:
            pipe_refs["stderr"] = proc.stderr
        start_gate.set()
        persist_process("pending_live")
        child_code = None
        while child_code is None:
            try:
                child_code = proc.poll()
            except BaseException as poll_exc:
                capture.signal("process_poll_error", type(poll_exc).__name__)
                report["process_poll_error"] = type(poll_exc).__name__
            if child_code is None:
                try:
                    time.sleep(0.05)
                except BaseException:
                    capture.signal("keyboard_interrupt_observed", "emergency supervision continues until natural exit")
        report["exit_code"] = int(child_code)
        report["finished_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        report["elapsed_ns"] = max(0, _RUNTIME_CLOCK_NS() - started_ns)
        for thread in started_threads:
            while thread.is_alive():
                try:
                    thread.join(timeout=0.05)
                except BaseException as join_exc:
                    capture.signal("reader_join_error", type(join_exc).__name__)
                    try:
                        time.sleep(0.05)
                    except BaseException:
                        pass
        for pipe in (proc.stdout, proc.stderr):
            try:
                pipe.close()
            except BaseException:
                pass
        while True:
            try:
                proc.wait()
                break
            except BaseException as wait_exc:
                capture.signal("process_reap_error", type(wait_exc).__name__)
                report["process_reap_error"] = type(wait_exc).__name__
                try:
                    time.sleep(0.05)
                except BaseException:
                    pass
        if memory is not None:
            while getattr(memory, "handle", None):
                try:
                    memory.close()
                except BaseException as close_exc:
                    capture.signal("memory_handle_close_failed", type(close_exc).__name__)
                    report["memory_handle_close_error"] = type(close_exc).__name__
                    break
        report["reader_errors"] = list(reader_failures)
        report["sample_discarded_bytes"] = capture.sample_discarded
        report["sample_observed_bytes"] = capture.sample_observed_bytes
        report["sample_saved_bytes"] = capture.sample_saved_bytes
        report["sample_sha256_saved"] = capture.hashes["samples"].hexdigest()
        report["streams"] = {name: {**counts, "sha256_saved_prefix": capture.hashes[name].hexdigest()}
                             for name, counts in capture.counts.items()}
        report["finished_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        report["elapsed_ns"] = max(0, _RUNTIME_CLOCK_NS() - started_ns)
        report["failure"] = capture.failure
        report["status"] = "failed"
        report["outcome"] = "failed"
        persist_process("failed", finished=True)
        return report

    proc = None
    try:
        proc = _RUNTIME_POPEN(argv, **kwargs)
        try:
            pipe_refs["stdout"] = proc.stdout
            pipe_refs["stderr"] = proc.stderr
            start_gate.set()
            report["pid"] = int(proc.pid)
            capture.pid = int(proc.pid)
            report["status"] = "running"
            persist_process("running")
            _RUNTIME_AFTER_SPAWN(proc)
            if _RUNTIME_MEMORY_OPEN is not None:
                report["memory_source"] = "windows-direct-child-query-unavailable"
                try:
                    owned_handle = getattr(proc, "_handle", None) if os.name == "nt" else None
                    memory = _RUNTIME_MEMORY_OPEN(proc.pid, expected_exe, spawn_utc, owned_handle)
                    report["memory_source"] = "psapi.GetProcessMemoryInfo/direct-child-working-set"
                    report["process_creation_filetime"] = memory.creation_filetime
                    report["process_executable_identity"] = memory.executable
                except ProcessIdentityError as exc:
                    capture.signal("memory_identity_mismatch", type(exc).__name__)
                except KeyboardInterrupt:
                    capture.signal("keyboard_interrupt_observed", "opening process counters; child supervision continues")
                    report["memory_source"] = "windows-direct-child-query-unavailable"
                    report["memory_error"] = "KeyboardInterrupt"
                except Exception as exc:
                    report["memory_source"] = "windows-direct-child-query-unavailable"
                    report["memory_error"] = type(exc).__name__
            if capture.failure is not None:
                report["failure"] = dict(capture.failure)
                persist_process("pending_live")
            else:
                persist_process("running")
        except BaseException as exc:
            return emergency_after_spawn(proc, exc)

        interval_ns = sample_ms * 1_000_000
        next_sample_ns = started_ns
        watchdog_ns = int(watchdog * 1_000_000_000)
        previous_sample_ns = None
        child_exit_seen = None
        first_sample_pending = True

        def take_sample(now_ns):
            nonlocal previous_sample_ns, next_sample_ns
            scheduled_ns = next_sample_ns
            sample = {"cell": cell, "pid": int(proc.pid), "elapsed_ns": max(0, now_ns - started_ns),
                      "sample_delta_ns": None if previous_sample_ns is None else now_ns - previous_sample_ns,
                      "sample_source": report["memory_source"], "working_set_bytes": None,
                      "peak_working_set_bytes": None, "private_usage_bytes": None}
            if memory is not None:
                try:
                    sample.update(memory.sample())
                except Exception as exc:
                    sample["sample_error"] = type(exc).__name__
                    report["memory_query_errors"] += 1
                    report["memory_source"] = "windows-direct-child-query-partial"
                    checkpoint_status = "pending_live" if report["status"] == "pending_live" else "running"
                    persist_process(checkpoint_status)
            if on_sample is not None:
                try:
                    phase = on_sample(sample)
                    if isinstance(phase, str) and phase:
                        sample.setdefault("latest_observed_phase", phase[:128])
                except Exception as exc:
                    sample["sample_callback_error"] = type(exc).__name__
                    capture.signal("sample_callback_failed", type(exc).__name__)
            try:
                payload = (json.dumps(sample, ensure_ascii=True, separators=(",", ":")) + "\n").encode("utf-8")
            except Exception as exc:
                capture.signal("sample_serialization_failed", type(exc).__name__)
                fallback = {key: value for key, value in sample.items()
                            if key in ("cell", "pid", "elapsed_ns", "sample_delta_ns", "sample_source",
                                       "working_set_bytes", "peak_working_set_bytes", "private_usage_bytes")}
                fallback["sample_error"] = "serialization:" + type(exc).__name__
                payload = (json.dumps(fallback, ensure_ascii=True, separators=(",", ":")) + "\n").encode("utf-8")
            capture.append("samples", payload)
            report["samples"] += 1
            delta = sample["sample_delta_ns"]
            if delta is not None:
                report["sample_delta_count"] += 1
                report["sample_delta_min_ns"] = delta if report["sample_delta_min_ns"] is None else min(report["sample_delta_min_ns"], delta)
                report["sample_delta_max_ns"] = delta if report["sample_delta_max_ns"] is None else max(report["sample_delta_max_ns"], delta)
                report["sample_delta_sum_ns"] += delta
            previous_sample_ns = now_ns
            slots = max(1, (now_ns - scheduled_ns) // interval_ns + 1)
            report["missed_sample_slots"] += slots - 1
            next_sample_ns = scheduled_ns + slots * interval_ns

        persisted_failure = None
        while child_exit_seen is None or any(t.is_alive() for t in threads) or not line_queue.empty():
            try:
                now_ns = _RUNTIME_CLOCK_NS()
                if child_exit_seen is None:
                    code = proc.poll()
                    if code is not None:
                        child_exit_seen = code
                        report["exit_code"] = int(code)
                        report["status"] = "child_exited_draining"
                        capture.status = "child_exited_draining"
                        if int(code) != 0:
                            capture.signal("process_exit_nonzero", "exit code " + str(int(code)))
                            report["failure"] = dict(capture.failure)
                        report["finished_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
                        report["elapsed_ns"] = max(0, now_ns - started_ns)
                        persist_process("child_exited_draining")
                    else:
                        if now_ns - started_ns >= watchdog_ns and not report["watchdog_elapsed"]:
                            report["watchdog_elapsed"] = True
                            capture.signal("observer_watchdog_elapsed", "child remains live; observing without termination")
                            report["status"] = "pending_live"
                            persist_process("pending_live")
                        if first_sample_pending or now_ns >= next_sample_ns:
                            first_sample_pending = False
                            take_sample(now_ns)
                try:
                    line = line_queue.get(timeout=0.01)
                except queue.Empty:
                    line = None
                if line is not None and capture.parsing_enabled():
                    try:
                        on_line(line)
                    except Exception as exc:
                        capture.stop_parsing()
                        capture.signal("protocol_callback_failed", type(exc).__name__)
                if capture.failure is not None and persisted_failure != capture.failure:
                    persisted_failure = dict(capture.failure)
                    report["failure"] = persisted_failure
                    status = "pending_live" if child_exit_seen is None else "child_exited_draining"
                    report["status"] = status
                    persist_process(status)
            except KeyboardInterrupt:
                capture.signal("keyboard_interrupt_observed", "waiting for owned child/readers; no termination requested")
                report["status"] = "pending_live" if child_exit_seen is None else "child_exited_draining"
                report["failure"] = dict(capture.failure)
                persist_process(report["status"])
                # Keep supervising even after repeated interrupts. Never orphan a
                # child, close a live pipe, or return the lane before natural exit.
                continue
            except BaseException as exc:
                capture.signal("runtime_supervision_error", type(exc).__name__)
                report["runtime_supervision_error"] = type(exc).__name__
                report["failure"] = dict(capture.failure)
                status = "pending_live" if child_exit_seen is None else "child_exited_draining"
                report["status"] = status
                persist_process(status)
                try:
                    time.sleep(0.05)
                except KeyboardInterrupt:
                    capture.signal("keyboard_interrupt_observed", "waiting for owned child/readers; no termination requested")
                    report["failure"] = dict(capture.failure)
                    persist_process(status)

        # Reap only after poll observed natural termination. No terminate/kill path exists.
        for pipe in (proc.stdout, proc.stderr):
            try:
                pipe.close()
            except Exception:
                pass
        while True:
            try:
                report["exit_code"] = int(proc.wait())
                break
            except KeyboardInterrupt:
                capture.signal("keyboard_interrupt_observed", "waiting for owned child/readers; no termination requested")
                report["status"] = "child_exited_draining"
                persist_process("child_exited_draining")
            except Exception as exc:
                capture.signal("process_reap_error", type(exc).__name__)
                report["process_reap_error"] = type(exc).__name__
                report["failure"] = dict(capture.failure)
                report["status"] = "child_exited_draining"
                persist_process("child_exited_draining")
                try:
                    time.sleep(0.05)
                except KeyboardInterrupt:
                    capture.signal("keyboard_interrupt_observed", "waiting for owned child/readers; no termination requested")
        for thread in threads:
            while thread.is_alive():
                try:
                    thread.join(timeout=0.05)
                except KeyboardInterrupt:
                    capture.signal("keyboard_interrupt_observed", "waiting for owned child/readers; no termination requested")
                    persist_process("child_exited_draining")
                except Exception as exc:
                    capture.signal("reader_join_error", type(exc).__name__)
                    report["failure"] = dict(capture.failure)
                    persist_process("child_exited_draining")
                    try:
                        time.sleep(0.05)
                    except KeyboardInterrupt:
                        capture.signal("keyboard_interrupt_observed", "waiting for owned child/readers; no termination requested")
        if memory is not None:
            while getattr(memory, "handle", None):
                try:
                    memory.close()
                except KeyboardInterrupt:
                    capture.signal("keyboard_interrupt_observed", "closing owned process query handle")
                    continue
                except Exception as exc:
                    capture.signal("memory_handle_close_failed", type(exc).__name__)
                    report["memory_handle_close_error"] = type(exc).__name__
                    break
        report["reader_errors"] = reader_failures
        report["finished_at_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        report["elapsed_ns"] = max(0, _RUNTIME_CLOCK_NS() - started_ns)
        report["sample_discarded_bytes"] = capture.sample_discarded
        report["sample_observed_bytes"] = capture.sample_observed_bytes
        report["sample_saved_bytes"] = capture.sample_saved_bytes
        report["sample_sha256_saved"] = capture.hashes["samples"].hexdigest()
        if report["sample_delta_count"]:
            report["sample_delta_mean_ns"] = report["sample_delta_sum_ns"] // report["sample_delta_count"]
        else:
            report["sample_delta_mean_ns"] = None
        report["streams"] = {name: {**counts, "sha256_saved_prefix": capture.hashes[name].hexdigest()}
                             for name, counts in capture.counts.items()}
        report["failure"] = capture.failure
        if report["failure"] is None and report["exit_code"] == 0:
            report["status"] = "passed"
        else:
            report["status"] = "failed"
        persist_process(report["status"], finished=True)
        return report
    except BaseException as exc:
        if proc is None:
            capture.signal("spawn_failed", type(exc).__name__)
            start_gate.set()
            for thread in threads:
                while thread.is_alive():
                    try:
                        thread.join(timeout=0.05)
                    except BaseException:
                        pass
            report.update({"failure": capture.failure, "status": "spawn_failed",
                           "outcome": "failed",
                           "finished_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                           "elapsed_ns": max(0, _RUNTIME_CLOCK_NS() - started_ns)})
            persist_process("spawn_failed", finished=True)
            return report
        return emergency_after_spawn(proc, exc)

class RuntimeCaptureTests(unittest.TestCase):
    """Only fake pipes/processes/clocks/budgets; never starts a real child."""
    class FakeBudget:
        def __init__(self, total):
            self.total = total
            self.files = {}
            self.reports = {}
            self.used = 0
        def write_artifact(self, path, data):
            data = bytes(data)
            if self.used + len(data) > self.total:
                raise BudgetExceeded("fake quota")
            key = str(path)
            self.files[key] = self.files.get(key, b"") + data
            self.used += len(data)
            return len(data)
        def replace_report(self, path, data):
            self.reports[str(path)] = bytes(data)
            return len(data)

    class FakePipe:
        def __init__(self, chunks):
            self.chunks = list(chunks)
            self.closed = False
        def read(self, size):
            if not self.chunks:
                return b""
            chunk = self.chunks.pop(0)
            if len(chunk) <= size:
                return chunk
            self.chunks.insert(0, chunk[size:])
            return chunk[:size]
        def close(self):
            self.closed = True

    class ReadFailurePipe(FakePipe):
        def read(self, size):
            raise OSError("injected underlying read failure")

    class FakeProcess:
        pid = 424242
        def __init__(self, stdout, stderr, finish_after_polls=1):
            self.stdout, self.stderr = stdout, stderr
            self.returncode = None
            self.polls = 0
            self.finish_after_polls = finish_after_polls
            self.terminate_calls = 0
            self.kill_calls = 0
        def poll(self):
            self.polls += 1
            if self.polls >= self.finish_after_polls:
                self.returncode = 0
            return self.returncode
        def wait(self):
            if self.returncode is None:
                raise AssertionError("test fake wait before natural exit")
            return self.returncode
        def terminate(self):
            self.terminate_calls += 1
            raise AssertionError("runtime must never terminate child")
        def kill(self):
            self.kill_calls += 1
            raise AssertionError("runtime must never kill child")

    class FakeClock:
        def __init__(self, start=1_000_000_000, step=10_000_000):
            self.now = start
            self.step = step
        def __call__(self):
            self.now += self.step
            return self.now

    def _run_fake(self, fake, budget, *, watchdog=30, on_line=None, after_spawn=None):
        old_popen, old_memory, old_clock, old_after = _RUNTIME_POPEN, _RUNTIME_MEMORY_OPEN, _RUNTIME_CLOCK_NS, _RUNTIME_AFTER_SPAWN
        try:
            globals()["_RUNTIME_POPEN"] = lambda *a, **k: fake
            globals()["_RUNTIME_MEMORY_OPEN"] = None
            globals()["_RUNTIME_CLOCK_NS"] = self.FakeClock()
            globals()["_RUNTIME_AFTER_SPAWN"] = (lambda proc: after_spawn(proc)) if after_spawn else (lambda proc: None)
            output = pathlib.Path.cwd()
            return run_probe([str(pathlib.Path.cwd() / "fake-probe.exe"), "bounded_normal_fixture_probe"], cell="unit_cell",
                             cwd=pathlib.Path.cwd(), output_dir=output, budget=budget,
                             on_line=on_line or (lambda line: None), watchdog=watchdog, env={})
        finally:
            globals()["_RUNTIME_POPEN"] = old_popen
            globals()["_RUNTIME_MEMORY_OPEN"] = old_memory
            globals()["_RUNTIME_CLOCK_NS"] = old_clock
            globals()["_RUNTIME_AFTER_SPAWN"] = old_after

    def test_complete_lines_cross_chunks_and_callback_gets_payload_only(self):
        fake = self.FakeProcess(self.FakePipe([b'{"event":"sta', b'rt"}\r\n']), self.FakePipe([]))
        seen = []
        report = self._run_fake(fake, self.FakeBudget(1024), on_line=seen.append)
        self.assertEqual(seen, [b'{"event":"start"}'])
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["streams"]["stdout"]["saved_bytes"], len(b'{"event":"start"}\r\n'))

    def test_shared_cap_is_all_or_nothing_and_discards_remainder(self):
        budget = self.FakeBudget(10)
        paths = {name: pathlib.Path.cwd() / (name + ".fake") for name in ("stdout", "stderr", "samples")}
        state = _CaptureState(budget, "unit_cell", paths)
        for name in ("stdout", "stderr", "samples"):
            self.assertEqual(budget.write_artifact(paths[name], b""), 0)
        self.assertTrue(state.append("stdout", b"123456"))
        self.assertFalse(state.append("stderr", b"78901"))
        self.assertFalse(state.append("stdout", b"later"))
        self.assertEqual(state.counts["stdout"]["saved_bytes"] + state.counts["stderr"]["saved_bytes"], 6)
        self.assertEqual(state.counts["stderr"]["discarded_bytes"], 5)
        self.assertEqual(state.counts["stdout"]["discarded_bytes"], 5)
        self.assertEqual(budget.used, 6)

    def test_budget_failure_keeps_child_observed_until_natural_exit(self):
        fake = self.FakeProcess(self.FakePipe([b"123456", b"7890"]), self.FakePipe([b"stderr"]),
                                finish_after_polls=2)
        budget = self.FakeBudget(1)
        report = self._run_fake(fake, budget)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["failure"]["kind"], "output_budget_exceeded")
        self.assertEqual(report["exit_code"], 0)
        self.assertGreater(report["streams"]["stdout"]["discarded_bytes"] +
                           report["streams"]["stderr"]["discarded_bytes"], 0)
        process_record = json.loads(next(iter(budget.reports.values())).decode("utf-8"))
        self.assertEqual(process_record["status"], "failed")
        self.assertEqual(process_record["exit_code"], 0)
        self.assertEqual((fake.terminate_calls, fake.kill_calls), (0, 0))

    def test_observer_watchdog_reports_without_terminating(self):
        fake = self.FakeProcess(self.FakePipe([]), self.FakePipe([]), finish_after_polls=2)
        report = self._run_fake(fake, self.FakeBudget(1024), watchdog=0)
        self.assertTrue(report["watchdog_elapsed"])
        self.assertEqual(report["failure"]["kind"], "observer_watchdog_elapsed")
        self.assertEqual(report["exit_code"], 0)
        self.assertEqual((fake.terminate_calls, fake.kill_calls), (0, 0))

    def test_post_spawn_setup_error_still_drains_and_waits_for_child(self):
        fake = self.FakeProcess(self.FakePipe([b"raw stdout"]), self.FakePipe([b"raw stderr"]),
                                finish_after_polls=2)
        def fail_setup(proc):
            raise RuntimeError("injected post-spawn setup failure")
        report = self._run_fake(fake, self.FakeBudget(1024), after_spawn=fail_setup)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["outcome"], "failed")
        self.assertEqual(report["exit_code"], 0)
        self.assertEqual(report["post_spawn_runtime_error"], "RuntimeError")
        self.assertEqual(report["streams"]["stdout"]["saved_bytes"], len(b"raw stdout"))
        self.assertEqual(report["streams"]["stderr"]["saved_bytes"], len(b"raw stderr"))
        self.assertEqual((fake.terminate_calls, fake.kill_calls), (0, 0))

    def test_repeated_checkpoint_interrupt_still_polls_and_reaps(self):
        class InterruptedBudget(self.FakeBudget):
            def replace_report(self, path, data):
                raise KeyboardInterrupt("injected checkpoint interrupt")
        fake = self.FakeProcess(self.FakePipe([b"raw\n"]), self.FakePipe([]),
                                finish_after_polls=2)
        report = self._run_fake(fake, InterruptedBudget(1024))
        self.assertEqual(report["exit_code"], 0)
        self.assertGreaterEqual(fake.polls, 2)
        self.assertEqual(report["outcome"], "failed")
        self.assertEqual(report["process_report_write_error"], "KeyboardInterrupt")
        self.assertEqual((fake.terminate_calls, fake.kill_calls), (0, 0))

    def test_reader_start_failure_returns_without_spawning(self):
        old_popen, old_thread_start, old_clock = _RUNTIME_POPEN, _RUNTIME_THREAD_START, _RUNTIME_CLOCK_NS
        calls = {"thread": 0, "popen": 0}
        def thread_start(thread):
            calls["thread"] += 1
            if calls["thread"] == 2:
                raise OSError("injected reader-start failure")
            thread.start()
        def popen(*args, **kwargs):
            calls["popen"] += 1
            raise AssertionError("Popen must not run when a reader failed to start")
        try:
            globals()["_RUNTIME_POPEN"] = popen
            globals()["_RUNTIME_THREAD_START"] = thread_start
            globals()["_RUNTIME_CLOCK_NS"] = self.FakeClock()
            report = run_probe([str(pathlib.Path.cwd() / "fake-probe.exe")], cell="prestart_failure",
                               cwd=pathlib.Path.cwd(), output_dir=pathlib.Path.cwd(),
                               budget=self.FakeBudget(1024), on_line=lambda line: None, env={})
        finally:
            globals()["_RUNTIME_POPEN"] = old_popen
            globals()["_RUNTIME_THREAD_START"] = old_thread_start
            globals()["_RUNTIME_CLOCK_NS"] = old_clock
        self.assertTrue(report["prestart_failure"])
        self.assertEqual(report["outcome"], "failed")
        self.assertEqual(calls["popen"], 0)

    def test_underlying_reader_error_is_preserved_until_natural_child_exit(self):
        stdout = self.ReadFailurePipe([])
        fake = self.FakeProcess(stdout, self.FakePipe([]), finish_after_polls=2)
        report = self._run_fake(fake, self.FakeBudget(1024))
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["exit_code"], 0)
        self.assertTrue(any(error.get("stream") == "stdout" and error.get("complete") is False
                            for error in report["reader_errors"]))
        self.assertTrue(stdout.closed)
        self.assertEqual((fake.terminate_calls, fake.kill_calls), (0, 0))


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha256_file(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def strict_json_bytes(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result

    def constant(value):
        raise ValueError("non-finite JSON value: " + value)

    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=pairs,
                          parse_constant=constant)
    except RecursionError as exc:
        raise ValueError("JSON nesting exceeds the observer parser limit") from exc


def fixed_plan(value):
    if (not isinstance(value, dict) or set(value) != {"protocol", "cells"}
            or type(value["protocol"]) is not int or value["protocol"] != 1
            or value["cells"] != list(CELL_IDS)):
        raise ValueError("plan must be protocol 1 with exactly the ordered nine cell IDs")
    return [(cell, role, index) for cell in CELL_IDS
            for role, index in [("warmup", 0), ("sample", 1),
                                ("sample", 2), ("sample", 3)]]


def describe_samples(rows):
    """Summarize measured values only; preserve all three without tail claims."""
    summaries = {}
    for cell in CELL_IDS:
        measured = [r for r in rows if r["cell"] == cell and r["role"] == "sample"
                    and r.get("outcome") == "passed"]
        if len(measured) != 3:
            continue
        stage_stats = {}
        for stage in PHASES:
            values = [r["protocol"]["result"]["timings_ns"][stage]
                      for r in measured]
            if all(x is None for x in values):
                stage_stats[stage] = None
            elif any(type(x) is not int or x < 0 for x in values):
                # Preserve incomplete evidence without inventing a statistic.
                stage_stats[stage] = {"all_three_ns": values,
                                      "error": "inconsistent stage availability"}
            else:
                stage_stats[stage] = {"all_three_ns": values,
                                      "median_ns": statistics.median(values),
                                      "min_ns": min(values), "max_ns": max(values)}
        summaries[cell] = {"samples": 3, "timings": stage_stats}
    return summaries


def git_value(root, *args):
    # Arguments are fixed; environment and credential values are never recorded.
    result = subprocess.run(["git", *args], cwd=root, capture_output=True,
                            timeout=15, check=True)
    return result.stdout.decode("utf-8", errors="strict").strip()


def provenance(root, binary, expected_head=None):
    root = pathlib.Path(root).resolve()
    paths = {"rust_probe": "crates/orr_remote/tests/verification_budget_probe.rs",
             "python_driver": "tools/verification_budget_probe.py",
             "normal_scene": "scenes/physics_demo.scene.yaml"}
    head = git_value(root, "rev-parse", "HEAD")
    if expected_head is not None and head != expected_head:
        raise ValueError("source HEAD differs from the caller's exact pin")
    if git_value(root, "status", "--porcelain=v1", "--untracked-files=normal"):
        raise ValueError("measurement requires a clean, committed coherent source")
    sources = {}
    for label, relative in paths.items():
        path = (root / relative).resolve()
        if not path.is_relative_to(root) or not path.is_file():
            raise ValueError("missing or escaping source path: " + relative)
        sources[label] = {"path": relative, "bytes": path.stat().st_size,
                          "sha256": sha256_file(path)}
    if sources["normal_scene"]["bytes"] > 256 * 1024:
        raise ValueError("normal YAML exceeds the harness-only 256 KiB bound")
    binary = pathlib.Path(binary).resolve(strict=True)
    if not binary.is_file():
        raise ValueError("prebuilt executable is not a file")
    return {"observed_at": utc_now(), "head": head,
            "tree": git_value(root, "rev-parse", "HEAD^{tree}"),
            "branch": git_value(root, "branch", "--show-current"),
            "source_root": str(root), "source_files": sources,
            "binary": {"path": str(binary), "bytes": binary.stat().st_size,
                       "sha256": sha256_file(binary)},
            "environment": {"system": platform.system(), "release": platform.release(),
                            "machine": platform.machine(), "python": platform.python_version(),
                            "logical_cpu_count": os.cpu_count(), "cpu_model": None,
                            "toolchain": None, "cache_state": None, "cpu_isolation": None},
            "binary_build_source_association": "requires the owner's separate prebuild evidence",
            "collection_scope": "allowlisted source/fixture/binary/OS counters; no environment dump"}


def stable_provenance(before, after):
    return all(before[key] == after[key]
               for key in ("head", "tree", "branch", "source_root", "source_files", "binary"))


def compact_protocol(value):
    fixture = dict(value["fixture"])
    fixture.pop("replay_base64", None)  # Exact bytes remain in raw stdout.
    return {"fixture": fixture, "result": value["result"],
            "replay_sha256": value["replay_sha256"],
            "event_count": len(value["events"])}


CHILD_ENV_KEYS = ("SystemRoot", "WINDIR", "PATH", "TEMP", "TMP", "LD_LIBRARY_PATH")


def child_environment(cell, getenv=None):
    # Read only OS/runtime essentials; never enumerate the environment or tokens.
    getenv = os.environ.get if getenv is None else getenv
    values = {key: value for key in CHILD_ENV_KEYS
              if (value := getenv(key)) is not None}
    values["ORR_VERIFICATION_BUDGET_CELL"] = cell
    return values


def campaign(args):
    root = pathlib.Path(args.source_root or pathlib.Path(__file__).resolve().parents[1]).resolve()
    plan_path = pathlib.Path(args.plan).resolve(strict=True)
    if plan_path.stat().st_size > 16 * 1024:
        raise ValueError("plan exceeds 16 KiB")
    raw_plan = plan_path.read_bytes()
    schedule = fixed_plan(strict_json_bytes(raw_plan))
    before = provenance(root, args.test_executable, args.expected_head)
    output = pathlib.Path(args.output).resolve()
    target = (root / "target").resolve()
    if not output.is_relative_to(target) or output == target:
        raise ValueError("output must be a new directory below this source root's target")
    output.mkdir(parents=True, exist_ok=False)
    budget = Budget()
    report = {"schema": "orr.verification-budget-observer/1", "started_at": utc_now(),
              "status": "preparing", "planned_invocations": 36, "measured_samples": 27,
              "limits": {"concurrent_probe_processes": 1, "sample_ms": 10,
                         "observer_watchdog_seconds": 30, "output_bytes": 64 * 1024 * 1024,
                         "warmups_per_cell": 1, "samples_per_cell": 3},
              "provenance_before": before,
              "plan": {"sha256": hashlib.sha256(raw_plan).hexdigest(),
                       "content": strict_json_bytes(raw_plan)},
              "attempts": [], "summaries": {},
              "fixture_reference": None, "replay_reference_sha256": None,
              "replay_reference_descriptors": None,
              "scope": {"rust_elapsed": "public API boundary timing; not private tick/assemble/admission",
                        "memory": "owned process lifetime and sampled WorkingSet proxy including setup",
                        "watchdog": "observer limit, no termination or hard-preemption guarantee",
                        "phase_memory": "receiver-observed marker labels; delay/transients are unbounded",
                        "performance_comparison": None, "product_budget": None}}

    def save():
        budget.replace_report(output / "report.json",
                              (json.dumps(report, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))

    budget.write_artifact(output / "plan.json", raw_plan)
    save()
    print(json.dumps({"output": str(output), "status": "started", "planned": 36}), flush=True)
    try:
        for cell, role, index in schedule:
            # Hash each source/binary before a new launch; never continue after drift.
            if not stable_provenance(before, provenance(root, args.test_executable, args.expected_head)):
                raise ValueError("source or executable changed before a probe launch")
            validator = ProtocolValidator(cell)
            run_dir = output / f"{len(report['attempts']) + 1:02d}-{cell}-{role}-{index}"
            run_dir.mkdir(exist_ok=False)
            env = child_environment(cell)
            argv = [str(pathlib.Path(args.test_executable).resolve()),
                    "bounded_normal_fixture_probe", "--ignored", "--exact", "--nocapture",
                    "--test-threads=1"]
            report["status"] = "observing"
            row = {"cell": cell, "role": role, "index": index,
                   "planned_order": len(report["attempts"]) + 1}
            report["attempts"].append(row)
            save()  # Record the attempted cell before starting its process.
            captured = run_probe(argv, cell=cell, cwd=root, output_dir=run_dir,
                                 budget=budget, on_line=validator.feed,
                                 on_sample=lambda sample: sample.update({"receiver_observed_phase": validator.phase}),
                                 watchdog=args.watchdog_seconds, sample_ms=args.sample_ms, env=env)
            row.update(captured)
            if captured.get("outcome") != "passed":
                report["status"] = "stopped_first_failure"
                save()
                break
            try:
                row["protocol"] = compact_protocol(validator.finish())
            except (ValueError, KeyError, TypeError) as exc:
                row["outcome"] = "protocol_failure"
                row["error"] = str(exc)
                report["status"] = "stopped_first_failure"
                save()
                break
            # This length check is consistency only. Rust embeds include_str!
            # bytes at build time; association with the hashed source scene is
            # supplied by the owner's separate pinned prebuild evidence.
            if row["protocol"]["fixture"]["yaml_bytes"] != before["source_files"]["normal_scene"]["bytes"]:
                row["outcome"] = "fixture_provenance_failure"
                report["status"] = "stopped_first_failure"
                save()
                break
            fixture = row["protocol"]["fixture"]
            reference = {key: fixture[key] for key in
                         ("game", "yaml_bytes", "entities", "players",
                          "frame_bytes", "initial_checksum")}
            if report["fixture_reference"] is None:
                report["fixture_reference"] = reference
            elif report["fixture_reference"] != reference:
                row["outcome"] = "fixture_changed_between_attempts"
                report["status"] = "stopped_first_failure"
                save()
                break
            replay_hash = row["protocol"]["replay_sha256"]
            if replay_hash is not None:
                descriptors = {key: fixture[key] for key in
                               ("replay_base64_bytes", "replay_file_bytes",
                                "replay_ticks", "keyframes")}
                if report["replay_reference_sha256"] is None:
                    report["replay_reference_sha256"] = replay_hash
                    report["replay_reference_descriptors"] = descriptors
                elif (report["replay_reference_sha256"] != replay_hash
                      or report["replay_reference_descriptors"] != descriptors):
                    row["outcome"] = "replay_changed_between_attempts"
                    report["status"] = "stopped_first_failure"
                    save()
                    break
            save()
        else:
            report["status"] = "completed_all_planned"
    except (ValueError, OSError, subprocess.SubprocessError) as exc:
        report["status"] = "observer_failure"
        report["error"] = str(exc)
    finally:
        try:
            after = provenance(root, args.test_executable, args.expected_head)
            report["provenance_after"] = after
            if not stable_provenance(before, after):
                report["status"] = "provenance_changed"
        except (ValueError, OSError, subprocess.SubprocessError) as exc:
            report["status"] = "provenance_check_failed"
            report["post_error"] = str(exc)
        report["summaries"] = describe_samples(report["attempts"])
        report["finished_at"] = utc_now()
        save()
    print(json.dumps({"output": str(output), "status": report["status"],
                      "attempts": len(report["attempts"])}), flush=True)
    return 0 if report["status"] == "completed_all_planned" else 1


class BudgetTests(unittest.TestCase):
    def test_shared_quota_rejects_chunk_before_any_write(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            budget = Budget(total=100, reserve=20)
            budget.write_artifact(root / "stdout", b"a" * 60)
            budget.write_artifact(root / "stderr", b"b" * 20)
            with self.assertRaises(BudgetExceeded):
                budget.write_artifact(root / "stdout", b"c")
            self.assertEqual((root / "stdout").read_bytes(), b"a" * 60)
            self.assertEqual(budget.evidence_used, 80)

    def test_metadata_replacement_counts_transient_copy(self):
        with tempfile.TemporaryDirectory() as folder:
            report = pathlib.Path(folder) / "report"
            budget = Budget(total=100, reserve=20)
            budget.replace_report(report, b"a" * 12)
            with self.assertRaises(BudgetExceeded):
                budget.replace_report(report, b"b" * 12)
            self.assertEqual(report.read_bytes(), b"a" * 12)
            budget.replace_report(report, b"c" * 8)
            self.assertEqual(report.read_bytes(), b"c" * 8)
            self.assertFalse(report.with_name("report.tmp").exists())


class DriverTests(unittest.TestCase):
    def test_child_environment_reads_only_allowlisted_keys(self):
        reads = []

        def fake_getenv(key):
            reads.append(key)
            return {"PATH": "os-runtime-path", "SECRET_TOKEN": "never-read"}.get(key)

        env = child_environment(CELL_IDS[0], fake_getenv)
        self.assertEqual(reads, list(CHILD_ENV_KEYS))
        self.assertEqual(env, {"PATH": "os-runtime-path",
                               "ORR_VERIFICATION_BUDGET_CELL": CELL_IDS[0]})

    def test_fixed_plan_has_36_serial_attempts(self):
        rows = fixed_plan({"protocol": 1, "cells": list(CELL_IDS)})
        self.assertEqual(len(rows), 36)
        for cell in CELL_IDS:
            self.assertEqual([(r, i) for c, r, i in rows if c == cell],
                             [("warmup", 0), ("sample", 1), ("sample", 2), ("sample", 3)])

    def test_campaign_cli_requires_expected_head_before_preflight(self):
        with mock.patch(__name__ + ".campaign") as launch, mock.patch.object(sys, "stderr"):
            with self.assertRaises(SystemExit) as caught:
                main(["--test-executable", "prebuilt", "--plan", "plan.json", "--output", "target/new"])
        self.assertEqual(caught.exception.code, 2)
        launch.assert_not_called()

    def test_custom_or_duplicate_plan_is_rejected(self):
        for value in ({"protocol": True, "cells": list(CELL_IDS)},
                      {"protocol": 1, "cells": list(reversed(CELL_IDS))},
                      {"protocol": 1, "cells": list(CELL_IDS), "samples": 4}):
            with self.assertRaises(ValueError):
                fixed_plan(value)

    def test_json_duplicates_and_nonfinite_are_rejected(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":Infinity}'):
            with self.assertRaises(ValueError):
                strict_json_bytes(raw)
        # Parser recursion limits vary between supported Python versions. The
        # boundary here is exception classification, not a claimed JSON-depth cap.
        with mock.patch.object(json, "loads", side_effect=RecursionError("parser depth")):
            with self.assertRaisesRegex(ValueError, "JSON nesting"):
                strict_json_bytes(b"[]")

    def test_provenance_comparison_ignores_capture_time_only(self):
        before = dict.fromkeys(("head", "tree", "branch", "source_root", "source_files", "binary"), "same")
        after = dict(before, observed_at="later")
        self.assertTrue(stable_provenance(before, after))
        after["binary"] = "changed"
        self.assertFalse(stable_provenance(before, after))

    def test_summary_excludes_warmup_and_requires_all_three(self):
        cell = CELL_IDS[0]
        rows = [{"cell": cell, "role": role, "outcome": "passed",
                 "protocol": {"result": {"timings_ns": {p: number for p in PHASES}}}}
                for role, number in [("warmup", 999), ("sample", 10),
                                     ("sample", 30), ("sample", 20)]]
        summary = describe_samples(rows)[cell]["timings"][PHASES[0]]
        self.assertEqual(summary, {"all_three_ns": [10, 30, 20], "median_ns": 20,
                                  "min_ns": 10, "max_ns": 30})
        self.assertNotIn(cell, describe_samples(rows[:-1]))

    def test_summary_preserves_mixed_unknown_without_exception(self):
        rows = [{"cell": CELL_IDS[0], "role": "sample", "outcome": "passed",
                 "protocol": {"result": {"timings_ns": dict.fromkeys(PHASES, x)}}}
                for x in (None, 3, 4)]
        result = describe_samples(rows)[CELL_IDS[0]]["timings"][PHASES[0]]
        self.assertEqual(result["all_three_ns"], [None, 3, 4])
        self.assertIn("error", result)

    def _synthetic_campaign(self, fault=None):
        # Campaign orchestration only: no executable or game process exists.
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            (root / "target").mkdir()
            plan = root / "plan.json"
            plan.write_text(json.dumps({"protocol": 1, "cells": list(CELL_IDS)}))
            frozen = dict.fromkeys(("head", "tree", "branch", "source_root", "binary"), "synthetic")
            frozen["source_files"] = {"normal_scene": {"bytes": 12000}}
            calls = []

            def fake_probe(argv, **kwargs):
                calls.append(kwargs["cell"])
                if fault == "exit":
                    return {"outcome": "failed", "exit_code": 1}
                rows = synthetic_records(kwargs["cell"])
                if fault == "fixture" and len(calls) == 2:
                    rows[0]["fixture"]["initial_checksum"] = "0xabcdef0123456789"
                    rows[-1]["checksums"]["initial"] = "0xabcdef0123456789"
                for record in rows[:-1] if fault == "incomplete" else rows:
                    kwargs["on_line"]((PROTOCOL_MARKER + json.dumps(record)).encode())
                return {"outcome": "passed", "exit_code": 0}

            args = argparse.Namespace(source_root=str(root), plan=str(plan),
                                      output=str(root / "target" / "capture"),
                                      test_executable=str(root / "synthetic.exe"),
                                      expected_head=None, watchdog_seconds=30, sample_ms=10)
            with mock.patch(__name__ + ".provenance", return_value=frozen), \
                    mock.patch(__name__ + ".run_probe", side_effect=fake_probe), \
                    mock.patch("builtins.print"):
                code = campaign(args)
            report = json.loads((root / "target" / "capture" / "report.json").read_bytes())
            return code, calls, report

    def test_campaign_stops_first_exit_or_incomplete_protocol(self):
        for fault in ("exit", "incomplete"):
            with self.subTest(fault=fault):
                code, calls, report = self._synthetic_campaign(fault)
                self.assertEqual(code, 1)
                self.assertEqual(len(calls), 1)
                self.assertEqual(report["status"], "stopped_first_failure")

    def test_campaign_rejects_changed_fixture_between_fresh_attempts(self):
        code, calls, report = self._synthetic_campaign("fixture")
        self.assertEqual(code, 1)
        self.assertEqual(len(calls), 2)
        self.assertEqual(report["attempts"][-1]["outcome"], "fixture_changed_between_attempts")

    def test_synthetic_campaign_exactly_36_and_27_measured(self):
        code, calls, report = self._synthetic_campaign()
        self.assertEqual(code, 0)
        self.assertEqual(len(calls), 36)
        self.assertEqual(sum(x["role"] == "sample" for x in report["attempts"]), 27)
        self.assertEqual(len(report["summaries"]), 9)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--print-plan", action="store_true")
    parser.add_argument("--test-executable")
    parser.add_argument("--plan")
    parser.add_argument("--output")
    parser.add_argument("--source-root")
    parser.add_argument("--expected-head")
    parser.add_argument("--warmups", type=int, choices=[1], default=1)
    parser.add_argument("--samples", type=int, choices=[3], default=3)
    parser.add_argument("--sample-ms", type=int, choices=[10], default=10)
    parser.add_argument("--watchdog-seconds", type=int, choices=[30], default=30)
    args = parser.parse_args(argv)
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    if args.print_plan:
        print(json.dumps({"protocol": 1, "cells": list(CELL_IDS)}, indent=2))
        return 0
    if not all((args.test_executable, args.plan, args.output, args.expected_head)):
        parser.error("--test-executable, --plan, --output and --expected-head are required")
    if args.expected_head is not None and not re.fullmatch(r"[0-9a-f]{40}", args.expected_head):
        parser.error("--expected-head must be an exact lowercase 40-hex commit SHA")
    try:
        return campaign(args)
    except (ValueError, OSError, subprocess.SubprocessError) as exc:
        print("observer preflight failed: " + str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
