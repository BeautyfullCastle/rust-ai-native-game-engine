#!/usr/bin/env python3
"""Mandatory #282 acceptance: fresh producers, source-hidden consumers and pixels.

Fixture validation is not native execution. The independent #279 gate is neither
replaced nor fed this receipt. No OS-window-input or physical-GPU claim is made.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import stat
import sys

import arena_editor_export_gate as legacy

runner, fence, close, guide = legacy.runner, legacy.fence, legacy.close, legacy.guide
NATIVE = 'actual_widget_saved_korean_ui_exports_relocates_and_runs'
MODULE = 'relocated_ui_production_consumer'
REGRESSION = 'ui_consumer::korean_mask_oracle_rejects_nonadjacent_repeats'
APP = legacy.UI_APP
STAGES = ('project-only',) + legacy.STAGES + ('isolated-ui-controls', 'isolated-app-restart')
DIAGNOSTIC = 'error: saved project declares UI but this host does not support it'
EXPORT_DIAGNOSTIC = 'trusted runtime smoke failed (exit status: 1): ' + DIAGNOSTIC
CONSUMER_PROOF = 'consumer-source-hidden=true consumer-runtime-hidden=true consumer-producer-hidden=true consumer-bundle-read-only=true\n'
CONSUMER_SCRIPT = 'test ! -e "$1/Cargo.toml" && test ! -e "$2/orr.project.json" && test ! -e "$3" && test ! -e "$4" || exit 91; if touch "$5/.ui-consumer-write-probe" 2>/dev/null; then rm -f "$5/.ui-consumer-write-probe"; exit 92; fi; printf \'consumer-source-hidden=true consumer-runtime-hidden=true consumer-producer-hidden=true consumer-bundle-read-only=true\\n\'; shift 5; exec "$@"'
SPECS = (
    ('ui-runtime', 'orr_sample', 'arena', 'bin', 'project,game-ui', ['game-ui','input-actions','project','sprites']),
    ('project-runtime', 'orr_sample', 'arena', 'bin', 'project', ['project','sprites']),
    ('ui-harness', 'orr_editor', 'arena_editor_ui_export', 'test', 'arena-project-export,project-create,project-ui', ['arena-project-export','project-create','project-ui','sprites']),
    ('app-harness', 'orr_sample', 'orr_sample', 'lib', 'project,game-ui', ['game-ui','input-actions','project','sprites']),
)
CAPTURES = ('initial.png','moved.png','ui-negative-no-overlay.png','ui-negative-no-text.png','ui-glyphs-before.png','ui-glyphs-after.png','ui-title.png','ui-playing.png','ui-menu.png','ui-continued.png','ui-restart-action.png')
FIXTURE_COUNT = 30  # Set to exact independent fixture census below, never native passes.


def producer_fence():
    for entry in runner.report['producers'].values():
        for path in (Path(entry['producer']['executable']), runner.evidence / entry['file']):
            assert path.is_file() and path == path.resolve() and not path.is_symlink()
            assert stat.S_ISREG(path.stat().st_mode) and os.access(path, os.X_OK)
            assert path.stat().st_size == entry['bytes'] and runner.digest(path) == entry['sha256'], 'producer source/copy changed'


def gpu_fence():
    if 'gpu_environment' not in runner.report:
        return
    expected = runner.report['gpu_environment']
    assert set(expected) == {'VK_ICD_FILENAMES','VK_DRIVER_FILES','WGPU_BACKEND','ORR_REQUIRE_GPU','XDG_RUNTIME_DIR'}
    assert all(os.environ.get(key) == value for key,value in expected.items())
    assert expected['VK_ICD_FILENAMES'] == expected['VK_DRIVER_FILES']
    assert expected['WGPU_BACKEND'] == 'vulkan' and expected['ORR_REQUIRE_GPU'] == '1'
    assert runner.digest(Path(expected['VK_ICD_FILENAMES'])) == runner.report['icd_sha256']
    runtime = Path(expected['XDG_RUNTIME_DIR'])
    assert runtime == runner.evidence / 'xdg-runtime' and runtime.is_dir() and not runtime.is_symlink()
    assert stat.S_IMODE(runtime.stat().st_mode) == 0o700 and runtime.stat().st_uid == os.geteuid()


def run(label, command, seconds=1200):
    gpu_fence()
    producer_fence()
    result = legacy.run(label, command, seconds)
    gpu_fence()
    producer_fence()
    return result


def select_target(spec):
    """Finite UI-only selector; each producer gets an independently empty root."""
    rows = runner.report.setdefault('ui_target_selections', [])
    assert len(rows) < len(SPECS) and spec == SPECS[len(rows)], 'finite producer order/identity'
    if rows:
        runner.target_dir = runner.new_target('cpu')
        os.environ['CARGO_TARGET_DIR'] = str(runner.target_dir)
    else:
        private = runner.report['private_target']
        assert runner.target_dir == Path(private['path'])
        assert (runner.target_dir.stat().st_dev, runner.target_dir.stat().st_ino) == (private['device'], private['inode'])
        assert private['fresh_empty'] is True
    target = runner.target_dir
    assert target.parent == runner.repo / 'target' and target == target.resolve() and not target.is_symlink()
    assert os.environ['CARGO_TARGET_DIR'] == str(target) and not list(target.iterdir())
    assert all(row['target'] != str(target) for row in rows)
    rows.append({'label':spec[0], 'package':spec[1], 'target_name':spec[2], 'kind':spec[3],
                 'features':spec[5], 'profile':{'opt_level':'3','test':spec[3] != 'bin'},
                 'target':str(target), 'fresh_empty':True, 'fresh_artifact_required':True})
    runner.record()


def pin(spec, output):
    label, package, target, kind, requested, features = spec
    assert label not in runner.report['producers'], 'duplicate producer'
    binary = Path(legacy.producer(output, label, package, target, kind, features))
    directory = runner.evidence / 'producers' / label
    directory.mkdir(parents=True)
    destination = directory / binary.name
    binary.rename(destination)
    runner.report['producers'][label]['file'] = destination.relative_to(runner.evidence).as_posix()
    producer_fence()
    runner.record()
    return str(destination)


def producer_path(label):
    return runner.evidence / runner.report['producers'][label]['file']


def bound(label):
    return {'path':str(producer_path(label)), 'sha256':runner.report['producers'][label]['sha256']}


def read(path):
    return legacy.read_json(path)


def text(path, raw_newlines=False):
    assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 8*1024**2
    with path.open(encoding="utf-8", newline="" if raw_newlines else None) as stream:
        return stream.read()


def mounts(root, app=False):
    runtime, harness = producer_path('ui-runtime'), producer_path('ui-harness')
    result = ['--ro-bind','/','/','--bind',str(root),str(root),'--ro-bind',str(root/'bundle'),str(root/'bundle'),
              '--tmpfs',str(runner.repo),'--tmpfs',str(root/'project')]
    for parent in (runtime.parent, harness.parent) + ((producer_path('app-harness').parent,) if app else ()):
        assert not root.is_relative_to(parent) and not parent.is_relative_to(root)
        result += ['--tmpfs',str(parent)]
    return result


def consumer_environment(root, label):
    return {'ORR_UI_CONSUMER_BUNDLE':str(root/'bundle'), 'ORR_UI_CONSUMER_EVIDENCE':str(root),
            'ORR_UI_CONSUMER_SOURCE_PROJECT':str(root/'project'), 'ORR_UI_CONSUMER_SOURCE_ROOT':str(runner.repo),
            'ORR_UI_CONSUMER_SOURCE_RUNTIME':str(producer_path('ui-runtime')),
            'ORR_UI_CONSUMER_SOURCE_PRODUCER':str(producer_path(label))}


def expected_command(root, stage):
    runtime = producer_path('ui-runtime')
    if stage == 'project-only':
        return str(producer_path('project-runtime')), ['--project',str(root/'project'),'--headless','--ticks','0'], {}, 1
    if stage == 'source-move-fire':
        return str(runtime), ['--project',str(root/'project'),'--headless','--ticks','3','--hold','right,fire'], {}, 0
    hidden = [str(runner.repo/'Cargo.toml'),str(root/'project/orr.project.json'),str(runtime)]
    if stage == 'isolated-proof':
        return 'bwrap', mounts(root)+['--','/bin/sh','-c',legacy.PROBE_SCRIPT,'sh']+hidden+[str(root/'bundle')], {}, 0
    if stage in ('isolated-ui-controls','isolated-app-restart'):
        app = stage == 'isolated-app-restart'
        label, copied, test = ('app-harness','app-ui-consumer',APP) if app else ('ui-harness','ui-consumer',MODULE)
        args = mounts(root,app)+['--','/bin/sh','-c',CONSUMER_SCRIPT,'sh',str(runner.repo),str(root/'project'),str(runtime),str(producer_path(label)),str(root/'bundle'),str(root/copied),'--ignored','--exact',test,'--nocapture','--test-threads=1']
        return 'bwrap', args, consumer_environment(root,label), 0
    assert stage in legacy.STAGES[2:]
    ticks = '0' if stage == 'capture-initial' else '3'
    hold = 'idle' if stage == 'capture-initial' else 'right' if stage in ('relocated-move','capture-moved') else 'right,fire'
    args = mounts(root)+['--','/bin/sh','-c',legacy.HIDE_SCRIPT,'sh']+hidden+[str(root/'bundle/run-arena'),'--headless','--ticks',ticks,'--hold',hold]
    if stage.startswith('capture-'):
        args += ['--capture',str(root/('initial.png' if stage == 'capture-initial' else 'moved.png'))]
    return 'bwrap', args, {}, 0


def verify_commands(root, receipt):
    commands = [json.loads(line) for line in text(root/'commands.jsonl').splitlines()]
    assert [row['stage'] for row in commands] == list(STAGES) == receipt['commands']
    for row in commands:
        stage = row['stage']
        program, args, environment, status = expected_command(root,stage)
        assert row['program'] == program and row['args'] == args and row['cwd'] == str(root)
        assert row['environment'] == environment
        assert row['gpu_environment'] == runner.report['gpu_environment']
        assert all(isinstance(value,str) and len(value.encode()) <= 4096 for value in row['gpu_environment'].values())
        assert type(row['status']) is int and row['status'] == status
        assert type(row['expected_status']) is int and row['expected_status'] == status
        assert read(root/(stage+'.command-state.json')) == dict(row,state='completed',error=None)
        for stream in ('stdout','stderr'):
            assert row[stream+'_file'] == stage+'.'+stream+'.txt'
            path = root/row[stream+'_file']; text(path)
            assert runner.digest(path) == row[stream+'_sha256']
    assert text(root/'project-only.stdout.txt') == ''
    # Rust's raw smoke error preserves the runtime's single eprintln newline.
    # Permit only that terminal LF, never generic whitespace or extra output.
    assert text(root/'project-only.stderr.txt', raw_newlines=True) in (DIAGNOSTIC, DIAGNOSTIC + '\n')
    export_error = text(root/'project-only-export-rejection.txt', raw_newlines=True)
    assert export_error in (EXPORT_DIAGNOSTIC, EXPORT_DIAGNOSTIC + '\n')
    negative = read(root/'project-only-runtime.json')
    assert negative == dict(bound('project-runtime'),status=1,stdout='project-only.stdout.txt',stderr='project-only.stderr.txt',export_error=export_error,published=False)
    assert type(negative['status']) is int and negative['published'] is False
    assert not (root/'project-only-rejected').exists()
    assert text(root/'isolated-proof.stdout.txt') == 'source-hidden=true bundle-read-only=true write-denied=true\n'
    assert not (root/'bundle/.write-probe').exists() and not (root/'bundle/.ui-consumer-write-probe').exists()
    for stage, name, total in [('isolated-ui-controls',MODULE,3),('isolated-app-restart',APP,runner.report['app_inventory_total'])]:
        output = text(root/(stage+'.stdout.txt'))
        assert output.startswith(CONSUMER_PROOF)
        guide.result(output[len(CONSUMER_PROOF):].lstrip('\n'),[name],total)


def software_adapter(name, software):
    assert isinstance(name,str) and re.search(r'llvmpipe|lavapipe',name,re.I) and not re.search('swiftshader',name,re.I)
    assert software is True


def verify_closure(root, receipt):
    assert receipt['source_files_before'] == receipt['source_files_after'] == receipt['source_files'] == legacy.files(root/'project')
    assert read(root/'source-files-before.json') == receipt['source_files_before']
    assert receipt['bundle_files'] == legacy.files(root/'bundle')
    manifest = read(root/'bundle/orr.export.json'); payload = manifest['payload']
    assert manifest['schema'] == 1 and payload['profile'] == 'arena-authored-linux-x86_64-v1' and payload['entry']['game'] == 'arena'
    canonical = json.dumps(payload,ensure_ascii=False,separators=(',',':')).encode()
    assert manifest['content_digest'] == hashlib.sha256(b'orrery.arena.export.content.v1\0'+canonical).hexdigest()
    actual = {entry['path']:entry for entry in receipt['bundle_files']}
    listed = payload['files']
    assert len({entry['path'] for entry in listed}) == len(listed)
    assert set(actual) == {entry['path'] for entry in listed} | {'orr.export.json'}
    for entry in listed:
        assert actual[entry['path']] == {key:entry[key] for key in ('path','bytes','sha256')}
        assert stat.S_IMODE((root/'bundle'/entry['path']).stat().st_mode) == entry['mode']
    runtime = runner.report['producers']['ui-runtime']
    assert actual['bin/arena']['sha256'] == runtime['sha256'] and actual['bin/arena']['bytes'] == runtime['bytes']
    assert payload['runtime']['sha256'] == runtime['sha256'] and payload['runtime']['bytes'] == runtime['bytes']
    source = read(root/'project/orr.project.json')
    descriptor = {'profile':'arena-korean-v1','font':{'package':'korean-game-ui','asset':'OrreryKoreanUI.otf'}}
    assert source['entry']['ui'] == descriptor and payload['entry'] == source['entry']
    lock = read(root/'project/orr.packages.lock.json')
    assert payload['packages'] == {name:item['digest'] for name,item in lock['packages'].items()}
    closure = {'orr.project.json','orr.packages.lock.json',source['entry']['scene'],source['entry']['sprites']}
    for name, package in lock['packages'].items():
        assert re.fullmatch('[0-9a-f]{64}',package['digest'])
        prefix = '.orr/packages/objects/' + package['digest'] + '/'
        closure.update(prefix + filename for filename in set(package['files']) | {'orr.package.json'})
        folder = root/'bundle/project/.orr/packages/objects'/package['digest']
        package_manifest = read(folder/'orr.package.json')
        assert package_manifest['name'] == name and package_manifest == package['manifest']
        ordered = {key:package_manifest[key] for key in ('schema','name','version','engine','capabilities','dependencies','files')}
        ordered['capabilities'] = sorted(ordered['capabilities']); ordered['files'] = sorted(ordered['files'])
        ordered['dependencies'] = dict(sorted(ordered['dependencies'].items()))
        identity = json.dumps(['orr-package-v1',ordered,dict(sorted(package['files'].items()))],ensure_ascii=False,separators=(',',':')).encode()
        assert hashlib.sha256(identity).hexdigest() == package['digest'], 'whole package canonical identity'
        assert set(package_manifest['files']) == set(package['files'])
        assert {entry['path'] for entry in legacy.files(folder)} == set(package['files']) | {'orr.package.json'}
        for filename, digest in package['files'].items():
            assert runner.digest(folder/filename) == digest
    assert set(actual) == {'bin/arena','run-arena','orr.export.json'} | {'project/'+name for name in closure}
    roles = {'bin/arena':'runtime','run-arena':'launcher','project/orr.project.json':'project_manifest',
             'project/orr.packages.lock.json':'package_lock','project/'+source['entry']['scene']:'scene',
             'project/'+source['entry']['sprites']:'sprite_sidecar'}
    for name in closure:
        if name.startswith('.orr/packages/objects/'):
            roles['project/'+name] = 'package_manifest' if name.endswith('/orr.package.json') else 'package_asset'
    for entry in listed:
        assert entry['role'] == roles[entry['path']]
        assert entry['mode'] == (0o755 if entry['path'] in ('bin/arena','run-arena') else 0o644)
    source_files = {entry['path']:entry for entry in receipt['source_files']}
    for name in closure:
        assert actual['project/'+name] == dict(source_files[name],path='project/'+name), 'complete active source closure byte identity'
    font = root/'bundle/project/.orr/packages/objects'/payload['packages']['korean-game-ui']
    assert {entry['path'] for entry in legacy.files(font)} == {'orr.package.json','OrreryKoreanUI.otf','OFL.txt','COPYRIGHT.txt','corpus.txt','font-manifest.json'}
    assert (font/'OrreryKoreanUI.otf').stat().st_size > 0
    return manifest, font


def verify_simulation(root, manifest):
    outputs = {stage:text(root/(stage+'.stdout.txt')) for stage in legacy.STAGES if stage != 'isolated-proof'}
    parsed = {stage:legacy.simulation_output(output) for stage,output in outputs.items()}
    assert outputs['source-move-fire'] == outputs['relocated-fire-1'] == outputs['relocated-fire-2']
    assert parsed['relocated-move'][1] != parsed['source-move-fire'][1]
    assert {pair[0] for pair in parsed.values()} == {manifest['payload']['initial_checksum']}
    assert parsed['capture-initial'][1] == parsed['capture-initial'][0]
    assert parsed['capture-moved'] == parsed['relocated-move']
    for stage in ('capture-initial','capture-moved'):
        adapters = [line for line in outputs[stage].splitlines() if line.startswith('project capture adapter:')]
        assert len(adapters) == 1 and re.search(r'software\s*:\s*true',adapters[0],re.I)
        software_adapter(adapters[0],True)


def verify_app(root, manifest):
    original, copied = producer_path('app-harness'), root/'app-ui-consumer'
    assert read(root/'app-consumer-producer.json') == {'source_head':runner.report['head'],'source_tree':runner.report['tree'],
        'original':str(original),'copied':str(copied),'sha256':runner.report['producers']['app-harness']['sha256'],'test':APP}
    receipt = read(root/'app-restart-receipt.json')
    assert receipt['bundle'] == str(root/'bundle') and receipt['initial_checksum'] == manifest['payload']['initial_checksum']
    assert receipt['actual_widget_dispatch'] is True and type(receipt['restart_tick']) is int and receipt['restart_tick'] == 0
    assert receipt['original_entities_positions_players_restored'] is True
    checksums = receipt['replayed_checksums']
    assert len(checksums) == 12 and all(type(x) is int and 0 <= x < 2**64 for x in checksums)
    assert checksums[-1] != int(receipt['initial_checksum'],16)


def changed_mask(before, after, region):
    x, y, width, height = region
    assert all(type(v) is int for v in region) and 0 <= x < x+width <= 512 and 0 <= y < y+height <= 512
    assert len(before) == len(after) == 512*512*4
    return bytes(before[(yy*512+xx)*4:(yy*512+xx)*4+4] != after[(yy*512+xx)*4:(yy*512+xx)*4+4]
                 for yy in range(y,y+height) for xx in range(x,x+width))


def glyph_oracle(before, after):
    masks = [changed_mask(before,after,(10+i*42,100,40,40)) for i in range(5)]
    counts = [sum(mask) for mask in masks]
    assert all(count > 40 for count in counts), 'five nonempty Korean glyph regions required'
    assert all(a != b for i,a in enumerate(masks) for b in masks[i+1:]), 'repeated tofu masks rejected'
    offset = (300*512+300)*4
    assert before[offset:offset+4] == after[offset:offset+4], 'outside pixel changed'
    return counts


def text_oracle(before, after, region):
    assert len(region) == 4
    x0,y0,x1,y1 = region
    mask = changed_mask(before,after,(x0,y0,x1-x0,y1-y0))
    bright = sum(before[(y*512+x)*4:(y*512+x)*4+4] != after[(y*512+x)*4:(y*512+x)*4+4]
                 and all(c > 100 for c in after[(y*512+x)*4:(y*512+x)*4+3])
                 for y in range(y0,y1) for x in range(x0,x1))
    assert sum(mask) > 100 and bright > 30, 'actual Korean HUD text-only delta required'
    return sum(mask),bright


def verify_text(root, pixels, font):
    t = read(root/'ui-text-oracle.json')
    assert t['positive'] == 'ui-playing.png' and t['control'] == 'ui-negative-no-text.png' and t['no_overlay'] == 'ui-negative-no-overlay.png'
    assert t['width'] == t['height'] == 512
    for key in ('positive','control','no_overlay'):
        assert t[key+'_sha256'] == runner.digest(root/t[key])
    assert t['region_text'] == '오러리 투기장' and t['suppressed_text_shapes'] == 1
    assert t['control_method'] == 'clone exact output and override only production HUD TextShape glyph color'
    assert t['control_text_color_rgba'] == [0,0,0,0] and t['control_against_itself'] is False
    assert t['matching_layout'] is True and t['positive_buttons'] == t['control_buttons']
    assert t['positive_buttons'] and [b['action'] for b in t['positive_buttons']] == ['Menu']
    assert (t['minimum_changed_exclusive'],t['minimum_bright_exclusive'],t['bright_channel_threshold_exclusive']) == (100,30,100)
    control, positive = pixels[t['control']],pixels[t['positive']]
    assert control != pixels[t['no_overlay']], 'textless control retains visible nontext overlay'
    assert (t['changed_pixels'],t['bright_different_pixels']) == text_oracle(control,positive,t['region_xyxy'])
    # Identical same-layout missing-text input cannot satisfy this very oracle.
    try: text_oracle(control,control,t['region_xyxy'])
    except AssertionError: pass
    else: raise AssertionError('no-text negative accepted')
    for name in ('ui-continued.png','ui-restart-action.png'):
        text_oracle(control,pixels[name],t['region_xyxy'])
    assert pixels['ui-title.png'] != pixels['ui-negative-no-overlay.png'] and pixels['ui-menu.png'] != positive
    g = read(root/'ui-glyph-oracle.json')
    assert g['before'] == 'ui-glyphs-before.png' and g['after'] == 'ui-glyphs-after.png'
    assert g['width'] == g['height'] == 512 and g['glyphs'] == ['오','러','리','메','뉴']
    assert g['regions_xywh'] == [[10+i*42,100,40,40] for i in range(5)]
    assert g['minimum_changed_exclusive'] == 40 and g['all_masks_pairwise_distinct'] is True and g['pair_comparisons'] == 10
    assert g['unchanged_pixel_xy'] == [300,300] and g['font_source'] == 'relocated bundle admission'
    for key in ('before','after'):
        assert g[key+'_sha256'] == runner.digest(root/g[key])
    assert g['descriptor'] == {'profile':'arena-korean-v1','font':{'package':'korean-game-ui','asset':'OrreryKoreanUI.otf'}}
    assert g['font_sha256'] == runner.digest(font/'OrreryKoreanUI.otf') and g['font_bytes'] == (font/'OrreryKoreanUI.otf').stat().st_size
    assert pixels[g['before']] == pixels['ui-negative-no-overlay.png'], 'same scene glyph control'
    assert g['changed_pixels'] == glyph_oracle(pixels[g['before']],pixels[g['after']])
    try: glyph_oracle(pixels[g['before']],pixels[g['before']])
    except AssertionError: pass
    else: raise AssertionError('blank glyph control accepted')


def verify_receipt(root):
    producer_fence()
    receipt = read(root/'receipt.json')
    assert receipt['schema'] == 1 and receipt['source_head'] == runner.report['head'] and receipt['source_tree'] == runner.report['tree']
    assert receipt['runtime'] == bound('ui-runtime') and receipt['test_producer'] == bound('ui-harness')
    for key in ('terminal_joined_success','source_hidden','bundle_read_only','source_preserved'):
        assert receipt[key] is True
    assert read(root/'producer-receipt.json') == {key:receipt[key] for key in ('source_head','source_tree','runtime','test_producer')}
    assert read(root/'stage-outcomes.json') == {'schema':1,'source_head':receipt['source_head'],'source_tree':receipt['source_tree'],'stage':'complete','terminal_joined_success':True}
    verify_commands(root,receipt)
    manifest,font = verify_closure(root,receipt)
    assert runner.report['producers']['ui-runtime']['sha256'] != runner.report['producers']['project-runtime']['sha256']
    assert not (root/'exported with spaces').exists()
    joined = f"Actual export widget reached terminal joined success for captured scene {root/'project/arena.scene.yaml'} at checksum {manifest['payload']['initial_checksum']}\n"
    assert text(root/'terminal-joined-success.txt') == joined
    verify_simulation(root,manifest)
    verify_app(root,manifest)
    for label, name in [('ui-harness','ui-consumer'),('app-harness','app-ui-consumer')]:
        path = root/name; original = runner.report['producers'][label]
        assert path.is_file() and not path.is_symlink() and os.access(path,os.X_OK)
        assert path.stat().st_size == original['bytes'] and runner.digest(path) == original['sha256']
    adapter = read(root/'ui-consumer-adapter.json'); software_adapter(adapter['name'],adapter['software'])
    captures = receipt['captures']
    assert [entry['path'] for entry in captures] == list(CAPTURES)
    pixels = {}
    for entry in captures:
        path = root/entry['path']
        assert entry['width'] == entry['height'] == 512 and not path.is_symlink()
        assert runner.digest(path) == entry['sha256']
        pixels[entry['path']] = fence.png_pixels(path,512,512)
    assert pixels['initial.png'] != pixels['moved.png']
    legacy.sprite_oracle(root/'project',pixels['initial.png'])
    verify_text(root,pixels,font)
    acceptance = read(root/'ui-acceptance.json')
    assert acceptance['source_head'] == receipt['source_head'] and acceptance['source_tree'] == receipt['source_tree']
    assert all(acceptance[key] is True for key in ('font_closure','actual_app_dispatch','actual_app_restart'))
    assert acceptance['independent_no_ui_runtime_sha256'] == runner.report['producers']['project-runtime']['sha256']
    assert acceptance['consumer_sha256'] == runner.report['producers']['ui-harness']['sha256']
    assert acceptance['app_consumer_sha256'] == runner.report['producers']['app-harness']['sha256']
    runner.report['ui_acceptance_receipt'] = receipt
    runner.record()


def native():
    assert sys.platform == 'linux' and platform.machine() == 'x86_64' and os.geteuid() != 0
    run('bwrap-preflight',['bwrap','--ro-bind','/','/','--','/bin/true'])
    runner.software()
    binaries = {}
    for spec in SPECS:
        select_target(spec)
        label, package, target, kind, requested, features = spec
        cmd = ['cargo','build' if kind == 'bin' else 'test','--locked','--release','-p',package,'--no-default-features','--features',requested]
        cmd += ['--lib'] if kind == 'lib' else ['--'+kind,target]
        if kind != 'bin': cmd += ['--no-run']
        binaries[label] = pin(spec,run(label+'-build',cmd+['--message-format=json']))
    binary = binaries['ui-harness']
    close.inventory(run('ui-full-list',[binary,'--list']),[NATIVE,MODULE,REGRESSION])
    close.inventory(run('ui-full-ignored',[binary,'--ignored','--list']),[NATIVE,MODULE])
    close.inventory(run('ui-regression-ignored',[binary,REGRESSION,'--exact','--ignored','--list']),[])
    guide.result(run('ui-regression-run',[binary,REGRESSION,'--exact','--test-threads=1','--nocapture','--color=never']),[REGRESSION],3)
    app = binaries['app-harness']
    names = legacy.inventory(run('app-full-list',[app,'--list']))
    ignored = legacy.inventory(run('app-full-ignored',[app,'--ignored','--list']))
    assert APP in names and ignored == [APP]
    close.inventory(run('app-selected-list',[app,APP,'--exact','--list']),[APP])
    close.inventory(run('app-selected-ignored',[app,APP,'--exact','--ignored','--list']),[APP])
    runner.report['app_inventory_total'] = len(names); runner.record()
    root = runner.evidence/'acceptance'; assert not root.exists()
    os.environ.update(ORR_ARENA_UI_EXPORT_RUNTIME=binaries['ui-runtime'],ORR_ARENA_UI_PROJECT_ONLY_RUNTIME=binaries['project-runtime'],
                      ORR_ARENA_UI_APP_TEST_PRODUCER=app,ORR_ARENA_UI_EXPORT_EVIDENCE_DIR=str(root))
    guide.result(run('ui-native-run',[binary,NATIVE,'--ignored','--exact','--test-threads=1','--nocapture','--color=never']),[NATIVE],3)
    verify_receipt(root)
    runner.report['coverage']['native-ui'] = {'names':[NATIVE,MODULE,APP,REGRESSION],'passed':4,'failed':0,'ignored':0,
        'scope':'exported runtime gameplay/captures; isolated production-module widgets/pixels; private App restart, not OS-window input'}
    runner.record()


def main():
    assert sys.argv[1:] == ['native']
    error = None
    try:
        root = Path(__file__).resolve().parents[1]
        destination = Path(os.environ['ORR_ARENA_UI_EXPORT_GATE_EVIDENCE'])
        assert destination.is_absolute() and destination == destination.resolve() and not destination.is_relative_to(root)
        assert os.environ.get('PYTHONDONTWRITEBYTECODE') == '1'
        runner.initialize(root,destination,'cpu')
        fence.source_root = root; fence.frozen = None
        assert not run('clean-source-before',['git','status','--porcelain=v1','--untracked-files=all']).strip()
        runner.report['head'] = run('checkout-head',['git','rev-parse','HEAD']).strip()
        runner.report['tree'] = run('checkout-tree',['git','rev-parse','HEAD^{tree}']).strip()
        assert os.environ.get('GITHUB_SHA',runner.report['head']) == runner.report['head']
        paths = run('tracked-source',['git','ls-files','-z']).split('\0'); assert paths.pop() == '' and paths
        fence.check_source_inventory(paths,'before')
        runner.frozen = runner.source_snapshot(paths); fence.frozen = fence.snapshot()
        runner.report.update(source_sha256=runner.frozen,coverage={},mode='native-ui',legacy_census_proof=legacy.structural_proof(),ui_census_proof=legacy.ui_structural_proof())
        runner.record()
        output = run('independent-ui-fixtures',[sys.executable,'-B',str(root/'tools/test_arena_editor_ui_export_gate.py')])
        assert output.rstrip().endswith(f'ARENA_UI_EXPORT_GATE_FIXTURES: {FIXTURE_COUNT} passed; 0 failed; 0 skipped')
        native(); producer_fence(); gpu_fence(); fence.source_fence()
    except BaseException as failure:
        error = failure
    finally:
        if runner.evidence is not None and runner.evidence.is_dir(): runner.finish(error)
        elif error is not None: raise error


if __name__ == '__main__':
    main()
