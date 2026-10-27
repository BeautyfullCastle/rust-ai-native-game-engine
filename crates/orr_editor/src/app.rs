//! The egui application: menu and toolbar, hierarchy, inspector, viewport,
//! timeline and history panels (layout of design doc 7.3, simplified).
//!
//! All state changes go through [`Editor`]; this file only draws and
//! translates input into `Editor` calls.

use std::io::{self, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Instant;

use egui::{Color32, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2, Rect, RichText, Sense, Ui};
use orr_fp::{FPVec2, FP};
use orr_reflect::Value;
use orr_remote::{CaptureError, CaptureRequest, CapturedImage, ViewState};

use crate::editor::{fp_of_f64, Editor, Message, Mode, Owner};
use crate::inspector::{show_component, InspEvent};
use crate::model::{EntityRow, Target};
use crate::viewport::{self, GpuViewport};

#[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
#[path = "app/arena_export.rs"]
mod arena_export;

#[path = "app/close_review.rs"]
mod close_review;
#[path = "app/work_status.rs"]
mod work_status;
#[path = "app/export_guide.rs"]
mod export_guide;

#[cfg(test)]
#[path = "app/export_guide_tests.rs"]
mod export_guide_tests;

#[cfg(test)]
#[path = "app/work_status_tests.rs"]
mod work_status_tests;

#[cfg(all(test, feature = "sprites"))]
#[path = "app/unsaved_close_tests.rs"]
mod unsaved_close_tests;

#[cfg(feature = "terrain")]
#[path = "app/terrain_ramp.rs"]
mod terrain_ramp;

#[cfg(all(test, feature = "terrain"))]
#[path = "app/terrain_ramp_tests.rs"]
mod terrain_ramp_tests;

#[cfg(feature = "mesh-navigation")]
#[path = "app/mesh_workspace_save.rs"]
mod mesh_workspace_save;

#[cfg(feature = "mesh-export")]
#[path = "app/mesh_recorded_playback.rs"]
mod mesh_recorded_playback;

#[cfg(feature = "mesh-export")]
#[path = "app/mesh_player_session.rs"]
mod mesh_player_session;

#[cfg(feature = "room-project")]
#[path = "app/room_camera_capture.rs"]
mod room_camera_capture;

#[cfg(feature = "room-authoring")]
#[path = "app/room_wall_footprint.rs"]
mod room_wall_footprint;
#[cfg(feature = "room-authoring")]
#[path = "app/room_actor_placement.rs"]
mod room_actor_placement;
#[cfg(feature="room-authoring")]
#[path="app/room_wall_placement.rs"]
mod room_wall_placement;
#[cfg(feature="package-manager")]
#[path="app/package_model_preview.rs"]
mod package_model_preview;
#[cfg(feature="models")]
#[path="app/blockout_uv_density.rs"]
mod blockout_uv_density;
#[cfg(feature="models")]
#[path="app/frame_selection.rs"]
mod frame_selection;
#[path="app/selection_outline.rs"]
mod selection_outline;

#[cfg(feature = "collect-ui")]
#[path = "app/collect_ui_preview.rs"]
mod collect_ui_preview;
#[cfg(feature = "collect-ui")]
#[path = "app/collect_ui_reparent.rs"]
mod collect_ui_reparent;
#[cfg(feature = "collect-ui")]
#[path = "app/collect_ui_position.rs"]
mod collect_ui_position;
#[cfg(feature = "collect-ui")]
#[path = "app/collect_ui_alignment.rs"]
mod collect_ui_alignment;

#[cfg(feature = "collect-ui")]
#[path = "app/collect_ui_resize.rs"]
mod collect_ui_resize;

#[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
#[path = "app/collect_controls.rs"]
mod collect_controls;

#[cfg(feature = "static-game-actions")]
#[path = "app/static_actions.rs"]
mod static_actions;

#[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
#[path = "app/collect_project.rs"]
mod collect_project;

/// Button labels (tests click them by these texts).
pub const LBL_PLAY: &str = "\u{25B6} Play";
/// See [`LBL_PLAY`].
pub const LBL_PAUSE: &str = "\u{23F8} Pause";
/// See [`LBL_PLAY`].
pub const LBL_STEP: &str = "\u{23ED} Step";
/// See [`LBL_PLAY`].
pub const LBL_STOP: &str = "\u{25A0} Stop";

/// The button that starts a stopped local host again (tests click it by this text).
pub const LBL_RESTART: &str = "Restart simulation host";
/// The button that reconnects to a remote host.
pub const LBL_RECONNECT: &str = "Reconnect";


/// Which tab the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BottomTab {
    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
    ArenaExport,
    /// Play controls and the scrub bar.
    #[default]
    Timeline,
    /// The undo history.
    History,
    /// What AI agents do through ERP: a read-only activity feed.
    Agent,
    /// Explicitly opened, read-only inventory of retained authoring work.
    UnsavedWork,
    /// User-selected guidance to existing export routes, not an export permit.
    ExportGuide,
}

/// A path prompt (no native file dialog in the MVP).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogKind {
    /// Open a scene file.
    Open,
    /// Save under a new name.
    SaveAs,
    #[cfg(feature="mesh-navigation")]
    NewMesh,
    #[cfg(feature="mesh-navigation")]
    OpenMesh,
}

/// An open path prompt.
#[derive(Clone, Debug)]
pub struct Dialog {
    /// What the path is for.
    pub kind: DialogKind,
    /// The text of the path field.
    pub text: String,
}

/// An entity an agent just edited: the viewport outlines it for [`viewport::PULSE_SECONDS`].
#[derive(Clone, Debug, PartialEq)]
pub struct Pulse {
    /// The entity (GUID, or handle in play mode).
    pub target: Target,
    /// egui time (seconds) the pulse started.
    pub started: f64,
}

/// A viewport drag of a body in progress.
#[derive(Clone, Copy, Debug)]
struct BodyDrag {
    /// Body position minus pointer world position at the press.
    offset: [f32; 2],
}

/// Screen state of the window that is not part of the document.
#[derive(Default)]
pub struct UiState {
    /// Advisory close review; dismissal never grants permission to exit later.
    close_review: close_review::State,
    /// Bottom panel tab.
    pub bottom_tab: BottomTab,
    /// Hierarchy filter text.
    pub filter: String,
    /// Open path prompt.
    pub dialog: Option<Dialog>,
    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
    collect_project: collect_project::Panel,
    /// Where the viewport was drawn last frame, in points.
    pub viewport_rect: Option<Rect>,
    /// Size of the viewport in pixels last frame.
    pub viewport_px: (u32, u32),
    /// View-only opt-in. Exposure and tone mapping continue to come from Lighting.
    #[cfg(feature = "models")]
    pub yard_post_process: orr_render::PostProcessSettings,
    /// Last rejected display-policy/frame request; the previous texture stays visible.
    #[cfg(feature = "models")]
    pub yard_post_process_error: Option<String>,
    /// Entities an agent just edited, pulsing in the viewport (see [`Pulse`]).
    pub pulses: Vec<Pulse>,
    body_drag: Option<BodyDrag>,
    yard_transform_target: Option<Target>,
    yard_xyz: [String;3],
    yard_quat: [String;4],
    #[cfg(feature = "static-game-actions")]
    static_actions: static_actions::Router,
    input_player: u8,
    take_control: bool,
    #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
    collect_controls: collect_controls::CollectControls,
    #[cfg(feature = "animated-models")]
    animated_window: bool,
}

/// A request to save the app's own framebuffer as a PNG and quit (see [`crate::cli`]).
pub struct ScreenshotJob {
    /// Output file.
    pub path: PathBuf,
    /// Frames to render before asking for the screenshot.
    pub frames: u64,
    requested_at: Option<u64>,
    settle: bool,
    started_at: Option<f64>,
}

const MAX_CAPTURE_PIXELS: u64 = 1_048_576;
const MAX_CAPTURE_RGBA_BYTES: usize = 4 << 20;

#[derive(Clone, Copy)]
struct FramebufferInfo {
    viewport: egui::ViewportId,
    logical_width: f32,
    logical_height: f32,
    pixels_per_point: f32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct CapturePass {
    state: ViewState,
    frame_seq: u64,
    ui_frame: u64,
    framebuffer: FramebufferInfo,
}

struct RemoteCapture {
    request: CaptureRequest,
    pass: Option<CapturePass>,
    screenshot_requested: bool,
    image_received: bool,
    orphaned: bool,
}

struct EncoderWorker {
    serial: u64,
    pass: CapturePass,
    handle: JoinHandle<Result<Vec<u8>, CaptureError>>,
}

struct BoundedPngWriter<'a>(&'a mut Vec<u8>);

