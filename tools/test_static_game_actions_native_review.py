"""Independent synthetic rejection cases. These are not native execution evidence."""
import copy
import unittest
from unittest import mock

import static_game_actions_native_gate as gate


def independent_timeline():
    state = {'x': 0, 'y': 0, 'counts': [0, 0, 0, 0], 'held': 0}
    ticks = [{'tick': 0, 'state': copy.deepcopy(state), 'checksum': '0x0000000000000000'}]
    for number, (x, y, mask) in enumerate([(0, 0, 8), (0, 0, 0), (1, -1, 15),
                                          (1, -1, 7), (0, 0, 0)], 1):
        # Deliberately do not call the validator's advance implementation.
        state['x'] += x
        state['y'] += y
        state['counts'] = [count + ((mask >> bit) & 1)
                           for bit, count in enumerate(state['counts'])]
        state['held'] = mask
        ticks.append({'tick': number, 'input': {'dx': x, 'dy': y, 'actions': mask},
                      'state': copy.deepcopy(state), 'checksum': f'0x{number:016x}'})
    return ticks


class IndependentConsumedTickReview(unittest.TestCase):
    def reject(self, mutate):
        ticks = independent_timeline()
        mutate(ticks)
        with self.assertRaises((ValueError, KeyError, TypeError)):
            gate.validate_ticks(ticks)

    def test_separately_derived_valid_timeline(self):
        gate.validate_ticks(independent_timeline())

    def test_fourth_bit_cannot_be_truncated_or_aliased(self):
        self.reject(lambda ticks: ticks[1]['input'].update(actions=0))
        self.reject(lambda ticks: ticks[1]['input'].update(actions=1))
        self.reject(lambda ticks: ticks[1]['state']['counts'].__setitem__(3, 0))

    def test_partial_release_must_not_clear_unrelated_actions(self):
        self.reject(lambda ticks: ticks[4]['state'].update(held=0))
        self.reject(lambda ticks: ticks[4]['state']['counts'].__setitem__(0, 1))

    def test_axes_cannot_swap_or_use_opposite_polarity(self):
        self.reject(lambda ticks: ticks[3]['input'].update(dx=-1, dy=1))
        self.reject(lambda ticks: ticks[3]['state'].update(x=-1, y=1))

    def test_omitted_duplicate_and_invented_tick_numbers(self):
        self.reject(lambda ticks: ticks.pop(2))
        self.reject(lambda ticks: ticks.insert(2, copy.deepcopy(ticks[1])))
        self.reject(lambda ticks: ticks[2].update(tick=3))
        self.reject(lambda ticks: ticks[0].update(tick=False))

    def test_neutral_tick_cannot_hide_counter_or_movement_change(self):
        self.reject(lambda ticks: ticks[2]['state']['counts'].__setitem__(0, 1))
        self.reject(lambda ticks: ticks[2]['state'].update(x=1))
        self.reject(lambda ticks: ticks[2]['state'].update(held=8))

    def test_initial_input_or_pass_flags_are_not_ignored(self):
        self.reject(lambda ticks: ticks[0].update(input={'dx': 1, 'dy': 0, 'actions': 15}))
        self.reject(lambda ticks: ticks[2].update(passed=True))

    def test_numeric_lookalikes_and_noncanonical_checksums(self):
        self.reject(lambda ticks: ticks[1]['input'].update(actions=8.0))
        self.reject(lambda ticks: ticks[1]['state'].update(held=True))
        self.reject(lambda ticks: ticks[1].update(checksum='0x000000000000000A'))
        self.reject(lambda ticks: ticks[1].update(checksum='0x0000000000000001\n'))


def clear_fixture():
    state = {'x': 1, 'y': 1, 'counts': [1, 1, 1, 1], 'held': 15}
    neutral_state = {**state, 'counts': list(state['counts']), 'held': 0}
    ticks = [
        {'tick': 0, 'state': copy.deepcopy(gate.INITIAL), 'checksum': '0x0000000000000000'},
        {'tick': 1, 'input': {'dx': 1, 'dy': 1, 'actions': 15},
         'state': state, 'checksum': '0x0000000000000001'},
        {'tick': 2, 'input': dict(gate.NEUTRAL), 'state': neutral_state,
         'checksum': '0x0000000000000002'},
    ]
    observations = [{'sequence': sequence, 'tick': 1, 'state': copy.deepcopy(state),
                     'paused': True, 'focused': True, 'consumed': []}
                    for sequence in range(10, 17)]
    resumed = {'sequence': 17, 'tick': 2, 'state': neutral_state, 'paused': False,
               'focused': True, 'resume_from_tick': 1, 'neutral_consumed_tick': 2,
               'consumed': [ticks[2]]}
    observations.append(resumed)
    proof = {'paused': observations[0], 'no_auto_resume': observations[-2],
             'resumed': resumed, 'first_consumed': ticks[2],
             'before': observations[0], 'after': resumed}
    return proof, observations, ticks


