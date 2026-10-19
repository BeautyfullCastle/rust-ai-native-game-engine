//! Bounded, versioned particle authoring documents. No simulation state or renderer.
//!
//! File operations require a trusted directory tree: static final-component symlinks
//! and nonregular inputs are rejected, but parent symlinks and concurrent path
//! replacement are not sandboxed. Save As uses `create_new`, never overwrites an
//! existing target, and syncs the new file; it is not a crash-atomic publication
//! protocol. A failed write or crash may leave a partial new target for the user
//! to inspect/remove. Neither operation modifies the source document.
use orr_view::{
    particles::{EmitterDesc, ParticleBlend},
    Vec3,
};
use serde::{
    de::{self, MapAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub const MAX_PRESET_BYTES: usize = 64 * 1024;
pub const PRESET_SCHEMA: u32 = 1;
pub const TIMED_PRESET_SCHEMA: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetBlend {
    Alpha,
    Additive,
}

/// Normalized authoring data. Schema 1 omits timed fields on the wire and requires
/// them to be zero in memory; schema 2 requires both fields, even when zero.
/// Neither schema stores runtime event identity, origin, or seed.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticlePreset {
    pub schema: u32,
    pub burst: u32,
    pub rate: f32,
    pub duration: f32,
    pub lifetime: f32,
    pub velocity: [f32; 3],
    pub velocity_jitter: [f32; 3],
    pub size_start: f32,
    pub size_end: f32,
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub blend: PresetBlend,
}
impl Default for ParticlePreset {
    fn default() -> Self {
        Self {
            schema: PRESET_SCHEMA,
            burst: 16,
            rate: 0.0,
            duration: 0.0,
            lifetime: 0.5,
            velocity: [0.0, 1.0, 0.0],
            velocity_jitter: [1.1, 0.8, 1.1],
            size_start: 0.12,
            size_end: 0.0,
            color_start: [1.0, 0.72, 0.15, 0.9],
            color_end: [1.0, 0.3, 0.05, 0.0],
            blend: PresetBlend::Additive,
        }
    }
}

// Keep these strict wire types separate from normalized public state. In particular,
// a V1 document cannot sneak in timed fields with zero or null values.
macro_rules! wire_type {
    ($name:ident { $($extra:tt)* }) => {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $name {
            schema: u32,
            burst: u32,
            lifetime: f32,
            velocity: [f32; 3],
            velocity_jitter: [f32; 3],
            size_start: f32,
            size_end: f32,
            color_start: [f32; 4],
            color_end: [f32; 4],
            blend: PresetBlend,
            $($extra)*
        }
    };
}
wire_type!(WireV1 {});
wire_type!(WireV2 {
    rate: f32,
    duration: f32,
});
wire_type!(WireEither {
    #[serde(default, deserialize_with = "present_number")]
    rate: Option<f32>,
    #[serde(default, deserialize_with = "present_number")]
    duration: Option<f32>,
});

fn present_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    // Absence is distinct from null; null must never stand for a missing field.
    f32::deserialize(d).map(Some)
}

macro_rules! normalize_wire {
    ($wire:expr, $rate:expr, $duration:expr) => {{
        let wire = $wire;
        ParticlePreset {
            schema: wire.schema,
            burst: wire.burst,
            rate: $rate,
            duration: $duration,
            lifetime: wire.lifetime,
            velocity: wire.velocity,
            velocity_jitter: wire.velocity_jitter,
            size_start: wire.size_start,
            size_end: wire.size_end,
            color_start: wire.color_start,
            color_end: wire.color_end,
            blend: wire.blend,
        }
    }};
}