impl Write for BoundedPngWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_CAPTURE_RGBA_BYTES {
            return Err(io::Error::other("PNG exceeds screenshot byte limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ScreenshotJob {
    /// Screenshot after `frames` frames.
    pub fn new(path: PathBuf, frames: u64) -> Self {
        Self { path, frames, requested_at: None, settle: false, started_at: None }
    }

    /// Also wait for refreshed panels, a current paused frame and expired
    /// agent pulses. Readiness is checked without blocking the UI; failure
    /// to settle within ten seconds of UI time fails the screenshot job.
    pub fn with_settle(mut self) -> Self {
        self.settle = true;
        self
    }

    fn ready(&mut self, frames: u64, now: f64, waiting_for: Option<&str>) -> Result<bool, String> {
        let started = *self.started_at.get_or_insert(now);
        if !self.settle {
            return Ok(frames >= self.frames);
        }
        if now - started >= 10.0 {
            return Err(format!("settle deadline exceeded waiting for {}", waiting_for.unwrap_or("minimum frame count")));
        }
        Ok(frames >= self.frames && waiting_for.is_none())
    }
}

/// The editor window.
pub struct EditorApp {
    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
    arena_export: arena_export::State,
    /// The state machine.
    pub editor: Editor,
    /// Window-only state.
    pub ui: UiState,
    render_state: Option<egui_wgpu::RenderState>,
    gpu: Option<GpuViewport>,
    gpu3d: Option<crate::viewport3d::GpuViewport3d>,
    viewport_hdr_disabled: bool,
    #[cfg(feature = "static-gameplay")]
    registered_gameplay_view: crate::registered_gameplay_view::State,
    #[cfg(feature="irradiance-probes")]
    pub irradiance: crate::irradiance_panel::IrradiancePanel,
    #[cfg(feature="models")]
    pub models: crate::model_panel::ModelPanel,
    #[cfg(feature="models")]
    blockout_uv_density: blockout_uv_density::Panel,
    #[cfg(feature="terrain")]
    pub terrain: crate::terrain_panel::TerrainPanel,
    #[cfg(feature = "terrain")]
    pub(crate) terrain_ramp: terrain_ramp::State,
    #[cfg(feature="navigation")]
    pub navigation: crate::navigation_panel::NavigationPanel,
    #[cfg(feature="mesh-navigation")]
    pub mesh_navigation: crate::mesh_navigation::Panel,
    #[cfg(feature = "mesh-navigation")]
    mesh_play_controls: crate::mesh_play_controls::Panel,
    #[cfg(feature = "mesh-export")]
    mesh_player_input_allowed: bool,
    #[cfg(feature = "mesh-export")]
    mesh_recorded_playback: mesh_recorded_playback::State,
    #[cfg(feature = "mesh-export")]
    mesh_player_session: mesh_player_session::State,
    #[cfg(feature = "mesh-navigation")]
    mesh_camera: crate::mesh_camera_panel::Panel,
    #[cfg(feature = "mesh-navigation")]
    mesh_lighting: crate::mesh_lighting_panel::Panel,
    #[cfg(feature="mesh-navigation")]
    mesh_hud: crate::mesh_hud_panel::Panel,
    #[cfg(feature="mesh-mission")]
    mesh_mission: crate::mesh_mission_panel::Panel,
    #[cfg(feature="mesh-hazards")]
    mesh_hazards: crate::mesh_hazards_panel::Panel,
    #[cfg(feature="mesh-hazards")]
    mesh_hazard_pointer_owned: bool,
    #[cfg(feature="mesh-mission")]
    mesh_game_creator: crate::mesh_game_creator::Panel,
    #[cfg(all(windows,target_arch="x86_64",feature="mesh-navigation"))]
    mesh_copy: crate::mesh_workspace::copy_panel::Panel,
    #[cfg(feature = "mesh-animated-models")]
    mesh_animation: crate::mesh_animation_panel::Panel,
    #[cfg(feature="mesh-export")]
    mesh_export: crate::mesh_export_panel::Panel,
    #[cfg(feature="mesh-hazards-export")]
    mesh_hazards_export: crate::mesh_hazards_export_panel::Panel,
    #[cfg(all(windows, target_arch="x86_64", feature="windows-export"))]
    pub windows_export: crate::windows_export_panel::Panel,
    #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
    pub windows_projects: crate::windows_project_panel::Panel,
    #[cfg(feature="room-authoring")]
    pub room_authoring: crate::room_authoring_panel::Panel,
    #[cfg(feature="room-authoring")]
    room_wall_pointer_owned: bool,
    #[cfg(feature="room-authoring")]
    room_actor_pointer_owned: bool,
    #[cfg(feature="room-authoring")]
    room_wall_placement_pointer_owned: bool,
    #[cfg(all(any(windows,target_os="linux"),target_arch="x86_64",feature="room-authoring"))]
    room_wall_stamps: crate::room_wall_stamp_panel::Panel,
    #[cfg(feature="package-manager")]
    pub package_manager: crate::package_manager_panel::Panel,
    #[cfg(feature="package-manager")]
    package_model_preview: package_model_preview::PreviewWindow,
    shot: Option<ScreenshotJob>,
    remote_capture: Option<RemoteCapture>,
    encoder: Option<EncoderWorker>,
    ui_settled_reported: bool,
    frames: u64,
    last_title: String,
    #[cfg(feature = "sprites")]
    pub sprites: crate::sprite_panel::SpritePanel,
    #[cfg(feature = "tilemap")]
    pub tilemap: Option<crate::tilemap_panel::TilemapPanel>,
    #[cfg(feature = "project-ui")]
    pub project_ui: Option<crate::project_ui::Preview>,
    #[cfg(feature="collect-ui")]
    pub collect_ui: Option<crate::collect_ui_panel::Panel>,
    #[cfg(feature="collect-ui")]
    collect_ui_preview: collect_ui_preview::PreviewWindow,
    #[cfg(feature="room-project")]
    pub room_camera: Option<crate::room_camera_panel::Panel>,
    #[cfg(feature="room-character")]
    pub room_character: Option<crate::room_character_panel::Panel>,
    #[cfg(feature = "room-lighting")]
    pub room_lighting: Option<crate::room_lighting_panel::Panel>,
    #[cfg(feature="room-project")]
    room_camera_capture: room_camera_capture::Capture,
    #[cfg(feature = "linked-prefabs")]
    pub linked_prefabs: crate::linked_prefab_panel::Panel,
    #[cfg(feature = "animated-models")]
    pub animated_models: crate::animated_panel::AnimatedPanel,
}

impl EditorApp {
    /// An app on `editor`. `render_state` is eframe's wgpu state (None without
    /// a GPU: the viewport then shows a notice, everything else works).
    pub fn new(editor: Editor, render_state: Option<egui_wgpu::RenderState>) -> Self {
        Self { #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))] arena_export: arena_export::State::default(), editor, #[cfg(feature="room-character")] room_character: None, #[cfg(feature="tilemap")] tilemap: None, #[cfg(feature="mesh-export")] mesh_player_session: mesh_player_session::State::default(), #[cfg(feature="mesh-mission")] mesh_game_creator: crate::mesh_game_creator::Panel::default(), #[cfg(feature="mesh-export")] mesh_recorded_playback: mesh_recorded_playback::State::default(), #[cfg(feature="mesh-export")] mesh_player_input_allowed: true, #[cfg(feature="mesh-navigation")] mesh_play_controls: crate::mesh_play_controls::Panel::default(), #[cfg(all(any(windows,target_os="linux"),target_arch="x86_64",feature="room-authoring"))] room_wall_stamps: crate::room_wall_stamp_panel::Panel::default(), #[cfg(feature="mesh-navigation")] mesh_lighting: crate::mesh_lighting_panel::Panel::default(), #[cfg(feature="mesh-navigation")] mesh_hud: crate::mesh_hud_panel::Panel::default(), #[cfg(feature="mesh-mission")] mesh_mission: crate::mesh_mission_panel::Panel::default(), #[cfg(feature="mesh-hazards")] mesh_hazards: crate::mesh_hazards_panel::Panel::default(), #[cfg(feature="mesh-hazards")] mesh_hazard_pointer_owned: false, #[cfg(all(windows,target_arch="x86_64",feature="mesh-navigation"))] mesh_copy: crate::mesh_workspace::copy_panel::Panel::default(), #[cfg(feature="mesh-animated-models")] mesh_animation: crate::mesh_animation_panel::Panel::default(), #[cfg(feature="mesh-navigation")] mesh_camera: crate::mesh_camera_panel::Panel::default(), #[cfg(feature="mesh-export")] mesh_export: crate::mesh_export_panel::Panel::default(), #[cfg(feature="mesh-hazards-export")] mesh_hazards_export: crate::mesh_hazards_export_panel::Panel::default(), #[cfg(feature="package-manager")] package_manager: crate::package_manager_panel::Panel::default(), #[cfg(feature="package-manager")] package_model_preview: package_model_preview::PreviewWindow::default(), #[cfg(feature="room-authoring")] room_authoring: crate::room_authoring_panel::Panel::default(), #[cfg(feature="room-authoring")] room_wall_pointer_owned: false, #[cfg(feature="room-authoring")] room_actor_pointer_owned: false, #[cfg(feature="room-authoring")] room_wall_placement_pointer_owned: false, #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))] windows_projects: crate::windows_project_panel::Panel::default(), #[cfg(all(windows, target_arch="x86_64", feature="windows-export"))] windows_export: crate::windows_export_panel::Panel::default(), #[cfg(feature="room-lighting")] room_lighting: None, #[cfg(feature="room-project")] room_camera: None, #[cfg(feature="room-project")] room_camera_capture: room_camera_capture::Capture::default(), #[cfg(feature="linked-prefabs")] linked_prefabs: crate::linked_prefab_panel::Panel::default(), #[cfg(feature="collect-ui")] collect_ui: None, #[cfg(feature="collect-ui")] collect_ui_preview: collect_ui_preview::PreviewWindow::default(), #[cfg(feature="project-ui")] project_ui: None, #[cfg(feature="navigation")] navigation: crate::navigation_panel::NavigationPanel::default(), #[cfg(feature="mesh-navigation")] mesh_navigation: crate::mesh_navigation::Panel::default(), #[cfg(feature="terrain")] terrain_ramp: terrain_ramp::State::default(), #[cfg(feature="terrain")] terrain: crate::terrain_panel::TerrainPanel::default(), ui: UiState::default(), render_state, gpu: None, gpu3d: None, viewport_hdr_disabled: false, #[cfg(feature = "static-gameplay")] registered_gameplay_view: crate::registered_gameplay_view::State::default(), #[cfg(feature="irradiance-probes")] irradiance: crate::irradiance_panel::IrradiancePanel::default(), #[cfg(feature="models")] models: crate::model_panel::ModelPanel::default(), #[cfg(feature="models")] blockout_uv_density: blockout_uv_density::Panel::default(), shot: None, remote_capture: None, encoder: None, ui_settled_reported: false, frames: 0, last_title: String::new(), #[cfg(feature = "sprites")] sprites: crate::sprite_panel::SpritePanel::default(), #[cfg(feature = "animated-models")] animated_models: crate::animated_panel::AnimatedPanel::default() }
    }

    #[cfg(feature="package-manager")]
    fn package_manager_root(&self) -> Option<PathBuf> {
        if !self.editor.game().is_room() || !self.editor.spec().is_local() { return None; }
        let scene = self.editor.path()?;
        scene.parent()?.ancestors().take(16)
            .find(|root| root.join("orr.project.json").is_file()).map(|root|root.to_path_buf())
    }

    #[cfg(feature="package-manager")]
    fn package_manager_blocker(&self) -> Option<String> {
        if !self.editor.game().is_room() || !self.editor.spec().is_local()
            || self.editor.mode() != Mode::Edit || !self.editor.can_mutate()
            || self.editor.previewing().is_some() || self.editor.in_gesture()
            || self.editor.history().in_tx || self.editor.sim().in_tx {
            return Some("Content packages require an idle editable local Room project".into());
        }
        #[cfg(feature="room-character")]
        if self.room_character.as_ref().is_some_and(|panel| panel.dirty()) {
            return Some("Save or undo character edits before changing projects or packages".into());
        }
        if self.editor.is_dirty() || self.models.bindings.as_ref().is_some_and(|bindings| bindings.dirty())
            || self.room_camera.as_ref().is_some_and(|camera| camera.dirty()) {
            return Some("Save the scene and model/camera sidecars before managing packages".into());
        }
        if self.shot.is_some() || self.remote_capture.is_some() || self.encoder.is_some() || self.ui.dialog.is_some() {
            return Some("Finish capture and path dialogs before managing packages".into());
        }
        #[cfg(all(windows,target_arch="x86_64",feature="windows-export"))]
        if self.windows_export.open || self.windows_export.busy() {
            return Some("Close the Windows export window before managing packages".into());
        }
        #[cfg(all(windows,target_arch="x86_64",feature="windows-projects"))]
        if self.windows_projects.busy() {
            return Some("Finish project creation/opening before managing packages".into());
        }
        None
    }

    #[cfg(feature="package-manager")]
    fn package_model_request(&mut self) {
        let Some((package,asset,kind)) = self.package_manager.take_model_asset_request() else { return };
        let result = (|| -> Result<(),String> {
            if let Some(blocked) = self.package_manager_blocker() { return Err(blocked); }
            let root = self.package_manager_root().ok_or("No current Room project")?;
            let guid = self.editor.selected_guid().ok_or("Select a persistent entity before assigning a package model")?;
            let transform = self.models.bindings.as_ref()
                .and_then(|bindings|bindings.document().bindings.get(&guid.to_string()))
                .map(|binding|binding.transform).unwrap_or_default();
            self.models.project = root.display().to_string();
            self.models.package = package.clone();
            self.models.asset = asset.clone();
            self.models.kind = kind;
            #[cfg(feature="animated-models")]
            if kind == orr_model_bindings::model_bindings::ModelKind::Animated {
                self.models.animation = orr_model_bindings::model_bindings::AnimationDescriptor {
                    clip_index: 0,
                    playback: orr_model_bindings::model_bindings::PlaybackMode::Loop,
                };
            }
            self.models.transform = transform;
            self.models.load_candidate(&self.editor)?;
            self.models.assign(&self.editor)
        })();
        self.package_manager.status = match result {
            Ok(()) => format!("Assigned verified {}model {package}/{asset}. Save model bindings in the inspector to persist it.", if kind == orr_model_bindings::model_bindings::ModelKind::Animated { "animated " } else { "static " }),
            Err(error) => format!("Package model was not assigned: {error}"),
        };
    }

    fn hazard_pick_busy(&self) -> bool {
        #[cfg(feature="mesh-hazards")]
        { self.mesh_hazards.hazard_pick_busy() }
        #[cfg(not(feature="mesh-hazards"))]
        { false }
    }

    fn hazard_export_busy(&self) -> bool {
        #[cfg(feature="mesh-hazards-export")]
        { self.mesh_hazards_export.busy() }
        #[cfg(not(feature="mesh-hazards-export"))]
        { false }
    }

    fn package_removal_pending(&self) -> bool {
        #[cfg(feature="package-manager")]
        { self.package_manager.removal_busy() }
        #[cfg(not(feature="package-manager"))]
        { false }
    }

    fn package_work_pending(&self) -> bool {
        #[cfg(feature="package-manager")]
        { self.package_manager.busy() }
        #[cfg(not(feature="package-manager"))]
        { false }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_workspace_blocker(&self) -> Option<String> {
        #[cfg(feature="mesh-mission")]
        if self.mesh_game_creator.busy() { return Some("Finish or cancel new mesh game creation before changing workspaces".into()); }
        self.mesh_workspace_base_blocker()
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_workspace_base_blocker(&self) -> Option<String> {
        if !matches!(self.editor.game(), crate::game::EditorGame::PhysGame | crate::game::EditorGame::MeshNavigation) {
            return Some("Legacy workspace changes require Physics or MeshNavigation".into());
        }
        self.mesh_creation_base_blocker()
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_creation_base_blocker(&self) -> Option<String> {
        if self.hazard_export_busy() { return Some("Wait for hazard export completion before changing workspaces".into()); }
        #[cfg(all(windows,target_arch="x86_64"))]
        if self.mesh_copy.busy() { return Some("Finish the mesh workspace duplicate before switching workspaces".into()); }
        #[cfg(feature="mesh-export")]
        if self.mesh_export.busy() {
            return Some("Finish the mesh game export before changing mesh workspaces".into());
        }
        self.mesh_creation_authoring_blocker()
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_animation_dirty(&self) -> bool {
        #[cfg(feature="mesh-animated-models")]
        { self.mesh_animation.dirty() }
        #[cfg(not(feature="mesh-animated-models"))]
        { false }
    }

    #[cfg(all(windows,target_arch="x86_64",feature="mesh-navigation"))]
    fn mesh_copy_blocker(&self) -> Option<String> {
        #[cfg(feature="mesh-export")]
        if self.mesh_export.busy() { return Some("Finish the mesh game export before duplicating the workspace".into()); }
        if !self.editor.game().is_mesh_navigation() { return Some("Open a saved mesh workspace before duplicating it".into()); }
        self.mesh_authoring_blocker().or_else(|| self.mesh_presentation_blocker())
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_animation_blocker(&self) -> Option<String> {
        if !self.editor.game().is_mesh_navigation() { return None; }
        #[cfg(feature="mesh-animated-models")]
        { self.mesh_animation.play_blocker(&self.editor) }
        #[cfg(not(feature="mesh-animated-models"))]
        {
            let scene = self.editor.path()?;
            match crate::mesh_animation_store::has_sidecar(&scene) {
                Ok(false) => None,
                Ok(true) => Some("Mesh animation sidecar is present; rebuild the editor with mesh-animated-models support".into()),
                Err(error) => Some(format!("Cannot admit mesh animation sidecar: {error}")),
            }
        }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_hazards_dirty(&self) -> bool {
        #[cfg(feature="mesh-hazards")]
        { self.mesh_hazards.dirty() }
        #[cfg(not(feature="mesh-hazards"))]
        { false }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_hazards_blocker(&self) -> Option<String> {
        #[cfg(feature="mesh-hazards")]
        { self.mesh_hazards.play_blocker(&self.editor) }
        #[cfg(not(feature="mesh-hazards"))]
        { None }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_mission_dirty(&self) -> bool {
        #[cfg(feature="mesh-mission")]
        { self.mesh_mission.dirty() || self.mesh_mission.mission_pick_busy() }
        #[cfg(not(feature="mesh-mission"))]
        { false }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_mission_pick_busy(&self) -> bool {
        #[cfg(feature="mesh-mission")]
        { self.mesh_mission.mission_pick_busy() }
        #[cfg(not(feature="mesh-mission"))]
        { false }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_mission_blocker(&self) -> Option<String> {
        #[cfg(feature="mesh-mission")]
        { self.mesh_mission.play_blocker(&self.editor) }
        #[cfg(not(feature="mesh-mission"))]
        { None }
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_presentation_blocker(&self) -> Option<String> {
        self.mesh_navigation.play_blocker(&self.editor)
            .or_else(|| self.mesh_camera.play_blocker(&self.editor))
            .or_else(|| self.mesh_lighting.play_blocker(&self.editor))
            .or_else(|| self.mesh_hud.play_blocker(&self.editor))
            .or_else(|| self.mesh_mission_blocker())
            .or_else(|| self.mesh_hazards_blocker())
            .or_else(|| self.mesh_animation_blocker())
    }

    #[cfg(feature="mesh-navigation")]
    fn sync_mesh_presentation(&mut self) {
        self.mesh_camera.sync(&mut self.editor);
        self.mesh_lighting.sync(&self.editor);
        self.mesh_hud.sync(&self.editor);
        #[cfg(feature="mesh-mission")]
        self.mesh_mission.sync(&self.editor);
        #[cfg(feature="mesh-hazards")]
        self.mesh_hazards.sync(&self.editor);
        #[cfg(feature="mesh-animated-models")]
        self.mesh_animation.sync(&self.editor);
        let geometry_pending = self.mesh_camera.dirty() || self.mesh_lighting.dirty() || self.mesh_hud.dirty() || self.mesh_mission_dirty() || self.mesh_hazards_dirty() || self.mesh_animation_dirty();
        self.mesh_navigation.set_geometry_presentation_pending(geometry_pending);
        self.mesh_navigation.sync(&self.editor);
        let blocker = self.mesh_presentation_blocker();
        self.editor.set_mesh_navigation_blocker(blocker);
    }

    #[cfg(all(feature="mesh-navigation", any(feature="mesh-export", all(windows,target_arch="x86_64"))))]
    fn mesh_authoring_blocker(&self) -> Option<String> {
        if !matches!(self.editor.game(), crate::game::EditorGame::PhysGame | crate::game::EditorGame::MeshNavigation) {
            return Some("Legacy mesh authoring requires Physics or MeshNavigation".into());
        }
        self.mesh_creation_authoring_blocker()
    }

    #[cfg(feature="mesh-navigation")]
    fn mesh_creation_authoring_blocker(&self) -> Option<String> {
        if self.mesh_navigation.agent_pick_busy() || self.mesh_mission_pick_busy() || self.hazard_pick_busy() {
            return Some("Finish or cancel viewport endpoint picking before changing workspaces or exporting".into());
        }
        if self.mesh_navigation.prop_gesture_busy() {
            return Some("Finish or cancel the viewport prop move before changing mesh workspaces or exporting".into());
        }
        if self.mesh_navigation.art_import_busy() {
            return Some("Finish model package imports before changing mesh workspaces or exporting".into());
        }
        if !(self.editor.game() == crate::game::EditorGame::PhysGame || self.editor.game().has_mesh_view())
            || !self.editor.spec().is_local() || self.editor.mode() != Mode::Edit
            || !self.editor.can_mutate() || self.editor.previewing().is_some()
            || self.editor.in_gesture() || self.editor.erp_status().is_some() {
            return Some("Mesh workspace changes require an idle local Physics or MeshNavigation editor without an ERP listener".into());
        }
        if self.editor.is_dirty() || self.mesh_navigation.has_unplaced_draft() || self.mesh_camera.dirty() || self.mesh_lighting.dirty() || self.mesh_hud.dirty() || self.mesh_mission_dirty() || self.mesh_hazards_dirty() || self.mesh_animation_dirty() {
            return Some("Save or explicitly undo/discard scene, unplaced mesh, mission, camera, lighting, HUD, animation, agent art and prop changes before changing mesh workspaces".into());
        }
        if self.package_work_pending() || self.shot.is_some() || self.remote_capture.is_some()
            || self.encoder.is_some() || self.ui.dialog.is_some() {
            return Some("Finish content operations, captures and path dialogs before changing mesh workspaces".into());
        }
        #[cfg(feature="models")]
        if self.models.bindings.as_ref().is_some_and(|bindings| bindings.dirty()) {
            return Some("Save or explicitly discard model bindings before changing mesh workspaces".into());
        }
        #[cfg(feature="sprites")]
        if self.sprites.bindings.as_ref().is_some_and(|bindings| bindings.dirty()) {
            return Some("Save or explicitly discard sprite bindings before changing mesh workspaces".into());
        }
        #[cfg(feature="animated-models")]
        if self.animated_models.bindings.as_ref().is_some_and(|bindings| bindings.dirty()) {
            return Some("Save or explicitly discard animation preview bindings before changing mesh workspaces".into());
        }
        #[cfg(feature="terrain")]
        if self.terrain.document.dirty() {
            return Some("Save or explicitly discard terrain edits before changing mesh workspaces".into());
        }
        #[cfg(feature="irradiance-probes")]
        if self.irradiance.bindings.as_ref().is_some_and(|bindings| bindings.dirty())
            || self.irradiance.is_baking() || self.irradiance.is_validating_bake() {
            return Some("Finish irradiance work and save or explicitly discard its bindings before changing mesh workspaces".into());
        }
        #[cfg(feature="room-character")]
        if self.room_character.as_ref().is_some_and(|panel| panel.dirty()) {
            return Some("Save or undo character edits before changing projects or packages".into());
        }
        #[cfg(feature="room-project")]
        if self.room_camera.as_ref().is_some_and(|camera| camera.dirty()) {
            return Some("Save or explicitly discard camera edits before changing mesh workspaces".into());
        }
        #[cfg(all(windows, target_arch="x86_64", feature="windows-export"))]
        if self.windows_export.busy() {
            return Some("Finish Windows export before changing mesh workspaces".into());
        }
        #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
        if self.windows_projects.busy() {
            return Some("Finish Room project preparation before changing mesh workspaces".into());
        }
        None
    }

    #[cfg(feature="mesh-navigation")]
    fn switch_mesh_workspace(&mut self, path: &std::path::Path, intent: crate::mesh_workspace::Intent) -> bool {
        let candidate = (|| -> Result<Self, String> {
            if let Some(blocked) = self.mesh_workspace_blocker() { return Err(blocked); }
            let original = self.editor.mesh_workspace_source_stamp()?;
            let editor = crate::mesh_workspace::prepare(path, intent)?;
            let mut replacement = Self::new(editor, self.render_state.clone());
            replacement.sync_mesh_presentation();
            #[cfg(not(feature="mesh-animated-models"))]
            if let Some(blocked) = replacement.mesh_animation_blocker() { return Err(blocked); }
            replacement.viewport_hdr_disabled = self.viewport_hdr_disabled;
            if let Some(blocked) = self.mesh_workspace_blocker() { return Err(blocked); }
            if self.editor.mesh_workspace_source_stamp()? != original {
                return Err("Current document changed during preparation; save it before retrying the requested mesh workspace".into());
            }
            Ok(replacement)
        })();
        match candidate {
            Ok(mut replacement) => {
                replacement.gpu = self.gpu.take();
                replacement.gpu3d = self.gpu3d.take();
                *self = replacement;
                true
            }
            Err(error) => {
                self.editor.error(format!("Current workspace retained; cannot use {}: {error}. Any newly created scene is retained.", path.display()));
                false
            }
        }
    }

    #[cfg(feature="mesh-mission")]
    fn mesh_game_creation_blocker(&self) -> Option<String> {
        #[cfg(feature="mesh-hazards")]
        if self.mesh_game_creator.hazard_request().is_some() {
            return self.mesh_creation_base_blocker();
        }
        self.mesh_workspace_base_blocker()
    }

    #[cfg(feature="mesh-mission")]
    fn create_mesh_game(&mut self, path: &std::path::Path, settings: &crate::mesh_workspace::mission::Settings) -> bool {
        let candidate = (|| -> Result<Self, String> {
            if let Some(blocked) = self.mesh_game_creation_blocker() { return Err(blocked); }
            let original = self.editor.mesh_creation_source_stamp()?;
            #[cfg(feature="mesh-hazards")]
            let editor = match self.mesh_game_creator.hazard_request() {
                Some(include_hazard) => crate::mesh_hazards_creator::prepare(path, settings, include_hazard)?,
                None => crate::mesh_workspace::mission::prepare(path, settings)?,
            };
            #[cfg(not(feature="mesh-hazards"))]
            let editor = crate::mesh_workspace::mission::prepare(path, settings)?;
            let mut replacement = Self::new(editor, self.render_state.clone());
            replacement.sync_mesh_presentation();
            if let Some(blocked) = replacement.mesh_presentation_blocker() { return Err(blocked); }
            replacement.viewport_hdr_disabled = self.viewport_hdr_disabled;
            if let Some(blocked) = self.mesh_game_creation_blocker() { return Err(blocked); }
            if self.editor.mesh_creation_source_stamp()? != original {
                return Err("Current document changed during new game preparation; save it before retrying".into());
            }
            Ok(replacement)
        })();
        match candidate {
            Ok(mut replacement) => {
                self.mesh_game_creator.succeeded();
                replacement.gpu = self.gpu.take();
                replacement.gpu3d = self.gpu3d.take();
                let ready = if replacement.editor.game().is_mesh_hazards() {
                    format!("New hazard game ready: {}. Customize objectives in Mesh mission and spheres in Mesh hazards.", path.display())
                } else {
                    format!("New mesh game ready: {}. Play follows the authored route; customize it in Mesh mission.", path.display())
                };
                replacement.editor.info(ready);
                *self = replacement;
                true
            }
            Err(error) => {
                let message = format!("Current workspace retained; cannot create {}: {error}. Any fully created scene is retained.", path.display());
                self.mesh_game_creator.failed(message.clone());
                self.editor.error(message);
                false
            }
        }
    }

    #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
    fn windows_project_blocker(&self) -> Option<String> {
        #[cfg(feature="mesh-mission")]
        if self.mesh_game_creator.busy() { return Some("Finish or cancel new mesh game creation before changing projects".into()); }
        #[cfg(feature="package-manager")]
        if self.package_manager.busy() {
            return Some("Finish the content package operation before changing projects".into());
        }
        #[cfg(feature="mesh-navigation")]
        if self.mesh_navigation.art_dirty() || self.mesh_camera.dirty() || self.mesh_lighting.dirty() || self.mesh_hud.dirty() || self.mesh_mission_dirty() || self.mesh_hazards_dirty() || self.mesh_animation_dirty() {
            return Some("Save or explicitly discard mission, camera, lighting, HUD, animation, agent art and prop changes before changing projects".into());
        }
        if (!self.editor.game().is_room() && self.editor.game() != crate::game::EditorGame::PhysGame) || !self.editor.spec().is_local()
            || self.editor.mode() != Mode::Edit || !self.editor.can_mutate()
            || self.editor.previewing().is_some() || self.editor.in_gesture() {
            return Some("Room project changes require an idle local Room or PhysGame editor in Edit mode".into());
        }
        if self.editor.erp_status().is_some() {
            return Some("Close the ERP-listening editor and reopen without --erp before switching projects; its endpoint is never silently replaced".into());
        }
        #[cfg(feature="sprites")]
        if self.sprites.bindings.as_ref().is_some_and(|bindings| bindings.dirty()) {
            return Some("Save or explicitly discard sprite bindings before changing projects".into());
        }
        #[cfg(feature="animated-models")]
        if self.animated_models.bindings.as_ref().is_some_and(|bindings| bindings.dirty()) {
            return Some("Save or explicitly discard animation preview bindings before changing projects".into());
        }
        #[cfg(feature="terrain")]
        if self.terrain.document.dirty() {
            return Some("Save or explicitly discard terrain edits before changing projects".into());
        }
        #[cfg(feature="irradiance-probes")]
        if self.irradiance.bindings.as_ref().is_some_and(|bindings| bindings.dirty())
            || self.irradiance.is_baking() || self.irradiance.is_validating_bake() {
            return Some("Finish irradiance work and save or explicitly discard its bindings before changing projects".into());
        }
        #[cfg(feature="room-character")]
        if self.room_character.as_ref().is_some_and(|panel| panel.dirty()) {
            return Some("Save or undo character edits before changing projects or packages".into());
        }
        if self.editor.is_dirty()
            || self.models.bindings.as_ref().is_some_and(|bindings| bindings.dirty())
            || self.room_camera.as_ref().is_some_and(|camera| camera.dirty()) {
            return Some("Save the current scene and model/camera sidecars, or explicitly undo/discard their edits, before changing projects".into());
        }
        if self.windows_export.busy() || self.shot.is_some() || self.remote_capture.is_some()
            || self.encoder.is_some() || self.ui.dialog.is_some() {
            return Some("Finish active export, capture and path dialogs before changing projects".into());
        }
        None
    }

    #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
    fn poll_windows_project(&mut self) {
        self.windows_projects.poll();
        let Some(root) = self.windows_projects.take_ready() else { return };
        if let Some(blocked) = self.windows_project_blocker() {
            self.windows_projects.status = format!("Project is retained at {}; open it after resolving: {blocked}", root.display());
            return;
        }
        let original_game = self.editor.game();
        let original = match self.editor.windows_project_source_stamp() {
            Ok(stamp) => stamp,
            Err(error) => {
                self.windows_projects.status = format!("Current project retained: {error}. Requested project: {}", root.display());
                return;
            }
        };
        let candidate = (|| -> Result<Self, String> {
            let mut prepared = orr_sample::windows_project::open_room_project(&root)?;
            let expected_checksum = prepared.scene().frame().checksum();
            let camera = prepared.take_camera();
            let (_, path, scene, models) = prepared.into_parts();
            let mut editor = Editor::start(&crate::HostSpec::PreparedRoom {
                scene: path, text: scene.text().to_owned(), listen: None, debug_hooks: false,
            })?;
            editor.sync();
            if editor.checksum() != expected_checksum {
                return Err("New Room host differs from the admitted project".into());
            }
            let camera = match camera {
                Some(camera) => {
                    editor.install_room_camera(camera.document.clone())?;
                    Some(crate::room_camera_panel::Panel::new(camera))
                }
                None => None,
            };
            let mut replacement = Self::new(editor, self.render_state.clone());
            replacement.models.install_room(models)?;
            replacement.room_camera = camera;
            replacement.viewport_hdr_disabled = self.viewport_hdr_disabled;
            if let Some(blocked) = self.windows_project_blocker() { return Err(blocked); }
            if self.editor.game() != original_game || self.editor.windows_project_source_stamp()? != original {
                return Err("Current document changed during preparation; save it and reopen the requested project".into());
            }
            Ok(replacement)
        })();
        match candidate {
            Ok(mut replacement) => {
                replacement.windows_projects = std::mem::take(&mut self.windows_projects);
                replacement.windows_projects.root = root.display().to_string();
                replacement.windows_projects.status = format!("Opened Room project: {}", root.display());
                replacement.gpu = self.gpu.take();
                replacement.gpu3d = self.gpu3d.take();
                *self = replacement;
            }
            Err(error) => {
                self.windows_projects.status = format!("Current project retained; cannot open {}: {error}", root.display());
            }
        }
    }

    /// Deliberately restrict HDR for compatibility. This one-way builder must
    /// be applied before the first frame; the GPU device/adapter is unchanged.
    pub fn without_hdr_support(mut self) -> Self {
        assert!(self.gpu3d.is_none(), "restrict HDR before the first viewport frame");
        self.viewport_hdr_disabled = true;
        self
    }

    /// Asks for a screenshot of the window after some frames, then quits.
    pub fn with_screenshot(mut self, job: ScreenshotJob) -> Self {
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
        if self.arena_export.busy() {
            self.arena_export.status = "Screenshot mode refused while an Arena export worker is owned; wait for terminal and joined completion.".into();
            return self;
        }
        self.shot = Some(job);
        self
    }

    /// Frames drawn so far.
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// True if the viewport draws on the GPU.
    pub fn has_gpu(&self) -> bool {
        self.render_state.is_some()
    }

    /// The viewport's GPU texture target, once it has drawn (readback in tests).
    pub fn viewport3d_gpu(&self) -> Option<&crate::viewport3d::GpuViewport3d> { self.gpu3d.as_ref() }

    pub fn viewport_gpu(&self) -> Option<&GpuViewport> {
        self.gpu.as_ref()
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.with_close_review(ui, Self::editor_ui);
    }
}

impl EditorApp {
    fn editor_ui(&mut self, ui: &mut Ui) {
        let started = Instant::now();
        let ctx = ui.ctx().clone();
        self.frames += 1;

        // Disarm before ingesting a late claim/update; focus returning is not intent.
        let keyboard_viewport = if self.editor.game().is_room() { "room_keyboard_viewport" } else { "arena_keyboard_viewport" };
        let input_focus = ctx.memory(|m| m.focused()) == Some(egui::Id::new(keyboard_viewport));
        let outside_click = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !self.ui.viewport_rect.is_some_and(|r| r.contains(p))));
        // Registered presenters own focus inside their complete viewport, including
        // project buttons and explicit payload fields. Other games keep their key focus contract.
        let keyboard_focus_lost = if self.editor.game().is_gameplay() { outside_click }
            else { !self.editor.game().has_mesh_view() && (!input_focus || outside_click) };
        #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
        let mapped_collect = self.editor.game().is_collect();
        #[cfg(not(all(feature = "collect-ui", feature = "collect-input-map-editor")))]
        let mapped_collect = false;
        let collect_owned_on_entry = mapped_collect && input_focus && !outside_click
            && self.ui.dialog.is_none() && ctx.input(|input| input.focused)
            && matches!(self.editor.input_phase(), crate::editor::input::Phase::Claiming | crate::editor::input::Phase::Active);
        #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
        let mapped_release = if mapped_collect {
            self.ui.collect_controls.sync_session(self.editor.scene_load_epoch(), true);
            let enabled = input_focus && !outside_click && self.ui.dialog.is_none()
                && self.editor.previewing().is_none() && self.editor.can_take_control()
                && matches!(self.editor.input_phase(), crate::editor::input::Phase::Claiming | crate::editor::input::Phase::Active);
            let release = ctx.input(|input| self.ui.collect_controls.release_requested(&input.events, enabled && input.focused));
            if release {
                // Commit the release frame once before pump: retain physical
                // quarantine, discard gameplay intents, and keep Quit out of UI shortcuts.
                let routed = ctx.input(|input| self.ui.collect_controls.route(&input.events, enabled && input.focused));
                ctx.input_mut(|input| {
                    let mut index = 0;
                    input.events.retain(|_| {
                        let keep = routed.consumed.binary_search(&index).is_err();
                        index += 1;
                        keep
                    });
                });
                ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new(keyboard_viewport)));
            }
            release
        } else { false };
        #[cfg(not(all(feature = "collect-ui", feature = "collect-input-map-editor")))]
        let mapped_release = false;
        if self.ui.dialog.is_some() || self.editor.previewing().is_some()
            || ctx.input(|input| !mapped_collect && input.key_pressed(Key::Escape)) || mapped_release {
            self.editor.release_control();
        } else if keyboard_focus_lost || ctx.input(|input| !input.focused) {
            #[cfg(feature = "static-gameplay")]
            self.editor.release_for_ui_focus_loss();
            #[cfg(not(feature = "static-gameplay"))]
            self.editor.release_control();
        }
        // The host simulates and edits on its own; this takes in what it says (never waits for it).
        self.editor.pump();
        #[cfg(feature="collect-ui")]
        self.sync_collect_ui_preview();
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
        if self.collect_project_modal(&ctx) { return; }
        #[cfg(feature="package-manager")]
        self.sync_package_model_preview();
        #[cfg(feature="room-project")]
        { self.sync_room_camera_capture(); if ctx.input(|input|input.key_pressed(Key::Escape)) { self.room_camera_capture.invalidate(); } }
        #[cfg(feature="package-manager")]
        if self.package_removal_pending() {
            // Freeze the whole input frame, including the completion frame: no
            // keyboard, AccessKit, scene/model/camera or history action dispatch.
            self.editor.release_control();
            #[cfg(feature="room-authoring")]
            { self.room_authoring.cancel_wall_footprint(); self.room_authoring.cancel_actor_placement(); self.room_authoring.cancel_wall_placement(); }
            let root = self.package_manager_root();
            ui.label("Package selection change is committing; authoring and playback are paused until it completes.");
            self.package_manager.show(&ctx, root.as_deref(), Some("Wait for package selection change completion before editing, saving or playing"));
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
            return;
        }
        #[cfg(feature="room-authoring")]
        { self.sync_room_wall_footprint(&ctx); self.sync_room_actor_placement(&ctx); self.sync_room_wall_placement(&ctx); }
        #[cfg(feature="mesh-hazards")]
        {
            // Preserve ownership across sync/cancellation so this frame's pointer
            // cannot fall through to another picker or scene selection.
            self.mesh_hazard_pointer_owned = self.mesh_hazards.hazard_pick_busy();
            self.mesh_hazards.sync_hazard_picking(&self.editor);
            let cancel = ctx.input(|input| !input.focused || input.key_pressed(Key::Escape))
                || ctx.text_edit_focused() || self.ui.dialog.is_some()
                || self.editor.previewing().is_some() || self.hazard_export_busy()
                || self.mesh_navigation.art_import_busy() || self.mesh_navigation.has_unplaced_draft();
            if cancel { self.mesh_hazards.cancel_hazard_pick(); }
        }

        #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
        self.poll_windows_project();
        #[cfg(feature="mesh-navigation")]
        {
            self.sync_mesh_presentation();
        }
        #[cfg(feature="navigation")]
        {
            self.terrain.sync_for_editor(&self.editor);
            self.navigation.sync_from_editor(&self.editor);
            self.navigation.observe_terrain(&self.editor, &self.terrain);
            let blocker = self.navigation.play_blocker(&self.editor, &self.terrain);
            self.editor.set_navigation_blocker(blocker);
        }
        #[cfg(feature = "terrain")]
        self.terrain_ramp_sync(&ctx);
        self.poll_remote_capture_request();
        self.start_pulses(ctx.input(|i| i.time));
        if self.editor.agent_mut().take_tab_request() {
            self.ui.bottom_tab = BottomTab::Agent;
        }
        // Notifications and frames arrive any time: look again soon, and at once while playing.
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
        if self.editor.is_playing_mode() && self.editor.timeline().is_some_and(|t| t.playing) {
            ctx.request_repaint();
        }
        // Managed Collect resolves physical Escape before pump; a logical
        // Escape on another physical key must not cancel authoring or selection.
        let editor_escape = !collect_owned_on_entry && ctx.input(|i| i.key_pressed(Key::Escape));
        if editor_escape || ctx.input(|i| !i.focused) {
            // Escape cancels an active drag without changing its inspection
            // target. With no gesture, Escape clears the GUID selection.
            let cancelling_gesture = self.editor.in_gesture();
            #[cfg(feature="room-authoring")]
            let cancelling_gesture = cancelling_gesture || self.room_wall_pointer_owned || self.room_actor_pointer_owned || self.room_wall_placement_pointer_owned;
            #[cfg(feature="mesh-navigation")]
            let cancelling_gesture = {
                let moving = self.mesh_navigation.prop_gesture_busy() || self.mesh_navigation.agent_pick_busy() || self.mesh_navigation.geometry_vertex_busy() || self.mesh_mission_pick_busy() || self.hazard_pick_busy();
                #[cfg(feature="mesh-hazards")]
                let moving = moving || self.mesh_hazard_pointer_owned;
                self.mesh_navigation.cancel_prop_gesture();
                self.mesh_navigation.cancel_agent_pick();
                #[cfg(feature="mesh-mission")]
                self.mesh_mission.cancel_mission_pick();
                #[cfg(feature="mesh-hazards")]
                self.mesh_hazards.cancel_hazard_pick();
                if self.mesh_navigation.geometry_vertex_busy() { self.mesh_navigation.cancel_geometry_vertex(); }
                cancelling_gesture || moving
            };
            self.editor.cancel_edit();
            if !cancelling_gesture && editor_escape { self.editor.select(None); }
            // Focus loss may hide the physical release. Forget egui's drag
            // ownership so later moves cannot revive it; a fresh press starts
            // a fresh gesture even when the old pointer-up never arrived.
            ctx.stop_dragging();
            self.editor.end_edit();
            self.ui.body_drag = None;
        }
        self.shortcuts(&ctx);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
        if self.collect_project_modal(&ctx) { return; }
        if self.editor.down().is_some() {
            egui::Panel::top("down").show(ui, |ui| self.down_banner(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        // The Agent tab needs more room than the timeline; each has its own remembered height.
        let (bottom_id, bottom_h) = if self.ui.bottom_tab == BottomTab::Agent { ("bottom_agent", 470.0) } else { ("bottom", 120.0) };
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
        let (bottom_id, bottom_h) = if self.ui.bottom_tab == BottomTab::ArenaExport { ("bottom_arena_export", 350.0) } else { (bottom_id, bottom_h) };
        egui::Panel::bottom(bottom_id).resizable(true).default_size(bottom_h).show(ui, |ui| self.bottom(ui));
        egui::Panel::left("hierarchy").resizable(true).default_size(230.0).show(ui, |ui| self.hierarchy(ui));
        let inspector = egui::Panel::right("inspector").resizable(true).default_size(340.0);
        let inspector = if self.editor.game().has_mesh_view() { inspector.max_size(340.0) } else { inspector };
        inspector.show(ui, |ui| {
            #[cfg(feature = "terrain")]
            if self.editor.game() == crate::game::EditorGame::Yard3D || self.editor.game().is_navigation() {
                // Keep expanded terrain, ramp and navigation controls reachable
                // without changing the other games' inspector layout.
                egui::ScrollArea::vertical()
                    .id_salt("terrain_authoring_inspector")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.inspector(ui));
                return;
            }
            self.inspector(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));

        // A scrub widget can disappear after selection/navigation changes.
        // Release still closes its gesture even if that widget saw no End.
        if !ctx.input(|i| i.pointer.button_down(PointerButton::Primary)) {
            self.editor.end_edit();
            self.ui.body_drag = None;
        }
        self.dialogs(&ctx);
        #[cfg(feature="mesh-mission")]
        {
            let blocked = self.mesh_game_creation_blocker();
            if let Some((path, settings)) = self.mesh_game_creator.show(&ctx, blocked.as_deref()) {
                self.create_mesh_game(&path, &settings);
                ctx.request_repaint();
            }
        }
        #[cfg(feature="package-manager")]
        if self.editor.game().is_room() {
            let root = self.package_manager_root();
            let blocked = self.package_manager_blocker();
            self.package_manager.show(&ctx,root.as_deref(),blocked.as_deref());
            if self.package_removal_pending() { ctx.request_repaint(); return; }
            self.package_model_request();
        }
        #[cfg(feature="package-manager")]
        self.show_package_model_preview(&ctx);
        #[cfg(feature="collect-ui")]
        self.show_collect_ui_preview(&ctx);
        #[cfg(feature="room-authoring")]
        if self.editor.game().is_room() {
            let removal_blocker = if !self.models.scene_matches(&self.editor) || self.models.bindings.as_ref().is_none_or(|bindings| bindings.dirty()) {
                Some("Open matching model bindings and save or discard model edits before removing a Room wall.")
            } else { None };
            self.room_authoring.show(&ctx, &mut self.editor, removal_blocker);
        }
        #[cfg(all(any(windows,target_os="linux"),target_arch="x86_64",feature="room-authoring"))]
        if self.editor.game().is_room() {
            self.room_wall_stamps.show(&ctx, &mut self.editor);
        }
        #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
        {
            let blocked = self.windows_project_blocker();
            let editor = &mut self.editor;
            self.windows_projects.show(&ctx, blocked.as_deref(), || editor.windows_project_source_stamp().map(|_| ()));
        }
        #[cfg(all(windows, target_arch="x86_64", feature="windows-export"))]
        {
            let presentation_dirty = self.models.bindings.as_ref().is_some_and(|bindings| bindings.dirty())
                || self.room_camera.as_ref().is_some_and(|camera| camera.dirty());
            #[cfg(feature="room-character")]
            let presentation_dirty = presentation_dirty || self.room_character.as_ref().is_some_and(|panel| panel.dirty());
            self.windows_export.show(&ctx, &mut self.editor, presentation_dirty);
        }
        #[cfg(feature="mesh-hazards-export")]
        {
            let blocked = self.mesh_creation_authoring_blocker().or_else(|| self.mesh_presentation_blocker());
            self.mesh_hazards_export.show(&ctx, &mut self.editor, blocked.as_deref());
        }
        #[cfg(feature="mesh-export")]
        {
            let blocked = self.mesh_authoring_blocker().or_else(|| self.mesh_presentation_blocker());
            self.mesh_export.show(&ctx, &mut self.editor, blocked.as_deref());
        }
        #[cfg(all(windows,target_arch="x86_64",feature="mesh-navigation"))]
        {
            if self.mesh_copy.visible() || self.mesh_copy.busy() {
            let blocked = self.mesh_copy_blocker();
            if let Some(path) = self.mesh_copy.show(&ctx, &mut self.editor, blocked) {
                if self.switch_mesh_workspace(&path, crate::mesh_workspace::Intent::Open) {
                    self.editor.info(format!("Mesh workspace duplicate ready: {}", path.display()));
                    ctx.request_repaint();
                }
            }
            }
        }
        #[cfg(feature = "animated-models")]
        egui::Window::new("Animation authoring · local sidecar")
            .open(&mut self.ui.animated_window)
            .default_pos([280.0, 70.0])
            .default_width(540.0)
            .resizable(true)
            .show(&ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.animated_models.show(ui, &self.editor, self.render_state.as_ref());
                });
            });
        #[cfg(feature = "animated-models")]
        if !self.ui.animated_window { self.animated_models.suspend_preview(); }
        if !self.ui.pulses.is_empty() {
            // The pulses animate until they expire.
            ctx.request_repaint();
        }

        let title = self.editor.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.process_remote_capture(&ctx);
        self.report_ui_settled(&ctx);
        self.screenshot(&ctx);
        self.editor.record_ui_frame(started.elapsed());
    }
}

impl EditorApp {
    #[cfg(feature="mesh-export")]
    pub(crate) fn set_mesh_player_input_allowed(&mut self, allowed: bool) {
        self.mesh_player_input_allowed = allowed;
        if !allowed { self.editor.set_mesh_play_input_enabled(false); }
    }

    #[cfg(feature="mesh-export")]
    pub(crate) fn mesh_player_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.frames += 1;
        if ctx.input(|input| !input.focused || input.key_pressed(Key::Escape)) {
            self.editor.set_mesh_play_input_enabled(false);
            if self.mesh_recorded_playback.active() {
                self.mesh_recorded_playback.pause(&mut self.editor);
            }
        }
        self.editor.pump();
        let now = ctx.input(|input| input.time);
        self.mesh_recorded_playback.pump(&mut self.editor, now);
        self.sync_mesh_presentation();
        egui::Panel::top("mesh_player_controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                let playing = self.mesh_recorded_playback.running() || self.editor.timeline().is_some_and(|timeline| timeline.playing);
                let available = self.editor.down().is_none() && self.editor.mode() == Mode::Play && !self.mesh_player_session.dialog_open();
                let label = if playing { "Pause" } else { "Resume" };
                if ui.add_enabled(available, egui::Button::new(label)).clicked() {
                    if self.mesh_recorded_playback.running() {
                        self.mesh_recorded_playback.pause(&mut self.editor);
                    } else if playing {
                        self.editor.pause();
                    } else if self.mesh_recorded_playback.active() || self.editor.sim().head_tick < self.editor.sim().last_tick {
                        if !self.mesh_recorded_playback.resume(&mut self.editor, now)
                            && self.mesh_recorded_playback.error().is_none()
                            && self.editor.sim().head_tick >= self.editor.sim().last_tick {
                            self.editor.play();
                        }
                    } else {
                        self.editor.play();
                    }
                }
                if ui.add_enabled(available, egui::Button::new("Replay")).clicked() {
                    self.mesh_recorded_playback.restart(&mut self.editor, now);
                }
                if self.mesh_player_session.show(ui, &mut self.editor, &mut self.mesh_recorded_playback) {
                    self.mesh_play_controls = crate::mesh_play_controls::Panel::default();
                    self.mesh_recorded_playback = mesh_recorded_playback::State::default();
                    self.sync_mesh_presentation();
                }
                if let Some(timeline) = self.editor.timeline() {
                    ui.label(format!("Tick {} · {}", timeline.tick, if timeline.playing { "Playing" } else { "Paused" }));
                }
                if self.mesh_recorded_playback.active() {
                    ui.label(if self.mesh_recorded_playback.running() { "Replaying recorded timeline" } else { "Recorded timeline paused" });
                }
                if let Some(error) = self.editor.down() {
                    ui.colored_label(Color32::RED, &error.reason);
                }
            });
            ui.horizontal(|ui| {
                let sim = self.editor.sim();
                let first = sim.first_tick;
                let last = sim.last_tick;
                let mut tick = sim.head_tick;
                let seek_available = self.editor.down().is_none()
                    && self.editor.mode() == Mode::Play
                    && !sim.playing
                    && !self.mesh_player_session.dialog_open()
                    && first <= last;
                if ui.add_enabled(seek_available,
                    egui::Slider::new(&mut tick, first..=last.max(first))
                        .text("Recorded tick").integer()).changed() {
                    self.editor.set_mesh_play_input_enabled(false);
                    self.mesh_recorded_playback.seek_recorded(&mut self.editor, tick, now);
                }
                let mut speed = self.mesh_recorded_playback.speed();
                ui.add_enabled_ui(seek_available, |ui| {
                    egui::ComboBox::from_id_salt("mesh_recorded_speed")
                        .selected_text(speed.label())
                        .show_ui(ui, |ui| {
                            for candidate in mesh_recorded_playback::PlaybackSpeed::ALL {
                                ui.selectable_value(&mut speed, candidate, candidate.label());
                            }
                        });
                });
                if speed != self.mesh_recorded_playback.speed() {
                    self.mesh_recorded_playback.set_speed(&self.editor, speed, now);
                }
            });
            if let Some(error) = self.mesh_recorded_playback.error() {
                ui.colored_label(Color32::RED, error);
            }
            if self.mesh_player_input_allowed {
                ui.add_enabled_ui(!self.mesh_player_session.dialog_open() && !self.mesh_recorded_playback.active(), |ui| self.mesh_play_controls.show(ui, &mut self.editor));
            } else {
                ui.label("Legacy replay-only mesh bundle · live goal input unavailable");
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.yard_viewport(ui));
        self.screenshot(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }

    /// Turns the entities agents just touched into pulses (and drops the expired ones).
    fn start_pulses(&mut self, now: f64) {
        for name in self.editor.feed_mut().take_touched() {
            if let Some(target) = Target::parse(&name) {
                self.ui.pulses.retain(|p| p.target != target);
                self.ui.pulses.push(Pulse { target, started: now });
            }
        }
        self.ui.pulses.retain(|p| now - p.started < viewport::PULSE_SECONDS);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
        if self.collect_project_dialog_open() { return; }
        #[cfg(feature="mesh-mission")]
        if self.mesh_game_creator.busy() { return; }
        #[cfg(feature="mesh-navigation")]
        if self.mesh_navigation.art_import_busy() || self.mesh_navigation.prop_gesture_busy() || self.mesh_navigation.agent_pick_busy() || self.mesh_navigation.geometry_vertex_busy() || self.mesh_mission_pick_busy() || self.hazard_pick_busy() { return; }
        if self.editor.input_phase() != crate::editor::input::Phase::Off || ctx.egui_wants_keyboard_input() || !self.editor.can_mutate() || self.editor.previewing().is_some() {
            return;
        }
        let redo_a = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo_b = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let (do_redo, do_undo, do_save) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&redo_a) | i.consume_shortcut(&redo_b);
            let undo = !redo && i.consume_shortcut(&undo);
            (redo, undo, i.consume_shortcut(&save))
        });
        if do_redo {
            self.editor.redo();
        }
        if do_undo {
            self.editor.undo();
        }
        if do_save {
            self.save();
        }
    }

    // Modal pointer capture does not block accessibility actions. Check both widget
    // availability and the action entry point before starting or advancing playback.
    fn playback_start_blocker(&self) -> Option<String> {
        #[cfg(feature = "tilemap")]
        if self.tilemap.as_ref().is_some_and(|panel| panel.dirty()) { return Some("Save tilemap or Undo its edits before Play or Step".into()); }
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
        if self.collect_project_dialog_open() { return Some("Finish or cancel the Collect project dialog before Play or Step".into()); }
        if self.package_removal_pending() { return Some("Wait for package selection change completion before Play or Step".into()); }
        if self.hazard_pick_busy() { return Some("Finish or cancel hazard center picking before Play or Step".into()); }
        if self.hazard_export_busy() { return Some("Wait for hazard export completion before Play or Step".into()); }
        #[cfg(feature="mesh-mission")]
        if self.mesh_game_creator.busy() {
            return Some("Finish or cancel new game creation before Play or Step".into());
        }
        #[cfg(feature="mesh-navigation")]
        if self.editor.mode() != Mode::Play {
            return self.mesh_presentation_blocker();
        }
        None
    }

    fn play_from_ui(&mut self) {
        if self.playback_start_blocker().is_none() { self.editor.play(); }
    }

    fn step_from_ui(&mut self) {
        if self.playback_start_blocker().is_none() { self.editor.step(1); }
    }

    fn restart_from_ui(&mut self) {
        #[cfg(feature = "tilemap")]
        if self.tilemap.is_some() {
            self.editor.error("Tilemap host/cache identity is pinned; save tiles and reopen the complete project instead of Restart/Reconnect");
            return;
        }
        self.editor.restart();
    }

    fn save(&mut self) {
        #[cfg(feature = "tilemap")]
        if let Some(panel) = &self.tilemap {
            if !panel.scene_matches(&self.editor) {
                self.editor.error("Tilemap host identity changed; reopen the complete project before Scene Save");
                return;
            }
            let scene = self.editor.path();
            let result = scene.ok_or_else(|| "tilemap requires its saved scene path".to_owned()).and_then(|path|
                crate::project::validate_tilemap_host(&mut self.editor, &path, self.sprites.bindings.as_ref().map(|bindings| bindings.document())));
            if let Err(error) = result { self.editor.error(error); return; }
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
        if self.collect_project_dialog_open() { return; }
        if self.package_removal_pending() { self.editor.error("Wait for package selection change completion before saving"); return; }
        if self.hazard_pick_busy() { self.editor.error("Finish or cancel hazard center picking before saving"); return; }
        if self.hazard_export_busy() { self.editor.error("Wait for hazard export completion before saving"); return; }
        #[cfg(feature="mesh-mission")]
        if self.editor.game().has_mesh_view() {
            self.mesh_mission.sync(&self.editor);
            if self.mesh_mission_dirty() {
                self.editor.error("Apply or reload mission fields before saving the scene");
                return;
            }
        }
        #[cfg(feature="mesh-hazards")]
        if self.editor.game().is_mesh_hazards() {
            self.mesh_hazards.sync(&self.editor);
            if self.mesh_hazards.dirty() {
                self.editor.error("Apply or reload hazard fields before saving the scene");
                return;
            }
        }
        if self.editor.sim().scene_path.is_some() {
            self.editor.save();
        } else {
            self.ui.dialog = Some(Dialog { kind: DialogKind::SaveAs, text: String::new() });
        }
    }

    // ---- top bar ----

    fn top_bar(&mut self, ui: &mut Ui) {
        let mutable = self.editor.can_mutate() && self.editor.previewing().is_none();
        #[cfg(feature="mesh-mission")]
        let mutable = mutable && !self.mesh_game_creator.busy();
        #[cfg(feature="mesh-navigation")]
        let mutable = mutable && !self.mesh_navigation.art_import_busy() && !self.mesh_navigation.prop_gesture_busy() && !self.mesh_navigation.agent_pick_busy() && !self.mesh_navigation.geometry_vertex_busy() && !self.mesh_mission_pick_busy() && !self.hazard_pick_busy();
        let live = self.editor.down().is_none();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if !(mutable) { ui.disable(); }
                #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
                { self.collect_project_menu(ui); if self.collect_project_dialog_open() { ui.disable(); } }
                #[cfg(feature="mesh-mission")]
                if matches!(self.editor.game(), crate::game::EditorGame::PhysGame | crate::game::EditorGame::MeshNavigation)
                    && ui.add_enabled(self.mesh_workspace_blocker().is_none(), egui::Button::new("New mesh game\u{2026}")).clicked() {
                    self.mesh_game_creator.open();
                    self.editor.release_control();
                    ui.close();
                }
                #[cfg(feature="mesh-hazards")]
                if (self.editor.game() == crate::game::EditorGame::PhysGame || self.editor.game().has_mesh_view())
                    && ui.add_enabled(self.mesh_creation_base_blocker().is_none(), egui::Button::new("New hazard game\u{2026}")).clicked() {
                    self.mesh_game_creator.open_hazards();
                    self.editor.release_control();
                    ui.close();
                }
                #[cfg(feature="mesh-navigation")]
                if matches!(self.editor.game(), crate::game::EditorGame::PhysGame | crate::game::EditorGame::MeshNavigation)
                    && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("New mesh scene\u{2026}")).clicked() {
                    self.ui.dialog = Some(Dialog { kind: DialogKind::NewMesh, text: String::new() });
                    ui.close();
                }
                #[cfg(feature="mesh-navigation")]
                if matches!(self.editor.game(), crate::game::EditorGame::PhysGame | crate::game::EditorGame::MeshNavigation)
                    && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("Open mesh scene\u{2026}")).clicked() {
                    self.ui.dialog = Some(Dialog { kind: DialogKind::OpenMesh, text: String::new() });
                    ui.close();
                }
                if ui.add_enabled(!self.package_work_pending() && !self.hazard_export_busy(), egui::Button::new("Open scene\u{2026}")).clicked() {
                    let text = self.editor.path().map(|p| p.display().to_string()).unwrap_or_default();
                    self.ui.dialog = Some(Dialog { kind: DialogKind::Open, text });
                    ui.close();
                }
                if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                    self.save();
                    ui.close();
                }
                #[cfg(feature="mesh-navigation")]
                if self.editor.game().is_mesh_navigation() {
                    let blocker = self.mesh_workspace_save_blocker();
                    if ui.add_enabled(blocker.is_none(), egui::Button::new("Save mesh workspace"))
                        .on_disabled_hover_text(blocker.as_deref().unwrap_or("Save the scene and all applied mesh presentation changes; unapplied forms are not applied"))
                        .clicked() {
                        self.save_mesh_workspace();
                        ui.close();
                    }
                }
                #[cfg(feature="mesh-hazards")]
                if self.editor.game().is_mesh_hazards() {
                    ui.weak("MeshHazardsV1 uses scene Save; legacy workspace copy/save and legacy export are not supported.");
                }
                if ui.add_enabled(!self.package_work_pending() && !self.hazard_export_busy(), egui::Button::new("Save As\u{2026}")).clicked() {
                    let text = self.editor.path().map(|p| p.display().to_string()).unwrap_or_default();
                    self.ui.dialog = Some(Dialog { kind: DialogKind::SaveAs, text });
                    ui.close();
                }
                #[cfg(feature="room-authoring")]
                if self.editor.game().is_room() && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("Room level authoring\u{2026}")).clicked() {
                    self.room_authoring.open = true;
                    ui.close();
                }
                #[cfg(all(any(windows,target_os="linux"),target_arch="x86_64",feature="room-authoring"))]
                if self.editor.game().is_room() && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("Room wall groups\u{2026}")).clicked() {
                    self.room_wall_stamps.open = true;
                    ui.close();
                }
                #[cfg(feature="package-manager")]
                if self.editor.game().is_room() && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("Content packages\u{2026}")).clicked() {
                    self.package_manager.open = true;
                    ui.close();
                }
                #[cfg(feature="mesh-hazards-export")]
                if self.editor.game().is_mesh_hazards() && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("Export hazard game\u{2026}")).clicked() {
                    self.mesh_hazards_export.open = true;
                    ui.close();
                }
                #[cfg(feature="mesh-export")]
                if self.editor.game().is_mesh_navigation() && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new("Export mesh game\u{2026}")).clicked() {
                    self.mesh_export.open = true;
                    ui.close();
                }
                #[cfg(all(windows,target_arch="x86_64",feature="mesh-navigation"))]
                if self.editor.game().is_mesh_navigation() && ui.add_enabled(!self.mesh_copy.busy() && self.mesh_copy_blocker().is_none(), egui::Button::new("Duplicate mesh workspace\u{2026}")).clicked() {
                    self.mesh_copy.open();
                    ui.close();
                }
                #[cfg(all(windows, target_arch="x86_64", feature="windows-export"))]
                if self.editor.game().is_room() && ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local() && !self.package_work_pending(), egui::Button::new("Export Windows game\u{2026}")).clicked() {
                    self.windows_export.use_current_scene(self.editor.path().as_deref());
                    self.windows_export.open = true;
                    ui.close();
                }
                #[cfg(all(windows, target_arch="x86_64", feature="windows-projects"))]
                if self.editor.game().is_room() || self.editor.game() == crate::game::EditorGame::PhysGame {
                    for label in ["New Room project\u{2026}", "Open Room project\u{2026}"] {
                        if ui.add_enabled(self.editor.mode() == Mode::Edit && self.editor.spec().is_local(), egui::Button::new(label)).clicked() {
                            self.windows_projects.open = true;
                            ui.close();
                        }
                    }
                }
            });
            #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
            if self.collect_project_dialog_open() { ui.disable(); }
            ui.menu_button("Edit", |ui| {
                let editing = mutable && self.editor.mode() == Mode::Edit;
                if ui.add_enabled(editing && self.editor.history().can_undo, egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    self.editor.undo();
                    ui.close();
                }
                if ui.add_enabled(editing && self.editor.history().can_redo, egui::Button::new("Redo").shortcut_text("Ctrl+Y")).clicked() {
                    self.editor.redo();
                    ui.close();
                }
            });
            ui.separator();
            let playing = self.editor.timeline().is_some_and(|t| t.playing);
            let start_blocker = self.playback_start_blocker();
            if ui.add_enabled(live && !playing && start_blocker.is_none(), egui::Button::new(LBL_PLAY)).on_disabled_hover_text(start_blocker.as_deref().unwrap_or("Playback is already running or the host is unavailable")).clicked() {
                self.play_from_ui();
            }
            if ui.add_enabled(live && playing, egui::Button::new(LBL_PAUSE)).clicked() {
                self.editor.pause();
            }
            if ui.add_enabled(live && !playing && start_blocker.is_none(), egui::Button::new(LBL_STEP)).on_disabled_hover_text(start_blocker.as_deref().unwrap_or("Pause playback or reconnect the host before stepping")).clicked() {
                self.step_from_ui();
            }
            if ui.add_enabled(live && self.editor.is_playing_mode(), egui::Button::new(LBL_STOP)).clicked() {
                self.editor.stop();
            }
            ui.separator();
            match self.editor.mode() {
                Mode::Edit => ui.label(RichText::new("EDIT").strong()),
                Mode::Play => ui.label(RichText::new("PLAY").strong().color(Color32::from_rgb(255, 150, 60))),
            };
            ui.weak(self.editor.game().name());
            if self.editor.game().is_3d() {
                #[cfg(feature = "models")]
                ui.menu_button("Viewport effects", |ui| {
                    ui.checkbox(&mut self.ui.yard_post_process.enabled, "HDR post-processing");
                    ui.add_enabled_ui(self.ui.yard_post_process.enabled, |ui| {
                        ui.checkbox(&mut self.ui.yard_post_process.bloom, "Bloom");
                        ui.add(egui::Slider::new(&mut self.ui.yard_post_process.threshold, 0.0..=8.0).text("Bloom threshold"));
                        ui.add(egui::Slider::new(&mut self.ui.yard_post_process.strength, 0.0..=8.0).text("Bloom strength"));
                        ui.add(egui::Slider::new(&mut self.ui.yard_post_process.radius, 1..=8).text("Bloom radius"));
                        ui.add(egui::Slider::new(&mut self.ui.yard_post_process.iterations, 1..=4).text("Bloom iterations"));
                    });
                    ui.weak("View only · scene Lighting controls exposure and tone mapping");
                    if let Some(error) = &self.ui.yard_post_process_error {
                        ui.colored_label(Color32::RED, format!("Last rejected request: {error}"));
                    }
                });
                #[cfg(not(feature = "models"))]
                ui.add_enabled(false, egui::Button::new("Viewport effects"))
                    .on_disabled_hover_text("HDR/bloom requires an editor built with the models or animated-models feature");
            }
            if self.editor.is_viewer() { ui.label("REPLAY VIEWER · read-only"); }
            if self.editor.is_dirty() {
                ui.label(RichText::new("\u{25CF} unsaved").color(Color32::from_rgb(240, 200, 80)));
            }
        });
        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "project-create", feature = "collect-ui"))]
        if self.collect_project_dialog_open() { return; }
        #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
        self.ui.collect_controls.sync_session(self.editor.scene_load_epoch(), self.editor.game().is_collect());
        if self.editor.game().has_keyboard() {
            ui.horizontal_wrapped(|ui| {
                use crate::editor::input::Phase;
                let phase = self.editor.input_phase();
                #[cfg(feature = "collect-dodge")]
                if self.editor.game().is_collect() {
                    if let Some(snapshot) = self.editor.snapshot() { let run = snapshot.predicted().singleton::<orr_sample::collect_game::CollectRun>(); ui.label(format!("{} · Score {}/{}", orr_sample::collect_view::status(run), run.score, run.goal)); }
                }
                ui.label("Keyboard player slot");
                ui.add_enabled(phase == Phase::Off, egui::DragValue::new(&mut self.ui.input_player).range(0..=self.editor.sim().player_count.saturating_sub(1)));
                if self.editor.game() == crate::game::EditorGame::Arena && self.editor.mode() == Mode::Edit && ui.add_enabled(
                        self.editor.can_mutate() && self.editor.previewing().is_none() && phase == Phase::Off,
                        egui::Button::new("+ Player"),
                ).on_hover_text("Create a player in this unused slot at the viewport center.").clicked() {
                    let at = self.editor.camera.center;
                    self.editor.spawn_arena_player(self.ui.input_player, at);
                }
                if ui.add_enabled(phase == Phase::Off && self.editor.can_take_control(), egui::Button::new("Take control")).on_hover_text("Replaces this slot's current held input. Does not cancel commands or take over the simulation.").clicked() {
                    self.ui.take_control = true;
                }
                if ui.add_enabled(phase != Phase::Off, egui::Button::new("Release control")).clicked() { self.editor.release_control(); }
                if self.editor.input_cleanup_pending() && ui.button("Reconnect control").clicked() { self.restart_from_ui(); }
                #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
                let mapped_collect_hint = self.editor.game().is_collect().then(|| format!("Active · {}", self.ui.collect_controls.hint()));
                #[cfg(not(all(feature = "collect-ui", feature = "collect-input-map-editor")))]
                let mapped_collect_hint: Option<String> = None;
                ui.label(match phase { Phase::Off => self.editor.input_hint(), Phase::Claiming => "Claiming…", Phase::Active => if let Some(hint) = self.editor.game().registered_input_hint() { hint } else if self.editor.game().is_room() { "Active · WASD/arrows · E interact · Escape releases" } else if self.editor.game().is_collect() { mapped_collect_hint.as_deref().unwrap_or("Active · WASD/arrows · press Space to restart · Escape releases") } else { "Active · WASD/arrows · hold Space to fire · Escape releases" }, Phase::Releasing => "Releasing…" });
            });
            #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
            if self.editor.game().is_collect() {
                ui.horizontal_wrapped(|ui| {
                    let controls_label = ui.label("Collect controls");
                    let editable = self.editor.mode() == Mode::Edit
                        && self.editor.input_phase() == crate::editor::input::Phase::Off;
                    ui.add_enabled_ui(editable, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.ui.collect_controls.path)
                            .hint_text("Input-map JSON path").desired_width(240.0)).labelled_by(controls_label.id);
                        if ui.button("Load controls").clicked() { self.ui.collect_controls.load(); }
                        if ui.button("Default controls").clicked() { self.ui.collect_controls.use_defaults(); }
                    });
                    if !editable { ui.weak("Edit mode and released control required to change bindings"); }
                    if let Some(error) = &self.ui.collect_controls.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
            }
        }
    }

    /// The banner of a host that is gone: why, and the way back.
    fn down_banner(&mut self, ui: &mut Ui) {
        let Some(down) = self.editor.down().cloned() else { return };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, RichText::new(&down.reason).strong());
            let label = if down.local { LBL_RESTART } else { LBL_RECONNECT };
            if ui.button(label).clicked() {
                self.restart_from_ui();
            }
            ui.weak(if down.local {
                "Restart opens the scene file again in a new simulation host: edits that were not saved are lost."
            } else {
                "Reconnect attaches to the host again and shows its scene (edits the host kept are still there)."
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            self.close_review_status(ui);
            let status = match self.editor.status() {
                Some(Message { text, error: true }) => RichText::new(text).color(ui.visuals().error_fg_color),
                Some(Message { text, error: false }) => RichText::new(text),
                None => RichText::new("ready").weak(),
            };
            ui.add_sized([ui.available_width() * 0.4, ui.text_style_height(&egui::TextStyle::Body)], egui::Label::new(status).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let gpu = self.gpu3d.as_ref().map(|g|g.gpu().adapter_name()).or_else(||self.gpu.as_ref().map(|g|g.gpu().adapter_name())).unwrap_or_else(||"no GPU".to_string());
                ui.weak(gpu);
                ui.weak(format!("checksum {:#018x}", self.editor.checksum()));
                if let Some((url, clients)) = self.editor.erp_status() {
                    ui.weak(format!("ERP {url} ({clients} connected)"));
                }
                if let Some(action) = self.editor.feed().last_action() {
                    let short: String = if action.chars().count() > 70 { action.chars().take(69).chain(std::iter::once('\u{2026}')).collect() } else { action.to_string() };
                    ui.label(RichText::new(short).color(Color32::from_rgb(120, 200, 255)));
                }
            });
        });
    }

    // ---- hierarchy ----

    fn hierarchy(&mut self, ui: &mut Ui) {
        ui.heading("Hierarchy");
        ui.horizontal(|ui| {
            if !(self.editor.can_mutate() && self.editor.previewing().is_none()) { ui.disable(); }
            if self.editor.game() == crate::game::EditorGame::PhysGame && ui.button("+ Body").on_hover_text("Spawn a dynamic circle at the view center").clicked() {
                let c = self.editor.camera.center;
                self.editor.spawn_body(c);
            }
            if self.editor.game().is_3d() && !self.editor.game().is_room() && !self.editor.game().is_navigation() && !self.editor.game().has_mesh_view() && ui.add_enabled(self.editor.mode()==Mode::Edit, egui::Button::new(if self.editor.game().is_terrain() { "+ Terrain Sphere" } else { "+ 3D Box" })).clicked() {
                self.editor.spawn_yard_body(orr_fp::FPVec3::new(FP::ZERO,FP::from_int(5),FP::ZERO));
            }
            if ui.add_enabled(!self.editor.game().is_room() && self.editor.selection().is_some() && self.editor.selected_guids().len() <= 1, egui::Button::new("Delete")).on_disabled_hover_text("Room walls use the validated removal action in Room level authoring.").clicked() {
                self.editor.delete_selected();
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.ui.filter).hint_text("filter").desired_width(f32::INFINITY));
        ui.separator();
        let filter = self.ui.filter.to_lowercase();
        let rows: Vec<(String, Target, String)> = self
            .editor
            .rows()
            .iter()
            .filter_map(|row| {
                let label = row.label();
                if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                    return None;
                }
                Some((label, row.target(), row.components.join(", ")))
            })
            .collect();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        let mut clicked: Option<Target> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
            for (label, target, tip) in &rows[range] {
                let selected = self.editor.is_selected(target);
                if ui.selectable_label(selected, label).on_hover_text(tip).clicked() {
                    clicked = Some(target.clone());
                }
            }
        });
        if let Some(t) = clicked {
            if ui.input(|i| i.modifiers.ctrl || i.modifiers.command) { self.editor.toggle_selection(t); }
            else { self.editor.select(Some(t)); }
        }
    }

    // ---- inspector ----

    fn inspector(&mut self, ui: &mut Ui) {
        ui.heading("Inspector");
        #[cfg(feature="mesh-navigation")]
        if self.editor.game().has_mesh_view() {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            egui::ScrollArea::vertical().id_salt("mesh_navigation_inspector").auto_shrink([false, false]).show(ui, |ui| {
                self.mesh_navigation.show(ui, &mut self.editor);
                self.mesh_play_controls.show(ui, &mut self.editor);
                #[cfg(feature="mesh-hazards")]
                ui.add_enabled_ui(!self.mesh_navigation.has_unplaced_draft(), |ui| self.mesh_hazards.show(ui, &mut self.editor));
                self.mesh_camera.show(ui, &mut self.editor);
                ui.add_enabled_ui(!self.mesh_navigation.has_unplaced_draft(), |ui| self.mesh_lighting.show(ui, &self.editor));
                ui.add_enabled_ui(!self.mesh_navigation.has_unplaced_draft(), |ui| self.mesh_hud.show(ui, &self.editor));
                #[cfg(feature="mesh-mission")]
                ui.add_enabled_ui(!self.mesh_navigation.has_unplaced_draft(), |ui| self.mesh_mission.show(ui, &mut self.editor));
                #[cfg(feature="mesh-animated-models")]
                self.mesh_animation.show(ui, &self.editor);
                #[cfg(not(feature="mesh-animated-models"))]
                if let Some(blocker) = self.mesh_animation_blocker() { ui.colored_label(Color32::RED, blocker); }
            });
            return;
        }
        #[cfg(feature = "linked-prefabs")]
        self.linked_prefabs.show(ui, &mut self.editor);
        if self.editor.game().is_3d() { self.yard_transform(ui); }
        #[cfg(feature="terrain")]
        {
            self.terrain.sync_for_editor(&self.editor);
            let terrain_slots = usize::from(self.terrain.attached_for_editor(&self.editor));
            #[cfg(feature="navigation")]
            let terrain_slots = terrain_slots + usize::from(self.editor.game().is_navigation() && self.editor.admitted_navigation().admitted);
            self.models.reserve_scene_models(terrain_slots);
        }
        #[cfg(feature="models")]
        self.models.show(ui,&self.editor);
        #[cfg(feature="models")]
        self.blockout_uv_density.show(ui, &self.editor, &mut self.models);
        #[cfg(feature="terrain")]
        {
            let error = if self.models.scene_matches(&self.editor) { self.models.terrain_admission_error() } else { None };
            self.terrain.document.set_render_admission_error(error);
            if self.editor.game() == crate::game::EditorGame::Yard3D || self.editor.game().is_navigation() {
                self.terrain.show_in_parent_scroll(ui, &self.editor);
            } else {
                self.terrain.show(ui, &self.editor);
            }
            self.terrain_ramp_show(ui);
        }
        #[cfg(feature="navigation")]
        self.navigation.show(ui, &mut self.editor, &self.terrain);
        #[cfg(all(feature="terrain", feature="irradiance-probes"))]
        self.irradiance.set_terrain_attached(self.terrain.attached_for_editor(&self.editor));
        #[cfg(feature="irradiance-probes")]
        self.irradiance.show_with_models(ui,&self.editor,&self.models);
        #[cfg(feature = "sprites")]
        self.sprites.show(ui, &self.editor);
        #[cfg(feature = "tilemap")]
        if let Some(panel) = &mut self.tilemap { panel.show(ui, &self.editor, &self.sprites); }
        #[cfg(feature = "project-ui")]
        if let Some(preview) = &self.project_ui { preview.show(ui); }
        #[cfg(feature="collect-ui")]
        if let Some(panel) = &mut self.collect_ui { panel.show(ui, !self.editor.is_playing_mode()); }
        #[cfg(feature="collect-ui")]
        if self.collect_ui.is_some() { self.collect_ui_preview_button(ui); }
        #[cfg(feature = "animated-models")]
        if ui.add_enabled(self.editor.game() == crate::game::EditorGame::Arena && self.editor.spec().is_local(), egui::Button::new("Animated model authoring")).on_hover_text("Presentation bindings require a local Arena scene").clicked() { self.ui.animated_window = true; }
        #[cfg(feature="room-project")]
        {
            #[cfg(feature = "room-lighting")]
            {
                if self.editor.room_lighting_document().is_none() { self.room_lighting = None; }
                if let Some(panel) = &mut self.room_lighting { panel.show_for_editor(ui, &mut self.editor); }
            }
            #[cfg(feature="room-character")]
            {
                if self.editor.room_character().is_none() { self.room_character = None; }
                if let Some(panel) = &mut self.room_character { panel.show(ui, &mut self.editor, &mut self.models); }
            }
            if !self.editor.has_room_camera() { self.room_camera = None; }
            if !self.room_camera_capture.owns_history() {
            if let Some(panel) = &mut self.room_camera {
                let editable = self.editor.mode()==Mode::Edit && self.editor.can_mutate()
                    && self.editor.previewing().is_none() && self.editor.yard_rows_coherent();
                if panel.show(ui, editable) { self.room_camera_capture.invalidate(); let _ = self.editor.install_room_camera(panel.document().clone()); }
            }
            }
            self.show_room_camera_capture(ui);
        }
        let count = self.editor.selected_guids().len();
        if count>0&&self.editor.game().is_3d() {ui.label("Single 3D selection · Esc to clear");}
        if count > 0 && !self.editor.game().is_3d() {
            ui.label(format!("{count} selected · Ctrl-click to toggle · Esc to clear"));
            let enabled = self.editor.can_nudge_selection();
            ui.horizontal_wrapped(|ui| {
                for (label, x, y) in [("Move left", -1, 0), ("Move right", 1, 0), ("Move up", 0, 1), ("Move down", 0, -1)] {
                    if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text("Move every selected document GUID by one world unit in a single undo entry").clicked() {
                        self.editor.nudge_selected(FPVec2::new(FP::from_int(x), FP::from_int(y)));
                    }
                }
            });
            if self.editor.batch_outcome_uncertain() {
                ui.colored_label(Color32::YELLOW, "Move result unconfirmed. Reconnect to read document and history.");
            }
        }
        if self.editor.previewing().is_some() {
            ui.weak("Live document inspector · read-only during proposal preview");
        } else if self.editor.is_viewer() {
            ui.weak("Replay Viewer · read-only");
        }
        let mut events: Vec<(Owner, InspEvent)> = Vec::new();
        let mut add: Option<String> = None;
        let mut remove: Option<String> = None;
        let selection = self.editor.selection().cloned();
        let inspect = self.editor.inspect().cloned();
        let editor = &self.editor;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !(editor.can_mutate() && editor.previewing().is_none()) { ui.disable(); }
            let types = editor.types();
            match (&selection, &inspect) {
                (Some(_), Some(ins)) => {
                    let row = ins.row.as_ref();
                    let key = ins.target.param();
                    ui.label(RichText::new(row.map_or_else(|| key.clone(), EntityRow::label)).strong());
                    ui.weak(&key);
                    ui.separator();
                    for (name, value) in &ins.components {
                        let Some(ty) = types.get(name) else { continue };
                        let base = egui::Id::new(("inspector", &key, name));
                        egui::CollapsingHeader::new(RichText::new(name).strong()).id_salt(base).default_open(true).show(ui, |ui| {
                            let mut evs = Vec::new();
                            show_component(ui, base, ty.desc(), value, &mut evs);
                            events.extend(evs.into_iter().map(|e| (Owner::Component(name.clone()), e)));
                            if ui.small_button("Remove component").clicked() {
                                remove = Some(name.clone());
                            }
                        });
                    }
                    let have: Vec<&str> = row.map(|r| r.components.iter().map(String::as_str).collect()).unwrap_or_default();
                    let addable: Vec<&str> = types.components().map(|t| t.name()).filter(|n| !have.contains(n) && !ins.components.iter().any(|(c, _)| c == n)).collect();
                    ui.add_enabled_ui(!addable.is_empty(), |ui| {
                        egui::ComboBox::from_id_salt("add_component").selected_text("+ Add component").show_ui(ui, |ui| {
                            for n in addable {
                                if ui.selectable_label(false, n).clicked() {
                                    add = Some(n.to_string());
                                }
                            }
                        });
                    });
                }
                (Some(t), None) => {
                    if editor.row_of(t).is_some() || editor.rows().is_empty() {
                        ui.weak("loading\u{2026}");
                    } else {
                        ui.weak("selection no longer exists");
                    }
                }
                (None, _) => {
                    ui.weak("Select an entity in the hierarchy or click an entity in the viewport.");
                }
            }
            ui.separator();
            ui.label(RichText::new("Singletons").strong());
            for (name, value) in editor.singletons() {
                let Some(ty) = types.get(name) else { continue };
                let base = egui::Id::new(("singleton", name));
                egui::CollapsingHeader::new(name).id_salt(base).default_open(editor.game() == crate::game::EditorGame::Arena && name == "Score").show(ui, |ui| {
                    let mut evs = Vec::new();
                    show_component(ui, base, ty.desc(), value, &mut evs);
                    events.extend(evs.into_iter().map(|e| (Owner::Singleton(name.clone()), e)));
                });
            }
        });
        for (owner, ev) in events {
            match ev {
                InspEvent::Begin(label) => self.editor.begin_edit(&label),
                InspEvent::Set { path, value } => {
                    self.editor.set_field(&owner, &path, value);
                }
                InspEvent::End => self.editor.end_edit(),
                InspEvent::Error(m) => self.editor.error(m),
            }
        }
        if let Some(n) = add {
            self.editor.add_component(&n);
        }
        if let Some(n) = remove {
            self.editor.remove_component(&n);
        }
    }

    fn yard_transform(&mut self, ui:&mut Ui) {
        let target=self.editor.selection().cloned();
        let reset=self.ui.yard_transform_target!=target;
        if reset || ui.button("Read 3D transform").clicked() {
            if let Some(body)=self.editor.inspect().and_then(|inspect|inspect.components.iter().find(|(name,_)|name=="orr_physics3d::Body").map(|(_,value)|value)) {
                if let Some(Value::Vec3(pos))=body.field("pos") { self.ui.yard_xyz=[orr_reflect::decimal::fp_to_decimal(pos.x),orr_reflect::decimal::fp_to_decimal(pos.y),orr_reflect::decimal::fp_to_decimal(pos.z)]; }
                if let Some(rot)=body.field("rot") { for (i,key) in ["x","y","z","w"].into_iter().enumerate(){ if let Some(Value::Fixed(v))=rot.field(key){self.ui.yard_quat[i]=orr_reflect::decimal::fp_to_decimal(*v);} } }
                self.ui.yard_transform_target=target;
            }
        }
        ui.collapsing("Exact 3D transform",|ui|{
            ui.add_enabled_ui(self.editor.mode()==Mode::Edit && self.editor.can_mutate() && self.editor.selected_guids().len()==1,|ui|{
                for (i,axis) in ["X","Y","Z"].into_iter().enumerate(){ui.horizontal(|ui|{let label=ui.label(format!("Position {axis}"));ui.add(egui::TextEdit::singleline(&mut self.ui.yard_xyz[i]).id(egui::Id::new(("yard_xyz",i))).hint_text(format!("Position {axis}"))).labelled_by(label.id);});}
                ui.label("Unit quaternion (x, y, z, w)");
                for (i,axis) in ["x","y","z","w"].into_iter().enumerate(){ui.horizontal(|ui|{let label=ui.label(format!("Rotation {axis}"));ui.add(egui::TextEdit::singleline(&mut self.ui.yard_quat[i]).id(egui::Id::new(("yard_quat",i))).hint_text(format!("Rotation {axis}"))).labelled_by(label.id);});}
                if ui.button("Apply XYZ + rotation").clicked(){
                    let pos:Result<Vec<FP>,_>=self.ui.yard_xyz.iter().map(|v|v.parse()).collect();
                    let rot:Result<Vec<FP>,_>=self.ui.yard_quat.iter().map(|v|v.parse()).collect();
                    match (pos,rot){(Ok(p),Ok(q))=>{self.editor.set_yard_transform(orr_fp::FPVec3::new(p[0],p[1],p[2]),[q[0],q[1],q[2],q[3]]);},_=>self.editor.error("3D transform requires exact finite decimal values")}
                }
            });
        });
    }

    fn yard_viewport(&mut self,ui:&mut Ui){
        #[cfg(feature="models")]
        self.frame_selection_button(ui);
        #[cfg(feature="terrain")]
        self.terrain.sync_for_editor(&self.editor);
        #[cfg(all(feature="terrain", feature="irradiance-probes"))]
        self.irradiance.set_terrain_attached(self.terrain.attached_for_editor(&self.editor));
        #[cfg(feature="irradiance-probes")]
        {
            self.irradiance.sync_for_editor(&self.editor);
            self.irradiance.sync_bake_for_editor(&self.editor,&self.models);
            if self.irradiance.is_baking() || self.irradiance.is_validating_bake() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        #[cfg(feature="room-project")]
        if self.editor.game().is_room() {
            let run=self.editor.snapshot().map(|snapshot|*snapshot.predicted().singleton::<orr_sample::room_game::RoomRun>());
            ui.horizontal(|ui| {
                if let Some(run)=run {
                    ui.label(if run.won!=0 { "Escaped!" } else if run.key_collected!=0 { "Key collected · Find exit and press E" } else { "Exit locked · Find key and press E" });
                }
                if ui.add_enabled(self.editor.mode()==Mode::Play,egui::Button::new("Restart room")).clicked() {
                    self.editor.stop();
                    if self.editor.mode()==Mode::Edit { self.editor.reset_room_camera(); self.editor.play(); }
                    self.ui.take_control=false;
                }
            });
        }
        let (rect,resp)=if self.editor.game().is_room() {
            let (rect,_)=ui.allocate_exact_size(ui.available_size(),Sense::hover());
            (rect,ui.interact(rect,egui::Id::new("room_keyboard_viewport"),Sense::click_and_drag()))
        } else if self.editor.game().has_mesh_view() {
            let (rect,_)=ui.allocate_exact_size(ui.available_size(),Sense::hover());
            let response=ui.interact(rect,egui::Id::new("mesh_keyboard_viewport"),Sense::click_and_drag());
            response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Other,true,"Mesh gameplay viewport"));
            (rect,response)
        } else { ui.allocate_exact_size(ui.available_size(),Sense::click_and_drag()) };
        if self.editor.game().is_room() {
            if self.ui.take_control {
                self.ui.take_control=false;
                if self.ui.dialog.is_none() && ui.input(|i|i.focused) { resp.request_focus(); self.editor.take_control(0,true); }
            }
            let focused=resp.has_focus() && self.ui.dialog.is_none() && ui.input(|i|i.focused && !i.key_pressed(Key::Escape));
            let keys=ui.input(|i|crate::editor::input::Keys {
                x:i8::from(i.key_down(Key::D)||i.key_down(Key::ArrowRight))-i8::from(i.key_down(Key::A)||i.key_down(Key::ArrowLeft)),
                y:i8::from(i.key_down(Key::W)||i.key_down(Key::ArrowUp))-i8::from(i.key_down(Key::S)||i.key_down(Key::ArrowDown)),
                fire:i.key_down(Key::E),
            });
            self.editor.arena_keys(focused,keys);
            if resp.has_focus() {
                let controlled=focused && self.editor.input_phase()==crate::editor::input::Phase::Active;
                ui.memory_mut(|memory|memory.set_focus_lock_filter(resp.id,egui::EventFilter { horizontal_arrows:controlled,vertical_arrows:controlled,..Default::default() }));
            }
        }
        let ppp=ui.ctx().pixels_per_point();
        let px=(((rect.width()*ppp).round() as u32).clamp(1,8192),((rect.height()*ppp).round() as u32).clamp(1,8192));
        self.ui.viewport_rect=Some(rect);self.ui.viewport_px=px;
        #[cfg(feature="models")]
        self.frame_selection_shortcut(ui, &resp, px);
        #[cfg(feature="mesh-navigation")]
        let prop_moving = self.mesh_navigation.prop_gesture_busy() || self.mesh_navigation.geometry_vertex_dragging();
        #[cfg(not(feature="mesh-navigation"))]
        let prop_moving = false;
        if !prop_moving {
            if resp.dragged_by(PointerButton::Secondary){let delta=resp.drag_delta();self.editor.orbit3d([delta.x*ppp,delta.y*ppp]);}
            if resp.dragged_by(PointerButton::Middle){let delta=resp.drag_delta();self.editor.pan3d([delta.x*ppp,delta.y*ppp],px);}
            if resp.hovered(){let scroll=ui.input(|input|input.smooth_scroll_delta.y);if scroll!=0.0 {self.editor.zoom3d(scroll*0.01);}}
        }
        let camera = match self.editor.presentation_camera3d(px) {
            Ok(camera) => camera,
            Err(error) => {
                #[cfg(feature="room-authoring")]
                { self.room_authoring.cancel_wall_footprint(); self.room_authoring.cancel_actor_placement(); self.room_authoring.cancel_wall_placement(); }
                if let Some(tex)=self.gpu3d.as_ref().and_then(|gpu|gpu.last_texture()) { ui.painter().image(tex,rect,Rect::from_min_max(Pos2::ZERO,Pos2::new(1.0,1.0)),Color32::WHITE); }
                ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,error,egui::FontId::proportional(14.0),Color32::RED); return;
            }
        };
        let mut frame_drawn = false;
        #[cfg(feature="room-authoring")]
        let wall_pointer_consumed = self.room_wall_pointer_owned || self.room_authoring.wall_footprint_busy() || self.room_actor_pointer_owned || self.room_authoring.actor_placement_busy() || self.room_wall_placement_pointer_owned || self.room_authoring.wall_placement_busy();
        #[cfg(not(feature="room-authoring"))]
        let wall_pointer_consumed = false;
        #[cfg(feature="mesh-hazards")]
        let hazard_pointer_consumed = {
            let owned = self.mesh_hazard_pointer_owned || self.mesh_hazards.hazard_pick_busy();
            if owned {
                #[cfg(feature="mesh-mission")]
                self.mesh_mission.cancel_mission_pick();
                self.mesh_navigation.cancel_agent_pick();
                self.mesh_navigation.cancel_prop_gesture();
                if self.mesh_navigation.geometry_vertex_busy() { self.mesh_navigation.cancel_geometry_vertex(); }
            }
            let to_pixel = |position: Pos2| [(position.x - rect.min.x) * ppp, (position.y - rect.min.y) * ppp];
            let cancel = ui.input(|input| !input.focused || input.key_pressed(Key::Escape))
                || ui.ctx().text_edit_focused() || self.ui.dialog.is_some()
                || self.editor.previewing().is_some() || self.hazard_export_busy()
                || self.mesh_navigation.art_import_busy() || self.mesh_navigation.has_unplaced_draft();
            let consumed = self.mesh_hazards.hazard_viewport_input(&self.editor, &camera, px, crate::mesh_hazards_panel::ViewInput {
                pixel: resp.interact_pointer_pos().or_else(|| resp.hover_pos()).map(to_pixel),
                clicked: resp.clicked_by(PointerButton::Primary),
                cancel,
            });
            owned || consumed
        };
        #[cfg(not(feature="mesh-hazards"))]
        let hazard_pointer_consumed = false;
        let hazard_pointer_consumed = hazard_pointer_consumed || wall_pointer_consumed;

        #[cfg(feature = "mesh-navigation")]
        let mission_pointer_consumed = {
            let active = self.mesh_mission_pick_busy();
            #[cfg(feature="mesh-mission")]
            let consumed = {
                if active {
                    self.mesh_navigation.cancel_agent_pick();
                    self.mesh_navigation.cancel_prop_gesture();
                    if self.mesh_navigation.geometry_vertex_busy() { self.mesh_navigation.cancel_geometry_vertex(); }
                }
                let to_pixel = |position: Pos2| [(position.x - rect.min.x) * ppp, (position.y - rect.min.y) * ppp];
                let content_busy = self.mesh_navigation.art_import_busy();
                #[cfg(feature="mesh-export")]
                let content_busy = content_busy || self.mesh_export.busy();
                self.mesh_mission.mission_viewport_input(&self.editor, &camera, px, crate::mesh_mission_panel::ViewInput {
                    pixel: resp.interact_pointer_pos().or_else(|| resp.hover_pos()).map(to_pixel),
                    clicked: !hazard_pointer_consumed && resp.clicked_by(PointerButton::Primary),
                    cancel: hazard_pointer_consumed || ui.input(|input| !input.focused || input.key_pressed(Key::Escape)) || self.ui.dialog.is_some() || content_busy || self.mesh_navigation.has_unplaced_draft(),
                })
            };
            #[cfg(not(feature="mesh-mission"))]
            let consumed = active;
            consumed
        };
        #[cfg(not(feature="mesh-navigation"))]
        let mission_pointer_consumed = false;
        #[cfg(feature="mesh-navigation")]
        let mesh_play_pointer_consumed = {
            let blocked = hazard_pointer_consumed || self.ui.dialog.is_some() || self.editor.previewing().is_some()
                || ui.input(|input| !input.focused || input.key_pressed(Key::Escape));
            #[cfg(feature = "mesh-export")]
            let blocked = blocked || !self.mesh_player_input_allowed || self.mesh_player_session.dialog_open() || self.mesh_recorded_playback.active();
            if blocked { self.editor.set_mesh_play_input_enabled(false); }
            let click_blocked = blocked || ui.ctx().text_edit_focused();
            let keyboard_enabled = self.editor.mesh_play_input_enabled() && !blocked;
            if self.mesh_play_controls.take_keyboard_focus() && keyboard_enabled {
                resp.request_focus();
            }
            if resp.clicked_by(PointerButton::Primary) && keyboard_enabled && !click_blocked {
                resp.request_focus();
            }
            let keyboard_focused = keyboard_enabled && resp.has_focus() && !ui.ctx().text_edit_focused();
            if resp.has_focus() {
                ui.memory_mut(|memory|memory.set_focus_lock_filter(resp.id,egui::EventFilter {horizontal_arrows:keyboard_focused,vertical_arrows:keyboard_focused,..Default::default()}));
            }
            let direction = ui.input(crate::mesh_play_controls::keyboard_direction);
            let keyboard_blocked = !keyboard_focused || ui.input(|input|input.modifiers != Modifiers::NONE);
            let keyboard_consumed = self.mesh_play_controls.keyboard_input(&mut self.editor,direction,keyboard_blocked);
            if keyboard_consumed {
                ui.input_mut(|input|input.events.retain(|event|!matches!(event,egui::Event::Key {key:Key::W|Key::A|Key::S|Key::D|Key::ArrowUp|Key::ArrowDown|Key::ArrowLeft|Key::ArrowRight,..})));
            }
            let pointer_consumed = resp.clicked_by(PointerButton::Primary) && resp.interact_pointer_pos().is_some_and(|position| {
                let pixel = [(position.x - rect.min.x) * ppp, (position.y - rect.min.y) * ppp];
                self.mesh_play_controls.viewport_click(&mut self.editor, &camera, px, pixel, click_blocked)
            });
            keyboard_consumed || pointer_consumed
        };
        #[cfg(not(feature = "mesh-navigation"))]
        let mesh_play_pointer_consumed = false;
        #[cfg(feature="mesh-navigation")]
        let vertex_pointer_consumed = {
            let to_pixel = |position: Pos2| [(position.x - rect.min.x) * ppp, (position.y - rect.min.y) * ppp];
            let pointer_down = ui.input(|input| input.pointer.primary_down());
            let moving = self.mesh_navigation.geometry_vertex_dragging();
            let content_busy = self.mesh_navigation.art_import_busy();
            #[cfg(feature="mesh-export")]
            let content_busy = content_busy || self.mesh_export.busy();
            self.mesh_navigation.geometry_vertex_input(&self.editor, &camera, px, crate::mesh_navigation::GeometryVertexViewInput {
                pixel: resp.interact_pointer_pos().or_else(|| resp.hover_pos()).map(to_pixel),
                press_pixel: ui.input(|input| input.pointer.press_origin()).map(to_pixel),
                started: !hazard_pointer_consumed && resp.drag_started_by(PointerButton::Primary),
                dragging: !hazard_pointer_consumed && resp.dragged_by(PointerButton::Primary),
                released: !hazard_pointer_consumed && (resp.drag_stopped() || moving) && !pointer_down,
                clicked: !hazard_pointer_consumed && !mission_pointer_consumed && resp.clicked_by(PointerButton::Primary),
                cancel: hazard_pointer_consumed || mission_pointer_consumed || ui.input(|input| !input.focused || input.key_pressed(Key::Escape)) || self.ui.dialog.is_some() || content_busy,
            })
        };
        #[cfg(feature="mesh-navigation")]
        let agent_pointer_consumed = {
            let to_pixel = |position: Pos2| [(position.x - rect.min.x) * ppp, (position.y - rect.min.y) * ppp];
            let content_busy = self.mesh_navigation.art_import_busy();
            #[cfg(feature="mesh-export")]
            let content_busy = content_busy || self.mesh_export.busy();
            self.mesh_navigation.agent_viewport_input(&self.editor, &camera, px, crate::mesh_navigation::AgentViewInput {
                pixel: resp.interact_pointer_pos().or_else(|| resp.hover_pos()).map(to_pixel),
                clicked: !hazard_pointer_consumed && !vertex_pointer_consumed && resp.clicked_by(PointerButton::Primary),
                cancel: hazard_pointer_consumed || mission_pointer_consumed || vertex_pointer_consumed || ui.input(|input| !input.focused || input.key_pressed(Key::Escape)) || self.ui.dialog.is_some() || content_busy,
            })
        };
        #[cfg(not(feature="mesh-navigation"))]
        let agent_pointer_consumed = false;
        #[cfg(feature="mesh-navigation")]
        let prop_pointer_consumed = {
            let to_pixel = |position: Pos2| [(position.x - rect.min.x) * ppp, (position.y - rect.min.y) * ppp];
            let pointer_down = ui.input(|input| input.pointer.primary_down());
            let moving = self.mesh_navigation.prop_gesture_busy();
            let content_busy = self.mesh_navigation.art_import_busy();
            #[cfg(feature="mesh-export")]
            let content_busy = content_busy || self.mesh_export.busy();
            !hazard_pointer_consumed && !mission_pointer_consumed && !vertex_pointer_consumed && !agent_pointer_consumed && self.mesh_navigation.prop_viewport(&self.editor, &camera, px, crate::mesh_props_panel::ViewInput {
                pixel: resp.interact_pointer_pos().or_else(|| resp.hover_pos()).map(to_pixel),
                press_pixel: ui.input(|input| input.pointer.press_origin()).map(to_pixel),
                started: !hazard_pointer_consumed && resp.drag_started_by(PointerButton::Primary),
                dragging: !hazard_pointer_consumed && resp.dragged_by(PointerButton::Primary),
                released: !hazard_pointer_consumed && (resp.drag_stopped() || moving) && !pointer_down,
                clicked: !hazard_pointer_consumed && resp.clicked_by(PointerButton::Primary),
                cancel: hazard_pointer_consumed || ui.input(|input| !input.focused || input.key_pressed(Key::Escape)) || self.ui.dialog.is_some() || content_busy,
            })
        };
        #[cfg(not(feature="mesh-navigation"))]
        let prop_pointer_consumed = false;
        let selected=if self.editor.yard_rows_coherent(){self.editor.selection().and_then(|t|self.editor.row_of(t)).map(|r|r.entity)}else{None};
        #[cfg(feature="models")]
        let placements=self.models.placements(&self.editor);
        #[cfg(feature="animated-models")]
        let animated_placements = match self.models.animated_placements(&self.editor) {
            Ok(placements) => placements,
            Err(error) => {
                if let Some(texture) = self.gpu3d.as_ref().and_then(|gpu| gpu.last_texture()) {
                    ui.painter().image(texture,rect,Rect::from_min_max(Pos2::ZERO,Pos2::new(1.0,1.0)),Color32::WHITE);
                }
                ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,error,egui::FontId::proportional(14.0),Color32::RED);
                return;
            }
        };
        #[cfg(feature="mesh-animated-models")]
        let animated_placements = {
            let mut placements = animated_placements;
            let animation = if self.editor.game().has_mesh_view() && !self.mesh_navigation.geometry_preview_active() { self.mesh_animation.placement(&self.editor) } else { Ok(None) };
            match animation {
                Ok(Some(placement)) => placements.push(placement),
                Ok(None) => {}
                Err(error) => {
                    ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,error,egui::FontId::proportional(14.0),Color32::RED);
                    return;
                }
            }
            placements
        };
        #[cfg(all(feature="models", feature="animated-models"))]
        let hidden:Vec<_>=placements.iter().map(|p|p.entity).chain(animated_placements.iter().map(|p|p.entity)).collect();
        #[cfg(all(feature="models", not(feature="animated-models")))]
        let hidden:Vec<_>=placements.iter().map(|p|p.entity).collect();
        #[cfg(not(feature="models"))]
        let hidden=Vec::new();
        let mut list=self.editor.yard_frame().list(&hidden,None);
        if let Some(item) = self.editor.yard_frame().items.iter().find(|item| Some(item.entity) == selected) {
            selection_outline::append(&mut list, item);
        }
        #[cfg(feature="mesh-navigation")]
        let list = {
            let mut list = list;
            if self.editor.game().has_mesh_view() {
                if let Some(lighting) = self.mesh_lighting.applied_lighting(&self.editor) { list.lighting = lighting; }
            }
            self.mesh_navigation.paint_geometry_vertices(&self.editor, &mut list);
            if !self.mesh_navigation.geometry_preview_active() {
                self.mesh_navigation.paint_agent_endpoints(&self.editor, &mut list);
                if let Some(bounds) = self.mesh_navigation.selected_prop_bounds(&self.editor) {
                    list.aabb(bounds.min, bounds.max, 2.0, [1.0, 0.85, 0.2, 1.0]);
                }
            }
            list
        };
        #[cfg(feature="terrain")]
        let scene_models: Vec<_> = {
            #[cfg(feature="navigation")]
            let model = if self.editor.game().is_navigation() {
                if self.editor.mode() == Mode::Edit {
                    self.terrain.viewport_model(&self.editor).or_else(|| self.editor.admitted_navigation().model())
                } else { self.editor.admitted_navigation().model() }
            } else { self.terrain.viewport_model(&self.editor) };
            #[cfg(not(feature="navigation"))]
            let model = self.terrain.viewport_model(&self.editor);
            #[allow(unused_mut)]
            let mut models: Vec<_> = model.into_iter().map(|model| crate::viewport3d::SceneModel { model, instance: Default::default() }).collect();
            #[cfg(feature="navigation")]
            if self.editor.game().is_navigation() {
                if let Some(model) = self.editor.admitted_navigation().overlay_model(self.navigation.route_stale(&self.editor, &self.terrain)) {
                    models.push(crate::viewport3d::SceneModel { model, instance: Default::default() });
                }
            }
            models
        };
        #[cfg(feature="mesh-navigation")]
        let scene_models: Vec<_> = {
            #[cfg(feature="terrain")]
            let mut models = scene_models;
            #[cfg(not(feature="terrain"))]
            let mut models = Vec::new();
            if self.editor.game().has_mesh_view() {
                if let Some(model) = self.mesh_navigation.model() {
                    models.push(crate::viewport3d::SceneModel { model, instance: Default::default() });
                }
                #[cfg(feature="mesh-animated-models")]
                let show_static_art = !self.mesh_animation.active(&self.editor);
                #[cfg(not(feature="mesh-animated-models"))]
                let show_static_art = true;
                if show_static_art && !self.mesh_navigation.geometry_preview_active() {
                    if let Some(model) = self.mesh_navigation.art_model(&self.editor) {
                        models.push(model);
                    }
                }
                if !self.mesh_navigation.geometry_preview_active() {
                    models.extend(self.mesh_navigation.prop_models(&self.editor));
                }
            }
            models
        };
        if let Some(rs)=&self.render_state {
            let gpu=self.gpu3d.get_or_insert_with(||crate::viewport3d::GpuViewport3d::with_hdr_support(rs,px,!self.viewport_hdr_disabled));
            #[cfg(feature = "models")]
            if let Err(error) = gpu.set_point_light_settings(self.editor.room_point_light_settings()) {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, error, egui::FontId::proportional(14.0), Color32::RED);
                return;
            }
            #[cfg(all(feature="animated-models", not(feature="irradiance-probes"), not(any(feature="terrain", feature="mesh-navigation"))))]
            let rendered = gpu.render_mixed_post_processed(px,&list,&camera,&placements,&animated_placements,self.ui.yard_post_process);
            #[cfg(all(feature="models", not(feature="animated-models"), not(feature="irradiance-probes"), not(any(feature="terrain", feature="mesh-navigation"))))]
            let rendered = gpu.render_post_processed(px,&list,&camera,&placements,self.ui.yard_post_process);
            #[cfg(all(feature="irradiance-probes", not(any(feature="terrain", feature="mesh-navigation"))))]
            let rendered = gpu.render_irradiance(px,&list,&camera,&placements,
                #[cfg(feature="animated-models")] &animated_placements,
                self.ui.yard_post_process,self.irradiance.grid_for_editor(&self.editor));
            #[cfg(any(feature="terrain", feature="mesh-navigation"))]
            let rendered = gpu.render_scene(px, &list, &camera, &placements, &scene_models,
                #[cfg(feature="animated-models")] &animated_placements,
                self.ui.yard_post_process,
                #[cfg(feature="irradiance-probes")] self.irradiance.grid_for_editor(&self.editor));
            #[cfg(not(feature="models"))]
            let rendered = gpu.render(px,&list,&camera);
            #[cfg(feature="models")]
            if let Err(error) = &rendered {
                // Unsupported/invalid requests cannot strand the editor in a broken
                // mode. Keep the admitted policy and working native texture.
                self.ui.yard_post_process = gpu.gpu().post_process_settings();
                self.ui.yard_post_process_error = Some(error.clone());
            }
            match rendered {
                Ok(tex)=>{frame_drawn = true; ui.painter().image(tex,rect,Rect::from_min_max(Pos2::ZERO,Pos2::new(1.0,1.0)),Color32::WHITE);}
                Err(error)=>{if let Some(tex)=gpu.last_texture(){ui.painter().image(tex,rect,Rect::from_min_max(Pos2::ZERO,Pos2::new(1.0,1.0)),Color32::WHITE);}ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,error,egui::FontId::proportional(14.0),Color32::RED);}
            }
        } else {
            ui.painter().rect_filled(rect,0.0,Color32::from_rgb(10,10,16));
            ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,"Yard3D viewport needs a GPU",egui::FontId::proportional(14.0),Color32::GRAY);
        }
        #[cfg(feature="room-authoring")]
        self.room_wall_footprint_input(ui, &resp, &camera, px, rect, ppp, frame_drawn);
        #[cfg(feature="room-authoring")]
        self.room_actor_placement_input(ui, &resp, &camera, px, rect, ppp, frame_drawn);
        #[cfg(feature="room-authoring")]
        self.room_wall_placement_input(ui, &resp, &camera, px, rect, ppp, frame_drawn);
        if !hazard_pointer_consumed && !mission_pointer_consumed && !mesh_play_pointer_consumed && !agent_pointer_consumed && !prop_pointer_consumed && (frame_drawn || !self.editor.has_room_camera()) && resp.clicked_by(PointerButton::Primary) && self.editor.previewing().is_none() {
            if let Some(at) = resp.interact_pointer_pos() {
                let pixel = [(at.x-rect.min.x)*ppp, (at.y-rect.min.y)*ppp];
                #[cfg(feature="terrain")]
                let consumed = self.terrain_ramp_pick(&camera, pixel, px)
                    || self.terrain.pick(&self.editor, &camera, pixel, px);
                #[cfg(not(feature="terrain"))]
                let consumed = false;
                if !consumed { let target = self.editor.pick3d(pixel, px); self.editor.select(target); }
            }
        }
        #[cfg(feature="terrain")]
        if frame_drawn || !self.editor.has_room_camera() {
            self.terrain.paint_selection(&self.editor, ui.painter(), &camera, rect);
            self.terrain_ramp_paint(ui.painter(), &camera, rect);
        }
        #[cfg(feature="mesh-mission")]
        if self.editor.game().is_mesh_navigation() && frame_drawn && !self.mesh_navigation.geometry_preview_active() {
            if let Some(snapshot) = self.editor.snapshot() {
                if let Ok(Some(mission)) = orr_remote::mesh_navigation::mission::from_view(snapshot.predicted()) {
                    crate::mesh_mission_overlay::draw(ui, rect, &mission);
                }
            }
        }
        #[cfg(feature="mesh-hazards")]
        if self.editor.game().is_mesh_hazards() && !self.mesh_navigation.geometry_preview_active() {
            let status = crate::mesh_navigation::hazard_status(&self.editor);
            let text = match status {
                Ok(Some(text)) => text,
                Ok(None) => String::new(),
                Err(error) => format!("Hazard state unavailable: {error}"),
            };
            // Keep the legacy viewport hint on its original first row. Gameplay
            // status gets a separate, clipped panel below it for legible contrast.
            let overlay_rect = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(8.0, 34.0),
                egui::vec2((rect.width() - 16.0).clamp(0.0, 680.0), 62.0),
            ).intersect(rect);
            ui.painter().with_clip_rect(rect).rect_filled(
                overlay_rect, 4.0, Color32::from_rgba_unmultiplied(10, 20, 35, 235),
            );
            let mut overlay = ui.new_child(egui::UiBuilder::new()
                .id_salt("mesh_hazards_overlay")
                .max_rect(overlay_rect.shrink(8.0))
                .layout(egui::Layout::top_down(egui::Align::Min)));
            overlay.set_clip_rect(overlay_rect);
            overlay.add(egui::Label::new(egui::RichText::new(text)
                .strong().size(14.0).color(Color32::from_rgb(255, 180, 90))).wrap());
        }
        let viewport_hint = if self.editor.game().has_mesh_view() { "Static mesh · point agent · right-drag orbit · middle-drag pan" } else { "Yard3D · collider-proxy picking · right-drag orbit · middle-drag pan" };
        #[cfg(feature="mesh-navigation")]
        let mesh_hud_visible = if self.editor.game().has_mesh_view() && frame_drawn && !self.mesh_navigation.geometry_preview_active() {
            match (self.mesh_hud.applied_document(&self.editor), self.editor.snapshot()) {
                (Some(document), Some(snapshot)) if document.enabled => {
                    let frame = snapshot.predicted();
                    if let Ok(view) = orr_remote::mesh_navigation::mesh_from_view(frame) {
                        document.draw(ui, rect, &view, frame.tick());
                        true
                    } else { false }
                }
                _ => false,
            }
        } else { false };
        #[cfg(not(feature="mesh-navigation"))]
        let mesh_hud_visible = false;
        if !mesh_hud_visible {
        ui.painter().text(rect.left_top()+egui::vec2(10.0,10.0),egui::Align2::LEFT_TOP,viewport_hint,egui::FontId::proportional(12.0),Color32::WHITE);
        }
        if self.editor.previewing().is_some(){ui.painter().text(rect.center(),egui::Align2::CENTER_CENTER,"3D proposal preview unavailable; displaying live host snapshot",egui::FontId::proportional(14.0),Color32::YELLOW);}
    }

    // ---- viewport ----

    fn viewport(&mut self, ui: &mut Ui) {
        #[cfg(feature = "static-gameplay")]
        if self.editor.game().is_gameplay() {
            let panel_rect = ui.available_rect_before_wrap();
            let force_blocked = self.ui.dialog.is_some() || self.editor.previewing().is_some();
            let blocked = force_blocked || ui.input(|input| !input.focused);
            let embedded = self.registered_gameplay_view.show(ui, &mut self.editor,
                &mut self.ui.take_control, self.ui.input_player, blocked, force_blocked);
            if !embedded { self.legacy_viewport(ui); }
            // Include capability controls in pointer ownership; their clicks must
            // not look like a click outside the game on the following frame.
            self.ui.viewport_rect = Some(panel_rect);
            return;
        }
        self.legacy_viewport(ui);
    }

    fn legacy_viewport(&mut self, ui: &mut Ui) {
        if self.editor.game().is_3d() { self.yard_viewport(ui);return; }
        let (rect, resp) = if self.editor.game().has_keyboard() {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            (rect, ui.interact(rect, egui::Id::new("arena_keyboard_viewport"), Sense::click_and_drag()))
        } else {
            ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag())
        };
        if self.ui.take_control {
            self.ui.take_control = false;
            if self.ui.dialog.is_none() && ui.input(|i| i.focused) {
                resp.request_focus();
                self.editor.take_control(self.ui.input_player, true);
            }
        }
        #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
        let mapped_collect = self.editor.game().is_collect();
        #[cfg(not(all(feature = "collect-ui", feature = "collect-input-map-editor")))]
        let mapped_collect = false;
        // Collect resolves Escape by physical key, just like its action map.
        // Keep the legacy logical-key release rule for all other games.
        let focused = resp.has_focus() && self.ui.dialog.is_none()
            && ui.input(|i| i.focused && (mapped_collect || !i.key_pressed(Key::Escape)));
        #[cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
        if mapped_collect {
            use crate::editor::input::Phase;
            self.ui.collect_controls.sync_session(self.editor.scene_load_epoch(), true);
            let phase = self.editor.input_phase();
            let enabled = focused && self.editor.can_take_control()
                && matches!(phase, Phase::Claiming | Phase::Active);
            let events = ui.input(|i| i.events.clone());
            let routed = self.ui.collect_controls.route(&events, enabled);
            if routed.release {
                self.editor.release_control();
                resp.surrender_focus();
            } else {
                for pressed in routed.restart_transitions {
                    self.editor.collect_space_event(enabled, pressed);
                }
            }
            self.editor.arena_keys(enabled && !routed.release, routed.keys);
            if enabled && phase == Phase::Active {
                ui.input_mut(|i| {
                    let mut index = 0;
                    i.events.retain(|_| {
                        let keep = routed.consumed.binary_search(&index).is_err();
                        index += 1;
                        keep
                    });
                });
            }
        }
        #[cfg(feature = "static-game-actions")]
        let mapped_static = self.editor.game().registered().is_some_and(|g| g.action_profile().is_some());
        #[cfg(not(feature = "static-game-actions"))]
        let mapped_static = false;
        #[cfg(feature = "static-game-actions")]
        {
            let enabled = focused && self.editor.can_take_control();
            let (keys, consumed, release) = ui.input(|i| self.ui.static_actions.route(
                self.editor.game().registered(), self.editor.scene_load_epoch(), self.editor.static_action_intent(), enabled,
                self.editor.input_phase(), &i.events));
            if mapped_static {
                self.editor.static_action_keys(enabled && !release, keys);
                if release { resp.surrender_focus(); }
                ui.input_mut(|i| {
                    let mut index = 0;
                    i.events.retain(|_| { let keep = !consumed.contains(&index); index += 1; keep });
                });
            }
        }
        if !mapped_collect && !mapped_static {
            let keys = ui.input(|i| crate::editor::input::Keys {
                x: i8::from(i.key_down(Key::D) || i.key_down(Key::ArrowRight)) - i8::from(i.key_down(Key::A) || i.key_down(Key::ArrowLeft)),
                y: i8::from(i.key_down(Key::W) || i.key_down(Key::ArrowUp)) - i8::from(i.key_down(Key::S) || i.key_down(Key::ArrowDown)),
                fire: i.key_down(Key::Space),
            });
            if self.editor.game().is_collect() && focused {
                let edges = ui.input(|i| i.events.iter().filter_map(|event| match event {
                    egui::Event::Key { key:Key::Space, pressed, repeat:false, .. } => Some(*pressed), _ => None,
                }).collect::<Vec<_>>());
                for pressed in edges {
                    self.editor.collect_space_event(focused, pressed);
                }
            }
            self.editor.arena_keys(focused, keys);
        }
        if self.editor.game().has_keyboard() && resp.has_focus() {
            // Collect's physical routing owns Escape/Tab while claiming or
            // active; egui must not drop focus first based on the logical key.
            // Other games retain their existing arrow-only focus lock.
            let controlled = focused && self.editor.input_phase() == crate::editor::input::Phase::Active;
            let collect_controlled = mapped_collect && focused && matches!(self.editor.input_phase(),
                crate::editor::input::Phase::Claiming | crate::editor::input::Phase::Active);
            ui.memory_mut(|memory| memory.set_focus_lock_filter(resp.id, egui::EventFilter {
                horizontal_arrows: controlled, vertical_arrows: controlled,
                escape: collect_controlled, tab: collect_controlled,
            }));
        }
        if !mapped_collect && !mapped_static && focused && self.editor.input_phase() == crate::editor::input::Phase::Active {
            ui.input_mut(|i| i.events.retain(|event| !matches!(event, egui::Event::Key { key: Key::W | Key::A | Key::S | Key::D | Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::Space, .. })));
        }
        let ppp = ui.ctx().pixels_per_point();
        let px = (((rect.width() * ppp).round() as u32).clamp(1, 8192), ((rect.height() * ppp).round() as u32).clamp(1, 8192));
        self.ui.viewport_rect = Some(rect);
        self.ui.viewport_px = px;
        let to_px = |p: Pos2| [(p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp];

        #[cfg(feature = "sprites")]
        self.sprites.update(&mut self.editor);

        // Camera: pan with middle or right drag, zoom with the wheel.
        if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) {
            let d = resp.drag_delta();
            #[cfg(feature = "sprites")]
            if d != egui::Vec2::ZERO { self.sprites.playback.suspend_follow(); }
            self.editor.camera.pan_pixels([d.x * ppp, d.y * ppp], px);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if let (true, Some(at)) = (scroll != 0.0, resp.hover_pos()) {
                #[cfg(feature = "sprites")]
                self.sprites.playback.suspend_follow();
                self.editor.camera.zoom_at((scroll * 0.0015).exp(), to_px(at), px);
            }
        }

        #[cfg(feature = "tilemap")]
        let tile_pointer_owned = self.tilemap.as_mut().is_some_and(|panel| panel.paint_viewport(ui, &self.editor, &self.sprites, rect, px));
        #[cfg(not(feature = "tilemap"))]
        let tile_pointer_owned = false;
        if tile_pointer_owned && self.ui.body_drag.take().is_some() { self.editor.cancel_edit(); }
        // While a proposal is previewed the viewport is read-only (the frame on screen is not the document's).
        let previewing = self.editor.previewing().map(str::to_string);
        let live = self.editor.down().is_none();
        // Select and move bodies with the primary button.
        if !tile_pointer_owned && self.editor.can_mutate() && previewing.is_none() && self.editor.selected_guids().len() <= 1
            && !ui.input(|i| i.modifiers.ctrl || i.modifiers.command) && resp.drag_started_by(PointerButton::Primary) {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                let world = self.editor.camera.screen_to_world(to_px(origin), px);
                if let Some(target) = self.editor.pick(world) {
                    let pos = self.editor.body_pos(&target);
                    self.editor.select(Some(target));
                    if let Some(pos) = pos.filter(|_| self.editor.movement_owner().is_some()) {
                        self.ui.body_drag = Some(BodyDrag { offset: [pos[0] - world[0], pos[1] - world[1]] });
                        self.editor.begin_edit("move entity");
                    }
                }
            }
        }
        if let (Some(drag), true) = (self.ui.body_drag, resp.dragged_by(PointerButton::Primary)) {
            if let Some(at) = resp.interact_pointer_pos() {
                let w = self.editor.camera.screen_to_world(to_px(at), px);
                let p = [w[0] + drag.offset[0], w[1] + drag.offset[1]];
                if let (Some(x), Some(y)) = (fp_of_f64(f64::from(p[0])), fp_of_f64(f64::from(p[1]))) {
                    if let Some(owner) = self.editor.movement_owner() {
                        self.editor.set_field(&owner, self.editor.game().position_field(), Value::Vec2(FPVec2::new(x, y)));
                    }
                }
            }
        }
        if resp.drag_stopped() && self.ui.body_drag.take().is_some() {
            self.editor.end_edit();
        }
        if !tile_pointer_owned && live && previewing.is_none() && resp.clicked_by(PointerButton::Primary) {
            if let Some(at) = resp.interact_pointer_pos() {
                let world = self.editor.camera.screen_to_world(to_px(at), px);
                let hit = self.editor.pick(world);
                if ui.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                    if let Some(hit) = hit { self.editor.toggle_selection(hit); }
                } else { self.editor.select(hit); }
            }
        }

        // Draw.
        let now = ui.ctx().input(|i| i.time);
        let pulses: Vec<(Target, f32)> = self.ui.pulses.iter().map(|p| (p.target.clone(), (1.0 - (now - p.started) / viewport::PULSE_SECONDS).clamp(0.0, 1.0) as f32)).collect();
        let list = self.editor.viewport_list(px, &pulses);
        match &self.render_state {
            Some(rs) => {
                let gpu = self.gpu.get_or_insert_with(|| GpuViewport::new(rs, px));
                #[cfg(feature = "tilemap")]
                let rendered = match self.tilemap.as_mut().filter(|panel| panel.scene_matches(&self.editor)) {
                    Some(panel) => panel.render(gpu, px, &list, &self.editor.camera, self.sprites.bindings.as_ref().map_or(0, |bindings| bindings.document().bindings.len()), &self.editor),
                    None => Ok(gpu.render(px, &list, &self.editor.camera)),
                };
                #[cfg(not(feature = "tilemap"))]
                let rendered: Result<egui::TextureId, String> = Ok(gpu.render(px, &list, &self.editor.camera));
                match rendered {
                    Ok(tex) => { ui.painter().image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); },
                    Err(error) => { self.editor.error(&error); ui.painter().rect_filled(rect, 0.0, Color32::BLACK); },
                }
            }
            None => {
                ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(10, 10, 16));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "viewport needs a GPU (wgpu adapter)", egui::FontId::proportional(14.0), Color32::GRAY);
            }
        }
        #[cfg(feature = "sprites")]
        self.sprites.paint(ui, &self.editor, rect, px);
        #[cfg(feature = "tilemap")]
        if let Some(panel) = &self.tilemap { panel.paint_grid(ui, &self.editor, rect, px); }
        if !live {
            ui.painter().rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 120));
        }
        let label = match self.editor.timeline() {
            Some(t) => format!("{}  tick {}", if self.editor.is_viewer() { "REPLAY VIEWER" } else { "PLAY" }, t.tick),
            None => "EDIT".to_string(),
        };
        ui.painter().text(rect.min + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, label, egui::FontId::monospace(13.0), Color32::from_gray(200));
        if let Some(id) = previewing {
            let font = egui::FontId::monospace(15.0);
            let text = format!("PREVIEW {id} · read-only");
            let galley = ui.painter().layout_no_wrap(text, font, Color32::BLACK);
            let box_rect = Rect::from_min_size(rect.min + egui::vec2(8.0, 26.0), galley.size() + egui::vec2(14.0, 6.0));
            ui.painter().rect_filled(box_rect, 3.0, Color32::from_rgb(90, 200, 240));
            ui.painter().galley(box_rect.min + egui::vec2(7.0, 3.0), galley, Color32::BLACK);
            let hint = "cyan = changed, green = added, ghost = removed / old place";
            let hint_galley = ui.painter().layout_no_wrap(hint.to_string(), egui::FontId::monospace(11.0), Color32::from_gray(210));
            let hint_rect = Rect::from_min_size(box_rect.left_bottom() + egui::vec2(0.0, 4.0), hint_galley.size() + egui::vec2(8.0, 4.0));
            ui.painter().rect_filled(hint_rect, 3.0, Color32::from_rgba_unmultiplied(10, 10, 16, 200));
            ui.painter().galley(hint_rect.min + egui::vec2(4.0, 2.0), hint_galley, Color32::from_gray(210));
            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(2.0, Color32::from_rgb(90, 200, 240)), egui::StrokeKind::Inside);
        }
        if self.editor.is_playing_mode() {
            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(2.0, Color32::from_rgb(255, 150, 60)), egui::StrokeKind::Inside);
        }
    }

    // ---- bottom panel: timeline and history ----

    fn bottom(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::Timeline, "Timeline");
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::History, "History");
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::Agent, "Agent");
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::UnsavedWork, "Unsaved work details");
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::ExportGuide, "Export guide");
            #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::ArenaExport, "Arena export");
            let unseen = self.editor.feed().unseen();
            if unseen > 0 && self.ui.bottom_tab != BottomTab::Agent {
                // Badge: what agents did since the tab was last shown (reads excluded).
                ui.label(RichText::new(format!(" {unseen} ")).strong().color(Color32::BLACK).background_color(Color32::from_rgb(240, 170, 60)))
                    .on_hover_text(format!("{unseen} new agent action(s)"));
            }
        });
        ui.separator();
        match self.ui.bottom_tab {
            BottomTab::UnsavedWork => self.show_close_review(ui),
            BottomTab::ExportGuide => self.show_export_guide(ui),
            #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
            BottomTab::ArenaExport => { egui::ScrollArea::vertical().id_salt("arena_export_scroll").show(ui, |ui| self.show_arena_export(ui)); },
            BottomTab::Timeline => self.timeline(ui),
            BottomTab::History => self.history(ui),
            BottomTab::Agent => {
                self.agent_tab(ui);
                self.editor.feed_mut().mark_seen();
            }
        }
    }

    fn timeline(&mut self, ui: &mut Ui) {
        let Some(tl) = self.editor.timeline() else {
            ui.weak("Edit mode. Press Play to bake the scene and start a session; Step runs one tick.");
            return;
        };
        let start_allowed = self.playback_start_blocker().is_none();
        ui.horizontal(|ui| {
            if self.editor.down().is_some() { ui.disable(); }
            if ui.button("\u{23EE}").on_hover_text("Rewind to the first tick").clicked() {
                self.editor.seek(tl.first_tick);
            }
            if ui.add_enabled(start_allowed && !tl.playing, egui::Button::new("\u{25B6}")).on_hover_text(if self.editor.is_viewer() { "Play recorded ticks only" } else { "Play by the clock (from a rewound tick this branches)" }).clicked() {
                self.play_from_ui();
            }
            if ui.add_enabled(tl.playing, egui::Button::new("\u{23F8}")).on_hover_text("Pause").clicked() {
                self.editor.pause();
            }
            if ui.add_enabled(start_allowed && !tl.playing, egui::Button::new("\u{23ED}")).on_hover_text("Step one tick").clicked() {
                self.step_from_ui();
            }
            ui.separator();
            ui.label("speed");
            let mut speed = tl.speed.permille() as f32 / 1000.0;
            if ui.add(egui::Slider::new(&mut speed, 0.25..=4.0).logarithmic(true).suffix("x")).changed() {
                self.editor.set_speed(speed);
            }
            ui.separator();
            let can_branch = !self.editor.is_viewer() && tl.tick < tl.last_tick;
            if ui.add_enabled(can_branch, egui::Button::new("Branch")).on_hover_text("Drop the recorded ticks after the current one").clicked() {
                self.editor.branch();
            }
        });
        ui.horizontal(|ui| {
            let mut tick = tl.tick;
            ui.label("tick");
            let slider = egui::Slider::new(&mut tick, tl.first_tick..=tl.last_tick.max(tl.first_tick)).integer();
            ui.spacing_mut().slider_width = (ui.available_width() - 330.0).max(120.0);
            if ui.add(slider).changed() && tick != tl.tick {
                self.editor.seek(tick);
            }
            ui.monospace(format!("{} / {}", tl.tick, tl.last_tick));
        });
        ui.horizontal(|ui| {
            ui.monospace(format!("checksum {:#018x}", tl.checksum));
            ui.weak(format!("keyframes {}", tl.keyframes.len()));
            ui.weak(format!("branches {}", tl.branches));
            if tl.pending_edits > 0 {
                ui.weak(format!("{} edit(s) pending", tl.pending_edits));
            }
            ui.weak(if tl.playing { "playing" } else { "paused" });
        });
    }

    fn history(&mut self, ui: &mut Ui) {
        let history = self.editor.history().entries.clone();
        if history.is_empty() {
            ui.weak("No edits yet.");
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for h in history.iter().rev() {
                let text = format!("#{}  {}  [{}]{}", h.id, h.label, h.origin, if h.op_count > 1 { format!("  ({} ops)", h.op_count) } else { String::new() });
                if h.undone {
                    ui.label(RichText::new(text).weak().strikethrough());
                } else {
                    ui.label(text);
                }
            }
        });
    }

    // ---- dialogs and screenshot ----

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.ui.dialog.take() else { return };
        let title = match dialog.kind {
            DialogKind::Open => "Open scene",
            DialogKind::SaveAs => "Save scene as",
            #[cfg(feature="mesh-navigation")]
            DialogKind::NewMesh => "New mesh scene",
            #[cfg(feature="mesh-navigation")]
            DialogKind::OpenMesh => "Open mesh scene",
        };
        let mut keep = true;
        let mut accept = false;
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            let label = ui.label("Scene path");
            let r = ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(420.0)).labelled_by(label.id);
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                accept = true;
            }
            ui.horizontal(|ui| {
                #[cfg(feature="mesh-navigation")]
                let action = if dialog.kind == DialogKind::NewMesh { "Create" } else { "OK" };
                #[cfg(not(feature="mesh-navigation"))]
                let action = "OK";
                if ui.button(action).clicked() {
                    accept = true;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
        if accept && !dialog.text.trim().is_empty() {
            #[cfg(feature = "tilemap")]
            if self.tilemap.is_some() {
                self.editor.error("Tilemap scene identity is fixed; reopen a complete project instead of scene Open/Save As");
                self.ui.dialog = Some(dialog);
                return;
            }
            if self.hazard_export_busy() {
                self.editor.error("Wait for hazard export completion before saving or opening a scene");
                self.ui.dialog = Some(dialog);
                return;
            }
            let path = PathBuf::from(dialog.text.trim());
            #[cfg(feature="mesh-mission")]
            if self.editor.game().has_mesh_view() {
                self.mesh_mission.sync(&self.editor);
                if self.mesh_mission_dirty() {
                    self.editor.error("Apply or reload mission fields before saving or opening a scene");
                    self.ui.dialog = Some(dialog);
                    return;
                }
            }
            #[cfg(feature="mesh-hazards")]
            if self.editor.game().is_mesh_hazards() {
                self.mesh_hazards.sync(&self.editor);
                if self.mesh_hazards.dirty() {
                    self.editor.error("Apply or reload hazard fields before saving or opening a scene");
                    self.ui.dialog = Some(dialog);
                    return;
                }
            }

            #[cfg(feature="mesh-navigation")]
            if self.mesh_navigation.art_import_busy() || self.mesh_navigation.prop_gesture_busy() || self.mesh_navigation.agent_pick_busy() || self.mesh_mission_pick_busy() || self.hazard_pick_busy() {
                self.editor.error("Finish content imports, viewport moves or endpoint picking before changing the scene path");
                self.ui.dialog = Some(dialog);
                return;
            }
            #[cfg(feature="mesh-navigation")]
            if dialog.kind != DialogKind::SaveAs && (self.mesh_navigation.has_unplaced_draft() || self.mesh_camera.dirty() || self.mesh_lighting.dirty() || self.mesh_hud.dirty() || self.mesh_mission_dirty() || self.mesh_hazards_dirty() || self.mesh_animation_dirty()) {
                self.editor.error("Apply, save or explicitly discard mesh drafts, mission, camera, lighting, HUD, animation, agent art and prop changes before changing scenes");
                self.ui.dialog = Some(dialog);
                return;
            }
            #[cfg(feature="terrain")]
            if self.terrain.document.dirty() {
                self.editor.error("Save or explicitly discard terrain changes before changing the scene path");
                self.ui.dialog = Some(dialog);
                return;
            }
            let ok = match dialog.kind {
                DialogKind::Open => self.editor.open_path(&path),
                DialogKind::SaveAs => self.editor.save_as(&path),
                #[cfg(feature="mesh-navigation")]
                DialogKind::NewMesh => self.switch_mesh_workspace(&path, crate::mesh_workspace::Intent::Create),
                #[cfg(feature="mesh-navigation")]
                DialogKind::OpenMesh => self.switch_mesh_workspace(&path, crate::mesh_workspace::Intent::Open),
            };
            keep = !ok;
        }
        if keep {
            self.ui.dialog = Some(dialog);
        }
    }

    fn poll_remote_capture_request(&mut self) {
        let mut clear_unrequested = false;
        if let Some(capture) = self.remote_capture.as_mut() {
            if !self.editor.screenshot_is_active(capture.request.serial) {
                self.editor.cancel_screenshot_validation(capture.request.serial);
                if capture.screenshot_requested && !capture.image_received {
                    // Keep the old ticket/pass until its matching GPU event is
                    // consumed; this remains true across a backend restart in
                    // the same App because the physical renderer is unchanged.
                    capture.orphaned = true;
                } else if !capture.screenshot_requested {
                    clear_unrequested = true;
                }
            }
        }
        if clear_unrequested {
            self.remote_capture = None;
        }
        if self.remote_capture.is_some() {
            return;
        }
        let Some(request) = self.editor.take_screenshot_request() else { return };
        if self.shot.is_some() {
            self.editor.screenshot_complete(request.serial, Err(CaptureError::Unavailable));
            return;
        }
        match self.editor.begin_screenshot_validation(&request) {
            Ok(()) => {
                self.remote_capture = Some(RemoteCapture { request, pass: None, screenshot_requested: false, image_received: false, orphaned: false });
            }
            Err(error) => self.editor.screenshot_complete(request.serial, Err(error)),
        }
    }

    fn process_remote_capture(&mut self, ctx: &egui::Context) {
        self.reap_capture_encoder(ctx);
        self.receive_capture_event(ctx);
        self.request_capture_frame(ctx);
        if self.remote_capture.as_ref().is_some_and(|capture| !capture.orphaned) || self.encoder.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn request_capture_frame(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.remote_capture.as_ref() else { return };
        if capture.orphaned || capture.pass.is_some() || !self.editor.screenshot_is_active(capture.request.serial) {
            return;
        }
        if self.encoder.is_some() {
            return;
        }
        let serial = capture.request.serial;
        let request = capture.request.clone();
        let state = match self.editor.screenshot_capture_ready(serial) {
            Ok(Some(state)) => state,
            Ok(None) => return,
            Err(error) => {
                self.finish_remote_capture(serial, Err(error));
                return;
            }
        };
        if !self.ui.pulses.is_empty() || self.editor.previewing().is_some() || self.editor.screenshot_gesture_busy() {
            return;
        }
        let Some(frame_seq) = self.editor.capture_snapshot_seq().filter(|seq| *seq > 0) else { return };
        let Some(framebuffer) = framebuffer_info(ctx) else {
            self.finish_remote_capture(serial, Err(CaptureError::Unavailable));
            return;
        };
        let pixels = u64::from(framebuffer.width) * u64::from(framebuffer.height);
        if framebuffer.width > request.options.max_width
            || framebuffer.height > request.options.max_height
            || pixels > MAX_CAPTURE_PIXELS
            || pixels.saturating_mul(4) > MAX_CAPTURE_RGBA_BYTES as u64
        {
            self.finish_remote_capture(serial, Err(CaptureError::Failed));
            return;
        }
        let pass = CapturePass { state, frame_seq, ui_frame: self.frames, framebuffer };
        if self.editor.capture_view_state() != state || !self.editor.capture_snapshot_matches(state) {
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        }
        if let Err(error) = self.editor.screenshot_begin_capture(serial) {
            self.finish_remote_capture(serial, Err(error));
            return;
        }
        if let Some(capture) = self.remote_capture.as_mut().filter(|capture| capture.request.serial == serial) {
            capture.pass = Some(pass);
            capture.screenshot_requested = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(serial)));
    }

    fn receive_capture_event(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.remote_capture.as_ref() else { return };
        if !capture.screenshot_requested || capture.image_received {
            return;
        }
        let serial = capture.request.serial;
        let Some(pass) = capture.pass else { return };
        let event = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if screenshot_event_matches(event, pass.framebuffer.viewport, serial) {
                    match event {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
        });
        let Some(image) = event else { return };
        if let Some(capture) = self.remote_capture.as_mut().filter(|capture| capture.request.serial == serial) {
            capture.image_received = true;
        }
        if !self.editor.screenshot_is_active(serial) {
            self.editor.screenshot_end_capture(serial);
            self.editor.cancel_screenshot_validation(serial);
            self.remote_capture = None;
            return;
        }
        if !self.capture_pass_still_current(ctx, pass) {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        }
        let (Ok(width), Ok(height)) = (u32::try_from(image.width()), u32::try_from(image.height())) else {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        };
        let pixels = u64::from(width) * u64::from(height);
        if width != pass.framebuffer.width
            || height != pass.framebuffer.height
            || pixels > MAX_CAPTURE_PIXELS
            || pixels.saturating_mul(4) > MAX_CAPTURE_RGBA_BYTES as u64
            || image.pixels.len() as u64 != pixels
        {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        }
        if let Err(error) = self.editor.screenshot_begin_encoder(serial) {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(error));
            return;
        }
        let handle = std::thread::Builder::new()
            .name("orr-screenshot-png".into())
            .spawn(move || encode_png_bounded(&image));
        match handle {
            Ok(handle) => self.encoder = Some(EncoderWorker { serial, pass, handle }),
            Err(_) => {
                self.editor.screenshot_end_encoder(serial);
                self.finish_remote_capture(serial, Err(CaptureError::Failed));
            }
        }
    }

    fn reap_capture_encoder(&mut self, ctx: &egui::Context) {
        if !self.encoder.as_ref().is_some_and(|worker| worker.handle.is_finished()) {
            return;
        }
        let worker = self.encoder.take().expect("finished screenshot worker exists");
        let result = worker.handle.join().unwrap_or(Err(CaptureError::Failed));
        if !self.editor.screenshot_is_active(worker.serial) {
            self.editor.screenshot_end_encoder(worker.serial);
            self.editor.cancel_screenshot_validation(worker.serial);
            if self.remote_capture.as_ref().is_some_and(|capture| capture.request.serial == worker.serial) {
                self.remote_capture = None;
            }
            return;
        }
        if !self.capture_pass_still_current(ctx, worker.pass) {
            self.editor.screenshot_end_encoder(worker.serial);
            self.finish_remote_capture(worker.serial, Err(CaptureError::Stale));
            return;
        }
        match result {
            Ok(png) => {
                let image = CapturedImage {
                    png,
                    width: worker.pass.framebuffer.width,
                    height: worker.pass.framebuffer.height,
                    captured: worker.pass.state,
                    frame_seq: worker.pass.frame_seq,
                    ui_frame: worker.pass.ui_frame,
                };
                self.editor.cancel_screenshot_validation(worker.serial);
                // Publish while the just-finished worker still owns the shared
                // capture permit; release only after its real exit is observed.
                self.editor.screenshot_complete(worker.serial, Ok(image));
                self.editor.screenshot_end_encoder(worker.serial);
                self.remote_capture = None;
            }
            Err(error) => {
                self.editor.screenshot_end_encoder(worker.serial);
                self.finish_remote_capture(worker.serial, Err(error));
            }
        }
    }

    fn capture_pass_still_current(&self, ctx: &egui::Context, pass: CapturePass) -> bool {
        let Some(current) = framebuffer_info(ctx) else { return false };
        same_framebuffer(current, pass.framebuffer)
            && self.editor.capture_view_state() == pass.state
            && self.editor.capture_snapshot_seq() == Some(pass.frame_seq)
            && self.editor.capture_snapshot_matches(pass.state)
            && self.editor.previewing().is_none()
            && !self.editor.screenshot_gesture_busy()
            && self.ui.pulses.is_empty()
            && self.editor.screenshot_waiting_for().is_none()
    }

    fn finish_remote_capture(&mut self, serial: u64, result: Result<CapturedImage, CaptureError>) {
        self.editor.cancel_screenshot_validation(serial);
        self.editor.screenshot_complete(serial, result);
        if self.remote_capture.as_ref().is_some_and(|capture| capture.request.serial == serial) {
            self.remote_capture = None;
        }
    }

    fn report_ui_settled(&mut self, ctx: &egui::Context) {
        if self.ui_settled_reported || !self.editor.has_local_screenshot_owner() || self.render_state.is_none() {
            return;
        }
        if !self.ui.pulses.is_empty() || self.editor.previewing().is_some() || self.editor.screenshot_gesture_busy() {
            return;
        }
        if framebuffer_info(ctx).is_none() || self.editor.screenshot_waiting_for().is_some() {
            return;
        }
        let Some((url, _)) = self.editor.erp_status() else { return };
        eprintln!("Editor UI settled: {url}");
        self.ui_settled_reported = true;
    }

    /// `--screenshot`: after N frames ask the window for a screenshot of its
    /// own framebuffer, write the PNG and quit.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let frames = self.frames;
        let waiting_for = if self.ui.pulses.is_empty() { self.editor.screenshot_waiting_for() } else { Some("agent pulses to expire") };
        let Some(job) = &mut self.shot else { return };
        ctx.request_repaint();
        match job.requested_at {
            None => match job.ready(frames, ctx.input(|i| i.time), waiting_for) {
                Ok(true) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                    job.requested_at = Some(frames);
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("screenshot failed: {e}");
                    std::process::exit(2);
                }
            }
            Some(at) => {
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { viewport_id, user_data, image }
                            if *viewport_id == egui::ViewportId::ROOT && user_data.data.is_none() => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = image {
                    match write_png(&job.path, &image) {
                        Ok(()) => eprintln!("screenshot written: {} ({}x{})", job.path.display(), image.width(), image.height()),
                        Err(e) => {
                            eprintln!("screenshot failed: {e}");
                            std::process::exit(2);
                        }
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    self.shot = None;
                } else if frames > at + 600 {
                    eprintln!("screenshot failed: the window never answered");
                    std::process::exit(2);
                }
            }
        }
    }
}

