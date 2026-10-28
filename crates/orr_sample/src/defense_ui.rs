//! Responsive HUD, validated local-board pointer commands and frame-derived combat feedback.
use crate::defense_view::{self as view, Layout, BRONZE, INK, MUTED, PAPER, TEAL};
use egui::{vec2, Color32, Key, Rect, Stroke};
use orr_games::coop_defense_game as game;
use std::collections::BTreeMap;

fn just_pressed(ctx: &egui::Context, key: Key) -> bool {
    ctx.input(|i|i.events.iter().any(|event|matches!(event,egui::Event::Key{key:pressed,pressed:true,repeat:false,..} if *pressed==key)))
}

pub struct DefenseUi {
    pub context: egui::Context,
    pub selected: Option<u32>,
    pub started: bool,
    sequence: u32,
    previous: BTreeMap<u32, game::DefenseEnemy>,
    impacts: Vec<(i32, i32, u32, u32)>,
    observed_tick: u32,
    notice: String,
}
impl Default for DefenseUi {
    fn default() -> Self {
        Self::new()
    }
}
impl DefenseUi {
    pub fn new() -> Self {
        Self {
            context: egui::Context::default(),
            selected: None,
            started: false,
            sequence: 0,
            previous: BTreeMap::new(),
            impacts: Vec::new(),
            observed_tick: 0,
            notice: "Summon a crew. Match identical role + tier to merge.".into(),
        }
    }
    fn command(&mut self, action: u32, source_cell: u32, target_cell: u32) -> game::DefenseInput {
        self.sequence = self.sequence.saturating_add(1);
        game::DefenseInput {
            action,
            source_cell,
            target_cell,
            sequence: self.sequence,
        }
    }
    pub fn show(
        &mut self,
        input: egui::RawInput,
        frame: &orr_ecs::Frame,
    ) -> (egui::FullOutput, Option<game::DefenseInput>) {
        let run = game::run(frame);
        let max_hp = frame.singleton::<game::Scenario>().reactor_hp;
        let units = game::units(frame);
        let enemies = game::enemies(frame);
        if run.tick < self.observed_tick {
            self.previous.clear();
            self.impacts.clear();
            self.selected = None;
        }
        if run.tick != self.observed_tick {
            for old in self.previous.values() {
                if let Some(now) = enemies.iter().find(|e| e.id == old.id) {
                    if now.hp < old.hp {
                        let (x, y) = game::enemy_position(old.lane, old.progress);
                        self.impacts.push((x, y, old.hp - now.hp, run.tick));
                    }
                }
            }
            self.previous = enemies
                .iter()
                .filter(|e| e.active != 0)
                .map(|e| (e.id, *e))
                .collect();
            self.observed_tick = run.tick;
        }
        self.impacts.retain(|v| run.tick.saturating_sub(v.3) < 40);
        self.sequence = self.sequence.max(run.last_sequence);
        let mut action = None;
        let ctx = self.context.clone();
        let output = ctx.run_ui(input, |root| {
            let ctx = root.ctx();
            let screen = ctx.input(|i| i.content_rect());
            let l = Layout::new(screen);
            let p = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("foundry"),
            ));
            p.rect_filled(screen, 0.0, INK);
            view::paint(&p, l, frame, self.selected);
            l.text(&p, 40.0, 37.0, 27.0, PAPER, "STORMGLASS  /  FOUNDRY");
            l.text(
                &p,
                42.0,
                67.0,
                12.0,
                MUTED,
                "A COOPERATIVE DEFENSE OF THE LIVING ENGINE",
            );
            l.text(&p, 805.0, 32.0, 13.0, TEAL, "LOCAL CO-OP  ·  AI PARTNER");
            l.text(
                &p,
                805.0,
                60.0,
                18.0,
                PAPER,
                format!(
                    "WAVE {:02} / 10     CORE {} / {}",
                    run.wave, run.reactor_hp, max_hp
                ),
            );
            let core = l.rect(805.0, 78.0, 345.0, 4.0);
            p.rect_filled(core, 0.0, BRONZE.gamma_multiply(0.3));
            p.rect_filled(
                Rect::from_min_size(
                    core.min,
                    vec2(
                        core.width() * core_fraction(run.reactor_hp, max_hp),
                        core.height(),
                    ),
                ),
                0.0,
                TEAL,
            );
            l.text(&p, 150.0, 236.0, 17.0, PAPER, "01  YOUR FOUNDRY");
            l.text(&p, 650.0, 236.0, 17.0, PAPER, "02  AI PARTNER");
            l.text(
                &p,
                150.0,
                251.0,
                12.0,
                MUTED,
                "SELECT > EMPTY TILE TO MOVE / MATCH TO MERGE",
            );
            l.text(
                &p,
                650.0,
                251.0,
                12.0,
                MUTED,
                format!("AUTONOMOUS CREW   ·   {} BRASS", run.gold[1]),
            );
            for &(x, y, damage, tick) in &self.impacts {
                let age = run.tick.saturating_sub(tick) as f32;
                let c = l.world(x, y) - vec2(0.0, age * 0.6) * l.scale;
                p.circle_stroke(
                    c,
                    (8.0 + age * 0.6) * l.scale,
                    Stroke::new(
                        2.0 * l.scale,
                        Color32::from_rgba_unmultiplied(245, 197, 128, ((40.0 - age) * 5.0) as u8),
                    ),
                );
                p.text(
                    c - vec2(0.0, 17.0) * l.scale,
                    egui::Align2::CENTER_CENTER,
                    format!("−{damage}"),
                    egui::FontId::proportional(16.0 * l.scale),
                    PAPER,
                );
            }
            if let Some(boss) = enemies
                .iter()
                .find(|e| e.active != 0 && e.kind == game::BOSS)
            {
                l.text(
                    &p,
                    350.0,
                    132.0,
                    15.0,
                    PAPER,
                    "SIEGE ENGINE  /  BREAK THE SHELL",
                );
                let r = l.rect(350.0, 151.0, 500.0, 8.0);
                p.rect_filled(r, 2.0, INK);
                p.rect_filled(
                    Rect::from_min_size(
                        r.min,
                        vec2(
                            r.width() * boss.hp as f32 / boss.max_hp.max(1) as f32,
                            r.height(),
                        ),
                    ),
                    2.0,
                    Color32::from_rgb(220, 127, 87),
                );
            }
            if !self.started {
                p.rect_filled(
                    l.rect(0.0, 90.0, 1200.0, 850.0),
                    0.0,
                    Color32::from_black_alpha(145),
                );
                let r = l.rect(265.0, 260.0, 670.0, 420.0);
                p.rect_filled(r, 16.0 * l.scale, INK);
                p.rect_stroke(
                    r,
                    16.0 * l.scale,
                    Stroke::new(2.0 * l.scale, BRONZE),
                    egui::StrokeKind::Inside,
                );
                l.text(&p, 310.0, 310.0, 29.0, PAPER, "ASSEMBLE YOUR CREW");
                l.text(
                    &p,
                    310.0,
                    350.0,
                    16.0,
                    TEAL,
                    "Two foundries. One core. Ten incoming waves.",
                );
                for (i, rule) in [
                    "1   SPACE or SUMMON: recruit a random turret for 25 brass.",
                    "2   Click a unit, then an identical role + tier to merge.",
                    "3   U / FORGE increases the damage of your entire board.",
                    "4   I / INVEST adds passive income. Protect the shared core.",
                ]
                .iter()
                .enumerate()
                {
                    l.text(&p, 310.0, 398.0 + i as f32 * 34.0, 15.0, PAPER, rule);
                }
                l.text(
                    &p,
                    310.0,
                    555.0,
                    14.0,
                    MUTED,
                    "Your partner is a local AI. There is no online connection.",
                );
                let start = l.rect(310.0, 590.0, 580.0, 55.0);
                p.rect_filled(start, 7.0 * l.scale, Color32::from_rgb(61, 101, 84));
                l.text(
                    &p,
                    450.0,
                    617.0,
                    18.0,
                    PAPER,
                    "[ENTER / SPACE]  START DEFENSE",
                );
                let clicked = ctx.input(|i| {
                    i.pointer.primary_clicked()
                        && i.pointer.interact_pos().is_some_and(|p| start.contains(p))
                });
                if clicked || (just_pressed(ctx, Key::Enter) || just_pressed(ctx, Key::Space)) {
                    self.started = true;
                }
                return;
            }
            for role in 0..6 {
                let x = 160.0 + role as f32 * 175.0;
                view::turret(&p, l.p(x, 185.0), l.scale * 0.63, role, 1, run.tick);
                l.text(
                    &p,
                    x - 32.0,
                    218.0,
                    11.0,
                    MUTED,
                    view::ROLE_NAMES[role as usize].to_uppercase(),
                );
            }
            // Hover previews never change the authoritative state.
            let pointer = ctx.input(|i| i.pointer.interact_pos());
            let hover = pointer.and_then(|p| l.hit_cell(p));
            if let Some(source) = self.selected.and_then(|c| {
                units
                    .iter()
                    .find(|u| u.active != 0 && u.board == 0 && u.cell == c)
            }) {
                l.text(
                    &p,
                    45.0,
                    795.0,
                    15.0,
                    TEAL,
                    format!(
                        "{}  ·  TIER {}  ·  {}",
                        view::ROLE_NAMES[source.role as usize],
                        source.tier,
                        match source.role {
                            game::MORTAR => "SPLASH",
                            game::PRISM => "ARMOR PIERCE",
                            game::CONDENSER => "SLOW",
                            game::HASTE => "BOARD HASTE",
                            game::MINER => "PASSIVE INCOME",
                            _ => "RAPID FIRE",
                        }
                    ),
                );
                if let Some(target) = hover.filter(|c| *c != source.cell) {
                    let other = units.iter().find(|u| u.board == 0 && u.cell == target);
                    let possible = other.is_some_and(|u| {
                        u.active == 0
                            || (u.role == source.role
                                && u.tier == source.tier
                                && u.tier < game::MAX_TIER)
                    });
                    let text = if other.is_some_and(|u| u.active == 0) {
                        "MOVE HERE"
                    } else if possible {
                        "MERGE +1 TIER"
                    } else {
                        "NEEDS IDENTICAL ROLE + TIER"
                    };
                    let r = l.cell(0, target);
                    p.rect_stroke(
                        r.expand(4.0 * l.scale),
                        4.0,
                        Stroke::new(
                            2.0 * l.scale,
                            if possible { TEAL } else { Color32::LIGHT_RED },
                        ),
                        egui::StrokeKind::Outside,
                    );
                    l.text(
                        &p,
                        550.0,
                        795.0,
                        13.0,
                        if possible { TEAL } else { PAPER },
                        text,
                    );
                }
            } else {
                l.text(&p, 45.0, 795.0, 14.0, MUTED, &self.notice);
            }
            p.rect_filled(
                l.rect(30.0, 825.0, 1140.0, 90.0),
                12.0 * l.scale,
                Color32::from_rgb(30, 51, 51),
            );
            l.text(
                &p,
                50.0,
                852.0,
                23.0,
                PAPER,
                format!("{}  BRASS", run.gold[0]),
            );
            l.text(
                &p,
                50.0,
                880.0,
                12.0,
                MUTED,
                format!("{} KILLS    {} MERGES", run.kills, run.merges),
            );
            let buttons = [
                (game::SUMMON, "[SPACE] SUMMON", game::SUMMON_COST),
                (
                    game::UPGRADE,
                    "[U] FORGE",
                    game::upgrade_cost(run.upgrades[0]),
                ),
                (
                    game::INVEST,
                    "[I] INVEST",
                    game::investment_cost(run.investment[0]),
                ),
            ];
            for (i, (a, label, cost)) in buttons.iter().enumerate() {
                let r = l.rect(270.0 + i as f32 * 220.0, 842.0, 205.0, 55.0);
                let allowed = run.phase == game::PLAYING
                    && run.gold[0] >= *cost
                    && match *a {
                        game::SUMMON => units.iter().any(|u| u.board == 0 && u.active == 0),
                        game::UPGRADE => run.upgrades[0] < game::MAX_UPGRADE,
                        _ => run.investment[0] < game::MAX_INVESTMENT,
                    };
                p.rect_filled(
                    r,
                    6.0 * l.scale,
                    if allowed {
                        Color32::from_rgb(66, 89, 77)
                    } else {
                        Color32::from_rgb(43, 60, 57)
                    },
                );
                p.rect_stroke(
                    r,
                    6.0 * l.scale,
                    Stroke::new(
                        l.scale,
                        if allowed {
                            BRONZE
                        } else {
                            MUTED.gamma_multiply(0.3)
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                l.text(
                    &p,
                    282.0 + i as f32 * 220.0,
                    860.0,
                    13.0,
                    if allowed { PAPER } else { MUTED },
                    label,
                );
                l.text(
                    &p,
                    282.0 + i as f32 * 220.0,
                    882.0,
                    12.0,
                    MUTED,
                    format!(
                        "{cost} brass{}",
                        if *a == game::UPGRADE {
                            format!("  ·  LV {}", run.upgrades[0])
                        } else if *a == game::INVEST {
                            format!("  ·  LV {}", run.investment[0])
                        } else {
                            String::new()
                        }
                    ),
                );
                let key = match *a {
                    game::SUMMON => Key::Space,
                    game::UPGRADE => Key::U,
                    _ => Key::I,
                };
                let clicked = pointer.is_some_and(|point| r.contains(point))
                    && ctx.input(|i| i.pointer.primary_clicked());
                if allowed && (clicked || just_pressed(ctx, key)) {
                    action = Some(self.command(*a, 0, 0));
                }
            }
            let restart = l.rect(950.0, 842.0, 200.0, 55.0);
            p.rect_filled(restart, 6.0 * l.scale, Color32::from_rgb(49, 64, 61));
            l.text(&p, 970.0, 868.0, 14.0, PAPER, "[R] RESTART");
            if just_pressed(ctx, Key::R)
                || (pointer.is_some_and(|p| restart.contains(p))
                    && ctx.input(|i| i.pointer.primary_clicked()))
            {
                self.selected = None;
                action = Some(self.command(game::RESTART, 0, 0));
            }
            if run.phase == game::PLAYING && ctx.input(|i| i.pointer.primary_clicked()) {
                if let Some(cell) = hover {
                    if let Some(source) = self.selected {
                        if source == cell {
                            self.selected = None;
                        } else {
                            let a = units
                                .iter()
                                .find(|u| u.board == 0 && u.cell == source && u.active != 0);
                            let b = units.iter().find(|u| u.board == 0 && u.cell == cell);
                            if a.zip(b).is_some_and(|(a, b)| {
                                b.active == 0
                                    || (a.role == b.role
                                        && a.tier == b.tier
                                        && b.tier < game::MAX_TIER)
                            }) {
                                action = Some(self.command(game::MOVE_OR_MERGE, source, cell));
                                self.selected = Some(cell);
                            } else {
                                self.selected = units
                                    .iter()
                                    .find(|u| u.board == 0 && u.cell == cell && u.active != 0)
                                    .map(|u| u.cell);
                            }
                        }
                    } else {
                        self.selected = units
                            .iter()
                            .find(|u| u.board == 0 && u.cell == cell && u.active != 0)
                            .map(|u| u.cell);
                    }
                }
            }
            if just_pressed(ctx, Key::Escape) {
                self.selected = None;
            }
            if run.phase != game::PLAYING {
                let r = l.rect(300.0, 320.0, 600.0, 190.0);
                p.rect_filled(r, 12.0 * l.scale, INK);
                p.rect_stroke(
                    r,
                    12.0 * l.scale,
                    Stroke::new(2.0 * l.scale, BRONZE),
                    egui::StrokeKind::Inside,
                );
                l.text(
                    &p,
                    345.0,
                    370.0,
                    29.0,
                    PAPER,
                    if run.phase == game::WON {
                        "THE FOUNDRY ENDURES"
                    } else {
                        "THE STORM BROKE THROUGH"
                    },
                );
                l.text(
                    &p,
                    345.0,
                    418.0,
                    17.0,
                    TEAL,
                    format!(
                        "{} defeated  ·  {} merges  ·  wave {}",
                        run.kills, run.merges, run.wave
                    ),
                );
                l.text(
                    &p,
                    345.0,
                    463.0,
                    16.0,
                    MUTED,
                    "Press R or RESTART below to assemble a new crew.",
                );
            }
        });
        (output, action)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responsive_cell_hit_is_local_and_exact() {
        for size in [vec2(1200.0, 940.0), vec2(750.0, 900.0), vec2(1800.0, 800.0)] {
            let l = Layout::new(Rect::from_min_size(egui::pos2(0.0, 0.0), size));
            for c in 0..game::CELLS_PER_BOARD {
                assert_eq!(l.hit_cell(l.cell(0, c).center()), Some(c));
                assert_eq!(l.hit_cell(l.cell(1, c).center()), None);
            }
            assert_eq!(l.hit_cell(egui::pos2(-1.0, -1.0)), None);
        }
    }
    fn draw(
        ui: &mut DefenseUi,
        frame: &orr_ecs::Frame,
        events: Vec<egui::Event>,
    ) -> Option<game::DefenseInput> {
        let (mut output, action) = ui.show(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 940.0))),
                events,
                ..Default::default()
            },
            frame,
        );
        output.textures_delta.clear();
        action
    }
    fn key(key: Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn ready_start_does_not_spend_and_summon_has_one_sequence() {
        let sim =
            orr_sim::Simulation::<game::DefenseGame>::new(game::DefenseConfig::default(), 60, 1);
        let mut ui = DefenseUi::new();
        assert!(draw(&mut ui, sim.frame(), vec![]).is_none());
        assert!(!ui.started);
        assert!(draw(&mut ui, sim.frame(), vec![key(Key::Space, true)]).is_none());
        assert!(ui.started);
        draw(&mut ui, sim.frame(), vec![key(Key::Space, false)]);
        let action = draw(&mut ui, sim.frame(), vec![key(Key::Space, true)]).unwrap();
        assert_eq!(action.action, game::SUMMON);
        assert_eq!(action.sequence, 1);
        assert!(draw(&mut ui, sim.frame(), vec![]).is_none());
        let repeat = egui::Event::Key {
            key: Key::Space,
            physical_key: Some(Key::Space),
            pressed: true,
            repeat: true,
            modifiers: Default::default(),
        };
        assert!(draw(&mut ui, sim.frame(), vec![repeat]).is_none());
    }
    #[test]
    fn pointer_select_and_move_uses_only_local_board() {
        let mut sim =
            orr_sim::Simulation::<game::DefenseGame>::new(game::DefenseConfig::default(), 60, 1);
        let mut inputs = orr_sim::TickInputs::new(0, 1);
        inputs.set_input(
            orr_sim::PlayerSlot(0),
            game::DefenseInput {
                action: game::SUMMON,
                sequence: 1,
                ..Default::default()
            },
        );
        sim.step(&inputs);
        let mut ui = DefenseUi::new();
        ui.started = true;
        let l = Layout::new(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 940.0)));
        let click = |p| {
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        };
        draw(&mut ui, sim.frame(), vec![]);
        assert!(draw(&mut ui, sim.frame(), click(l.cell(1, 0).center())).is_none());
        assert_eq!(ui.selected, None);
        assert!(draw(&mut ui, sim.frame(), click(l.cell(0, 0).center())).is_none());
        assert_eq!(ui.selected, Some(0));
        let command = draw(&mut ui, sim.frame(), click(l.cell(0, 1).center())).unwrap();
        assert_eq!(
            (command.action, command.source_cell, command.target_cell),
            (game::MOVE_OR_MERGE, 0, 1)
        );
        assert!(command.sequence > 1);
    }
}

fn core_fraction(hp: u32, max_hp: u32) -> f32 {
    hp.min(max_hp) as f32 / max_hp.max(1) as f32
}

#[cfg(test)]
mod authored_core_tests {
    #[test]
    fn health_bar_uses_authored_maximum_and_stays_bounded() {
        assert_eq!(super::core_fraction(75, 75), 1.0);
        assert_eq!(super::core_fraction(15, 75), 0.2);
        assert_eq!(super::core_fraction(90, 75), 1.0);
        assert_eq!(super::core_fraction(0, 75), 0.0);
        assert_eq!(super::core_fraction(10, 0), 0.0);
    }
}
