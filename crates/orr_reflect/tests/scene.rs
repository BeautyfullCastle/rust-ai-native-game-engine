//! Scene files: strict parsing, clean writing, bake and unbake.

mod common;

use common::*;
use orr_ecs::Entity;
use orr_fp::{FPVec2, FP};
use orr_reflect::{Guid, Scene, SceneIndex, TypeRegistry, Value};

fn parse(text: &str) -> Scene {
    Scene::parse(text, &types()).unwrap_or_else(|e| panic!("scene should parse:\n{e}"))
}

fn err(text: &str) -> orr_reflect::SceneError {
    match Scene::parse(text, &types()) {
        Ok(_) => panic!("scene should be rejected:\n{text}"),
        Err(e) => e,
    }
}

/// The text holds one `¦` marker just before the token an error must point at.
/// The first error must be at that position and mention `needle`.
fn expect_err(marked: &str, needle: &str) {
    let idx = marked.find('¦').expect("marker");
    let before = &marked[..idx];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap().chars().count() + 1;
    let text = marked.replacen('¦', "", 1);
    let e = err(&text);
    let d = e.first();
    assert!(d.message.contains(needle), "message {:?} should contain {needle:?}, file:\n{text}", d.message);
    assert_eq!((d.pos.line, d.pos.col), (line, col), "position of {:?}, file:\n{text}", d.message);
}

fn one_entity(component_line: &str) -> String {
    format!("schema: orr.scene/1\nentities:\n  e_00000001:\n    {component_line}\n")
}


#[test]
fn sample_scene_round_trips_text_exactly() {
    let scene = parse(SAMPLE);
    assert_eq!(scene.to_yaml(), SAMPLE);
    assert_eq!(scene.header_comments, vec!["A test scene.".to_string()]);
    assert_eq!(scene.comments["entity:e_00000001"], vec!["The hero.".to_string()]);
    assert_eq!(scene.comments["component:e_00000001:Blob"], vec!["Where it stands.".to_string()]);
}

#[test]
fn bake_creates_entities_in_guid_order_with_stable_ids() {
    let reg = types();
    let scene = parse(SAMPLE);
    let mut frame = frame();
    let index = scene.bake(&reg, &mut frame).unwrap();

    let hero = index.entity(&Guid::parse("e_00000001").unwrap()).unwrap();
    let second = index.entity(&Guid::parse("e_00000002").unwrap()).unwrap();
    let third = index.entity(&Guid::parse("e_00000003").unwrap()).unwrap();
    assert_eq!((hero.index, second.index, third.index), (0, 1, 2));
    assert_eq!(frame.alive_count(), 3);

    let t = frame.get::<Transform>(hero).unwrap();
    assert_eq!(t.pos, FPVec2::new(orr_fp::fp!(12.5), orr_fp::fp!(-4)));
    assert_eq!(t.rot, FP::from_int(90));
    let h = frame.get::<Health>(hero).unwrap();
    assert_eq!((h.current, h.max, h.alive, h.cache), (80, 100, 1, 0));
    let f = frame.get::<Follow>(hero).unwrap();
    assert_eq!(f.target, second);
    assert_eq!((f.mode, f.traits), (1, 1 | 4));
    assert_eq!(frame.get::<Follow>(second).unwrap().target, Entity::NONE);
    assert_eq!(index.name(&Guid::parse("e_00000002").unwrap()), Some("no"));
    assert_eq!(frame.singleton::<Stats>().scores, [1, 2, 3, 4]);
    assert!(frame.get::<Transform>(third).is_none());
}

#[test]
fn unbake_writes_the_same_file_back() {
    let reg = types();
    let scene = parse(SAMPLE);
    let mut frame = frame();
    let index = scene.bake(&reg, &mut frame).unwrap();
    let mut back = Scene::unbake(&reg, &frame, Some(&index)).unwrap();
    back.carry_comments_from(&scene);
    assert_eq!(back.to_yaml(), SAMPLE);
}

