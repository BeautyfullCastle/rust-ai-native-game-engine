//! Strict authoring JSON and deterministic source importers for the asset cooker.
//!
//! This module owns only the authoring boundary. It does not read files;
//! callers enforce package-root containment and aggregate byte budgets.

use crate::{CookError, Result};
use orr_asset::{
    AssetRef, Domain, IMPACT_PCM16_TYPE_ID, MAX_PCM_FRAMES, MAX_SIM_RECORDS, MAX_VIEW_RECORDS,
    MOTION_PROFILE_TYPE_ID,
};
use orr_fp::FP;
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// Maximum bytes in one index or source JSON document.
pub const MAX_INPUT_BYTES: usize = 1_048_576;
/// Maximum bytes in one source JSON document.
pub const MAX_SOURCE_BYTES: usize = 65_536;
/// Maximum index entries, including tombstones.
pub const MAX_INDEX_RECORDS: usize = 1_024;
const MAX_JSON_DEPTH: usize = 8;
const INDEX_FORMAT: &str = "orr.asset-index/1";
const PCM_SAMPLE_RATE: u32 = 48_000;
const MAX_PCM_PEAK: u32 = 8_192;
const MAX_MOTION_RAW: i64 = 16 * FP::SCALE;

/// A registered v1 authoring type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetType {
    /// sim.motion_profile/1.
    Motion,
    /// view.impact_pcm16/1.
    Impact,
}

impl AssetType {
    /// Stable registry type identifier used by ORAM manifests.
    pub const fn type_id(self) -> u32 {
        match self {
            Self::Motion => MOTION_PROFILE_TYPE_ID,
            Self::Impact => IMPACT_PCM16_TYPE_ID,
        }
    }

    /// Manifest domain for this asset type.
    pub const fn domain(self) -> Domain {
        match self {
            Self::Motion => Domain::Sim,
            Self::Impact => Domain::View,
        }
    }

    /// Canonical authoring type name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Motion => "sim.motion_profile",
            Self::Impact => "view.impact_pcm16",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "sim.motion_profile" => Some(Self::Motion),
            "view.impact_pcm16" => Some(Self::Impact),
            _ => None,
        }
    }
}

/// One live index record or a permanent deleted-ID marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
    /// A live asset with stable ID, type and package-relative source path.
    Live {
        /// Stable nonzero asset ID.
        id: AssetRef,
        /// Registered asset type.
        asset_type: AssetType,
        /// Canonical portable path relative to the package root.
        source: String,
    },
    /// A deleted ID retained forever to prevent accidental reuse.
    Tombstone {
        /// Stable nonzero asset ID.
        id: AssetRef,
    },
}

impl Entry {
    /// Stable ID carried by this record.
    pub const fn id(&self) -> AssetRef {
        match self {
            Self::Live { id, .. } | Self::Tombstone { id } => *id,
        }
    }
}

/// Validated asset authoring index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Index {
    /// Records, maintained in numeric asset-ID order by parsing and lifecycle
    /// operations. Encoding revalidates this public field before writing.
    pub entries: Vec<Entry>,
}

