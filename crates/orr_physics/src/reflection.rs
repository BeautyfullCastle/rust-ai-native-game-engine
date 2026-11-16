//! Reflection of the physics types, for the editor and scene files.
//!
//! `Body`, `Collider`, `PhysicsConfig` and `PhysicsState` derive `Reflect`
//! (see the attributes in `types.rs`). Fields that are runtime state are
//! hidden: sleep state and island id of a body, the `FrameList` handles of
//! `PhysicsState`, and padding.
//!
//! # `Shape` as a friendly view
//!
//! The raw `Shape` holds fixed arrays (`verts`, `normals`, 8 entries each), a
//! kind code and derived data (edge normals, capsule axis and length). An
//! author never sets those by hand. The reflected view is a tagged value with
//! a `kind` key:
//!
//! | kind | fields | notes |
//! |---|---|---|
//! | `circle` | `radius` | `0.05 ..= 1000` |
//! | `box` | `half_extents` (vec2) | each `0.05 ..= 1000`, centered on the body |
//! | `polygon` | `verts` (3 to 8 vec2) | counter-clockwise, convex; re-centered on the centroid when written |
//! | `capsule` | `half_length`, `radius` | upright (along local y); `0 < half_length`, `radius >= 0.05`, `half_length + radius <= 1000` |
//! | `capsule_segment` | `a`, `b`, `radius` | any axis; `a` and `b` are re-centered on the midpoint when written |
//!
//! Reading picks the simplest kind that gives back the same bytes: a
//! 4-corner polygon that equals `Shape::box_shape` reads as `box`, an upright
//! centered capsule reads as `capsule`. Writing goes through the
//! constructors (`Shape::circle`, `box_shape`, `polygon`, `capsule`,
//! `capsule_segment`), so normals and lengths are always computed by the
//! engine and never trusted from a file.
//!
//! A `polygon` or `capsule_segment` whose points are not already centered on
//! their centroid or midpoint is centered when written. Saving a scene writes
//! the centered points, so the first save can change such values by the
//! centering offset. After that the text is stable.

use core::mem::{offset_of, size_of};

use orr_fp::{fp, FPVec2, FP};
use orr_reflect::{FieldDesc, Reflect, TaggedDesc, TypeDesc, TypeRegistry, Value, VariantDesc, ViewField};

use crate::types::{MAX_POLY_VERTS, SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_POLYGON};
use crate::joints::UNRECORDED;
use crate::{Body, Collider, Joint, PhysicsConfig, PhysicsState, Shape, JOINT_LIMIT, JOINT_MOTOR, JOINT_PIN, JOINT_PRISMATIC, JOINT_ROD, JOINT_ROPE, JOINT_SPRING};

const SIZE_RANGE: &str = "0.05..=1000";
const POINT_RANGE: &str = "-1000..=1000";
const MAX_HALF_LENGTH_PLUS_RADIUS: FP = FP(1000 << 16);

fn v2(v: FPVec2) -> Value {
    Value::Vec2(v)
}

fn fixed(v: FP) -> Value {
    Value::Fixed(v)
}

fn field<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn get_fixed(fields: &[(String, Value)], name: &str) -> Result<FP, String> {
    match field(fields, name) {
        Some(Value::Fixed(x)) => Ok(*x),
        _ => Err(format!("{name}: expected a number")),
    }
}

fn get_vec2(fields: &[(String, Value)], name: &str) -> Result<FPVec2, String> {
    match field(fields, name) {
        Some(Value::Vec2(x)) => Ok(*x),
        _ => Err(format!("{name}: expected a vec2")),
    }
}