impl Serialize for ParticlePreset {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        let timed = self.schema == TIMED_PRESET_SCHEMA;
        let mut state =
            serializer.serialize_struct("ParticlePreset", if timed { 12 } else { 10 })?;
        // Preserve the original V1 serialized field order exactly.
        state.serialize_field("schema", &self.schema)?;
        state.serialize_field("burst", &self.burst)?;
        state.serialize_field("lifetime", &self.lifetime)?;
        state.serialize_field("velocity", &self.velocity)?;
        state.serialize_field("velocity_jitter", &self.velocity_jitter)?;
        state.serialize_field("size_start", &self.size_start)?;
        state.serialize_field("size_end", &self.size_end)?;
        state.serialize_field("color_start", &self.color_start)?;
        state.serialize_field("color_end", &self.color_end)?;
        state.serialize_field("blend", &self.blend)?;
        if timed {
            state.serialize_field("rate", &self.rate)?;
            state.serialize_field("duration", &self.duration)?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for ParticlePreset {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PresetVisitor;
        impl<'de> Visitor<'de> for PresetVisitor {
            type Value = ParticlePreset;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a schema 1 or 2 particle preset object")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                // Consume the original map, preserving duplicate-field detection.
                let wire = WireEither::deserialize(de::value::MapAccessDeserializer::new(map))?;
                let (rate, duration) = match wire.schema {
                    PRESET_SCHEMA if wire.rate.is_none() && wire.duration.is_none() => (0.0, 0.0),
                    PRESET_SCHEMA => {
                        return Err(de::Error::custom("schema 1 forbids rate and duration"))
                    }
                    TIMED_PRESET_SCHEMA => (
                        wire.rate.ok_or_else(|| de::Error::missing_field("rate"))?,
                        wire.duration
                            .ok_or_else(|| de::Error::missing_field("duration"))?,
                    ),
                    schema => {
                        return Err(de::Error::custom(PresetError::UnsupportedSchema(schema)))
                    }
                };
                let preset = normalize_wire!(wire, rate, duration);
                preset.validate().map_err(de::Error::custom)?;
                Ok(preset)
            }
        }
        // Unlike a derived struct deserializer, never accept positional arrays.
        deserializer.deserialize_map(PresetVisitor)
    }
}

