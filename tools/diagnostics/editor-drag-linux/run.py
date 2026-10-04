#!/usr/bin/env python3
"""Run one bounded Linux drag measurement for each pinned source tree."""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, Callable


BASELINE = "569fa578f82c440ab783a4937d0e84e223bb868b"
CANDIDATE = "cd4959c6b9991bfc6a7a9d8aac78c14dd08cfbb2"
CANDIDATE_PARENTS = [
    BASELINE,
    "dc432c08df97a11c18ae49afccf6491c4f3b160c",
]
CANDIDATE_TREE = "ee16e09d2c529497b1f43db45815f5ca53bc4b74"
BASELINE_TREE = "682eace74381f968337c78dd19e6bb3c7df6bb9f"
MEASURE_SHA256 = "C2852E324F3E2022C3AEBCA5936378730BEF04D9AFA929BF780AC6E9AB0C5919"
MEASURE_BEFORE_SHA256 = "493E53C3927009582822F88140FB3CA31599A3C90110CD78BFD76E760074DB0F"
MEASURE_AFTER_LF_SHA256 = "5FEB30527288C38EAA4B77261DA72770F1C130BA5E95D25537A57CBB08E0EC4F"
BASELINE_EDITOR_BEFORE_SHA256 = "57848DAE1650C331118E1801F21752273B42FD93B0AEF2EC1883C04AC1E719BA"
BASELINE_EDITOR_AFTER_LF_SHA256 = "05E58756CF0D3431502A7A286277998F5F914482397DAEA1A467797874419110"
BASELINE_EDITOR_AFTER_RAW_SHA256 = "9E44C485502B71271FA1B62D08F255B2938358114DAEB6C0ED41B306E9DFDD3C"
CANDIDATE_EDITOR_BEFORE_SHA256 = BASELINE_EDITOR_BEFORE_SHA256
CANDIDATE_EDITOR_AFTER_LF_SHA256 = "80F9745C4C5B883C6B944E9973E27866B935B9C41D7529FB58FF16C5A665B6BC"
CANDIDATE_EDITOR_AFTER_RAW_SHA256 = "0E708C987438B4344F1B428D95FE60E70730CF2CEA733ACB8B9C9AEAEEEF41AB"
BASELINE_EDITOR_CRLF_LINES = [401, 402]
CANDIDATE_EDITOR_CRLF_LINES = [419, 420]
MEASURE_CRLF_LINES = [142, 143, 150, 176]
APPROVED_PATCH_SHA256 = {
    "measure": "607F92D9E7E36503A36CCA2B299CEF228C04CC019EEE4FFE3B60B0D2745263DD",
    "baseline_editor": "E4218A0A96394967FBC15C00DADCD9EFB058B9322AE16C1CD6A98CFD3A3775FC",
    "candidate_editor": "8B8D025A85DAE4632CE1EE128F562DC5C9CCB96B2D499DE034AAF9D7CD6217E4",
}
MAX_MANAGED_BYTES = 64 * 1024 * 1024
SUMMARY_RESERVE = 512 * 1024
WATCHDOG_SECONDS = 900
MAX_JSON_LINE_BYTES = 1024 * 1024
EDITOR_PATH = Path("crates/orr_editor/src/editor.rs")
MEASURE_PATH = Path("crates/orr_editor/tests/measure.rs")
ALLOWED_SOURCE_PATHS = {EDITOR_PATH.as_posix(), MEASURE_PATH.as_posix()}

def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


