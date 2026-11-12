//! Explicit viewport-to-draft cell snapping. Never writes host or terrain state.
use super::EditorApp;
use crate::{
    game::EditorGame,
    model::Mode,
    navigation_panel::Endpoint,
    terrain_document::{format_fixed, parse_fixed, TerrainSession, TerrainSource},
    terrain_pick,
};
use egui::Context;
use orr_fp::FP;
use orr_navigation::{AgentProfile, TerrainGraph};
use orr_render::Camera3D;
use orr_terrain::Terrain;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq)]
struct Pin {
    scene_epoch: u64,
    host_epoch: u64,
    checksum: u64,
    #[cfg(feature="terrain-patrol")]
    patrol: Option<crate::navigation_panel::PatrolPickToken>,
    scene: PathBuf,
    document_scene: PathBuf,
    source: TerrainSource,
    revision: [u8; 32],
    draft: [String; 6],
}

pub(crate) struct State {
    armed: Option<Endpoint>,
    pin: Option<Pin>,
    focused: bool,
    // Keep a cancelled press owned through release; never redirect its click.
    consume_cancelled: bool,
    pub(crate) status: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            armed: None,
            pin: None,
            focused: true,
            consume_cancelled: false,
            status: String::new(),
        }
    }
}
impl State {
    pub(crate) fn armed(&self) -> Option<Endpoint> {
        self.armed
    }
    fn cancel(&mut self, reason: &str) {
        self.consume_cancelled |= self.armed.is_some();
        self.armed = None;
        self.pin = None;
        self.status = reason.into();
    }
}

/// Floor the half-spacing in raw units, not the signed world coordinate.
fn cell_center(terrain: &Terrain, cell: [u32; 2]) -> Result<[FP; 2], String> {
    if cell[0] >= terrain.width() - 1 || cell[1] >= terrain.depth() - 1 {
        return Err("Picked cell lies outside terrain".into());
    }
    let spacing = i128::from(terrain.spacing().raw());
    let axis = |i: usize| -> Result<FP, String> {
        let raw = i128::from(cell[i])
            .checked_mul(spacing)
            .and_then(|offset| i128::from(terrain.origin()[i].raw()).checked_add(offset))
            .and_then(|raw| raw.checked_add(spacing / 2))
            .and_then(|raw| i64::try_from(raw).ok())
            .ok_or("Cell center exceeds fixed-point range")?;
        Ok(FP::from_raw(raw))
    };
    Ok([axis(0)?, axis(1)?])
}

