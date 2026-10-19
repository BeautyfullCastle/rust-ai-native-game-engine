#!/usr/bin/env python3
"""Fail-closed, current-source acceptance for File New/Open Collect projects.

Each lane builds its own fresh executable. Compilation and feature-off checks
are never counted as executed tests. Existing creator/export gates are separate.
"""
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import sys
import urllib.parse

if not __debug__ or sys.flags.optimize or os.environ.get("PYTHONOPTIMIZE") not in (None, "", "0"):
    raise SystemExit("Collect project acceptance requires nonoptimized Python")

# Reuse the reviewed bounded process/source runner and PNG decoder unchanged.
import static_model_extract_gate as runner
import png_sequence_clips_gate as shared

TARGET = "collect_project_creator_ui"
REQUESTED_FEATURES = "project-create,collect-ui"
FEATURES = ["collect-dodge", "collect-ui", "project-create", "project-ui", "sprites"]
CPU_TESTS = (
    "creator_file_widgets_create_save_reopen_preserve_identity",
    "creator_cancel_invalid_inputs_and_existing_dest_preserve_host",
    "creator_dirty_or_stale_presentation_blocks_switch",
)
GPU_TEST = "creator_file_widgets_llvmpipe_korean_sprites_gameplay"
CHILD_TEST = "creator_source_hidden_reopen"
IGNORED_TESTS = (GPU_TEST, CHILD_TEST)
ALL_TESTS = CPU_TESTS + IGNORED_TESTS
FIXTURE_COUNT = 23
SOURCE_PATHS = (
    "crates/orr_editor/src/app.rs",
    "crates/orr_editor/src/editor.rs",
    "crates/orr_editor/src/app/collect_ui_preview.rs",
    "crates/orr_editor/src/app/collect_project.rs",
    "crates/orr_editor/src/app/collect_project/panel.rs",
    "crates/orr_editor/src/app/collect_project/tests.rs",
    "crates/orr_editor/src/editor/collect_project.rs",
    "crates/orr_editor/src/editor/collect_project_tests.rs",
    "crates/orr_editor/src/app/collect_ui_project_switch_tests.rs",
    "crates/orr_editor/tests/collect_project_creator_ui.rs",
    "crates/orr_sample/src/project_create.rs",
    "crates/orr_sample/src/collect_project.rs",
    "docs/editor-collect-projects.md",
    "tools/collect_project_creator_gate.py",
    "tools/test_collect_project_creator_gate.py",
    ".github/workflows/collect-project-creator.yml",
)
LIBRARY_SUITES = {
    "app::collect_project::tests": (
        "host_failure_retains_created_project_and_explicit_open_recovery",
        "same_path_reopen_during_preparation_rejects_adoption_and_retains_created_files",
        "presentation_change_during_preparation_prevents_replacement",
        "repeated_creation_never_replaces_preexisting_output",
        "pending_control_rejects_before_creating_any_files",
        "open_rejects_incomplete_project_without_replacement",
        "wrong_candidate_host_cannot_replace_current_project",
        "collect_candidate_with_wrong_scene_checksum_cannot_replace_current_project",
    ),
    "app::collect_project::panel::tests": (
        "validates_explicit_seed_path_and_canonical_identity_without_repair",
        "repeated_open_preserves_pending_request_and_created_recovery",
        "open_uses_explicit_path_without_seed_or_uuid",
    ),
    "editor::collect_project::tests": (
        "clean_source_stamp_is_read_only_and_repeatable",
        "collect_source_is_supported_without_mutation",
        "stale_clean_caches_cannot_hide_dirty_host",
        "stale_caches_cannot_hide_transaction_or_proposal",
        "stale_edit_cache_cannot_hide_play",
        "pending_control_gesture_and_uncertain_edits_fail_closed",
        "unsupported_remote_viewer_down_and_listener_sources_fail_closed",
        "missing_or_stale_snapshot_and_path_fail_closed",
        "restarted_host_invalidates_stamp_even_with_same_path_and_checksum",
        "edit_then_undo_invalidates_stamp_without_losing_history",
        "malformed_authoritative_answers_fail_closed",
        "changed_or_missing_saved_scene_blocks_even_when_host_is_clean",
        "saved_ui_comparison_is_complete_and_read_only",
        "unsafe_missing_malformed_oversized_and_directory_ui_fail_closed",
        "symlink_leaf_ancestor_and_special_ui_fail_closed",
    ),
    "app::collect_ui_preview::project_switch_tests": (
        "clean_ui_switch_guard_is_read_only",
        "dirty_ui_rejection_preserves_document_and_undo",
        "missing_admission_and_changed_host_identity_fail_closed",
        "missing_or_changed_saved_ui_blocks_without_losing_document",
        "pending_move_resize_alignment_and_pointer_are_preserved",
    ),
}
WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/collect-project-creator.yml"
EVIDENCE_EXPRESSION = "${{ runner.temp }}/collect-project-creator-${{ matrix.mode }}-${{ github.run_id }}-${{ github.run_attempt }}"
require = shared.require
load_json = shared.load_json
ordinary_file = shared.ordinary_file
png_rgba = shared.png_rgba
source_fence = shared.source_fence


