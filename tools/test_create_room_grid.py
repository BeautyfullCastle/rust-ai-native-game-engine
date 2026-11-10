"""Grid compiler tests; run_integration uses explicit trusted existing binaries."""
from contextlib import redirect_stderr, redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import create_room_grid as grid

TOOL = Path(grid.__file__).resolve()
DRAWING = b".......\nPK.#..E\n.......\n"


def inventory(root):
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(root.rglob("*")) if path.is_file()}


class GridTests(unittest.TestCase):
    def test_exact_schema_and_tile_positions(self):
        result = json.loads(grid.encode(DRAWING))
        self.assertEqual(result, {
            "schema": 1, "player": ["-3", "0"], "key": ["-2", "0"],
            "exit": ["3", "0"],
            "walls": [{"center": ["0", "0"], "half_extents": ["0.5", "0.5"]}],
        })

    def test_deterministic_line_endings_and_final_newline(self):
        expected = grid.encode(DRAWING)
        self.assertEqual(expected, grid.encode(DRAWING[:-1]))
        self.assertEqual(expected, grid.encode(DRAWING.replace(b"\n", b"\r\n")))
        self.assertEqual(expected, grid.encode(DRAWING))
        self.assertTrue(expected.endswith(b"\n"))

    def test_centered_even_dimensions_without_float_rounding(self):
        result = json.loads(grid.encode(b"P.K.\n#..E"))
        self.assertEqual(result["player"], ["-1.5", "-0.5"])
        self.assertEqual(result["key"], ["0.5", "-0.5"])
        self.assertEqual(result["exit"], ["1.5", "0.5"])
        self.assertEqual(result["walls"][0]["center"], ["-1.5", "0.5"])
        self.assertEqual([grid.half_integer(n) for n in range(-3, 4)],
                         ["-1.5", "-1", "-0.5", "0", "0.5", "1", "1.5"])

    def test_wall_order_is_row_major(self):
        result = json.loads(grid.encode(b"PK#\n#E#"))
        self.assertEqual([wall["center"] for wall in result["walls"]],
                         [["1", "-0.5"], ["-1", "0.5"], ["1", "0.5"]])

    def test_zero_and_sixty_four_walls(self):
        self.assertEqual(json.loads(grid.encode(b"PKE"))["walls"], [])
        sixty_four = b"PKE" + b"." * 29 + b"\n" + b"#" * 32 + b"\n" + b"#" * 32
        result = grid.encode(sixty_four)
        self.assertEqual(len(json.loads(result)["walls"]), 64)
        self.assertLess(len(result), 65536)
        with self.assertRaisesRegex(ValueError, "more than 64"):
            grid.encode(sixty_four.replace(b".", b"#", 1))

    def test_dimension_boundaries(self):
        self.assertEqual(json.loads(grid.encode(b"P\nK\nE"))["player"], ["0", "-1"])
        rows = [b"PK" + b"." * 31] + [b"." * 33] * 31 + [b"." * 32 + b"E"]
        result = json.loads(grid.encode(b"\n".join(rows)))
        self.assertEqual(result["player"], ["-16", "-16"])
        self.assertEqual(result["exit"], ["16", "16"])
        for bad in [b"PKE" + b"." * 31, b"P\nK\nE\n" + b".\n" * 31]:
            with self.subTest(bad=bad), self.assertRaisesRegex(ValueError, "1..33"):
                grid.encode(bad)

    def test_missing_duplicate_and_lowercase_actor_tiles(self):
        for bad in [b"...", b"P..", b"PK.", b"PPEK", b"PKKE", b"PKEE", b"pKE"]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                grid.encode(bad)

    def test_invalid_whitespace_ragged_rows_and_non_ascii(self):
        for bad in [b"", b"\n", b"PKE\n\n", b"\nPKE", b"PK\nE", b"P K E",
                    b"PKE\t", b"PKE\r", b"PKE\x00", b"PKE\v", b"PKE\f",
                    "PKÉ".encode(), b"\xef\xbb\xbfPKE", b"PKE/../file", b"PKE$(x)"]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                grid.encode(bad)

    def test_input_type_and_byte_bound(self):
        for bad in [None, "PKE", bytearray(b"PKE"), b"x" * 4097]:
            with self.subTest(kind=type(bad)), self.assertRaises(ValueError):
                grid.encode(bad)

    def test_regular_file_is_captured_read_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "맵 Ω.txt"
            path.write_bytes(DRAWING)
            with patch.object(grid.os, "open", wraps=os.open) as opened:
                self.assertEqual(grid.read_grid(str(path)), DRAWING)
            self.assertEqual(opened.call_args.args,
                             (str(path), os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK))
            self.assertEqual(path.read_bytes(), DRAWING)

    def test_special_files_and_symlinks_rejected_without_hanging(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            regular = root / "grid.txt"
            regular.write_bytes(DRAWING)
            link = root / "link"
            link.symlink_to(regular)
            dangling = root / "dangling"
            dangling.symlink_to(root / "missing")
            fifo = root / "fifo"
            os.mkfifo(fifo)
            for path in [root, link, dangling, fifo]:
                result = subprocess.run([sys.executable, "-B", str(TOOL), "--input", str(path)],
                                        capture_output=True, timeout=5)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, b"")
            self.assertEqual(regular.read_bytes(), DRAWING)
            self.assertTrue(link.is_symlink())
            self.assertTrue(dangling.is_symlink())
            self.assertTrue(stat.S_ISFIFO(fifo.lstat().st_mode))

    def test_file_size_and_growth_bounds(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "large"
            path.write_bytes(b"x" * 4097)
            with self.assertRaisesRegex(ValueError, "at most 4096"):
                grid.read_grid(str(path))
            actual = path.stat()
            fields = list(actual)
            fields[6] = 0  # File grows after metadata capture.
            with patch.object(grid.os, "fstat", return_value=os.stat_result(fields)):
                with self.assertRaisesRegex(ValueError, "exceeds 4096"):
                    grid.read_grid(str(path))

    def test_read_error_closes_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "grid"
            path.write_bytes(DRAWING)
            descriptors = []
            real_open = os.open

            def track(*args):
                descriptor = real_open(*args)
                descriptors.append(descriptor)
                return descriptor

            with patch.object(grid.os, "open", side_effect=track), \
                 patch.object(grid.os, "fstat", side_effect=OSError("metadata failure")):
                with self.assertRaisesRegex(OSError, "metadata failure"):
                    grid.read_grid(str(path))
            with self.assertRaises(OSError):
                os.fstat(descriptors[0])

    def test_path_bounds(self):
        for path in [None, "", "x" * 4097]:
            with self.subTest(path=type(path)), self.assertRaises(ValueError):
                grid.read_grid(path)

    def test_cli_emits_only_json_and_changes_no_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "grid Ω.txt"
            path.write_bytes(DRAWING)
            (root / "orr.project.json").write_bytes(b"untouched")
            before = inventory(root)
            for args in [["--input", str(path)], ["--input=" + str(path)]]:
                result = subprocess.run([sys.executable, "-B", str(TOOL), *args],
                                        capture_output=True, timeout=5, cwd=root)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, b"")
                self.assertEqual(result.stdout, grid.encode(DRAWING))
                self.assertEqual(inventory(root), before)

    def test_cli_bad_input_has_empty_stdout_and_exit_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "bad"
            path.write_bytes(b"PKE\n\n")
            before = inventory(root)
            for name in [path, root / "missing"]:
                result = subprocess.run([sys.executable, "-B", str(TOOL), "--input", str(name)],
                                        capture_output=True, timeout=5)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, b"")
                self.assertTrue(result.stderr.startswith(b"error: "))
                self.assertEqual(inventory(root), before)

    def test_cli_usage_rejected_before_reading(self):
        for args in [[], ["--input"], ["--input=x", "--input=y"],
                     ["--input", "x", "--input", "y"], ["--input=x", "--output=y"],
                     ["--in=x"], ["--input=x", "extra"], ["--input", "x" * 4097]]:
            with self.subTest(args=args), patch.object(grid, "read_grid") as read:
                with redirect_stdout(io.StringIO()) as out, redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        grid.main(args)
                self.assertEqual(error.exception.code, 2)
                self.assertEqual(out.getvalue(), "")
                read.assert_not_called()


