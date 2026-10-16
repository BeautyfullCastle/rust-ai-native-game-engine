//! Optional, GPU-free presentation data. No simulation state, wall clock, image
//! decoder or renderer is owned here. Times are caller-supplied milliseconds.
//! JSON schema version 1 is separate from ORAM, motion and PCM formats.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

pub const FORMAT: &str = "orr_sprite";
pub const VERSION: u32 = 1;
/// Conservative version-1 resource limits, checked before runtime indexing.
pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_IMAGE_NAME_BYTES: usize = 1024;
pub const MAX_CLIP_ID_BYTES: usize = 128;
pub const MAX_REGIONS: usize = 4096;
pub const MAX_CLIPS: usize = 256;
pub const MAX_FRAMES_PER_CLIP: usize = 4096;
pub const MAX_TOTAL_FRAMES: usize = 16384;
pub const MAX_ATLAS_DIMENSION: u32 = 2048;

/// Unvalidated authoring data; load with [`SpriteDocument::new`]. Image paths are
/// inert metadata: callers decide how to resolve and decode them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteSource {
    pub format: String,
    pub version: u32,
    pub atlas: Atlas,
    pub regions: Vec<Region>,
    pub clips: Vec<ClipSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Atlas {
    pub image: String,
    pub width: u32,
    pub height: u32,
}

