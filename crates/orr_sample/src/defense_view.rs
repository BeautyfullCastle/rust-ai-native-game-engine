//! Original vector art for Stormglass Foundry. Presentation never mutates simulation.
use egui::{vec2, Align2, Color32 as C, FontId, Painter, Pos2, Rect, Shape, Stroke};
use orr_games::coop_defense_game as game;

pub const INK: C = C::from_rgb(12, 32, 37);
pub const BRONZE: C = C::from_rgb(186, 140, 87);
pub const TEAL: C = C::from_rgb(105, 223, 197);
pub const PAPER: C = C::from_rgb(238, 226, 189);
pub const MUTED: C = C::from_rgb(137, 163, 158);
pub const ROLE_NAMES: [&str; 6] = ["Rivet", "Mortar", "Prism", "Condenser", "Haste", "Miner"];
pub const ROLE_COLORS: [C; 6] = [
    BRONZE,
    C::from_rgb(231, 137, 83),
    C::from_rgb(145, 184, 248),
    TEAL,
    C::from_rgb(179, 207, 123),
    C::from_rgb(238, 202, 105),
];

#[derive(Clone, Copy)]
pub struct Layout {
    pub origin: Pos2,
    pub scale: f32,
}
impl Layout {
    pub fn new(rect: Rect) -> Self {
        let scale = (rect.width() / 1200.0)
            .min(rect.height() / 940.0)
            .max(0.001);
        Self {
            origin: rect.center() - vec2(600.0, 470.0) * scale,
            scale,
        }
    }
    pub fn p(self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }
    pub fn world(self, x: i32, y: i32) -> Pos2 {
        self.p(x as f32 + 100.0, y as f32 + 105.0)
    }
    pub fn rect(self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.p(x, y), vec2(w, h) * self.scale)
    }
    pub fn cell(self, board: u32, cell: u32) -> Rect {
        let (x, y) = game::board_cell_position(board, cell);
        Rect::from_center_size(self.world(x, y), vec2(78.0, 68.0) * self.scale)
    }
    pub fn hit_cell(self, point: Pos2) -> Option<u32> {
        (0..game::CELLS_PER_BOARD).find(|&cell| self.cell(0, cell).contains(point))
    }
    pub fn text(self, p: &Painter, x: f32, y: f32, size: f32, color: C, text: impl ToString) {
        p.text(
            self.p(x, y),
            Align2::LEFT_CENTER,
            text.to_string(),
            FontId::proportional(size * self.scale),
            color,
        );
    }
}
fn polygon(p: &Painter, points: Vec<Pos2>, color: C, stroke: Stroke) {
    p.add(Shape::convex_polygon(points, color, stroke));
}
fn diamond(p: &Painter, c: Pos2, r: f32, color: C) {
    polygon(
        p,
        vec![
            c + vec2(0.0, -r),
            c + vec2(r * 0.65, 0.0),
            c + vec2(0.0, r),
            c + vec2(-r * 0.65, 0.0),
        ],
        color,
        Stroke::new(1.0, PAPER),
    );
}

