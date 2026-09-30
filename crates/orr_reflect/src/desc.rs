//! Type descriptors: what a component looks like, field by field, and how to
//! read and write it as raw bytes.
//!
//! A [`TypeDesc`] describes either a *layout* (fields at byte offsets of a
//! `Pod` value) or a *view* (a friendlier shape that a hand-written
//! [`TaggedDesc`] converts from and to the bytes with two functions).
//!
//! Every read and write goes through [`Value`], so an editor can inspect and
//! edit a component without knowing its Rust type.

use orr_ecs::Entity;
use orr_fp::{FPVec2, FPVec3, FP, FP32};

use crate::decimal;
use crate::value::{PathSeg, ReflectError, Value};

/// Integer field types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntKind {
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `u64`
    U64,
    /// `i8`
    I8,
    /// `i16`
    I16,
    /// `i32`
    I32,
    /// `i64`
    I64,
}

impl IntKind {
    /// Size in bytes.
    pub fn size(self) -> usize {
        match self {
            IntKind::U8 | IntKind::I8 => 1,
            IntKind::U16 | IntKind::I16 => 2,
            IntKind::U32 | IntKind::I32 => 4,
            IntKind::U64 | IntKind::I64 => 8,
        }
    }
    /// Smallest value.
    pub fn min(self) -> i128 {
        match self {
            IntKind::U8 | IntKind::U16 | IntKind::U32 | IntKind::U64 => 0,
            IntKind::I8 => i128::from(i8::MIN),
            IntKind::I16 => i128::from(i16::MIN),
            IntKind::I32 => i128::from(i32::MIN),
            IntKind::I64 => i128::from(i64::MIN),
        }
    }
    /// Largest value.
    pub fn max(self) -> i128 {
        match self {
            IntKind::U8 => i128::from(u8::MAX),
            IntKind::U16 => i128::from(u16::MAX),
            IntKind::U32 => i128::from(u32::MAX),
            IntKind::U64 => i128::from(u64::MAX),
            IntKind::I8 => i128::from(i8::MAX),
            IntKind::I16 => i128::from(i16::MAX),
            IntKind::I32 => i128::from(i32::MAX),
            IntKind::I64 => i128::from(i64::MAX),
        }
    }
    /// Name used in messages and the schema (`u32`).
    pub fn name(self) -> &'static str {
        match self {
            IntKind::U8 => "u8",
            IntKind::U16 => "u16",
            IntKind::U32 => "u32",
            IntKind::U64 => "u64",
            IntKind::I8 => "i8",
            IntKind::I16 => "i16",
            IntKind::I32 => "i32",
            IntKind::I64 => "i64",
        }
    }
}

/// Documented value range, inclusive, in raw units: the integer itself, or
/// the raw Q48.16 / Q16.16 value for fixed-point fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    /// Smallest allowed raw value.
    pub min: Option<i128>,
    /// Largest allowed raw value.
    pub max: Option<i128>,
}

impl Range {
    /// True if `raw` is inside the range.
    pub fn contains(&self, raw: i128) -> bool {
        self.min.is_none_or(|m| raw >= m) && self.max.is_none_or(|m| raw <= m)
    }
    /// True if no bound is set.
    pub fn is_open(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }
}

/// A field of a struct layout.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDesc {
    /// Field name (also the YAML key).
    pub name: String,
    /// Byte offset inside the struct.
    pub offset: usize,
    /// Type of the field.
    pub ty: TypeDesc,
    /// Documentation, shown in the inspector and the schema.
    pub doc: String,
}

impl FieldDesc {
    /// A field at `offset`.
    pub fn new(name: &str, offset: usize, ty: TypeDesc) -> Self {
        Self { name: name.to_string(), offset, ty, doc: String::new() }
    }
    /// Sets the documentation.
    pub fn with_doc(mut self, doc: &str) -> Self {
        self.doc = doc.trim().to_string();
        self
    }
}

/// A field of a tagged variant (no byte offset: the view is not a layout).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewField {
    /// Field name.
    pub name: String,
    /// Type of the field.
    pub ty: TypeDesc,
    /// Documentation.
    pub doc: String,
}

impl ViewField {
    /// A view field.
    pub fn new(name: &str, ty: TypeDesc, doc: &str) -> Self {
        Self { name: name.to_string(), ty, doc: doc.trim().to_string() }
    }
}

/// One variant of a tagged value.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantDesc {
    /// Variant name, the value of the `kind` key.
    pub name: String,
    /// Documentation.
    pub doc: String,
    /// Fields of the variant.
    pub fields: Vec<ViewField>,
    /// Field values used when an editor switches a value to this variant.
    pub default: Vec<(String, Value)>,
}

