//! The design tokens of the editor shell and its vector icons: text contrast on
//! every surface, icons inside their rectangle at display scales 1, 1.5 and 2,
//! and the accessible names of the icon buttons.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

use egui::epaint::Primitive;
use egui::{Color32, Pos2, Rect, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use orr_editor::icons::{Icon, IconButton, ICON_SIZE};
use orr_editor::theme::*;

/// WCAG contrast ratio of two opaque colors.
fn contrast(a: Color32, b: Color32) -> f64 {
    fn channel(c: u8) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }
    let l = |c: Color32| 0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b());
    let (hi, lo) = (l(a).max(l(b)), l(a).min(l(b)));
    (hi + 0.05) / (lo + 0.05)
}

/// The accent fill of a selection, laid over `ground`.
fn selected_over(ground: Color32) -> Color32 {
    let a = f32::from(ACCENT_FILL.a()) / 255.0;
    let mix = |fill: u8, ground: u8| (f32::from(fill) + f32::from(ground) * (1.0 - a)).round() as u8;
    Color32::from_rgb(mix(ACCENT_FILL.r(), ground.r()), mix(ACCENT_FILL.g(), ground.g()), mix(ACCENT_FILL.b(), ground.b()))
}

#[test]
fn every_text_token_is_readable_on_its_surfaces() {
    // Where each text token is drawn: surfaces that hold text.
    let body_surfaces = [("PANEL", PANEL), ("BAR", BAR), ("CARD", CARD), ("CARD_HEADER", CARD_HEADER), ("INPUT", INPUT)];
    let control_surfaces = [("BUTTON", BUTTON), ("BUTTON_HOVER", BUTTON_HOVER), ("BUTTON_ACTIVE", BUTTON_ACTIVE)];
    for (name, color) in [("TEXT", TEXT), ("TEXT_STRONG", TEXT_STRONG), ("TEXT_WEAK", TEXT_WEAK)] {
        for (ground_name, ground) in body_surfaces.iter().chain(control_surfaces.iter()) {
            // Weak text never sits on a control that is pressed down.
            if name == "TEXT_WEAK" && *ground_name == "BUTTON_ACTIVE" {
                continue;
            }
            let ratio = contrast(color, *ground);
            assert!(ratio >= 4.5, "{name} on {ground_name}: {ratio:.2}");
        }
    }
    // Colors with a meaning are text colors on the panels, bars, cards and wells.
    let marks = [
        ("ACCENT", ACCENT),
        ("PLAY", PLAY),
        ("WARN", WARN),
        ("ERROR", ERROR),
        ("INFO", INFO),
        ("KIND_DRAGON", kind::DRAGON),
        ("KIND_LIGHT", kind::LIGHT),
        ("KIND_CAMERA", kind::CAMERA),
        ("KIND_TERRAIN", kind::TERRAIN),
        ("KIND_WATER", kind::WATER),
        ("KIND_SCATTER", kind::SCATTER),
    ];
    for (name, color) in marks {
        for (ground_name, ground) in &body_surfaces {
            let ratio = contrast(color, *ground);
            assert!(ratio >= 4.5, "{name} on {ground_name}: {ratio:.2}");
        }
    }
    // Text on a filled control: dark text on the accent and play fills.
    for (name, fill) in [("ACCENT", ACCENT), ("PLAY", PLAY)] {
        let ratio = contrast(ON_ACCENT, fill);
        assert!(ratio >= 4.5, "ON_ACCENT on {name}: {ratio:.2}");
    }
    // Strong text on the selected row, over each surface a list sits on.
    for (name, ground) in [("PANEL", PANEL), ("CARD", CARD)] {
        let ratio = contrast(TEXT_STRONG, selected_over(ground));
        assert!(ratio >= 4.5, "selected row on {name}: {ratio:.2}");
        for &(mark_name, mark) in marks.iter().filter(|(name, _)| name.starts_with("KIND_")) {
            let ratio = contrast(mark, selected_over(ground));
            assert!(ratio >= 3.0, "{mark_name} icon on the selected row over {name}: {ratio:.2}");
        }
    }
}

#[test]
fn control_edges_and_focus_are_visible() {
    // Component boundaries and the focus ring need 3:1 against what is around them.
    for (name, ground) in [("PANEL", PANEL), ("BAR", BAR), ("CARD", CARD)] {
        let edge = contrast(CONTROL_EDGE, ground);
        assert!(edge >= 3.0, "CONTROL_EDGE on {name}: {edge:.2}");
        let focus = contrast(FOCUS, ground);
        assert!(focus >= 3.0, "FOCUS on {name}: {focus:.2}");
    }
}