/// Six silhouettes, assembled from purpose-built mechanical parts rather than dots.
pub fn turret(p: &Painter, c: Pos2, s: f32, role: u32, tier: u32, tick: u32) {
    let color = ROLE_COLORS[role as usize % 6];
    p.circle_filled(c + vec2(3.0, 9.0) * s, 23.0 * s, C::from_black_alpha(85));
    p.circle_filled(c + vec2(0.0, 4.0) * s, 21.0 * s, C::from_rgb(45, 49, 44));
    p.circle_stroke(
        c + vec2(0.0, 3.0) * s,
        19.0 * s,
        Stroke::new(3.0 * s, BRONZE),
    );
    for i in 0..4 {
        let a = i as f32 * std::f32::consts::FRAC_PI_2;
        p.circle_filled(
            c + vec2(a.cos() * 16.0, a.sin() * 16.0 + 3.0) * s,
            2.0 * s,
            PAPER,
        );
    }
    // Higher tiers physically gain bracing, shoulder radiators and a crown.
    if tier >= 2 {
        for side in [-1.0, 1.0] {
            p.line_segment(
                [
                    c + vec2(side * 15.0, 7.0) * s,
                    c + vec2(side * 23.0, -14.0) * s,
                ],
                Stroke::new(4.0 * s, BRONZE),
            );
        }
    }
    if tier >= 3 {
        for side in [-1.0, 1.0] {
            p.rect_filled(
                Rect::from_center_size(c + vec2(side * 23.0, -4.0) * s, vec2(7.0, 20.0) * s),
                2.0 * s,
                color,
            );
        }
    }
    if tier >= 4 {
        p.circle_stroke(c, 26.0 * s, Stroke::new(2.5 * s, PAPER));
    }
    if tier >= 5 {
        diamond(p, c + vec2(0.0, -35.0) * s, 8.0 * s, TEAL);
    }
    let c = c - vec2(0.0, 7.0) * s;
    match role {
        0 => {
            p.rect_filled(
                Rect::from_center_size(c, vec2(27.0, 23.0) * s),
                3.0 * s,
                color,
            );
            p.line_segment([c, c + vec2(0.0, -27.0) * s], Stroke::new(10.0 * s, BRONZE));
            p.line_segment(
                [c + vec2(-5.0, -27.0) * s, c + vec2(5.0, -27.0) * s],
                Stroke::new(3.0 * s, PAPER),
            );
            p.circle_filled(c, 6.0 * s, INK);
        }
        1 => {
            p.circle_filled(c, 17.0 * s, color);
            p.circle_filled(c - vec2(0.0, 5.0) * s, 11.0 * s, INK);
            p.circle_stroke(
                c - vec2(0.0, 5.0) * s,
                11.0 * s,
                Stroke::new(3.0 * s, PAPER),
            );
        }
        2 => {
            for x in [-12.0, 12.0] {
                p.line_segment(
                    [c + vec2(x, 10.0) * s, c + vec2(x, -14.0) * s],
                    Stroke::new(4.0 * s, BRONZE),
                );
            }
            diamond(p, c - vec2(0.0, 7.0) * s, 23.0 * s, color);
        }
        3 => {
            for x in [-10.0, 10.0] {
                p.rect_filled(
                    Rect::from_center_size(c + vec2(x, 0.0) * s, vec2(9.0, 34.0) * s),
                    4.0 * s,
                    color,
                );
                for y in [-10.0, 0.0, 10.0] {
                    p.line_segment(
                        [c + vec2(x - 5.0, y) * s, c + vec2(x + 5.0, y) * s],
                        Stroke::new(2.0 * s, PAPER),
                    );
                }
            }
        }
        4 => {
            let angle = tick as f32 * 0.025;
            for i in 0..3 {
                let a = angle + i as f32 * std::f32::consts::TAU / 3.0;
                polygon(
                    p,
                    vec![
                        c,
                        c + vec2(a.cos() * 25.0, a.sin() * 25.0) * s,
                        c + vec2((a + 0.8).cos() * 16.0, (a + 0.8).sin() * 16.0) * s,
                    ],
                    color,
                    Stroke::new(s, BRONZE),
                );
            }
            p.circle_filled(c, 6.0 * s, PAPER);
        }
        _ => {
            p.rect_filled(
                Rect::from_center_size(c, vec2(25.0, 26.0) * s),
                3.0 * s,
                color,
            );
            for y in [-7.0, 0.0, 7.0] {
                p.line_segment(
                    [c + vec2(-8.0, y) * s, c + vec2(8.0, y) * s],
                    Stroke::new(2.0 * s, INK),
                );
            }
            p.line_segment(
                [c + vec2(0.0, -13.0) * s, c + vec2(0.0, -28.0) * s],
                Stroke::new(3.0 * s, BRONZE),
            );
            p.circle_filled(c + vec2(0.0, -29.0) * s, 4.0 * s, TEAL);
        }
    }
    for i in 0..tier.min(5) {
        p.circle_filled(
            c + vec2((i as f32 - (tier.min(5) - 1) as f32 / 2.0) * 7.0, 33.0) * s,
            2.5 * s,
            PAPER,
        );
    }
}