impl Index {
    /// Parse a bounded, strict orr.asset-index/1 document.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(invalid("asset index exceeds the 1 MiB input limit"));
        }
        check_json_depth(bytes)?;
        let parsed: IndexWire = parse_json(bytes)?;
        if parsed.format != INDEX_FORMAT {
            return Err(invalid("unsupported asset index format"));
        }
        let mut entries = Vec::with_capacity(parsed.entries.0.len());
        for wire in parsed.entries.0 {
            entries.push(wire.into_entry()?);
        }
        validate_entries(&entries)?;
        entries.sort_by_key(|entry| entry.id().get());
        Ok(Self { entries })
    }

    /// Serialize records in numeric-ID order as canonical compact JSON with a
    /// final newline.
    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_entries(&self.entries)?;
        // Bound output before allocation. Paths are restricted to printable,
        // portable characters, so escaping cannot multiply their byte length.
        let mut estimated = 64usize;
        for entry in &self.entries {
            let extra = match entry {
                Entry::Live { source, .. } => source.len().checked_add(128),
                Entry::Tombstone { .. } => Some(64),
            }
            .ok_or_else(|| invalid("asset index size overflow"))?;
            estimated = estimated
                .checked_add(extra)
                .ok_or_else(|| invalid("asset index size overflow"))?;
            if estimated > MAX_INPUT_BYTES {
                return Err(invalid("encoded asset index exceeds the 1 MiB input limit"));
            }
        }
        let mut ordered: Vec<&Entry> = self.entries.iter().collect();
        ordered.sort_by_key(|entry| entry.id().get());
        let mut bytes = serde_json::to_vec(&IndexOut { entries: ordered })?;
        if bytes
            .len()
            .checked_add(1)
            .filter(|len| *len <= MAX_INPUT_BYTES)
            .is_none()
        {
            return Err(invalid("encoded asset index exceeds the 1 MiB input limit"));
        }
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Ensure each declared initial root is nonnull, present and live.
    pub fn validate_roots(&self, roots: &[AssetRef]) -> Result<()> {
        for root in roots {
            if root.is_null() {
                return Err(invalid("asset roots cannot contain the null ID"));
            }
            match self.entries.iter().find(|entry| entry.id() == *root) {
                Some(Entry::Live { .. }) => {}
                Some(Entry::Tombstone { .. }) => {
                    return Err(invalid(format!("asset root {root} is tombstoned")))
                }
                None => return Err(invalid(format!("asset root {root} is missing"))),
            }
        }
        Ok(())
    }

    /// Register an ID that has never appeared in this index.
    pub fn register(&mut self, id: AssetRef, asset_type: AssetType, source: String) -> Result<()> {
        ensure_nonnull(id)?;
        validate_source_path(&source)?;
        self.ensure_new_id_and_source(id, &source)?;
        self.ensure_type_capacity(asset_type)?;
        if self.entries.len() >= MAX_INDEX_RECORDS {
            return Err(invalid("asset index exceeds the 1024-record limit"));
        }
        self.entries.push(Entry::Live {
            id,
            asset_type,
            source,
        });
        self.entries.sort_by_key(|entry| entry.id().get());
        Ok(())
    }

    /// Copy the type of a live record to a new ID and source path.
    pub fn clone_entry(
        &mut self,
        from: AssetRef,
        new_id: AssetRef,
        new_source: String,
    ) -> Result<()> {
        ensure_nonnull(new_id)?;
        validate_source_path(&new_source)?;
        self.ensure_new_id_and_source(new_id, &new_source)?;
        if self.entries.len() >= MAX_INDEX_RECORDS {
            return Err(invalid("asset index exceeds the 1024-record limit"));
        }
        let asset_type = match self.entries.iter().find(|entry| entry.id() == from) {
            Some(Entry::Live { asset_type, .. }) => *asset_type,
            Some(Entry::Tombstone { .. }) => {
                return Err(invalid(format!("cannot clone tombstoned asset {from}")))
            }
            None => return Err(invalid(format!("cannot clone missing asset {from}"))),
        };
        self.ensure_type_capacity(asset_type)?;
        self.entries.push(Entry::Live {
            id: new_id,
            asset_type,
            source: new_source,
        });
        self.entries.sort_by_key(|entry| entry.id().get());
        Ok(())
    }

    /// Move or rename a source path without changing the asset identity.
    pub fn move_source(&mut self, id: AssetRef, new_source: String) -> Result<()> {
        ensure_nonnull(id)?;
        validate_source_path(&new_source)?;
        self.ensure_source_available(id, &new_source)?;
        match self.entries.iter_mut().find(|entry| entry.id() == id) {
            Some(Entry::Live { source, .. }) => {
                *source = new_source;
                Ok(())
            }
            Some(Entry::Tombstone { .. }) => Err(invalid(format!("asset {id} is tombstoned"))),
            None => Err(invalid(format!("asset {id} is missing"))),
        }
    }

    /// Tombstone a live ID. A declared fixture root cannot be removed and a
    /// tombstoned ID can never be registered again.
    pub fn tombstone(&mut self, id: AssetRef, roots: &[AssetRef]) -> Result<()> {
        ensure_nonnull(id)?;
        self.validate_roots(roots)?;
        if roots.contains(&id) {
            return Err(invalid(format!("cannot tombstone referenced root {id}")));
        }
        match self.entries.iter_mut().find(|entry| entry.id() == id) {
            Some(entry @ Entry::Live { .. }) => {
                *entry = Entry::Tombstone { id };
                Ok(())
            }
            Some(Entry::Tombstone { .. }) => {
                Err(invalid(format!("asset {id} is already tombstoned")))
            }
            None => Err(invalid(format!("asset {id} is missing"))),
        }
    }

    fn ensure_new_id_and_source(&self, id: AssetRef, source: &str) -> Result<()> {
        if self.entries.iter().any(|entry| entry.id() == id) {
            return Err(invalid(format!("asset ID {id} has already been used")));
        }
        self.ensure_source_available(id, source)
    }

    fn ensure_source_available(&self, id: AssetRef, source: &str) -> Result<()> {
        let source_key = source.to_lowercase();
        if self.entries.iter().any(|entry| {
            matches!(entry, Entry::Live { id: other_id, source: other_source, .. }
                if *other_id != id && other_source.to_lowercase() == source_key)
        }) {
            return Err(invalid(format!(
                "case-insensitive source path {source:?} is already registered"
            )));
        }
        Ok(())
    }

    fn ensure_type_capacity(&self, asset_type: AssetType) -> Result<()> {
        let limit = match asset_type.domain() {
            Domain::Sim => MAX_SIM_RECORDS,
            Domain::View => MAX_VIEW_RECORDS,
        };
        let count = self.entries.iter().filter(|entry| {
            matches!(entry, Entry::Live { asset_type: kind, .. } if *kind == asset_type)
        }).count();
        if count >= limit {
            return Err(invalid(format!(
                "{} live records exceed the per-domain limit of {limit}",
                asset_type.name()
            )));
        }
        Ok(())
    }
}

