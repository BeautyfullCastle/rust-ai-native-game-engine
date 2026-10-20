#!/usr/bin/env python3
"""Pure Python negative fixtures. No Cargo, native windows, or GPU execution."""
import copy
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest import mock

import unsaved_close_gate as gate


class GateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.saved = (gate.runner.repo, gate.runner.evidence, gate.runner.report, gate.runner.frozen, gate.runner.target_dir)
        gate.runner.repo = self.root
        gate.runner.evidence = self.root / "evidence"
        gate.runner.evidence.mkdir()
        gate.runner.target_dir = self.root / "target"
        gate.runner.report = {"producers": {}, "commands": [], "coverage": {"cpu": "NOT RUN"}}
        self.addCleanup(self.restore)

    def restore(self):
        gate.runner.repo, gate.runner.evidence, gate.runner.report, gate.runner.frozen, gate.runner.target_dir = self.saved

    @staticmethod
    def listed(names):
        return "".join(name + ": test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n"

    @staticmethod
    def result(total=20):
        return (f"running {len(gate.NAMES)} tests\n" + "".join(f"test {name} ... ok\n" for name in gate.NAMES)
                + f"test result: ok. {len(gate.NAMES)} passed; 0 failed; 0 ignored; 0 measured; {total-len(gate.NAMES)} filtered out; finished in 0.10s\n")

    def producer(self):
        crate = self.root / "crates/orr_editor"
        binary = gate.runner.target_dir / "release/deps/orr_editor-fixture"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"fixture binary, not executable acceptance")
        item = {"reason": "compiler-artifact", "fresh": True, "package_id": "path+" + crate.as_uri() + "#orr_editor@0.0.1",
                "manifest_path": str(crate / "Cargo.toml"), "executable": str(binary),
                "target": {"name": "orr_editor", "kind": ["lib"], "crate_types": ["lib"], "src_path": str(crate / "src/lib.rs")},
                "features": gate.FEATURES, "profile": {"test": True, "opt_level": "3"}}
        return item

    @staticmethod
    def cargo(item):
        return json.dumps(item) + '\n{"reason":"build-finished","success":true}\n'

    def test_reviewed_names_match_source_and_platform_contract(self):
        self.assertEqual((len(gate.NAMES), len(set(gate.NAMES))), (19, 19))
        source = (Path(__file__).resolve().parents[1] / "crates/orr_editor/src/app/unsaved_close_tests.rs").read_text()
        self.assertEqual(set(gate.NAMES), {gate.PREFIX + name for name in re.findall(r"^fn (actual_app_\w+)\(", source, re.M)})
        self.assertEqual(gate.REQUESTED, "collect-ui,project-create,tilemap")
        with mock.patch.object(gate.sys, "platform", "win32"), self.assertRaises(ValueError):
            gate.cpu()

    def test_exact_inventory_accepts_crlf_and_empty_ignored_only(self):
        gate.inventory(self.listed(gate.NAMES).replace("\n", "\r\n"), gate.NAMES)
        gate.inventory(self.listed([]), [])
        self.assertEqual(gate.full_inventory(self.listed((*gate.NAMES, "other::test")))[-1], "other::test")

    def test_missing_duplicate_extra_and_ignored_inventory_fail(self):
        for actual in ((), gate.NAMES[:-1], (*gate.NAMES, gate.NAMES[0]), (*gate.NAMES, "unexpected")):
            with self.subTest(actual=actual), self.assertRaises((ValueError, AssertionError)):
                gate.inventory(self.listed(actual), gate.NAMES)
        for actual in ((), gate.NAMES[:-1], (*gate.NAMES, gate.NAMES[0])):
            with self.assertRaises((ValueError, AssertionError)):
                gate.full_inventory(self.listed(actual))
        with self.assertRaises(AssertionError):
            gate.inventory(self.listed([gate.NAMES[0]]), [])
        with self.assertRaises(AssertionError):
            gate.inventory(self.listed(gate.NAMES).replace("19 tests", "18 tests"), gate.NAMES)

    def test_exact_results_and_interleaved_diagnostic(self):
        good = self.result()
        gate.result(good, 20)
        gate.result(good.replace("\n", "\r\n"), 20)
        gate.result(good.replace(" ... ok", " ... screenshot written: fixture.png\nok", 1), 20)

    def test_failure_skip_zero_missing_duplicate_and_summary_reject(self):
        good = self.result()
        for bad in ("", good.replace("19 passed", "0 passed"), good.replace("0 ignored", "1 ignored"),
                    good.replace("... ok", "... FAILED", 1), good.replace("... ok", "... ignored", 1),
                    good.replace("... ok", "... SKIP: unavailable\nok", 1), good + good,
                    good.replace(f"test {gate.NAMES[0]} ... ok\n", ""),
                    good.replace(gate.NAMES[0], gate.NAMES[1]), good.replace("running 19", "running 0"),
                    good.replace("1 filtered", "2 filtered")):
            with self.subTest(output=bad), self.assertRaises((ValueError, AssertionError)):
                gate.result(bad, 20)
        for total in (0, True, 18):
            with self.assertRaises(ValueError):
                gate.result(good, total)

    def test_producer_accepts_current_cargo_artifact_not_freshness_claim(self):
        item = self.producer()
        path = Path(gate.producer(self.cargo(item)))
        self.assertEqual(path.read_bytes(), Path(item["executable"]).read_bytes())
        self.assertEqual(gate.runner.report["producers"]["focused-libtest"]["producer"]["fresh"], True)

    def test_cargo_failure_duplicates_and_missing_terminal_record_reject(self):
        good = self.cargo(self.producer())
        for bad in ("", good.replace('"success":true', '"success":false'),
                    good.replace('"success":true', '"success":1'), good + good,
                    good + '{"reason":"late"}\n', good.splitlines()[0],
                    good.replace('"success":true', '"success":true,"success":false')):
            with self.subTest(output=bad), self.assertRaises(ValueError):
                gate.producer(bad)

    def test_wrong_producer_feature_target_path_and_profile_reject(self):
        good = self.producer()
        variants = []
        for key, value in (("features", gate.FEATURES[:-1]), ("manifest_path", str(self.root / "wrong/Cargo.toml")),
                           ("package_id", "registry+wrong"), ("executable", str(self.root / "unowned"))):
            bad = copy.deepcopy(good); bad[key] = value; variants.append(bad)
        for key, value in (("kind", ["bin"]), ("src_path", str(self.root / "other.rs"))):
            bad = copy.deepcopy(good); bad["target"][key] = value; variants.append(bad)
        bad = copy.deepcopy(good); bad["profile"]["test"] = False; variants.append(bad)
        for bad in variants:
            with self.subTest(item=bad), self.assertRaises(ValueError):
                gate.producer(self.cargo(bad))

    def test_boundary_commands_are_only_scoped_production_check_and_strict(self):
        commands = dict(gate.boundary_commands())
        self.assertEqual(list(commands), ["no-default-check", "no-default-strict", "optional-union-check", "optional-union-strict"])
        for name, command in commands.items():
            self.assertEqual(command[0], "cargo")
            self.assertIn(command[1], ("check", "clippy"))
            self.assertIn("--no-default-features", command)
            self.assertIn("--locked", command)
            self.assertIn("--release", command)
            self.assertIn("--lib", command)
            self.assertEqual(command[command.index("--bin") + 1], "orr_editor")
            self.assertNotIn("--all-targets", command)
            self.assertNotIn("--workspace", command)
            if name.endswith("strict"):
                self.assertEqual(command[-3:], ["--", "-D", "warnings"])
            self.assertEqual("--features" in command, name.startswith("optional"))

    def test_failed_execution_never_records_coverage(self):
        output = self.cargo(self.producer())
        with mock.patch.object(gate.sys, "platform", "linux"), mock.patch.object(gate.platform, "machine", return_value="x86_64"), mock.patch.object(gate, "run", side_effect=[output, self.listed(gate.NAMES), self.listed(gate.NAMES), self.listed([]), gate.runner.GateFailure("fixture failure", 101)]):
            with self.assertRaises(gate.runner.GateFailure):
                gate.cpu()
        self.assertEqual(gate.runner.report["coverage"]["cpu"], "NOT RUN")

    def test_zero_ignored_list_and_named_result_are_both_required(self):
        output = self.cargo(self.producer())
        with mock.patch.object(gate.sys, "platform", "linux"), mock.patch.object(gate.platform, "machine", return_value="x86_64"), mock.patch.object(gate, "run", side_effect=[output, self.listed(gate.NAMES), self.listed(gate.NAMES), self.listed([gate.NAMES[0]])]):
            with self.assertRaises(AssertionError):
                gate.cpu()
        self.assertEqual(gate.runner.report["coverage"]["cpu"], "NOT RUN")

    def test_workflow_uses_existing_job_platform_guard_and_always_evidence(self):
        text = (Path(__file__).resolve().parents[1] / ".github/workflows/determinism.yml").read_text()
        sample = text.split("  sample-build:\n", 1)[1].split("\n  sample-linked-prefabs:", 1)[0]
        for phase in ("cpu", "cfg"):
            self.assertIn(f"run: python -B tools/unsaved_close_gate.py {phase}", sample)
            path = "${{ runner.temp }}/unsaved-close-" + phase + "-${{ github.run_id }}-${{ github.run_attempt }}"
            self.assertEqual(sample.count("ORR_UNSAVED_CLOSE_EVIDENCE: " + path), 1)
            self.assertEqual(sample.count("path: " + path), 1)
        self.assertIn("- name: Required popup-free unsaved close CPU acceptance\n        if: runner.os == 'Linux' && runner.arch == 'X64'", sample)
        self.assertIn("- name: Retain unsaved close CPU evidence\n        if: always() && runner.os == 'Linux' && runner.arch == 'X64'", sample)
        self.assertIn("- name: Retain unsaved close production cfg evidence\n        if: always()", sample)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GateTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    passed = result.testsRun - len(result.failures) - len(result.errors) - len(result.skipped)
    print(f"UNSAVED_CLOSE_GATE_FIXTURES: {passed} passed; {len(result.failures) + len(result.errors)} failed; {len(result.skipped)} skipped", flush=True)
    sys.exit(0 if result.wasSuccessful() and result.testsRun == gate.FIXTURE_COUNT and not result.skipped else 1)
