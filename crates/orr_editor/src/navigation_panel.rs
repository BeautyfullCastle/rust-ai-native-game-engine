//! Explicit, scene-owned point-route authoring. Text fields are presentation
//! state; one reflected singleton edit admits the complete route to the host.
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    terrain_document::{format_fixed, parse_fixed, TerrainSource},
    terrain_panel::TerrainPanel,
};
use egui::Ui;
use orr_fp::FP;
use orr_navigation::{AgentProfile, Navigator, SearchBudget, TerrainGraph};
use orr_remote::navigation_yard3d::{NavigationAgentSpec, NavigationScenePin, PIN_NAME};
use serde_json::json;

pub struct NavigationPanel {
    pub start: [String; 2],
    pub goal: [String; 2],
    pub max_slope: String,
    pub distance_per_tick: String,
    pub error: Option<String>,
    loaded_scene: Option<String>,
    loaded_pin: Option<NavigationScenePin>,
    saved_pinned_source_mismatch: bool,
}

impl Default for NavigationPanel {
    fn default() -> Self {
        Self {
            start: ["-3".into(), "-3".into()],
            goal: ["3".into(), "3".into()],
            max_slope: "1".into(),
            distance_per_tick: "0.25".into(),
            error: None,
            loaded_scene: None,
            loaded_pin: None,
            saved_pinned_source_mismatch: false,
        }
    }
}

impl NavigationPanel {
    pub fn sync_from_editor(&mut self, editor: &Editor) {
        if editor.game() != EditorGame::NavigationYard3D {
            return;
        }
        if editor.snapshot().is_none() {
            return;
        }
        let scene = editor.path().map(|p| p.display().to_string());
        let pin = editor
            .snapshot()
            .map(|s| *s.predicted().singleton::<NavigationScenePin>());
        let pin = pin.filter(|p| p.source_len != 0);
        if self.loaded_scene == scene && self.loaded_pin == pin {
            return;
        }
        self.loaded_scene = scene;
        self.loaded_pin = pin;
        self.saved_pinned_source_mismatch = false;
        self.error = None;
        if let Some(pin) = pin {
            self.start = pin.agent.start.map(format_fixed);
            self.goal = pin.agent.goal.map(format_fixed);
            self.max_slope = format_fixed(pin.agent.max_slope);
            self.distance_per_tick = format_fixed(pin.agent.distance_per_tick);
        } else {
            *self = Self {
                loaded_scene: self.loaded_scene.clone(),
                ..Self::default()
            };
        }
    }

    /// Remember a saved change to the currently pinned local source even after
    /// the terrain document is closed. An unsaved edit remains live-only:
    /// Discard may restore the admitted route without a rebuild.
    pub fn observe_terrain(&mut self, editor: &Editor, terrain: &TerrainPanel) {
        if editor.game() != EditorGame::NavigationYard3D || !terrain.scene_matches(editor) {
            return;
        }
        let Some(pin) = editor
            .snapshot()
            .map(|s| *s.predicted().singleton::<NavigationScenePin>())
        else {
            return;
        };
        if pin.source_len == 0 || terrain.document.dirty() {
            return;
        }
        let Some(TerrainSource::Local { relative_path }) = terrain.document.source() else {
            return;
        };
        if relative_path != pin.source().unwrap_or_default() {
            return;
        }
        let Some(authored) = terrain.document.terrain() else {
            return;
        };
        self.saved_pinned_source_mismatch = terrain.document.revision()
            != Some(pin.terrain_revision)
            || authored.asset_id() != pin.identity().unwrap_or_default();
    }

