//! Portable immutable height/hole snapshots, with no filesystem authority.
use super::{Clipboard, Geometry, footprint};
use orr_fp::FP;
use orr_terrain::{MAX_EDITS, MAX_SIDE, Terrain};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    fmt,
    path::{Component, Path},
};
#[cfg(unix)]
use std::{fs, io::Read, path::PathBuf};

const MAX_ASSET_BYTES: usize = 256 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    format: String,
    version: u32,
    cells: [u32; 2],
    spacing_raw: i64,
    #[serde(deserialize_with = "bounded")]
    heights_raw: Vec<i64>,
    #[serde(deserialize_with = "bounded")]
    holes: Vec<bool>,
}
fn bounded<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Values<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for Values<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {MAX_EDITS} patch values")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = seq.next_element()? {
                if values.len() == MAX_EDITS {
                    return Err(de::Error::custom("patch array exceeds edit budget"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    d.deserialize_seq(Values(std::marker::PhantomData))
}
impl Asset {
    fn validate(&self) -> Result<(), String> {
        if self.format != "orr_terrain_patch" || self.version != 1 {
            return Err("Unsupported terrain patch format or version".into());
        }
        if self.cells.iter().any(|n| *n == 0 || *n >= MAX_SIDE) {
            return Err("Patch cell dimensions must be between 1 and 128".into());
        }
        footprint(self.cells)?;
        if self.spacing_raw <= 0 {
            return Err("Patch spacing must be positive".into());
        }
        // Portable snapshots have no origin. A negative source origin can make
        // an extent above i64::MAX legal; reject only spans impossible anywhere.
        if self
            .cells
            .iter()
            .any(|n| u128::from(*n) * self.spacing_raw as u128 > u128::from(u64::MAX))
        {
            return Err("Patch extent exceeds the fixed-point coordinate domain".into());
        }
        let [x, z] = self.cells.map(|n| n as usize);
        if self.heights_raw.len() != (x + 1) * (z + 1) || self.holes.len() != x * z {
            return Err("Patch height or hole count mismatch".into());
        }
        Ok(())
    }
}
impl Clipboard {
    pub(super) fn encode_asset(&self) -> Result<Vec<u8>, String> {
        let asset = Asset {
            format: "orr_terrain_patch".into(),
            version: 1,
            cells: self.cells,
            spacing_raw: self.geometry.spacing.raw(),
            heights_raw: self.heights.iter().map(|v| v.raw()).collect(),
            holes: self.holes.clone(),
        };
        asset.validate()?;
        let bytes = serde_json::to_vec_pretty(&asset).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err("Terrain patch exceeds 256 KiB".into());
        }
        Ok(bytes)
    }
    pub(super) fn decode_asset(bytes: &[u8], target: &Terrain) -> Result<Self, String> {
        if bytes.len() > MAX_ASSET_BYTES {
            return Err("Terrain patch exceeds 256 KiB".into());
        }
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
            return Err("Terrain patch must be a JSON object".into());
        }
        let asset: Asset =
            serde_json::from_slice(bytes).map_err(|e| format!("Invalid terrain patch: {e}"))?;
        asset.validate()?;
        if asset.spacing_raw != target.spacing().raw() {
            return Err("Terrain patch spacing differs from the current terrain".into());
        }
        Ok(Self {
            geometry: Geometry::of(target),
            cells: asset.cells,
            heights: asset.heights_raw.into_iter().map(FP::from_raw).collect(),
            holes: asset.holes,
        })
    }
}

fn checked_path(path: &Path) -> Result<(), String> {
    let text = path.to_str().ok_or("terrain patch path must be Unicode")?;
    if !path.is_absolute()
        || text.len() > 4096
        || text.contains('\0')
        || text
            .split(['/', '\\'])
            .any(|part| part == "." || part == "..")
        || path.components().any(|part| {
            !matches!(
                part,
                Component::RootDir | Component::Normal(_) | Component::Prefix(_)
            )
        })
        || path.extension().and_then(|e| e.to_str()) != Some("json")
    {
        return Err("terrain patch path must be an absolute .json path without dot components, at most 4096 bytes".into());
    }
    Ok(())
}

#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}
#[cfg(unix)]
fn observation(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}
#[cfg(unix)]
type Ancestors = Vec<(PathBuf, (u64, u64))>;
#[cfg(unix)]
fn ancestry(path: &Path) -> Result<Ancestors, String> {
    let mut result = Vec::new();
    for parent in path
        .parent()
        .ok_or("terrain patch path has no parent")?
        .ancestors()
    {
        let metadata = fs::symlink_metadata(parent)
            .map_err(|e| format!("inspect terrain patch parent: {e}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || identity(&metadata).1 == 0 {
            return Err(
                "terrain patch ancestors must be real directories with stable identities".into(),
            );
        }
        result.push((parent.to_path_buf(), identity(&metadata)));
    }
    Ok(result)
}

/// Requires trusted, quiescent directories. Observations detect ordinary
/// replacement, not an adversarial ancestor swap-and-restore race.
pub(super) fn read_patch(path: &Path) -> Result<Vec<u8>, String> {
    checked_path(path)?;
    #[cfg(unix)]
    {
        let parents = ancestry(path)?;
        let before =
            fs::symlink_metadata(path).map_err(|e| format!("inspect terrain patch: {e}"))?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || identity(&before).1 == 0
            || before.len() > MAX_ASSET_BYTES as u64
        {
            return Err(
                "terrain patch must be a regular, bounded file with a stable identity".into(),
            );
        }
        let descriptor = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|e| format!("open terrain patch: {e}"))?;
        let file = fs::File::from(descriptor);
        let held = file.metadata().map_err(|e| e.to_string())?;
        if !held.is_file()
            || observation(&held) != observation(&before)
            || ancestry(path)? != parents
        {
            return Err("terrain patch changed while opening".into());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_ASSET_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read terrain patch: {e}"))?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        let named = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_ASSET_BYTES
            || bytes.len() as u64 != before.len()
            || !named.is_file()
            || named.file_type().is_symlink()
            || observation(&after) != observation(&before)
            || observation(&named) != observation(&before)
            || ancestry(path)? != parents
        {
            return Err("terrain patch changed while reading or exceeded the size bound".into());
        }
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        Err("bounded no-follow terrain patch reads are unsupported on this platform".into())
    }
}

#[cfg(test)]
mod tests;
