//! Strict-YAML scene files: parse, write, JSON Schema, bake and unbake.
//!
//! # File format (`orr.scene/1`)
//!
//! ```yaml
//! schema: orr.scene/1
//! singletons:
//!   Score: { kills: [0, 0, 0, 0, 0, 0, 0, 0] }
//! entities:
//!   e_7f3a91c2:
//!     name: Spawner_North
//!     Transform: { pos: [12.5, 0, -4], rot: 90 }
//! ```
//!
//! # Strictness (enforced by this crate, with line and column in errors)
//!
//! - No anchors, aliases, tags, merge keys (`<<`), complex keys, duplicate
//!   keys or second documents. Nesting is limited to 48 levels.
//! - No implicit types. The registry decides the type of every value:
//!   numbers are plain decimals (`-12`, `0.5`; no `+`, `.5`, `1e3`, `0x10`,
//!   `1_000`, no quotes), bools are exactly `true` and `false`, enum and flag
//!   names must match, `null` is only valid for an entity reference. A
//!   quoted number is an error, `no` is a string unless the field is a bool
//!   (and then an error).
//! - Unknown keys, unknown types, missing fields, wrong types, values outside
//!   the documented range and entity references to missing GUIDs are errors.
//!   Every problem is reported with its position; a bad file never panics.
//! - Entity keys are GUIDs: `e_` and 8 to 32 lowercase hex digits.
//!
//! # Clean diffs
//!
//! Saving sorts entities by GUID and components by name. Struct fields keep
//! their declaration order. See `encode.rs` for the layout rules.
//!
//! # Comments
//!
//! The YAML parser does not keep comments, so the loader reads them from the
//! text itself. Comment blocks directly above the header (`schema:`), above a
//! singleton, above an entity GUID key and above a component key are kept and
//! written back. Trailing comments (after a value) and comments inside a
//! component are dropped when a file is saved.

mod bake;
mod decode;
mod encode;
mod json;
#[cfg(feature = "linked-prefabs")]
mod linked;
mod node;
mod schema;

use std::collections::BTreeMap;
use std::fmt;

pub use bake::{BakeError, SceneIndex};
#[cfg(feature = "linked-prefabs")]
pub use linked::PrefabLink;
pub use node::{Diag as SceneDiagnostic, Pos as ScenePos};

use crate::registry::TypeRegistry;
use crate::value::Value;

/// The `schema:` value of the format this crate reads and writes.
pub const SCENE_SCHEMA: &str = "orr.scene/1";

/// A stable entity id in a scene file: `e_` and 8 to 32 lowercase hex digits.
/// Ordered by its text.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guid(String);

impl Guid {
    /// Checks the format.
    pub fn parse(text: &str) -> Result<Guid, String> {
        let hex = text.strip_prefix("e_").ok_or_else(|| {
            format!("'{text}' is not an entity GUID (expected e_ and 8 to 32 lowercase hex digits, like e_7f3a91c2)")
        })?;
        if hex.len() < 8 || hex.len() > 32 || !hex.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return Err(format!("'{text}' is not an entity GUID (expected e_ and 8 to 32 lowercase hex digits, like e_7f3a91c2)"));
        }
        Ok(Guid(text.to_string()))
    }

    /// The GUID `e_` + `n` as 8 hex digits.
    pub fn from_u32(n: u32) -> Guid {
        Guid(format!("e_{n:08x}"))
    }

    /// The text (`e_7f3a91c2`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One entity of a scene document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneEntity {
    /// Display name (editor only, not part of the `Frame`).
    pub name: Option<String>,
    /// Components by type name, sorted by name. Values are whole structs
    /// (or tagged values); entity references are [`Value::EntityGuid`].
    pub components: Vec<(String, Value)>,
}

/// A scene document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Editor-only, validated links from instance roots to embedded source snapshots.
    #[cfg(feature = "linked-prefabs")]
    pub prefab_links: BTreeMap<Guid, PrefabLink>,
    /// Singleton values by type name, sorted by name.
    pub singletons: Vec<(String, Value)>,
    /// Entities by GUID (sorted).
    pub entities: BTreeMap<Guid, SceneEntity>,
    /// Comment lines above the `schema:` line, without the `#`.
    pub header_comments: Vec<String>,
    /// Leading comment blocks by key (`entity:e_..`, `component:e_..:Name`,
    /// `singleton:Name`), without the `#`.
    pub comments: BTreeMap<String, Vec<String>>,
}

/// Why a scene file was rejected: one or more problems with positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneError {
    diagnostics: Vec<SceneDiagnostic>,
}

impl SceneError {
    /// All problems, in file order of discovery.
    pub fn diagnostics(&self) -> &[SceneDiagnostic] {
        &self.diagnostics
    }
    /// The first problem.
    pub fn first(&self) -> &SceneDiagnostic {
        &self.diagnostics[0]
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}:{}: {}", d.pos.line, d.pos.col, d.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for SceneError {}

impl Scene {
    /// Whether this document carries editor-only linked prefab metadata.
    /// Available in every scene build to enforce downstream feature boundaries.
    pub fn has_prefab_links(&self) -> bool {
        #[cfg(feature = "linked-prefabs")]
        { !self.prefab_links.is_empty() }
        #[cfg(not(feature = "linked-prefabs"))]
        { false }
    }

