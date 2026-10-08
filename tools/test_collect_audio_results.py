#!/usr/bin/env python3
"""Regression checks for the strict positive audio evidence parser."""
import json
import pathlib
import subprocess
import sys
import tempfile
root = pathlib.Path(__file__).parent
cases = json.loads((root / 'collect_audio_cases.json').read_text())['editor']
good = '\n'.join(f'test {name} ... ok' for name in cases) + f'\ntest result: ok. {len(cases)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n'
variants = [good, '', good.replace(cases[0], 'wrong'), good + good, good.replace(f'{len(cases)} passed', '0 passed'), good.replace('0 ignored', '1 ignored'), good.replace('... ok', '... FAILED', 1), good + 'SKIP: no device\n', good.rsplit('test result:', 1)[0]]
with tempfile.TemporaryDirectory() as directory:
    path = pathlib.Path(directory) / 'log'
    for index, value in enumerate(variants):
        path.write_text(value)
        result = subprocess.run([sys.executable, str(root / 'check_collect_audio_results.py'), 'editor', str(path)], capture_output=True)
        assert (result.returncode == 0) == (index == 0), (index, result.stdout, result.stderr)
print(f'{len(variants)} audio result-parser fixtures passed')
