use crate::{sha256, Error, Result};
use serde::{Deserialize, Serialize};

/// Validated fields of the separate `orr.asset-release/1` release record.
/// JSON spelling/order is not identity. This is not a signature or code hash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseBinding {
    pub(crate) game: String,
    pub(crate) game_code_id: u64,
    pub(crate) build_id: u64,
    pub(crate) sim_digest: [u8; 32],
    pub(crate) view_digest: [u8; 32],
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireBinding {
    format: String,
    game: String,
    game_code_id: String,
    build_id: String,
    frame_format_version: u32,
    sim_manifest_sha256: String,
    view_manifest_sha256: String,
}

fn decimal(s: &str) -> Result<u64> {
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Binding(
            "IDs must be nonzero canonical decimal strings",
        ));
    }
    s.parse().map_err(|_| Error::Binding("ID exceeds u64"))
}

fn digest(s: &str) -> Result<[u8; 32]> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Binding(
            "SHA-256 must be 64 lowercase hex characters",
        ));
    }
    let mut out = [0; 32];
    for (byte, pair) in out.iter_mut().zip(s.as_bytes().chunks_exact(2)) {
        let nibble = |b| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        *byte = nibble(pair[0]) * 16 + nibble(pair[1]);
    }
    Ok(out)
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl ReleaseBinding {
    /// Strict schema, duplicate-field, decimal, format and frame/build checks.
    /// The preparation gate additionally compares binary-pinned expectations.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        // Shares the fixture's existing 1 MiB input budget; check before JSON allocation.
        if bytes.len() > crate::MAX_INPUT_BYTES {
            return Err(Error::BudgetExceeded);
        }
        let w: WireBinding = serde_json::from_slice(bytes)?;
        if w.format != "orr.asset-release/1"
            || w.frame_format_version != 2
            || w.frame_format_version != orr_ecs::FRAME_FORMAT_VERSION
            || w.game.is_empty()
        {
            return Err(Error::Binding(
                "wrong release format, frame version or empty game",
            ));
        }
        let game_code_id = decimal(&w.game_code_id)?;
        let build_id = decimal(&w.build_id)?;
        if orr_sim::frame_build_id(game_code_id) != build_id {
            return Err(Error::Binding(
                "build ID is not frame_build_id(game_code_id)",
            ));
        }
        Ok(Self {
            game: w.game,
            game_code_id,
            build_id,
            sim_digest: digest(&w.sim_manifest_sha256)?,
            view_digest: digest(&w.view_manifest_sha256)?,
        })
    }

    /// Dedicated fixture/test release writer; does not change deployment v1.
    pub fn to_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(&WireBinding {
            format: "orr.asset-release/1".into(),
            game: self.game.clone(),
            game_code_id: self.game_code_id.to_string(),
            build_id: self.build_id.to_string(),
            frame_format_version: orr_ecs::FRAME_FORMAT_VERSION,
            sim_manifest_sha256: hex(&self.sim_digest),
            view_manifest_sha256: hex(&self.view_digest),
        })?)
    }

    pub fn game(&self) -> &str {
        &self.game
    }
    pub fn game_code_id(&self) -> u64 {
        self.game_code_id
    }
    pub fn build_id(&self) -> u64 {
        self.build_id
    }
    pub fn sim_manifest_sha256(&self) -> [u8; 32] {
        self.sim_digest
    }
    pub fn view_manifest_sha256(&self) -> [u8; 32] {
        self.view_digest
    }
}

/// Release-owner ledger. Preserve this ledger alongside retained releases.
/// A `(game, raw ID)` may name only one sim digest; view-only updates are allowed.
#[derive(Default, Debug)]
pub struct ReleaseIndex {
    releases: Vec<ReleaseBinding>,
}

impl ReleaseIndex {
    pub fn admit(&mut self, binding: &ReleaseBinding) -> Result<()> {
        for old in &self.releases {
            if old.game == binding.game && old.game_code_id == binding.game_code_id {
                if old.sim_digest != binding.sim_digest {
                    return Err(Error::ReleaseIdReuse);
                }
                return Ok(());
            }
        }
        self.releases.push(binding.clone());
        Ok(())
    }
}

/// Exact bytes of one cooked content-addressed object, supplied before launch.
#[derive(Clone, Copy, Debug)]
pub struct ArtifactBytes<'a> {
    pub sha256: [u8; 32],
    pub bytes: &'a [u8],
}

/// Borrowed package input. Preparation performs no filesystem access.
#[derive(Clone, Copy, Debug)]
pub struct BundleBytes<'a> {
    pub sim_manifest: &'a [u8],
    pub view_manifest: &'a [u8],
    pub objects: &'a [ArtifactBytes<'a>],
}

pub(crate) fn validate_objects(
    manifest: orr_asset::Manifest<'_>,
    objects: &[ArtifactBytes<'_>],
) -> Result<()> {
    for entry in manifest.entries() {
        let mut matches = objects
            .iter()
            .filter(|object| object.sha256 == entry.payload_sha256);
        let object = matches.next().ok_or(Error::MissingArtifact)?;
        if matches.next().is_some() {
            return Err(Error::Binding("ambiguous object digest"));
        }
        if u64::try_from(object.bytes.len()).ok() != Some(entry.payload_len) {
            return Err(Error::Asset(orr_asset::AssetError::LengthMismatch));
        }
        if sha256(object.bytes) != entry.payload_sha256 {
            return Err(Error::DigestMismatch);
        }
        if manifest.domain() == orr_asset::Domain::Sim {
            orr_asset::MotionProfileV1::decode(object.bytes)?;
        } else {
            // Structural PCM16 validation only. Float conversion, clip ownership
            // and audio policy belong to the separate opt-in audio integration.
            let sample_rate = u32::from_le_bytes(
                object.bytes[..4]
                    .try_into()
                    .map_err(|_| Error::Binding("PCM header"))?,
            );
            let frames = u32::from_le_bytes(
                object.bytes[4..8]
                    .try_into()
                    .map_err(|_| Error::Binding("PCM header"))?,
            );
            if sample_rate != 48000
                || frames == 0
                || frames > orr_asset::MAX_PCM_FRAMES
                || 8 + u64::from(frames) * 2 != entry.payload_len
            {
                return Err(Error::Binding(
                    "PCM sample rate, frame count or exact length",
                ));
            }
        }
    }
    Ok(())
}
