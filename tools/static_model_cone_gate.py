#!/usr/bin/env python3
"""Private cone extension of the unchanged four-row cylinder acceptance matrix.

All commands and executable pins use the parent's source/mode/target fences.
No standalone entry point, fallback runner, synthetic GPU pass, or native claim.
"""
import functools
import hashlib
import json
import math
import os
import pathlib
import re
import struct
import sys
import urllib.parse

FEATURE_PATHS = (
    'crates/orr_model/examples/static_model_cone.rs',
    'crates/orr_model/examples/static_model_cone/authoring.rs',
    'crates/orr_model/tests/static_model_cone_cli.rs',
    'crates/orr_editor/tests/static_model_cone_package_preview_ui.rs',
    'docs/static-model-cone.md',
    'tools/static_model_cone_gate.py',
    'tools/test_static_model_cone_gate.py',
)
UI_TEST = 'installed_cone_key_preview_undo_reopen_collect_restart'
CLI_TARGET = 'static_model_cone'
MODEL_TARGET = 'static_model_cone_cli'
UI_TARGET = 'static_model_cone_package_preview_ui'
FIXTURE_COUNT = 12


def build_cli(g):
    output = g.run('cone-build-cli', ['cargo', 'build', *g.cargo_flags(), '--features', 'import',
                                    '--example', CLI_TARGET, '--message-format=json'])
    binary = g.pin(output, 'cone-cli', 'orr_model', CLI_TARGET, 'example', ['import'])
    os.environ['ORR_STATIC_MODEL_CONE_BIN'] = binary
    return binary


def retained_producer(g, label):
    entry = g.runner.report['producers'][label]
    assert entry['source_inventory_sha256'] == g.runner.report['source_inventory_sha256'], 'stale cone producer source inventory'
    assert entry['source_root'] == str(g.source_root), 'stale cone producer checkout'
    expected = {
        'cone-cli':('orr_model',CLI_TARGET,'example',['import']),
        'cylinder-cli':('orr_model','static_model_cylinder','example',['import']),
        'cone-editor-test':('orr_editor',UI_TARGET,'test',['models','package-manager','room-project']),
    }
    package,target_name,kind,features=expected[label]
    producer=entry['producer']; target=producer['target']; crate=g.source_root/'crates'/package
    assert producer['reason']=='compiler-artifact' and producer['fresh'] is False
    assert pathlib.Path(producer['manifest_path']).resolve()==crate/'Cargo.toml'
    assert producer['package_id'].startswith('path+')
    uri=urllib.parse.urlsplit(producer['package_id'][5:].split('#',1)[0])
    assert uri.scheme=='file' and uri.netloc==''
    package_path=urllib.parse.unquote(uri.path)
    if os.name=='nt': package_path=package_path.lstrip('/')
    assert pathlib.Path(package_path).resolve()==crate
    folder='examples' if kind=='example' else 'tests'
    assert target['name']==target_name and target['kind']==[kind] and target['crate_types']==['bin']
    assert pathlib.Path(target['src_path']).resolve()==crate/folder/(target_name+'.rs')
    assert producer['profile']['test'] is (kind=='test') and producer['profile']['opt_level']=='3'
    assert sorted(producer['features'])==features
    original=pathlib.Path(producer['executable'])
    directory=g.runner.target_dir/'release'/('examples' if kind=='example' else 'deps')
    assert original.is_absolute() and original==original.resolve() and original.parent==directory
    path = g.runner.evidence / entry['file']
    assert path.parent == g.runner.evidence and path.name == label + ('.exe' if os.name == 'nt' else '')
    g.regular(path, 128 * 1024**2)
    assert path.stat().st_size == entry['bytes'] > 0
    assert g.runner.digest(path) == entry['sha256'], 'changed retained cone producer'
    return path