/// A hand-written friendly view of some bytes.
///
/// `read` must give a value that passes `check` for every byte pattern the
/// simulation can produce (or one that `check` rejects with a clear message).
/// `write` gets a value that already passed `check`; it may still reject a
/// combination (a non-convex polygon) with a message.
#[derive(Clone, Debug)]
pub struct TaggedDesc {
    /// Size of the bytes the view covers.
    pub size: usize,
    /// The variants.
    pub variants: Vec<VariantDesc>,
    /// Bytes to value.
    pub read: fn(&[u8]) -> Value,
    /// Value to bytes.
    pub write: fn(&mut [u8], &Value) -> Result<(), String>,
}

impl PartialEq for TaggedDesc {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.variants == other.variants
    }
}

/// What kind of value a [`TypeDesc`] describes.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// 1 or 4 bytes holding 0 or 1.
    Bool {
        /// Storage size in bytes.
        width: usize,
    },
    /// An integer.
    Int {
        /// Which integer type.
        int: IntKind,
        /// Allowed values.
        range: Range,
    },
    /// `FP`.
    Fixed {
        /// Allowed raw values.
        range: Range,
    },
    /// `FP32`.
    Fixed32 {
        /// Allowed raw values.
        range: Range,
    },
    /// `FPVec2`; the range applies to each component.
    Vec2 {
        /// Allowed raw values per component.
        range: Range,
    },
    /// `FPVec3`; the range applies to each component.
    Vec3 {
        /// Allowed raw values per component.
        range: Range,
    },
    /// `Entity` (8 bytes).
    Entity,
    /// A unit enum stored as an unsigned integer.
    Enum {
        /// Storage size in bytes (1, 2, 4 or 8).
        width: usize,
        /// Names and stored values.
        variants: Vec<(String, u64)>,
    },
    /// Named bits of an unsigned integer.
    Flags {
        /// Storage size in bytes (1, 2, 4 or 8).
        width: usize,
        /// Names and bit masks.
        bits: Vec<(String, u64)>,
    },
    /// A fixed size array.
    Array {
        /// Element type.
        elem: Box<TypeDesc>,
        /// Element count.
        len: usize,
    },
    /// A struct layout.
    Struct {
        /// Size of the struct in bytes.
        size: usize,
        /// Visible fields. Hidden fields (padding, caches) are left out.
        fields: Vec<FieldDesc>,
    },
    /// A hand-written tagged view.
    Tagged(Box<TaggedDesc>),
    /// A variable length list. Only valid inside a tagged view.
    List {
        /// Element type.
        elem: Box<TypeDesc>,
        /// Fewest elements.
        min: usize,
        /// Most elements.
        max: usize,
    },
}

/// Describes one type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDesc {
    /// The kind.
    pub kind: Kind,
    /// Documentation.
    pub doc: String,
}

fn ne_uint(bytes: &[u8]) -> u64 {
    match bytes.len() {
        1 => u64::from(bytes[0]),
        2 => u64::from(u16::from_ne_bytes([bytes[0], bytes[1]])),
        4 => u64::from(u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        8 => u64::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]),
        _ => 0,
    }
}

fn put_ne_uint(bytes: &mut [u8], v: u64) {
    match bytes.len() {
        1 => bytes[0] = v as u8,
        2 => bytes.copy_from_slice(&(v as u16).to_ne_bytes()),
        4 => bytes.copy_from_slice(&(v as u32).to_ne_bytes()),
        8 => bytes.copy_from_slice(&v.to_ne_bytes()),
        _ => {}
    }
}

fn ne_int(bytes: &[u8], signed: bool) -> i128 {
    let u = ne_uint(bytes);
    if !signed {
        return i128::from(u);
    }
    match bytes.len() {
        1 => i128::from(u as u8 as i8),
        2 => i128::from(u as u16 as i16),
        4 => i128::from(u as u32 as i32),
        _ => i128::from(u as i64),
    }
}

