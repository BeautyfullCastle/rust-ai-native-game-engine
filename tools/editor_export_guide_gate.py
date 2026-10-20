#!/usr/bin/env python3
"""Required #275 CPU route/widget acceptance in the existing sample-build job.

Reuses bounded process ownership, exact result parsing and source fences. Three
cfg executions overlap: 10 unique cases, not 20. No GPU/native-player claim.
"""
import copy
import json
import os
from pathlib import Path
import re
import shutil
import sys
import urllib.parse

import unsaved_close_gate as source
import static_model_extract_gate as runner
import png_sequence_clips_gate as shared

require = shared.require
PREFIX = "app::export_guide_tests::"
DEFAULT = tuple(PREFIX + name for name in (
    "routes_do_not_invent_default_or_cli_editor_export",
    "actual_app_guide_is_explicit_and_passive_even_when_source_disappears",
    "actual_app_dirty_edit_does_not_open_export_guide_or_save",
))
OFF = DEFAULT + tuple(PREFIX + name for name in (
    "cli_profile_route_is_not_an_editor_feature_claim",
    "room_route_obeys_host_and_feature_boundary",
    "mesh_route_preserves_pe_target_and_compiled_feature_boundary",
    "hazard_route_never_falls_back_to_legacy_mesh_export",
))
ON = OFF + tuple(PREFIX + name for name in (
    "actual_app_mesh_guide_opens_existing_settings_without_export_or_trust",
    "actual_app_hazard_guide_opens_only_hazard_settings_and_explains_inputs",
    "actual_app_play_does_not_allow_opening_export_settings_or_switch_tabs",
))
OFF_FEATURES = ["collect-dodge", "mesh-hazards", "mesh-mission", "mesh-navigation", "models", "room-project"]
ON_FEATURES = ["animated-models", "collect-dodge", "collect-ui", "irradiance-probes", "mesh-animated-models", "mesh-export", "mesh-hazards", "mesh-hazards-export", "mesh-mission", "mesh-navigation", "models", "navigation", "package-manager", "project-create", "project-ui", "room-animated-models", "room-authoring", "room-project", "sprites", "terrain", "terrain-physics", "tilemap", "windows-export", "windows-projects"]
CONFIGS = (
    ("default", "", [], DEFAULT),
    ("export-off", "mesh-hazards,room-project,collect-dodge", OFF_FEATURES, OFF),
    ("export-on", source.OPTIONAL, ON_FEATURES, ON),
)
FIXTURE_COUNT = 8


def producer(output, label, features):
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
    require(sorted(item.get("features", [])) == features, "wrong feature closure")
    original = Path(item["executable"])
    require(original.is_absolute() and original == original.resolve() and not original.is_symlink()
            and original.parent == runner.target_dir / "release/deps", "wrong artifact path")
    shared.ordinary_file(original, 256 * 1024**2)
    copied = runner.evidence / (label + (".exe" if os.name == "nt" else ""))
    with original.open("rb") as src, copied.open("xb") as dst:
        shutil.copyfileobj(src, dst, 1024**2)
    shutil.copymode(original, copied)
    require(runner.digest(original) == runner.digest(copied), "artifact copy differs")
    runner.report["producers"][label] = {"producer": item, "file": copied.name,
        "sha256": runner.digest(copied), "bytes": copied.stat().st_size}
    runner.record()
    return str(copied)


def binary_fence(label, binary):
    shared.ordinary_file(Path(binary), 256 * 1024**2)
    require(runner.digest(Path(binary)) == runner.report["producers"][label]["sha256"], "producer bytes changed")


def execute(label, binary, suffix, arguments):
    binary_fence(label, binary)
    output = source.run(label + "-" + suffix, [binary, *arguments])
    binary_fence(label, binary)
    return output


def result(output, names, total):
    require(type(total) is int and total >= len(names) > 0, "invalid inventory count")
    filtered = total - len(names)
    output = output.replace("\r\n", "\n")
    require(re.findall(r"; (\d+) filtered out;", output) == [str(filtered)], "wrong filtered count")
    runner.result(output.replace(f"; {filtered} filtered out;", "; 0 filtered out;"), names)


