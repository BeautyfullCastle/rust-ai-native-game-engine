//! The editor's own icons, drawn with the egui painter from a few lines, circles
//! and polygons on a 16 by 16 grid. There are no image files, no SVG files and no
//! icon font. Every icon uses one stroke weight and one color that the caller
//! picks, so an icon follows the text color of its state. An icon never stands
//! alone without a name: [`IconButton`] always carries its label as the
//! accessible name and as the tooltip when the text is hidden, and a decorative
//! icon sits next to text that says the same thing.

use egui::epaint::PathShape;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, Vec2, Widget, WidgetInfo, WidgetType};

use crate::theme;

/// Side of an icon in points when text of the body size sits next to it.
pub const ICON_SIZE: f32 = 14.0;

/// Grid size the icon coordinates use.
const GRID: f32 = 16.0;
/// Stroke weight on the grid. Every icon shares it.
const STROKE: f32 = 1.5;

/// Every icon of the editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Pause,
    Step,
    Rewind,
    Stop,
    Save,
    Undo,
    Redo,
    Eye,
    EyeOff,
    Lock,
    Folder,
    Entity,
    Warning,
    Error,
    Info,
    Layout,
    Light,
    Camera,
    Terrain,
    Water,
    Scatter,
    Body,
    Dragon,
}

impl Icon {
    /// All icons, in the order of the enum.
    pub const ALL: [Icon; 24] = [
        Icon::Play,
        Icon::Pause,
        Icon::Step,
        Icon::Rewind,
        Icon::Stop,
        Icon::Save,
        Icon::Undo,
        Icon::Redo,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Lock,
        Icon::Folder,
        Icon::Entity,
        Icon::Warning,
        Icon::Error,
        Icon::Info,
        Icon::Layout,
        Icon::Light,
        Icon::Camera,
        Icon::Terrain,
        Icon::Water,
        Icon::Scatter,
        Icon::Body,
        Icon::Dragon,
    ];

