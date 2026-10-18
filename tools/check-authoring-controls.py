#!/usr/bin/env python3
"""Bounded CPU-only authoring checks; reject empty or incomplete test runs."""

import argparse
from pathlib import Path
import re
import subprocess
import sys
import unittest


FEATURES = "mesh-export,mesh-mission,package-manager,mesh-animated-models"
SUITES = (
    ("app::mesh_recorded_playback::tests", 13),
    ("package_manager_panel::tests", 6),
    ("mesh_model::tests", 2),
    ("mesh_navigation::geometry_import::tests", 10),
)
SUMMARY = re.compile(
    r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out; finished in .+$", re.MULTILINE
)
CASE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)(?: .*)?$", re.MULTILINE)


def validate_tests(output, suite, expected):
    """Require one libtest run and exactly the expected distinct passing cases."""
    output = output.replace("\r\n", "\n")
    summaries = SUMMARY.findall(output)
    if len(summaries) != 1 or tuple(map(int, summaries[0][:4])) != (expected, 0, 0, 0):
        raise ValueError(f"{suite}: expected one summary with {expected} passed and no skips")
    runs = re.findall(r"^running (\d+) tests?$", output, re.MULTILINE)
    if runs != [str(expected)]:
        raise ValueError(f"{suite}: missing or inconsistent test run count")
    cases = CASE.findall(output)
    names = [name for name, status in cases]
    if len(cases) != expected or len(set(names)) != expected:
        raise ValueError(f"{suite}: missing or duplicate individual test results")
    if any(not name.startswith(suite + "::") or status != "ok" for name, status in cases):
        raise ValueError(f"{suite}: unexpected test name or non-passing result")


def run_logged(command, path):
    """Write evidence before execution, including process failures and timeouts."""
    print("+ " + " ".join(command), flush=True)
    with path.open("w", encoding="utf-8") as log:
        log.write("+ " + " ".join(command) + "\n")
        log.flush()
        try:
            result = subprocess.run(
                command, stdout=log, stderr=subprocess.STDOUT, timeout=2700, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            log.write(f"\nExecution failed: {error}\n")
            raise
    output = path.read_text(encoding="utf-8", errors="replace")
    print(output, end="", flush=True)
    if result.returncode:
        raise RuntimeError(f"Command exited {result.returncode}; see {path}")
    return output


def check():
    root = Path(__file__).resolve().parent.parent
    if Path.cwd().resolve() != root:
        raise RuntimeError("Run from the repository root")
    evidence = root / "target" / "authoring-controls-evidence"
    evidence.mkdir(parents=True, exist_ok=True)
    commands = (
        ("check-default", ["cargo", "check", "--locked", "-p", "orr_editor", "--lib"]),
        ("check-mesh-export", ["cargo", "check", "--locked", "-p", "orr_editor", "--lib",
                               "--features", "mesh-export"]),
        ("clippy-production", ["cargo", "clippy", "--locked", "-p", "orr_editor", "--lib",
                               "--features", FEATURES, "--", "-D", "warnings"]),
    )
    for label, command in commands:
        run_logged(command, evidence / f"{label}.log")
    for suite, expected in SUITES:
        path = evidence / (suite.replace("::", "-") + ".log")
        output = run_logged(
            ["cargo", "test", "--locked", "-p", "orr_editor", "--lib", "--features", FEATURES,
             suite + "::", "--", "--format=pretty", "--color=never", "--test-threads=1"], path
        )
        try:
            validate_tests(output, suite, expected)
        except ValueError as error:
            with path.open("a", encoding="utf-8") as log:
                log.write(f"\nEvidence validation failed: {error}\n")
            raise
        print(f"Verified {suite}: exactly {expected} passed", flush=True)


class ValidatorTests(unittest.TestCase):
    suite = "app::mesh_recorded_playback::tests"

    def output(self, count=2):
        cases = "".join(f"test {self.suite}::case_{i} ... ok\n" for i in range(count))
        return (f"running {count} tests\n{cases}\n"
                f"test result: ok. {count} passed; 0 failed; 0 ignored; "
                "0 measured; 47 filtered out; finished in 0.01s\n")

    def test_valid_and_windows_newlines(self):
        for output in (self.output(), self.output().replace("\n", "\r\n")):
            validate_tests(output, self.suite, 2)

    def test_empty_selection(self):
        with self.assertRaises(ValueError):
            validate_tests(self.output(0), self.suite, 2)

    def test_missing_summary(self):
        with self.assertRaises(ValueError):
            validate_tests(self.output().split("test result:")[0], self.suite, 2)

    def test_duplicate_runs(self):
        with self.assertRaises(ValueError):
            validate_tests(self.output() * 2, self.suite, 2)

    def test_wrong_suite(self):
        with self.assertRaises(ValueError):
            validate_tests(self.output().replace(self.suite, "other::tests"), self.suite, 2)

    def test_ignored_or_failed(self):
        for output in (self.output().replace("0 ignored", "1 ignored"),
                       self.output().replace("... ok", "... ignored", 1),
                       self.output().replace("... ok", "... FAILED", 1)):
            with self.assertRaises(ValueError):
                validate_tests(output, self.suite, 2)

    def test_missing_or_duplicate_cases(self):
        for output in (self.output().replace(f"test {self.suite}::case_0 ... ok\n", ""),
                       self.output().replace("case_1", "case_0")):
            with self.assertRaises(ValueError):
                validate_tests(output, self.suite, 2)

    def test_inconsistent_run_count(self):
        with self.assertRaises(ValueError):
            validate_tests(self.output().replace("running 2", "running 0"), self.suite, 2)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="Run stdlib validator tests only")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[sys.argv[0]])
    else:
        check()
