//! Single-window docking of the editor panels.
//!
//! [`Dock`] owns the panel tree (the `egui_tiles` crate does the split, tab, resize and drag
//! work) and the user's layout settings. The layout is plain data in [`crate::layout`]. This
//! module turns that data into a live tree and back, draws the `Window` menu, keeps the
//! viewport reachable, and writes the layout after the user changes it.
//!
//! A layout change never touches the scene, the undo history, the simulation or a checksum.
//! It does not save a work document, start Play or move the keyboard focus.
#![allow(clippy::float_arithmetic)]

use egui::{Color32, Context, Event, Key, Rect, Sense, Stroke, Ui, Vec2, WidgetText};
use egui_tiles::{Behavior, Container, ContainerKind, EditAction, Linear, LinearDir, Tabs, Tile, TileId, Tiles, Tree, UiResponse};

use crate::icons::{Icon, IconButton};
use crate::layout::{self, Child, Dir, Edge, Layout, LayoutError, LayoutStore, Node, Panel};

const TREE_ID: &str = "orr_editor_dock";

/// Memory key that tells a panel's title code that the panel sits in a tab group.
pub(crate) fn tabbed_id() -> egui::Id {
    egui::Id::new("orr_dock_tabbed")
}
/// No panel is dragged smaller than this, in points, along either axis.
pub const MIN_PANE: f32 = 100.0;
/// Width of the grab strip between two panels.
const GAP: f32 = 3.0;
/// Default pixel sizes of the first layout (same as before docking existed).
const DEFAULT_LEFT: f32 = 230.0;
const DEFAULT_RIGHT: f32 = 340.0;
const DEFAULT_BOTTOM: f32 = 120.0;
/// A layout that leaves the viewport smaller than this at load time is replaced by the default.
const LOAD_MIN_VIEWPORT: Vec2 = Vec2::new(300.0, 200.0);
/// A layout that leaves the viewport smaller than this at any time is replaced by the default.
const LIVE_MIN_VIEWPORT: Vec2 = Vec2::new(60.0, 60.0);
/// Height of the grab strip of a panel that has a title strip, and of one that has not.
const TITLE_GRAB: f32 = 28.0;
const EDGE_GRAB: f32 = 8.0;

/// The panel arrangement of the window and the user's saved layouts.
pub struct Dock {
    tree: Tree<Panel>,
    store: Option<LayoutStore>,
    notice: Option<String>,
    presets: Vec<String>,
    /// The layout changed and is not in the layout file yet.
    changed: bool,
    /// The layout that the layout file holds now.
    saved: Option<Layout>,
    /// Size the first default layout in points on the next frame.
    default_sizes: bool,
    /// Check on the next frame that a loaded layout leaves a usable viewport.
    strict_check: bool,
    scale: f32,
    /// A panel was being dragged in the frame before.
    was_dragging: bool,
    /// The text of the open "Save layout as" prompt.
    save_prompt: Option<String>,
}

impl Default for Dock {
    fn default() -> Self {
        Dock { tree: build(&Layout::default()), default_sizes: true, ..Dock::bare() }
    }
}

impl Dock {
    /// An empty dock with no allocation. The app swaps it in while the real dock draws.
    pub(crate) fn placeholder() -> Self {
        Dock { tree: Tree::empty(TREE_ID), default_sizes: false, ..Dock::bare() }
    }

    fn bare() -> Self {
        Dock {
            tree: Tree::empty(TREE_ID),
            store: None,
            notice: None,
            presets: Vec::new(),
            changed: false,
            saved: None,
            default_sizes: false,
            strict_check: false,
            scale: 0.0,
            was_dragging: false,
            save_prompt: None,
        }
    }

    /// Keeps the layout in `store` from now on and restores the layout of the last session.
    /// A layout file that is damaged, too new or unusable gives the default layout and a notice.
    pub fn attach(&mut self, store: LayoutStore) {
        match store.read_last() {
            Ok(None) => {}
            Ok(Some(layout)) => {
                self.apply(&layout);
                self.strict_check = true;
                self.saved = Some(layout);
            }
            Err(error) => {
                self.notice = Some(format!("The saved layout was not used because {error}. The default layout is shown."));
            }
        }
        self.presets = store.list_presets();
        self.store = Some(store);
    }

