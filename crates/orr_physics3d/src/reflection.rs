//! Scene-file and editor views of the 3D physics state.
//!
//! Runtime fields, cache handles, and padding are deliberately omitted. The
//! collision shape is a small tagged value rather than the raw solver layout.

use core::mem::{offset_of, size_of};

use orr_ecs::Frame;
use orr_fp::{FP, FPQuat, FPVec3, fp};
use orr_reflect::{
    FieldDesc, IntKind, Reflect, TaggedDesc, TypeDesc, TypeRegistry, Value, VariantDesc, ViewField,
};

use crate::{
    BODY_DYNAMIC, BODY_KINEMATIC, BODY_STATIC, Body, Collider, ContactCache, PhysicsConfig,
    PhysicsState, SHAPE_BOX, SHAPE_CAPSULE, SHAPE_SPHERE, Shape,
};

const POSITION_RANGE: &str = "-30000..=30000";
const SHAPE_RANGE: &str = "0.05..=1000";
const SHAPE_HALF_RANGE: &str = "0.05..=1000";

fn fixed(value: FP) -> Value {
    Value::Fixed(value)
}

fn vec3(value: FPVec3) -> Value {
    Value::Vec3(value)
}

fn field<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find(|(field_name, _)| field_name == name)
        .map(|(_, value)| value)
}

fn get_fixed(fields: &[(String, Value)], name: &str) -> Result<FP, String> {
    match field(fields, name) {
        Some(Value::Fixed(value)) => Ok(*value),
        _ => Err(format!("{name}: expected a fixed-point number")),
    }
}

fn get_vec3(fields: &[(String, Value)], name: &str) -> Result<FPVec3, String> {
    match field(fields, name) {
        Some(Value::Vec3(value)) => Ok(*value),
        _ => Err(format!("{name}: expected a vec3")),
    }
}

fn unknown_shape() -> Value {
    Value::Variant("unknown".into(), Vec::new())
}

fn read_shape(bytes: &[u8]) -> Value {
    let shape: Shape = bytemuck::pod_read_unaligned(bytes);
    match shape.kind {
        SHAPE_SPHERE if shape.radius > FP::ZERO && shape.radius <= fp!(1000) => {
            let canonical = Shape::sphere(shape.radius);
            if bytemuck::bytes_of(&canonical) == bytemuck::bytes_of(&shape) {
                Value::Variant(
                    "sphere".into(),
                    vec![("radius".into(), fixed(shape.radius))],
                )
            } else {
                unknown_shape()
            }
        }
        SHAPE_CAPSULE
            if shape.radius > FP::ZERO
                && shape.half.x == FP::ZERO
                && shape.half.z == FP::ZERO
                && shape.half.y > FP::ZERO =>
        {
            let canonical = Shape::capsule(shape.half.y, shape.radius);
            if bytemuck::bytes_of(&canonical) == bytemuck::bytes_of(&shape) {
                Value::Variant(
                    "capsule".into(),
                    vec![
                        ("half_length".into(), fixed(shape.half.y)),
                        ("radius".into(), fixed(shape.radius)),
                    ],
                )
            } else {
                unknown_shape()
            }
        }
        SHAPE_BOX
            if shape.radius == FP::ZERO
                && shape.half.x > FP::ZERO
                && shape.half.y > FP::ZERO
                && shape.half.z > FP::ZERO =>
        {
            let canonical = Shape::cuboid(shape.half.x, shape.half.y, shape.half.z);
            if bytemuck::bytes_of(&canonical) == bytemuck::bytes_of(&shape) {
                Value::Variant(
                    "box".into(),
                    vec![("half_extents".into(), vec3(shape.half))],
                )
            } else {
                unknown_shape()
            }
        }
        _ => unknown_shape(),
    }
}

