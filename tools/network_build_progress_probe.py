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


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE).decode().strip()


def source_identity(root):
    require(not git(root, "status", "--porcelain", "--untracked-files=no"), "tracked source tree is dirty")
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

    def __init__(self, argv, cwd, outfile, errfile, env=None):
        k = _configure_api()
        if not argv or not isinstance(argv[0], str):
            raise ValueError("argv must be a nonempty string sequence")
        app = _resolve_application(argv[0], cwd, env)
        self.pid = 0
        self._process = self._thread = self._job = self._stdin = None
        self._assigned = False
        self._closed = False
        self._attr_storage = None
        self._attr_list = None
        self._owned_after_failure = False

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
        return int(code.value)

    def wait(self):
        if self._closed:
            raise RuntimeError("process handle is closed")
        result = _api().WaitForSingleObject(self._process, 0xFFFFFFFF)
        if result != _WAIT_OBJECT_0:
            raise ctypes.WinError(ctypes.get_last_error(), "WaitForSingleObject")
        return self.poll()

    def sample(self):
        """Sample job CPU/active count and currently listed rustc process identities."""
        if not self._assigned or self._closed:
            raise RuntimeError("job containment is unverified or handles are closed")
        k = _api()
        acct = _JOB_ACCOUNTING()
        _check(k.QueryInformationJobObject(self._job, _JOB_BASIC_ACCOUNTING,
                                           ctypes.byref(acct), ctypes.sizeof(acct), None),
               "QueryInformationJobObject(accounting)")
        pids = _job_pids(k, self._job, int(acct.ActiveProcesses))
        compilers = []
        for pid in pids:
            ph = k.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
            if not ph:
                if ctypes.get_last_error() == _ERROR_INVALID_PARAMETER:
                    continue  # The job-listed process exited between the two samples.
                raise ctypes.WinError(ctypes.get_last_error(), f"OpenProcess({pid})")
            try:
                needed = wintypes.DWORD(32768)
                image = ctypes.create_unicode_buffer(needed.value)
                _check(k.QueryFullProcessImageNameW(ph, 0, image, ctypes.byref(needed)),
                       f"QueryFullProcessImageNameW({pid})")
                if os.path.basename(image.value).lower() not in ("rustc.exe", "rustc"):
                    continue
                member = wintypes.BOOL()
                _check(k.IsProcessInJob(ph, self._job, ctypes.byref(member)),
                       f"IsProcessInJob({pid})")
                if not member.value:
                    continue  # A PID may have exited and been reused after job enumeration.
                created, exited, user, kernel = _FILETIME(), _FILETIME(), _FILETIME(), _FILETIME()
                _check(k.GetProcessTimes(ph, ctypes.byref(created), ctypes.byref(exited),
                                         ctypes.byref(kernel), ctypes.byref(user)),
                       f"GetProcessTimes({pid})")
                compilers.append({"pid": int(pid), "creation_100ns": _ft(created),
                                  "image": image.value, "cpu_100ns": _ft(user) + _ft(kernel)})
            finally:
                _close(k, ph)
        return {"active": int(acct.ActiveProcesses),
                "cpu_100ns": int(acct.TotalUserTime + acct.TotalKernelTime),
                "compilers": compilers}

    def close(self):
        if self._closed:
            return
        if self.poll() is None:
            raise RuntimeError("refusing to close a live process; allow natural exit")
        if not self._assigned or self.sample()["active"] != 0:
            raise RuntimeError("refusing cleanup: job is uncontained or still has active processes")
        k = _api()
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
        self.record = {"argv": self.argv, "cwd": str(root), "started_at": utc(), "start": time.monotonic(),
                       "watchdog_seconds": watchdog, "watchdog_triggered": False, "failure": None,
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
                def drain(reader, raw, state):
                    gate.wait()
                    h = hashlib.sha256()
                    try:
                        for block in iter(lambda: reader.read(8192), b""):
                            state["observed_bytes"] += len(block)
                            keep = block[:max(0, MAX_STREAM - state["saved_bytes"])]
                            raw.write(keep)
                            h.update(keep)
                            state["saved_bytes"] += len(keep)
                            if len(keep) != len(block):
                                self.fail("raw_output_cap_exceeded")
                        state["eof"] = True
                    except BaseException as exc:
                        self.fail("stream_capture_error: " + repr(exc))
                        # Keep draining after a storage failure to avoid backpressure.
                        try:
                            for _ in iter(lambda: reader.read(8192), b""):
                                pass
                        except BaseException:
                            pass
                    finally:
                        raw.close()
                        reader.close()
                        state["sha256"] = h.hexdigest()

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
            for writer in writers:
                writer.close()
            gate.set()
            if self.child is None:
                for reader in self.readers:
                    reader.join()
                save(folder / "process.json", self.record)

    def fail(self, reason):
        with self.record_lock:
            if self.record["failure"] is None:
                self.record["failure"] = reason

    def sample(self):
        now = time.monotonic()
        if now - self.record["start"] > self.watchdog:
            self.record["watchdog_triggered"] = True
            self.fail("watchdog_expired_natural_exit_pending")
        try:
            sample = {**self.child.sample(), "monotonic": now, "utc": utc()}
            # Bound metadata even if a child violates its natural-exit contract.
            if len(self.record["samples"]) < 10000:
                self.record["samples"].append(sample)
            else:
                self.fail("sample_cap_exceeded")
            return sample
        except BaseException as exc:
            self.fail("process_observation_error: " + repr(exc))
            return {"active": None}

    def finish(self):
        """No termination path: retain ownership, drain and await natural exit."""
        while self.child.poll() is None:
            self.sample()
            save(self.folder / "process.json", self.record)
            time.sleep(POLL)
        self.record["actual_exit"] = self.child.wait()
        self.record["end"] = time.monotonic()
        self.record["exited_at"] = utc()
        sample = self.sample()
        # On Windows wait for every descendant, including post-parent survivors.
        while sample.get("active") not in (0, None):
            save(self.folder / "process.json", self.record)
            time.sleep(POLL)
            sample = self.sample()
        for thread in self.readers:
            thread.join()
        self.record["cleanup_complete"] = sample.get("active") == 0 and all(
            s["eof"] for s in self.record["streams"].values())
        if not self.record["cleanup_complete"]:
            self.fail("descendant_cleanup_unverified")
        if self.record["actual_exit"] != 0:
            self.fail("child_exit_nonzero")
        try:
            self.child.close()
            self.record["handles_closed"] = True
        except BaseException as exc:
            self.record["handles_closed"] = False
            self.fail("handle_closure_unverified: " + repr(exc))
        save(self.folder / "process.json", self.record)
        return self.record


def launch(argv, root, folder, watchdog, env):
    return Capture(argv, root, folder, watchdog, env)


def validate_plan(plan, root):
    require(type(plan.get("version")) is int and plan["version"] == 1, "unsupported plan version")
    require(plan.get("source") == source_identity(root), "source identity changed")
    require(type(plan.get("repeats")) is int and 1 <= plan["repeats"] <= 3, "repeats must be 1..3")
    require(plan.get("profile") == "release-default-features", "unexpected profile/features")
    require(plan.get("environment") == environment_identity(), "build environment changed")
    require(plan.get("toolchain") == toolchain_identity(), "toolchain identity changed")
    cases = plan.get("cases", [])
    require(1 <= len(cases) <= 4, "case count must be 1..4")
    require(len({(c["binary"], c["name"]) for c in cases}) == len(cases), "duplicate case")
    for case in cases:
        require(case["binary"] in SAFE_CASES and case["name"] in SAFE_CASES[case["binary"]][1], "unapproved case")
        require(type(case["watchdog_seconds"]) is int and 10 <= case["watchdog_seconds"] <= 300, "invalid test watchdog")
        binary = plan["binaries"][case["binary"]]
        require(digest(binary["path"]) == binary["sha256"], "prebuilt executable changed")
        require(binary["listed_tests"].count(case["name"]) == 1, "exact case missing/duplicated in binary")
    require(type(plan.get("build_watchdog_seconds")) is int and 30 <= plan["build_watchdog_seconds"] <= 1800, "invalid build watchdog")
    require(type(plan.get("campaign_watchdog_seconds")) is int and 30 <= plan["campaign_watchdog_seconds"] <= 3600, "invalid campaign watchdog")


def prepare(root, output, head):
    require_runtime_platform()
    require(git(root, "rev-parse", "HEAD") == head and re.fullmatch("[0-9a-f]{40}", head), "expected-head mismatch")
    before = source_identity(root)
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    plan = {"version": 1, "source": before, "profile": "release-default-features", "repeats": 1,
            "build_watchdog_seconds": 900, "campaign_watchdog_seconds": 1800,
            "environment": environment_identity(), "binaries": {}, "cases": [],
            "os": platform.platform(), "cpu": platform.processor(), "python": platform.python_version(),
            "toolchain": toolchain_identity(),
            "preparation": [], "status": "preparing"}
    save(output / "prepared.json", plan)
    try:
        for target, (crate, names) in SAFE_CASES.items():
            argv = ["cargo", "test", "--release", "--locked", "--offline", "-p", crate, "--test", target,
                    "--no-run", "--message-format=json"]
            rec = launch(argv, root, output / ("prepare-" + target), 900, env).finish()
            plan["preparation"].append(rec)
            require(rec["failure"] is None, "preparation failed: " + str(rec["failure"]))
            rows = [json.loads(line) for line in (output / ("prepare-" + target) / "stdout.txt").read_text(encoding="utf-8").splitlines() if line.startswith("{")]
            artifacts = [r for r in rows if r.get("reason") == "compiler-artifact" and r.get("target", {}).get("name") == target and r.get("executable")]
            require(len(artifacts) == 1, "ambiguous/missing compiler artifact")
            path = str(Path(artifacts[0]["executable"]).resolve())
            listing = launch([path, "--list"], root, output / ("list-" + target), 30, env).finish()
            require(listing["failure"] is None, "binary listing failed")
            text = (output / ("list-" + target) / "stdout.txt").read_text(encoding="utf-8")
            listed = re.findall(r"^(.+): test$", text, re.M)
            require(all(listed.count(n) == 1 for n in names), "missing/duplicate named case")
            plan["binaries"][target] = {"path": path, "sha256": digest(path), "listed_tests": listed,
                                        "artifact": artifacts[0], "listing": listing}
            for name in names:
                plan["cases"].append({"binary": target, "name": name, "watchdog_seconds": 300 if target == "client_session" else 90})
        require(source_identity(root) == before, "source changed during preparation")
        plan["status"] = "prepared"
    except BaseException as exc:
        plan["status"], plan["failure"] = "preparation_failed", repr(exc)
        raise
    finally:
        save(output / "prepared.json", plan)
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
        deadline = time.monotonic() + plan["campaign_watchdog_seconds"]
        for condition in (("idle",) if mode == "idle-only" else ("idle", "build")):
            if condition == "build":
                require(os.name == "nt", "compiler containment/CPU attribution currently requires Windows")
                target = output / "load-target"
                require(not target.exists(), "load target must be fresh")
                argv = ["cargo", "build", "--release", "--locked", "--offline", "-p", "orr_server", "-p", "orr_ffi",
                        "--target-dir", str(target), "--message-format=json"]
                builder = launch(argv, root, output / "build", plan["build_watchdog_seconds"], env)
                # One bounded wait for observed compiler work, no build retry.
                until = time.monotonic() + 60
                while True:
                    builder.sample()
                    samples = builder.record["samples"]
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
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--mode", choices=("paired", "idle-only"), default="paired")
    args = parser.parse_args()
    root, output = args.root.resolve(), args.output.resolve()
    require(output.is_relative_to(root / "target") and output != root / "target", "output must be a fresh path under own target")
    require(not output.exists(), "output already exists; preserve it")
    if args.prepare:
        require(args.expected_head and args.plan is None, "prepare needs --expected-head and no --plan")
        prepare(root, output, args.expected_head)
    else:
        require(args.plan is not None, "run needs --plan")
        report = campaign(json.loads(args.plan.read_text(encoding="utf-8")), root, output, args.mode)
        print(json.dumps({"status": report["status"], "report": str(output / "report.json"), "failure": report["failure"]}))
        return 0 if report["failure"] is None else 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