/// Check for a canonical, portable relative UTF-8 source path.
pub fn validate_source_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(invalid(
            "source path must be a nonempty relative path using '/'",
        ));
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(invalid("drive-qualified source paths are not allowed"));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid(
                "source path contains an empty, '.' or '..' component",
            ));
        }
        if component.ends_with(' ') || component.ends_with('.') {
            return Err(invalid(
                "source path components cannot end in a space or dot",
            ));
        }
        if is_windows_reserved_component(component) {
            return Err(invalid(
                "source path contains a reserved Windows device name",
            ));
        }
        if component
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        {
            return Err(invalid("source path contains a nonportable character"));
        }
    }
    Ok(())
}

/// Import one bounded v1 source to its deterministic binary payload.
pub fn import_source(asset_type: AssetType, bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(invalid("asset source exceeds the 64 KiB source limit"));
    }
    check_json_depth(bytes)?;
    match asset_type {
        AssetType::Motion => import_motion(bytes),
        AssetType::Impact => import_impact(bytes),
    }
}

/// Validate a cooked payload for its registered type.
pub fn validate_payload(asset_type: AssetType, bytes: &[u8]) -> Result<()> {
    match asset_type {
        AssetType::Motion => {
            if bytes.len() != 8 {
                return Err(invalid("motion payload must be exactly 8 bytes"));
            }
            let raw = i64::from_le_bytes(bytes.try_into().expect("length checked"));
            if raw <= 0 || raw > MAX_MOTION_RAW {
                return Err(invalid("motion speed must be in (0, 16]"));
            }
            Ok(())
        }
        AssetType::Impact => {
            if bytes.len() < 8 {
                return Err(invalid("PCM payload is shorter than its 8-byte header"));
            }
            let sample_rate = u32::from_le_bytes(bytes[0..4].try_into().expect("slice length"));
            let frames = u32::from_le_bytes(bytes[4..8].try_into().expect("slice length"));
            if sample_rate != PCM_SAMPLE_RATE {
                return Err(invalid("PCM sample rate must be exactly 48000 Hz"));
            }
            if !(1..=MAX_PCM_FRAMES).contains(&frames) {
                return Err(invalid("PCM frame count must be in 1..=48000"));
            }
            let sample_bytes = usize::try_from(frames)
                .ok()
                .and_then(|count| count.checked_mul(2))
                .ok_or_else(|| invalid("PCM frame length overflow"))?;
            let expected = 8usize
                .checked_add(sample_bytes)
                .ok_or_else(|| invalid("PCM payload length overflow"))?;
            if bytes.len() != expected {
                return Err(invalid("PCM payload length does not match its frame count"));
            }
            for sample in bytes[8..].chunks_exact(2) {
                let value = i16::from_le_bytes([sample[0], sample[1]]) as i32;
                if value.abs() > MAX_PCM_PEAK as i32 {
                    return Err(invalid("PCM sample peak exceeds 8192"));
                }
            }
            Ok(())
        }
    }
}