fn framebuffer_info(ctx: &egui::Context) -> Option<FramebufferInfo> {
    let (viewport, inner_rect) = ctx.input(|input| (input.raw.viewport_id, input.raw.viewport().inner_rect));
    framebuffer_info_from(viewport, inner_rect?, ctx.pixels_per_point())
}

fn framebuffer_info_from(viewport: egui::ViewportId, rect: Rect, ppp: f32) -> Option<FramebufferInfo> {
    if viewport != egui::ViewportId::ROOT { return None; }
    let physical_width = rect.width() * ppp;
    let physical_height = rect.height() * ppp;
    if !rect.width().is_finite()
        || !rect.height().is_finite()
        || !ppp.is_finite()
        || !physical_width.is_finite()
        || !physical_height.is_finite()
        || rect.width() <= 0.0
        || rect.height() <= 0.0
        || ppp <= 0.0
        || physical_width < 1.0
        || physical_height < 1.0
        || physical_width >= u32::MAX as f32
        || physical_height >= u32::MAX as f32
    {
        return None;
    }
    Some(FramebufferInfo {
        viewport,
        logical_width: rect.width(),
        logical_height: rect.height(),
        pixels_per_point: ppp,
        width: physical_width.round() as u32,
        height: physical_height.round() as u32,
    })
}

