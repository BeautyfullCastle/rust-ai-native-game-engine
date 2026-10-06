//! `orr_render`: the 2D renderer of the view layer (design doc 4.2), built on
//! [`orr_rhi`].
//!
//! Flow each frame: the view world (`orr_view`) produces `RenderItem`s;
//! [`extract::extract_items`] turns them into a [`RenderList`] (plus debug
//! lines and shapes the caller adds); a [`Renderer`] draws the list through a
//! [`Camera`] into a texture view. Two targets are provided:
//!
//! - [`WindowRenderer`]: a window surface (the samples).
//! - [`OffscreenTarget`]: a texture (the editor viewport: show
//!   [`OffscreenTarget::sample_view`] in egui).
//!
//! 3D: [`Renderer3D`] draws a [`RenderList3D`] (instanced sphere, box, capsule
//! and plane meshes with a [`Material`] each, 3D debug lines, one shadow
//! casting sun plus hemisphere ambient) through a [`Camera3D`] with a depth
//! buffer and MSAA. See `renderer3d.rs` for the frame and the shading model.
//!
//! Drawn in 2D: filled circles, boxes and capsules (instanced, distance-field
//! edges), and constant-pixel-width lines for gizmos. Text is not drawn yet
//! (use the UI toolkit's text for overlays).
//!
//! This is view layer code: it may use floats. Sim crates never depend on it.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod camera;
pub mod camera3d;
pub mod extract;
pub mod list;
pub mod list3d;
pub mod math3;
pub mod mesh;
pub mod renderer;
pub mod renderer3d;
mod stats;
pub mod targets;
pub mod text;

pub use camera::Camera;
pub use camera3d::{Camera3D, OrbitCamera, Projection};
pub use list3d::{Instance3D, Lighting, LineInstance3D, Material, RenderList3D, IDENTITY_ROT};
pub use mesh::{MeshKind, MeshSet, Vertex3, DEFAULT_SEGMENTS};
pub use renderer3d::{light_view_proj, Renderer3D, Settings3D, SphereLod3D, SphereLodError3D, SphereLodStats3D, DEFAULT_CLEAR_3D};
pub use extract::{extract_items, instance_of};
pub use list::{LineInstance, RenderList, ShapeInstance};
pub use renderer::{Renderer, DEFAULT_CLEAR};
pub use stats::{FrameStats, PassStats};
pub use targets::{OffscreenTarget, WindowRenderer, WindowRenderer3D};

pub use orr_rhi;

#[cfg(feature = "sprites")]
pub mod sprite_list;
#[cfg(feature = "sprites")]
pub mod sprite_renderer;
#[cfg(feature = "sprites")]
pub use sprite_list::{SpriteDrawList, SpriteInstance};
#[cfg(feature = "sprites")]
pub use sprite_renderer::{SpriteRenderError, SpriteRenderer};

#[cfg(feature = "models")]
pub mod model_renderer;
#[cfg(feature = "models")]
pub use model_renderer::{ModelRenderer, ModelRenderError, StaticInstance, StaticInstanceError};

#[cfg(feature = "animation")]
pub mod skinned;
#[cfg(feature = "animation")]
pub use skinned::{SkinnedBounds, SkinnedInstance, SkinnedModelRenderer, SkinnedRenderError};

#[cfg(feature = "imported-scene")]
pub mod point_light;
#[cfg(feature = "imported-scene")]
pub mod imported_scene;
#[cfg(feature = "imported-scene")]
pub use point_light::{PointLight, PointLightError, PointLightSettings};
#[cfg(feature = "imported-scene")]
pub use imported_scene::{ImportedBatch, ImportedSceneError, ImportedSceneRenderer, ImportedSceneTarget};

#[cfg(feature = "imported-scene")]
pub use renderer3d::ProceduralSceneError;

#[cfg(feature = "imported-scene")]
mod shared_shadow;
#[cfg(feature = "imported-scene")]
pub use shared_shadow::{MAX_IMPORTED_CASTERS, SHARED_SHADOW_MAP_SIZE};
