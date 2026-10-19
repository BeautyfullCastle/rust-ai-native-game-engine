#!/usr/bin/env python3
"""Independent failure-injection fixtures for named-clips evidence admission."""
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import tempfile
import unittest
from unittest import mock
import zlib

import png_sequence_clips_gate as gate


def chunk(kind, payload):
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))


def png(width, height, pixels, filters=None):
    stride, raw, previous = width * 4, bytearray(), bytes(width * 4)
    for y in range(height):
        method = filters[y] if filters else 0
        current = pixels[y * stride:(y + 1) * stride]
        raw.append(method)
        for x, value in enumerate(current):
            left, up, corner = current[x - 4] if x >= 4 else 0, previous[x], previous[x - 4] if x >= 4 else 0
            estimate = left + up - corner
            distances = (abs(estimate - left), abs(estimate - up), abs(estimate - corner))
            predictor = (0, left, up, (left + up) // 2, (left, up, corner)[distances.index(min(distances))])[method]
            raw.append((value - predictor) & 255)
        previous = current
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw))) + chunk(b"IEND", b""))


class GateFixtures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.saved = (gate.runner.repo, gate.runner.frozen, gate.runner.report, gate.runner.evidence)
        gate.runner.repo, gate.runner.frozen, gate.runner.report, gate.runner.evidence = self.root, None, {}, self.root
        self.addCleanup(self.restore)

    def restore(self):
        gate.runner.repo, gate.runner.frozen, gate.runner.report, gate.runner.evidence = self.saved

    def write_receipt(self, value):
        (self.root / "receipt.json").write_text(json.dumps(value))

    def test_fixed_inventory_is_nonempty_unique_and_keeps_private_parser(self):
        self.assertEqual((len(gate.CPU_TESTS), len(gate.SAMPLE_TESTS), len(gate.EDITOR_TESTS)), (18, 1, 1))
        for names in (gate.CPU_TESTS, gate.SAMPLE_TESTS, gate.EDITOR_TESTS):
            self.assertEqual(len(names), len(set(names)))
        self.assertIn("crates/orr_sample/examples/png_sequence_package/clips.rs", gate.FEATURE_PATHS)
        self.assertIn("crates/orr_editor/tests/png_sequence_clips_ui.rs", gate.FEATURE_PATHS)
        self.assertEqual(len(gate.sample_inventory()), 675)
        self.assertEqual(len(gate.EDITOR_CAPTURES), 5)

    def test_empty_inventory_cannot_be_acceptance(self):
        with self.assertRaises(ValueError):
            gate.tests("bad", "binary", [])
        output = "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n"
        with mock.patch.object(gate, "run", side_effect=["0 tests, 0 benchmarks\n", "0 tests, 0 benchmarks\n", output]), mock.patch.object(gate.runner, "record"):
            gate.tests("boundary", "binary", [], boundary=True)
        self.assertTrue(gate.runner.report["accepted_tests"]["boundary"]["boundary_only"])

    def test_exact_test_inventory_rejects_extra_missing_and_ignored(self):
        good = "required: test\n1 test, 0 benchmarks\n"
        gate.runner.inventory(good, ["required"])
        for bad in ("0 tests, 0 benchmarks\n", good + "extra: test\n", good.replace("required", "other")):
            with self.assertRaises(AssertionError):
                gate.runner.inventory(bad, ["required"])
        with mock.patch.object(gate, "run", side_effect=[good, good]):
            with self.assertRaises(AssertionError):
                gate.tests("ordinary", "binary", ["required"])

    def test_executed_inventory_rejects_skip_filtered_and_false_success(self):
        good = "running 1 test\ntest required ... diagnostic\nok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n"
        gate.runner.result(good, ["required"])
        for bad in ("", good.replace("diagnostic", "SKIP: adapter"), good.replace("0 filtered", "1 filtered"),
                    good.replace("required", "wrong"), good.replace("\nok\n", "\nignored\n"), good + good):
            with self.assertRaises(AssertionError):
                gate.runner.result(bad, ["required"])

    def test_json_duplicate_keys_and_nonfinite_values_reject(self):
        for value in ('{"schema":1,"schema":1}', '{"nested":{"a":1,"a":1}}', '{"n":NaN}', '{"n":Infinity}'):
            with self.assertRaises(ValueError):
                gate.load_json(value)

    def test_source_bytes_modes_and_untracked_paths_are_fenced(self):
        path = self.root / "source.rs"
        path.write_text("source")
        gate.runner.frozen = gate.runner.source_snapshot(["source.rs"])
        gate.source_fence()
        path.write_text("changed")
        with self.assertRaises(ValueError):
            gate.source_fence()
        path.write_text("source")
        path.chmod(0o700)
        with self.assertRaises(ValueError):
            gate.source_fence()
        gate.runner.frozen = gate.runner.source_snapshot(["source.rs"])
        (self.root / "ignored.rs").write_text("injected")
        with self.assertRaises(ValueError):
            gate.source_fence()

    def test_source_symlinks_are_rejected(self):
        (self.root / "real").write_text("source")
        (self.root / "alias").symlink_to("real")
        with self.assertRaises(ValueError):
            gate.source_paths()

    def test_cargo_must_finish_successfully_exactly_once_before_pin(self):
        finish = '{"reason":"build-finished","success":true}\n'
        for bad in ("", finish.replace("true", "false"), finish + finish, finish + '{"reason":"compiler-artifact"}\n'):
            with mock.patch.object(gate, "run", return_value=bad), mock.patch.object(gate.runner, "pin") as pin:
                with self.assertRaises((ValueError, IndexError)):
                    gate.build("cpu", "orr_sample", gate.CPU_TARGET, "test", gate.SAMPLE_FEATURES)
                pin.assert_not_called()

    def test_editor_build_requests_only_declared_features(self):
        finish = '{"reason":"build-finished","success":true}\n'
        gate.runner.report = {"producers": {"ui": {}}, "source_inventory_sha256": "frozen"}
        with mock.patch.object(gate, "run", return_value=finish) as run, mock.patch.object(gate.runner, "pin", return_value="accepted-ui"), mock.patch.object(gate.runner, "record"):
            self.assertEqual(gate.build("ui", "orr_editor", gate.EDITOR_TARGET, "test", gate.EDITOR_FEATURES), "accepted-ui")
        command = run.call_args.args[1]
        self.assertIn("--no-run", command)
        self.assertEqual(command[command.index("--features") + 1], "sprites")
        self.assertEqual(gate.runner.report["producers"]["ui"]["source_inventory_sha256"], "frozen")

    def test_cpu_requires_the_newly_built_real_binary(self):
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(gate, "build", return_value="accepted-cli"), mock.patch.object(gate, "run", return_value="png_sequence_package --recipe PATH --out NEW_DIR"):
            gate.build_cli()
            self.assertEqual(os.environ["ORR_PNG_SEQUENCE_PACKAGE_BIN"], "accepted-cli")
            self.assertEqual(os.environ["ORR_REQUIRE_PNG_SEQUENCE_CLIPS"], "1")

    def test_refusal_is_exact_and_cannot_create_output(self):
        refusal = "png_sequence_package requires Linux and --features project-create\n"
        with mock.patch.object(gate, "run", return_value=refusal):
            gate.refusal("binary", "disabled")
        for bad in ("", "requires project-create", refusal + "opened input\n"):
            with mock.patch.object(gate, "run", return_value=bad):
                with self.assertRaises(ValueError):
                    gate.refusal("binary", "disabled")
        def io(*args, **kwargs):
            (self.root / "disabled-must-not-exist").mkdir()
            return refusal
        with mock.patch.object(gate, "run", side_effect=io):
            with self.assertRaises(ValueError):
                gate.refusal("binary", "disabled")

    def manifest_fixture(self):
        path = self.root / "lvp_icd.json"
        value = {"file_format_version": "1.0.0", "ICD": {"api_version": "1.4.305", "library_path": "libvulkan_lvp.so"}}
        path.write_text(json.dumps(value))
        return path, value

    def test_explicit_vulkan_manifest_and_single_system_fallback(self):
        path, _ = self.manifest_fixture()
        for environment in ({"VK_DRIVER_FILES": str(path)}, {"VK_ICD_FILENAMES": str(path)},
                            {"VK_DRIVER_FILES": str(path), "VK_ICD_FILENAMES": str(path)}, {}):
            selected, raw = gate.software_manifest(environment, self.root)
            self.assertEqual(selected, path)
            self.assertEqual(raw, path.read_bytes())
        (self.root / "lvp_icd.second.json").write_bytes(path.read_bytes())
        with self.assertRaises(ValueError):
            gate.software_manifest({}, self.root)

    def test_explicit_vulkan_rejects_paths_lists_symlinks_and_invalid_manifest(self):
        path, value = self.manifest_fixture()
        for environment in ({"VK_DRIVER_FILES": ""}, {"VK_DRIVER_FILES": str(path) + ":" + str(path)},
                            {"VK_DRIVER_FILES": str(path) + ";" + str(path)}, {"VK_DRIVER_FILES": "lvp_icd.json"},
                            {"VK_DRIVER_FILES": str(path), "VK_ICD_FILENAMES": str(self.root / "other.json")},
                            {"VK_DRIVER_FILES": str(self.root / "missing.json")}):
            with self.assertRaises(ValueError):
                gate.software_manifest(environment, self.root)
        alias = self.root / "lvp_icd.alias.json"
        alias.symlink_to(path.name)
        with self.assertRaises(ValueError):
            gate.software_manifest({"VK_DRIVER_FILES": str(alias)}, self.root)
        for bad in (b"not JSON", b" " * 65537, b'{"ICD":{},"ICD":{}}',
                    json.dumps({**value, "file_format_version": "2.0.0"}).encode(),
                    json.dumps({**value, "ICD": {"api_version": "1.4.0", "library_path": "other-driver.so"}}).encode()):
            path.write_bytes(bad)
            with self.assertRaises(ValueError):
                gate.software_manifest({"VK_DRIVER_FILES": str(path)}, self.root)

    def test_software_selection_records_hash_and_keeps_gpu_mandatory(self):
        path, _ = self.manifest_fixture()
        with mock.patch.dict(os.environ, {"VK_DRIVER_FILES": str(path)}, clear=True), mock.patch.object(gate.runner, "record"):
            gate.software()
            self.assertEqual(os.environ["VK_DRIVER_FILES"], os.environ["VK_ICD_FILENAMES"])
            self.assertEqual(os.environ["ORR_REQUIRE_GPU"], "1")
            self.assertEqual(os.environ["WGPU_BACKEND"], "vulkan")
            self.assertEqual(gate.runner.report["icd_sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertEqual((self.root / "lavapipe-icd.json").read_bytes(), path.read_bytes())

    def test_clock_boundaries_and_host_epochs_are_independent(self):
        self.assertEqual([gate.sample_frame(t, False, "good") for t in (0, 99, 100, 249, 250)], [0, 0, 2, 2, 0])
        self.assertEqual([gate.sample_frame(t, True, "good") for t in (0, 69, 70, 199, 200, 400, 2**64 - 1)], [1, 1, 2, 2, 1, 1, 1])
        self.assertEqual(gate.sample_frame(100, True, "wrong-duration"), 1)
        self.assertEqual(gate.sample_frame(0, True, "wrong-order"), 2)
        self.assertEqual(gate.sample_time("good-host-21.png"), ("good", "host", 0, True))
        self.assertEqual(gate.sample_time("good-host-75.png"), ("good", "host", 0, False))
        self.assertEqual(gate.sample_time("good-host-104.png"), ("good", "host", 483, False))

    def test_pixel_oracle_rejects_blank_wrong_source_and_wrong_registration(self):
        images = [gate.sample_oracle(frame, dx) for frame in range(3) for dx in range(2)]
        self.assertEqual(len(set(images)), 6)
        for image in images:
            self.assertEqual(len(image), 128 * 128 * 4)
            self.assertTrue(gate.pixel_matches(image, image))
            self.assertFalse(gate.pixel_matches(bytes(len(image)), image))
        self.assertFalse(gate.pixel_matches(images[0], images[1]))
        self.assertFalse(gate.pixel_matches(images[0], images[2]))

    def test_png_all_five_filters_roundtrip(self):
        pixels = bytes((index * 47 + 13) % 256 for index in range(4 * 4 * 5))
        path = self.root / "capture.png"
        path.write_bytes(png(4, 5, pixels, filters=list(range(5))))
        self.assertEqual(gate.png_rgba(path, (4, 5)), pixels)

    def test_png_rejects_corruption_trailing_stream_wrong_size_and_symlink(self):
        path = self.root / "capture.png"
        good = png(2, 2, bytes(range(16)))
        for bad in (b"", b"not PNG", good[:-1], good[:-1] + bytes([good[-1] ^ 1]), good + b"trailing",
                    good.replace(chunk(b"IEND", b""), chunk(b"IHDR", b"x") + chunk(b"IEND", b""))):
            path.write_bytes(bad)
            with self.assertRaises((ValueError, zlib.error)):
                gate.png_rgba(path, (2, 2))
        path.write_bytes(good)
        with self.assertRaises(ValueError):
            gate.png_rgba(path, (3, 2))
        (self.root / "alias.png").symlink_to(path.name)
        with self.assertRaises(ValueError):
            gate.png_rgba(self.root / "alias.png", (2, 2))
        data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(18)) + zlib.compress(b"hidden")) + chunk(b"IEND", b"")
        path.write_bytes(data)
        with self.assertRaises(ValueError):
            gate.png_rgba(path, (2, 2))

    def sample_fixture(self):
        oracles = {(frame, dx): gate.sample_oracle(frame, dx) for frame in range(3) for dx in range(2)}
        encoded = {key: png(128, 128, pixels) for key, pixels in oracles.items()}
        records, controls, pixels_by_name = [], {case: {"case": case, "direct_rejected": 0, "host_rejected": 0} for case in gate.CASES}, {}
        for name in sorted(gate.sample_inventory()):
            case, phase, elapsed, moving = gate.sample_time(name)
            frame = gate.sample_frame(elapsed, moving != (case == "wrong-mapping"), case)
            key = (frame, int(case == "wrong-offset"))
            data = encoded[key]
            (self.root / name).write_bytes(data)
            records.append({"file": name, "sha256": hashlib.sha256(data).hexdigest()})
            pixels_by_name[name] = oracles[key]
            good = oracles[gate.sample_frame(elapsed, moving, "good"), 0]
            controls[case][phase + "_rejected"] += int(oracles[key] != good)
        value = dict(schema=1, test=gate.SAMPLE_TESTS[0], adapter="llvmpipe fixture", backend="Vulkan", device_type="Cpu",
                     is_software=True, installed_bytes_preserved=True, source_hidden=True, same_document_locomotion=True,
                     every_host_tick_state_and_checksum_equal=True, capture_count=675, captures=records,
                     negative_controls=list(controls.values()))
        self.write_receipt(value)
        return value, pixels_by_name

    def fast_sample(self, pixels):
        return mock.patch.object(gate, "png_rgba", side_effect=lambda path, size: pixels[path.name])

    def test_complete_sample_receipt_and_real_pixel_payloads(self):
        value, _ = self.sample_fixture()
        self.assertEqual(gate.validate_sample(self.root), value)

    def test_sample_receipt_rejects_wrong_proofs_inventory_and_control_counts(self):
        original, pixels = self.sample_fixture()
        mutations = [lambda v: v.update(schema=True), lambda v: v.update(test="wrong"), lambda v: v.update(adapter="hardware"),
                     lambda v: v.update(backend="Gl"), lambda v: v.update(device_type="DiscreteGpu"), lambda v: v.update(is_software=False),
                     lambda v: v.update(source_hidden=False), lambda v: v.update(installed_bytes_preserved=False),
                     lambda v: v.update(every_host_tick_state_and_checksum_equal=False), lambda v: v.update(capture_count=674),
                     lambda v: v["captures"].pop(), lambda v: v["captures"].__setitem__(0, v["captures"][1]),
                     lambda v: v["captures"][0].update(file="../outside.png"), lambda v: v["captures"][0].update(sha256="0" * 64),
                     lambda v: v["negative_controls"][0].update(direct_rejected=True),
                     lambda v: v["negative_controls"][1].update(direct_rejected=0), lambda v: v["negative_controls"].pop()]
        with self.fast_sample(pixels), mock.patch.object(gate, "pixel_matches", side_effect=lambda a, b: a == b):
            for mutate in mutations:
                value = copy.deepcopy(original)
                mutate(value)
                self.write_receipt(value)
                with self.assertRaises(ValueError):
                    gate.validate_sample(self.root)

    def test_sample_files_missing_extra_symlink_and_hash_reject(self):
        value, pixels = self.sample_fixture()
        first = self.root / value["captures"][0]["file"]
        original = first.read_bytes()
        with self.fast_sample(pixels), mock.patch.object(gate, "pixel_matches", side_effect=lambda a, b: a == b):
            first.unlink()
            with self.assertRaises(ValueError):
                gate.validate_sample(self.root)
            first.symlink_to(value["captures"][1]["file"])
            with self.assertRaises(ValueError):
                gate.validate_sample(self.root)
            first.unlink()
            first.write_bytes(original + b"tampered")
            with self.assertRaises(ValueError):
                gate.validate_sample(self.root)
            first.write_bytes(original)
            (self.root / "extra.png").write_bytes(original)
            with self.assertRaises(ValueError):
                gate.validate_sample(self.root)

    def test_sample_blank_replacement_rejects_even_with_updated_hash(self):
        value, _ = self.sample_fixture()
        entry = value["captures"][0]
        blank = png(128, 128, bytes(128 * 128 * 4))
        (self.root / entry["file"]).write_bytes(blank)
        entry["sha256"] = hashlib.sha256(blank).hexdigest()
        self.write_receipt(value)
        with self.assertRaises(ValueError):
            gate.validate_sample(self.root)

    def editor_fixture(self):
        captures, pixels = [], {}
        for index, name in enumerate(gate.EDITOR_CAPTURES):
            file = name + ".png"
            (self.root / file).write_bytes(b"image fixture " + bytes([index]))
            moving = name == "controlled-walk"
            elapsed = 350 if moving else (50 if name == "released-idle" else 0)
            frame = 1 if name == "wrong-mapping" else gate.sample_frame(elapsed, moving, "good")
            probes = [[620 + x * 52, 581 + y * 52] for y in range(5) for x in range(6)]
            captures.append(dict(file=file, width=1500, height=1400, source_frame=frame, tick=40 + index, probe_pixels=probes,
                                 elapsed_ms=elapsed, moving=moving, viewport=[230., 90., 1270., 1280.], checksum="0x1234567890abcdef"))
            raw = bytearray([index]) * (1500 * 1400 * 4)
            for (x, y), color in zip(probes, gate.editor_source(frame)):
                at = (y * 1500 + x) * 4
                if color[3] == 255:
                    raw[at:at + 4] = bytes(color)
            pixels[file] = bytes(raw)
        value = dict(schema=1, test=gate.EDITOR_TESTS[0], adapter="llvmpipe fixture", backend="Vulkan", device_type="Cpu",
                     main_editor_app=True, source_hidden=True, installed_bytes_preserved=True, real_clip_widgets=True,
                     saved_and_reopened=True, real_keyboard_move_and_release=True, presentation_free_state_and_checksum_equal=True,
                     compared_live_frames=3, recorded_ticks=70, capture_count=5, captures=captures)
        self.write_receipt(value)
        return value, pixels

    def test_editor_metadata_inventory_and_phase_proofs(self):
        original, pixels = self.editor_fixture()
        with mock.patch.object(gate, "png_rgba", side_effect=lambda path, size: pixels[path.name]):
            _, hashes = gate.validate_editor(self.root)
            self.assertEqual(len(hashes), 5)
            mutations = [lambda v: v.update(main_editor_app=False), lambda v: v.update(saved_and_reopened=False),
                         lambda v: v.update(real_keyboard_move_and_release=False), lambda v: v.update(compared_live_frames=1),
                         lambda v: v.update(recorded_ticks=0), lambda v: v.update(capture_count=4),
                         lambda v: v["captures"][0].update(source_frame=0), lambda v: v["captures"][0].update(width=1400),
                         lambda v: v["captures"][0].update(viewport=[0, 0, 9999, 1]), lambda v: v["captures"][0].update(checksum="bad"),
                         lambda v: v["captures"][3].update(moving=False), lambda v: v["captures"][3].update(elapsed_ms=0),
                         lambda v: v["captures"][3].update(tick=None), lambda v: v["captures"][4].update(elapsed_ms=0), lambda v: v["captures"].__setitem__(0, v["captures"][1]),
                         lambda v: v["captures"][0].update(probe_pixels=[]),
                         lambda v: v["captures"][0]["probe_pixels"].__setitem__(0, [0, 0]),
                         lambda v: v["captures"][0]["probe_pixels"].__setitem__(0, [700, 700])]
            for mutate in mutations:
                value = copy.deepcopy(original)
                mutate(value)
                self.write_receipt(value)
                with self.assertRaises(ValueError):
                    gate.validate_editor(self.root)

    def test_editor_missing_extra_symlink_and_blank_captures_reject(self):
        value, pixels = self.editor_fixture()
        first = self.root / value["captures"][0]["file"]
        with mock.patch.object(gate, "png_rgba", side_effect=lambda path, size: pixels[path.name]):
            first.unlink()
            with self.assertRaises(ValueError):
                gate.validate_editor(self.root)
            first.symlink_to(value["captures"][1]["file"])
            with self.assertRaises(ValueError):
                gate.validate_editor(self.root)
            first.unlink()
            first.write_bytes(b"fixture")
            (self.root / "extra.png").write_bytes(b"extra")
            with self.assertRaises(ValueError):
                gate.validate_editor(self.root)
            (self.root / "extra.png").unlink()
        with mock.patch.object(gate, "png_rgba", return_value=bytes(1500 * 1400 * 4)):
            with self.assertRaises(ValueError):
                gate.validate_editor(self.root)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GateFixtures)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.skipped or result.testsRun != gate.FIXTURE_COUNT:
        raise SystemExit(1)
    print(f"PNG_SEQUENCE_CLIPS_GATE_FIXTURES: {result.testsRun} passed; 0 failed; 0 skipped")