fn read_shape(bytes: &[u8]) -> Value {
    let s: Shape = bytemuck::pod_read_unaligned(bytes);
    let unknown = || Value::Variant("unknown".to_string(), Vec::new());
    match s.kind {
        SHAPE_CIRCLE => Value::Variant("circle".into(), vec![("radius".into(), fixed(s.radius))]),
        SHAPE_POLYGON => {
            let n = s.count as usize;
            if !(3..=MAX_POLY_VERTS).contains(&n) {
                return unknown();
            }
            if n == 4 && s.verts[1].x > FP::ZERO && s.verts[2].y > FP::ZERO {
                let b = Shape::box_shape(s.verts[1].x, s.verts[2].y);
                if bytemuck::bytes_of(&b) == bytemuck::bytes_of(&s) {
                    return Value::Variant("box".into(), vec![("half_extents".into(), v2(FPVec2::new(s.verts[1].x, s.verts[2].y)))]);
                }
            }
            Value::Variant("polygon".into(), vec![("verts".into(), Value::Array(s.verts[..n].iter().map(|p| v2(*p)).collect()))])
        }
        SHAPE_CAPSULE => {
            let h = s.verts[1].y;
            if h > FP::ZERO && s.verts[0] == FPVec2::new(FP::ZERO, -h) && s.verts[1] == FPVec2::new(FP::ZERO, h) {
                let c = Shape::capsule(h, s.radius);
                if bytemuck::bytes_of(&c) == bytemuck::bytes_of(&s) {
                    return Value::Variant("capsule".into(), vec![("half_length".into(), fixed(h)), ("radius".into(), fixed(s.radius))]);
                }
            }
            Value::Variant(
                "capsule_segment".into(),
                vec![("a".into(), v2(s.verts[0])), ("b".into(), v2(s.verts[1])), ("radius".into(), fixed(s.radius))],
            )
        }
        _ => unknown(),
    }
}

fn write_shape(bytes: &mut [u8], v: &Value) -> Result<(), String> {
    let Value::Variant(kind, f) = v else {
        return Err("expected a shape".to_string());
    };
    let shape = match kind.as_str() {
        "circle" => Shape::circle(get_fixed(f, "radius")?),
        "box" => {
            let h = get_vec2(f, "half_extents")?;
            if h.x <= FP::ZERO || h.y <= FP::ZERO {
                return Err("half_extents: both half extents must be greater than 0".to_string());
            }
            Shape::box_shape(h.x, h.y)
        }
        "polygon" => {
            let Some(Value::Array(items)) = field(f, "verts") else {
                return Err("verts: expected a list of points".to_string());
            };
            let pts: Vec<FPVec2> = items.iter().map(|p| if let Value::Vec2(x) = p { *x } else { FPVec2::ZERO }).collect();
            Shape::polygon(&pts).ok_or_else(|| "verts: polygon must be convex, counter-clockwise and not degenerate".to_string())?
        }
        "capsule" => {
            let (h, r) = (get_fixed(f, "half_length")?, get_fixed(f, "radius")?);
            if h <= FP::ZERO {
                return Err("half_length: must be greater than 0 (use a circle for a round shape)".to_string());
            }
            if h + r > MAX_HALF_LENGTH_PLUS_RADIUS {
                return Err("half_length: half_length + radius must be at most 1000".to_string());
            }
            Shape::capsule(h, r)
        }
        "capsule_segment" => {
            let (a, b, r) = (get_vec2(f, "a")?, get_vec2(f, "b")?, get_fixed(f, "radius")?);
            if a == b {
                return Err("b: a and b must differ (use a circle for a round shape)".to_string());
            }
            let half = (b - a).length() * FP::HALF;
            if half + r > MAX_HALF_LENGTH_PLUS_RADIUS {
                return Err("b: half the segment length + radius must be at most 1000".to_string());
            }
            Shape::capsule_segment(a, b, r)
        }
        other => return Err(format!("unknown shape kind '{other}'")),
    };
    bytes.copy_from_slice(bytemuck::bytes_of(&shape));
    Ok(())
}