#[test]
fn unbake_without_an_index_makes_stable_guids() {
    let reg = types();
    let scene = parse(SAMPLE);
    let mut frame = frame();
    scene.bake(&reg, &mut frame).unwrap();
    let a = Scene::unbake(&reg, &frame, None).unwrap().to_yaml();
    let b = Scene::unbake(&reg, &frame, None).unwrap().to_yaml();
    assert_eq!(a, b);
    // The generated file loads back into an identical frame.
    let again = parse(&a);
    let mut frame2 = frame_clone_empty();
    again.bake(&reg, &mut frame2).unwrap();
    assert_eq!(frame.checksum(), frame2.checksum());
}

fn frame_clone_empty() -> orr_ecs::Frame {
    frame()
}

#[test]
fn bake_checksum_is_stable() {
    let reg = types();
    let mut f1 = frame();
    parse(SAMPLE).bake(&reg, &mut f1).unwrap();
    let mut f2 = frame();
    parse(SAMPLE).bake(&reg, &mut f2).unwrap();
    assert_eq!(f1.checksum(), f2.checksum());
    // Golden: the frame checksum of SAMPLE. Changes only if the format of a
    // component, the checksum format, the bake order or entity allocation
    // changes on purpose.
    assert_eq!(f1.checksum(), GOLDEN_SAMPLE_CHECKSUM, "checksum is {}", f1.checksum());
}

// ORRF v2 hashes the complete frame body. SAMPLE and its bake order are
// unchanged; the incomplete v1 checksum was 15635741705100430814.
const GOLDEN_SAMPLE_CHECKSUM: u64 = 14713264168317327511;

#[test]
fn entity_order_in_the_file_does_not_change_the_frame() {
    let reg = types();
    let a = "schema: orr.scene/1\nentities:\n  e_00000001:\n    Health: { current: 1, max: 5, alive: true }\n  e_00000002:\n    Health: { current: 2, max: 5, alive: true }\n";
    let b = "schema: orr.scene/1\nentities:\n  e_00000002:\n    Health: { current: 2, max: 5, alive: true }\n  e_00000001:\n    Health: { current: 1, max: 5, alive: true }\n";
    let (mut fa, mut fb) = (frame(), frame());
    parse(a).bake(&reg, &mut fa).unwrap();
    parse(b).bake(&reg, &mut fb).unwrap();
    assert_eq!(fa.checksum(), fb.checksum());
    assert_eq!(parse(a).to_yaml(), parse(b).to_yaml());
}

#[test]
fn saving_sorts_keys_and_drops_nothing_else() {
    let messy = "schema: orr.scene/1\nentities:\n  e_0000000b:\n    Transform: { rot: 1, pos: [1, 2] }\n    Health: {alive: false, current: 3, max: 4}\n  e_0000000a: {}\n";
    let out = parse(messy).to_yaml();
    let a = out.find("e_0000000a").unwrap();
    let b = out.find("e_0000000b").unwrap();
    assert!(a < b);
    assert!(out.find("Health").unwrap() < out.find("Transform").unwrap());
    assert!(out.contains("Transform: { pos: [1, 2], rot: 1 }"), "{out}");
    assert!(out.contains("Health: { current: 3, max: 4, alive: false }"), "{out}");
    // Writing again changes nothing.
    assert_eq!(parse(&out).to_yaml(), out);
}

#[test]
fn long_values_use_block_style_and_still_round_trip() {
    let reg = types();
    let text = "schema: orr.scene/1\nsingletons:\n  Stats: { scores: [4000000000, 4000000001, 4000000002, 4000000003], gravity: [-123456.0000152587890625, 123456.0000152587890625], weight: -32768 }\nentities: {}\n";
    let scene = Scene::parse(text, &reg).unwrap();
    let out = scene.to_yaml();
    assert!(out.contains("Stats:\n"), "{out}");
    let again = Scene::parse(&out, &reg).unwrap();
    assert_eq!(again.singletons, scene.singletons);
    assert_eq!(again.to_yaml(), out);
}

