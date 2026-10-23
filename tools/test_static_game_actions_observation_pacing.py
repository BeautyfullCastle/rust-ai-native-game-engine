"""Producer scheduling source regressions, not native runtime evidence."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'examples/static-game-starter/src/standalone.rs'


def check_pacing(source):
    clock = source.split('mod view_clock {', 1)[1].split('\nfn mapping()', 1)[0]
    assert 'next_frame: Instant' in clock
    assert 'if now < self.next_frame' in clock
    assert 'return false;' in clock
    assert 'self.next_frame = now + FRAME_INTERVAL;' in clock
    assert 'Duration::from_millis(16)' in clock
    wait = source.split('fn about_to_wait(', 1)[1].split('\npub fn main_for', 1)[0]
    gate = wait.index('if self.clock.request_frame()')
    redraw = wait.index('gfx.window.request_redraw();')
    assert gate < redraw
    assert 'ControlFlow::WaitUntil(self.clock.next_frame())' in wait


class ObservationPacing(unittest.TestCase):
    def test_producer_retains_deadline_and_gates_redraw_requests(self):
        check_pacing(SOURCE.read_text())

    def test_unconditional_redraw_regression_rejected(self):
        source = SOURCE.read_text().replace('if self.clock.request_frame()', 'if true')
        with self.assertRaises(ValueError):
            check_pacing(source)

    def test_deadline_must_not_slide_on_every_wake(self):
        source = SOURCE.read_text().replace('if now < self.next_frame', 'if false')
        with self.assertRaises(AssertionError):
            check_pacing(source)

    def test_caps_and_all_presentations_remain_observed(self):
        source = SOURCE.read_text()
        self.assertIn('const OBSERVATION_LIMIT: usize = 64 * 1024 * 1024;', source)
        self.assertIn('.filter(|n| *n <= OBSERVATION_LIMIT)', source)
        self.assertIn('if self.pending.len() > 600', source)
        frame = source.split('fn frame(&mut self)', 1)[1].split('fn fail(', 1)[0]
        self.assertNotIn('request_frame', frame)
        self.assertIn('observer.write(&json!', frame)
        self.assertIn('"consumed":self.pending', frame)
        self.assertIn('self.pending.clear();', frame)
        self.assertIn('.checked_add(1)', frame)
        for name in ('standalone_x11.rs', 'multi_action_standalone_x11.rs'):
            reader = (ROOT / 'examples/static-game-starter/tests' / name).read_text()
            self.assertIn('> 32 * 1024 * 1024', reader)
            self.assertIn('file_type().is_symlink()', reader)
            self.assertIn('observation prefix was rewritten', reader)


if __name__ == '__main__':
    unittest.main()
