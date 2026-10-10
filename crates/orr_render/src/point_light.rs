//! A bounded, saveable unshadowed point light for imported scenes.
//!
//! This is stylized diffuse lighting, not a photometric or glTF punctual-light
//! implementation. For distance `d`, range `r`, and unit surface normal `n`, the
//! added linear RGB is `color * intensity * max(dot(n, L), 0) * max(1-d/r, 0)^2`.
//! `L` points from the shaded world-space position toward the light. At distances
//! at most `1e-6`, the contribution is explicitly zero (no undefined direction).
//! It is added before the existing exposure, ACES, and output encoding.
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub const POINT_LIGHT_SETTINGS_VERSION: u32 = 1;
pub const MAX_POINT_LIGHT_SETTINGS_BYTES: usize = 4096;

/// Positions and ranges are world units. RGB is linear, with no sRGB conversion.
/// Positions are bounded to +/-1e9; RGB/intensity to 0..=1e4; range to 1e-6..=1e9.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointLight {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
}

/// Exactly zero or one light. `None` preserves the prior imported-model shading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointLightSettings {
    pub point_light: Option<PointLight>,
}

#[derive(Debug)]
pub enum PointLightError {
    InvalidLight,
    UnsupportedVersion(u32),
    TooLarge,
    Json(serde_json::Error),
    Io(std::io::Error),
}
impl std::fmt::Display for PointLightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLight => f.write_str("point light requires finite bounded world position, nonnegative linear RGB/intensity, and positive range"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported point-light settings version {v}"),
            Self::TooLarge => write!(f, "point-light settings exceed {MAX_POINT_LIGHT_SETTINGS_BYTES} bytes"),
            Self::Json(e) => write!(f, "point-light settings JSON: {e}"),
            Self::Io(e) => write!(f, "point-light settings I/O: {e}"),
        }
    }
}
impl std::error::Error for PointLightError {}
impl From<std::io::Error> for PointLightError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for PointLightError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    // Explicit deserialization makes the field required, while accepting null.
    #[serde(deserialize_with = "required_light")]
    point_light: Option<PointLight>,
}
fn required_light<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<PointLight>, D::Error> {
    Option::<PointLight>::deserialize(deserializer)
}