fn graph(terrain: &Terrain, slope: &str) -> Result<TerrainGraph, String> {
    if terrain.width() > 17 || terrain.depth() > 17 {
        return Err("Navigation terrain is limited to 17 × 17 vertices".into());
    }
    let graph = TerrainGraph::build(
        terrain,
        AgentProfile {
            max_slope: parse_fixed(slope)?,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .map_err(|e| e.to_string())?;
    if graph.triangles().len() > 512 {
        return Err("Navigation graph exceeds 512 triangles".into());
    }
    Ok(graph)
}

impl EditorApp {
    /// In-memory context only: no graph construction, hashing or filesystem work.
    fn navigation_endpoint_pin(&self, endpoint: Option<Endpoint>) -> Option<Pin> {
        let editor = &self.editor;
        let document = &self.terrain.document;
        if !self.navigation_endpoint.focused
            || self.ui.dialog.is_some()
            || editor.game() != EditorGame::NavigationYard3D
            || !editor.spec().is_local()
            || editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || !editor.yard_rows_coherent()
            || editor.previewing().is_some()
            || editor.history().in_tx
            || editor.sim().in_tx
            || editor.in_gesture()
            || document.dirty()
            || document.read_only()
            || self.terrain_ramp.active()
            || self.package_work_pending()
            || self.hazard_export_busy()
            || self.navigation_export_busy()
            || self.shot.is_some() || self.remote_capture.is_some() || self.encoder.is_some()
        {
            return None;
        }
        let source = document.source()?;
        if !matches!(source, TerrainSource::Local { .. }) {
            return None;
        }
        document.terrain()?;
        let scene = editor.path()?;
        if editor.sim().scene_path.as_deref() != scene.to_str() {
            return None;
        }
        if document.scene()? != scene { return None; }
        #[cfg(feature="terrain-patrol")]
        let patrol = match endpoint {
            Some(Endpoint::Patrol(row)) => Some(self.navigation.patrol_pick_token(editor, &self.terrain, row).ok()?),
            _ => None,
        };
        #[cfg(not(feature="terrain-patrol"))]
        let _ = endpoint;
        Some(Pin {
            checksum: editor.snapshot()?.predicted().checksum(),
            #[cfg(feature="terrain-patrol")]
            patrol,
            scene_epoch: editor.scene_load_epoch(),
            host_epoch: editor.sim().epoch,
            scene,
            document_scene: document.scene()?.to_owned(),
            source: source.clone(),
            revision: document.revision()?,
            draft: [
                self.navigation.start[0].clone(),
                self.navigation.start[1].clone(),
                self.navigation.goal[0].clone(),
                self.navigation.goal[1].clone(),
                self.navigation.max_slope.clone(),
                self.navigation.distance_per_tick.clone(),
            ],
        })
    }

    pub(crate) fn navigation_endpoint_validate(&mut self) {
        if self.navigation_endpoint.armed().is_some()
            && (self.navigation_endpoint_pin(self.navigation_endpoint.armed()).is_none()
                || self.navigation_endpoint_pin(self.navigation_endpoint.armed()).as_ref() != self.navigation_endpoint.pin.as_ref())
        {
            self.navigation_endpoint.cancel(
                "Navigation pick cancelled: terrain, route settings or editing context changed",
            );
        }
    }

    pub(crate) fn navigation_endpoint_sync(&mut self, ctx: &Context) {
        if ctx.input(|i| {
            !i.pointer.button_down(egui::PointerButton::Primary)
                && !i.pointer.button_released(egui::PointerButton::Primary)
        }) {
            self.navigation_endpoint.consume_cancelled = false;
        }
        self.navigation_endpoint.focused = ctx.input(|input| {
            input.focused && !input.events.iter().any(|event| matches!(event,egui::Event::WindowFocused(false)))
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && self.navigation_endpoint.armed().is_some()
        {
            self.navigation_endpoint.cancel("Navigation pick cancelled");
        }
        self.navigation_endpoint_validate();
        #[cfg(feature = "terrain-patrol")]
        {
            let busy = self.navigation_endpoint_owns_input();
            self.navigation.set_patrol_order_busy(busy);
        }
    }

    /// Re-admit the actual saved local source at operation boundaries, using
    /// TerrainSession's existing lexical, symlink and asset admission policy.
    fn navigation_endpoint_source(&self) -> Result<&Terrain, String> {
        let (source, terrain) = self.navigation.source(&self.editor, &self.terrain)?;
        let mut saved = TerrainSession::default();
        saved.set_scene(self.editor.path().as_deref())?;
        saved.open_local(source)?;
        if saved.bytes() != self.terrain.document.bytes() {
            return Err(
                "Saved terrain changed; reopen it before picking navigation endpoints".into(),
            );
        }
        Ok(terrain)
    }

    pub(crate) fn navigation_endpoint_arm(&mut self, endpoint: Endpoint) -> Result<(), String> {
        let result: Result<(), String> = (|| {
            let pin = self.navigation_endpoint_pin(Some(endpoint)).ok_or("Navigation picking requires focused, saved local terrain in Edit mode; finish or cancel any ramp first")?;
            let terrain = self.navigation_endpoint_source()?;
            graph(terrain, &self.navigation.max_slope)?;
            self.navigation_endpoint.pin = Some(pin);
            self.navigation_endpoint.armed = Some(endpoint);
            self.navigation_endpoint.consume_cancelled = false;
            self.navigation_endpoint.status = format!(
                "Click a terrain cell for navigation {} (draft only)",
                match endpoint {
                    Endpoint::Start => "start".to_owned(),
                    Endpoint::Goal => "goal".to_owned(),
                    #[cfg(feature="terrain-patrol")]
                    Endpoint::Patrol(row) => format!("patrol waypoint {}", row+1),
                }
            );
            Ok(())
        })();
        if let Err(error) = &result {
            self.navigation_endpoint.cancel(error);
        }
        result
    }

    pub(crate) fn navigation_endpoint_owns_input(&self) -> bool {
        self.navigation_endpoint.armed().is_some() || self.navigation_endpoint.consume_cancelled
    }
    #[cfg(feature="terrain-patrol")]
    pub(crate) fn navigation_patrol_request(&mut self) {
        match self.navigation.take_patrol_pick_request() {
            Some(crate::navigation_panel::PatrolPickRequest::Pick(row)) => {let _ = self.navigation_endpoint_arm(Endpoint::Patrol(row));},
            Some(crate::navigation_panel::PatrolPickRequest::Cancel) => {
                if matches!(self.navigation_endpoint.armed(),Some(Endpoint::Patrol(_))) {
                    self.navigation_endpoint.cancel("Terrain patrol waypoint pick cancelled");
                }
            },
            None => {},
        }
    }
    #[cfg(feature="terrain-patrol")]
    pub(crate) fn navigation_patrol_frame_drawn(&mut self, drawn: bool) {
        if !drawn && matches!(self.navigation_endpoint.armed(),Some(Endpoint::Patrol(_))) {
            self.navigation_endpoint.cancel("Terrain patrol pick cancelled: no fresh rendered viewport");
        }
    }

    /// An owned miss or rejected/stale click is consumed, never entity selection.
    pub(crate) fn navigation_endpoint_pick(
        &mut self,
        camera: &Camera3D,
        pixel: [f32; 2],
        size: (u32, u32),
    ) -> bool {
        let owned = self.navigation_endpoint.armed().is_some()
            || self.navigation_endpoint.consume_cancelled;
        if !owned {
            return false;
        }
        self.navigation_endpoint_validate();
        let Some(endpoint) = self.navigation_endpoint.armed() else {
            return true;
        };
        let result = (|| -> Result<[FP; 2], String> {
            let terrain = self.navigation_endpoint_source()?;
            let cell = terrain_pick::pick_cell(terrain, camera, pixel, size)
                .ok_or("No terrain cell at this pointer")?;
            let center = cell_center(terrain, cell)?;
            graph(terrain, &self.navigation.max_slope)?
                .project(terrain, center)
                .map_err(|e| e.to_string())?;
            Ok(center)
        })();
        match result {
            Ok(center) => {
                let staged: Result<(), String> = match endpoint {
                    Endpoint::Start => { self.navigation.start = center.map(format_fixed); Ok(()) },
                    Endpoint::Goal => { self.navigation.goal = center.map(format_fixed); Ok(()) },
                    #[cfg(feature="terrain-patrol")]
                    Endpoint::Patrol(_) => self.navigation_endpoint.pin.as_ref()
                        .and_then(|pin|pin.patrol.clone()).ok_or("Terrain patrol pick token expired".to_owned())
                        .and_then(|token|self.navigation.stage_patrol_pick(&self.editor,&self.terrain,&token,center)),
                };
                match staged {
                    Ok(()) => self.navigation_endpoint.cancel("Navigation point captured in draft; explicitly apply the route and save the scene"),
                    Err(error) => self.navigation_endpoint.cancel(&format!("Navigation pick rejected: {error}; arm a pick again")),
                }
            }
            Err(error) => {
                self.navigation_endpoint.cancel(&format!(
                    "Navigation endpoint rejected: {error}; arm a pick to try again"
                ));
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_types, clippy::float_arithmetic)]
    use super::*;
    use crate::editor::Editor;

    fn terrain(origin: i64, spacing: i64) -> Terrain {
        Terrain::new(
            "endpoint/test".into(),
            3,
            3,
            [FP::from_raw(origin); 2],
            FP::from_raw(spacing),
            vec![FP::ZERO; 9],
            vec![false; 4],
        )
        .unwrap()
    }
    #[test]
    fn exact_negative_odd_and_minimum_raw_cell_centers_round_trip() {
        for (origin, spacing, expected) in [
            (-65536, 65536, -32768),
            (-20, 5, -18),
            (-5, 1, -5),
            (i64::MIN, 1, i64::MIN),
        ] {
            let t = terrain(origin, spacing);
            assert_eq!(
                cell_center(&t, [0, 0]).unwrap(),
                [FP::from_raw(expected); 2]
            );
            let center = cell_center(&t, [1, 1]).unwrap();
            assert_eq!(center, [FP::from_raw(expected + spacing); 2]);
            for value in center {
                assert_eq!(parse_fixed(&format_fixed(value)).unwrap(), value);
            }
            assert!(cell_center(&t, [2, 0]).is_err());
        }
    }
    #[test]
    fn holes_and_steep_triangles_are_rejected_without_nearest_snap() {
        let mut t = terrain(-65536, 65536);
        let mut document = orr_terrain::TerrainDocument::new(t.clone());
        document
            .apply(&[orr_terrain::Edit::SetHole {
                x: 0,
                z: 0,
                hole: true,
            }])
            .unwrap();
        t = document.terrain().clone();
        assert!(graph(&t, "1")
            .unwrap()
            .project(&t, cell_center(&t, [0, 0]).unwrap())
            .is_err());
        assert!(graph(&t, "1")
            .unwrap()
            .project(&t, cell_center(&t, [1, 1]).unwrap())
            .is_ok());
        let steep = Terrain::new(
            "steep".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::ZERO, FP::from_int(4), FP::ZERO, FP::from_int(4)],
            vec![false],
        )
        .unwrap();
        assert!(graph(&steep, "1")
            .unwrap()
            .project(&steep, cell_center(&steep, [0, 0]).unwrap())
            .is_err());
        assert!(graph(&steep, "bad").is_err());
    }

    pub(super) fn fixture() -> (tempfile::TempDir, EditorApp) {
        let root = tempfile::tempdir().unwrap();
        let scene = root.path().join("scene.yaml");
        std::fs::write(
            &scene,
            include_str!("../../../../scenes/navigation_blank.scene.yaml"),
        )
        .unwrap();
        let t = terrain(-65536, 65536);
        std::fs::write(root.path().join("terrain.orrt"), t.cook()).unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            editor.sync();
            if editor.can_mutate() && editor.yard_rows_coherent() && editor.snapshot().is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < until,
                "local editor did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut app = EditorApp::new(editor, None);
        app.terrain.document.set_scene(Some(&scene)).unwrap();
        app.terrain.document.open_local("terrain.orrt").unwrap();
        app.navigation.sync_from_editor(&app.editor);
        (root, app)
    }
    pub(super) fn camera() -> Camera3D {
        let mut camera = Camera3D::orthographic([0.0, 10.0, 0.0], [0.0, 0.0, 0.0], 2.0);
        camera.up = [0.0, 0.0, -1.0];
        camera
    }
    #[test]
    fn app_draft_only_capture_and_owned_rejections_preserve_host_and_terrain_history() {
        let (_root, mut app) = fixture();
        let history = app.editor.history().clone();
        let checksum = app.editor.snapshot().unwrap().predicted().checksum();
        let bytes = app.terrain.document.bytes().unwrap().to_vec();
        let selection = app.editor.selection().cloned();
        let camera = camera();
        let pixel = camera
            .world_to_screen([-0.5, 0.0, -0.5], (600, 600))
            .unwrap();
        app.navigation_endpoint_arm(Endpoint::Start).unwrap();
        assert!(app.navigation_endpoint_pick(&camera, pixel, (600, 600)));
        assert_eq!(app.navigation.start, ["-0.5", "-0.5"]);
        let goal = app.navigation.goal.clone();
        for case in 0..8 {
            app.navigation_endpoint_arm(Endpoint::Goal).unwrap();
            match case {
                0 => app.navigation.start[0].push('0'),
                1 => app.navigation.start[1].push('0'),
                2 => app.navigation.goal[0].push('0'),
                3 => app.navigation.goal[1].push('0'),
                4 => app.navigation.max_slope.push('0'),
                5 => app.navigation.distance_per_tick.push('0'),
                6 => app.navigation_endpoint.focused = false,
                _ => {}
            }
            let expected_start = app.navigation.start.clone();
            let expected_goal = app.navigation.goal.clone();
            assert!(app.navigation_endpoint_pick(
                &camera,
                if case == 7 { [f32::NAN, 0.0] } else { pixel },
                (600, 600)
            ));
            assert_eq!(app.navigation.start, expected_start);
            assert_eq!(app.navigation.goal, expected_goal);
            assert!(app.navigation_endpoint.armed().is_none());
            app.navigation.start = ["-0.5".into(), "-0.5".into()];
            app.navigation.goal = goal.clone();
            app.navigation.max_slope = "1".into();
            app.navigation.distance_per_tick = "0.25".into();
            app.navigation_endpoint.focused = true;
        }
        assert_eq!(app.editor.history(), &history);
        assert_eq!(
            app.editor.snapshot().unwrap().predicted().checksum(),
            checksum
        );
        assert_eq!(app.editor.selection(), selection.as_ref());
        assert_eq!(app.terrain.document.bytes().unwrap(), bytes);
        assert!(!app.terrain.document.can_undo());
        assert!(!app.terrain.document.dirty());
    }

    #[test]
    fn saved_source_replacement_and_active_transaction_reject_capture() {
        let (root, mut app) = fixture();
        let camera = camera();
        let pixel = camera
            .world_to_screen([-0.5, 0.0, -0.5], (600, 600))
            .unwrap();
        let draft = app.navigation.start.clone();
        app.navigation_endpoint_arm(Endpoint::Start).unwrap();
        std::fs::write(
            root.path().join("terrain.orrt"),
            terrain(-65536, 32768).cook(),
        )
        .unwrap();
        assert!(app.navigation_endpoint_pick(&camera, pixel, (600, 600)));
        assert_eq!(app.navigation.start, draft);
        assert!(app.navigation_endpoint.armed().is_none());
        assert!(app.navigation_endpoint_arm(Endpoint::Start).is_err());
        std::fs::write(
            root.path().join("terrain.orrt"),
            app.terrain.document.bytes().unwrap(),
        )
        .unwrap();
        app.navigation_endpoint_arm(Endpoint::Start).unwrap();
        app.editor
            .host_call("tx.begin", serde_json::json!({"label":"endpoint guard"}))
            .unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.editor.history().in_tx && !app.editor.sim().in_tx {
            app.editor.sync();
            assert!(
                std::time::Instant::now() < until,
                "transaction did not synchronize"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let history = app.editor.history().clone();
        assert!(app.navigation_endpoint_pick(&camera, pixel, (600, 600)));
        assert_eq!(app.navigation.start, draft);
        assert_eq!(app.editor.history(), &history);
        assert!(app.navigation_endpoint_arm(Endpoint::Start).is_err());
        app.editor
            .host_call("tx.rollback", serde_json::Value::Null)
            .unwrap();
    }
    #[test]
    fn cancelled_primary_press_remains_owned_until_release() {
        let (_root, mut app) = fixture();
        let ctx = Context::default();
        let camera = camera();
        let pixel = camera
            .world_to_screen([-0.5, 0.0, -0.5], (600, 600))
            .unwrap();
        app.navigation_endpoint_arm(Endpoint::Goal).unwrap();
        let event = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(300.0, 300.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let output = ctx.run_ui(
            egui::RawInput {
                events: vec![event(true)],
                ..Default::default()
            },
            |ui| app.navigation_endpoint_sync(ui.ctx()),
        );
        output.drop_without_applying_deltas();
        app.navigation.distance_per_tick = "0.5".into();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.navigation_endpoint_sync(ui.ctx())
        });
        output.drop_without_applying_deltas();
        assert!(app.navigation_endpoint.armed().is_none());
        let goal = app.navigation.goal.clone();
        let output = ctx.run_ui(
            egui::RawInput {
                events: vec![event(false)],
                ..Default::default()
            },
            |ui| app.navigation_endpoint_sync(ui.ctx()),
        );
        output.drop_without_applying_deltas();
        assert!(app.navigation_endpoint_pick(&camera, pixel, (600, 600)));
        assert_eq!(app.navigation.goal, goal);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.navigation_endpoint_sync(ui.ctx())
        });
        output.drop_without_applying_deltas();
        assert!(!app.navigation_endpoint_pick(&camera, pixel, (600, 600)));
    }
    fn wait_for(app: &mut EditorApp, ready: impl Fn(&EditorApp) -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.editor.sync();
            if ready(app) {
                return;
            }
            assert!(
                std::time::Instant::now() < until,
                "lifecycle did not settle: {:?}",
                app.editor.log()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn assert_lifecycle_rejects_owned_click(app: &mut EditorApp) {
        let draft = (app.navigation.start.clone(), app.navigation.goal.clone());
        let history = app.editor.history().clone();
        let checksum = app.editor.snapshot().unwrap().predicted().checksum();
        let bytes = app.terrain.document.bytes().unwrap().to_vec();
        let selection = app.editor.selection().cloned();
        let camera = camera();
        let pixel = camera
            .world_to_screen([-0.5, 0.0, -0.5], (600, 600))
            .unwrap();
        assert!(app.navigation_endpoint_pick(&camera, pixel, (600, 600)));
        assert!(app.navigation_endpoint.armed().is_none());
        assert_eq!(
            (app.navigation.start.clone(), app.navigation.goal.clone()),
            draft
        );
        assert_eq!(app.editor.history(), &history);
        assert_eq!(
            app.editor.snapshot().unwrap().predicted().checksum(),
            checksum
        );
        assert_eq!(app.terrain.document.bytes().unwrap(), bytes);
        assert_eq!(app.editor.selection(), selection.as_ref());
        assert!(!app.terrain.document.can_undo());
    }

    #[test]
    fn same_path_reload_and_actual_play_invalidate_endpoint_pick() {
        let (_root, mut app) = fixture();
        let scene = app.editor.path().unwrap();
        let epoch = app.editor.scene_load_epoch();
        let generation = app.editor.view_recovery_generation();
        app.navigation_endpoint_arm(Endpoint::Goal).unwrap();
        assert!(app.editor.open_path(&scene));
        assert_ne!(app.editor.scene_load_epoch(), epoch);
        wait_for(&mut app, |app| {
            app.editor.view_recovery_generation() > generation
                && app.editor.mode() == Mode::Edit
                && app.editor.yard_rows_coherent()
                && app
                    .editor
                    .snapshot()
                    .is_some_and(|s| s.timeline().is_none())
        });
        assert_lifecycle_rejects_owned_click(&mut app);

        app.navigation.start = ["-0.5".into(), "-0.5".into()];
        app.navigation.goal = ["0.5".into(), "0.5".into()];
        app.navigation
            .build_route(&mut app.editor, &app.terrain)
            .unwrap();
        wait_for(&mut app, |app| {
            app.editor.admitted_navigation().admitted
                && app.editor.admitted_navigation().error.is_none()
                && app.editor.yard_rows_coherent()
        });
        app.navigation_endpoint_arm(Endpoint::Start).unwrap();
        let generation = app.editor.view_recovery_generation();
        assert!(app.editor.start_play());
        wait_for(&mut app, |app| {
            app.editor.view_recovery_generation() > generation
                && app.editor.mode() == Mode::Play
                && app
                    .editor
                    .snapshot()
                    .is_some_and(|s| s.timeline().is_some())
        });
        assert_lifecycle_rejects_owned_click(&mut app);
        assert!(app.navigation_endpoint_arm(Endpoint::Goal).is_err());
        app.editor.stop();
        wait_for(&mut app, |app| {
            app.editor.mode() == Mode::Edit
                && app.editor.yard_rows_coherent()
                && app
                    .editor
                    .snapshot()
                    .is_some_and(|s| s.timeline().is_none())
        });
    }

    #[test]
    fn real_ramp_controls_cancel_navigation_and_keep_staged_ramp_on_refusal() {
        use egui_kittest::{kittest::Queryable, Harness};
        let (_root, mut app) = fixture();
        app.navigation_endpoint_arm(Endpoint::Start).unwrap();
        let draft = app.navigation.start.clone();
        let history = app.editor.history().clone();
        let bytes = app.terrain.document.bytes().unwrap().to_vec();
        let mut h = Harness::builder().with_size([900.0, 700.0]).build_ui_state(
            |ui, app: &mut EditorApp| {
                app.terrain_ramp_sync(ui.ctx());
                app.terrain_ramp_show(ui);
                app.navigation_endpoint_sync(ui.ctx());
            },
            app,
        );
        h.run_steps(3);
        let camera = camera();
        for (label, point) in [
            ("Pick ramp start", [-1.0, 0.0, 0.0]),
            ("Pick ramp end", [1.0, 0.0, 0.0]),
        ] {
            h.get_by_label(label).click();
            h.run_steps(3);
            let pixel = camera.world_to_screen(point, (600, 600)).unwrap();
            assert!(h.state_mut().terrain_ramp_pick(&camera, pixel, (600, 600)));
            h.run_steps(2);
        }
        assert!(h.state().navigation_endpoint.armed().is_none());
        assert_eq!(h.state().navigation.start, draft);
        h.get_by_label("Stage ramp").click();
        h.run_steps(3);
        let candidate = h
            .state()
            .terrain_ramp
            .staged
            .as_ref()
            .unwrap()
            .candidate
            .clone();
        assert!(h
            .state_mut()
            .navigation_endpoint_arm(Endpoint::Goal)
            .is_err());
        assert_eq!(
            h.state().terrain_ramp.staged.as_ref().unwrap().candidate,
            candidate
        );
        assert_eq!(h.state().editor.history(), &history);
        assert_eq!(h.state().terrain.document.bytes().unwrap(), bytes);
        assert!(!h.state().terrain.document.can_undo());
        assert!(!h.state().terrain.document.dirty());
    }
}

#[cfg(all(test,target_os="linux",feature="terrain-patrol"))]
#[path="navigation_patrol_pick_tests.rs"]
mod patrol_tests;

#[cfg(all(test, target_os = "linux", feature = "terrain-patrol"))]
#[path = "navigation_patrol_order_tests.rs"]
mod patrol_order_tests;