fn import_motion(bytes: &[u8]) -> Result<Vec<u8>> {
    let source: MotionSource = parse_json(bytes)?;
    if !is_decimal(&source.speed_per_tick) {
        return Err(invalid(
            "speed_per_tick must be a plain decimal string without exponent or whitespace",
        ));
    }
    let speed = FP::parse(&source.speed_per_tick)
        .map_err(|_| invalid("speed_per_tick is not a valid fixed-point decimal"))?;
    if speed.raw() <= 0 || speed.raw() > MAX_MOTION_RAW {
        return Err(invalid("speed_per_tick must round to a value in (0, 16]"));
    }
    let payload = speed.raw().to_le_bytes().to_vec();
    validate_payload(AssetType::Motion, &payload)?;
    Ok(payload)
}

fn import_impact(bytes: &[u8]) -> Result<Vec<u8>> {
    let source: ImpactSource = parse_json(bytes)?;
    if source.generator != "triangle_decay_v1" {
        return Err(invalid("unsupported impact PCM generator"));
    }
    if source.sample_rate != PCM_SAMPLE_RATE {
        return Err(invalid("impact PCM sample_rate must be exactly 48000"));
    }
    if !(1..=MAX_PCM_FRAMES).contains(&source.frames) {
        return Err(invalid("impact PCM frames must be in 1..=48000"));
    }
    if !(4..=48_000).contains(&source.period_frames) || source.period_frames % 4 != 0 {
        return Err(invalid(
            "period_frames must be a multiple of 4 in 4..=48000",
        ));
    }
    if !(1..=MAX_PCM_PEAK).contains(&source.peak_pcm16) {
        return Err(invalid("peak_pcm16 must be in 1..=8192"));
    }
    let sample_bytes = usize::try_from(source.frames)
        .ok()
        .and_then(|frames| frames.checked_mul(2))
        .ok_or_else(|| invalid("impact PCM payload length overflow"))?;
    let payload_len = 8usize
        .checked_add(sample_bytes)
        .ok_or_else(|| invalid("impact PCM payload length overflow"))?;
    let mut payload = Vec::with_capacity(payload_len);
    payload.extend_from_slice(&source.sample_rate.to_le_bytes());
    payload.extend_from_slice(&source.frames.to_le_bytes());
    let period = i64::from(source.period_frames);
    let frames = i64::from(source.frames);
    let peak = i64::from(source.peak_pcm16);
    let denominator = period * frames;
    for i in 0..source.frames {
        let i64_i = i64::from(i);
        let phase = i64::from(i % source.period_frames);
        let triangle = if phase < period / 2 {
            4 * phase - period
        } else {
            3 * period - 4 * phase
        };
        // Bounded v1 parameters keep products in i64; signed division is
        // truncation toward zero, as required by the PCM format.
        let sample = peak * triangle * (frames - i64_i) / denominator;
        let sample = i16::try_from(sample)
            .map_err(|_| invalid("generated PCM sample is outside signed 16-bit range"))?;
        payload.extend_from_slice(&sample.to_le_bytes());
    }
    validate_payload(AssetType::Impact, &payload)?;
    Ok(payload)
}

