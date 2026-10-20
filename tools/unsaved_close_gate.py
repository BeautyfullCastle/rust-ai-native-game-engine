#!/usr/bin/env python3
"""Bounded #265 CPU acceptance and production cfg checks, not GPU/native input QA.

Reuse the existing Cargo target/cache. Cargo builds the current checkout before
we pin its exact libtest artifact; freshness is not confused with execution.
No retries, unreviewed test discovery, new transport, or whole-workspace suites.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import urllib.parse

if not __debug__ or sys.flags.optimize or os.environ.get("PYTHONOPTIMIZE") not in (None, "", "0"):
    raise SystemExit("Unsaved-close acceptance requires nonoptimized Python")

# Reuse reviewed bounded process ownership, logging, and source checks unchanged.
import static_model_extract_gate as runner
import png_sequence_clips_gate as shared

PREFIX = "app::unsaved_close_tests::"
NAMES = tuple(PREFIX + name for name in (
    "actual_app_blocked_close_does_not_stall_erp_edits_or_screenshot_completion",
    "actual_app_clean_scene_close_is_not_cancelled",
    "actual_app_controlled_screenshot_exit_is_not_a_native_close_request",
    "actual_app_dirty_collect_ui_close_is_cancelled_with_clean_scene",
    "actual_app_dirty_scene_close_is_cancelled_without_losing_file_or_history",
    "actual_app_dirty_sprite_bindings_close_is_cancelled_with_clean_scene",
    "actual_app_dirty_update_ingested_during_close_frame_is_protected",
    "actual_app_existing_tilemap_close_protection_remains_without_gpu",
    "actual_app_noop_failure_and_read_refresh_do_not_invent_unsaved_scene",
    "actual_app_passive_close_preserves_focus_geometry_and_ordinary_input",
    "actual_app_play_spawn_does_not_mark_saved_authoring_scene_dirty",
    "actual_app_project_modal_early_return_cannot_bypass_close_protection",
    "actual_app_ready_collect_ui_draft_survives_passive_close_and_explicit_status_clear",
    "actual_app_remote_scene_is_not_owned_but_retained_local_sidecar_is_protected",
    "actual_app_same_frame_spawn_and_undo_to_saved_use_acknowledged_history",
    "actual_app_same_frame_widget_edit_is_seen_by_final_close_check",
    "actual_app_save_as_cancel_and_failure_do_not_clear_close_protection",
    "actual_app_scene_save_does_not_authorize_closing_dirty_sidecar_or_auto_close",
    "actual_app_transaction_commit_history_is_visible_before_close",
))
REQUESTED = "collect-ui,project-create,tilemap"
FEATURES = ["collect-dodge", "collect-ui", "project-create", "project-ui", "sprites", "tilemap"]
OPTIONAL = "collect-ui,project-create,tilemap,windows-projects,room-animated-models,terrain-physics,navigation,irradiance-probes,mesh-hazards-export,mesh-animated-models"
FIXTURE_COUNT = 12
require = shared.require


def run(label, command, seconds=1200):
    shared.source_fence()
    output = runner.run(label, command, seconds=seconds)
    shared.source_fence()
    return output


def inventory(output, expected):
    output = output.replace("\r\n", "\n")
    require(len(expected) == len(set(expected)), "duplicate expected inventory")
    runner.inventory(output, expected)
    require(not re.search(r"^.+: benchmark$", output, re.M), "unexpected benchmark")
    return re.findall(r"^([A-Za-z0-9_:]+): test$", output, re.M)


def full_inventory(output):
    names = re.findall(r"^([A-Za-z0-9_:]+): test$", output.replace("\r\n", "\n"), re.M)
    require(names and len(names) == len(set(names)), "nonempty unique library inventory required")
    inventory(output, names)
    require(set(NAMES) <= set(names), "missing required focused tests")
    return names


def result(output, total):
    require(type(total) is int and total >= len(NAMES), "invalid total inventory")
    output = output.replace("\r\n", "\n")
    filtered = total - len(NAMES)
    require(re.findall(r"; (\d+) filtered out;", output) == [str(filtered)], "wrong filtered count")
    runner.result(output.replace(f"; {filtered} filtered out;", "; 0 filtered out;"), NAMES)


def producer(output):
    rows = [shared.load_json(line) for line in output.splitlines() if line.startswith("{")]
    require(rows and rows[-1] == {"reason": "build-finished", "success": True}
            and rows[-1].get("success") is True, "successful terminal Cargo record required")
    require([row for row in rows if row.get("reason") == "build-finished"] == [rows[-1]], "one Cargo completion required")
    candidates = [row for row in rows if row.get("reason") == "compiler-artifact"
                  and row.get("target", {}).get("name") == "orr_editor" and row.get("executable")]
    require(len(candidates) == 1, "one exact editor library producer required")
    item = candidates[0]
    crate = runner.repo / "crates/orr_editor"
    target = item.get("target", {})
    require(Path(item.get("manifest_path", "")).resolve() == crate / "Cargo.toml", "wrong manifest")
    package = item.get("package_id", "")
    require(isinstance(package, str) and package.startswith("path+"), "local package required")
    uri = urllib.parse.urlsplit(package[5:].split("#", 1)[0])
    require(uri.scheme == "file" and not uri.netloc and Path(urllib.parse.unquote(uri.path).lstrip("/") if os.name == "nt" else urllib.parse.unquote(uri.path)).resolve() == crate, "wrong package identity")
    require(target.get("kind") == ["lib"] and target.get("crate_types") == ["lib"]
            and Path(target.get("src_path", "")).resolve() == crate / "src/lib.rs", "wrong library target")
    require(item.get("profile", {}).get("test") is True and item["profile"].get("opt_level") == "3", "release libtest required")
    require(sorted(item.get("features", [])) == FEATURES, "wrong feature closure")
    original = Path(item["executable"])
    require(original.is_absolute() and original == original.resolve() and not original.is_symlink()
            and original.parent == runner.target_dir / "release/deps", "wrong artifact path")
    shared.ordinary_file(original, 256 * 1024**2)
    copied = runner.evidence / "unsaved-close-libtest"
    with original.open("rb") as source, copied.open("xb") as destination:
        shutil.copyfileobj(source, destination, 1024**2)
    shutil.copymode(original, copied)
    require(runner.digest(original) == runner.digest(copied), "artifact copy differs")
    runner.report["producers"]["focused-libtest"] = {"producer": item, "file": copied.name,
        "sha256": runner.digest(copied), "bytes": copied.stat().st_size}
    runner.record()
    return str(copied)


def cpu():
    require(sys.platform == "linux" and platform.machine() == "x86_64", "all 19 cases require Linux x64")
    binary = producer(run("build-focused", ["cargo", "test", "--locked", "--release", "-p", "orr_editor",
        "--no-default-features", "--features", REQUESTED, "--lib", "--no-run", "--message-format=json"]))
    all_names = full_inventory(run("full-inventory", [binary, "--list"]))
    inventory(run("focused-inventory", [binary, PREFIX, "--list"]), NAMES)
    inventory(run("focused-ignored", [binary, PREFIX, "--ignored", "--list"]), [])
    output = run("focused-run", [binary, PREFIX, "--nocapture", "--test-threads=1", "--color=never"])
    result(output, len(all_names))
    runner.report["coverage"]["cpu"] = {"passed": len(NAMES), "failed": 0, "ignored": 0,
        "names": NAMES, "library_total": len(all_names), "filtered": len(all_names) - len(NAMES)}
    runner.record()


def boundary_commands():
    for label, features in (("no-default", None), ("optional-union", OPTIONAL)):
        common = ["--locked", "--release", "-p", "orr_editor", "--no-default-features"]
        if features:
            common += ["--features", features]
        common += ["--lib", "--bin", "orr_editor"]
        yield label + "-check", ["cargo", "check", *common]
        yield label + "-strict", ["cargo", "clippy", *common, "--", "-D", "warnings"]


def cfg():
    commands = list(boundary_commands())
    for label, command in commands:
        run(label, command)
    runner.report["coverage"]["production_cfg"] = [label for label, _ in commands]
    runner.record()


def initialize(root, destination, phase):
    require(destination.is_absolute() and destination == destination.resolve()
            and not destination.is_relative_to(root), "fresh canonical evidence path outside checkout required")
    destination.mkdir(parents=True, exist_ok=False)
    runner.repo, runner.evidence, runner.frozen = root, destination, None
    # No fresh private target: these bounded commands reuse the existing CI cache.
    runner.target_dir = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve()
    os.environ.update(CARGO_TARGET_DIR=str(runner.target_dir), CARGO_TERM_COLOR="never", PYTHONDONTWRITEBYTECODE="1")
    runner.report = {"status": "started", "phase": phase, "platform": sys.platform,
        "commands": [], "producers": {}, "coverage": {"cpu": "NOT RUN", "production_cfg": "NOT RUN"},
        "limits": "known dirty/staged documents; synthetic screenshot; no native/GPU or universal async shutdown proof"}
    runner.record()
    require(not runner.run("clean-source-before", ["git", "status", "--porcelain=v1", "--untracked-files=all"]).strip(), "dirty source")
    runner.report["head"] = runner.run("checkout-head", ["git", "rev-parse", "HEAD"]).strip()
    runner.report["tree"] = runner.run("checkout-tree", ["git", "rev-parse", "HEAD^{tree}"]).strip()
    require(re.fullmatch(r"[0-9a-f]{40}", runner.report["head"]) and re.fullmatch(r"[0-9a-f]{40}", runner.report["tree"]), "invalid source identity")
    require(os.environ.get("GITHUB_SHA", runner.report["head"]) == runner.report["head"], "wrong hosted checkout")
    paths = runner.run("tracked-source", ["git", "ls-files", "-z"]).split("\0")
    require(paths.pop() == "" and paths and len(paths) == len(set(paths)), "invalid tracked inventory")
    runner.frozen = runner.source_snapshot(paths)
    require(shared.source_paths() == list(runner.frozen), "untracked/ignored source cannot enter build")
    runner.report["source_sha256"] = runner.frozen
    runner.report["source_inventory_sha256"] = hashlib.sha256(json.dumps(runner.frozen, sort_keys=True).encode()).hexdigest()
    runner.record()


def main():
    require(len(sys.argv) == 2 and sys.argv[1] in {"cpu", "cfg"}, "usage: unsaved_close_gate.py cpu|cfg")
    phase = sys.argv[1]
    error = None
    try:
        initialize(Path(__file__).resolve().parents[1], Path(os.environ["ORR_UNSAVED_CLOSE_EVIDENCE"]), phase)
        checked = run("checker-fixtures", [sys.executable, "-B", str(runner.repo / "tools/test_unsaved_close_gate.py")])
        require(checked.rstrip().endswith(f"UNSAVED_CLOSE_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped"), "fixture census mismatch")
        {"cpu": cpu, "cfg": cfg}[phase]()
        require(runner.report["coverage"]["cpu" if phase == "cpu" else "production_cfg"] != "NOT RUN", "incomplete phase")
        shared.source_fence()
    except BaseException as failure:
        error = failure
    finally:
        if runner.evidence is not None and runner.evidence.is_dir():
            runner.finish(error)
        elif error is not None:
            raise error


if __name__ == "__main__":
    main()
