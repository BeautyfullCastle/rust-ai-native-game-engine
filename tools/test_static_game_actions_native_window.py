"""Source regressions for private native window selection; not native evidence."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
EDITOR = ROOT / 'examples/static-game-starter/tests/multi_action_editor_x11.rs'
OBSERVER = ROOT / 'examples/static-game-starter/tests/support/standalone_x11.rs'


def check_title_order(source):
    ui = source.split('impl App for Wrapper {', 1)[1].split('fn editor_native_child()', 1)[0]
    production = ui.index('self.app.ui(ui, frame);')
    identity = ui.index('ctx.send_viewport_cmd(egui::ViewportCommand::Title(TITLE.into()));')
    assert production < identity, 'production UI must not overwrite private identity'


def check_child_ownership(source):
    spawn = source.index('let child = OwnedChild(')
    pid = source.index('let process_id = child.0.id();', spawn)
    lookup = source.index('let window = observer.window(process_id, TITLE)?;', pid)
    driver = source.index('let mut d = Driver {', lookup)
    assert spawn < pid < lookup < driver
    assert 'child,' in source[driver:source.index('let initial', driver)]


class EditorWindowSelectionContract(unittest.TestCase):
    def test_production_title_update_precedes_private_identity(self):
        check_title_order(EDITOR.read_text())

    def test_old_missing_title_override_is_rejected(self):
        source = EDITOR.read_text().replace(
            'ctx.send_viewport_cmd(egui::ViewportCommand::Title(TITLE.into()));', '')
        with self.assertRaises(ValueError):
            check_title_order(source)

    def test_title_override_before_production_ui_is_rejected(self):
        source = EDITOR.read_text()
        statement = 'ctx.send_viewport_cmd(egui::ViewportCommand::Title(TITLE.into()));'
        source = source.replace(statement, '').replace('self.app.ui(ui, frame);', statement + '\nself.app.ui(ui, frame);')
        with self.assertRaises(AssertionError):
            check_title_order(source)

    def test_child_cleanup_owned_before_fallible_window_lookup(self):
        check_child_ownership(EDITOR.read_text())
        source = EDITOR.read_text().replace('let child = OwnedChild(', 'let child = (')
        with self.assertRaises(ValueError):
            check_child_ownership(source)

    def test_pid_mapping_title_and_ambiguity_checks_preserved(self):
        source = OBSERVER.read_text()
        for clause in ('pid == 0 || title_prefix.is_empty()',
                       'attrs.map_state != MapState::VIEWABLE',
                       'attrs.class != WindowClass::INPUT_OUTPUT',
                       '!= Some(pid)', '.starts_with(title_prefix)',
                       'ambiguous player windows', 'player did not map its own window'):
            self.assertIn(clause, source)
        self.assertIn('Duration::from_secs(15)', source)


if __name__ == '__main__':
    unittest.main()
