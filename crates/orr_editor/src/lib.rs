//! `orr_editor`: the egui editor of Orrery (M4 MVP).
//!
//! The success criterion is the loop *scene edit, play, rewind*: open a scene
//! (default `scenes/physics_demo.scene.yaml`), change bodies with the
//! inspector or by dragging them in the viewport, press Play, scrub the
//! timeline back, and play forward again to the same checksums.
//!
//! # The editor is a view of a simulation host
//!
//! The editor owns no document, no play session and no ERP server. The
//! simulation and the scene document live in a **host** (`orr_remote::Host`):
//! a thread of this process (the default) or another process
//! (`orr_remote_host`, `--connect`). The editor is a client: it speaks ERP
//! for everything a person does and reads the frames it draws through the
//! bridge (`orr_bridge`: `Bridge`, `SimControl`, `Snapshot`, here
//! `orr_remote::RemoteBridge`). A host thread that panics, or a remote host
//! that goes away, does not take the editor down: it says so and offers
//! Restart / Reconnect. See [`backend`] and [`editor`].
//!
//! # Pieces
//!
//! - [`editor`]: [`Editor`], the state machine (no egui): caches of host
//!   answers, actions as ERP calls, and the per-frame [`Editor::pump`].
//! - [`backend`]: [`HostSpec`], the two channels to the host.
//! - [`model`]: plain data parsed from ERP answers.
//! - [`inspector`]: widgets generated from `orr_reflect` descriptors.
//! - [`viewport`]: the render list of the frame on screen (`orr_render`),
//!   picking, and the offscreen texture shared with egui on eframe's own
//!   wgpu device.
//! - [`agent`] and [`agent_ui`]: the Agent tab, a read-only activity feed of
//!   what AI agents do through ERP (no approval step; Undo takes a change
//!   back), plus a view-only preview of the proposals an agent has open (the
//!   staged frame comes from the host as a second frame stream).
//! - [`app`]: [`EditorApp`], the `eframe::App` with all panels.
//! - [`cli`] and [`script`]: command line flags (`--scene`, `--screenshot`,
//!   `--frames`, `--play-ticks`, `--select`, `--script`) for headless checks.
//!
//! Compiled adapters support local/remote `PhysGame` and remote `Arena`.
//! Frame decoders, reflection and viewport mappings come through `orr_sample`.
//! Arena optionally claims a focused realtime keyboard slot through negotiated
//! structured ERP input. Ordinary attachment never changes agent-held input.
//!
//! This is view layer code: floats and the wall clock are fine here, but every
//! value that reaches the document goes through exact decimal parsing.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod agent;
pub mod agent_ui;
pub mod app;
pub mod backend;
pub mod cli;
pub mod diagnostics;
pub mod editor;
pub mod inspector;
pub mod game;
pub mod model;
pub mod script;
pub mod viewport;

pub use app::{EditorApp, ScreenshotJob};
pub use backend::HostSpec;
pub use editor::{Editor, Mode, Owner};
pub use model::Target;

#[cfg(feature = "sprites")]
pub mod sprite_bindings;
#[cfg(feature = "sprites")]
pub mod sprite_panel;
#[cfg(feature = "sprites")]
pub mod sprite_playback;
#[cfg(feature = "sprites")]
pub mod project;

#[cfg(feature = "animated-models")]
pub mod animated_bindings;
#[cfg(feature = "animated-models")]
pub mod animated_preview;
#[cfg(feature = "animated-models")]
pub mod animated_panel;

pub mod viewport3d;
#[cfg(feature = "models")]
pub mod model_bindings;
#[cfg(feature = "models")]
pub mod model_panel;
#[cfg(feature = "models")]
pub mod blockout;
#[cfg(feature = "models")]
pub mod blockout_panel;

#[cfg(feature = "animated-models")]
pub mod yard_animation;

#[cfg(feature = "irradiance-probes")]
pub mod irradiance_bindings;
#[cfg(feature = "irradiance-probes")]
pub mod irradiance_bake;
#[cfg(feature = "irradiance-probes")]
pub mod irradiance_panel;

#[cfg(feature = "terrain")]
pub mod terrain_document;
#[cfg(feature = "terrain")]
pub mod terrain_panel;
#[cfg(feature = "terrain")]
pub mod terrain_pick;

#[cfg(feature = "terrain-physics")]
pub mod terrain_physics;

#[cfg(feature = "navigation")]
pub mod navigation_panel;

