//! One-shot framing request for an already inspected current draft. No camera writes.
use super::*;
fn points(report: &super::super::patrol_routes::Report) -> Result<Vec<[f32; 3]>, String> {
    const MAX: usize = 9 * (orr_navigation::MAX_TRIANGLES as usize + 1) + 9;
    let count = 1usize
        .checked_add(report.waypoints.len())
        .and_then(|n| {
            report
                .legs
                .iter()
                .try_fold(n, |n, leg| n.checked_add(leg.points.len()))
        })
        .ok_or("Route point count overflow")?;
    if count > MAX || report.legs.is_empty() || report.legs.len() > 9 {
        return Err("Inspected route point count is invalid".into());
    }
    let mut out = Vec::with_capacity(count);
    for point in std::iter::once(&report.start)
        .chain(report.waypoints.iter())
        .chain(report.legs.iter().flat_map(|leg| leg.points.iter()))
    {
        let point = super::super::point_to_f32(*point)?;
        out.push(point);
    }
    Ok(out)
}
impl Panel {
    fn current_patrol_frame_report(
        &self,
        editor: &Editor,
    ) -> Option<&super::super::patrol_routes::Report> {
        let (saved, result) = self.patrol.inspection.as_ref()?;
        let current = match self.patrol_inspection_context(editor) {
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
    ) -> Result<Vec<[f32; 3]>, String> {
        points(
            self.current_patrol_frame_report(editor)
                .ok_or("Inspect a current patrol route before framing")?,
        )
    }
    pub(crate) fn set_patrol_frame_busy(&mut self, busy: bool) {
        self.patrol.frame_busy = busy;
        if busy {
            self.patrol.frame_request = None;
        }
    }
    pub(super) fn show_patrol_frame_controls(&mut self, ui: &mut Ui, editor: &Editor) {
        let focused = ui.input(|i| {
            i.focused
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if !focused || self.current_patrol_frame_report(editor).is_none() {
            self.patrol.frame_request = None;
        }
        let ready = focused
            && !self.patrol.frame_busy
            && !self.patrol_pick_owns_input()
            && self.inspected_patrol_frame_points(editor).is_ok();
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
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        let Some((pin, report, profile, host_epoch)) = self.patrol.frame_request.take() else {
            return Ok(None);
        };
        if self.patrol.frame_busy
            || self.patrol_pick_owns_input()
            || editor.game() != profile
            || editor.sim().epoch != host_epoch
        {
            return Err("Viewport is busy".into());
        }
        let current = self
            .current_patrol_frame_report(editor)
            .ok_or("Inspect a current patrol route before framing")?;
        if current != &report || self.patrol.inspection.as_ref().map(|(p, _)| p) != Some(&pin) {
            return Err("Inspected route changed before framing".into());
        }
        points(current).map(Some)
    }
}
#[cfg(test)]
#[path = "patrol_frame_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "patrol_frame_ui_tests.rs"]
mod ui_tests;