def run(label, command, expected=0, seconds=1200):
    source_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    return output


def validate_build(output):
    rows = [load_json(line) for line in output.splitlines() if line.startswith("{")]
    require(rows and all(type(row) is dict for row in rows), "missing Cargo JSON records")
    finished = {"reason": "build-finished", "success": True}
    require([row for row in rows if row.get("reason") == "build-finished"] == [finished]
            and rows[-1] == finished and rows[-1].get("success") is True, "Cargo must finish exactly once and successfully")


def build():
    output = run("build-creator", ["cargo", "test", "--locked", "--release", "-p", "orr_editor",
                 "--no-default-features", "--features", REQUESTED_FEATURES,
                 "--test", TARGET, "--no-run", "--message-format=json"])
    validate_build(output)
    binary = runner.pin(output, "creator-test", "orr_editor", TARGET, "test", FEATURES)
    runner.report["producers"]["creator-test"]["source_inventory_sha256"] = runner.report["source_inventory_sha256"]
    runner.record()
    return binary


def pin_library(output):
    rows = [load_json(line) for line in output.splitlines() if line.startswith("{")]
    producers = [row for row in rows if row.get("reason") == "compiler-artifact"
                 and row.get("target", {}).get("name") == "orr_editor" and row.get("executable")]
    require(len(producers) == 1, "one exact editor library test producer required")
    item = producers[0]
    crate = runner.repo / "crates/orr_editor"
    require(item.get("fresh") is False, "freshly compiled library producer required")
    require(Path(item.get("manifest_path", "")).resolve() == crate / "Cargo.toml", "wrong library manifest")
    package = item.get("package_id")
    require(type(package) is str and package.startswith("path+"), "local library package required")
    uri = urllib.parse.urlsplit(package[5:].split("#", 1)[0])
    require(uri.scheme == "file" and uri.netloc == "" and
            Path(urllib.parse.unquote(uri.path)).resolve() == crate, "wrong library package identity")
    target = item.get("target", {})
    require(target.get("kind") == ["lib"] and target.get("crate_types") == ["lib"] and
            Path(target.get("src_path", "")).resolve() == crate / "src/lib.rs", "wrong library source")
    require(item.get("profile", {}).get("test") is True and item["profile"].get("opt_level") == "3",
            "release libtest profile required")
    require(sorted(item.get("features", [])) == sorted(FEATURES), "wrong library features")
    original = Path(item["executable"])
    require(original.is_absolute() and original == original.resolve() and not original.is_symlink()
            and original.parent == runner.target_dir / "release/deps", "wrong library artifact authority")
    ordinary_file(original, 128 * 1024 * 1024)
    copied = runner.evidence / "creator-library"
    with original.open("rb") as source, copied.open("xb") as destination:
        shutil.copyfileobj(source, destination, 1024 * 1024)
    shutil.copymode(original, copied)
    require(runner.digest(original) == runner.digest(copied), "library copy differs")
    runner.report["producers"]["creator-library"] = {
        "producer": item, "file": copied.name, "sha256": runner.digest(copied), "bytes": copied.stat().st_size,
        "source_inventory_sha256": runner.report["source_inventory_sha256"],
    }
    runner.record()
    return str(copied)


