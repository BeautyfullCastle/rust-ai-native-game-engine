#!/usr/bin/env python3
"""Required Linux native-window acceptance. No skipped/headless substitute is success."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import struct
import subprocess
import sys
import zlib

SCHEMA = 'orr.static-game-native/1'
GAMES = {'delivery': ('StarterDelivery', '0x5354415254455201'),
         'switchboard': ('StarterSwitchboard', '0x5354415254455202')}
PHASES = ['edit', 'won', 'neutral', 'reopened']
TEST = 'registered_games_native_window'
MANAGED = {'version': 1, 'lease_ms': 2000, 'heartbeat_ms': 500}


def require(value, reason):
    if not value:
        raise ValueError(reason)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def json_file(path):
    require(path.is_file() and not path.is_symlink(), f'missing/linked {path}')
    require(path.stat().st_size <= 4 * 1024 * 1024, f'oversize {path}')
    def pairs(items):
        out = {}
        for key, value in items:
            require(key not in out, f'duplicate JSON key {key}')
            out[key] = value
        return out
    return json.loads(path.read_text(), object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))


def png(path):
    """Decode bounded RGBA8 PNG independently of the Rust producer; verify every CRC."""
    require(path.is_file() and not path.is_symlink(), 'missing/linked PNG')
    require(path.stat().st_size <= 16 * 1024 * 1024, 'oversize PNG')
    data = path.read_bytes()
    require(data[:8] == b'\x89PNG\r\n\x1a\n', 'PNG signature')
    at, compressed, header, ended = 8, bytearray(), None, False
    while at < len(data):
        require(at + 12 <= len(data) and not ended, 'PNG chunk/trailing data')
        size = struct.unpack('>I', data[at:at+4])[0]
        kind = data[at+4:at+8]
        body = data[at+8:at+8+size]
        require(at+12+size <= len(data), 'truncated PNG')
        crc = struct.unpack('>I', data[at+8+size:at+12+size])[0]
        require(zlib.crc32(kind+body) == crc, 'PNG CRC')
        if kind == b'IHDR':
            require(header is None and at == 8 and size == 13, 'PNG IHDR')
            header = struct.unpack('>IIBBBBB', body)
            require(header == (1400, 1000, 8, 6, 0, 0, 0), 'PNG dimensions/format')
        elif kind == b'IDAT':
            require(header is not None, 'PNG IDAT order')
            compressed.extend(body)
        elif kind == b'IEND':
            require(size == 0, 'PNG IEND')
            ended = True
        else:
            require(kind[0] & 32, 'unsupported critical PNG chunk')
        at += 12 + size
    require(ended and header is not None, 'unfinished PNG')
    stride, height = 1400 * 4, 1000
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, (stride+1)*height+1)
    require(len(raw) == (stride+1)*height and decoder.eof and not decoder.unused_data,
            'PNG decompressed size/stream')
    pixels = bytearray(stride*height)
    previous = bytearray(stride)
    for y in range(height):
        pos = y*(stride+1)
        mode = raw[pos]
        require(mode <= 4, 'PNG filter')
        row = bytearray(raw[pos+1:pos+1+stride])
        if mode:
            for x in range(stride):
                a, b, c = (row[x-4] if x >= 4 else 0), previous[x], (previous[x-4] if x >= 4 else 0)
                if mode == 1:
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
        pixels[y*stride:(y+1)*stride] = row
        previous = row
    return pixels


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def screen(capture, world):
    camera, viewport = capture['camera'], capture['viewport']
    w, h = viewport['px']
    half = camera['half_extent']
    require(0 < w <= 1400 and 0 < h <= 1000 and 0.001 <= half <= 100, 'camera bounds')
    ppu = f32(f32(min(w, h)*0.5)/half)
    center = camera['center']
    x = f32(f32(w*0.5)+f32(f32(world[0]-center[0])*ppu))
    y = f32(f32(h*0.5)-f32(f32(world[1]-center[1])*ppu))
    return f32(x+viewport['min'][0]), f32(y+viewport['min'][1])


def pixel_counts(pixels, capture, alias):
    def color(x, y, kind):
        r, g, b = pixels[(y*1400+x)*4:(y*1400+x)*4+3]
        return {'blue': b-r > 25 and b-g > 10,
                'green': g-r > 40 and g-b > 40,
                'gold': r > 200 and g > 150 and b < 180}[kind]
    def count(world, kind):
        x, y = map(int, screen(capture, world))
        require(8 <= x < 1392 and 8 <= y < 992, 'oracle outside framebuffer')
        return sum(color(xx, yy, kind) for yy in range(y-8, y+9) for xx in range(x-8, x+9))
    if alias == 'delivery':
        result = {'courier_origin_blue': count([2, 2], 'blue'), 'goal_gold': count([3, 3], 'gold')}
    else:
        result = {'cursor_origin_blue': count([2, 0], 'blue'), 'target_lamp_green': count([4, 2], 'green')}
    # Independent circular selection oracle. Float-boundary count may differ by one pixel
    # from Rust; require the actual ring, rather than trusting producer boolean/count.
    result['selection_outline'] = 0
    if capture['phase'] == 'edit':
        world = [2, 2] if alias == 'delivery' else [2, 0]
        x, y = screen(capture, world)
        edge, _ = screen(capture, [f32(world[0]+0.4), world[1]])
        radius = abs(edge-x)
        bound = math.ceil(radius)+5
        require(bound < 500, 'selection radius bound')
        for yy in range(max(0, int(y)-bound), min(999, int(y)+bound)+1):
            for xx in range(max(0, int(x)-bound), min(1399, int(x)+bound)+1):
                if abs(math.hypot(xx-x, yy-y)-radius) < 5:
                    r, g, b = pixels[(yy*1400+xx)*4:(yy*1400+xx)*4+3]
                    result['selection_outline'] += r > 220 and g > 180 and b < 180
        require(result['selection_outline'] > 10, 'missing selection outline')
    phase = capture['phase']
    if alias == 'delivery':
        require(result['goal_gold'] > 20, 'missing delivery goal')
        if phase in ('edit', 'reopened'):
            require(result['courier_origin_blue'] > 20, 'missing courier')
        if phase == 'won':
            require(result['courier_origin_blue'] == 0, 'courier did not move')
    else:
        if phase in ('edit', 'reopened'):
            require(result['cursor_origin_blue'] > 20 and result['target_lamp_green'] == 0, 'switch edit pixels')
        if phase == 'won':
            require(result['target_lamp_green'] > 20, 'switch win pixels')
    for key, value in result.items():
        reported = capture['pixel_counts'][key]
        # Ring uses sqrt in Rust f32. Interior square counts use exact f32 transforms.
        require(abs(reported-value) <= (2 if key == 'selection_outline' else 0), 'pixel count mismatch '+key)
    return result


def held(typed, alias):
    return typed['state']['action_held'] if alias == 'delivery' else typed['actor']['held']


def terminal(typed, alias):
    return typed['state']['won'] if alias == 'delivery' else typed['state']['solved']


def validate(root, provenance):
    root = Path(root)
    attempt = json_file(root/'attempt.json')
    require(attempt['schema'] == SCHEMA and attempt['status'] == 'passed', 'native attempt failed/missing')
    require(attempt['provenance'] == provenance, 'attempt provenance')
    require(attempt['required_games'] == list(GAMES), 'required games')
    expected_path = root/'expected-contracts.json'
    expected = json_file(expected_path)
    expected_hash = digest(expected_path)
    require(expected['schema'] == SCHEMA and set(expected['games']) == set(GAMES), 'expected games')
    require(attempt['expected_contracts_sha256'] == expected_hash, 'attempt contract binding')
    require(expected['games']['delivery']['identity']['schema'] != expected['games']['switchboard']['identity']['schema'], 'two distinct scene schemas')
    require(expected['games']['delivery']['input_contract']['schema'] != expected['games']['switchboard']['input_contract']['schema'], 'two distinct input schemas')
    for alias, (game_id, build) in GAMES.items():
        directory = root/alias
        require(directory.is_dir() and not directory.is_symlink(), 'missing/linked game directory')
        receipt = json_file(directory/'native-receipt.json')
        contract = expected['games'][alias]
        require(receipt['schema'] == SCHEMA and receipt['status'] == 'passed' and receipt['game'] == alias, 'game receipt status')
        require(receipt['provenance'] == provenance, 'game provenance')
        require(receipt['source'] == 'editor.app_framebuffer' and receipt['input_source'] == 'queued_egui_raw_input', 'native source')
        require(receipt['expected_contracts_sha256'] == expected_hash, 'contract hash')
        require(receipt['identity'] == contract['identity'] and receipt['input_contract'] == contract['input_contract'], 'full compiled contracts')
        require(contract['identity']['game'] == game_id and contract['identity']['build_id'] == build, 'compiled game/build')
        require(contract['identity']['schema'] and contract['input_contract']['schema'] and contract['input_contract']['value_format'], 'empty descriptor')
        require(contract['input_contract']['max_players'] == 1 and contract['managed_held'] == MANAGED, 'managed contract')
        neutral = {'dx': 0, 'dy': 0, 'deliver': 0} if alias == 'delivery' else {'selection': 0, 'toggle': 0}
        require(contract['neutral'] == neutral, 'neutral contract')
        require(receipt['boundaries'] == {'native_window': True, 'physical_keyboard': False, 'physical_gpu': False, 'packaged_export': False}, 'claim boundaries')
        adapter = receipt['adapter']
        desc = ' '.join(str(adapter[k]) for k in ('name', 'driver', 'driver_info')).lower()
        require(adapter['backend'] == 'Vulkan' and adapter['device_type'] == 'Cpu' and any(x in desc for x in ('mesa', 'llvmpipe', 'lavapipe')) and 'swiftshader' not in desc, 'required Mesa native backend')
        checks = ('viewport_pick', 'take_control_ui', 'keyboard_win', 'space_restart', 'escape_neutral', 'focus_neutral', 'no_auto_reclaim', 'focus_regain_no_auto_reclaim', 'reopen_identity')
        require(all(receipt['checks'].get(k) is True for k in checks), 'missing native assertion')
        require(receipt['scene']['path'] == 'authored.scene.yaml', 'scene path')
        scene = directory/'authored.scene.yaml'
        require(scene.is_file() and not scene.is_symlink() and digest(scene) == receipt['scene']['sha256'], 'scene hash')
        captures = receipt['captures']
        require([c['phase'] for c in captures] == PHASES, 'exact capture phases')
        frames = [c['ui_frame'] for c in captures]
        require(all(isinstance(f, int) and f > 0 for f in frames) and frames == sorted(set(frames)), 'capture frame ordering')
        for capture in captures:
            require(capture['png'] == capture['phase']+'.png', 'PNG path')
            path = directory/capture['png']
            require(digest(path) == capture['sha256'], 'PNG hash')
            require(capture['state']['playing'] is False, 'capture not paused')
            if capture['phase'] == 'won':
                require(terminal(capture['typed'], alias) == 1, 'typed win absent')
            if capture['phase'] in ('edit', 'reopened'):
                require(terminal(capture['typed'], alias) == 0, 'edit/reopen terminal')
            pixel_counts(png(path), capture, alias)
        require(captures[0]['typed'] == captures[3]['typed'], 'reopened authored state')
        require('e_00000001' in captures[0]['selected_guids'], 'viewport pick GUID')
        require(terminal(receipt['restart_proof'], alias) == 0, 'Space restart state')
        for label in ('escape_neutral', 'focus_neutral'):
            proof = receipt['release_proofs'][label]
            before, after = proof['before'], proof['after']
            require(held(proof['before_release'], alias) == 1 and terminal(proof['before_release'], alias) == 0, 'release must start held/nonterminal')
            require(proof['neutral_ticks'] == 1 and proof['phase'] == 'Off' and proof['cleanup_pending'] is False, 'release handshake')
            require(before['actor']['position'] == after['actor']['position'] and held(after, alias) == 0, 'stuck held input')
            if alias == 'switchboard':
                require(before['state']['moves'] == after['state']['moves'], 'stuck toggle')
        for label in ('escape', 'focus'):
            proof = receipt['no_reclaim_proofs'][label]
            require(proof['observed_ui_frames'] >= 5 and proof['phase'] == 'Off' and proof['cleanup_pending'] is False, 'regain grant')
            require(proof['before']['actor']['position'] == proof['after']['actor']['position'] and held(proof['after'], alias) == 0, 'auto input after release')
    return {'games': list(GAMES), 'captures': 8, 'status': 'passed'}


def test_output(text):
    require(not re.search(r'\bskip(?:ped)?\b', text, re.I), 'skip marker in native output')
    require(re.search(r'(?m)^test '+TEST+r' \.\.\. ', text), 'required test did not execute')
    summaries = re.findall(r'test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', text)
    require(summaries == [('ok', '1', '0', '0', '0', '0')], 'exactly one executed native test required')


def command(args, root, name, env, timeout):
    process = subprocess.Popen(args, env=env, text=True, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, start_new_session=True)
    timed_out = False
    try:
        output, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            output, _ = process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            output, _ = process.communicate()
    (root/(name+'.log')).write_text(output)
    print(output, end='', flush=True)
    require(not timed_out, f'{name} timed out after {timeout}s; process group terminated')
    require(process.returncode == 0, f'{name} exit {process.returncode}')
    return output


def clean_checkout(sha, cwd=None):
    directory = Path(cwd or '.')
    actual = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=directory, text=True).strip()
    require(actual == sha, 'checkout does not match hosted SHA')
    for args in (['git', 'diff', '--quiet', '--'], ['git', 'diff', '--cached', '--quiet', '--']):
        require(subprocess.run(args, cwd=directory, check=False).returncode == 0, 'tracked source/index is dirty')
    # Compare actual bytes to every reviewed Git blob, independently of index
    # assume-unchanged/stat-cache flags. This inventory also makes the fence auditable.
    tree = subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=directory, text=True).strip()
    entries = subprocess.check_output(['git', 'ls-tree', '-rz', '--full-tree', 'HEAD'], cwd=directory)
    files = []
    for entry in entries.split(b'\0'):
        if not entry:
            continue
        header, name = entry.split(b'\t', 1)
        mode, kind, object_id = header.decode().split()
        path = name.decode()
        require(kind == 'blob' and mode in ('100644', '100755'), 'unsupported source link/submodule '+path)
        file = directory/path
        require(file.is_file() and not file.is_symlink(), 'missing/linked tracked source '+path)
        data = file.read_bytes()
        actual_blob = hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()
        require(actual_blob == object_id, 'tracked source bytes differ from reviewed tree '+path)
        files.append({'path': path, 'mode': mode, 'git_blob': object_id,
                      'sha256': hashlib.sha256(data).hexdigest()})
    # Ignore fresh output directories, NOT new compiler inputs hidden by .gitignore.
    # These source roots contain Rust modules, manifests, shaders and included assets.
    for flags in (['--others', '--exclude-standard'], ['--others', '--ignored', '--exclude-standard']):
        unknown = subprocess.check_output(['git', 'ls-files', '-z', *flags], cwd=directory)
        for raw in unknown.split(b'\0'):
            if not raw:
                continue
            path = raw.decode()
            compiler_input = path.startswith(('crates/', 'examples/', '.cargo/'))
            compiler_input |= path in ('Cargo.toml', 'Cargo.lock', 'build.rs')
            compiler_input |= path.startswith('tools/') and '__pycache__/' not in path
            require(not compiler_input, 'untracked compiler/gate input '+path)
    return {'commit': sha, 'tree': tree, 'files': files}


def run(root):
    root = root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    require(not any(root.iterdir()), 'artifact directory must be fresh, never reuse stale evidence')
    receipt = {'schema': SCHEMA, 'status': 'started'}
    def save():
        (root/'gate.json').write_text(json.dumps(receipt, indent=2)+'\n')
    save()
    try:
        env = dict(os.environ)
        sha, run_id, attempt = (env.get(k, '') for k in ('GITHUB_SHA', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT'))
        require(re.fullmatch('[0-9a-f]{40}', sha) and run_id.isdecimal() and attempt.isdecimal(), 'hosted source/run identity required')
        source = clean_checkout(sha)
        source_path = root/'source-inventory.json'
        source_path.write_text(json.dumps(source, indent=2)+'\n')
        receipt['source_inventory_sha256'] = digest(source_path)
        receipt['source_tree'] = source['tree']
        save()
        env.update(ORR_STATIC_NATIVE_BUILD_SHA=sha, ORR_NATIVE_STATIC_GAME_ARTIFACT_DIR=str(root),
                   ORR_REQUIRE_NATIVE_EDITOR='1', ORR_REQUIRE_GPU='1', WGPU_BACKEND='vulkan')
        command([sys.executable, '-m', 'unittest', 'discover', '-s', 'tools', '-p',
                 'test_static_game_native_gate.py', '-v'], root, 'validator-tests', env, 120)
        output = command(['cargo', 'test', '--locked', '--offline', '--release', '-j1', '-p', 'orr_static_starter',
                          '--features', 'native-acceptance', '--test', 'native_window', '--no-run',
                          '--message-format=json-render-diagnostics'], root, 'compile', env, 900)
        artifacts = []
        for line in output.splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                target = item.get('target', {})
                if target.get('name') == 'native_window' and target.get('kind') == ['test']:
                    require(target.get('src_path', '').endswith('/examples/static-game-starter/tests/native_window.rs'), 'wrong native test target')
                    artifacts.append(Path(item['executable']).resolve())
        require(len(artifacts) == 1, 'exactly one compiled native test binary required')
        executable = artifacts[0]
        provenance = {'compiled_source_sha': sha, 'git_sha': sha, 'run_id': run_id,
                      'run_attempt': attempt, 'executable_sha256': digest(executable)}
        receipt['provenance'] = provenance
        receipt['executable'] = str(executable)
        save()
        listing = command([str(executable), '--list'], root, 'inventory', env, 30)
        require(listing.strip() == f'{TEST}: test\n\n1 test, 0 benchmarks', 'native inventory must contain exactly the selected test')
        output = command(['xvfb-run', '-a', '-s', '-screen 0 1400x1000x24', str(executable),
                          TEST, '--exact', '--test-threads=1', '--nocapture'], root, 'native', env, 180)
        test_output(output)
        require(digest(executable) == provenance['executable_sha256'], 'test executable changed during execution')
        receipt['validation'] = validate(root, provenance)
        require(clean_checkout(sha) == source, 'source tree changed during native execution')
        receipt['status'] = 'passed'
    except Exception as error:
        receipt['status'] = 'failed'
        receipt['error'] = str(error)
        raise
    finally:
        save()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', required=True, type=Path)
    args = parser.parse_args()
    try:
        run(args.artifacts)
    except Exception as error:
        print(f'native gate FAILED: {error}', file=sys.stderr)
        sys.exit(1)