    /// The plain name of the icon, for tooltips and the accessible name.
    pub fn name(self) -> &'static str {
        match self {
            Icon::Play => "Play",
            Icon::Pause => "Pause",
            Icon::Step => "Step",
            Icon::Rewind => "Rewind",
            Icon::Stop => "Stop",
            Icon::Save => "Save",
            Icon::Undo => "Undo",
            Icon::Redo => "Redo",
            Icon::Eye => "Shown",
            Icon::EyeOff => "Hidden",
            Icon::Lock => "Locked",
            Icon::Folder => "Folder",
            Icon::Entity => "Entity",
            Icon::Warning => "Warning",
            Icon::Error => "Error",
            Icon::Info => "Information",
            Icon::Layout => "Layout",
            Icon::Light => "Light",
            Icon::Camera => "Camera",
            Icon::Terrain => "Terrain",
            Icon::Water => "Water",
            Icon::Scatter => "Scatter",
            Icon::Body => "Physics body",
            Icon::Dragon => "Dragon",
        }
    }

    /// Draws the icon inside `rect` (the largest centered square of it).
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        painter.extend(self.shapes(rect, color));
    }

    /// The shapes of the icon. All of them lie inside `rect`.
    pub fn shapes(self, rect: Rect, color: Color32) -> Vec<Shape> {
        let side = rect.width().min(rect.height());
        let s = side / GRID;
        let origin = rect.center() - Vec2::splat(side / 2.0);
        let g = |x: f32, y: f32| origin + Vec2::new(x, y) * s;
        let w = (STROKE * s).max(1.0);
        let stroke = Stroke::new(w, color);
        let mut out: Vec<Shape> = Vec::new();
        let line = |out: &mut Vec<Shape>, points: &[(f32, f32)]| out.push(Shape::line(points.iter().map(|&(x, y)| g(x, y)).collect(), stroke));
        let closed = |out: &mut Vec<Shape>, points: &[(f32, f32)]| out.push(Shape::closed_line(points.iter().map(|&(x, y)| g(x, y)).collect(), stroke));
        let solid = |out: &mut Vec<Shape>, points: &[(f32, f32)]| {
            out.push(Shape::Path(PathShape::convex_polygon(points.iter().map(|&(x, y)| g(x, y)).collect(), color, Stroke::NONE)));
        };
        let fill_rect = |out: &mut Vec<Shape>, x0: f32, y0: f32, x1: f32, y1: f32, radius: u8| {
            out.push(Shape::rect_filled(Rect::from_min_max(g(x0, y0), g(x1, y1)), CornerRadius::same(radius), color));
        };
        let outline_rect = |out: &mut Vec<Shape>, x0: f32, y0: f32, x1: f32, y1: f32, radius: u8| {
            out.push(Shape::rect_stroke(Rect::from_min_max(g(x0, y0), g(x1, y1)), CornerRadius::same(radius), stroke, StrokeKind::Middle));
        };
        let ring = |out: &mut Vec<Shape>, x: f32, y: f32, r: f32| out.push(Shape::circle_stroke(g(x, y), r * s, stroke));
        let dot = |out: &mut Vec<Shape>, x: f32, y: f32, r: f32| out.push(Shape::circle_filled(g(x, y), r * s, color));
        match self {
            Icon::Play => solid(&mut out, &[(4.0, 2.5), (13.0, 8.0), (4.0, 13.5)]),
            Icon::Pause => {
                fill_rect(&mut out, 3.5, 2.5, 6.5, 13.5, 0);
                fill_rect(&mut out, 9.5, 2.5, 12.5, 13.5, 0);
            }
            Icon::Step => {
                solid(&mut out, &[(3.0, 2.5), (10.5, 8.0), (3.0, 13.5)]);
                fill_rect(&mut out, 11.5, 2.5, 13.5, 13.5, 0);
            }
            Icon::Rewind => {
                fill_rect(&mut out, 2.5, 2.5, 4.5, 13.5, 0);
                solid(&mut out, &[(13.0, 2.5), (5.5, 8.0), (13.0, 13.5)]);
            }
            Icon::Stop => fill_rect(&mut out, 3.5, 3.5, 12.5, 12.5, 1),
            Icon::Save => {
                outline_rect(&mut out, 2.5, 2.5, 13.5, 13.5, 1);
                line(&mut out, &[(5.0, 2.5), (5.0, 6.0), (11.0, 6.0), (11.0, 2.5)]);
                outline_rect(&mut out, 5.0, 9.5, 11.0, 13.5, 0);
            }
            Icon::Undo | Icon::Redo => {
                let flip = self == Icon::Redo;
                let x = |v: f32| if flip { GRID - v } else { v };
                let mut arc: Vec<(f32, f32)> = vec![(x(4.5), 4.5)];
                for step in 0..=8 {
                    let angle = (-90.0 + 180.0 * step as f32 / 8.0).to_radians();
                    arc.push((x(8.5 + 4.5 * angle.cos()), 9.0 + 4.5 * angle.sin()));
                }
                arc.push((x(4.5), 13.5));
                line(&mut out, &arc);
                line(&mut out, &[(x(6.5), 2.5), (x(4.0), 4.5), (x(6.5), 6.5)]);
            }
            Icon::Eye | Icon::EyeOff => {
                let mut lid: Vec<(f32, f32)> = Vec::new();
                for step in 0..=12 {
                    let t = step as f32 / 12.0;
                    lid.push((1.5 + 13.0 * t, 8.0 - 4.5 * (std::f32::consts::PI * t).sin()));
                }
                for step in (1..12).rev() {
                    let t = step as f32 / 12.0;
                    lid.push((1.5 + 13.0 * t, 8.0 + 4.5 * (std::f32::consts::PI * t).sin()));
                }
                closed(&mut out, &lid);
                dot(&mut out, 8.0, 8.0, 2.2);
                if self == Icon::EyeOff {
                    line(&mut out, &[(3.0, 13.5), (13.0, 2.5)]);
                }
            }
            Icon::Lock => {
                outline_rect(&mut out, 3.5, 7.0, 12.5, 14.0, 1);
                line(&mut out, &[(5.5, 7.0), (5.5, 5.0), (6.5, 3.0), (9.5, 3.0), (10.5, 5.0), (10.5, 7.0)]);
                dot(&mut out, 8.0, 10.5, 1.1);
            }
            Icon::Folder => closed(&mut out, &[(2.0, 3.5), (6.0, 3.5), (7.5, 5.5), (14.0, 5.5), (14.0, 12.5), (2.0, 12.5)]),
            Icon::Entity => {
                closed(&mut out, &[(8.0, 2.0), (14.0, 8.0), (8.0, 14.0), (2.0, 8.0)]);
                dot(&mut out, 8.0, 8.0, 1.6);
            }
            Icon::Warning => {
                closed(&mut out, &[(8.0, 2.0), (14.5, 13.5), (1.5, 13.5)]);
                line(&mut out, &[(8.0, 6.0), (8.0, 9.5)]);
                dot(&mut out, 8.0, 11.6, 0.9);
            }
            Icon::Error => {
                ring(&mut out, 8.0, 8.0, 6.5);
                line(&mut out, &[(5.5, 5.5), (10.5, 10.5)]);
                line(&mut out, &[(10.5, 5.5), (5.5, 10.5)]);
            }
            Icon::Info => {
                ring(&mut out, 8.0, 8.0, 6.5);
                dot(&mut out, 8.0, 5.0, 1.0);
                line(&mut out, &[(8.0, 7.5), (8.0, 11.5)]);
            }
            Icon::Layout => {
                outline_rect(&mut out, 2.0, 2.5, 14.0, 13.5, 1);
                line(&mut out, &[(6.5, 2.5), (6.5, 13.5)]);
                line(&mut out, &[(6.5, 8.0), (14.0, 8.0)]);
            }
            Icon::Light => {
                ring(&mut out, 8.0, 6.5, 4.5);
                line(&mut out, &[(6.0, 12.0), (10.0, 12.0)]);
                line(&mut out, &[(6.8, 14.0), (9.2, 14.0)]);
            }
            Icon::Camera => {
                outline_rect(&mut out, 2.0, 5.0, 14.0, 13.0, 1);
                line(&mut out, &[(5.5, 5.0), (6.5, 3.0), (9.5, 3.0), (10.5, 5.0)]);
                ring(&mut out, 8.0, 9.0, 2.2);
            }
            Icon::Terrain => closed(&mut out, &[(2.5, 12.5), (6.0, 5.0), (9.0, 10.0), (10.5, 8.0), (13.5, 12.5)]),
            Icon::Water => {
                closed(&mut out, &[(8.0, 2.0), (11.5, 7.5), (12.3, 10.0), (10.8, 12.8), (8.0, 14.0), (5.2, 12.8), (3.7, 10.0), (4.5, 7.5)]);
            }
            Icon::Scatter => {
                closed(&mut out, &[(8.0, 2.0), (12.5, 10.0), (3.5, 10.0)]);
                line(&mut out, &[(8.0, 10.0), (8.0, 14.0)]);
            }
            Icon::Body => {
                outline_rect(&mut out, 3.0, 3.0, 13.0, 13.0, 1);
                fill_rect(&mut out, 6.0, 6.0, 10.0, 10.0, 0);
            }
            Icon::Dragon => closed(&mut out, &[(2.0, 12.5), (5.0, 3.0), (8.0, 8.0), (11.0, 3.0), (14.0, 12.5), (8.0, 10.5)]),
        }
        out
    }
}