def run_integration(binary_dir, output_dir):
    """Create retained fixtures and real CPU receipts, never build or download binaries."""
    binaries = Path(binary_dir).resolve(strict=True)
    root = Path(output_dir).absolute()
    root.mkdir()  # New evidence directory only; do not overwrite an earlier run.
    root = root.resolve(strict=True)
    executable_hashes = {name: hashlib.sha256((binaries / name).read_bytes()).hexdigest()
                         for name in ("orr_new_arena", "orr_playtest_room")}

    def require(condition, message):
        if not condition:
            raise AssertionError(message)

    def unchanged(label, project, before):
        after = inventory(project)
        (root / (label + "-inventory.json")).write_text(
            json.dumps({"before": before, "after": after}, indent=2) + "\n")
        require(before == after, f"{label}: playtest changed the project")

    def run(label, args, expected=0):
        command = list(map(str, args))
        (root / (label + ".command.json")).write_text(json.dumps(command) + "\n")
        result = subprocess.run(command, capture_output=True, timeout=90)
        (root / (label + ".stdout")).write_bytes(result.stdout)
        (root / (label + ".stderr")).write_bytes(result.stderr)
        (root / (label + ".exit")).write_text(str(result.returncode) + "\n")
        if result.returncode != expected:
            raise AssertionError(f"{label}: expected {expected}, got {result.returncode}: "
                                 f"{result.stdout!r} {result.stderr!r}")
        return result.stdout

    def create(label, drawing):
        input_path = root / (label + " Ω.txt")
        input_path.write_bytes(drawing)
        recipe = run(label + "-compile", [sys.executable, "-B", TOOL, "--input", input_path])
        recipe_path = root / (label + ".json")
        recipe_path.write_bytes(recipe)
        project = root / (label + " Project Ω")
        run(label + "-create", [binaries / "orr_new_arena", "--output", project,
                               "--template", "room-escape-3d-v1", "--seed", "ascii-grid",
                               "--room-layout", recipe_path])
        return project

    def step(ticks, x=0, z=0, interact=False, key=True, won=False):
        return {"ticks": ticks, "move_x": x, "move_z": z, "interact": interact,
                "expect_key": key, "expect_won": won}

    def play(label, project, steps, expected=0):
        script = root / (label + "-route.json")
        script.write_text(json.dumps({"schema": 1, "steps": steps}) + "\n")
        return run(label, [binaries / "orr_playtest_room", "--project", project,
                           "--script", script, "--json"], expected)

    project = create("wall", DRAWING)
    before = inventory(project)
    direct = [step(1, interact=True), step(120, x=1), step(1, interact=True, won=True)]
    blocked = json.loads(play("wall-direct", project, direct, expected=1))
    require(blocked["status"] == "error" and blocked["error"]["kind"] == "assertion",
            "wall route must fail a gameplay assertion")
    require(blocked["steps"][-1]["observed"] == {"key": True, "won": False},
            "wall route must retain key without reaching victory")
    detour = [step(1, interact=True), step(40, z=1), step(120, x=1),
              step(40, z=-1), step(1, interact=True, won=True)]
    first = play("wall-detour", project, detour)
    repeated = play("wall-detour-repeat", project, detour)
    require(first == repeated, "repeated detour reports differ")
    result = json.loads(first)
    require(result["status"] == "passed" and result["ticks"] == 202,
            "detour did not pass at tick 202")
    require(result["steps"][-1]["observed"] == {"key": True, "won": True},
            "detour did not reach victory")
    unchanged("wall", project, before)

    control = create("no-wall", DRAWING.replace(b"#", b"."))
    control_before = inventory(control)
    control_result = json.loads(play("no-wall-direct", control, direct))
    require(control_result["status"] == "passed" and control_result["ticks"] == 122,
            "zero-wall control did not pass at tick 122")
    unchanged("no-wall", control, control_before)

    rows = [list("." * 33) for _ in range(33)]
    rows[0][0], rows[0][1], rows[32][32] = "P", "K", "E"
    for i in range(64):
        rows[16 + i // 33][i % 33] = "#"
    maximum = create("maximum", "\n".join("".join(row) for row in rows).encode())
    maximum_before = inventory(maximum)
    require(json.loads(play("maximum-admit", maximum, [step(0, key=False)]))["status"] == "passed",
            "maximum grid was not admitted")
    unchanged("maximum", maximum, maximum_before)
    even = create("even", b"P.K.\n#..E")
    even_before = inventory(even)
    require(json.loads(play("even-admit", even, [step(0, key=False)]))["status"] == "passed",
            "even-dimension grid was not admitted")
    unchanged("even", even, even_before)
    require(executable_hashes == {
        name: hashlib.sha256((binaries / name).read_bytes()).hexdigest()
        for name in executable_hashes}, "executables changed during integration")
    summary = {"status": "passed", "binaries": executable_hashes,
               "tool_sha256": hashlib.sha256(TOOL.read_bytes()).hexdigest(),
               "checks": ["grid-cli", "creator-admission", "wall-blocks-direct-route",
                          "detour-wins", "byte-identical-repeat", "project-unchanged",
                          "zero-wall-control-wins", "33x33-and-64-walls-admitted",
                          "even-dimension-half-integers-admitted"],
               "scope": "retained existing producer binaries; scripted CPU, not native input/GPU"}
    (root / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    unittest.main()
