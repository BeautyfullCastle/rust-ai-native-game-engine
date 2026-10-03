#!/usr/bin/env python3
"""Regression tests for the simulation float type guard."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any


from check_sim_float_types import PACKAGES, check_config


ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = ROOT / "tools" / "sim-float-guard" / "fixtures"
CLIPPY_CONFIG = ROOT / "tools" / "sim-float-guard" / "clippy.toml"
TARGET_DIR = ROOT / "target" / "sim-float-fixtures"


@dataclass(frozen=True)
class Fixture:
    name: str
    should_fail: bool
    diagnostic_line: int | None = None
    diagnostic_text: str | None = None


FIXTURES = (
    Fixture("fp_state.rs", False),
    Fixture("view_interop.rs", False),
    Fixture("direct_f32.rs", True, 9, "pub value: f32"),
    Fixture("direct_f64.rs", True, 9, "pub value: f64"),
    Fixture("array_f32.rs", True, 9, "pub values: [f32; 4]"),
    Fixture("nested_f64.rs", True, 9, "pub value: f64"),
    Fixture("alias_f32.rs", True, 6, "type Scalar = f32"),
    Fixture("qualified_f64.rs", True, 9, "pub value: core::primitive::f64"),
)


def run(command: list[str], *, cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    print("+", subprocess.list2cmdline(command), flush=True)
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )


def cargo_diagnostics(output: str) -> list[dict[str, Any]]:
    diagnostics: list[dict[str, Any]] = []
    for line in output.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("reason") == "compiler-message":
            diagnostics.append(record["message"])
    return diagnostics


def fail(message: str, output: str | None = None) -> int:
    print(f"FAIL: {message}", file=sys.stderr)
    if output:
        print(output, file=sys.stderr)
    return 1


def main() -> int:
    check_config()
    required = {"orr_fp", "orr_ecs", "orr_sim", "orr_session", "orr_testgame",
                "orr_physics", "orr_physics3d", "orr_games", "orr_asset"}
    if not required.issubset(PACKAGES):
        return fail("simulation guard dropped a required core library")
    cargo = shutil.which("cargo")
    if cargo is None:
        return fail("cargo was not found on PATH")
    if not CLIPPY_CONFIG.is_file():
        return fail(f"required Clippy config is missing: {CLIPPY_CONFIG}")
    missing = [fixture.name for fixture in FIXTURES if not (FIXTURE_DIR / fixture.name).is_file()]
    if missing:
        return fail("missing fixture files: " + ", ".join(missing))

    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    crate_dir = Path(tempfile.mkdtemp(prefix="harness-", dir=TARGET_DIR))
    (crate_dir / "src").mkdir()
    manifest = crate_dir / "Cargo.toml"
    manifest.write_text(
        "\n".join(
            (
                "[package]",
                'name = "sim-float-guard-fixtures"',
                'version = "0.0.0"',
                'edition = "2021"',
                "",
                "[workspace]",
                "",
                "[dependencies]",
                'bytemuck = { version = "1", features = ["derive"] }',
                f'orr_ecs = {{ path = {json.dumps((ROOT / "crates" / "orr_ecs").as_posix())} }}',
                f'orr_fp = {{ path = {json.dumps((ROOT / "crates" / "orr_fp").as_posix())}, features = ["float-interop"] }}',
                "",
            )
        ),
        encoding="utf-8",
    )
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(TARGET_DIR.resolve())
    env.pop("RUSTC_WRAPPER", None)
    manifest_arg = str(manifest.resolve())

    def module_decl(name: str, source: Path) -> str:
        absolute_path = json.dumps(source.resolve().as_posix(), ensure_ascii=False)
        return f"#[path = {absolute_path}]\nmod {name};"

    # Compile every positive and negative fixture before linting. This keeps
    # syntax, Pod, and Component failures distinct from the lint regression.
    all_modules = [
        module_decl(f"fixture_{index}", FIXTURE_DIR / fixture.name)
        for index, fixture in enumerate(FIXTURES)
    ]
    lib_rs = crate_dir / "src" / "lib.rs"
    lib_rs.write_text("\n".join(all_modules) + "\n", encoding="utf-8")
    check = run(
        [cargo, "check", "--manifest-path", manifest_arg, "--lib"],
        cwd=ROOT,
        env=env,
    )
    if check.returncode != 0:
        return fail("all fixtures must compile before lint checks", check.stdout)

    for fixture in FIXTURES:
        source = FIXTURE_DIR / fixture.name
        lib_rs.write_text(
            module_decl("fixture", source) + "\n",
            encoding="utf-8",
        )
        clippy_env = env.copy()
        config_dir = ROOT if fixture.name == "view_interop.rs" else CLIPPY_CONFIG.parent
        clippy_env["CLIPPY_CONF_DIR"] = str(config_dir.resolve())
        result = run(
            [
                cargo,
                "clippy",
                "--manifest-path",
                manifest_arg,
                "--lib",
                "--message-format=json",
                "--",
                "-D",
                "clippy::disallowed_types",
            ],
            cwd=ROOT,
            env=clippy_env,
        )
        diagnostics = cargo_diagnostics(result.stdout)
        errors = [diagnostic for diagnostic in diagnostics if diagnostic.get("level") == "error"]
        if not fixture.should_fail:
            if result.returncode != 0:
                return fail(f"allowed fixture {fixture.name} should pass Clippy", result.stdout)
            if any(
                (diagnostic.get("code") or {}).get("code") == "clippy::disallowed_types"
                for diagnostic in diagnostics
            ):
                return fail(f"allowed fixture {fixture.name} emitted disallowed_types", result.stdout)
            print(f"PASS: {fixture.name} is accepted", flush=True)
            continue

        matching = [
            diagnostic
            for diagnostic in errors
            if (diagnostic.get("code") or {}).get("code") == "clippy::disallowed_types"
        ]
        if result.returncode == 0 or not matching:
            return fail(f"forbidden fixture {fixture.name} must fail clippy::disallowed_types", result.stdout)
        unexpected_errors = [diagnostic for diagnostic in errors if diagnostic not in matching]
        if unexpected_errors:
            return fail(f"{fixture.name} had unrelated compiler errors", result.stdout)

        expected_path = source.resolve().as_posix()
        expected_line = fixture.diagnostic_line
        expected_text = fixture.diagnostic_text
        assert expected_line is not None and expected_text is not None
        matching_span = False
        for diagnostic in matching:
            for span in diagnostic.get("spans", []):
                if not span.get("is_primary"):
                    continue
                span_path = Path(span.get("file_name", ""))
                try:
                    resolved_span = span_path.resolve().as_posix()
                except OSError:
                    resolved_span = str(span_path)
                if resolved_span == expected_path and span.get("line_start") == expected_line:
                    line_text = (span.get("text") or [{}])[0].get("text", "")
                    if not line_text:
                        line_text = source.read_text(encoding="utf-8").splitlines()[expected_line - 1]
                    if expected_text in line_text:
                        matching_span = True
        if not matching_span:
            return fail(
                f"{fixture.name} must diagnose the expected primitive at {expected_path}:{expected_line}",
                result.stdout,
            )
        print(f"PASS: {fixture.name} fails at its expected field/type span", flush=True)

    print("All simulation float guard fixtures passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