class IndependentResetReview(unittest.TestCase):
    def test_complete_ordered_neutral_boundary(self):
        gate.clear_proof(*clear_fixture())

    def test_resume_cannot_precede_the_pause(self):
        proof, observations, ticks = clear_fixture()
        proof['resumed']['sequence'] = 9
        with self.assertRaises(ValueError):
            gate.clear_proof(proof, observations, ticks)

    def test_first_consumed_cannot_be_replaced_by_a_later_neutral(self):
        proof, observations, ticks = clear_fixture()
        proof['first_consumed'] = {**ticks[2], 'tick': 3}
        with self.assertRaises(ValueError):
            gate.clear_proof(proof, observations, ticks)

    def test_quiet_render_frames_do_not_substitute_for_neutral_consumption(self):
        proof, observations, ticks = clear_fixture()
        proof['resumed']['neutral_consumed_tick'] = None
        with self.assertRaises(ValueError):
            gate.clear_proof(proof, observations, ticks)

    def test_paused_interval_cannot_consume_a_hidden_tick(self):
        proof, observations, ticks = clear_fixture()
        observations[2]['consumed'] = [ticks[2]]
        with self.assertRaises(ValueError):
            gate.clear_proof(proof, observations, ticks)



# Complete synthetic receipt assembly. Pixel decoding is tested separately by the
# gate owner; semantic tests mock only that decoder, never trace/provenance checks.
def standalone_receipt_fixture(directory):
    import json
    from pathlib import Path
    root = Path(directory)
    source = 'a' * 40
    identity = {'game': 'PrivateNativeActions', 'build_id': '0x4d414354494f4e01',
                'seed': 42, 'tick_rate': 60, 'player_count': 1,
                'initial_checksum': '0x0000000000000000'}
    ticks = [{'tick': 0, 'state': copy.deepcopy(gate.INITIAL),
              'checksum': identity['initial_checksum']}]
    observations, captures, scenarios = [], [], []
    keys = [0x20, 0x66, 0x65, 0x71]

    def observe(supplied=None, paused=False, focused=True, **fields):
        consumed = []
        if supplied is not None:
            old = ticks[-1]['state']
            mask = supplied['actions']
            state = {'x': old['x'] + supplied['dx'], 'y': old['y'] + supplied['dy'],
                     'counts': [n + ((mask >> i) & 1) for i, n in enumerate(old['counts'])],
                     'held': mask}
            tick = {'tick': len(ticks), 'input': supplied, 'state': state,
                    'checksum': f'0x{len(ticks):016x}'}
            ticks.append(tick)
            consumed = [copy.deepcopy(tick)]
        latest = ticks[-1]
        observation = {'schema': 'orr.static-standalone-observation/1', 'identity': identity,
                       'source_sha': source, 'process_id': 123, 'sequence': len(observations) + 1,
                       'tick': latest['tick'], 'state': copy.deepcopy(latest['state']),
                       'checksum': latest['checksum'], 'paused': paused, 'focused': focused,
                       'consumed': consumed, 'adapter': {'backend': 'Vulkan', 'device_type': 'Cpu'},
                       'size': [640, 480], 'camera': {'center': [0, 0], 'half_extent': 5.175},
                       **fields}
        observations.append(observation)
        return observation

    def capture(phase):
        before = observe(paused=True)
        after = observe(paused=True)
        path = root / (phase + '.png')
        path.write_bytes(b'synthetic-pixels-not-native-proof-' + phase.encode())
        captures.append({'phase': phase, 'before': before, 'after': after, 'observation': before,
                         'png': path.name, 'sha256': gate.digest(path),
                         'x11': {'width': 640, 'height': 480, 'depth': 24, 'bits_per_pixel': 32}})

    def event(key, down, before, after):
        return {'keysym': key, 'pressed': down, 'after_sequence': before['sequence'],
                'before_sequence': after['sequence']}

    observe(paused=True)
    capture('initial')
    for index, key in enumerate(keys):
        before = observe(dict(gate.NEUTRAL))
        after = observe({'dx': 0, 'dy': 0, 'actions': 1 << index})
        scenarios.append({'name': f'independent_{1 << index}', 'before': before, 'after': after,
                          'events': [event(key, True, before, after)]})
        capture(f'action_{1 << index}')
    before = observe(dict(gate.NEUTRAL))
    after = observe({'dx': 1, 'dy': 1, 'actions': 15})
    scenarios.append({'name': 'simultaneous', 'before': before, 'after': after,
                      'events': [event(k, True, before, after) for k in keys + [0x64, 0x77]]})
    capture('simultaneous')
    before = observe({'dx': 1, 'dy': 1, 'actions': 15})
    after = observe({'dx': 1, 'dy': 1, 'actions': 14})
    scenarios.append({'name': 'partial_release', 'before': before, 'after': after,
                      'events': [event(keys[0], False, before, after)]})
    capture('partial_release')
    before = observe({'dx': 0, 'dy': 0, 'actions': 15})
    after = observe(dict(gate.NEUTRAL))
    scenarios.append({'name': 'neutral', 'before': before, 'after': after,
                      'events': [event(k, False, before, after) for k in keys]})
    capture('neutral')
    for blur in [False, True]:
        pre = observe(dict(gate.NEUTRAL))
        before = observe({'dx': 1, 'dy': 1, 'actions': 15})
        paused = observe(paused=True, focused=not blur)
        for _ in range(6):
            no_auto = observe(paused=True)
        resumed = observe(dict(gate.NEUTRAL), resume_from_tick=paused['tick'],
                          neutral_consumed_tick=paused['tick'] + 1)
        events = [event(k, True, pre, before) for k in keys + [0x64, 0x77]]
        if blur:
            events.extend([{'kind': 'focus_loss', 'after_sequence': before['sequence'],
                            'before_sequence': paused['sequence']},
                           {'kind': 'focus_regain', 'after_sequence': paused['sequence'],
                            'before_sequence': no_auto['sequence']}])
        else:
            events.extend([event(0xff1b, down, before, paused) for down in [True, False]])
        events.extend([event(k, False, paused, no_auto) for k in keys + [0x64, 0x77]])
        events.extend([event(0xff0d, down, no_auto, resumed) for down in [True, False]])
        scenarios.append({'name': 'focus_clear' if blur else 'escape_clear', 'before': before,
                          'after': resumed, 'events': events, 'paused': paused,
                          'no_auto_resume': no_auto, 'resumed': resumed,
                          'first_consumed': copy.deepcopy(ticks[-1])})
    provenance = {'source_sha': source, 'run_id': 'synthetic', 'run_attempt': '1',
                  'test_sha256': 'b' * 64, 'player_sha256': 'c' * 64, 'process_id': 123}
    (root / 'authored.scene.yaml').write_text(gate.SCENE)
    (root / 'record.orrp').write_bytes(b'ORRPsynthetic-unit-fixture')
    (root / 'source-replay.json').write_text(json.dumps({'ticks': ticks}))
    (root / 'observations.ndjson').write_text(''.join(json.dumps(o) + '\n' for o in observations))
    artifacts = {key: {'path': filename, 'sha256': gate.digest(root / filename)} for key, filename in
                 [('observations', 'observations.ndjson'), ('replay', 'record.orrp'),
                  ('source_replay', 'source-replay.json')]}
    receipt = {'schema': gate.SCHEMA, 'mode': 'standalone', 'status': 'passed',
               'boundaries': copy.deepcopy(gate.BOUNDARIES), 'packaged': False, 'source_hidden': False,
               'profile': copy.deepcopy(gate.PROFILE), 'identity': identity, 'provenance': provenance,
               'scene': {'path': 'authored.scene.yaml', 'sha256': gate.digest(root / 'authored.scene.yaml')},
               'artifacts': artifacts, 'captures': captures, 'scenarios': scenarios,
               'replay': {'path': 'record.orrp', 'sha256': artifacts['replay']['sha256'],
                          'source_replay_sha256': artifacts['source_replay']['sha256'],
                          'ticks': len(ticks) - 1, 'initial_checksum': ticks[0]['checksum'],
                          'final_checksum': ticks[-1]['checksum']}}
    (root / 'receipt.json').write_text(json.dumps(receipt))
    return receipt, copy.deepcopy(provenance)


