//! Bounded portable cell assets. File contents never grant filesystem authority.
use orr_sprite::SpriteDocument;
use orr_tilemap::ValidatedDocument;
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    fmt,
    path::{Component, Path},
};
#[cfg(unix)]
use std::{fs, io::Read, path::PathBuf};

// A 4 MiB atlas can expand sixfold when escaped inside the outer JSON string.
pub(super) const MAX_ASSET_BYTES: usize = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Asset {
    format: String,
    version: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    #[serde(deserialize_with = "bounded_cells")]
    pub(super) cells: Vec<Option<u32>>,
    tile_size: [u32; 2],
    sprite_package: String,
    sprite_document: String,
    package_digest: String,
    atlas_json: String,
}

fn bounded_cells<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Option<u32>>, D::Error> {
    struct Cells;
    impl<'de> de::Visitor<'de> for Cells {
        type Value = Vec<Option<u32>>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("at most 4096 null or u32 cells")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut cells = Vec::new();
            while let Some(cell) = seq.next_element::<Option<u32>>()? {
                if cells.len() == orr_tilemap::MAX_CELLS {
                    return Err(de::Error::custom("pattern exceeds 4096 cells"));
                }
                cells.push(cell);
            }
            Ok(cells)
        }
    }
    d.deserialize_seq(Cells)
}

fn checked_digest(digest: &str) -> Result<(), String> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("pattern requires the installed package's lowercase SHA-256 digest".into());
    }
    Ok(())
}

impl Asset {
    pub(super) fn encode(
        width: u32,
        height: u32,
        cells: &[Option<u32>],
        source: &ValidatedDocument,
        atlas: &SpriteDocument,
        digest: &str,
    ) -> Result<Vec<u8>, String> {
        let doc = source.document();
        let asset = Self {
            format: "orr_tile_pattern".into(),
            version: 1,
            width,
            height,
            cells: cells.to_vec(),
            tile_size: doc.tile_size,
            sprite_package: doc.sprite.package.clone(),
            sprite_document: doc.sprite.document.clone(),
            package_digest: digest.into(),
            atlas_json: atlas.to_json().map_err(|e| e.to_string())?,
        };
        asset.validate(source, atlas, digest)?;
        let bytes = serde_json::to_vec_pretty(&asset).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_ASSET_BYTES {
            return Err("pattern asset exceeds 32 MiB".into());
        }
        Ok(bytes)
    }
    pub(super) fn decode(
        bytes: &[u8],
        source: &ValidatedDocument,
        atlas: &SpriteDocument,
        digest: &str,
    ) -> Result<Self, String> {
        if bytes.len() > MAX_ASSET_BYTES {
            return Err("pattern asset exceeds 32 MiB".into());
        }
        // Derived struct visitors accept positional arrays. Requiring the first
        // JSON token to be an object retains duplicate-field diagnostics too.
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
            return Err("pattern asset must be a JSON object".into());
        }
        let asset: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid pattern asset: {e}"))?;
        asset.validate(source, atlas, digest)?;
        Ok(asset)
    }
    fn validate(
        &self,
        source: &ValidatedDocument,
        atlas: &SpriteDocument,
        digest: &str,
    ) -> Result<(), String> {
        checked_digest(digest)?;
        if self.format != "orr_tile_pattern" || self.version != 1 {
            return Err("unsupported pattern format or version".into());
        }
        if self.package_digest != digest {
            return Err("pattern package digest differs from installed package".into());
        }
        let doc = source.document();
        if self.sprite_package != doc.sprite.package || self.sprite_document != doc.sprite.document
        {
            return Err("pattern sprite identity differs from current map".into());
        }
        if self.tile_size != doc.tile_size {
            return Err("pattern tile size differs from current map".into());
        }
        if self.atlas_json.len() > orr_sprite::MAX_JSON_BYTES {
            return Err("pattern atlas exceeds 4 MiB".into());
        }
        let current = atlas.to_json().map_err(|e| e.to_string())?;
        if self.atlas_json != current {
            return Err("pattern atlas differs from current canonical atlas".into());
        }
        source.document().validate(atlas)?;
        // Validate independently of the target footprint: a larger pattern may
        // load, but existing Stamp admission still requires whole-map fit.
        let mut candidate = doc.clone();
        candidate.width = self.width;
        candidate.height = self.height;
        candidate.cells = self.cells.clone();
        candidate.validate(atlas)?;
        Ok(())
    }
}

fn checked_path(path: &Path) -> Result<(), String> {
    let text = path.to_str().ok_or("pattern path must be Unicode")?;
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
        return Err("pattern path must be an absolute .json path without dot components, at most 4096 bytes".into());
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
        .ok_or("pattern path has no parent")?
        .ancestors()
    {
        let metadata =
            fs::symlink_metadata(parent).map_err(|e| format!("inspect pattern parent: {e}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || identity(&metadata).1 == 0 {
            return Err("pattern ancestors must be real directories with stable identities".into());
        }
        result.push((parent.to_path_buf(), identity(&metadata)));
    }
    Ok(result)
}

/// Requires trusted, quiescent directories. Observations detect ordinary
/// replacement, not an adversarial ancestor swap-and-restore race.
pub(super) fn read_pattern(path: &Path) -> Result<Vec<u8>, String> {
    checked_path(path)?;
    #[cfg(unix)]
    {
        let parents = ancestry(path)?;
        let before = fs::symlink_metadata(path).map_err(|e| format!("inspect pattern: {e}"))?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || identity(&before).1 == 0
            || before.len() > MAX_ASSET_BYTES as u64
        {
            return Err("pattern must be a regular, bounded file with a stable identity".into());
        }
        let descriptor = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|e| format!("open pattern: {e}"))?;
        let file = fs::File::from(descriptor);
        let held = file.metadata().map_err(|e| e.to_string())?;
        if !held.is_file()
            || observation(&held) != observation(&before)
            || ancestry(path)? != parents
        {
            return Err("pattern changed while opening".into());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_ASSET_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read pattern: {e}"))?;
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
            return Err("pattern changed while reading or exceeded the size bound".into());
        }
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        Err("bounded no-follow pattern reads are unsupported on this platform".into())
    }
}

#[cfg(test)]
mod tests;