// ---- strictness ----

#[test]
fn anchors_aliases_tags_and_merge_keys_are_rejected() {
    expect_err(&one_entity("Health: &h ¦{ current: 1, max: 2, alive: true }"), "anchors");
    expect_err(&one_entity("Health: { current: 1, max: 2, alive: true }\n    Transform: &t ¦{ pos: [0, 0], rot: 0 }"), "anchors");
    expect_err(&one_entity("Health: !!map ¦{ current: 1, max: 2, alive: true }"), "tags");
    expect_err(&one_entity("Health: { current: !!int ¦1, max: 2, alive: true }"), "tags");
    expect_err(&one_entity("¦<<: { name: x }"), "merge keys");
    expect_err("schema: orr.scene/1\nentities: {}\n¦---\nschema: orr.scene/1\n", "one YAML document");
}

#[test]
fn aliases_are_rejected() {
    let e = err("schema: orr.scene/1\nentities:\n  e_00000001:\n    Health: *h\n");
    assert!(e.first().message.contains("alias") || e.first().message.contains("anchor"), "{}", e.first().message);
    assert_eq!(e.first().pos.line, 4);
    // With a defined anchor the anchor itself is the first problem.
    let e = err("schema: orr.scene/1\nentities:\n  e_00000001:\n    Health: &h { current: 1, max: 2, alive: true }\n  e_00000002:\n    Health: *h\n");
    assert!(e.first().message.contains("anchors"));
}

#[test]
fn duplicate_keys_are_rejected_with_the_first_position() {
    expect_err(
        "schema: orr.scene/1\nentities:\n  e_00000001: {}\n  ¦e_00000001: {}\n",
        "duplicate key 'e_00000001' (first used at line 3)",
    );
    expect_err(&one_entity("Health: { current: 1, max: 2, alive: true, ¦max: 3 }"), "duplicate key 'max'");
}

#[test]
fn types_come_from_the_schema_not_from_the_text() {
    // `no` is a string for a string field ...
    let scene = parse("schema: orr.scene/1\nentities:\n  e_00000001:\n    name: no\n");
    assert_eq!(scene.entities.values().next().unwrap().name.as_deref(), Some("no"));
    // ... and an error for a bool field.
    for bad in ["no", "yes", "on", "off", "True", "1", "0", "y", "\"true\"", "null", "~"] {
        expect_err(&one_entity(&format!("Health: {{ current: 1, max: 2, alive: ¦{bad} }}")), "true or false");
    }
    // Numbers: only plain decimals.
    for bad in ["\"1\"", "1.5", "+1", "0x10", "1e3", "1_0", "1 0", "null", "true", "[1]", "{}", "~"] {
        let e = err(&one_entity(&format!("Health: {{ current: {bad}, max: 2, alive: true }}")));
        assert_eq!(e.first().pos, orr_reflect::ScenePos { line: 4, col: 24 }, "{bad}: {}", e.first().message);
    }
    for bad in ["\"1.5\"", "+1", ".5", "5.", "1e3", "0x10", "1_000", "true", "yes", "~", "nan", ".inf"] {
        let e = err(&one_entity(&format!("Transform: {{ pos: [0, 0], rot: {bad} }}")));
        assert_eq!(e.first().pos, orr_reflect::ScenePos { line: 4, col: 36 }, "{bad}: {}", e.first().message);
    }
    // Entity refs: null only unquoted.
    expect_err(&one_entity("Follow: { target: ¦\"null\", mode: idle, traits: [] }"), "GUID");
    // A quoted number is not a number.
    expect_err(&one_entity("Transform: { pos: [0, 0], rot: ¦\"1.5\" }"), "without quotes");
}