    /// The message about the layout that the user has not dismissed yet.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Hides the message.
    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    /// The names of the saved presets.
    pub fn presets(&self) -> &[String] {
        &self.presets
    }

    /// The layout as it is now. Fails only when the tree holds something a layout cannot hold.
    pub fn layout(&self) -> Result<Layout, LayoutError> {
        let root = self.tree.root().ok_or_else(|| LayoutError::Io("the layout is empty".into()))?;
        let layout = Layout { root: export(&self.tree.tiles, root)? };
        layout.validate()?;
        Ok(layout)
    }

    /// Shows `layout`. The caller has checked it.
    pub fn apply(&mut self, layout: &Layout) {
        self.tree = build(layout);
        self.default_sizes = false;
        self.changed = true;
    }

    /// Shows the default layout again.
    pub fn reset(&mut self) {
        self.tree = build(&Layout::default());
        self.default_sizes = true;
        self.changed = true;
    }

    /// True while `panel` is shown.
    pub fn is_visible(&self, panel: Panel) -> bool {
        self.tree.tiles.find_pane(&panel).is_some_and(|id| self.tree.tiles.is_visible(id))
    }

    /// Shows or hides `panel`. The viewport cannot be hidden.
    pub fn set_visible(&mut self, panel: Panel, visible: bool) {
        if !panel.can_hide() && !visible {
            return;
        }
        if let Some(id) = self.tree.tiles.find_pane(&panel) {
            self.tree.tiles.set_visible(id, visible);
            self.changed = true;
        }
    }

    /// Shows `panel` and brings its tab to the front. Other panels keep their place.
    pub fn reveal(&mut self, panel: Panel) {
        self.set_visible(panel, true);
        self.tree.make_active(|_, tile| matches!(tile, egui_tiles::Tile::Pane(shown) if *shown == panel));
    }

    /// Moves `panel` to a window edge.
    pub fn dock(&mut self, panel: Panel, edge: Edge) {
        self.edit(|layout| layout.dock(panel, edge));
    }

    /// Moves `panel` into the tab group of `other`.
    pub fn tab_with(&mut self, panel: Panel, other: Panel) {
        self.edit(|layout| layout.tab_with(panel, other));
    }

    fn edit(&mut self, change: impl FnOnce(&mut Layout)) {
        if let Ok(mut layout) = self.layout() {
            change(&mut layout);
            if layout.validate().is_ok() {
                self.apply(&layout);
            }
        }
    }

    /// Saves the layout under `name`. On failure the notice says why.
    pub fn save_preset(&mut self, name: &str) -> bool {
        let Some(store) = &self.store else {
            self.notice = Some("Layouts cannot be saved because this system has no user settings folder.".into());
            return false;
        };
        let result = self.layout().and_then(|layout| store.write_preset(name, &layout));
        self.presets = store.list_presets();
        match result {
            Ok(()) => true,
            Err(error) => {
                self.notice = Some(format!("The layout was not saved: {error}. Earlier presets are unchanged."));
                false
            }
        }
    }

    /// Shows the preset `name`. When the preset cannot be used the current layout stays.
    pub fn apply_preset(&mut self, name: &str) {
        let Some(store) = &self.store else { return };
        match store.read_preset(name) {
            Ok(layout) => {
                self.apply(&layout);
                self.strict_check = true;
            }
            Err(error) => self.notice = Some(format!("The preset \"{name}\" was not used because {error}. The current layout stays.")),
        }
    }

    /// Deletes the preset `name`.
    pub fn delete_preset(&mut self, name: &str) {
        let Some(store) = &self.store else { return };
        if let Err(error) = store.delete_preset(name) {
            self.notice = Some(format!("The preset \"{name}\" was not deleted: {error}"));
        }
        self.presets = store.list_presets();
    }

