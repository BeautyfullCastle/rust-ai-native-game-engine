//! `orr_reflect`: a type registry, JSON Schema output and Strict-YAML scene
//! files for Orrery components.
//!
//! # Why
//!
//! The editor must show and change any component without knowing its Rust
//! type, and scene files must load into a `Frame` the same way on every
//! machine. This crate describes each `Pod` type field by field. It reads and
//! writes the raw component bytes through one dynamic [`Value`] type.
//!
//! # Layers
//!
//! - **Descriptors** ([`TypeDesc`], [`Reflect`], `#[derive(Reflect)]`): what a
//!   type looks like and how to read and write it as bytes.
//! - **Registry** ([`TypeRegistry`]): names, listings and type-erased access to
//!   a `Frame` (add and remove components, get and set a field by path).
//! - **Scene files** (feature `scene`, on by default): Strict YAML in and out
//!   ([`Scene`]), JSON Schema ([`TypeRegistry::json_schema`]), and baking a scene
//!   into a `Frame` and back ([`Scene::bake`], [`Scene::unbake`]).
//!
//! Simulation crates depend on this crate with `default-features = false`.
//! That gives them the derive macro and the descriptors only. There is no
//! runtime cost in the simulation.
//!
//! # Field attributes of the derive
//!
//! See `orr_reflect_derive`: `skip`, `bool`, `enumeration = "a=0,b=1"`,
//! `flags = "sensor=1"`, `range = "0.05..=1000"`. Doc comments become
//! descriptions.
//!
//! # Numbers
//!
//! Fixed-point fields are decimal numbers in files. They are parsed with
//! `orr_fp`'s integer parser (never through `f64`) and written as the
//! shortest decimal text that reads back to exactly the same raw value. See
//! [`decimal`]. Decimal text with more precision than `1/65536` is rounded
//! to the nearest step when read.
//!
//! Nothing in the fixed-point path uses floating point. Every ordered
//! container is a `BTreeMap` or a sorted `Vec`, so output never depends on
//! hash order.

extern crate self as orr_reflect;

#[doc(hidden)]
pub use bytemuck;

pub mod decimal;
mod desc;
mod reflect;
mod registry;
mod value;

#[cfg(feature = "scene")]
mod scene;

pub use desc::{
    range_text, FieldDesc, IntKind, Kind, Range, TaggedDesc, TypeDesc, VariantDesc, ViewField,
};
pub use orr_reflect_derive::Reflect;
pub use reflect::Reflect;
pub use registry::{SingletonInit, TypeInfo, TypeKind, TypeRegistry};
pub use value::{parse_path, PathSeg, ReflectError, Value};

#[cfg(feature = "scene")]
pub use scene::{BakeError, Guid, Scene, SceneDiagnostic, SceneEntity, SceneError, SceneIndex, ScenePos, SCENE_SCHEMA};