    /// Parses and validates scene text against the registry.
    ///
    /// Never panics on bad input. On success every component value is valid
    /// and every entity reference points at an entity of the scene, so
    /// [`bake`](Self::bake) can only fail if the target `Frame` lacks a type.
    pub fn parse(text: &str, registry: &TypeRegistry) -> Result<Scene, SceneError> {
        let root = node::parse(text).map_err(|d| SceneError { diagnostics: vec![d] })?;
        #[cfg(not(feature = "linked-prefabs"))]
        let mut scene = decode::decode_scene(&root, registry).map_err(|diagnostics| SceneError { diagnostics })?;
        #[cfg(feature = "linked-prefabs")]
        let mut scene = linked::decode_scene(&root, text.len(), registry).map_err(|diagnostics| SceneError { diagnostics })?;
        scene.read_comments(text, &root);
        Ok(scene)
    }

    /// Writes the scene as Strict YAML (see the module docs for the layout).
    pub fn to_yaml(&self) -> String {
        encode::write_scene(self)
    }

    /// Copies the comments of `old` for keys that this scene still has.
    /// Use it after [`unbake`](Self::unbake) to keep the comments of the file
    /// the scene was loaded from.
    pub fn carry_comments_from(&mut self, old: &Scene) {
        self.header_comments.clone_from(&old.header_comments);
        for (key, lines) in &old.comments {
            let keep = match key.split_once(':') {
                Some(("entity", g)) => Guid::parse(g).is_ok_and(|g| self.entities.contains_key(&g)),
                Some(("component", rest)) => rest.split_once(':').is_some_and(|(g, c)| {
                    Guid::parse(g).ok().and_then(|g| self.entities.get(&g)).is_some_and(|e| e.components.iter().any(|(n, _)| n == c))
                }),
                Some(("singleton", n)) => self.singletons.iter().any(|(s, _)| s == n),
                _ => false,
            };
            if keep {
                self.comments.insert(key.clone(), lines.clone());
            }
        }
    }

    /// Collects full-line comments from the text and attaches them to the keys below them.
    fn read_comments(&mut self, text: &str, root: &node::Node) {
        use node::NodeKind;
        // Line number of each key we can attach a comment to.
        let mut keys: BTreeMap<usize, String> = BTreeMap::new();
        let mut first_content_line = usize::MAX;
        if let NodeKind::Map(top) = &root.kind {
            for (k, v) in top {
                first_content_line = first_content_line.min(k.pos.line);
                let NodeKind::Map(entries) = &v.kind else { continue };
                match k.key_text() {
                    Some("singletons") => {
                        for (sk, _) in entries {
                            if let Some(n) = sk.key_text() {
                                keys.insert(sk.pos.line, format!("singleton:{n}"));
                            }
                        }
                    }
                    Some("entities") => {
                        for (ek, ev) in entries {
                            let Some(g) = ek.key_text() else { continue };
                            keys.insert(ek.pos.line, format!("entity:{g}"));
                            if let NodeKind::Map(comps) = &ev.kind {
                                for (ck, _) in comps {
                                    if let Some(c) = ck.key_text() {
                                        if c != "name" {
                                            keys.insert(ck.pos.line, format!("component:{g}:{c}"));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut pending: Vec<String> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let t = raw.trim();
            if let Some(c) = t.strip_prefix('#') {
                pending.push(c.strip_prefix(' ').unwrap_or(c).trim_end().to_string());
                continue;
            }
            if t.is_empty() {
                if line_no < first_content_line {
                    continue; // blank lines inside the header block
                }
                pending.clear();
                continue;
            }
            if line_no == first_content_line {
                self.header_comments = std::mem::take(&mut pending);
            }
            if let Some(key) = keys.get(&line_no) {
                if !pending.is_empty() {
                    self.comments.insert(key.clone(), std::mem::take(&mut pending));
                }
            }
            pending.clear();
        }
    }

    /// Creates the entities and components of the scene in `frame`. See [`bake`](bake).
    pub fn bake(&self, registry: &TypeRegistry, frame: &mut orr_ecs::Frame) -> Result<SceneIndex, BakeError> {
        bake::bake(self, registry, frame)
    }

    /// Reads a `Frame` back into a scene document. See [`bake`](bake).
    pub fn unbake(registry: &TypeRegistry, frame: &orr_ecs::Frame, index: Option<&SceneIndex>) -> Result<Scene, BakeError> {
        bake::unbake(registry, frame, index)
    }
}

impl TypeRegistry {
    /// The JSON Schema (draft 2020-12) of scene files for this registry.
    /// Output is deterministic: the same registry gives the same text.
    pub fn json_schema(&self) -> String {
        #[cfg(feature = "linked-prefabs")]
        if linked::supports_profile(self) {
            return schema::linked_scene_schema(self).pretty();
        }
        schema::scene_schema(self).pretty()
    }

    /// Legacy scene/1 schema for consumers that have not opted into linked metadata.
    /// This remains available even when another crate enables the optional parser.
    pub fn legacy_json_schema(&self) -> String {
        schema::scene_schema(self).pretty()
    }

    /// The JSON Schema of one component or singleton, standalone.
    pub fn type_json_schema(&self, name: &str) -> Option<String> {
        schema::type_schema(self, name).map(|j| j.pretty())
    }

    /// Writes [`json_schema`](Self::json_schema) to a file.
    pub fn write_json_schema(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.json_schema())
    }
}
