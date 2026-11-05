#!/usr/bin/env python3
"""Source-bound optional PixelOE skin CPU/CLI gate. No window/GPU/playtest."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

from check_stormglass_pixelfx import verify_tests, self_test as parser_self_test

ROOT = Path(__file__).resolve().parents[1]
COMBINED = "coop-defense-pixel-motion,coop-defense-pixelfx,coop-defense-audio,coop-defense-run-save"
CONFIGURATIONS = {"skin-only": "coop-defense-pixel-motion", "combined": COMBINED}
CORE = [
    "admitted_asset_bytes_geometry_timing_provenance_and_endpoints_are_unchanged",
    "nearest_straight_rgba_upload_is_exact_and_prepare_is_idempotent",
    "complete_visual_cycle_preserves_neutral_hold_and_snapshot_clock",
    "rivet_skin_uses_fixed_pivot_uv_and_readable_role_tier_badge_only",
    "real_session_rendering_does_not_change_any_simulation_bytes",
    "unprepared_optional_skin_preserves_default_vector_scene_exactly",
    "all_frame_alpha_bounds_fit_cells_without_per_frame_normalization",
]
APP = [
    "actual_app_pause_focus_resume_and_inert_redraws_freeze_pixel_frame",
    "actual_app_prestart_is_neutral_and_restart_uses_new_run_clock",
]
RESTORE = "actual_app_loaded_save_restores_pixel_frame_without_backlog_or_catchup"
IGNORED = ["defense_audio::tests::cli_authored_bank_source_hidden"]
ASSET_HASH = "efcfb8bc0acbffb76c230846c111ba561d74cd6970ac4e2a721d4e981b6c5b56"
ASSETS = ROOT / "crates/orr_sample/assets/stormglass-pixel-motion"


def asset_check():
    names = {"atlas.png", "sprites.json", "orr.package.json", "LICENSE.txt", "SOURCE.json"}
    if {p.name for p in ASSETS.iterdir()} != names:
        raise ValueError("embedded asset inventory differs")
    provenance = json.loads((ASSETS / "SOURCE.json").read_bytes())
    if set(provenance["sha256"]) != names - {"SOURCE.json"}:
        raise ValueError("source hash inventory differs")
    for name, expected in provenance["sha256"].items():
        if hashlib.sha256((ASSETS / name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"admitted asset hash mismatch: {name}")
    if provenance["sha256"]["atlas.png"] != ASSET_HASH:
        raise ValueError("accepted atlas changed")
    sprite = json.loads((ASSETS / "sprites.json").read_bytes())
    if sprite["clips"][0]["mode"] != "once":
        raise ValueError("package Once metadata changed")
    return provenance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if args.self_test:
        parser_self_test()
        asset_check()
        print("pixel motion gate parser and source asset checks passed")
        return
    if args.evidence is None or not args.evidence.is_absolute():
        parser.error("--evidence requires a new absolute directory outside the checkout")
    evidence = args.evidence.resolve()
    if evidence.is_relative_to(ROOT):
        parser.error("evidence must remain outside git")
    evidence.mkdir(parents=False, exist_ok=False)

    def run(label, command):
        print(label, flush=True)
        (evidence / (label + ".command.json")).write_text(json.dumps({"cwd": str(ROOT), "argv": command}, indent=2) + "\n")
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (evidence / (label + ".log")).write_text(result.stdout)
        (evidence / (label + ".exit-status")).write_text(str(result.returncode) + "\n")
        if result.returncode:
            raise RuntimeError(f"{label} failed: exit {result.returncode}; log preserved")
        return result.stdout.replace("\r\n", "\n")

    def build(label, features):
        output = run(label, ["cargo", "build", "--locked", "--release", "-p", "orr_sample", "--no-default-features", "--features", features, "--bin", "coop_defense", "--message-format=json"])
        binaries = [record["executable"] for line in output.splitlines() if line.startswith("{") and (record := json.loads(line)).get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "coop_defense" and record.get("executable")]
        if len(binaries) != 1:
            raise RuntimeError("build must identify exactly one native executable")
        binary = evidence / "bin" / label
        shutil.copy2(binaries[0], binary)
        return binary

    source = run("source", ["git", "rev-parse", "HEAD", "HEAD^{tree}"]).splitlines()
    if len(source) != 2 or run("status", ["git", "status", "--porcelain"]):
        raise RuntimeError("gate requires clean committed source")
    provenance = asset_check()
    run("rustc", ["rustc", "--version", "--verbose"])
    run("cargo", ["cargo", "--version"])
    executed = {}
    for configuration, features in CONFIGURATIONS.items():
        suites = {"defense_pixel_motion::tests": CORE, "defense_app::pixel_motion_tests": APP + ([RESTORE] if configuration == "combined" else [])}
        for suite, names in suites.items():
            label = configuration + "-" + suite.replace("::", "-")
            output = run(label, ["cargo", "test", "--locked", "--release", "-p", "orr_sample", "--no-default-features", "--features", features, "--lib", suite, "--", "--test-threads=1", "--color=never"])
            executed[label] = verify_tests(output, [suite + "::" + name for name in names])
        run(configuration + "-clippy", ["cargo", "clippy", "--locked", "--release", "-p", "orr_sample", "--no-default-features", "--features", features, "--lib", "--bin", "coop_defense", "--", "-D", "warnings"])
    for label, features, ignored in [("default-native", "coop-defense", []), ("combined-native", COMBINED, IGNORED)]:
        output = run(label + "-regression", ["cargo", "test", "--locked", "--release", "-p", "orr_sample", "--no-default-features", "--features", features, "--lib", "defense_", "--", "--test-threads=1", "--color=never"])
        executed[label] = verify_tests(output, allowed_ignored=ignored)
        if label == "default-native" and any("pixel_motion" in name for name in executed[label]):
            raise ValueError("optional pixel motion leaked into feature-off build")
    (evidence / "bin").mkdir()
    baseline = build("default-native", "coop-defense")
    binary = build("pixel-motion-native", COMBINED)
    license_text = run("licenses", [str(binary), "--pixel-motion-licenses"])
    if license_text.encode() != (ASSETS / "LICENSE.txt").read_bytes():
        raise ValueError("standalone binary notices differ")
    help_text = run("help", [str(binary), "--help"])
    if "not attack-synchronized" not in help_text:
        raise ValueError("optional visual demo labeling missing")
    summaries = []
    for label, executable in [("baseline", baseline), ("optional", binary)]:
        output = run(label + "-headless", [str(executable), "--headless", "--demo", "--ticks", "360"])
        matches = re.findall(r"^Stormglass tick=.+ checksum=0x[0-9a-f]+$", output, re.M)
        if len(matches) != 1:
            raise ValueError("missing or duplicate native simulation summary")
        summaries.append(matches[0])
    if summaries[0] != summaries[1]:
        raise ValueError("optional skin changed native simulation results")
    if run("source-after", ["git", "rev-parse", "HEAD", "HEAD^{tree}"]).splitlines() != source or run("status-after", ["git", "status", "--porcelain"]):
        raise RuntimeError("source changed during validation; run is invalid")
    summary = {"head": source[0], "tree": source[1], "dirty": False, "executed": executed, "configurations": CONFIGURATIONS, "assetSha256": ASSET_HASH, "provenance": provenance, "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "notExecutedSeparateAudioGate": IGNORED, "simulationSummary": summaries[0], "workspaceAggregate": False, "playtest": False, "window": False, "gpu": False, "browser": False}
    (evidence / "scope.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"Optional native pixel motion CPU/CLI checks passed: {evidence}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, AssertionError) as error:
        print(f"Pixel motion gate failed: {error}", file=sys.stderr)
        raise SystemExit(1)