fn same_framebuffer(a: FramebufferInfo, b: FramebufferInfo) -> bool {
    a.viewport == b.viewport
        && a.width == b.width
        && a.height == b.height
        && a.logical_width.to_bits() == b.logical_width.to_bits()
        && a.logical_height.to_bits() == b.logical_height.to_bits()
        && a.pixels_per_point.to_bits() == b.pixels_per_point.to_bits()
}

fn screenshot_event_matches(event: &egui::Event, viewport: egui::ViewportId, serial: u64) -> bool {
    matches!(event,
        egui::Event::Screenshot { viewport_id, user_data, .. }
            if *viewport_id == viewport
                && user_data.data.as_ref().and_then(|data| data.downcast_ref::<u64>()).copied() == Some(serial)
    )
}

fn encode_png_bounded(image: &egui::ColorImage) -> Result<Vec<u8>, CaptureError> {
    let width = u32::try_from(image.width()).map_err(|_| CaptureError::Failed)?;
    let height = u32::try_from(image.height()).map_err(|_| CaptureError::Failed)?;
    let pixels = u64::from(width) * u64::from(height);
    let rgba_len = usize::try_from(pixels.checked_mul(4).ok_or(CaptureError::Failed)?)
        .map_err(|_| CaptureError::Failed)?;
    if width == 0 || height == 0 || pixels > MAX_CAPTURE_PIXELS || rgba_len > MAX_CAPTURE_RGBA_BYTES || image.pixels.len() as u64 != pixels {
        return Err(CaptureError::Failed);
    }
    let mut rgba = Vec::with_capacity(rgba_len);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&pixel.to_array());
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(BoundedPngWriter(&mut png), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|_| CaptureError::Failed)?;
    writer.write_image_data(&rgba).map_err(|_| CaptureError::Failed)?;
    writer.finish().map_err(|_| CaptureError::Failed)?;
    Ok(png)
}

