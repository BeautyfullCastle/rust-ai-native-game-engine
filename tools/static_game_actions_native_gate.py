#!/usr/bin/env python3
"""Four-action native acceptance; synthetic validator tests are not native evidence.

This private acceptance fixture does not replace either original game gate or
source-hidden/generated-user delivery acceptance.
"""
import json
import re
from pathlib import Path

from static_game_native_gate import ExecutionContext, digest, json_file, require
from static_game_standalone_gate import png

SCHEMA = 'orr.static-game-actions-native/1'


def exact_keys(value, expected, label):
    require(isinstance(value, dict) and set(value) == set(expected), label+' exact fields')


def integer(value, lower, upper, label):
    require(type(value) is int and lower <= value <= upper, label+' integer range')
    return value


def evidence_file(root, name):
    """Reject traversal and every symlink component, not just the final leaf."""
    require(isinstance(name, str) and name and '\\' not in name, 'evidence name')
    relative = Path(name)
    require(not relative.is_absolute() and all(p not in ('..', '.') for p in relative.parts), 'evidence traversal')
    require(relative.as_posix() == name, 'noncanonical evidence name')
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), 'evidence root')
    path = root
    for part in relative.parts:
        path /= part
        require(not path.is_symlink(), 'linked evidence component')
    require(path.is_file(), 'missing evidence')
    return path


def test_inventory(output, expected):
    lines = output.splitlines()
    actual = [line[:-6] for line in lines if line.endswith(': test')]
    require(len(actual) == len(set(actual)) and set(actual) == set(expected), 'exact test inventory')
    require(lines and lines[-1] == f'{len(expected)} tests, 0 benchmarks', 'inventory summary')


def test_result(output, name, filtered):
    require(re.search(r'(?m)^test '+re.escape(name)+r' \.\.\. ok$', output), 'exact native test execution')
    require(f'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; {filtered} filtered out;' in output,
            'native test result (no skip/headless substitution)')


def producers(output, context, expected):
    """Admit only exact Cargo package/name/kind/source and canonical artifacts."""
    result = {}
    for line in output.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get('reason') != 'compiler-artifact' or not item.get('executable'):
            continue
        name = item.get('target', {}).get('name')
        if name not in expected:
            require(item.get('package_id') != context.package_id, 'unexpected fixture executable producer')
            continue
        require(name not in result, 'duplicate executable producer')
        context.admit(item, name, expected[name])
        path = context.artifact_path(item['executable'])
        require(item.get('profile', {}).get('opt_level') == '3', 'release producer profile')
        require(item.get('profile', {}).get('test') is (expected[name] == 'test'), 'producer test profile')
        result[name] = {'package_id': item['package_id'], 'kind': item['target']['kind'],
                        'source': item['target']['src_path'], 'source_sha256': digest(Path(item['target']['src_path'])),
                        'path': str(path), 'sha256': digest(path), 'profile': item['profile']}
    require(set(result) == set(expected), 'exact executable producer inventory')
    return result


NEUTRAL = {'dx': 0, 'dy': 0, 'actions': 0}
INITIAL = {'x': 0, 'y': 0, 'counts': [0, 0, 0, 0], 'held': 0}


def input_value(value):
    exact_keys(value, NEUTRAL, 'typed input')
    integer(value['dx'], -1, 1, 'dx')
    integer(value['dy'], -1, 1, 'dy')
    integer(value['actions'], 0, 15, 'actions')
    return value


def state_value(value):
    exact_keys(value, INITIAL, 'typed state')
    integer(value['x'], -3600, 3600, 'x')
    integer(value['y'], -3600, 3600, 'y')
    require(isinstance(value['counts'], list) and len(value['counts']) == 4, 'four authoritative counters')
    for count in value['counts']:
        integer(count, 0, 3600, 'action counter')
    integer(value['held'], 0, 15, 'held bits')
    return value


def advance(previous, supplied):
    state_value(previous)
    input_value(supplied)
    mask = supplied['actions']
    return {'x': previous['x']+supplied['dx'], 'y': previous['y']+supplied['dy'],
            'counts': [n+int(bool(mask & (1 << i))) for i, n in enumerate(previous['counts'])], 'held': mask}


def validate_ticks(ticks):
    require(isinstance(ticks, list) and 1 < len(ticks) <= 3601, 'bounded complete consumed timeline')
    require(ticks[0]['tick'] == 0 and ticks[0]['state'] == INITIAL, 'initial fixture state')
    for i, tick in enumerate(ticks):
        exact_keys(tick, {'tick', 'state', 'checksum'} | ({'input'} if i else set()), 'tick record')
        integer(tick['tick'], 0, 3600, 'tick')
        require(tick['tick'] == i, 'contiguous consumed ticks')
        state_value(tick['state'])
        require(isinstance(tick['checksum'], str) and re.fullmatch('0x[0-9a-f]{16}', tick['checksum']), 'canonical exact checksum')
        if i:
            require(tick['state'] == advance(ticks[i-1]['state'], tick['input']), 'independent fixture transition')
    return ticks


PROFILE = {'id': 'private-native-actions/v1', 'actions': [
    {'name': 'jump', 'key': 'Space', 'mask': 1},
    {'name': 'fire', 'key': 'F', 'mask': 2},
    {'name': 'interact', 'key': 'E', 'mask': 4},
    {'name': 'shield', 'key': 'Q', 'mask': 8}]}
BOUNDARIES = {'native_window': True, 'server_keyboard_delivery': True,
              'physical_hardware_keyboard': False, 'physical_gpu': False,
              'software_gpu': True, 'packaged_export': False,
              'source_hidden_delivery': False}
