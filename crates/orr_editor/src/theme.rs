//! The editor's look: "ink and ember". Near-black panels, high contrast text
//! and one warm accent, after the dense dark editors of large engines. Every
//! color, radius, spacing step and text size of the editor chrome comes from
//! here; panels and widgets take these values instead of their own.
//!
//! The tokens, by group:
//!
//! - Surfaces, from the deepest to the highest: [`INPUT`] (wells), [`BG`]
//!   (behind the viewport), [`BAR`] (top bar, status bar, panel title strip),
//!   [`PANEL`], [`CARD`], [`CARD_HEADER`] and the three button states.
//! - Text: [`TEXT`], [`TEXT_STRONG`], [`TEXT_WEAK`]. [`ON_ACCENT`] is the text on
//!   an [`ACCENT`] or [`PLAY`] fill.
//! - The accent [`ACCENT`] marks selection, the main action and focus ([`FOCUS`]).
//! - States with a meaning: [`PLAY`], [`WARN`], [`ERROR`], [`INFO`].
//! - Lines: [`LINE`] between panels, [`CONTROL_EDGE`] around controls.
//! - Scales: [`space`] (spacing steps), [`text`] (three text sizes), [`RADIUS`]
//!   and [`WINDOW_RADIUS`] (two corner radii).
//! - Entity kinds: [`kind`].

use egui::{Color32, CornerRadius, Margin, Stroke, Ui};

use crate::icons::Icon;

/// Spacing steps in points. Gaps inside a group use [`space::SM`] or less, and
/// gaps between groups use [`space::LG`] or more.
pub mod space {
    pub const XS: f32 = 2.0;
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 12.0;
}

/// Text sizes in points. Body text is the egui default (13). A caption is a
/// badge or a hint line under a field. A title is a heading in a panel or a
/// window and the entity name in the Inspector.
pub mod text {
    pub const CAPTION: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 15.0;
}

/// The colors of the entity kinds, for their icons in the Hierarchy and the Inspector.
pub mod kind {
    use egui::Color32;

    pub const DRAGON: Color32 = Color32::from_rgb(0x6F, 0xC2, 0x8E);
    pub const LIGHT: Color32 = Color32::from_rgb(0xF2, 0xC9, 0x4C);
    pub const CAMERA: Color32 = Color32::from_rgb(0x9B, 0xB7, 0xE0);
    pub const TERRAIN: Color32 = Color32::from_rgb(0xC9, 0xB5, 0x8B);
    pub const WATER: Color32 = Color32::from_rgb(0x6D, 0xB6, 0xE0);
    pub const SCATTER: Color32 = Color32::from_rgb(0x7F, 0xC0, 0x6F);
}

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
/// The edge of a control (button, text field): 3 to 1 against the panel colors.
pub const CONTROL_EDGE: Color32 = Color32::from_rgb(0x6E, 0x71, 0x78);
/// Body text, strong text and secondary text. Secondary text reaches 4.5 to 1
/// on every surface that holds text, buttons included.
pub const TEXT: Color32 = Color32::from_rgb(0xC8, 0xCA, 0xCF);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xF2, 0xF3, 0xF5);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x9E, 0xA1, 0xA8);
/// The one accent: selection, the main action and panel title underlines.
pub const ACCENT: Color32 = Color32::from_rgb(0xE8, 0xA3, 0x3D);
/// The ring around a control that has keyboard focus.
pub const FOCUS: Color32 = ACCENT;
/// Text and icons on an [`ACCENT`] or [`PLAY`] fill.
pub const ON_ACCENT: Color32 = BG;
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
/// Height of a control and of a list row, in points.
pub const CONTROL_HEIGHT: f32 = 20.0;

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
        v.error_fg_color = ERROR;
        v.warn_fg_color = WARN;
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
        w.inactive.bg_stroke = Stroke::new(1.0, CONTROL_EDGE);
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
        s.item_spacing = egui::vec2(space::MD, space::XS);
        s.button_padding = egui::vec2(space::MD, space::XS);
        s.interact_size.y = CONTROL_HEIGHT;
        s.window_margin = Margin::same(space::LG as i8);
        // Three sizes: caption, body and title. The built-in Small and Heading sizes
        // (9 and 18) fall to the nearest of them.
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(text::CAPTION));
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(text::TITLE));
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
    egui::Frame::new().fill(BAR).inner_margin(Margin::symmetric(space::LG as i8, space::SM as i8)).stroke(Stroke::new(1.0, LINE))
}

/// The top bar while the simulation plays: the same bar with a [`PLAY`] outline.
pub fn play_bar_frame() -> egui::Frame {
    bar_frame().stroke(Stroke::new(2.0, PLAY))
}

