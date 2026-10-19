//! Bounded, burst-only particle authoring documents. No simulation state or renderer.
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
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub const MAX_PRESET_BYTES: usize = 64 * 1024;
pub const PRESET_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetBlend {
    Alpha,
    Additive,
}

/// Schema 1 requires every field, rejects unknown/duplicate fields, and stores no
/// event identity, origin, seed, continuous emission rate, or duration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticlePreset {
    pub schema: u32,
    pub burst: u32,
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
                write!(f, "unsupported particle preset schema {schema}; expected 1")
            }
            Self::InvalidEmitter => {
                f.write_str("particle preset violates burst-only emitter bounds")
            }
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
    /// Runtime supplies the event's origin and seed. Rate and duration are always zero.
    pub fn to_emitter(&self, origin: Vec3, seed: u64) -> Result<EmitterDesc, PresetError> {
        if self.schema != PRESET_SCHEMA {
            return Err(PresetError::UnsupportedSchema(self.schema));
        }
        if self.burst == 0 {
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
            rate: 0.0,
            duration: 0.0,
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
        let preset: Self = serde_json::from_slice(bytes)?;
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