fn is_decimal(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let i = if matches!(bytes[0], b'+' | b'-') {
        1
    } else {
        0
    };
    if i == bytes.len() {
        return false;
    }
    let mut digits = 0usize;
    let mut dots = 0usize;
    for &byte in &bytes[i..] {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' => {
                dots += 1;
                if dots > 1 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    digits > 0
}

fn ensure_nonnull(id: AssetRef) -> Result<()> {
    if id.is_null() {
        Err(invalid("asset ID zero is reserved for null"))
    } else {
        Ok(())
    }
}

fn is_windows_reserved_component(component: &str) -> bool {
    let base = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    base.eq_ignore_ascii_case("CON")
        || base.eq_ignore_ascii_case("PRN")
        || base.eq_ignore_ascii_case("AUX")
        || base.eq_ignore_ascii_case("NUL")
        || (base.len() == 4
            && (base.as_bytes()[..3].eq_ignore_ascii_case(b"COM")
                || base.as_bytes()[..3].eq_ignore_ascii_case(b"LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

fn validate_entries(entries: &[Entry]) -> Result<()> {
    if entries.len() > MAX_INDEX_RECORDS {
        return Err(invalid("asset index exceeds the 1024-record limit"));
    }
    let mut sim_count = 0usize;
    let mut view_count = 0usize;
    let mut source_keys = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        ensure_nonnull(entry.id())?;
        if let Entry::Live {
            asset_type, source, ..
        } = entry
        {
            match asset_type.domain() {
                Domain::Sim => sim_count += 1,
                Domain::View => view_count += 1,
            }
            validate_source_path(source)?;
            if !source_keys.insert(source.to_lowercase()) {
                return Err(invalid(format!(
                    "duplicate case-insensitive asset source path {source:?}"
                )));
            }
        }
        if entries[..index]
            .iter()
            .any(|prior| prior.id() == entry.id())
        {
            return Err(invalid(format!("duplicate asset ID {}", entry.id())));
        }
    }
    if sim_count > MAX_SIM_RECORDS {
        return Err(invalid(format!(
            "sim records exceed the limit of {MAX_SIM_RECORDS}"
        )));
    }
    if view_count > MAX_VIEW_RECORDS {
        return Err(invalid(format!(
            "view records exceed the limit of {MAX_VIEW_RECORDS}"
        )));
    }
    Ok(())
}

fn parse_id(text: &str) -> Result<AssetRef> {
    let id = AssetRef::from_str(text)
        .map_err(|_| invalid(format!("invalid canonical asset ID {text:?}")))?;
    ensure_nonnull(id)?;
    Ok(id)
}

fn check_json_depth(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(invalid("JSON input exceeds the 1 MiB input limit"));
    }
    std::str::from_utf8(bytes).map_err(|_| invalid("JSON input is not valid UTF-8"))?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| invalid("JSON nesting depth overflow"))?;
                if depth > MAX_JSON_DEPTH {
                    return Err(invalid("JSON nesting exceeds the fixed depth limit"));
                }
            }
            b'}' | b']' => {
                if depth == 0 {
                    return Err(invalid("JSON has an unmatched closing container"));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

fn invalid(message: impl Into<String>) -> CookError {
    CookError::invalid(message)
}

struct IndexWire {
    format: String,
    entries: EntriesWire,
}

impl<'de> Deserialize<'de> for IndexWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct IndexVisitor;
        impl<'de> Visitor<'de> for IndexVisitor {
            type Value = IndexWire;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an orr.asset-index/1 object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut format: Option<String> = None;
                let mut entries: Option<EntriesWire> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => {
                            if format.is_some() {
                                return Err(de::Error::duplicate_field("format"));
                            }
                            format = Some(map.next_value()?);
                        }
                        "entries" => {
                            if entries.is_some() {
                                return Err(de::Error::duplicate_field("entries"));
                            }
                            entries = Some(map.next_value()?);
                        }
                        _ => return Err(de::Error::unknown_field(&key, &["format", "entries"])),
                    }
                }
                Ok(IndexWire {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    entries: entries.ok_or_else(|| de::Error::missing_field("entries"))?,
                })
            }
        }
        deserializer.deserialize_map(IndexVisitor)
    }
}

