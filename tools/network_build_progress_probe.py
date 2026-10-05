#!/usr/bin/env python3
"""Bounded, fail-stop prebuilt network/FFI checks during owned Cargo compilation.

Watchdogs stop scheduling; they never kill children. A live child retains the
lane until its natural exit and all owned descendants have exited.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import threading
import time

SAFE_CASES = {
    "loopback": ("orr_net", ("idle_timeout_detected::ws",)),
    "net_e2e": ("orr_server", ("two_clients_over_websocket", "disconnect_and_rejoin_over_quic")),
    "client_session": ("orr_ffi", ("two_c_abi_clients_and_a_rust_client_play_with_prediction_and_rollback",)),
}
CLAIMED = ("tools/network_build_progress_probe.py", "tools/tests/test_network_build_progress_probe.py",
           "docs/network-build-progress.md")
MAX_STREAM = 4 * 1024 * 1024
POLL = 0.2
ENV_KEYS = ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL",
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTDOCFLAGS",
            "RUSTC_WORKSPACE_WRAPPER", "RUSTUP_TOOLCHAIN", "CARGO_HOME", "RUSTUP_HOME", "RUSTDOC",
            "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_DEBUG", "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_INCREMENTAL")


def environment_identity():
    # Never capture the whole environment, tokens, credentials or user data.
    require(not any(key not in ENV_KEYS and (key.startswith("CARGO_PROFILE_") or
                key.startswith("CARGO_TARGET_") or key.startswith("CARGO_BUILD_")) for key in os.environ),
            "unsupported inherited build override")
    return {key: os.environ.get(key) for key in ENV_KEYS}


def toolchain_identity():
    tools = {}
    for name, flag in (("rustc", "-Vv"), ("cargo", "-V")):
        path = shutil.which(name)
        require(path is not None, "missing tool: " + name)
        tools[name] = {"path": str(Path(path).resolve()), "sha256": digest(path),
                       "version": subprocess.check_output([path, flag], stderr=subprocess.PIPE).decode()}
    return tools


def require_runtime_platform():
    require(os.name == "nt" and _kernel32 is not None,
            "unsupported: owned descendant cleanup and compiler attribution require Windows")


def require(ok, message):
    if not ok:
        raise ValueError(message)


def utc():
    return datetime.now(timezone.utc).isoformat()


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for block in iter(lambda: f.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def save(path, value):
    temp = Path(str(path) + ".tmp")
    temp.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temp.replace(path)


def canonical_sha256(value):
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def preparation_identity(source, profile, parent_environment, toolchain):
    return {"source": source, "profile": profile, "parent_environment": parent_environment,
            "toolchain": toolchain}


def preparation_once_key(source, profile, parent_environment, toolchain):
    return canonical_sha256(preparation_identity(source, profile, parent_environment, toolchain))


def reserve_marker(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation burns the identity even if the owner later fails.
    with path.open("x", encoding="utf-8", newline="\n") as marker:
        marker.write(json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n")


def finalize_marker(path, value):
    # Retain the exclusive reservation; only replace its owned contents.
    save(Path(path), value)


def once_marker_root(root):
    return Path(root).resolve() / "target" / "n6-probe-once"


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE).decode().strip()


def source_identity(root):
    require(not git(root, "status", "--porcelain", "--untracked-files=all"),
            "source tree has tracked or untracked changes")
    paths = set(git(root, "ls-files").splitlines()) | set(CLAIMED)
    require(all((root / p).is_file() for p in paths), "source manifest contains a missing file")
    files = {p: digest(root / p) for p in sorted(paths)}
    return {"head": git(root, "rev-parse", "HEAD"), "tree": git(root, "rev-parse", "HEAD^{tree}"),
            "files": files, "manifest_sha256": hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()}


def exact_result(text, name, expected_filtered=None):
    summaries = re.findall(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", text, re.M)
    require(len(summaries) == 1, "missing/duplicate libtest summary")
    status, *counts = summaries[0]
    counts = list(map(int, counts))
    require(status == "ok" and counts[:4] == [1, 0, 0, 0], "exact case did not pass once without ignore")
    # --nocapture may put the case's diagnostics after the prefix and its
    # final `ok` on a later line. The single-case summary supplies completion.
    require(len(re.findall(r"^test " + re.escape(name) + r" \.\.\.(?:[ \t]|\r?$)", text, re.M)) == 1,
            "intended exact case was not executed once")
    if expected_filtered is not None:
        require(counts[4] == expected_filtered, "filtered count differs from prepared inventory")
    return dict(zip(("passed", "failed", "ignored", "measured", "filtered"), counts))


def compiler_overlap(samples, start, end):
    """Require observed rustc CPU increases across the entire interval.

    Intervals are sample-resolution evidence, not continuous CPU utilization.
    Cargo alone, cached artifacts and alive-but-idle rustc do not qualify.
    """
    spans = []
    for a, b in zip(samples, samples[1:]):
        # A metadata gap invalidates both adjacent spans, even when the same
        # rustc identity appears again later. Never bridge missing evidence.
        if a.get("attribution_complete") is False or b.get("attribution_complete") is False:
            continue
        old = {(p["pid"], p["creation_100ns"]): p["cpu_100ns"] for p in a.get("compilers", [])}
        moving = any(p["cpu_100ns"] > old.get((p["pid"], p["creation_100ns"]), p["cpu_100ns"])
                     for p in b.get("compilers", []))
        if moving and 0 < b["monotonic"] - a["monotonic"] <= 1.0:
            spans.append((a["monotonic"], b["monotonic"]))
    cursor = start
    for lo, hi in spans:
        if hi < cursor:
            continue
        if lo > cursor:
            return False
        cursor = max(cursor, hi)
        if cursor >= end:
            return True
    return False


class PosixChild:
    """Natural-exit process group; compiler attribution is unavailable here."""
    requires_job_cleanup = False
    def __init__(self, argv, cwd, stdout, stderr, env):
        self.p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stdout, stderr=stderr, start_new_session=True)
        self.pid = self.p.pid

    def poll(self):
        return self.p.poll()

    def sample(self):
        # We deliberately cannot attest descendant emptiness portably.
        return {"active": None, "cpu_100ns": None, "compilers": [], "containment": "unverified"}

    def wait(self):
        return self.p.wait()

    def close(self):
        pass


import ctypes
import os
import pathlib
import shutil
import subprocess
from ctypes import wintypes

if os.name != "nt":  # Importable for static review; construction is Windows-only.
    _kernel32 = None
else:
    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

_INVALID = ctypes.c_void_p(-1).value
_CREATE_SUSPENDED = 0x00000004
_CREATE_NO_WINDOW = 0x08000000
_CREATE_UNICODE_ENVIRONMENT = 0x00000400
_EXTENDED_STARTUPINFO_PRESENT = 0x00080000
_STARTF_USESHOWWINDOW = 0x00000001
_STARTF_USESTDHANDLES = 0x00000100
_SW_HIDE = 0
_PROC_THREAD_ATTRIBUTE_HANDLE_LIST = 0x00020002
_JOB_BASIC_ACCOUNTING = 1
_JOB_BASIC_PROCESS_ID_LIST = 3
_WAIT_OBJECT_0 = 0
_WAIT_TIMEOUT = 258
_PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
_ERROR_INVALID_PARAMETER = 87


def _configure_api():
    """Declare pointer-sized signatures; ctypes' default c_int is unsafe for HANDLEs."""
    k = _api()
    k.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    k.CreateJobObjectW.restype = wintypes.HANDLE
    k.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    k.AssignProcessToJobObject.restype = wintypes.BOOL
    k.ResumeThread.argtypes = [wintypes.HANDLE]
    k.ResumeThread.restype = wintypes.DWORD
    k.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                              ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    k.CreateFileW.restype = wintypes.HANDLE
    k.CreateProcessW.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p,
                                 ctypes.c_void_p, wintypes.BOOL, wintypes.DWORD,
                                 ctypes.c_void_p, wintypes.LPCWSTR, ctypes.c_void_p,
                                 ctypes.POINTER(_PROCESS_INFORMATION)]
    k.CreateProcessW.restype = wintypes.BOOL
    k.InitializeProcThreadAttributeList.argtypes = [ctypes.c_void_p, wintypes.DWORD,
                                                     wintypes.DWORD, ctypes.POINTER(ctypes.c_size_t)]
    k.InitializeProcThreadAttributeList.restype = wintypes.BOOL
    k.UpdateProcThreadAttribute.argtypes = [ctypes.c_void_p, wintypes.DWORD, ctypes.c_size_t,
                                            ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p,
                                            ctypes.POINTER(ctypes.c_size_t)]
    k.UpdateProcThreadAttribute.restype = wintypes.BOOL
    k.DeleteProcThreadAttributeList.argtypes = [ctypes.c_void_p]
    k.DeleteProcThreadAttributeList.restype = None
    k.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                            wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    k.QueryInformationJobObject.restype = wintypes.BOOL
    k.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    k.OpenProcess.restype = wintypes.HANDLE
    k.IsProcessInJob.argtypes = [wintypes.HANDLE, wintypes.HANDLE, ctypes.POINTER(wintypes.BOOL)]
    k.IsProcessInJob.restype = wintypes.BOOL
    k.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR,
                                             ctypes.POINTER(wintypes.DWORD)]
    k.QueryFullProcessImageNameW.restype = wintypes.BOOL
    k.GetProcessTimes.argtypes = [wintypes.HANDLE, ctypes.POINTER(_FILETIME),
                                  ctypes.POINTER(_FILETIME), ctypes.POINTER(_FILETIME),
                                  ctypes.POINTER(_FILETIME)]
    k.GetProcessTimes.restype = wintypes.BOOL
    k.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    k.GetExitCodeProcess.restype = wintypes.BOOL
    k.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    k.WaitForSingleObject.restype = wintypes.DWORD
    k.CloseHandle.argtypes = [wintypes.HANDLE]
    k.CloseHandle.restype = wintypes.BOOL
    return k