pub fn paint(p: &Painter, l: Layout, frame: &orr_ecs::Frame, selected: Option<u32>) {
    let run = game::run(frame);
    p.rect_filled(l.rect(0.0, 0.0, 1200.0, 940.0), 0.0, INK);
    // Slate floor, hand-laid garden edges, and fine brass conduit work.
    p.rect_filled(
        l.rect(30.0, 90.0, 1140.0, 730.0),
        18.0 * l.scale,
        C::from_rgb(24, 47, 48),
    );
    for i in 0..22 {
        for j in 0..14 {
            let x = 42.0 + i as f32 * 52.0;
            let y = 105.0 + j as f32 * 50.0;
            p.line_segment(
                [l.p(x, y), l.p(x + 45.0, y)],
                Stroke::new(l.scale, C::from_rgb(31, 57, 56)),
            );
            if (i * 7 + j * 11) % 17 == 0 {
                p.circle_filled(
                    l.p(x + 12.0, y + 10.0),
                    10.0 * l.scale,
                    C::from_rgb(44, 70, 52),
                );
                p.circle_filled(
                    l.p(x + 22.0, y + 7.0),
                    7.0 * l.scale,
                    C::from_rgb(58, 83, 57),
                );
            }
        }
    }
    // Routes use the simulation's exact position function, never an artist's approximation.
    for lane in 0..2 {
        let points: Vec<_> = (0..=100)
            .map(|i| {
                let (x, y) = game::enemy_position(lane, i * 20);
                l.world(x, y)
            })
            .collect();
        p.add(Shape::line(
            points.clone(),
            Stroke::new(34.0 * l.scale, C::from_rgb(12, 28, 30)),
        ));
        p.add(Shape::line(
            points.clone(),
            Stroke::new(27.0 * l.scale, C::from_rgb(104, 105, 83)),
        ));
        p.add(Shape::line(
            points,
            Stroke::new(2.0 * l.scale, C::from_rgb(169, 153, 113)),
        ));
    }
    for board in 0..2 {
        for cell in 0..game::CELLS_PER_BOARD {
            let r = l.cell(board, cell);
            p.rect_filled(
                r.translate(vec2(0.0, 9.0) * l.scale),
                4.0 * l.scale,
                C::from_rgb(32, 36, 31),
            );
            let color = if (cell + cell / game::BOARD_COLS) % 2 == 0 {
                C::from_rgb(92, 111, 91)
            } else {
                C::from_rgb(66, 88, 78)
            };
            p.rect_filled(r, 3.0 * l.scale, color);
            p.rect_stroke(
                r.shrink(l.scale),
                3.0 * l.scale,
                Stroke::new(l.scale, BRONZE),
                egui::StrokeKind::Inside,
            );
            for corner in [r.left_top(), r.right_bottom()] {
                p.circle_filled(
                    corner
                        + vec2(
                            if corner == r.left_top() { 5.0 } else { -5.0 },
                            if corner == r.left_top() { 5.0 } else { -5.0 },
                        ) * l.scale,
                    1.8 * l.scale,
                    PAPER,
                );
            }
            if board == 0 && selected == Some(cell) {
                p.rect_stroke(
                    r.expand(3.0 * l.scale),
                    4.0 * l.scale,
                    Stroke::new(3.0 * l.scale, TEAL),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }
    let units = game::units(frame);
    let shots = game::projectiles(frame);
    if let Some(unit) = selected.and_then(|cell| {
        units
            .iter()
            .find(|u| u.board == 0 && u.cell == cell && u.active != 0)
    }) {
        // The simulation uses Manhattan distance, so its truthful footprint is a diamond.
        if unit.role != game::HASTE && unit.role != game::MINER {
            let center = l.cell(0, unit.cell).center();
            let radius = if unit.role == game::MORTAR {
                600.0
            } else {
                390.0
            } * l.scale;
            let clipped = p.with_clip_rect(l.rect(130.0, 280.0, 450.0, 440.0));
            clipped.add(Shape::closed_line(
                vec![
                    center + vec2(0.0, -radius),
                    center + vec2(radius, 0.0),
                    center + vec2(0.0, radius),
                    center + vec2(-radius, 0.0),
                ],
                Stroke::new(1.5 * l.scale, C::from_rgba_unmultiplied(105, 223, 197, 120)),
            ));
        }
    }
    for unit in units.into_iter().filter(|u| u.active != 0) {
        let (x, y) = game::board_cell_position(unit.board, unit.cell);
        let firing = shots.iter().any(|s| {
            s.active != 0
                && s.board == unit.board
                && s.role == unit.role
                && (s.x - x).abs() + (s.y - y).abs() < 25
        });
        let c = l.cell(unit.board, unit.cell).center();
        turret(
            p,
            c + vec2(0.0, if firing { 3.0 } else { 0.0 }) * l.scale,
            l.scale,
            unit.role,
            unit.tier,
            run.tick,
        );
        if firing {
            p.circle_stroke(
                c - vec2(0.0, 25.0) * l.scale,
                11.0 * l.scale,
                Stroke::new(2.0 * l.scale, PAPER),
            );
        }
    }
    for enemy in game::enemies(frame).into_iter().filter(|e| e.active != 0) {
        let (x, y) = game::enemy_position(enemy.lane, enemy.progress);
        let c = l.world(x, y);
        let s = l.scale;
        let boss = enemy.kind == game::BOSS;
        let r = if boss { 24.0 } else { 12.0 } * s;
        let color = match enemy.kind {
            0 => C::from_rgb(199, 121, 100),
            1 => C::from_rgb(220, 172, 103),
            2 => C::from_rgb(153, 136, 171),
            _ => C::from_rgb(213, 113, 87),
        };
        p.circle_filled(c + vec2(3.0, 8.0) * s, r, C::from_black_alpha(90));
        for side in [-1.0, 1.0] {
            for k in [-1.0, 1.0] {
                p.line_segment(
                    [
                        c + vec2(side * r * 0.5, k * r * 0.6),
                        c + vec2(side * r * 1.5, k * r),
                    ],
                    Stroke::new(3.0 * s, color),
                );
            }
        }
        match enemy.kind {
            1 => diamond(p, c, r * 1.3, color),
            2 => {
                p.rect_filled(
                    Rect::from_center_size(c, vec2(r * 1.7, r * 1.7)),
                    3.0 * s,
                    color,
                );
                p.rect_stroke(
                    Rect::from_center_size(c, vec2(r * 1.9, r * 1.9)),
                    3.0 * s,
                    Stroke::new(3.0 * s, BRONZE),
                    egui::StrokeKind::Outside,
                );
            }
            _ => {
                polygon(
                    p,
                    vec![
                        c + vec2(-r, -r * 0.5),
                        c + vec2(-r * 0.6, -r),
                        c + vec2(r * 0.6, -r),
                        c + vec2(r, -r * 0.5),
                        c + vec2(r * 0.8, r),
                        c + vec2(-r * 0.8, r),
                    ],
                    color,
                    Stroke::new(2.0 * s, BRONZE),
                );
            }
        }
        p.line_segment(
            [c + vec2(-r * 0.5, -2.0 * s), c + vec2(r * 0.5, -2.0 * s)],
            Stroke::new(3.0 * s, INK),
        );
        if enemy.slow_ticks > 0 {
            p.circle_stroke(c, r + 4.0 * s, Stroke::new(2.0 * s, TEAL));
        }
        let hp = enemy.hp as f32 / enemy.max_hp.max(1) as f32;
        let bar = Rect::from_min_size(c + vec2(-r, -r - 8.0 * s), vec2(r * 2.0, 3.0 * s));
        p.rect_filled(bar, 0.0, INK);
        p.rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * hp, bar.height())),
            0.0,
            TEAL,
        );
    }
    let visible_enemies = game::enemies(frame);
    for shot in game::projectiles(frame)
        .into_iter()
        .filter(|s| s.active != 0)
    {
        let c = l.world(shot.x, shot.y);
        let color = ROLE_COLORS[shot.role as usize % 6];
        p.circle_filled(c, 8.0 * l.scale, color.gamma_multiply(0.22));
        p.circle_filled(c, 3.0 * l.scale, PAPER);
        let direction = visible_enemies
            .iter()
            .find(|e| e.active != 0 && e.id == shot.target_id)
            .map(|e| {
                let (x, y) = game::enemy_position(e.lane, e.progress);
                (l.world(x, y) - c).normalized()
            })
            .unwrap_or(vec2(0.0, -1.0));
        p.line_segment(
            [c, c - direction * 11.0 * l.scale],
            Stroke::new(2.0 * l.scale, color),
        );
    }
    // Shared reactor: concentric copper shell, turbine blades, luminous core.
    let c = l.world(500, 590);
    for r in [42.0, 34.0] {
        p.circle_filled(c, r * l.scale, C::from_rgb(37, 54, 51));
        p.circle_stroke(c, r * l.scale, Stroke::new(3.0 * l.scale, BRONZE));
    }
    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0 + run.tick as f32 * 0.008;
        p.line_segment(
            [
                c + vec2(a.cos() * 20.0, a.sin() * 20.0) * l.scale,
                c + vec2(a.cos() * 32.0, a.sin() * 32.0) * l.scale,
            ],
            Stroke::new(5.0 * l.scale, TEAL),
        );
    }
    diamond(p, c, 20.0 * l.scale, TEAL);
    l.text(p, 520.0, 755.0, 13.0, MUTED, "SHARED STORMGLASS CORE");
}