fn read_i64(bytes: &[u8]) -> i64 {
    i64::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]])
}
fn read_i32(bytes: &[u8]) -> i32 {
    i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

impl TypeDesc {
    /// A descriptor of `kind`.
    pub fn new(kind: Kind) -> Self {
        Self { kind, doc: String::new() }
    }
    /// Sets the documentation.
    pub fn with_doc(mut self, doc: &str) -> Self {
        self.doc = doc.trim().to_string();
        self
    }

    /// A flag stored in `width` bytes (1 or 4).
    pub fn bool_of_width(width: usize) -> Self {
        Self::new(Kind::Bool { width })
    }
    /// An integer.
    pub fn int(int: IntKind) -> Self {
        Self::new(Kind::Int { int, range: Range::default() })
    }
    /// `FP`.
    pub fn fixed() -> Self {
        Self::new(Kind::Fixed { range: Range::default() })
    }
    /// `FP32`.
    pub fn fixed32() -> Self {
        Self::new(Kind::Fixed32 { range: Range::default() })
    }
    /// `FPVec2`.
    pub fn vec2() -> Self {
        Self::new(Kind::Vec2 { range: Range::default() })
    }
    /// `FPVec3`.
    pub fn vec3() -> Self {
        Self::new(Kind::Vec3 { range: Range::default() })
    }
    /// `Entity`.
    pub fn entity() -> Self {
        Self::new(Kind::Entity)
    }
    /// A unit enum in `width` bytes.
    pub fn enumeration(width: usize, variants: &[(&str, u64)]) -> Self {
        Self::new(Kind::Enum { width, variants: variants.iter().map(|(n, v)| ((*n).to_string(), *v)).collect() })
    }
    /// Named bits in `width` bytes.
    pub fn flags(width: usize, bits: &[(&str, u64)]) -> Self {
        Self::new(Kind::Flags { width, bits: bits.iter().map(|(n, v)| ((*n).to_string(), *v)).collect() })
    }
    /// A fixed size array.
    pub fn array(elem: TypeDesc, len: usize) -> Self {
        Self::new(Kind::Array { elem: Box::new(elem), len })
    }
    /// A struct layout of `size` bytes.
    pub fn structure(size: usize, fields: Vec<FieldDesc>) -> Self {
        Self::new(Kind::Struct { size, fields })
    }
    /// A hand-written tagged view.
    pub fn tagged(t: TaggedDesc) -> Self {
        Self::new(Kind::Tagged(Box::new(t)))
    }
    /// A variable length list (inside tagged views only).
    pub fn list(elem: TypeDesc, min: usize, max: usize) -> Self {
        Self::new(Kind::List { elem: Box::new(elem), min, max })
    }

    /// Size in bytes of the bytes this type covers (0 for a list).
    pub fn size(&self) -> usize {
        match &self.kind {
            Kind::Bool { width } | Kind::Enum { width, .. } | Kind::Flags { width, .. } => *width,
            Kind::Int { int, .. } => int.size(),
            Kind::Fixed { .. } | Kind::Entity => 8,
            Kind::Fixed32 { .. } => 4,
            Kind::Vec2 { .. } => 16,
            Kind::Vec3 { .. } => 24,
            Kind::Array { elem, len } => elem.size() * len,
            Kind::Struct { size, .. } => *size,
            Kind::Tagged(t) => t.size,
            Kind::List { .. } => 0,
        }
    }

    /// Restricts numeric leaves to `"min..=max"` in decimal text. Either bound
    /// may be left out (`"..=1000"`, `"0.05..="`). Applies to the elements of
    /// arrays and lists and to the components of vectors.
    ///
    /// Panics on invalid text or on a type without numbers: that is a bug in
    /// the type's descriptor, and it shows in the first test that builds it.
    pub fn with_range_str(mut self, text: &str) -> Self {
        let (lo, hi) = text.split_once("..=").unwrap_or_else(|| panic!("orr_reflect: range {text:?} must look like \"min..=max\""));
        self.apply_range(lo.trim(), hi.trim(), text);
        self
    }

    fn apply_range(&mut self, lo: &str, hi: &str, whole: &str) {
        let fixed = |s: &str| -> Option<i128> {
            if s.is_empty() {
                None
            } else {
                Some(i128::from(decimal::parse_fp(s).unwrap_or_else(|_| panic!("orr_reflect: bad number in range {whole:?}")).raw()))
            }
        };
        match &mut self.kind {
            Kind::Int { range, .. } => {
                let int = |s: &str| -> Option<i128> {
                    if s.is_empty() {
                        None
                    } else {
                        Some(decimal::parse_int(s).unwrap_or_else(|_| panic!("orr_reflect: bad integer in range {whole:?}")))
                    }
                };
                *range = Range { min: int(lo), max: int(hi) };
            }
            Kind::Fixed { range } | Kind::Fixed32 { range } | Kind::Vec2 { range } | Kind::Vec3 { range } => {
                *range = Range { min: fixed(lo), max: fixed(hi) };
            }
            Kind::Array { elem, .. } | Kind::List { elem, .. } => elem.apply_range(lo, hi, whole),
            _ => panic!("orr_reflect: range {whole:?} on a type without numbers"),
        }
    }

    /// Checks that every field lies inside `total` bytes and that fields do
    /// not overlap. Call once per registered type.
    pub fn check_layout(&self, total: usize) -> Result<(), String> {
        if self.size() > total {
            return Err(format!("descriptor covers {} bytes, the type has {total}", self.size()));
        }
        match &self.kind {
            Kind::Struct { fields, .. } => {
                let mut spans: Vec<(usize, usize, &str)> = Vec::new();
                for f in fields {
                    let end = f.offset + f.ty.size();
                    if end > total {
                        return Err(format!("field '{}' ends at byte {end}, the struct has {total}", f.name));
                    }
                    f.ty.check_layout(f.ty.size())?;
                    spans.push((f.offset, end, &f.name));
                }
                spans.sort();
                for w in spans.windows(2) {
                    if w[0].1 > w[1].0 {
                        return Err(format!("fields '{}' and '{}' overlap", w[0].2, w[1].2));
                    }
                }
                Ok(())
            }
            Kind::Array { elem, .. } => elem.check_layout(elem.size()),
            _ => Ok(()),
        }
    }

    // ---- reading ----

    /// Reads the value from `bytes` (at least [`size`](Self::size) bytes).
    pub fn read(&self, bytes: &[u8]) -> Value {
        match &self.kind {
            Kind::Bool { width } => Value::Bool(ne_uint(&bytes[..*width]) != 0),
            Kind::Int { int, .. } => {
                Value::Int(ne_int(&bytes[..int.size()], matches!(int, IntKind::I8 | IntKind::I16 | IntKind::I32 | IntKind::I64)))
            }
            Kind::Fixed { .. } => Value::Fixed(FP::from_raw(read_i64(bytes))),
            Kind::Fixed32 { .. } => Value::Fixed32(FP32::from_raw(read_i32(bytes))),
            Kind::Vec2 { .. } => Value::Vec2(FPVec2::new(FP::from_raw(read_i64(&bytes[0..])), FP::from_raw(read_i64(&bytes[8..])))),
            Kind::Vec3 { .. } => Value::Vec3(FPVec3 {
                x: FP::from_raw(read_i64(&bytes[0..])),
                y: FP::from_raw(read_i64(&bytes[8..])),
                z: FP::from_raw(read_i64(&bytes[16..])),
            }),
            Kind::Entity => Value::Entity(Entity { index: read_u32(bytes), version: read_u32(&bytes[4..]) }),
            Kind::Enum { width, variants } => {
                let raw = ne_uint(&bytes[..*width]);
                match variants.iter().find(|(_, v)| *v == raw) {
                    Some((n, _)) => Value::Enum(n.clone()),
                    None => Value::Enum(format!("unknown({raw})")),
                }
            }
            Kind::Flags { width, bits } => {
                let raw = ne_uint(&bytes[..*width]);
                let mut names = Vec::new();
                let mut left = raw;
                for (n, m) in bits {
                    if raw & m == *m && *m != 0 {
                        names.push(n.clone());
                        left &= !m;
                    }
                }
                if left != 0 {
                    names.push(format!("unknown(0x{left:x})"));
                }
                Value::Flags(names)
            }
            Kind::Array { elem, len } => {
                let stride = elem.size();
                Value::Array((0..*len).map(|i| elem.read(&bytes[i * stride..])).collect())
            }
            Kind::Struct { fields, .. } => {
                Value::Struct(fields.iter().map(|f| (f.name.clone(), f.ty.read(&bytes[f.offset..]))).collect())
            }
            Kind::Tagged(t) => (t.read)(&bytes[..t.size]),
            Kind::List { .. } => Value::Array(Vec::new()),
        }
    }

    // ---- checking ----

    /// Checks that `v` is a valid value of this type. With `ranges` the
    /// documented numeric ranges apply too.
    pub fn check(&self, v: &Value, ranges: bool) -> Result<(), ReflectError> {
        let mismatch = |found: &Value| ReflectError::TypeMismatch { expected: self.expected_name(), found: found.kind_name().to_string() };
        match (&self.kind, v) {
            (Kind::Bool { .. }, Value::Bool(_)) | (Kind::Entity, Value::Entity(_) | Value::EntityGuid(_)) => Ok(()),
            (Kind::Int { int, range }, Value::Int(n)) => {
                if *n < int.min() || *n > int.max() {
                    return Err(ReflectError::OutOfRange(format!("{n} does not fit {}", int.name())));
                }
                if ranges && !range.contains(*n) {
                    return Err(ReflectError::OutOfRange(format!("{n} is outside {}", range_text(range, false))));
                }
                Ok(())
            }
            (Kind::Fixed { range }, Value::Fixed(x)) => check_fixed(i128::from(x.raw()), range, ranges),
            (Kind::Fixed32 { range }, Value::Fixed32(x)) => check_fixed(i128::from(x.raw()), range, ranges),
            (Kind::Vec2 { range }, Value::Vec2(x)) => {
                check_fixed(i128::from(x.x.raw()), range, ranges).map_err(|e| e.at("x"))?;
                check_fixed(i128::from(x.y.raw()), range, ranges).map_err(|e| e.at("y"))
            }
            (Kind::Vec3 { range }, Value::Vec3(x)) => {
                check_fixed(i128::from(x.x.raw()), range, ranges).map_err(|e| e.at("x"))?;
                check_fixed(i128::from(x.y.raw()), range, ranges).map_err(|e| e.at("y"))?;
                check_fixed(i128::from(x.z.raw()), range, ranges).map_err(|e| e.at("z"))
            }
            (Kind::Enum { variants, .. }, Value::Enum(name)) => {
                if variants.iter().any(|(n, _)| n == name) {
                    Ok(())
                } else {
                    Err(ReflectError::Invalid(format!(
                        "'{name}' is not one of {}",
                        variants.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
                    )))
                }
            }
            (Kind::Flags { bits, .. }, Value::Flags(names)) => {
                for (i, n) in names.iter().enumerate() {
                    if !bits.iter().any(|(b, _)| b == n) {
                        return Err(ReflectError::Invalid(format!(
                            "'{n}' is not one of {}",
                            bits.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
                        )));
                    }
                    if names[..i].contains(n) {
                        return Err(ReflectError::Invalid(format!("'{n}' is listed twice")));
                    }
                }
                Ok(())
            }
            (Kind::Array { elem, len }, Value::Array(items)) => {
                if items.len() != *len {
                    return Err(ReflectError::Invalid(format!("expected {len} elements, found {}", items.len())));
                }
                for (i, it) in items.iter().enumerate() {
                    elem.check(it, ranges).map_err(|e| e.at(&format!("[{i}]")))?;
                }
                Ok(())
            }
            (Kind::List { elem, min, max }, Value::Array(items)) => {
                if items.len() < *min || items.len() > *max {
                    return Err(ReflectError::Invalid(format!("expected {min} to {max} elements, found {}", items.len())));
                }
                for (i, it) in items.iter().enumerate() {
                    elem.check(it, ranges).map_err(|e| e.at(&format!("[{i}]")))?;
                }
                Ok(())
            }
            (Kind::Struct { fields, .. }, Value::Struct(vals)) => {
                check_fields(fields.iter().map(|f| (f.name.as_str(), &f.ty)), vals, ranges)
            }
            (Kind::Tagged(t), Value::Variant(name, vals)) => {
                let Some(var) = t.variants.iter().find(|x| &x.name == name) else {
                    return Err(ReflectError::Invalid(format!(
                        "'{name}' is not one of {}",
                        t.variants.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", ")
                    )));
                };
                check_fields(var.fields.iter().map(|f| (f.name.as_str(), &f.ty)), vals, ranges)
            }
            (_, other) => Err(mismatch(other)),
        }
    }

    /// Name of the expected value, for messages.
    pub fn expected_name(&self) -> String {
        match &self.kind {
            Kind::Bool { .. } => "bool".into(),
            Kind::Int { int, .. } => format!("integer ({})", int.name()),
            Kind::Fixed { .. } | Kind::Fixed32 { .. } => "number".into(),
            Kind::Vec2 { .. } => "vec2".into(),
            Kind::Vec3 { .. } => "vec3".into(),
            Kind::Entity => "entity".into(),
            Kind::Enum { .. } => "enum".into(),
            Kind::Flags { .. } => "flags".into(),
            Kind::Array { .. } | Kind::List { .. } => "array".into(),
            Kind::Struct { .. } => "struct".into(),
            Kind::Tagged(_) => "tagged value".into(),
        }
    }

    // ---- writing ----

    /// Checks `v` (with ranges) and writes it into `bytes`. For a struct only
    /// the visible fields are written; other bytes stay as they are.
    pub fn write(&self, bytes: &mut [u8], v: &Value) -> Result<(), ReflectError> {
        self.check(v, true)?;
        self.write_checked(bytes, v)
    }

    /// Like [`write`](Self::write) without the range check (structure only).
    pub fn write_unranged(&self, bytes: &mut [u8], v: &Value) -> Result<(), ReflectError> {
        self.check(v, false)?;
        self.write_checked(bytes, v)
    }

    fn write_checked(&self, bytes: &mut [u8], v: &Value) -> Result<(), ReflectError> {
        match (&self.kind, v) {
            (Kind::Bool { width }, Value::Bool(b)) => put_ne_uint(&mut bytes[..*width], u64::from(*b)),
            (Kind::Int { int, .. }, Value::Int(n)) => put_ne_uint(&mut bytes[..int.size()], *n as u64),
            (Kind::Fixed { .. }, Value::Fixed(x)) => bytes[..8].copy_from_slice(&x.raw().to_ne_bytes()),
            (Kind::Fixed32 { .. }, Value::Fixed32(x)) => bytes[..4].copy_from_slice(&x.raw().to_ne_bytes()),
            (Kind::Vec2 { .. }, Value::Vec2(x)) => {
                bytes[0..8].copy_from_slice(&x.x.raw().to_ne_bytes());
                bytes[8..16].copy_from_slice(&x.y.raw().to_ne_bytes());
            }
            (Kind::Vec3 { .. }, Value::Vec3(x)) => {
                bytes[0..8].copy_from_slice(&x.x.raw().to_ne_bytes());
                bytes[8..16].copy_from_slice(&x.y.raw().to_ne_bytes());
                bytes[16..24].copy_from_slice(&x.z.raw().to_ne_bytes());
            }
            (Kind::Entity, Value::Entity(e)) => {
                bytes[0..4].copy_from_slice(&e.index.to_ne_bytes());
                bytes[4..8].copy_from_slice(&e.version.to_ne_bytes());
            }
            (Kind::Enum { width, variants }, Value::Enum(name)) => {
                let raw = variants.iter().find(|(n, _)| n == name).map_or(0, |(_, v)| *v);
                put_ne_uint(&mut bytes[..*width], raw);
            }
            (Kind::Flags { width, bits }, Value::Flags(names)) => {
                let raw = names.iter().fold(0u64, |acc, n| acc | bits.iter().find(|(b, _)| b == n).map_or(0, |(_, m)| *m));
                put_ne_uint(&mut bytes[..*width], raw);
            }
            (Kind::Array { elem, .. }, Value::Array(items)) => {
                let stride = elem.size();
                for (i, it) in items.iter().enumerate() {
                    elem.write_checked(&mut bytes[i * stride..], it).map_err(|e| e.at(&format!("[{i}]")))?;
                }
            }
            (Kind::Struct { fields, .. }, Value::Struct(vals)) => {
                for f in fields {
                    if let Some((_, fv)) = vals.iter().find(|(n, _)| *n == f.name) {
                        f.ty.write_checked(&mut bytes[f.offset..], fv).map_err(|e| e.at(&f.name))?;
                    }
                }
            }
            (Kind::Entity, Value::EntityGuid(_)) => {
                return Err(ReflectError::Invalid("entity reference by GUID must be resolved before writing".into()));
            }
            (Kind::Tagged(t), v @ Value::Variant(..)) => {
                (t.write)(&mut bytes[..t.size], v).map_err(ReflectError::Invalid)?;
            }
            _ => return Err(ReflectError::Invalid("value does not match the type".into())),
        }
        Ok(())
    }

    // ---- paths ----

    /// Reads the sub-value at `path` (empty = whole value).
    pub fn get(&self, bytes: &[u8], path: &[PathSeg]) -> Result<Value, ReflectError> {
        let whole = self.read(bytes);
        get_in_value(self, &whole, path).cloned().or_else(|e| match e {
            // `kind` of a tagged value is not stored as a field.
            ReflectError::BadPath(_) => kind_of(self, &whole, path).ok_or(e),
            other => Err(other),
        })
    }

    /// Writes `new` at `path` (empty = whole value). Checks `new` against the
    /// sub-type first. Touches only the bytes of that sub-value.
    pub fn set(&self, bytes: &mut [u8], path: &[PathSeg], new: Value) -> Result<(), ReflectError> {
        let Some((first, rest)) = path.split_first() else {
            return self.write(bytes, &new);
        };
        match (&self.kind, first) {
            (Kind::Struct { fields, .. }, PathSeg::Field(name)) => {
                let f = fields.iter().find(|f| &f.name == name).ok_or_else(|| no_field(name, self))?;
                f.ty.set(&mut bytes[f.offset..], rest, new).map_err(|e| e.at(name))
            }
            (Kind::Array { elem, len }, PathSeg::Index(i)) => {
                if i >= len {
                    return Err(ReflectError::BadPath(format!("index {i} is out of range for {len} elements")));
                }
                elem.set(&mut bytes[i * elem.size()..], rest, new).map_err(|e| e.at(&format!("[{i}]")))
            }
            (Kind::Struct { .. } | Kind::Array { .. }, _) => Err(ReflectError::BadPath("expected a field name or an index".into())),
            _ => {
                let mut whole = self.read(bytes);
                set_in_value(self, &mut whole, path, new)?;
                self.write(bytes, &whole)
            }
        }
    }
}

