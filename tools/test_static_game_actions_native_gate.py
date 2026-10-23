"""Synthetic adversarial unit tests; never physical/native acceptance evidence."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

import static_game_actions_native_gate as gate


class TypedTimelineTests(unittest.TestCase):
    def timeline(self):
        result = [{'tick': 0, 'state': copy.deepcopy(gate.INITIAL), 'checksum': '0x'+'0'*16}]
        for mask in (1, 0, 2, 0, 4, 0, 8, 0, 15, 14, 0):
            supplied = {'dx': int(mask == 15), 'dy': 0, 'actions': mask}
            result.append({'tick': len(result), 'input': supplied,
                           'state': gate.advance(result[-1]['state'], supplied), 'checksum': '0x'+'0'*16})
        return result

    def test_independent_four_bits_and_partial_release(self):
        gate.validate_ticks(self.timeline())

    def test_collapsed_or_aliased_bit_rejected(self):
        for index in range(4):
            ticks = self.timeline()
            ticks[2*index+1]['state']['counts'][(index+1) % 4] += 1
            with self.assertRaises(ValueError):
                gate.validate_ticks(ticks)

    def test_missing_reordered_duplicate_ticks(self):
        for mutation in (lambda t: t.pop(2), lambda t: t.insert(2, t[1]), lambda t: t.reverse()):
            ticks = self.timeline()
            mutation(ticks)
            with self.assertRaises(ValueError):
                gate.validate_ticks(ticks)

    def test_typed_input_rejects_bool_float_unknown_bits_and_extra_fields(self):
        for mutation in (lambda v: v.update(actions=True), lambda v: v.update(dx=1.0),
                         lambda v: v.update(actions=16), lambda v: v.update(dy=-2),
                         lambda v: v.update(other=0)):
            value = dict(gate.NEUTRAL)
            mutation(value)
            with self.assertRaises(ValueError):
                gate.input_value(value)

    def test_counter_and_checksum_corruption(self):
        for mutation in (lambda t: t[-1]['state'].update(held=15),
                         lambda t: t[-1]['state'].update(x=10),
                         lambda t: t[-1].update(checksum='0'*16),
                         lambda t: t[-1]['state'].update(counts=[1, 2, 3])):
            ticks = self.timeline()
            mutation(ticks)
            with self.assertRaises(ValueError):
                gate.validate_ticks(ticks)


class EvidenceBoundaryTests(unittest.TestCase):
    def test_path_escape_and_linked_directory_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root/'file.json').write_text('{}')
            (root/'linked').symlink_to(root, target_is_directory=True)
            self.assertEqual(gate.evidence_file(root, 'file.json'), root/'file.json')
            for value in ('../file.json', '/file.json', 'linked/file.json', './file.json', 'x//file.json', 'x\\file.json'):
                with self.assertRaises(ValueError):
                    gate.evidence_file(root, value)

    def test_duplicate_json_keys_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'receipt.json'
            path.write_text('{"schema":1,"schema":2}')
            with self.assertRaises(ValueError):
                gate.json_file(path)

    def test_exact_test_inventory(self):
        gate.test_inventory('a: test\nb: test\n\n2 tests, 0 benchmarks', {'a', 'b'})
        for output in ('a: test\na: test\n\n2 tests, 0 benchmarks',
                       'a: test\nc: test\n\n2 tests, 0 benchmarks',
                       'a: test\nb: test\n\n2 tests, 1 benchmarks'):
            with self.assertRaises(ValueError):
                gate.test_inventory(output, {'a', 'b'})

    def test_skipped_native_test_rejected(self):
        for status in ('ignored', 'FAILED', 'ok (headless)'):
            with self.assertRaises(ValueError):
                gate.test_result(f'test native ... {status}\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;', 'native', 0)


class ProducerTests(unittest.TestCase):
    def test_exact_source_package_kind_profile_and_hash(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root/'fixture.rs'
            source.write_text('// synthetic fixture')
            executable = root/'test-binary'
            executable.write_bytes(b'synthetic-not-executable')
            context = gate.ExecutionContext(root, 'fixture', 'path+file://fixture#0.1', {'native': source}, 'a'*40, root, 'a'*40, target_root=root)
            item = {'reason': 'compiler-artifact', 'package_id': context.package_id,
                    'target': {'name': 'native', 'kind': ['test'], 'src_path': str(source)},
                    'profile': {'opt_level': '3', 'test': True}, 'executable': str(executable)}
            result = gate.producers(json.dumps(item), context, {'native': 'test'})
            self.assertEqual(result['native']['sha256'], gate.digest(executable))
            changes = [lambda v: v.update(package_id='wrong'),
                       lambda v: v['target'].update(kind=['bin']),
                       lambda v: v['target'].update(src_path=str(root/'other.rs')),
                       lambda v: v['profile'].update(opt_level='0'),
                       lambda v: v['profile'].update(test=False),
                       lambda v: v.update(executable=str(root/'missing'))]
            for change in changes:
                altered = copy.deepcopy(item)
                change(altered)
                with self.assertRaises(ValueError):
                    gate.producers(json.dumps(altered), context, {'native': 'test'})
            with self.assertRaises(ValueError):
                gate.producers(json.dumps(item)+'\n'+json.dumps(item), context, {'native': 'test'})
            with self.assertRaises(ValueError):
                gate.producers('', context, {'native': 'test'})


class PixelOracleTests(unittest.TestCase):
    def image(self, path, mask, width=1024, height=768):
        import struct
        import zlib
        pixels = bytearray(width*height*4)
        colors = [(255, 80, 80), (80, 255, 80), (80, 120, 255), (255, 240, 80)]
        for i, x in enumerate((-3, -1, 1, 3)):
            cx, cy = round(width/2+x*min(width,height)/(2*5.175)), round(height/2-2*min(width,height)/(2*5.175))
            color = colors[i] if mask & (1 << i) else (100, 100, 100)
            for y in range(cy-9, cy+10):
                for xx in range(cx-9, cx+10):
                    pixels[(y*width+xx)*4:(y*width+xx)*4+4] = bytes((*color, 255))
        def chunk(kind, data):
            return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data))
        raw = b''.join(b'\0'+pixels[y*width*4:(y+1)*width*4] for y in range(height))
        path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', width,height,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,1))+chunk(b'IEND', b''))

    def test_each_independent_bit_and_wrong_bit_pixels(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'synthetic.png'
            for mask in (0,1,2,4,8,15,14):
                capture = {'observation': {'size':[1024,768], 'camera':{'center':[0,0],'half_extent':5.175}, 'state':{**gate.INITIAL,'held':mask}}}
                self.image(path,mask)
                gate.fixture_pixels(path,capture)
                self.image(path,mask^8)
                with self.assertRaises(ValueError):
                    gate.fixture_pixels(path,capture)

    def test_producer_camera_cannot_redirect_pixel_oracle(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'synthetic.png'
            self.image(path,0)
            capture = {'observation': {'size':[1024,768], 'camera':{'center':[1,0],'half_extent':5.175}, 'state':gate.INITIAL}}
            with self.assertRaises(ValueError):
                gate.fixture_pixels(path,capture)


class IncidentalPlayerProducerTests(unittest.TestCase):
    def test_known_headless_player_is_exact_not_wildcard_admission(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root/'src/bin/player.rs'
            source.parent.mkdir(parents=True)
            source.write_text('// synthetic headless source')
            executable = root/'player'
            executable.write_bytes(b'synthetic-not-native-evidence')
            context = gate.ExecutionContext(root, 'fixture', 'path+file://fixture#0.1',
                {'player': source}, 'a'*40, root, 'a'*40, target_root=root)
            item = {'reason':'compiler-artifact','package_id':context.package_id,
                    'target':{'name':'player','kind':['bin'],'src_path':str(source)},
                    'profile':{'opt_level':'3','test':False},'executable':str(executable)}
            result = gate.producers(json.dumps(item), context, {'player':'bin'})
            self.assertEqual(result['player']['source'], str(source))
            self.assertEqual(gate.TARGETS['player'], ('bin','src/bin/player.rs'))
            for change in (lambda v: v['target'].update(name='unknown-player'),
                           lambda v: v['target'].update(src_path=str(root/'spoof.rs')),
                           lambda v: v['target'].update(kind=['test']),
                           lambda v: v.update(package_id='path+file://spoof#0.1')):
                altered = copy.deepcopy(item)
                change(altered)
                with self.assertRaises(ValueError):
                    gate.producers(json.dumps(altered), context, {'player':'bin'})


if __name__ == '__main__':
    unittest.main()
