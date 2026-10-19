#!/usr/bin/env python3
"""Mandatory current-source animation-pose acceptance; standard library only.

Hosted cpu/gpu each compile independent, retained producers in a fresh
target. Local focused checks cannot certify full hosted acceptance. The reviewed static
prop runner supplies bounded output, process-tree cleanup and producer identity;
its source and independent fixtures are part of every prebuild source fence.
"""
import hashlib
import json
import os
import pathlib
import re
import sys
import traceback
import struct
import zlib
import math

if not __debug__ or sys.flags.optimize or os.environ.get("PYTHONOPTIMIZE") not in (None, "", "0"):
    raise SystemExit("pose acceptance requires nonoptimized Python")

import static_model_extract_gate as runner

FEATURE_PATHS = (
    'crates/orr_model/examples/animation_pose_bake.rs',
    'crates/orr_model/examples/animation_pose_bake/authoring.rs',
    'crates/orr_model/tests/animation_pose_bake.rs',
    'crates/orr_render/tests/animation_pose_bake.rs',
    'docs/animation-pose-bake.md',
    'tools/animation_pose_bake_gate.py',
    'tools/test_animation_pose_bake_gate.py',
    '.github/workflows/animation-pose-bake.yml',
)
INFRA_PATHS = ('Cargo.toml', 'Cargo.lock', 'tools/static_model_extract_gate.py',
               'tools/test_static_model_extract_gate.py')
GPU_TEST = 'fresh_cli_pose_bake_matches_independent_gpu_oracle'
FIXTURE_COUNT = 9
source_root = None
frozen = None
mode = None


def source_paths(root):
    """Complete source inventory, including new files in a non-Git local copy."""
    result = []
    for directory, folders, files in os.walk(root, followlinks=False):
        parent = pathlib.Path(directory)
        if parent == root:
            folders[:] = [name for name in folders if name not in {'.git', 'target'}]
        for name in folders:
            assert not (parent / name).is_symlink(), 'symlink source directory'
        for name in files:
            path = parent / name
            assert path.is_file() and not path.is_symlink(), 'non-regular source'
            result.append(path.relative_to(root).as_posix())
    return sorted(result)


def check_source_inventory(tracked, phase):
    observed = source_paths(source_root)
    expected = sorted(tracked)
    entry = {'tracked': expected, 'observed': observed,
             'extra': sorted(set(observed) - set(expected)),
             'missing': sorted(set(expected) - set(observed)),
             'matches': expected == observed}
    runner.report.setdefault('source_path_inventories', {})[phase] = entry
    # Preserve the actual path difference even when Git ignores an unexpected
    # file and therefore reports a clean worktree. Do not exempt or remove it.
    runner.record()
    assert expected == observed, 'untracked/ignored source cannot enter producers'


def snapshot():
    result = {}
    for name in source_paths(source_root):
        path = source_root / name
        result[name] = {'sha256': runner.digest(path), 'bytes': path.stat().st_size,
                        'mode': path.stat().st_mode & 0o7777}
    return result


def require_sources(value):
    assert len(FEATURE_PATHS) == len(set(FEATURE_PATHS)) == 8
    required = set(FEATURE_PATHS + INFRA_PATHS)
    for package in ('orr_model', 'orr_render'):
        required.add(f'crates/{package}/Cargo.toml')
        required.add(f'crates/{package}/src/lib.rs')
    assert required <= set(value), 'complete feature and producer sources required'


def source_fence():
    if frozen is not None:
        current = snapshot()
        if list(current) != list(frozen):
            check_source_inventory(list(frozen), 'source-fence')
        assert current == frozen, 'source bytes, modes or paths changed'


def run(label, command, expected=0, seconds=1200):
    source_fence()
    output = runner.run(label, command, expected=expected, seconds=seconds)
    source_fence()
    return output


def initialize(root, destination, phase):
    global source_root, frozen, mode
    assert phase in {'cpu', 'gpu'}
    source_root = pathlib.Path(root).resolve()
    frozen = None
    mode = phase
    destination = pathlib.Path(destination)
    assert not destination.is_relative_to(source_root), 'evidence must be outside source'
    runner.initialize(source_root, destination, phase)
    runner.report['mode'] = phase
    runner.report['coverage'] = {name: 'NOT RUN' for name in ('model', 'gpu')}
    runner.report['local_model_only_cannot_certify_hosted_acceptance'] = True
    runner.record()