def cpu(g):
    flags = g.cargo_flags()
    output = g.run('cone-build-feature-off', ['cargo', 'build', *flags, '--example', CLI_TARGET, '--message-format=json'])
    disabled = g.pin(output, 'cone-feature-off-cli', 'orr_model', CLI_TARGET, 'example', [])
    assert 'requires --features import' in g.run('cone-feature-off-exit2', [disabled, '--help'], expected=2)
    cli = build_cli(g)
    assert 'Usage: static_model_cone' in g.run('cone-actual-help', [cli, '--help'])
    output = g.run('cone-build-model-test', ['cargo', 'test', *flags, '--features', 'import',
                      '--test', MODEL_TARGET, '--no-run', '--message-format=json'])
    binary = g.pin(output, 'cone-model-test', 'orr_model', MODEL_TARGET, 'test', ['import'])
    names = g.model_tests(sys.platform)
    g.runner.inventory(g.run('cone-model-inventory', [binary, '--list']), names + [g.CLI_TEST])
    g.runner.inventory(g.run('cone-model-ignored', [binary, '--ignored', '--list']), [g.CLI_TEST])
    g.executed(g.run('cone-model-ordinary', [binary, '--test-threads=1']), names, ignored_names=[g.CLI_TEST])
    g.executed(g.run('cone-model-required-cli', [binary, '--ignored', '--exact', g.CLI_TEST, '--test-threads=1']),
               [g.CLI_TEST], filtered=len(names))
    g.run('cone-strict-model-import', ['cargo', 'clippy', *flags, '--features', 'import',
             '--example', CLI_TARGET, '--test', MODEL_TARGET, '--', '-D', 'warnings'])
    g.run('cone-strict-model-default', ['cargo', 'clippy', *flags, '--example', CLI_TARGET, '--', '-D', 'warnings'])
    captures = g.runner.evidence / 'cone-cli-captures'
    captures.mkdir()  # Existing output is a hard failure, never reused.
    recipe = captures / 'key.json'
    recipe.write_text(json.dumps({'format':'orr_static_model_cone', 'version':1,
                                 'radius':.55, 'height':2., 'segments':32}), encoding='utf-8')
    cooked = captures / 'key.orrmodel'
    g.run('cone-retained-cli-proof', [cli, '--recipe', str(recipe), '--asset-id', 'props/key.cone.json', '--output', str(cooked)])
    check_cli(g, captures)
    g.runner.report['coverage']['cone_model'] = 'PASSED'
    g.runner.record()


def check_cli(g, captures):
    inventory = g.capture_files(captures, {'key.json', 'key.orrmodel'})
    recipe=g.strict_json(captures/'key.json')
    assert (recipe['radius'],recipe['height'],recipe['segments'])==(.55,2.,32)
    check_cone(g, captures/'key.json', captures/'key.orrmodel', 'props/key.cone.json')
    producer = retained_producer(g, 'cone-cli')
    g.runner.report['cone_cli_receipt'] = {
        'source_inventory_sha256':g.runner.report['source_inventory_sha256'],
        'producer_sha256':g.runner.digest(producer), 'capture_inventory':inventory,
        'recipe_sha256':g.runner.digest(captures/'key.json'),
        'cooked_sha256':g.runner.digest(captures/'key.orrmodel'),
        'geometry_and_recipe_provenance':'PASSED', 'native_gpu':False,
    }
    g.runner.record()


def ui(g):
    # The existing ui() has already configured/fenced software Vulkan and built
    # the independent cylinder CLI. Bind that exact retained negative control.
    cylinder = retained_producer(g, 'cylinder-cli')
    assert os.environ['ORR_STATIC_MODEL_CYLINDER_BIN'] == str(cylinder)
    build_cli(g)
    captures = g.runner.evidence / 'cone-captures'
    assert not captures.exists(), 'fresh cone captures required'
    os.environ['ORR_STATIC_MODEL_CONE_PREVIEW_CAPTURE_DIR'] = str(captures)
    flags = g.cargo_flags('orr_editor')
    output = g.run('cone-build-editor-test', ['cargo', 'test', *flags, '--features', 'package-manager',
                   '--test', UI_TARGET, '--no-run', '--message-format=json'], seconds=2400)
    binary = g.pin(output, 'cone-editor-test', 'orr_editor', UI_TARGET, 'test', ['models','package-manager','room-project'])
    g.run('cone-strict-editor-test', ['cargo', 'clippy', *flags, '--features', 'package-manager',
                                    '--test', UI_TARGET, '--', '-D', 'warnings'])
    g.test('cone-editor', binary, [UI_TEST], ignored=True)
    check_ui(g, captures)
    g.runner.report['coverage']['cone_ui'] = 'PASSED'
    g.runner.record()

