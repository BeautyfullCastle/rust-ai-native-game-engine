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

/// Play mode: the badge, the top bar outline and the Inspector cue.
pub const PLAY: Color32 = Color32::from_rgb(0xFF, 0x96, 0x3C);
/// Unsaved work and other states that need a look.
pub const WARN: Color32 = Color32::from_rgb(0xF0, 0xC8, 0x50);
/// A refused action. Readable on the panel and bar colors (see the contrast test).
pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
/// The name of the last action in the status bar.
pub const INFO: Color32 = Color32::from_rgb(0x78, 0xC8, 0xFF);

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
        // Section headers are full-width bars.
        v.collapsing_header_frame = true;
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
    set_fonts(ctx, egui::FontDefinitions::default());
}

/// Installs `fonts` with a system font that has Hangul and other CJK glyphs
/// added behind the fonts of every family. The built-in fonts have no such
/// glyphs, so those names would draw as empty boxes. A project font stays first
/// in its family. Every font change of the editor goes through this function.
/// Nothing is bundled and a missing system font changes nothing else.
pub fn set_fonts(ctx: &egui::Context, mut fonts: egui::FontDefinitions) {
    if let Some(bytes) = system_cjk_font() {
        fonts.font_data.insert("system-cjk".into(), egui::FontData::from_owned(bytes).into());
        for list in fonts.families.values_mut() {
            if !list.iter().any(|name| name == "system-cjk") {
                list.push("system-cjk".into());
            }
        }
    }
    ctx.set_fonts(fonts);
}

fn system_cjk_font() -> Option<Vec<u8>> {
    const MAX_FONT_BYTES: u64 = 48 << 20;
    let root = std::env::var_os("SystemRoot").map_or_else(|| std::path::PathBuf::from("C:\\Windows"), std::path::PathBuf::from);
    let candidates = [
        root.join("Fonts").join("malgun.ttf"),
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf".into(),
        "/System/Library/Fonts/AppleSDGothicNeo.ttc".into(),
    ];
    candidates.iter().find_map(|path| {
        let size = std::fs::metadata(path).ok()?.len();
        if size == 0 || size > MAX_FONT_BYTES {
            return None;
        }
        std::fs::read(path).ok()
    })
}

/// The frame of the top bar and the status bar.
pub fn bar_frame() -> egui::Frame {
    egui::Frame::new().fill(BAR).inner_margin(Margin::symmetric(8, 4)).stroke(Stroke::new(1.0, LINE))
}

/// The top bar while the simulation plays: the same bar with a [`PLAY`] outline.
pub fn play_bar_frame() -> egui::Frame {
    bar_frame().stroke(Stroke::new(2.0, PLAY))
}

/// The edit or play state as a badge: quiet while editing, filled with [`PLAY`] while playing.
pub fn mode_badge(ui: &mut Ui, playing: bool) -> egui::Response {
    let (text, text_color, fill, line, hint) = if playing {
        ("PLAY", BG, PLAY, PLAY, "Play mode: the simulation runs. Field edits are temporary and the scene file does not change. Press Stop to edit the scene.")
    } else {
        ("EDIT", TEXT_STRONG, CARD_HEADER, TEXT_STRONG.gamma_multiply(0.5), "Edit mode: changes go to the scene and can be undone and saved.")
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(text).strong().size(11.5).color(text_color)))
        .response
        .on_hover_text(hint)
}

/// The frame of a side or bottom panel.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(PANEL).inner_margin(Margin::same(8)).stroke(Stroke::new(1.0, LINE))
}

/// A panel title as a tab on a header strip across the top of the panel,
/// with the accent along the tab's top edge.
pub fn panel_title(ui: &mut Ui, title: &str) {
    panel_title_with(ui, title, None);
}