def require_inherited_bytecode_prevention():
    observed = {'PYTHONDONTWRITEBYTECODE': os.environ.get('PYTHONDONTWRITEBYTECODE'),
                'sys_dont_write_bytecode': sys.dont_write_bytecode}
    runner.report['python_bytecode_prevention'] = observed
    # Record the inherited state before rejection. A local -B flag alone does
    # not protect Python children launched by earlier workflow producers.
    runner.record()
    assert observed['PYTHONDONTWRITEBYTECODE'] == '1', 'inherited PYTHONDONTWRITEBYTECODE=1 required'


def begin():
    global frozen
    require_inherited_bytecode_prevention()
    assert run('clean-source-before', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
    for key, revision in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
        runner.report[key] = run('checkout-' + key, ['git', 'rev-parse', revision]).strip()
        assert re.fullmatch('[0-9a-f]{40}', runner.report[key])
    if 'GITHUB_SHA' in os.environ:
        assert os.environ['GITHUB_SHA'] == runner.report['head']
    tracked = run('tracked-source', ['git', 'ls-files', '-z']).split('\0')
    assert tracked.pop() == ''
    check_source_inventory(tracked, 'before')
    frozen = snapshot()
    require_sources(frozen)
    runner.report.update(checkout=str(source_root), source_sha256=frozen,
                         source_inventory_sha256=hashlib.sha256(json.dumps(frozen, sort_keys=True).encode()).hexdigest(),
                         feature_paths=list(FEATURE_PATHS))
    runner.record()
    for file, marker in [
        ('test_static_model_extract_gate.py', 'STATIC_PROP_GATE_FIXTURES: 29 passed; 0 failed; 0 skipped'),
        ('test_animation_pose_bake_gate.py', f'ANIMATION_POSE_BAKE_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped'),
    ]:
        output = run('checker-' + file[:-3], [sys.executable, '-B', str(source_root / 'tools' / file)])
        assert output.rstrip().endswith(marker), 'independent gate fixture census'


def pin(output, label, package, target, kind, features):
    source_fence()
    binary = runner.pin(output, label, package, target, kind, features)
    producer = runner.report['producers'][label]
    producer['source_inventory_sha256'] = runner.report['source_inventory_sha256']
    producer['source_root'] = str(source_root)
    runner.record()
    return binary


def test(label, binary, names, ignored=False):
    runner.inventory(run(label + '-inventory', [binary, '--list']), names)
    runner.inventory(run(label + '-ignored', [binary, '--ignored', '--list']), names if ignored else [])
    flags = ['--ignored', '--exact', names[0]] if ignored and len(names) == 1 else (['--ignored'] if ignored else [])
    runner.result(run(label + '-run', [binary, *flags, '--test-threads=1']), names)


def cargo_flags(package='orr_model'):
    return ['--locked', '--release', '-p', package, '--no-default-features']


def build_cli():
    flags = cargo_flags()
    output = run('build-cli', ['cargo', 'build', *flags, '--features', 'animation,import',
                              '--example', 'animation_pose_bake', '--message-format=json'])
    binary = pin(output, 'pose-cli', 'orr_model', 'animation_pose_bake', 'example', ['animation', 'import'])
    os.environ['ORR_ANIMATION_POSE_BAKE_BIN'] = binary
    return binary


def model_tests(platform):
    assert platform in {'linux', 'darwin', 'win32'}
    names = ['weighted_skin_hierarchy_inverse_bind_and_rigid_model_space_oracle', 'f64_time_bounds_precede_f32_rounding_and_clip_index_is_exact', 'recipe_accepts_exact_object_whitespace_and_16kib_boundary', 'recipe_rejects_missing_duplicate_unknown_null_and_positional_aliases', 'logical_paths_reject_nonportable_unnormalized_suffixes_and_aliases', 'logical_paths_accept_240_byte_boundary', 'source_resolves_once_no_historical_dependencies_and_errors_propagate', 'provenance_binds_exact_bytes_and_roundtrip_is_deterministic', 'cli_publishes_once_without_overwrite_and_preserves_all_inputs', 'cli_help_and_three_unique_pairs_are_bounded', 'cli_bad_arguments_fail_without_output_or_staging', 'cli_portable_output_names_and_package_store_rejection', 'cli_missing_nonregular_and_oversized_inputs_leave_no_output', 'cli_resolver_is_anchored_to_recipe_parent', 'cli_parent_traversal_preserves_raw_spelling_and_never_publishes', 'step_boundaries_late_early_single_tracks_and_zero_duration', 'full_source_admission_rejects_unused_invalid_assets_and_unknown_fields', 'recipe_wrong_types_versions_and_positive_underflow_reject', 'singular_and_mirrored_weighted_samples_fail_without_rest_pose_fallback', 'baked_static_local_position_bounds_are_stricter_than_animation_bounds', 'combined_recipe_and_source_input_budget_is_enforced']
    if platform == 'win32':
        names.append('cli_windows_nonlocal_prefixes_reject')
    else:
        names.extend(['cli_symlink_inputs_ancestors_and_destinations_reject', 'cli_non_utf8_option_names_and_logical_ids_reject', 'cli_unix_socket_inputs_reject_without_opening_or_publishing'])
        if platform == 'linux':
            names.append('cli_non_utf8_filesystem_inputs_work')
    return names


def model():
    flags = cargo_flags()
    output = run('build-feature-off', ['cargo', 'build', *flags, '--example',
                                      'animation_pose_bake', '--message-format=json'])
    disabled = pin(output, 'feature-off-cli', 'orr_model', 'animation_pose_bake', 'example', [])
    assert 'requires --features animation,import' in run('feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli()
    assert 'Usage: animation_pose_bake' in run('actual-help', [cli, '--help'])
    output = run('build-model-test', ['cargo', 'test', *flags, '--features', 'animation,import',
                                    '--test', 'animation_pose_bake', '--no-run', '--message-format=json'])
    binary = pin(output, 'model-test', 'orr_model', 'animation_pose_bake', 'test', ['animation', 'import'])
    test('model', binary, model_tests(sys.platform))
    # Full original model regression suites remain mandatory, including Unix
    # socket refusal fixtures; restricted local execution is not a waiver.
    run('model-full-regressions', ['cargo', 'test', *flags, '--features', 'animation,import', '--tests', '--', '--test-threads=1'])
    run('strict-model-import', ['cargo', 'clippy', *flags, '--features', 'animation,import',
                               '--lib', '--example', 'animation_pose_bake', '--test',
                               'animation_pose_bake', '--', '-D', 'warnings'])
    run('strict-model-default', ['cargo', 'clippy', *flags, '--lib', '--example',
                                'animation_pose_bake', '--', '-D', 'warnings'])
    runner.report['coverage']['model'] = 'PASSED'
    runner.record()


def check_coverage():
    expected = {'cpu': {'model'}, 'gpu': {'gpu'}}[mode]
    assert runner.report['coverage'] == {name: ('PASSED' if name in expected else 'NOT RUN')
                                         for name in ('model', 'gpu')}


def finish(error):
    try:
        assert frozen is not None, 'prebuild source snapshot missing'
        source_fence()
        runner.fence()
        assert run('clean-source-after', ['git', 'status', '--porcelain=v1', '--untracked-files=all']).strip() == ''
        for key, revision in [('head', 'HEAD'), ('tree', 'HEAD^{tree}')]:
            assert run('checkout-' + key + '-after', ['git', 'rev-parse', revision]).strip() == runner.report[key]
        tracked = run('tracked-source-after', ['git', 'ls-files', '-z']).split('\0')
        assert tracked.pop() == ''
        check_source_inventory(tracked, 'after')
        assert sorted(tracked) == list(frozen)
        runner.report['source_unchanged'] = True
        if error is None:
            check_coverage()
    except BaseException as final_error:
        runner.report['final_fence_error'] = repr(final_error)
        error = error or final_error
    runner.report['status'] = 'failed' if error is not None else 'passed'
    if error is not None:
        runner.report['failure'] = ''.join(traceback.format_exception(error))
    runner.record()
    if error is not None:
        raise SystemExit(error.code if isinstance(error, runner.GateFailure) else 1)


def main():
    assert len(sys.argv) == 2 and sys.argv[1] in {'cpu', 'gpu'}, (
        'usage: animation_pose_bake_gate.py cpu|gpu')
    initialize(pathlib.Path(__file__).resolve().parents[1],
               pathlib.Path(os.environ['ORR_ANIMATION_POSE_BAKE_EVIDENCE']), sys.argv[1])
    error = None
    try:
        begin()
        {'cpu': model, 'gpu': gpu}[mode]()
    except BaseException as failure:
        error = failure
    finally:
        finish(error)


CONTROLS = ('wrong-pose', 'wrong-weights', 'double-mesh-transform', 'blended-normal-matrices')
FRAMES = ('static-midpoint', 'skinned-midpoint') + CONTROLS

def pixel_oracle():
    scales = ((1.,1.,1.), (1.8,.55,1.1), (.7,1.4,.8), (1.3,.8,1.5))
    matrices = []
    for angle, (sx, sy, sz) in zip((0.,30.,-25.,20.), scales):
        c, s = math.cos(math.radians(angle)), math.sin(math.radians(angle))
        a,b,cc,d = c*sx,s*sx,-s*sy,c*sy
        matrices.append((1.1*a,.9*b,1.1*cc,.9*d,1.2*sz,-.9+1.1*(.2-a*.2-cc*.3),.1+.9*(.3-b*.2-d*.3)))
    blend = tuple(sum(w*m[k] for w,m in zip((.1,.4,.2,.3),matrices)) for k in range(7))
    affine = (blend, (.7,0.,0.,.8,1.3,1.1,-.65))
    quad = ((-.65,-.65),(.65,-.65),(.65,.65),(-.65,.65))
    uv = ((0.,1.),(1.,1.),(1.,0.),(0.,0.))
    checker = ((225,83,41),(47,198,112),(108,56,237),(173,219,91))
    screens, colors = [], []
    for a,b,c,d,e,tx,ty in affine:
        screens.append([(((a*x+c*y+tx)*1.1+.1)/2.5*128+128,128-((b*x+d*y+ty)*.85+.2)/2.5*128) for x,y in quad])
        determinant = a*d-b*c
        normal = ((d-b)/determinant/1.1,(a-c)/determinant/.85,1/e/1.4)
        brightness = .1+.85*normal[2]/math.sqrt(sum(v*v for v in normal))
        def color(texel):
            out=[]
            for byte, factor in zip(texel,(.8,.7,.9)):
                v=byte/255
                linear=v/12.92 if v<=.04045 else ((v+.055)/1.055)**2.4
                v=linear*factor*brightness
                srgb=12.92*v if v<=.0031308 else 1.055*v**(1/2.4)-.055
                out.append(int(max(0,min(1,srgb))*255+.5))
            return tuple(out)+(255,)
        colors.append([color(texel) for texel in checker])
    def bary(p,a,b,c):
        d=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
        u=((b[1]-c[1])*(p[0]-c[0])+(c[0]-b[0])*(p[1]-c[1]))/d
        v=((c[1]-a[1])*(p[0]-c[0])+(a[0]-c[0])*(p[1]-c[1]))/d
        return u,v,1-u-v
    pixels=[]; interior=[0,0];clear=0
    for y in range(256):
        for x in range(256):
            expected=None;near=False
            for part in range(2):
                for tri in ((0,1,2),(0,2,3)):
                    weights=bary((x+.5,y+.5),*(screens[part][i] for i in tri))
                    near |= min(weights)>=-.08
                    if min(weights)<=.08:continue
                    tex=[2*sum(weights[j]*uv[tri[j]][k] for j in range(3)) for k in range(2)]
                    if any(v%1<.03 or v%1>.97 for v in tex):continue
                    expected=colors[part][int(tex[1])*2+int(tex[0])];interior[part]+=1
            if not near:expected=(0,0,0,255);clear+=1
            pixels.append(expected)
    assert min(interior)>500 and clear>40000
    return pixels,interior,clear

def mismatches(data, pixels):
    assert len(data)==256*256*4
    return sum(expected is not None and any(abs(a-b)>3 for a,b in zip(data[i*4:i*4+4],expected)) for i,expected in enumerate(pixels))

def check_gpu(captures):
    expected={'report.json'} | {f'{name}.{ext}' for name in FRAMES for ext in ('rgba','ppm')} | {f'{name}.{ext}' for name in ('midpoint','wrong-pose','wrong-weights') for ext in ('animated.json','pose.json','model.json','stdout','stderr')}
    assert captures.is_dir() and not captures.is_symlink()
    assert {p.name for p in captures.iterdir()}==expected, 'exact capture inventory'
    files={}
    for path in captures.iterdir():
        assert path.is_file() and not path.is_symlink() and path.stat().st_size<=64*1024*1024
        files[path.name]={'sha256':runner.digest(path),'bytes':path.stat().st_size}
    report=json.loads((captures/'report.json').read_text())
    assert report['size']==256 and report['tolerance']==3
    assert report['adapter']['software'] is True and report['adapter']['backend']=='Vulkan'
    assert 'llvmpipe' in report['adapter']['name'].lower() or 'lavapipe' in report['adapter']['name'].lower()
    pixels,interior,clear=pixel_oracle()
    assert report['interior']==interior and report['clear']==clear
    assert report['static_oracle_mismatches']==report['skinned_oracle_mismatches']==0
    controls={entry['name']:entry['oracle_mismatches'] for entry in report['controls']}
    assert len(report['controls'])==4 and set(controls)==set(CONTROLS)
    measured={}
    for name in FRAMES:
        data=(captures/(name+'.rgba')).read_bytes()
        ppm=b'P6\n256 256\n255\n'+b''.join(data[i:i+3] for i in range(0,len(data),4))
        assert (captures/(name+'.ppm')).read_bytes()==ppm, 'PPM pixels must match raw frame'
        measured[name]=mismatches(data,pixels)
        if name in CONTROLS:assert measured[name]>=100 and measured[name]==controls[name]
        else:assert measured[name]==0, 'independent pixel oracle mismatch'
    for name in ('midpoint','wrong-pose','wrong-weights'):
        recipe_bytes=(captures/(name+'.pose.json')).read_bytes()
        source_bytes=(captures/(name+'.animated.json')).read_bytes()
        recipe=json.loads(recipe_bytes);source=json.loads(source_bytes)
        output=json.loads((captures/(name+'.model.json')).read_bytes())
        assert output['format']=='orr_static_model' and output['version']==1
        assert output['dependencies']==[{'uri':'$source','sha256':hashlib.sha256(recipe_bytes).hexdigest()},{'uri':recipe['source'],'sha256':hashlib.sha256(source_bytes).hexdigest()}]
        assert output['materials']==source['materials'] and output['images']==source['images']
        assert len(output['primitives'])==len(source['primitives'])==2
        for before,after in zip(source['primitives'],output['primitives']):
            assert after['indices']==before['indices'] and after['material']==before['material']
            assert after['transform']==[[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]
            assert [v['uv'] for v in after['vertices']]==[v['vertex']['uv'] for v in before['vertices']]
        assert b'published ' in (captures/(name+'.stdout')).read_bytes()
        assert (captures/(name+'.stderr')).read_bytes()==b''
    runner.report['gpu_capture_files']=files
    runner.report['independent_pixel_oracle']={'interior':interior,'clear':clear,'mismatches':measured}
    runner.record()

def gpu():
    runner.software()
    build_cli()
    captures=runner.evidence/'captures'
    assert not captures.exists()
    os.environ['ORR_ANIMATION_POSE_BAKE_CAPTURE_DIR']=str(captures)
    flags=cargo_flags('orr_render')
    output=run('build-render-test',['cargo','test',*flags,'--features','animation,imported-scene,metallic-roughness','--test','animation_pose_bake','--no-run','--message-format=json'])
    binary=pin(output,'render-test','orr_render','animation_pose_bake','test',['animation','imported-scene','metallic-roughness','models'])
    run('strict-render',['cargo','clippy',*flags,'--features','animation,imported-scene,metallic-roughness','--test','animation_pose_bake','--','-D','warnings'])
    test('render',binary,[GPU_TEST],ignored=True)
    check_gpu(captures)
    runner.report['coverage']['gpu']='PASSED'


if __name__ == '__main__':
    main()
