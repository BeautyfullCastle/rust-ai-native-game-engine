#!/usr/bin/env python3
"""One Linux batch appended to the existing sampler UI job; no new job/matrix.

Lifecycle and bounded process ownership reuse the existing sampler/extract gate.
Rust owns cooked float-bit and Frame decoding assertions. Python independently
binds producers, source, exact executed census, PNG/raw manifests and receipts.
"""
import hashlib
import json
import os
import pathlib
import re
import sys

if not __debug__ or sys.flags.optimize or os.environ.get('PYTHONOPTIMIZE') not in (None, '', '0'):
    raise SystemExit('planar UV gate requires unoptimized Python')
import static_model_sampler_gate as lifecycle
from static_model_sampler_gate import runner

CPU = [
    'all_planes_use_model_positions_and_preserve_unrelated_bits_and_direct_provenance',
    'strict_schema_paths_and_numeric_recipe_admission_are_bounded',
    'full_source_admission_precedes_selection_and_binds_only_direct_inputs',
    'projection_limits_reject_before_rounding_and_total_input_budget_is_bounded',
]
CLI = 'cli::actual_cli_new_only_preserves_inputs_and_refuses_protected_files_and_symlinks'
GPU = 'static_planar_uv_matches_independent_gpu_oracle'
EDITOR = 'actual_cli_installed_room_assign_undo_redo_save_reopen_hidden_sources_and_frame_parity'
FEATURES = {
    'orr_model': ['import'],
    'orr_render': ['imported-scene', 'models'],
    'orr_editor': ['package-manager'],
}
# Exact unified graph, including empty default markers and the editor's
# existing transitive sprite renderer. Never silently accept extra features.
PIN_FEATURES = {
    'orr_model': ['default', 'import'],
    'orr_render': ['default', 'imported-scene', 'models', 'sprites'],
    'orr_editor': ['models', 'package-manager', 'room-project'],
}
PATHS = (
    'crates/orr_model/examples/static_model_planar_uv.rs',
    'crates/orr_model/examples/static_model_planar_uv/authoring.rs',
    'crates/orr_model/tests/static_model_planar_uv.rs',
    'crates/orr_render/tests/static_model_planar_uv.rs',
    'crates/orr_render/tests/common/static_model_planar_uv_oracle.rs',
    'crates/orr_editor/tests/static_model_planar_uv_consumer.rs',
    'docs/static-model-planar-uv.md',
    'tools/static_model_planar_uv_gate.py',
    'tools/test_static_model_planar_uv_gate.py',
    '.github/workflows/static-model-sampler.yml',
)


