//! Painted controls modal using the HUD's existing validated pointer-click ownership.
use super::*;
impl DefenseUi {
    pub(super) fn controls_targets(
        &mut self,
        l: Layout,
        mut extra: Vec<(Rect, ClickTarget)>,
        modal: bool,
    ) -> (Vec<(Rect, ClickTarget)>, bool) {
        if self.controls.open {
            extra.clear();
            for index in 0..4 {
                extra.push((
                    l.rect(350.0, 350.0 + f32::from(index) * 52.0, 500.0, 40.0),
                    ClickTarget::Controls(index + 1),
                ));
            }
            for (id, x, width) in [
                (5, 350.0, 110.0),
                (6, 470.0, 110.0),
                (7, 590.0, 115.0),
                (8, 715.0, 130.0),
            ] {
                if id != 6 || self.controls.writable {
                    extra.push((l.rect(x, 610.0, width, 40.0), ClickTarget::Controls(id)));
                }
            }
            (extra, true)
        } else {
            if !modal {
                extra.push((l.rect(1040.0, 15.0, 120.0, 30.0), ClickTarget::Controls(0)));
            }
            (extra, modal)
        }
    }
    pub(super) fn paint_controls(
        &mut self,
        p: &egui::Painter,
        l: Layout,
        clicked: Option<ClickTarget>,
        modal: bool,
    ) -> bool {
        p.rect_filled(
            l.rect(1040.0, 15.0, 120.0, 30.0),
            5.0 * l.scale,
            Color32::from_rgb(49, 64, 61),
        );
        l.text(
            p,
            1060.0,
            30.0,
            12.0,
            if !modal { PAPER } else { MUTED },
            "KEYS",
        );
        if clicked == Some(ClickTarget::Controls(0)) {
            self.controls.open_panel();
            if self.started {
                self.pause();
            }
            self.pointer_clicks.press = None;
        }
        if !self.controls.open {
            return false;
        }
        let capturing = self.controls.waiting.is_some();
        match clicked {
            Some(ClickTarget::Controls(id @ 1..=4)) => self.controls.arm(usize::from(id - 1)),
            Some(ClickTarget::Controls(5)) => {
                if let Err(e) = self.controls.apply() {
                    self.controls.status = e;
                }
            }
            Some(ClickTarget::Controls(6)) if self.controls.writable => {
                self.controls.save_requested = true
            }
            Some(ClickTarget::Controls(7)) => self.controls.reset_draft(),
            Some(ClickTarget::Controls(8)) => self.controls.close_panel(),
            _ if capturing => self.controls.capture(),
            _ if self.controls.escaped() => self.controls.close_panel(),
            _ => {}
        }
        if matches!(clicked, Some(ClickTarget::Controls(_)))
            || !self.controls.open
            || capturing != self.controls.waiting.is_some()
        {
            self.pointer_clicks.press = None;
        }
        p.rect_filled(
            l.rect(0.0, 90.0, 1200.0, 850.0),
            0.0,
            Color32::from_black_alpha(175),
        );
        p.rect_filled(l.rect(300.0, 240.0, 600.0, 530.0), 12.0 * l.scale, INK);
        l.text(p, 340.0, 280.0, 26.0, PAPER, "GAMEPLAY KEYS");
        l.text(
            p,
            340.0,
            315.0,
            13.0,
            MUTED,
            "Physical keys. P / Esc pause; Enter continues. These stay fixed.",
        );
        for (index, label) in ["SUMMON", "FORGE / UPGRADE", "INVEST", "RESTART"]
            .into_iter()
            .enumerate()
        {
            let y = 350.0 + index as f32 * 52.0;
            p.rect_filled(
                l.rect(350.0, y, 500.0, 40.0),
                5.0 * l.scale,
                Color32::from_rgb(43, 60, 57),
            );
            l.text(p, 365.0, y + 20.0, 15.0, PAPER, label);
            let hint = if self.controls.waiting == Some(index) {
                "PRESS KEY...".into()
            } else {
                self.controls.hint(index, true)
            };
            l.text(p, 680.0, y + 20.0, 15.0, TEAL, hint);
        }
        for (label, x, width) in [
            ("APPLY", 350.0, 110.0),
            ("SAVE", 470.0, 110.0),
            ("DEFAULTS", 590.0, 115.0),
            ("CLOSE", 715.0, 130.0),
        ] {
            p.rect_filled(
                l.rect(x, 610.0, width, 40.0),
                5.0 * l.scale,
                Color32::from_rgb(49, 64, 61),
            );
            l.text(
                p,
                x + 15.0,
                630.0,
                13.0,
                if label == "SAVE" && !self.controls.writable {
                    MUTED
                } else {
                    PAPER
                },
                label,
            );
        }
        l.text(
            p,
            340.0,
            690.0,
            12.0,
            MUTED,
            self.controls.status.chars().take(85).collect::<String>(),
        );
        l.text(
            p,
            340.0,
            725.0,
            12.0,
            MUTED,
            "Close discards the draft; applied session controls stay. Continue explicitly.",
        );
        if !self.controls.writable {
            l.text(
                p,
                340.0,
                746.0,
                12.0,
                MUTED,
                if self.controls.store.is_some() {
                    "Save disabled for this launch; existing settings are preserved."
                } else {
                    "Saving is not configured; changes are session-only."
                },
            );
        }
        true
    }
}
