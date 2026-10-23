use super::{EditorApp, LBL_PAUSE, LBL_PLAY};
use egui::{Pos2, Rect, ViewportId};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::{thread, time::Duration};

#[test]
fn expanded_mesh_panels_stay_inside_the_editor_viewport_at_narrow_widths() {
    for size in [[700.0, 946.0], [1000.0, 946.0], [1421.0, 946.0]] {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("mesh-panel-layout.scene.yaml");
        let editor = crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        let mut harness = Harness::builder()
            .with_size(size)
            .build_eframe(move |_| EditorApp::new(editor, None));
        harness
            .input_mut()
            .viewports
            .get_mut(&ViewportId::ROOT)
            .unwrap()
            .inner_rect = Some(Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        ));
        harness.run_steps(5);

        for (header, child) in [
            ("Static-mesh navigation", "Create ramp and bridge"),
            ("Mesh agent art", "Agent art offset X"),
            ("Mesh props", "Add prop"),
            ("Mesh game camera", "Game camera target X"),
            ("Mesh game lighting", "Lighting direction X"),
            ("Mesh game HUD", "HUD title"),
            ("Mesh mission", "Mission collectible count"),
            ("Mesh agent animation", "Animation clip index"),
        ] {
            if harness.query_by_label(child).is_none() {
                let header_node = harness.get_by_label(header);
                header_node.scroll_to_me();
                harness.run_steps(2);
                harness.get_by_label(header).click_accesskit();
                harness.run_steps(3);
            }
            assert!(
                harness.query_by_label(child).is_some(),
                "missing expanded control {child} under {header} at width {}",
                size[0]
            );
        }

        for action in [
            "Place mesh agent",
            "Apply agent art",
            "Add prop",
            "Apply game camera",
            "Apply game lighting",
            "Apply game HUD",
            "Apply mesh mission",
            "Save mesh animation",
        ] {
            let action_node = harness.get_by_label(action);
            action_node.scroll_to_me();
            harness.run_steps(2);
            let action_rect = harness.get_by_label(action).rect();
            let inspector =
                egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
                    .expect("rendered mesh inspector panel state")
                    .outer_rect;
            let screen = harness.ctx.input(|input| input.viewport_rect());
            assert!(
                action_rect.min.x >= inspector.min.x - 0.5
                    && action_rect.max.x <= inspector.max.x + 0.5,
                "{action} escaped inspector bounds at width {}: {action_rect:?}, {inspector:?}",
                size[0]
            );
            assert!(
                action_rect.min.x >= screen.min.x - 0.5 && action_rect.max.x <= screen.max.x + 0.5,
                "{action} escaped viewport bounds at width {}: {action_rect:?}, {screen:?}",
                size[0]
            );
        }

        for id in ["top", "status", "bottom", "hierarchy", "inspector"] {
            let state = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new(id))
                .unwrap_or_else(|| panic!("missing rendered {id} panel at width {}", size[0]));
            let rect = state.outer_rect;
            let screen = harness.ctx.input(|input| input.viewport_rect());
            assert!(
                rect.min.x >= screen.min.x - 0.5
                    && rect.max.x <= screen.max.x + 0.5
                    && rect.min.y >= screen.min.y - 0.5
                    && rect.max.y <= screen.max.y + 0.5,
                "{id} panel escaped viewport at width {}: {rect:?}, {screen:?}",
                size[0]
            );
        }

        let play = harness.get_by_label(LBL_PLAY);
        play.scroll_to_me();
        harness.run_steps(2);
        assert!(!harness
            .get_by_label(LBL_PLAY)
            .accesskit_node()
            .is_disabled());
        harness.get_by_label(LBL_PLAY).click_accesskit();
        let mut observed_playing = false;
        for _ in 0..100 {
            harness.run_steps(1);
            thread::sleep(Duration::from_millis(10));
            let editor = &harness.state().editor;
            if editor.is_playing_mode()
                && editor.timeline().is_some_and(|timeline| timeline.playing)
            {
                observed_playing = true;
                break;
            }
        }
        assert!(
            observed_playing,
            "Play never reached an active timeline at width {}",
            size[0]
        );
        let pause = harness.get_by_label(LBL_PAUSE);
        pause.scroll_to_me();
        harness.run_steps(2);
        harness.get_by_label(LBL_PAUSE).click_accesskit();
        let mut observed_paused = false;
        for _ in 0..100 {
            harness.run_steps(1);
            thread::sleep(Duration::from_millis(10));
            let editor = &harness.state().editor;
            if editor.is_playing_mode()
                && editor.timeline().is_some_and(|timeline| !timeline.playing)
            {
                observed_paused = true;
                break;
            }
        }
        assert!(
            observed_paused,
            "Pause never reached a paused timeline at width {}",
            size[0]
        );

        let inspector =
            egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
                .expect("inspector panel remains present after Play/Pause")
                .outer_rect;
        let screen = harness.ctx.input(|input| input.viewport_rect());
        assert!(inspector.max.x <= screen.max.x + 0.5);
    }
}