/// The edit or play state as a badge: quiet while editing, filled with [`PLAY`] while playing.
pub fn mode_badge(ui: &mut Ui, playing: bool) -> egui::Response {
    let (label, text_color, fill, line, hint) = if playing {
        ("PLAY", ON_ACCENT, PLAY, PLAY, "Play mode: the simulation runs. Field edits are temporary and the scene file does not change. Press Stop to edit the scene.")
    } else {
        ("EDIT", TEXT_STRONG, CARD_HEADER, CONTROL_EDGE, "Edit mode: changes go to the scene and can be undone and saved.")
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(space::MD as i8, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(label).strong().size(text::CAPTION).color(text_color)))
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
    let margin = space::LG;
    let full = ui.max_rect();
    let top = ui.cursor().top() - margin;
    let strip = egui::Rect::from_min_max(egui::pos2(full.left() - margin, top), egui::pos2(full.right() + margin, top + 28.0));
    ui.painter().rect_filled(strip, CornerRadius::ZERO, BAR);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, LINE));
    let galley = ui.painter().layout_no_wrap(title.to_string(), egui::FontId::proportional(text::BODY), TEXT_STRONG);
    let tab = egui::Rect::from_min_size(egui::pos2(strip.left() + space::MD, strip.top() + space::SM), egui::vec2(galley.size().x + 2.0 * space::XL, strip.height() - space::SM));
    ui.painter().rect_filled(tab, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, PANEL);
    ui.painter().line_segment([tab.left_top() + egui::vec2(1.0, 0.5), tab.right_top() + egui::vec2(-1.0, 0.5)], Stroke::new(2.0, ACCENT));
    let label = egui::Rect::from_min_size(egui::pos2(tab.left() + space::XL, tab.center().y - galley.size().y / 2.0), galley.size());
    ui.put(label, egui::Label::new(egui::RichText::new(title).size(text::BODY).strong().color(TEXT_STRONG)).selectable(false));
    if let Some((badge_text, color)) = badge {
        let size = ui.painter().layout_no_wrap(badge_text.to_string(), egui::FontId::proportional(text::CAPTION), color).size();
        let pill = egui::Rect::from_min_size(
            egui::pos2(strip.right() - space::LG - size.x - space::XL, strip.center().y - size.y / 2.0 - space::XS),
            size + egui::vec2(space::XL, space::SM),
        );
        if pill.left() > tab.right() + space::MD {
            ui.painter().rect_stroke(pill, CornerRadius::same(RADIUS), Stroke::new(1.0, color), egui::StrokeKind::Inside);
            let inner = egui::Rect::from_min_size(pill.min + egui::vec2(space::MD, space::XS), size);
            ui.put(inner, egui::Label::new(egui::RichText::new(badge_text).size(text::CAPTION).color(color)).selectable(false));
        }
    }
    // The content starts under the strip.
    let below = strip.bottom() + space::MD - ui.cursor().top();
    if below > 0.0 {
        ui.add_space(below);
    }
}

/// A heading inside a panel or a window.
pub fn heading(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    ui.label(egui::RichText::new(text.into()).strong().size(text::TITLE).color(TEXT_STRONG))
}

/// A small section title inside a panel, such as "Singletons". The space above
/// it is at least twice the space between it and its first line.
pub fn section_title(ui: &mut Ui, title: &str) {
    ui.add_space(space::LG);
    ui.label(egui::RichText::new(title).strong().size(text::BODY).color(TEXT_WEAK));
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
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(RADIUS)).inner_margin(Margin::symmetric(space::MD as i8, space::SM as i8)).outer_margin(Margin::symmetric(0, space::XS as i8))
}

/// A type name with its path dimmed: `orr_gfx::` weak, `Material` strong. The
/// whole name stays the text of the widget, so it reads and searches as before.
pub fn type_name(name: &str) -> egui::text::LayoutJob {
    let (path, short) = match name.rfind("::") {
        Some(at) => name.split_at(at + 2),
        None => ("", name),
    };
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::proportional(text::BODY);
    job.append(path, 0.0, egui::TextFormat { font_id: font.clone(), color: TEXT_WEAK, ..Default::default() });
    job.append(short, 0.0, egui::TextFormat { font_id: font, color: TEXT_STRONG, ..Default::default() });
    job
}

/// What an entity is, for the Inspector header, and the icon and color the Hierarchy shows for it.
fn entity_kind(components: &[String]) -> (&'static str, Icon, Color32) {
    let has = |name: &str| components.iter().any(|c| c.ends_with(name));
    if has("Dragon") {
        ("Dragon", Icon::Dragon, kind::DRAGON)
    } else if has("::Light") {
        ("Light", Icon::Light, kind::LIGHT)
    } else if has("::Camera") {
        ("Camera", Icon::Camera, kind::CAMERA)
    } else if has("::Terrain") {
        ("Terrain", Icon::Terrain, kind::TERRAIN)
    } else if has("::Water") {
        ("Water", Icon::Water, kind::WATER)
    } else if has("::Scatter") {
        ("Scatter", Icon::Scatter, kind::SCATTER)
    } else if has("::Body") || has("::Collider") {
        ("Physics body", Icon::Body, TEXT)
    } else {
        ("Entity", Icon::Entity, TEXT_WEAK)
    }
}

/// The icon and its color for an entity with these components, for the hierarchy.
pub fn entity_mark(components: &[String]) -> (Icon, Color32) {
    let (_, icon, color) = entity_kind(components);
    (icon, color)
}

/// The icon of an entity kind that has no components to look at (an Inspector
/// header whose row is not known yet).
pub const UNKNOWN_ENTITY_MARK: (Icon, Color32) = (Icon::Entity, TEXT_WEAK);

/// The kind of an entity with these components, as a word.
pub fn entity_role(components: &[String]) -> &'static str {
    entity_kind(components).0
}