class _STARTUPINFOW(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("lpReserved", wintypes.LPWSTR),
                ("lpDesktop", wintypes.LPWSTR), ("lpTitle", wintypes.LPWSTR),
                ("dwX", wintypes.DWORD), ("dwY", wintypes.DWORD),
                ("dwXSize", wintypes.DWORD), ("dwYSize", wintypes.DWORD),
                ("dwXCountChars", wintypes.DWORD), ("dwYCountChars", wintypes.DWORD),
                ("dwFillAttribute", wintypes.DWORD), ("dwFlags", wintypes.DWORD),
                ("wShowWindow", wintypes.WORD), ("cbReserved2", wintypes.WORD),
                ("lpReserved2", ctypes.POINTER(ctypes.c_ubyte)),
                ("hStdInput", wintypes.HANDLE), ("hStdOutput", wintypes.HANDLE),
                ("hStdError", wintypes.HANDLE)]


class _STARTUPINFOEXW(ctypes.Structure):
    _fields_ = [("StartupInfo", _STARTUPINFOW), ("lpAttributeList", ctypes.c_void_p)]


class _PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [("hProcess", wintypes.HANDLE), ("hThread", wintypes.HANDLE),
                ("dwProcessId", wintypes.DWORD), ("dwThreadId", wintypes.DWORD)]


class _FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", wintypes.DWORD), ("dwHighDateTime", wintypes.DWORD)]


class _JOB_ACCOUNTING(ctypes.Structure):
    _fields_ = [("TotalUserTime", ctypes.c_longlong), ("TotalKernelTime", ctypes.c_longlong),
                ("ThisPeriodTotalUserTime", ctypes.c_longlong),
                ("ThisPeriodTotalKernelTime", ctypes.c_longlong),
                ("TotalPageFaultCount", wintypes.DWORD), ("TotalProcesses", wintypes.DWORD),
                ("ActiveProcesses", wintypes.DWORD),
                ("TotalTerminatedProcesses", wintypes.DWORD)]


class ProcessSetupPending(RuntimeError):
    """A created process remains owned by `child`; containment/cleanup is unproven."""

    def __init__(self, message: str, child: "WindowsChild") -> None:
        super().__init__(message)
        self.child = child


def _api():
    if _kernel32 is None:
        raise OSError("WindowsChild is available only on Windows")
    return _kernel32


def _check(ok, what: str):
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error(), what)


def _ft(value: _FILETIME) -> int:
    return (int(value.dwHighDateTime) << 32) | int(value.dwLowDateTime)