def library_guards():
    output = run("build-creator-library", ["cargo", "test", "--locked", "--release", "-p", "orr_editor",
                 "--no-default-features", "--features", REQUESTED_FEATURES,
                 "--lib", "--no-run", "--message-format=json"])
    validate_build(output)
    binary = pin_library(output)
    complete = run("creator-library-inventory", [binary, "--list"])
    all_names = re.findall(r"^([A-Za-z0-9_:]+): test$", complete, re.M)
    require(all_names and len(all_names) == len(set(all_names)), "nonempty unique library inventory required")
    runner.inventory(complete, all_names)
    for suite, short_names in LIBRARY_SUITES.items():
        names = [suite + "::" + name for name in short_names]
        require(names and len(names) == len(set(names)), "nonempty unique reviewed guard inventory required")
        label = suite.replace("::", "-")
        runner.inventory(run(label + "-inventory", [binary, suite + "::", "--list"]), names)
        runner.inventory(run(label + "-ignored", [binary, suite + "::", "--ignored", "--list"]), [])
        output = run(label + "-run", [binary, suite + "::", "--nocapture", "--test-threads=1", "--color=never"])
        filtered = len(all_names) - len(names)
        require(re.findall(r"; (\d+) filtered out;", output) == [str(filtered)], "incorrect guard filter count")
        runner.result(output.replace(f"; {filtered} filtered out;", "; 0 filtered out;"), names)
        runner.report.setdefault("accepted_guard_tests", {})[suite] = names
        runner.record()


def inventory(binary):
    require(ALL_TESTS and len(ALL_TESTS) == len(set(ALL_TESTS)), "empty or duplicate expected test inventory")
    runner.inventory(run("creator-inventory", [binary, "--list"]), ALL_TESTS)
    runner.inventory(run("creator-ignored-inventory", [binary, "--ignored", "--list"]), IGNORED_TESTS)
    runner.report["test_inventory"] = {"all": list(ALL_TESTS), "ignored": list(IGNORED_TESTS)}
    runner.record()


def validate_exact(output, name, total):
    require(name in ALL_TESTS and type(total) is int and total == len(ALL_TESTS), "unknown or empty test authority")
    output = output.replace("\r\n", "\n")
    summaries = re.findall(r"^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; (\d+) filtered out; finished in [0-9]+(?:\.[0-9]+)?s$", output, re.M)
    require(summaries == [str(total - 1)], "one exact executed passing case required")
    normalized = output.replace(f"; {total - 1} filtered out;", "; 0 filtered out;")
    runner.result(normalized, [name])


def execute(binary, name):
    require(name in ALL_TESTS, "unreviewed acceptance test")
    arguments = [binary, name, "--exact", "--nocapture", "--format=pretty", "--test-threads=1", "--color=never"]
    if name in IGNORED_TESTS:
        arguments += ["--ignored"]
    output = run(name, arguments)
    validate_exact(output, name, len(ALL_TESTS))
    runner.report.setdefault("accepted_tests", {})[name] = {"count": 1, "explicit_ignored": name in IGNORED_TESTS}
    runner.record()