    /// Takes an Escape key press away from the editor when it cancels a panel drag, so the
    /// press does not also clear the selection or cancel a gesture. Returns true when it did.
    /// Call it first in every frame. egui itself ends the drag on Escape before the frame
    /// runs, so the dock remembers whether a panel was being dragged in the frame before.
    pub fn cancel_drag_on_escape(&mut self, ctx: &Context) -> bool {
        let tree_id = self.tree.id();
        let dragging = self.tree.tiles.tile_ids().any(|tile| ctx.dragged_id() == Some(tile.egui_id(tree_id)));
        let was_dragging = std::mem::replace(&mut self.was_dragging, dragging);
        if !(was_dragging || dragging) || !ctx.input(|input| input.key_pressed(Key::Escape)) {
            return false;
        }
        self.was_dragging = false;
        ctx.stop_dragging();
        ctx.input_mut(|input| input.events.retain(|event| !matches!(event, Event::Key { key: Key::Escape, .. })));
        true
    }

    /// Draws the panels in the space left in `ui`. `render` draws the content of one panel.
    pub fn show(&mut self, ui: &mut Ui, render: &mut dyn FnMut(&mut Ui, Panel)) {
        let ctx = ui.ctx().clone();
        let scale = ctx.pixels_per_point();
        if (scale - self.scale).abs() > f32::EPSILON {
            if self.scale != 0.0 {
                // A display scale change can leave a saved layout without room for the viewport.
                self.strict_check = true;
            }
            self.scale = scale;
        }
        let available = ui.available_size();
        if self.default_sizes {
            self.default_sizes = false;
            self.size_default(available);
        }
        {
            let Dock { tree, changed, .. } = self;
            let tabbed = tabbed_panels(&tree.tiles);
            let mut panes = Panes { render, changed, tabbed, wider: Vec::new() };
            tree.ui(&mut panes, ui);
            let wider = panes.wider;
            self.widen(&wider, available.x);
        }
        self.check_viewport(available);
        self.autosave(&ctx);
    }

    /// Gives a panel more width when its content cannot fit, as a side panel used to grow.
    /// The panel takes at most 60 percent of the window, and the viewport gives the room.
    fn widen(&mut self, wider: &[(Panel, f32)], window_width: f32) {
        for (panel, needed) in wider {
            let Some(pane) = self.tree.tiles.find_pane(panel) else { continue };
            // The panel, or its tab group, is a child of a horizontal split.
            let mut child = pane;
            let row = loop {
                let Some(parent) = self.tree.tiles.parent_of(child) else { break None };
                if matches!(self.tree.tiles.get(parent), Some(Tile::Container(Container::Linear(linear))) if linear.dir == LinearDir::Horizontal) {
                    break Some(parent);
                }
                child = parent;
            };
            let (Some(row), Some(current)) = (row, self.tree.tiles.rect(child).map(|rect| rect.width())) else { continue };
            let target = needed.min(window_width * 0.6);
            if current <= 0.0 || target <= current + 1.0 {
                continue;
            }
            if let Some(Tile::Container(Container::Linear(linear))) = self.tree.tiles.get_mut(row) {
                let share = linear.shares[child];
                linear.shares.set_share(child, share * target / current);
                self.changed = true;
            }
        }
    }

    fn size_default(&mut self, available: Vec2) {
        let tiles = &mut self.tree.tiles;
        let Some(root) = self.tree.root else { return };
        let Some(Tile::Container(Container::Linear(outer))) = tiles.get(root) else { return };
        if outer.dir != LinearDir::Vertical || outer.children.len() != 2 {
            return;
        }
        let (top, bottom) = (outer.children[0], outer.children[1]);
        if let Some(Tile::Container(Container::Linear(outer))) = tiles.get_mut(root) {
            outer.shares.set_share(top, (available.y - DEFAULT_BOTTOM).max(MIN_PANE));
            outer.shares.set_share(bottom, DEFAULT_BOTTOM);
        }
        let Some(Tile::Container(Container::Linear(row))) = tiles.get_mut(top) else { return };
        if row.children.len() == 3 {
            let (left, middle, right) = (row.children[0], row.children[1], row.children[2]);
            row.shares.set_share(left, DEFAULT_LEFT);
            row.shares.set_share(middle, (available.x - DEFAULT_LEFT - DEFAULT_RIGHT).max(MIN_PANE));
            row.shares.set_share(right, DEFAULT_RIGHT);
        }
    }

