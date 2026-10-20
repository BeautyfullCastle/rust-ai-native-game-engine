#!/usr/bin/env python3
"""Mandatory #279 CPU/native acceptance. No cached/native/ignored-list success.

Native artifacts are retained under ORR_ARENA_EXPORT_GATE_EVIDENCE; the actual
libtest writes ORR_ARENA_EXPORT_EVIDENCE_DIR. CPU never certifies native output.
Legacy census increases are structural proof, not fabricated old execution.
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

import static_model_extract_gate as runner
import static_model_retexture_gate as fence
import unsaved_close_gate as close
import editor_export_guide_gate as guide

APP = ('app::arena_export::tests::result_does_not_release_ownership_until_terminal_and_joined', 'app::arena_export::tests::hidden_tab_close_and_clear_status_do_not_drop_worker', 'app::arena_export::tests::outer_early_return_poll_and_normal_teardown_join', 'app::arena_export::tests::panic_after_result_requires_destination_inspection', 'app::arena_export::tests::screenshot_mode_refuses_without_source_mutation', 'app::arena_export::tests::actual_tab_is_explicit_and_guidance_does_not_dispatch', 'app::arena_export::tests::terminal_disconnected_without_result_is_uncertain_and_joined', 'app::arena_export::tests::owned_job_destructor_waits_for_delayed_worker_release', 'app::arena_export::tests::screenshot_builder_refuses_active_job_and_preserves_inactive_behavior', 'app::arena_export::tests::spawn_failure_is_terminal_without_running_worker_or_publishing', 'app::arena_export::tests::path_dialog_refuses_and_completion_retains_pre_reload_identity', 'app::arena_export::tests::same_frame_owned_worker_root_close_vetoes', 'app::arena_export::tests::actual_widget_runtime_hash_changes_clear_trust_and_inspection_does_not_retry', 'app::arena_export::tests::active_arena_job_refuses_mesh_whole_app_replacement', 'app::arena_export::tests::remote_capture_and_encoder_refuse_without_cancellation', 'app::arena_export::tests::loaded_ui_is_refused_even_without_manifest_declaration', 'app::arena_export::tests::loaded_tilemap_is_refused_independently_of_saved_manifest', 'app::arena_export::tests::actual_escape_does_not_cancel_worker_or_bypass_next_close', 'app::arena_export::tests::active_arena_job_refuses_mesh_creation_replacement')
EDITOR = ('editor::arena_export::tests::saved_source_inventory_and_equal_contents_reload_epoch', 'editor::arena_export::tests::cached_dirty_uncertain_and_transaction_flags_refused', 'editor::arena_export::tests::declared_ui_tilemap_unloaded_bindings_and_unrelated_loaded_bindings_refused', 'editor::arena_export::tests::raw_scene_changes_are_pinned_and_semantic_changes_refused', 'editor::arena_export::tests::bounded_reader_refuses_symlinks_and_over_limit_files', 'editor::arena_export::tests::referenced_cached_pixels_documents_and_missing_cache_are_checked', 'editor::arena_export::tests::stale_cached_clean_state_does_not_hide_host_transaction_proposal_or_play', 'editor::arena_export::tests::actual_managed_control_pending_gesture_and_listener_are_preserved_and_refused', 'editor::arena_export::tests::missing_frame_stale_checksum_and_clean_saved_unloaded_sprites_fail_closed')
SAMPLE = ('project_export::editor_arena::tests::constructors_bound_hash_bytes_paths_count_and_presence', 'project_export::editor_arena::tests::constructors_reject_duplicates_collisions_and_total_overflow_bound', 'project_export::editor_arena::tests::exact_complete_inventory_exports_with_and_without_sprites_or_lock', 'project_export::editor_arena::tests::canonical_scene_and_initial_checksum_are_independent_requirements', 'project_export::editor_arena::tests::unchanged_checksum_does_not_hide_different_raw_scene_bytes', 'project_export::editor_arena::tests::raw_manifest_lock_and_sprite_changes_reject_before_runtime_or_stage', 'project_export::editor_arena::tests::missing_extra_and_wrong_presence_inventory_reject_before_runtime', 'project_export::editor_arena::tests::lock_appearance_and_disappearance_are_not_equal_to_empty_lock', 'project_export::editor_arena::tests::byte_lengths_are_compared_as_well_as_digests', 'project_export::editor_arena::tests::expected_source_is_revalidated_at_transaction_boundary', 'project_export::editor_arena::tests::source_shape_rejects_other_games_and_all_unsupported_entries', 'project_export::editor_arena::tests::declared_unsupported_shapes_fail_before_runtime_or_staging', 'project_export::editor_arena::tests::source_pin_preserves_trust_hash_and_elf_requirements', 'project_export::editor_arena::tests::same_snapshot_stages_pinned_bytes_then_final_recheck_rejects_changes', 'project_export::editor_arena::tests::exact_source_retains_transaction_failpoints_and_previous_output', 'project_export::editor_arena::tests::exact_source_does_not_bypass_whole_active_lock_asset_validation')

EXTENDED_ONLY = tuple("app::arena_export::tests::" + n for n in (
    "active_arena_job_refuses_mesh_whole_app_replacement",
    "loaded_ui_is_refused_even_without_manifest_declaration",
    "loaded_tilemap_is_refused_independently_of_saved_manifest",
    "active_arena_job_refuses_mesh_creation_replacement"))
NEW_APP = tuple("app::arena_export::tests::" + n for n in ("actual_package_verification_worker_blocks_arena_export_without_cancellation", "active_arena_job_retains_app_through_disabled_collect_replacement_menu"))
BASELINE = tuple(n for n in APP + EDITOR if n not in EXTENDED_ONLY)
NATIVE = "actual_widget_generated_edited_project_relocates_and_runs_with_sprites"
STAGES = ("source-move-fire", "isolated-proof", "relocated-fire-1", "relocated-fire-2", "relocated-move", "capture-initial", "capture-moved")
LEGACY_BASE = "20ff934"
# Existing mandatory counts, before precisely sixteen nonignored additions.
LEGACY_COUNTS = {"export-unit":25, "authored-ui":73, "player-settings":111,
 "new-arena":64, "collect-authored":68, "progress-disabled":68,
 "progress":94, "progress-joint":175, "progress-pinned":96,
 "collect-creator":113, "collect-ui":162, "collect-ui-creator":183}
FIXTURE_COUNT = 12


def run(label, command, seconds=1200):
    fence.source_fence()
    runner.fence()
    output = runner.run(label, command, seconds=seconds)
    runner.fence()
    fence.source_fence()
    return output


def fresh_target(label):
    runner.target_dir = runner.new_target("cpu")
    os.environ["CARGO_TARGET_DIR"] = str(runner.target_dir)
    runner.report.setdefault("fresh_targets", {})[label] = str(runner.target_dir)
    runner.record()


def cargo_rows(output):
    rows = [json.loads(line) for line in output.splitlines() if line.startswith("{")]
    assert rows and rows[-1] == {"reason":"build-finished", "success":True}, "terminal Cargo success required"
    assert [r for r in rows if r.get("reason") == "build-finished"] == [rows[-1]]
    return rows


def producer(output, label, package, target_name, kind, features):
    rows = cargo_rows(output)
    found = [r for r in rows if r.get("reason") == "compiler-artifact"
             and r.get("target", {}).get("name") == target_name and r.get("executable")]
    assert len(found) == 1 and found[0].get("fresh") is False, "one fresh exact producer required"
    if kind != "lib":
        return runner.pin(output, label, package, target_name, kind, features)
    item = found[0]; target = item["target"]
    crate = runner.repo / "crates" / package
    assert Path(item["manifest_path"]).resolve() == crate / "Cargo.toml"
    assert item["package_id"].startswith("path+")
    uri = urllib.parse.urlsplit(item["package_id"][5:].split("#", 1)[0])
    path = urllib.parse.unquote(uri.path)
    assert uri.scheme == "file" and not uri.netloc
    assert Path(path.lstrip("/") if os.name == "nt" else path).resolve() == crate
    assert target["kind"] == ["lib"] and target["crate_types"] == ["lib"]
    assert Path(target["src_path"]).resolve() == crate / "src/lib.rs"
    assert item["profile"]["test"] is True and item["profile"]["opt_level"] == "3"
    assert sorted(item["features"]) == sorted(features), item["features"]
    original = Path(item["executable"])
    assert original.is_absolute() and original == original.resolve() and not original.is_symlink()
    assert original.parent == runner.target_dir / "release/deps"
    assert 0 < original.stat().st_size <= 256 * 1024**2
    copied = runner.evidence / (label + (".exe" if os.name == "nt" else ""))
    with original.open("rb") as src, copied.open("xb") as dst:
        shutil.copyfileobj(src, dst, 1024**2)
    shutil.copymode(original, copied)
    assert runner.digest(original) == runner.digest(copied)
    runner.report["producers"][label] = {"producer":item, "file":copied.name,
        "sha256":runner.digest(copied), "bytes":copied.stat().st_size}
    runner.record()
    return str(copied)


def inventory(output):
    names = re.findall(r"^([A-Za-z0-9_:]+): test$", output.replace("\r\n", "\n"), re.M)
    assert len(names) == len(set(names))
    close.inventory(output, names)
    return names


def selected(label, binary, names, total):
    # One exact run per case prevents a broad substring from hiding extra cases.
    for number, name in enumerate(names):
        close.inventory(run(f"{label}-{number}-list", [binary, name, "--exact", "--list"]), [name])
        close.inventory(run(f"{label}-{number}-ignored", [binary, name, "--exact", "--ignored", "--list"]), [])
        guide.result(run(f"{label}-{number}-run", [binary, name, "--exact", "--test-threads=1", "--nocapture", "--color=never"]), [name], total)
    runner.report["coverage"][label] = {"names":names, "passed":len(names), "failed":0, "ignored":0}
    runner.record()


def library(label, package, requested, features, names, supported=True):
    fresh_target(label)
    command = ["cargo", "test", "--locked", "--release", "-p", package, "--no-default-features"]
    if requested:
        command += ["--features", requested]
    command += ["--lib", "--no-run", "--message-format=json"]
    binary = producer(run(label + "-build", command), label, package, package, "lib", features)
    all_names = inventory(run(label + "-full-list", [binary, "--list"]))
    ignored = inventory(run(label + "-full-ignored", [binary, "--ignored", "--list"]))
    arena = [n for n in all_names if n.startswith(("app::arena_export::tests::", "editor::arena_export::tests::", "project_export::editor_arena::tests::"))]
    assert sorted(arena) == sorted(names) and not set(names) & set(ignored), "exact Arena inventory"
    selected(label, binary, names if names else guide.DEFAULT, len(all_names))
    return binary, all_names


def cpu():
    supported = sys.platform == "linux" and platform.machine() == "x86_64"
    library("default", "orr_editor", "", [], [])
    library("sprites", "orr_editor", "sprites", ["sprites"], [])
    baseline = "arena-project-export,project-create"
    library("baseline", "orr_editor", baseline, ["arena-project-export", "project-create", "sprites"], BASELINE if supported else [])
    if supported:
        library("extended", "orr_editor", baseline + ",project-ui,tilemap,mesh-mission,package-manager,collect-ui",
            ["arena-project-export", "project-create", "project-ui", "tilemap", "mesh-mission", "mesh-navigation", "models", "sprites", "package-manager", "room-project", "collect-ui", "collect-dodge"], APP + EDITOR + NEW_APP)
        library("sample", "orr_sample", "project-export", ["project-export", "project", "sprites"], SAMPLE)
        binary, names = library("close", "orr_editor", baseline + ",collect-ui,tilemap",
            ["arena-project-export", "project-create", "collect-ui", "collect-dodge", "project-ui", "sprites", "tilemap"],
            BASELINE + tuple(n for n in EXTENDED_ONLY if "loaded_" in n) + (NEW_APP[1],))
        close.inventory(run("close-inventory", [binary, close.PREFIX, "--list"]), close.NAMES)
        selected("close-interaction", binary, close.NAMES, len(names))
    else:
        fresh_target("unsupported-integration")
        binary = producer(run("unsupported-build", ["cargo", "test", "--locked", "--release", "-p", "orr_editor", "--no-default-features", "--features", baseline, "--test", "arena_editor_export", "--no-run", "--message-format=json"]),
            "unsupported-integration", "orr_editor", "arena_editor_export", "test", ["arena-project-export", "project-create", "sprites"])
        close.inventory(run("unsupported-list", [binary, "--list"]), [])
        close.inventory(run("unsupported-ignored", [binary, "--ignored", "--list"]), [])
    run("strict-default", ["cargo", "clippy", "--locked", "-p", "orr_editor", "--no-default-features", "--all-targets", "--", "-D", "warnings"])
    run("strict-enabled", ["cargo", "clippy", "--locked", "-p", "orr_editor", "-p", "orr_sample", "--no-default-features", "--features", "orr_editor/arena-project-export,orr_editor/project-create,orr_editor/project-ui,orr_editor/tilemap,orr_editor/mesh-mission,orr_editor/package-manager,orr_editor/collect-ui", "--all-targets", "--", "-D", "warnings"])


def files(root):
    assert root.is_dir() and not root.is_symlink()
    result = []
    for directory, folders, names in os.walk(root, followlinks=False):
        for name in folders + names:
            assert not (Path(directory) / name).is_symlink(), "symlink evidence"
        for name in names:
            path = Path(directory) / name
            assert path.is_file()
            result.append({"path":path.relative_to(root).as_posix(), "bytes":path.stat().st_size, "sha256":runner.digest(path)})
    return sorted(result, key=lambda item: item["path"])


def read_json(path):
    assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 8 * 1024**2
    def unique(pairs):
        result = {}
        for k, v in pairs:
            assert k not in result, "duplicate JSON key"
            result[k] = v
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def verify_receipt(root):
    receipt = read_json(root / "receipt.json")
    assert receipt["schema"] == 1
    assert receipt["source_head"] == runner.report["head"] and receipt["source_tree"] == runner.report["tree"]
    for key, label in (("runtime", "runtime"), ("test_producer", "native-harness")):
        pinned = runner.report["producers"][label]
        assert receipt[key] == {"path":str(runner.evidence / pinned["file"]), "sha256":pinned["sha256"]}
    for key in ("terminal_joined_success", "source_hidden", "bundle_read_only", "source_preserved"):
        assert receipt[key] is True, key
    assert receipt["source_files_before"] == receipt["source_files_after"] == receipt["source_files"]
    assert sorted(receipt["source_files"], key=lambda f:f["path"]) == files(root / "project")
    assert sorted(receipt["bundle_files"], key=lambda f:f["path"]) == files(root / "bundle")
    assert any("license" in f["path"].lower() for f in receipt["bundle_files"])
    manifest = read_json(root / "bundle/orr.export.json")
    payload = manifest["payload"]["files"]
    actual = {f["path"]:f for f in receipt["bundle_files"]}
    assert payload and len({f["path"] for f in payload}) == len(payload)
    for entry in payload:
        assert entry["path"] in actual
        assert entry["sha256"] == actual[entry["path"]]["sha256"]
        assert entry["bytes"] == actual[entry["path"]]["bytes"]
    assert actual["bin/arena"]["sha256"] == runner.report["producers"]["runtime"]["sha256"]
    assert manifest["payload"]["runtime"]["sha256"] == actual["bin/arena"]["sha256"]
    assert manifest["payload"]["runtime"]["bytes"] == actual["bin/arena"]["bytes"]
    assert manifest["payload"]["entry"]["game"] == "arena"
    assert manifest["payload"]["profile"] == "arena-authored-linux-x86_64-v1"
    assert manifest["schema"] == 1
    canonical = json.dumps(manifest["payload"], ensure_ascii=False, separators=(",", ":")).encode()
    assert manifest["content_digest"] == hashlib.sha256(b"orrery.arena.export.content.v1\0" + canonical).hexdigest()
    assert set(actual) == {entry["path"] for entry in payload} | {"orr.export.json"}
    assert read_json(root / "source-files-before.json") == receipt["source_files_before"]
    pre = read_json(root / "producer-receipt.json")
    assert all(pre[key] == receipt[key] for key in ("source_head", "source_tree", "runtime", "test_producer"))
    stages = read_json(root / "stage-outcomes.json")
    assert stages["schema"] == 1 and stages["stage"] == "complete" and stages["terminal_joined_success"] is True
    assert stages["source_head"] == receipt["source_head"] and stages["source_tree"] == receipt["source_tree"]
    log = root / "commands.jsonl"
    assert log.is_file() and not log.is_symlink() and log.stat().st_size <= 8 * 1024**2
    commands = [json.loads(line) for line in log.read_text().splitlines()]
    assert [c["stage"] for c in commands] == list(STAGES) == receipt["commands"]
    for cmd in commands:
        assert type(cmd["status"]) is int and cmd["status"] == 0
        assert isinstance(cmd["program"], str) and cmd["program"] and isinstance(cmd["args"], list) and cmd["cwd"]
        for stream in ("stdout", "stderr"):
            assert cmd[stream + "_file"] == cmd["stage"] + "." + stream + ".txt"
            path = root / cmd[stream + "_file"]
            assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 8 * 1024**2
    first = commands[0]
    assert first["program"] == receipt["runtime"]["path"]
    assert first["args"] == ["--project", str(root / "project"), "--headless", "--ticks", "3", "--hold", "right,fire"]
    assert all(cmd["cwd"] == str(root) for cmd in commands)
    for cmd in commands[1:]:
        assert cmd["program"] == "bwrap"
        args = cmd["args"]
        assert args[:3] == ["--ro-bind", "/", "/"]
        for flag, path in (("--ro-bind", str(root / "bundle")), ("--tmpfs", str(runner.repo)), ("--tmpfs", str(root / "project")), ("--tmpfs", str(Path(receipt["runtime"]["path"]).parent))):
            assert any(args[i:i+2] == [flag, path] for i in range(len(args)-1)), (flag, path)
    proof = (root / "isolated-proof.stdout.txt").read_text()
    assert "source-hidden=true" in proof and "write-denied=true" in proof
    captures = receipt["captures"]
    assert [c["path"] for c in captures] == ["initial.png", "moved.png"]
    pixels = []
    for capture, stage in zip(captures, ("capture-initial", "capture-moved")):
        path = root / capture["path"]
        assert not path.is_symlink() and capture["width"] == capture["height"] == 512
        assert runner.digest(path) == capture["sha256"]
        pixels.append(fence.png_pixels(path, 512, 512))
        stdout = (root / (stage + ".stdout.txt")).read_text()
        adapter = [line for line in stdout.splitlines() if "project capture adapter:" in line]
        assert len(adapter) == 1 and re.search(r"llvmpipe|lavapipe", adapter[0], re.I)
        assert re.search(r"software\s*:\s*true", adapter[0], re.I) and not re.search("swiftshader", stdout, re.I)
    assert pixels[0] != pixels[1], "movement capture must differ"
    runner.report["acceptance_receipt"] = receipt
    runner.record()


def native():
    assert sys.platform == "linux" and platform.machine() == "x86_64" and os.geteuid() != 0
    run("bwrap-preflight", ["bwrap", "--ro-bind", "/", "/", "--", "/bin/true"])
    runner.software()
    fresh_target("runtime")
    runtime = producer(run("runtime-build", ["cargo", "build", "--locked", "--release", "-p", "orr_sample", "--no-default-features", "--features", "project", "--bin", "arena", "--message-format=json"]), "runtime", "orr_sample", "arena", "bin", ["project", "sprites"])
    directory = runner.evidence / "runtime-files"; directory.mkdir()
    path = directory / "arena"; Path(runtime).rename(path)
    runner.report["producers"]["runtime"]["file"] = "runtime-files/arena"
    runner.record()
    fresh_target("native-harness")
    binary = producer(run("harness-build", ["cargo", "test", "--locked", "--release", "-p", "orr_editor", "--no-default-features", "--features", "arena-project-export,project-create", "--test", "arena_editor_export", "--no-run", "--message-format=json"]), "native-harness", "orr_editor", "arena_editor_export", "test", ["arena-project-export", "project-create", "sprites"])
    close.inventory(run("native-list", [binary, "--list"]), [NATIVE])
    close.inventory(run("native-ignored", [binary, "--ignored", "--list"]), [NATIVE])
    acceptance = runner.evidence / "acceptance"
    assert not acceptance.exists()
    os.environ.update(ORR_ARENA_EXPORT_RUNTIME=str(path), ORR_ARENA_EXPORT_EVIDENCE_DIR=str(acceptance))
    runner.result(run("native-run", [binary, NATIVE, "--ignored", "--exact", "--test-threads=1", "--nocapture", "--color=never"]), [NATIVE])
    verify_receipt(acceptance)
    runner.report["coverage"]["native"] = {"passed":1, "failed":0, "ignored":0, "names":[NATIVE]}
    runner.record()



def census(path, old_count, ignored_path=None, old_path=None, old_ignored_path=None):
    """Runtime inventory proof, retaining old full sets only when supplied."""
    names = inventory(Path(path).read_text())
    additions = sorted(n for n in names if n.startswith("project_export::editor_arena::tests::"))
    assert additions == sorted(SAMPLE) and len(names) == old_count + len(SAMPLE)
    if ignored_path is not None:
        ignored = inventory(Path(ignored_path).read_text())
        assert not set(SAMPLE) & set(ignored)
    if old_path is not None:
        old = inventory(Path(old_path).read_text())
        assert len(old) == old_count and set(old) <= set(names)
        assert set(names) - set(old) == set(SAMPLE)
        assert ignored_path is not None and old_ignored_path is not None
        assert inventory(Path(old_ignored_path).read_text()) == inventory(Path(ignored_path).read_text())


def structural_proof():
    """Source/cfg proof, explicitly NOT an executed old/new inventory claim."""
    text = (runner.repo / "crates/orr_sample/src/project_export.rs").read_text()
    marker = "#[cfg(test)]\nmod tests {"
    assert hashlib.sha256(text[text.index(marker):].encode()).hexdigest() == "4ad19d9dd79bac4ce4c339f25ec13cf7e4d292e595bb2fefe89b0fdd6fbe7824"
    source = (runner.repo / "crates/orr_sample/src/project_export/editor_arena.rs").read_text()
    suffix = source[source.index(marker):]
    assert "#[ignore" not in suffix and "#[cfg" not in suffix[len("#[cfg(test)]"):]
    assert sorted(re.findall(r"#\[test\]\s*fn (\w+)", suffix)) == sorted(n.split("::")[-1] for n in SAMPLE)
    library = (runner.repo / "crates/orr_sample/src/lib.rs").read_text()
    assert '#[cfg(all(feature = "project-export", target_os = "linux", target_arch = "x86_64"))]\npub mod project_export;' in library
    return {"kind":"structural-source-and-cfg-only", "baseline":LEGACY_BASE,
        "old_export_tests_sha256":"4ad19d9dd79bac4ce4c339f25ec13cf7e4d292e595bb2fefe89b0fdd6fbe7824", "new_names":SAMPLE,
        "legacy_counts":LEGACY_COUNTS, "addition_count":16,
        "executed_old_new_inventory_proof":False,
        "qualification":"All old sample files unchanged except exporter implementation; old test module bytes unchanged. Hosted exact inventories and original runs remain mandatory."}


def main():
    if len(sys.argv) in (4, 5, 7) and sys.argv[1] == "census":
        census(sys.argv[2], int(sys.argv[3]), *sys.argv[4:])
        return
    assert len(sys.argv) == 2 and sys.argv[1] in {"cpu", "native"}
    error = None
    try:
        root = Path(__file__).resolve().parents[1]
        destination = Path(os.environ["ORR_ARENA_EXPORT_GATE_EVIDENCE"])
        assert destination.is_absolute() and destination == destination.resolve() and not destination.is_relative_to(root)
        assert os.environ.get("PYTHONDONTWRITEBYTECODE") == "1"
        runner.initialize(root, destination, "cpu")
        fence.source_root = root
        fence.frozen = None
        assert not run("clean-source-before", ["git", "status", "--porcelain=v1", "--untracked-files=all"]).strip()
        runner.report["head"] = run("checkout-head", ["git", "rev-parse", "HEAD"]).strip()
        runner.report["tree"] = run("checkout-tree", ["git", "rev-parse", "HEAD^{tree}"]).strip()
        assert os.environ.get("GITHUB_SHA", runner.report["head"]) == runner.report["head"]
        paths = run("tracked-source", ["git", "ls-files", "-z"]).split("\0")
        assert paths.pop() == "" and paths
        fence.check_source_inventory(paths, "before")
        runner.frozen = runner.source_snapshot(paths)
        fence.frozen = fence.snapshot()
        runner.report.update(source_sha256=runner.frozen, coverage={}, mode=sys.argv[1], legacy_census_proof=structural_proof())
        runner.record()
        output = run("independent-fixtures", [sys.executable, "-B", str(root / "tools/test_arena_editor_export_gate.py")])
        assert output.rstrip().endswith(f"ARENA_EXPORT_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped")
        {"cpu":cpu, "native":native}[sys.argv[1]]()
        fence.source_fence()
    except BaseException as failure:
        error = failure
    finally:
        if runner.evidence is not None and runner.evidence.is_dir():
            runner.finish(error)
        elif error is not None:
            raise error


if __name__ == "__main__":
    main()