def boundary_commands():
    profiles = (("default", []), ("feature-off", ["--no-default-features"]),
                ("creator-only", ["--no-default-features", "--features", "project-create"]),
                ("ui-only", ["--no-default-features", "--features", "collect-ui"]),
                ("creator-ui", ["--no-default-features", "--features", REQUESTED_FEATURES]))
    for name, flags in profiles:
        common = ["--locked", "--release", "-p", "orr_editor", *flags,
                  "--lib", "--bin", "orr_editor", "--test", TARGET]
        yield name + "-check", ["cargo", "check", *common]
        yield name + "-strict", ["cargo", "clippy", *common, "--", "-D", "warnings"]


def cpu():
    binary = build()
    inventory(binary)
    for name in CPU_TESTS:
        execute(binary, name)
    library_guards()
    for label, command in boundary_commands():
        run(label, command)
    runner.report["coverage"]["cpu"] = "PASSED"
    runner.report["coverage"]["strict_feature_boundaries"] = "PASSED"
    runner.record()


def json_file(path):
    value = load_json(ordinary_file(path, 1024 * 1024).decode())
    require(type(value) is dict, "JSON object required: " + path.name)
    return value


def required_true(value, names):
    for name in names:
        require(value.get(name) is True, "missing creator proof: " + name)


def adapter(directory):
    value = json_file(directory / "adapter.json")
    require(type(value.get("name")) is str and "llvmpipe" in value["name"].lower()
            and value.get("backend") == "Vulkan" and value.get("device_type") == "Cpu",
            "explicit software Vulkan adapter required")
    require(value.get("renderer_options") == "default" and value.get("predictable_texture_filtering") is False
            and value.get("dithering") is True, "actual production renderer options required")


WINDOW = (1800, 1200)
MAIN_FRAMES = ("created", "same-app-reopened", "playing", "won", "restarted", "stopped")
CHILD_FRAMES = ("fresh-reopened", "playing", "won", "restarted", "stopped")
PREVIEW_IMAGES = ("korean-title", "title-without-korean", "korean-title-restored", "hud-preview")


def bounded_rect(rect, size, dimensions=None):
    require(type(rect) is list and len(rect) == 4 and all(type(x) in (int, float) and math.isfinite(x) for x in rect),
            "numeric bounded rectangle required")
    x, y, width, height = rect
    require(x >= 0 and y >= 0 and width > 0 and height > 0 and x + width <= size[0] and y + height <= size[1],
            "rectangle outside full editor")
    require(dimensions is None or [width, height] == list(dimensions), "unexpected preview dimensions")
    return x, y, width, height


def frame(directory, name):
    value = json_file(directory / (name + ".json"))
    won = name == "won"
    require(type(value.get("phase")) is int and value["phase"] == int(won)
            and type(value.get("score")) is int and value["score"] == (2 if won else 0), "wrong authoritative phase/score")
    require(type(value.get("tick")) is int and value["tick"] >= 0 and type(value.get("verified_tick")) is int
            and value["tick"] == value["verified_tick"], "capture must display verified tick")
    require(type(value.get("checksum")) is str and re.fullmatch(r"[0-9a-f]{16}", value["checksum"]), "missing authoritative checksum")
    require(value.get("window_pixels") == list(WINDOW), "full editor dimensions required")
    bounded_rect(value.get("viewport"), WINDOW)
    size = value.get("viewport_pixels")
    require(type(size) is list and len(size) == 2 and all(type(x) is int and 0 < x <= max(WINDOW) for x in size),
            "positive viewport backing dimensions required")
    required_true(value, ("exact_opaque_source_texels",))
    actors = value.get("actors")
    require(type(actors) is list and len(actors) == (2 if won else 4), "wrong visible sprite inventory")
    guids = set()
    for actor in actors:
        require(type(actor) is dict and type(actor.get("guid")) is str
                and re.fullmatch(r"e_[0-9a-f]{32}", actor["guid"]) and actor["guid"] not in guids,
                "missing or duplicate generated actor GUID")
        guids.add(actor["guid"])
        require(type(actor.get("region")) is int and 0 <= actor["region"] <= 4294967295, "sampled sprite region must be u32")
        require(type(actor.get("opaque_texels")) is int and actor["opaque_texels"] > 40, "insufficient exact source texels")
        position = actor.get("position")
        require(type(position) is list and len(position) == 2 and
                all(type(x) in (int, float) and math.isfinite(x) for x in position), "invalid actor position")
    return value


