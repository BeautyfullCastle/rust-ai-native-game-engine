import copy
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
import zlib
import terrain_ramp_gate as gate

HEAD = 'a' * 40


def png(pixels, size=(11, 10)):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    w, h = size
    rows = b''.join(b'\0' + pixels[y*w*4:(y+1)*w*4] for y in range(h))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def write(root, name, value):
    (root / name).write_text(json.dumps(value))


def refresh(root):
    value = gate.document(root / 'complete.json')
    value['files'] = {p.name: {'bytes': p.stat().st_size, 'sha256': gate.sha(p.read_bytes())}
                      for p in root.iterdir() if p.name != 'complete.json'}
    write(root, 'complete.json', value)


def fixture(root):
    adapter = dict(phase='adapter-created', backend='Vulkan', device_type='Cpu', adapter='llvmpipe test', source_sha=HEAD)
    write(root, 'adapter.json', adapter); write(root, 'reopen-adapter.json', adapter)
    trace = []
    for i, p in enumerate(gate.WAYPOINTS):
        # Deliberately opaque fake bytes: Python integrity tests do not decode ECS Frames.
        data = b'opaque-fixture-frame-' + bytes([i])
        name = f'step-{i:02}.frame'; (root / name).write_bytes(data)
        trace.append(dict(step=i, tick=i, checksum=f'{i:016x}', file=name, sha256=gate.sha(data), position_raw=p, status='Arrived' if i == len(gate.WAYPOINTS)-1 else 'Moving',
                          next_waypoint=i+1, path=dict(corridor=gate.CORRIDOR, waypoints_raw=gate.WAYPOINTS)))
    write(root, 'traversal-partial.json', trace)
    drains = []
    for i in range(len(trace)):
        drains.extend([dict(step=i, tick=i, phase='before-render'),
                       dict(step=i, tick=i, phase='after-render', ok=True, dimensions=[1440, 1200])])
    write(root, 'render-drains-partial.json', drains)
    indexes = {'05-play-paused': 0, '05b-middle': 1, '06-arrival': len(trace)-1}
    for phase in gate.PHASES:
        i = indexes.get(phase, 0); row = trace[i]
        data = (root / row['file']).read_bytes()
        (root / (phase + '.frame')).write_bytes(data)
        pixels = bytes([255 if phase.startswith('negative') or phase == '01-before' else 0, 0, 0, 255]) * 110
        for suffix in ('', '-app'):
            (root / (phase + suffix + '.rgba')).write_bytes(pixels)
            (root / (phase + suffix + '.png')).write_bytes(png(pixels))
        terrain = 0 if phase in ('01-before', '01b-picked-endpoints', '02-stage', '03b-undo') else 1
        if phase.startswith('negative') or phase.startswith(('07-', '08-')):
            terrain = 2
        undo = phase in ('03-applied', '03c-redo', 'negative-wrong-axis', 'negative-wrong-slope', '04-built', '07-stale', '08-stale-play-refused')
        redo = phase in ('03b-undo', '04-built')
        dirty = undo and phase != '04-built'
        value = dict(phase=phase, partial=True, source_sha=HEAD, adapter=adapter['adapter'],
                     mode='Play' if phase in indexes else 'Edit', tick=i, checksum=row['checksum'],
                     display_tick=i, display_checksum=row['checksum'], route_stale=phase.startswith(('07-', '08-')),
                     terrain_revision=[terrain]*32, terrain_bytes_sha256=str(terrain)*64,
                     terrain_can_undo=undo, terrain_can_redo=redo, terrain_dirty=dirty, terrain_error=None,
                     diagnostics=[], navigation_error=None,
                     viewport=[11, 10], composed_size=[11, 10], frame_sha256=gate.sha(data),
                     rgba_sha256=gate.sha(pixels), composed_rgba_sha256=gate.sha(pixels),
                     position_raw=row['position_raw'], status=row['status'],
                     next_waypoint=row['next_waypoint'], path=row['path'])
        write(root, phase + '.json', value)
    complete = dict(complete=True, phases=gate.PHASES, source_sha=HEAD, scope=gate.SCOPE,
                    expected_corridor=gate.CORRIDOR, expected_waypoints_raw=gate.WAYPOINTS,
                    independent_raw_profile=[0, 16384, 32768, 49152, 65536, 81920, 98304],
                    pointer_endpoints=True, stage_cancel_apply=True, single_undo_redo=True,
                    save_fresh_reopen=True, stale_play_refused=True, authoritative_frame_trace=trace, render_drains=drains)
    write(root, 'complete.json', complete); refresh(root)


