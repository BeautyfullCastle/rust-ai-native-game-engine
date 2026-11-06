"""Verification-only runner. Never merged into the product branch."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time

EXPECTED_HEAD = "8ffa5ebae1f9c51fa4ef5ae5b245f3b96218f3fb"
EXPECTED_TREE = "5c3f82669435ed3feab7049ebcab845c26e13a6c"
ROOT = Path(os.environ["GITHUB_WORKSPACE"]) / "product"
EVIDENCE = Path(os.environ["EVIDENCE"])
MODE = sys.argv[1]
assert MODE in ("focused", "aggregate")


def write(name, value):
    path = EVIDENCE / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def digest(path):
    return hashlib.file_digest(path.open("rb"), "sha256").hexdigest()


def snapshot(label):
    head = git("rev-parse", "HEAD").decode().strip()
    tree = git("rev-parse", "HEAD^{tree}").decode().strip()
    status = git("status", "--porcelain=v1", "--untracked-files=all").decode()
    files = []
    for raw in git("ls-tree", "-r", "-z", "HEAD").split(b"\0"):
        if not raw:
            continue
        metadata, raw_path = raw.split(b"\t", 1)
        mode, kind, blob = metadata.decode().split()
        name = raw_path.decode()
        path = ROOT / name
        assert kind == "blob", name
        data = os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes()
        actual_blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        actual_mode = "120000" if path.is_symlink() else ("100755" if path.stat().st_mode & 0o111 else "100644")
        assert actual_blob == blob and actual_mode == mode, name
        files.append({"path": name, "mode": mode, "blob": blob,
                      "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    report = {"head": head, "tree": tree, "status": status, "files": files}
    write("source-" + label + ".json", report)
    assert head == EXPECTED_HEAD and tree == EXPECTED_TREE and not status and files
    return report


def plan():
    if MODE == "aggregate":
        return [("linux-aggregate", ["bash", "tools/check-linux-local.sh"], None)]
    base = ["cargo", "test", "--locked", "-p", "orr_sample", "--no-default-features"]
    union = "project-create,project-export,tilemap"
    sprites = "project-create,project-export"
    tail = ["--", "--test-threads=1"]
    return [
        ("creator-union", base + ["--features", union, "--lib", "project_create"] + tail, (25, 0)),
        ("sprite-delivery", base + ["--features", sprites, "--test", "custom_sprite_delivery"] + tail, (8, 0)),
        ("tileset-delivery", base + ["--features", union, "--test", "custom_tileset_delivery"] + tail, (4, 2)),
        ("strict-clippy", ["cargo", "clippy", "--locked", "-p", "orr_sample", "--no-default-features",
                           "--features", sprites, "--lib", "--bin", "orr_new_arena", "--test",
                           "custom_sprite_delivery", "--", "-D", "warnings"], None),
        ("feature-off", ["cargo", "check", "--locked", "-p", "orr_sample", "--no-default-features", "--lib"], None),
    ]


def execute(label, command, counts):
    record = {"command": command, "cwd": str(ROOT), "started": now(), "exit": None,
              "status": "running", "expected_counts": counts}
    write(label + ".json", record)
    path = EVIDENCE / (label + ".log")
    child = None
    start = time.monotonic()
    timeout = 18000 if MODE == "aggregate" else 2400
    minimum_disk = None
    try:
        with path.open("xb") as out:
            child = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                while selector.get_map():
                    free = shutil.disk_usage(os.environ["CARGO_TARGET_DIR"]).free
                    minimum_disk = free if minimum_disk is None else min(free, minimum_disk)
                    if free < 2684354560:
                        raise RuntimeError("Stopped before using the 2.5 GiB disk reserve")
                    if time.monotonic() - start > timeout:
                        raise TimeoutError("Bounded command timeout")
                    for key, _ in selector.select(timeout=0.5):
                        data = os.read(key.fileobj.fileno(), 65536)
                        if data:
                            out.write(data)
                            out.flush()
                            sys.stdout.buffer.write(data)
                            sys.stdout.buffer.flush()
                        else:
                            selector.unregister(key.fileobj)
            record["exit"] = child.wait(timeout=10)
        assert record["exit"] == 0, "Command failed"
        text = path.read_text()
        if counts is not None:
            summaries = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", text, re.M)
            assert summaries == [(str(counts[0]), "0", str(counts[1]))], summaries
            cases = re.findall(r"^test (\S+) \.\.\. ok$", text, re.M)
            assert len(cases) == counts[0] and len(set(cases)) == counts[0], cases
            record["passed_names"] = cases
        if MODE == "aggregate":
            assert "Linux-local build/test baseline passed." in text
            record["advisory_rustfmt"] = re.findall(r"Advisory rustfmt exit status: (\d+)", text)
        record["status"] = "passed"
    except BaseException as exc:
        record["status"] = "failed"
        record["error"] = repr(exc)
        if child is not None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            record["exit"] = child.returncode
    finally:
        if child is not None and child.stdout is not None:
            child.stdout.close()
        record.update(finished=now(), elapsed_seconds=time.monotonic() - start,
                      minimum_disk_bytes=minimum_disk)
        if path.exists():
            record["log_sha256"] = digest(path)
        write(label + ".json", record)
    return record


def main():
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    target = Path(os.environ["CARGO_TARGET_DIR"])
    assert target.is_relative_to(Path(os.environ["RUNNER_TEMP"]))
    target.mkdir(parents=True, exist_ok=False)
    report = {"tested_head": EXPECTED_HEAD, "tested_tree": EXPECTED_TREE,
              "workflow_head": os.environ["GITHUB_SHA"], "mode": MODE,
              "command_plan": plan(), "status": "failed", "commands": [],
              "limitations": ["Linux CPU/CLI only; no hardware/GPU, native window, browser or other OS acceptance",
                              "Ignored tests are not counted as passed", "Focused checks are not the full aggregate"]}
    write("receipt.json", report)
    before = None
    try:
        before = snapshot("before")
        prohibited = [k for k, v in os.environ.items() if v and (k.startswith("CARGO_PROFILE_RELEASE_")
                      or k in ("RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "ORR_LOCAL_POLICY_RECEIPTS"))]
        assert not prohibited, prohibited
        for label, command, counts in plan():
            result = execute(label, command, counts)
            report["commands"].append(result)
            write("receipt.json", report)
            assert result["status"] == "passed", label
        report["status"] = "passed"
    except BaseException as exc:
        report["error"] = repr(exc)
    finally:
        try:
            assert snapshot("after") == before
        except BaseException as exc:
            report.update(status="failed", source_error=repr(exc))
        write("receipt.json", report)
        write("evidence-sha256.json", [{"path": str(p.relative_to(EVIDENCE)), "sha256": digest(p), "bytes": p.stat().st_size}
                                      for p in sorted(EVIDENCE.rglob("*")) if p.is_file() and p.name != "evidence-sha256.json"])
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
