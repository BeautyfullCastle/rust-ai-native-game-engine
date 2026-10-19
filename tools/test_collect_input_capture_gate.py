"""Negative mutations for capture inventories, artifacts and execution routing."""
import copy
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zlib

import collect_input_capture_gate as gate


class CaptureGateTests(unittest.TestCase):
    def test_compiled_inventory_is_exact_on_both_platforms(self):
        for windows, count in ((False, 35), (True, 33)):
            expected = gate.document_inventory(windows) | gate.CAPTURE_NAMES
            self.assertEqual(len(expected), count)
            output = "\n".join(name + ": test" for name in sorted(expected)) + f"\n{count} tests, 0 benchmarks\n"
            gate.validate_inventory(output, expected)
            gate.validate_inventory(output.replace("\n", "\r\n"), expected)
            for bad in ("", output + output, output.replace(": test", ": benchmark", 1),
                        output.replace(f"{count} tests", "0 tests"), output + "unexpected: test\n"):
                with self.subTest(windows=windows, bad=bad), self.assertRaises(ValueError):
                    gate.validate_inventory(bad, expected)
        with self.assertRaises(ValueError):
            gate.validate_inventory("0 tests, 0 benchmarks\n", set())

    def fixture(self, directory):
        directory = Path(directory)
        receipt = {"schema": 1, "window_pixels": [720, 650], "native_os_keyboard": False,
                   "adapter_details": {"name": "llvmpipe", "backend": "Vulkan", "device_type": "Cpu",
                                       "driver": "llvmpipe", "driver_info": "test fixture"},
                   **dict.fromkeys(gate.CAPTURE_FLAGS, True), "frames": []}
        def chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
        png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 720, 650, 8, 6, 0, 0, 0))
               + chunk(b"IDAT", zlib.compress((b"\0" + b"\0" * (720 * 4)) * 650)) + chunk(b"IEND", b""))
        for filename, labels in gate.CAPTURE_FRAMES.items():
            (directory / filename).write_bytes(png)
            receipt["frames"].append({"filename": filename, "proof": "assertion verified in test",
                                      "widgets": [{"label": label, "rect": [1, 1, 100, 21]} for label in sorted(labels)]})
        saved = {"version": 1, "actions": [
            {"name": action, "bindings": [{"device": "Keyboard", "key": key}]}
            for action, key in (("left", "j"), ("right", "l"), ("up", "i"), ("down", "k"), ("restart", "enter"), ("quit", "q"))]}
        (directory / "capture-accepted-only.json").write_text(json.dumps(saved))
        return receipt

    def write(self, directory, receipt):
        (Path(directory) / "collect-input-capture-acceptance.json").write_text(json.dumps(receipt))

    def test_gpu_valid_receipt_and_each_missing_or_forged_scope_flag(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = self.fixture(directory)
            self.write(directory, receipt)
            gate.validate_capture_gpu(Path(directory))
            for key in ["schema", "window_pixels", "native_os_keyboard", "adapter_details", "frames", *gate.CAPTURE_FLAGS]:
                for value in (None, "true", False if key != "native_os_keyboard" else True):
                    invalid = copy.deepcopy(receipt)
                    invalid[key] = value
                    self.write(directory, invalid)
                    with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                        gate.validate_capture_gpu(Path(directory))

    def test_gpu_missing_duplicate_wrong_frame_and_invalid_widget_bounds_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = self.fixture(directory)
            invalids = []
            for key, value in (("filename", "../escape.png"), ("proof", ""), ("widgets", [])):
                invalid = copy.deepcopy(receipt); invalid["frames"][0][key] = value; invalids.append(invalid)
            invalid = copy.deepcopy(receipt); invalid["frames"][1] = invalid["frames"][0]; invalids.append(invalid)
            invalid = copy.deepcopy(receipt); invalid["frames"].pop(); invalids.append(invalid)
            for rect in ([0, 0, 721, 650], [0, 0, 720, 651], [1, 1, 2, 2], [False, 0, 20, 20],
                         [0, 0, float("nan"), 20], [0, 0, float("inf"), 20], [10, 10, 0, 0], [0, 0, 10]):
                invalid = copy.deepcopy(receipt); invalid["frames"][0]["widgets"][0]["rect"] = rect; invalids.append(invalid)
            invalid = copy.deepcopy(receipt); invalid["frames"][0]["widgets"][1] = invalid["frames"][0]["widgets"][0]; invalids.append(invalid)
            for invalid in invalids:
                self.write(directory, invalid)
                with self.assertRaises(ValueError):
                    gate.validate_capture_gpu(Path(directory))

    def test_gpu_rejects_hardware_adapter_and_changed_saved_map(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = self.fixture(directory)
            for key, value in (("name", "hardware"), ("backend", "Metal"), ("device_type", "DiscreteGpu"), ("driver", None)):
                invalid = copy.deepcopy(receipt); invalid["adapter_details"][key] = value; self.write(directory, invalid)
                with self.assertRaises(ValueError): gate.validate_capture_gpu(Path(directory))
            self.write(directory, receipt)
            (Path(directory) / "capture-accepted-only.json").write_text('{}')
            with self.assertRaises(ValueError): gate.validate_capture_gpu(Path(directory))

    def test_gpu_requires_real_complete_pngs(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = self.fixture(directory); self.write(directory, receipt)
            path = Path(directory) / "capture-listening.png"
            original = path.read_bytes()
            for data in (original[:24], original[:-8], original + b"trailing", original[:50] + b"bad" + original[53:]):
                path.write_bytes(data)
                with self.assertRaises(ValueError): gate.validate_capture_gpu(Path(directory))
            path.unlink()
            with self.assertRaises(ValueError): gate.validate_capture_gpu(Path(directory))

    def test_gpu_runs_ignored_capture_before_required_main_editor_gpu(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory); calls = []
            def run(base, name, folder, label, ignored=False):
                calls.append((name, ignored))
                self.assertTrue(ignored)
                self.assertEqual(base, gate.GPU_BASE)
                captures = Path(os.environ["ORR_COLLECT_INPUT_CAPTURE_EVIDENCE"])
                self.assertEqual(captures, evidence / "capture-software-gpu")
                self.write(captures, self.fixture(captures))
            with patch.object(gate.previous, "run_case", side_effect=run), patch.object(gate.previous, "gpu", side_effect=lambda p: calls.append(("main-editor", p))), patch.dict(os.environ):
                gate.gpu(evidence)
                self.assertEqual(calls, [(gate.GPU_TEST, True), ("main-editor", evidence)])
                with self.assertRaises(FileExistsError): gate.gpu(evidence)


if __name__ == "__main__":
    unittest.main()