    /// Replaces a layout that left no usable viewport with the default layout.
    fn check_viewport(&mut self, available: Vec2) {
        let Some(id) = self.tree.tiles.find_pane(&Panel::Viewport) else { return };
        let Some(rect) = self.tree.tiles.rect(id) else { return };
        let strict = std::mem::take(&mut self.strict_check);
        let minimum = if strict { LOAD_MIN_VIEWPORT } else { LIVE_MIN_VIEWPORT };
        // A window that is itself too small for the default layout is not the layout's fault.
        let room = available.x >= LOAD_MIN_VIEWPORT.x + 2.0 * MIN_PANE && available.y >= LOAD_MIN_VIEWPORT.y + MIN_PANE;
        if room && (rect.width() < minimum.x || rect.height() < minimum.y) {
            self.reset();
            self.notice = Some("The saved layout does not fit this window size or display scale, so the default layout is shown.".into());
        }
    }

    /// Writes the layout file once the user has let go of the mouse.
    fn autosave(&mut self, ctx: &Context) {
        if !self.changed || ctx.input(|input| input.pointer.any_down()) {
            return;
        }
        self.changed = false;
        let Some(store) = &self.store else { return };
        let Ok(layout) = self.layout() else { return };
        if self.saved.as_ref() == Some(&layout) {
            return;
        }
        match store.write_last(&layout) {
            Ok(()) => self.saved = Some(layout),
            Err(error) => self.notice = Some(format!("The layout was not saved: {error}. The earlier layout file is kept.")),
        }
    }