    /// A locally edited terrain document cannot silently keep the old route.
    /// Host Frame state remains visible (red) until explicit Build succeeds.
    pub fn route_stale(&self, editor: &Editor, terrain: &TerrainPanel) -> bool {
        if editor.game() != EditorGame::NavigationYard3D {
            return false;
        }
        let Some(pin) = editor
            .snapshot()
            .map(|s| *s.predicted().singleton::<NavigationScenePin>())
        else {
            return false;
        };
        if pin.source_len == 0 {
            return false;
        }
        if self.saved_pinned_source_mismatch {
            return true;
        }
        if !self.draft_matches_pin(&pin) {
            return true;
        }
        if !terrain.scene_matches(editor) {
            return false;
        }
        let Some(authored) = terrain.document.terrain() else {
            return false;
        };
        terrain.document.dirty()
            || terrain.document.revision() != Some(pin.terrain_revision)
            || authored.asset_id() != pin.identity().unwrap_or_default()
            || !matches!(terrain.document.source(), Some(TerrainSource::Local { relative_path }) if relative_path == pin.source().unwrap_or_default())
    }

    fn draft_matches_pin(&self, pin: &NavigationScenePin) -> bool {
        let Ok(values) = (|| -> Result<NavigationAgentSpec, String> {
            Ok(NavigationAgentSpec {
                start: [parse_fixed(&self.start[0])?, parse_fixed(&self.start[1])?],
                goal: [parse_fixed(&self.goal[0])?, parse_fixed(&self.goal[1])?],
                max_slope: parse_fixed(&self.max_slope)?,
                distance_per_tick: parse_fixed(&self.distance_per_tick)?,
            })
        })() else {
            return false;
        };
        values == pin.agent
    }

    pub fn play_blocker(&self, editor: &Editor, terrain: &TerrainPanel) -> Option<String> {
        if editor.game() != EditorGame::NavigationYard3D {
            return None;
        }
        let pin = editor
            .snapshot()
            .map(|s| *s.predicted().singleton::<NavigationScenePin>())?;
        if pin.source_len == 0 {
            return Some("Build a navigation route before Play".into());
        }
        if self.route_stale(editor, terrain) {
            return Some(
                "Terrain or route settings changed; save terrain if needed, then Build route again"
                    .into(),
            );
        }
        if !editor.admitted_navigation().admitted {
            return Some(
                editor
                    .admitted_navigation()
                    .error
                    .clone()
                    .unwrap_or_else(|| "Wait for an admitted navigation Frame".into()),
            );
        }
        editor.admitted_navigation().error.clone()
    }