fn write_shape(bytes: &mut [u8], value: &Value) -> Result<(), String> {
    let Value::Variant(kind, fields) = value else {
        return Err("expected a collision shape".into());
    };
    let shape = match kind.as_str() {
        "sphere" => {
            let radius = get_fixed(fields, "radius")?;
            if radius < fp!(0.05) || radius > fp!(1000) {
                return Err("radius: must be in 0.05..=1000".into());
            }
            Shape::sphere(radius)
        }
        "capsule" => {
            let half_length = get_fixed(fields, "half_length")?;
            let radius = get_fixed(fields, "radius")?;
            if half_length < fp!(0.05) {
                return Err("half_length: must be in 0.05..=1000".into());
            }
            if radius < fp!(0.05) || radius > fp!(1000) {
                return Err("radius: must be in 0.05..=1000".into());
            }
            if half_length + radius > fp!(1000) {
                return Err("half_length: half_length + radius must be at most 1000".into());
            }
            Shape::capsule(half_length, radius)
        }
        "box" => {
            let half = get_vec3(fields, "half_extents")?;
            if half.x < fp!(0.05) || half.y < fp!(0.05) || half.z < fp!(0.05) {
                return Err("half_extents: every half extent must be in 0.05..=1000".into());
            }
            if half.x > fp!(1000) || half.y > fp!(1000) || half.z > fp!(1000) {
                return Err("half_extents: every half extent must be at most 1000".into());
            }
            Shape::cuboid(half.x, half.y, half.z)
        }
        other => return Err(format!("unknown collision shape kind '{other}'")),
    };
    bytes.copy_from_slice(bytemuck::bytes_of(&shape));
    Ok(())
}

fn read_quat(bytes: &[u8]) -> Value {
    let q: FPQuat = bytemuck::pod_read_unaligned(bytes);
    Value::Variant(
        "unit".into(),
        vec![
            ("x".into(), fixed(q.x)),
            ("y".into(), fixed(q.y)),
            ("z".into(), fixed(q.z)),
            ("w".into(), fixed(q.w)),
        ],
    )
}

fn write_quat(bytes: &mut [u8], value: &Value) -> Result<(), String> {
    let Value::Variant(kind, fields) = value else {
        return Err("expected a unit quaternion".into());
    };
    if kind != "unit" {
        return Err("expected quaternion kind 'unit'".into());
    }
    let q = FPQuat::new(
        get_fixed(fields, "x")?,
        get_fixed(fields, "y")?,
        get_fixed(fields, "z")?,
        get_fixed(fields, "w")?,
    );
    let tolerance = fp!(0.001);
    let norm_sq = q.length_sq();
    if norm_sq < FP::ONE - tolerance || norm_sq > FP::ONE + tolerance {
        return Err(
            "rot: quaternion must have unit length (squared length within 0.001 of 1)".into(),
        );
    }
    bytes.copy_from_slice(bytemuck::bytes_of(&q));
    Ok(())
}

fn shape_desc() -> TypeDesc {
    let size = || TypeDesc::fixed().with_range_str(SHAPE_RANGE);
    let half_extents = TypeDesc::vec3().with_range_str(SHAPE_HALF_RANGE);
    TypeDesc::tagged(TaggedDesc {
        size: size_of::<Shape>(),
        variants: vec![
            VariantDesc {
                name: "sphere".into(),
                doc: "A sphere centered on the body's origin.".into(),
                fields: vec![ViewField::new("radius", size(), "Radius in 0.05..=1000.")],
                default: vec![("radius".into(), fixed(fp!(0.5)))],
            },
            VariantDesc {
                name: "capsule".into(),
                doc: "A capsule whose center segment follows the body's local y axis.".into(),
                fields: vec![
                    ViewField::new(
                        "half_length",
                        TypeDesc::fixed().with_range_str(SHAPE_HALF_RANGE),
                        "Half the segment length in 0.05..=1000.",
                    ),
                    ViewField::new(
                        "radius",
                        size(),
                        "Radius in 0.05..=1000; half_length + radius must be at most 1000.",
                    ),
                ],
                default: vec![
                    ("half_length".into(), fixed(fp!(0.5))),
                    ("radius".into(), fixed(fp!(0.25))),
                ],
            },
            VariantDesc {
                name: "box".into(),
                doc: "An oriented box centered on the body's origin.".into(),
                fields: vec![ViewField::new(
                    "half_extents",
                    half_extents,
                    "Positive half extents, each at most 1000.",
                )],
                default: vec![(
                    "half_extents".into(),
                    vec3(FPVec3::new(fp!(0.5), fp!(0.5), fp!(0.5))),
                )],
            },
        ],
        read: read_shape,
        write: write_shape,
    })
    .with_doc("Collision geometry in body-local space; derived solver fields are kept internal.")
}

