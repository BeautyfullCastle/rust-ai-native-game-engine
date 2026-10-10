"""Focused tests for deployment_manifest.py; run with Python's unittest."""

from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

from tools import deployment_manifest as dm


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "deployment_manifest.py"


class DeploymentManifestTests(unittest.TestCase):
    def manifest(self, game_code_id="42", **kwargs):
        return dm.create_manifest("arena", "arena-release-1", game_code_id, **kwargs)

    def test_frame_build_id_matches_expected_reference_values(self):
        self.assertEqual(dm.frame_build_id(1), 2372055272327396894)
        self.assertNotEqual(dm.frame_build_id(1), dm.frame_build_id(2))
        self.assertGreater(dm.frame_build_id(dm.MAX_U64), 0)

    def test_precision_boundaries_remain_decimal_strings_in_manifest_and_exports(self):
        for value in (1 << 53, (1 << 53) + 1, dm.MAX_U64):
            manifest = self.manifest(str(value))
            encoded = json.dumps(manifest)
            loaded = dm.validate_manifest(json.loads(encoded))
            plan = dm.export_manifest(loaded)
            self.assertEqual(loaded["game_code_id"], str(value))
            self.assertEqual(loaded["build_id"], str(dm.frame_build_id(value)))
            self.assertIsInstance(plan["browser"]["opts"]["build_id"], str)
            self.assertEqual(plan["browser"]["opts"]["build_id"], loaded["build_id"])
            self.assertEqual(plan["native_host"]["args"][-1], loaded["build_id"])

    def test_create_normalizes_hex_input_but_manifest_requires_canonical_decimal(self):
        self.assertEqual(self.manifest("0x2a")["game_code_id"], "42")
        with self.assertRaisesRegex(dm.ManifestError, "canonical"):
            dm.validate_manifest({**self.manifest(), "game_code_id": "0x2a"})
        for bad in ("042", "+42", "42 ", "4.2", "4e1", "0", "0x0"):
            with self.subTest(bad=bad), self.assertRaises(dm.ManifestError):
                dm.create_manifest("arena", "arena-release-1", bad)

    def test_zero_numeric_json_and_u64_overflow_are_rejected(self):
        for value in (0, "0", 0.0, True, "18446744073709551616", "0x10000000000000000"):
            with self.subTest(value=value), self.assertRaises(dm.ManifestError):
                dm.create_manifest("arena", "arena-release-1", value)
        valid = self.manifest()
        for field in ("game_code_id", "build_id"):
            changed = copy.deepcopy(valid)
            changed[field] = 42
            with self.subTest(field=field), self.assertRaises(dm.ManifestError):
                dm.validate_manifest(changed)

    def test_schema_u64_regex_boundaries_and_structural_type(self):
        schema = json.loads((ROOT / "deploy" / "deployment-manifest.schema.json").read_text(encoding="utf-8"))
        pattern = re.compile(schema["$defs"]["nonzeroU64"]["pattern"])
        self.assertIsNotNone(pattern.fullmatch(str(dm.MAX_U64)))
        self.assertIsNone(pattern.fullmatch(str(dm.MAX_U64 + 1)))
        self.assertIsNone(pattern.fullmatch("0"))
        self.assertIsNone(pattern.fullmatch("00"))
        self.assertEqual(schema["$defs"]["nonzeroU64"]["type"], "string")
        self.assertIn("2.0 as an integer", schema["description"])

    def test_manifest_rejects_zero_wrong_build_and_frame_format(self):
        manifest = self.manifest()
        for changed in (
            {**manifest, "game_code_id": "0"},
            {**manifest, "build_id": "42"},
            {**manifest, "frame_format_version": 3},
            {**manifest, "unexpected": "value"},
        ):
            with self.subTest(changed=changed), self.assertRaises(dm.ManifestError):
                dm.validate_manifest(changed)

    def test_expected_game_and_code_identity_must_match(self):
        manifest = self.manifest()
        with self.assertRaisesRegex(dm.ManifestError, "game identity mismatch"):
            dm.validate_manifest(manifest, expect_game="physics")
        with self.assertRaisesRegex(dm.ManifestError, "game_code_identity mismatch"):
            dm.validate_manifest(manifest, expect_game_code_identity="arena-release-2")
        self.assertIs(dm.validate_manifest(
            manifest, expect_game="arena", expect_game_code_identity="arena-release-1"
        ), manifest)

    def test_duplicate_json_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "duplicate.json"
            path.write_text('{"format":"orr.deployment/1","format":"orr.deployment/1"}', encoding="utf-8")
            with self.assertRaisesRegex(dm.ManifestError, "duplicate JSON key"):
                dm.load_manifest(path)

    def test_source_and_binary_provenance_are_separate_and_informational(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "client.wasm"
            binary.write_bytes(b"sample binary")
            manifest = dm.create_manifest(
                "physics", "physics-release-a", "9", "source-abc", [f"wasm={binary}"]
            )
        self.assertEqual(manifest["provenance"]["source_revision"], "source-abc")
        self.assertEqual(manifest["provenance"]["binaries"], [{
            "label": "wasm",
            "sha256": hashlib.sha256(b"sample binary").hexdigest(),
        }])
        self.assertNotIn("source_revision", manifest)
        self.assertNotIn("sha256", manifest)
        with self.assertRaisesRegex(dm.ManifestError, "source_revision"):
            dm.create_manifest("arena", "arena-release-1", "1", source_revision=42)

    def test_export_lists_are_independent_for_library_consumers(self):
        plan = dm.export_manifest(self.manifest())
        plan["native_host"]["args"].append("--bind")
        self.assertNotIn("--bind", plan["relay"]["args"])

    def test_previous_manifest_rejects_reused_raw_id_for_changed_identity_or_game(self):
        previous = self.manifest("42")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "prior.json"
            path.write_text(json.dumps(previous), encoding="utf-8")
            same = self.manifest("42")
            dm._check_previous(same, path)
            changed_identity = dm.create_manifest("arena", "arena-release-2", "42")
            with self.assertRaisesRegex(dm.ManifestError, "game_code_id was reused"):
                dm._check_previous(changed_identity, path)
            changed_game = dm.create_manifest("physics", "arena-release-1", "42")
            with self.assertRaisesRegex(dm.ManifestError, "game_code_id was reused"):
                dm._check_previous(changed_game, path)

    def test_export_contains_exact_browser_strings_and_argv(self):
        manifest = self.manifest("9007199254740993")
        plan = dm.export_manifest(manifest)
        build_id = manifest["build_id"]
        self.assertEqual(plan["native_host"]["args"], ["--game", "arena", "--build-id", build_id])
        self.assertEqual(plan["relay"]["args"], ["--game", "arena", "--build-id", build_id])
        self.assertEqual(plan["native_client"]["args"], ["--build-id", build_id])
        self.assertEqual(plan["browser"]["opts"], {"game": "arena", "build_id": build_id})
        self.assertEqual(plan["browser"]["query"], {"game": "arena", "build": build_id})

    def test_cli_create_validate_id_export_and_exclusive_output(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "deployment.json"
            cmd = [sys.executable, str(SCRIPT), "create", "--game", "arena",
                   "--game-code-identity", "arena-v1", "--game-code-id", "9007199254740993",
                   "--output", str(path)]
            result = subprocess.run(cmd, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            for command, expected in (("validate", "valid"), ("id", dm.frame_build_id((1 << 53) + 1))):
                result = subprocess.run(
                    [sys.executable, str(SCRIPT), command, str(path), "--expect-game", "arena",
                     "--expect-game-code-identity", "arena-v1"],
                    capture_output=True, text=True, check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                if command == "id":
                    self.assertEqual(result.stdout.strip(), str(expected))
                else:
                    self.assertEqual(result.stdout.strip(), expected)
            result = subprocess.run([sys.executable, str(SCRIPT), "export", str(path)],
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["browser"]["opts"]["build_id"],
                             str(dm.frame_build_id((1 << 53) + 1)))
            result = subprocess.run(cmd, capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("refusing to overwrite", result.stderr)


if __name__ == "__main__":
    unittest.main()