class WindowsChild:
    """Suspended CreateProcess, assigned to a non-killing Job Object, then resumed.

    `outfile` and `errfile` are caller-owned binary streams (often os.pipe
    writers with ready readers). Only their handles and NUL stdin are inherited.
    Closing is allowed only after this process exited naturally and job Active=0.
    """
    requires_job_cleanup = True

    def __init__(self, argv, cwd, outfile, errfile, env=None):
        k = _configure_api()
        if not argv or not isinstance(argv[0], str):
            raise ValueError("argv must be a nonempty string sequence")
        app = _resolve_application(argv[0], cwd, env)
        self.pid = 0
        self._process = self._thread = self._job = self._stdin = None
        self._assigned = False
        self._closed = False
        self._parent_natural_exit = False
        self._cleanup_zero_verified = False
        self._attr_storage = None
        self._attr_list = None
        self._owned_after_failure = False
        self._query_handles = []

        job = k.CreateJobObjectW(None, None)
        _check(job, "CreateJobObjectW")
        self._job = job  # No limits are set; in particular, no kill-on-close flag.
        nul_sa = _SECURITY_ATTRIBUTES(ctypes.sizeof(_SECURITY_ATTRIBUTES), None, True)
        self._stdin = k.CreateFileW("NUL", 0x80000000, 3, ctypes.byref(nul_sa), 3, 0x80, None)
        if self._stdin in (None, _INVALID):
            err = ctypes.get_last_error()
            _close(k, self._job)
            self._job = None
            raise ctypes.WinError(err, "CreateFileW(NUL)")

        try:
            out_value = msvcrt_handle(outfile)
            err_value = msvcrt_handle(errfile)
            out_h = wintypes.HANDLE(out_value)
            err_h = wintypes.HANDLE(err_value)
            in_h = wintypes.HANDLE(self._stdin)
            handles = (wintypes.HANDLE * 3)(in_h, out_h, err_h)
            self._attr_list, self._attr_storage = _make_handle_list(handles)
        except BaseException:
            self._discard_precreate(k)
            raise
        try:
            sx = _STARTUPINFOEXW()
            sx.StartupInfo.cb = ctypes.sizeof(_STARTUPINFOEXW)
            sx.StartupInfo.dwFlags = _STARTF_USESHOWWINDOW | _STARTF_USESTDHANDLES
            sx.StartupInfo.wShowWindow = _SW_HIDE
            sx.StartupInfo.hStdInput, sx.StartupInfo.hStdOutput, sx.StartupInfo.hStdError = in_h, out_h, err_h
            sx.lpAttributeList = self._attr_list
            flags = _CREATE_SUSPENDED | _CREATE_NO_WINDOW | _EXTENDED_STARTUPINFO_PRESENT
            env_block = None
            if env is not None:
                env_block = ctypes.create_unicode_buffer("\0".join(f"{key}={value}" for key, value in sorted(env.items())) + "\0\0")
                flags |= _CREATE_UNICODE_ENVIRONMENT
            cmd = ctypes.create_unicode_buffer(subprocess.list2cmdline(list(argv)))
            pi = _PROCESS_INFORMATION()
            out_value, out_inherit = inheritable_handle(outfile)
        except BaseException:
            self._discard_precreate(k)
            raise
        try:
            err_value, err_inherit = inheritable_handle(errfile)
        except BaseException:
            os.set_handle_inheritable(out_value, out_inherit)
            self._discard_precreate(k)
            raise
        restore_error = None
        try:
            ok = k.CreateProcessW(app, cmd, None, None, True, flags,
                                  ctypes.cast(env_block, ctypes.c_void_p) if env_block is not None else None,
                                  os.fspath(cwd) if cwd is not None else None,
                                  ctypes.byref(sx), ctypes.byref(pi))
            if ok:
                self.pid, self._process, self._thread = int(pi.dwProcessId), pi.hProcess, pi.hThread
        except BaseException:
            self._discard_precreate(k)
            raise
        finally:
            for handle, was_inheritable in ((out_value, out_inherit), (err_value, err_inherit)):
                try:
                    os.set_handle_inheritable(handle, was_inheritable)
                except BaseException as exc:
                    restore_error = restore_error or exc
        if not ok:
            self._discard_precreate(k)
            _check(ok, "CreateProcessW")
        if not k.AssignProcessToJobObject(self._job, self._process):
            # Do not strand a suspended process. Resume it for natural exit, but
            # retain every handle and make the uncontained state explicit.
            self._owned_after_failure = True
            resume_ok = k.ResumeThread(self._thread) != 0xFFFFFFFF
            if resume_ok:
                try:
                    self._delete_attributes()
                except BaseException as exc:
                    raise ProcessSetupPending("Job assignment failed; setup handles retained", self) from exc
            raise ProcessSetupPending("Job assignment failed; process containment is unverified",
                                      self)
        self._assigned = True
        if k.ResumeThread(self._thread) == 0xFFFFFFFF:
            self._owned_after_failure = True
            raise ProcessSetupPending("ResumeThread failed; suspended process handles retained", self)
        try:
            self._delete_attributes()
            _close(k, self._thread)
            self._thread = None
            _close(k, self._stdin)
            self._stdin = None
        except BaseException as exc:
            self._owned_after_failure = True
            raise ProcessSetupPending("Process started but setup handle cleanup failed", self) from exc
        if restore_error is not None:
            self._owned_after_failure = True
            raise ProcessSetupPending("Process started but inherited-handle flags were not restored", self) from restore_error

    def _delete_attributes(self):
        if self._attr_list is not None:
            _api().DeleteProcThreadAttributeList(self._attr_list)
            self._attr_list = None
            self._attr_storage = None

    def _discard_precreate(self, k):
        """Close setup resources only while CreateProcess has not returned success."""
        self._delete_attributes()
        for attr in ("_stdin", "_job"):
            handle = getattr(self, attr)
            if handle:
                _close(k, handle)
                setattr(self, attr, None)

    def poll(self):
        if self._closed:
            raise RuntimeError("process handle is closed")
        k = _api()
        result = k.WaitForSingleObject(self._process, 0)
        if result == _WAIT_TIMEOUT:
            return None
        if result != _WAIT_OBJECT_0:
            raise ctypes.WinError(ctypes.get_last_error(), "WaitForSingleObject")
        code = wintypes.DWORD()
        _check(k.GetExitCodeProcess(self._process, ctypes.byref(code)), "GetExitCodeProcess")
        self._parent_natural_exit = True
        return int(code.value)

    def wait(self):
        if self._closed:
            raise RuntimeError("process handle is closed")
        result = _api().WaitForSingleObject(self._process, 0xFFFFFFFF)
        if result != _WAIT_OBJECT_0:
            raise ctypes.WinError(ctypes.get_last_error(), "WaitForSingleObject")
        return self.poll()

    def job_accounting(self):
        """Cleanup evidence independent of optional compiler image metadata."""
        if not self._assigned or self._closed:
            raise RuntimeError("job containment is unverified or handles are closed")
        k = _api()
        acct = _JOB_ACCOUNTING()
        _check(k.QueryInformationJobObject(self._job, _JOB_BASIC_ACCOUNTING,
                                           ctypes.byref(acct), ctypes.sizeof(acct), None),
               "QueryInformationJobObject(accounting)")
        return {"active": int(acct.ActiveProcesses),
                "cpu_100ns": int(acct.TotalUserTime + acct.TotalKernelTime),
                "compilers": []}

    def _process_sample(self, k, pid):
        """Bind membership/creation before image lookup on the same query handle."""
        identity = {"pid": int(pid), "member": None, "creation_100ns": None}
        result = {"identity": identity, "compiler": None, "errors": [], "attribution_complete": True,
                  "attribution_errors": [], "containment_errors": []}
        ph = None
        stage = "OpenProcess"
        try:
            ph = k.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
            _check(ph, f"OpenProcess({pid})")
            member = wintypes.BOOL()
            stage = "IsProcessInJob"
            _check(k.IsProcessInJob(ph, self._job, ctypes.byref(member)),
                   f"IsProcessInJob({pid})")
            identity["member"] = bool(member.value)
            if not member.value:
                result["attribution_complete"] = False
                return result  # No image attribution for a reopened non-member PID.
            created, exited, user, kernel = _FILETIME(), _FILETIME(), _FILETIME(), _FILETIME()
            stage = "GetProcessTimes"
            _check(k.GetProcessTimes(ph, ctypes.byref(created), ctypes.byref(exited),
                                     ctypes.byref(kernel), ctypes.byref(user)),
                   f"GetProcessTimes({pid})")
            identity["creation_100ns"] = _ft(created)
            identity["exit_100ns"] = _ft(exited)
            needed = wintypes.DWORD(32768)
            image = ctypes.create_unicode_buffer(needed.value)
            stage = "QueryFullProcessImageNameW"
            _check(k.QueryFullProcessImageNameW(ph, 0, image, ctypes.byref(needed)),
                   f"QueryFullProcessImageNameW({pid})")
            identity["image"] = image.value
            if os.path.basename(image.value).lower() in ("rustc.exe", "rustc"):
                result["compiler"] = {"pid": int(pid), "creation_100ns": _ft(created),
                                      "image": image.value, "cpu_100ns": _ft(user) + _ft(kernel)}
        except Exception as exc:
            result["attribution_complete"] = False
            result["errors"].append(repr(exc))
            result["attribution_errors"].append({"stage": stage, "utc": utc(),
                                                  **identity, "error": repr(exc)})
        finally:
            if ph:
                try:
                    _close(k, ph)
                except BaseException as exc:
                    result["errors"].append(repr(exc))
                    result["containment_errors"].append(repr(exc))
                    # A failed CloseHandle still owns this exact opened handle.
                    # Keep its bound identity; reopening the PID cannot prove close.
                    if not hasattr(self, "_query_handles"):
                        self._query_handles = []
                    self._query_handles.append({"handle": ph, "identity": dict(identity),
                                                "close_error": repr(exc)})
        return result

    def _retry_query_handle_closes(self, k):
        errors = []
        for owned in tuple(getattr(self, "_query_handles", ())):
            try:
                _close(k, owned["handle"])
            except BaseException as exc:
                owned["close_error"] = repr(exc)
                errors.append(repr(exc))
            else:
                self._query_handles.remove(owned)
        return errors

    def _query_handle_receipt(self):
        owned = getattr(self, "_query_handles", ())
        return {"query_handles_pending": len(owned),
                "query_handle_identities": [{**entry["identity"],
                                             "close_error": entry["close_error"]}
                                            for entry in owned]}

    def sample(self):
        """Keep required containment failures separate from compiler metadata."""
        sample = self.job_accounting()
        sample.update(observation_errors=[], containment_errors=[], attribution_errors=[],
                      attribution_complete=True, process_identities=[], **self._query_handle_receipt())
        if sample["query_handles_pending"]:
            errors = self._retry_query_handle_closes(_api())
            sample["observation_errors"].extend(errors)
            sample["containment_errors"].extend(errors)
            sample.update(self._query_handle_receipt())
            if sample["query_handles_pending"]:
                sample["attribution_complete"] = False
                return sample  # Do not accumulate more query handles while held.
        if sample["active"] == 0:
            return sample
        k = _api()
        try:
            pids = _job_pids(k, self._job, sample["active"])
        except Exception as exc:
            sample["observation_errors"].append(repr(exc))
            sample["attribution_errors"].append({"stage": "job_process_ids", "utc": utc(),
                                                 "pid": None, "member": None,
                                                 "creation_100ns": None, "error": repr(exc)})
            sample["attribution_complete"] = False
            return sample
        for pid in pids:
            observed = self._process_sample(k, pid)
            sample["process_identities"].append(observed["identity"])
            if observed["compiler"] is not None:
                sample["compilers"].append(observed["compiler"])
            sample["observation_errors"].extend(observed["errors"])
            sample["containment_errors"].extend(observed["containment_errors"])
            sample["attribution_errors"].extend(observed["attribution_errors"])
            if not observed["attribution_complete"]:
                sample["attribution_complete"] = False
            sample.update(self._query_handle_receipt())
            if sample["query_handles_pending"]:
                sample["attribution_complete"] = False
                break
        if not pids:
            # Known nonzero accounting with no listed identity is incomplete
            # attribution; it does not establish why the identities are absent.
            sample["attribution_complete"] = False
        return sample

    def close(self):
        if self._closed:
            return
        if self._process:
            if self.poll() is None:
                raise RuntimeError("refusing to close a live process; allow natural exit")
        elif not self._parent_natural_exit:
            raise RuntimeError("parent natural exit is unverified")
        if not self._assigned:
            raise RuntimeError("refusing cleanup: job is uncontained or still has active processes")
        if self._job:
            if self.job_accounting()["active"] != 0:
                raise RuntimeError("refusing cleanup: job still has active processes")
            self._cleanup_zero_verified = True
        elif not self._cleanup_zero_verified:
            raise RuntimeError("Job zero was not verified before closing its handle")
        k = _api()
        errors = self._retry_query_handle_closes(k)
        if getattr(self, "_query_handles", ()):
            raise RuntimeError("query handle closure pending: " + "; ".join(errors))
        self._delete_attributes()
        for attr in ("_thread", "_process", "_job", "_stdin"):
            handle = getattr(self, attr)
            if handle:
                _close(k, handle)
                setattr(self, attr, None)
        self._closed = True