fn quat_desc() -> TypeDesc {
    let component = || TypeDesc::fixed().with_range_str("-1..=1");
    TypeDesc::tagged(TaggedDesc {
        size: size_of::<FPQuat>(),
        variants: vec![VariantDesc {
            name: "unit".into(),
            doc: "Scene form: {kind: unit, x, y, z, w}. A normalized quaternion in x, y, z, w order.".into(),
            fields: vec![
                ViewField::new("x", component(), "Unit quaternion x component."),
                ViewField::new("y", component(), "Unit quaternion y component."),
                ViewField::new("z", component(), "Unit quaternion z component."),
                ViewField::new("w", component(), "Unit quaternion scalar component."),
            ],
            default: vec![
                ("x".into(), fixed(FP::ZERO)),
                ("y".into(), fixed(FP::ZERO)),
                ("z".into(), fixed(FP::ZERO)),
                ("w".into(), fixed(FP::ONE)),
            ],
        }],
        read: read_quat,
        write: write_quat,
    })
    .with_doc("Scene form {kind: unit, x, y, z, w}; Q48.16 normalized quaternion with squared length within 0.001 of 1.")
}

impl Reflect for Shape {
    fn describe() -> TypeDesc {
        shape_desc()
    }

    fn default_value() -> Self {
        Shape::sphere(fp!(0.5))
    }
}

impl Reflect for Body {
    fn describe() -> TypeDesc {
        TypeDesc::structure(
            size_of::<Body>(),
            vec![
                FieldDesc::new(
                    "pos",
                    offset_of!(Body, pos),
                    TypeDesc::vec3().with_range_str(POSITION_RANGE),
                )
                .with_doc("World position of the center of mass."),
                FieldDesc::new("rot", offset_of!(Body, rot), quat_desc())
                    .with_doc("World orientation."),
                FieldDesc::new(
                    "vel",
                    offset_of!(Body, vel),
                    TypeDesc::vec3().with_range_str("-500..=500"),
                )
                .with_doc("Linear velocity in units per second."),
                FieldDesc::new(
                    "omega",
                    offset_of!(Body, omega),
                    TypeDesc::vec3().with_range_str("-60..=60"),
                )
                .with_doc("Angular velocity in radians per second."),
                FieldDesc::new(
                    "inv_mass",
                    offset_of!(Body, inv_mass),
                    TypeDesc::fixed().with_range_str("0..=1000"),
                )
                .with_doc("Inverse mass; ignored unless kind is dynamic."),
                FieldDesc::new(
                    "inv_inertia",
                    offset_of!(Body, inv_inertia),
                    TypeDesc::vec3().with_range_str("0..=100000"),
                )
                .with_doc("Inverse principal inertia; ignored unless kind is dynamic."),
                FieldDesc::new(
                    "linear_damping",
                    offset_of!(Body, linear_damping),
                    TypeDesc::fixed().with_range_str("0..=100"),
                )
                .with_doc("Per-second linear velocity damping."),
                FieldDesc::new(
                    "angular_damping",
                    offset_of!(Body, angular_damping),
                    TypeDesc::fixed().with_range_str("0..=100"),
                )
                .with_doc("Per-second angular velocity damping."),
                FieldDesc::new(
                    "kind",
                    offset_of!(Body, kind),
                    TypeDesc::enumeration(
                        4,
                        &[
                            ("static", BODY_STATIC as u64),
                            ("dynamic", BODY_DYNAMIC as u64),
                            ("kinematic", BODY_KINEMATIC as u64),
                        ],
                    ),
                )
                .with_doc("Whether the body is static, dynamic or kinematic."),
            ],
        )
        .with_doc("Rigid body state. Sleep and island bookkeeping are maintained by the solver.")
    }

    fn default_value() -> Self {
        Body::new_static(FPVec3::ZERO)
    }
}