impl Reflect for Shape {
    fn describe() -> TypeDesc {
        let point = TypeDesc::vec2().with_range_str(POINT_RANGE);
        let size = || TypeDesc::fixed().with_range_str(SIZE_RANGE);
        let variants = vec![
            VariantDesc {
                name: "circle".into(),
                doc: "A disc centered on the body origin.".into(),
                fields: vec![ViewField::new("radius", size(), "Radius, 0.05 to 1000.")],
                default: vec![("radius".into(), fixed(fp!(0.5)))],
            },
            VariantDesc {
                name: "box".into(),
                doc: "A box centered on the body origin (an OBB when the body rotates).".into(),
                fields: vec![ViewField::new(
                    "half_extents",
                    TypeDesc::vec2().with_range_str(SIZE_RANGE),
                    "Half width and half height, each 0.05 to 1000.",
                )],
                default: vec![("half_extents".into(), v2(FPVec2::new(fp!(0.5), fp!(0.5))))],
            },
            VariantDesc {
                name: "polygon".into(),
                doc: "A convex polygon. Corners go counter-clockwise. The corners are re-centered on the centroid.".into(),
                fields: vec![ViewField::new("verts", TypeDesc::list(point.clone(), 3, MAX_POLY_VERTS), "3 to 8 corners in body space.")],
                default: vec![(
                    "verts".into(),
                    Value::Array(vec![
                        v2(FPVec2::new(fp!(-0.5), fp!(-0.5))),
                        v2(FPVec2::new(fp!(0.5), fp!(-0.5))),
                        v2(FPVec2::new(FP::ZERO, fp!(0.5))),
                    ]),
                )],
            },
            VariantDesc {
                name: "capsule".into(),
                doc: "An upright capsule: a segment along the local y axis grown by the radius.".into(),
                fields: vec![
                    ViewField::new("half_length", TypeDesc::fixed().with_range_str("0..=1000"), "Half the segment length, greater than 0."),
                    ViewField::new("radius", size(), "Radius, at least 0.05. half_length + radius must be at most 1000."),
                ],
                default: vec![("half_length".into(), fixed(fp!(0.5))), ("radius".into(), fixed(fp!(0.25)))],
            },
            VariantDesc {
                name: "capsule_segment".into(),
                doc: "A capsule around any segment a-b. The points are re-centered on the midpoint.".into(),
                fields: vec![
                    ViewField::new("a", point.clone(), "First end of the segment."),
                    ViewField::new("b", point, "Second end of the segment. Must differ from a."),
                    ViewField::new("radius", size(), "Radius, at least 0.05."),
                ],
                default: vec![
                    ("a".into(), v2(FPVec2::new(fp!(-0.5), FP::ZERO))),
                    ("b".into(), v2(FPVec2::new(fp!(0.5), FP::ZERO))),
                    ("radius".into(), fixed(fp!(0.25))),
                ],
            },
        ];
        TypeDesc::tagged(TaggedDesc { size: core::mem::size_of::<Shape>(), variants, read: read_shape, write: write_shape })
            .with_doc("Collision shape in body space. A friendly view: the kind and its size, not the raw vertex and normal arrays.")
    }

    fn default_value() -> Self {
        Shape::circle(fp!(0.5))
    }
}

/// Registers the physics types for reflection, under the same names as
/// [`register`](crate::register) uses in the `ComponentRegistry`:
/// `orr_physics::Body`, `orr_physics::Collider` and the singleton
/// `orr_physics::PhysicsState`.
///
/// Baking a scene runs the init hook of `PhysicsState`. It allocates the two
/// `FrameList`s and stores the config (the scene's, or `PhysicsConfig::default()`
/// when the scene has none), like [`init`](crate::init).
pub fn register_reflect(types: &mut TypeRegistry) {
    types.register_component::<Body>("orr_physics::Body");
    types.register_component::<Collider>("orr_physics::Collider");
    types.register_singleton_with_init::<PhysicsState>("orr_physics::PhysicsState", |frame, present| {
        let config = if present { frame.singleton::<PhysicsState>().config } else { PhysicsConfig::default() };
        crate::init(frame, config);
    });
}