class _SECURITY_ATTRIBUTES(ctypes.Structure):
    _fields_ = [("nLength", wintypes.DWORD), ("lpSecurityDescriptor", ctypes.c_void_p),
                ("bInheritHandle", wintypes.BOOL)]


def msvcrt_handle(stream):
    import msvcrt
    return msvcrt.get_osfhandle(stream.fileno())


def inheritable_handle(stream):
    import msvcrt
    handle = msvcrt.get_osfhandle(stream.fileno())
    was_inheritable = os.get_handle_inheritable(handle)
    if not was_inheritable:
        os.set_handle_inheritable(handle, True)
    return handle, was_inheritable


def _make_handle_list(handles):
    k = _api()
    size = ctypes.c_size_t()
    k.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(size))
    if not size.value:
        raise ctypes.WinError(ctypes.get_last_error(), "InitializeProcThreadAttributeList(size)")
    storage = ctypes.create_string_buffer(size.value)
    ptr = ctypes.cast(storage, ctypes.c_void_p)
    _check(k.InitializeProcThreadAttributeList(ptr, 1, 0, ctypes.byref(size)),
           "InitializeProcThreadAttributeList")
    if not k.UpdateProcThreadAttribute(ptr, 0, _PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                                       ctypes.cast(handles, ctypes.c_void_p),
                                       ctypes.sizeof(handles), None, None):
        k.DeleteProcThreadAttributeList(ptr)
        raise ctypes.WinError(ctypes.get_last_error(), "UpdateProcThreadAttribute(handle list)")
    return ptr, storage


