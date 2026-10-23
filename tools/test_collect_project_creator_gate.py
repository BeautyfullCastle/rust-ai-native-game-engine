#!/usr/bin/env python3
"""Independent negative fixtures for source-bound Collect creator acceptance."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import collect_project_creator_gate as gate


class GateFixtures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.saved = (gate.runner.repo, gate.runner.frozen, gate.runner.report, gate.runner.evidence, gate.runner.target_dir)
        gate.runner.repo, gate.runner.frozen, gate.runner.report, gate.runner.evidence = self.root, None, {}, self.root
        gate.runner.target_dir = self.root / "target"
        self.addCleanup(self.restore)

    def restore(self):
        gate.runner.repo, gate.runner.frozen, gate.runner.report, gate.runner.evidence, gate.runner.target_dir = self.saved

    def output(self, name=None):
        name = name or gate.CPU_TESTS[0]
        return (f"running 1 test\ntest {name} ... ok\n"
                f"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; {len(gate.ALL_TESTS)-1} filtered out; finished in 0.01s\n")

    def receipt(self):
        gate.runner.report = {"producers": {"creator-test": {"file": "creator-test"}}}
        child = {"schema": 1, "source_hidden": True, "actual_file_open_dialog": True, "fresh_app": True,
                 "installed_bytes_only": True, "files_and_identity_unchanged": True, "gameplay_win_restart_stop": True,
                 "renderer_options": "default", "native_os_keyboard": False, "current_exe": "/tmp/isolated-creator-test"}
        value = {"schema": 1, "template": "collect-dodge-ui-2d-v1", "seed": "editor-file-collect-creator-v1",
                 "game_id": "12345678-1234-4234-8234-123456789abc", "native_os_keyboard": False,
                 "renderer_options": "default", "window_pixels": list(gate.WINDOW),
                 "current_exe": str(self.root / "creator-test"), "source_hidden": child,
                 "package_files": ["orr.packages.lock.json"] +
                    [".orr/packages/objects/" + "a" * 64 + "/" + name for name in
                     ("COPYRIGHT.txt", "OFL.txt", "OrreryKoreanUI.otf", "corpus.txt", "font-manifest.json", "orr.package.json")] +
                    [".orr/packages/objects/" + "b" * 64 + "/" + name for name in
                     ("LICENSE.txt", "lantern_keeper.png", "lantern_keeper.rgba", "orr.package.json", "sprites.json")]}
        value.update({name: True for name in ("main_editor_app", "starts_in_phys_game", "actual_file_new_dialog",
                     "actual_file_open_dialog", "same_app_save_reopen", "fresh_app_source_hidden_reopen",
                     "installed_package_bytes_unchanged", "identity_and_guids_unchanged", "authoritative_win_restart_stop",
                     "exact_opaque_source_texels", "korean_preview_pixels", "physical_egui_events")})
        child_dir = self.root / "source-hidden"
        child_dir.mkdir(exist_ok=True)
        (child_dir / "receipt.json").write_text(json.dumps(child))
        (self.root / "source-hidden-child.log").write_text(self.output(gate.CHILD_TEST))
        adapter = {"name": "llvmpipe fixture", "backend": "Vulkan", "device_type": "Cpu", "renderer_options": "default",
                   "predictable_texture_filtering": False, "dithering": True}
        for folder, frames, previews in ((self.root, gate.MAIN_FRAMES, gate.PREVIEW_IMAGES),
                                        (child_dir, gate.CHILD_FRAMES, ("saved-korean-hud",))):
            (folder / "adapter.json").write_text(json.dumps(adapter))
            for name in (*frames, *previews):
                (folder / (name + ".png")).write_bytes(name.encode())
            for name in frames:
                won = name == "won"
                tick = {"playing": 1, "won": 20, "restarted": 21}.get(name, 0)
                frame = {"phase": int(won), "score": 2 if won else 0, "tick": tick, "verified_tick": tick,
                         "checksum": ("1" if name == "created" else "2") * 16, "window_pixels": list(gate.WINDOW),
                         "viewport": [200, 80, 1000, 900], "viewport_pixels": [1000, 900], "exact_opaque_source_texels": True,
                         "actors": [{"guid": "e_" + f"{i:032x}", "region": i, "position": [i, 0],
                                     "opaque_texels": 50} for i in range(2 if won else 4)]}
                (folder / (name + ".json")).write_text(json.dumps(frame))
        (self.root / "korean-preview.json").write_text(json.dumps({"canvas": [100, 100, 640, 360],
            "changed_korean_pixels": 1, "changes_confined_to_title_geometry": True,
            "undo_restores_exact_canvas_pixels": True, "host_and_files_unchanged": True}))
        return value

    def pixels(self, path, dimensions):
        self.assertEqual(tuple(dimensions), gate.WINDOW)
        image = bytearray(gate.WINDOW[0] * gate.WINDOW[1] * 4)
        if path.stem in ("korean-title", "korean-title-restored"):
            at = (170 * gate.WINDOW[0] + 400) * 4
            image[at:at + 4] = b"\xff\xff\xff\xff"
        return bytes(image)

    def write_receipt(self, value):
        (self.root / "collect-creator-acceptance.json").write_text(json.dumps(value))

    def test_fixed_reviewed_inventory_is_nonempty_and_unique(self):
        self.assertEqual((len(gate.CPU_TESTS), len(gate.ALL_TESTS), len(gate.IGNORED_TESTS)), (3, 5, 2))
        self.assertEqual(len(gate.ALL_TESTS), len(set(gate.ALL_TESTS)))
        self.assertIn(gate.GPU_TEST, gate.IGNORED_TESTS)
        self.assertEqual(gate.REQUESTED_FEATURES, "project-create,collect-ui")

    def test_exact_pass_accepts_only_one_named_case_and_crlf(self):
        gate.validate_exact(self.output(), gate.CPU_TESTS[0], len(gate.ALL_TESTS))
        gate.validate_exact(self.output().replace("\n", "\r\n"), gate.CPU_TESTS[0], len(gate.ALL_TESTS))

    def test_zero_ignored_failed_extra_and_wrong_cases_reject(self):
        good = self.output()
        for bad in ("", good.replace("1 passed", "0 passed"), good.replace("0 ignored", "1 ignored"),
                    good.replace("0 failed", "1 failed"), good.replace("... ok", "... ignored"),
                    good.replace(gate.CPU_TESTS[0], "wrong"), good * 2, good.replace("running 1 test", "running 0 tests")):
            with self.subTest(output=bad), self.assertRaises((ValueError, AssertionError)):
                gate.validate_exact(bad, gate.CPU_TESTS[0], len(gate.ALL_TESTS))

    def test_filtered_count_and_empty_authority_reject(self):
        for total in (0, True, len(gate.ALL_TESTS) + 1):
            with self.assertRaises(ValueError):
                gate.validate_exact(self.output(), gate.CPU_TESTS[0], total)
        with self.assertRaises(ValueError):
            gate.validate_exact(self.output(), "unreviewed", len(gate.ALL_TESTS))
        with self.assertRaises(ValueError):
            gate.validate_exact(self.output().replace(f"{len(gate.ALL_TESTS)-1} filtered", "99 filtered"), gate.CPU_TESTS[0], len(gate.ALL_TESTS))

    def test_skip_diagnostics_and_malformed_nested_harness_reject(self):
        good = self.output()
        for message in ("SKIP: unavailable", "adapter unavailable", "running 1 test", "test other ... ok"):
            bad = good.replace("... ok", "... " + message + "\nok")
            with self.subTest(message=message), self.assertRaises((ValueError, AssertionError)):
                gate.validate_exact(bad, gate.CPU_TESTS[0], len(gate.ALL_TESTS))

    def test_complete_and_ignored_inventories_are_required(self):
        def listed(names):
            return "".join(name + ": test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n"
        with mock.patch.object(gate, "run", side_effect=[listed(gate.ALL_TESTS), listed(gate.IGNORED_TESTS)]), mock.patch.object(gate.runner, "record"):
            gate.inventory("binary")
        for ignored in ((), gate.ALL_TESTS):
            with mock.patch.object(gate, "run", side_effect=[listed(gate.ALL_TESTS), listed(ignored)]), self.assertRaises(AssertionError):
                gate.inventory("binary")

    def test_explicit_ignored_execution_never_counts_as_ordinary_cpu(self):
        with mock.patch.object(gate, "run", return_value=self.output(gate.GPU_TEST)) as run, mock.patch.object(gate.runner, "record"):
            gate.execute("pinned", gate.GPU_TEST)
        self.assertIn("--ignored", run.call_args.args[1])
        self.assertIn("--exact", run.call_args.args[1])
        self.assertNotIn(gate.GPU_TEST, gate.CPU_TESTS)

    def test_cargo_completion_is_exact_successful_last_record(self):
        good = '{"reason":"compiler-artifact"}\n{"reason":"build-finished","success":true}\n'
        gate.validate_build(good)
        for bad in ("", good * 2, good.replace("true", "false"), good + '{"reason":"late"}\n',
                    good.replace('"success":true', '"success":1'),
                    good.replace('"success":true', '"success":true,"success":false')):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                gate.validate_build(bad)

    def test_feature_boundaries_are_checks_and_strict_not_zero_test_passes(self):
        commands = dict(gate.boundary_commands())
        self.assertEqual(len(commands), 10)
        for profile in ("default", "feature-off", "creator-only", "ui-only", "creator-ui"):
            check, strict = commands[profile + "-check"], commands[profile + "-strict"]
            self.assertEqual(check[:2], ["cargo", "check"])
            self.assertEqual(strict[:2], ["cargo", "clippy"])
            self.assertEqual(strict[-3:], ["--", "-D", "warnings"])
            self.assertIn(gate.TARGET, strict)
        self.assertNotIn("--no-default-features", commands["default-check"])
        self.assertIn("--no-default-features", commands["feature-off-check"])

    def test_workflow_uses_step_context_and_identical_artifact_path(self):
        gate.validate_workflow(gate.WORKFLOW.read_text())

    def test_workflow_rejects_job_runner_context_and_wrong_upload_path(self):
        good = gate.WORKFLOW.read_text()
        bads = (good.replace("    env:\n", "    env:\n      BAD: ${{ runner.temp }}/bad\n", 1),
                good.replace("          ORR_COLLECT_CREATOR_GATE_EVIDENCE:", "      ORR_COLLECT_CREATOR_GATE_EVIDENCE:"),
                good.replace("          path: " + gate.EVIDENCE_EXPRESSION, "          path: ${{ env.ORR_COLLECT_CREATOR_GATE_EVIDENCE }}"),
                good.replace("          path: " + gate.EVIDENCE_EXPRESSION, "          path: " + gate.EVIDENCE_EXPRESSION + "-wrong"))
        for bad in bads:
            with self.assertRaises(ValueError):
                gate.validate_workflow(bad)

    def test_workflow_keeps_required_lanes_and_partial_failure_artifacts(self):
        good = gate.WORKFLOW.read_text()
        for old, new in (("mode: [cpu, gpu]", "mode: [cpu]"), ("needs: acceptance", "needs: missing"),
                         ('test "$ACCEPTANCE" = success', "true"), ("if-no-files-found: error", "if-no-files-found: ignore"),
                         ("timeout-minutes: 90", "timeout-minutes: 90\n    continue-on-error: true")):
            with self.subTest(old=old), self.assertRaises(ValueError):
                gate.validate_workflow(good.replace(old, new))

    def test_duplicate_json_keys_nonfinite_values_and_wrong_schema_reject(self):
        for raw in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":Infinity}'):
            with self.assertRaises(ValueError):
                gate.load_json(raw)
        value = self.receipt()
        value["schema"] = True
        self.write_receipt(value)
        with self.assertRaises(ValueError):
            gate.validate_gpu(self.root)

    def test_missing_gpu_proof_or_native_input_claim_rejects(self):
        value = self.receipt()
        with mock.patch.object(gate, "png_rgba", side_effect=self.pixels):
            self.write_receipt(value)
            gate.validate_gpu(self.root)
            for key in value:
                bad = copy.deepcopy(value)
                del bad[key]
                self.write_receipt(bad)
                with self.subTest(key=key), self.assertRaises(ValueError):
                    gate.validate_gpu(self.root)
            value["native_os_keyboard"] = True
            self.write_receipt(value)
            with self.assertRaises(ValueError):
                gate.validate_gpu(self.root)

    def test_hardware_wrong_dimensions_hash_and_duplicate_capture_reject(self):
        value = self.receipt()
        self.write_receipt(value)
        adapter_file = self.root / "adapter.json"
        adapter = json.loads(adapter_file.read_text())
        with mock.patch.object(gate, "png_rgba", side_effect=self.pixels):
            for key, changed in (("name", "physical GPU"), ("device_type", "DiscreteGpu"), ("backend", "Gl"),
                                 ("renderer_options", "predictable"), ("predictable_texture_filtering", True), ("dithering", False)):
                bad = {**adapter, key: changed}
                adapter_file.write_text(json.dumps(bad))
                with self.assertRaises(ValueError):
                    gate.validate_gpu(self.root)
            adapter_file.write_text(json.dumps(adapter))
            for key, changed in (("window_pixels", [1600, 1000]), ("current_exe", "/tmp/stale-executable"),
                                 ("package_files", ["orr.packages.lock.json"]),
                                 ("package_files", [name.replace("OFL.txt", "OMITTED.txt") for name in value["package_files"]]),
                                 ("package_files", [name.replace("/objects/", "/objects/../") for name in value["package_files"]]),
                                 ("game_id", "other")):
                bad = copy.deepcopy(value)
                bad[key] = changed
                self.write_receipt(bad)
                with self.assertRaises(ValueError):
                    gate.validate_gpu(self.root)

    def test_missing_extra_symlink_and_corrupted_png_reject(self):
        value = self.receipt()
        self.write_receipt(value)
        with self.assertRaises(ValueError):
            gate.validate_gpu(self.root)
        first = self.root / "created.png"
        first.unlink()
        with mock.patch.object(gate, "png_rgba", side_effect=self.pixels):
            with self.assertRaises(ValueError):
                gate.validate_gpu(self.root)
            first.symlink_to(self.root / "same-app-reopened.png")
            with mock.patch.object(gate, "png_rgba", wraps=gate.shared.png_rgba), self.assertRaises(ValueError):
                gate.validate_gpu(self.root)
            first.unlink()
            first.write_bytes(b"created")
            (self.root / "extra.png").write_bytes(b"extra")
            with self.assertRaises(ValueError):
                gate.validate_gpu(self.root)

    def test_private_guard_inventory_is_fixed_and_complete(self):
        self.assertEqual([len(names) for names in gate.LIBRARY_SUITES.values()], [8, 3, 15, 5])
        self.assertEqual(sum(map(len, gate.LIBRARY_SUITES.values())), 31)
        for names in gate.LIBRARY_SUITES.values():
            self.assertEqual(len(names), len(set(names)))
        self.assertIn("changed_or_missing_saved_scene_blocks_even_when_host_is_clean",
                      gate.LIBRARY_SUITES["editor::collect_project::tests"])

    def test_library_artifact_rejects_cached_wrong_source_features_and_profile(self):
        binary = gate.runner.target_dir / "release/deps/orr_editor-123"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"compiled test fixture")
        crate = self.root / "crates/orr_editor"
        item = {"reason": "compiler-artifact", "fresh": False, "manifest_path": str(crate / "Cargo.toml"),
                "package_id": "path+" + crate.as_uri() + "#orr_editor@0.0.1", "features": gate.FEATURES,
                "executable": str(binary), "profile": {"test": True, "opt_level": "3"},
                "target": {"name": "orr_editor", "kind": ["lib"], "crate_types": ["lib"], "src_path": str(crate / "src/lib.rs")}}
        gate.runner.report = {"producers": {}, "source_inventory_sha256": "a" * 64}
        with mock.patch.object(gate.runner, "record"):
            for key, changed in (("fresh", True), ("features", []), ("package_id", "registry+wrong"),
                                 ("manifest_path", str(self.root / "Cargo.toml")),
                                 ("profile", {"test": True, "opt_level": "0"}),
                                 ("target", {"name": "orr_editor", "kind": ["bin"]})):
                bad = copy.deepcopy(item)
                bad[key] = changed
                with self.subTest(key=key), self.assertRaises(ValueError):
                    gate.pin_library(json.dumps(bad))
            copied = gate.pin_library(json.dumps(item))
            self.assertEqual(Path(copied).read_bytes(), binary.read_bytes())
            self.assertEqual(gate.runner.report["producers"]["creator-library"]["source_inventory_sha256"], "a" * 64)

    def test_source_hidden_child_requires_exact_pass_and_matching_receipt(self):
        value = self.receipt()
        self.write_receipt(value)
        child_log = self.root / "source-hidden-child.log"
        with mock.patch.object(gate, "png_rgba", side_effect=self.pixels):
            for bad in ("", self.output(gate.CHILD_TEST).replace("1 passed", "0 passed"), self.output(gate.GPU_TEST)):
                child_log.write_text(bad)
                with self.assertRaises((ValueError, AssertionError)):
                    gate.validate_gpu(self.root)
            child_log.write_text(self.output(gate.CHILD_TEST))
            child = value["source_hidden"]
            for key in child:
                bad = copy.deepcopy(child)
                del bad[key]
                (self.root / "source-hidden/receipt.json").write_text(json.dumps(bad))
                value["source_hidden"] = bad
                self.write_receipt(value)
                with self.subTest(key=key), self.assertRaises(ValueError):
                    gate.validate_gpu(self.root)

    def test_korean_negative_control_and_exact_undo_pixels(self):
        self.receipt()
        images = {name: self.pixels(self.root / (name + ".png"), gate.WINDOW) for name in gate.PREVIEW_IMAGES[:3]}
        gate.korean_preview(self.root, images)
        for name, replacement in (("title-without-korean", images["korean-title"]),
                                  ("korean-title-restored", images["title-without-korean"])):
            with self.assertRaises(ValueError):
                gate.korean_preview(self.root, {**images, name: replacement})
        outside = bytearray(images["korean-title"])
        outside[(105 * gate.WINDOW[0] + 105) * 4] = 1
        with self.assertRaises(ValueError):
            gate.korean_preview(self.root, {**images, "korean-title": bytes(outside), "korean-title-restored": bytes(outside)})

    def test_gpu_authoritative_states_and_exact_phase_inventory(self):
        value = self.receipt()
        self.write_receipt(value)
        path = self.root / "won.json"
        original = json.loads(path.read_text())
        for key, bad in (("phase", 0), ("phase", True), ("score", 0), ("tick", -1), ("verified_tick", 99),
                         ("checksum", "invalid"), ("actors", []), ("exact_opaque_source_texels", False),
                         ("viewport", [-1, 0, 30, 30]), ("viewport_pixels", [0, 10])):
            path.write_text(json.dumps({**original, key: bad}))
            with self.subTest(key=key), self.assertRaises(ValueError):
                gate.frame(self.root, "won")
        for bad in ("sprite", True, -1, 4294967296):
            changed = copy.deepcopy(original)
            changed["actors"][0]["region"] = bad
            path.write_text(json.dumps(changed))
            with self.assertRaises(ValueError):
                gate.frame(self.root, "won")
        path.write_text(json.dumps(original))
        (self.root / "source-hidden/saved-korean-hud.png").unlink()
        with mock.patch.object(gate, "png_rgba", side_effect=self.pixels), self.assertRaises(ValueError):
            gate.validate_gpu(self.root)

    def test_source_addition_mutation_and_mode_changes_reject(self):
        source = self.root / "source.rs"
        source.write_text("original")
        gate.runner.frozen = gate.runner.source_snapshot(["source.rs"])
        gate.source_fence()
        source.write_text("changed")
        with self.assertRaises(ValueError):
            gate.source_fence()
        source.write_text("original")
        source.chmod(0o700)
        with self.assertRaises(ValueError):
            gate.source_fence()
        source.chmod(gate.runner.frozen["source.rs"]["mode"])
        (self.root / "untracked.rs").write_text("new")
        with self.assertRaises(ValueError):
            gate.source_fence()

    def test_python_optimization_cannot_disable_fail_closed_assertions(self):
        result = subprocess.run([sys.executable, "-B", "-O", str(Path(gate.__file__))],
                                capture_output=True, text=True, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires nonoptimized Python", result.stdout + result.stderr)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GateFixtures)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.skipped or result.testsRun != gate.FIXTURE_COUNT:
        raise SystemExit(1)
    print(f"COLLECT_CREATOR_GATE_FIXTURES: {result.testsRun} passed; 0 failed; 0 skipped")