class Receipts(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); fixture(self.root)

    def mutate(self, filename, change, rehash=True):
        value = gate.document(self.root / filename); change(value); write(self.root, filename, value)
        if rehash:
            refresh(self.root)

    def fail(self):
        with self.assertRaises((RuntimeError, ValueError, KeyError, TypeError)):
            gate.receipts(self.root, HEAD)

    def test_valid_integrity_not_frame_decoder(self):
        self.assertEqual(gate.receipts(self.root, HEAD), {'phases': 15, 'frames_in_traversal': 14, 'files': 109})

    def rewrite_drains(self, change):
        drains = gate.document(self.root / 'render-drains-partial.json'); change(drains)
        write(self.root, 'render-drains-partial.json', drains)
        self.mutate('complete.json', lambda value: value.update(render_drains=drains))

    def test_render_drain_file_required(self):
        (self.root / 'render-drains-partial.json').unlink(); refresh(self.root); self.fail()

    def test_render_drain_partial_complete_disagreement(self):
        self.mutate('render-drains-partial.json', lambda rows: rows[1].update(ok=False)); self.fail()

    def test_render_drain_census_order_and_failure_rejected(self):
        changes = [lambda rows: rows.pop(), lambda rows: rows.extend(rows[:2]),
                   lambda rows: rows.reverse(), lambda rows: rows[1].update(ok=False),
                   lambda rows: rows[1].update(ok=1), lambda rows: rows[1].update(error='timeout'),
                   lambda rows: rows[1].update(dimensions=[4097, 1200]),
                   lambda rows: rows[1].update(dimensions=[1440.0, 1200]),
                   lambda rows: rows[2].update(step=0), lambda rows: rows[1].update(tick=99),
                   lambda rows: rows[0].update(phase='after-render'),
                   lambda rows: rows[0].update(step=False)]
        for change in changes:
            with self.subTest(change=change):
                fixture(self.root); self.rewrite_drains(change); self.fail()

    def test_render_drain_original_hash_failure(self):
        (self.root / 'render-drains-partial.json').write_text('[]'); self.fail()

    def rewrite_trace(self, change):
        trace = gate.document(self.root / 'traversal-partial.json'); change(trace)
        write(self.root, 'traversal-partial.json', trace)
        complete = gate.document(self.root / 'complete.json'); complete['authoritative_frame_trace'] = trace
        write(self.root, 'complete.json', complete)
        for phase, i in [('05-play-paused', 0), ('05b-middle', 1), ('06-arrival', len(trace)-1)]:
            value = gate.document(self.root / (phase + '.json'))
            value.update({k: trace[i][k] for k in ('position_raw', 'status', 'next_waypoint', 'path')})
            write(self.root, phase + '.json', value)
        refresh(self.root)

    def test_wrong_route_geometry_with_all_receipts_rehashed(self):
        for key in ('corridor', 'waypoints_raw'):
            with self.subTest(key=key):
                fixture(self.root)
                def change(trace):
                    for row in trace:
                        if key == 'corridor':
                            row['path'][key][0] = 65
                        else:
                            row['path'][key][1][2] = 0
                self.rewrite_trace(change); self.fail()

    def test_wrong_complete_expected_geometry(self):
        self.mutate('complete.json', lambda v: v.update(expected_corridor=list(range(65, 78)))); self.fail()

    def test_boolean_geometry_is_not_integer_geometry(self):
        self.mutate('complete.json', lambda v: v['expected_waypoints_raw'][0].__setitem__(1, False)); self.fail()
        fixture(self.root)
        self.rewrite_trace(lambda rows: rows[1]['path']['waypoints_raw'][0].__setitem__(1, False)); self.fail()

    def test_arrived_requires_terminal_waypoint(self):
        self.rewrite_trace(lambda rows: rows[-1].update(next_waypoint=13)); self.fail()

    def test_straight_line_shortcut_rejected_with_consistent_receipts(self):
        self.rewrite_trace(lambda rows: [row['position_raw'].__setitem__(2, 0) for row in rows])
        self.fail()

    def test_corridor_bounds_and_triangle_owner(self):
        for position, next_waypoint in [([-163840, 8192, -1], 2),
                                        ([-163840, 8192, 32769], 2),
                                        ([-163840, 8192, 0], 1),
                                        ([-163840, 8192, 32768], 4)]:
            with self.subTest(position=position, next_waypoint=next_waypoint):
                fixture(self.root)
                self.rewrite_trace(lambda rows: rows[1].update(position_raw=position, next_waypoint=next_waypoint))
                self.fail()

    def test_next_waypoint_missing_invalid_or_wrong_arrival(self):
        for index in (None, True, 0, -1, 15, 14):
            with self.subTest(index=index):
                fixture(self.root)
                self.rewrite_trace(lambda rows: rows[1].update(next_waypoint=index)); self.fail()

    def test_stationary_or_backwards_steps_with_consistent_receipts(self):
        for index in (2, 3):
            with self.subTest(index=index):
                fixture(self.root)
                def change(rows):
                    rows[index].update({k: rows[1][k] for k in ('position_raw', 'next_waypoint')})
                self.rewrite_trace(change); self.fail()

    def test_final_vertical_leg_cannot_increase_z(self):
        fixture(self.root)
        self.rewrite_trace(lambda rows: rows[11].update(position_raw=[196608, 98304, 16384], next_waypoint=13))
        self.fail()

    def test_old_twelve_phase_schema_rejected(self):
        removed = {'03b-undo', '03c-redo', '04b-reopened'}
        for phase in removed:
            for suffix in gate.SUFFIXES:
                (self.root / (phase + suffix)).unlink()
        self.mutate('complete.json', lambda v: v.update(phases=[p for p in gate.PHASES if p not in removed]))
        self.fail()

    def test_missing_new_phase_artifacts(self):
        for phase in ('03b-undo', '03c-redo', '04b-reopened'):
            for suffix in gate.SUFFIXES:
                with self.subTest(phase=phase, suffix=suffix):
                    path = self.root / (phase + suffix); data = path.read_bytes(); path.unlink()
                    refresh(self.root); self.fail(); path.write_bytes(data); refresh(self.root)

    def test_new_terrain_receipt_fields_required_and_typed(self):
        fields = {'terrain_bytes_sha256': ['bad', None, 1], 'terrain_can_undo': [0, None],
                  'terrain_can_redo': [1, None], 'terrain_dirty': [0, None], 'terrain_error': ['failure']}
        filename = '04b-reopened.json'; original = gate.document(self.root / filename)
        for key, values in fields.items():
            for value in values:
                with self.subTest(key=key, value=value):
                    changed = dict(original); changed[key] = value
                    write(self.root, filename, changed); refresh(self.root); self.fail()
            changed = dict(original); del changed[key]
            write(self.root, filename, changed); refresh(self.root); self.fail()
        write(self.root, filename, original); refresh(self.root)

    def test_history_state_lies_rejected_after_manifest_rehash(self):
        for phase in gate.PHASES:
            filename = phase + '.json'; original = gate.document(self.root / filename)
            for key in ('terrain_can_undo', 'terrain_can_redo', 'terrain_dirty'):
                with self.subTest(phase=phase, key=key):
                    changed = dict(original); changed[key] = not changed[key]
                    write(self.root, filename, changed); refresh(self.root); self.fail()
            write(self.root, filename, original); refresh(self.root)

    def test_terrain_identity_drift_after_manifest_rehash(self):
        for phase in gate.PHASES:
            if phase.startswith('negative') or phase == '07-stale':
                continue
            filename = phase + '.json'; original = gate.document(self.root / filename)
            for key, value in [('terrain_bytes_sha256', 'f'*64), ('terrain_revision', [255]*32)]:
                with self.subTest(phase=phase, key=key):
                    changed = dict(original); changed[key] = value
                    write(self.root, filename, changed); refresh(self.root); self.fail()
            write(self.root, filename, original); refresh(self.root)

    def test_vacuous_terrain_controls_after_manifest_rehash(self):
        authored = gate.document(self.root / '03-applied.json')
        for phase in ('negative-wrong-axis', 'negative-wrong-slope', '07-stale'):
            filename = phase + '.json'; original = gate.document(self.root / filename)
            for key in ('terrain_bytes_sha256', 'terrain_revision'):
                changed = dict(original); changed[key] = authored[key]
                write(self.root, filename, changed); refresh(self.root); self.fail()
            write(self.root, filename, original); refresh(self.root)

    def test_missing_phase(self):
        (self.root / '05b-middle.frame').unlink(); refresh(self.root); self.fail()

    def test_extra_file_even_if_manifest_contains_it(self):
        (self.root / 'unexpected').write_bytes(b'x'); refresh(self.root); self.fail()

    def test_symlink(self):
        p = self.root / '01-before.rgba'; p.unlink(); p.symlink_to('02-stage.rgba'); self.fail()

    def test_manifest_original_hash_failure(self):
        (self.root / '01-before.frame').write_bytes(b'corruption'); self.fail()

    def test_frame_receipt_hash_failure_after_manifest_rehash(self):
        (self.root / '01-before.frame').write_bytes(b'corruption'); refresh(self.root); self.fail()

    def test_crc_corruption_with_manifest_rehash(self):
        p = self.root / '01-before.png'; data = bytearray(p.read_bytes()); data[-1] ^= 1; p.write_bytes(data)
        refresh(self.root); self.fail()

    def test_png_raw_mismatch(self):
        (self.root / '01-before.png').write_bytes(png(bytes([1, 2, 3, 255])*110)); refresh(self.root); self.fail()

    def test_dimensions(self):
        self.mutate('01-before.json', lambda v: v.update(viewport=[4097, 1])); self.fail()

    def test_display_identity(self):
        self.mutate('06-arrival.json', lambda v: v.update(display_tick=99)); self.fail()

    def test_stale_post_attempt_mode(self):
        self.mutate('08-stale-play-refused.json', lambda v: v.update(mode='Play')); self.fail()

    def test_stale_flag(self):
        self.mutate('08-stale-play-refused.json', lambda v: v.update(route_stale=False)); self.fail()

    def test_actual_hosted_adapter(self):
        self.mutate('adapter.json', lambda v: v.update(adapter='SwiftShader')); self.fail()

    def test_wrong_source(self):
        self.mutate('05b-middle.json', lambda v: v.update(source_sha='b'*40)); self.fail()

    def test_wrong_complete_type(self):
        self.mutate('complete.json', lambda v: v.update(complete=1)); self.fail()

    def test_trace_disagrees(self):
        self.mutate('traversal-partial.json', lambda v: v[1].update(tick=99)); self.fail()

    def test_profile_mismatch_with_both_trace_copies_changed(self):
        trace = gate.document(self.root / 'traversal-partial.json'); trace[1]['position_raw'][1] += 1
        write(self.root, 'traversal-partial.json', trace)
        self.mutate('complete.json', lambda v: v.update(authoritative_frame_trace=trace)); self.fail()

    def test_duplicate_json_key(self):
        p = self.root / 'complete.json'; p.write_text(p.read_text().replace('"complete": true', '"complete": true, "complete": true')); self.fail()

    def test_vacuous_control_with_all_pixel_hashes_consistent(self):
        phase = 'negative-wrong-axis'; pixels = (self.root / '03-applied.rgba').read_bytes()
        (self.root / (phase + '.rgba')).write_bytes(pixels); (self.root / (phase + '.png')).write_bytes(png(pixels))
        self.mutate(phase + '.json', lambda v: v.update(rgba_sha256=gate.sha(pixels))); self.fail()

