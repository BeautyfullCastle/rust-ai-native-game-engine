#!/usr/bin/env python3
"""Fail closed on a missing, ignored, failed or duplicated audio acceptance case."""
import json
import pathlib
import re
import sys

inventory = json.loads(pathlib.Path(__file__).with_name('collect_audio_cases.json').read_text())
group, log = sys.argv[1:]
expected = inventory[group]
text = pathlib.Path(log).read_text()
actual = re.findall(r'^test ([A-Za-z0-9_:]+) \.\.\. ok$', text, re.M)
for case in expected:
    assert actual.count(case) == 1, (case, actual.count(case))
rows = re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text, re.M)
assert len(rows) == 1 and text.count('test result:') == 1, rows
assert rows[0] == (str(len(expected)), '0', '0'), rows
assert sorted(actual) == sorted(expected), actual
assert not re.search(r'(?im)^\s*(?:SKIP:|.*\bskipping\b)', text)
print(f'{group}: {len(expected)} exact positive cases')