impl Reflect for Joint {
    fn describe() -> TypeDesc {
        let vec2 = |range| TypeDesc::vec2().with_range_str(range);
        let fixed = |range| TypeDesc::fixed().with_range_str(range);
        TypeDesc::structure(
            size_of::<Joint>(),
            vec![
                FieldDesc::new("a", offset_of!(Joint, a), TypeDesc::entity()).with_doc("The first body."),
                FieldDesc::new("b", offset_of!(Joint, b), TypeDesc::entity()).with_doc("The second body."),
                FieldDesc::new("anchor_a", offset_of!(Joint, anchor_a), vec2("-1000..=1000"))
                    .with_doc("Anchor on a, in a's own frame (from its center of mass, unrotated)."),
                FieldDesc::new("anchor_b", offset_of!(Joint, anchor_b), vec2("-1000..=1000"))
                    .with_doc("Anchor on b, in b's own frame."),
                FieldDesc::new("length", offset_of!(Joint, length), fixed("-3.1416..=1000"))
                    .with_doc("Distance between the anchors of a rod, the most of a rope, or the rest length of a spring. A prismatic joint's slide axis as an angle in radians in a's own frame."),
                FieldDesc::new(
                    "kind",
                    offset_of!(Joint, kind),
                    TypeDesc::enumeration(
                        4,
                        &[
                            ("pin", JOINT_PIN as u64),
                            ("rod", JOINT_ROD as u64),
                            ("rope", JOINT_ROPE as u64),
                            ("prismatic", JOINT_PRISMATIC as u64),
                            ("spring", JOINT_SPRING as u64),
                        ],
                    ),
                )
                .with_doc("What the joint keeps: anchors together, a distance, a most distance, a slide along one axis, or a soft distance."),
                FieldDesc::new(
                    "flags",
                    offset_of!(Joint, flags),
                    TypeDesc::flags(4, &[("motor", JOINT_MOTOR as u64), ("limit", JOINT_LIMIT as u64)]),
                )
                .with_doc("A pin's or slide's motor and limits."),
                FieldDesc::new("lower", offset_of!(Joint, lower), fixed("-1000..=1000"))
                    .with_doc("Lowest relative angle of a pin in radians, measured from the pose at the first step, or lowest slide of a prismatic joint in meters, as the gap between the anchors along the axis (0 when the anchors coincide)."),
                FieldDesc::new("upper", offset_of!(Joint, upper), fixed("-1000..=1000"))
                    .with_doc("Highest relative angle of a pin in radians, or highest slide of a prismatic joint in meters."),
                FieldDesc::new("motor_speed", offset_of!(Joint, motor_speed), fixed("-100..=100"))
                    .with_doc("Turn rate in radians per second a pin motor aims for, or slide speed of a prismatic motor. A spring's frequency in hertz; 0 is rigid like a rod."),
                FieldDesc::new("max_motor_torque", offset_of!(Joint, max_motor_torque), fixed("0..=100000"))
                    .with_doc("Largest torque the motor applies, or force for a prismatic motor. A spring's damping ratio: 0 never settles, 1 settles without overshoot."),
            ],
        )
        .with_doc("A joint between two bodies, on an entity of its own. The zero angle is recorded from the poses at the first step.")
    }

    fn default_value() -> Self {
        Joint {
            length: FP::ONE,
            lower: -FP::ONE,
            upper: FP::ONE,
            ref_angle: UNRECORDED,
            ..Joint::new(JOINT_PIN, orr_ecs::Entity::NONE, orr_ecs::Entity::NONE, FPVec2::ZERO, FPVec2::ZERO, FP::ZERO)
        }
    }
}

/// Registers [`Joint`] for scene files, the inspector and the `orr` command.
/// Call it in a game that calls [`crate::register_joints`].
pub fn register_joints_reflect(types: &mut TypeRegistry) {
    types.register_component::<Joint>("orr_physics::Joint");
}
