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
import xml.etree.ElementTree as ET
from fractions import Fraction
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



class GridTransformTests(unittest.TestCase):
    def test_explicit_default_is_byte_identical(self):
        for drawing in [DRAWING, b"P.K.\n#..E", b"P\nK\nE"]:
            self.assertEqual(grid.encode(drawing), grid.encode(drawing, rotate=0, mirror_x=False))

    def test_all_eight_transforms_match_hand_drawn_rectangles(self):
        source = b"P.K.\n#..E"
        expected = {
            (0, False): b"P.K.\n#..E", (90, False): b"#P\n..\n.K\nE.",
            (180, False): b"E..#\n.K.P", (270, False): b".E\nK.\n..\nP#",
            (0, True): b".K.P\nE..#", (90, True): b"E.\n.K\n..\n#P",
            (180, True): b"#..E\nP.K.", (270, True): b"P#\n..\nK.\n.E",
        }
        for (rotate, mirror), drawing in expected.items():
            with self.subTest(rotate=rotate, mirror=mirror):
                self.assertEqual(grid.encode(source, rotate=rotate, mirror_x=mirror),
                                 grid.encode(drawing))

    def test_transformed_walls_are_in_new_row_major_order(self):
        source = b"P#K\n#E#"
        for rotation in (0, 90, 180, 270):
            for mirror in (False, True):
                result = json.loads(grid.encode(source, rotate=rotation, mirror_x=mirror))
                centers = [wall["center"] for wall in result["walls"]]
                # All coordinates in this fixture are integer/half-integer: doubled
                # Decimal values remain exact and avoid an oracle using grid code.
                from decimal import Decimal
                pairs = [(Decimal(z), Decimal(x)) for x, z in centers]
                self.assertEqual(pairs, sorted(pairs))
                self.assertEqual(len(centers), 3)

    def test_all_transform_coordinates_have_exact_geometric_relation(self):
        from decimal import Decimal
        for height, width in ((1, 33), (33, 1), (2, 4), (4, 2), (33, 33)):
            cells = ["."] * (height * width)
            cells[0], cells[1], cells[-1] = "P", "K", "E"
            drawing = "\n".join("".join(cells[r * width:(r + 1) * width])
                                for r in range(height)).encode()
            original = json.loads(grid.encode(drawing))
            for rotate in (0, 90, 180, 270):
                for mirror in (False, True):
                    result = json.loads(grid.encode(drawing, rotate=rotate, mirror_x=mirror))
                    for actor in ("player", "key", "exit"):
                        x, z = map(Decimal, original[actor])
                        if mirror:
                            x = -x
                        for _ in range(rotate // 90):
                            x, z = -z, x
                        self.assertEqual(list(map(Decimal, result[actor])), [x, z])

    def test_transform_parameter_types_and_values_are_strict(self):
        for rotate in (True, False, 90.0, "90", None, -90, 360, 45):
            with self.subTest(rotate=rotate), self.assertRaises(ValueError):
                grid.encode(DRAWING, rotate=rotate)
        for mirror in (0, 1, "true", None, []):
            with self.subTest(mirror=mirror), self.assertRaises(ValueError):
                grid.encode(DRAWING, mirror_x=mirror)

    def test_transforms_never_relax_input_admission(self):
        bad = [b"PKEE", b"PK\nE", b"PKE ", b"PKE\n\n", b"PKE" + b"#" * 65,
               b"PKE" + b"." * 31, b"PKE\x00", b"PKE" + bytes([255])]
        for drawing in bad:
            for rotation in (0, 90, 180, 270):
                for mirror in (False, True):
                    with self.subTest(drawing=drawing, rotate=rotation, mirror=mirror):
                        with self.assertRaises(ValueError):
                            grid.encode(drawing, rotate=rotation, mirror_x=mirror)

    def test_cli_transforms_in_any_option_order_without_file_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "grid Ω.txt"
            path.write_bytes(b"P.K.\n#..E")
            before = inventory(Path(tmp))
            for rotation in (0, 90, 180, 270):
                for mirror in (False, True):
                    option = ["--mirror-x"] if mirror else []
                    for args in (["--input", str(path), "--rotate", str(rotation), *option],
                                 [*option, "--rotate=" + str(rotation), "--input=" + str(path)]):
                        result = subprocess.run([sys.executable, "-B", str(TOOL), *args],
                                                capture_output=True, timeout=5)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(result.stderr, b"")
                        self.assertEqual(result.stdout, grid.encode(path.read_bytes(),
                                         rotate=rotation, mirror_x=mirror))
                        self.assertEqual(inventory(Path(tmp)), before)

    def test_invalid_and_duplicate_options_fail_before_read(self):
        for extra in (["--rotate", "90", "--rotate", "180"], ["--rotate=0", "--rotate=0"],
                      ["--mirror-x", "--mirror-x"], ["--rotate", "45"], ["--rotate=-90"],
                      ["--rotate=360"], ["--rotate=90.0"], ["--rotate=090"],
                      ["--mirror-x=true"], ["--mirror"], ["--rotate"]):
            with self.subTest(extra=extra), patch.object(grid, "read_grid") as read:
                with redirect_stdout(io.StringIO()) as out, redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        grid.main(["--input=x", *extra])
                self.assertEqual(error.exception.code, 2)
                self.assertEqual(out.getvalue(), "")
                read.assert_not_called()


class GridPreviewTests(unittest.TestCase):
    def test_static_svg_security_allowlist_and_labels(self):
        raw = grid.preview_svg(DRAWING)
        root = ET.fromstring(raw)
        allowed = {"svg", "title", "desc", "rect", "text", "line", "circle"}
        attributes = {"width", "height", "viewBox", "role", "aria-labelledby", "id",
                      "x", "y", "fill", "font-family", "font-size", "font-weight",
                      "stroke", "x1", "x2", "y1", "y2", "cx", "cy", "r", "text-anchor"}
        for element in root.iter():
            self.assertTrue(element.tag.startswith("{http://www.w3.org/2000/svg}"))
            self.assertIn(element.tag.split("}")[1], allowed)
            self.assertLessEqual(set(element.attrib), attributes)
        self.assertEqual(root.attrib["role"], "img")
        texts = [element.text for element in root.iter() if element.tag.endswith("}text")]
        for letter in "PKE":
            self.assertIn(letter, texts)
        self.assertIn("Schematic only", raw.decode())
        for forbidden in [b"<!", b"href", b"url(", b"script", b"foreignObject", b"style=", b"file:"]:
            # 'description' is an accessibility identifier, not a script element.
            if forbidden == b"script":
                self.assertNotIn(b"<script", raw)
            else:
                self.assertNotIn(forbidden, raw)

    def test_all_eight_preview_coordinates_match_admitted_json(self):
        source = b"P.K.\n#..E"
        for mirror in (False, True):
            for rotate in (0, 90, 180, 270):
                with self.subTest(rotate=rotate, mirror=mirror):
                    recipe = json.loads(grid.encode(source, rotate=rotate, mirror_x=mirror))
                    root = ET.fromstring(grid.preview_svg(source, rotate=rotate, mirror_x=mirror))
                    width, rows = (2, 4) if rotate in (90, 270) else (4, 2)
                    left = (int(root.attrib["width"]) - width * 40) // 2
                    elements = {e.attrib["id"]: e for e in root.iter() if "id" in e.attrib}
                    for name in ("player", "key", "exit"):
                        x, z = map(Fraction, recipe[name])
                        self.assertEqual(Fraction(elements[name].attrib["cx"]), left + 20 * width + 40 * x)
                        self.assertEqual(Fraction(elements[name].attrib["cy"]), 112 + 20 * rows + 40 * z)
                    for index, wall in enumerate(recipe["walls"]):
                        x, z = map(Fraction, wall["center"])
                        element = elements[f"wall-{index}"]
                        self.assertEqual(Fraction(element.attrib["x"]), left + 20 * width + 40 * x - 20)
                        self.assertEqual(Fraction(element.attrib["y"]), 112 + 20 * rows + 40 * z - 20)
                        self.assertEqual((element.attrib["width"], element.attrib["height"]), ("40", "40"))

    def test_preview_deterministic_line_endings_and_repeat(self):
        expected = grid.preview_svg(DRAWING, rotate=90, mirror_x=True)
        for data in (DRAWING, DRAWING[:-1], DRAWING.replace(b"\n", b"\r\n")):
            self.assertEqual(grid.preview_svg(data, rotate=90, mirror_x=True), expected)
        self.assertTrue(expected.endswith(b"\n"))

    def test_preview_minimum_maximum_and_wall_count_bounds(self):
        maximum = list("." * (33 * 33))
        maximum[:3] = "PKE"
        maximum[3:67] = "#" * 64
        drawing = "\n".join("".join(maximum[i:i + 33]) for i in range(0, len(maximum), 33)).encode()
        for data, walls in [(b"PKE", 0), (b"P\nK\nE", 0), (drawing, 64)]:
            for mirror in (False, True):
                for rotation in (0, 90, 180, 270):
                    raw = grid.preview_svg(data, rotate=rotation, mirror_x=mirror)
                    self.assertLess(len(raw), 256 * 1024)
                    root = ET.fromstring(raw)
                    actual = [e for e in root.iter() if e.attrib.get("id", "").startswith("wall-")]
                    self.assertEqual(len(actual), walls)
                    width, height = int(root.attrib["width"]), int(root.attrib["height"])
                    for element in actual:
                        self.assertGreaterEqual(int(element.attrib["x"]), 0)
                        self.assertGreaterEqual(int(element.attrib["y"]), 0)
                        self.assertLessEqual(int(element.attrib["x"]) + 40, width)
                        self.assertLessEqual(int(element.attrib["y"]) + 40, height)

    def test_preview_preserves_admission_errors(self):
        for data in (b"", b"PKE\n\n", b"PKE<script>", b"PK", b"PPKE", b"PKE\xff"):
            with self.subTest(data=data), self.assertRaises(ValueError):
                grid.preview_svg(data)
        for options in ({"rotate": True}, {"rotate": 360}, {"mirror_x": 1}):
            with self.assertRaises(ValueError):
                grid.preview_svg(DRAWING, **options)
        with patch.object(grid, "encode", wraps=grid.encode) as encoded:
            grid.preview_svg(DRAWING)
            encoded.assert_called_once_with(DRAWING, rotate=0, mirror_x=False)

    def test_cli_svg_all_transforms_capture_once_and_no_file_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "private-name-secret.txt"
            path.write_bytes(DRAWING)
            before = inventory(root)
            for mirror in (False, True):
                for rotate in (0, 90, 180, 270):
                    args = ["--input", str(path), "--format", "svg", "--rotate", str(rotate)]
                    if mirror:
                        args.append("--mirror-x")
                    result = subprocess.run([sys.executable, "-B", str(TOOL), *args], capture_output=True, timeout=5)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stderr, b"")
                    self.assertEqual(result.stdout, grid.preview_svg(DRAWING, rotate=rotate, mirror_x=mirror))
                    self.assertNotIn(b"private-name-secret", result.stdout)
            self.assertEqual(inventory(root), before)
        with patch.object(grid, "read_grid", return_value=DRAWING) as read:
            with redirect_stdout(io.StringIO()):
                self.assertEqual(grid.main(["--input=x", "--format=svg"]), 0)
            read.assert_called_once_with("x")

    def test_cli_explicit_json_preserves_all_transform_output(self):
        for mirror in (False, True):
            for rotate in (0, 90, 180, 270):
                args = ["--input=x", "--format=json", f"--rotate={rotate}"]
                if mirror:
                    args.append("--mirror-x")
                with patch.object(grid, "read_grid", return_value=DRAWING), redirect_stdout(io.StringIO()) as out:
                    self.assertEqual(grid.main(args), 0)
                self.assertEqual(out.getvalue().encode(), grid.encode(DRAWING, rotate=rotate, mirror_x=mirror))

    def test_cli_invalid_formats_rejected_before_reading(self):
        for args in [["--input=x", "--format"], ["--input=x", "--format=png"],
                     ["--input=x", "--format=svg", "--format=json"],
                     ["--input=x", "--format=svg", "--format=svg"],
                     ["--input=x", "--for=svg"], ["--input=x", "--format=SVG"],
                     ["--input=x", "--format=svg", "extra"]]:
            with patch.object(grid, "read_grid") as read, redirect_stdout(io.StringIO()) as out, redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    grid.main(args)
                self.assertEqual(error.exception.code, 2)
                self.assertEqual(out.getvalue(), "")
                read.assert_not_called()

    def test_cli_svg_invalid_input_has_empty_stdout(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bad").write_bytes(b"PKE<svg>")
            (root / "link").symlink_to(root / "bad")
            for name in ("bad", "link", "missing"):
                result = subprocess.run([sys.executable, "-B", str(TOOL), "--input", str(root / name), "--format=svg"], capture_output=True, timeout=5)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, b"")
                self.assertTrue(result.stderr.startswith(b"error: "))