def korean_preview(directory, images):
    value = json_file(directory / "korean-preview.json")
    required_true(value, ("changes_confined_to_title_geometry", "undo_restores_exact_canvas_pixels", "host_and_files_unchanged"))
    x, y, width, height = bounded_rect(value.get("canvas"), WINDOW, (640, 360))
    korean, blank, restored = (images[name] for name in PREVIEW_IMAGES[:3])
    require(all(len(image) == WINDOW[0] * WINDOW[1] * 4 for image in (korean, blank, restored)), "incomplete decoded PNG")
    changed = 0
    for py in range(math.ceil(y), math.floor(y + height)):
        begin = (py * WINDOW[0] + math.ceil(x)) * 4
        end = (py * WINDOW[0] + math.floor(x + width)) * 4
        require(korean[begin:end] == restored[begin:end], "Undo did not restore exact Korean preview pixels")
        for px in range(math.ceil(x), math.floor(x + width)):
            at = (py * WINDOW[0] + px) * 4
            if korean[at:at + 4] != blank[at:at + 4]:
                require(160 <= px - x <= 480 and 56 <= py - y <= 92, "Korean negative control changed outside title geometry")
                changed += 1
    require(type(value.get("changed_korean_pixels")) is int and value["changed_korean_pixels"] == changed and changed > 0,
            "Korean glyph pixel delta differs from retained PNGs")