def check():
    for label, requested, features, names in CONFIGS:
        command = ["cargo", "test", "--locked", "--release", "-p", "orr_editor", "--no-default-features"]
        if requested:
            command += ["--features", requested]
        command += ["--lib", "--no-run", "--message-format=json"]
        binary = producer(source.run(label + "-build", command), label, features)
        full = execute(label, binary, "full-list", ["--list"])
        all_names = re.findall(r"^([A-Za-z0-9_:]+): test$", full.replace("\r\n", "\n"), re.M)
        require(all_names and len(all_names) == len(set(all_names)), "invalid full inventory")
        source.inventory(full, all_names)
        source.inventory(execute(label, binary, "list", [PREFIX, "--list"]), names)
        source.inventory(execute(label, binary, "ignored", [PREFIX, "--ignored", "--list"]), [])
        output = execute(label, binary, "run", [PREFIX, "--nocapture", "--test-threads=1", "--color=never"])
        result(output, names, len(all_names))
        runner.report["coverage"][label] = {"passed": len(names), "failed": 0, "ignored": 0, "names": names}
        runner.record()
    runner.report["coverage"]["unique_cases"] = len(ON)
    runner.report["coverage"]["cfg_executions"] = sum(len(config[3]) for config in CONFIGS)
    runner.record()