/// Writes a `ColorImage` as an RGBA PNG.
pub fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), image.width() as u32, image.height() as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Best);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())
}

#[cfg(all(test, feature="mesh-mission"))]
#[path = "app/mesh_game_creator_tests.rs"]
mod mesh_game_creator_tests;

#[cfg(test)]
mod screenshot_tests {
    use super::*;

    #[test]
    fn long_status_messages_preserve_the_available_viewport_rect() {
        let editor = Editor::open(&crate::editor::default_scene_path()).unwrap();
        let mut app = EditorApp::new(editor, None);
        app.editor.sync();
        let checksum = app.editor.checksum();
        let history_count = app.editor.history().entries.len();
        let long_message = "Mesh mission checkpoint authoring status ".repeat(256);
        for width in [480.0, 900.0, 1500.0] {
            let context = egui::Context::default();
            let mut baseline = None;
            for (message, error) in [("ready", false), (long_message.as_str(), false), (long_message.as_str(), true)] {
                if error { app.editor.error(message); } else { app.editor.info(message); }
                let mut available = None;
                let _output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 400.0))),
                    ..Default::default()
                }, |ui| {
                    egui::Panel::bottom("status").show(ui, |ui| app.status_bar(ui));
                    egui::Panel::right("inspector").exact_size(120.0).show(ui, |_| {});
                    available = Some(ui.available_rect_before_wrap());
                });
                let available = available.unwrap();
                assert!((available.max.x - (width - 120.0)).abs() < 0.5, "status expanded viewport at width {width}: {available:?}");
                if let Some(baseline) = baseline { assert_eq!(available, baseline); } else { baseline = Some(available); }
            }
        }
        assert_eq!(app.editor.checksum(), checksum);
        assert_eq!(app.editor.history().entries.len(), history_count);
    }

    #[cfg(all(feature="mesh-navigation", not(feature="mesh-animated-models")))]
    #[test]
    fn mesh_animation_presence_blocks_play_and_preserves_the_current_workspace() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let scene = root.join("animation.scene.yaml");
        crate::mesh_scene::create(&scene).unwrap();
        let sidecar = scene.with_file_name("animation.scene.yaml.mesh-animation.json");
        let mut editor = crate::mesh_workspace::prepare(&scene, crate::mesh_workspace::Intent::Open).unwrap();
        let mut navigation = crate::mesh_navigation::Panel::default();
        navigation.create_ramp_and_bridge().unwrap();
        navigation.place_agent(&mut editor).unwrap();
        editor.sync();
        assert!(editor.save());
        editor.sync();
        let mut app = EditorApp::new(editor, None);
        app.sync_mesh_presentation();
        assert!(app.mesh_presentation_blocker().is_none());
        std::fs::write(&sidecar, b"{}").unwrap();
        app.sync_mesh_presentation();
        assert!(app.mesh_animation_blocker().unwrap().contains("mesh-animated-models"));
        app.editor.play();
        app.editor.sync();
        assert_eq!(app.editor.mode(), Mode::Edit);
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"{}");

        let editor = Editor::open(&crate::editor::default_scene_path()).unwrap();
        let mut app = EditorApp::new(editor, None);
        app.editor.sync();
        let original = app.editor.path();
        assert!(!app.switch_mesh_workspace(&scene, crate::mesh_workspace::Intent::Open));
        assert_eq!(app.editor.path(), original);
        assert_eq!(app.editor.game(), crate::game::EditorGame::PhysGame);
    }

    #[test]
    fn ordinary_capture_keeps_frame_count_behavior() {
        let mut job = ScreenshotJob::new("unused.png".into(), 30);
        assert!(!job.ready(29, 0.0, Some("agent pulses")).unwrap());
        assert!(job.ready(30, 0.0, Some("agent pulses")).unwrap());
    }

    #[test]
    fn settle_is_condition_based_and_has_a_deadline() {
        let mut job = ScreenshotJob::new("unused.png".into(), 30).with_settle();
        assert!(!job.ready(30, 3.0, Some("agent pulses")).unwrap());
        assert!(!job.ready(3000, 4.0, Some("model refresh")).unwrap());
        assert!(job.ready(3001, 4.1, None).unwrap());
        assert!(job.ready(3002, 13.0, Some("model refresh")).unwrap_err().contains("deadline exceeded waiting for model refresh"));
    }

    #[test]
    fn settled_capture_waits_for_actual_pulse_expiry_on_egui_clock() {
        let mut editor = Editor::open(&crate::editor::default_scene_path()).unwrap();
        editor.select_named("body_05");
        editor.sync();
        assert_eq!(editor.screenshot_waiting_for(), None);
        let target = editor.selection().unwrap().clone();
        let mut app = EditorApp::new(editor, None).with_screenshot(ScreenshotJob::new("unused.png".into(), 30).with_settle());
        app.frames = 30;
        app.ui.pulses.push(Pulse { target: target.clone(), started: 0.0 });
        let ctx = egui::Context::default();
        let draw = |app: &mut EditorApp, now| {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(now), ..Default::default() }, |ui| {
                let ctx = ui.ctx();
                app.start_pulses(ctx.input(|i| i.time));
                app.screenshot(ctx);
            });
            output.textures_delta.clear(); // This clock/command test deliberately has no GPU.
            output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Screenshot(_))))
        };
        assert!(!draw(&mut app, 0.0));
        assert!(!draw(&mut app, viewport::PULSE_SECONDS - 0.01));
        assert_eq!(app.ui.pulses.len(), 1, "waiting must not remove agent activity");
        // A later edit extends readiness according to its real pulse, not a fixed delay.
        app.ui.pulses.push(Pulse { target, started: 1.0 });
        assert!(!draw(&mut app, viewport::PULSE_SECONDS));
        assert_eq!(app.ui.pulses.len(), 1);
        assert!(draw(&mut app, 1.0 + viewport::PULSE_SECONDS));
        assert!(app.ui.pulses.is_empty());
        assert_eq!(app.shot.as_ref().unwrap().requested_at, Some(30));
    }

    #[test]
    fn remote_screenshot_event_requires_its_ticket_and_root_viewport() {
        let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], Color32::BLACK));
        let matching = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(42_u64),
            image: image.clone(),
        };
        let duplicate = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(41_u64),
            image: image.clone(),
        };
        let foreign_viewport = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::from_hash_of("secondary"),
            user_data: egui::UserData::new(42_u64),
            image,
        };
        assert!(screenshot_event_matches(&matching, egui::ViewportId::ROOT, 42));
        assert!(!screenshot_event_matches(&duplicate, egui::ViewportId::ROOT, 42));
        assert!(!screenshot_event_matches(&foreign_viewport, egui::ViewportId::ROOT, 42));
    }

    #[test]
    fn physical_framebuffer_identity_invalidates_resize_and_dpi_changes() {
        let root = egui::ViewportId::ROOT;
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let original = framebuffer_info_from(root, rect, 1.0).unwrap();
        assert!(same_framebuffer(original, framebuffer_info_from(root, rect, 1.0).unwrap()));
        assert!(!same_framebuffer(original, framebuffer_info_from(root, Rect::from_min_size(Pos2::ZERO, egui::vec2(801.0, 600.0)), 1.0).unwrap()));
        assert!(!same_framebuffer(original, framebuffer_info_from(root, rect, 1.25).unwrap()));
        assert!(framebuffer_info_from(egui::ViewportId::from_hash_of("secondary"), rect, 1.0).is_none());
    }

    #[test]
    fn bounded_png_encoder_and_writer_enforce_allocation_limits() {
        let image = egui::ColorImage::filled([2, 2], Color32::from_rgb(10, 20, 30));
        let png = encode_png_bounded(&image).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() <= MAX_CAPTURE_RGBA_BYTES);

        let mut output = vec![0; MAX_CAPTURE_RGBA_BYTES];
        let mut writer = BoundedPngWriter(&mut output);
        assert!(writer.write_all(b"x").is_err());
        assert_eq!(writer.0.len(), MAX_CAPTURE_RGBA_BYTES);

        let oversized = egui::ColorImage::filled([1025, 1024], Color32::BLACK);
        assert_eq!(encode_png_bounded(&oversized), Err(CaptureError::Failed));
    }
}

