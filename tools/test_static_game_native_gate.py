"""Synthetic validator tests, never native/GPU acceptance evidence."""
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zlib
import math
import subprocess
from unittest import mock

import static_game_native_gate as gate

PROVENANCE = dict(compiled_source_sha='a'*40, git_sha='a'*40, run_id='1', run_attempt='1', executable_sha256='b'*64)


def write(path, value):
    path.write_text(json.dumps(value))


def rgba_png(path, pixels):
    def chunk(kind, data):
        return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data))
    raw = b''.join(b'\0'+pixels[y*5600:(y+1)*5600] for y in range(1000))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1400, 1000, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(raw, 1))+chunk(b'IEND', b''))


def typed(alias, won=0, held=0):
    if alias == 'delivery':
        return {'actor': {'position': [2, 2]}, 'state': {'goal': [3, 3], 'won': won, 'action_held': held}}
    return {'actor': {'position': [2, 0], 'selected': 1, 'held': held}, 'state': {'bits': 4 if won else 0, 'target': 4, 'solved': won, 'moves': 1 if won else 0}}


def fixture(root):
    contracts = {'schema': gate.SCHEMA, 'games': {}}
    for alias, (game_id, build) in gate.GAMES.items():
        contracts['games'][alias] = {
            'identity': {'game': game_id, 'build_id': build, 'schema': {'name': game_id}},
            'input_contract': {'schema': {'name': alias+'Input'}, 'value_format': 'synthetic fixture format', 'max_players': 1},
            'neutral': {'dx': 0, 'dy': 0, 'deliver': 0} if alias == 'delivery' else {'selection': 0, 'toggle': 0},
            'managed_held': gate.MANAGED}
    write(root/'expected-contracts.json', contracts)
    expected_hash = gate.digest(root/'expected-contracts.json')
    write(root/'attempt.json', {'schema': gate.SCHEMA, 'status': 'passed', 'provenance': PROVENANCE,
                               'required_games': list(gate.GAMES), 'expected_contracts_sha256': expected_hash})
    for alias in gate.GAMES:
        directory = root/alias
        directory.mkdir()
        (directory/'authored.scene.yaml').write_text('# synthetic, not runnable evidence\n')
        contract = contracts['games'][alias]
        receipt = {'schema': gate.SCHEMA, 'status': 'passed', 'game': alias, 'provenance': PROVENANCE,
                   'source': 'editor.app_framebuffer', 'input_source': 'queued_egui_raw_input',
                   'expected_contracts_sha256': expected_hash, 'identity': contract['identity'], 'input_contract': contract['input_contract'],
                   'boundaries': {'native_window': True, 'physical_keyboard': False, 'physical_gpu': False, 'packaged_export': False},
                   'adapter': {'backend': 'Vulkan', 'device_type': 'Cpu', 'name': 'llvmpipe', 'driver': 'Mesa', 'driver_info': 'synthetic'},
                   'checks': {key: True for key in ('viewport_pick', 'take_control_ui', 'keyboard_win', 'space_restart', 'escape_neutral', 'focus_neutral', 'no_auto_reclaim', 'focus_regain_no_auto_reclaim', 'reopen_identity')},
                   'scene': {'path': 'authored.scene.yaml', 'sha256': gate.digest(directory/'authored.scene.yaml')},
                   'captures': [], 'restart_proof': typed(alias), 'release_proofs': {}, 'no_reclaim_proofs': {}}
        for label in ('escape_neutral', 'focus_neutral'):
            receipt['release_proofs'][label] = {'before_release': typed(alias, held=1), 'before': typed(alias), 'after': typed(alias), 'neutral_ticks': 1, 'phase': 'Off', 'cleanup_pending': False}
        for label in ('escape', 'focus'):
            receipt['no_reclaim_proofs'][label] = {'before': typed(alias), 'after': typed(alias), 'observed_ui_frames': 6, 'phase': 'Off', 'cleanup_pending': False}
        for index, phase in enumerate(gate.PHASES):
            capture = {'phase': phase, 'ui_frame': 10*(index+1), 'state': {'playing': False}, 'typed': typed(alias, won=int(phase == 'won')),
                       'selected_guids': ['e_00000001'], 'camera': {'center': [0, 0], 'half_extent': 10},
                       'viewport': {'min': [100, 100], 'px': [1000, 800]}, 'png': phase+'.png'}
            pixels = bytearray(1400*1000*4)
            def paint(world, rgb, ring=False):
                cx, cy = gate.screen(capture, world)
                count = 0
                for y in range(int(cy)-20, int(cy)+21):
                    for x in range(int(cx)-20, int(cx)+21):
                        distance = math.hypot(x-cx, y-cy)
                        if (abs(distance-16) < 1.5) if ring else (distance < 14):
                            pixels[(y*1400+x)*4:(y*1400+x)*4+4] = bytes((*rgb, 255))
                            count += 1
                return count
            origin = [2, 2] if alias == 'delivery' else [2, 0]
            if phase != 'won' or alias == 'switchboard':
                paint(origin, [50, 150, 255])
            if alias == 'delivery':
                paint([3, 3], [250, 210, 50])
                counts = {'courier_origin_blue': 0 if phase == 'won' else 289, 'goal_gold': 289}
            else:
                if phase == 'won':
                    paint([4, 2], [40, 250, 40])
                counts = {'cursor_origin_blue': 289, 'target_lamp_green': 289 if phase == 'won' else 0}
            counts['selection_outline'] = paint(origin, [250, 210, 50], ring=True) if phase == 'edit' else 0
            capture['pixel_counts'] = counts
            path = directory/capture['png']
            rgba_png(path, pixels)
            capture['sha256'] = gate.digest(path)
            receipt['captures'].append(capture)
        write(directory/'native-receipt.json', receipt)


class NativeGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.root = Path(cls.temp.name)
        fixture(cls.root)
        cls.original = {p: p.read_bytes() for p in cls.root.rglob('*') if p.is_file()}

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def tearDown(self):
        for path, data in self.original.items():
            if path.is_symlink():
                path.unlink()
            path.write_bytes(data)

    def mutate(self, transform, alias='delivery'):
        path = self.root/alias/'native-receipt.json'
        receipt = json.loads(path.read_text())
        transform(receipt)
        write(path, receipt)
        with self.assertRaises((ValueError, KeyError)):
            gate.validate(self.root, PROVENANCE)

    def test_positive_complete_both_games(self):
        self.assertEqual(gate.validate(self.root, PROVENANCE)['captures'], 8)

    def test_missing_game(self):
        (self.root/'switchboard/native-receipt.json').unlink()
        with self.assertRaises(ValueError):
            gate.validate(self.root, PROVENANCE)

    def test_contract_provenance_and_required_checks_mutations(self):
        mutations = [
            lambda r: r.update(status='failed'),
            lambda r: r.update(source='headless'),
            lambda r: r['identity'].update(game='PhysGame'),
            lambda r: r['identity'].update(build_id='0x0000000000000000'),
            lambda r: r['identity'].update(schema={'wrong': 1}),
            lambda r: r['input_contract'].update(schema={'wrong': 1}),
            lambda r: r['input_contract'].update(max_players=2),
            lambda r: r['provenance'].update(compiled_source_sha='c'*40),
            lambda r: r['provenance'].update(executable_sha256='c'*64),
            lambda r: r['provenance'].update(run_attempt='2'),
            lambda r: r.update(expected_contracts_sha256='c'*64),
            lambda r: r['adapter'].update(name='SwiftShader'),
            lambda r: r['checks'].update(focus_neutral=False),
            lambda r: r['checks'].pop('focus_regain_no_auto_reclaim'),
            lambda r: r['boundaries'].update(physical_gpu=True),
            lambda r: r['scene'].update(path='../outside'),
            lambda r: r['captures'].pop(),
            lambda r: r['captures'][0].update(sha256='c'*64),
            lambda r: r['captures'][0].update(png='../edit.png'),
            lambda r: r['captures'][1]['typed']['state'].update(won=0),
            lambda r: r['release_proofs']['escape_neutral'].update(neutral_ticks=4),
            lambda r: r['release_proofs']['focus_neutral']['before_release']['state'].update(action_held=0),
            lambda r: r['release_proofs']['focus_neutral']['after']['state'].update(action_held=1),
            lambda r: r['no_reclaim_proofs']['focus'].update(phase='Active'),
            lambda r: r['no_reclaim_proofs']['focus'].update(observed_ui_frames=0),
        ]
        path = self.root/'delivery/native-receipt.json'
        for transform in mutations:
            with self.subTest(mutation=mutations.index(transform)):
                path.write_bytes(self.original[path])
                self.mutate(transform)

    def test_switch_direction_toggle_neutral_mutations(self):
        self.mutate(lambda r: r['release_proofs']['focus_neutral']['after']['actor'].update(position=[4, 0]), 'switchboard')
        path = self.root/'switchboard/native-receipt.json'
        path.write_bytes(self.original[path])
        self.mutate(lambda r: r['release_proofs']['escape_neutral']['after']['actor'].update(held=1), 'switchboard')
        path.write_bytes(self.original[path])
        self.mutate(lambda r: r['release_proofs']['escape_neutral']['after']['state'].update(moves=2), 'switchboard')

    def test_blank_png_with_updated_hash_and_claimed_pixel_counts_fails(self):
        path = self.root/'delivery/edit.png'
        rgba_png(path, bytes(1400*1000*4))
        self.mutate(lambda r: r['captures'][0].update(sha256=gate.digest(path)))

    def test_png_all_adaptive_filters_decode_independently(self):
        def chunk(kind, data):
            return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data))
        stride = 5600
        plain = bytes([17])*stride
        first = {1: bytes([17])*4+bytes(stride-4), 2: plain,
                 3: bytes([17])*4+bytes([9])*(stride-4), 4: bytes([17])*4+bytes(stride-4)}
        later = {1: first[1], 2: bytes(stride), 3: bytes([9])*4+bytes(stride-4), 4: bytes(stride)}
        with tempfile.TemporaryDirectory() as directory:
            for mode in (1, 2, 3, 4):
                with self.subTest(filter=mode):
                    raw = bytes([mode])+first[mode]+(bytes([mode])+later[mode])*999
                    encoded = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1400, 1000, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(raw, 1))+chunk(b'IEND', b'')
                    path = Path(directory)/'filtered.png'
                    path.write_bytes(encoded)
                    self.assertEqual(gate.png(path), plain*1000)

    def test_png_crc_and_symlink_rejected(self):
        path = self.root/'delivery/edit.png'
        data = bytearray(path.read_bytes())
        data[-1] ^= 1
        path.write_bytes(data)
        with self.assertRaises(ValueError):
            gate.png(path)
        path.unlink()
        path.symlink_to(self.root/'delivery/won.png')
        with self.assertRaises(ValueError):
            gate.png(path)

    def test_dirty_tracked_source_cannot_inherit_head_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q', directory], check=True)
            source = root/'source.rs'
            source.write_text('original')
            subprocess.run(['git', '-C', directory, 'add', 'source.rs'], check=True)
            subprocess.run(['git', '-C', directory, '-c', 'user.name=fixture', '-c',
                            'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
            sha = subprocess.check_output(['git', '-C', directory, 'rev-parse', 'HEAD'], text=True).strip()
            (root/'untracked-artifact.log').write_text('allowed generated artifact')
            gate.clean_checkout(sha, directory)
            (root/'examples').mkdir()
            unknown = root/'examples/unreviewed.rs'
            unknown.write_text('new compiler input')
            with self.assertRaises(ValueError):
                gate.clean_checkout(sha, directory)
            unknown.unlink()
            (root/'.git/info/exclude').write_text('examples/*.rs\n')
            unknown.write_text('ignored compiler input')
            with self.assertRaises(ValueError):
                gate.clean_checkout(sha, directory)
            unknown.unlink()
            source.write_text('changed compiled source')
            with self.assertRaises(ValueError):
                gate.clean_checkout(sha, directory)
            subprocess.run(['git', '-C', directory, 'add', 'source.rs'], check=True)
            with self.assertRaises(ValueError):
                gate.clean_checkout(sha, directory)

    def test_early_failure_still_writes_attempt(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(gate.os.environ, {}, clear=True):
            with self.assertRaises(ValueError):
                gate.run(Path(directory))
            self.assertEqual(json.loads((Path(directory)/'gate.json').read_text())['status'], 'failed')

    def test_timeout_preserves_partial_log_and_stops_group(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(ValueError):
                gate.command([gate.sys.executable, '-c', 'import time; print("started", flush=True); time.sleep(5)'],
                             root, 'timeout', dict(gate.os.environ), 0.1)
            self.assertIn('started', (root/'timeout.log').read_text())

    def test_test_inventory_result_cannot_be_zero_filtered_or_skipped(self):
        text = f'test {gate.TEST} ... native log\nok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n'
        gate.test_output(text)
        for invalid in (text.replace('1 passed', '0 passed'), text.replace('0 ignored', '1 ignored'), text.replace('0 filtered out', '1 filtered out'), text+'SKIP no GPU\n', text.replace(gate.TEST, 'some_other_test')):
            with self.subTest(output=invalid), self.assertRaises(ValueError):
                gate.test_output(invalid)


if __name__ == '__main__':
    unittest.main()
