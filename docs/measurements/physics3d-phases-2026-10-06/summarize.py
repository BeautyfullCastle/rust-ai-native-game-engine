"""Derive phase diagnostics from raw JSONL; never compare against Criterion."""
import hashlib
import json
from pathlib import Path
import statistics
import sys

root = Path(sys.argv[1]) if len(sys.argv) == 2 else Path(__file__).resolve().parent
raw_path = root / ('diagnostic.jsonl' if (root / 'diagnostic.jsonl').exists() else 'diagnostic.log')
rows = [json.loads(line) for line in raw_path.read_text().splitlines()]
assert rows[0]['kind'] == 'schema'
phases, stats_names = rows[0]['phases'], rows[0]['stats']
complete = rows[-1]
assert complete == {'kind': 'complete', 'cases': 8, 'instrumented_ticks': 1520,
                    'unprobed_validation_ticks': 1520, 'accumulator_self_check': True}
case_rows = [r for r in rows if r['kind'] == 'case']
assert len(case_rows) == 8
assert sum(r['kind'] == 'tick' for r in rows) == 1520
assert sum(r['kind'] == 'copy' for r in rows) == 52
expected = [('mixed_settling_500', 90, 200, 1, 100, 505),
            ('mixed_settling_1000', 90, 200, 1, 100, 1005),
            ('mixed_settled_500', 600, 200, 1, 100, 505),
            ('mixed_settled_1000', 600, 200, 1, 100, 1005),
            ('field_sleeping_500', 900, 200, 1, 100, 501),
            ('field_sleeping_1000', 900, 200, 1, 100, 1001),
            ('rollback8_mixed_settling_1000', 90, 20, 8, 8, 1005),
            ('rollback8_field_sleeping_1000', 900, 20, 8, 8, 1001)]

def describe(values):
    return {'sum': sum(values), 'mean': statistics.mean(values),
            'median': statistics.median(values), 'min': min(values), 'max': max(values)}

cal = [r for r in rows if r['kind'] == 'clock_calibration']
assert len(cal) == 1 and not cal[0]['subtracted']
assert len(cal[0]['empty_bracket_ns']) == 1000
result = {'raw_sha256': hashlib.sha256(raw_path.read_bytes()).hexdigest(),
          'units': 'nanoseconds',
          'interpretation': 'One instrumented invocation; phase fractions of summed timed step; copy measured separately; derived operation sums are not contiguous latency; no speedup claim',
          'clock_calibration': describe(cal[0]['empty_bracket_ns']), 'cases': []}
for case, spec in zip(case_rows, expected):
    name, warmup, reps, tpr, reset, body_count = spec
    assert case['name'] == name and case['warmup_ticks'] == warmup
    assert case['repetitions'] == reps and case['ticks_per_repetition'] == tpr
    assert case['reset_every'] == reset and case['instrumented_ticks'] == reps * tpr
    assert case['substeps'] == 8 and case['velocity_iterations'] == 1
    ticks = [r for r in rows if r['kind'] == 'tick' and r['case'] == name]
    copies = [r for r in rows if r['kind'] == 'copy' and r['case'] == name]
    validation = [r for r in rows if r['kind'] == 'validation' and r['case'] == name]
    assert len(ticks) == reps * tpr and len(copies) == len(ticks) // reset
    assert len(validation) == 1
    assert validation[0]['probed_unprobed_equal_ticks'] == len(ticks)
    assert validation[0]['reset_replay_equal_ticks'] == len(ticks)
    assert [c['before_index'] for c in copies] == list(range(0, len(ticks), reset))
    for i, tick in enumerate(ticks):
        assert tick['index'] == i and tick['tick'] == warmup + i % reset + 1
        assert tick['stats'][0] == body_count
        assert sum(tick['phase_ns']) + tick['tail_ns'] == tick['total_ns']
        assert len(tick['phase_calls']) == 9 and len(tick['phase_ns']) == 9
        first = ticks[i % reset]
        assert tick['checksum'] == first['checksum'] and tick['stats'] == first['stats']
        assert tick['phase_calls'] == first['phase_calls']
        for ns, calls in zip(tick['phase_ns'], tick['phase_calls']):
            assert calls > 0 or ns == 0
    total = sum(t['total_ns'] for t in ticks)
    copy_total = sum(c['ns'] for c in copies)
    row = dict(case)
    row.pop('kind')
    row['step_ns'] = describe([t['total_ns'] for t in ticks])
    row['phase_ns'] = {p: describe([t['phase_ns'][i] for t in ticks]) for i, p in enumerate(phases)}
    row['phase_percent_of_timed_step'] = {p: row['phase_ns'][p]['sum'] / total * 100 for p in phases}
    row['phase_calls'] = {p: describe([t['phase_calls'][i] for t in ticks]) for i, p in enumerate(phases)}
    row['tail_ns'] = describe([t['tail_ns'] for t in ticks])
    row['copy_ns'] = describe([c['ns'] for c in copies])
    row['copy_amortized_ns_per_tick'] = copy_total / len(ticks)
    row['derived_step_plus_copy_ns_per_operation'] = (total + copy_total) / reps
    row['copy_percent_of_derived_operation_sum'] = copy_total / (total + copy_total) * 100
    row['stats'] = {p: describe([t['stats'][i] for t in ticks]) for i, p in enumerate(stats_names)}
    row['first_tick_ns'] = ticks[0]['total_ns']
    row['final_checksum'] = ticks[-1]['checksum']
    row['checksum_sequence_sha256'] = hashlib.sha256(''.join(t['checksum'] + '\n' for t in ticks).encode()).hexdigest()
    row['validation'] = validation[0]
    result['cases'].append(row)
(root / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
print('case | step us/tick | broad % | narrow % | prepare % | solve % | finish % | gather % | copy us/copy (count) | awake | asleep')
for c in result['cases']:
    percent = c['phase_percent_of_timed_step']
    print(c['name'], '|', round(c['step_ns']['mean'] / 1000, 3), '|',
          ' | '.join(f'{percent[p]:.2f}' for p in ['broad', 'narrow', 'prepare', 'solve', 'finish', 'gather']),
          '|', round(c['copy_ns']['mean'] / 1000, 3), f"({c['copy_count']})", '|',
          f"{c['stats']['awake']['min']}..{c['stats']['awake']['max']}", '|',
          f"{c['stats']['asleep']['min']}..{c['stats']['asleep']['max']}")
print('Clock calibration:', result['clock_calibration'])
print('Validation: eight exact workloads, 1520 ticks, 52 copies, reset equality and complete marker confirmed')