/// [`panel_title`] with a small state badge at the right end of the strip.
/// The badge is left out when the panel is too narrow for it.
pub fn panel_title_with(ui: &mut Ui, title: &str, badge: Option<(&str, Color32)>) {
    // A panel that shares a tab group already has its name on the tab.
    if ui.data(|data| data.get_temp::<bool>(crate::dock::tabbed_id())).unwrap_or(false) {
        return;
    }
    let margin = 8.0;
    let full = ui.max_rect();
    let top = ui.cursor().top() - margin;
    let strip = egui::Rect::from_min_max(egui::pos2(full.left() - margin, top), egui::pos2(full.right() + margin, top + 28.0));
    ui.painter().rect_filled(strip, CornerRadius::ZERO, BAR);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, LINE));
    let galley = ui.painter().layout_no_wrap(title.to_string(), egui::FontId::proportional(13.0), TEXT_STRONG);
    let tab = egui::Rect::from_min_size(egui::pos2(strip.left() + 6.0, strip.top() + 4.0), egui::vec2(galley.size().x + 24.0, strip.height() - 4.0));
    ui.painter().rect_filled(tab, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, PANEL);
    ui.painter().line_segment([tab.left_top() + egui::vec2(1.0, 0.5), tab.right_top() + egui::vec2(-1.0, 0.5)], Stroke::new(2.0, ACCENT));
    let text = egui::Rect::from_min_size(egui::pos2(tab.left() + 12.0, tab.center().y - galley.size().y / 2.0), galley.size());
    ui.put(text, egui::Label::new(egui::RichText::new(title).size(13.0).strong().color(TEXT_STRONG)).selectable(false));
    if let Some((label, color)) = badge {
        let size = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), color).size();
        let pill = egui::Rect::from_min_size(
            egui::pos2(strip.right() - 8.0 - size.x - 12.0, strip.center().y - size.y / 2.0 - 2.0),
            size + egui::vec2(12.0, 4.0),
        );
        if pill.left() > tab.right() + 6.0 {
            ui.painter().rect_stroke(pill, CornerRadius::same(RADIUS), Stroke::new(1.0, color), egui::StrokeKind::Inside);
            let inner = egui::Rect::from_min_size(pill.min + egui::vec2(6.0, 2.0), size);
            ui.put(inner, egui::Label::new(egui::RichText::new(label).size(11.0).color(color)).selectable(false));
        }
    }
    // The content starts under the strip.
    let below = strip.bottom() + 6.0 - ui.cursor().top();
    if below > 0.0 {
        ui.add_space(below);
    }
}

/// A heading inside a panel or a window.
pub fn heading(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    ui.label(egui::RichText::new(text.into()).strong().size(14.0).color(TEXT_STRONG))
}

/// A small section title inside a panel, such as "Singletons".
pub fn section_title(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong().size(12.5).color(TEXT_WEAK));
}

/// The fold mark of a section header: a bar at the left edge, amber while
/// the section is open and gray while it is closed. It is not an arrow; the
/// whole header is the control.
pub fn fold_mark(ui: &mut Ui, openness: f32, response: &egui::Response) {
    let rect = response.rect;
    let bar = egui::Rect::from_center_size(egui::pos2(rect.left() + 1.0, rect.center().y), egui::vec2(3.0, rect.height() + 4.0));
    let rest = if response.hovered() { TEXT_WEAK } else { LINE };
    let color = egui::lerp(egui::Rgba::from(rest)..=egui::Rgba::from(ACCENT), openness);
    ui.painter().rect_filled(bar, CornerRadius::same(1), Color32::from(color));
}

/// A section that folds: a full-width header bar with [`fold_mark`].
pub fn header(text: impl Into<egui::WidgetText>) -> egui::CollapsingHeader {
    egui::CollapsingHeader::new(text).icon(fold_mark)
}

/// `ui.collapsing` with the editor's section header.
pub fn collapsing<R>(ui: &mut Ui, heading: impl Into<egui::WidgetText>, add_contents: impl FnOnce(&mut Ui) -> R) -> egui::CollapsingResponse<R> {
    header(heading).show(ui, add_contents)
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

/// What an entity is, for the Inspector header, and the mark and color the Hierarchy shows for it.
fn entity_kind(components: &[String]) -> (&'static str, &'static str, Color32) {
    let has = |name: &str| components.iter().any(|c| c.ends_with(name));
    if has("Dragon") {
        ("Dragon", "\u{1F409}", Color32::from_rgb(0x6F, 0xC2, 0x8E))
    } else if has("::Light") {
        ("Light", "\u{1F4A1}", Color32::from_rgb(0xF2, 0xC9, 0x4C))
    } else if has("::Camera") {
        ("Camera", "\u{1F4F7}", Color32::from_rgb(0x9B, 0xB7, 0xE0))
    } else if has("::Terrain") {
        ("Terrain", "\u{26F0}", Color32::from_rgb(0xB0, 0x9A, 0x6E))
    } else if has("::Water") {
        ("Water", "\u{1F4A7}", Color32::from_rgb(0x5D, 0xA9, 0xD6))
    } else if has("::Scatter") {
        ("Scatter", "\u{1F332}", Color32::from_rgb(0x6F, 0xB0, 0x5F))
    } else if has("::Body") || has("::Collider") {
        ("Physics body", "\u{25A3}", Color32::from_rgb(0xC8, 0xCA, 0xCF))
    } else {
        ("Entity", "\u{25C6}", TEXT_WEAK)
    }
}

/// A mark and its color for an entity with these components, for the hierarchy.
pub fn entity_mark(components: &[String]) -> (&'static str, Color32) {
    let (_, mark, color) = entity_kind(components);
    (mark, color)
}

/// The kind of an entity with these components, as a word.
pub fn entity_role(components: &[String]) -> &'static str {
    entity_kind(components).0
}