def _resolve_application(program, cwd, env):
    candidate = pathlib.Path(program)
    if candidate.is_absolute() or os.path.dirname(program):
        if not candidate.is_absolute() and cwd is not None:
            candidate = pathlib.Path(cwd) / candidate
        return os.path.abspath(os.fspath(candidate))
    if cwd is not None:
        local = pathlib.Path(cwd) / candidate
        if local.is_file():
            return os.path.abspath(os.fspath(local))
    resolved = shutil.which(program, path=(env or {}).get("PATH"))
    if resolved is None:
        raise FileNotFoundError(f"cannot resolve executable: {program}")
    return os.path.abspath(resolved)


def _job_pids(k, job, active):
    cap = max(1, active, 16)
    while cap <= 4096:
        size = 8 + cap * ctypes.sizeof(ctypes.c_size_t)
        buf = ctypes.create_string_buffer(size)
        if not k.QueryInformationJobObject(job, _JOB_BASIC_PROCESS_ID_LIST, buf, size, None):
            if ctypes.get_last_error() == 234:  # ERROR_MORE_DATA; retry larger, still bounded.
                cap *= 2
                continue
            raise ctypes.WinError(ctypes.get_last_error(), "QueryInformationJobObject(process IDs)")
        assigned = int.from_bytes(buf.raw[0:4], "little")
        listed = int.from_bytes(buf.raw[4:8], "little")
        if listed <= cap and listed == assigned:
            base = 8
            width = ctypes.sizeof(ctypes.c_size_t)
            return [int.from_bytes(buf.raw[base + i * width:base + (i + 1) * width], "little")
                    for i in range(listed)]
        cap = max(cap * 2, assigned, listed, 1)
    raise RuntimeError("job process list exceeded bounded 4096-PID sample")


def _close(k, handle):
    if handle and not k.CloseHandle(handle):
        raise ctypes.WinError(ctypes.get_last_error(), "CloseHandle")


class Capture:
    def __init__(self, argv, root, folder, watchdog, env, factory=None):
        folder.mkdir(exist_ok=False)
        self.folder, self.argv, self.watchdog = folder, list(argv), watchdog
        self.attribution_required = False  # prep/list/idle do not assert load.
        self.record = {"argv": self.argv, "cwd": str(root), "started_at": utc(), "start": time.monotonic(),
                       "watchdog_seconds": watchdog, "watchdog_triggered": False, "failure": None,
                       "secondary_failures": [],
                       "attribution_required": False, "attribution_complete": True,
                       "attribution_first_error": None, "attribution_secondary_errors": [],
                       "streams": {}, "samples": [], "actual_exit": None, "cleanup_complete": False}
        self.child, self.readers, writers = None, [], []
        gate = threading.Event()
        self.record_lock = threading.Lock()
        try:
            for stream in ("stdout", "stderr"):
                rfd, wfd = os.pipe()
                reader = writer = None
                try:
                    reader = os.fdopen(rfd, "rb")
                    writer = os.fdopen(wfd, "wb", buffering=0)
                except BaseException:
                    if reader is None:
                        os.close(rfd)
                    else:
                        reader.close()
                    if writer is None:
                        os.close(wfd)
                    raise
                state = {"observed_bytes": 0, "saved_bytes": 0, "sha256": None, "eof": False}
                self.record["streams"][stream] = state
                def drain(reader, raw, state, stream=stream):
                    gate.wait()
                    try:
                        for block in iter(lambda: reader.read(8192), b""):
                            state["observed_bytes"] += len(block)
                            keep = block[:max(0, MAX_STREAM - state["saved_bytes"])]
                            written = raw.write(keep)
                            if written != len(keep):
                                raise OSError("partial raw write")
                            state["saved_bytes"] += len(keep)
                            if len(keep) != len(block):
                                self.fail("raw_output_cap_exceeded")
                        state["eof"] = True
                    except BaseException as exc:
                        self.fail("stream_capture_error: " + repr(exc))
                        # Keep draining after a storage failure to avoid backpressure.
                        try:
                            for block in iter(lambda: reader.read(8192), b""):
                                state["observed_bytes"] += len(block)
                            state["eof"] = True
                        except BaseException:
                            pass
                    finally:
                        try:
                            raw.close()
                        except BaseException as exc:
                            self.fail("raw_close_error: " + repr(exc))
                        try:
                            reader.close()
                        except BaseException as exc:
                            self.fail("pipe_reader_close_error: " + repr(exc))
                        try:
                            saved_path = self.folder / (stream + ".txt")
                            state["sha256"] = digest(saved_path)
                            state["saved_bytes"] = saved_path.stat().st_size
                        except BaseException as exc:
                            self.fail("saved_stream_hash_error: " + repr(exc))

                # Before a drain worker starts, every pipe/file remains owned
                # by this stack. Never join an unstarted thread after setup fails.
                with ExitStack() as setup:
                    setup.enter_context(reader)
                    setup.enter_context(writer)
                    raw = setup.enter_context((folder / (stream + ".txt")).open("wb"))
                    thread = threading.Thread(target=drain, args=(reader, raw, state), daemon=False)
                    thread.start()
                    self.readers.append(thread)
                    writers.append(writer)
                    setup.pop_all()  # Drain owns reader/raw; finally owns writer.
            factory = factory or (WindowsChild if os.name == "nt" else PosixChild)
            resolved = shutil.which(self.argv[0]) or self.argv[0]
            self.argv[0] = str(Path(resolved).resolve())
            self.child = factory(self.argv, str(root), writers[0], writers[1], env)
            self.record["pid"] = self.child.pid
        except BaseException as exc:
            self.fail("launch_or_setup_error: " + repr(exc))
            self.child = getattr(exc, "child", self.child)
            if self.child:
                self.record["pid"] = self.child.pid
            else:
                raise
        finally:
            try:
                for writer in writers:
                    try:
                        writer.close()
                    except BaseException as exc:
                        self.fail("pipe_writer_close_error: " + repr(exc))
            finally:
                gate.set()
            if self.child is None:
                for reader in self.readers:
                    reader.join()
                save(folder / "process.json", self.record)

    def fail(self, reason):
        with self.record_lock:
            if self.record["failure"] is None:
                self.record["failure"] = reason
            elif reason != self.record["failure"] and reason not in self.record["secondary_failures"]:
                if len(self.record["secondary_failures"]) < 32:
                    self.record["secondary_failures"].append(reason)

    def sample(self):
        now = time.monotonic()
        if now - self.record["start"] > self.watchdog:
            self.record["watchdog_triggered"] = True
            self.fail("watchdog_expired_natural_exit_pending")
        try:
            sample = {**self.child.sample(), "monotonic": now, "utc": utc()}
            self.record["attribution_required"] = self.attribution_required
            # Unclassified legacy observations remain mandatory failures. Only
            # explicitly classified Windows samples may use optional metadata.
            errors = sample.get("containment_errors", sample.get("observation_errors", []))
            for error in errors:
                self.fail("process_observation_error: " + error)
            if getattr(self.child, "requires_job_cleanup", False) and sample.get("active") is None:
                self.fail("job_accounting_unknown")
            if sample.get("query_handles_pending", 0):
                self.fail("query_handle_closure_pending")
            attribution_errors = sample.get("attribution_errors", [])
            if attribution_errors or sample.get("attribution_complete") is False:
                sample["attribution_complete"] = False
                self.record["attribution_complete"] = False  # First gap stays latched.
                for error in attribution_errors or [{"stage": "attribution_incomplete",
                                                      "utc": sample["utc"], "pid": None,
                                                      "member": None, "creation_100ns": None,
                                                      "error": "compiler attribution incomplete"}]:
                    if self.record["attribution_first_error"] is None:
                        self.record["attribution_first_error"] = error
                    elif error not in self.record["attribution_secondary_errors"]:
                        if len(self.record["attribution_secondary_errors"]) < 32:
                            self.record["attribution_secondary_errors"].append(error)
                if self.attribution_required:
                    self.fail("compiler_attribution_incomplete: " + repr(self.record["attribution_first_error"]))
            # Bound metadata even if a child violates its natural-exit contract.
            if len(self.record["samples"]) < 10000:
                self.record["samples"].append(sample)
            else:
                self.fail("sample_cap_exceeded")
            return sample
        except BaseException as exc:
            self.fail("process_observation_error: " + repr(exc))
            sample = {"active": None, "observation_error": True, "attribution_complete": False,
                      "containment_errors": [repr(exc)], "compilers": [], "monotonic": now, "utc": utc()}
            self.record["attribution_complete"] = False
            if len(self.record["samples"]) < 10000:
                self.record["samples"].append(sample)
            return sample

    def _cleanup_pending(self, sample):
        if getattr(self.child, "requires_job_cleanup", False):
            return sample.get("active") != 0 or sample.get("query_handles_pending", 0) != 0
        return sample.get("active") not in (0, None)

    def finish(self):
        """No termination path: retain ownership, drain and await natural exit."""
        windows_cleanup = getattr(self.child, "requires_job_cleanup", False)
        while True:
            try:
                exited = self.child.poll() is not None
            except BaseException as exc:
                self.fail("parent_exit_observation_error: " + repr(exc))
                if not windows_cleanup:
                    raise
                exited = False
            if exited:
                try:
                    self.record["actual_exit"] = self.child.wait()
                except BaseException as exc:
                    self.fail("parent_exit_wait_error: " + repr(exc))
                    if not windows_cleanup:
                        raise
                else:
                    break
            self.sample()
            save(self.folder / "process.json", self.record)
            time.sleep(POLL)
        self.record["end"] = time.monotonic()
        self.record["exited_at"] = utc()
        if self.record["actual_exit"] != 0:
            self.fail("child_exit_nonzero")
        sample = self.sample()
        # On Windows wait for every descendant, including post-parent survivors.
        while self._cleanup_pending(sample):
            if sample.get("query_handles_pending", 0):
                self.record["handles_closed"] = False
            save(self.folder / "process.json", self.record)
            time.sleep(POLL)
            sample = self.sample()
        for thread in self.readers:
            thread.join()
        self.record["cleanup_complete"] = sample.get("active") == 0 and all(
            s["eof"] for s in self.record["streams"].values())
        if not self.record["cleanup_complete"]:
            self.fail("descendant_cleanup_unverified")
        while True:
            try:
                self.child.close()
                self.record["handles_closed"] = True
                self.record["handles_closed_at"] = utc()
                if windows_cleanup:
                    self.record["cleanup_complete"] = all(
                        s["eof"] for s in self.record["streams"].values())
                break
            except BaseException as exc:
                self.record["handles_closed"] = False
                self.record["cleanup_complete"] = False
                self.fail("handle_closure_unverified: " + repr(exc))
                if not windows_cleanup:
                    break
                # Remain the owner of any unclosed handles. No new child or
                # campaign attempt occurs while cleanup/close is pending.
                self.sample()  # Preserve watchdog/observation failures while held.
                save(self.folder / "process.json", self.record)
                time.sleep(POLL)
        save(self.folder / "process.json", self.record)
        return self.record