class ProducerPin(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'source'; self.root.mkdir()
        self.evidence = Path(self.tmp.name) / 'evidence'; self.evidence.mkdir()
        gate.runner.evidence = self.evidence
        gate.runner.target_dir = self.root / 'target/private'
        (gate.runner.target_dir / 'release/deps').mkdir(parents=True)
        gate.runner.report = {'producers': {}}
        crate = self.root / 'crates/orr_editor'
        self.rows = []
        for name, kind, src, types in [('orr_editor', 'lib', 'src/lib.rs', ['lib']), ('terrain_ramp', 'test', 'tests/terrain_ramp.rs', ['bin'])]:
            binary = gate.runner.target_dir / ('release/deps/' + name); binary.write_bytes(b'not-executed-fixture'); binary.chmod(0o700)
            self.rows.append(dict(reason='compiler-artifact', fresh=False, manifest_path=str(crate / 'Cargo.toml'),
                package_id='path+' + crate.as_uri() + '#orr_editor@0.0.1', features=['terrain', 'models', 'navigation'],
                profile=dict(test=True, opt_level='3'), executable=str(binary),
                target=dict(name=name, kind=[kind], crate_types=types, src_path=str(crate / src))))

    def test_exact_grouped_artifacts(self):
        bins = gate.pin('\n'.join(map(json.dumps, self.rows)), self.root)
        self.assertEqual(set(bins), {'orr_editor', 'terrain_ramp'})
        gate.runner.fence()

    def test_wrong_producer_identity(self):
        mutations = [lambda r: r.pop(), lambda r: r.append(copy.deepcopy(r[0])),
                     lambda r: r[0].update(fresh=True), lambda r: r[0].update(features=['terrain']),
                     lambda r: r[0]['target'].update(src_path='/wrong/lib.rs'),
                     lambda r: r[0].update(package_id='registry+invalid')]
        for change in mutations:
            with self.subTest(change=change):
                rows = copy.deepcopy(self.rows); change(rows)
                with self.assertRaises(RuntimeError):
                    gate.pin('\n'.join(map(json.dumps, rows)), self.root)


if __name__ == '__main__':
    unittest.main()