#[test]
fn schema_level_errors_have_positions() {
    expect_err("¦entities: {}\n", "missing 'schema");
    expect_err("schema: ¦orr.scene/2\nentities: {}\n", "unsupported schema 'orr.scene/2'");
    expect_err("¦schema: orr.scene/1\n", "missing 'entities'");
    expect_err("schema: orr.scene/1\nentities: {}\n¦extra: 1\n", "unknown top-level key 'extra'");
    expect_err("schema: orr.scene/1\nentities:\n  ¦e_1: {}\n", "not an entity GUID");
    expect_err("schema: orr.scene/1\nentities:\n  ¦E_00000001: {}\n", "not an entity GUID");
    expect_err("schema: orr.scene/1\nentities:\n  ¦e_0000000G: {}\n", "not an entity GUID");
    expect_err(&one_entity("¦Transfrom: { pos: [0, 0], rot: 0 }"), "unknown component 'Transfrom'");
    expect_err(&one_entity("¦Stats: { scores: [1, 2, 3, 4], gravity: [0, 0], weight: 0 }"), "singleton");
    expect_err(&one_entity("Health: ¦{ current: 1, max: 2 }"), "missing field 'alive'");
    expect_err(&one_entity("Health: { current: 1, max: 2, alive: true, ¦mana: 3 }"), "unknown field 'mana'");
    expect_err(&one_entity("Health: ¦5"), "expected a map");
    expect_err(&one_entity("Health¦:"), "expected a map");
    expect_err(&one_entity("Transform: { pos: ¦[1], rot: 0 }"), "list of 2 numbers");
    expect_err(&one_entity("Transform: { pos: ¦[1, 2, 3], rot: 0 }"), "list of 2 numbers");
    expect_err(&one_entity("Follow: { target: ¦e_00000009, mode: idle, traits: [] }"), "no entity with GUID 'e_00000009'");
    expect_err(&one_entity("Follow: { target: null, mode: ¦sleepy, traits: [] }"), "not one of idle, chase, flee");
    expect_err(&one_entity("Follow: { target: null, mode: idle, traits: [fast, ¦fast] }"), "listed twice");
    expect_err(&one_entity("Follow: { target: null, mode: idle, traits: [¦swim] }"), "not one of fast, armored, flying");
    expect_err(&one_entity("name:\n      ¦- a"), "'name' must be text");
}

#[test]
fn ranges_are_checked_where_the_value_is() {
    expect_err(&one_entity("Health: { current: 1, max: ¦0, alive: true }"), "outside [1, 1000]");
    expect_err(&one_entity("Health: { current: 1, max: ¦1001, alive: true }"), "outside [1, 1000]");
    expect_err(&one_entity("Health: { current: ¦3000000000, max: 5, alive: true }"), "does not fit i32");
    expect_err(&one_entity("Transform: { pos: [0, ¦1000.5], rot: 0 }"), "outside [-1000, 1000]");
    expect_err(&one_entity("Transform: { pos: [0, 0], rot: ¦140737488355328 }"), "too large for fixed point");
    parse(&one_entity("Transform: { pos: [-1000, 1000], rot: 0 }"));
}

#[test]
fn tagged_values_are_validated_by_their_type() {
    parse(&one_entity("Blob: { kind: dot, radius: 0.5 }"));
    expect_err(&one_entity("Blob: { kind: dot, radius: ¦0.01 }"), "outside [0.05, 100]");
    expect_err(&one_entity("Blob: { kind: ¦hexagon }"), "not one of dot, box, poly");
    expect_err(&one_entity("Blob: ¦{ radius: 1 }"), "missing 'kind'");
    expect_err(&one_entity("Blob: { kind: dot, ¦half: [1, 1] }"), "unknown field 'half'");
    expect_err(&one_entity("Blob: { kind: poly, pts: ¦[[0, 0], [1, 0]] }"), "3 to 4 values");
    // Clockwise: caught by the type's own writer, reported at the field.
    expect_err(&one_entity("Blob: { kind: poly, pts: ¦[[0, 0], [0, 1], [1, 0]] }"), "polygon must be convex and counter-clockwise");
    parse(&one_entity("Blob: { kind: poly, pts: [[0, 0], [1, 0], [0, 1]] }"));
}