STANDALONE_SCENARIOS = ['independent_1', 'independent_2', 'independent_4', 'independent_8',
                        'simultaneous', 'partial_release', 'neutral', 'escape_clear', 'focus_clear']
CAPTURES = ['initial', 'action_1', 'action_2', 'action_4', 'action_8', 'simultaneous', 'partial_release', 'neutral']


def observation_stream(path):
    require(path.stat().st_size <= 32*1024*1024, 'bounded observation file')
    from static_game_standalone_gate import strict_json
    values = [strict_json(line) for line in path.read_text().splitlines()]
    require(1 < len(values) <= 20000, 'bounded observation count')
    return values


def validate_observations(observations, ticks, source_sha):
    consumed = []
    pid = observations[0]['process_id']
    integer(pid, 1, 2**31-1, 'owned process ID')
    for index, observation in enumerate(observations):
        require(observation['source_sha'] == source_sha and observation['process_id'] == pid, 'observation producer binding')
        integer(observation['sequence'], 0, 1000000, 'observation sequence')
        tick = integer(observation['tick'], 0, len(ticks)-1, 'observation tick')
        if index:
            require(observation['sequence'] == observations[index-1]['sequence']+1 and tick >= observations[index-1]['tick'], 'complete ordered observation stream')
        require(observation['state'] == ticks[tick]['state'] and observation['checksum'] == ticks[tick]['checksum'], 'observation differs from source replay')
        adapter = observation['adapter']
        require(adapter['backend'] == 'Vulkan' and adapter['device_type'] == 'Cpu' and 'swiftshader' not in str(adapter).lower(), 'software Vulkan adapter')
        consumed.extend(observation['consumed'])
    require(consumed == ticks[1:], 'actual consumed inputs/state/checksums differ from independent source replay')


def clear_proof(proof, observations, ticks):
    paused, no_auto, resumed, first = (proof[k] for k in ('paused', 'no_auto_resume', 'resumed', 'first_consumed'))
    require(all(v in observations for v in (paused, no_auto, resumed)), 'clear proof outside actual stream')
    require(proof['before']['sequence'] <= paused['sequence'] < no_auto['sequence'] < resumed['sequence'] and proof['after'] == resumed, 'clear proof chronological anchors')
    require(paused['paused'] is True and paused['state']['held'] == 15 and ticks[paused['tick']]['input'] == {'dx': 1, 'dy': 1, 'actions': 15}, 'all four bits and axes held before clear')
    require(no_auto['sequence'] >= paused['sequence']+6, 'bounded no-auto-resume window')
    segment = [v for v in observations if paused['sequence'] <= v['sequence'] <= no_auto['sequence']]
    require(all(v['paused'] is True and v['tick'] == paused['tick'] and v['state'] == paused['state'] for v in segment), 'paused state advanced or auto-resumed')
    require(all(not v['consumed'] for v in segment[1:]), 'paused consumed ticks')
    next_tick = paused['tick']+1
    require(next_tick < len(ticks) and first == ticks[next_tick], 'not first naturally consumed resumed tick')
    require(first['input'] == NEUTRAL and first['state'] == advance(paused['state'], NEUTRAL), 'stale input on first resumed tick')
    require(resumed['paused'] is False and resumed['resume_from_tick'] == paused['tick'] and resumed['neutral_consumed_tick'] == next_tick, 'explicit neutral resume evidence')


def validate_scenarios(scenarios, observations, ticks):
    require([s['name'] for s in scenarios] == STANDALONE_SCENARIOS, 'exact ordered scenario inventory')
    previous_sequence = -1
    for scenario in scenarios:
        name, before, after = (scenario[k] for k in ('name', 'before', 'after'))
        require(before in observations and after in observations and before['sequence'] < after['sequence'], 'scenario actual observation order')
        require(before['sequence'] >= previous_sequence, 'scenario windows overlap or reorder')
        previous_sequence = after['sequence']
        if name.endswith('_clear'):
            clear_proof(scenario, observations, ticks)
            require(scenario['paused']['focused'] is (name == 'escape_clear'), 'actual clear focus boundary')
            if name == 'focus_clear':
                require(scenario['no_auto_resume']['focused'] is True, 'actual focus regain')
            continue
        expected = dict(NEUTRAL)
        if name.startswith('independent_'):
            expected['actions'] = int(name.rsplit('_', 1)[1])
            require(before['tick'] > 0 and ticks[before['tick']]['input'] == NEUTRAL, 'independent case missing consumed neutral baseline')
        elif name in ('simultaneous', 'partial_release'):
            expected.update(dx=1, dy=1, actions=15 if name == 'simultaneous' else 14)
            if name == 'partial_release':
                require(before['state']['held'] == 15, 'partial release precondition')
        segment = ticks[before['tick']+1:after['tick']+1]
        require(segment and any(t['input'] == expected for t in segment), 'required input tuple never consumed')
        if name.startswith('independent_'):
            require(all(t['input'] in (NEUTRAL, expected) for t in segment), 'unrelated input in independent action scenario')
        require(ticks[after['tick']]['input'] == expected, 'scenario final input tuple')