    /// The content of the `Window` menu.
    pub fn window_menu(&mut self, ui: &mut Ui) {
        for panel in Panel::ALL {
            // The eye at the right end shows whether the panel is shown or hidden.
            let visible = self.is_visible(panel);
            let icon = if visible { Icon::Eye } else { Icon::EyeOff };
            let response = ui.add_enabled(panel.can_hide(), IconButton::new(icon, panel.title()).menu().icon_trailing().toggled(visible));
            let response = if panel.can_hide() { response } else { response.on_disabled_hover_text("The viewport always stays visible.") };
            if response.clicked() {
                self.set_visible(panel, !visible);
            }
        }
        ui.separator();
        ui.menu_button("Layout", |ui| {
            if ui.button("Save as\u{2026}").on_hover_text("Keep this arrangement under a name.").clicked() {
                self.save_prompt = Some(String::new());
                ui.close();
            }
            if !self.presets.is_empty() {
                ui.separator();
            }
            for name in self.presets.clone() {
                if ui.add(IconButton::new(Icon::Layout, &name).menu().icon_trailing()).on_hover_text("Show this saved arrangement.").clicked() {
                    self.apply_preset(&name);
                    ui.close();
                }
            }
            if !self.presets.is_empty() {
                ui.menu_button("Delete preset", |ui| {
                    for name in self.presets.clone() {
                        if ui.button(&name).clicked() {
                            self.delete_preset(&name);
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
            if ui.button("Reset to default").on_hover_text("Show every panel in its first place.").clicked() {
                self.reset();
                ui.close();
            }
        });
        ui.menu_button("Move panel", |ui| {
            for panel in Panel::ALL {
                ui.menu_button(panel.title(), |ui| {
                    for (label, edge) in [("Dock at left edge", Edge::Left), ("Dock at right edge", Edge::Right), ("Dock at top edge", Edge::Top), ("Dock at bottom edge", Edge::Bottom)] {
                        if ui.button(label).clicked() {
                            self.dock(panel, edge);
                            ui.close();
                        }
                    }
                    ui.separator();
                    for other in Panel::ALL.into_iter().filter(|other| *other != panel) {
                        if ui.button(format!("Tab with {}", other.title())).clicked() {
                            self.tab_with(panel, other);
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    /// The "Save layout as" prompt. It is a small window, not a modal dialog.
    pub fn prompts(&mut self, ctx: &Context) {
        let Some(mut text) = self.save_prompt.take() else { return };
        let mut keep_open = true;
        let mut save = false;
        egui::Window::new("Save layout as").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut text).hint_text("Preset name").char_limit(layout::MAX_NAME_CHARS).desired_width(220.0));
            if !field.has_focus() && !field.lost_focus() && ui.memory(|memory| memory.focused().is_none()) {
                field.request_focus();
            }
            let valid = layout::clean_name(&text).is_some();
            if !valid && !text.is_empty() {
                crate::icons::notice_line(ui, Icon::Error, crate::theme::ERROR, "Use 1 to 40 characters without control characters.");
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(valid, egui::Button::new("Save")).clicked() || (valid && field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))) {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    keep_open = false;
                }
            });
        });
        if save {
            self.save_preset(&text);
        } else if keep_open {
            self.save_prompt = Some(text);
        }
    }

    /// The one-line message bar. Call it only while [`Dock::notice`] holds a message.
    pub fn notice_bar(&mut self, ui: &mut Ui) {
        let Some(text) = self.notice.clone() else { return };
        ui.horizontal(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            crate::icons::icon(ui, Icon::Warning, crate::theme::WARN, crate::icons::ICON_SIZE);
            ui.colored_label(crate::theme::WARN, "Layout");
            ui.label(&text).on_hover_text(&text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Dismiss").clicked() {
                    self.notice = None;
                }
                if ui.button("Reset layout").clicked() {
                    self.reset();
                    self.notice = None;
                }
            });
        });
    }
}

/// What `egui_tiles` asks of the editor: how to draw a panel and how a panel behaves.
struct Panes<'a> {
    render: &'a mut dyn FnMut(&mut Ui, Panel),
    changed: &'a mut bool,
    /// The panels that share a tab group with another panel.
    tabbed: Vec<Panel>,
    /// Panels whose content needed more width than they had, with the width needed.
    wider: Vec<(Panel, f32)>,
}

fn tabbed_panels(tiles: &Tiles<Panel>) -> Vec<Panel> {
    let mut found = Vec::new();
    for tile in tiles.tiles() {
        if let Tile::Container(Container::Tabs(tabs)) = tile {
            if tabs.children.len() > 1 {
                found.extend(tabs.children.iter().filter_map(|child| match tiles.get(*child) {
                    Some(Tile::Pane(panel)) => Some(*panel),
                    _ => None,
                }));
            }
        }
    }
    found
}

impl Behavior<Panel> for Panes<'_> {
    fn pane_ui(&mut self, ui: &mut Ui, _tile: TileId, pane: &mut Panel) -> UiResponse {
        let rect = ui.max_rect();
        ui.shrink_clip_rect(rect);
        let frame = if *pane == Panel::Viewport { egui::Frame::NONE.fill(crate::theme::BG) } else { crate::theme::panel_frame() };
        let tabbed = self.tabbed.contains(pane);
        let shown = frame.show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.data_mut(|data| data.insert_temp(tabbed_id(), tabbed));
            (self.render)(ui, *pane);
            ui.data_mut(|data| data.insert_temp(tabbed_id(), false));
        });
        if *pane != Panel::Viewport && shown.response.rect.width() > rect.width() + 1.0 {
            self.wider.push((*pane, shown.response.rect.width()));
        }
        if tabbed {
            // The tab bar is the handle of a tabbed panel.
            return UiResponse::None;
        }
        // The title strip (or the top margin of a panel without one) is the grab handle.
        let grab = match *pane {
            Panel::Viewport => return UiResponse::None,
            Panel::Timeline => EDGE_GRAB,
            Panel::Hierarchy | Panel::Inspector => TITLE_GRAB,
        };
        let strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), grab));
        let response = ui.interact(strip, ui.id().with("dock_grab"), Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Move the {} panel", pane.title())));
        if *pane == Panel::Timeline {
            let y = strip.center().y;
            for step in -2..=2 {
                ui.painter().circle_filled(egui::pos2(strip.center().x + step as f32 * 6.0, y), 1.0, crate::theme::TEXT_WEAK);
            }
        }
        if response.drag_started() { UiResponse::DragStarted } else { UiResponse::None }
    }

    fn tab_title_for_pane(&mut self, pane: &Panel) -> WidgetText {
        pane.title().into()
    }

    fn is_tab_closable(&self, tiles: &Tiles<Panel>, tile: TileId) -> bool {
        matches!(tiles.get(tile), Some(Tile::Pane(panel)) if panel.can_hide())
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Panel>, tile: TileId) -> bool {
        // Closing a tab hides the panel. The Window menu brings it back to the same place.
        tiles.set_visible(tile, false);
        *self.changed = true;
        false
    }

    fn min_size(&self) -> f32 {
        MIN_PANE
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        GAP
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        crate::theme::BAR
    }

    /// The same height as the title strip of a panel that is not tabbed.
    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        TITLE_GRAB
    }

    fn tab_bg_color(&self, _visuals: &egui::Visuals, _tiles: &Tiles<Panel>, _tile_id: TileId, state: &egui_tiles::TabState) -> Color32 {
        if state.active { crate::theme::PANEL } else { Color32::TRANSPARENT }
    }

    /// The active tab has the accent edge of a panel title.
    fn tab_outline_stroke(&self, _visuals: &egui::Visuals, _tiles: &Tiles<Panel>, _tile_id: TileId, state: &egui_tiles::TabState) -> Stroke {
        if state.active { Stroke::new(1.0, crate::theme::ACCENT) } else { Stroke::NONE }
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(1.0, crate::theme::LINE)
    }

    fn tab_text_color(&self, _visuals: &egui::Visuals, _tiles: &Tiles<Panel>, _tile_id: TileId, state: &egui_tiles::TabState) -> Color32 {
        if state.active { crate::theme::TEXT_STRONG } else { crate::theme::TEXT_WEAK }
    }

    fn drag_preview_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(2.0, crate::theme::ACCENT)
    }

    fn drag_preview_color(&self, _visuals: &egui::Visuals) -> Color32 {
        crate::theme::ACCENT.gamma_multiply(0.25)
    }

    fn on_edit(&mut self, action: EditAction) {
        if action != EditAction::TileDragged {
            *self.changed = true;
        }
    }
}

