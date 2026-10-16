//! Optional presentation owner. Preload once, then pass each atomic
//! `Bridge::poll_view()` result once to [`FixtureAudio::update`]. The same batch
//! can also be read by the visual view; never drain/poll again for audio.
//!
//! A prepared fixture is mandatory. Presentation failure can mute this owner,
//! but cannot bypass the strict package or replay admission gates.
#![allow(clippy::float_arithmetic)]

use crate::{sha256, ArtifactBytes, Error, Impact, PreparedFixture, Result, IMPACT_ID};
use orr_asset::{AssetRef, Domain, Manifest};
use orr_audio::{AudioConfig, AudioStats, Clip, OfflineAudio};
use orr_bridge::ViewUpdate;

pub const SAMPLE_RATE: u32 = 48_000;
pub const MAX_DECODED_BYTES: usize = 4 * 1024 * 1024;
const STEREO_FRAME_BYTES: usize = 2 * std::mem::size_of::<f32>();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioMode {
    /// No validation, decoding, mixer or device is opened by the view owner.
    Off,
    /// Return an explicit muted diagnostic if presentation cannot be prepared.
    Auto,
    /// Any missing/corrupt/over-budget presentation data fails construction.
    Required,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioStatus {
    Off,
    /// Real Kira renderer, no physical output device.
    OfflineReady,
    Muted {
        reason: String,
    },
}

/// View-only borrowed bytes. No filesystem lookup, fallback asset or path input.
#[derive(Clone, Copy, Debug)]
pub struct ViewBundleBytes<'a> {
    pub manifest: &'a [u8],
    pub objects: &'a [ArtifactBytes<'a>],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreloadStats {
    pub records: usize,
    pub manifest_bytes: usize,
    /// Sum per manifest record (shared payloads are conservatively charged again).
    pub cooked_bytes: usize,
    /// Retained stereo f32 sample bytes, excluding mixer and allocator overhead.
    pub decoded_bytes: usize,
}

struct ClipBank {
    clips: Vec<(AssetRef, Clip)>,
    stats: PreloadStats,
}

fn bounded_add(total: usize, extra: usize, limit: usize) -> Result<usize> {
    let next = total.checked_add(extra).ok_or(Error::BudgetExceeded)?;
    if next > limit {
        return Err(Error::BudgetExceeded);
    }
    Ok(next)
}

impl ClipBank {
    fn preload(fixture: &PreparedFixture, bundle: ViewBundleBytes<'_>) -> Result<Self> {
        Self::preload_inner(
            fixture,
            bundle,
            #[cfg(feature = "decode-memory-probe")]
            || {},
        )
    }

    fn preload_inner(
        fixture: &PreparedFixture,
        bundle: ViewBundleBytes<'_>,
        #[cfg(feature = "decode-memory-probe")] mut on_conversion: impl FnMut(),
    ) -> Result<Self> {
        let manifest = Manifest::decode(bundle.manifest, Domain::View)?;
        if sha256(bundle.manifest) != fixture.release().view_manifest_sha256() {
            return Err(Error::DigestMismatch);
        }
        manifest.find(IMPACT_ID)?;
        if bundle.objects.len() > orr_asset::MAX_VIEW_RECORDS {
            return Err(Error::BudgetExceeded);
        }
        let mut actual_bytes = 0;
        for object in bundle.objects {
            actual_bytes = bounded_add(
                actual_bytes,
                object.bytes.len(),
                orr_asset::MAX_VIEW_PAYLOAD_BYTES as usize,
            )?;
            if !manifest
                .entries()
                .any(|entry| entry.payload_sha256 == object.sha256)
            {
                return Err(Error::Binding("unreferenced view object"));
            }
        }
        // Lengths, exact SHA-256, unique object lookup, PCM header/rate/count.
        // This is the same integer validator used by strict package admission.
        crate::binding::validate_objects(manifest, bundle.objects)?;
        let mut stats = PreloadStats {
            records: manifest.entries().len(),
            manifest_bytes: bundle.manifest.len(),
            ..PreloadStats::default()
        };
        // Admit the whole bank before any decoded sample allocation. Every
        // payload has an exact validated 8-byte header and 1..=48000 i16 frames.
        for entry in manifest.entries() {
            let bytes = usize::try_from(entry.payload_len).map_err(|_| Error::BudgetExceeded)?;
            stats.cooked_bytes = bounded_add(
                stats.cooked_bytes,
                bytes,
                orr_asset::MAX_VIEW_PAYLOAD_BYTES as usize,
            )?;
            let decoded = ((bytes - 8) / 2)
                .checked_mul(STEREO_FRAME_BYTES)
                .ok_or(Error::BudgetExceeded)?;
            stats.decoded_bytes = bounded_add(stats.decoded_bytes, decoded, MAX_DECODED_BYTES)?;
        }
        let mut clips = Vec::with_capacity(stats.records);
        for entry in manifest.entries() {
            let object = bundle
                .objects
                .iter()
                .find(|object| object.sha256 == entry.payload_sha256)
                .ok_or(Error::MissingArtifact)?;
            // Diagnostic callback is absent from the default build. It runs
            // after whole-bank admission and immediately before conversion.
            #[cfg(feature = "decode-memory-probe")]
            on_conversion();
            let frames = object.bytes[8..]
                .chunks_exact(2)
                .map(|sample| {
                    let value = f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0;
                    [value, value]
                })
                .collect();
            let clip = Clip::from_stereo(SAMPLE_RATE, frames).map_err(Error::Audio)?;
            clips.push((entry.id, clip));
        }
        Ok(Self { clips, stats })
    }