def fixture_pixels(path, capture):
    """Independent known-fixture marker geometry, never producer pixel counts."""
    observation = capture['observation']
    w, h = observation['size']
    pixels = png(path, w, h)
    camera = observation['camera']
    require(camera['center'] == [0, 0] and abs(camera['half_extent']-5.175) < 0.00001, 'known fixture camera')
    mask = state_value(observation['state'])['held']
    for i, wx in enumerate((-3, -1, 1, 3)):
        cx = round(w/2+wx*min(w,h)/(2*5.175))
        cy = round(h/2-2*min(w,h)/(2*5.175))
        require(8 <= cx < w-8 and 8 <= cy < h-8, 'marker pixel bounds')
        colors = [0]*5
        for y in range(cy-8, cy+9):
            for x in range(cx-8, cx+9):
                r, g, b = pixels[(y*w+x)*4:(y*w+x)*4+3]
                matches = (r > 180 and r-g > 70 and r-b > 70,
                           g > 180 and g-r > 70 and g-b > 70,
                           b > 180 and b-r > 70 and b-g > 40,
                           r > 180 and g > 180 and b < 150,
                           max(r,g,b)-min(r,g,b) < 8 and 20 < r < 150)
                colors = [n+int(v) for n,v in zip(colors,matches)]
        expected = i if mask & (1 << i) else 4
        require(colors[expected] > 200 and sum(colors[:4]) == (colors[expected] if expected != 4 else 0), 'independent action marker pixels')


def validate_captures(directory, captures, observations):
    require([c['phase'] for c in captures] == CAPTURES, 'exact ordered capture inventory')
    masks = dict(initial=0, action_1=1, action_2=2, action_4=4, action_8=8, simultaneous=15, partial_release=14, neutral=0)
    for capture in captures:
        before, after, observation = (capture[k] for k in ('before', 'after', 'observation'))
        require(all(v in observations for v in (before,after,observation)), 'capture outside actual observations')
        require(observation == before and before['sequence'] < after['sequence'], 'capture before/after order')
        require(before['tick'] == after['tick'] and before['state'] == after['state'] and before['checksum'] == after['checksum'], 'capture crossed state transition')
        segment = [v for v in observations if before['sequence'] <= v['sequence'] <= after['sequence']]
        require(all(v['paused'] is True and v['tick'] == before['tick'] and v['state'] == before['state'] for v in segment), 'capture not stable paused presentation')
        require(observation['state']['held'] == masks[capture['phase']], 'capture wrong action mask')
        if capture['phase'] == 'initial':
            require(observation['tick'] == 0 and observation['state'] == INITIAL, 'capture is not initial fixture state')
        require(capture['png'] == capture['phase']+'.png', 'capture exact filename')
        path = evidence_file(directory, capture['png'])
        require(digest(path) == capture['sha256'], 'capture hash')
        x11 = capture['x11']
        require([x11['width'], x11['height']] == observation['size'] and x11['depth'] == 24 and x11['bits_per_pixel'] == 32, 'XGetImage framebuffer format')
        fixture_pixels(path, capture)


SCENE = ('schema: orr.scene/1\nsingletons:\n  State: {x: 0, y: 0, jump: 0, fire: 0, interact: 0, shield: 0, held: 0}\n'
         'entities:\n  e_00000001:\n    name: actor\n    Actor: {position: [0, 0]}\n')


def validate_receipt(directory, provenance):
    """Validate standalone evidence against gate-owned compiler/run provenance."""
    directory = Path(directory)
    receipt = json_file(evidence_file(directory, 'receipt.json'))
    require(receipt['schema'] == SCHEMA and receipt['mode'] == 'standalone' and receipt['status'] == 'passed', 'receipt schema/mode/status')
    require(receipt['boundaries'] == BOUNDARIES and all(type(v) is bool for v in receipt['boundaries'].values()) and receipt['packaged'] is False and receipt['source_hidden'] is False, 'honest native acceptance boundaries')
    require(receipt['profile'] == PROFILE and all(type(a['mask']) is int for a in receipt['profile']['actions']), 'exact four-action profile')
    for key, value in provenance.items():
        require(receipt['provenance'].get(key) == value, 'compiled/run provenance '+key)
    scene = receipt['scene']
    require(scene['path'] == 'authored.scene.yaml', 'exact fixture scene path')
    path = evidence_file(directory, scene['path'])
    require(path.read_bytes() == SCENE.encode() and digest(path) == scene['sha256'], 'known fixture scene bytes/hash')
    identity = receipt['identity']
    require(identity['game'] == 'PrivateNativeActions' and identity['build_id'] == '0x4d414354494f4e01', 'private fixture identity')
    require(all(type(identity[k]) is int for k in ('seed','tick_rate','player_count')) and identity['seed'] == 42 and identity['tick_rate'] == 60 and identity['player_count'] == 1, 'fixture deterministic configuration')
    artifacts = receipt['artifacts']
    expected_paths = {'observations':'observations.ndjson', 'replay':'record.orrp', 'source_replay':'source-replay.json'}
    exact_keys(artifacts, expected_paths, 'artifact inventory')
    for key, name in expected_paths.items():
        require(artifacts[key]['path'] == name and digest(evidence_file(directory, name)) == artifacts[key]['sha256'], 'artifact identity/hash '+key)
    raw = evidence_file(directory, 'record.orrp')
    require(raw.stat().st_size <= 16*1024*1024 and raw.read_bytes()[:4] == b'ORRP', 'bounded raw replay')
    ticks = validate_ticks(json_file(evidence_file(directory, 'source-replay.json'))['ticks'])
    replay = receipt['replay']
    require(replay['path'] == 'record.orrp' and replay['sha256'] == artifacts['replay']['sha256'] and replay['source_replay_sha256'] == artifacts['source_replay']['sha256'], 'raw/source replay binding')
    require(replay['ticks'] == len(ticks)-1 and replay['initial_checksum'] == ticks[0]['checksum'] == identity['initial_checksum'] and replay['final_checksum'] == ticks[-1]['checksum'], 'exact replay checksum endpoints')
    observations = observation_stream(evidence_file(directory, 'observations.ndjson'))
    require(all(o['schema'] == 'orr.static-standalone-observation/1' and o['identity'] == identity for o in observations), 'observation schema/identity')
    require(observations[0]['process_id'] == receipt['provenance']['process_id'], 'owned window process')
    validate_observations(observations, ticks, provenance['source_sha'])
    validate_scenarios(receipt['scenarios'], observations, ticks)
    for scenario in receipt['scenarios']:
        validate_events(scenario, observations)
    validate_captures(directory, receipt['captures'], observations)
    return {'mode': 'standalone', 'ticks': len(ticks)-1, 'captures': len(receipt['captures']), **BOUNDARIES}