#[cfg(feature = "navigation")]
pub mod navigation_view;

#[cfg(feature = "mesh-navigation")]
pub mod mesh_navigation;
#[cfg(feature = "mesh-navigation")]
pub(crate) mod mesh_context;
#[cfg(feature = "mesh-hazards")]
pub(crate) mod mesh_hazards_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_play_controls;
#[cfg(feature = "mesh-navigation")]
mod mesh_art;
#[cfg(feature = "mesh-navigation")]
mod mesh_art_import;
#[cfg(feature = "mesh-navigation")]
mod mesh_art_import_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_surface_package_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_art_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_props;
#[cfg(feature = "mesh-navigation")]
mod mesh_prop_prefab;
#[cfg(feature = "mesh-navigation")]
mod mesh_prop_prefab_store;
#[cfg(feature = "mesh-navigation")]
mod mesh_prop_prefab_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_props_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_prop_view;
#[cfg(feature = "mesh-navigation")]
mod mesh_agent_view;
#[cfg(feature = "mesh-navigation")]
mod mesh_rotation;
#[cfg(feature = "mesh-navigation")]
mod mesh_facing;
#[cfg(feature = "mesh-navigation")]
mod mesh_camera;
#[cfg(feature = "mesh-navigation")]
mod mesh_material;
#[cfg(feature = "mesh-navigation")]
mod mesh_material_slots;
#[cfg(feature = "mesh-navigation")]
mod mesh_material_slots_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_surface;
#[cfg(feature = "mesh-navigation")]
mod mesh_camera_store;
#[cfg(feature = "mesh-navigation")]
mod mesh_camera_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_lighting;
#[cfg(feature = "mesh-navigation")]
mod mesh_lighting_store;
#[cfg(feature = "mesh-navigation")]
mod mesh_lighting_panel;
#[cfg(feature = "mesh-navigation")]
mod mesh_hud;
#[cfg(feature = "mesh-navigation")]
mod mesh_hud_store;
#[cfg(feature = "mesh-navigation")]
mod mesh_hud_panel;
#[cfg(feature = "mesh-mission")]
mod mesh_mission_panel;
#[cfg(feature = "mesh-mission")]
mod mesh_mission_overlay;
#[cfg(feature = "mesh-mission")]
mod mesh_game_creator;
#[cfg(feature = "mesh-hazards")]
mod mesh_hazards_creator;
#[cfg(feature = "mesh-navigation")]
mod mesh_animation_store;
#[cfg(feature = "mesh-animated-models")]
mod mesh_animation;
#[cfg(feature = "mesh-animated-models")]
mod mesh_animation_panel;
#[cfg(feature="mesh-navigation")]
mod mesh_scene;
#[cfg(feature="mesh-navigation")]
mod mesh_workspace;
#[cfg(feature="mesh-export")]
mod mesh_export;
#[cfg(feature="mesh-export")]
mod mesh_export_panel;
#[cfg(feature="mesh-export")]
pub mod mesh_player;
#[cfg(all(windows, target_arch = "x86_64", feature="windows-export"))]
pub mod windows_export_panel;
#[cfg(all(windows, target_arch = "x86_64", feature="windows-projects"))]
pub mod windows_project_panel;
#[cfg(feature="room-authoring")]
pub mod room_authoring_panel;
#[cfg(feature="room-authoring")]
mod room_wall_stamp;
#[cfg(all(windows,target_arch="x86_64",feature="room-authoring"))]
mod room_wall_stamp_store;
#[cfg(all(windows,target_arch="x86_64",feature="room-authoring"))]
mod room_wall_stamp_panel;
#[cfg(feature="package-manager")]
pub mod package_manager;
#[cfg(feature = "package-manager")]
mod model_package_import;
#[cfg(feature = "package-manager")]
mod model_package_import_panel;
#[cfg(feature="package-manager")]
pub mod package_manager_panel;

#[cfg(feature = "project-ui")]
pub mod project_ui;

#[cfg(feature="collect-ui")]
pub mod collect_ui_panel;

#[cfg(feature = "linked-prefabs")]
pub mod linked_prefab_panel;

#[cfg(feature="room-project")]
pub mod room_camera_panel;

#[cfg(feature = "mesh-hazards-export")]
pub mod mesh_hazards_player;
#[cfg(feature = "mesh-hazards-export")]
mod mesh_hazards_export;
#[cfg(feature = "mesh-hazards-export")]
mod mesh_hazards_export_panel;
