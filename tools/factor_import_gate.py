#!/usr/bin/env python3
"""Mandatory factor-only import gates; reused portable process/PNG primitives."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import sys
import zlib
sys.dont_write_bytecode = True
import material_sample_gate as common

SIZE = 192
RAW_BYTES = SIZE * SIZE * 4
CAPTURE_CAP = 4 * 1024**2
CACHE_CAP = 16 * 1024**2
FACTORS = ((.25, .5, .75, 1.), (1., .25, .5, 1.))
FRAMES = {
    "static": ["static-factor", "static-white"],
    "animated": [f"animated-{mode}-{pose}" for mode in ("factor", "white") for pose in ("start", "midpoint", "key")],
}
HELPER_TESTS = [
    "test_platform_command_and_work_census", "test_exact_results_names_and_ignored",
    "test_transfer_missing_and_double_encoding", "test_raw_shape_and_metadata_admission",
    "test_display_and_linear_channel_bounds", "test_nonvacuous_foreground_and_background",
    "test_static_pair_equality", "test_animation_pairs_and_pose_changes",
    "test_full_adapter_identity", "test_cross_platform_file_census",
    "test_lossless_png_and_changed_pixels", "test_fences_and_partial_failure_measurements",
]
LANES = [{'label': 'model-import', 'package': 'orr_model', 'features': ['import'], 'target': 'untextured', 'cpu': ['fallback_charges_decoded_budget_before_allocation', 'static_factor_gltf_glb_cook_and_optional_uv', 'static_factor_rejections_preserve_diffuse_subset', 'static_mixed_material_image_bounds_and_authored_index_fence', 'static_textured_cooked_bytes_match_immutable_prechange_goldens'], 'ignored': []}, {'label': 'model-animation', 'package': 'orr_model', 'features': ['import', 'animation'], 'target': 'untextured', 'cpu': ['animated_factor_gltf_glb_cook_optional_uv_and_poses', 'animated_factor_rejections_and_mixed_image_bounds', 'animated_mixed_static_and_skinned_primitives_share_optional_uv_contract', 'animated_textured_cooked_bytes_match_immutable_prechange_goldens', 'fallback_charges_decoded_budget_before_allocation', 'static_factor_gltf_glb_cook_and_optional_uv', 'static_factor_rejections_preserve_diffuse_subset', 'static_mixed_material_image_bounds_and_authored_index_fence', 'static_textured_cooked_bytes_match_immutable_prechange_goldens'], 'ignored': []}, {'label': 'old-static', 'package': 'orr_model', 'features': ['import', 'animation'], 'target': 'import', 'cpu': ['binary_glb_header_chunks_indices_and_nonfinite_geometry_reject', 'cooked_model_is_revalidated_and_inverse_transpose_is_correct', 'data_uri_resources_and_changed_content_preserve_identity', 'external_paths_and_symlink_escape_reject_before_read', 'fifo_is_rejected_before_open_can_block', 'genuine_external_and_embedded_import_cook_load_match', 'malformed_accessor_bounds_stride_references_and_shapes_reject', 'node_cycles_duplicate_roots_parents_and_bad_transforms_reject', 'png_animation_and_decoded_extent_reject', 'unsupported_features_and_material_defaults_are_explicit_errors', 'unused_reverse_ordered_deep_tree_is_rejected'], 'ignored': []}, {'label': 'old-animated', 'package': 'orr_model', 'features': ['import', 'animation'], 'target': 'animation_import', 'cpu': ['absent_skin_animation_collections_allowed_but_present_empty_reject', 'binary_nonfinite_times_rotations_weights_joints_inverse_binds_reject', 'collection_and_aggregate_key_budgets_reject', 'cooked_reload_rejects_invalid_data_and_clip_switch_resets_missing_tracks', 'explicit_null_new_properties_reject_before_resource_resolution', 'genuine_gltf_glb_cook_reload_and_original_hierarchy_match', 'malformed_animation_shapes_paths_duplicates_and_unsupported_modes_reject', 'malformed_glb_and_external_paths_fail_closed', 'malformed_skin_hierarchy_references_and_scene_membership_reject', 'missing_inverse_bind_defaults_to_identity', 'normalized_integer_weights_require_exact_raw_unit_sum', 'orphan_f32_accessors_are_finite_and_scan_work_is_bounded', 'static_import_entrypoint_still_rejects_animated_sources_before_resolver', 'u8_u16_joints_and_float_normalized_integer_weights_are_supported', 'unsupported_extra_influences_morphs_extensions_sparse_and_bad_accessor_layout_reject', 'unused_mesh_values_and_unselected_node_skin_references_are_checked'], 'ignored': []}, {'label': 'bindings-static', 'package': 'orr_model_bindings', 'features': [], 'target': 'untextured', 'cpu': ['installed_static_factor_binding_survives_reload_removal_and_reinstall'], 'ignored': []}, {'label': 'bindings-animation', 'package': 'orr_model_bindings', 'features': ['animated-models'], 'target': 'untextured', 'cpu': ['installed_animated_factor_binding_survives_reload_removal_and_reinstall', 'installed_static_factor_binding_survives_reload_removal_and_reinstall'], 'ignored': []}, {'label': 'render-static', 'package': 'orr_render', 'features': ['imported-scene'], 'target': 'untextured', 'cpu': ['package_fixture_bytes_and_white_reference_preserve_geometry'], 'ignored': ['installed_factor_static_matches_white_textured_linear_readback']}, {'label': 'render-animation', 'package': 'orr_render', 'features': ['imported-scene', 'animation'], 'target': 'untextured', 'cpu': ['package_fixture_bytes_and_white_reference_preserve_geometry'], 'ignored': ['installed_factor_animation_matches_white_at_three_fixed_poses', 'installed_factor_static_matches_white_textured_linear_readback']}]


def require(condition, message):
    common.require(condition, message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def lanes_for(platform):
    lanes = json.loads(json.dumps(LANES))
    if platform == "win32":
        lanes[2]["cpu"].remove("fifo_is_rejected_before_open_can_block")
    return lanes


def command_inventory(platform):
    """Exact CPU compile/list/run triples, six strict lanes and two checks."""
    commands = []
    for lane in lanes_for(platform):
        commands.extend((lane["label"]+"-compile", lane["label"]+"-list", lane["label"]+"-cpu"))
    commands.extend(lane["label"]+"-strict" for lane in lanes_for(platform) if not lane["label"].startswith("old-"))
    return ["helper-tests"] + commands + ["model-default-check", "render-models-check"]



def helper_results(text, class_name, expected):
    """Python 3.10 and newer verbose forms, with exact method identity."""
    rows = re.findall(r'^(test_\w+) \(__main__\.'+re.escape(class_name)+r'(?:\.(test_\w+))?\) \.\.\. ok$', text, re.M)
    require(sorted(name for name, _ in rows) == sorted(expected), 'exact helper names')
    require(all(not suffix or suffix == name for name, suffix in rows), 'helper method suffix mismatch')
    require(re.findall(r'^Ran (\d+) tests? in [0-9.]+s$', text, re.M) == [str(len(expected))] and text.rstrip().endswith('OK'), 'helper result summary')


def encode(value):
    return int(255 * (12.92 * value if value <= .0031308 else 1.055 * value**(1/2.4) - .055) + .5)


def decode(code):
    value = code / 255
    return value / 12.92 if value <= .04045 else ((value+.055)/1.055)**2.4


def color_matches(pixel, factor):
    return len(pixel) == 4 and all(abs(actual-encode(expected)) <= 2 and abs(decode(actual)-expected) <= 2/255 for actual, expected in zip(pixel, factor))


def raw_controls(raw, kind):
    require(kind in FRAMES and len(raw) == RAW_BYTES, "raw dimensions/length")
    counts, clear, maximum = [0, 0], 0, 0.
    for offset in range(0, len(raw), 4):
        pixel = raw[offset:offset+4]
        if pixel == bytes((0, 0, 0, 255)):
            clear += 1
            continue
        hits = [index for index, factor in enumerate(FACTORS) if color_matches(pixel, factor)]
        require(len(hits) == 1, "display or linear factor mismatch")
        counts[hits[0]] += 1
        maximum = max(maximum, max(abs(decode(actual)-expected) for actual, expected in zip(pixel, FACTORS[hits[0]])))
    require(counts[0] >= 100 and clear >= 100, "vacuous foreground/background")
    require(kind != "static" or counts[1] >= 20, "vacuous second material")
    return {"width": SIZE, "height": SIZE, "counts": counts, "clear": clear, "max_decoded_linear_error": maximum}


def check_pairs(raws, kind):
    require(set(raws) == set(FRAMES[kind]), "frame names")
    if kind == "static":
        require(raws["static-factor"] == raws["static-white"], "static pair differs")
    else:
        for pose in ("start", "midpoint", "key"):
            require(raws["animated-factor-"+pose] == raws["animated-white-"+pose], "animated pair differs")
        require(raws["animated-factor-start"] != raws["animated-factor-midpoint"] != raws["animated-factor-key"], "pose did not change")


def observed_adapter(text, previous=None):
    found = re.findall(r'^factor adapter: (.+)$', text, re.M)
    require(len(found) == 1, "one full adapter tuple")
    adapter = found[0]
    require(re.fullmatch(r'name="[^\r\n]+" backend=Vulkan device_type=Cpu vendor=\d+ device=\d+ driver="[^\r\n]*" driver_info="[^\r\n]*" software=true', adapter) is not None, "software Vulkan adapter required")
    require(previous is None or adapter == previous, "adapter changed")
    return adapter


def encode_png(raw):
    require(len(raw) == RAW_BYTES, "raw size for PNG")
    def chunk(kind, data):
        return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data)&0xffffffff)
    rows = b''.join(b'\0'+raw[y*SIZE*4:(y+1)*SIZE*4] for y in range(SIZE))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', SIZE, SIZE, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(rows, 9))+chunk(b'IEND', b'')


def require_files(directory, expected):
    actual = {common.relative_capture_key(p, directory) for p in directory.rglob('*') if p.is_file() or p.is_symlink()}
    require(actual == set(expected), "exact capture file census")
    for key in expected:
        path = directory/key
        require(path.is_file() and not path.is_symlink(), "regular capture file")


def audit_capture(directory, kind, text):
    names = FRAMES[kind]
    raw_names = {name+'.rgba' for name in names}
    actual = {common.relative_capture_key(p, directory) for p in directory.rglob('*') if p.is_file() or p.is_symlink()}
    require(raw_names <= actual <= raw_names | {name+'.png' for name in names}, "capture census before decode")
    require_files(directory, actual)
    rows = re.findall(r'^factor color controls: counts=\[(\d+), (\d+)\] clear=(\d+)$', text, re.M)
    require(len(rows) == len(names), "per-frame color metadata count")
    raws, files = {}, {}
    for index, name in enumerate(names):
        raw_path = directory/(name+'.rgba')
        require(raw_path.stat().st_size == RAW_BYTES, "raw extent")
        raw = raw_path.read_bytes()
        controls = raw_controls(raw, kind)
        require([*controls['counts'], controls['clear']] == list(map(int, rows[index])), "reported pixel controls mismatch")
        raws[name] = raw
        files[raw_path.name] = {"bytes": len(raw), "sha256": digest(raw), **controls}
        png_path = directory/(name+'.png')
        if not png_path.exists():
            with png_path.open('xb') as output:
                output.write(encode_png(raw))
        require(png_path.is_file() and not png_path.is_symlink() and png_path.stat().st_size <= 2*1024**2, "PNG admission")
        data = png_path.read_bytes()
        require(common.png_rgba(data, (SIZE, SIZE)) == raw, "PNG decoded pixels differ")
        files[png_path.name] = {"bytes": len(data), "sha256": digest(data), "width": SIZE, "height": SIZE, "decoded_rgba_sha256": digest(raw)}
    check_pairs(raws, kind)
    require_files(directory, {name+suffix for name in names for suffix in ('.rgba', '.png')})
    return files


def directory_bytes(path):
    total = 0
    for entry in path.rglob('*'):
        require(not entry.is_symlink(), "owned output must not be a symlink")
        if entry.is_file():
            total += entry.stat().st_size
    return total


def record_budgets(captures, cache, path):
    """Completed-command measurements, including failure paths; not live monitoring."""
    record = {"supervision": "pre/post-command; time/log/disk are continuous in reused bounded_run", "measurement_errors": []}
    for key, directory in (("capture_bytes", captures), ("cache_bytes", cache)):
        try:
            record[key] = directory_bytes(directory)
        except (OSError, AssertionError) as error:
            record[key] = None
            record["measurement_errors"].append(str(error))
    common.save(path, record)
    require(not record['measurement_errors'], "capture/cache measurement failed")
    require(record['capture_bytes'] <= CAPTURE_CAP and record['cache_bytes'] <= CACHE_CAP, "capture/cache byte cap")
    return record


def verify_hashes(records):
    for path, expected in records.items():
        require(Path(path).is_file() and digest(Path(path).read_bytes()) == expected, "source/producer hash changed")


class FactorGate(common.Gate):
    def __init__(self, root, evidence):
        super().__init__(root, evidence)
        self.captures, self.cache = self.evidence/'captures', self.evidence/'cache'
        for path in (self.captures, self.cache, self.cache/'mesa', self.evidence/'runtime', self.evidence/'budgets'):
            path.mkdir()
        for key in list(self.env):
            if key.startswith('MESA_') or key.startswith('WGPU_'):
                del self.env[key]
        self.env.update(PYTHONDONTWRITEBYTECODE='1', TMPDIR=str(self.evidence/'runtime'), TEMP=str(self.evidence/'runtime'), TMP=str(self.evidence/'runtime'), XDG_RUNTIME_DIR=str(self.evidence/'runtime'), XDG_CACHE_HOME=str(self.cache), MESA_SHADER_CACHE_DIR=str(self.cache/'mesa'), MESA_SHADER_CACHE_MAX_SIZE='8M')

    def run(self, label, argv, expected_exit=0, env=None, timeout=1200):
        number = self.number + 1
        try:
            require(directory_bytes(self.captures) <= CAPTURE_CAP and directory_bytes(self.cache) <= CACHE_CAP, "pre-command byte cap")
            return super().run(label, argv, expected_exit, env, timeout)
        finally:
            record_budgets(self.captures, self.cache, self.evidence/'budgets'/f'{number:03d}-{label}.json')

    def compile_lane(self, lane):
        args = ['cargo', 'test', '--release', '--locked', '-p', lane['package'], '--no-default-features']
        if lane['features']:
            args += ['--features', ','.join(lane['features'])]
        args += ['--test', lane['target'], '--no-run', '--message-format=json']
        text = self.run(lane['label']+'-compile', args)
        candidates = []
        for line in text.splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable') and item.get('profile', {}).get('test') and item.get('target', {}).get('name') == lane['target']:
                candidates.append(item)
        require(len(candidates) == 1, 'exact integration producer')
        original = Path(candidates[0]['executable'])
        saved = self.evidence/(lane['label']+('.exe' if os.name == 'nt' else ''))
        require(original.is_file() and not saved.exists(), 'fresh saved producer')
        shutil.copy2(original, saved)
        self.producers[lane['label']] = {'path': str(saved), 'sha256': digest(saved.read_bytes()), 'package': lane['package'], 'features': lane['features'], 'target': lane['target']}
        common.inventory(self.run(lane['label']+'-list', [str(saved), '--list']), sorted(lane['cpu']+lane['ignored']))
        return saved

    def cpu(self):
        text = self.run('helper-tests', [sys.executable, '-B', str(self.root/'tools/test_factor_import_gate.py'), '-v'])
        helper_results(text, 'FactorGateTests', HELPER_TESTS)
        lanes = lanes_for(sys.platform)
        for lane in lanes:
            saved = self.compile_lane(lane)
            text = self.run(lane['label']+'-cpu', [str(saved), '--test-threads=1', '--show-output'])
            common.result(text, lane['cpu'], lane['ignored'], 0)
            require('factor adapter:' not in text, 'device launched in CPU phase')
        for lane in lanes:
            if lane['label'].startswith('old-'):
                continue
            args = ['cargo', 'clippy', '--release', '--locked', '-p', lane['package'], '--no-default-features']
            if lane['features']:
                args += ['--features', ','.join(lane['features'])]
            args += ['--tests'] if lane['package'] == 'orr_model' else ['--lib', '--test', 'untextured']
            self.run(lane['label']+'-strict', args+['--', '-D', 'warnings'])
        for label, package, features in [('model-default', 'orr_model', []), ('render-models', 'orr_render', ['models'])]:
            args = ['cargo', 'check', '--release', '--locked', '-p', package, '--no-default-features']
            if features:
                args += ['--features', ','.join(features)]
            self.run(label+'-check', args+['--lib', '--test', 'untextured'])
        require([command['label'] for command in self.commands] == command_inventory(sys.platform), '33 command census')
        common.save(self.evidence/'cpu-summary.json', {'executions': sum(len(lane['cpu']) for lane in lanes), 'unique_names': len({name for lane in lanes for name in lane['cpu']}), 'helper_tests': 12, 'commands': 33, 'device_executed': False})

    def gpu(self):
        require(sys.platform.startswith('linux'), 'device gate is Linux-only')
        self.env['ORR_REQUIRE_GPU'] = '1'
        icd = Path(self.env.get('VK_DRIVER_FILES', ''))
        require(icd.is_file() and str(icd) == self.env.get('VK_ICD_FILENAMES') and icd.stat().st_size <= 65536, 'one explicit pinned ICD')
        common.save(self.evidence/'driver.json', {'path': str(icd), 'sha256': digest(icd.read_bytes())})
        adapter = None
        for kind, lane in zip(('static', 'animated'), lanes_for(sys.platform)[-2:]):
            saved = self.compile_lane(lane)
            name = lane['ignored'][0] if kind == 'static' else 'installed_factor_animation_matches_white_at_three_fixed_poses'
            directory = self.captures/kind
            directory.mkdir()
            env = dict(self.env, ORR_FACTOR_CAPTURE_DIR=str(directory))
            text = self.run(kind+'-device', [str(saved), '--exact', name, '--ignored', '--test-threads=1', '--show-output'], env=env, timeout=180)
            common.result(text, [name], [], len(lane['cpu'])+len(lane['ignored'])-1)
            adapter = observed_adapter(text, adapter)
            files = audit_capture(directory, kind, text)
            common.save(self.evidence/('capture-audit-'+kind+'.json'), {'adapter': adapter, 'files': files, 'visual_review_required': True})
            require(directory_bytes(self.captures) <= CAPTURE_CAP, 'post-audit capture cap')
        require(len(self.commands) == 6, 'two compile/list/device triples')
        require_files(self.captures, {kind+'/'+name+suffix for kind, names in FRAMES.items() for name in names for suffix in ('.rgba', '.png')})
        common.save(self.evidence/'device-summary.json', {'adapter': adapter, 'invocations': 2, 'raw_frames': 8, 'pngs': 8, 'files': 16, 'renderer_calls': 8, 'indexed_draws': 10, 'triangles': 44, 'portable_cross_adapter_pixel_golden': False})


def main():
    require(sys.flags.optimize == 0, 'Python optimization is unsupported')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('cpu', 'gpu'))
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    gate = FactorGate(Path(__file__).resolve().parents[1], args.evidence)
    state = 'STOPPED'
    try:
        getattr(gate, args.phase)()
        require(gate.source_hashes() == gate.source, 'final source fence')
        state = 'PASSED'
    finally:
        receipts = [gate.evidence/Path(command['log']).with_suffix('.json') for command in gate.commands]
        reaped = bool(receipts) and all(path.is_file() and json.loads(path.read_text()).get('owned_children_reaped') for path in receipts)
        sizes = {}
        measurement_errors = []
        for label, directory in (('capture_bytes', gate.captures), ('cache_bytes', gate.cache)):
            try:
                sizes[label] = directory_bytes(directory)
            except (OSError, AssertionError) as error:
                sizes[label] = None
                measurement_errors.append(str(error))
        if measurement_errors:
            state = 'STOPPED'
        common.save(gate.evidence/'terminal.json', {'state': state, 'phase': args.phase, 'commands': gate.commands, 'producers': gate.producers, 'source_sha256': digest((gate.evidence/'source.json').read_bytes()), 'owned_children_reaped': reaped, **sizes, 'measurement_errors': measurement_errors, 'directory_cap_supervision': 'pre/post-command; continuous time/log/disk via parent helper'})


if __name__ == '__main__':
    main()
