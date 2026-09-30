//! The `Schema` message: what a foreign view needs to know about a game
//! before it reads its first frame. JSON text, sent once per connection
//! (see `docs/view-stream.md` for every key).

use orr_reflect::{IntKind, Kind, Reflect, TypeDesc};
use serde_json::{json, Map, Value as J};

use crate::format::{HEADER_LEN, RECORD_LEN, VERSION};

/// The type of one custom property word (always 4 bytes, little-endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropType {
    /// IEEE 754 binary32.
    F32,
    /// Unsigned 32-bit integer.
    U32,
    /// Signed 32-bit integer.
    I32,
}

impl PropType {
    fn name(self) -> &'static str {
        match self {
            PropType::F32 => "f32",
            PropType::U32 => "u32",
            PropType::I32 => "i32",
        }
    }
}

/// One custom property of an entity kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropDef {
    /// Name, for the foreign view's own use.
    pub name: String,
    /// Word type.
    pub ty: PropType,
}

/// A kind of entity (the game's vocabulary: "paddle", "ball", "wall").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindDef {
    /// The id the records carry.
    pub id: u16,
    /// Name.
    pub name: String,
    /// Custom properties, in the order their words follow in a frame's property section.
    pub props: Vec<PropDef>,
}

impl KindDef {
    /// A kind with no properties.
    pub fn new(id: u16, name: &str) -> Self {
        Self { id, name: name.to_string(), props: Vec::new() }
    }

    /// Adds a property.
    pub fn with_prop(mut self, name: &str, ty: PropType) -> Self {
        self.props.push(PropDef { name: name.to_string(), ty });
        self
    }
}

/// A type of sim event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDef {
    /// The id the event records carry.
    pub id: u16,
    /// Name.
    pub name: String,
    /// Payload size in bytes.
    pub payload_size: usize,
}

/// The byte layout of the game's input type: what a foreign view writes for
/// `set_input`. Built from the `orr_reflect` descriptor of the type, so the
/// offsets are the real ones.
#[derive(Clone, Debug, PartialEq)]
pub struct InputLayout {
    /// Total size in bytes (the input must be exactly this long).
    pub size: usize,
    /// The fields, as JSON objects (`name`, `type`, `offset`, `size`, and
    /// type-specific keys).
    pub fields: Vec<J>,
}

/// The layout of `T` from its reflection descriptor. Panics if `T` is not a
/// struct (an input type is always one).
pub fn input_layout_of<T: Reflect>() -> InputLayout {
    let desc = T::describe();
    let Kind::Struct { size, fields } = &desc.kind else {
        panic!("an input type must describe itself as a struct");
    };
    InputLayout { size: *size, fields: fields.iter().map(|f| field_json(&f.name, f.offset, &f.ty, &f.doc)).collect() }
}

fn int_name(i: IntKind) -> &'static str {
    i.name()
}

fn field_json(name: &str, offset: usize, ty: &TypeDesc, doc: &str) -> J {
    let mut m = Map::new();
    m.insert("name".into(), json!(name));
    m.insert("offset".into(), json!(offset));
    m.insert("size".into(), json!(ty.size()));
    let type_name = match &ty.kind {
        Kind::Bool { .. } => "bool".to_string(),
        Kind::Int { int, .. } => int_name(*int).to_string(),
        // Q48.16: an i64 holding value * 65536.
        Kind::Fixed { .. } => "fixed".to_string(),
        // Q16.16: an i32 holding value * 65536.
        Kind::Fixed32 { .. } => "fixed32".to_string(),
        Kind::Vec2 { .. } => "fixed_vec2".to_string(),
        Kind::Vec3 { .. } => "fixed_vec3".to_string(),
        Kind::Entity => "entity".to_string(),
        Kind::Enum { variants, .. } => {
            let mut v = Map::new();
            for (n, x) in variants {
                v.insert(n.clone(), json!(x));
            }
            m.insert("values".into(), J::Object(v));
            "enum".to_string()
        }
        Kind::Flags { bits, .. } => {
            let mut v = Map::new();
            for (n, x) in bits {
                v.insert(n.clone(), json!(x));
            }
            m.insert("bits".into(), J::Object(v));
            "flags".to_string()
        }
        Kind::Array { elem, len } => {
            m.insert("len".into(), json!(len));
            m.insert("elem".into(), field_json("", 0, elem, ""));
            "array".to_string()
        }
        Kind::Struct { fields, .. } => {
            m.insert("fields".into(), J::Array(fields.iter().map(|f| field_json(&f.name, f.offset, &f.ty, &f.doc)).collect()));
            "struct".to_string()
        }
        Kind::Tagged(_) | Kind::List { .. } => "opaque".to_string(),
    };
    m.insert("type".into(), json!(type_name));
    if !doc.is_empty() {
        m.insert("doc".into(), json!(doc));
    }
    J::Object(m)
}

/// Everything a foreign view learns once per connection.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    /// Name of the game.
    pub game: String,
    /// Build id of the simulation (two hosts of the same build id simulate identically).
    pub build_id: u64,
    /// Sim ticks per second.
    pub tick_rate: u32,
    /// Players of the session (input slots `0..player_count`).
    pub player_count: u8,
    /// Entity kinds.
    pub kinds: Vec<KindDef>,
    /// The input layout.
    pub input: InputLayout,
    /// Size in bytes of the game's command encoding (0 = the game has none).
    pub command_size: usize,
    /// Event types.
    pub events: Vec<EventDef>,
}

impl Schema {
    /// How many 32-bit property words an entity of `kind` has (0 for an unknown kind).
    pub fn props_words(&self, kind: u16) -> usize {
        self.kinds.iter().find(|k| k.id == kind).map_or(0, |k| k.props.len())
    }

    /// The schema as a JSON value.
    pub fn to_json(&self) -> J {
        json!({
            "format": "orrery.viewstream",
            "version": VERSION,
            "game": self.game,
            "build_id": format!("0x{:016x}", self.build_id),
            "tick_rate": self.tick_rate,
            "player_count": self.player_count,
            "endian": "little",
            "world": {"units": "game units, y up", "rotation": "radians, counter-clockwise"},
            "fixed_point": {"type": "q48.16", "raw": "i64", "frac_bits": 16, "note": "input fields of type `fixed` hold value * 65536"},
            "frame": {
                "magic": "OVS1",
                "header_len": HEADER_LEN,
                "record_len": RECORD_LEN,
                "shapes": ["circle", "quad", "capsule"],
                "interp_modes": ["prediction", "snapshot", "none"],
                "flags": {"rolled_back": 1, "discontinuity": 2, "paused": 4},
            },
            "kinds": self.kinds.iter().map(|k| json!({
                "id": k.id,
                "name": k.name,
                "props": k.props.iter().map(|p| json!({"name": p.name, "type": p.ty.name()})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "input": {"size": self.input.size, "fields": self.input.fields},
            "command": {"size": self.command_size},
            "events": self.events.iter().map(|e| json!({"id": e.id, "name": e.name, "payload_size": e.payload_size})).collect::<Vec<_>>(),
        })
    }

    /// The schema as compact JSON text.
    pub fn to_json_string(&self) -> String {
        self.to_json().to_string()
    }
}
