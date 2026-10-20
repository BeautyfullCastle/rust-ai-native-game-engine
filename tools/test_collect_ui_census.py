#!/usr/bin/env python3
"""Source and adversarial census checks; not Rust/native execution evidence."""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

import yaml
import arena_editor_export_gate as gate

ROOT = Path(__file__).resolve().parents[1]
ADDED = [
    "collect_ui::tests::layout_document_matches_rendered_frame_order_geometry_and_clips",
    "collect_ui::tests::layout_document_rejects_invalid_document_and_viewport",
]


def heredoc(script, tag):
    return script.split("<<'" + tag + "'\n", 1)[1].split("\n" + tag, 1)[0]


class CollectUiCensusTests(unittest.TestCase):
    def test_ordinary_layout_tests_are_in_every_collect_ui_inventory(self):
        source = (ROOT / "crates/orr_sample/src/collect_ui.rs").read_text()
        for name in ADDED:
            self.assertRegex(source, r"#\[test\]\s+fn " + name.split("::")[-1] + r"\(")
        self.assertIn('#[cfg(feature = "collect-ui")]\npub mod collect_ui;',
                      (ROOT / "crates/orr_sample/src/lib.rs").read_text())
        steps = yaml.safe_load((ROOT / ".github/workflows/determinism.yml").read_text())["jobs"]["sample-build"]["steps"]
        inventories = {}
        hooks = {}
        for step in steps:
            script = step.get("run", "")
            if "collect_ui::tests::clipped_container" in script:
                inventory = json.loads(heredoc(script, "JSONINVENTORY"))
                for label, item in inventory.items():
                    names = item["names"] if isinstance(item, dict) else item
                    if any(name.startswith("collect_ui::") for name in names):
                        inventories[label] = (script, inventory, names)
            if "<<'PYHOOKS'" in script:
                for label in ("ui-library", "minimal-ui-progress-library"):
                    if "expected['" + label + "']" in script:
                        hooks[label] = heredoc(script, "PYHOOKS")
        expected = {"ui-library": (189, 4, "collect-ui"),
                    "ui-creator-library": (210, 4, "collect-ui-creator"),
                    "minimal-ui-progress-library": (137, 4, None),
                    "sample-cpu": (200, 5, "image-reimport")}
        self.assertEqual(set(inventories), set(expected))
        for label, (passed, ignored_count, profile) in expected.items():
            script, inventory, names = inventories[label]
            with self.subTest(label=label):
                self.assertEqual(len(names), passed + ignored_count)
                self.assertEqual(len(names), len(set(names)))
                for name in ADDED:
                    self.assertIn(name, names)
                ignored = inventory[label]["ignored_names"] if label == "sample-cpu" else inventory["ignored-library-hooks"]
                self.assertEqual(len(ignored), ignored_count)
                self.assertFalse(set(ADDED) & set(ignored))
                if profile:
                    self.assertEqual(gate.UI_CENSUS[profile], (passed, ignored_count, 25, True))
                if label in ("ui-library", "ui-creator-library"):
                    self.assertIn(f"check_result {label} {passed} {ignored_count} cargo test", script)
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    (root / "expected-inventories.json").write_text(json.dumps(inventory))
                    def listing(items):
                        return "".join(name + ": test\n" for name in items) + f"\n{len(items)} tests, 0 benchmarks\n"
                    (root / f"{label}-ignored.txt").write_text(listing(ignored))
                    (root / "pinned-library-ignored.txt").write_text(listing(ignored))
                    for mode, actual in (("exact", names), ("original-omission", [n for n in names if n not in ADDED]),
                                         ("delete", names[1:]), ("extra", names + ["unexpected::test"]),
                                         ("duplicate", names + [names[0]])):
                        with self.subTest(label=label, mode=mode):
                            path = root / f"{label}-list.txt"
                            path.write_text(listing(actual))
                            (root / "pinned-library-list.txt").write_text(listing(actual))
                            if label == "sample-cpu":
                                auditor = heredoc(script, "PYCHECKER")
                                args = [str(root), label, "list"]
                            elif label == "minimal-ui-progress-library":
                                auditor = hooks[label]
                                args = [str(root)]
                            else:
                                auditor = heredoc(script, "PYLIST")
                                args = [str(root), label, str(passed), str(ignored_count)]
                            result = subprocess.run([sys.executable, "-B", "-c", auditor, *args], capture_output=True, text=True)
                            self.assertEqual(result.returncode == 0, mode == "exact", result.stderr)
                            if label == "ui-library":
                                result = subprocess.run([sys.executable, "-B", "-c", hooks[label], str(root)], capture_output=True, text=True)
                                self.assertEqual(result.returncode == 0, mode == "exact", result.stderr)
                            if profile:
                                if mode == "exact":
                                    gate.profile_census(profile, path, root / f"{label}-ignored.txt")
                                else:
                                    with self.assertRaises(AssertionError):
                                        gate.profile_census(profile, path, root / f"{label}-ignored.txt")


if __name__ == "__main__":
    unittest.main()
