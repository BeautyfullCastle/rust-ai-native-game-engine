//! One-shot framing request for an already inspected current draft. No camera writes.
use super::*;
impl NavigationPanel {
    fn current_patrol_frame_report(
        &self,
        editor: &Editor,
        terrain: &TerrainPanel,
    ) -> Option<&super::super::patrol_routes::Report> {
        let (saved, result) = self.patrol.inspection.as_ref()?;
        let current = match self.patrol_inspection_context(editor, terrain) {
            Ok((pin, _)) => pin,
            Err(_) => {
                self.retire_patrol_route();
                return None;
            }
        };
        if saved != &current {
            self.retire_patrol_route();
        }
        if self.patrol.retired.get() {
            return None;
        }
        result.as_ref().ok()
    }

    pub(crate) fn has_patrol_frame_request(&self) -> bool {
        self.patrol.frame_request.is_some()
    }
    pub(crate) fn inspected_patrol_frame_points(
        &self,
        editor: &Editor,
        terrain: &TerrainPanel,
    ) -> Result<Vec<[f32; 3]>, String> {
        super::super::patrol_frame_points::points(
            self.current_patrol_frame_report(editor, terrain)
                .ok_or("Inspect a current patrol route before framing")?,
        )
    }
    pub(crate) fn set_patrol_frame_busy(&mut self, busy: bool) {
        self.patrol.frame_busy = busy;
        if busy {
            self.patrol.frame_request = None;
        }
    }
    pub(super) fn show_patrol_frame_controls(
        &mut self,
        ui: &mut Ui,
        editor: &Editor,
        terrain: &TerrainPanel,
    ) {
        let focused = ui.input(|i| {
            i.focused
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if !focused || self.current_patrol_frame_report(editor, terrain).is_none() {
            self.patrol.frame_request = None;
        }
        let ready = focused
            && !self.patrol.frame_busy
            && !self.patrol.order_busy
            && self.inspected_patrol_frame_points(editor, terrain).is_ok();
        if ui
            .add_enabled(ready, egui::Button::new("Frame inspected patrol route"))
            .clicked()
            && ready
        {
            if let Some((pin, Ok(report))) = &self.patrol.inspection {
                self.patrol.frame_request = Some((
                    pin.clone(),
                    report.clone(),
                    editor.game(),
                    editor.sim().epoch,
                ));
            }
        }
        ui.weak("Frames the current inspected route, including bends and its closing leg. This changes only the transient viewport camera.");
    }
    pub(crate) fn take_patrol_frame_request(
        &mut self,
        editor: &Editor,
        terrain: &TerrainPanel,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        let Some((pin, report, profile, host_epoch)) = self.patrol.frame_request.take() else {
            return Ok(None);
        };
        if self.patrol.frame_busy
            || self.patrol.order_busy
            || editor.game() != profile
            || editor.sim().epoch != host_epoch
        {
            return Err("Viewport is busy".into());
        }
        let current = self
            .current_patrol_frame_report(editor, terrain)
            .ok_or("Inspect a current patrol route before framing")?;
        if current != &report || self.patrol.inspection.as_ref().map(|(p, _)| p) != Some(&pin) {
            return Err("Inspected route changed before framing".into());
        }
        super::super::patrol_frame_points::points(current).map(Some)
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "patrol_frame_ui_tests.rs"]
mod ui_tests;