class GridMergeTests(unittest.TestCase):
    def test_rectangle_partition_exact_positions(self):
        data = b"PKE..\n####.\n####."
        result = json.loads(grid.encode(data, merge_walls=True))
        self.assertEqual(result["walls"], [{"center": ["-0.5", "0.5"], "half_extents": ["2", "1"]}])
        self.assertEqual(result["player"], ["-2", "-1"])

    def test_merge_option_types_are_strict(self):
        for value in [None, 0, 1, "true", [], {}]:
            with self.assertRaises(ValueError):
                grid.encode(DRAWING, merge_walls=value)
            with self.assertRaises(ValueError):
                grid.preview_svg(DRAWING, merge_walls=value)

    def test_partition_covers_exact_cells_without_overlaps(self):
        for bits in range(512):
            rows = ["PKE"] + ["".join("#" if bits & (1 << (y * 3 + x)) else "." for x in range(3)) for y in range(3)]
            recipe = json.loads(grid.encode("\n".join(rows).encode(), merge_walls=True))
            covered = set()
            for wall in recipe["walls"]:
                cx, cz = map(Fraction, wall["center"])
                hx, hz = map(Fraction, wall["half_extents"])
                cells = {(x, y) for y in range(4) for x in range(3)
                         if abs(Fraction(x - 1) - cx) < hx and abs(Fraction(2*y - 3, 2) - cz) < hz}
                self.assertTrue(cells)
                self.assertFalse(covered & cells)
                covered |= cells
            self.assertEqual(covered, {(x,y) for y,row in enumerate(rows) for x,tile in enumerate(row) if tile == "#"})

    def test_transform_partition_and_svg_geometry(self):
        data = b"PKE..\n####.\n.###."
        for mirror in (False, True):
            for rotation in (0,90,180,270):
                rows = data.decode().splitlines()
                if mirror:
                    rows = [r[::-1] for r in rows]
                for _ in range(rotation//90):
                    rows = ["".join(r) for r in zip(*rows[::-1])]
                expected = grid.encode("\n".join(rows).encode(), merge_walls=True)
                actual = grid.encode(data, rotate=rotation, mirror_x=mirror, merge_walls=True)
                self.assertEqual(actual, expected)
                recipe = json.loads(actual)
                root = ET.fromstring(grid.preview_svg(data, rotate=rotation, mirror_x=mirror, merge_walls=True))
                for i,wall in enumerate(recipe["walls"]):
                    node = next(n for n in root if n.get("id") == f"wall-{i}")
                    self.assertEqual(Fraction(node.get("width")), Fraction(wall["half_extents"][0])*80)
                    self.assertEqual(Fraction(node.get("height")), Fraction(wall["half_extents"][1])*80)

    def test_more_than_sixty_four_tiles_can_merge(self):
        data = ("PKE" + "."*30 + "\n" + ("#"*33 + "\n")*3).encode()
        with self.assertRaises(ValueError):
            grid.encode(data)
        walls = json.loads(grid.encode(data, merge_walls=True))["walls"]
        self.assertEqual(len(walls), 3)
        self.assertEqual([wall["half_extents"] for wall in walls], [["8", "1.5"], ["8", "1.5"], ["0.5", "1.5"]])
        for mirror in (False, True):
            for rotation in (0,90,180,270):
                changed = json.loads(grid.encode(data, rotate=rotation, mirror_x=mirror, merge_walls=True))
                self.assertEqual(len(changed["walls"]),3)
                self.assertTrue(all(Fraction(v) <= 8 for wall in changed["walls"] for v in wall["half_extents"]))

    def test_more_than_sixty_four_boxes_rejected(self):
        rows = ["PKE" + "."*30] + ["".join("#" if (x+y)%2 == 0 else "." for x in range(33)) for y in range(5)]
        with self.assertRaisesRegex(ValueError, "64 wall boxes"):
            grid.encode("\n".join(rows).encode(), merge_walls=True)

    def test_merge_does_not_relax_invalid_input(self):
        for data in [b"PKE ", b"PK", b"PKE\n#", b"PPKE", b"PKE\r", b"PKE\n\n"]:
            with self.assertRaises(ValueError):
                grid.encode(data, merge_walls=True)
        self.assertEqual(json.loads(grid.encode(b"PKE", merge_walls=True))["walls"], [])

    def test_cli_merge_formats_and_duplicate_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/"input.txt"
            path.write_bytes(b"PKE..\n####.\n####.")
            before = inventory(Path(directory))
            for fmt in ("json","svg"):
                result = subprocess.run([sys.executable,"-B",TOOL,"--input",path,"--merge-walls","--rotate","90","--mirror-x","--format",fmt], capture_output=True)
                self.assertEqual(result.returncode,0,result.stderr)
                method = grid.encode if fmt == "json" else grid.preview_svg
                self.assertEqual(result.stdout,method(path.read_bytes(),rotate=90,mirror_x=True,merge_walls=True))
            for extra in [["--merge-walls","--merge-walls"], ["--merge-walls=true"], ["--merge"]]:
                result = subprocess.run([sys.executable,"-B",TOOL,"--input",path,*extra],capture_output=True)
                self.assertEqual(result.returncode,2)
                self.assertEqual(result.stdout,b"")
            self.assertEqual(inventory(Path(directory)),before)


def run_integration(binary_dir, output_dir, *, rotate=0, mirror_x=False):
    """Create retained fixtures and real CPU receipts, never build or download binaries."""
    grid.encode(DRAWING, rotate=rotate, mirror_x=mirror_x)  # Validate before any writes.
    transform_args = ["--rotate", str(rotate)] + (["--mirror-x"] if mirror_x else [])
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
        recipe = run(label + "-compile", [sys.executable, "-B", TOOL, "--input", input_path, *transform_args])
        recipe_path = root / (label + ".json")
        recipe_path.write_bytes(recipe)
        project = root / (label + " Project Ω")
        run(label + "-create", [binaries / "orr_new_arena", "--output", project,
                               "--template", "room-escape-3d-v1", "--seed", "ascii-grid",
                               "--room-layout", recipe_path])
        return project

    def step(ticks, x=0, z=0, interact=False, key=True, won=False):
        if mirror_x:
            x = -x
        for _ in range(rotate // 90):
            x, z = -z, x
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
    if rotate or mirror_x:
        summary["transformation"] = {"rotate": rotate, "mirror_x": mirror_x}
    (root / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


def run_transform_integration(binary_dir, output_dir):
    """Exercise seven nonidentity transforms through actual creator and CPU gameplay."""
    root = Path(output_dir).absolute()
    root.mkdir()  # Preserve all earlier evidence; never reuse a directory.
    checks = []
    binaries = None
    tool_hash = hashlib.sha256(TOOL.read_bytes()).hexdigest()
    expected = ["grid-cli", "creator-admission", "wall-blocks-direct-route",
                "detour-wins", "byte-identical-repeat", "project-unchanged",
                "zero-wall-control-wins", "33x33-and-64-walls-admitted",
                "even-dimension-half-integers-admitted"]
    for mirror in (False, True):
        for rotation in (0, 90, 180, 270):
            if rotation == 0 and not mirror:
                continue
            label = f"mirror-{int(mirror)}-rotate-{rotation}"
            with redirect_stdout(io.StringIO()) as printed:
                run_integration(binary_dir, root / label, rotate=rotation, mirror_x=mirror)
            report_bytes = (root / label / "result.json").read_bytes()
            if len(report_bytes) > 16384 or len(printed.getvalue()) > 16384:
                raise AssertionError(f"{label}: oversized result")
            report = json.loads(report_bytes)
            transform = report.get("transformation", {"rotate": 0, "mirror_x": False})
            if (json.loads(printed.getvalue()) != report or report["status"] != "passed"
                    or report["checks"] != expected or report["tool_sha256"] != tool_hash
                    or transform != {"rotate": rotation, "mirror_x": mirror}):
                raise AssertionError(f"{label}: invalid result")
            if binaries is None:
                binaries = report["binaries"]
            if report["binaries"] != binaries:
                raise AssertionError(f"{label}: producer identities changed")
            checks.append(label)
    summary = {"status": "passed", "checks": checks, "binaries": binaries,
               "tool_sha256": tool_hash, "checks_per_transform": expected,
               "scope": "retained existing producer binaries; scripted CPU, not native input/GPU"}
    (root / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


def _run_merge_case(binary_dir, output_dir, *, rotate=0, mirror_x=False):
    """Create retained fixtures and real CPU receipts, never build or download binaries."""
    grid.encode(DRAWING, rotate=rotate, mirror_x=mirror_x)  # Validate before any writes.
    transform_args = ["--rotate", str(rotate)] + (["--mirror-x"] if mirror_x else [])
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
        recipe = run(label + "-compile", [sys.executable, "-B", TOOL, "--input", input_path, "--merge-walls", *transform_args])
        recipe_path = root / (label + ".json")
        recipe_path.write_bytes(recipe)
        project = root / (label + " Project Ω")
        run(label + "-create", [binaries / "orr_new_arena", "--output", project,
                               "--template", "room-escape-3d-v1", "--seed", "ascii-grid",
                               "--room-layout", recipe_path])
        return project

    def step(ticks, x=0, z=0, interact=False, key=True, won=False):
        if mirror_x:
            x = -x
        for _ in range(rotate // 90):
            x, z = -z, x
        return {"ticks": ticks, "move_x": x, "move_z": z, "interact": interact,
                "expect_key": key, "expect_won": won}

    def play(label, project, steps, expected=0):
        script = root / (label + "-route.json")
        script.write_text(json.dumps({"schema": 1, "steps": steps}) + "\n")
        return run(label, [binaries / "orr_playtest_room", "--project", project,
                           "--script", script, "--json"], expected)

    project = create("wall", b"...#...\nPK.#..E\n.......\n")
    require(len(json.loads((root / "wall.json").read_bytes())["walls"]) == 1,
            "contiguous barrier did not merge into one wall box")
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

    control = create("no-wall", b"...#...\nPK.#..E\n.......\n".replace(b"#", b"."))
    control_before = inventory(control)
    control_result = json.loads(play("no-wall-direct", control, direct))
    require(control_result["status"] == "passed" and control_result["ticks"] == 122,
            "zero-wall control did not pass at tick 122")
    unchanged("no-wall", control, control_before)

    rows = [list("." * 33) for _ in range(33)]
    rows[0][0], rows[0][1], rows[32][32] = "P", "K", "E"
    for i in range(99):
        rows[16 + i // 33][i % 33] = "#"
    maximum = create("maximum", "\n".join("".join(row) for row in rows).encode())
    require(len(json.loads((root / "maximum.json").read_bytes())["walls"]) == 3,
            "99 wall tiles did not merge into three bounded wall boxes")
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
                          "zero-wall-control-wins", "33x33-and-99-wall-tiles-merged-admitted",
                          "even-dimension-half-integers-admitted"],
               "scope": "retained existing producer binaries; merged geometry CPU only, not collision/checksum equivalence"}
    summary["transformation"] = {"rotate": rotate, "mirror_x": mirror_x}
    (root / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))



def run_merge_integration(binary_dir, output_dir):
    """All eight merged geometry cases against explicit production executables."""
    root = Path(output_dir).absolute()
    root.mkdir()
    checks = []
    hashes = None
    tool_hash = hashlib.sha256(TOOL.read_bytes()).hexdigest()
    expected_checks = ["grid-cli", "creator-admission", "wall-blocks-direct-route",
                       "detour-wins", "byte-identical-repeat", "project-unchanged",
                       "zero-wall-control-wins", "33x33-and-99-wall-tiles-merged-admitted",
                       "even-dimension-half-integers-admitted"]
    for mirror in (False, True):
        for rotation in (0,90,180,270):
            label = f"mirror-{int(mirror)}-rotate-{rotation}"
            with redirect_stdout(io.StringIO()) as captured:
                _run_merge_case(binary_dir, root/label, rotate=rotation, mirror_x=mirror)
            result = json.loads((root/label/"result.json").read_bytes())
            if (result != json.loads(captured.getvalue()) or result["status"] != "passed"
                    or result["checks"] != expected_checks or result["tool_sha256"] != tool_hash
                    or result["transformation"] != {"rotate":rotation,"mirror_x":mirror}):
                raise AssertionError("merged integration result identity mismatch")
            if hashes is None:
                hashes = result["binaries"]
            if result["binaries"] != hashes:
                raise AssertionError("merged integration producer identities changed")
            checks.append(label)
    # A disconnected checkerboard requires >64 boxes even after merging.
    rejected = root/"too-many-boxes.txt"
    rejected.write_text("PKE" + "."*30 + "\n" + "\n".join("".join("#" if (x+y)%2 == 0 else "." for x in range(33)) for y in range(5)))
    command = [sys.executable,"-B",str(TOOL),"--input",str(rejected),"--merge-walls"]
    result = subprocess.run(command,capture_output=True,timeout=10)
    (root/"too-many-boxes.command.json").write_text(json.dumps(command)+"\n")
    (root/"too-many-boxes.stdout").write_bytes(result.stdout)
    (root/"too-many-boxes.stderr").write_bytes(result.stderr)
    (root/"too-many-boxes.exit").write_text(str(result.returncode)+"\n")
    if result.returncode != 1 or result.stdout or b"64 wall boxes" not in result.stderr:
        raise AssertionError("too many merged boxes were not refused")
    checks.append("more-than-64-boxes-rejected")
    summary = {"status":"passed","checks":checks,"checks_per_transform":expected_checks,
               "binaries":hashes,"tool_sha256":tool_hash,
               "scope":"retained production binaries, merged geometry CPU; not physics/checksum equivalence"}
    (root/"result.json").write_text(json.dumps(summary,indent=2)+"\n")
    print(json.dumps(summary,indent=2))


if __name__ == "__main__":
    unittest.main()