def launch(argv, root, folder, watchdog, env):
    return Capture(argv, root, folder, watchdog, env)


def validate_plan(plan, root):
    require(type(plan.get("version")) is int and plan["version"] == 1, "unsupported plan version")
    require(plan.get("status") == "prepared" and not plan.get("failure"), "preparation is not complete")
    source = source_identity(root)
    require(plan.get("source") == source, "source identity changed")
    require(type(plan.get("repeats")) is int and 1 <= plan["repeats"] <= 3, "repeats must be 1..3")
    require(plan.get("profile") == "release-default-features", "unexpected profile/features")
    parent_environment = environment_identity()
    require(plan.get("parent_environment") == parent_environment, "parent build environment changed")
    require(plan.get("toolchain") == toolchain_identity(), "toolchain identity changed")
    root_target = (Path(root).resolve() / "target").resolve()
    target_dir = Path(plan.get("target_dir", "")).resolve()
    preparation_dir = Path(plan.get("preparation_dir", "")).resolve()
    require(target_dir != root_target and target_dir.is_relative_to(root_target), "preparation target must be under root/target")
    require(preparation_dir != root_target and preparation_dir.is_relative_to(root_target), "preparation output must be under root/target")
    marker_root = once_marker_root(root)
    require(not target_dir.is_relative_to(marker_root) and not marker_root.is_relative_to(target_dir),
            "preparation target overlaps the persistent marker directory")
    expected_child_environment = dict(parent_environment)
    expected_child_environment["CARGO_TARGET_DIR"] = str(target_dir)
    require(plan.get("child_environment") == expected_child_environment, "preparation child environment changed")
    require(plan.get("child_environment_overrides") == {
        "CARGO_TARGET_DIR": str(target_dir), "CARGO_TERM_COLOR": "never"}, "preparation child overrides changed")
    key = preparation_once_key(source, plan["profile"], parent_environment, plan["toolchain"])
    require(plan.get("once_key") == key, "preparation once-key mismatch")
    marker_path = marker_root / ("prepare-" + key + ".json")
    require(plan.get("preparation_marker") == str(marker_path.resolve()), "preparation marker path mismatch")
    marker = json.loads(marker_path.read_text(encoding="utf-8"))
    require(marker.get("status") == "prepared" and marker.get("once_key") == key,
            "preparation marker is missing or unfinished")
    require(marker.get("identity") == preparation_identity(source, plan["profile"], parent_environment, plan["toolchain"]),
            "preparation marker identity mismatch")
    require(marker.get("origin") == {"preparation_dir": str(preparation_dir), "target_dir": str(target_dir)},
            "preparation marker origin mismatch")
    require(marker.get("plan_sha256") == canonical_sha256(plan), "prepared plan differs from its once marker")
    cases = plan.get("cases", [])
    require(1 <= len(cases) <= 4, "case count must be 1..4")
    require(len({(c["binary"], c["name"]) for c in cases}) == len(cases), "duplicate case")
    require(set(plan.get("binaries", {})) == set(SAFE_CASES), "prepared binary inventory is incomplete")
    require(len(plan.get("preparation", [])) == len(SAFE_CASES), "prepared build receipts are incomplete")
    for rec in plan["preparation"]:
        require(rec.get("actual_exit") == 0 and rec.get("cleanup_complete") is True and
                rec.get("handles_closed") is True and rec.get("failure") is None,
                "preparation build receipt is incomplete or failed")
    for target, (_, names) in SAFE_CASES.items():
        binary = plan["binaries"][target]
        listing = binary.get("listing", {})
        require(listing.get("actual_exit") == 0 and listing.get("cleanup_complete") is True and
                listing.get("handles_closed") is True and listing.get("failure") is None,
                "test-list receipt is incomplete or failed")
        listed = binary.get("listed_tests", [])
        require(isinstance(listed, list) and listed and all(isinstance(name, str) and name for name in listed) and
                len(set(listed)) == len(listed), "invalid/duplicate prepared test inventory")
        require(all(listed.count(name) == 1 for name in names), "approved case missing/duplicated in prepared inventory")
        artifact = binary.get("artifact", {})
        artifact_path = Path(artifact.get("executable", "")).resolve()
        binary_path = Path(binary.get("path", "")).resolve()
        require(artifact_path == binary_path and binary_path.is_relative_to(target_dir),
                "prepared executable is outside the isolated target")
        require(digest(binary_path) == binary.get("sha256"), "prepared executable hash mismatch")
    for case in cases:
        require(case["binary"] in SAFE_CASES and case["name"] in SAFE_CASES[case["binary"]][1], "unapproved case")
        maximum = 300 if case["binary"] == "client_session" else 90
        require(type(case["watchdog_seconds"]) is int and 10 <= case["watchdog_seconds"] <= maximum,
                "invalid test watchdog")
        binary = plan["binaries"][case["binary"]]
        require(digest(binary["path"]) == binary["sha256"], "prebuilt executable changed")
        require(binary["listed_tests"].count(case["name"]) == 1, "exact case missing/duplicated in binary")
    require(type(plan.get("build_watchdog_seconds")) is int and 30 <= plan["build_watchdog_seconds"] <= 900,
            "invalid build watchdog")
    require(type(plan.get("campaign_watchdog_seconds")) is int and 30 <= plan["campaign_watchdog_seconds"] <= 1800,
            "invalid campaign watchdog")


