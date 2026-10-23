"""Harness source regressions and a synthetic frame model, not native evidence."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'examples/static-game-starter/tests/native_window.rs'


def arm(source, stage):
    return re.search(rf'^                {stage} if .*?(?=^                (?:\d+ if|_ =>))',
                     source, re.MULTILINE | re.DOTALL).group()


def check_settle(source):
    restart = arm(source, 9)
    assert 'self.app.editor.stop();' in restart
    assert 'self.stage(26);' in restart
    assert 'self.click(self.play);' not in restart
    for stage in (12, 17, 26):
        block = arm(source, stage)
        assert '!self.app.editor.sim().playing' in block
        assert 'self.app.frame_count() > self.stage_at + 1' in block
        assert block.count('self.click(self.play);') == 1
    for stage in (12, 17):
        assert 'self.app.editor.timeline().is_some_and(|timeline| !timeline.playing)' in arm(source, stage)
    assert 'self.released()' in arm(source, 26)
    assert 'self.stage(21);' in arm(source, 26)


class NativeUiSettleTests(unittest.TestCase):
    def test_play_waits_for_a_post_transition_ui_frame(self):
        check_settle(SOURCE.read_text())

    def test_immediate_restart_click_is_rejected(self):
        source = SOURCE.read_text().replace('self.stage(26);',
                    'self.click(self.play);\n                    self.stage(21);')
        with self.assertRaises(AssertionError):
            check_settle(source)

    def test_pause_settle_guards_are_required_independently(self):
        for stage in (12, 17, 26):
            with self.subTest(stage=stage):
                source = SOURCE.read_text()
                block = arm(source, stage)
                mutant = block.replace('self.app.frame_count() > self.stage_at + 1', 'true')
                with self.assertRaises(AssertionError):
                    check_settle(source.replace(block, mutant))

    def test_paused_sim_alone_is_not_rendered_play_readiness(self):
        for stage in (12, 17):
            with self.subTest(stage=stage):
                source = SOURCE.read_text()
                block = arm(source, stage)
                mutant = block.replace('self.app.editor.timeline().is_some_and(|timeline| !timeline.playing)', 'true')
                with self.assertRaises(AssertionError):
                    check_settle(source.replace(block, mutant))

    def test_budget_and_required_evidence_are_preserved(self):
        source = SOURCE.read_text()
        assert 'if self.app.frame_count() > 1800 {' in source
        for proof in ('viewport_pick', 'take_control_ui', 'keyboard_win', 'space_restart',
                      'escape_neutral', 'focus_neutral', 'no_auto_reclaim',
                      'focus_regain_no_auto_reclaim', 'reopen_identity'):
            self.assertIn('"' + proof + '"', source)
        for phase in ('edit', 'won', 'neutral', 'reopened'):
            self.assertIn('self.capture(&ctx, "' + phase + '");', source)
        self.assertIn('self.app.ui(ui, frame);', source)
        self.assertIn('input.events.append(&mut self.queued);', source)
        self.assertIn('self.save_capture(phase, metadata, &image);', source)
        self.assertIn('native selected: ORR_REQUIRE_NATIVE_EDITOR=1 required, never skipped', source)

    def test_previous_frame_interaction_model(self):
        # Model the documented egui interaction order only. This is not an egui run.
        def accepted_click(wait_frames):
            previous_enabled = False  # UI frame before stop/pause
            playing = False           # transition after the disabled Play was laid out
            accepted = False
            for frame in range(wait_frames + 1):
                click = frame == wait_frames
                accepted = accepted or (click and previous_enabled and not playing)
                previous_enabled = not playing
            return accepted
        self.assertFalse(accepted_click(0))
        self.assertTrue(accepted_click(1))
        self.assertTrue(accepted_click(2))


if __name__ == '__main__':
    unittest.main()
