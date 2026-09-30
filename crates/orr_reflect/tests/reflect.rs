//! The registry API the editor inspector uses: list, get, set, add, remove.

mod common;

use common::*;
use orr_ecs::Entity;
use orr_fp::{fp, FPVec2, FP};
use orr_reflect::{Kind, ReflectError, TypeKind, Value};

#[test]
fn types_are_listed_sorted_with_kind_and_fields() {
    let reg = types();
    let names: Vec<&str> = reg.types().map(|t| t.name()).collect();
    assert_eq!(names, ["Blob", "Follow", "Health", "Stats", "Transform"]);
    let comps: Vec<&str> = reg.components().map(|t| t.name()).collect();
    assert_eq!(comps, ["Blob", "Follow", "Health", "Transform"]);
    assert_eq!(reg.singletons().map(|t| t.name()).collect::<Vec<_>>(), ["Stats"]);
    assert_eq!(reg.get("Stats").unwrap().kind(), TypeKind::Singleton);

    let health = reg.get("Health").unwrap();
    // The hidden `cache` field is not listed.
    let fields: Vec<&str> = health.fields().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(fields, ["current", "max", "alive"]);
    assert_eq!(health.doc(), "Hit points.");
    assert!(matches!(health.fields()[2].ty.kind, Kind::Bool { width: 4 }));
    let t = reg.get("Transform").unwrap();
    assert_eq!(t.fields()[0].doc, "World position.");
    assert_eq!(t.fields()[1].doc, "Heading in degrees.");
    assert_eq!(t.size(), 24);
    // A tagged view has no fixed field list.
    assert!(reg.get("Blob").unwrap().fields().is_empty());
    assert!(reg.get("Nope").is_none());
}

#[test]
fn get_and_set_fields_by_path_on_a_frame() {
    let reg = types();
    let mut frame = frame();
    let e = frame.spawn();
    reg.add_component(&mut frame, e, "Transform").unwrap();
    reg.add_component(&mut frame, e, "Health").unwrap();
    assert_eq!(reg.component_names(&frame, e), ["Health", "Transform"]);

    // Whole value, sub-value, component of a vector.
    reg.set_field(&mut frame, e, "Transform", "pos", Value::Vec2(FPVec2::new(fp!(1.5), fp!(-2)))).unwrap();
    assert_eq!(reg.get_field(&frame, e, "Transform", "pos.x").unwrap(), Value::Fixed(fp!(1.5)));
    assert_eq!(reg.get_field(&frame, e, "Transform", "pos.y").unwrap(), Value::Fixed(fp!(-2)));
    reg.set_field(&mut frame, e, "Transform", "pos.y", Value::Fixed(fp!(7.25))).unwrap();
    assert_eq!(frame.get::<Transform>(e).unwrap().pos, FPVec2::new(fp!(1.5), fp!(7.25)));
    reg.set_field(&mut frame, e, "Transform", "rot", Value::Fixed(fp!(45))).unwrap();
    assert_eq!(frame.get::<Transform>(e).unwrap().rot, fp!(45));

    // Integers and bools.
    reg.set_field(&mut frame, e, "Health", "current", Value::Int(-5)).unwrap();
    reg.set_field(&mut frame, e, "Health", "alive", Value::Bool(false)).unwrap();
    let h = *frame.get::<Health>(e).unwrap();
    assert_eq!((h.current, h.alive, h.max), (-5, 0, 100));

    // Whole component.
    let whole = reg.read_component(&frame, e, "Health").unwrap();
    assert_eq!(
        whole,
        Value::Struct(vec![
            ("current".into(), Value::Int(-5)),
            ("max".into(), Value::Int(100)),
            ("alive".into(), Value::Bool(false)),
        ])
    );
}