fn no_field(name: &str, desc: &TypeDesc) -> ReflectError {
    let names: Vec<&str> = match &desc.kind {
        Kind::Struct { fields, .. } => fields.iter().map(|f| f.name.as_str()).collect(),
        _ => Vec::new(),
    };
    ReflectError::BadPath(format!("no field '{name}' (fields: {})", names.join(", ")))
}

fn check_fixed(raw: i128, range: &Range, ranges: bool) -> Result<(), ReflectError> {
    if ranges && !range.contains(raw) {
        return Err(ReflectError::OutOfRange(format!(
            "{} is outside {}",
            decimal::fp_to_decimal(FP::from_raw(raw as i64)),
            range_text(range, true)
        )));
    }
    Ok(())
}

/// `[min, max]` text of a range; fixed-point bounds print as decimals.
pub fn range_text(range: &Range, fixed: bool) -> String {
    let b = |v: Option<i128>| match v {
        None => "..".to_string(),
        Some(x) if fixed => decimal::fp_to_decimal(FP::from_raw(x as i64)),
        Some(x) => x.to_string(),
    };
    format!("[{}, {}]", b(range.min), b(range.max))
}

fn check_fields<'a>(
    fields: impl Iterator<Item = (&'a str, &'a TypeDesc)> + Clone,
    vals: &[(String, Value)],
    ranges: bool,
) -> Result<(), ReflectError> {
    for (name, _) in vals {
        if !fields.clone().any(|(n, _)| n == name) {
            return Err(ReflectError::Invalid(format!("unknown field '{name}'")));
        }
    }
    for (name, ty) in fields {
        let Some((_, v)) = vals.iter().find(|(n, _)| n == name) else {
            return Err(ReflectError::Invalid(format!("missing field '{name}'")));
        };
        ty.check(v, ranges).map_err(|e| e.at(name))?;
    }
    Ok(())
}

