use egui::{Align2, Color32, FontId, Rect, Ui};
use orr_remote::mesh_navigation::mission::MissionView;

pub(crate) fn draw(ui: &mut Ui, viewport: Rect, mission: &MissionView) {
    let Some((status, progress)) = text(mission) else {
        return;
    };
    let width = (viewport.width() - 16.0).min(360.0);
    let height = (viewport.height() - 16.0).min(70.0);
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let rectangle = Rect::from_min_size(
        viewport.left_bottom() + egui::vec2(8.0, -height - 8.0),
        egui::vec2(width, height),
    );
    let clip = rectangle.intersect(viewport);
    let painter = ui.painter().with_clip_rect(clip);
    painter.rect_filled(
        rectangle,
        4.0,
        Color32::from_rgba_unmultiplied(10, 20, 35, 225),
    );
    painter.text(
        rectangle.left_top() + egui::vec2(10.0, 9.0),
        Align2::LEFT_TOP,
        status,
        FontId::proportional(18.0),
        if mission.run.won == 1 {
            Color32::from_rgb(100, 230, 170)
        } else {
            Color32::WHITE
        },
    );
    painter.text(
        rectangle.left_top() + egui::vec2(10.0, 36.0),
        Align2::LEFT_TOP,
        &progress,
        FontId::proportional(16.0),
        Color32::WHITE,
    );
    let mut overlay = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("mesh_mission_overlay")
            .max_rect(rectangle)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    overlay.set_clip_rect(clip);
    for label in [status, &progress] {
        overlay.add(egui::Label::new(
            egui::RichText::new(label).color(Color32::TRANSPARENT),
        ));
    }
}

fn text(mission: &MissionView) -> Option<(&'static str, String)> {
    if mission.config.enabled != 1 || !(1..=8).contains(&mission.config.count) {
        return None;
    }
    let mask = (1u32 << mission.config.count) - 1;
    if mission.run.collected & !mask != 0
        || mission.run.won > 1
        || (mission.run.won == 1 && mission.run.collected != mask)
    {
        return None;
    }
    let status = if mission.run.won == 1 {
        "Game mission complete"
    } else {
        "Game mission in progress"
    };
    Some((
        status,
        format!(
            "Collected {} / {}",
            mission.run.collected.count_ones(),
            mission.config.count
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::text;
    use orr_remote::mesh_navigation::mission::{MeshMission, MeshMissionRun, MissionView};

    fn fixture() -> MissionView {
        MissionView {
            config: MeshMission {
                enabled: 1,
                count: 2,
                ..MeshMission::default()
            },
            run: MeshMissionRun::default(),
        }
    }

    #[test]
    fn collecting_every_objective_is_not_an_exit_win() {
        let mut mission = fixture();
        mission.run.collected = 3;
        assert_eq!(
            text(&mission),
            Some(("Game mission in progress", "Collected 2 / 2".into()))
        );
        mission.run.won = 1;
        assert_eq!(
            text(&mission),
            Some(("Game mission complete", "Collected 2 / 2".into()))
        );
    }

    #[test]
    fn disabled_and_malformed_state_never_claims_victory() {
        let mut mission = fixture();
        mission.config.enabled = 0;
        assert_eq!(text(&mission), None);
        mission.config.enabled = 1;
        mission.run.won = 1;
        assert_eq!(text(&mission), None);
        mission.run.collected = 7;
        assert_eq!(text(&mission), None);
    }
}
