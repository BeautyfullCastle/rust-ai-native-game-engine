//! Everything about the look of the scene that a person can tune: light, sky,
//! fog, shadows, wind, water, vegetation, ground and the camera response.
//!
//! [`Settings`] is plain data. It is saved as JSON, edited through the
//! [`SLIDERS`], [`TOGGLES`] and [`COLORS`] tables, and read by the renderer
//! every frame, so a change shows on the next frame. [`Quality`] holds the few
//! choices that need new GPU resources.

use crate::math::{normalize, V3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sun {
    /// Hour of the day. Sets the height and direction of the sun.
    pub hour: f32,
    pub intensity: f32,
    /// Scale of the light from the sky and the ground on shaded surfaces.
    pub ambient: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sky {
    pub brightness: f32,
    /// Bright haze around the sun.
    pub sun_glow: f32,
    /// Share of the sky that clouds cover.
    pub cloud_cover: f32,
    pub cloud_speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fog {
    /// Thickness of the valley fog. Zero removes it.
    pub density: f32,
    /// How fast the fog thins with height, per meter.
    pub falloff: f32,
    /// Thin haze that does not depend on height, as a share of the fog.
    pub haze: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shadows {
    pub enabled: bool,
    /// Width of the soft edge, in shadow map texels.
    pub softness: f32,
    /// Half side of the sharp shadow map around the camera, in meters.
    pub near_range: f32,
    /// Half side of the wide shadow map, in meters.
    pub far_range: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wind {
    pub strength: f32,
    pub speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Water {
    /// Color of light scattered back from deep water.
    pub color: V3,
    /// Higher values let the stream bed show through deeper water.
    pub clarity: f32,
    pub ripples: f32,
    pub flow_speed: f32,
    pub foam: f32,
    /// Sun sparkle on the surface.
    pub glint: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vegetation {
    /// Multiplies the color of all leaves and grass.
    pub tint: V3,
    /// Light that passes through leaves when the sun is behind them.
    pub translucency: f32,
    /// Contrast of the leaf pattern on tree crowns.
    pub leaf_detail: f32,
    /// Share of broadleaf trees in autumn colors.
    pub autumn: f32,
    /// Ground cover is drawn up to this distance, in meters.
    pub grass_distance: f32,
    /// Scale of the distances where trees change to simpler meshes.
    pub detail_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ground {
    /// Strength of the small bumps that tilt the ground normal.
    pub bumps: f32,
    /// Moving light lines on the stream bed.
    pub caustics: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    /// When on, the exposure follows the brightness of the view, as an eye does.
    pub auto_exposure: bool,
    /// Exposure multiplier. With auto exposure off it is the whole exposure.
    pub exposure: f32,
    /// How fast auto exposure follows a change, per second.
    pub adaptation_speed: f32,
    pub bloom: f32,
    /// Scene brightness where bloom starts.
    pub bloom_threshold: f32,
    pub light_shafts: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub vignette: f32,
    pub grain: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub sun: Sun,
    pub sky: Sky,
    pub fog: Fog,
    pub shadows: Shadows,
    pub wind: Wind,
    pub water: Water,
    pub vegetation: Vegetation,
    pub ground: Ground,
    pub look: Look,
}

impl Default for Sun {
    fn default() -> Self {
        Self { hour: 16.6, intensity: 8.5, ambient: 1.0 }
    }
}
impl Default for Sky {
    fn default() -> Self {
        Self { brightness: 1.5, sun_glow: 1.0, cloud_cover: 0.34, cloud_speed: 1.0 }
    }
}
impl Default for Fog {
    fn default() -> Self {
        Self { density: 1.0, falloff: 0.03, haze: 0.08 }
    }
}
impl Default for Shadows {
    fn default() -> Self {
        Self { enabled: true, softness: 1.5, near_range: 46.0, far_range: 330.0 }
    }
}
impl Default for Wind {
    fn default() -> Self {
        Self { strength: 1.0, speed: 1.0 }
    }
}
impl Default for Water {
    fn default() -> Self {
        Self { color: [0.012, 0.05, 0.04], clarity: 1.0, ripples: 1.0, flow_speed: 1.0, foam: 1.0, glint: 1.0 }
    }
}
impl Default for Vegetation {
    fn default() -> Self {
        Self { tint: [1.0, 1.0, 1.0], translucency: 1.0, leaf_detail: 1.0, autumn: 0.14, grass_distance: 85.0, detail_distance: 1.0 }
    }
}
impl Default for Ground {
    fn default() -> Self {
        Self { bumps: 0.9, caustics: 0.5 }
    }
}
impl Default for Look {
    fn default() -> Self {
        Self {
            auto_exposure: true,
            exposure: 1.0,
            adaptation_speed: 2.2,
            bloom: 0.1,
            bloom_threshold: 0.9,
            light_shafts: 0.35,
            saturation: 1.2,
            contrast: 0.22,
            vignette: 0.2,
            grain: 1.0,
        }
    }
}

/// One number of [`Settings`] with its allowed range.
pub struct Slider {
    pub group: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub help: &'static str,
    pub get: fn(&Settings) -> f32,
    pub set: fn(&mut Settings, f32),
}

pub struct Toggle {
    pub group: &'static str,
    pub name: &'static str,
    pub help: &'static str,
    pub get: fn(&Settings) -> bool,
    pub set: fn(&mut Settings, bool),
}

pub struct Color {
    pub group: &'static str,
    pub name: &'static str,
    /// Largest value of one channel.
    pub max: f32,
    pub help: &'static str,
    pub get: fn(&Settings) -> V3,
    pub set: fn(&mut Settings, V3),
}

macro_rules! slider {
    ($group:literal, $name:literal, $($field:ident).+, $min:literal, $max:literal, $help:literal) => {
        Slider { group: $group, name: $name, min: $min, max: $max, help: $help, get: |s| s.$($field).+, set: |s, v| s.$($field).+ = v }
    };
}
macro_rules! toggle {
    ($group:literal, $name:literal, $($field:ident).+, $help:literal) => {
        Toggle { group: $group, name: $name, help: $help, get: |s| s.$($field).+, set: |s, v| s.$($field).+ = v }
    };
}
macro_rules! color {
    ($group:literal, $name:literal, $($field:ident).+, $max:literal, $help:literal) => {
        Color { group: $group, name: $name, max: $max, help: $help, get: |s| s.$($field).+, set: |s, v| s.$($field).+ = v }
    };
}

/// Group names in display order.
pub const GROUPS: [&str; 9] = ["Sun", "Sky", "Fog", "Shadows", "Wind", "Water", "Vegetation", "Ground", "Look"];

pub const SLIDERS: &[Slider] = &[
    slider!("Sun", "Hour", sun.hour, 5.0, 19.0, "Hour of the day. Sets the height and direction of the sun."),
    slider!("Sun", "Intensity", sun.intensity, 0.0, 20.0, "Brightness of direct sunlight."),
    slider!("Sun", "Ambient", sun.ambient, 0.0, 3.0, "Light from the sky and the ground on shaded surfaces."),
    slider!("Sky", "Brightness", sky.brightness, 0.0, 4.0, "Brightness of the sky itself."),
    slider!("Sky", "Sun glow", sky.sun_glow, 0.0, 4.0, "Bright haze around the sun."),
    slider!("Sky", "Cloud cover", sky.cloud_cover, 0.0, 1.0, "Share of the sky that clouds cover."),
    slider!("Sky", "Cloud speed", sky.cloud_speed, 0.0, 10.0, "How fast the clouds drift."),
    slider!("Fog", "Density", fog.density, 0.0, 5.0, "Thickness of the valley fog. Zero removes it."),
    slider!("Fog", "Height falloff", fog.falloff, 0.002, 0.2, "How fast the fog thins with height, per meter."),
    slider!("Fog", "Haze", fog.haze, 0.0, 1.0, "Thin haze at every height, as a share of the fog."),
    slider!("Shadows", "Softness", shadows.softness, 0.5, 4.0, "Width of the soft shadow edge, in shadow map texels."),
    slider!("Shadows", "Sharp range", shadows.near_range, 15.0, 120.0, "Half side of the sharp shadow map around the camera, in meters."),
    slider!("Shadows", "Wide range", shadows.far_range, 150.0, 600.0, "Half side of the wide shadow map, in meters."),
    slider!("Wind", "Strength", wind.strength, 0.0, 4.0, "How far trees and grass sway."),
    slider!("Wind", "Speed", wind.speed, 0.0, 4.0, "How fast trees and grass sway."),
    slider!("Water", "Clarity", water.clarity, 0.1, 5.0, "Higher values let the stream bed show through deeper water."),
    slider!("Water", "Ripples", water.ripples, 0.0, 3.0, "Height of the surface ripples."),
    slider!("Water", "Flow speed", water.flow_speed, 0.0, 3.0, "How fast ripples and foam move downstream."),
    slider!("Water", "Foam", water.foam, 0.0, 2.0, "Amount of whitewater at cascades and foam at the shore."),
    slider!("Water", "Sun glint", water.glint, 0.0, 4.0, "Sun sparkle on the surface."),
    slider!("Vegetation", "Translucency", vegetation.translucency, 0.0, 3.0, "Light that passes through leaves when the sun is behind them."),
    slider!("Vegetation", "Leaf detail", vegetation.leaf_detail, 0.0, 2.0, "Contrast of the leaf pattern on tree crowns."),
    slider!("Vegetation", "Autumn trees", vegetation.autumn, 0.0, 1.0, "Share of broadleaf trees in autumn colors."),
    slider!("Vegetation", "Grass distance", vegetation.grass_distance, 20.0, 200.0, "Ground cover is drawn up to this distance, in meters."),
    slider!("Vegetation", "Tree detail distance", vegetation.detail_distance, 0.3, 3.0, "Scale of the distances where trees change to simpler meshes."),
    slider!("Ground", "Bumps", ground.bumps, 0.0, 3.0, "Strength of the small bumps on the ground."),
    slider!("Ground", "Caustics", ground.caustics, 0.0, 3.0, "Moving light lines on the stream bed."),
    slider!("Look", "Exposure", look.exposure, 0.1, 8.0, "Exposure multiplier. With auto exposure off it is the whole exposure."),
    slider!("Look", "Adaptation speed", look.adaptation_speed, 0.2, 10.0, "How fast auto exposure follows a change, per second."),
    slider!("Look", "Bloom", look.bloom, 0.0, 1.0, "Glow around very bright areas."),
    slider!("Look", "Bloom threshold", look.bloom_threshold, 0.0, 5.0, "Scene brightness where bloom starts."),
    slider!("Look", "Light shafts", look.light_shafts, 0.0, 2.0, "Rays of sunlight between trees and ridges."),
    slider!("Look", "Saturation", look.saturation, 0.0, 2.0, "Color strength. Zero is black and white."),
    slider!("Look", "Contrast", look.contrast, 0.0, 1.0, "Extra contrast after tone mapping."),
    slider!("Look", "Vignette", look.vignette, 0.0, 1.0, "Darkening toward the corners."),
    slider!("Look", "Grain", look.grain, 0.0, 4.0, "Fine noise that hides color banding."),
];

pub const TOGGLES: &[Toggle] = &[
    toggle!("Shadows", "Sun shadows", shadows.enabled, "Trees, rocks and terrain cast shadows."),
    toggle!("Look", "Auto exposure", look.auto_exposure, "The exposure follows the brightness of the view, as an eye does."),
];

pub const COLORS: &[Color] = &[
    color!("Water", "Deep water color", water.color, 0.5, "Color of light scattered back from deep water."),
    color!("Vegetation", "Leaf tint", vegetation.tint, 2.0, "Multiplies the color of all leaves and grass."),
];

impl Settings {
    /// The same settings with every value finite and inside its range.
    /// A value that is not a number becomes its default.
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        for slider in SLIDERS {
            let value = (slider.get)(&self);
            let value = if value.is_finite() { value } else { (slider.get)(&defaults) };
            (slider.set)(&mut self, value.clamp(slider.min, slider.max));
        }
        for color in COLORS {
            let (value, fallback) = ((color.get)(&self), (color.get)(&defaults));
            let fixed = std::array::from_fn(|i| if value[i].is_finite() { value[i].clamp(0.0, color.max) } else { fallback[i] });
            (color.set)(&mut self, fixed);
        }
        // The wide shadow map must contain the sharp one.
        self.shadows.far_range = self.shadows.far_range.max(self.shadows.near_range * 2.0);
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings are plain numbers")
    }

    /// Reads settings saved by [`Settings::to_json`]. Missing values take their
    /// defaults, so files from an older version still load.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str::<Self>(text).map(Self::sanitized).map_err(|e| format!("scene settings: {e}"))
    }

    /// Unit vector toward the sun.
    pub fn sun_direction(&self) -> V3 {
        let day = ((self.sun.hour - 6.0) / 12.0).clamp(-0.1, 1.1);
        let elevation = 64.0_f32.to_radians() * (day * std::f32::consts::PI).sin();
        // The sun moves from upstream-left in the morning to upstream-right in the evening.
        let azimuth = (162.0 + (self.sun.hour - 12.0) * 13.0).to_radians();
        normalize([elevation.cos() * azimuth.sin(), elevation.sin(), elevation.cos() * azimuth.cos()])
    }
}

/// A saved scene look: which valley, at which quality, with which settings.
/// This is the file the editor writes and `forest_valley --settings` reads.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preset {
    /// Seed of the generated valley.
    pub seed: u32,
    pub quality: Quality,
    pub settings: Settings,
}

impl Default for Preset {
    fn default() -> Self {
        Self { seed: 7, quality: Quality::default(), settings: Settings::default() }
    }
}

impl Preset {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a preset is plain numbers")
    }

    /// Reads a preset saved by [`Preset::to_json`]. Missing values take their
    /// defaults and every value is brought inside its range. A file with
    /// other top-level entries is not a preset and is rejected.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let preset: Self = serde_json::from_str(text).map_err(|e| format!("scene preset: {e}"))?;
        Ok(Self { seed: preset.seed, quality: preset.quality.sanitized(), settings: preset.settings.sanitized() })
    }
}

/// Choices that need new GPU resources. Pass them to `Renderer::new`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    /// Samples per pixel for edge smoothing: 1, 2 or 4.
    pub antialiasing: u32,
    /// Side of the wide shadow map in texels: 1024, 2048 or 4096. The sharp map uses half.
    pub shadow_resolution: u32,
}

impl Default for Quality {
    fn default() -> Self {
        Self { antialiasing: 4, shadow_resolution: 4096 }
    }
}

impl Quality {
    pub const ANTIALIASING: [u32; 3] = [1, 2, 4];
    pub const SHADOW_RESOLUTION: [u32; 3] = [1024, 2048, 4096];

    /// The nearest supported values.
    pub fn sanitized(self) -> Self {
        let nearest = |value: u32, allowed: &[u32]| *allowed.iter().min_by_key(|&&a| a.abs_diff(value)).expect("allowed values exist");
        Self { antialiasing: nearest(self.antialiasing, &Self::ANTIALIASING), shadow_resolution: nearest(self.shadow_resolution, &Self::SHADOW_RESOLUTION) }
    }
}