#[test]
fn set_rejects_wrong_types_ranges_and_paths_without_changing_anything() {
    let reg = types();
    let mut frame = frame();
    let e = frame.spawn();
    reg.add_component(&mut frame, e, "Health").unwrap();
    reg.add_component(&mut frame, e, "Transform").unwrap();
    let before = frame.checksum();

    let cases: Vec<(&str, &str, Value, &str)> = vec![
        ("Health", "max", Value::Int(0), "outside [1, 1000]"),
        ("Health", "max", Value::Int(1001), "outside [1, 1000]"),
        ("Health", "current", Value::Int(1 << 40), "does not fit i32"),
        ("Health", "current", Value::Fixed(fp!(1)), "expected integer"),
        ("Health", "alive", Value::Int(1), "expected bool"),
        ("Health", "mana", Value::Int(1), "no field 'mana'"),
        ("Health", "current.x", Value::Int(1), "cannot look up"),
        ("Health", "current[0]", Value::Int(1), "cannot look up"),
        ("Transform", "pos.z", Value::Fixed(fp!(1)), "a vec2 has fields"),
        ("Transform", "pos.x", Value::Fixed(fp!(1000.5)), "outside [-1000, 1000]"),
        ("Transform", "pos", Value::Vec2(FPVec2::new(fp!(0), fp!(-1001))), "y:"),
        ("Transform", "pos.x", Value::Int(1), "expected"),
        ("Transform", "", Value::Int(1), "expected struct"),
        ("Transform", "pos..x", Value::Fixed(fp!(1)), "bad field path"),
        ("Transform", "pos.", Value::Fixed(fp!(1)), "bad field path"),
        ("Transform", "[0]", Value::Fixed(fp!(1)), "expected a field name"),
    ];
    for (comp, path, val, needle) in cases {
        let err = reg.set_field(&mut frame, e, comp, path, val.clone()).expect_err(&format!("{comp}.{path} = {val:?}"));
        assert!(err.to_string().contains(needle), "{comp}.{path}: {err}");
    }
    assert_eq!(frame.checksum(), before);

    assert_eq!(reg.get_field(&frame, e, "Follow", "mode"), Err(ReflectError::NoComponent("Follow".into())));
    assert_eq!(reg.get_field(&frame, e, "Nope", "x"), Err(ReflectError::UnknownType("Nope".into())));
    assert_eq!(reg.get_field(&frame, e, "Stats", "gravity"), Err(ReflectError::WrongTypeKind("Stats".into())));
    assert_eq!(reg.get_field(&frame, Entity { index: 9, version: 0 }, "Health", "max"), Err(ReflectError::NoEntity));
}

#[test]
fn arrays_enums_flags_and_entity_refs() {
    let reg = types();
    let mut frame = frame();
    let a = frame.spawn();
    let b = frame.spawn();
    reg.add_component(&mut frame, a, "Follow").unwrap();
    reg.set_field(&mut frame, a, "Follow", "target", Value::Entity(b)).unwrap();
    reg.set_field(&mut frame, a, "Follow", "mode", Value::Enum("flee".into())).unwrap();
    reg.set_field(&mut frame, a, "Follow", "traits", Value::Flags(vec!["fast".into(), "flying".into()])).unwrap();
    let f = frame.get::<Follow>(a).unwrap();
    assert_eq!((f.target, f.mode, f.traits), (b, 2, 5));
    assert_eq!(reg.get_field(&frame, a, "Follow", "traits").unwrap(), Value::Flags(vec!["fast".into(), "flying".into()]));
    assert!(reg.set_field(&mut frame, a, "Follow", "mode", Value::Enum("dance".into())).is_err());
    assert!(reg.set_field(&mut frame, a, "Follow", "traits", Value::Flags(vec!["swim".into()])).is_err());
    // An unknown stored value reads as a name that no write accepts.
    frame.get_mut::<Follow>(a).unwrap().mode = 9;
    assert_eq!(reg.get_field(&frame, a, "Follow", "mode").unwrap(), Value::Enum("unknown(9)".into()));

    reg.set_singleton_field(&mut frame, "Stats", "scores[2]", Value::Int(77)).unwrap();
    assert_eq!(frame.singleton::<Stats>().scores, [0, 0, 77, 0]);
    assert_eq!(reg.get_singleton_field(&frame, "Stats", "scores[2]").unwrap(), Value::Int(77));
    assert!(reg.set_singleton_field(&mut frame, "Stats", "scores[4]", Value::Int(1)).is_err());
    assert!(reg.set_singleton_field(&mut frame, "Stats", "scores[x]", Value::Int(1)).is_err());
    reg.set_singleton_field(&mut frame, "Stats", "weight", Value::Fixed32(orr_fp::FP32::from_raw(98304))).unwrap();
    assert_eq!(frame.singleton::<Stats>().weight.raw(), 98304);
    let all = reg.read_singleton(&frame, "Stats").unwrap();
    assert!(matches!(all.field("scores"), Some(Value::Array(v)) if v.len() == 4));
}