#[derive(Debug)]
pub enum PresetError {
    Io(std::io::Error),
    Json(serde_json::Error),
    TooLarge,
    NotRegularFile,
    InvalidRoot,
    UnsupportedSchema(u32),
    InvalidEmitter,
}
impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "particle preset I/O: {error}"),
            Self::Json(error) => write!(f, "particle preset JSON: {error}"),
            Self::TooLarge => f.write_str("particle preset exceeds 64 KiB"),
            Self::InvalidRoot => f.write_str("particle preset must be a JSON object"),
            Self::NotRegularFile => {
                f.write_str("particle preset must be a regular file, not a symlink")
            }
            Self::UnsupportedSchema(schema) => {
                write!(
                    f,
                    "unsupported particle preset schema {schema}; expected 1 or 2"
                )
            }
            Self::InvalidEmitter => f.write_str("particle preset violates emitter bounds"),
        }
    }
}
impl std::error::Error for PresetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}
impl From<std::io::Error> for PresetError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for PresetError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl ParticlePreset {
    pub fn validate(&self) -> Result<(), PresetError> {
        self.to_emitter(Vec3::new(0.0, 0.0, 0.0), 0).map(|_| ())
    }
    /// Runtime supplies the event's origin and seed; authoring supplies emission timing.
    pub fn to_emitter(&self, origin: Vec3, seed: u64) -> Result<EmitterDesc, PresetError> {
        if !matches!(self.schema, PRESET_SCHEMA | TIMED_PRESET_SCHEMA) {
            return Err(PresetError::UnsupportedSchema(self.schema));
        }
        if (self.schema == PRESET_SCHEMA && (self.rate != 0.0 || self.duration != 0.0))
            || (self.burst == 0 && !(self.rate > 0.0 && self.duration > 0.0))
        {
            return Err(PresetError::InvalidEmitter);
        }
        let emitter = EmitterDesc {
            origin,
            seed,
            burst: self.burst,
            lifetime: self.lifetime,
            velocity: Vec3::new(self.velocity[0], self.velocity[1], self.velocity[2]),
            velocity_jitter: Vec3::new(
                self.velocity_jitter[0],
                self.velocity_jitter[1],
                self.velocity_jitter[2],
            ),
            rate: self.rate,
            duration: self.duration,
            size_start: self.size_start,
            size_end: self.size_end,
            color_start: self.color_start,
            color_end: self.color_end,
            blend: match self.blend {
                PresetBlend::Alpha => ParticleBlend::Alpha,
                PresetBlend::Additive => ParticleBlend::Additive,
            },
        };
        emitter
            .validate()
            .map_err(|_| PresetError::InvalidEmitter)?;
        Ok(emitter)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PresetError> {
        if bytes.len() > MAX_PRESET_BYTES {
            return Err(PresetError::TooLarge);
        }
        // Derived struct deserializers also accept positional sequences. The
        // document contract requires a named-field JSON object instead.
        if bytes
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            != Some(b'{')
        {
            return Err(PresetError::InvalidRoot);
        }
        #[derive(Deserialize)]
        struct Header {
            schema: u32,
        }
        // Dispatch is bounded by MAX_PRESET_BYTES. Reparse ORIGINAL bytes with a
        // strict version-specific type: no Value map can discard duplicate keys.
        let header: Header = serde_json::from_slice(bytes)?;
        let preset = match header.schema {
            PRESET_SCHEMA => {
                let wire: WireV1 = serde_json::from_slice(bytes)?;
                normalize_wire!(wire, 0.0, 0.0)
            }
            TIMED_PRESET_SCHEMA => {
                let wire: WireV2 = serde_json::from_slice(bytes)?;
                let (rate, duration) = (wire.rate, wire.duration);
                normalize_wire!(wire, rate, duration)
            }
            schema => return Err(PresetError::UnsupportedSchema(schema)),
        };
        preset.validate()?;
        Ok(preset)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, PresetError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_PRESET_BYTES {
            return Err(PresetError::TooLarge);
        }
        Ok(bytes)
    }
    /// Bounded regular-file read. Requires trusted, stable parent directories.
    /// An adversarial replacement between metadata and open can evade the path
    /// check or block opening a substituted FIFO; this is not a hostile-path API.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PresetError> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Err(PresetError::NotRegularFile);
        }
        if metadata.len() > MAX_PRESET_BYTES as u64 {
            return Err(PresetError::TooLarge);
        }
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(PresetError::NotRegularFile);
        }
        if metadata.len() > MAX_PRESET_BYTES as u64 {
            return Err(PresetError::TooLarge);
        }
        let mut bytes = Vec::new();
        file.take(MAX_PRESET_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }
    /// Validate before touching disk; create a new target exclusively. Existing
    /// targets (including symlinks) are never overwritten. On I/O failure a
    /// partial new target may remain; no path-based cleanup races are attempted.
    pub fn save_as(&self, path: impl AsRef<Path>) -> Result<(), PresetError> {
        let bytes = self.to_bytes()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Literal historical V1 document: independent of the current serializer.
    const OLD_V1: &str = r#"{"schema":1,"burst":16,"lifetime":0.5,"velocity":[0.0,1.0,0.0],"velocity_jitter":[1.1,0.8,1.1],"size_start":0.12,"size_end":0.0,"color_start":[1.0,0.72,0.15,0.9],"color_end":[1.0,0.3,0.05,0.0],"blend":"additive"}"#;

    fn assert_rejected_by_both(text: &str) {
        assert!(
            ParticlePreset::from_bytes(text.as_bytes()).is_err(),
            "{text}"
        );
        assert!(
            serde_json::from_str::<ParticlePreset>(text).is_err(),
            "{text}"
        );
    }

    #[test]
    fn historical_v1_literal_and_field_order_are_unchanged() {
        let preset = ParticlePreset::from_bytes(OLD_V1.as_bytes()).unwrap();
        assert_eq!(preset, ParticlePreset::default());
        assert_eq!((preset.schema, preset.burst), (1, 16));
        assert_eq!((preset.rate, preset.duration), (0.0, 0.0));
        assert_eq!(serde_json::to_string(&preset).unwrap(), OLD_V1);
        assert_eq!(
            serde_json::from_str::<ParticlePreset>(OLD_V1).unwrap(),
            preset
        );
        assert_eq!(
            ParticlePreset::from_bytes(&preset.to_bytes().unwrap()).unwrap(),
            preset
        );
        for field in ["rate", "duration"] {
            for value in ["0", "null", "1"] {
                assert_rejected_by_both(&OLD_V1.replacen(
                    '{',
                    &format!("{{\"{field}\":{value},"),
                    1,
                ));
            }
            let mut invalid = preset.clone();
            if field == "rate" {
                invalid.rate = 1.0;
            } else {
                invalid.duration = 1.0;
            }
            assert!(invalid.validate().is_err());
            assert!(invalid.to_bytes().is_err());
            assert!(serde_json::to_string(&invalid).is_err());
        }
    }

    #[test]
    fn timed_schema_round_trip_and_inactive_dimensions_preserve_version() {
        for blend in [PresetBlend::Alpha, PresetBlend::Additive] {
            for (burst, rate, duration) in [
                (0, 12.0, 0.5),
                (16, 0.0, 0.0),
                (16, 12.0, 0.0),
                (16, 0.0, 0.5),
            ] {
                let preset = ParticlePreset {
                    schema: TIMED_PRESET_SCHEMA,
                    burst,
                    rate,
                    duration,
                    blend,
                    ..ParticlePreset::default()
                };
                let bytes = preset.to_bytes().unwrap();
                let text = std::str::from_utf8(&bytes).unwrap();
                assert_eq!(ParticlePreset::from_bytes(&bytes).unwrap(), preset);
                assert_eq!(
                    serde_json::from_slice::<ParticlePreset>(&bytes).unwrap(),
                    preset
                );
                assert!(text.contains("\"schema\": 2"));
                assert!(text.contains("\"rate\":"));
                assert!(text.contains("\"duration\":"));
                assert!(!text.contains("origin"));
                assert!(!text.contains("seed"));
                let emitter = preset.to_emitter(Vec3::new(1.0, 2.0, 3.0), 42).unwrap();
                assert_eq!(
                    (emitter.burst, emitter.rate, emitter.duration),
                    (burst, rate, duration)
                );
                assert_eq!(emitter.origin, Vec3::new(1.0, 2.0, 3.0));
                assert_eq!(emitter.seed, 42);
                assert_eq!(
                    emitter.blend,
                    match blend {
                        PresetBlend::Alpha => ParticleBlend::Alpha,
                        PresetBlend::Additive => ParticleBlend::Additive,
                    }
                );
            }
        }
    }

    #[test]
    fn timed_schema_strict_fields_and_direct_serde_contract() {
        let base = OLD_V1.replace("\"schema\":1", "\"schema\":2");
        assert_rejected_by_both(&base);
        let valid = base.replacen('{', "{\"rate\":12,\"duration\":0.5,", 1);
        assert!(ParticlePreset::from_bytes(valid.as_bytes()).is_ok());
        for (old, replacement) in [
            ("\"rate\":12,", ""),
            ("\"duration\":0.5,", ""),
            ("\"rate\":12", "\"rate\":null"),
            ("\"duration\":0.5", "\"duration\":null"),
            ("\"rate\":12", "\"rate\":\"12\""),
            ("\"duration\":0.5", "\"duration\":false"),
            ("\"schema\":2", "\"schema\":3"),
        ] {
            assert_rejected_by_both(&valid.replace(old, replacement));
        }
        for member in [
            "\"rate\":12",
            "\"duration\":0.5",
            "\"schema\":2",
            "\"burst\":16",
            "\"extra\":0",
            "\"origin\":[0,0,0]",
            "\"seed\":42",
        ] {
            assert_rejected_by_both(&valid.replacen('{', &format!("{{{member},"), 1));
        }
        // A schema field at the end also works, with no order-dependent defaults.
        let last_schema = valid
            .replace("\"schema\":2,", "")
            .replace('}', ",\"schema\":2}");
        assert!(serde_json::from_str::<ParticlePreset>(&last_schema).is_ok());
        assert!(ParticlePreset::from_bytes(last_schema.as_bytes()).is_ok());
        assert_rejected_by_both(&format!("{valid} {{}}"));
        for invalid in [
            "[]",
            "null",
            "true",
            r#"[2,16,0.5,[0,1,0],[1,1,1],0.12,0,[1,1,1,1],[1,1,1,0],"additive",12,0.5]"#,
        ] {
            assert_rejected_by_both(invalid);
        }
    }

    #[test]
    fn timed_emitter_boundaries_and_nonfinite_values_are_rejected_consistently() {
        let boundary = ParticlePreset {
            schema: TIMED_PRESET_SCHEMA,
            burst: 1024,
            rate: 4096.0,
            duration: 10.0,
            lifetime: 10.0,
            ..ParticlePreset::default()
        };
        assert!(boundary.validate().is_ok());
        for (burst, rate, duration, lifetime) in [
            (0, 1.0, 0.001, 0.001),
            (1024, 4096.0, 10.0, 10.0),
            (1, 0.0, 0.0, 0.001),
        ] {
            let preset = ParticlePreset {
                burst,
                rate,
                duration,
                lifetime,
                ..boundary.clone()
            };
            assert_eq!(
                ParticlePreset::from_bytes(&preset.to_bytes().unwrap()).unwrap(),
                preset
            );
        }
        for (burst, rate, duration) in [
            (0, 0.0, 1.0),
            (0, 1.0, 0.0),
            (0, 0.0, 0.0),
            (1025, 1.0, 1.0),
        ] {
            let invalid = ParticlePreset {
                burst,
                rate,
                duration,
                ..boundary.clone()
            };
            assert!(invalid.validate().is_err());
            assert!(serde_json::to_string(&invalid).is_err());
        }
        for field in ["rate", "duration", "lifetime"] {
            for value in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -0.001,
                100_001.0,
            ] {
                let mut invalid = boundary.clone();
                match field {
                    "rate" => invalid.rate = value,
                    "duration" => invalid.duration = value,
                    _ => invalid.lifetime = value,
                }
                assert!(invalid.validate().is_err(), "{field}={value}");
                assert!(invalid.to_bytes().is_err());
                assert!(serde_json::to_string(&invalid).is_err());
            }
        }
        for (rate, duration) in [(4096.001, 10.0), (4096.0, 10.001)] {
            assert!(ParticlePreset {
                rate,
                duration,
                ..boundary.clone()
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn round_trip_preserves_every_field_and_runtime_inputs_are_not_serialized() {
        for blend in [PresetBlend::Alpha, PresetBlend::Additive] {
            let preset = ParticlePreset {
                blend,
                ..ParticlePreset::default()
            };
            let bytes = preset.to_bytes().unwrap();
            assert_eq!(ParticlePreset::from_bytes(&bytes).unwrap(), preset);
            let text = String::from_utf8(bytes).unwrap();
            for excluded in ["origin", "seed", "rate", "duration"] {
                assert!(!text.contains(excluded));
            }
            let origin = Vec3::new(1.0, 2.0, 3.0);
            let emitter = preset.to_emitter(origin, 42).unwrap();
            assert_eq!(emitter.origin, origin);
            assert_eq!(emitter.seed, 42);
            assert_eq!((emitter.rate, emitter.duration), (0.0, 0.0));
            assert!(preset
                .to_emitter(Vec3::new(f32::NAN, 0.0, 0.0), 42)
                .is_err());
        }
    }

    #[test]
    fn rejects_unknown_duplicate_missing_malformed_and_runtime_fields() {
        let text = String::from_utf8(ParticlePreset::default().to_bytes().unwrap()).unwrap();
        for member in [
            "\"extra\":0",
            "\"schema\":1",
            "\"burst\":16",
            "\"origin\":[0,0,0]",
            "\"seed\":1",
            "\"rate\":0",
            "\"duration\":0",
        ] {
            let changed = text.replacen('{', &format!("{{{member},"), 1);
            assert!(
                ParticlePreset::from_bytes(changed.as_bytes()).is_err(),
                "{member}"
            );
        }
        for malformed in [
            "{}",
            "[]",
            "null",
            "{",
            "true",
            "NaN",
            r#"[1,16,0.5,[0,1,0],[1,1,1],0.12,0,[1,1,1,1],[1,1,1,0],"additive"]"#,
        ] {
            assert!(ParticlePreset::from_bytes(malformed.as_bytes()).is_err());
        }
        for (old, new) in [
            ("\"schema\": 1", "\"schema\": 2"),
            ("\"burst\": 16", "\"burst\": 1.5"),
            ("\"additive\"", "\"opaque\""),
            ("\"lifetime\": 0.5", "\"lifetime\": 1e400"),
        ] {
            assert!(text.contains(old));
            assert!(ParticlePreset::from_bytes(text.replace(old, new).as_bytes()).is_err());
        }
        assert!(ParticlePreset::from_bytes(format!("{text} {{}}").as_bytes()).is_err());
    }

    #[test]
    fn validates_all_numeric_fields_and_boundaries() {
        let boundary = ParticlePreset {
            burst: 1024,
            lifetime: 10.0,
            velocity: [-10_000.0, 0.0, 10_000.0],
            velocity_jitter: [10_000.0; 3],
            size_start: 1000.0,
            size_end: 1000.0,
            color_start: [0.0; 4],
            color_end: [1.0; 4],
            ..ParticlePreset::default()
        };
        assert!(boundary.validate().is_ok());
        assert!(ParticlePreset {
            burst: 1,
            lifetime: 0.001,
            ..boundary.clone()
        }
        .validate()
        .is_ok());
        for burst in [0, 1025, u32::MAX] {
            assert!(ParticlePreset {
                burst,
                ..boundary.clone()
            }
            .validate()
            .is_err());
        }
        for field in 0..17 {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 100_001.0] {
                let mut preset = boundary.clone();
                match field {
                    0 => preset.lifetime = value,
                    1 => preset.size_start = value,
                    2 => preset.size_end = value,
                    3..=5 => preset.velocity[field - 3] = value,
                    6..=8 => preset.velocity_jitter[field - 6] = value,
                    9..=12 => preset.color_start[field - 9] = value,
                    _ => preset.color_end[field - 13] = value,
                }
                assert!(preset.to_bytes().is_err(), "field {field}, {value}");
            }
        }
        let mut preset = boundary;
        preset.color_end[3] = f32::NAN;
        assert!(preset.validate().is_err());
        assert!(ParticlePreset {
            lifetime: 0.0,
            ..ParticlePreset::default()
        }
        .validate()
        .is_err());
        assert!(ParticlePreset {
            velocity_jitter: [-0.1, 0.0, 0.0],
            ..ParticlePreset::default()
        }
        .validate()
        .is_err());
        assert!(ParticlePreset {
            size_end: -0.1,
            ..ParticlePreset::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn bounded_bytes_and_regular_file_loading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preset.json");
        let preset = ParticlePreset::default();
        preset.save_as(&path).unwrap();
        assert_eq!(ParticlePreset::open(&path).unwrap(), preset);
        assert!(matches!(
            ParticlePreset::open(dir.path()),
            Err(PresetError::NotRegularFile)
        ));
        let mut bytes = preset.to_bytes().unwrap();
        bytes.resize(MAX_PRESET_BYTES, b' ');
        assert!(ParticlePreset::from_bytes(&bytes).is_ok());
        bytes.push(b' ');
        assert!(matches!(
            ParticlePreset::from_bytes(&bytes),
            Err(PresetError::TooLarge)
        ));
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            ParticlePreset::open(&path),
            Err(PresetError::TooLarge)
        ));
    }

    #[test]
    fn save_as_never_clobbers_source_or_existing_target_and_validates_first() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.json");
        let target = dir.path().join("target.json");
        let preset = ParticlePreset::default();
        preset.save_as(&source).unwrap();
        let original = fs::read(&source).unwrap();
        let mut edited = ParticlePreset::open(&source).unwrap();
        edited.burst = 32;
        assert!(edited.save_as(&source).is_err());
        edited.save_as(&target).unwrap();
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(ParticlePreset::open(&target).unwrap(), edited);
        assert!(preset.save_as(&target).is_err());
        assert_eq!(ParticlePreset::open(&target).unwrap(), edited);
        edited.lifetime = f32::NAN;
        let absent = dir.path().join("invalid.json");
        assert!(edited.save_as(&absent).is_err());
        assert!(!absent.exists());
        assert_eq!(fs::read(&source).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_fifo_without_opening_them() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.json");
        let link = dir.path().join("link.json");
        let missing_link = dir.path().join("missing-link.json");
        let fifo = dir.path().join("pipe.json");
        ParticlePreset::default().save_as(&source).unwrap();
        let before = fs::read(&source).unwrap();
        symlink(&source, &link).unwrap();
        symlink(dir.path().join("absent"), &missing_link).unwrap();
        for path in [&link, &missing_link] {
            assert!(matches!(
                ParticlePreset::open(path),
                Err(PresetError::NotRegularFile)
            ));
            assert!(ParticlePreset::default().save_as(path).is_err());
        }
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        assert!(matches!(
            ParticlePreset::open(&fifo),
            Err(PresetError::NotRegularFile)
        ));
        assert!(ParticlePreset::default().save_as(&fifo).is_err());
        assert_eq!(fs::read(&source).unwrap(), before);
    }
}