fn build(layout: &Layout) -> Tree<Panel> {
    let mut tiles = Tiles::default();
    let root = add(&mut tiles, &layout.root);
    Tree::new(TREE_ID, root, tiles)
}

fn add(tiles: &mut Tiles<Panel>, node: &Node) -> TileId {
    match node {
        Node::Pane { panel, visible } => {
            let id = tiles.insert_pane(*panel);
            if !visible {
                tiles.set_visible(id, false);
            }
            id
        }
        Node::Split { dir, children } => {
            let ids: Vec<TileId> = children.iter().map(|child| add(tiles, &child.node)).collect();
            let mut linear = Linear::new(if *dir == Dir::Horizontal { LinearDir::Horizontal } else { LinearDir::Vertical }, ids.clone());
            for (id, child) in ids.iter().zip(children) {
                linear.shares.set_share(*id, child.share as f32);
            }
            tiles.insert_container(linear)
        }
        Node::Tabs { children, active } => {
            let ids: Vec<TileId> = children.iter().map(|child| add(tiles, child)).collect();
            let mut tabs = Tabs::new(ids.clone());
            tabs.active = ids.get(*active).copied();
            tiles.insert_container(tabs)
        }
    }
}

fn export(tiles: &Tiles<Panel>, id: TileId) -> Result<Node, LayoutError> {
    match tiles.get(id) {
        Some(Tile::Pane(panel)) => Ok(Node::Pane { panel: *panel, visible: tiles.is_visible(id) }),
        Some(Tile::Container(Container::Linear(linear))) => {
            let total: f32 = linear.children.iter().map(|child| linear.shares[*child]).sum();
            let children = linear
                .children
                .iter()
                .map(|child| {
                    let share = if total > 0.0 { (linear.shares[*child] / total * 1000.0).round() } else { 1.0 };
                    Ok(Child { share: share.clamp(1.0, layout::MAX_SHARE as f32) as u32, node: export(tiles, *child)? })
                })
                .collect::<Result<Vec<_>, LayoutError>>()?;
            let dir = if linear.dir == LinearDir::Horizontal { Dir::Horizontal } else { Dir::Vertical };
            Ok(Node::Split { dir, children })
        }
        Some(Tile::Container(Container::Tabs(tabs))) => {
            let children = tabs.children.iter().map(|child| export(tiles, *child)).collect::<Result<Vec<_>, _>>()?;
            let active = tabs.active.and_then(|active| tabs.children.iter().position(|child| *child == active)).unwrap_or(0);
            Ok(Node::Tabs { children, active })
        }
        Some(Tile::Container(container)) if container.kind() == ContainerKind::Grid => Err(LayoutError::Corrupt("a grid cannot be saved".into())),
        _ => Err(LayoutError::Corrupt("the panel tree is incomplete".into())),
    }
}