def prepare(root, output, head, target_dir=None):
    require_runtime_platform()
    require(git(root, "rev-parse", "HEAD") == head and re.fullmatch("[0-9a-f]{40}", head), "expected-head mismatch")
    root = Path(root).resolve()
    output = Path(output).resolve()
    target_dir = Path(target_dir if target_dir is not None else output / "prepare-target").resolve()
    before = source_identity(root)
    profile = "release-default-features"
    parent_environment = environment_identity()
    toolchain = toolchain_identity()
    key = preparation_once_key(before, profile, parent_environment, toolchain)
    root_target = root / "target"
    marker_root = once_marker_root(root)
    require(output != root_target and output.is_relative_to(root_target), "preparation output must be under root/target")
    require(target_dir != root_target and target_dir.is_relative_to(root_target), "preparation target must be under root/target")
    require(output != target_dir and not output.is_relative_to(target_dir), "preparation output overlaps the target directory")
    require(not target_dir.is_relative_to(marker_root) and not marker_root.is_relative_to(target_dir),
            "preparation target overlaps the persistent marker directory")
    require(not output.is_relative_to(marker_root) and not marker_root.is_relative_to(output),
            "preparation output overlaps the persistent marker directory")
    require(not output.exists(), "preparation output must be fresh")
    require(not target_dir.exists(), "preparation target must not preexist or contain artifacts")
    marker_path = marker_root / ("prepare-" + key + ".json")
    origin = {"preparation_dir": str(output), "target_dir": str(target_dir)}
    identity = preparation_identity(before, profile, parent_environment, toolchain)
    reservation = {"version": 1, "kind": "preparation", "status": "preparing", "once_key": key,
                   "identity": identity, "origin": origin, "created_at": utc()}
    reserve_marker(marker_path, reservation)
    child_environment = dict(parent_environment)
    child_environment["CARGO_TARGET_DIR"] = str(target_dir)
    child_overrides = {"CARGO_TARGET_DIR": str(target_dir), "CARGO_TERM_COLOR": "never"}
    env = dict(os.environ, **child_overrides)
    plan = {"version": 1, "source": before, "profile": profile, "repeats": 1,
            "build_watchdog_seconds": 900, "campaign_watchdog_seconds": 1800,
            "parent_environment": parent_environment, "child_environment": child_environment,
            "child_environment_overrides": child_overrides, "target_dir": str(target_dir),
            "preparation_dir": str(output), "preparation_marker": str(marker_path.resolve()),
            "once_key": key, "binaries": {}, "cases": [],
            "os": platform.platform(), "cpu": platform.processor(), "python": platform.python_version(),
            "toolchain": toolchain, "preparation": [], "status": "preparing"}
    failure = None
    output_created = False
    try:
        output.mkdir(parents=True, exist_ok=False)
        output_created = True
        target_dir.mkdir(parents=True, exist_ok=False)
        save(output / "prepared.json", plan)
        for target, (crate, names) in SAFE_CASES.items():
            argv = ["cargo", "test", "--release", "--locked", "--offline", "-p", crate, "--test", target,
                    "--no-run", "--message-format=json", "--target-dir", str(target_dir)]
            rec = launch(argv, root, output / ("prepare-" + target), 900, env).finish()
            plan["preparation"].append(rec)
            require(rec.get("failure") is None and rec.get("actual_exit") == 0 and
                    rec.get("cleanup_complete") is True and rec.get("handles_closed") is True,
                    "preparation failed or cleanup is unverified: " + str(rec.get("failure")))
            rows = [json.loads(line) for line in (output / ("prepare-" + target) / "stdout.txt").read_text(encoding="utf-8").splitlines() if line.startswith("{")]
            artifacts = [r for r in rows if r.get("reason") == "compiler-artifact" and r.get("target", {}).get("name") == target and r.get("executable")]
            require(len(artifacts) == 1, "ambiguous/missing compiler artifact")
            path = str(Path(artifacts[0]["executable"]).resolve())
            require(Path(path).is_relative_to(target_dir), "compiler artifact escaped isolated target")
            listing = launch([path, "--list"], root, output / ("list-" + target), 30, env).finish()
            require(listing.get("failure") is None and listing.get("actual_exit") == 0 and
                    listing.get("cleanup_complete") is True and listing.get("handles_closed") is True,
                    "binary listing failed or cleanup is unverified")
            text = (output / ("list-" + target) / "stdout.txt").read_text(encoding="utf-8")
            listed = re.findall(r"^(.+): test$", text, re.M)
            require(listed and len(set(listed)) == len(listed) and all(name for name in listed),
                    "empty or invalid prepared test inventory")
            require(all(listed.count(n) == 1 for n in names), "missing/duplicate named case")
            plan["binaries"][target] = {"path": path, "sha256": digest(path), "listed_tests": listed,
                                        "artifact": artifacts[0], "listing": listing}
            for name in names:
                plan["cases"].append({"binary": target, "name": name,
                                      "watchdog_seconds": 300 if target == "client_session" else 90})
        require(source_identity(root) == before, "source changed during preparation")
        plan["status"] = "prepared"
    except BaseException as exc:
        plan["status"], plan["failure"] = "preparation_failed", repr(exc)
        failure = exc
    finally:
        if output_created:
            try:
                save(output / "prepared.json", plan)
            except BaseException as exc:
                if failure is None:
                    failure = exc
                    plan["status"], plan["failure"] = "preparation_failed", repr(exc)
                else:
                    plan.setdefault("secondary_failures", []).append("prepared receipt save: " + repr(exc))
        final_marker = {**reservation, "status": plan["status"], "plan_sha256": canonical_sha256(plan),
                        "failure": plan.get("failure")}
        try:
            finalize_marker(marker_path, final_marker)
        except BaseException as exc:
            if failure is None:
                failure = exc
                plan["status"], plan["failure"] = "preparation_failed", repr(exc)
            else:
                plan.setdefault("secondary_failures", []).append("preparation marker finalize: " + repr(exc))
            if output_created:
                try:
                    save(output / "prepared.json", plan)
                except BaseException:
                    pass
    if failure is not None:
        raise failure
    return plan


