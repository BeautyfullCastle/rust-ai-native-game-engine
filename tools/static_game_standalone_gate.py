#!/usr/bin/env python3
"""Required source-hidden X11 standalone acceptance, never a headless/skip fallback."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import selectors
import stat
import struct
import subprocess
import sys
import time
import zlib

from static_game_native_gate import clean_checkout, digest, json_file, require, engine_context, CONTEXT_TESTS, context_test_output

SCHEMA = 'orr.static-standalone-native/1'
TEST = 'source_hidden_standalone_windows'
DECODERS = {'little_and_big_endian_pixels', 'rgb565_masks_and_scanline_padding',
            'swapped_masks_and_packed24', 'reject_bad_bounds_masks_and_lengths'}
GAMES = {'delivery': ('StarterDelivery', '0x5354415254455201'),
         'switchboard': ('StarterSwitchboard', '0x5354415254455202')}


def strict_json(text):
    def pairs(items):
        value = {}
        for key, item in items:
            require(key not in value, 'duplicate JSON key')
            value[key] = item
        return value
    return json.loads(text, object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))


def stream(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 32*1024*1024,
            'missing/linked/oversize observation stream')
    data = path.read_bytes()
    require(data.endswith(b'\n'), 'unfinished NDJSON')
    values = [strict_json(line) for line in data.splitlines()]
    require(1 < len(values) < 100000, 'observation count')
    return values


def png(path, width, height):
    """Independent bounded PNG decoder, including CRC, scanline filters and EOF."""
    require(0 < width <= 2048 and 0 < height <= 2048, 'PNG dimension bounds')
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 20*1024*1024, 'PNG file')
    data = path.read_bytes()
    require(data[:8] == b'\x89PNG\r\n\x1a\n', 'PNG signature')
    at, compressed, header, ended, idat_ended = 8, bytearray(), False, False, False
    while at < len(data):
        require(at+12 <= len(data) and not ended, 'PNG trailing/truncated chunk')
        size = struct.unpack('>I', data[at:at+4])[0]
        kind, body = data[at+4:at+8], data[at+8:at+8+size]
        require(at+12+size <= len(data), 'PNG truncated body')
        require(zlib.crc32(kind+body) == struct.unpack('>I', data[at+8+size:at+12+size])[0], 'PNG CRC')
        if kind == b'IHDR':
            require(not header and at == 8 and size == 13, 'PNG IHDR')
            require(struct.unpack('>IIBBBBB', body) == (width, height, 8, 6, 0, 0, 0), 'PNG format')
            header = True
        elif kind == b'IDAT':
            require(header and not idat_ended, 'PNG IDAT order')
            compressed.extend(body)
        elif kind == b'IEND':
            require(header and compressed and size == 0, 'PNG IEND')
            ended = True
        else:
            require(kind[0] & 32, 'PNG unknown critical chunk')
            idat_ended |= bool(compressed)
        at += size+12
    require(ended, 'PNG missing IEND')
    stride = width*4
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, (stride+1)*height+1)
    require(len(raw) == (stride+1)*height and decoder.eof and not decoder.unused_data, 'PNG decompression bound/EOF')
    pixels, previous = bytearray(), bytearray(stride)
    for y in range(height):
        mode, row = raw[y*(stride+1)], bytearray(raw[y*(stride+1)+1:(y+1)*(stride+1)])
        require(mode <= 4, 'PNG filter')
        for x in range(stride):
            a, b, c = row[x-4] if x >= 4 else 0, previous[x], previous[x-4] if x >= 4 else 0
            if mode == 0:
                predictor = 0
            elif mode == 1:
                predictor = a
            elif mode == 2:
                predictor = b
            elif mode == 3:
                predictor = (a+b)//2
            else:
                p = a+b-c
                distances = [abs(p-a), abs(p-b), abs(p-c)]
                predictor = [a, b, c][distances.index(min(distances))]
            row[x] = (row[x]+predictor) & 255
        pixels.extend(row)
        previous = row
    return pixels


def expected_camera(alias):
    # Independently derive from known authored fixture positions and shared 0.4
    # circle radii, matching f32 arithmetic without accepting producer geometry.
    f32 = lambda n: struct.unpack('<f', struct.pack('<f', n))[0]
    points = [[2,2],[2,2]] if alias == 'delivery' else [[0,0],[0,2],[2,2],[4,2],[6,2]]
    radius = f32(0.4)
    low = [min(f32(p[i]-radius) for p in points) for i in range(2)]
    high = [max(f32(p[i]+radius) for p in points) for i in range(2)]
    return {'center':[f32(f32(low[i]+high[i])*0.5) for i in range(2)],
            'half_extent':max(f32(max(f32(high[i]-low[i]) for i in range(2))*0.75),5.0)}


def pixel_oracle(path, capture, alias):
    observation = capture['observation']
    w, h = observation['size']
    pixels = png(path, w, h)
    camera = observation['camera']
    expected = expected_camera(alias)
    require(all(abs(a-b) < 0.00001 for a,b in zip(camera['center'],expected['center'])) and abs(camera['half_extent']-expected['half_extent']) < 0.00001, 'unexpected fixture camera')
    def count(world, color):
        x, y = round(w/2+(world[0]-expected['center'][0])*min(w,h)/(2*expected['half_extent'])), round(h/2-(world[1]-expected['center'][1])*min(w,h)/(2*expected['half_extent']))
        require(8 <= x < w-8 and 8 <= y < h-8, 'pixel oracle out of bounds')
        count = 0
        for yy in range(y-8, y+9):
            for xx in range(x-8, x+9):
                r, g, b = pixels[(yy*w+xx)*4:(yy*w+xx)*4+3]
                count += {'blue': b-r > 25 and b-g > 10, 'gold': r > 200 and g > 150 and b < 180,
                          'green': g-r > 40 and g-b > 40, 'gray': max(r,g,b)-min(r,g,b) < 6 and 80 < r < 200}[color]
        return count
    if alias == 'delivery':
        require(count([2, 2], 'gold') > 20, 'native delivery goal pixels missing')
        if capture['phase'] == 'restarted':
            require(count([0, 0], 'blue') > 20, 'native courier restart transition missing')
        else:
            require(count([0, 0], 'blue') == 0, 'courier prematurely at restart origin')
    else:
        require(count([0, 0], 'blue') > 20, 'native switch cursor missing')
        require(count([0, 2], 'green' if capture['phase'] == 'won' else 'gray') > 20, 'native lamp transition missing')


def bundle(directory, receipt, source_sha, expected_binary):
    root = directory/'relocated bundle with spaces'
    manifest = json_file(root/'bundle.json')
    require(manifest['schema'] == 1 and manifest['source_sha'] == source_sha, 'bundle schema/source')
    require(manifest['target'] == 'x86_64-unknown-linux-gnu', 'bundle target')
    require(manifest['runtime_provenance'] == {'source_sha': source_sha}, 'compiled runtime provenance')
    require(manifest['runtime_sha256'] == expected_binary == receipt['runtime_sha256'], 'runtime producer mismatch')
    require(digest(root/'bundle.json') == receipt['bundle_sha256'], 'bundle receipt hash')
    expected = {'bin/standalone': 0o755, 'scene/game.scene.yaml': 0o644, 'run-game': 0o755}
    require([f['path'] for f in manifest['files']] == sorted(expected), 'bundle inventory order/duplicates')
    actual = set()
    for path in root.rglob('*'):
        require(not path.is_symlink(), 'bundle symlink')
        if path.is_dir():
            require(path.relative_to(root).as_posix() in ('bin','scene'), 'extra bundle directory')
        else:
            require(path.is_file() and path.stat().st_nlink == 1, 'bundle special/hardlink')
            actual.add(path.relative_to(root).as_posix())
    require(actual == set(expected)|{'bundle.json'}, 'bundle exact inventory')
    for entry in manifest['files']:
        path = root/entry['path']
        require(entry['mode'] == expected[entry['path']] == stat.S_IMODE(path.stat().st_mode), 'bundle mode')
        require(entry['sha256'] == digest(path) and entry['bytes'] == path.stat().st_size, 'bundle content inventory')
    require(digest(root/'bin/standalone') == expected_binary, 'executed bundle binary hash')
    require(digest(root/'scene/game.scene.yaml') == receipt['scene_sha256'] == digest(directory/'authored.scene.yaml'), 'bundled scene mismatch')
    identity = manifest['identity']
    for field, digest_field in [('schema','scene_descriptor_sha256'),('input_contract','input_contract_sha256')]:
        encoded = json.dumps(identity[field], ensure_ascii=False, separators=(',',':')).encode()
        require(hashlib.sha256(encoded).hexdigest() == manifest[digest_field], 'descriptor digest')
    return identity


def clear_proof(proof, observations, ticks, alias):
    paused, no_auto, resumed, first = (proof[k] for k in ('paused','no_auto_resume','resumed','first_consumed'))
    require(all(v in observations for v in (paused,no_auto,resumed)), 'clear proof not in actual stream')
    require(paused['paused'] and paused['state']['won'] == 0 and paused['state']['held'] == 1, 'nonterminal action-held clear precondition')
    require(no_auto['sequence'] >= paused['sequence']+6, 'no-auto observation window')
    segment = [v for v in observations if paused['sequence'] <= v['sequence'] <= no_auto['sequence']]
    require(all(v['paused'] and v['tick'] == paused['tick'] and v['state'] == paused['state'] for v in segment), 'secret paused tick/auto resume')
    require(all(not v['consumed'] for v in segment[1:]), 'paused consumed ticks')
    expected_tick = paused['tick']+1
    require(first == ticks[expected_tick] and first['tick'] == expected_tick, 'not first natural resumed tick')
    neutral = {'dx':0,'dy':0,'deliver':0} if alias == 'delivery' else {'selection':0,'toggle':0}
    require(first['input'] == neutral and first['state']['held'] == 0, 'resumed stale input')
    require({k:v for k,v in first['state'].items() if k != 'held'} == {k:v for k,v in paused['state'].items() if k != 'held'}, 'neutral tick moved state')
    require(not resumed['paused'] and resumed['resume_from_tick'] == paused['tick'] and resumed['neutral_consumed_tick'] == expected_tick, 'explicit resume evidence')


def validate(root, provenance):
    result = json_file(root/'result.json')
    require(result['schema'] == SCHEMA and result['status'] == 'passed' and result['source_sha'] == provenance['source_sha'], 'native result')
    require([g['alias'] for g in result['games']] == list(GAMES), 'exact two games required')
    for game in result['games']:
        alias = game['alias']
        directory, execution = root/alias, root/alias/'execution'
        receipt = json_file(directory/'receipt.json')
        require(digest(directory/'receipt.json') == game['receipt_sha256'], 'game receipt hash')
        require(receipt['schema'] == SCHEMA and receipt['status'] == 'passed' and receipt['alias'] == alias, 'game status/schema')
        require(receipt['source_sha'] == provenance['source_sha'] and receipt['test_executable_sha256'] == provenance['test_sha256'], 'native producer binding')
        require(receipt['input_source'] == 'XTEST' and receipt['framebuffer_source'] == 'XGetImage', 'real input/framebuffer boundary')
        require(receipt['physical_gpu'] is False and all(receipt[k] is True for k in ('source_hidden','network_disabled','pid_namespace')), 'isolation/physical scope')
        require((execution/'isolation.txt').read_text() == 'source-hidden\n', 'missing in-sandbox preflight')
        require(digest(execution/'mountinfo.txt') == receipt['mountinfo_sha256'] and digest(execution/'process-status.txt') == receipt['process_status_sha256'], 'sandbox kernel evidence hash')
        mounts = {}
        for line in (execution/'mountinfo.txt').read_text().splitlines():
            fields = line.split()
            require(len(fields) >= 10 and '-' in fields, 'mountinfo format')
            target = fields[4].replace('\\040',' ')
            require(target not in mounts, 'stacked sandbox mount')
            mounts[target] = set(fields[5].split(','))
        require('ro' in mounts.get('/bundle with spaces',set()) and 'rw' in mounts.get('/evidence',set()), 'bundle/evidence mount permissions')
        require('ro' in mounts.get('/',set()) and '/proc' in mounts and '/tmp' in mounts, 'sandbox root/private runtime mounts')
        binding = receipt['process_binding']
        require(binding['namespace_pid'] == receipt['process_id'] == binding['namespace_ids'][-1] and binding['host_pid'] > 0 and binding['owned_launcher_pid'] > 0, 'host namespace process binding')
        require(binding['executed_path'] == '/bundle with spaces/bin/standalone' and binding['sha256'] == receipt['runtime_sha256'], 'actual sandbox executable binding')
        require(len(binding['namespace_ids']) >= 2 and binding['interfaces'] == ['lo'], 'PID/network namespace boundary')
        for kind in ('pid','net','mnt'):
            names = binding['namespaces'][kind]
            require(names['host'] != names['sandbox'] and all(re.fullmatch(kind+r':\[[0-9]+\]',names[key]) for key in ('host','sandbox')), 'kernel namespace identity')
        status_ids = re.findall(r'^NSpid:\s+([0-9\s]+)$',(execution/'process-status.txt').read_text(),re.M)
        require(len(status_ids) == 1 and int(status_ids[0].split()[-1]) == receipt['process_id'], 'sandbox exec PID continuity')


        require(len(receipt['hidden_paths']) >= 6 and len(set(receipt['hidden_paths'])) == len(receipt['hidden_paths']), 'hidden path inventory')
        expected_hidden = {provenance['source_root'],provenance['toolchain_root'],provenance['cargo_home'],
                           provenance['binaries']['standalone_x11']['path'],provenance['binaries'][alias+'-player']['path'],
                           str(Path(provenance['binaries'][alias+'-player']['path']).parent.parent),
                           str((directory/'authored.scene.yaml').resolve()),str((directory/'trusted runtime copy').resolve()),
                           str((directory/'relocated bundle with spaces').resolve())}
        expected_hidden.update(provenance.get('hidden_source_roots', []))
        require(expected_hidden <= set(receipt['hidden_paths']), 'missing exact known source/target/toolchain/executable hiding')

        identity = bundle(directory, receipt, provenance['source_sha'], provenance['binaries'][alias+'-player']['sha256'])
        require(identity['game'] == GAMES[alias][0] and identity['build_id'] == GAMES[alias][1], 'concrete game identity')
        require(identity['seed'] == 42 and identity['tick_rate'] == 60 and identity['player_count'] == 1, 'deterministic configuration')
        observations = stream(execution/'observations.ndjson')
        require(digest(execution/'observations.ndjson') == receipt['observations_sha256'], 'observation hash')
        consumed = []
        for index, observation in enumerate(observations):
            require(observation['schema'] == 'orr.static-standalone-observation/1', 'observation schema')
            require(observation['identity'] == identity and observation['source_sha'] == provenance['source_sha'], 'observation identity')
            require(observation['process_id'] == receipt['process_id'] > 0, 'window process identity')
            require(observation['adapter']['backend'] == 'Vulkan' and observation['adapter']['device_type'] == 'Cpu', 'expected software Vulkan adapter')
            require('swiftshader' not in str(observation['adapter']).lower(), 'SwiftShader substitution forbidden')
            if index:
                require(observation['sequence'] == observations[index-1]['sequence']+1 and observation['tick'] >= observations[index-1]['tick'], 'observation sequence')
            consumed.extend(observation['consumed'])
        replay = receipt['replay']
        raw = execution/replay['path']
        require(replay['path'] == 'record.orrp' and raw.read_bytes()[:4] == b'ORRP' and digest(raw) == replay['sha256'], 'raw replay integrity')
        source_path = execution/'source-replay.json'
        require(digest(source_path) == replay['source_replay_sha256'], 'independent source replay hash')
        ticks = json_file(source_path)['ticks']
        require(ticks[0]['tick'] == 0 and len(ticks) == replay['ticks']+1 and 0 < replay['ticks'] <= 3600, 'source replay tick count')
        require([t['tick'] for t in ticks] == list(range(len(ticks))) and consumed == ticks[1:], 'presented consumed ticks differ from raw/source')
        require(ticks[0]['checksum'] == identity['initial_checksum'] == replay['initial_checksum'] and ticks[-1]['checksum'] == replay['final_checksum'], 'initial/final checksum')
        require(all(o['state'] == ticks[o['tick']]['state'] and o['checksum'] == ticks[o['tick']]['checksum'] for o in observations), 'presented state differs from raw/source')
        require([c['phase'] for c in receipt['captures']] == ['initial','won','restarted'], 'capture phases')
        for capture in receipt['captures']:
            observation = capture['observation']
            require(observation in observations and observation['paused'], 'capture not bound to stable actual presentation')
            require(observation['state']['won'] == int(capture['phase'] == 'won'), 'game win/restart state')
            require(capture['png'] == capture['phase']+'.png' and digest(execution/capture['png']) == capture['sha256'], 'capture hash/path')
            require([capture['x11']['width'],capture['x11']['height']] == observation['size'], 'X11/presentation dimensions')
            require(capture['x11']['depth'] == 24 and capture['x11']['bits_per_pixel'] == 32, 'owned Xvfb pixel format')
            pixel_oracle(execution/capture['png'], capture, alias)
        for label in ('focus_clear','escape_clear'):
            clear_proof(receipt[label], observations, ticks, alias)
        require(receipt['focus_clear']['paused']['focused'] is False and receipt['focus_clear']['no_auto_resume']['focused'] is True, 'actual focus loss/regain boundary')
        require(receipt['escape_clear']['paused']['focused'] is True, 'Escape should pause focused window')
    return {'games':list(GAMES),'source_hidden':True,'native_window':True,'physical_gpu':False}


def validate_editor(root, provenance):
    output = json_file(root/'editor-trace.json')
    require(output['schema'] == 'orr.static-standalone-editor-trace/1' and output['source_sha'] == provenance['source_sha'], 'editor trace schema/source')
    require(output['run_id'] == provenance['run_id'] and output['run_attempt'] == provenance['run_attempt'], 'editor trace run binding')
    require(output['status'] == 'passed' and output['native_result_sha256'] == digest(root/'result.json'), 'editor native result binding')
    require(output['native_gui_proof'] is False and [g['alias'] for g in output['games']] == list(GAMES), 'editor verifier scope/two games')
    for item in output['games']:
        directory = root/item['alias']
        receipt = json_file(directory/'receipt.json')
        identity = json_file(directory/'relocated bundle with spaces'/'bundle.json')['identity']
        require(item['identity'] == identity and item['receipt_sha256'] == digest(directory/'receipt.json'), 'editor native receipt/identity binding')
        require(item['scene_sha256'] == receipt['scene_sha256'] and item['replay_sha256'] == receipt['replay']['sha256']
                and item['source_replay_sha256'] == receipt['replay']['source_replay_sha256'], 'editor exact native trace binding')
        require(item['typed_state_match'] is True and item['checked_registered_host'] is True and item['compared_frames'] == item['ticks']+1, 'editor registered host/all typed states')
        require(item['ticks'] == receipt['replay']['ticks'] and item['initial_checksum'] == receipt['replay']['initial_checksum']
                and item['final_checksum'] == receipt['replay']['final_checksum'] and item['all_ticks_match'] is True, 'editor full native tick equality')
    return {'games':list(GAMES),'same_native_trace':True,'editor_trace_sha256':digest(root/'editor-trace.json')}


def command(args, root, name, env, timeout, cwd=None):
    path = root/(name+'.log')
    started = time.monotonic()
    byte_count = 0
    with path.open('xb') as output, selectors.DefaultSelector() as selector:
        process = subprocess.Popen(args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
        selector.register(process.stdout, selectors.EVENT_READ)
        deadline = time.monotonic()+timeout
        def exited_unreaped():
            # Retain the leader PID until all group signaling has completed.
            return os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
        try:
            while True:
                require(time.monotonic() < deadline, name+' timeout')
                for key, _ in selector.select(timeout=0.1):
                    chunk = os.read(key.fileobj.fileno(),65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    require(byte_count+len(chunk) <= 32*1024*1024, name+' output overflow')
                    output.write(chunk)
                    byte_count += len(chunk)
                status = exited_unreaped()
                if status is not None and not selector.get_map():
                    require(status.si_code == os.CLD_EXITED and status.si_status == 0, name+' failed')
                    break
        finally:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            stop = time.monotonic()+5
            while exited_unreaped() is None and time.monotonic() < stop:
                time.sleep(0.05)
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            process.stdout.close()
            output.flush()
            (root/(name+'-timing.json')).write_text(json.dumps({'seconds':time.monotonic()-started,
                'timeout_seconds':timeout,'exit_code':process.returncode,'output_bytes':byte_count})+'\n')
    text = path.read_text()
    print(text, end='', flush=True)
    return text


def test_summary(text, passed, filtered):
    found = re.findall(r'test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', text)
    require(found == [('ok',str(passed),'0','0','0',str(filtered))], 'actual executed test counts')


def run(root, context=None, environment=None):
    root = root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    require(not any(root.iterdir()), 'fresh evidence directory required')
    receipt = {'schema':SCHEMA,'status':'started'}
    def save():
        (root/'gate.json').write_text(json.dumps(receipt, indent=2)+'\n')
    save()
    try:
        env = dict(os.environ if environment is None else environment)
        sha = env.get('GITHUB_SHA','')
        require(re.fullmatch('[0-9a-f]{40}',sha) and all(env.get(k,'').isdecimal() for k in ('GITHUB_RUN_ID','GITHUB_RUN_ATTEMPT')), 'hosted provenance required')
        if context is None:
            clean_checkout(sha)
            require(not env.get('ORR_ACCEPTANCE_CONTEXT') and not env.get('ORR_ACCEPTANCE_CONTEXT_SHA256'), 'unexpected execution context')
            context = engine_context(env)
        context.environment(env)
        if context.context_path:
            receipt['acceptance_context_sha256'] = digest(context.context_path)
        sha = context.source_sha
        source = context.fence()
        execute = lambda args, root, name, env, timeout: command(args, root, name, env, timeout, cwd=context.project_root)
        (root/'source-inventory.json').write_text(json.dumps(source,indent=2)+'\n')
        receipt.update(source_inventory_sha256=digest(root/'source-inventory.json'),source_tree=source['tree'])
        env.update(ORR_STATIC_STANDALONE_BUILD_SHA=sha,ORR_REQUIRE_STANDALONE_NATIVE='1',ORR_STANDALONE_ARTIFACT_DIR=str(root))
        env.pop('DISPLAY',None)
        toolchain = execute(['rustc','--print','sysroot'],root,'toolchain-root',env,15).strip()
        require(Path(toolchain).is_absolute() and Path(toolchain).is_dir(), 'toolchain identity')
        cargo_home = Path(env.get('CARGO_HOME',str(Path.home()/'.cargo'))).resolve()
        require(cargo_home.is_dir(), 'Cargo home identity')
        env.update(ORR_STANDALONE_TOOLCHAIN_ROOT=str(Path(toolchain).resolve()), ORR_STANDALONE_CARGO_HOME=str(cargo_home))
        output = execute([sys.executable,'-m','unittest','discover','-s',str(context.gate_root/'tools'),'-p','test_static_game_standalone_gate.py','-v'],root,'validator-tests',env,60)
        require(re.search(r'Ran [1-9][0-9]* tests? in ',output) and output.rstrip().endswith('OK'), 'validator tests not executed')
        output = execute(['cargo','test','--locked','--offline','--release','-j1','-p',context.package_name,
            '--no-default-features','--features','standalone-native-acceptance','--test','standalone_x11','--no-run',
            '--message-format=json-render-diagnostics'],root,'compile',env,900)
        receipt['library_producer'] = context.library_artifact(output)
        artifacts = {}
        for line in output.splitlines():
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            if value.get('reason') == 'compiler-artifact' and value.get('executable'):
                target = value['target']
                if target['name'] in ('standalone_x11','delivery-player','switchboard-player','bundle'):
                    require(target['name'] not in artifacts, 'duplicate executable producer')
                    context.admit(value, target['name'], 'test' if target['name'] == 'standalone_x11' else 'bin')
                    path = context.artifact_path(value['executable'])
                    artifacts[target['name']] = {'path':str(path),'sha256':digest(path),'source':target['src_path']}
        require(set(artifacts) == {'standalone_x11','delivery-player','switchboard-player','bundle'}, 'exact compiler executable producers required')
        provenance = {'source_root':str(context.project_root),'toolchain_root':str(Path(toolchain).resolve()),'cargo_home':str(cargo_home),'source_sha':sha,'test_sha256':artifacts['standalone_x11']['sha256'],'binaries':artifacts,
                      'run_id':env['GITHUB_RUN_ID'],'run_attempt':env['GITHUB_RUN_ATTEMPT']}
        if context.hidden_roots:
            provenance['hidden_source_roots'] = context.hidden_roots
        receipt['provenance'] = provenance
        save()
        executable = artifacts['standalone_x11']['path']
        listing = execute([executable,'--list'],root,'inventory',env,10)
        names = {line[:-6] for line in listing.splitlines() if line.endswith(': test')}
        require(names == {TEST}|{'x11::tests::'+name for name in DECODERS}|CONTEXT_TESTS and listing.rstrip().endswith('8 tests, 0 benchmarks'), 'exact actual test inventory')
        output = execute([executable,'acceptance_context::tests::','--test-threads=1','--nocapture'],root,'context-tests',env,30)
        context_test_output(output,5)
        output = execute([executable,'x11::tests::','--test-threads=1','--nocapture'],root,'pixel-decoder-tests',env,30)
        test_summary(output,4,4)
        output = execute([executable,TEST,'--exact','--test-threads=1','--nocapture'],root,'native',env,180)
        test_summary(output,1,7)
        require(re.search(r'(?m)^test '+TEST+r' \.\.\. ',output), 'required native test not executed')
        for producer in artifacts.values():
            require(digest(Path(producer['path'])) == producer['sha256'], 'producer changed during execution')
        context.validate_binding(root, standalone=True)
        receipt['validation'] = validate(root,provenance)
        # A separate editor-only CPU verifier consumes these exact raw native
        # recordings. Never enable editor in the delivered player feature graph.
        env['ORR_REQUIRE_STANDALONE_EDITOR_TRACE'] = '1'
        output = execute(['cargo','test','--locked','--offline','--release','-j1','-p',context.package_name,
            '--no-default-features','--features','editor-trace-acceptance','--test','standalone_editor_trace','--no-run',
            '--message-format=json-render-diagnostics'],root,'editor-compile',env,900)
        receipt['editor_library_producer'] = context.library_artifact(output)
        editor_artifacts = []
        for line in output.splitlines():
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            if value.get('reason') == 'compiler-artifact' and value.get('executable') and value.get('target',{}).get('name') == 'standalone_editor_trace':
                context.admit(value, 'standalone_editor_trace', 'test')
                editor_artifacts.append(context.artifact_path(value['executable']))
        require(len(editor_artifacts) == 1, 'exact editor verifier producer required')
        editor_executable = editor_artifacts[0]
        editor_hash = digest(editor_executable)
        listing = execute([str(editor_executable),'--list'],root,'editor-inventory',env,10)
        names = {line[:-6] for line in listing.splitlines() if line.endswith(': test')}
        require(names == {'native_recordings_match_editor_route'}|CONTEXT_TESTS and listing.rstrip().endswith('4 tests, 0 benchmarks'), 'exact editor verifier inventory')
        output = execute([str(editor_executable),'acceptance_context::tests::','--test-threads=1','--nocapture'],root,'editor-context-tests',env,30)
        context_test_output(output,1)
        output = execute([str(editor_executable),'native_recordings_match_editor_route','--exact','--test-threads=1','--nocapture'],root,'editor-trace',env,180)
        test_summary(output,1,3)
        require(re.search(r'(?m)^test native_recordings_match_editor_route \.\.\. ',output), 'editor trace test did not execute')
        require(digest(editor_executable) == editor_hash, 'editor verifier changed during execution')
        receipt['editor_verifier'] = {'path':str(editor_executable),'sha256':editor_hash,'validation':validate_editor(root,provenance)}
        for producer in artifacts.values():
            require(digest(Path(producer['path'])) == producer['sha256'], 'editor verification rebuilt a delivered producer')

        context.validate_binding(root, standalone=True)
        require(context.fence() == source, 'reviewed tree changed during acceptance')
        receipt['status'] = 'passed'
    except Exception as error:
        receipt.update(status='failed',error=str(error))
        raise
    finally:
        save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts',type=Path,required=True)
    args = parser.parse_args()
    try:
        run(args.artifacts)
    except Exception as error:
        print('standalone native gate FAILED: '+str(error),file=sys.stderr)
        sys.exit(1)