def validate_gpu(directory):
    require(directory.is_dir() and not directory.is_symlink(), "ordinary GPU evidence directory required")
    value = json_file(directory / "collect-creator-acceptance.json")
    require(type(value.get("schema")) is int and value["schema"] == 1, "unknown creator receipt schema")
    require(value.get("template") == "collect-dodge-ui-2d-v1" and value.get("seed") == "editor-file-collect-creator-v1"
            and value.get("game_id") == "12345678-1234-4234-8234-123456789abc", "wrong creator identity")
    required_true(value, ("main_editor_app", "starts_in_phys_game", "actual_file_new_dialog", "actual_file_open_dialog",
                         "same_app_save_reopen", "fresh_app_source_hidden_reopen", "installed_package_bytes_unchanged",
                         "identity_and_guids_unchanged", "authoritative_win_restart_stop", "exact_opaque_source_texels",
                         "korean_preview_pixels", "physical_egui_events"))
    require(value.get("native_os_keyboard") is False and value.get("renderer_options") == "default"
            and value.get("window_pixels") == list(WINDOW), "wrong renderer or input scope")
    executable = value.get("current_exe")
    pinned = runner.evidence / runner.report["producers"]["creator-test"]["file"]
    require(type(executable) is str and Path(executable).is_absolute() and Path(executable).resolve() == pinned,
            "receipt must come from pinned current-source executable")
    packages = value.get("package_files")
    require(type(packages) is list and len(packages) == 12 and all(type(name) is str for name in packages)
            and len(packages) == len(set(packages)) and "orr.packages.lock.json" in packages,
            "exact installed package path closure required")
    groups = {}
    for name in packages:
        if name == "orr.packages.lock.json":
            continue
        match = re.fullmatch(r"\.orr/packages/objects/([0-9a-f]{64})/([A-Za-z_][A-Za-z0-9_.-]*)", name)
        require(match is not None, "unsafe or noncanonical installed package path")
        groups.setdefault(match[1], set()).add(match[2])
    require({frozenset(names) for names in groups.values()} == {
        frozenset(("COPYRIGHT.txt", "OFL.txt", "OrreryKoreanUI.otf", "corpus.txt", "font-manifest.json", "orr.package.json")),
        frozenset(("LICENSE.txt", "lantern_keeper.png", "lantern_keeper.rgba", "orr.package.json", "sprites.json")),
    } and len(groups) == 2, "full installed font/sprite licenses and content closure required")
    child_dir = directory / "source-hidden"
    require(child_dir.is_dir() and not child_dir.is_symlink(), "source-hidden child evidence required")
    child = json_file(child_dir / "receipt.json")
    require(type(child.get("schema")) is int and child["schema"] == 1 and value.get("source_hidden") == child,
            "parent/child receipt disagreement")
    required_true(child, ("source_hidden", "actual_file_open_dialog", "fresh_app", "installed_bytes_only",
                         "files_and_identity_unchanged", "gameplay_win_restart_stop"))
    require(child.get("native_os_keyboard") is False and child.get("renderer_options") == "default",
            "child must retain production renderer/input scope")
    child_exe = child.get("current_exe")
    require(type(child_exe) is str and Path(child_exe).is_absolute() and not Path(child_exe).is_relative_to(runner.repo),
            "source-hidden child executable must be outside checkout")
    validate_exact(ordinary_file(directory / "source-hidden-child.log", 8 * 1024 * 1024).decode(), CHILD_TEST, len(ALL_TESTS))
    captures = {}
    frames = {}
    for folder, frame_names, previews in ((directory, MAIN_FRAMES, PREVIEW_IMAGES),
                                           (child_dir, CHILD_FRAMES, ("saved-korean-hud",))):
        adapter(folder)
        expected = {name + ".png" for name in (*frame_names, *previews)}
        require({path.name for path in folder.glob("*.png")} == expected, "exact full-window PNG inventory required")
        decoded_previews = {}
        for name in (*frame_names, *previews):
            path = folder / (name + ".png")
            pixels = png_rgba(path, WINDOW)
            captures[path.relative_to(directory).as_posix()] = {"bytes": path.stat().st_size, "sha256": runner.digest(path)}
            if name in PREVIEW_IMAGES[:3] and folder == directory:
                decoded_previews[name] = pixels
            if name in frame_names:
                frames[path.relative_to(directory).with_suffix("").as_posix()] = frame(folder, name)
        if folder == directory:
            korean_preview(folder, decoded_previews)
    require(frames["same-app-reopened"]["checksum"] == frames["stopped"]["checksum"]
            == frames["source-hidden/fresh-reopened"]["checksum"] == frames["source-hidden/stopped"]["checksum"],
            "Stop must restore unchanged authoritative scene")
    require(frames["created"]["checksum"] != frames["same-app-reopened"]["checksum"],
            "actual saved inspector edit must change the authoritative scene")
    for prefix in ("", "source-hidden/"):
        require(frames[prefix + "won"]["tick"] > 0 and
                frames[prefix + "restarted"]["tick"] > frames[prefix + "won"]["tick"],
                "victory and restart need ordered authoritative ticks")
    runner.report["creator_capture_sha256"] = captures
    return value


def gpu():
    shared.software()
    binary = build()
    inventory(binary)
    captures = runner.evidence / "captures"
    captures.mkdir(exist_ok=False)
    os.environ["ORR_COLLECT_CREATOR_EVIDENCE"] = str(captures)
    os.environ["ORR_EXPORT_HIDE_ROOT"] = str(runner.repo)
    execute(binary, GPU_TEST)
    value = validate_gpu(captures)
    runner.report["creator_acceptance"] = value
    runner.report["coverage"]["software_gpu"] = "PASSED"
    runner.record()