#[test]
fn add_and_remove_components() {
    let reg = types();
    let mut frame = frame();
    let e = frame.spawn();
    assert!(!reg.has_component(&frame, e, "Health").unwrap());
    reg.add_component(&mut frame, e, "Health").unwrap();
    // The default value comes from the type: max 100, alive, hidden field 0.
    assert_eq!(*frame.get::<Health>(e).unwrap(), Health { current: 100, max: 100, alive: 1, cache: 0 });
    assert!(reg.add_component(&mut frame, e, "Health").is_err(), "adding twice is an error");
    reg.add_component(&mut frame, e, "Follow").unwrap();
    assert_eq!(frame.get::<Follow>(e).unwrap().target, Entity::NONE, "zeroed struct with default entity");
    assert!(reg.remove_component(&mut frame, e, "Health").unwrap());
    assert!(!reg.remove_component(&mut frame, e, "Health").unwrap());
    assert!(!frame.has::<Health>(e));
    assert_eq!(reg.entities_with(&frame, "Follow").unwrap(), vec![e]);
    assert_eq!(reg.add_component(&mut frame, Entity { index: 5, version: 0 }, "Health"), Err(ReflectError::NoEntity));

    let v = Value::Struct(vec![
        ("current".into(), Value::Int(3)),
        ("max".into(), Value::Int(9)),
        ("alive".into(), Value::Bool(true)),
    ]);
    reg.add_component_value(&mut frame, e, "Health", &v).unwrap();
    assert_eq!(frame.get::<Health>(e).unwrap().max, 9);
    let bad = Value::Struct(vec![("current".into(), Value::Int(3))]);
    assert!(reg.add_component_value(&mut frame, e, "Health", &bad).unwrap_err().to_string().contains("missing field"));
}

#[test]
fn tagged_view_reads_writes_and_switches_variants() {
    let reg = types();
    let mut frame = frame();
    let e = frame.spawn();
    reg.add_component(&mut frame, e, "Blob").unwrap();
    assert_eq!(reg.read_component(&frame, e, "Blob").unwrap(), Value::Variant("dot".into(), vec![("radius".into(), Value::Fixed(FP::ONE))]));
    assert_eq!(reg.get_field(&frame, e, "Blob", "kind").unwrap(), Value::Enum("dot".into()));
    reg.set_field(&mut frame, e, "Blob", "radius", Value::Fixed(fp!(2.5))).unwrap();
    assert_eq!(frame.get::<Blob>(e).unwrap().a, fp!(2.5));
    assert!(reg.set_field(&mut frame, e, "Blob", "radius", Value::Fixed(fp!(500))).is_err());
    assert!(reg.set_field(&mut frame, e, "Blob", "half", Value::Vec2(FPVec2::ZERO)).unwrap_err().to_string().contains("no field 'half'"));

    // Switching the variant fills the new fields with the variant defaults.
    reg.set_field(&mut frame, e, "Blob", "kind", Value::Enum("box".into())).unwrap();
    assert_eq!(frame.get::<Blob>(e).unwrap().kind, 1);
    assert_eq!(reg.get_field(&frame, e, "Blob", "half.x").unwrap(), Value::Fixed(FP::ONE));
    reg.set_field(&mut frame, e, "Blob", "half.y", Value::Fixed(fp!(3))).unwrap();
    assert_eq!(frame.get::<Blob>(e).unwrap().pts[0], FPVec2::new(FP::ONE, fp!(3)));
    assert!(reg.set_field(&mut frame, e, "Blob", "kind", Value::Enum("cube".into())).is_err());

    reg.set_field(&mut frame, e, "Blob", "kind", Value::Enum("poly".into())).unwrap();
    assert_eq!(frame.get::<Blob>(e).unwrap().count, 3);
    reg.set_field(&mut frame, e, "Blob", "pts[1].x", Value::Fixed(fp!(2))).unwrap();
    assert_eq!(reg.get_field(&frame, e, "Blob", "pts[1]").unwrap(), Value::Vec2(FPVec2::new(fp!(2), fp!(0))));
    // A change that makes the polygon invalid is refused by the type and nothing changes.
    let before = *frame.get::<Blob>(e).unwrap();
    let err = reg.set_field(&mut frame, e, "Blob", "pts[2]", Value::Vec2(FPVec2::new(fp!(3), fp!(-5)))).unwrap_err();
    assert!(err.to_string().contains("convex"), "{err}");
    assert_eq!(*frame.get::<Blob>(e).unwrap(), before);
    assert!(reg.set_field(&mut frame, e, "Blob", "pts[3]", Value::Vec2(FPVec2::ZERO)).is_err(), "no fourth point yet");
}

#[test]
fn set_singleton_and_paths_parse() {
    use orr_reflect::{parse_path, PathSeg};
    assert_eq!(
        parse_path("shape.verts[2].y").unwrap(),
        vec![PathSeg::Field("shape".into()), PathSeg::Field("verts".into()), PathSeg::Index(2), PathSeg::Field("y".into())]
    );
    assert_eq!(parse_path("").unwrap(), vec![]);
    assert_eq!(parse_path("[1][2]").unwrap(), vec![PathSeg::Index(1), PathSeg::Index(2)]);
    for bad in [".x", "a..b", "a.", "a[", "a[x]", "a[1]b", "a[-1]"] {
        assert!(parse_path(bad).is_err(), "{bad}");
    }
}