class IndependentStandaloneReceiptReview(unittest.TestCase):
    def run_receipt(self, mutate=None):
        import json
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temp:
            receipt, provenance = standalone_receipt_fixture(temp)
            if mutate:
                mutate(receipt)
                (Path(temp) / 'receipt.json').write_text(json.dumps(receipt))
            with patch.object(gate, 'fixture_pixels'):
                return gate.validate_receipt(temp, provenance)

    def test_complete_synthetic_semantic_receipt(self):
        self.assertEqual(self.run_receipt()['mode'], 'standalone')

    def test_pass_flag_cannot_replace_actual_scenarios(self):
        with self.assertRaises(ValueError):
            self.run_receipt(lambda r: r.update(scenarios=[]))

    def test_scene_and_profile_identity_cannot_be_relabelled(self):
        for mutate in [lambda r: r['profile']['actions'][3].update(mask=4),
                       lambda r: r['identity'].update(build_id='0x0000000000000000'),
                       lambda r: r['scene'].update(sha256='0' * 64)]:
            with self.assertRaises(ValueError):
                self.run_receipt(mutate)

    def test_source_hidden_and_hardware_claims_are_rejected(self):
        for mutate in [lambda r: r.update(source_hidden=True),
                       lambda r: r['boundaries'].update(physical_hardware_keyboard=True)]:
            with self.assertRaises(ValueError):
                self.run_receipt(mutate)

    def test_capture_must_reference_actual_bracketed_state(self):
        def mutate(receipt):
            receipt['captures'][1]['observation'] = copy.deepcopy(receipt['captures'][1]['observation'])
            receipt['captures'][1]['observation']['process_id'] = 999
        with self.assertRaises(ValueError):
            self.run_receipt(mutate)

    def test_numeric_lookalikes_cannot_change_receipt_field_types(self):
        mutations = [lambda r: r['boundaries'].update(software_gpu=1),
                     lambda r: r['profile']['actions'][0].update(mask=1.0),
                     lambda r: r['identity'].update(seed=42.0)]
        for mutate in mutations:
            with self.assertRaises(ValueError):
                self.run_receipt(mutate)

    def test_compiled_producer_cannot_be_relabelled(self):
        for key in ['source_sha', 'test_sha256', 'player_sha256', 'run_attempt']:
            with self.assertRaises(ValueError):
                self.run_receipt(lambda r, key=key: r['provenance'].update({key: 'wrong'}))

    def test_resume_event_cannot_be_backdated_before_pause(self):
        def mutate(receipt):
            scenario = receipt['scenarios'][-1]
            for event in scenario['events']:
                if event.get('keysym') == 0xff0d:
                    event['after_sequence'] = scenario['before']['sequence']
                    event['before_sequence'] = scenario['paused']['sequence']
        with self.assertRaises(ValueError):
            self.run_receipt(mutate)

    def test_unrelated_old_key_event_cannot_prove_independent_action(self):
        def mutate(receipt):
            event = receipt['scenarios'][1]['events'][0]
            event['after_sequence'] = receipt['scenarios'][0]['before']['sequence']
            event['before_sequence'] = receipt['scenarios'][0]['after']['sequence']
        with self.assertRaises(ValueError):
            self.run_receipt(mutate)

    def test_cannot_reuse_old_reset_as_later_focus_proof(self):
        def mutate(receipt):
            receipt['scenarios'][-1]['resumed'] = receipt['scenarios'][-2]['resumed']
        with self.assertRaises(ValueError):
            self.run_receipt(mutate)