impl Reflect for Collider {
    fn describe() -> TypeDesc {
        TypeDesc::structure(
            size_of::<Collider>(),
            vec![
                FieldDesc::new("shape", offset_of!(Collider, shape), shape_desc())
                    .with_doc("Local collision geometry."),
                FieldDesc::new(
                    "restitution",
                    offset_of!(Collider, restitution),
                    TypeDesc::fixed().with_range_str("0..=1"),
                )
                .with_doc("Bounciness."),
                FieldDesc::new(
                    "friction",
                    offset_of!(Collider, friction),
                    TypeDesc::fixed().with_range_str("0..=100"),
                )
                .with_doc("Coulomb friction coefficient."),
                FieldDesc::new(
                    "layer",
                    offset_of!(Collider, layer),
                    TypeDesc::int(IntKind::U32),
                )
                .with_doc("Collision layer bits."),
                FieldDesc::new(
                    "mask",
                    offset_of!(Collider, mask),
                    TypeDesc::int(IntKind::U32),
                )
                .with_doc("Collision mask bits."),
            ],
        )
        .with_doc("Collision shape and material. Reserved flags and padding are kept internal.")
    }

    fn default_value() -> Self {
        Collider::new(Shape::sphere(fp!(0.5)))
    }
}

impl Reflect for PhysicsConfig {
    fn describe() -> TypeDesc {
        TypeDesc::structure(
            size_of::<PhysicsConfig>(),
            vec![
                FieldDesc::new(
                    "gravity",
                    offset_of!(PhysicsConfig, gravity),
                    TypeDesc::vec3().with_range_str("-10000..=10000"),
                ),
                FieldDesc::new(
                    "dt",
                    offset_of!(PhysicsConfig, dt),
                    TypeDesc::fixed().with_range_str("0.0001..=1"),
                ),
                FieldDesc::new(
                    "baumgarte",
                    offset_of!(PhysicsConfig, baumgarte),
                    TypeDesc::fixed().with_range_str("0..=1"),
                ),
                FieldDesc::new(
                    "linear_slop",
                    offset_of!(PhysicsConfig, linear_slop),
                    TypeDesc::fixed().with_range_str("0..=1"),
                ),
                FieldDesc::new(
                    "contact_margin",
                    offset_of!(PhysicsConfig, contact_margin),
                    TypeDesc::fixed().with_range_str("0..=10"),
                ),
                FieldDesc::new(
                    "restitution_threshold",
                    offset_of!(PhysicsConfig, restitution_threshold),
                    TypeDesc::fixed().with_range_str("0..=1000"),
                ),
                FieldDesc::new(
                    "max_correction_speed",
                    offset_of!(PhysicsConfig, max_correction_speed),
                    TypeDesc::fixed().with_range_str("0..=1000"),
                ),
                FieldDesc::new(
                    "max_linear_speed",
                    offset_of!(PhysicsConfig, max_linear_speed),
                    TypeDesc::fixed().with_range_str("0..=10000"),
                ),
                FieldDesc::new(
                    "max_angular_speed",
                    offset_of!(PhysicsConfig, max_angular_speed),
                    TypeDesc::fixed().with_range_str("0..=1000"),
                ),
                FieldDesc::new(
                    "velocity_iterations",
                    offset_of!(PhysicsConfig, velocity_iterations),
                    TypeDesc::int(IntKind::U32).with_range_str("1..=64"),
                ),
                FieldDesc::new(
                    "substeps",
                    offset_of!(PhysicsConfig, substeps),
                    TypeDesc::int(IntKind::U32).with_range_str("1..=64"),
                ),
                FieldDesc::new(
                    "sleep_ticks",
                    offset_of!(PhysicsConfig, sleep_ticks),
                    TypeDesc::int(IntKind::U32).with_range_str("0..=1000000"),
                ),
                FieldDesc::new(
                    "sleep_linear_speed",
                    offset_of!(PhysicsConfig, sleep_linear_speed),
                    TypeDesc::fixed().with_range_str("0..=100"),
                ),
                FieldDesc::new(
                    "sleep_angular_speed",
                    offset_of!(PhysicsConfig, sleep_angular_speed),
                    TypeDesc::fixed().with_range_str("0..=100"),
                ),
            ],
        )
        .with_doc("Deterministic 3D solver settings stored in the simulation frame.")
    }

    fn default_value() -> Self {
        PhysicsConfig::default()
    }
}