/// Draws a decorative icon of `size` points and returns its response. The
/// caller puts text next to it that says the same thing. The icon only shows its
/// name as a tooltip.
pub fn icon(ui: &mut Ui, icon: Icon, color: Color32, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        icon.paint(ui.painter(), rect, color);
    }
    response
}

/// A line of a notice or a status: the icon in `color`, then the text in `color`.
pub fn notice_line(ui: &mut Ui, icon_kind: Icon, color: Color32, text: impl Into<String>) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::space::SM;
        icon(ui, icon_kind, color, ICON_SIZE);
        ui.label(egui::RichText::new(text.into()).color(color))
    })
    .inner
}

/// Where the icon of a button sits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// At the left of the label.
    Leading(Icon),
    /// At the right end of the row: a menu row whose text keeps the left edge.
    Trailing(Icon),
    /// No icon, but the room of one, so a menu row lines up with the rows that have an icon.
    Blank,
}

/// A button that shows an icon and, unless hidden, its label. The label is the
/// accessible name in both cases.
pub struct IconButton<'a> {
    slot: Slot,
    label: &'a str,
    primary: bool,
    menu: bool,
    show_label: bool,
    shortcut: Option<&'a str>,
    selected: Option<bool>,
}

impl<'a> IconButton<'a> {
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self { slot: Slot::Leading(icon), label, primary: false, menu: false, show_label: true, shortcut: None, selected: None }
    }

    /// A menu row without an icon that lines up with the rows that have one.
    pub fn plain(label: &'a str) -> Self {
        Self { slot: Slot::Blank, label, primary: false, menu: true, show_label: true, shortcut: None, selected: None }
    }

    /// The main action of its bar: filled with the accent while it is enabled.
    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    /// A row of a menu: no frame at rest, the full width of the menu.
    pub fn menu(mut self) -> Self {
        self.menu = true;
        self
    }

    /// Puts the icon at the right end of the row.
    pub fn icon_trailing(mut self) -> Self {
        if let Slot::Leading(icon) = self.slot {
            self.slot = Slot::Trailing(icon);
        }
        self
    }

    /// Shows only the icon. The label stays as the tooltip and the accessible name.
    pub fn icon_only(mut self) -> Self {
        self.show_label = false;
        self
    }

    /// The shortcut text at the right end of a menu row.
    pub fn shortcut(mut self, text: &'a str) -> Self {
        self.shortcut = Some(text);
        self
    }

    /// Makes the button a toggle with this state. The state is the accessible state too.
    pub fn toggled(mut self, on: bool) -> Self {
        self.selected = Some(on);
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let font = TextStyle::Button.resolve(ui.style());
        let pad = ui.spacing().button_padding;
        let gap = theme::space::SM;
        let galley = self.show_label.then(|| ui.painter().layout_no_wrap(self.label.to_owned(), font.clone(), Color32::PLACEHOLDER));
        let shortcut = self.shortcut.map(|text| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER));
        let lead = matches!(self.slot, Slot::Leading(_) | Slot::Blank);
        let mut width = pad.x * 2.0;
        let mut height = ICON_SIZE;
        if lead || !self.show_label {
            width += ICON_SIZE;
        }
        if let Some(galley) = &galley {
            if lead {
                width += gap;
            }
            width += galley.size().x;
            height = height.max(galley.size().y);
        }
        if matches!(self.slot, Slot::Trailing(_)) && self.show_label {
            width += gap + ICON_SIZE;
        }
        if let Some(shortcut) = &shortcut {
            width += theme::space::XL * 2.0 + shortcut.size().x;
        }
        let height = (height + pad.y * 2.0).max(ui.spacing().interact_size.y);
        if self.menu {
            width = width.max(ui.available_width());
        }
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
        let enabled = ui.is_enabled();
        let selected = self.selected;
        let label = self.label;
        response.widget_info(|| match selected {
            Some(on) => WidgetInfo::selected(WidgetType::Checkbox, enabled, on, label),
            None => WidgetInfo::labeled(WidgetType::Button, enabled, label),
        });
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);
            let accent = self.primary && enabled;
            let (fill, text_color) = if accent {
                (theme::ACCENT, theme::ON_ACCENT)
            } else if self.menu && !response.hovered() && !response.has_focus() {
                (Color32::TRANSPARENT, visuals.fg_stroke.color)
            } else {
                (visuals.weak_bg_fill, visuals.fg_stroke.color)
            };
            let stroke = if self.menu && !response.hovered() { Stroke::NONE } else { visuals.bg_stroke };
            let painter = ui.painter();
            painter.rect(rect, visuals.corner_radius, fill, stroke, StrokeKind::Inside);
            if response.has_focus() {
                painter.rect_stroke(rect, visuals.corner_radius, Stroke::new(2.0, if accent { theme::TEXT_STRONG } else { theme::FOCUS }), StrokeKind::Inside);
            }
            let icon_color = if self.selected.unwrap_or(true) { text_color } else { theme::TEXT_WEAK };
            let icon_at = |x: f32| Rect::from_min_size(Pos2::new(x, rect.center().y - ICON_SIZE / 2.0), Vec2::splat(ICON_SIZE));
            let mut x = rect.left() + pad.x;
            match self.slot {
                Slot::Leading(icon) => {
                    icon.paint(painter, icon_at(x), icon_color);
                    x += ICON_SIZE + gap;
                }
                Slot::Blank => x += ICON_SIZE + gap,
                Slot::Trailing(icon) => {
                    let at = if self.show_label { rect.right() - pad.x - ICON_SIZE } else { x };
                    icon.paint(painter, icon_at(at), icon_color);
                }
            }
            if let Some(galley) = galley {
                let at = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
                painter.galley(at, galley, text_color);
            }
            if let Some(shortcut) = shortcut {
                let at = Pos2::new(rect.right() - pad.x - shortcut.size().x, rect.center().y - shortcut.size().y / 2.0);
                painter.galley(at, shortcut, text_color);
            }
        }
        if self.show_label {
            response
        } else if enabled {
            response.on_hover_text(self.label)
        } else {
            response.on_disabled_hover_text(self.label)
        }
    }
}