def self_test():
    import tempfile
    import unittest
    from unittest import mock

    class Tests(unittest.TestCase):
        def setUp(self):
            self.temp = tempfile.TemporaryDirectory()
            self.addCleanup(self.temp.cleanup)
            self.root = Path(self.temp.name).resolve()
            self.old = runner.repo, runner.evidence, runner.report, runner.frozen, runner.target_dir
            runner.repo = self.root
            runner.evidence = self.root / "evidence"
            runner.evidence.mkdir()
            runner.target_dir = self.root / "target"
            runner.report = {"producers": {}, "coverage": {}, "head": "a" * 40, "tree": "b" * 40}
            runner.frozen = {"file": "digest"}
            self.addCleanup(self.restore)

        def restore(self):
            runner.repo, runner.evidence, runner.report, runner.frozen, runner.target_dir = self.old

        def item(self):
            crate = self.root / "crates/orr_editor"
            binary = runner.target_dir / "release/deps/orr_editor-fixture"
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_bytes(b"negative-control fixture, not a Rust producer")
            return {"reason": "compiler-artifact", "package_id": "path+" + crate.as_uri() + "#orr_editor@0.0.1",
                "manifest_path": str(crate / "Cargo.toml"), "executable": str(binary),
                "target": {"name": "orr_editor", "kind": ["lib"], "crate_types": ["lib"], "src_path": str(crate / "src/lib.rs")},
                "features": [], "profile": {"test": True, "opt_level": "3"}}

        def cargo(self, item):
            return json.dumps(item) + '\n{"reason":"build-finished","success":true}\n'

        def good(self):
            return ("running 3 tests\n" + "".join(f"test {name} ... ok\n" for name in DEFAULT)
                    + "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.1s\n")

        def test_exact_inventory_matches_source(self):
            self.assertEqual([len(c[3]) for c in CONFIGS], [3, 7, 10])
            text = (Path(__file__).resolve().parents[1] / "crates/orr_editor/src/app/export_guide_tests.rs").read_text()
            self.assertTrue(all(re.search(r"^fn " + name.removeprefix(PREFIX) + r"\(", text, re.M) for name in ON))
            self.assertEqual(len(set(ON)), 10)

        def test_results_reject_missing_zero_ignored_failed_duplicate(self):
            good = self.good()
            result(good, DEFAULT, 4)
            for bad in ("", good.replace("3 passed", "0 passed"), good.replace("0 ignored", "1 ignored"),
                        good.replace("... ok", "... FAILED", 1), good.replace("... ok", "... ignored", 1),
                        good.replace("... ok", "... SKIP: unavailable\nok", 1), good + good,
                        good.replace(f"test {DEFAULT[0]} ... ok\n", ""), good.replace(DEFAULT[0], DEFAULT[1])):
                with self.assertRaises((ValueError, AssertionError)):
                    result(bad, DEFAULT, 4)

        def test_inventory_rejects_empty_or_ignored(self):
            with self.assertRaises(AssertionError):
                source.inventory("0 tests, 0 benchmarks\n", DEFAULT)
            with self.assertRaises(AssertionError):
                source.inventory(DEFAULT[0] + ": test\n1 test, 0 benchmarks\n", [])

        def test_producer_rejects_missing_failed_or_ambiguous(self):
            good = self.cargo(self.item())
            for bad in ("", '{"reason":"build-finished","success":true}\n', good + good,
                        good.replace('"success":true', '"success":false'), good.splitlines()[0]):
                with self.assertRaises(ValueError):
                    producer(bad, "invalid", [])

        def test_producer_rejects_wrong_features_target_or_path(self):
            good = self.item()
            for key, value in (("features", ["default"]), ("executable", str(self.root / "outside")),
                               ("package_id", "registry+wrong"), ("manifest_path", str(self.root / "wrong"))):
                item = copy.deepcopy(good); item[key] = value
                with self.assertRaises(ValueError):
                    producer(self.cargo(item), "invalid", [])
            item = copy.deepcopy(good); item["target"]["kind"] = ["bin"]
            with self.assertRaises(ValueError):
                producer(self.cargo(item), "invalid", [])

        def test_changed_binary_or_source_identity_rejected(self):
            binary = producer(self.cargo(self.item()), "valid", [])
            binary_fence("valid", binary)
            Path(binary).write_bytes(b"changed")
            with self.assertRaises(ValueError):
                binary_fence("valid", binary)
            with self.assertRaises(AssertionError):
                runner.source_authority("c" * 40, "b" * 40, ["file"])
            with mock.patch.object(shared, "source_paths", return_value=["file"]), mock.patch.object(runner, "source_snapshot", return_value={"file": "changed"}), self.assertRaises(ValueError):
                shared.source_fence()

        def test_failed_command_cannot_record_passing_coverage(self):
            with mock.patch.object(source, "run", side_effect=runner.GateFailure("fixture failure", 101)), self.assertRaises(runner.GateFailure):
                check()
            self.assertEqual(runner.report["coverage"], {})

        def test_workflow_retains_artifacts_in_existing_job(self):
            text = (Path(__file__).resolve().parents[1] / ".github/workflows/determinism.yml").read_text()
            sample = text.split("  sample-build:\n", 1)[1].split("\n  sample-linked-prefabs:", 1)[0]
            self.assertIn("run: python -B tools/editor_export_guide_gate.py", sample)
            self.assertIn("- name: Retain export guide CPU evidence\n        if: always()", sample)
            self.assertIn("timeout-minutes: 45", sample)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Tests)
    outcome = unittest.TextTestRunner(verbosity=2).run(suite)
    require(outcome.wasSuccessful() and outcome.testsRun == FIXTURE_COUNT and not outcome.skipped, "gate fixtures failed")
    print(f"EXPORT_GUIDE_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped", flush=True)


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    require(len(sys.argv) == 1, "usage: editor_export_guide_gate.py [--self-test]")
    error = None
    try:
        source.initialize(Path(__file__).resolve().parents[1], Path(os.environ["ORR_EXPORT_GUIDE_EVIDENCE"]), "editor-export-guide")
        runner.report["coverage"] = {}
        runner.report["limits"] = "CPU actual-App widgets and cfg routes only; no GPU, native OS input, Windows-player execution or export success claim"
        runner.record()
        fixtures = source.run("gate-fixtures", [sys.executable, "-B", __file__, "--self-test"])
        require(fixtures.rstrip().endswith(f"EXPORT_GUIDE_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped"), "fixture census mismatch")
        check()
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
