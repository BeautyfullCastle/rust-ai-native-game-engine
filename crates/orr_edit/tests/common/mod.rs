#![allow(dead_code)]

use orr_edit::{EditorDoc, Target};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Guid, TypeRegistry, Value};
use orr_sample::physics_game::{register_reflect, PhysGame};
use orr_sim::Simulation;

pub const DEMO_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml");
pub const SEED: u64 = 7;

pub fn types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    register_reflect(&mut t);
    t
}

pub fn demo_text() -> String {
    std::fs::read_to_string(DEMO_PATH).expect("scenes/physics_demo.scene.yaml")
}

pub fn demo_doc() -> EditorDoc {
    EditorDoc::from_yaml(&demo_text(), types(), Simulation::<PhysGame>::build_registry(), SEED).expect("load demo scene")
}

pub fn empty_doc() -> EditorDoc {
    EditorDoc::for_game::<PhysGame>(types(), SEED).expect("empty doc")
}

/// GUID of the entity with this display name.
pub fn guid_named(doc: &EditorDoc, name: &str) -> Guid {
    doc.view()
        .entities()
        .into_iter()
        .find(|e| e.name.as_deref() == Some(name))
        .and_then(|e| e.guid)
        .unwrap_or_else(|| panic!("no entity named {name}"))
}

pub fn target_named(doc: &EditorDoc, name: &str) -> Target {
    Target::Guid(guid_named(doc, name))
}

pub fn fixed(n: i32) -> Value {
    Value::Fixed(FP::from_int(n))
}

pub fn vec2(x: i32, y: i32) -> Value {
    Value::Vec2(FPVec2::new(FP::from_int(x), FP::from_int(y)))
}

/// The state that must come back exactly after an undo.
pub fn state(doc: &EditorDoc) -> (String, u64) {
    (doc.to_yaml(), doc.checksum())
}

/// The preview frame equals a from-scratch bake of the scene text.
pub fn assert_synced(doc: &EditorDoc) {
    let text = doc.scene().to_yaml();
    let fresh = EditorDoc::from_yaml(&text, types(), Simulation::<PhysGame>::build_registry(), SEED).unwrap();
    assert_eq!(fresh.checksum(), doc.checksum(), "preview frame differs from a fresh bake");
}