KEYSYMS = [0x20, 0x66, 0x65, 0x71]


def validate_events(scenario, observations):
    name = scenario['name']
    events = scenario['events']
    require(isinstance(events, list) and events, 'missing XTEST event evidence')
    sequences = {o['sequence'] for o in observations}
    for event in events:
        lower, upper = event['after_sequence'], event['before_sequence']
        integer(lower, 0, 1000000, 'event lower sequence')
        integer(upper, 0, 1000000, 'event upper sequence')
        require(lower in sequences and upper in sequences and lower < upper, 'event must be bracketed by actual observations')
        require(upper <= scenario['after']['sequence'], 'event after scenario outcome')
        if 'keysym' in event:
            exact_keys(event, {'keysym', 'pressed', 'after_sequence', 'before_sequence'}, 'server key event')
            require(type(event['pressed']) is bool and type(event['keysym']) is int, 'typed server key event')
        else:
            exact_keys(event, {'kind', 'after_sequence', 'before_sequence'}, 'server focus event')
    expected = []
    if name.startswith('independent_'):
        mask = int(name.rsplit('_', 1)[1])
        expected = [(KEYSYMS[mask.bit_length()-1], True)]
    elif name in ('simultaneous', 'escape_clear', 'focus_clear'):
        expected = [(k, True) for k in KEYSYMS+[0x64, 0x77]]
        if name == 'escape_clear':
            expected.extend([(0xff1b, True), (0xff1b, False)])
        if name.endswith('_clear'):
            expected.extend((k, False) for k in KEYSYMS+[0x64, 0x77])
            expected.extend([(0xff0d, True), (0xff0d, False)])
    elif name == 'partial_release':
        expected = [(KEYSYMS[0], False)]
    elif name == 'neutral':
        expected = [(k, False) for k in KEYSYMS]
    for event in events:
        lower, upper = event['after_sequence'], event['before_sequence']
        if name.endswith('_clear'):
            if event.get('keysym') == 0xff0d:
                require(scenario['no_auto_resume']['sequence'] <= lower < upper <= scenario['resumed']['sequence'], 'resume event outside explicit resume window')
            elif event.get('pressed') is False and event.get('keysym') in KEYSYMS+[0x64, 0x77]:
                require(scenario['paused']['sequence'] <= lower < upper <= scenario['no_auto_resume']['sequence'], 'release event outside paused window')
            elif event.get('keysym') == 0xff1b or event.get('kind') == 'focus_loss':
                require(scenario['before']['sequence'] <= lower < upper <= scenario['paused']['sequence'], 'clear trigger outside active-to-paused boundary')
            elif event.get('kind') == 'focus_regain':
                require(scenario['paused']['sequence'] <= lower < upper <= scenario['no_auto_resume']['sequence'], 'focus regain outside paused window')
        else:
            require(scenario['before']['sequence'] <= lower < upper <= scenario['after']['sequence'], 'key event outside scenario window')
    actual = [(e['keysym'], e['pressed']) for e in events if 'keysym' in e]
    require(actual == expected, 'exact native key inventory/order')
    focus = [e['kind'] for e in events if 'kind' in e]
    require(focus == (['focus_loss', 'focus_regain'] if name == 'focus_clear' else []), 'exact focus transition event inventory')


TARGETS = {
    'orr_static_starter': ('lib', 'src/lib.rs'),
    'multi_action_standalone_x11': ('test', 'tests/multi_action_standalone_x11.rs'),
    'multi_action_editor_x11': ('test', 'tests/multi_action_editor_x11.rs'),
    'multi-action-fixture-player': ('bin', 'tests/support/multi_action_fixture_player.rs'),
    'delivery-player': ('bin', 'src/bin/delivery-player.rs'),
    'switchboard-player': ('bin', 'src/bin/switchboard-player.rs'),
    'editor': ('bin', 'src/bin/editor.rs'),
    'player': ('bin', 'src/bin/player.rs'),
}
DECODERS = {'x11::tests::'+name for name in ('little_and_big_endian_pixels',
    'rgb565_masks_and_scanline_padding', 'swapped_masks_and_packed24', 'reject_bad_bounds_masks_and_lengths')}
FIXTURE_TESTS = {'fixture::contract_tests::private_scene_and_all_action_states_are_admitted'}


def execution_context(env, execute):
    """The private fixture is engine-only, never a generated-context wildcard."""
    from static_game_native_gate import clean_checkout
    metadata = json.loads(execute(['cargo', 'metadata', '--offline', '--locked', '--no-deps', '--format-version=1'], 'metadata', 30))
    packages = [p for p in metadata['packages'] if p['name'] == 'orr_static_starter']
    require(len(packages) == 1, 'exact fixture package')
    package = packages[0]
    root = Path.cwd().resolve()
    source_root = root/'examples/static-game-starter'
    require(package['manifest_path'] == str(source_root/'Cargo.toml'), 'exact fixture manifest')
    sources = {}
    for name, (kind, relative) in TARGETS.items():
        candidates = [t for t in package['targets'] if t['name'] == name and t['kind'] == [kind]]
        require(len(candidates) == 1 and candidates[0]['src_path'] == str(source_root/relative), 'exact metadata target '+name)
        sources[name] = source_root/relative
    sha = env['GITHUB_SHA']
    return ExecutionContext(root, package['name'], package['id'], sources, sha, root, sha,
                            fence=lambda: clean_checkout(sha, root), target_root=metadata['target_directory'])