    fn source<'a>(
        &self,
        editor: &Editor,
        terrain: &'a TerrainPanel,
    ) -> Result<(&'a str, &'a orr_terrain::Terrain), String> {
        if editor.game() != EditorGame::NavigationYard3D || !editor.spec().is_local() {
            return Err("Navigation authoring requires a local NavigationYard3D host".into());
        }
        if editor.mode() != Mode::Edit || !editor.can_mutate() || editor.previewing().is_some() {
            return Err("Stop Play and clear previews before building a route".into());
        }
        if !terrain.scene_matches(editor) {
            return Err("Open terrain belonging to this saved scene".into());
        }
        if terrain.document.dirty() {
            return Err("Save terrain before building the route".into());
        }
        let source = match terrain.document.source() {
            Some(TerrainSource::Local { relative_path }) => relative_path.as_str(),
            Some(TerrainSource::Package { .. }) => {
                return Err("Copy the package terrain to this scene before building a route".into())
            }
            None => return Err("Create or open terrain first".into()),
        };
        let terrain = terrain.document.terrain().ok_or("Open terrain first")?;
        Ok((source, terrain))
    }

    pub fn build_route(
        &mut self,
        editor: &mut Editor,
        terrain: &TerrainPanel,
    ) -> Result<(), String> {
        let result = (|| {
            let (source, terrain) = self.source(editor, terrain)?;
            let spec = NavigationAgentSpec {
                start: [parse_fixed(&self.start[0])?, parse_fixed(&self.start[1])?],
                goal: [parse_fixed(&self.goal[0])?, parse_fixed(&self.goal[1])?],
                max_slope: parse_fixed(&self.max_slope)?,
                distance_per_tick: parse_fixed(&self.distance_per_tick)?,
            };
            if spec.distance_per_tick <= FP::ZERO {
                return Err("Distance per tick must be positive".into());
            }
            if terrain.width() > 17 || terrain.depth() > 17 {
                return Err("Navigation terrain is limited to 17 × 17 vertices".into());
            }
            let profile = AgentProfile {
                max_slope: spec.max_slope,
                radius: FP::ZERO,
                headroom: FP::ZERO,
                max_step: FP::ZERO,
            };
            let graph = TerrainGraph::build(terrain, profile).map_err(|e| e.to_string())?;
            if graph.triangles().len() > 512 {
                return Err("Navigation graph exceeds 512 triangles".into());
            }
            // Validate the same projection/path and presentation boundaries
            // before the one atomic host edit. A rejected view cannot leave an
            // admitted but invisible route in an ordinary editor Build.
            let mut navigator =
                Navigator::new(&graph, terrain, spec.start).map_err(|e| e.to_string())?;
            navigator
                .replan(&graph, terrain, spec.goal, SearchBudget::default())
                .map_err(|e| e.to_string())?;
            crate::navigation_view::validate_candidate(terrain, &graph, &navigator, spec)?;
            let pin = NavigationScenePin::new(source, terrain, &graph, spec)?;
            editor.host_call(
                "world.singleton.patch",
                json!({"name":PIN_NAME,"value":{
                    "source_len":pin.source_len,
                    "identity_len":pin.identity_len,
                    "source_path":pin.source_path.as_slice(),
                    "asset_id":pin.asset_id.as_slice(),
                    "terrain_revision":pin.terrain_revision,
                    "graph_revision":pin.graph_revision,
                    "agent":{
                        "start":spec.start.map(format_fixed),
                        "goal":spec.goal.map(format_fixed),
                        "max_slope":format_fixed(spec.max_slope),
                        "distance_per_tick":format_fixed(spec.distance_per_tick),
                    }
                }}),
            )?;
            editor.navigation_mark_edited();
            self.loaded_pin = Some(pin);
            self.saved_pinned_source_mismatch = false;
            Ok(())
        })();
        self.error = result.as_ref().err().cloned();
        result
    }

    pub fn show(&mut self, ui: &mut Ui, editor: &mut Editor, terrain: &TerrainPanel) {
        if editor.game() != EditorGame::NavigationYard3D {
            return;
        }
        self.sync_from_editor(editor);
        self.observe_terrain(editor, terrain);
        ui.collapsing("Point navigation", |ui| {
            ui.weak("One point agent · terrain triangles · zero radius, headroom and step");
            let context = self.source(editor, terrain);
            if let Err(reason) = &context { ui.label(reason); }
            let enabled = context.is_ok();
            ui.add_enabled_ui(editor.mode() == Mode::Edit, |ui| {
                field(ui, "Navigation start X", &mut self.start[0]);
                field(ui, "Navigation start Z", &mut self.start[1]);
                field(ui, "Navigation goal X", &mut self.goal[0]);
                field(ui, "Navigation goal Z", &mut self.goal[1]);
                field(ui, "Navigation maximum slope", &mut self.max_slope);
                field(ui, "Navigation distance per tick", &mut self.distance_per_tick);
                if ui.add_enabled(enabled, egui::Button::new("Build route")).clicked() {
                    let _ = self.build_route(editor, terrain);
                }
            });
            if self.route_stale(editor, terrain) {
                ui.colored_label(egui::Color32::RED, "Terrain or route settings changed. Save terrain if needed, then Build route again");
            }
            let view = editor.admitted_navigation();
            if view.admitted {
                ui.label(format!("Admitted: {} ({})", view.identity, view.source));
                if let (Some(status), Some(position)) = (view.status, view.position) {
                    ui.label(format!(
                        "Tick {} · {:?} · position [{}, {}, {}]",
                        view.tick,
                        status,
                        format_fixed(position[0]),
                        format_fixed(position[1]),
                        format_fixed(position[2]),
                    ));
                }
                ui.weak("Host Frame owns the route and movement; use Play, Pause, Step, Seek and Stop below");
            }
            if let Some(error) = self.error.as_ref().or(view.error.as_ref()) {
                ui.colored_label(egui::Color32::RED, error);
            }
        });
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        let label_response = ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(115.0))
            .labelled_by(label_response.id);
    });
}