#[test]
fn syntax_errors_have_positions_and_bad_input_never_panics() {
    let e = err("schema: orr.scene/1\nentities: [\n");
    assert!(e.first().pos.line >= 2);
    let e = err("");
    assert_eq!(e.first().pos.line, 1);
    let e = err("just text\n");
    assert!(e.first().message.contains("map"));
    let e = err("- a\n- b\n");
    assert!(e.first().message.contains("map"));
    let deep = format!("schema: orr.scene/1\nentities: {}\n", "[".repeat(2000));
    assert!(err(&deep).first().message.contains("deeper") || err(&deep).first().message.contains("YAML"));
    let deep = format!("schema: orr.scene/1\nx: {}{}\n", "[".repeat(200), "]".repeat(200));
    assert!(err(&deep).first().message.contains("deeper"));
}

#[test]
fn several_problems_are_all_reported() {
    let text = "schema: orr.scene/1\nentities:\n  e_00000001:\n    Health: { current: 1, max: 0, alive: true }\n    Transform: { pos: [0, 0], rot: x }\n";
    let e = err(text);
    assert_eq!(e.diagnostics().len(), 2);
    let shown = e.to_string();
    assert!(shown.contains("4:32:") && shown.contains("5:36:"), "{shown}");
}

#[test]
fn bake_rejects_types_missing_from_the_frame() {
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    let reg = types();
    let scene = parse(SAMPLE);
    let mut b = ComponentRegistryBuilder::new();
    b.register_component::<Transform>("Transform");
    let mut frame = Frame::new(b.build());
    let e = scene.bake(&reg, &mut frame).unwrap_err();
    assert!(e.to_string().contains("not registered in the frame"), "{e}");
    assert_eq!(frame.alive_count(), 0, "a failed bake leaves the frame untouched");
    assert!(!reg.check_against(frame.registry()).is_empty());
    assert!(reg.check_against(&ecs_registry()).is_empty());
}

#[test]
fn singleton_init_hook_runs_for_every_singleton() {
    use bytemuck::{Pod, Zeroable};
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable, orr_reflect::Reflect)]
    struct Clock {
        rate: u32,
        #[reflect(skip)]
        ready: u32,
    }
    let mut reg = TypeRegistry::new();
    reg.register_singleton_with_init::<Clock>("Clock", |frame, present| {
        let c = frame.singleton_mut::<Clock>();
        if !present {
            c.rate = 60;
        }
        c.ready = 1;
    });
    let mut b = ComponentRegistryBuilder::new();
    b.register_singleton::<Clock>("Clock");
    let mut frame = Frame::new(b.build());
    let scene = Scene::parse("schema: orr.scene/1\nentities: {}\n", &reg).unwrap();
    scene.bake(&reg, &mut frame).unwrap();
    assert_eq!((frame.singleton::<Clock>().rate, frame.singleton::<Clock>().ready), (60, 1));

    let mut frame = Frame::new({
        let mut b = ComponentRegistryBuilder::new();
        b.register_singleton::<Clock>("Clock");
        b.build()
    });
    let scene = Scene::parse("schema: orr.scene/1\nsingletons:\n  Clock: { rate: 30 }\nentities: {}\n", &reg).unwrap();
    scene.bake(&reg, &mut frame).unwrap();
    assert_eq!((frame.singleton::<Clock>().rate, frame.singleton::<Clock>().ready), (30, 1));
}

#[test]
fn index_maps_both_ways_and_is_ordered() {
    let reg = types();
    let mut frame = frame();
    let index: SceneIndex = parse(SAMPLE).bake(&reg, &mut frame).unwrap();
    let guids: Vec<&str> = index.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(guids, ["e_00000001", "e_00000002", "e_00000003"]);
    for (g, e) in index.iter() {
        assert_eq!(index.guid(e), Some(g));
    }
    let v = reg.read_component(&frame, index.entity(&Guid::parse("e_00000001").unwrap()).unwrap(), "Follow").unwrap();
    assert!(matches!(v.field("target"), Some(Value::Entity(e)) if e.index == 1));
}