#[cfg(all(test, feature = "mesh-mission", feature = "mesh-animated-models"))]
#[path = "app/mesh_panel_layout_tests.rs"]
mod mesh_panel_layout_tests;

#[cfg(all(test, windows, target_arch = "x86_64", feature = "mesh-export", feature = "mesh-mission"))]
#[path = "app/mesh_keyboard_play_tests.rs"]
mod mesh_keyboard_play_tests;

#[cfg(feature = "mesh-hazards-export")]
#[path = "app/mesh_hazards_player.rs"]
pub(crate) mod mesh_hazards_player;

#[cfg(all(test, feature = "tilemap", feature = "project-create", target_os = "linux"))]
#[path = "app/tilemap_tests.rs"]
mod tilemap_tests;

#[cfg(all(test, feature = "tilemap", feature = "project-create", target_os = "linux"))]
#[path = "app/tilemap_fill_tests.rs"]
mod tilemap_fill_tests;

#[cfg(all(test, feature = "tilemap", feature = "project-create", target_os = "linux"))]
#[path = "app/tilemap_resize_tests.rs"]
mod tilemap_resize_tests;

#[cfg(all(test, feature = "tilemap", feature = "project-create", target_os = "linux"))]
#[path = "app/tilemap_rectangle_tests.rs"]
mod tilemap_rectangle_tests;