    fn clip(&self, event: &Impact) -> Option<Clip> {
        // Source-owned cue mapping, never an unvalidated event-supplied GUID.
        if event.cue != 1 {
            return None;
        }
        self.clips
            .binary_search_by_key(&IMPACT_ID, |(id, _)| *id)
            .ok()
            .map(|index| self.clips[index].1.clone())
    }
}

/// Opt-in diagnostic owner for the real preload helper, without a mixer.
///
/// The supplied capability and bundle pass exactly the normal strict admission
/// checks. The callback marks each admitted record's conversion start; the
/// observer must avoid allocating or printing inside it. Retain this owner to
/// observe the returned bank's allocation lifetimes, then drop it explicitly.
/// It exposes neither clips nor mutable simulation/package state.
#[cfg(feature = "decode-memory-probe")]
pub struct DecodeProbeBank(ClipBank);

#[cfg(feature = "decode-memory-probe")]
impl DecodeProbeBank {
    pub fn preload(
        fixture: &PreparedFixture,
        bundle: ViewBundleBytes<'_>,
        on_conversion: impl FnMut(),
    ) -> Result<Self> {
        ClipBank::preload_inner(fixture, bundle, on_conversion).map(Self)
    }

    pub fn stats(&self) -> PreloadStats {
        self.0.stats
    }
}

struct ActiveAudio {
    bank: ClipBank,
    mixer: OfflineAudio,
}

/// Owns immutable preloaded clips and the bounded offline event mixer. It has no
/// path back to simulation, mutable Frame, session, replay or cooked bytes.
/// Default mixer limits stay at 32 voices and 4096 history entries.
pub struct FixtureAudio {
    status: AudioStatus,
    active: Option<ActiveAudio>,
}

impl FixtureAudio {
    pub fn open_offline(
        fixture: &PreparedFixture,
        mode: AudioMode,
        bundle: ViewBundleBytes<'_>,
    ) -> Result<Self> {
        if mode == AudioMode::Off {
            return Ok(Self {
                status: AudioStatus::Off,
                active: None,
            });
        }
        let prepare = || -> Result<ActiveAudio> {
            let bank = ClipBank::preload(fixture, bundle)?;
            let mixer = OfflineAudio::new_offline(AudioConfig::default(), SAMPLE_RATE)
                .map_err(Error::Audio)?;
            Ok(ActiveAudio { bank, mixer })
        };
        match prepare() {
            Ok(active) => Ok(Self {
                status: AudioStatus::OfflineReady,
                active: Some(active),
            }),
            Err(error) if mode == AudioMode::Auto => Ok(Self {
                status: AudioStatus::Muted {
                    reason: error.to_string(),
                },
                active: None,
            }),
            Err(error) => Err(error),
        }
    }

    /// Convenience for the checked-in view bytes. Still compares the supplied
    /// admitted release's full view digest, so a different release cannot silently
    /// substitute the embedded clip.
    pub fn embedded_offline(fixture: &PreparedFixture, mode: AudioMode) -> Result<Self> {
        Self::open_offline(
            fixture,
            mode,
            ViewBundleBytes {
                manifest: crate::VIEW_BYTES,
                objects: &[ArtifactBytes {
                    sha256: sha256(crate::IMPACT_BYTES),
                    bytes: crate::IMPACT_BYTES,
                }],
            },
        )
    }

    pub fn status(&self) -> &AudioStatus {
        &self.status
    }
    pub fn preload_stats(&self) -> PreloadStats {
        self.active
            .as_ref()
            .map_or(PreloadStats::default(), |a| a.bank.stats)
    }
    pub fn stats(&self) -> AudioStats {
        self.active
            .as_ref()
            .map_or(AudioStats::default(), |a| a.mixer.stats())
    }
    pub fn voice_count(&self) -> usize {
        self.active.as_ref().map_or(0, |a| a.mixer.voice_count())
    }
    pub fn history_len(&self) -> usize {
        self.active.as_ref().map_or(0, |a| a.mixer.history_len())
    }

    /// Use a stable source ID per bridge/session owner, and a fresh ID when that
    /// owner is replaced. Forward lifecycle/resync unchanged and in order. Do not
    /// derive source IDs from a snapshot epoch or feed a batch through two paths.
    pub fn update(&mut self, source: u64, update: &ViewUpdate<Impact>) {
        if let Some(active) = &mut self.active {
            active
                .mixer
                .update(source, update, |event| active.bank.clip(event));
        }
    }

    /// Render interleaved stereo with the actual Kira offline backend. Off/muted
    /// owners produce explicit silence. No device is ever opened by this module.
    pub fn render(&mut self, stereo: &mut [f32]) -> Result<()> {
        if let Some(active) = &mut self.active {
            active.mixer.render(stereo).map_err(Error::Audio)?;
        } else {
            stereo.fill(0.0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
