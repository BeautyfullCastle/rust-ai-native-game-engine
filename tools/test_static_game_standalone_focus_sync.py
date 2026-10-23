"""Driver scheduling regressions only; these do not replace native acceptance."""
from pathlib import Path
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'examples/static-game-starter/tests/standalone_x11.rs'


def check_focus_sync(source):
    clear = source.split('fn clear_proof(', 1)[1].split('fn capture(', 1)[0]
    focus = clear.split('let stable_since = if focus_loss {', 1)[1].split('} else {', 1)[0]
    assert focus.index('observer.focus(window)?') < focus.index('obs.until(process')
    assert 'v["focused"] == true' in focus
    assert 'v["sequence"].as_u64() > paused["sequence"].as_u64()' in focus
    assert clear.index('paused_proof(obs, process, &paused, &stable_since)?') < clear.index('let resumed =')
    proof = source.split('fn paused_proof(', 1)[1].split('fn clear_proof(', 1)[0]
    assert 'let seq = number(before, "sequence")?' in proof
    assert 'let stable_seq = number(stable_since, "sequence")?' in proof
    assert 's >= stable_seq + 6' in proof
    assert '.filter(|v| v["sequence"].as_u64().is_some_and(|s| s > seq))' in proof
    for guard in ('v["paused"] != true', 'v["tick"] != before["tick"]',
                  'v["state"] != before["state"]', 'v["consumed"].as_array().is_none_or(|a| !a.is_empty())',
                  's >= stable_seq', 'v["focused"] != stable_since["focused"]'):
        assert guard in proof


class FocusSynchronization(unittest.TestCase):
    def test_actual_focus_receipt_anchors_stable_window(self):
        check_focus_sync(SOURCE.read_text())

    def test_regressions_fail_closed(self):
        source = SOURCE.read_text()
        mutations = [
            ('v["focused"] == true &&', 'true &&'),
            ('v["sequence"].as_u64() > paused["sequence"].as_u64()', 'true'),
            ('s >= stable_seq + 6', 's >= seq + 6'),
            ('s > seq)', 's > stable_seq)'),
            ('v["focused"] != stable_since["focused"]', 'false'),
            ('v["tick"] != before["tick"]', 'false'),
            ('v["state"] != before["state"]', 'false'),
            ('v["consumed"].as_array().is_none_or(|a| !a.is_empty())', 'false'),
        ]
        for old, new in mutations:
            with self.subTest(mutation=old), self.assertRaises(AssertionError):
                check_focus_sync(source.replace(old, new))


if __name__ == '__main__':
    unittest.main()