def run(root, environment=None):
    import os
    import sys
    from static_game_native_gate import command, clean_checkout
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=True)
    require(not any(root.iterdir()), 'fresh evidence directory required')
    receipt = {'schema': SCHEMA, 'status': 'started', 'boundaries': BOUNDARIES}
    def save():
        (root/'gate.json').write_text(json.dumps(receipt, indent=2)+'\n')
    save()
    try:
        env = dict(os.environ if environment is None else environment)
        sha = env.get('GITHUB_SHA', '')
        require(re.fullmatch('[0-9a-f]{40}', sha) and all(env.get(k, '').isdecimal() for k in ('GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT')), 'hosted source/run identity required')
        require(not env.get('ORR_ACCEPTANCE_CONTEXT') and not env.get('ORR_ACCEPTANCE_CONTEXT_SHA256'), 'private engine fixture does not accept generated context')
        source = clean_checkout(sha)
        (root/'source-inventory.json').write_text(json.dumps(source, indent=2)+'\n')
        receipt.update(source_inventory_sha256=digest(root/'source-inventory.json'), source_tree=source['tree'])
        execute = lambda args, name, seconds: command(args, root, name, env, seconds, cwd=Path.cwd())
        context = execution_context(env, execute)
        context.environment(env)
        env.update(ORR_STATIC_NATIVE_BUILD_SHA=sha, ORR_STATIC_STANDALONE_BUILD_SHA=sha,
                   ORR_MULTI_ACTION_ARTIFACT_DIR=str(root), ORR_REQUIRE_GPU='1', ORR_REQUIRE_NATIVE_EDITOR='1',
                   WGPU_BACKEND='vulkan', WINIT_UNIX_BACKEND='x11', PYTHONDONTWRITEBYTECODE='1')
        env.pop('DISPLAY', None)
        env.pop('ORR_MULTI_EDITOR_CHILD', None)
        env.pop('WAYLAND_DISPLAY', None)
        output = execute([sys.executable, '-B', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_static_game_actions_native*.py', '-v'], 'validator-tests', 120)
        require(re.search(r'Ran [1-9][0-9]* tests? in ', output) and output.rstrip().endswith('OK'), 'validator suite did not execute')
        modes = [('standalone', 'multi-action-standalone-acceptance', 'multi_action_standalone_x11', 'physical_four_action_standalone', 180),
                 ('editor', 'multi-action-editor-acceptance', 'multi_action_editor_x11', 'multi_action_editor_x11', 240)]
        receipt['modes'] = {}
        for mode, feature, target, test, timeout in modes:
            output = execute(['cargo', 'test', '--locked', '--offline', '--release', '-j1', '-p', context.package_name,
                              '--no-default-features', '--features', feature, '--test', target, '--no-run',
                              '--message-format=json-render-diagnostics'], mode+'-compile', 900)
            entry = {'library_producer': context.library_artifact(output)}
            # Cargo auto-discovers this ungated headless binary in both graphs.
            # It is an admitted incidental build product, never native evidence.
            expected = {target: 'test', 'player': 'bin'}
            expected.update({'multi-action-fixture-player': 'bin', 'delivery-player': 'bin', 'switchboard-player': 'bin'} if mode == 'standalone' else {'editor': 'bin'})
            binaries = producers(output, context, expected)
            entry['producers'] = binaries
            receipt['modes'][mode] = entry
            save()
            executable = binaries[target]['path']
            inventory = {test} | DECODERS | FIXTURE_TESTS
            listing = execute([executable, '--list'], mode+'-inventory', 10)
            test_inventory(listing, inventory)
            output = execute([executable, '--skip', test, '--test-threads=1', '--nocapture'], mode+'-helpers', 30)
            require('test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;' in output, 'exact fixture/helper execution')
            for helper in DECODERS | FIXTURE_TESTS:
                require(re.search(r'(?m)^test '+re.escape(helper)+r' \.\.\. ok$', output), 'required helper did not execute')
            output = execute([executable, test, '--exact', '--test-threads=1', '--nocapture'], mode+'-native', timeout)
            test_result(output, test, 5)
            for artifact in binaries.values():
                require(digest(Path(artifact['path'])) == artifact['sha256'], 'compiled producer changed during execution')
            provenance = {'source_sha': sha, 'run_id': env['GITHUB_RUN_ID'], 'run_attempt': env['GITHUB_RUN_ATTEMPT'],
                          'test_sha256': binaries[target]['sha256']}
            if mode == 'standalone':
                provenance['player_sha256'] = binaries['multi-action-fixture-player']['sha256']
                folder = context.project_root/'examples/static-game-starter/tests'
                provenance['producer_sources'] = {'fixture_sha256': digest(folder/'support/multi_action_fixture.rs'),
                                                'player_sha256': digest(folder/'support/multi_action_fixture_player.rs'),
                                                'test_sha256': digest(folder/'multi_action_standalone_x11.rs')}
                entry['validation'] = validate_receipt(root/mode, provenance)
            else:
                folder = context.project_root/'examples/static-game-starter/tests'
                provenance['fixture_source_sha256'] = digest(folder/'support/multi_action_fixture.rs')
                provenance['test_source_sha256'] = digest(folder/'multi_action_editor_x11.rs')
                entry['validation'] = validate_editor_receipt(root/mode, provenance)
            save()
        require(context.fence() == source, 'reviewed source tree changed during acceptance')
        receipt['status'] = 'passed'
    except Exception as error:
        receipt.update(status='failed', error=str(error))
        raise
    finally:
        save()


EDITOR_SCENARIOS = ['independent_1', 'independent_2', 'independent_4', 'independent_8',
    'simultaneous', 'partial_release', 'neutral', 'escape_clear', 'tab_clear',
    'focus_clear', 'claiming_clear', 'modified_key', 'text_focus', 'scene_reset', 'post_reset_fresh_press']
EDITOR_CAPTURES = EDITOR_SCENARIOS[:7] + ['escape_clear', 'tab_clear', 'post_reset_fresh_press']
EDITOR_BOUNDARIES = {'native_window': True, 'xtest_server_events': True, 'physical_keyboard': False,
    'physical_gpu': False, 'source_hidden': False, 'production_editor_app': True, 'raw_input_injection': False,
    'wrapper_lifecycle_api': True, 'scene_generation_is_wrapper_label': True, 'full_tick_state_is_replay_derived': True,
    'packaged_export': False, 'physical_hardware_keyboard': False, 'software_gpu': True}


def editor_pixels(path, capture):
    width, height = capture['x11']['width'], capture['x11']['height']
    pixels = png(path, width, height)
    require(capture['camera'] == {'center': [0, 0], 'half_extent': 6.0}, 'fixed editor camera')
    viewport = capture['viewport']
    require(len(viewport['min']) == len(viewport['px']) == 2, 'editor viewport shape')
    x0, y0 = viewport['min']
    w, h = viewport['px']
    require(0 <= x0 < width and 0 <= y0 < height and 0 < w <= width-x0 and 0 < h <= height-y0, 'editor viewport bounds')
    mask = capture['observation']['state']['held']
    for i, x in enumerate((-3, -1, 1, 3)):
        cx, cy = round(x0+w/2+x*min(w,h)/12), round(y0+h/2-2*min(w,h)/12)
        require(8 <= cx < width-8 and 8 <= cy < height-8, 'editor marker bounds')
        matching = 0
        for yy in range(cy-8, cy+9):
            for xx in range(cx-8, cx+9):
                r, g, b = pixels[(yy*width+xx)*4:(yy*width+xx)*4+3]
                colors = (r > 180 and r-g > 70 and r-b > 70,
                          g > 180 and g-r > 70 and g-b > 70,
                          b > 180 and b-r > 70 and b-g > 40,
                          r > 180 and g > 180 and b < 150,
                          max(r,g,b)-min(r,g,b) < 8 and 20 < r < 150)
                matching += colors[i if mask & (1 << i) else 4]
        require(matching > 200, 'independent editor action marker pixels')


def validate_editor_receipt(directory, provenance):
    directory = Path(directory)
    receipt = json_file(evidence_file(directory, 'receipt.json'))
    require(receipt['schema'] == SCHEMA and receipt['mode'] == 'editor' and receipt['status'] == 'passed' and receipt['game'] == 'private-native-actions', 'editor receipt identity')
    require(receipt['boundaries'] == EDITOR_BOUNDARIES and all(type(v) is bool for v in receipt['boundaries'].values()), 'honest editor proof boundaries')
    require(receipt['input_source'] == 'XTEST_server_events' and receipt['capture_source'] == 'XGetImage_pid_bound_window', 'actual editor IO boundaries')
    aliases = {'source_sha': 'compiled_source_sha', 'test_sha256': 'executable_sha256'}
    for key, value in provenance.items():
        require(receipt['provenance'].get(aliases.get(key, key)) == value, 'editor compiled/run provenance '+key)
    require(receipt['provenance']['git_sha'] == provenance['source_sha'], 'editor gate/source SHA')
    require(receipt['scene']['file'] == 'fixture.scene.yaml', 'exact editor fixture scene')
    scene = evidence_file(directory, 'fixture.scene.yaml')
    require(scene.read_bytes() == SCENE.encode() and digest(scene) == receipt['scene']['sha256'], 'editor fixture scene bytes/hash')
    adapter = receipt['adapter']
    require(adapter['backend'] == 'Vulkan' and adapter['device_type'] == 'Cpu' and 'swiftshader' not in str(adapter).lower(), 'editor software Vulkan')
    integer(receipt['process_id'], 1, 2**31-1, 'editor owned PID')
    integer(receipt['window_id'], 1, 2**32-1, 'editor owned window')
    require(receipt['observations_file'] == 'observations.ndjson', 'editor observation path')
    path = evidence_file(directory, 'observations.ndjson')
    require(digest(path) == receipt['observations_sha256'], 'editor observations hash')
    observations = observation_stream(path)
    replays = receipt['replays']
    require([r['scene_generation'] for r in replays] == [0, 1], 'exact wrapper scene generations')
    timelines = []
    for generation, replay in enumerate(replays):
        require(replay['file'] == f'epoch-{generation}.orrp', 'exact editor replay file')
        raw = evidence_file(directory, replay['file'])
        require(raw.stat().st_size <= 16*1024*1024 and raw.read_bytes()[:4] == b'ORRP' and digest(raw) == replay['sha256'], 'editor actual recording hash')
        require(replay['record_source'] == 'actual_editor_local_host_ORRP' and replay['state_source'] == 'independent_replay_resimulation', 'honest replay-derived complete trace')
        timelines.append(validate_ticks(replay['ticks']))
    for i, observation in enumerate(observations):
        generation = integer(observation['scene_generation'], 0, 1, 'wrapper generation')
        tick = integer(observation['tick'], 0, len(timelines[generation])-1, 'live observed tick')
        actual = timelines[generation][tick]
        require(observation['state'] == actual['state'] and observation['checksum'] == actual['checksum'], 'live snapshot differs from actual host replay')
        require(observation['phase'] in ('Off', 'Claiming', 'Active', 'Releasing') and type(observation['cleanup_pending']) is bool and type(observation['focused']) is bool, 'observed ownership phase')
        for key in observation['raw_keys']:
            require(all(type(key[m]) is bool for m in ('pressed','repeat','ctrl','alt','command')), 'typed native raw key flags')
        if i:
            previous = observations[i-1]
            require(observation['sequence'] == previous['sequence']+1 and generation >= previous['scene_generation'], 'complete ordered live snapshot stream')
            if generation == previous['scene_generation']:
                require(tick >= previous['tick'], 'live tick regression')
    validate_editor_scenarios(receipt, observations, timelines)
    require([c['phase'] for c in receipt['captures']] == EDITOR_CAPTURES, 'exact editor capture inventory')
    for capture in receipt['captures']:
        expected_masks = {'independent_1':1,'independent_2':2,'independent_4':4,'independent_8':8,'simultaneous':15,'partial_release':13,'neutral':0,'escape_clear':0,'tab_clear':0,'post_reset_fresh_press':8}
        require(capture['observation']['state']['held'] == expected_masks[capture['phase']], 'editor capture phase/action mismatch')
        before, after = capture['before'], capture['after']
        require(before in observations and after in observations and capture['observation'] == before, 'editor capture live anchors')
        require(before['sequence'] < after['sequence'] and before['scene_generation'] == after['scene_generation'], 'editor capture temporal bounds')
        segment = [v for v in observations if before['sequence'] <= v['sequence'] <= after['sequence']]
        require(all(v['state']['held'] == before['state']['held'] and v['camera'] == capture['camera'] and v['viewport'] == capture['viewport'] for v in segment), 'editor marker/camera changed during capture')
        ticks = timelines[before['scene_generation']][before['tick']:after['tick']+1]
        require(all(t['state']['held'] == before['state']['held'] for t in ticks), 'unobserved marker transition during capture')
        require(capture['png'] == capture['phase']+'.png', 'editor capture name')
        path = evidence_file(directory, capture['png'])
        require(digest(path) == capture['sha256'], 'editor capture hash')
        require(capture['x11']['depth'] == 24 and capture['x11']['bits_per_pixel'] == 32, 'editor XGetImage format')
        editor_pixels(path, capture)
    return {'mode': 'editor', 'scene_generations': 2, 'captures': len(receipt['captures']), **EDITOR_BOUNDARIES}


def validate_editor_scenarios(receipt, observations, timelines):
    scenarios = receipt['scenarios']
    require([s['name'] for s in scenarios] == EDITOR_SCENARIOS, 'exact editor scenario inventory')
    events = receipt['events']
    require(0 < len(events) <= 1000 and [e['sequence'] for e in events] == list(range(len(events))), 'exact ordered XTEST event stream')
    for event in events:
        integer(event['sequence'], 0, 999, 'server event sequence')
        require(event['operation'] in ('key', 'focus', 'blur', 'take_control_click'), 'unknown server operation')
        expected_keys = {'sequence', 'after_observation', 'operation'}
        expected_keys |= {'keysym', 'pressed'} if event['operation'] == 'key' else ({'x', 'y'} if event['operation'] == 'take_control_click' else set())
        exact_keys(event, expected_keys, 'editor server event')
        require(any(o['sequence'] == event['after_observation'] for o in observations), 'server event lacks actual live anchor')
        if event['operation'] == 'key':
            require(type(event['pressed']) is bool and type(event['keysym']) is int and event['keysym'] in KEYSYMS+[0x64,0x77,0xff1b,0xff09,0xffe3], 'typed exact editor server key')
            require(any(o['sequence'] == event['after_observation'] for o in observations), 'server key lacks live anchor')
    previous_event = 0
    previous_observation = -1
    for scenario in scenarios:
        name, before, after = (scenario[k] for k in ('name', 'before', 'after'))
        require(before in observations and after in observations and before['sequence'] < after['sequence'], 'editor scenario live anchors')
        require(before['sequence'] >= previous_observation, 'editor scenarios reordered or overlapping')
        previous_observation = after['sequence']
        end = integer(scenario['event_end'], previous_event, len(events), 'scenario event cursor')
        current_events = [e for e in events[previous_event:end] if before['sequence'] <= e.get('after_observation', -1) <= after['sequence']]
        previous_event = end
        current = [o for o in observations if before['sequence'] <= o['sequence'] <= after['sequence']]
        def observed(value):
            require(value in current, 'ownership proof outside scenario live window')
            return value
        def neutral_window(start, finish):
            require(start['scene_generation'] == finish['scene_generation'] and start['tick'] < finish['tick'], 'positive neutral consumed window')
            timeline = timelines[start['scene_generation']]
            require(all(t['input'] == NEUTRAL for t in timeline[start['tick']+1:finish['tick']+1]), 'nonneutral replay input inside released/reclaimed interval')
        if name == 'scene_reset':
            reset = observed(scenario['reset'])
            require(scenario['boundary'] == 'supported_stop_save_replay_open_path_play' and scenario['scene_generation_is_wrapper_label'] is True, 'truthful scene API boundary')
            require(before['scene_generation'] == 0 and after['scene_generation'] == reset['scene_generation'] == 1, 'actual wrapper scene generation transition')
            require(before['state']['held'] == 15 and reset['phase'] == 'Off' and after['phase'] == 'Active', 'scene active reset/reclaim phases')
            require(after['tick'] > 0 and after['state'] == INITIAL and all(t['input'] == NEUTRAL for t in timelines[1][1:after['tick']+1]), 'old actions leaked across scene reset')
            continue
        require(before['scene_generation'] == after['scene_generation'], 'unexpected scene generation transition')
        timeline = timelines[before['scene_generation']]
        segment = timeline[before['tick']+1:after['tick']+1]
        require(segment, 'scenario has no actually recorded ticks')
        if name.startswith('independent_') or name in ('simultaneous', 'partial_release', 'neutral', 'post_reset_fresh_press'):
            mask = int(name.rsplit('_', 1)[1]) if name.startswith('independent_') else {'simultaneous':15, 'partial_release':13, 'neutral':0, 'post_reset_fresh_press':8}[name]
            expected = {'dx': int(name in ('simultaneous', 'partial_release')), 'dy': int(name in ('simultaneous', 'partial_release')), 'actions': mask}
            require(segment[-1]['input'] == expected and any(t['input'] == expected for t in segment), 'editor expected input was not consumed')
            if name.startswith('independent_') or name == 'post_reset_fresh_press':
                require(before['state']['held'] == 0 and before['tick'] > 0 and timeline[before['tick']]['input'] == NEUTRAL, 'editor independent neutral baseline')
                require(all(t['input'] in (NEUTRAL, expected) for t in segment), 'editor unrelated independent action')
                index = mask.bit_length()-1
                require(any(e['operation'] == 'key' and e['keysym'] == KEYSYMS[index] and e['pressed'] is True for e in current_events), 'independent physical press missing')
                require(any(k['physical_key'] == ['Space','F','E','Q'][index] and k['pressed'] is True and not any(k[m] for m in ('ctrl','alt','command')) for o in current for k in o['raw_keys']), 'independent unmodified native raw key missing')
            elif name == 'partial_release':
                require(before['state']['held'] == 15 and any(e['operation'] == 'key' and e['keysym'] == 0x66 and e['pressed'] is False for e in current_events), 'editor partial release physical precondition')
            continue
        if name in ('escape_clear', 'tab_clear', 'focus_clear'):
            held = observed(scenario['held_before'])
            off = observed(scenario['off'])
            no_auto = observed(scenario['no_auto_reclaim'])
            reclaimed = observed(scenario['reclaimed'])
            neutral = observed(scenario['neutral_after_reclaim'])
            require(held['state']['held'] == 15 and held['phase'] == 'Active' and timeline[held['tick']]['input'] == {'dx':1,'dy':1,'actions':15}, 'all editor actions and axes held before release')
            require(held['sequence'] < off['sequence'] <= no_auto['sequence'] < reclaimed['sequence'] <= neutral['sequence'], 'editor release/reclaim ordering')
            require(off['phase'] == no_auto['phase'] == 'Off' and not off['cleanup_pending'] and not no_auto['cleanup_pending'], 'editor cleanup did not complete')
            require(no_auto['tick'] >= off['tick']+3 and reclaimed['phase'] == 'Active' and neutral['phase'] == 'Active', 'editor no-auto and explicit reclaim window')
            neutral_window(off, neutral)
            require(any(e['operation'] == 'take_control_click' and no_auto['sequence'] <= e['after_observation'] <= reclaimed['sequence'] for e in current_events), 'explicit native reclaim click missing')
            if name == 'focus_clear':
                require(off['focused'] is False and no_auto['focused'] is True and any(e['operation'] == 'blur' for e in current_events), 'editor real blur/regain boundary')
            else:
                key = 0xff1b if name == 'escape_clear' else 0xff09
                require(any(e['operation'] == 'key' and e['keysym'] == key and e['pressed'] is True for e in current_events), 'editor physical release key missing')
        elif name == 'no_reclaim':
            require(all(o['phase'] == 'Off' and not o['cleanup_pending'] for o in current) and after['tick'] >= before['tick']+3, 'automatic editor reclaim')
            neutral_window(before, after)
        elif name == 'claiming_clear':
            claiming, active = observed(scenario['claiming']), observed(scenario['active'])
            require(before['phase'] == 'Off' and claiming['phase'] == 'Claiming' and active['phase'] == after['phase'] == 'Active', 'real Claiming transition')
            require(before['sequence'] < claiming['sequence'] < active['sequence'] <= after['sequence'], 'Claiming order')
            require(any(k['physical_key'] == 'Space' and k['pressed'] is True and not any(k[m] for m in ('ctrl','alt','command')) for k in claiming['raw_keys']), 'Claiming physical press absent')
            neutral_window(before, after)
        elif name == 'modified_key':
            require(any(k['physical_key'] == 'F' and k['pressed'] is True and k['ctrl'] is True for o in current for k in o['raw_keys']), 'actual modified native key absent')
            neutral_window(before, after)
        elif name == 'text_focus':
            require(isinstance(after['text'], str) and after['text'] and after['phase'] == 'Off', 'actual text widget focus absent')
            require(any(k['physical_key'] in ('Space','F','E','Q') and k['pressed'] is True for o in current for k in o['raw_keys']), 'text physical press absent')
            neutral_window(before, after)


if __name__ == '__main__':
    import argparse
    import sys
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    arguments = parser.parse_args()
    try:
        run(arguments.artifacts)
    except Exception as error:
        print('four-action native gate FAILED: '+str(error), file=sys.stderr)
        sys.exit(1)