class EvidenceStore:
    """Budget raw child streams and source proof; list binary copies separately."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True)
        self.preexisting_files = []
        self.saved_bytes = 0
        self.discarded_bytes = 0
        self.incomplete = False
        self._lock = threading.Lock()
        for path in sorted(item for item in root.rglob("*") if item.is_file()):
            size = path.stat().st_size
            self.saved_bytes += size
            self.preexisting_files.append(
                {"path": str(path.relative_to(root)), "size_bytes": size, "sha256": sha256_file(path).upper()}
            )
        if self.saved_bytes > MAX_MANAGED_BYTES - SUMMARY_RESERVE:
            self.incomplete = True

    def write_chunk(self, stream: Any, data: bytes, *, reserve_summary: bool = True) -> int:
        with self._lock:
            remaining = MAX_MANAGED_BYTES - self.saved_bytes
            if reserve_summary:
                remaining = max(0, remaining - SUMMARY_RESERVE)
            saved = min(len(data), remaining)
            if saved:
                written = stream.write(data[:saved])
                if written is not None and written < saved:
                    saved = written
                    self.incomplete = True
                self.saved_bytes += saved
            if saved != len(data):
                self.discarded_bytes += len(data) - saved
                self.incomplete = True
            return saved

    def write_file(self, path: Path, data: bytes, *, reserve_summary: bool = True) -> bool:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("wb") as stream:
            saved = self.write_chunk(stream, data, reserve_summary=reserve_summary)
        return saved == len(data)

    def copy_file(self, source: Path, destination: Path) -> bool:
        destination.parent.mkdir(parents=True, exist_ok=True)
        # Executable copies are tracked separately from the 64 MiB managed
        # raw-stream/source-proof budget. They are exact evidence artifacts.
        with source.open("rb") as src, destination.open("wb") as dst:
            shutil.copyfileobj(src, dst, length=1024 * 1024)
        os.chmod(destination, source.stat().st_mode & 0o777)
        return destination.stat().st_size == source.stat().st_size

    def copy_managed_file(self, source: Path, destination: Path) -> bool:
        destination.parent.mkdir(parents=True, exist_ok=True)
        complete = True
        with source.open("rb") as src, destination.open("wb") as dst:
            while True:
                block = src.read(1024 * 1024)
                if not block:
                    break
                if self.write_chunk(dst, block) != len(block):
                    complete = False
        return complete

    def write_summary(self, path: Path, data: bytes) -> bool:
        return self.write_file(path, data, reserve_summary=False)

    def record_discarded(self, byte_count: int) -> None:
        with self._lock:
            self.discarded_bytes += byte_count
            self.incomplete = True


class StageRunner:
    def __init__(self, store: EvidenceStore, stage_root: Path, summary: dict[str, Any]) -> None:
        self.store = store
        self.stage_root = stage_root
        self.summary = summary
        self.stage_root.mkdir(parents=True, exist_ok=True)

    def run(
        self,
        label: str,
        argv: list[str],
        *,
        cwd: Path,
        env: dict[str, str] | None = None,
        stdout_line: Callable[[bytes], None] | None = None,
    ) -> dict[str, Any]:
        stage_dir = self.stage_root / label
        stage_dir.mkdir(parents=True, exist_ok=True)
        stdout_path = stage_dir / "stdout.raw"
        stderr_path = stage_dir / "stderr.raw"
        started_at = utc_now()
        started = time.monotonic()
        actual_argv = ["git", "-c", "core.autocrlf=false", *argv[1:]] if argv and argv[0] == "git" else argv
        record: dict[str, Any] = {
            "label": label,
            "argv": actual_argv,
            "cwd": str(cwd),
            "started_utc": started_at,
            "pid": None,
            "exit_code": None,
            "watchdog_seconds": WATCHDOG_SECONDS,
            "watchdog_observed": False,
            "natural_exit_observed": False,
            "stdout": {"path": str(stdout_path.relative_to(self.store.root)), "observed_bytes": 0, "saved_bytes": 0, "discarded_bytes": 0, "sha256_observed": None, "sha256_saved": None, "drain_complete": False, "drain_errors": []},
            "stderr": {"path": str(stderr_path.relative_to(self.store.root)), "observed_bytes": 0, "saved_bytes": 0, "discarded_bytes": 0, "sha256_observed": None, "sha256_saved": None, "drain_complete": False, "drain_errors": []},
        }
        self.summary["stages"].append(record)
        observed_lock = threading.Lock()
        parse_buffer = bytearray()
        callback_error: list[str] = []

        try:
            child = subprocess.Popen(
                actual_argv,
                cwd=cwd,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                bufsize=0,
                start_new_session=True,
            )
        except Exception as exc:  # Preserve failure metadata even if spawn fails.
            record["spawn_error"] = f"{type(exc).__name__}: {exc}"
            record["finished_utc"] = utc_now()
            record["elapsed_seconds"] = time.monotonic() - started
            return record

        record["pid"] = child.pid
        print(json.dumps({"event": "stage_started", "label": label, "pid": child.pid, "utc": started_at}), flush=True)

        def drain(name: str, source: Any, output_path: Path) -> None:
            digest_all = hashlib.sha256()
            digest_saved = hashlib.sha256()
            output = None
            parser_enabled = name == "stdout" and stdout_line is not None
            try:
                output = output_path.open("wb")
            except Exception as exc:
                record[name]["drain_errors"].append(f"raw file open: {type(exc).__name__}: {exc}")
                self.store.incomplete = True
            try:
                while True:
                    block = source.read(64 * 1024)
                    if not block:
                        break
                    digest_all.update(block)
                    with observed_lock:
                        record[name]["observed_bytes"] += len(block)
                    if parser_enabled:
                        parse_buffer.extend(block)
                        if len(parse_buffer) > MAX_JSON_LINE_BYTES and b"\n" not in parse_buffer:
                            record[name]["parser_overflow"] = True
                            callback_error.append(f"JSON parser line exceeded {MAX_JSON_LINE_BYTES} bytes")
                            parse_buffer.clear()
                            parser_enabled = False
                            self.store.incomplete = True
                        while b"\n" in parse_buffer:
                            line, _, tail = parse_buffer.partition(b"\n")
                            parse_buffer[:] = tail
                            if len(line) > MAX_JSON_LINE_BYTES:
                                record[name]["parser_overflow"] = True
                                callback_error.append(f"JSON parser line exceeded {MAX_JSON_LINE_BYTES} bytes")
                                parse_buffer.clear()
                                parser_enabled = False
                                self.store.incomplete = True
                                break
                            try:
                                stdout_line(line)
                            except Exception as exc:
                                callback_error.append(f"{type(exc).__name__}: {exc}")
                                record[name]["parser_failed"] = True
                                parse_buffer.clear()
                                parser_enabled = False
                                self.store.incomplete = True
                                break
                    saved = 0
                    if output is not None:
                        try:
                            saved = self.store.write_chunk(output, block)
                        except Exception as exc:
                            record[name]["drain_errors"].append(f"raw file write: {type(exc).__name__}: {exc}")
                            self.store.incomplete = True
                            try:
                                output.close()
                            except Exception:
                                pass
                            output = None
                    else:
                        self.store.incomplete = True
                    if output is None:
                        saved = 0
                        self.store.record_discarded(len(block))
                    if saved:
                        digest_saved.update(block[:saved])
                    with observed_lock:
                        record[name]["saved_bytes"] += saved
                        record[name]["discarded_bytes"] += len(block) - saved
                if parser_enabled and parse_buffer:
                    try:
                        stdout_line(bytes(parse_buffer))
                    except Exception as exc:
                        callback_error.append(f"{type(exc).__name__}: {exc}")
                        record[name]["parser_failed"] = True
                        self.store.incomplete = True
            except Exception as exc:
                record[name]["drain_errors"].append(f"pipe read: {type(exc).__name__}: {exc}")
                self.store.incomplete = True
            finally:
                if output is not None:
                    try:
                        output.close()
                    except Exception as exc:
                        record[name]["drain_errors"].append(f"raw file close: {type(exc).__name__}: {exc}")
                        self.store.incomplete = True
                record[name]["sha256_observed"] = digest_all.hexdigest()
                record[name]["sha256_saved"] = digest_saved.hexdigest()
                record[name]["drain_complete"] = not record[name]["drain_errors"]

        threads = [
            threading.Thread(target=drain, args=("stdout", child.stdout, stdout_path), daemon=True),
            threading.Thread(target=drain, args=("stderr", child.stderr, stderr_path), daemon=True),
        ]
        for thread in threads:
            thread.start()

        while child.poll() is None:
            if time.monotonic() - started >= WATCHDOG_SECONDS and not record["watchdog_observed"]:
                record["watchdog_observed"] = True
                record["watchdog_observed_utc"] = utc_now()
                print(json.dumps({"event": "watchdog_observed_no_kill", "label": label, "pid": child.pid, "utc": record["watchdog_observed_utc"]}), flush=True)
            time.sleep(0.2)
        record["exit_code"] = child.returncode
        record["natural_exit_observed"] = True
        record["process_exit_observed_utc"] = utc_now()
        record["elapsed_seconds"] = time.monotonic() - started
        for thread in threads:
            thread.join()
        record["finished_utc"] = utc_now()
        if callback_error:
            record["stdout_parser_errors"] = callback_error
        record["observed_after_watchdog"] = record["watchdog_observed"] and record["natural_exit_observed"]
        print(
            json.dumps(
                {
                    "event": "stage_finished",
                    "label": label,
                    "pid": child.pid,
                    "exit_code": record["exit_code"],
                    "utc": record["finished_utc"],
                    "elapsed_seconds": record["elapsed_seconds"],
                    "watchdog_observed": record["watchdog_observed"],
                    "drain_complete": all(record[name]["drain_complete"] for name in ("stdout", "stderr")),
                }
            ),
            flush=True,
        )
        return record


def source_environment() -> dict[str, Any]:
    allowed = (
        "RUNNER_OS",
        "RUNNER_ARCH",
        "ImageOS",
        "ImageVersion",
        "GITHUB_SHA",
        "GITHUB_REF",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "GITHUB_JOB",
        "RUNNER_NAME",
        "LANG",
        "LC_ALL",
        "TZ",
        "RUST_BACKTRACE",
        "CARGO_TERM_COLOR",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_JOBS",
    )
    environment = {name: os.environ[name] for name in allowed if name in os.environ}
    environment.update(
        {
            name: value
            for name, value in os.environ.items()
            if name.startswith("CARGO_PROFILE_RELEASE_")
        }
    )
    return {
        "captured_utc": utc_now(),
        "python": sys.version,
        "platform": platform.platform(),
        "uname": list(platform.uname()),
        "allowed_environment": environment,
        "cpu_model": cpu_model(),
        "cpu_count": os.cpu_count(),
    }


def cpu_model() -> str | None:
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace").splitlines():
            if line.lower().startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        return None
    return None


def fail(message: str) -> None:
    raise RuntimeError(message)


def stage_ok(record: dict[str, Any], store: EvidenceStore) -> bool:
    streams = (record.get("stdout", {}), record.get("stderr", {}))
    drains_ok = all(
        stream.get("drain_complete") is True
        and stream.get("sha256_observed") is not None
        and stream.get("observed_bytes") == stream.get("saved_bytes", 0) + stream.get("discarded_bytes", 0)
        for stream in streams
    )
    return (
        record.get("spawn_error") is None
        and record.get("exit_code") == 0
        and not record.get("watchdog_observed")
        and drains_ok
        and not record.get("stdout_parser_errors")
        and not any(stream.get("parser_overflow") or stream.get("parser_failed") for stream in streams)
        and not store.incomplete
    )


def run() -> int:
    runner_temp = Path(os.environ.get("RUNNER_TEMP", tempfile.gettempdir())).resolve()
    evidence_root = runner_temp / "editor-drag-linux-evidence"
    store = EvidenceStore(evidence_root)
    summary: dict[str, Any] = {
        "schema_version": 1,
        "purpose": "PR45 first-drag Linux diagnostic; exactly one baseline and one candidate attempt maximum",
        "started_utc": utc_now(),
        "baseline_sha": BASELINE,
        "baseline_tree": BASELINE_TREE,
        "candidate_sha": CANDIDATE,
        "candidate_tree": CANDIDATE_TREE,
        "candidate_parents": CANDIDATE_PARENTS,
        "measure_sha256_required": MEASURE_SHA256,
        "attempt_policy": "baseline once, then candidate once; stop at first failure; no automatic retries",
        "managed_evidence_budget_bytes": MAX_MANAGED_BYTES,
        "preexisting_evidence_files": store.preexisting_files,
        "watchdog_seconds_observed_only_no_kill": WATCHDOG_SECONDS,
        "binary_copy_bytes_separate_from_managed_budget": 0,
        "stages": [],
        "sources": [],
        "environment": source_environment(),
    }
    root = Path(__file__).resolve().parents[3]
    runner = StageRunner(store, evidence_root / "stages", summary)
    overall_exit = 1
    worktree_root: Path | None = None

    try:
        if store.saved_bytes >= MAX_MANAGED_BYTES - SUMMARY_RESERVE:
            fail("managed evidence budget has no room for source proof")

        patch_dir = root / "tools/diagnostics/editor-drag-linux"
        patch_paths = {
            "measure": patch_dir / "measure.patch",
            "baseline_editor": patch_dir / "editor-baseline.patch",
            "candidate_editor": patch_dir / "editor-candidate.patch",
        }
        for name, patch_path in patch_paths.items():
            if not patch_path.is_file():
                fail(f"required approved patch is missing: {patch_path}")
            summary.setdefault("patch_inputs", {})[name] = {
                "path": str(patch_path.relative_to(root)),
                "sha256": sha256_file(patch_path).upper(),
                "size_bytes": patch_path.stat().st_size,
            }
            if summary["patch_inputs"][name]["sha256"] != APPROVED_PATCH_SHA256[name]:
                fail(f"patch input differs from its approved SHA-256: {name}")

        for label, argv in (
            ("rustc-version", ["rustc", "+1.99.0", "-Vv"]),
            ("cargo-version", ["cargo", "+1.99.0", "-V"]),
        ):
            version_stage = runner.run(label, argv, cwd=root)
            if not stage_ok(version_stage, store):
                fail(f"pinned toolchain version probe failed: {label}")

        remotes = runner.run("repo-origin", ["git", "remote", "get-url", "origin"], cwd=root)
        if not stage_ok(remotes, store):
            fail("could not read source repository origin")
        origin_data = (evidence_root / remotes["stdout"]["path"]).read_bytes()
        origin = origin_data.decode("utf-8", "replace").strip()
        if not origin.startswith("https://github.com/"):
            fail("origin is not the expected public GitHub HTTPS repository")
        summary["origin"] = origin

        worktree_root = Path(tempfile.mkdtemp(prefix="orr-editor-drag-worktrees-", dir=runner_temp))
        for label, source_sha, editor_patch_key in (
            ("baseline", BASELINE, "baseline_editor"),
            ("candidate", CANDIDATE, "candidate_editor"),
        ):
            source = prepare_source(
                label=label,
                source_sha=source_sha,
                editor_patch=patch_paths[editor_patch_key],
                measure_patch=patch_paths["measure"],
                checkout_root=worktree_root,
                repository_root=root,
                runner=runner,
                store=store,
                summary=summary,
            )
            if not source["prepared"]:
                fail(f"{label} source preparation failed; stop before later source or test")
            if not save_source_proof(label, Path(source["path"]), source, store):
                fail(f"{label} source proof could not be saved completely; stop before cargo preparation")
            binary = prepare_binary(label, Path(source["path"]), runner_temp, runner, store, summary)
            source["after_prepare"] = verify_source_proof(Path(source["path"]), source)
            source["after_prepare_modified_paths"] = capture_modified_paths(label, "after-prepare", Path(source["path"]), runner, store)
            if binary is None:
                fail(f"{label} measure binary preparation failed; stop before later source or test")
            if not source["after_prepare"]["unchanged"] or not source["after_prepare_modified_paths"]["allowlist_matches"]:
                fail(f"{label} source changed during cargo preparation; stop before measurement")
            binary_sha_before = sha256_file(binary).upper()
            result = runner.run(
                f"{label}-measure-once",
                [str(binary), "drag_to_viewport_latency", "--exact", "--test-threads=1", "--nocapture"],
                cwd=source["path"],
                env={**os.environ, "RUST_BACKTRACE": "1", "CARGO_TERM_COLOR": "never"},
            )
            binary_sha_after = sha256_file(binary).upper()
            source["after_measurement"] = verify_source_proof(Path(source["path"]), source)
            source["after_measurement_modified_paths"] = capture_modified_paths(
                label, "after-measurement", Path(source["path"]), runner, store
            )
            source["attempt"] = {
                "number": 1,
                "binary_sha256_before": binary_sha_before,
                "binary_sha256_after": binary_sha_after,
                "binary_unchanged": binary_sha_before == binary_sha_after,
                "stage": result["label"],
                "exit_code": result.get("exit_code"),
                "watchdog_observed": result.get("watchdog_observed", False),
                "assertion_failed_or_test_failed": result.get("exit_code") != 0,
            }
            source["measurement"] = summarize_measurement(result, store)
            source["measurement"]["source_sha"] = source_sha
            if source["measurement"].get("raw_incomplete") or (
                result.get("exit_code") == 0 and not source["measurement"].get("observer_trace_complete")
            ):
                store.incomplete = True
            if (
                not source["after_measurement"]["unchanged"]
                or not source["after_measurement_modified_paths"]["allowlist_matches"]
                or binary_sha_before != binary_sha_after
            ):
                fail(f"{label} source or binary changed during measurement")
            if not stage_ok(result, store):
                fail(f"{label} first measurement failed or its evidence is incomplete; stop immediately")
            if store.incomplete:
                fail("managed evidence budget exhausted after measurement")

        overall_exit = 0
    except Exception as exc:
        summary["failure"] = f"{type(exc).__name__}: {exc}"
        print(f"diagnostic stopped: {summary['failure']}", file=sys.stderr, flush=True)
    finally:
        summary["finished_utc"] = utc_now()
        summary["managed_evidence_saved_bytes_before_summary"] = store.saved_bytes
        summary["managed_evidence_discarded_bytes"] = store.discarded_bytes
        summary["evidence_incomplete"] = store.incomplete
        if worktree_root is not None:
            summary["temporary_worktree_root"] = str(worktree_root)
        summary_path = evidence_root / "summary.json"
        data = (json.dumps(summary, indent=2, sort_keys=True) + "\n").encode("utf-8")
        if not store.write_summary(summary_path, data):
            overall_exit = 1
            print("diagnostic summary exceeded the reserved evidence budget", file=sys.stderr, flush=True)
        if store.incomplete:
            overall_exit = 1
        print(
            json.dumps(
                {
                    "result": "pass" if overall_exit == 0 else "fail",
                    "summary": str(summary_path),
                    "saved_bytes": store.saved_bytes,
                    "discarded_bytes": store.discarded_bytes,
                    "evidence_incomplete": store.incomplete,
                },
                sort_keys=True,
            ),
            flush=True,
        )
    return overall_exit


def prepare_source(
    *,
    label: str,
    source_sha: str,
    editor_patch: Path,
    measure_patch: Path,
    checkout_root: Path,
    repository_root: Path,
    runner: StageRunner,
    store: EvidenceStore,
    summary: dict[str, Any],
) -> dict[str, Any]:
    source_path = checkout_root / f"source-{label}"
    source_record: dict[str, Any] = {
        "label": label,
        "source_sha": source_sha,
        "path": str(source_path),
        "prepared": False,
        "patches": [],
    }
    summary["sources"].append(source_record)

    fetch = runner.run(
        f"{label}-fetch-pinned-sha",
        ["git", "fetch", "--no-tags", "--no-progress", "origin", source_sha],
        cwd=repository_root,
    )
    if not stage_ok(fetch, store):
        source_record["failure"] = "pinned SHA fetch failed"
        return source_record
    add = runner.run(f"{label}-worktree-add", ["git", "worktree", "add", "--detach", str(source_path), source_sha], cwd=repository_root)
    if not stage_ok(add, store):
        source_record["failure"] = "temporary worktree creation failed"
        return source_record
    head = runner.run(f"{label}-verify-head", ["git", "rev-parse", "HEAD"], cwd=source_path)
    if not stage_ok(head, store):
        source_record["failure"] = "could not verify checkout HEAD"
        return source_record
    actual_head = (evidence_root_file(store, head)).decode("utf-8", "replace").strip()
    source_record["actual_head"] = actual_head
    if actual_head != source_sha:
        source_record["failure"] = "checkout HEAD differs from pinned source SHA"
        return source_record
    tree = runner.run(f"{label}-verify-tree", ["git", "show", "-s", "--format=%T", "HEAD"], cwd=source_path)
    if not stage_ok(tree, store):
        source_record["failure"] = "could not verify source tree id"
        return source_record
    actual_tree = evidence_root_file(store, tree).decode("utf-8", "replace").strip()
    source_record["actual_tree"] = actual_tree
    expected_tree = BASELINE_TREE if label == "baseline" else CANDIDATE_TREE
    if actual_tree != expected_tree:
        source_record["failure"] = "source tree differs from approved pinned tree"
        return source_record
    if label == "candidate":
        parents = runner.run(f"{label}-verify-parents", ["git", "rev-list", "--parents", "-n", "1", "HEAD"], cwd=source_path)
        if not stage_ok(parents, store):
            source_record["failure"] = "could not verify candidate parents"
            return source_record
        actual_parents = evidence_root_file(store, parents).decode("utf-8", "replace").strip().split()[1:]
        source_record["actual_parents"] = actual_parents
        if actual_parents != CANDIDATE_PARENTS:
            source_record["failure"] = "candidate parent list differs from approved merge checkout"
            return source_record

    before = {
        "editor_sha256": sha256_file(source_path / EDITOR_PATH).upper(),
        "measure_sha256": sha256_file(source_path / MEASURE_PATH).upper(),
    }
    source_record["before"] = before
    expected_editor_before = BASELINE_EDITOR_BEFORE_SHA256 if label == "baseline" else CANDIDATE_EDITOR_BEFORE_SHA256
    if before["editor_sha256"] != expected_editor_before:
        source_record["failure"] = "editor.rs pre-patch SHA-256 differs from approved source proof"
        return source_record
    if before["measure_sha256"] != MEASURE_BEFORE_SHA256:
        source_record["failure"] = "measure.rs pre-patch SHA-256 differs from approved source proof"
        return source_record

    for patch_label, patch_path in (("measure", measure_patch), ("editor", editor_patch)):
        check = runner.run(
            f"{label}-{patch_label}-patch-check",
            ["git", "apply", "--check", str(patch_path)],
            cwd=source_path,
        )
        if not stage_ok(check, store):
            source_record["failure"] = f"{patch_label} patch check failed"
            return source_record
        apply = runner.run(f"{label}-{patch_label}-patch-apply", ["git", "apply", str(patch_path)], cwd=source_path)
        if not stage_ok(apply, store):
            source_record["failure"] = f"{patch_label} patch application failed"
            return source_record
        source_record["patches"].append(
            {"name": patch_label, "sha256": sha256_file(patch_path).upper(), "applied": True}
        )

    names = runner.run(f"{label}-verify-modified-paths", ["git", "diff", "--name-only"], cwd=source_path)
    if not stage_ok(names, store):
        source_record["failure"] = "could not verify modified path allowlist"
        return source_record
    modified_paths = {
        line for line in evidence_root_file(store, names).decode("utf-8", "replace").splitlines() if line
    }
    source_record["modified_paths"] = sorted(modified_paths)
    if modified_paths != ALLOWED_SOURCE_PATHS:
        source_record["failure"] = "diagnostic patches modified paths outside the two-file allowlist"
        return source_record

    after_lf = {
        "editor_sha256": sha256_file(source_path / EDITOR_PATH).upper(),
        "measure_sha256": sha256_file(source_path / MEASURE_PATH).upper(),
    }
    source_record["after_lf"] = after_lf
    expected_editor_lf = BASELINE_EDITOR_AFTER_LF_SHA256 if label == "baseline" else CANDIDATE_EDITOR_AFTER_LF_SHA256
    if after_lf["editor_sha256"] != expected_editor_lf:
        source_record["failure"] = "normalized-LF editor.rs SHA-256 differs from approved patch proof"
        return source_record
    if after_lf["measure_sha256"] != MEASURE_AFTER_LF_SHA256:
        source_record["failure"] = "normalized-LF measure.rs SHA-256 differs from approved patch proof"
        return source_record
    editor_crlf_lines = BASELINE_EDITOR_CRLF_LINES if label == "baseline" else CANDIDATE_EDITOR_CRLF_LINES
    restore_crlf_lines(source_path / EDITOR_PATH, editor_crlf_lines)
    restore_crlf_lines(source_path / MEASURE_PATH, MEASURE_CRLF_LINES)
    after = {
        "editor_sha256": sha256_file(source_path / EDITOR_PATH).upper(),
        "measure_sha256": sha256_file(source_path / MEASURE_PATH).upper(),
    }
    source_record["after"] = after
    expected_editor_raw = BASELINE_EDITOR_AFTER_RAW_SHA256 if label == "baseline" else CANDIDATE_EDITOR_AFTER_RAW_SHA256
    if after["editor_sha256"] != expected_editor_raw:
        source_record["failure"] = "restored editor.rs SHA-256 differs from approved raw patch proof"
        return source_record
    if after["measure_sha256"] != MEASURE_SHA256:
        source_record["failure"] = "restored measure.rs SHA-256 differs from approved common observer"
        return source_record
    if store.incomplete:
        source_record["failure"] = "source proof capture exceeded the managed evidence budget"
        return source_record
    source_record["prepared"] = True
    return source_record


def restore_crlf_lines(path: Path, zero_based_lines: list[int]) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    for index in zero_based_lines:
        if index >= len(lines):
            raise ValueError(f"CRLF restoration line {index} is outside {path}")
        lines[index] = lines[index].rstrip(b"\r\n") + b"\r\n"
    path.write_bytes(b"".join(lines))


def source_proof_files(source_path: Path) -> dict[str, Path]:
    return {
        "editor.rs": source_path / EDITOR_PATH,
        "measure.rs": source_path / MEASURE_PATH,
        "Cargo.lock": source_path / "Cargo.lock",
    }


def save_source_proof(label: str, source_path: Path, source: dict[str, Any], store: EvidenceStore) -> bool:
    files: dict[str, dict[str, Any]] = {}
    complete = True
    for name, file_path in source_proof_files(source_path).items():
        artifact = store.root / "source-proof" / label / name
        saved = store.copy_managed_file(file_path, artifact)
        digest = sha256_file(file_path).upper()
        files[name] = {
            "source_sha256_before_prep": digest,
            "copy_sha256": sha256_file(artifact).upper() if artifact.is_file() else None,
            "copy_size_bytes": artifact.stat().st_size if artifact.is_file() else 0,
            "source_size_bytes": file_path.stat().st_size,
            "copy_path": str(artifact.relative_to(store.root)),
            "saved_completely": saved,
        }
        if not saved or files[name]["copy_sha256"] != digest:
            complete = False
    source["proof_files"] = files
    return complete


def verify_source_proof(source_path: Path, source: dict[str, Any]) -> dict[str, Any]:
    observed: dict[str, str] = {}
    for name, file_path in source_proof_files(source_path).items():
        observed[name] = sha256_file(file_path).upper()
    expected = {
        name: proof["source_sha256_before_prep"]
        for name, proof in source.get("proof_files", {}).items()
    }
    return {
        "observed_sha256": observed,
        "expected_sha256": expected,
        "unchanged": observed == expected,
    }


def capture_modified_paths(
    label: str, phase: str, source_path: Path, runner: StageRunner, store: EvidenceStore
) -> dict[str, Any]:
    if store.incomplete:
        return {"stage": None, "allowlist_matches": False, "skipped_after_evidence_incomplete": True}
    stage = runner.run(f"{label}-verify-modified-paths-{phase}", ["git", "diff", "--name-only"], cwd=source_path)
    if not stage_ok(stage, store):
        return {"stage": stage["label"], "allowlist_matches": False, "error": "diff command failed or capture incomplete"}
    modified = sorted(
        line for line in evidence_root_file(store, stage).decode("utf-8", "replace").splitlines() if line
    )
    return {"stage": stage["label"], "paths": modified, "allowlist_matches": set(modified) == ALLOWED_SOURCE_PATHS}


def evidence_root_file(store: EvidenceStore, stage: dict[str, Any]) -> bytes:
    path = store.root / stage["stdout"]["path"]
    return path.read_bytes()


def summarize_measurement(stage: dict[str, Any], store: EvidenceStore) -> dict[str, Any]:
    stderr_path = store.root / stage["stderr"]["path"]
    stdout_path = store.root / stage["stdout"]["path"]
    stream_records = (stage["stderr"], stage["stdout"])
    complete = all(
        stream["discarded_bytes"] == 0 and stream["saved_bytes"] == stream["observed_bytes"]
        for stream in stream_records
    )
    stderr_text = stderr_path.read_bytes().decode("utf-8", "replace") if stderr_path.exists() else ""
    stdout_text = stdout_path.read_bytes().decode("utf-8", "replace") if stdout_path.exists() else ""
    text = stderr_text + "\n" + stdout_text
    moves: list[dict[str, Any]] = []
    traces: list[dict[str, Any]] = []
    report_lines: list[str] = []
    for line in text.splitlines():
        if line.startswith("ORR_BASELINE "):
            try:
                value = json.loads(line[len("ORR_BASELINE ") :])
            except json.JSONDecodeError:
                continue
            if value.get("case") == "drag_move":
                moves.append(value)
        elif line.startswith("ORR_DRAG_STEP_TRACE "):
            try:
                traces.append(json.loads(line[len("ORR_DRAG_STEP_TRACE ") :]))
            except json.JSONDecodeError:
                continue
        elif line.startswith("MEASURE drag->viewport:"):
            report_lines.append(line)

    observer_costs = [
        float(sample["observer_sample_ms"])
        for sample in traces
        if isinstance(sample.get("observer_sample_ms"), (int, float))
    ]
    first_move_samples = [sample for sample in traces if sample.get("move_index") == 1]
    timed_out = sum(1 for move in moves if move.get("timed_out"))
    wall_values = [float(move["wall_ms"]) for move in moves if isinstance(move.get("wall_ms"), (int, float))]
    frame_values = [int(move["frames"]) for move in moves if isinstance(move.get("frames"), (int, float))]
    assertion_text = "a dragged body shows within a couple of display frames"
    if stage.get("exit_code") == 0 and any("test drag_to_viewport_latency ... ok" in line for line in text.splitlines()):
        assertion_status = "passed"
    elif assertion_text in text:
        assertion_status = "failed_original_assertion"
    elif stage.get("exit_code") == 0:
        assertion_status = "test_process_passed_status_line_not_found"
    else:
        assertion_status = "not_reached_or_other_test_failure"
    move_indexes_with_step = {
        sample.get("move_index")
        for sample in traces
        if sample.get("phase") == "after_ui_step" and sample.get("move_index") is not None
    }
    observer_trace_complete = len(moves) == 40 and move_indexes_with_step == set(range(1, 41)) and len(report_lines) == 1
    return {
        "source_sha": None,
        "original_assertions": {
            "viewport_appearance": {
                "frame_cap_exclusive": 200,
                "timed_out_moves": timed_out,
                "status": "failed_original_guard" if timed_out else ("passed" if len(moves) == 40 else "not_reached"),
            },
            "latency": {"text": assertion_text, "limit_ms_exclusive": 40.0, "status": assertion_status},
        },
        "measurement_report_lines": report_lines,
        "move_record_count": len(moves),
        "first_move_record": next((move for move in moves if move.get("move_index") == 1), None),
        "steady_move_record_count": sum(1 for move in moves if move.get("group") == "steady"),
        "timed_out_move_count": timed_out,
        "maximum_frames_observed": max(frame_values) if frame_values else None,
        "maximum_wall_ms_observed": max(wall_values) if wall_values else None,
        "first_move_trace_sample_count": len(first_move_samples),
        "first_move_trace_first_sample": first_move_samples[0] if first_move_samples else None,
        "first_move_trace_last_sample": first_move_samples[-1] if first_move_samples else None,
        "trace_sample_count": len(traces),
        "observer_trace_complete": observer_trace_complete,
        "observer_sample_ms_count": len(observer_costs),
        "observer_sample_ms_max": max(observer_costs) if observer_costs else None,
        "trace_stdout_stderr_complete": complete,
        "raw_incomplete": not complete,
    }


def prepare_binary(
    label: str,
    source_path: Path,
    runner_temp: Path,
    runner: StageRunner,
    store: EvidenceStore,
    summary: dict[str, Any],
) -> Path | None:
    target_dir = runner_temp / f"editor-drag-cargo-target-{label}"
    target_dir.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "CARGO_TARGET_DIR": str(target_dir), "RUST_BACKTRACE": "1", "CARGO_TERM_COLOR": "never"}
    artifacts: list[dict[str, Any]] = []

    def collect_artifact(line: bytes) -> None:
        try:
            event = json.loads(line.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return
        if event.get("reason") != "compiler-artifact":
            return
        target = event.get("target", {})
        package_id = event.get("package_id", "")
        executable = event.get("executable")
        profile = event.get("profile", {})
        if target.get("name") == "measure" and "test" in target.get("kind", []) and profile.get("test") is True and executable:
            artifacts.append({"package_id": package_id, "target": target, "profile": profile, "executable": executable})

    prepare = runner.run(
        f"{label}-cargo-prepare-measure",
        [
            "cargo",
            "+1.99.0",
            "test",
            "--release",
            "--locked",
            "-p",
            "orr_editor",
            "--test",
            "measure",
            "--no-run",
            "--message-format=json",
        ],
        cwd=source_path,
        env=env,
        stdout_line=collect_artifact,
    )
    if not stage_ok(prepare, store):
        summary.setdefault("binary_preparations", []).append({"source": label, "stage": prepare["label"], "ok": False})
        return None
    if len(artifacts) != 1:
        summary.setdefault("binary_preparations", []).append(
            {"source": label, "stage": prepare["label"], "ok": False, "matching_cargo_artifacts": artifacts}
        )
        return None

    artifact = artifacts[0]
    executable = Path(artifact["executable"]).resolve()
    target_root = target_dir.resolve()
    package_prefix = f"path+{source_path.resolve().as_uri()}#orr_editor@"
    if target_root not in executable.parents or not executable.is_file():
        summary.setdefault("binary_preparations", []).append(
            {"source": label, "stage": prepare["label"], "ok": False, "failure": "executable path outside isolated target"}
        )
        return None
    expected_src_path = (source_path / MEASURE_PATH).resolve()
    actual_src_path = Path(artifact["target"].get("src_path", "")).resolve()
    if actual_src_path != expected_src_path:
        summary.setdefault("binary_preparations", []).append(
            {"source": label, "stage": prepare["label"], "ok": False,
             "failure": "Cargo JSON target source does not identify pinned measure.rs",
             "cargo_json_artifact": artifact, "expected_src_path": str(expected_src_path)}
        )
        return None
    if not artifact["package_id"].startswith(package_prefix):
        summary.setdefault("binary_preparations", []).append(
            {
                "source": label,
                "stage": prepare["label"],
                "ok": False,
                "failure": "Cargo JSON package ID does not identify the pinned source checkout",
                "cargo_json_artifact": artifact,
                "expected_package_prefix": package_prefix,
            }
        )
        return None
    before_hash = sha256_file(executable).upper()
    copied_binary = store.root / "binaries" / f"{label}-measure"
    copied = store.copy_file(executable, copied_binary)
    after_hash = sha256_file(copied_binary).upper() if copied_binary.is_file() else None
    binary_record = {
        "source": label,
        "ok": copied and before_hash == after_hash,
        "cargo_target_dir": str(target_dir),
        "cargo_json_artifact": artifact,
        "executable_path": str(executable),
        "executable_size_bytes": executable.stat().st_size,
        "executable_sha256_before_copy": before_hash,
        "executable_mode_before_copy": oct(executable.stat().st_mode & 0o777),
        "copy_path": str(copied_binary.relative_to(store.root)),
        "copy_sha256_after_copy": after_hash,
        "copy_mode_after_copy": oct(copied_binary.stat().st_mode & 0o777) if copied_binary.is_file() else None,
        "copy_saved_completely": copied,
    }
    summary.setdefault("binary_preparations", []).append(binary_record)
    summary["binary_copy_bytes_separate_from_managed_budget"] = (
        summary.get("binary_copy_bytes_separate_from_managed_budget", 0) + copied_binary.stat().st_size
    )
    if not copied or before_hash != after_hash or store.incomplete:
        return None
    return copied_binary


if __name__ == "__main__":
    try:
        sys.exit(run())
    except KeyboardInterrupt:
        print("diagnostic interrupted by external runner", file=sys.stderr)
        sys.exit(1)
