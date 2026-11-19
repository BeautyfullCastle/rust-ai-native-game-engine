//! Sizes and finds the docked Hierarchy and Inspector in a test window.
//!
//! The editor draws its panels in a dock. A test that needs a known panel width builds a
//! layout and applies it through the public dock, the way a saved user layout does.
#![allow(dead_code)]
#![allow(clippy::float_arithmetic)]

use egui::{pos2, Rect};
use egui_kittest::Harness;
use orr_editor::layout::{Child, Dir, Layout, Node, Panel};
use orr_editor::EditorApp;

/// The gap between two docked panels, in points (the dock's `gap_width`).
pub const DOCK_GAP: f32 = 3.0;

/// Sizes the docked Hierarchy and Inspector to the given widths in points. The viewport takes the rest.
pub fn set_panel_widths(h: &mut Harness<'_, EditorApp>, inspector: f32, hierarchy: f32) {
    let total = h.ctx.viewport_rect().width();
    let middle = total - 2.0 * DOCK_GAP - inspector - hierarchy;
    assert!(middle.max(inspector).max(hierarchy) <= 1000.0, "the layout shares are limited to 1000 points");
    let share = |width: f32| width.round().clamp(1.0, 1000.0) as u32;
    let pane = |panel| Node::Pane { panel, visible: true };
    let layout = Layout {
        root: Node::Split {
            dir: Dir::Vertical,
            children: vec![
                Child {
                    share: 840,
                    node: Node::Split {
                        dir: Dir::Horizontal,
                        children: vec![
                            Child { share: share(hierarchy), node: pane(Panel::Hierarchy) },
                            Child { share: share(middle), node: pane(Panel::Viewport) },
                            Child { share: share(inspector), node: pane(Panel::Inspector) },
                        ],
                    },
                },
                Child { share: 160, node: pane(Panel::Timeline) },
            ],
        },
    };
    layout.validate().expect("a valid layout");
    h.state_mut().ui.dock.apply(&layout);
    h.run_steps(5);
}

/// Where the docked Inspector is: right of the viewport, to the window edge.
pub fn inspector_rect(h: &Harness<'_, EditorApp>) -> Rect {
    let screen = h.ctx.viewport_rect();
    let viewport = h.state().ui.viewport_rect.expect("viewport");
    Rect::from_min_max(pos2(viewport.max.x + DOCK_GAP, screen.min.y), screen.max)
}

/// Where the docked Hierarchy is: from the window edge to the viewport.
pub fn hierarchy_rect(h: &Harness<'_, EditorApp>) -> Rect {
    let screen = h.ctx.viewport_rect();
    let viewport = h.state().ui.viewport_rect.expect("viewport");
    Rect::from_min_max(screen.min, pos2(viewport.min.x - DOCK_GAP, screen.max.y))
}
