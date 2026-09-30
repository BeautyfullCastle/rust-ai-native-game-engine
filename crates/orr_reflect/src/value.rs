//! Dynamic values and field paths.

use core::fmt;

use orr_ecs::Entity;
use orr_fp::{FPVec2, FPVec3, FP, FP32};

/// A field value read from or written to raw component bytes.
///
/// The editor inspector works only with this type. Which variant a field has
/// is fixed by its [`TypeDesc`](crate::TypeDesc).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A flag stored as an integer (0 or 1).
    Bool(bool),
    /// Any integer field (`u8` to `u64`, `i8` to `i64`).
    Int(i128),
    /// `FP`, Q48.16.
    Fixed(FP),
    /// `FP32`, Q16.16.
    Fixed32(FP32),
    /// `FPVec2`.
    Vec2(FPVec2),
    /// `FPVec3`.
    Vec3(FPVec3),
    /// An entity handle. `Entity::NONE` means "no entity".
    Entity(Entity),
    /// An entity reference by scene GUID (`None` = no entity). Only scene
    /// documents hold this; baking resolves it to [`Value::Entity`].
    EntityGuid(Option<String>),
    /// A unit enum, by variant name.
    Enum(String),
    /// A set of named bits, by name, in declaration order.
    Flags(Vec<String>),
    /// A fixed or variable length list.
    Array(Vec<Value>),
    /// A struct: `(field name, value)` in declaration order.
    Struct(Vec<(String, Value)>),
    /// A tagged value: variant name and its fields, in declaration order.
    Variant(String, Vec<(String, Value)>),
}

impl Value {
    /// Short name of the variant, for error messages.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "integer",
            Value::Fixed(_) => "fixed-point number",
            Value::Fixed32(_) => "fixed-point number (32 bit)",
            Value::Vec2(_) => "vec2",
            Value::Vec3(_) => "vec3",
            Value::Entity(_) => "entity",
            Value::EntityGuid(_) => "entity reference",
            Value::Enum(_) => "enum",
            Value::Flags(_) => "flags",
            Value::Array(_) => "array",
            Value::Struct(_) => "struct",
            Value::Variant(..) => "tagged value",
        }
    }

    /// The field with `name` of a struct or tagged value.
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Struct(f) | Value::Variant(_, f) => f.iter().find(|(n, _)| n == name).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// One step of a field path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSeg {
    /// `name`, also `x`, `y`, `z` of a vector.
    Field(String),
    /// `[3]`.
    Index(usize),
}

/// Parses `pos.x`, `kills[3]`, `shape.verts[2].y`. The empty path is the
/// whole value.
pub fn parse_path(path: &str) -> Result<Vec<PathSeg>, ReflectError> {
    let bad = |why: &str| ReflectError::BadPath(format!("{path:?}: {why}"));
    fn read_name(path: &str, i: &mut usize) -> String {
        let b = path.as_bytes();
        let start = *i;
        while *i < b.len() && b[*i] != b'.' && b[*i] != b'[' {
            *i += 1;
        }
        path[start..*i].to_string()
    }
    let mut out = Vec::new();
    let b = path.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'[' => {
                let end = path[i..].find(']').ok_or_else(|| bad("missing ']'"))? + i;
                let n: usize = path[i + 1..end].parse().map_err(|_| bad("index is not a number"))?;
                out.push(PathSeg::Index(n));
                i = end + 1;
            }
            b'.' => {
                if out.is_empty() {
                    return Err(bad("path starts with '.'"));
                }
                i += 1;
                if i >= b.len() || b[i] == b'.' || b[i] == b'[' {
                    return Err(bad("empty name"));
                }
                out.push(PathSeg::Field(read_name(path, &mut i)));
            }
            _ => {
                if !out.is_empty() {
                    return Err(bad("expected '.' or '['"));
                }
                out.push(PathSeg::Field(read_name(path, &mut i)));
            }
        }
    }
    Ok(out)
}

/// Error of a reflection operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReflectError {
    /// No type with this name is registered.
    UnknownType(String),
    /// The type is registered for reflection but not in the `Frame`'s `ComponentRegistry`.
    NotInFrame(String),
    /// The entity does not exist.
    NoEntity,
    /// The entity has no component of this type.
    NoComponent(String),
    /// The name is a singleton, not a component (or the other way round).
    WrongTypeKind(String),
    /// A field path does not match the type.
    BadPath(String),
    /// A value has the wrong variant for the field.
    TypeMismatch {
        /// What the field needs.
        expected: String,
        /// What was given.
        found: String,
    },
    /// A value is outside the documented range of the field.
    OutOfRange(String),
    /// Any other invalid value.
    Invalid(String),
}

impl fmt::Display for ReflectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReflectError::UnknownType(n) => write!(f, "unknown type '{n}'"),
            ReflectError::NotInFrame(n) => write!(f, "type '{n}' is not registered in the frame"),
            ReflectError::NoEntity => write!(f, "the entity does not exist"),
            ReflectError::NoComponent(n) => write!(f, "the entity has no component '{n}'"),
            ReflectError::WrongTypeKind(n) => write!(f, "'{n}' is the wrong kind of type for this operation"),
            ReflectError::BadPath(m) => write!(f, "bad field path {m}"),
            ReflectError::TypeMismatch { expected, found } => write!(f, "expected {expected}, found {found}"),
            ReflectError::OutOfRange(m) | ReflectError::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ReflectError {}