def validate_workflow(text):
    """Validate runner context location without adding a YAML dependency."""
    lines = text.splitlines()
    require(not re.search(r"continue-on-error|paths-ignore|^\s*paths:", text, re.M), "acceptance must not be optional")
    require("        mode: [cpu, gpu]" in lines and "      fail-fast: false" in lines, "both lanes required")
    require("  required:" in lines and "    if: always()" in lines and "    needs: acceptance" in lines,
            "fail-closed aggregate required")
    require('        run: test "$ACCEPTANCE" = success' in lines, "aggregate must reject every non-success")
    # runner is unsupported in jobs.<id>.env. The only evidence expression is
    # attached to the acceptance step and repeated verbatim in artifact.path.
    expected = "          ORR_COLLECT_CREATOR_GATE_EVIDENCE: " + EVIDENCE_EXPRESSION
    require(lines.count(expected) == 1, "evidence requires one step-scoped environment assignment")
    index = lines.index(expected)
    require(lines[index - 2] == "      - name: Run current-source fail-closed acceptance"
            and lines[index - 1] == "        env:" and
            lines[index + 1] == "        run: python -B tools/collect_project_creator_gate.py '${{ matrix.mode }}'",
            "evidence environment must belong to executing step")
    require(lines.count("          path: " + EVIDENCE_EXPRESSION) == 1,
            "artifact path must equal executing step's literal expression")
    for line in lines:
        if "runner.temp" in line:
            require(line in (expected, "          path: " + EVIDENCE_EXPRESSION), "runner.temp in unsupported or mismatched context")
    upload = lines.index("        uses: actions/upload-artifact@v4") if "        uses: actions/upload-artifact@v4" in lines else -1
    require(upload > 0 and lines[upload - 1] == "        if: always()", "artifact upload must survive failed execution")
    require("          if-no-files-found: error" in lines and "          include-hidden-files: true" in lines,
            "partial failure evidence must be retained")


def begin():
    require(os.environ.get("PYTHONDONTWRITEBYTECODE") == "1", "inherited bytecode prevention required")
    runner.frozen = runner.begin()
    require(shared.source_paths() == list(runner.frozen), "untracked or ignored source files cannot enter producers")
    require(set(SOURCE_PATHS) <= set(runner.frozen), "missing Collect creator feature sources")
    runner.report["source_inventory_sha256"] = hashlib.sha256(json.dumps(runner.frozen, sort_keys=True).encode()).hexdigest()
    runner.record()
    validate_workflow(WORKFLOW.read_text())
    output = run("creator-checker-fixtures", [sys.executable, "-B", str(runner.repo / "tools/test_collect_project_creator_gate.py")])
    require(output.rstrip().endswith(f"COLLECT_CREATOR_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped"),
            "independent gate fixture census")


def main():
    require(len(sys.argv) == 2 and sys.argv[1] in {"cpu", "gpu"}, "usage: collect_project_creator_gate.py cpu|gpu")
    require(sys.platform == "linux" and os.uname().machine == "x86_64", "Collect creation acceptance requires Linux x86_64")
    mode = sys.argv[1]
    root = Path(__file__).resolve().parents[1]
    raw = Path(os.environ["ORR_COLLECT_CREATOR_GATE_EVIDENCE"])
    require(raw.is_absolute() and raw == raw.resolve(), "absolute canonical fresh evidence path required")
    require(not raw.is_relative_to(root), "evidence must be outside source checkout")
    runner.initialize(root, raw, mode)
    runner.report["mode"] = mode
    runner.report["coverage"] = {"cpu": "NOT RUN", "strict_feature_boundaries": "NOT RUN", "software_gpu": "NOT RUN"}
    error = None
    try:
        begin()
        {"cpu": cpu, "gpu": gpu}[mode]()
        expected = {"cpu", "strict_feature_boundaries"} if mode == "cpu" else {"software_gpu"}
        require(runner.report["coverage"] == {key: "PASSED" if key in expected else "NOT RUN"
                                             for key in runner.report["coverage"]}, "incomplete lane coverage")
        source_fence()
    except BaseException as failure:
        error = failure
    finally:
        runner.finish(error)


if __name__ == "__main__":
    main()
