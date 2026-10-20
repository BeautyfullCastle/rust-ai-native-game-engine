#!/usr/bin/env python3
"""Fail-closed current-source acceptance for PNG sequence named clips.

The CPU and GPU lanes build their own real CLI and retain exact Cargo artifacts.
A compiled test's complete inventory, ignored inventory and execution must agree.
The shared bounded runner is reused unchanged; old PNG gates stay independent.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import struct
import zlib

if not __debug__ or sys.flags.optimize or os.environ.get("PYTHONOPTIMIZE") not in (None, "", "0"):
    raise SystemExit("PNG named-clips acceptance requires nonoptimized Python")

import static_model_extract_gate as runner

CLI = "png_sequence_package"
CPU_TARGET = "png_sequence_clips"
SAMPLE_TARGET = "png_sequence_clips_gpu"
EDITOR_TARGET = "png_sequence_clips_ui"
SAMPLE_FEATURES = ["project", "project-create", "sprites"]
EDITOR_FEATURES = ["sprites"]
# Fixed acceptance names are reviewed with the consumer contracts, never
# discovered from a successful run and silently accepted as the inventory.
CPU_TESTS = (
    "shared_source_table_preserves_literal_pixels_regions_clips_and_relocated_bytes",
    "overlapping_repeated_clips_have_independent_exact_loop_once_boundaries",
    "unused_source_pixels_are_packed_but_do_not_change_selected_clip_semantics",
    "collection_caps_accept_exact_limits_and_reject_empty_or_one_past",
    "schema3_requires_its_exact_fields_and_rejects_legacy_or_unknown_fields",
    "every_schema3_object_rejects_duplicates_null_and_positional_arrays",
    "clip_names_use_unique_nonblank_utf8_byte_bounded_identifiers",
    "references_and_durations_are_strict_and_duration_overflow_is_per_clip",
    "common_canvas_full_rectangles_reject_clipping_bad_numbers_and_overflow",
    "byte_budgets_include_each_source_table_entry_but_not_clip_reuse",
    "dimension_and_decoded_budgets_preflight_all_sources_before_decoding",
    "even_unused_sources_reject_malformed_truncated_apng_and_invalid_license",
    "recipe_relative_paths_reject_traversal_absolute_backslash_and_colon",
    "symlink_fifo_directory_and_missing_inputs_never_publish_or_change_sources",
    "existing_outputs_and_symlink_or_missing_parents_are_preserved",
    "actual_cli_requires_standalone_help_and_exact_nonempty_options",
    "actual_cli_schema1_matches_all_four_frozen_pre_canvas_files",
    "actual_cli_schema2_matches_all_four_frozen_pre_named_clip_files",
)
SAMPLE_TESTS = ("cli_named_clips_same_document_locomotion_pixels_and_authoritative_parity",)
EDITOR_TESTS = ("generated_named_clips_drive_composed_editor_locomotion_after_reopen",)
FIXTURE_COUNT = 24
FEATURE_PATHS = (
    "crates/orr_sample/examples/png_sequence_package.rs",
    "crates/orr_sample/examples/png_sequence_package/authoring.rs",
    "crates/orr_sample/examples/png_sequence_package/offline_io.rs",
    "crates/orr_sample/examples/png_sequence_package/clips.rs",
    "crates/orr_sample/examples/png_sequence_package/tests.rs",
    "crates/orr_sample/tests/png_sequence_clips.rs",
    "crates/orr_sample/tests/png_sequence_clips_gpu.rs",
    "crates/orr_editor/tests/png_sequence_clips_ui.rs",
    "docs/png-sequence-package.md",
    "tools/png_sequence_clips_gate.py",
    "tools/test_png_sequence_clips_gate.py",
    ".github/workflows/png-sequence-clips.yml",
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key: " + key)
            result[key] = value
        return result

    def constant(value):
        raise ValueError("non-JSON constant: " + value)

    return json.loads(text, object_pairs_hook=pairs, parse_constant=constant)


def source_paths():
    result = []
    for directory, folders, files in os.walk(runner.repo, followlinks=False):
        parent = Path(directory)
        if parent == runner.repo:
            folders[:] = [name for name in folders if name not in {".git", "target"}]
            files = [name for name in files if name != ".git"]
        for name in folders:
            require(not (parent / name).is_symlink(), "symlink source directory")
        for name in files:
            path = parent / name
            require(path.is_file() and not path.is_symlink(), "nonregular source")
            result.append(path.relative_to(runner.repo).as_posix())
    return sorted(result)


def source_fence():
    if runner.frozen is not None:
        require(source_paths() == list(runner.frozen), "source path inventory changed")
        require(runner.source_snapshot(list(runner.frozen)) == runner.frozen,
                "source bytes or modes changed")


def run(label, command, expected=0, seconds=1200):
    source_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    return output


def cargo_flags(package):
    return ["--locked", "--release", "-p", package, "--no-default-features"]


def build(label, package, target, kind, features, default=False):
    flags = cargo_flags(package)
    if default:
        flags.remove("--no-default-features")
    requested = ["project-create"] if package == "orr_sample" and "project-create" in features else features
    if requested and not default:
        flags += ["--features", ",".join(requested)]
    command = ["cargo", "test" if kind == "test" else "build", *flags,
               "--" + kind, target, "--message-format=json"]
    if kind == "test":
        command.append("--no-run")
    output = run("build-" + label, command)
    # Check the complete Cargo stream ended in one successful compilation before
    # the shared artifact checker pins exact package/target/features/path/hash.
    rows = [load_json(line) for line in output.splitlines() if line.startswith("{")]
    require([row for row in rows if row.get("reason") == "build-finished"]
            == [{"reason": "build-finished", "success": True}], "Cargo build did not finish exactly once")
    require(rows[-1].get("reason") == "build-finished", "Cargo records after build completion")
    binary = runner.pin(output, label, package, target, kind, features)
    runner.report["producers"][label]["source_inventory_sha256"] = runner.report["source_inventory_sha256"]
    runner.record()
    return binary


def tests(label, binary, names, ignored=False, boundary=False):
    require(bool(names) or boundary, "empty acceptance inventory is not coverage")
    require(len(names) == len(set(names)), "duplicate expected test")
    runner.inventory(run(label + "-inventory", [binary, "--list"]), names)
    runner.inventory(run(label + "-ignored", [binary, "--ignored", "--list"]), names if ignored else [])
    flags = ["--ignored"] if ignored else []
    output = run(label + "-run", [binary, *flags, "--nocapture", "--test-threads=1", "--color=never"])
    runner.result(output, names)
    runner.report.setdefault("accepted_tests", {})[label] = {
        "names": list(names), "count": len(names), "ignored": ignored, "boundary_only": boundary,
    }
    runner.record()
    return output


def build_cli():
    cli = build("sequence-cli", "orr_sample", CLI, "example", SAMPLE_FEATURES)
    os.environ["ORR_PNG_SEQUENCE_PACKAGE_BIN"] = cli
    os.environ["ORR_REQUIRE_PNG_SEQUENCE_CLIPS"] = "1"
    require("png_sequence_package --recipe PATH --out NEW_DIR" in run("cli-help", [cli, "--help"]),
            "incorrect real CLI")
    return cli


def refusal(binary, label):
    missing = runner.evidence / (label + "-must-not-exist")
    require(not missing.exists(), "refusal destination already exists")
    for suffix, arguments in (("help", ["--help"]), ("io", ["--recipe", str(missing / "recipe.json"), "--out", str(missing)])):
        output = run(label + "-" + suffix, [binary, *arguments], expected=1, seconds=10)
        require(output.strip() == "png_sequence_package requires Linux and --features project-create",
                "disabled producer did not refuse before input/output")
        require(not missing.exists(), "disabled producer changed filesystem")


def cpu():
    for label, features, default in (("default-cli", ["default"], True), ("feature-off-cli", [], False)):
        refusal(build(label, "orr_sample", CLI, "example", features, default), label)
    disabled = build("feature-off-contracts", "orr_sample", CPU_TARGET, "test", [])
    tests("feature-off-contracts", disabled, [], boundary=True)
    build_cli()
    binary = build("clip-contracts", "orr_sample", CPU_TARGET, "test", SAMPLE_FEATURES)
    tests("clip-contracts", binary, CPU_TESTS)
    changed_rust = [name for name in FEATURE_PATHS if name.endswith(".rs")]
    run("rustfmt", ["rustfmt", "--edition", "2021", "--check", *changed_rust])
    for package in ("orr_sample", "orr_editor"):
        run(package + "-default", ["cargo", "check", "--locked", "--release", "-p", package, "--lib"])
        run(package + "-feature-off", ["cargo", "check", *cargo_flags(package), "--lib"])
    run("strict-default-cli", ["cargo", "clippy", "--locked", "--release", "-p", "orr_sample",
                               "--example", CLI, "--", "-D", "warnings"])
    run("strict-feature-off-cli", ["cargo", "clippy", *cargo_flags("orr_sample"),
                                   "--example", CLI, "--", "-D", "warnings"])
    run("strict-sample", ["cargo", "clippy", *cargo_flags("orr_sample"), "--features", "project-create",
                          "--example", CLI, "--test", CPU_TARGET, "--test", SAMPLE_TARGET,
                          "--", "-D", "warnings"])
    run("strict-editor", ["cargo", "clippy", *cargo_flags("orr_editor"), "--features", "sprites",
                          "--test", EDITOR_TARGET, "--", "-D", "warnings"])
    runner.report["coverage"]["cpu"] = "PASSED"
    runner.record()


def software_manifest(environment, system_directory=Path("/usr/share/vulkan/icd.d")):
    """Honor one explicit installed Lavapipe ICD, or the single system default."""
    keys = ("VK_DRIVER_FILES", "VK_ICD_FILENAMES")
    configured = [environment[key] for key in keys if key in environment]
    if configured:
        require(all(type(value) is str and value and ":" not in value and ";" not in value for value in configured),
                "exactly one explicit Vulkan manifest path required")
        require(len(set(configured)) == 1, "Vulkan manifest environment variables disagree")
        path = Path(configured[0])
    else:
        candidates = sorted(system_directory.glob("lvp_icd*.json"))
        require(len(candidates) == 1, "one configured or system Lavapipe manifest required")
        path = candidates[0]
    require(path.is_absolute() and path == path.resolve() and not path.is_symlink(), "absolute ordinary Vulkan manifest required")
    require(re.fullmatch(r"lvp_icd[^/]*\.json", path.name), "explicit manifest is not identified as Lavapipe")
    raw = ordinary_file(path, 65536)
    value = load_json(raw.decode("utf-8"))
    require(type(value) is dict and value.get("file_format_version") == "1.0.0", "unsupported Vulkan manifest format")
    icd = value.get("ICD")
    require(type(icd) is dict and type(icd.get("library_path")) is str
            and re.fullmatch(r"libvulkan_lvp\.so(?:\.[0-9]+)*", Path(icd["library_path"]).name), "manifest does not name the Lavapipe library")
    require(type(icd.get("api_version")) is str and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", icd["api_version"]),
            "missing Vulkan manifest API version")
    return path, raw


def software():
    manifest, raw = software_manifest(os.environ)
    runtime = runner.evidence / "xdg-runtime"
    runtime.mkdir(mode=0o700, exist_ok=False)
    os.environ.update(VK_DRIVER_FILES=str(manifest), VK_ICD_FILENAMES=str(manifest),
                      WGPU_BACKEND="vulkan", ORR_REQUIRE_GPU="1", XDG_RUNTIME_DIR=str(runtime))
    (runner.evidence / "lavapipe-icd.json").write_bytes(raw)
    runner.report["gpu_environment"] = {key: os.environ[key] for key in
        ("VK_DRIVER_FILES", "VK_ICD_FILENAMES", "WGPU_BACKEND", "ORR_REQUIRE_GPU", "XDG_RUNTIME_DIR")}
    runner.report["icd_sha256"] = hashlib.sha256(raw).hexdigest()
    runner.record()


def gpu():
    software()
    build_cli()
    captures = runner.evidence / "captures"
    require(not captures.exists(), "stale GPU evidence")
    os.environ["ORR_PNG_CLIPS_EVIDENCE"] = str(captures)
    sample = build("sample-gpu", "orr_sample", SAMPLE_TARGET, "test", SAMPLE_FEATURES)
    tests("sample-gpu", sample, SAMPLE_TESTS)
    validate_sample(captures / "sample")
    runner.report["coverage"]["sample_gpu"] = "PASSED"
    runner.record()
    editor = build("editor-gpu", "orr_editor", EDITOR_TARGET, "test", EDITOR_FEATURES)
    tests("editor-gpu", editor, EDITOR_TESTS)
    _, hashes = validate_editor(captures / "editor")
    runner.report["editor_capture_sha256"] = hashes
    runner.report["coverage"]["editor_gpu"] = "PASSED"
    runner.record()


CASES = ("good", "wrong-mapping", "wrong-order", "wrong-offset", "wrong-duration")
TIMES = (0, 69, 70, 99, 100, 169, 170, 199, 200, 249, 250, 299, 300, 400, 18446744073709551615)
HOST_TICKS = range(105)
EDITOR_CAPTURES = ("wrong-mapping", "authored-idle", "reopened-idle", "controlled-walk", "released-idle")


def sample_inventory():
    return {f"{case}-direct-{moving}-{time}.png" for case in CASES for moving in (0, 1) for time in TIMES} | {
        f"{case}-host-{tick}.png" for case in CASES for tick in HOST_TICKS}


def ordinary_file(path, limit):
    require(not path.is_symlink() and path.is_file(), "missing or symlink evidence: " + path.name)
    require(0 < path.stat().st_size <= limit, "unbounded or empty evidence: " + path.name)
    return path.read_bytes()


def png_rgba(path, size):
    """Bounded RGBA8 PNG decoding; hashes alone cannot validate image payloads."""
    data = ordinary_file(path, 16 * 1024 * 1024)
    require(data.startswith(b"\x89PNG\r\n\x1a\n"), "invalid PNG signature")
    offset, chunks = 8, []
    while offset < len(data):
        require(offset + 12 <= len(data), "truncated PNG chunk")
        length = struct.unpack(">I", data[offset:offset + 4])[0]
        end = offset + length + 12
        require(end <= len(data), "truncated PNG payload")
        kind, payload = data[offset + 4:offset + 8], data[offset + 8:end - 4]
        require(zlib.crc32(kind + payload) == struct.unpack(">I", data[end - 4:end])[0], "invalid PNG CRC")
        require(kind in (b"IHDR", b"IDAT", b"IEND"), "unexpected PNG chunk")
        chunks.append((kind, payload))
        offset = end
    require(chunks and chunks[0][0] == b"IHDR" and len(chunks[0][1]) == 13, "missing PNG header")
    require(chunks[-1] == (b"IEND", b"") and len(chunks) >= 3
            and all(kind == b"IDAT" for kind, _ in chunks[1:-1]), "incomplete PNG image")
    width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", chunks[0][1])
    require((width, height) == tuple(size) and 0 < width <= 2048 and 0 < height <= 2048,
            "unexpected capture dimensions")
    require((depth, color, compression, filtering, interlace) == (8, 6, 0, 0, 0), "unexpected capture format")
    stride = width * 4
    raw_size = (stride + 1) * height
    decoder = zlib.decompressobj()
    raw = decoder.decompress(b"".join(payload for _, payload in chunks[1:-1]), raw_size + 1)
    require(decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail and len(raw) == raw_size,
            "invalid PNG compressed image")
    pixels = bytearray()
    previous = bytes(stride)
    for row in range(height):
        start = row * (stride + 1)
        method = raw[start]
        require(method in range(5), "invalid PNG filter")
        current = bytearray(raw[start + 1:start + 1 + stride])
        if method:
            for column in range(stride):
                left = current[column - 4] if column >= 4 else 0
                up = previous[column]
                corner = previous[column - 4] if column >= 4 else 0
                if method == 1:
                    predictor = left
                elif method == 2:
                    predictor = up
                elif method == 3:
                    predictor = (left + up) // 2
                else:
                    estimate = left + up - corner
                    distances = (abs(estimate - left), abs(estimate - up), abs(estimate - corner))
                    predictor = (left, up, corner)[distances.index(min(distances))]
                current[column] = (current[column] + predictor) & 255
        pixels.extend(current)
        previous = current
    return bytes(pixels)


def receipt(directory, test):
    require(directory.is_dir() and not directory.is_symlink(), "missing or symlink GPU evidence directory")
    value = load_json(ordinary_file(directory / "receipt.json", 1024 * 1024).decode("utf-8"))
    require(type(value) is dict and type(value.get("schema")) is int and value["schema"] == 1,
            "invalid receipt schema")
    require(value.get("test") == test, "receipt from the wrong test")
    require(type(value.get("adapter")) is str and "llvmpipe" in value["adapter"].lower()
            and value.get("backend") == "Vulkan", "explicit software Vulkan evidence required")
    return value


def sample_time(name):
    match = re.fullmatch(r"(.+)-(direct|host)-(.*)\.png", name)
    require(match is not None, "invalid sample filename")
    case, phase, suffix = match.groups()
    if phase == "direct":
        moving, elapsed = map(int, suffix.split("-"))
        return case, phase, elapsed, bool(moving)
    tick = int(suffix)
    moving = 21 <= tick < 75
    epoch = 21 if moving else (75 if tick >= 75 else 0)
    return case, phase, (tick - epoch) * 1000 // 60, moving


def sample_frame(elapsed, moving, case):
    if not moving:
        return 0 if elapsed % 250 < 100 else 2
    first_end = 170 if case == "wrong-duration" else 70
    index = 0 if elapsed < first_end else (1 if elapsed < 200 else 2)
    return ((2, 1, 1) if case == "wrong-order" else (1, 2, 1))[index]


def pixel_matches(actual, expected):
    return len(actual) == len(expected) and all(abs(left - right) <= 3 for left, right in zip(actual, expected))


def sample_oracle(frame, dx):
    """Literal sources + fixed camera geometry, independent of generated JSON."""
    z, r, g, b = (0, 0, 0, 0), (233, 31, 19, 255), (19, 211, 47, 255), (27, 61, 229, 255)
    p, h, foot = (128, 71, 199, 128), (231, 117, 53, 0), (255, 0, 255, 255)
    sources = ((r, p, h, b, g, r, z, foot, b),
               (g, z, b, h, p, b, r, g, z, r, g, p, b, z, foot, r), (b, p, foot, h))
    width, height = ((3, 3), (4, 4), (2, 2))[frame]
    ox, oy = ((1, 1), (0, 0), (2, 2))[frame]
    ox += dx

    def linear(value):
        value /= 255
        return value / 12.92 if value <= .04045 else ((value + .055) / 1.055) ** 2.4

    def srgb(value):
        value = 12.92 * value if value <= .0031308 else 1.055 * value ** (1 / 2.4) - .055
        return int(value * 255 + .5)

    colors = {}
    for pixel in set(sources[frame]) | {z}:
        alpha = pixel[3] / 255
        colors[pixel] = bytes(srgb(linear(value) * alpha + bg * (1 - alpha))
                              for value, bg in zip(pixel[:3], (.02, .02, .035))) + bytes([255])
    result = bytearray()
    for y in range(128):
        for x in range(128):
            # Exact cell centers in the 6x5 canvas, eight screen pixels per unit.
            cx, cy = (x + .5 - 64) / 8 + 3, (y + .5 - 64) / 8 + 2.5
            pixel = z
            if ox <= cx < ox + width and oy <= cy < oy + height:
                pixel = sources[frame][(int(cy) - oy) * width + int(cx) - ox]
            result.extend(colors[pixel])
    return bytes(result)


def validate_sample(directory):
    value = receipt(directory, SAMPLE_TESTS[0])
    for key in ("source_hidden", "same_document_locomotion", "every_host_tick_state_and_checksum_equal", "installed_bytes_preserved", "is_software"):
        require(value.get(key) is True, "missing sample runtime proof: " + key)
    require(value.get("device_type") == "Cpu", "sample software device required")
    expected = sample_inventory()
    oracles = {(frame, dx): sample_oracle(frame, dx) for frame in range(3) for dx in range(2)}
    rejected = {case: {"direct": 0, "host": 0} for case in CASES}
    require(type(value.get("capture_count")) is int and value["capture_count"] == len(expected) == 675,
            "incorrect sample capture count")
    captures = value.get("captures")
    require(type(captures) is list and len(captures) == len(expected), "incomplete sample capture receipt")
    seen = set()
    for item in captures:
        require(type(item) is dict, "invalid capture record")
        name = item.get("file")
        require(type(name) is str and name in expected and name not in seen, "unexpected or duplicate capture")
        seen.add(name)
        path = directory / name
        data = ordinary_file(path, 16 * 1024 * 1024)
        require(item.get("sha256") == hashlib.sha256(data).hexdigest(), "sample capture hash mismatch")
        pixels = png_rgba(path, (128, 128))
        case, phase, elapsed, moving = sample_time(name)
        actual_frame = sample_frame(elapsed, moving != (case == "wrong-mapping"), case)
        require(pixel_matches(pixels, oracles[actual_frame, int(case == "wrong-offset")]),
                "sample capture differs from source-known pixel oracle: " + name)
        correct_frame = sample_frame(elapsed, moving, "good")
        rejected[case][phase] += int(not pixel_matches(pixels, oracles[correct_frame, 0]))
    require(seen == expected and {path.name for path in directory.glob("*.png")} == expected,
            "sample capture inventory differs")
    controls = value.get("negative_controls")
    require(type(controls) is list and len(controls) == len(CASES), "missing negative controls")
    seen = set()
    for control in controls:
        require(type(control) is dict and control.get("case") in CASES, "unexpected negative control")
        case = control["case"]
        require(case not in seen, "duplicate negative control")
        seen.add(case)
        for key, cap in (("direct_rejected", 30), ("host_rejected", 105)):
            count = control.get(key)
            require(type(count) is int and 0 <= count <= cap, "invalid negative-control mismatch count")
            require(count == rejected[case][key.removesuffix("_rejected")], "claimed negative-control count differs from retained pixels")
            require(count == 0 if case == "good" else count > 0, "negative control did not disagree with oracle")
    require(seen == set(CASES), "incomplete negative-control inventory")
    return value


def editor_source(frame):
    z, r, g, b = (0, 0, 0, 0), (233, 31, 19, 255), (19, 211, 47, 255), (27, 61, 229, 255)
    foot = (255, 0, 255, 255)
    sources = ((r, g, z, b, g, r, z, foot, b),
               (g, z, b, z, r, b, r, g, z, r, g, b, b, z, foot, r), (b, g, foot, z))
    width, height = ((3, 3), (4, 4), (2, 2))[frame]
    ox, oy = ((1, 1), (0, 0), (2, 2))[frame]
    return [sources[frame][(y - oy) * width + x - ox]
            if ox <= x < ox + width and oy <= y < oy + height else z
            for y in range(5) for x in range(6)]


def editor_pixels_match(pixels, probes, frame):
    return all(all(abs(pixels[(y * 1500 + x) * 4 + channel] - color[channel]) <= 3 for channel in range(3))
               for (x, y), color in zip(probes, editor_source(frame)) if color[3] == 255)


def validate_editor_probes(pixels, capture):
    require(len(pixels) == 1500 * 1400 * 4, "incomplete editor framebuffer")
    probes = capture.get("probe_pixels")
    require(type(probes) is list and len(probes) == 30, "all source-canvas probe centers required")
    left, top, right, bottom = capture["viewport"]
    # The test fixes half_extent=20 and units_per_pixel=2. The camera centers on
    # the hero, so the 6x5 source canvas has known centered sample coordinates.
    step = min(right - left, bottom - top) / 20
    for index, point in enumerate(probes):
        require(type(point) is list and len(point) == 2 and all(type(v) is int for v in point), "invalid editor probe coordinate")
        x, y = point
        require(left <= x < right and top <= y < bottom, "editor probe outside viewport")
        expected_x = (left + right) / 2 + (index % 6 - 2.5) * step
        expected_y = (top + bottom) / 2 + (index // 6 - 2) * step
        require(abs(x - expected_x) <= 2 and abs(y - expected_y) <= 2, "editor probe differs from fixed camera geometry")
    require(len({tuple(point) for point in probes}) == 30, "duplicate editor source probes")
    require(editor_pixels_match(pixels, probes, capture["source_frame"]), "editor capture differs from source-known opaque pixels")
    if capture["file"] == "wrong-mapping.png":
        require(not editor_pixels_match(pixels, probes, 0), "wrong editor mapping did not fail the correct idle oracle")


def validate_editor(directory):
    value = receipt(directory, EDITOR_TESTS[0])
    for key in ("main_editor_app", "source_hidden", "installed_bytes_preserved", "real_clip_widgets",
                "saved_and_reopened", "real_keyboard_move_and_release", "presentation_free_state_and_checksum_equal"):
        require(value.get(key) is True, "missing main EditorApp proof: " + key)
    require(value.get("device_type") == "Cpu", "main EditorApp software device required")
    require(type(value.get("compared_live_frames")) is int and value["compared_live_frames"] >= 2,
            "missing presentation-free live-frame comparisons")
    require(type(value.get("recorded_ticks")) is int and value["recorded_ticks"] > 0, "missing recorded-input replay")
    expected = {name + ".png" for name in EDITOR_CAPTURES}
    require(type(value.get("capture_count")) is int and value["capture_count"] == len(expected) == 5,
            "incorrect editor capture count")
    require({path.name for path in directory.glob("*.png")} == expected, "editor capture inventory differs")
    captures = value.get("captures")
    require(type(captures) is list and len(captures) == len(expected), "missing editor capture metadata")
    hashes, seen, frames = {}, set(), {}
    for capture in captures:
        require(type(capture) is dict, "invalid editor capture metadata")
        name = capture.get("file")
        require(type(name) is str and name in expected and name not in seen, "unexpected or duplicate editor capture")
        seen.add(name)
        require(type(capture.get("width")) is int and type(capture.get("height")) is int
                and (capture["width"], capture["height"]) == (1500, 1400), "wrong full editor dimensions")
        elapsed, moving, frame = capture.get("elapsed_ms"), capture.get("moving"), capture.get("source_frame")
        require(type(elapsed) is int and 0 <= elapsed <= 18446744073709551615
                and type(moving) is bool and type(frame) is int and frame in range(3), "invalid editor playback metadata")
        require(moving == (name == "controlled-walk.png"), "wrong editor movement phase")
        expected_frame = 1 if name == "wrong-mapping.png" else sample_frame(elapsed, moving, "good")
        require(frame == expected_frame, "editor source frame differs from playback clock")
        if name in {"authored-idle.png", "reopened-idle.png", "wrong-mapping.png"}:
            require(elapsed == 0, "edit-mode preview must start at zero")
        if moving:
            require(elapsed >= 350, "Once terminal frame was not reached during movement")
        if name == "released-idle.png":
            require(elapsed >= 50, "key release needs coherent stationary ticks, not an epoch reset")
        tick = capture.get("tick")
        require(tick is None or type(tick) is int and 0 <= tick <= value["recorded_ticks"], "invalid editor authoritative tick")
        if name in {"controlled-walk.png", "released-idle.png"}:
            require(type(tick) is int and tick > 0, "missing live editor tick")
        require(type(capture.get("checksum")) is str and re.fullmatch(r"0x[0-9a-f]{16}", capture["checksum"]),
                "missing editor checksum")
        viewport = capture.get("viewport")
        require(type(viewport) is list and len(viewport) == 4
                and all(type(v) in (int, float) for v in viewport)
                and 0 <= viewport[0] < viewport[2] <= 1500 and 0 <= viewport[1] < viewport[3] <= 1400,
                "invalid editor viewport bounds")
        data = ordinary_file(directory / name, 16 * 1024 * 1024)
        pixels = png_rgba(directory / name, (1500, 1400))
        validate_editor_probes(pixels, capture)
        hashes[name] = hashlib.sha256(data).hexdigest()
        frames[name] = pixels
    require(seen == expected, "incomplete editor capture inventory")
    require(frames["wrong-mapping.png"] != frames["authored-idle.png"]
            and frames["controlled-walk.png"] != frames["released-idle.png"], "missing distinct editor phases")
    return value, hashes


def begin():
    require(os.environ.get("PYTHONDONTWRITEBYTECODE") == "1", "inherited bytecode prevention required")
    runner.frozen = runner.begin()
    require(source_paths() == list(runner.frozen), "untracked or ignored source files cannot enter producers")
    require(set(FEATURE_PATHS) <= set(runner.frozen), "missing named-clips feature sources")
    runner.report["source_inventory_sha256"] = hashlib.sha256(
        json.dumps(runner.frozen, sort_keys=True).encode()).hexdigest()
    runner.record()
    output = run("named-clips-checker-fixtures", [sys.executable, "-B", str(runner.repo / "tools/test_png_sequence_clips_gate.py")])
    require(output.rstrip().endswith(f"PNG_SEQUENCE_CLIPS_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped"),
            "independent gate fixture census")


def main():
    require(len(sys.argv) == 2 and sys.argv[1] in {"cpu", "gpu"}, "usage: png_sequence_clips_gate.py cpu|gpu")
    require(sys.platform == "linux", "named-clips acceptance requires Linux")
    mode = sys.argv[1]
    root = Path(__file__).resolve().parents[1]
    destination = Path(os.environ["ORR_PNG_SEQUENCE_CLIPS_EVIDENCE"]).resolve()
    require(not destination.is_relative_to(root), "evidence must be outside the source checkout")
    runner.initialize(root, destination, mode)
    runner.report["mode"] = mode
    runner.report["coverage"] = {"cpu": "NOT RUN", "sample_gpu": "NOT RUN", "editor_gpu": "NOT RUN"}
    error = None
    try:
        begin()
        {"cpu": cpu, "gpu": gpu}[mode]()
        expected = {"cpu"} if mode == "cpu" else {"sample_gpu", "editor_gpu"}
        require(runner.report["coverage"] == {key: "PASSED" if key in expected else "NOT RUN"
                                             for key in runner.report["coverage"]}, "incomplete lane coverage")
        source_fence()
    except BaseException as failure:
        error = failure
    finally:
        runner.finish(error)


if __name__ == "__main__":
    main()
