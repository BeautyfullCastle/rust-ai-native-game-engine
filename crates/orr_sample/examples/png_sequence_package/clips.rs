//! Strict private schema-3 parsing; output remains the existing sprite schema.
use super::{rgba_size, Canvas, Offset, PreparedRecipe, SourceImage, MAX_FRAMES};
use orr_sprite::{
    ClipSource, Frame, PlaybackMode, MAX_CLIPS, MAX_CLIP_ID_BYTES, MAX_FRAMES_PER_CLIP,
    MAX_TOTAL_FRAMES,
};
use serde::{Deserialize, Deserializer};
use std::{collections::BTreeSet, marker::PhantomData};

// Derived struct deserializers also accept positional arrays. These private JSON
// records require objects; map forwarding preserves duplicate-field errors.
struct Object<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }
        deserializer.deserialize_map(ObjectVisitor(PhantomData))
    }
}

// Read only the discriminator, then deserialize the original bytes with the
// selected strict schema. Never normalize JSON through Value, which loses keys.
pub(super) fn schema(bytes: &[u8]) -> Result<u32, String> {
    #[derive(Deserialize)]
    struct Header {
        schema: u32,
    }
    serde_json::from_slice::<Object<Header>>(bytes)
        .map(|header| header.0.schema)
        .map_err(|e| format!("recipe: {e}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: u32,
    name: String,
    version: String,
    license: String,
    columns: u32,
    canvas: Object<Canvas>,
    #[serde(deserialize_with = "bounded_images")]
    frames: Vec<Image>,
    #[serde(deserialize_with = "bounded_clips")]
    clips: Vec<NamedClip>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    png: String,
    offset: Object<Offset>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedClip {
    id: String,
    mode: PlaybackMode,
    #[serde(deserialize_with = "bounded_references")]
    frames: Vec<Reference>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    frame: u32,
    duration_ms: u64,
}

fn bounded_objects<'de, D, T, const LIMIT: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ObjectsVisitor<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> serde::de::Visitor<'de>
        for ObjectsVisitor<T, LIMIT>
    {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "an array of at most {LIMIT} objects")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(Object(value)) = sequence.next_element::<Object<T>>()? {
                if values.len() == LIMIT {
                    return Err(serde::de::Error::custom(format!(
                        "collection count exceeds {LIMIT}"
                    )));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(ObjectsVisitor::<T, LIMIT>(PhantomData))
}
fn bounded_images<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Image>, D::Error> {
    bounded_objects::<D, Image, MAX_FRAMES>(deserializer)
}
fn bounded_references<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Reference>, D::Error> {
    bounded_objects::<D, Reference, MAX_FRAMES_PER_CLIP>(deserializer)
}
fn bounded_clips<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<NamedClip>, D::Error> {
    struct ClipsVisitor;
    impl<'de> serde::de::Visitor<'de> for ClipsVisitor {
        type Value = Vec<NamedClip>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "at most {MAX_CLIPS} clips and {MAX_TOTAL_FRAMES} references"
            )
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut clips = Vec::new();
            let mut references = 0usize;
            while let Some(Object(clip)) = sequence.next_element::<Object<NamedClip>>()? {
                if clips.len() == MAX_CLIPS {
                    return Err(serde::de::Error::custom("clip count exceeds runtime limit"));
                }
                references = references
                    .checked_add(clip.frames.len())
                    .filter(|count| *count <= MAX_TOTAL_FRAMES)
                    .ok_or_else(|| {
                        serde::de::Error::custom("total reference count exceeds runtime limit")
                    })?;
                clips.push(clip);
            }
            Ok(clips)
        }
    }
    deserializer.deserialize_seq(ClipsVisitor)
}

pub(super) fn prepare(bytes: &[u8]) -> Result<PreparedRecipe, String> {
    let Object(recipe): Object<Recipe> =
        serde_json::from_slice(bytes).map_err(|e| format!("recipe: {e}"))?;
    if recipe.schema != 3 {
        return Err("named clips require recipe schema 3".into());
    }
    rgba_size(recipe.canvas.0.width, recipe.canvas.0.height)?;
    if recipe.clips.is_empty() {
        return Err("require at least one named clip".into());
    }
    let mut ids = BTreeSet::new();
    for clip in &recipe.clips {
        if clip.id.trim().is_empty()
            || clip.id.len() > MAX_CLIP_ID_BYTES
            || !ids.insert(clip.id.as_str())
        {
            return Err("invalid or duplicate clip ID".into());
        }
        if clip.frames.is_empty() {
            return Err("require at least one reference per clip".into());
        }
        let mut duration = 0u64;
        for reference in &clip.frames {
            if reference.frame as usize >= recipe.frames.len() {
                return Err("clip frame reference is outside source table".into());
            }
            if reference.duration_ms == 0 {
                return Err("frame duration must be positive".into());
            }
            duration = duration
                .checked_add(reference.duration_ms)
                .ok_or("clip duration overflow")?;
        }
    }
    Ok(PreparedRecipe {
        schema: recipe.schema,
        name: recipe.name,
        version: recipe.version,
        license: recipe.license,
        columns: recipe.columns,
        canvas: Some(recipe.canvas.0),
        frames: recipe
            .frames
            .into_iter()
            .map(|image| SourceImage {
                png: image.png,
                offset: Some(image.offset.0),
            })
            .collect(),
        clips: recipe
            .clips
            .into_iter()
            .map(|clip| ClipSource {
                id: clip.id,
                mode: clip.mode,
                frames: clip
                    .frames
                    .into_iter()
                    .map(|reference| Frame {
                        region: reference.frame,
                        duration_ms: reference.duration_ms,
                    })
                    .collect(),
            })
            .collect(),
    })
}