impl PointLightSettings {
    pub fn validate(&self) -> Result<(), PointLightError> {
        if let Some(light) = self.point_light {
            if !light
                .position
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e9)
                || !light
                    .color
                    .iter()
                    .chain([&light.intensity])
                    .all(|v| v.is_finite() && (0.0..=1e4).contains(v))
                || !light.range.is_finite()
                || !(1e-6..=1e9).contains(&light.range)
            {
                return Err(PointLightError::InvalidLight);
            }
        }
        Ok(())
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PointLightError> {
        if bytes.len() > MAX_POINT_LIGHT_SETTINGS_BYTES {
            return Err(PointLightError::TooLarge);
        }
        let document: Document = serde_json::from_slice(bytes)?;
        if document.version != POINT_LIGHT_SETTINGS_VERSION {
            return Err(PointLightError::UnsupportedVersion(document.version));
        }
        let settings = Self {
            point_light: document.point_light,
        };
        settings.validate()?;
        Ok(settings)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, PointLightError> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(&Document {
            version: POINT_LIGHT_SETTINGS_VERSION,
            point_light: self.point_light,
        })?;
        if bytes.len() > MAX_POINT_LIGHT_SETTINGS_BYTES {
            return Err(PointLightError::TooLarge);
        }
        Ok(bytes)
    }
    /// Reads at most the byte budget plus one byte, even for growing files.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PointLightError> {
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(MAX_POINT_LIGHT_SETTINGS_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }
    /// Failure leaves the current in-memory settings unchanged.
    pub fn reload(&mut self, path: impl AsRef<Path>) -> Result<(), PointLightError> {
        let replacement = Self::load(path)?;
        *self = replacement;
        Ok(())
    }
    /// Atomically replaces a caller-owned settings file using a sibling temporary
    /// file. Never write this path into an immutable installed package snapshot.
    /// Parent directories must already exist. Invalid settings never touch disk.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PointLightError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let bytes = self.to_bytes()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut staged = None;
        for _ in 0..16 {
            let temporary = parent.join(format!(
                ".orr-point-light-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => {
                    staged = Some((temporary, file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        let (temporary, mut file) = staged.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "point-light temporary file collisions",
            )
        })?;
        let result = (|| -> Result<(), std::io::Error> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn light() -> PointLightSettings {
        PointLightSettings {
            point_light: Some(PointLight {
                position: [1.0, 2.0, 3.0],
                color: [1.0, 0.4, 0.0],
                intensity: 2.0,
                range: 8.0,
            }),
        }
    }
    #[test]
    fn strict_versioned_bounded_json() {
        for settings in [PointLightSettings::default(), light()] {
            assert_eq!(
                PointLightSettings::from_bytes(&settings.to_bytes().unwrap()).unwrap(),
                settings
            );
        }
        for bytes in [b"{}".as_slice(), br#"{"version":1}"#, br#"{"version":1,"point_light":null,"extra":0}"#, br#"{"version":1,"version":1,"point_light":null}"#, br#"{"version":1,"point_light":[]}"#, br#"{"version":1,"point_light":{"position":[0,0,0],"color":[1,1,1],"intensity":1,"range":2,"shadow":false}}"#] {
            assert!(PointLightSettings::from_bytes(bytes).is_err(), "{bytes:?}");
        }
        assert!(matches!(
            PointLightSettings::from_bytes(br#"{"version":2,"point_light":null}"#),
            Err(PointLightError::UnsupportedVersion(2))
        ));
        assert!(matches!(
            PointLightSettings::from_bytes(&[b' '; MAX_POINT_LIGHT_SETTINGS_BYTES + 1]),
            Err(PointLightError::TooLarge)
        ));
    }
    #[test]
    fn light_validation_bounds() {
        for value in [f32::NAN, f32::INFINITY, -1.0, 1e5] {
            let mut settings = light();
            settings.point_light.as_mut().unwrap().intensity = value;
            assert!(settings.validate().is_err());
            assert!(settings.to_bytes().is_err());
        }
        for range in [0.0, -1.0, 1e-7, 1e10, f32::NAN] {
            let mut settings = light();
            settings.point_light.as_mut().unwrap().range = range;
            assert!(settings.validate().is_err());
        }
        let mut settings = light();
        settings.point_light.as_mut().unwrap().intensity = 0.0;
        assert!(settings.validate().is_ok());
        settings.point_light.as_mut().unwrap().position[0] = 1e10;
        assert!(settings.validate().is_err());
        settings = light();
        settings.point_light.as_mut().unwrap().color[1] = -0.1;
        assert!(settings.validate().is_err());
    }
    #[test]
    fn saved_reload_failure_preserves_state_and_file() {
        let path = std::env::temp_dir().join(format!(
            "orr-point-light-settings-test-{}.json",
            std::process::id()
        ));
        let initial = light();
        initial.save(&path).unwrap();
        assert_eq!(PointLightSettings::load(&path).unwrap(), initial);
        let mut invalid = initial.clone();
        invalid.point_light.as_mut().unwrap().range = -1.0;
        assert!(invalid.save(&path).is_err());
        assert_eq!(PointLightSettings::load(&path).unwrap(), initial);
        PointLightSettings::default().save(&path).unwrap();
        let mut current = initial.clone();
        current.reload(&path).unwrap();
        assert_eq!(current, PointLightSettings::default());
        std::fs::write(&path, br#"{"version":999,"point_light":null}"#).unwrap();
        assert!(current.reload(&path).is_err());
        assert_eq!(current, PointLightSettings::default());
        std::fs::write(&path, vec![b' '; MAX_POINT_LIGHT_SETTINGS_BYTES + 1]).unwrap();
        assert!(matches!(
            current.reload(&path),
            Err(PointLightError::TooLarge)
        ));
        std::fs::remove_file(&path).unwrap();
        assert!(current.reload(&path).is_err());
    }
}
