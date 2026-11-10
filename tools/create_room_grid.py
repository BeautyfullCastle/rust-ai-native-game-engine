#!/usr/bin/env python3
"""Turn a small ASCII tile drawing into a Room schema-1 layout recipe."""
import argparse
import json
import os
import stat
import sys

MAX_INPUT_BYTES = 4096
MAX_SIDE = 33
MAX_WALLS = 64
ACTORS = {"P": "player", "K": "key", "E": "exit"}


def half_integer(twice):
    """Exact decimal text, without floating-point or negative zero."""
    sign = "-" if twice < 0 else ""
    whole, half = divmod(abs(twice), 2)
    return f"{sign}{whole}" + (".5" if half else "")


def encode(data, *, rotate=0, mirror_x=False, merge_walls=False, border=False):
    """Compile captured bytes. This is not scene admission or a puzzle solver."""
    if type(border) is not bool:
        raise ValueError("border must be a boolean")
    if type(merge_walls) is not bool:
        raise ValueError("merge_walls must be a boolean")
    if type(rotate) is not int or rotate not in (0, 90, 180, 270):
        raise ValueError("rotate must be 0, 90, 180 or 270")
    if type(mirror_x) is not bool:
        raise ValueError("mirror_x must be a boolean")
    if not isinstance(data, bytes) or not 1 <= len(data) <= MAX_INPUT_BYTES:
        raise ValueError("grid must contain 1..4096 bytes")
    try:
        text = data.decode("ascii").replace("\r\n", "\n")
    except UnicodeDecodeError as error:
        raise ValueError("grid requires ASCII P, K, E, # and . tiles") from error
    # Permit exactly one optional final newline, never strip significant tiles.
    rows = text.removesuffix("\n").split("\n")
    height, width = len(rows), len(rows[0])
    if not 1 <= height <= MAX_SIDE or not 1 <= width <= MAX_SIDE:
        raise ValueError("grid must be 1..33 rows and 1..33 columns")
    if any(len(row) != width for row in rows):
        raise ValueError("grid rows must have the same width; blank rows are not allowed")
    actors = {}
    walls = []
    for row, tiles in enumerate(rows):
        for column, tile in enumerate(tiles):
            position = [half_integer(2 * column - width + 1),
                        half_integer(2 * row - height + 1)]
            if tile in ACTORS:
                name = ACTORS[tile]
                if name in actors:
                    raise ValueError(f"grid requires exactly one {tile} tile")
                actors[name] = position
            elif tile == "#":
                walls.append({"center": position, "half_extents": ["0.5", "0.5"]})
                if not merge_walls and len(walls) > MAX_WALLS:
                    raise ValueError("grid has more than 64 wall tiles")
            elif tile != ".":
                raise ValueError(f"invalid tile at row {row + 1}, column {column + 1}; "
                                 "use P, K, E, # or .")
    if set(actors) != set(ACTORS.values()):
        raise ValueError("grid requires exactly one P, one K and one E tile")
    if border:
        if width > MAX_SIDE - 2 or height > MAX_SIDE - 2:
            raise ValueError("border requires an interior of at most 31 rows and columns")
        edge = "#" * (width + 2)
        padded = [edge, *["#" + row + "#" for row in rows], edge]
        return encode("\n".join(padded).encode("ascii"), rotate=rotate,
                      mirror_x=mirror_x, merge_walls=merge_walls)
    # Validate the original drawing first, so diagnostics retain input coordinates.
    # Re-encode the transformed drawing to preserve row-major wall identities.
    if mirror_x or rotate:
        if mirror_x:
            rows = [row[::-1] for row in rows]
        for _ in range(rotate // 90):
            rows = ["".join(row) for row in zip(*rows[::-1])]
        return encode("\n".join(rows).encode("ascii"), merge_walls=merge_walls)
    if merge_walls:
        walls = merged_rectangles(rows)
    recipe = {"schema": 1, **{name: actors[name] for name in ACTORS.values()},
              "walls": walls}
    return (json.dumps(recipe, indent=2) + "\n").encode("ascii")


def merged_rectangles(rows):
    """Row-major, widest-first rectangles; deterministic, not a minimum cover."""
    height, width = len(rows), len(rows[0])
    remaining = [[tile == "#" for tile in row] for row in rows]
    walls = []
    for row in range(height):
        for column in range(width):
            if not remaining[row][column]:
                continue
            right = column
            while right < min(width, column + 16) and remaining[row][right]:
                right += 1
            bottom = row + 1
            while bottom < min(height, row + 16) and all(remaining[bottom][x] for x in range(column, right)):
                bottom += 1
            for y in range(row, bottom):
                for x in range(column, right):
                    remaining[y][x] = False
            walls.append({"center": [half_integer(column + right - width),
                                     half_integer(row + bottom - height)],
                          "half_extents": [half_integer(right - column),
                                           half_integer(bottom - row)]})
            if len(walls) > MAX_WALLS:
                raise ValueError("merged grid has more than 64 wall boxes")
    return walls


def preview_svg(data, *, rotate=0, mirror_x=False, merge_walls=False, border=False):
    """Static schematic from the admitted recipe, not engine rendering or a solver."""
    if type(border) is not bool:
        raise ValueError("border must be a boolean")
    if type(merge_walls) is not bool:
        raise ValueError("merge_walls must be a boolean")
    options = {"merge_walls": True} if merge_walls else {}
    if border:
        options["border"] = True
    recipe = json.loads(encode(data, rotate=rotate, mirror_x=mirror_x, **options))
    # Admission above establishes rectangular ASCII input and bounded dimensions.
    rows = data.decode("ascii").replace("\r\n", "\n").removesuffix("\n").split("\n")
    columns, row_count = len(rows[0]), len(rows)
    if border:
        columns, row_count = columns + 2, row_count + 2
    if rotate in (90, 270):
        columns, row_count = row_count, columns
    cell = 40
    width, height = max(600, columns * cell + 64), row_count * cell + 176
    left, top = (width - columns * cell) // 2, 112

    def pixel(position):
        # encode emits only bounded exact integers and half-integers.
        def twice(value):
            sign = -1 if value.startswith("-") else 1
            whole, _, fraction = value.lstrip("-").partition(".")
            return sign * (int(whole) * 2 + (1 if fraction else 0))
        return (left + (twice(position[0]) + columns) * cell // 2,
                top + (twice(position[1]) + row_count) * cell // 2)

    lines = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
             f'viewBox="0 0 {width} {height}" role="img" aria-labelledby="title description">',
             '<title id="title">Room grid floor-plan preview</title>',
             '<desc id="description">Schematic only. P player, K key, E exit. '
             'Columns increase X; rows increase Z. No solvability or rendered-game claim.</desc>',
             f'<rect x="0" y="0" width="{width}" height="{height}" fill="#f8fafc"/>',
             '<text x="32" y="34" font-family="sans-serif" font-size="22" '
             'font-weight="bold" fill="#0f172a">Room grid preview</text>',
             '<text x="32" y="60" font-family="sans-serif" font-size="14" '
             'fill="#334155">P = player   K = key   E = exit   dark squares = walls</text>',
             f'<text x="32" y="86" font-family="sans-serif" font-size="14" '
             f'fill="#334155">{columns} columns x {row_count} rows | 1 cell = 1 unit | +X right, +Z down</text>',
             f'<rect x="{left}" y="{top}" width="{columns * cell}" '
             f'height="{row_count * cell}" fill="#ffffff" stroke="#64748b"/>']
    for column in range(1, columns):
        x = left + column * cell
        lines.append(f'<line x1="{x}" y1="{top}" x2="{x}" y2="{top + row_count * cell}" '
                     'stroke="#cbd5e1"/>')
    for row in range(1, row_count):
        y = top + row * cell
        lines.append(f'<line x1="{left}" y1="{y}" x2="{left + columns * cell}" y2="{y}" '
                     'stroke="#cbd5e1"/>')
    for index, wall in enumerate(recipe["walls"]):
        x, y = pixel(wall["center"])
        extents = []
        for value in wall["half_extents"]:
            whole, _, fraction = value.partition(".")
            extents.append((int(whole) * 2 + (1 if fraction else 0)) * cell // 2)
        half_x, half_z = extents
        lines.append(f'<rect id="wall-{index}" x="{x - half_x}" y="{y - half_z}" '
                     f'width="{half_x * 2}" height="{half_z * 2}" fill="#334155"/>')
    for name, letter, color in [("player", "P", "#1d4ed8"),
                                ("key", "K", "#92400e"), ("exit", "E", "#047857")]:
        x, y = pixel(recipe[name])
        lines.extend([f'<circle id="{name}" cx="{x}" cy="{y}" r="14" fill="{color}"/>',
                      f'<text x="{x}" y="{y + 5}" text-anchor="middle" font-family="sans-serif" '
                      f'font-size="16" font-weight="bold" fill="#ffffff">{letter}</text>'])
    lines.extend([f'<text x="32" y="{height - 22}" font-family="sans-serif" font-size="14" '
                  'fill="#475569">Schematic only. Check gameplay in the editor or CPU playtest.</text>',
                  '</svg>'])
    if merge_walls:
        lines = [line.replace("dark squares = walls", "dark rectangles = walls") for line in lines]
    result = ("\n".join(lines) + "\n").encode("ascii")
    if len(result) > 256 * 1024:
        raise ValueError("preview exceeds 256 KiB")
    return result


def read_grid(path):
    """Capture one bounded regular file, refusing leaf symlinks and special files."""
    if not isinstance(path, str) or not 1 <= len(path) <= 4096:
        raise ValueError("input path must be 1..4096 characters")
    if not hasattr(os, "O_NOFOLLOW") or not hasattr(os, "O_NONBLOCK"):
        raise OSError("safe grid-file capture requires O_NOFOLLOW and O_NONBLOCK")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_INPUT_BYTES:
            raise ValueError("input must be a regular nonsymlink file at most 4096 bytes")
        data = stream.read(MAX_INPUT_BYTES + 1)
    if len(data) > MAX_INPUT_BYTES:
        raise ValueError("grid exceeds 4096 bytes")
    return data


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--input", required=True, action="append", help="ASCII grid file")
    parser.add_argument("--rotate", choices=("0", "90", "180", "270"),
                        action="append", help="clockwise degrees after mirroring")
    parser.add_argument("--mirror-x", action="count", default=0,
                        help="mirror left/right before rotating")
    parser.add_argument("--format", choices=("json", "svg"), action="append",
                        help="output format (default: json; svg is a static schematic)")
    parser.add_argument("--merge-walls", action="count", default=0,
                        help="partition adjacent wall tiles into at most 64 rectangles")
    parser.add_argument("--border", action="count", default=0,
                        help="add one closed wall layer outside the interior drawing")
    if len(argv) > 9 or any(len(arg) > 4096 for arg in argv):
        parser.error("too many or oversized arguments")
    args = parser.parse_args(argv)
    if len(args.input) != 1:
        parser.error("--input must occur exactly once")
    if args.rotate is not None and len(args.rotate) != 1:
        parser.error("--rotate must occur at most once")
    if args.mirror_x > 1:
        parser.error("--mirror-x must occur at most once")
    if args.format is not None and len(args.format) != 1:
        parser.error("--format must occur at most once")
    if args.merge_walls > 1:
        parser.error("--merge-walls must occur at most once")
    if args.border > 1:
        parser.error("--border must occur at most once")
    try:
        output = preview_svg if args.format == ["svg"] else encode
        result = output(read_grid(args.input[0]),
                        rotate=int(args.rotate[0]) if args.rotate else 0,
                        mirror_x=bool(args.mirror_x), merge_walls=bool(args.merge_walls),
                        **({"border": True} if args.border else {}))
        sys.stdout.write(result.decode("ascii"))
    except (OSError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