def validate_cone(g, model, recipe, recipe_hash, asset_id):
    assert set(recipe) == {'format', 'version', 'radius', 'height', 'segments'}
    assert recipe['format'] == 'orr_static_model_cone' and type(recipe['version']) is int and recipe['version'] == 1
    assert type(recipe['segments']) is int and recipe['segments'] in (8, 16, 32, 64, 128)
    for key in ('radius', 'height'):
        assert type(recipe[key]) in (int, float) and math.isfinite(recipe[key]) and .001 <= recipe[key] <= 1000000
    assert set(model) == {'format', 'version', 'asset_id', 'dependencies', 'primitives', 'materials', 'images'}
    assert model['format'] == 'orr_static_model' and type(model['version']) is int and model['version'] == 1
    assert model['asset_id'] == asset_id
    assert model['dependencies'] == [{'uri': '$source', 'sha256': recipe_hash}]
    assert model['images'] == [{'width': 1, 'height': 1, 'rgba8': [255]*4}]
    expected_material = {'base_color': [1.]*4, 'image': 0, 'linear_filter': False,
                         'wrap_s': 'clamp', 'wrap_t': 'clamp'}
    assert len(model['materials']) == 2 and all(item == expected_material for item in model['materials'])
    assert len(model['primitives']) == 2, 'two capped-cone primitives required'
    identity = [[1.,0.,0.,0.], [0.,1.,0.,0.], [0.,0.,1.,0.], [0.,0.,0.,1.]]
    edges = {}
    expected = geometry_oracle(g, recipe['radius'], recipe['height'], recipe['segments'])
    for slot, (primitive, (vertices, indices)) in enumerate(zip(model['primitives'], expected)):
        assert set(primitive) == {'id', 'vertices', 'indices', 'material', 'transform'}
        assert primitive['id'] == f'{asset_id}#node=0/mesh=0/primitive={slot}'
        assert type(primitive['material']) is int and primitive['material'] == slot
        assert primitive['transform'] == identity
        assert primitive['indices'] == indices and all(type(index) is int for index in primitive['indices'])
        assert len(primitive['vertices']) == len(vertices)
        for actual, target in zip(primitive['vertices'], vertices):
            assert set(actual) == {'position', 'normal', 'uv'}
            for field, extent in (('position', 3), ('normal', 3), ('uv', 2)):
                assert type(actual[field]) is list and len(actual[field]) == extent
                # Match analytic geometry within binary32 rounding, not an
                # implementation-specific quarter-table byte copy.
                for value, wanted in zip(actual[field], target[field]):
                    assert abs(g.f32(value)-wanted) <= (2e-6 if field == 'normal' else max(abs(wanted)*2e-7, 1e-7)), f'cone {field} analytic oracle'
        for offset in range(0, len(indices), 3):
            triangle = [primitive['vertices'][index]['position'] for index in indices[offset:offset+3]]
            a, b, c = triangle
            ab, ac = [b[i]-a[i] for i in range(3)], [c[i]-a[i] for i in range(3)]
            normal = [ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0]]
            center = [sum(point[i] for point in triangle)/3 for i in range(3)]
            outward = [center[0], 0., center[2]] if slot == 0 else [0., -1., 0.]
            assert sum(normal[i]*outward[i] for i in range(3)) > 0, 'outward nondegenerate triangle'
            for first, second in ((a,b), (b,c), (c,a)):
                first, second = tuple(g.f32(v) for v in first), tuple(g.f32(v) for v in second)
                assert first != second
                key = tuple(sorted((first, second)))
                edges.setdefault(key, []).append((first, second))
    assert all(len(pair) == 2 and pair[0] == pair[1][::-1] for pair in edges.values()), 'closed oriented welded surface'
    return model


def check_cone(g, recipe_path, model_path, asset_id):
    assert g.regular(recipe_path).stat().st_size <= 16*1024
    return validate_cone(g, g.strict_json(model_path), g.strict_json(recipe_path), g.runner.digest(recipe_path), asset_id)