def campaign(plan, root, output, mode="paired"):
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    report = {"version": 1, "status": "running", "mode": mode, "plan": plan, "started_at": utc(),
              "attempts": [], "build": None, "failure": None,
              "network_fetch_observation": "unmeasured; --offline disables Cargo dependency fetching only",
              "limits": ["bounded samples do not resolve the historical failure", "not a performance benchmark",
                         "compiler CPU samples are not CPU utilization", "offline build does not test network fetch contention"]}
    builder = None
    try:
        require_runtime_platform()
        validate_plan(plan, root)
        require(mode in ("paired", "idle-only"), "unsupported campaign mode")
        campaign_marker = once_marker_root(root) / ("campaign-" + plan["once_key"] + ".json")
        reserve_marker(campaign_marker, {"version": 1, "kind": "campaign", "status": "reserved",
                                         "once_key": plan["once_key"],
                                         "plan_sha256": canonical_sha256(plan), "mode": mode,
                                         "output_dir": str(Path(output).resolve()), "created_at": utc()})
        deadline = time.monotonic() + plan["campaign_watchdog_seconds"]
        for condition in (("idle",) if mode == "idle-only" else ("idle", "build")):
            if condition == "build":
                require(os.name == "nt", "compiler containment/CPU attribution currently requires Windows")
                target = output / "load-target"
                require(not target.exists(), "load target must be fresh")
                argv = ["cargo", "build", "--release", "--locked", "--offline", "-p", "orr_server", "-p", "orr_ffi",
                        "--target-dir", str(target), "--message-format=json"]
                builder = launch(argv, root, output / "build", plan["build_watchdog_seconds"], env)
                # Enable before the first sample, including the admission wait.
                builder.attribution_required = True
                builder.record["attribution_required"] = True
                # One bounded wait for observed compiler work, no build retry.
                until = time.monotonic() + 60
                while True:
                    builder.sample()
                    samples = builder.record["samples"]
                    require(builder.record["failure"] is None, "load builder observation failed")
                    if len(samples) >= 2 and compiler_overlap(samples[-2:], samples[-2]["monotonic"], samples[-1]["monotonic"]):
                        break
                    require(builder.child.poll() is None and builder.record["failure"] is None and time.monotonic() < until,
                            "no observed compiler activity before probe")
                    time.sleep(POLL)
            for repeat in range(plan["repeats"]):
                for case in plan["cases"]:
                    require(time.monotonic() < deadline, "campaign_watchdog_expired")
                    validate_plan(plan, root)
                    binary = plan["binaries"][case["binary"]]
                    argv = [binary["path"], case["name"], "--exact", "--nocapture", "--test-threads=1"]
                    if builder:
                        builder.sample()
                        require(builder.record["failure"] is None and builder.child.poll() is None,
                                "build stopped or observation failed before probe launch")
                    probe = launch(argv, root, output / f"{condition}-{repeat}-{case['binary']}-{case['name'].replace(':', '_')}", case["watchdog_seconds"], env)
                    attempt = {"condition": condition, "repeat": repeat, "case": case, "process": probe.record, "compiler_overlap": None}
                    report["attempts"].append(attempt)
                    try:
                        while probe.child.poll() is None:
                            if time.monotonic() >= deadline:
                                probe.fail("campaign_watchdog_expired_natural_exit_pending")
                            probe.sample()
                            if builder:
                                builder.sample()
                            save(output / "report.json", report)
                            time.sleep(POLL)
                    finally:
                        rec = probe.finish()
                        if builder:
                            builder.sample()
                    require(rec["failure"] is None, "probe failed: " + str(rec["failure"]))
                    attempt["summary"] = exact_result((probe.folder / "stdout.txt").read_text(encoding="utf-8"), case["name"],
                                                      len(binary["listed_tests"]) - 1)
                    require(digest(binary["path"]) == binary["sha256"], "executable changed during probe")
                    validate_plan(plan, root)
                    if builder:
                        attempt["compiler_overlap"] = compiler_overlap(builder.record["samples"], rec["start"], rec["end"])
                        require(attempt["compiler_overlap"] and builder.record["failure"] is None, "insufficient sustained compiler overlap")
                    attempt["status"] = "passed"
        report["status"] = "idle_passed" if mode == "idle-only" else "pairs_passed"
    except BaseException as exc:
        report["status"], report["failure"] = "failed_stop", repr(exc)
    finally:
        if builder:
            report["build"] = builder.finish()
            if report["build"]["failure"] and report["failure"] is None:
                report["status"], report["failure"] = "load_build_failed", report["build"]["failure"]
        report["finished_at"] = utc()
        save(output / "report.json", report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("--expected-head")
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--mode", choices=("paired", "idle-only"), default="paired")
    args = parser.parse_args()
    root, output = args.root.resolve(), args.output.resolve()
    require(output.is_relative_to(root / "target") and output != root / "target", "output must be a fresh path under own target")
    require(not output.exists(), "output already exists; preserve it")
    if args.prepare:
        require(args.expected_head and args.plan is None,
                "prepare needs --expected-head and no --plan")
        prepare(root, output, args.expected_head, args.target_dir)
    else:
        require(args.plan is not None, "run needs --plan")
        report = campaign(json.loads(args.plan.read_text(encoding="utf-8")), root, output, args.mode)
        print(json.dumps({"status": report["status"], "report": str(output / "report.json"), "failure": report["failure"]}))
        return 0 if report["failure"] is None else 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