impl Reflect for PhysicsState {
    fn describe() -> TypeDesc {
        TypeDesc::structure(
            size_of::<PhysicsState>(),
            vec![FieldDesc::new(
                "config",
                offset_of!(PhysicsState, config),
                PhysicsConfig::describe(),
            )
            .with_doc(
                "Solver settings. The warm-start cache handle is initialized by the physics hook.",
            )],
        )
        .with_doc("3D physics singleton. The rollback-safe contact cache is runtime state.")
    }

    fn default_value() -> Self {
        PhysicsState {
            config: PhysicsConfig::default(),
            contacts: orr_ecs::FrameList::<ContactCache>::NONE,
        }
    }
}

fn init_physics_state(frame: &mut Frame, present: bool) {
    let config = if present {
        frame.singleton::<PhysicsState>().config
    } else {
        PhysicsConfig::default()
    };
    crate::init(frame, config);
}

/// Registers reflected 3D physics types using the same names as [`crate::register`].
///
/// Baking a scene initializes the persistent contact cache and preserves an
/// authored solver configuration, matching [`crate::init`].
pub fn register_reflect(types: &mut TypeRegistry) {
    types.register_component::<Body>("orr_physics3d::Body");
    types.register_component::<Collider>("orr_physics3d::Collider");
    types.register_singleton_with_init::<PhysicsState>(
        "orr_physics3d::PhysicsState",
        init_physics_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::{ComponentRegistryBuilder, FrameList};
    use orr_reflect::{Scene, SceneIndex};

    fn nonempty_target() -> (TypeRegistry, Frame, u32) {
        let mut builder = ComponentRegistryBuilder::new();
        crate::register(&mut builder);
        let mut target = Frame::new(builder.build());
        let config = PhysicsConfig {
            gravity: FPVec3::new(fp!(1), fp!(-4), fp!(2)),
            substeps: 6,
            ..PhysicsConfig::default()
        };
        crate::init(&mut target, config);
        let contacts = target.singleton::<PhysicsState>().contacts;
        target.list_push(
            contacts,
            ContactCache {
                a: 2,
                b: 7,
                id: 19,
                _pad: 0,
                normal_impulse: fp!(0.25),
                tangent_impulse: FPVec3::new(fp!(1), fp!(2), fp!(3)),
            },
        );
        let sentinel = crate::spawn_body(
            &mut target,
            Body::new_static(FPVec3::new(fp!(3), fp!(4), fp!(5))),
            Collider::new(Shape::sphere(fp!(0.5))),
        );
        let freed = target.spawn();
        assert!(target.despawn(freed));
        target.set_tick(23);

        assert_eq!(target.singleton::<PhysicsState>().config, config);
        assert_eq!(target.list(contacts).len(), 1);
        assert!(target.get::<Body>(sentinel).is_some());
        assert!(!target.exists(freed));

        let mut types = TypeRegistry::new();
        register_reflect(&mut types);
        (types, target, freed.index)
    }

    fn collider_fields_mut(scene: &mut Scene) -> &mut Vec<(String, Value)> {
        let value = scene
            .entities
            .values_mut()
            .find_map(|entity| {
                entity
                    .components
                    .iter_mut()
                    .find(|(name, _)| name == "orr_physics3d::Collider")
                    .map(|(_, value)| value)
            })
            .expect("the target scene has a collider");
        match value {
            Value::Struct(fields) => fields,
            other => panic!("Collider is not a struct: {other:?}"),
        }
    }

    fn body_fields_mut(scene: &mut Scene) -> &mut Vec<(String, Value)> {
        let value = scene
            .entities
            .values_mut()
            .find_map(|entity| {
                entity
                    .components
                    .iter_mut()
                    .find(|(name, _)| name == "orr_physics3d::Body")
                    .map(|(_, value)| value)
            })
            .expect("the target scene has a body");
        match value {
            Value::Struct(fields) => fields,
            other => panic!("Body is not a struct: {other:?}"),
        }
    }

    fn assert_invalid_scene_load_preserves_target(
        mutate: fn(&mut Scene),
        guard_message: &str,
    ) {
        fn load(
            text: &str,
            types: &TypeRegistry,
            target: &mut Frame,
        ) -> Result<SceneIndex, String> {
            Scene::parse(text, types)
                .map_err(|error| format!("parse: {error}"))
                .and_then(|scene| {
                    scene
                        .bake(types, target)
                        .map_err(|error| format!("bake: {error}"))
                })
        }

        let (types, mut target, freed_index) = nonempty_target();
        let valid_scene = Scene::unbake(&types, &target, None).unwrap();
        let valid_yaml = valid_scene.to_yaml();
        let mut invalid_scene = valid_scene.clone();
        mutate(&mut invalid_scene);
        let invalid_yaml = invalid_scene.to_yaml();

        let original = target.clone();
        let before_bytes = original.to_bytes();
        let before_checksum = original.checksum();
        let error = load(&invalid_yaml, &types, &mut target).unwrap_err();
        assert!(
            error.starts_with("parse: "),
            "invalid YAML must be rejected by Scene::parse before bake, got: {error}"
        );
        assert!(
            error.contains(guard_message),
            "parse should report the custom guard {guard_message:?}, got: {error}"
        );
        assert_eq!(
            target.to_bytes(),
            before_bytes,
            "a rejected load must not touch frame bytes"
        );
        assert_eq!(
            target.checksum(),
            before_checksum,
            "a rejected load must not alter frame state"
        );

        let mut expected = original.clone();
        let expected_index = Scene::parse(&valid_yaml, &types)
            .unwrap()
            .bake(&types, &mut expected)
            .unwrap();
        let actual_index = load(&valid_yaml, &types, &mut target).unwrap();
        assert_eq!(
            actual_index, expected_index,
            "failed loads preserve allocation identity"
        );
        assert_eq!(
            actual_index.iter().next().unwrap().1.index,
            freed_index,
            "the subsequent valid load reuses the original freed slot"
        );
        assert_eq!(target.to_bytes(), expected.to_bytes());
        assert_eq!(target.checksum(), expected.checksum());
    }

    #[test]
    fn body_descriptor_round_trips_layout_and_hides_runtime_words() {
        let mut body = Body::new_dynamic(
            FPVec3::new(fp!(1), fp!(2), fp!(3)),
            &Shape::sphere(fp!(0.5)),
            FP::ONE,
        )
        .with_rotation(FPQuat::new(fp!(0.1), fp!(0.2), fp!(0.3), fp!(0.9)).normalize());
        body.sleep = 7;
        body.island = 11;
        body._pad = u32::MAX;

        let desc = Body::describe();
        assert_eq!(desc.size(), size_of::<Body>());
        let value = desc.read(bytemuck::bytes_of(&body));
        assert!(value.field("sleep").is_none());
        assert!(value.field("island").is_none());
        assert!(value.field("_pad").is_none());
        assert_eq!(
            value.field("rot").and_then(|q| q.field("w")),
            Some(&Value::Fixed(body.rot.w))
        );

        let mut bytes = bytemuck::bytes_of(&body).to_vec();
        desc.write(&mut bytes, &value).unwrap();
        assert_eq!(desc.read(&bytes), value);
        let restored: Body = bytemuck::pod_read_unaligned(&bytes);
        assert_eq!(restored.pos, body.pos);
        assert_eq!(restored.rot, body.rot);
    }

    #[test]
    fn quaternion_descriptor_requires_unit_length_and_preserves_valid_rotations() {
        let desc = quat_desc();
        let valid = FPQuat::from_axis_angle(FPVec3::Y, fp!(0.75));
        let valid_value = desc.read(bytemuck::bytes_of(&valid));
        let original = bytemuck::bytes_of(&valid).to_vec();
        let mut bytes = original.clone();
        desc.write(&mut bytes, &valid_value).unwrap();
        assert_eq!(bytes, original);

        for invalid in [
            FPQuat::new(FP::ZERO, FP::ZERO, FP::ZERO, FP::ZERO),
            FPQuat::new(FP::ZERO, FP::ZERO, FP::ZERO, fp!(0.5)),
            FPQuat::new(FP::ZERO, FP::ZERO, FP::ZERO, fp!(1.01)),
        ] {
            let value = desc.read(bytemuck::bytes_of(&invalid));
            let before = bytes.clone();
            assert!(desc.write(&mut bytes, &value).is_err());
            assert_eq!(
                bytes, before,
                "rejected rotation must leave component bytes untouched"
            );
        }
    }

    #[test]
    fn shape_descriptors_round_trip_canonical_shapes_and_reject_bad_geometry() {
        let desc = Shape::describe();
        for shape in [
            Shape::sphere(fp!(0.5)),
            Shape::capsule(fp!(0.75), fp!(0.25)),
            Shape::cuboid(fp!(0.5), fp!(1), fp!(1.5)),
        ] {
            let original = bytemuck::bytes_of(&shape).to_vec();
            let value = desc.read(&original);
            let mut written = vec![0; size_of::<Shape>()];
            desc.write(&mut written, &value).unwrap();
            assert_eq!(written, original);
        }

        let mut bytes = vec![0; size_of::<Shape>()];
        let bad_box = Value::Variant(
            "box".into(),
            vec![(
                "half_extents".into(),
                vec3(FPVec3::new(FP::ZERO, FP::ONE, FP::ONE)),
            )],
        );
        assert!(desc.write(&mut bytes, &bad_box).is_err());
        let bad_capsule = Value::Variant(
            "capsule".into(),
            vec![
                ("half_length".into(), fixed(FP::ZERO)),
                ("radius".into(), fixed(fp!(0.5))),
            ],
        );
        assert!(desc.write(&mut bytes, &bad_capsule).is_err());
    }

    #[test]
    fn physics_state_init_allocates_a_fresh_cache_and_preserves_authored_config() {
        let mut builder = ComponentRegistryBuilder::new();
        crate::register(&mut builder);
        let frame_registry = builder.build();

        let mut types = TypeRegistry::new();
        register_reflect(&mut types);
        let hook = types
            .get("orr_physics3d::PhysicsState")
            .unwrap()
            .init_hook()
            .unwrap();

        let mut fresh = Frame::new(frame_registry.clone());
        hook(&mut fresh, false);
        let fresh_state = *fresh.singleton::<PhysicsState>();
        assert_eq!(fresh_state.config, PhysicsConfig::default());
        assert_ne!(fresh_state.contacts, FrameList::<ContactCache>::NONE);
        assert!(fresh.list(fresh_state.contacts).is_empty());

        let authored = PhysicsConfig {
            gravity: FPVec3::new(fp!(1), fp!(-4), fp!(2)),
            substeps: 6,
            ..PhysicsConfig::default()
        };
        let mut baked = Frame::new(frame_registry);
        baked.set_singleton(PhysicsState {
            config: authored,
            contacts: FrameList::<ContactCache>::NONE,
        });
        hook(&mut baked, true);
        let baked_state = *baked.singleton::<PhysicsState>();
        assert_eq!(baked_state.config, authored);
        assert_ne!(baked_state.contacts, FrameList::<ContactCache>::NONE);
        assert!(baked.list(baked_state.contacts).is_empty());
    }

    #[test]
    fn scene_parse_rejects_capsule_overall_extent_without_mutating_target() {
        assert_invalid_scene_load_preserves_target(
            |scene| {
                let (_, shape) = collider_fields_mut(scene)
                    .iter_mut()
                    .find(|(name, _)| name == "shape")
                    .unwrap();
                *shape = Value::Variant(
                    "capsule".into(),
                    vec![
                        ("half_length".into(), fixed(fp!(600))),
                        ("radius".into(), fixed(fp!(500))),
                    ],
                );
            },
            "half_length: half_length + radius must be at most 1000",
        );
    }

    #[test]
    fn scene_parse_rejects_non_unit_quaternion_without_mutating_target() {
        assert_invalid_scene_load_preserves_target(
            |scene| {
                let (_, rot) = body_fields_mut(scene)
                    .iter_mut()
                    .find(|(name, _)| name == "rot")
                    .unwrap();
                *rot = Value::Variant(
                    "unit".into(),
                    vec![
                        ("x".into(), fixed(FP::ONE)),
                        ("y".into(), fixed(FP::ONE)),
                        ("z".into(), fixed(FP::ZERO)),
                        ("w".into(), fixed(FP::ZERO)),
                    ],
                );
            },
            "quaternion must have unit length (squared length within 0.001 of 1)",
        );
    }
}
