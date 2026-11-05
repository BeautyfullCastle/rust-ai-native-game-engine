#!/usr/bin/env python3
"""Focused native CPU/CLI gate. No browser, window, or GPU execution."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FEATURES = "coop-defense-pixelfx,coop-defense-audio,coop-defense-run-save"
CONFIGURATIONS = {"vfx-only": "coop-defense-pixelfx", "combined": FEATURES}
# Existing audio bank acceptance needs separate source-hidden fixtures.
# Do not relabel its normal ignored status as execution by this VFX gate.
REGRESSION_IGNORED = ["defense_audio::tests::cli_authored_bank_source_hidden"]
CASES = {
    "defense_vfx::tests": [
        "genuine_embedded_pngs_decode_and_nearest_textures_preserve_alpha",
        "predicted_verified_duplicate_never_restarts_and_cancel_removes",
        "only_real_hit_release_map_and_all_other_events_are_inert",
        "one_shot_expiry_is_session_clock_driven_and_never_replays",
        "restart_generation_and_loaded_session_clear_history_without_backlog",
        "invalid_late_record_leaves_entire_batch_unpublished",
        "active_history_and_input_batches_are_bounded",
        "full_public_producer_budget_is_valid_and_drops_only_visual_overflow",
        "real_session_events_repeat_without_changes_to_any_gameplay_bytes",
        "painter_uses_exact_uv_pivot_and_clipped_nearest_texture_without_frame_writes",
    ],
    "defense_app::vfx_tests": [
        "actual_app_combat_pause_focus_and_resume_freeze_visual_clock",
        "actual_app_restart_clears_old_effects_and_changes_generation_once",
        "actual_app_prestart_and_inert_redraws_cannot_create_effects",
    ],
}


def verify_tests(output, expected=None, allowed_ignored=()):
    summaries = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", output, re.M)
    passed = re.findall(r"^test (\S+) \.\.\. ok$", output, re.M)
    ignored = re.findall(r"^test (\S+) \.\.\. ignored(?:,.*)?$", output, re.M)
    running = re.findall(r"^running (\d+) tests?$", output, re.M)
    if (len(summaries) != 1 or len(running) != 1 or int(summaries[0][0]) < 1
            or tuple(map(int, summaries[0][1:4])) != (0, len(allowed_ignored), 0)
            or sorted(ignored) != sorted(allowed_ignored)
            or int(running[0]) != len(passed) + len(ignored)
            or int(summaries[0][0]) != len(passed)
            or len(set(passed + ignored)) != len(passed) + len(ignored)):
        raise ValueError("missing, duplicate, failed, ignored, or inconsistent executed tests")
    if expected is not None and sorted(passed) != sorted(expected):
        raise ValueError("executed test names differ from independent inventory")
    return passed


def self_test():
    good = "running 1 test\ntest sample::case ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n"
    if verify_tests(good, ["sample::case"]) != ["sample::case"]:
        raise ValueError("valid test output rejected")
    for bad in ["", good.replace("1 passed", "0 passed"), good.replace("0 ignored", "1 ignored"), good + good, good.replace("... ok", "... ignored"), good.replace("running 1", "running 2")]:
        try:
            verify_tests(bad, ["sample::case"])
        except ValueError:
            pass
        else:
            raise AssertionError("false-green test output accepted")
    try:
        verify_tests(good, ["wrong::case"])
    except ValueError:
        pass
    else:
        raise AssertionError("wrong test inventory accepted")
    with_ignored = good.replace("running 1 test", "running 2 tests").replace("0 ignored", "1 ignored").replace("test result:", "test old::fixture ... ignored, separate gate\ntest result:")
    if verify_tests(with_ignored, ["sample::case"], ["old::fixture"]) != ["sample::case"]:
        raise ValueError("explicit separate-gate inventory rejected")
    for allowed in [[], ["wrong::fixture"], ["old::fixture", "old::fixture"]]:
        try:
            verify_tests(with_ignored, ["sample::case"], allowed)
        except ValueError:
            pass
        else:
            raise ValueError("unexpected or duplicate ignored test accepted")
    print("pixelfx gate parser self-test passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.evidence is None or not args.evidence.is_absolute():
        parser.error("--evidence requires a new absolute directory")
    evidence = args.evidence
    evidence.mkdir(parents=False, exist_ok=False)
    def run(label, command):
        print(label, flush=True)
        (evidence / (label + ".command.json")).write_text(json.dumps({"cwd": str(ROOT), "argv": command}, indent=2) + "\n")
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (evidence / (label + ".log")).write_text(result.stdout)
        (evidence / (label + ".exit-status")).write_text(str(result.returncode) + "\n")
        if result.returncode:
            raise RuntimeError(f"{label} failed with exit {result.returncode}; see preserved log")
        return result.stdout.replace("\r\n", "\n")
    source = run("source", ["git", "rev-parse", "HEAD", "HEAD^{tree}"]).splitlines()
    dirty = run("status", ["git", "status", "--porcelain"])
    if dirty or len(source) != 2:
        raise RuntimeError("gate requires a clean committed source tree")
    for label, command in [("rustc", ["rustc", "--version", "--verbose"]), ("cargo", ["cargo", "--version"])]:
        run(label, command)
    assets = ROOT / "crates/orr_sample/assets/stormglass-pixelfx"
    provenance = json.loads((assets / "SOURCE.json").read_bytes())
    for effect in provenance["effects"]:
        name = effect["id"].split("/")[1]
        if hashlib.sha256((assets / name / (name + ".png")).read_bytes()).hexdigest() != effect["correctedSha256"]:
            raise ValueError(f"asset hash mismatch: {name}")
    if (assets / "hit-spark/LICENSE.txt").read_bytes() != (assets / "zap-hit/LICENSE.txt").read_bytes():
        raise ValueError("effect license notices differ")
    executed = {}
    for configuration, features in CONFIGURATIONS.items():
        for suite, names in CASES.items():
            label = configuration + "-" + suite.replace("::", "-")
            output = run(label, ["cargo", "test", "--locked", "--release", "-p", "orr_sample", "--features", features, "--lib", suite, "--", "--test-threads=1", "--color=never"])
            executed[label] = verify_tests(output, [suite + "::" + name for name in names])
        run(configuration + "-clippy", ["cargo", "clippy", "--locked", "--release", "-p", "orr_sample", "--features", features, "--lib", "--bin", "coop_defense", "--", "-D", "warnings"])
    regression = run("native-defense-regression", ["cargo", "test", "--locked", "--release", "-p", "orr_sample", "--features", FEATURES, "--lib", "defense_", "--", "--test-threads=1", "--color=never"])
    executed["native-defense-regression"] = verify_tests(regression, allowed_ignored=REGRESSION_IGNORED)
    output = run("build", ["cargo", "build", "--locked", "--release", "-p", "orr_sample", "--features", FEATURES, "--bin", "coop_defense", "--message-format=json"])
    binaries = [record["executable"] for line in output.splitlines() if line.startswith("{") and (record := json.loads(line)).get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "coop_defense" and record.get("executable")]
    if len(binaries) != 1:
        raise RuntimeError("build did not identify exactly one native executable")
    (evidence / "bin").mkdir()
    binary = evidence / "bin/coop_defense"
    shutil.copy2(binaries[0], binary)
    licenses = run("licenses", [str(binary), "--pixelfx-licenses"])
    if licenses.encode() != (assets / "hit-spark/LICENSE.txt").read_bytes():
        raise ValueError("standalone binary license notice differs")
    headless = run("headless", [str(binary), "--headless", "--demo", "--ticks", "300"])
    match = re.search(r"^Stormglass pixelfx active=(\d+) started=(\d+) dropped=(\d+) canceled=(\d+)$", headless, re.M)
    if not match or int(match[2]) < 1:
        raise RuntimeError("native CLI did not consume actual combat effects")
    source_after = run("source-after", ["git", "rev-parse", "HEAD", "HEAD^{tree}"]).splitlines()
    dirty_after = run("status-after", ["git", "status", "--porcelain"])
    if source_after != source or dirty_after:
        raise RuntimeError("source changed during validation; discard the run")
    summary = {"head": source[0], "tree": source[1], "dirty": False, "configurations": CONFIGURATIONS, "executed": executed, "notExecutedSeparateAudioGate": REGRESSION_IGNORED, "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "activeAtCaptureTick": int(match[1]), "upstream": provenance["commit"], "browser": False, "gpu": False, "workspaceAggregate": False}
    (evidence / "scope.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"Native pixelfx CPU/CLI checks passed: {evidence}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, AssertionError) as error:
        print(f"Native pixelfx gate failed: {error}", file=sys.stderr)
        raise SystemExit(1)