def fnv(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f'{value:016x}'


def read_json(path):
    assert path.is_file() and not path.is_symlink()
    assert 0 < path.stat().st_size <= 64 * 1024**2
    return lifecycle.strict_json(path)


def manifest(root):
    """Record artifacts even when a producer or later receipt admission fails."""
    result = {}
    if root.exists():
        for path in sorted(root.rglob('*')):
            assert not path.is_symlink(), 'evidence symlink'
            if path.is_file():
                assert path.stat().st_size <= 64 * 1024**2
                result[path.relative_to(root).as_posix()] = {
                    'bytes': path.stat().st_size, 'sha256': runner.digest(path)}
    return result


def capture_reference(root, item):
    assert set(item) == {'file', 'bytes', 'fnv1a64'}
    name = item['file']
    assert isinstance(name, str) and pathlib.PurePosixPath(name).name == name
    path = root / name
    assert path.is_file() and not path.is_symlink()
    assert type(item['bytes']) is int and 0 < item['bytes'] <= 64 * 1024**2
    data = path.read_bytes()
    assert len(data) == item['bytes'] and fnv(data) == item['fnv1a64']
    return path, data


def check_provenance(value, root, binary):
    assert pathlib.Path(value['producer_path']) == pathlib.Path(binary)
    assert value['producer_fnv1a64'] == fnv(pathlib.Path(binary).read_bytes())
    for key, name in (
        ('test_source_fnv1a64', PATHS[3]),
        ('oracle_source_fnv1a64', PATHS[4]),
        ('cli_source_fnv1a64', PATHS[0]),
        ('authoring_source_fnv1a64', PATHS[1]),
    ):
        assert value[key] == fnv((root / name).read_bytes()), key


def check_gpu(captures, source, binary, *, local_adapter=None):
    receipt = read_json(captures / 'receipt.json')
    assert type(receipt['schema']) is int and receipt['schema'] == 1 and receipt['status'] == 'passed'
    adapter = read_json(captures / 'adapter.json')
    assert receipt['adapter'] == adapter
    assert adapter['software'] is True and adapter['backend'] == 'Vulkan'
    allowed = ('llvmpipe', 'lavapipe') if local_adapter is None else (local_adapter.lower(),)
    assert any(name in adapter['name'].lower() for name in allowed)
    provenance = read_json(captures / 'provenance.json')
    assert receipt['provenance'] == provenance
    check_provenance(provenance, source, binary)
    assert type(receipt['width']) is int and type(receipt['height']) is int
    assert receipt['width'] == receipt['height'] == 256
    assets = read_json(captures / 'assets.json')
    assert receipt['assets'] == assets
    assert set(assets) == {'imported_glb','source','recipe','cooked','manifest','asset_id','source_path'}
    for key in ('imported_glb','source','recipe','cooked','manifest'):
        capture_reference(captures, assets[key])
    assert assets['asset_id'] == 'variants/tile.planar-uv.json'
    assert assets['source_path'] == 'inputs/source.model.json'
    cooked = read_json(captures / assets['cooked']['file'])
    assert cooked['asset_id'] == assets['asset_id']
    dependencies = {d['uri']: d['sha256'] for d in cooked['dependencies']}
    assert dependencies == {
        '$source': runner.digest(captures / assets['recipe']['file']),
        assets['source_path']: runner.digest(captures / assets['source']['file'])}
    frames = read_json(captures / 'frames.json')
    assert receipt['frames'] == frames
    assert set(frames) == {'source','projected','scene','wrong_axis','wrong_repeat','local_space','affine_collapsed'}
    pixels = {}
    for name, entries in frames.items():
        assert set(entries) == {'rgba', 'png'}
        _, raw = capture_reference(captures, entries['rgba'])
        png, _ = capture_reference(captures, entries['png'])
        assert len(raw) == 256 * 256 * 4
        assert lifecycle.png_pixels(png, 256, 256) == raw, 'PNG/raw mismatch'
        assert all(raw[i] == 255 for i in range(3, len(raw), 4))
        pixels[name] = raw
    assert pixels['projected'] == pixels['scene']
    assert receipt['control_order'] == ['original_allzero','wrong_axis','wrong_repeat','local_space','affine_collapsed']
    counts = receipt['control_pixel_differences']
    assert len(counts) == 5 and all(type(n) is int and n > 3000 for n in counts)
    for name, count in zip(('source','wrong_axis','wrong_repeat','local_space','affine_collapsed'), counts):
        # Rust counts selected interior pixels, Python also requires visible raw
        # difference; do not pretend the whole-frame count is the interior count.
        assert lifecycle.difference(pixels['projected'], pixels[name], 0) >= count
    assert type(receipt['non_target_identical_pixels']) is int and receipt['non_target_identical_pixels'] > 500
    assert set(receipt['coverage']) == {'source','projected','wrong_axis','wrong_repeat','local_space'}
    for coverage in receipt['coverage'].values():
        assert all(type(coverage[key]) is int for key in ('target','non_target','background'))
        assert coverage['target'] > 5000 and coverage['non_target'] > 500 and coverage['background'] > 50000
    assert len(receipt['coverage']['projected']['target_texels']) == 16
    assert all(type(n) is int and n > 150 for n in receipt['coverage']['projected']['target_texels'])
    for key in ('installed_exact_cli_bytes','authoring_hidden','source_unchanged','geometry_preserved','production_renderers_equal'):
        assert receipt[key] is True, key
    return receipt


def check_editor(captures, source, binary, *, local_adapter=None):
    receipt = read_json(captures / 'acceptance.json')
    assert receipt['status'] == 'passed'
    for key in ('actual_cli','actual_egui_widgets_and_gpu','undo_redo_save_fresh_reopen',
                'authoring_and_package_source_hidden','authoritative_frame_byte_parity'):
        assert receipt[key] is True, key
    assert receipt['os_native_input'] is False
    assert read_json(captures / 'started.json')['cli'] == str(binary)
    adapter = read_json(captures / 'adapter.json')
    assert adapter['backend'] == 'Vulkan' and adapter['device_type'] == 'Cpu'
    allowed = ('llvmpipe','lavapipe') if local_adapter is None else (local_adapter.lower(),)
    assert any(word in adapter['name'].lower() for word in allowed)
    assert receipt['installed_asset'] == 'projected.model.json'
    assert receipt['recipe_identity'] == 'variants/projected.planar-uv.json'
    assert receipt['hidden_inputs'] == ['recipe','source cooked model','original CLI output','package source']
    assert receipt['retained_installed_files'] == ['projected.model.json','foreground.glb']
    assert not (captures/'authoring').exists() and not (captures/'package-source').exists()
    authoring = captures/'authoring-hidden'
    package = captures/'package-source-hidden'
    cooked = package/'projected.model.json'
    assert cooked.read_bytes() == (authoring/'projected.model.json').read_bytes()
    inputs = read_json(captures/'provenance-inputs.json')
    provenance = read_json(captures/'provenance.json')
    assert set(inputs) == {'algorithm','cli','recipe','source','source_files'}
    assert set(provenance) == set(inputs) | {'output','installed'}
    assert inputs['algorithm'] == 'sha256'
    assert all(provenance[key] == value for key,value in inputs.items())
    def check_hash(record, path):
        assert set(record) == {'sha256','length'}
        assert type(record['length']) is int and record['length'] == path.stat().st_size
        assert record['sha256'] == runner.digest(path)
    for key,path in (('cli',pathlib.Path(binary)),('recipe',authoring/'project.planar-uv.json'),
                     ('source',authoring/'inputs/source.model.json')):
        check_hash(inputs[key],path)
    assert set(inputs['source_files']) == {'test','oracle','creator','helper'}
    for key,index in (('test',5),('oracle',4),('creator',0),('helper',1)):
        check_hash(inputs['source_files'][key],source/PATHS[index])
    check_hash(provenance['output'],authoring/'projected.model.json')
    model = read_json(cooked)
    assert model['asset_id'] == receipt['recipe_identity']
    assert {d['uri']:d['sha256'] for d in model['dependencies']} == {
        '$source': runner.digest(authoring/'project.planar-uv.json'),
        'inputs/source.model.json': runner.digest(authoring/'inputs/source.model.json')}
    assert runner.digest(cooked) == receipt['source_hash']
    binding = read_json(captures/'room/room.models.json')['bindings'][receipt['assigned_guid']]
    assert binding['asset'] == receipt['installed_asset'] and binding['source_hash'] == receipt['source_hash']
    assert binding['kind'] == 'static'
    lock = read_json(captures/'room/orr.packages.lock.json')['packages'][binding['package']]
    assert lock['digest'] == binding['package_digest']
    assert re.fullmatch('[0-9a-f]{64}', lock['digest'])
    installed = captures/'room/.orr/packages/objects'/lock['digest']
    check_hash(provenance['installed'],installed/'projected.model.json')
    for name in receipt['retained_installed_files']:
        assert (installed/name).read_bytes() == (package/name).read_bytes()
        assert runner.digest(installed/name) == lock['files'][name]
    frames = {}
    for prefix in ('01-assigned','02-undo','03-redo','04-saved','05-fresh-reopen',
                   '06-source-hidden-play-off','07-source-hidden-play-on'):
        dimensions = read_json(captures/(prefix+'-dimensions.json'))
        assert any(word in dimensions['adapter'].lower() for word in allowed)
        for kind in ('ui','viewport'):
            width,height = dimensions[kind]
            assert type(width) is int and type(height) is int and 0 < width <= 2048 and 0 < height <= 2048
            raw = (captures/(prefix+'-'+kind+'.rgba')).read_bytes()
            assert len(raw) == width*height*4
            assert lifecycle.png_pixels(captures/(prefix+'-'+kind+'.png'),width,height) == raw
            frames[prefix+'-'+kind] = raw
    off = (captures/'presentation-off.frame').read_bytes()
    on = (captures/'presentation-on.frame').read_bytes()
    assert off and off == on
    observed = read_json(captures/'parity-observed.json')
    # Rust alone decodes authoritative Frames; Python binds retained bytes and
    # the exact producer's decoded observations, never invents a Frame layout.
    assert observed['off_tick'] == observed['on_tick'] == receipt['tick'] == 12
    assert observed['off_checksum'] == observed['on_checksum'] == receipt['checksum']
    assert observed['off_frame_bytes'] == observed['on_frame_bytes'] == len(off)
    left = frames['06-source-hidden-play-off-viewport']
    right = frames['07-source-hidden-play-on-viewport']
    assert observed['off_viewport_bytes'] == observed['on_viewport_bytes'] == len(left) == len(right)
    changed = lifecycle.difference(left,right,0)
    assert type(receipt['actual_viewport_changed_pixels']) is int
    assert changed == receipt['actual_viewport_changed_pixels'] and changed > 20
    return receipt


def check_sources(value):
    required = set(PATHS + lifecycle.FEATURE_PATHS + lifecycle.INFRA_PATHS)
    for package in FEATURES:
        required.update((f'crates/{package}/Cargo.toml', f'crates/{package}/src/lib.rs'))
    assert required <= set(value), 'complete feature and lifecycle sources required'


def coverage():
    assert runner.report['coverage'] == dict.fromkeys(('cpu', 'cli', 'gpu', 'editor'), 'PASSED')


def run(label, command, expected=0, seconds=1200):
    return lifecycle.run(label, command, expected=expected, seconds=seconds)


def census(binary, all_names, ignored):
    runner.inventory(run('inventory-' + pathlib.Path(binary).name, [binary, '--list']), all_names)
    runner.inventory(run('ignored-' + pathlib.Path(binary).name, [binary, '--ignored', '--list']), ignored)


def checked_result(output, names, filtered=0):
    summaries = re.findall(r'^test result: ok\. .*$', output, re.M)
    assert len(summaries) == 1
    suffix = f'; {filtered} filtered out; finished in '
    assert suffix in summaries[0]
    runner.result(output.replace(suffix, '; 0 filtered out; finished in '), names)


def execute(label, binary, names, ignored=False, filtered=0):
    flags = ['--ignored'] if ignored else []
    if len(names) == 1:
        flags += ['--exact', names[0]]
    checked_result(run(label, [binary, *flags, '--test-threads=1']), names, filtered)


def batch():
    assert sys.platform == 'linux', 'this required integration is explicitly Linux-only'
    runner.software()  # Mandatory hosted Lavapipe. No local override reaches this path.
    flags = ['--locked', '--release', '--no-default-features']
    off = run('default-off-cli', ['cargo','build',*flags,'-p','orr_model','--example','static_model_planar_uv','--message-format=json'])
    disabled = lifecycle.pin(off,'planar-cli-off','orr_model','static_model_planar_uv','example',[])
    run('default-off-help', [disabled, '--help'], expected=2)
    package_flags = []
    for package in FEATURES:
        package_flags += ['-p', package]
    enabled = ','.join(f'{package}/{feature}' for package, features in FEATURES.items() for feature in features)
    target_flags = ['--test','static_model_planar_uv','--test','static_model_planar_uv_consumer']
    run('grouped-strict', ['cargo','clippy',*flags,*package_flags,'--features',enabled,*target_flags,'--example','static_model_planar_uv','--','-D','warnings'], seconds=2400)
    output = run('creator-build', ['cargo','build',*flags,'-p','orr_model','--features','import','--example','static_model_planar_uv','--message-format=json'])
    binary = lifecycle.pin(output,'planar-cli','orr_model','static_model_planar_uv','example',['import'])
    os.environ.update(ORR_STATIC_PLANAR_UV_CLI=binary, ORR_STATIC_MODEL_PLANAR_UV_BIN=binary)
    output = run('grouped-test-build', ['cargo','test',*flags,*package_flags,'--features',enabled,*target_flags,'--no-run','--message-format=json'], seconds=2400)
    bins = {}
    for package in FEATURES:
        target = 'static_model_planar_uv_consumer' if package == 'orr_editor' else 'static_model_planar_uv'
        # Shared target names require package-filtered JSON before exact pinning.
        records = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
        selected = '\n'.join(json.dumps(r) for r in records if r.get('manifest_path') == str(lifecycle.source_root/'crates'/package/'Cargo.toml'))
        bins[package] = lifecycle.pin(selected,'planar-'+package,package,target,'test',PIN_FEATURES[package])
    census(bins['orr_model'], CPU + [CLI], [CLI])
    # Nonignored execution reports exactly four passes and one ignored entry.
    output = run('cpu-tests', [bins['orr_model'], '--skip', CLI, '--test-threads=1'])
    checked_result(output, CPU, filtered=1)
    runner.report['coverage']['cpu'] = 'PASSED'; runner.record()
    execute('actual-cli-test', bins['orr_model'], [CLI], True, filtered=4)
    runner.report['coverage']['cli'] = 'PASSED'; runner.record()
    for package, name, label, environment, checker in (
        ('orr_render', GPU, 'gpu', 'ORR_STATIC_MODEL_PLANAR_UV_CAPTURE_DIR', check_gpu),
        ('orr_editor', EDITOR, 'editor', 'ORR_PLANAR_UV_EDITOR_EVIDENCE', check_editor),
    ):
        census(bins[package], [name], [name])
        captures = runner.evidence / label
        os.environ[environment] = str(captures)
        try:
            execute(label+'-test', bins[package], [name], True)
        finally:
            runner.report[label+'_artifact_manifest'] = manifest(captures); runner.record()
        runner.report[label+'_receipt'] = checker(captures, lifecycle.source_root, binary)
        runner.report['coverage'][label] = 'PASSED'; runner.record()


def main():
    assert len(sys.argv) == 1, 'mandatory gate has no mode or adapter bypass arguments'
    lifecycle.initialize(pathlib.Path(__file__).resolve().parents[1], pathlib.Path(os.environ['ORR_STATIC_PLANAR_UV_EVIDENCE']), 'ui')
    lifecycle.require_sources = check_sources
    lifecycle.check_coverage = coverage
    runner.report['coverage'] = dict.fromkeys(('cpu','cli','gpu','editor'), 'NOT RUN')
    runner.report['scope'] = 'required Linux llvmpipe batch; not local SwiftShader or Windows certification'
    error = None
    try:
        lifecycle.begin()
        runner.report['planar_feature_paths'] = list(PATHS)
        runner.record()
        output = run('planar-gate-selftests', [sys.executable,'-B',str(lifecycle.source_root/'tools/test_static_model_planar_uv_gate.py')])
        assert output.rstrip().endswith('STATIC_PLANAR_UV_GATE_FIXTURES: passed')
        batch()
    except BaseException as failure:
        error = failure
    finally:
        lifecycle.finish(error)

if __name__ == '__main__':
    main()