struct EntriesWire(Vec<EntryWire>);

impl<'de> Deserialize<'de> for EntriesWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct EntriesVisitor;
        impl<'de> Visitor<'de> for EntriesVisitor {
            type Value = EntriesWire;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an array of at most 1024 asset records")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let reserve = seq.size_hint().unwrap_or(0).min(MAX_INDEX_RECORDS);
                let mut entries = Vec::new();
                entries
                    .try_reserve(reserve)
                    .map_err(|_| de::Error::custom("could not reserve bounded index entries"))?;
                loop {
                    if entries.len() == MAX_INDEX_RECORDS {
                        if seq.next_element::<IgnoredAny>()?.is_some() {
                            return Err(de::Error::custom(
                                "asset index exceeds the 1024-record limit",
                            ));
                        }
                        break;
                    }
                    match seq.next_element::<EntryWire>()? {
                        Some(entry) => entries.push(entry),
                        None => break,
                    }
                }
                Ok(EntriesWire(entries))
            }
        }
        deserializer.deserialize_seq(EntriesVisitor)
    }
}

struct EntryWire {
    id: String,
    asset_type: Option<String>,
    schema_version: Option<u32>,
    source: Option<String>,
    tombstone: Option<bool>,
}

impl EntryWire {
    fn into_entry(self) -> Result<Entry> {
        let id = parse_id(&self.id)?;
        match (
            self.asset_type,
            self.schema_version,
            self.source,
            self.tombstone,
        ) {
            (Some(name), Some(1), Some(source), None) => {
                let asset_type = AssetType::from_name(&name)
                    .ok_or_else(|| invalid(format!("unsupported asset type {name:?}")))?;
                validate_source_path(&source)?;
                Ok(Entry::Live {
                    id,
                    asset_type,
                    source,
                })
            }
            (None, None, None, Some(true)) => Ok(Entry::Tombstone { id }),
            (Some(_), Some(version), _, None) if version != 1 => Err(invalid(format!(
                "unsupported asset schema version {version}"
            ))),
            _ => Err(invalid(
                "asset entry fields do not match live or tombstone schema",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for EntryWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct EntryVisitor;
        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = EntryWire;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a live asset record or tombstone")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut id: Option<String> = None;
                let mut asset_type: Option<String> = None;
                let mut schema_version: Option<u32> = None;
                let mut source: Option<String> = None;
                let mut tombstone: Option<bool> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => {
                            if id.is_some() {
                                return Err(de::Error::duplicate_field("id"));
                            }
                            id = Some(map.next_value()?);
                        }
                        "type" => {
                            if asset_type.is_some() {
                                return Err(de::Error::duplicate_field("type"));
                            }
                            asset_type = Some(map.next_value()?);
                        }
                        "schema_version" => {
                            if schema_version.is_some() {
                                return Err(de::Error::duplicate_field("schema_version"));
                            }
                            schema_version = Some(map.next_value()?);
                        }
                        "source" => {
                            if source.is_some() {
                                return Err(de::Error::duplicate_field("source"));
                            }
                            source = Some(map.next_value()?);
                        }
                        "tombstone" => {
                            if tombstone.is_some() {
                                return Err(de::Error::duplicate_field("tombstone"));
                            }
                            tombstone = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["id", "type", "schema_version", "source", "tombstone"],
                            ))
                        }
                    }
                }
                Ok(EntryWire {
                    id: id.ok_or_else(|| de::Error::missing_field("id"))?,
                    asset_type,
                    schema_version,
                    source,
                    tombstone,
                })
            }
        }
        deserializer.deserialize_map(EntryVisitor)
    }
}

struct MotionSource {
    speed_per_tick: String,
}

struct ImpactSource {
    generator: String,
    sample_rate: u32,
    frames: u32,
    period_frames: u32,
    peak_pcm16: u32,
}

