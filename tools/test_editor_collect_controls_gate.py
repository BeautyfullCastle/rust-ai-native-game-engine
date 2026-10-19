"""Negative tests for the saved-map evidence gate (no Rust/GPU required)."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import struct
from unittest.mock import patch
import unittest

import editor_collect_controls_gate as gate


class GateTests(unittest.TestCase):
    def output(self):
        return "running 1 test\ntest suite::case ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.1s\n"

    def test_accepts_exact_execution_and_crlf(self):
        gate.validate_exact(self.output(), {"suite::case"})
        gate.validate_exact(self.output().replace("\n", "\r\n"), {"suite::case"})

    def test_rejects_empty_expected_inventory(self):
        with self.assertRaises(ValueError):
            gate.validate_exact("", set())

    def test_rejects_skip_duplicate_wrong_and_zero(self):
        for output in (self.output().replace("... ok", "... ignored"),
                       self.output() * 2, self.output().replace("suite::case", "suite::wrong"),
                       self.output().replace("1 passed", "0 passed"),
                       self.output().replace("running 1 test", "running 0 tests"),
                       self.output().replace("0 ignored", "1 ignored"), ""):
            with self.subTest(output=output), self.assertRaises(ValueError):
                gate.validate_exact(output, {"suite::case"})

    def test_input_runtime_inventory_preserves_nested_acceptance_hook_boundary(self):
        ordinary = sorted(gate.INPUT_RUNTIME_CASES)
        output = ("running 9 tests\n"
                  + "".join(f"test {name} ... ok\n" for name in ordinary)
                  + f"test {gate.CONSUMER} ... ignored, requires actual authoring App bytes\n"
                  + "test result: ok. 8 passed; 0 failed; 1 ignored; 0 measured; 50 filtered out; finished in 0.1s\n")
        gate.validate_input_runtime_regression(output)
        gate.validate_input_runtime_regression(output.replace("\n", "\r\n"))
        for invalid in (
            output.replace(gate.CONSUMER, gate.CONSUMER + "_unexpected"),
            output.replace(ordinary[0], ordinary[1]),
            output.replace(f"test {ordinary[0]} ... ok\n", ""),
            output.replace(f"test {ordinary[0]} ... ok", f"test {ordinary[0]} ... ignored"),
            output.replace(f"test {ordinary[0]} ... ok", f"test {ordinary[0]} ... FAILED"),
            output.replace(f"test {gate.CONSUMER} ... ignored", f"test {gate.CONSUMER} ... ok"),
            output.replace(f"test {ordinary[0]} ... ok", f"test {ordinary[0]} ... ignored")
                  .replace(f"test {gate.CONSUMER} ... ignored", f"test {gate.CONSUMER} ... ok"),
            output.replace("8 passed", "9 passed"),
            output.replace("1 ignored", "0 ignored"),
            output.replace("running 9 tests", "running 8 tests"),
            output + "test unexpected::nested ... ignored\n",
            output + "test unexpected::nested ... unknown\n",
            output + "test result: malformed\n",
            output + output,
            "",
        ):
            with self.subTest(output=invalid), self.assertRaises(ValueError):
                gate.validate_input_runtime_regression(invalid)

    def test_handoff_requires_exact_path_bytes_and_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "map.json"
            path.write_bytes(b"actual saved bytes")
            before = hashlib.sha256(path.read_bytes()).hexdigest()
            receipt = {"input_map_path": str(path), "input_map_sha256": before,
                       "production_keys": True, "real_bridge": True, "map_bytes_unchanged": True}
            gate.validate_handoff(receipt, path, before)
            for key in receipt:
                invalid = copy.deepcopy(receipt)
                del invalid[key]
                with self.subTest(key=key), self.assertRaises(ValueError):
                    gate.validate_handoff(invalid, path, before)
            path.write_bytes(b"rewritten substitute map")
            with self.assertRaises(ValueError):
                gate.validate_handoff(receipt, path, before)
            path.unlink()
            with self.assertRaises(ValueError):
                gate.validate_handoff(receipt, path, before)

    def test_contracts_run_real_producer_before_ignored_consumer(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory).resolve()
            calls = []
            def run(base, name, folder, label, ignored=False):
                calls.append((name, ignored))
                path = Path(gate.os.environ["ORR_COLLECT_ACCEPTANCE_MAP"])
                if name == gate.PRODUCER:
                    self.assertFalse(path.exists())
                    path.write_bytes(b"real widget producer")
                else:
                    self.assertEqual(calls[0], (gate.PRODUCER, True))
                    self.assertEqual(path.read_bytes(), b"real widget producer")
                    path.with_suffix(".receipt.json").write_text(json.dumps({
                        "input_map_path": str(path), "input_map_sha256": gate.os.environ["ORR_COLLECT_ACCEPTANCE_SHA256"],
                        "production_keys": True, "real_bridge": True, "map_bytes_unchanged": True}))
            with patch.object(gate, "run_case", side_effect=run), patch.dict(gate.os.environ):
                gate.contracts(evidence)
                self.assertEqual(calls, [(gate.PRODUCER, True), (gate.CONSUMER, True)])
                with self.assertRaises(ValueError):
                    gate.contracts(evidence)

    def test_gpu_validates_phase_verified_frame_and_all_capture_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            captures = Path(directory)
            receipt = {"schema": 1, "adapter": "llvmpipe", "backend": "Vulkan", "device_type": "Cpu",
                       "window_pixels": [1600, 1000], "authoring_pixels": [720, 650],
                       "main_editor_app": True, "standalone_authoring_app": True,
                       "physical_egui_events": True, "mapped_victory_restart": True,
                       "normalized_restart_viewport_equal": True, "fresh_app_explicit_reload": True,
                       "configuration_bytes_unchanged": True, "authoritative_victory": True,
                       "restart_acknowledged": True, "native_os_keyboard": False}
            (captures / "collect-controls-acceptance.json").write_text(json.dumps(receipt))
            def png(name, size):
                (captures / name).write_bytes(b"\x89PNG\r\n\x1a\n" + b"\0\0\0\rIHDR" + struct.pack(">II", *size))
            png("collect-controls-authoring.png", (720, 650))
            for name, phase in (("mapped", 0), ("victory", 1), ("restart", 0), ("reopened-victory", 1)):
                stem = "collect-controls-" + name
                png(stem + ".png", (1600, 1000))
                png(stem + "-viewport.png", (1030, 792))
                (captures / (stem + ".json")).write_text(json.dumps({
                    "phase": phase, "score": phase * 2, "active_actors": 4 - phase * 2, "tick": 5, "verified_tick": 5,
                    "checksum": "a" * 16, "verified_checksum": "a" * 16, "viewport": [230, 90, 1030, 792]}))
            gate.validate_gpu(captures)
            frame_path = captures / "collect-controls-victory.json"
            frame = json.loads(frame_path.read_text())
            for key, value in (("verified_checksum", "b" * 16), ("verified_tick", 6), ("phase", 0),
                               ("viewport", [1500, 0, 800, 600])):
                invalid = dict(frame, **{key: value})
                frame_path.write_text(json.dumps(invalid))
                with self.subTest(key=key), self.assertRaises(ValueError):
                    gate.validate_gpu(captures)
            frame_path.write_text(json.dumps(frame))
            png("collect-controls-restart.png", (720, 650))
            with self.assertRaises(ValueError):
                gate.validate_gpu(captures)

    def test_gpu_receipt_is_not_sufficient_without_captures(self):
        with tempfile.TemporaryDirectory() as directory:
            captures = Path(directory)
            receipt = {"schema": 1, "adapter": "llvmpipe", "backend": "Vulkan", "device_type": "Cpu",
                       "window_pixels": [1600, 1000], "authoring_pixels": [720, 650],
                       "main_editor_app": True, "standalone_authoring_app": True,
                       "physical_egui_events": True, "mapped_victory_restart": True,
                       "normalized_restart_viewport_equal": True, "fresh_app_explicit_reload": True,
                       "configuration_bytes_unchanged": True, "authoritative_victory": True,
                       "restart_acknowledged": True, "native_os_keyboard": False}
            (captures / "collect-controls-acceptance.json").write_text(json.dumps(receipt))
            with self.assertRaises(FileNotFoundError):
                gate.validate_gpu(captures)
            receipt["main_editor_app"] = False
            (captures / "collect-controls-acceptance.json").write_text(json.dumps(receipt))
            with self.assertRaises(ValueError):
                gate.validate_gpu(captures)


if __name__ == "__main__":
    unittest.main()