#[test]
fn scales_are_ordered_and_distinct() {
    let sizes = [text::CAPTION, text::BODY, text::TITLE];
    for pair in sizes.windows(2) {
        assert!(pair[1] - pair[0] >= 2.0, "type sizes {pair:?} are less than 2 apart");
    }
    let steps = [space::XS, space::SM, space::MD, space::LG, space::XL];
    assert!(steps.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn every_icon_lies_inside_its_rectangle() {
    for icon in Icon::ALL {
        for side in [12.0_f32, 14.0, 16.0, 24.0, 32.0] {
            let rect = Rect::from_min_size(Pos2::new(10.25, 7.5), Vec2::splat(side));
            let shapes = icon.shapes(rect, Color32::WHITE);
            assert!(!shapes.is_empty(), "{icon:?} draws nothing");
            for shape in &shapes {
                let bounds = shape.visual_bounding_rect();
                assert!(rect.expand(0.01).contains_rect(bounds), "{icon:?} at {side}: {bounds:?} leaves {rect:?}");
            }
        }
    }
}

/// Draws every icon at display scale `ppp` and returns the tessellated vertex
/// bounds of each icon with its rectangle.
fn tessellated_bounds(ppp: f32) -> Vec<(Icon, Rect, Rect)> {
    let ctx = egui::Context::default();
    let size = Vec2::splat(ICON_SIZE);
    let mut cells: Vec<(Icon, Rect)> = Vec::new();
    for (index, icon) in Icon::ALL.into_iter().enumerate() {
        // A rectangle on a physical pixel, as a laid-out row would give it.
        let x = (10.0 + index as f32 * 20.0 * ppp).round() / ppp;
        let y = (10.0_f32 * ppp).round() / ppp;
        cells.push((icon, Rect::from_min_size(Pos2::new(x, y), size)));
    }
    let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 60.0))), ..Default::default() };
    input.viewports.get_mut(&input.viewport_id).expect("the root viewport").native_pixels_per_point = Some(ppp);
    let output = ctx.run_ui(input, |ui| {
        for (icon, rect) in &cells {
            icon.paint(ui.painter(), *rect, Color32::WHITE);
        }
    });
    let mut found = Vec::new();
    for (icon, rect) in &cells {
        // Tessellate one icon at a time, from the shapes the icon makes.
        let shapes: Vec<egui::epaint::ClippedShape> =
            icon.shapes(*rect, Color32::WHITE).into_iter().map(|shape| egui::epaint::ClippedShape { clip_rect: Rect::EVERYTHING, shape }).collect();
        let primitives = ctx.tessellate(shapes, ppp);
        let mut bounds: Option<Rect> = None;
        for primitive in primitives {
            if let Primitive::Mesh(mesh) = primitive.primitive {
                for vertex in mesh.vertices {
                    bounds = Some(bounds.map_or(Rect::from_min_max(vertex.pos, vertex.pos), |b| b.union(Rect::from_min_max(vertex.pos, vertex.pos))));
                }
            }
        }
        found.push((*icon, *rect, bounds.unwrap_or(Rect::NOTHING)));
    }
    assert!(!output.shapes.is_empty(), "the frame painted nothing");
    found
}

#[test]
fn icons_render_inside_their_rectangle_at_every_display_scale() {
    for ppp in [1.0_f32, 1.5, 2.0] {
        for (icon, rect, bounds) in tessellated_bounds(ppp) {
            assert!(bounds.is_positive(), "{icon:?} at scale {ppp} has no vertices");
            // Anti-aliasing feathers an edge by one physical pixel.
            let allowed = rect.expand(1.0 / ppp + 0.01);
            assert!(allowed.contains_rect(bounds), "{icon:?} at scale {ppp}: {bounds:?} leaves {rect:?}");
            // The icon fills most of its square: it is not drawn tiny.
            assert!(bounds.width().max(bounds.height()) >= rect.width() * 0.5, "{icon:?} at scale {ppp} is too small: {bounds:?}");
        }
    }
}

#[test]
fn icon_buttons_keep_their_label_as_the_accessible_name() {
    let mut clicked = (false, false);
    let mut harness = Harness::new_ui(|ui| {
        ui.horizontal(|ui| {
            clicked.0 |= ui.add(IconButton::new(Icon::Play, "Play")).clicked();
            clicked.1 |= ui.add(IconButton::new(Icon::Save, "Save").icon_only()).clicked();
            ui.add_enabled(false, IconButton::new(Icon::Stop, "Stop"));
        });
    });
    harness.run();
    let play = harness.get_by_label("Play");
    let save = harness.get_by_label("Save");
    // The icon-only button has the size of a small square button.
    assert!(save.rect().width() >= ICON_SIZE && save.rect().width() < play.rect().width());
    assert!(harness.get_by_label("Stop").accesskit_node().is_disabled());
    harness.get_by_label("Play").click();
    harness.get_by_label("Save").click();
    harness.run();
    drop(harness);
    assert_eq!(clicked, (true, true));
}