def geometry_oracle(g, radius, height, segments):
    """Analytic ring, individually indexed facets and flat downward cap."""
    assert segments in (8, 16, 32, 64, 128)
    radius, half = g.f32(radius), g.f32(g.f32(height)*.5)
    ring = []
    for i in range(segments):
        angle = math.tau*i/segments
        x, z = math.cos(angle), math.sin(angle)
        if i % (segments//4) == 0:
            x, z = ((1.,0.),(0.,1.),(-1.,0.),(0.,-1.))[i//(segments//4)]
        ring.append((g.f32(x),g.f32(z)))
    points = [[g.f32(radius*x), -half, g.f32(radius*z)] for x,z in ring]
    side = []
    for i in range(segments):
        a, b, c = points[i], [0.,half,0.], points[(i+1)%segments]
        ab, ac = [b[k]-a[k] for k in range(3)], [c[k]-a[k] for k in range(3)]
        normal = [ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0]]
        length = math.sqrt(sum(v*v for v in normal))
        normal = [g.f32(v/length) for v in normal]
        for point, uv in zip((a,b,c), ((i/segments,1.),((i+.5)/segments,0.),((i+1)/segments,1.))):
            side.append({'position':point, 'normal':normal[:], 'uv':list(uv)})
    bottom = [{'position':[0.,-half,0.], 'normal':[0.,-1.,0.], 'uv':[.5,.5]}]
    for point,(x,z) in zip(points,ring):
        bottom.append({'position':point, 'normal':[0.,-1.,0.],
                       'uv':[g.f32(.5+g.f32(.5*x)), g.f32(.5+g.f32(.5*z))]})
    indices = [item for i in range(segments) for item in (0,i+1,(i+1)%segments+1)]
    return [(side,list(range(3*segments))), (bottom,indices)]


@functools.lru_cache(maxsize=2)
def silhouette_samples(radius, height, segments):
    """Independent convex halfspace rays, not the Rust triangle mask oracle."""
    width, rows = 448, 336
    sy, cy, sp, cp = math.sin(.45), math.cos(.45), math.sin(.25), math.cos(.25)
    scale = rows/(2*math.sqrt(2*radius*radius+height*height/4)*1.2)
    apothem = radius*math.cos(math.pi/segments)
    normals = [(math.cos((i+.5)*math.tau/segments), apothem/height,
                math.sin((i+.5)*math.tau/segments), apothem/2) for i in range(segments)]
    normals.append((0.,-1.,0.,height/2))
    direction = (sy*cp,sp,cy*cp)
    planes = [(nx,ny,nz,limit,nx*direction[0]+ny*direction[1]+nz*direction[2]) for nx,ny,nz,limit in normals]
    mask = []
    for y in range(rows):
        screen_y = (rows/2-y-.5)/scale
        for x in range(width):
            screen_x = (x+.5-width/2)/scale
            origin = (cy*screen_x-sy*sp*screen_y, cp*screen_y, -sy*screen_x-cy*sp*screen_y)
            low, high = -math.inf, math.inf
            for nx,ny,nz,limit,slope in planes:
                distance = limit-nx*origin[0]-ny*origin[1]-nz*origin[2]
                if abs(slope)<1e-12:
                    if distance<0: low,high=1.,0.;break
                elif slope>0: high=min(high,distance/slope)
                else: low=max(low,distance/slope)
                if low>high: break
            mask.append(low<=high)
    samples = []
    for y in range(3,rows-3):
        for x in range(3,width-3):
            want=mask[y*width+x]
            if all(mask[yy*width+xx]==want for yy in range(y-3,y+4) for xx in range(x-3,x+4)):
                samples.append(((y*width+x)*4,want))
    assert sum(want for _,want in samples)>4000
    return samples


def check_silhouette(raw, spec):
    assert len(raw)==448*336*4
    samples = silhouette_samples(*spec)
    background=raw[:3]
    mismatch=sum(any(abs(raw[offset+c]-background[c])>12 for c in range(3)) != want for offset,want in samples)
    return len(samples),mismatch


EXTRA_FIELDS = {'cylinder_control_producer_fnv1a64','cone_triangle_oracle',
                'tall_silhouette_pixels_checked','wide_silhouette_pixels_checked',
                'cylinder_negative_control_mismatches','native_os_input','native_gpu'}

def check_ui(g, captures):
    expected={name+'.png' for name in g.UI_PNGS}
    expected.add('03-cylinder-negative-control.png')
    raw_names = ('02-original-canvas','03-tall-canvas','03-cylinder-negative-control',
                 '04-wide-canvas','05-orbit-canvas','06-zoom-canvas','07-reset-canvas')
    expected.update(name+'.rgba' for name in raw_names)
    expected.update('cli-cylinder-control.'+stream+'.log' for stream in ('stdout','stderr'))
    expected.update(f'cli-{name}.{stream}.log' for name in g.GPU_SPECS for stream in ('stdout','stderr'))
    expected.update({'receipt.json','room','hidden-package-source','hidden-authoring','actual-gui.orrp',
                     'collected-authoritative.frame','presentation-free.frame'})
    inventory=g.capture_files(captures,expected)
    receipt=g.strict_json(captures/'receipt.json')
    check_ui_identity(g, receipt)
    frames={}
    for name in (*g.UI_PNGS, '03-cylinder-negative-control'):
        path=captures/(name+'.png');header=g.regular(path).read_bytes()[:33]
        assert len(header)==33 and header[12:16]==b'IHDR'
        width,height=struct.unpack_from('>II',header,16)
        assert 0<width<=2048 and 0<height<=2048
        if name.endswith('-canvas'):assert width<=1024 and height<=1024
        frames[name]=((width,height),g.png_pixels(path,width,height))
    for key,first,second in [('original_changed_pixels','02-original-canvas','03-tall-canvas'),
                             ('proportions_changed_pixels','03-tall-canvas','04-wide-canvas'),
                             ('orbit_changed_pixels','03-tall-canvas','05-orbit-canvas'),
                             ('zoom_changed_pixels','05-orbit-canvas','06-zoom-canvas'),
                             ('collect_changed_pixels','10-reopened-viewport','11-collected-viewport')]:
        assert frames[first][0]==frames[second][0]
        observed=g.difference(frames[first][1],frames[second][1])
        assert type(receipt[key]) is int and receipt[key]==observed and observed>1000,key
    assert frames['03-tall-canvas']==frames['07-reset-canvas']
    saved=frames['09-saved-viewport']
    assert saved==frames['10-reopened-viewport']==frames['12-restarted-viewport']
    for name in ('09-saved-viewport','10-reopened-viewport','11-collected-viewport','12-restarted-viewport'):
        dimensions,pixels=frames[name];assert dimensions==(900,850)
        assert sum(pixels[i:i+4]!=pixels[:4] for i in range(0,len(pixels),4))>1000, 'nonblank actual production viewport'
    entry=g.validate_installed(captures,'room','static-cone-preview',['original.orrmodel','tall.orrmodel','wide.orrmodel','cylinder-control.orrmodel'])
    for name,spec in g.GPU_SPECS.items():
        recipe_path=captures/'hidden-authoring'/(name+'.json')
        model_path=captures/'hidden-package-source'/(name+'.orrmodel')
        recipe=g.strict_json(recipe_path)
        assert (recipe['radius'],recipe['height'],recipe['segments'])==spec
        check_cone(g,recipe_path,model_path,f'props/{name}.cone.json')
        assert g.regular(captures/'hidden-authoring'/(name+'.orrmodel')).read_bytes()==model_path.read_bytes()
    control_recipe=captures/'hidden-authoring/cylinder-control.json'
    control_model=captures/'hidden-package-source/cylinder-control.orrmodel'
    control=g.strict_json(control_recipe)
    assert (control['radius'],control['height'],control['segments'])==g.GPU_SPECS['tall']
    g.check_cylinder(control_recipe,control_model,'props/cylinder-control.cylinder.json')
    assert g.regular(captures/'hidden-authoring/cylinder-control.model.json').read_bytes()==control_model.read_bytes()
    for name in raw_names:
        dimensions,pixels=frames[name]
        assert dimensions==(448,336)
        assert g.regular(captures/(name+'.rgba')).read_bytes()==pixels
    for name in ('tall','wide'):
        count,mismatches=check_silhouette(frames['03-tall-canvas' if name=='tall' else '04-wide-canvas'][1],g.GPU_SPECS[name])
        assert type(receipt[name+'_silhouette_pixels_checked']) is int
        assert receipt[name+'_silhouette_pixels_checked']==count and mismatches==0
    count,mismatches=check_silhouette(frames['03-cylinder-negative-control'][1],g.GPU_SPECS['tall'])
    assert type(receipt['cylinder_negative_control_mismatches']) is int
    assert receipt['cylinder_negative_control_mismatches']==mismatches>1000
    sidecar=g.strict_json(captures/'room/room.models.json')
    assert sidecar['version']==2 and sidecar['scene']=='room.scene.yaml' and sidecar['project']=='.'
    assert len(sidecar['bindings'])==2
    binding=sidecar['bindings'][receipt['assigned_guid']]
    assert set(binding)=={'kind','package','asset','package_digest','source_hash','transform'}
    assert binding==receipt['assigned_binding'] and binding['kind']=='static' and binding['package']=='static-cone-preview'
    assert binding['asset']=='tall.orrmodel'
    assert binding['package_digest']==receipt['installed_package_digest']==entry['digest']
    assert binding['source_hash']==receipt['variant_source_hash']==g.runner.digest(captures/'hidden-package-source/tall.orrmodel')
    assert receipt['variant_asset_id']=='props/tall.cone.json'
    assert binding['transform']=={'translation':[0.,0.,0.],'rotation':[0.,0.,0.,1.],'scale':[1.,1.,1.]}
    scene=g.regular(captures/'room/room.scene.yaml').read_text(encoding='utf-8')
    assert f"  {receipt['assigned_guid']}:\n    RoomEscapeV1::Actor: {{ kind: 2, ordinal: 0 }}\n" in scene, 'assigned saved entity is the actual Room KEY'
    project=g.strict_json(captures/'room/orr.project.json')
    assert project['entry']['game']=='room-escape-v1' and project['entry']['models']=='room.models.json'
    authoritative=g.regular(captures/'collected-authoritative.frame').read_bytes()
    assert len(authoritative)>32 and authoritative==g.regular(captures/'presentation-free.frame').read_bytes(), 'exact presentation-free authoritative frame parity'
    assert g.regular(captures/'actual-gui.orrp').stat().st_size>32
    g.runner.report['cone_ui_capture_inventory']=inventory
    g.runner.report['cone_ui_receipt']=receipt
    g.runner.record()



def check_ui_identity(g, receipt):
    assert set(receipt)==g.UI_FIELDS | EXTRA_FIELDS, 'exact Room KEY acceptance receipt fields'
    assert type(receipt['schema']) is int and receipt['schema']==1 and receipt['status']=='passed'
    assert receipt['test']==UI_TEST and type(receipt['mandatory_test_count']) is int and receipt['mandatory_test_count']==1
    assert receipt['test_source_fnv1a64']==g.fnv1a64((g.source_root/'crates/orr_editor/tests/static_model_cone_package_preview_ui.rs').read_bytes())
    assert receipt['producer_fnv1a64']==g.fnv1a64(retained_producer(g,'cone-cli').read_bytes())
    assert receipt['cylinder_control_producer_fnv1a64']==g.fnv1a64(retained_producer(g,'cylinder-cli').read_bytes())
    retained_producer(g,'cone-editor-test')
    assert receipt['cone_triangle_oracle'] is True
    assert receipt['native_gpu'] is False and receipt['native_os_input'] is False
    assert any(token in receipt['adapter'].lower() for token in ('llvmpipe','lavapipe'))
    assert receipt['renderer_options']=='default' and receipt['predictable_texture_filtering'] is False
    for key in g.UI_TRUE_FIELDS:assert receipt[key] is True,key
    assert receipt['assigned_role']=='KEY' and re.fullmatch('e_[0-9a-f]{8}',receipt['assigned_guid'])
    assert receipt['normalized_viewport_size']==[900,850]
    for key,value in [('key_placements_before_collect',2),('key_placements_after_collect',1),
                      ('key_placements_after_restart',2),('key_collected',1),('restart_key_collected',0)]:
        assert type(receipt[key]) is int and receipt[key]==value,key
    for key in ('recorded_ticks','recorded_checksums_checked'):
        assert type(receipt[key]) is int and 0<receipt[key]<=10000,key
    assert receipt['recorded_checksums_checked']<=receipt['recorded_ticks']+1
    assert re.fullmatch('[0-9a-f]{16}',receipt['authoritative_checksum'])