class IndependentEditorScenarioReview(unittest.TestCase):
    """Synthetic scenario semantics only. Inventory patched, never native evidence."""
    def fixture(self):
        ticks = [dict(tick=i, state=copy.deepcopy(gate.INITIAL), checksum=f'0x{i:016x}', **({'input': dict(dx=0, dy=0, actions=0)} if i else {})) for i in range(7)]
        def obs(tick, phase):
            return dict(sequence=tick, scene_generation=0, tick=tick,
                        state=copy.deepcopy(ticks[tick]['state']), phase=phase,
                        cleanup_pending=False, focused=True, raw_keys=[])
        before, claiming, active, after = [obs(t, phase) for t, phase in
                                          [(1, 'Off'), (2, 'Claiming'), (3, 'Active'), (6, 'Active')]]
        claiming['raw_keys'] = [dict(physical_key='Space', pressed=True,
                                    ctrl=False, alt=False, command=False)]
        scenario = dict(name='claiming_clear', before=before, claiming=claiming,
                        active=active, after=after, event_end=2)
        events = [dict(sequence=0, operation='take_control_click', after_observation=1, x=20, y=20),
                  dict(sequence=1, operation='key', keysym=0x20, pressed=True, after_observation=1)]
        return dict(scenarios=[scenario], events=events), [before, claiming, active, after], [ticks]

    def check(self, receipt, observations, timelines):
        with mock.patch.object(gate, 'EDITOR_SCENARIOS', ['claiming_clear']):
            gate.validate_editor_scenarios(receipt, observations, timelines)

    def scene_fixture(self):
        neutral = dict(x=0, y=0, counts=[0, 0, 0, 0], held=0)
        held = dict(x=0, y=0, counts=[1, 1, 1, 1], held=15)
        old = [dict(tick=0, state=neutral), dict(tick=1, state=held, input=dict(dx=0, dy=0, actions=15))]
        new = [dict(tick=0, state=neutral), dict(tick=1, state=neutral, input=dict(dx=0, dy=0, actions=0))]
        before = dict(sequence=1, scene_generation=0, tick=1, state=held, phase='Active')
        reset = dict(sequence=2, scene_generation=1, tick=0, state=neutral, phase='Off')
        after = dict(sequence=3, scene_generation=1, tick=1, state=neutral, phase='Active')
        scenario = dict(name='scene_reset', before=before, reset=reset, after=after, event_end=1,
                        boundary='supported_stop_save_replay_open_path_play', scene_generation_is_wrapper_label=True)
        events = [dict(sequence=0, operation='take_control_click', after_observation=2, x=20, y=20)]
        return dict(scenarios=[scenario], events=events), [before, reset, after], [old, new]

    def check_scene(self, r, o, t):
        with mock.patch.object(gate, 'EDITOR_SCENARIOS', ['scene_reset']):
            gate.validate_editor_scenarios(r, o, t)

    def test_scene_reset_without_any_consumed_tick_is_not_proof(self):
        r, o, t = self.scene_fixture()
        o[-1]['tick'] = 0
        with self.assertRaises(ValueError):
            self.check_scene(r, o, t)

    def test_modified_space_cannot_stand_in_for_claiming_clear(self):
        r, o, t = self.fixture()
        o[1]['raw_keys'][0]['ctrl'] = True
        with self.assertRaises(ValueError):
            self.check(r, o, t)

    def test_scene_reset_has_real_new_generation_consumption(self):
        self.check_scene(*self.scene_fixture())

    def test_scene_epoch_flag_without_actual_generation_change_rejected(self):
        r, o, t = self.scene_fixture()
        o[1]['scene_generation'] = 0
        with self.assertRaises(ValueError):
            self.check_scene(r, o, t)

    def test_scene_reset_replay_cannot_hide_first_tick_action(self):
        r, o, t = self.scene_fixture()
        t[1][1]['input']['actions'] = 8
        with self.assertRaises(ValueError):
            self.check_scene(r, o, t)

    def test_unknown_or_extra_editor_event_fields_are_rejected(self):
        for mutate in [lambda e: e.update(passed=True),
                       lambda e: e.update(keysym=0x123456),
                       lambda e: e.update(after_observation=500)]:
            r, o, t = self.fixture()
            mutate(r['events'][1])
            with self.assertRaises(ValueError):
                self.check(r, o, t)

    def test_unbound_native_click_cannot_prove_reclaim(self):
        r, o, t = self.fixture()
        del r['events'][0]['after_observation']
        with self.assertRaises(ValueError):
            self.check(r, o, t)

    def test_old_claiming_window_cannot_be_reused_as_later_scenario(self):
        r, o, t = self.fixture()
        r['scenarios'].append(copy.deepcopy(r['scenarios'][0]))
        with mock.patch.object(gate, 'EDITOR_SCENARIOS', ['claiming_clear', 'claiming_clear']):
            with self.assertRaises(ValueError):
                gate.validate_editor_scenarios(r, o, t)

    def test_real_claiming_anchor_and_complete_neutral_trace(self):
        self.check(*self.fixture())

    def test_invented_claiming_not_in_observation_stream(self):
        r, o, t = self.fixture()
        r['scenarios'][0]['claiming'] = copy.deepcopy(o[1])
        r['scenarios'][0]['claiming']['sequence'] = 4
        with self.assertRaises(ValueError):
            self.check(r, o, t)

    def test_active_pass_flag_cannot_replace_actual_claiming_phase(self):
        r, o, t = self.fixture()
        o[1]['phase'] = 'Active'
        r['scenarios'][0]['claiming_observed'] = True
        with self.assertRaises(ValueError):
            self.check(r, o, t)

    def test_unobserved_consumed_press_between_neutral_snapshots(self):
        r, o, t = self.fixture()
        t[0][4]['input']['actions'] = 8
        with self.assertRaises(ValueError):
            self.check(r, o, t)

    def test_logical_key_without_physical_key_does_not_prove_claiming(self):
        r, o, t = self.fixture()
        o[1]['raw_keys'][0]['physical_key'] = None
        with self.assertRaises(ValueError):
            self.check(r, o, t)


if __name__ == '__main__':
    unittest.main()