impl ReflectError {
    /// Prefixes the location of a nested problem: `x: 3 is outside ..` becomes
    /// `pos.x: 3 is outside ..`.
    pub fn at(self, seg: &str) -> ReflectError {
        match self {
            ReflectError::OutOfRange(m) => ReflectError::OutOfRange(join_path(seg, &m)),
            ReflectError::Invalid(m) => ReflectError::Invalid(join_path(seg, &m)),
            ReflectError::TypeMismatch { expected, found } => {
                ReflectError::Invalid(join_path(seg, &format!("expected {expected}, found {found}")))
            }
            other => other,
        }
    }
}

fn join_path(seg: &str, msg: &str) -> String {
    if let Some((path, rest)) = msg.split_once(": ") {
        if !path.is_empty() && path.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'[' | b']')) {
            let sep = if path.starts_with('[') { "" } else { "." };
            return format!("{seg}{sep}{path}: {rest}");
        }
    }
    format!("{seg}: {msg}")
}

// ---- value-level navigation (used for tagged views and for `get`) ----

fn get_in_value<'a>(desc: &TypeDesc, v: &'a Value, path: &[PathSeg]) -> Result<&'a Value, ReflectError> {
    let Some((first, rest)) = path.split_first() else {
        return Ok(v);
    };
    match (&desc.kind, v, first) {
        (Kind::Struct { fields, .. }, Value::Struct(vals), PathSeg::Field(name)) => {
            let f = fields.iter().find(|f| &f.name == name).ok_or_else(|| no_field(name, desc))?;
            let (_, fv) = vals.iter().find(|(n, _)| n == name).ok_or_else(|| no_field(name, desc))?;
            get_in_value(&f.ty, fv, rest)
        }
        (Kind::Tagged(t), Value::Variant(vname, vals), PathSeg::Field(name)) => {
            let var = t.variants.iter().find(|x| &x.name == vname).ok_or_else(|| ReflectError::BadPath(format!("unknown variant {vname}")))?;
            let f = var.fields.iter().find(|f| &f.name == name).ok_or_else(|| {
                ReflectError::BadPath(format!("variant '{vname}' has no field '{name}' (fields: {})", var.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")))
            })?;
            let (_, fv) = vals.iter().find(|(n, _)| n == name).ok_or_else(|| ReflectError::BadPath(format!("no field {name}")))?;
            get_in_value(&f.ty, fv, rest)
        }
        (Kind::Array { elem, .. } | Kind::List { elem, .. }, Value::Array(items), PathSeg::Index(i)) => {
            let it = items.get(*i).ok_or_else(|| ReflectError::BadPath(format!("index {i} is out of range for {} elements", items.len())))?;
            get_in_value(elem, it, rest)
        }
        (Kind::Vec2 { .. }, Value::Vec2(_), _) | (Kind::Vec3 { .. }, Value::Vec3(_), _) => {
            Err(ReflectError::BadPath("vector component read is handled by `component`".into()))
        }
        _ => Err(ReflectError::BadPath(format!("cannot look up {first:?} in a {}", v.kind_name()))),
    }
}

/// Reads through vector components and the `kind` key, which `get_in_value`
/// cannot return by reference.
fn kind_of(desc: &TypeDesc, v: &Value, path: &[PathSeg]) -> Option<Value> {
    let (first, rest) = path.split_first()?;
    match (&desc.kind, v, first) {
        (Kind::Vec2 { .. }, Value::Vec2(x), PathSeg::Field(n)) if rest.is_empty() => match n.as_str() {
            "x" => Some(Value::Fixed(x.x)),
            "y" => Some(Value::Fixed(x.y)),
            _ => None,
        },
        (Kind::Vec3 { .. }, Value::Vec3(x), PathSeg::Field(n)) if rest.is_empty() => match n.as_str() {
            "x" => Some(Value::Fixed(x.x)),
            "y" => Some(Value::Fixed(x.y)),
            "z" => Some(Value::Fixed(x.z)),
            _ => None,
        },
        (Kind::Tagged(_), Value::Variant(name, _), PathSeg::Field(n)) if n == "kind" && rest.is_empty() => Some(Value::Enum(name.clone())),
        (Kind::Struct { fields, .. }, Value::Struct(vals), PathSeg::Field(name)) => {
            let f = fields.iter().find(|f| &f.name == name)?;
            let (_, fv) = vals.iter().find(|(n, _)| n == name)?;
            kind_of(&f.ty, fv, rest)
        }
        (Kind::Tagged(t), Value::Variant(vname, vals), PathSeg::Field(name)) => {
            let var = t.variants.iter().find(|x| &x.name == vname)?;
            let f = var.fields.iter().find(|f| &f.name == name)?;
            let (_, fv) = vals.iter().find(|(n, _)| n == name)?;
            kind_of(&f.ty, fv, rest)
        }
        (Kind::Array { elem, .. } | Kind::List { elem, .. }, Value::Array(items), PathSeg::Index(i)) => kind_of(elem, items.get(*i)?, rest),
        _ => None,
    }
}

fn set_in_value(desc: &TypeDesc, v: &mut Value, path: &[PathSeg], new: Value) -> Result<(), ReflectError> {
    let Some((first, rest)) = path.split_first() else {
        desc.check(&new, true)?;
        *v = new;
        return Ok(());
    };
    match (&desc.kind, v, first) {
        (Kind::Struct { fields, .. }, Value::Struct(vals), PathSeg::Field(name)) => {
            let f = fields.iter().find(|f| &f.name == name).ok_or_else(|| no_field(name, desc))?;
            let slot = vals.iter_mut().find(|(n, _)| n == name).ok_or_else(|| no_field(name, desc))?;
            set_in_value(&f.ty, &mut slot.1, rest, new).map_err(|e| e.at(name))
        }
        (Kind::Tagged(t), slot @ Value::Variant(..), PathSeg::Field(name)) => {
            let Value::Variant(vname, vals) = slot else { unreachable!() };
            if name == "kind" && rest.is_empty() {
                let Value::Enum(target) = &new else {
                    return Err(ReflectError::TypeMismatch { expected: "variant name".into(), found: new.kind_name().into() });
                };
                let var = t.variants.iter().find(|x| &x.name == target).ok_or_else(|| {
                    ReflectError::Invalid(format!("'{target}' is not one of {}", t.variants.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", ")))
                })?;
                *slot = Value::Variant(var.name.clone(), var.default.clone());
                return Ok(());
            }
            let var = t.variants.iter().find(|x| &x.name == vname).ok_or_else(|| ReflectError::BadPath(format!("unknown variant {vname}")))?;
            let f = var.fields.iter().find(|f| &f.name == name).ok_or_else(|| {
                ReflectError::BadPath(format!("variant '{vname}' has no field '{name}' (fields: {})", var.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")))
            })?;
            let entry = vals.iter_mut().find(|(n, _)| n == name).ok_or_else(|| ReflectError::BadPath(format!("no field {name}")))?;
            set_in_value(&f.ty, &mut entry.1, rest, new).map_err(|e| e.at(name))
        }
        (Kind::Array { elem, .. } | Kind::List { elem, .. }, Value::Array(items), PathSeg::Index(i)) => {
            let len = items.len();
            let it = items.get_mut(*i).ok_or_else(|| ReflectError::BadPath(format!("index {i} is out of range for {len} elements")))?;
            set_in_value(elem, it, rest, new).map_err(|e| e.at(&format!("[{i}]")))
        }
        (Kind::Vec2 { range }, Value::Vec2(x), PathSeg::Field(n)) if rest.is_empty() => {
            let Value::Fixed(f) = new else {
                return Err(ReflectError::TypeMismatch { expected: "number".into(), found: new.kind_name().into() });
            };
            check_fixed(i128::from(f.raw()), range, true).map_err(|e| e.at(n))?;
            match n.as_str() {
                "x" => x.x = f,
                "y" => x.y = f,
                _ => return Err(ReflectError::BadPath(format!("a vec2 has fields x and y, not '{n}'"))),
            }
            Ok(())
        }
        (Kind::Vec3 { range }, Value::Vec3(x), PathSeg::Field(n)) if rest.is_empty() => {
            let Value::Fixed(f) = new else {
                return Err(ReflectError::TypeMismatch { expected: "number".into(), found: new.kind_name().into() });
            };
            check_fixed(i128::from(f.raw()), range, true).map_err(|e| e.at(n))?;
            match n.as_str() {
                "x" => x.x = f,
                "y" => x.y = f,
                "z" => x.z = f,
                _ => return Err(ReflectError::BadPath(format!("a vec3 has fields x, y and z, not '{n}'"))),
            }
            Ok(())
        }
        _ => Err(ReflectError::BadPath(format!("cannot look up {first:?} in a {}", desc.expected_name()))),
    }
}