/// Top-left pixel rectangle; IDs remain stable when definitions are reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    Loop,
    Once,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSource {
    pub id: String,
    pub mode: PlaybackMode,
    pub frames: Vec<Frame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub region: u32,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
fn invalid(message: impl Into<String>) -> Error {
    Error(message.into())
}

/// Validated immutable atlas and clips. All runtime clips have a positive,
/// non-overflowing duration and only reference in-bounds regions.
#[derive(Clone, Debug)]
pub struct SpriteDocument {
    atlas: Atlas,
    regions: Vec<Region>,
    clips: Vec<Clip>,
}
impl SpriteDocument {
    pub fn from_json(json: &str) -> Result<Self, Error> {
        if json.len() > MAX_JSON_BYTES {
            return Err(invalid("sprite JSON exceeds byte limit"));
        }
        Self::new(serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?)
    }
    pub fn new(source: SpriteSource) -> Result<Self, Error> {
        if source.format != FORMAT || source.version != VERSION {
            return Err(invalid("unsupported sprite format or version"));
        }
        // Check all aggregate bounds before creating indexes or runtime data.
        if source.atlas.image.len() > MAX_IMAGE_NAME_BYTES
            || source.regions.len() > MAX_REGIONS
            || source.clips.len() > MAX_CLIPS
        {
            return Err(invalid("sprite source exceeds name or collection limit"));
        }
        let mut total_frames = 0usize;
        for clip in &source.clips {
            if clip.id.len() > MAX_CLIP_ID_BYTES || clip.frames.len() > MAX_FRAMES_PER_CLIP {
                return Err(invalid("clip exceeds ID or frame limit"));
            }
            total_frames = total_frames
                .checked_add(clip.frames.len())
                .ok_or_else(|| invalid("total frame count overflow"))?;
            if total_frames > MAX_TOTAL_FRAMES {
                return Err(invalid("sprite source exceeds total frame limit"));
            }
        }
        let a = &source.atlas;
        if a.width == 0
            || a.height == 0
            || a.width > MAX_ATLAS_DIMENSION
            || a.height > MAX_ATLAS_DIMENSION
            || a.image.trim().is_empty()
        {
            return Err(invalid(
                "atlas requires dimensions in 1..=2048 and an image name",
            ));
        }
        let mut ids = BTreeSet::new();
        for r in &source.regions {
            if !ids.insert(r.id) {
                return Err(invalid(format!("duplicate region {}", r.id)));
            }
            if r.width == 0
                || r.height == 0
                || r.x.checked_add(r.width).is_none_or(|x| x > a.width)
                || r.y.checked_add(r.height).is_none_or(|y| y > a.height)
            {
                return Err(invalid(format!("region {} outside atlas or empty", r.id)));
            }
        }
        let mut clip_ids = BTreeSet::new();
        let mut durations = Vec::with_capacity(source.clips.len());
        for c in &source.clips {
            if c.id.trim().is_empty() || !clip_ids.insert(c.id.as_str()) {
                return Err(invalid("empty or duplicate clip ID"));
            }
            if c.frames.is_empty() {
                return Err(invalid(format!("clip {} has no frames", c.id)));
            }
            let mut duration_ms = 0u64;
            for f in &c.frames {
                if !ids.contains(&f.region) || f.duration_ms == 0 {
                    return Err(invalid(format!(
                        "clip {} has missing region or zero duration",
                        c.id
                    )));
                }
                duration_ms = duration_ms
                    .checked_add(f.duration_ms)
                    .ok_or_else(|| invalid(format!("clip {} duration overflow", c.id)))?;
            }
            durations.push(duration_ms);
        }
        // Move frame arrays into runtime clips instead of retaining two copies.
        let clips = source
            .clips
            .into_iter()
            .zip(durations)
            .map(|(source, duration_ms)| Clip {
                source,
                duration_ms,
            })
            .collect();
        Ok(Self {
            atlas: source.atlas,
            regions: source.regions,
            clips,
        })
    }
    pub fn to_json(&self) -> Result<String, Error> {
        #[derive(Serialize)]
        struct DocumentRef<'a> {
            format: &'static str,
            version: u32,
            atlas: &'a Atlas,
            regions: &'a [Region],
            clips: &'a [Clip],
        }
        serde_json::to_string_pretty(&DocumentRef {
            format: FORMAT,
            version: VERSION,
            atlas: &self.atlas,
            regions: &self.regions,
            clips: &self.clips,
        })
        .map_err(|e| invalid(e.to_string()))
    }
    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
    pub fn region(&self, id: u32) -> Option<&Region> {
        self.regions().iter().find(|r| r.id == id)
    }
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }
    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id() == id)
    }
    /// Normalized UV bounds in top-left image coordinates. Only validated regions
    /// can be addressed; renderer applies the instance's flip flags separately.
    pub fn uv_rect(&self, region: u32) -> Option<[f32; 4]> {
        let r = self.region(region)?;
        let a = self.atlas();
        Some([
            r.x as f32 / a.width as f32,
            r.y as f32 / a.height as f32,
            (r.x + r.width) as f32 / a.width as f32,
            (r.y + r.height) as f32 / a.height as f32,
        ])
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Clip {
    #[serde(flatten)]
    source: ClipSource,
    #[serde(skip)]
    duration_ms: u64,
}
impl Clip {
    pub fn id(&self) -> &str {
        &self.source.id
    }
    pub fn mode(&self) -> PlaybackMode {
        self.source.mode
    }
    pub fn frames(&self) -> &[Frame] {
        &self.source.frames
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    /// Half-open frame intervals; exact loop end wraps, exact once end holds the
    /// last frame with `finished = true`. Sampling is stateless and seekable.
    pub fn sample(&self, elapsed_ms: u64) -> Sample {
        let finished = self.mode() == PlaybackMode::Once && elapsed_ms >= self.duration_ms;
        let mut remaining = match self.mode() {
            PlaybackMode::Loop => elapsed_ms % self.duration_ms,
            PlaybackMode::Once => elapsed_ms.min(self.duration_ms - 1),
        };
        for (i, frame) in self.frames().iter().enumerate() {
            if remaining < frame.duration_ms {
                return Sample {
                    region: frame.region,
                    frame_index: i,
                    finished,
                };
            }
            remaining -= frame.duration_ms;
        }
        unreachable!("validated clip durations cover sampling interval")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub region: u32,
    pub frame_index: usize,
    pub finished: bool,
}

/// Per-instance optional cursor. No hidden/global clock; callers may instead
/// call Clip::sample directly. Saturates on advance overflow, reset seeks to 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playback {
    elapsed_ms: u64,
}
impl Playback {
    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }
    pub fn seek(&mut self, elapsed_ms: u64) {
        self.elapsed_ms = elapsed_ms;
    }
    pub fn reset(&mut self) {
        self.seek(0);
    }
    pub fn advance(&mut self, delta_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
    }
    pub fn sample(&self, clip: &Clip) -> Sample {
        clip.sample(self.elapsed_ms)
    }
}

/// Plain view instance, separate from the validated asset document and existing
/// Shape/Style ABI. Position is the center, size is full extent, rotation radians.
/// Tint is straight RGBA. Higher order draws later; consumers preserve input
/// order for ties. Resolve `region` in the document before drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteInstance {
    pub region: u32,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub rotation: f32,
    pub tint: [f32; 4],
    pub flip_x: bool,
    pub flip_y: bool,
    pub order: i32,
}
impl Default for SpriteInstance {
    fn default() -> Self {
        Self {
            region: 0,
            position: [0.0; 2],
            size: [1.0; 2],
            rotation: 0.0,
            tint: [1.0; 4],
            flip_x: false,
            flip_y: false,
            order: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const JSON: &str = include_str!("../../../assets/sprite_demo/sprites.json");
    fn source() -> SpriteSource {
        serde_json::from_str(JSON).unwrap()
    }
    fn doc() -> SpriteDocument {
        SpriteDocument::from_json(JSON).unwrap()
    }
    #[test]
    fn asset_roundtrip_and_stable_ids() {
        let d = doc();
        let mut s = source();
        s.regions.reverse();
        s.clips.reverse();
        let reordered = SpriteDocument::new(s).unwrap();
        assert_eq!(
            d.clip("walk").unwrap().sample(0),
            reordered.clip("walk").unwrap().sample(0)
        );
        assert_eq!(d.uv_rect(21), Some([0.75, 0.0, 1.0, 1.0]));
        assert_eq!(d.uv_rect(999), None);
        let roundtrip = SpriteDocument::from_json(&d.to_json().unwrap()).unwrap();
        assert_eq!(roundtrip.to_json().unwrap(), d.to_json().unwrap());
    }
    #[test]
    fn loops_and_once_use_half_open_intervals() {
        let d = doc();
        let idle = d.clip("idle").unwrap();
        for (t, expected) in [
            (0, 10),
            (599, 10),
            (600, 11),
            (999, 11),
            (1000, 10),
            (1600, 11),
        ] {
            assert_eq!(idle.sample(t).region, expected);
            assert!(!idle.sample(t).finished);
        }
        let once = d.clip("greet").unwrap();
        assert_eq!(once.sample(199).region, 10);
        assert_eq!(once.sample(200).region, 11);
        assert!(!once.sample(499).finished);
        assert!(once.sample(500).finished);
        assert_eq!(once.sample(u64::MAX).region, 11);
        assert_eq!(idle.sample(u64::MAX), idle.sample(u64::MAX % 1000));
    }
    #[test]
    fn cursors_seek_reset_and_are_independent() {
        let d = doc();
        let walk = d.clip("walk").unwrap();
        let mut a = Playback::default();
        let mut b = a;
        a.advance(140);
        assert_eq!(a.sample(walk).region, 21);
        assert_eq!(b.sample(walk).region, 20);
        b.seek(420);
        assert_eq!(a.sample(walk), b.sample(walk));
        a.reset();
        assert_eq!(a.sample(walk).region, 20);
        b.seek(u64::MAX);
        b.advance(1);
        assert_eq!(b.elapsed_ms(), u64::MAX);
    }
    #[test]
    fn rejects_invalid_atlases_and_regions() {
        for mutate in [
            |s: &mut SpriteSource| s.atlas.width = 0,
            |s: &mut SpriteSource| s.atlas.height = 0,
            |s: &mut SpriteSource| s.atlas.image.clear(),
            |s: &mut SpriteSource| s.regions[0].width = 0,
            |s: &mut SpriteSource| s.regions[0].height = 0,
            |s: &mut SpriteSource| s.regions[0].x = u32::MAX,
            |s: &mut SpriteSource| s.regions[0].y = u32::MAX,
            |s: &mut SpriteSource| s.regions[0].width = 65,
            |s: &mut SpriteSource| s.regions[0].height = 17,
            |s: &mut SpriteSource| s.regions[1].id = s.regions[0].id,
        ] {
            let mut s = source();
            mutate(&mut s);
            assert!(SpriteDocument::new(s).is_err());
        }
    }
    #[test]
    fn rejects_invalid_clips() {
        for mutate in [
            |s: &mut SpriteSource| s.clips[0].id.clear(),
            |s: &mut SpriteSource| s.clips[1].id = s.clips[0].id.clone(),
            |s: &mut SpriteSource| s.clips[0].frames.clear(),
            |s: &mut SpriteSource| s.clips[0].frames[0].region = 999,
            |s: &mut SpriteSource| s.clips[0].frames[0].duration_ms = 0,
            |s: &mut SpriteSource| s.clips[0].frames[0].duration_ms = u64::MAX,
        ] {
            let mut s = source();
            mutate(&mut s);
            assert!(SpriteDocument::new(s).is_err());
        }
    }
    #[test]
    fn strict_versioned_json_rejects_ambiguity() {
        for bad in [
            JSON.replace("\"version\": 1", "\"version\": 2"),
            JSON.replace("\"orr_sprite\"", "\"oram\""),
            JSON.replace("\"version\": 1", "\"version\": 1, \"version\": 1"),
            JSON.replace("\"version\": 1", "\"version\": 1, \"typo\": 1"),
            JSON.replace("\"width\": 64", "\"width\": 64, \"typo\": 1"),
            JSON.replace("\"duration_ms\": 600", "\"duration_ms\": 600, \"typo\": 1"),
            JSON.replace("\"duration_ms\": 600", "\"duration_ms\": -1"),
            JSON.replace("\"mode\": \"loop\"", "\"mode\": \"forever\""),
            format!("{JSON} {{}}"),
        ] {
            assert!(SpriteDocument::from_json(&bad).is_err(), "accepted {bad}");
        }
    }
    #[test]
    fn shipped_pixels_match_declared_dimensions() {
        let rgba = include_bytes!("../../../assets/sprite_demo/lantern_keeper.rgba");
        let d = doc();
        assert_eq!(
            rgba.len(),
            d.atlas().width as usize * d.atlas().height as usize * 4
        );
        assert!(rgba.chunks_exact(4).any(|p| p[3] == 0));
        assert!(rgba.chunks_exact(4).any(|p| p[3] == 255));
        let first: Vec<_> = rgba
            .chunks_exact(64 * 4)
            .flat_map(|r| &r[..16 * 4])
            .collect();
        let walk: Vec<_> = rgba
            .chunks_exact(64 * 4)
            .flat_map(|r| &r[32 * 4..48 * 4])
            .collect();
        assert_ne!(first, walk);
    }
    fn accepts_both(s: SpriteSource) {
        let json = serde_json::to_string(&s).unwrap();
        assert!(SpriteDocument::new(s).is_ok());
        assert!(SpriteDocument::from_json(&json).is_ok());
    }
    fn rejects_both(s: SpriteSource) {
        let json = serde_json::to_string(&s).unwrap();
        assert!(SpriteDocument::new(s).is_err());
        assert!(SpriteDocument::from_json(&json).is_err());
    }
    #[test]
    fn names_and_atlas_limits_are_inclusive_in_both_entrypoints() {
        let mut s = source();
        s.atlas.image = "a".repeat(MAX_IMAGE_NAME_BYTES);
        accepts_both(s.clone());
        s.atlas.image.push('a');
        rejects_both(s);
        let mut s = source();
        s.clips[0].id = "a".repeat(MAX_CLIP_ID_BYTES);
        accepts_both(s.clone());
        s.clips[0].id.push('a');
        rejects_both(s);
        for axis in 0..2 {
            let mut s = source();
            if axis == 0 {
                s.atlas.width = MAX_ATLAS_DIMENSION;
            } else {
                s.atlas.height = MAX_ATLAS_DIMENSION;
            }
            accepts_both(s.clone());
            if axis == 0 {
                s.atlas.width += 1;
            } else {
                s.atlas.height += 1;
            }
            rejects_both(s);
        }
    }
    #[test]
    fn collection_limits_are_inclusive_in_both_entrypoints() {
        let mut s = source();
        s.regions = (0..MAX_REGIONS)
            .map(|id| Region {
                id: id as u32,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            })
            .collect();
        accepts_both(s.clone());
        s.regions.push(Region {
            id: MAX_REGIONS as u32,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        rejects_both(s);
        let mut s = source();
        s.clips = (0..MAX_CLIPS)
            .map(|id| ClipSource {
                id: format!("clip{id}"),
                mode: PlaybackMode::Loop,
                frames: vec![Frame {
                    region: 10,
                    duration_ms: 1,
                }],
            })
            .collect();
        accepts_both(s.clone());
        s.clips.push(ClipSource {
            id: "extra".into(),
            mode: PlaybackMode::Loop,
            frames: vec![Frame {
                region: 10,
                duration_ms: 1,
            }],
        });
        rejects_both(s);
        let mut s = source();
        s.clips[0].frames = vec![
            Frame {
                region: 10,
                duration_ms: 1
            };
            MAX_FRAMES_PER_CLIP
        ];
        accepts_both(s.clone());
        s.clips[0].frames.push(Frame {
            region: 10,
            duration_ms: 1,
        });
        rejects_both(s);
    }
    #[test]
    fn total_frames_limit_is_inclusive_in_both_entrypoints() {
        let mut s = source();
        s.clips = (0..MAX_TOTAL_FRAMES / MAX_FRAMES_PER_CLIP)
            .map(|id| ClipSource {
                id: format!("clip{id}"),
                mode: PlaybackMode::Once,
                frames: vec![
                    Frame {
                        region: 10,
                        duration_ms: 1
                    };
                    MAX_FRAMES_PER_CLIP
                ],
            })
            .collect();
        accepts_both(s.clone());
        // Each clip remains within its individual bound.
        s.clips.push(ClipSource {
            id: "extra".into(),
            mode: PlaybackMode::Once,
            frames: vec![Frame {
                region: 10,
                duration_ms: 1,
            }],
        });
        rejects_both(s);
    }
    #[test]
    fn json_byte_limit_precedes_deserialization() {
        let mut padded = JSON.to_owned();
        padded.extend(std::iter::repeat_n(' ', MAX_JSON_BYTES - JSON.len()));
        assert_eq!(padded.len(), MAX_JSON_BYTES);
        assert!(SpriteDocument::from_json(&padded).is_ok());
        padded.push(' ');
        assert_eq!(
            SpriteDocument::from_json(&padded).unwrap_err().to_string(),
            "sprite JSON exceeds byte limit"
        );
        let invalid = "x".repeat(MAX_JSON_BYTES + 1);
        assert_eq!(
            SpriteDocument::from_json(&invalid).unwrap_err().to_string(),
            "sprite JSON exceeds byte limit"
        );
    }
}