impl<'de> Deserialize<'de> for MotionSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct MotionVisitor;
        impl<'de> Visitor<'de> for MotionVisitor {
            type Value = MotionSource;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a motion source object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut speed_per_tick = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "speed_per_tick" => {
                            if speed_per_tick.is_some() {
                                return Err(de::Error::duplicate_field("speed_per_tick"));
                            }
                            speed_per_tick = Some(map.next_value()?);
                        }
                        _ => return Err(de::Error::unknown_field(&key, &["speed_per_tick"])),
                    }
                }
                Ok(MotionSource {
                    speed_per_tick: speed_per_tick
                        .ok_or_else(|| de::Error::missing_field("speed_per_tick"))?,
                })
            }
        }
        deserializer.deserialize_map(MotionVisitor)
    }
}

impl<'de> Deserialize<'de> for ImpactSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ImpactVisitor;
        impl<'de> Visitor<'de> for ImpactVisitor {
            type Value = ImpactSource;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an impact PCM source object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut generator = None;
                let mut sample_rate = None;
                let mut frames = None;
                let mut period_frames = None;
                let mut peak_pcm16 = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "generator" => {
                            if generator.is_some() {
                                return Err(de::Error::duplicate_field("generator"));
                            }
                            generator = Some(map.next_value()?);
                        }
                        "sample_rate" => {
                            if sample_rate.is_some() {
                                return Err(de::Error::duplicate_field("sample_rate"));
                            }
                            sample_rate = Some(map.next_value()?);
                        }
                        "frames" => {
                            if frames.is_some() {
                                return Err(de::Error::duplicate_field("frames"));
                            }
                            frames = Some(map.next_value()?);
                        }
                        "period_frames" => {
                            if period_frames.is_some() {
                                return Err(de::Error::duplicate_field("period_frames"));
                            }
                            period_frames = Some(map.next_value()?);
                        }
                        "peak_pcm16" => {
                            if peak_pcm16.is_some() {
                                return Err(de::Error::duplicate_field("peak_pcm16"));
                            }
                            peak_pcm16 = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &[
                                    "generator",
                                    "sample_rate",
                                    "frames",
                                    "period_frames",
                                    "peak_pcm16",
                                ],
                            ))
                        }
                    }
                }
                Ok(ImpactSource {
                    generator: generator.ok_or_else(|| de::Error::missing_field("generator"))?,
                    sample_rate: sample_rate
                        .ok_or_else(|| de::Error::missing_field("sample_rate"))?,
                    frames: frames.ok_or_else(|| de::Error::missing_field("frames"))?,
                    period_frames: period_frames
                        .ok_or_else(|| de::Error::missing_field("period_frames"))?,
                    peak_pcm16: peak_pcm16.ok_or_else(|| de::Error::missing_field("peak_pcm16"))?,
                })
            }
        }
        deserializer.deserialize_map(ImpactVisitor)
    }
}

struct IndexOut<'a> {
    entries: Vec<&'a Entry>,
}

impl Serialize for IndexOut<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut structure = serializer.serialize_struct("Index", 2)?;
        structure.serialize_field("format", INDEX_FORMAT)?;
        structure.serialize_field("entries", &EntriesOut(&self.entries))?;
        structure.end()
    }
}

struct EntriesOut<'a>(&'a [&'a Entry]);

impl Serialize for EntriesOut<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            sequence.serialize_element(&EntryOut(entry))?;
        }
        sequence.end()
    }
}

struct EntryOut<'a>(&'a Entry);

impl Serialize for EntryOut<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            Entry::Live {
                id,
                asset_type,
                source,
            } => {
                map.serialize_entry("id", &id.to_string())?;
                map.serialize_entry("type", asset_type.name())?;
                map.serialize_entry("schema_version", &1u32)?;
                map.serialize_entry("source", source)?;
            }
            Entry::Tombstone { id } => {
                map.serialize_entry("id", &id.to_string())?;
                map.serialize_entry("tombstone", &true)?;
            }
        }
        map.end()
    }
}
