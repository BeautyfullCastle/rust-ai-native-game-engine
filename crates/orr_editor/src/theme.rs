//! The editor's look: "ink and ember". Near-black panels, high contrast text
//! and one warm accent, after the dense dark editors of large engines. Every
//! color, radius and spacing of the editor chrome comes from here; panels and
//! widgets take these values instead of their own.

use egui::{Color32, CornerRadius, Margin, Stroke, Ui};

/// Behind the viewport and the deepest wells.
pub const BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x15);
/// Side panels, the bottom panel and windows.
pub const PANEL: Color32 = Color32::from_rgb(0x1A, 0x1B, 0x1E);
/// The top bar and the status bar.
pub const BAR: Color32 = Color32::from_rgb(0x16, 0x17, 0x19);
/// A component card in the inspector.
pub const CARD: Color32 = Color32::from_rgb(0x1F, 0x21, 0x24);
/// The header strip of a card.
pub const CARD_HEADER: Color32 = Color32::from_rgb(0x26, 0x28, 0x2C);
/// Text fields and other inputs.
pub const INPUT: Color32 = Color32::from_rgb(0x0D, 0x0E, 0x10);
/// Buttons at rest, hovered and pressed.
pub const BUTTON: Color32 = Color32::from_rgb(0x26, 0x28, 0x2C);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x30, 0x33, 0x38);
pub const BUTTON_ACTIVE: Color32 = Color32::from_rgb(0x3A, 0x3D, 0x43);
/// Lines between panels, card borders and separators.
pub const LINE: Color32 = Color32::from_rgb(0x2C, 0x2E, 0x33);
/// Body text, strong text and secondary text.
pub const TEXT: Color32 = Color32::from_rgb(0xC8, 0xCA, 0xCF);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xF2, 0xF3, 0xF5);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8A, 0x8D, 0x94);
/// The one accent: selection, the main action and panel title underlines.
pub const ACCENT: Color32 = Color32::from_rgb(0xE8, 0xA3, 0x3D);
/// The accent behind selected text, 40 percent over the panel.
pub const ACCENT_FILL: Color32 = Color32::from_rgba_premultiplied(0x5D, 0x41, 0x18, 0x66);

/// Radius of widgets, and of windows and menus.
pub const RADIUS: u8 = 2;
pub const WINDOW_RADIUS: u8 = 4;

/// Applies the look to every style of `ctx`.
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
        v.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS);
        v.extreme_bg_color = INPUT;
        v.faint_bg_color = CARD;
        v.code_bg_color = CARD_HEADER;
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT_FILL;
        // Selected text is strong white on the amber fill.
        v.selection.stroke = Stroke::new(1.0, TEXT_STRONG);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.weak_bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.bg_fill = BUTTON;
        w.inactive.weak_bg_fill = BUTTON;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.bg_fill = BUTTON_HOVER;
        w.hovered.weak_bg_fill = BUTTON_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
        w.hovered.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
        w.active.bg_fill = BUTTON_ACTIVE;
        w.active.weak_bg_fill = BUTTON_ACTIVE;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(2.0, TEXT_STRONG);
        w.open.bg_fill = PANEL;
        w.open.weak_bg_fill = BUTTON;
        w.open.bg_stroke = Stroke::new(1.0, LINE);
        w.open.fg_stroke = Stroke::new(1.0, TEXT_STRONG);
        for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            state.corner_radius = CornerRadius::same(RADIUS);
        }
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(6.0, 3.0);
        s.button_padding = egui::vec2(6.0, 2.0);
        s.interact_size.y = 20.0;
        s.window_margin = Margin::same(8);
    });
}

/// The frame of the top bar and the status bar.
pub fn bar_frame() -> egui::Frame {
    egui::Frame::new().fill(BAR).inner_margin(Margin::symmetric(8, 4)).stroke(Stroke::new(1.0, LINE))
}

/// The frame of a side or bottom panel.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(PANEL).inner_margin(Margin::same(8)).stroke(Stroke::new(1.0, LINE))
}

/// A panel title: strong text with a short accent line under it.
pub fn panel_title(ui: &mut Ui, title: &str) {
    let response = ui.label(egui::RichText::new(title).strong().size(15.0).color(TEXT_STRONG));
    let y = response.rect.bottom() + 2.0;
    ui.painter().line_segment([egui::pos2(response.rect.left(), y), egui::pos2(response.rect.left() + 28.0, y)], Stroke::new(2.0, ACCENT));
    ui.add_space(6.0);
}

/// A small section title inside a panel, such as "Singletons".
pub fn section_title(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong().size(12.5).color(TEXT_WEAK));
}

/// The frame of an inspector card.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(RADIUS)).inner_margin(Margin::symmetric(6, 4)).outer_margin(Margin::symmetric(0, 2))
}

/// A type name with its path dimmed: `orr_gfx::` weak, `Material` strong. The
/// whole name stays the text of the widget, so it reads and searches as before.
pub fn type_name(name: &str) -> egui::text::LayoutJob {
    let (path, short) = match name.rfind("::") {
        Some(at) => name.split_at(at + 2),
        None => ("", name),
    };
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::proportional(13.5);
    job.append(path, 0.0, egui::TextFormat { font_id: font.clone(), color: TEXT_WEAK, ..Default::default() });
    job.append(short, 0.0, egui::TextFormat { font_id: font, color: TEXT_STRONG, ..Default::default() });
    job
}

/// A mark and its color for an entity with these components, for the hierarchy.
pub fn entity_mark(components: &[String]) -> (&'static str, Color32) {
    let has = |name: &str| components.iter().any(|c| c.ends_with(name));
    if has("Dragon") {
        ("\u{1F409}", Color32::from_rgb(0x6F, 0xC2, 0x8E))
    } else if has("::Light") {
        ("\u{1F4A1}", Color32::from_rgb(0xF2, 0xC9, 0x4C))
    } else if has("::Camera") {
        ("\u{1F4F7}", Color32::from_rgb(0x9B, 0xB7, 0xE0))
    } else if has("::Terrain") {
        ("\u{26F0}", Color32::from_rgb(0xB0, 0x9A, 0x6E))
    } else if has("::Water") {
        ("\u{1F4A7}", Color32::from_rgb(0x5D, 0xA9, 0xD6))
    } else if has("::Scatter") {
        ("\u{1F332}", Color32::from_rgb(0x6F, 0xB0, 0x5F))
    } else if has("::Body") || has("::Collider") {
        ("\u{25A3}", Color32::from_rgb(0xC8, 0xCA, 0xCF))
    } else {
        ("\u{25C6}", TEXT_WEAK)
    }
}
