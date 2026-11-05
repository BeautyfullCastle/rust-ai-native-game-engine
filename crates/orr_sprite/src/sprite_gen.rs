//! Pure, bounded adapter for materialized sprite-gen component-row manifests.
//! Pinned contract: aldegad/sprite-gen 4f5ccd55, compose/compose_atlas.py.
//! Does not run a generator, resolve paths or decode images. The caller supplies
//! the PNG explicitly; output always uses the portable package leaf `atlas.png`.
//! Explicit duration arrays are required. Legacy fps-only exports are rejected.

use crate::{
    invalid, Atlas, ClipSource, Error, Frame, PlaybackMode, Region, SpriteDocument, SpriteSource,
    FORMAT, MAX_CLIPS, MAX_FRAMES_PER_CLIP, MAX_JSON_BYTES, MAX_REGIONS, MAX_TOTAL_FRAMES, VERSION,
};
use serde::{de, Deserialize, Deserializer};
use std::{collections::BTreeMap, fmt, marker::PhantomData};

#[derive(Deserialize)]
struct Manifest {
    engine: String,
    degraded_static_fallback: bool,
    game_input: String,
    sprite_sheet_alpha: String,
    frame_layout: Layout,
    animation: Animation,
}
#[derive(Deserialize)]
struct Layout {
    #[serde(rename = "sheetWidth")]
    width: u32,
    #[serde(rename = "sheetHeight")]
    height: u32,
    #[serde(deserialize_with = "unique_rows")]
    rows: BTreeMap<String, Vec<Rectangle>>,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Rectangle {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}
#[derive(Deserialize)]
struct Animation {
    #[serde(deserialize_with = "unique_rows")]
    rows: BTreeMap<String, Timing>,
}
#[derive(Deserialize)]
struct Timing {
    row: usize,
    frames: usize,
    durations_ms: Vec<u64>,
    #[serde(rename = "loop")]
    looping: bool,
}

// Map keys are semantic state IDs. Unlike a default BTreeMap deserializer,
// reject duplicate names rather than silently selecting the last declaration.
fn unique_rows<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Rows<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for Rows<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded map of unique sprite state names")
        }
        fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut rows = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if rows.len() >= MAX_CLIPS || rows.contains_key(&key) {
                    return Err(de::Error::custom("too many or duplicate sprite states"));
                }
                rows.insert(key, map.next_value()?);
            }
            Ok(rows)
        }
    }
    deserializer.deserialize_map(Rows(PhantomData))
}

/// Convert the pinned materialized manifest into the engine's validated format.
/// Extra upstream curation/report metadata is ignored. Authoritative geometry,
/// state names, row indexes, sequence lengths, durations and loop flags are not.
pub fn from_json(json: &str) -> Result<SpriteDocument, Error> {
    if json.len() > MAX_JSON_BYTES {
        return Err(invalid("sprite-gen manifest exceeds byte limit"));
    }
    let manifest: Manifest =
        serde_json::from_str(json).map_err(|e| invalid(format!("sprite-gen manifest: {e}")))?;
    if manifest.engine != "component-row" || manifest.degraded_static_fallback {
        return Err(invalid(
            "requires a non-degraded component-row sprite-gen manifest",
        ));
    }
    if manifest.game_input.is_empty() || manifest.game_input != manifest.sprite_sheet_alpha {
        return Err(invalid("sprite-gen atlas names must agree"));
    }
    let layout = manifest.frame_layout;
    let timing = manifest.animation.rows;
    if timing.is_empty() || !layout.rows.keys().eq(timing.keys()) {
        return Err(invalid(
            "sprite-gen layout and animation states must match and be nonempty",
        ));
    }
    let mut ordered: Vec<_> = timing.into_iter().collect();
    ordered.sort_by_key(|(_, timing)| timing.row);
    let mut total = 0usize;
    for (index, (name, timing)) in ordered.iter().enumerate() {
        let frames = &layout.rows[name];
        if timing.row != index
            || frames.is_empty()
            || frames.len() > MAX_FRAMES_PER_CLIP
            || timing.frames != frames.len()
            || timing.durations_ms.len() != frames.len()
        {
            return Err(invalid(
                "sprite-gen row indexes or frame counts are inconsistent",
            ));
        }
        total = total
            .checked_add(frames.len())
            .ok_or_else(|| invalid("frame count overflow"))?;
        if total > MAX_TOTAL_FRAMES {
            return Err(invalid("sprite-gen exceeds total frame limit"));
        }
    }
    let mut ids = BTreeMap::new();
    let mut regions = Vec::new();
    let mut clips = Vec::new();
    for (name, timing) in ordered {
        let mut frames = Vec::new();
        for (rect, duration_ms) in layout.rows[&name].iter().zip(timing.durations_ms) {
            let id = if let Some(id) = ids.get(rect) {
                *id
            } else {
                if regions.len() >= MAX_REGIONS {
                    return Err(invalid("sprite-gen exceeds unique region limit"));
                }
                let id = regions.len() as u32;
                regions.push(Region {
                    id,
                    x: rect.x,
                    y: rect.y,
                    width: rect.w,
                    height: rect.h,
                });
                ids.insert(*rect, id);
                id
            };
            // Shared atlas cells still occupy separate playback positions.
            frames.push(Frame {
                region: id,
                duration_ms,
            });
        }
        clips.push(ClipSource {
            id: name,
            mode: if timing.looping {
                PlaybackMode::Loop
            } else {
                PlaybackMode::Once
            },
            frames,
        });
    }
    SpriteDocument::new(SpriteSource {
        format: FORMAT.into(),
        version: VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width: layout.width,
            height: layout.height,
        },
        regions,
        clips,
    })
}

#[cfg(test)]
#[path = "sprite_gen_tests.rs"]
mod tests;
